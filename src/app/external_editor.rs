use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{self, Write},
    process::{Command, ExitStatus, Stdio},
};

use tempfile::{Builder, NamedTempFile};

use crate::storage::Todo;

pub(super) struct PreparedEdit {
    program: OsString,
    arguments: Vec<OsString>,
    temp_file: NamedTempFile,
    marker_token: String,
}

#[derive(Debug)]
pub(super) struct EditedTodo {
    pub title: String,
    pub body: String,
    temp_file: NamedTempFile,
}

pub(super) fn prepare(todo: &Todo) -> Result<PreparedEdit, String> {
    let visual = std::env::var_os("VISUAL");
    let editor = std::env::var_os("EDITOR");
    let (program, arguments) = resolve_editor_command(visual.as_deref(), editor.as_deref())?;
    let mut temp_file = Builder::new()
        .prefix("refdo-")
        .suffix(".md")
        .tempfile()
        .map_err(|error| format!("editor: could not create temporary file: {error}"))?;
    let marker_token = temp_file
        .path()
        .file_name()
        .expect("temporary file has a name")
        .to_string_lossy()
        .into_owned();
    let body_marker = body_marker(&marker_token);
    let end_marker = end_marker(&marker_token);
    write!(
        temp_file,
        "{}\n{}\n{}\n{}\n",
        todo.title, body_marker, todo.body, end_marker
    )
    .and_then(|()| temp_file.flush())
    .map_err(|error| format!("editor: could not write temporary file: {error}"))?;

    Ok(PreparedEdit {
        program,
        arguments,
        temp_file,
        marker_token,
    })
}

impl PreparedEdit {
    pub(super) fn launch(&self) -> io::Result<ExitStatus> {
        Command::new(&self.program)
            .args(&self.arguments)
            .arg(self.temp_file.path())
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
    }

    pub(super) fn finish(self, status: ExitStatus) -> Result<EditedTodo, String> {
        if !status.success() {
            return Err(self.preserve(&format!("editor exited with status {status}")));
        }

        let bytes = match fs::read(self.temp_file.path()) {
            Ok(bytes) => bytes,
            Err(error) => {
                return Err(self.preserve(&format!("could not read edited file: {error}")));
            }
        };
        let contents = match String::from_utf8(bytes) {
            Ok(contents) => contents.replace("\r\n", "\n"),
            Err(error) => {
                return Err(self.preserve(&format!("edited file is not valid UTF-8: {error}")));
            }
        };
        let (title, body) = match parse_document(&contents, &self.marker_token) {
            Ok(todo) => todo,
            Err(error) => return Err(self.preserve(&error)),
        };

        Ok(EditedTodo {
            title,
            body,
            temp_file: self.temp_file,
        })
    }

    pub(super) fn preserve(self, reason: &str) -> String {
        preserve_file(self.temp_file, reason)
    }
}

impl EditedTodo {
    pub(super) fn preserve(self, reason: &str) -> String {
        preserve_file(self.temp_file, reason)
    }
}

pub(super) fn resolve_editor_command(
    visual: Option<&OsStr>,
    editor: Option<&OsStr>,
) -> Result<(OsString, Vec<OsString>), String> {
    let (name, value) = if visual.is_some_and(|value| !value.is_empty()) {
        ("VISUAL", visual.expect("checked above"))
    } else if editor.is_some_and(|value| !value.is_empty()) {
        ("EDITOR", editor.expect("checked above"))
    } else {
        return Err("editor: neither VISUAL nor EDITOR is set".to_owned());
    };
    let value = value
        .to_str()
        .ok_or_else(|| format!("editor: {name} is not valid UTF-8"))?;
    let mut words = shell_words::split(value)
        .map_err(|error| format!("editor: could not parse {name}: {error}"))?
        .into_iter();
    let program = words
        .next()
        .filter(|program| !program.is_empty())
        .ok_or_else(|| format!("editor: {name} command is empty"))?;

    Ok((program.into(), words.map(OsString::from).collect()))
}
#[cfg(test)]
pub(super) fn edited_for_test(title: &str, body: &str) -> EditedTodo {
    EditedTodo {
        title: title.to_owned(),
        body: body.to_owned(),
        temp_file: Builder::new()
            .prefix("refdo-test-edited-")
            .suffix(".md")
            .tempfile()
            .unwrap(),
    }
}

fn parse_document(contents: &str, marker_token: &str) -> Result<(String, String), String> {
    let body_marker = body_marker(marker_token);
    let end_marker = end_marker(marker_token);
    let body_offsets = marker_line_offsets(contents, &body_marker);
    let end_offsets = marker_line_offsets(contents, &end_marker);
    if body_offsets.len() != 1 || end_offsets.len() != 1 {
        return Err("edited document must contain each generated marker exactly once".to_owned());
    }
    let (body_start, body_end) = body_offsets[0];
    let (end_start, end_end) = end_offsets[0];
    if body_start >= end_start {
        return Err("edited document markers are out of order".to_owned());
    }
    if !contents[end_end..].is_empty() {
        return Err("edited document contains content after the end marker".to_owned());
    }

    let title_with_boundary = &contents[..body_start];
    let body_with_boundary = &contents[body_end..end_start];
    let title = title_with_boundary
        .strip_suffix('\n')
        .ok_or_else(|| "edited document is missing the title boundary newline".to_owned())?
        .trim()
        .to_owned();
    if title.is_empty() {
        return Err("todo title cannot be empty".to_owned());
    }
    let body = body_with_boundary
        .strip_suffix('\n')
        .ok_or_else(|| "edited document is missing the body boundary newline".to_owned())?
        .to_owned();

    Ok((title, body))
}

fn marker_line_offsets(contents: &str, marker: &str) -> Vec<(usize, usize)> {
    let mut offsets = Vec::new();
    let mut start = 0;
    for line in contents.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        if content == marker {
            offsets.push((start, start + line.len()));
        }
        start += line.len();
    }
    offsets
}

fn body_marker(marker_token: &str) -> String {
    format!("<!-- refdo:body:{marker_token} -->")
}

fn end_marker(marker_token: &str) -> String {
    format!("<!-- refdo:end:{marker_token} -->")
}

fn preserve_file(temp_file: NamedTempFile, reason: &str) -> String {
    match temp_file.keep() {
        Ok((_file, path)) => format!("editor: {reason}; edits preserved at {}", path.display()),
        Err(error) => format!(
            "editor: {reason}; could not preserve temporary file: {}",
            error.error
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, fs, process::Command};

    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;

    use super::*;

    fn todo(title: &str, body: &str) -> Todo {
        Todo {
            id: 1,
            branch_ref: "refs/heads/main".to_owned(),
            title: title.to_owned(),
            body: body.to_owned(),
            completed: false,
            sort_order: 0,
        }
    }

    fn successful_status() -> ExitStatus {
        Command::new("true").status().unwrap()
    }

    #[test]
    fn visual_takes_precedence_and_arguments_are_parsed_without_a_shell() {
        let (program, arguments) = resolve_editor_command(
            Some(OsStr::new("'/Applications/Visual Editor' -f \"two words\"")),
            Some(OsStr::new("fallback --wait")),
        )
        .unwrap();

        assert_eq!(program, "/Applications/Visual Editor");
        assert_eq!(arguments, ["-f", "two words"]);
    }

    #[test]
    fn empty_visual_falls_back_to_editor() {
        let command =
            resolve_editor_command(Some(OsStr::new("")), Some(OsStr::new("vim -f"))).unwrap();

        assert_eq!(command, ("vim".into(), vec!["-f".into()]));
    }

    #[test]
    fn rejects_missing_empty_and_malformed_commands() {
        assert_eq!(
            resolve_editor_command(None, None).unwrap_err(),
            "editor: neither VISUAL nor EDITOR is set"
        );
        assert_eq!(
            resolve_editor_command(Some(OsStr::new("")), Some(OsStr::new(""))).unwrap_err(),
            "editor: neither VISUAL nor EDITOR is set"
        );
        assert!(
            resolve_editor_command(Some(OsStr::new("   ")), None)
                .unwrap_err()
                .contains("VISUAL command is empty")
        );
        assert!(
            resolve_editor_command(Some(OsStr::new("vim 'unterminated")), None)
                .unwrap_err()
                .contains("could not parse VISUAL")
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_non_unicode_editor_variables() {
        let invalid = OsString::from_vec(vec![0xff]);
        assert_eq!(
            resolve_editor_command(Some(&invalid), Some(OsStr::new("vim"))).unwrap_err(),
            "editor: VISUAL is not valid UTF-8"
        );
        assert_eq!(
            resolve_editor_command(None, Some(&invalid)).unwrap_err(),
            "editor: EDITOR is not valid UTF-8"
        );
    }

    #[test]
    fn round_trips_multiline_titles_and_body_trailing_blank_lines() {
        let prepared = prepare_with_command(&todo(
            "first title line\nsecond title line",
            "## Body\n\nparagraph\n\n",
        ));

        let edited = prepared.finish(successful_status()).unwrap();

        assert_eq!(edited.title, "first title line\nsecond title line");
        assert_eq!(edited.body, "## Body\n\nparagraph\n\n");
    }

    #[test]
    fn round_trips_an_empty_body() {
        let prepared = prepare_with_command(&todo("title", ""));

        let edited = prepared.finish(successful_status()).unwrap();

        assert_eq!(edited.title, "title");
        assert_eq!(edited.body, "");
    }

    #[test]
    fn normalizes_crlf_without_changing_body_whitespace() {
        let prepared = prepare_with_command(&todo("title", "first\n\nlast\n"));
        let path = prepared.temp_file.path().to_owned();
        let contents = fs::read_to_string(&path).unwrap().replace('\n', "\r\n");
        fs::write(path, contents).unwrap();

        let edited = prepared.finish(successful_status()).unwrap();

        assert_eq!(edited.title, "title");
        assert_eq!(edited.body, "first\n\nlast\n");
    }

    #[test]
    fn rejects_missing_duplicated_and_reordered_markers() {
        let prepared = prepare_with_command(&todo("title", "body"));
        let path = prepared.temp_file.path().to_owned();
        let contents = fs::read_to_string(&path).unwrap();
        fs::write(
            &path,
            contents.replace(&body_marker(&prepared.marker_token), "removed"),
        )
        .unwrap();
        let message = prepared.finish(successful_status()).unwrap_err();
        remove_preserved(&message);
        assert!(message.contains("each generated marker exactly once"));

        let prepared = prepare_with_command(&todo("title", "body"));
        let path = prepared.temp_file.path().to_owned();
        let contents = fs::read_to_string(&path).unwrap();
        let marker = body_marker(&prepared.marker_token);
        fs::write(
            &path,
            contents.replace(&marker, &format!("{marker}\n{marker}")),
        )
        .unwrap();
        let message = prepared.finish(successful_status()).unwrap_err();
        remove_preserved(&message);
        assert!(message.contains("each generated marker exactly once"));

        let prepared = prepare_with_command(&todo("title", "body"));
        let path = prepared.temp_file.path().to_owned();
        let reversed = format!(
            "title\n{}\nbody\n{}\n",
            end_marker(&prepared.marker_token),
            body_marker(&prepared.marker_token)
        );
        fs::write(path, reversed).unwrap();
        let message = prepared.finish(successful_status()).unwrap_err();
        remove_preserved(&message);
        assert!(message.contains("markers are out of order"));
    }

    #[test]
    fn rejects_an_empty_trimmed_title_and_preserves_the_document() {
        let prepared = prepare_with_command(&todo("title", "body"));
        let path = prepared.temp_file.path().to_owned();
        let contents = fs::read_to_string(&path).unwrap();
        let body_start = contents.find(&body_marker(&prepared.marker_token)).unwrap();
        fs::write(path, format!(" \t\n{}", &contents[body_start..])).unwrap();

        let message = prepared.finish(successful_status()).unwrap_err();

        assert!(message.contains("todo title cannot be empty"));
        assert_preserved(&message);
        remove_preserved(&message);
    }

    #[test]
    fn nonzero_status_preserves_the_document() {
        let prepared = prepare_with_command(&todo("title", "body"));
        let status = Command::new("false").status().unwrap();

        let message = prepared.finish(status).unwrap_err();

        assert!(message.contains("editor exited with status"));
        assert_preserved(&message);
        remove_preserved(&message);
    }

    #[test]
    fn prepared_and_edited_preservation_keep_the_temporary_file() {
        let prepared = prepare_with_command(&todo("title", "body"));
        let message = prepared.preserve("launch failed");
        assert!(message.starts_with("editor: launch failed; edits preserved at "));
        assert_preserved(&message);
        remove_preserved(&message);

        let edited = prepare_with_command(&todo("title", "body"))
            .finish(successful_status())
            .unwrap();
        let message = edited.preserve("database failed");
        assert!(message.starts_with("editor: database failed; edits preserved at "));
        assert_preserved(&message);
        remove_preserved(&message);
    }

    fn prepare_with_command(todo: &Todo) -> PreparedEdit {
        let mut prepared = PreparedEdit {
            program: "true".into(),
            arguments: Vec::new(),
            temp_file: Builder::new()
                .prefix("refdo-test-")
                .suffix(".md")
                .tempfile()
                .unwrap(),
            marker_token: String::new(),
        };
        prepared.marker_token = prepared
            .temp_file
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        write!(
            prepared.temp_file,
            "{}\n{}\n{}\n{}\n",
            todo.title,
            body_marker(&prepared.marker_token),
            todo.body,
            end_marker(&prepared.marker_token)
        )
        .unwrap();
        prepared.temp_file.flush().unwrap();
        prepared
    }

    fn preserved_path(message: &str) -> &str {
        message
            .split_once("; edits preserved at ")
            .expect("message includes preserved path")
            .1
    }

    fn assert_preserved(message: &str) {
        assert!(fs::metadata(preserved_path(message)).is_ok());
    }

    fn remove_preserved(message: &str) {
        let _ = fs::remove_file(preserved_path(message));
    }
}
