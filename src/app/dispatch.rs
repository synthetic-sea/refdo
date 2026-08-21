use std::{
    io::Read,
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use crate::config::{DispatchDefinition, DispatchSettings};

use super::App;

type Reader = Box<dyn Read + Send>;
type ReaderTask = Box<dyn FnOnce() -> std::io::Result<BoundedCapture> + Send>;
type ReaderHandle = std::thread::JoinHandle<std::io::Result<BoundedCapture>>;

const MAX_GENERATOR_STDOUT_BYTES: usize = 4096;
const MAX_DIAGNOSTIC_STDERR_BYTES: usize = 16384;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProcessRole {
    Dispatch,
    Generator,
}

#[derive(Debug)]
struct BoundedCapture {
    bytes: Vec<u8>,
    discarded: bool,
}

#[derive(Debug)]
struct BoundedProcessOutput {
    status: ExitStatus,
    stdout: Option<BoundedCapture>,
    stderr: BoundedCapture,
}

fn drain_bounded(reader: &mut impl Read, limit: usize) -> std::io::Result<BoundedCapture> {
    let mut bytes = Vec::with_capacity(limit);
    let mut discarded = false;
    let mut buffer = [0u8; 8192];
    loop {
        let bytes_read = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => n,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        let available_space = limit.saturating_sub(bytes.len());
        let to_take = available_space.min(bytes_read);
        bytes.extend_from_slice(&buffer[..to_take]);
        if to_take < bytes_read {
            discarded = true;
        }
    }
    Ok(BoundedCapture { bytes, discarded })
}

fn make_reader_task(limit: usize) -> (mpsc::SyncSender<Reader>, ReaderTask) {
    let (tx, rx) = mpsc::sync_channel::<Reader>(0);
    let task: ReaderTask = Box::new(move || {
        let Ok(mut reader) = rx.recv() else {
            return Ok(BoundedCapture {
                bytes: Vec::new(),
                discarded: false,
            });
        };
        drain_bounded(&mut reader, limit)
    });
    (tx, task)
}

fn run_bash_with(
    source: &str,
    content: &str,
    branch: &str,
    cwd: &std::path::Path,
    role: ProcessRole,
    spawn_reader: &mut impl FnMut(ReaderTask) -> std::io::Result<ReaderHandle>,
    spawn_child: impl FnOnce(&mut Command) -> std::io::Result<Child>,
) -> std::io::Result<BoundedProcessOutput> {
    let (mut stdout_sender, mut stdout_handle) = if role == ProcessRole::Generator {
        let (tx, task) = make_reader_task(MAX_GENERATOR_STDOUT_BYTES);
        let handle = spawn_reader(task)?;
        (Some(tx), Some(handle))
    } else {
        (None, None)
    };

    let (mut stderr_sender, stderr_handle) = {
        let (tx, task) = make_reader_task(MAX_DIAGNOSTIC_STDERR_BYTES);
        match spawn_reader(task) {
            Ok(handle) => (Some(tx), handle),
            Err(start_error) => {
                drop(stdout_sender.take());
                if let Some(handle) = stdout_handle.take() {
                    let _ = handle.join();
                }
                return Err(start_error);
            }
        }
    };

    let mut command = Command::new("bash");
    command
        .arg("-lc")
        .arg(source)
        .arg("refdo-dispatch")
        .arg(content)
        .arg(branch)
        .current_dir(cwd);

    match role {
        ProcessRole::Dispatch => {
            command.stdout(Stdio::null());
            command.stderr(Stdio::piped());
        }
        ProcessRole::Generator => {
            command.stdout(Stdio::piped());
            command.stderr(Stdio::piped());
        }
    }

    let mut child = match spawn_child(&mut command) {
        Ok(child) => child,
        Err(spawn_error) => {
            drop(stdout_sender.take());
            drop(stderr_sender.take());
            if let Some(handle) = stdout_handle.take() {
                let _ = handle.join();
            }
            let _ = stderr_handle.join();
            return Err(spawn_error);
        }
    };

    let mut handoff_error = None;
    if role == ProcessRole::Generator {
        let stdout_pipe = child.stdout.take();
        if let Some(pipe) = stdout_pipe {
            if let Some(sender) = stdout_sender.take()
                && sender.send(Box::new(pipe)).is_err()
            {
                handoff_error = Some(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "failed to hand off child stdout to reader thread",
                ));
            }
        } else {
            handoff_error = Some(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "child stdout pipe was unexpectedly not captured",
            ));
        }
    }

    if handoff_error.is_none() {
        let stderr_pipe = child.stderr.take();
        if let Some(pipe) = stderr_pipe {
            if let Some(sender) = stderr_sender.take()
                && sender.send(Box::new(pipe)).is_err()
            {
                handoff_error = Some(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "failed to hand off child stderr to reader thread",
                ));
            }
        } else {
            handoff_error = Some(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "child stderr pipe was unexpectedly not captured",
            ));
        }
    }

    if let Some(err) = handoff_error {
        drop(stdout_sender.take());
        drop(stderr_sender.take());
        let _ = child.kill();
        let _ = child.wait();
        if let Some(handle) = stdout_handle.take() {
            let _ = handle.join();
        }
        let _ = stderr_handle.join();
        return Err(err);
    }

    let status = match child.wait() {
        Ok(status) => status,
        Err(wait_error) => {
            let _ = child.kill();
            let _ = child.wait();
            if let Some(handle) = stdout_handle.take() {
                let _ = handle.join();
            }
            let _ = stderr_handle.join();
            return Err(wait_error);
        }
    };

    let stdout_result = stdout_handle.take().map(|handle| match handle.join() {
        Ok(read_result) => read_result,
        Err(_) => Err(std::io::Error::other("output reader thread panicked")),
    });

    let stderr_result = match stderr_handle.join() {
        Ok(read_result) => read_result,
        Err(_) => Err(std::io::Error::other("output reader thread panicked")),
    };

    if let Some(Err(stdout_err)) = stdout_result {
        return Err(stdout_err);
    }
    let stderr_capture = stderr_result?;

    Ok(BoundedProcessOutput {
        status,
        stdout: stdout_result.map(|res| res.unwrap()),
        stderr: stderr_capture,
    })
}

pub(super) struct DispatchResult {
    pub(super) name: String,
    pub(super) result: Result<(), String>,
}

pub(super) struct DispatchController {
    settings: DispatchSettings,
    sender: Sender<DispatchResult>,
    receiver: Receiver<DispatchResult>,
    running: bool,
}

impl Default for DispatchController {
    fn default() -> Self {
        Self::new(DispatchSettings::default())
    }
}

impl DispatchController {
    pub(super) fn new(settings: DispatchSettings) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            settings,
            sender,
            receiver,
            running: false,
        }
    }

    pub(super) fn start(
        &mut self,
        name: &str,
        definition: DispatchDefinition,
        content: String,
        worktree_path: PathBuf,
    ) -> Result<(), String> {
        if self.running {
            return Err("dispatch: another dispatch is already running".to_owned());
        }

        let needs_branch = definition.command.contains("{{BRANCH}}");
        let generator = if needs_branch {
            Some(
                self.settings
                    .generate_branch_name_command
                    .clone()
                    .ok_or_else(|| {
                        format!(
                            "dispatch: '{name}' requires {{{{BRANCH}}}}, but no branch generator is configured"
                        )
                    })?,
            )
        } else {
            None
        };

        let request = WorkerRequest {
            name: name.to_owned(),
            content,
            worktree_path,
            dispatch_source: render_template(&definition.command),
            generator_source: generator.as_deref().map(render_template),
        };
        let result_name = request.name.clone();
        let sender = self.sender.clone();

        self.running = true;
        if let Err(error) = thread::Builder::new()
            .name("refdo-dispatch".to_owned())
            .spawn(move || {
                let result = run_worker(&request);
                let _ = sender.send(DispatchResult {
                    name: result_name,
                    result,
                });
            })
        {
            self.running = false;
            return Err(format!("dispatch: failed to start '{name}': {error}"));
        }

        Ok(())
    }

    pub(super) fn take_result(&mut self) -> Option<DispatchResult> {
        match self.receiver.try_recv() {
            Ok(result) => {
                self.running = false;
                Some(result)
            }
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => None,
        }
    }
}

impl App {
    pub(super) fn refresh_dispatch(&mut self) {
        let Some(result) = self.dispatch.take_result() else {
            return;
        };

        self.error = Some(match result.result {
            Ok(()) => format!("dispatch: '{}' completed", result.name),
            Err(reason) => format!("dispatch: '{}' failed: {reason}", result.name),
        });
    }
}

struct WorkerRequest {
    name: String,
    content: String,
    worktree_path: PathBuf,
    dispatch_source: String,
    generator_source: Option<String>,
}

fn render_template(template: &str) -> String {
    let mut rendered = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        rendered.push_str(&rest[..open]);
        let token = &rest[open..];
        if let Some(after) = token.strip_prefix("{{CONTENT}}") {
            rendered.push_str("\"$1\"");
            rest = after;
        } else if let Some(after) = token.strip_prefix("{{BRANCH}}") {
            rendered.push_str("\"$2\"");
            rest = after;
        } else {
            rendered.push_str("{{");
            rest = &token[2..];
        }
    }
    rendered.push_str(rest);
    rendered
}

fn run_worker(request: &WorkerRequest) -> Result<(), String> {
    let branch = match request.generator_source.as_deref() {
        Some(source) => run_generator(source, &request.content, &request.worktree_path)?,
        None => String::new(),
    };

    let output = run_bash(
        &request.dispatch_source,
        &request.content,
        &branch,
        &request.worktree_path,
        ProcessRole::Dispatch,
    )
    .map_err(|error| format!("could not start command: {error}"))?;

    command_success_ref(&output, "command")
}

fn run_generator(source: &str, content: &str, cwd: &std::path::Path) -> Result<String, String> {
    let output = run_bash(source, content, "", cwd, ProcessRole::Generator)
        .map_err(|error| format!("could not start branch generator: {error}"))?;
    command_success_ref(&output, "branch generator")?;

    let stdout_capture = output
        .stdout
        .as_ref()
        .ok_or_else(|| "could not read branch generator output".to_owned())?;
    if stdout_capture.discarded {
        return Err("branch generator output exceeds 4096 bytes".to_owned());
    }
    let stdout = std::str::from_utf8(&stdout_capture.bytes)
        .map_err(|_| "branch generator output is not valid UTF-8".to_owned())?;
    let branch = stdout.trim();
    if branch.is_empty() {
        return Err("branch generator produced no branch name".to_owned());
    }
    if branch.lines().count() != 1 {
        return Err("branch generator produced more than one line".to_owned());
    }
    Ok(branch.to_owned())
}

fn run_bash(
    source: &str,
    content: &str,
    branch: &str,
    cwd: &std::path::Path,
    role: ProcessRole,
) -> std::io::Result<BoundedProcessOutput> {
    run_bash_with(
        source,
        content,
        branch,
        cwd,
        role,
        &mut |task| {
            thread::Builder::new()
                .name("refdo-dispatch-reader".to_owned())
                .spawn(task)
        },
        |command| command.spawn(),
    )
}

fn floor_char_boundary(s: &str, mut index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    while index > 0 && !s.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn format_diagnostic_line(stderr: &BoundedCapture) -> Option<String> {
    let lossy = String::from_utf8_lossy(&stderr.bytes);
    let mut remainder = &lossy[..];
    while !remainder.is_empty() {
        let (line_with_newline, has_newline, next_remainder) = match remainder.find('\n') {
            Some(idx) => (&remainder[..idx], true, &remainder[idx + 1..]),
            None => (remainder, false, ""),
        };
        let trimmed_line = line_with_newline.trim_end_matches('\r').trim();
        if !trimmed_line.is_empty() {
            let is_incomplete = stderr.discarded && !has_newline;
            if !is_incomplete && trimmed_line.len() <= MAX_DIAGNOSTIC_STDERR_BYTES {
                return Some(trimmed_line.to_owned());
            }
            let max_slice_len = MAX_DIAGNOSTIC_STDERR_BYTES.saturating_sub('…'.len_utf8());
            let truncate_at = if trimmed_line.len() > max_slice_len {
                floor_char_boundary(trimmed_line, max_slice_len)
            } else {
                trimmed_line.len()
            };
            let mut result = trimmed_line[..truncate_at].to_owned();
            result.push('…');
            return Some(result);
        }
        remainder = next_remainder;
    }
    None
}

fn command_success_ref(output: &BoundedProcessOutput, label: &str) -> Result<(), String> {
    if output.status.success() {
        return Ok(());
    }

    if let Some(line) = format_diagnostic_line(&output.stderr) {
        return Err(line);
    }

    Err(format!("{label} exited with status {}", output.status))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::{
            Arc,
            atomic::{AtomicU64, AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };

    use super::*;

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "refdo-dispatch-test-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct ChunkedReader {
        total_bytes: usize,
        bytes_read: usize,
        chunk_size: usize,
        eof_reached: bool,
    }

    impl ChunkedReader {
        fn new(total_bytes: usize, chunk_size: usize) -> Self {
            Self {
                total_bytes,
                bytes_read: 0,
                chunk_size,
                eof_reached: false,
            }
        }
    }

    impl Read for ChunkedReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let remaining = self.total_bytes - self.bytes_read;
            if remaining == 0 {
                self.eof_reached = true;
                return Ok(0);
            }
            let to_read = self.chunk_size.min(remaining).min(buf.len());
            buf[..to_read].fill(b'a');
            self.bytes_read += to_read;
            Ok(to_read)
        }
    }

    fn definition(command: &str) -> DispatchDefinition {
        DispatchDefinition {
            command: command.to_owned(),
        }
    }

    struct TestController {
        controller: DispatchController,
        definition: DispatchDefinition,
    }

    impl std::ops::Deref for TestController {
        type Target = DispatchController;

        fn deref(&self) -> &Self::Target {
            &self.controller
        }
    }

    impl std::ops::DerefMut for TestController {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.controller
        }
    }

    fn controller(command: &str, generator: Option<&str>) -> TestController {
        TestController {
            controller: DispatchController::new(DispatchSettings {
                generate_branch_name_command: generator.map(str::to_owned),
            }),
            definition: definition(command),
        }
    }

    fn wait_for_result(controller: &mut DispatchController) -> DispatchResult {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(result) = controller.take_result() {
                return result;
            }
            assert!(Instant::now() < deadline, "dispatch did not finish in time");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn start_and_wait(
        controller: &mut TestController,
        content: &str,
        cwd: &Path,
    ) -> Result<(), String> {
        let definition = controller.definition.clone();
        controller
            .start("implement", definition, content.to_owned(), cwd.to_owned())
            .unwrap();
        wait_for_result(controller).result
    }

    #[test]
    fn renders_repeated_placeholders_in_one_pass() {
        assert_eq!(
            render_template("run {{CONTENT}} {{BRANCH}} {{CONTENT}} literal"),
            "run \"$1\" \"$2\" \"$1\" literal"
        );
        assert_eq!(
            render_template("printf '{{UNKNOWN}}'"),
            "printf '{{UNKNOWN}}'"
        );
    }

    #[test]
    fn hostile_content_is_one_argument_and_is_not_reparsed() {
        let directory = TestDirectory::new();
        let output = directory.path().join("argument");
        let command = format!(
            "printf '%s\\n' \"$#\" > {}; printf '%s' {{{{CONTENT}}}} >> {}",
            output.display(),
            output.display()
        );
        let mut controller = controller(&command, None);
        let content = "spaces ü $HOME $(touch injected)\n'\" {{BRANCH}}";

        assert_eq!(
            start_and_wait(&mut controller, content, directory.path()),
            Ok(())
        );
        assert_eq!(fs::read_to_string(output).unwrap(), format!("2\n{content}"));
        assert!(!directory.path().join("injected").exists());
    }

    #[test]
    fn implement_shape_receives_three_arguments_and_worktree_cwd() {
        let directory = TestDirectory::new();
        let scripts = directory.path().join("scripts");
        fs::create_dir_all(&scripts).unwrap();
        fs::write(
            scripts.join("implement.sh"),
            "#!/bin/bash\nprintf '%s\\n' \"$#\" \"$1\" \"$2\" \"$3\" \"$PWD\" > result\n",
        )
        .unwrap();
        let status = Command::new("chmod")
            .args(["+x", "scripts/implement.sh"])
            .current_dir(directory.path())
            .status()
            .unwrap();
        assert!(status.success());
        let mut controller = controller(
            "./scripts/implement.sh {{BRANCH}} omp {{CONTENT}}",
            Some("printf 'generated-branch\\n'"),
        );

        assert_eq!(
            start_and_wait(&mut controller, "Ship it safely", directory.path()),
            Ok(())
        );
        let expected_cwd = directory.path().canonicalize().unwrap();
        assert_eq!(
            fs::read_to_string(directory.path().join("result")).unwrap(),
            format!(
                "3\ngenerated-branch\nomp\nShip it safely\n{}\n",
                expected_cwd.display()
            )
        );
    }

    #[test]
    fn generator_is_skipped_without_branch_placeholder() {
        let directory = TestDirectory::new();
        let marker = directory.path().join("generator-ran");
        let done = directory.path().join("done");
        let mut controller = controller(
            &format!(
                "printf done > {}; printf '%s' {{{{CONTENT}}}} >> {}",
                done.display(),
                done.display()
            ),
            Some(&format!("touch {}; printf branch", marker.display())),
        );

        assert_eq!(
            start_and_wait(&mut controller, "content", directory.path()),
            Ok(())
        );
        assert!(!marker.exists());
        assert_eq!(fs::read_to_string(done).unwrap(), "donecontent");
    }

    #[test]
    fn generator_runs_first_and_its_trimmed_line_is_bound_to_branch() {
        let directory = TestDirectory::new();
        let order = directory.path().join("order");
        let mut controller = controller(
            &format!("printf 'dispatch:%s' {{{{BRANCH}}}} >> {}", order.display()),
            Some(&format!(
                "printf generator > {}; printf 'feature/name\\n'",
                order.display()
            )),
        );

        assert_eq!(
            start_and_wait(&mut controller, "todo", directory.path()),
            Ok(())
        );
        assert_eq!(
            fs::read_to_string(order).unwrap(),
            "generatordispatch:feature/name"
        );
    }

    #[test]
    fn invalid_generator_output_never_runs_dispatch() {
        for (generator, expected) in [
            ("printf ''", "branch generator produced no branch name"),
            (
                "printf 'one\\ntwo\\n'",
                "branch generator produced more than one line",
            ),
            (
                "printf '\\377'",
                "branch generator output is not valid UTF-8",
            ),
            (
                "printf 'generator failed\\n' >&2; exit 7",
                "generator failed",
            ),
        ] {
            let directory = TestDirectory::new();
            let marker = directory.path().join("dispatch-ran");
            let mut controller = controller(
                &format!("touch {}; printf '%s' {{{{BRANCH}}}}", marker.display()),
                Some(generator),
            );

            assert_eq!(
                start_and_wait(&mut controller, "todo", directory.path()),
                Err(expected.to_owned())
            );
            assert!(!marker.exists());
        }
    }

    #[test]
    fn cwd_and_nonzero_failures_are_concise() {
        let directory = TestDirectory::new();
        let missing = directory.path().join("missing");
        let mut bad_cwd = controller("true", None);
        let cwd_error = start_and_wait(&mut bad_cwd, "todo", &missing).unwrap_err();
        assert!(cwd_error.starts_with("could not start command: "));
        assert!(!cwd_error.contains('\n'));

        let mut stderr = controller("printf '\\nfixture failed\\nignored\\n' >&2; exit 9", None);
        assert_eq!(
            start_and_wait(&mut stderr, "todo", directory.path()),
            Err("fixture failed".to_owned())
        );

        let mut status = controller("exit 11", None);
        let status_error = start_and_wait(&mut status, "todo", directory.path()).unwrap_err();
        assert!(status_error.starts_with("command exited with status "));
        assert!(!status_error.contains('\n'));
    }

    #[test]
    fn running_guard_resets_after_result_is_taken() {
        let directory = TestDirectory::new();
        let mut controller = controller("sleep 0.1", None);
        controller
            .start(
                "implement",
                definition("sleep 0.1"),
                "first".to_owned(),
                directory.path().to_owned(),
            )
            .unwrap();

        assert_eq!(
            controller.start(
                "implement",
                definition("sleep 0.1"),
                "second".to_owned(),
                directory.path().to_owned()
            ),
            Err("dispatch: another dispatch is already running".to_owned())
        );
        assert_eq!(wait_for_result(&mut controller).result, Ok(()));
        controller
            .start(
                "implement",
                definition("sleep 0.1"),
                "third".to_owned(),
                directory.path().to_owned(),
            )
            .unwrap();
        assert_eq!(wait_for_result(&mut controller).result, Ok(()));
    }

    #[test]
    fn branch_placeholder_requires_generator_before_spawning() {
        let directory = TestDirectory::new();
        let mut controller = controller("printf '%s' {{BRANCH}}", None);

        assert_eq!(
            controller.start(
                "implement",
                definition("printf '%s' {{BRANCH}}"),
                "todo".to_owned(),
                directory.path().to_owned(),
            ),
            Err(
                "dispatch: 'implement' requires {{BRANCH}}, but no branch generator is configured"
                    .to_owned()
            )
        );
        assert!(!controller.running);
    }

    #[test]
    fn drain_bounded_caps_length_and_capacity_and_reaches_eof() {
        for limit in [MAX_GENERATOR_STDOUT_BYTES, MAX_DIAGNOSTIC_STDERR_BYTES] {
            let mut reader = ChunkedReader::new(limit + 1024, 257);
            let capture = drain_bounded(&mut reader, limit).unwrap();
            assert!(reader.eof_reached, "drain_bounded must continue to EOF");
            assert_eq!(capture.bytes.len(), limit);
            assert!(capture.bytes.capacity() <= limit);
            assert!(capture.discarded);
        }
    }

    #[test]
    fn reader_start_failure_aborts_before_spawning_and_joins_started_readers() {
        let reader_exited = Arc::new(AtomicUsize::new(0));
        let reader_exited_clone = Arc::clone(&reader_exited);

        let mut start_count = 0;
        let mut spawn_count = 0;

        let mut spawn_reader = move |task: ReaderTask| -> std::io::Result<ReaderHandle> {
            start_count += 1;
            if start_count == 1 {
                let exited = Arc::clone(&reader_exited_clone);
                thread::Builder::new().spawn(move || {
                    let res = task();
                    exited.fetch_add(1, Ordering::SeqCst);
                    res
                })
            } else {
                Err(std::io::Error::other("injected reader start error"))
            }
        };

        let spawn_child = |_command: &mut Command| -> std::io::Result<Child> {
            spawn_count += 1;
            Err(std::io::Error::other("spawn called"))
        };

        let result = run_bash_with(
            "true",
            "",
            "",
            Path::new("."),
            ProcessRole::Generator,
            &mut spawn_reader,
            spawn_child,
        );

        assert!(result.is_err());
        assert_eq!(spawn_count, 0, "child spawn hook must not be called");
        assert_eq!(
            reader_exited.load(Ordering::SeqCst),
            1,
            "first reader task must be joined before return"
        );
    }

    #[test]
    fn high_volume_stdout_and_stderr_are_drained_without_deadlock() {
        let directory = TestDirectory::new();

        // 1. High-volume dispatch stdout is discarded and completes inside deadline
        let mut stdout_controller = controller(
            "for ((i=0;i<70000;i++)); do printf '0123456789abcdef0123456789abcdef'; done",
            None,
        );
        assert_eq!(
            start_and_wait(&mut stdout_controller, "todo", directory.path()),
            Ok(())
        );

        // 2. High-volume dispatch stderr is ignored on exit 0
        let mut stderr_controller = controller(
            "for ((i=0;i<70000;i++)); do printf '0123456789abcdef0123456789abcdef' >&2; done",
            None,
        );
        assert_eq!(
            start_and_wait(&mut stderr_controller, "todo", directory.path()),
            Ok(())
        );

        // 3. Failing dispatch with short first line retains exact first line despite later large stderr
        let mut failing_controller = controller(
            "printf 'first error line\\n' >&2; for ((i=0;i<70000;i++)); do printf '0123456789abcdef0123456789abcdef' >&2; done; exit 1",
            None,
        );
        assert_eq!(
            start_and_wait(&mut failing_controller, "todo", directory.path()),
            Err("first error line".to_owned())
        );

        // 4. Failing dispatch with invalid stderr returns valid UTF-8 bounded with … and at most 16384 bytes
        let mut invalid_stderr_controller = controller(
            "for ((i=0;i<20000;i++)); do printf '\\377' >&2; done; exit 1",
            None,
        );
        let error =
            start_and_wait(&mut invalid_stderr_controller, "todo", directory.path()).unwrap_err();
        assert!(error.ends_with('…'));
        assert!(error.len() <= MAX_DIAGNOSTIC_STDERR_BYTES);
        assert!(std::str::from_utf8(error.as_bytes()).is_ok());

        // 5. Failing generator with large stderr retains first line without deadlock
        let mut failing_gen_controller = controller(
            "printf '%s' {{BRANCH}}",
            Some(
                "printf 'generator failure line\\n' >&2; for ((i=0;i<70000;i++)); do printf '0123456789abcdef0123456789abcdef' >&2; done; exit 1",
            ),
        );
        assert_eq!(
            start_and_wait(&mut failing_gen_controller, "todo", directory.path()),
            Err("generator failure line".to_owned())
        );
    }

    #[test]
    fn generator_output_boundary_exactly_4096_versus_4097() {
        let directory = TestDirectory::new();

        // 4096 bytes reaches normal one-line handling
        let marker_4096 = directory.path().join("dispatch-ran-4096");
        let mut controller_4096 = controller(
            &format!(
                "touch {}; printf '%s' {{{{BRANCH}}}}",
                marker_4096.display()
            ),
            Some("for ((i=0;i<4096;i++)); do printf 'x'; done"),
        );
        assert_eq!(
            start_and_wait(&mut controller_4096, "todo", directory.path()),
            Ok(())
        );
        assert!(marker_4096.exists());

        // 4097 bytes exceeds 4096 bytes and leaves dispatch marker absent
        let marker_4097 = directory.path().join("dispatch-ran-4097");
        let mut controller_4097 = controller(
            &format!(
                "touch {}; printf '%s' {{{{BRANCH}}}}",
                marker_4097.display()
            ),
            Some("for ((i=0;i<4097;i++)); do printf 'x'; done"),
        );
        assert_eq!(
            start_and_wait(&mut controller_4097, "todo", directory.path()),
            Err("branch generator output exceeds 4096 bytes".to_owned())
        );
        assert!(!marker_4097.exists());
    }
}
