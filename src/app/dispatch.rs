use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::{Command, Output},
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use crate::config::{DispatchDefinition, DispatchSettings};

use super::App;

pub(super) struct DispatchResult {
    pub(super) name: String,
    pub(super) result: Result<(), String>,
}

pub(super) struct DispatchController {
    settings: DispatchSettings,
    definitions: BTreeMap<String, DispatchDefinition>,
    sender: Sender<DispatchResult>,
    receiver: Receiver<DispatchResult>,
    running: bool,
}

impl Default for DispatchController {
    fn default() -> Self {
        Self::new(DispatchSettings::default(), BTreeMap::new())
    }
}

impl DispatchController {
    pub(super) fn new(
        settings: DispatchSettings,
        definitions: BTreeMap<String, DispatchDefinition>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            settings,
            definitions,
            sender,
            receiver,
            running: false,
        }
    }

    pub(super) fn start(
        &mut self,
        name: &str,
        content: String,
        worktree_path: PathBuf,
    ) -> Result<(), String> {
        if self.running {
            return Err("dispatch: another dispatch is already running".to_owned());
        }
        let definition = self
            .definitions
            .get(name)
            .cloned()
            .ok_or_else(|| format!("dispatch: unknown dispatch '{name}'"))?;

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
    )
    .map_err(|error| format!("could not start command: {error}"))?;

    command_success_ref(&output, "command")
}

fn run_generator(source: &str, content: &str, cwd: &std::path::Path) -> Result<String, String> {
    let output = run_bash(source, content, "", cwd)
        .map_err(|error| format!("could not start branch generator: {error}"))?;
    command_success_ref(&output, "branch generator")?;

    let stdout = std::str::from_utf8(&output.stdout)
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
) -> std::io::Result<Output> {
    Command::new("bash")
        .arg("-lc")
        .arg(source)
        .arg("refdo-dispatch")
        .arg(content)
        .arg(branch)
        .current_dir(cwd)
        .output()
}

fn command_success_ref(output: &Output, label: &str) -> Result<(), String> {
    if output.status.success() {
        return Ok(());
    }

    if let Some(line) = String::from_utf8_lossy(&output.stderr)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
    {
        return Err(line.to_owned());
    }

    Err(format!("{label} exited with status {}", output.status))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
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

    fn definition(command: &str) -> BTreeMap<String, DispatchDefinition> {
        BTreeMap::from([(
            "implement".to_owned(),
            DispatchDefinition {
                command: command.to_owned(),
            },
        )])
    }

    fn controller(command: &str, generator: Option<&str>) -> DispatchController {
        DispatchController::new(
            DispatchSettings {
                generate_branch_name_command: generator.map(str::to_owned),
            },
            definition(command),
        )
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
        controller: &mut DispatchController,
        content: &str,
        cwd: &Path,
    ) -> Result<(), String> {
        controller
            .start("implement", content.to_owned(), cwd.to_owned())
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
            .start("implement", "first".to_owned(), directory.path().to_owned())
            .unwrap();

        assert_eq!(
            controller.start(
                "implement",
                "second".to_owned(),
                directory.path().to_owned()
            ),
            Err("dispatch: another dispatch is already running".to_owned())
        );
        assert_eq!(wait_for_result(&mut controller).result, Ok(()));
        controller
            .start("implement", "third".to_owned(), directory.path().to_owned())
            .unwrap();
        assert_eq!(wait_for_result(&mut controller).result, Ok(()));
    }

    #[test]
    fn branch_placeholder_requires_generator_before_spawning() {
        let directory = TestDirectory::new();
        let mut controller = controller("printf '%s' {{BRANCH}}", None);

        assert_eq!(
            controller.start("implement", "todo".to_owned(), directory.path().to_owned()),
            Err(
                "dispatch: 'implement' requires {{BRANCH}}, but no branch generator is configured"
                    .to_owned()
            )
        );
        assert!(!controller.running);
    }
}
