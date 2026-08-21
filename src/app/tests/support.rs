use std::time::Instant;

use super::*;
pub(super) fn test_theme() -> Theme {
    Theme {
        background: Color::Red,
        foreground: Color::Green,
        foreground_muted: Color::Blue,
        hover_background: Color::LightCyan,
        selection_background: Color::LightYellow,
        status_bar_background: Color::Yellow,
        mode_background: Color::Magenta,
        mode_foreground: Color::Cyan,
    }
}

pub(super) fn section(name: &str) -> BranchSection {
    BranchSection {
        full_ref_name: format!("refs/heads/{name}"),
        display_name: name.to_owned(),
        worktree_path: format!("/worktrees/{name}").into(),
        is_current: name == "main",
        is_locked: false,
        is_stored_only: false,
    }
}

pub(super) fn app_with_sections(sections: Vec<BranchSection>) -> App {
    let store = TodoStore::open_in_memory().unwrap();
    let focus = sections
        .first()
        .map(|section| Focus::Branch(section.full_ref_name.clone()));
    App {
        exit: false,
        repository: RepositoryContext {
            head_label: "main".to_owned(),
            common_git_dir: Default::default(),
            sections,
        },
        store,
        persistence_available: true,
        todos: Vec::new(),
        focus,
        mode: Mode::Normal,
        todo_register: None,
        pending_operator: None,
        theme: test_theme(),
        system_theme: None,
        data_version: 0,
        dispatch: DispatchController::default(),
        error: None,
        repository_error: None,
        last_repository_refresh: Instant::now(),
        clipboard_request: None,
        external_edit_request: None,
        pointer_position: None,
        last_todo_click: None,
        frame_area: Rect::default(),
        viewport_start: 0,
        reveal_focus: true,
    }
}

pub(super) fn write_dispatch_config(
    worktree_path: &std::path::Path,
    definitions: &[(&str, &str)],
) -> DispatchConfigDigest {
    let contents = definitions
        .iter()
        .map(|(name, command)| {
            format!(
                "[dispatches.{name}]\ncommand = {}\n",
                toml::Value::String((*command).to_owned())
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(worktree_path.join(".refdo.toml"), contents).unwrap();
    load_repository_dispatch_config(worktree_path)
        .unwrap()
        .unwrap()
        .digest
}

pub(super) fn trust_dispatch_config(app: &App, digest: &DispatchConfigDigest) {
    app.store.trust_dispatch_config(digest).unwrap();
}

pub(super) fn app_with_dispatch(sections: Vec<BranchSection>, name: &str, command: &str) -> App {
    let mut app = app_with_sections(sections);
    app.dispatch = DispatchController::new(DispatchSettings::default());
    let worktrees = app
        .repository
        .sections
        .iter()
        .filter(|section| section.worktree_path.is_dir())
        .map(|section| section.worktree_path.clone())
        .collect::<Vec<_>>();
    for worktree in worktrees {
        let digest = write_dispatch_config(&worktree, &[(name, command)]);
        trust_dispatch_config(&app, &digest);
    }
    app
}

pub(super) fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

pub(super) fn modified_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

pub(super) fn type_text(app: &mut App, text: &str) {
    for character in text.chars() {
        app.handle_key_event(key(KeyCode::Char(character)));
    }
}

pub(super) fn row_text(terminal: &Terminal<TestBackend>, row: u16) -> String {
    let buffer = terminal.backend().buffer();
    (0..buffer.area.width)
        .map(|column| buffer[(column, row)].symbol())
        .collect()
}

pub(super) fn branch_titles(app: &App, branch_ref: &str) -> Vec<(String, bool)> {
    app.todos
        .iter()
        .filter(|todo| todo.branch_ref == branch_ref)
        .map(|todo| (todo.title.clone(), todo.completed))
        .collect()
}
