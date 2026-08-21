use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use super::support::*;
use super::*;

fn command_line(app: &App) -> &CommandLine {
    let Mode::Command(command) = &app.mode else {
        panic!("expected command mode");
    };
    command
}

fn run_command(app: &mut App, command: &str) {
    app.handle_key_event(key(KeyCode::Char(':')));
    type_text(app, command);
    app.handle_key_event(key(KeyCode::Enter));
}

#[test]
fn command_mode_captures_branch_and_renders_text_and_cursor_in_footer() {
    let mut app = app_with_sections(vec![section("main")]);
    app.handle_key_event(key(KeyCode::Char(':')));

    assert_eq!(
        command_line(&app).target_branch.as_deref(),
        Some("refs/heads/main")
    );
    assert!(command_line(&app).target_todos.is_empty());
    let backend = TestBackend::new(30, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(row_text(&terminal, 5).starts_with(':'));
    assert_eq!(
        terminal.backend_mut().get_cursor_position().unwrap(),
        Position::new(1, 5)
    );

    type_text(&mut app, "prune");
    assert_eq!(command_line(&app).text, "prune");
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(row_text(&terminal, 5).starts_with(":prune"));
    assert!(!row_text(&terminal, 5).contains("COMMAND"));
    assert_eq!(
        terminal.backend_mut().get_cursor_position().unwrap(),
        Position::new(6, 5)
    );
}

#[test]
fn command_editing_respects_unicode_boundaries_and_cancel_paths() {
    let mut app = app_with_sections(vec![section("main")]);
    app.handle_key_event(key(KeyCode::Char(':')));
    type_text(&mut app, "a👩‍🔬b");
    app.handle_key_event(key(KeyCode::Home));
    app.handle_key_event(key(KeyCode::Right));
    app.handle_key_event(key(KeyCode::Delete));
    assert_eq!(command_line(&app).text, "ab");
    assert_eq!(command_line(&app).cursor, 1);

    type_text(&mut app, "界");
    app.handle_key_event(key(KeyCode::Left));
    app.handle_key_event(key(KeyCode::Backspace));
    assert_eq!(command_line(&app).text, "界b");
    assert_eq!(command_line(&app).cursor, 0);
    app.handle_key_event(key(KeyCode::End));
    assert_eq!(command_line(&app).cursor, "界b".len());
    app.handle_key_event(key(KeyCode::Esc));
    assert!(matches!(&app.mode, Mode::Normal));
    assert_eq!(app.error, None);

    app.handle_key_event(key(KeyCode::Char(':')));
    type_text(&mut app, " \t ");
    app.handle_key_event(key(KeyCode::Enter));
    assert!(matches!(&app.mode, Mode::Normal));
    assert_eq!(app.error, None);
}

#[test]
fn unknown_command_returns_to_normal_and_is_visible_in_footer() {
    let mut app = app_with_sections(vec![section("main")]);
    run_command(&mut app, "  nope  ");

    assert!(matches!(&app.mode, Mode::Normal));
    assert_eq!(app.error.as_deref(), Some("Unknown command: nope"));
    let backend = TestBackend::new(40, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(row_text(&terminal, 5).contains("Unknown command: nope"));
}

#[test]
fn clean_is_rejected_after_prune_rename() {
    let mut app = app_with_sections(vec![section("main")]);

    run_command(&mut app, "clean");

    assert_eq!(app.error.as_deref(), Some("Unknown command: clean"));
}

#[test]
fn command_target_resolves_from_headers_and_todos_when_opened() {
    let mut app = app_with_sections(vec![section("main"), section("feature")]);
    let todo = app
        .store
        .insert_todo("refs/heads/feature", "feature work", None)
        .unwrap();
    app.reload();

    app.focus = Some(Focus::Branch("refs/heads/main".to_owned()));
    app.handle_key_event(key(KeyCode::Char(':')));
    assert_eq!(
        command_line(&app).target_branch.as_deref(),
        Some("refs/heads/main")
    );
    assert!(command_line(&app).target_todos.is_empty());
    app.handle_key_event(key(KeyCode::Esc));

    app.focus = Some(Focus::Todo(todo.id));
    app.handle_key_event(key(KeyCode::Char(':')));
    assert_eq!(
        command_line(&app).target_branch.as_deref(),
        Some("refs/heads/feature")
    );
    assert_eq!(command_line(&app).target_todos, vec![todo.id]);
    app.handle_key_event(key(KeyCode::Esc));
    app.todos.clear();
    app.focus = Some(Focus::Todo(todo.id));
    app.handle_key_event(key(KeyCode::Char(':')));
    assert!(command_line(&app).target_todos.is_empty());
}

#[test]
fn prune_deletes_only_completed_target_branch_todos_and_persists() {
    let mut app = app_with_sections(vec![section("main"), section("feature")]);
    let completed = app
        .store
        .insert_todo("refs/heads/main", "done here", None)
        .unwrap();
    let active = app
        .store
        .insert_todo("refs/heads/main", "keep here", Some(completed.id))
        .unwrap();
    let elsewhere = app
        .store
        .insert_todo("refs/heads/feature", "done elsewhere", None)
        .unwrap();
    app.store.toggle_todo(completed.id).unwrap();
    app.store.toggle_todo(elsewhere.id).unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(active.id));
    app.todo_register = Some(active.clone());

    run_command(&mut app, " prune ");

    assert_eq!(
        branch_titles(&app, "refs/heads/main"),
        vec![("keep here".to_owned(), false)]
    );
    assert_eq!(
        branch_titles(&app, "refs/heads/feature"),
        vec![("done elsewhere".to_owned(), true)]
    );
    assert_eq!(app.store.load_all().unwrap(), app.todos);
    assert_eq!(app.focus, Some(Focus::Todo(active.id)));
    assert_eq!(
        app.todo_register.as_ref().map(|todo| todo.id),
        Some(active.id)
    );
    assert_eq!(
        app.error.as_deref(),
        Some("prune: removed 1 completed items")
    );
}

#[test]
fn prune_reports_zero_matches_without_changing_state() {
    let mut app = app_with_sections(vec![section("main")]);
    let active = app
        .store
        .insert_todo("refs/heads/main", "still active", None)
        .unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(active.id));
    let before = app.todos.clone();

    run_command(&mut app, "prune");

    assert_eq!(app.todos, before);
    assert_eq!(app.store.load_all().unwrap(), before);
    assert_eq!(app.focus, Some(Focus::Todo(active.id)));
    assert_eq!(
        app.error.as_deref(),
        Some("prune: removed 0 completed items")
    );
}

#[test]
fn prune_without_focus_or_persistence_fails_without_deleting() {
    let mut app = app_with_sections(vec![section("main")]);
    let completed = app
        .store
        .insert_todo("refs/heads/main", "must remain", None)
        .unwrap();
    app.store.toggle_todo(completed.id).unwrap();
    app.reload();
    app.focus = None;
    run_command(&mut app, "prune");
    assert_eq!(app.error.as_deref(), Some("prune: no focused branch"));
    assert_eq!(app.store.load_all().unwrap().len(), 1);
    assert!(matches!(&app.mode, Mode::Normal));

    app.focus = Some(Focus::Branch("refs/heads/main".to_owned()));
    app.persistence_available = false;
    run_command(&mut app, "prune");
    assert_eq!(app.error.as_deref(), Some("prune: persistence unavailable"));
    assert_eq!(app.store.load_all().unwrap().len(), 1);
    assert!(matches!(&app.mode, Mode::Normal));
}

#[test]
fn prune_repairs_deleted_todo_focus_to_target_header_and_keeps_todo_register() {
    let mut app = app_with_sections(vec![section("main")]);
    let completed = app
        .store
        .insert_todo("refs/heads/main", "focused done", None)
        .unwrap();
    let registered = app
        .store
        .insert_todo("refs/heads/main", "register sentinel", Some(completed.id))
        .unwrap();
    app.store.toggle_todo(completed.id).unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(completed.id));
    app.todo_register = Some(registered.clone());

    run_command(&mut app, "prune");

    assert_eq!(app.focus, Some(Focus::Branch("refs/heads/main".to_owned())));
    assert_eq!(
        app.todo_register.as_ref().map(|todo| todo.id),
        Some(registered.id)
    );
    assert_eq!(
        branch_titles(&app, "refs/heads/main"),
        vec![("register sentinel".to_owned(), false)]
    );
}

#[test]
fn clear_requires_confirmation_and_enter_cancels_without_deleting() {
    let mut app = app_with_sections(vec![section("main")]);
    let completed = app
        .store
        .insert_todo("refs/heads/main", "completed", None)
        .unwrap();
    app.store
        .insert_todo("refs/heads/main", "incomplete", Some(completed.id))
        .unwrap();
    app.store.toggle_todo(completed.id).unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(completed.id));
    let before = app.todos.clone();

    run_command(&mut app, "clear");

    assert!(matches!(
        &app.mode,
        Mode::ConfirmClear(ClearConfirmation { target_branch, .. })
            if target_branch == "refs/heads/main"
    ));
    assert_eq!(app.todos, before);
    assert_eq!(app.store.load_all().unwrap(), before);
    let backend = TestBackend::new(60, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let footer = row_text(&terminal, 5);
    assert!(footer.contains("CONFIRM"));
    assert!(footer.contains("clear: remove all items from main? [y/N]"));

    app.handle_key_event(modified_key(KeyCode::Char('y'), KeyModifiers::CONTROL));
    assert!(matches!(&app.mode, Mode::ConfirmClear(_)));
    app.handle_key_event(key(KeyCode::Enter));

    assert!(matches!(&app.mode, Mode::Normal));
    assert_eq!(app.error, None);
    assert_eq!(app.todos, before);
    assert_eq!(app.store.load_all().unwrap(), before);
    assert_eq!(app.focus, Some(Focus::Todo(completed.id)));
}

#[test]
fn clear_confirmation_deletes_every_target_branch_todo_and_persists() {
    let mut app = app_with_sections(vec![section("main"), section("feature")]);
    let main_completed = app
        .store
        .insert_todo_with_completion("refs/heads/main", "main completed", true, None)
        .unwrap();
    app.store
        .insert_todo(
            "refs/heads/main",
            "main incomplete",
            Some(main_completed.id),
        )
        .unwrap();
    let feature = app
        .store
        .insert_todo_with_completion("refs/heads/feature", "feature completed", true, None)
        .unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(main_completed.id));
    app.todo_register = Some(main_completed.clone());

    run_command(&mut app, "clear");
    app.handle_key_event(key(KeyCode::Char('y')));

    assert!(branch_titles(&app, "refs/heads/main").is_empty());
    assert_eq!(
        branch_titles(&app, "refs/heads/feature"),
        vec![("feature completed".to_owned(), true)]
    );
    assert_eq!(app.store.load_all().unwrap(), app.todos);
    assert_eq!(app.focus, Some(Focus::Branch("refs/heads/main".to_owned())));
    assert_eq!(
        app.todo_register.as_ref().map(|todo| todo.id),
        Some(main_completed.id)
    );
    assert_eq!(app.error.as_deref(), Some("clear: removed 2 items"));
    assert!(matches!(&app.mode, Mode::Normal));
    assert!(app.todos.iter().any(|todo| todo.id == feature.id));
}

#[test]
fn clear_without_focus_or_persistence_never_prompts_or_deletes() {
    let mut app = app_with_sections(vec![section("main")]);
    app.store
        .insert_todo("refs/heads/main", "must remain", None)
        .unwrap();
    app.reload();
    app.focus = None;

    run_command(&mut app, "clear");

    assert_eq!(app.error.as_deref(), Some("clear: no focused branch"));
    assert_eq!(app.store.load_all().unwrap().len(), 1);
    assert!(matches!(&app.mode, Mode::Normal));

    app.focus = Some(Focus::Branch("refs/heads/main".to_owned()));
    app.persistence_available = false;
    run_command(&mut app, "clear");

    assert_eq!(app.error.as_deref(), Some("clear: persistence unavailable"));
    assert_eq!(app.store.load_all().unwrap().len(), 1);
    assert!(matches!(&app.mode, Mode::Normal));
}
#[test]
fn sort_orders_only_target_branch_persists_and_preserves_todo_focus_and_register() {
    let mut app = app_with_sections(vec![section("main"), section("feature")]);
    app.store
        .insert_todo("refs/heads/main", "first active", None)
        .unwrap();
    let first_completed = app
        .store
        .insert_todo("refs/heads/main", "first completed", None)
        .unwrap();
    let second_active = app
        .store
        .insert_todo("refs/heads/main", "second active", None)
        .unwrap();
    let second_completed = app
        .store
        .insert_todo("refs/heads/main", "second completed", None)
        .unwrap();
    let feature_first = app
        .store
        .insert_todo("refs/heads/feature", "feature first", None)
        .unwrap();
    app.store
        .insert_todo("refs/heads/feature", "feature second", None)
        .unwrap();
    app.store.toggle_todo(first_completed.id).unwrap();
    app.store.toggle_todo(second_completed.id).unwrap();
    app.reload();
    let feature_before = branch_titles(&app, "refs/heads/feature");
    app.focus = Some(Focus::Todo(second_active.id));
    app.todo_register = Some(feature_first.clone());

    run_command(&mut app, " sort ");

    assert_eq!(
        branch_titles(&app, "refs/heads/main"),
        vec![
            ("first active".to_owned(), false),
            ("second active".to_owned(), false),
            ("first completed".to_owned(), true),
            ("second completed".to_owned(), true),
        ]
    );
    assert_eq!(branch_titles(&app, "refs/heads/feature"), feature_before);
    assert_eq!(app.store.load_all().unwrap(), app.todos);
    assert_eq!(app.focus, Some(Focus::Todo(second_active.id)));
    assert_eq!(
        app.todo_register.as_ref().map(|todo| todo.id),
        Some(feature_first.id)
    );
    assert_eq!(app.error.as_deref(), Some("sort: sorted 4 items"));
    assert!(matches!(&app.mode, Mode::Normal));
}

#[test]
fn sort_uses_branch_captured_when_command_mode_opened_and_preserves_header_focus() {
    let mut app = app_with_sections(vec![section("main"), section("feature")]);
    app.store
        .insert_todo("refs/heads/main", "main first", None)
        .unwrap();
    app.store
        .insert_todo("refs/heads/main", "main second", None)
        .unwrap();
    app.store
        .insert_todo("refs/heads/feature", "feature first", None)
        .unwrap();
    app.store
        .insert_todo("refs/heads/feature", "feature second", None)
        .unwrap();
    app.reload();
    let feature_before = branch_titles(&app, "refs/heads/feature");
    app.focus = Some(Focus::Branch("refs/heads/main".to_owned()));
    app.handle_key_event(key(KeyCode::Char(':')));
    type_text(&mut app, "sort");
    app.focus = Some(Focus::Branch("refs/heads/feature".to_owned()));

    app.handle_key_event(key(KeyCode::Enter));

    assert_eq!(
        branch_titles(&app, "refs/heads/main"),
        vec![
            ("main first".to_owned(), false),
            ("main second".to_owned(), false),
        ]
    );
    assert_eq!(branch_titles(&app, "refs/heads/feature"), feature_before);
    assert_eq!(
        app.focus,
        Some(Focus::Branch("refs/heads/feature".to_owned()))
    );
    assert_eq!(app.error.as_deref(), Some("sort: sorted 2 items"));
    assert!(matches!(&app.mode, Mode::Normal));
}

#[test]
fn sort_empty_target_reports_zero_and_preserves_header_focus() {
    let mut app = app_with_sections(vec![section("main")]);
    let focus = Some(Focus::Branch("refs/heads/main".to_owned()));
    app.focus = focus.clone();

    run_command(&mut app, "sort");

    assert!(app.todos.is_empty());
    assert!(app.store.load_all().unwrap().is_empty());
    assert_eq!(app.focus, focus);
    assert_eq!(app.error.as_deref(), Some("sort: sorted 0 items"));
    assert!(matches!(&app.mode, Mode::Normal));
}

#[test]
fn sort_without_focus_or_persistence_fails_without_reordering() {
    let mut app = app_with_sections(vec![section("main")]);
    app.store
        .insert_todo("refs/heads/main", "first", None)
        .unwrap();
    app.store
        .insert_todo("refs/heads/main", "second", None)
        .unwrap();
    app.reload();
    let before = app.todos.clone();
    app.focus = None;

    run_command(&mut app, "sort");

    assert_eq!(app.error.as_deref(), Some("sort: no focused branch"));
    assert_eq!(app.store.load_all().unwrap(), before);
    assert!(matches!(&app.mode, Mode::Normal));

    app.focus = Some(Focus::Branch("refs/heads/main".to_owned()));
    app.persistence_available = false;
    run_command(&mut app, "sort");

    assert_eq!(app.error.as_deref(), Some("sort: persistence unavailable"));
    assert_eq!(app.store.load_all().unwrap(), before);
    assert!(matches!(&app.mode, Mode::Normal));
}

#[test]
fn group_stably_partitions_only_captured_branch_and_preserves_app_state() {
    let mut app = app_with_sections(vec![section("main"), section("feature")]);
    let first_active = app
        .store
        .insert_todo("refs/heads/main", "first active", None)
        .unwrap();
    let first_completed = app
        .store
        .insert_todo("refs/heads/main", "first completed", Some(first_active.id))
        .unwrap();
    let second_active = app
        .store
        .insert_todo("refs/heads/main", "second active", Some(first_completed.id))
        .unwrap();
    let second_completed = app
        .store
        .insert_todo(
            "refs/heads/main",
            "second completed",
            Some(second_active.id),
        )
        .unwrap();
    let feature_completed = app
        .store
        .insert_todo("refs/heads/feature", "feature completed", None)
        .unwrap();
    let feature_active = app
        .store
        .insert_todo(
            "refs/heads/feature",
            "feature active",
            Some(feature_completed.id),
        )
        .unwrap();
    app.store.toggle_todo(first_completed.id).unwrap();
    app.store.toggle_todo(second_completed.id).unwrap();
    app.store.toggle_todo(feature_completed.id).unwrap();
    app.reload();
    assert_eq!(
        branch_titles(&app, "refs/heads/main"),
        vec![
            ("first active".to_owned(), false),
            ("first completed".to_owned(), true),
            ("second active".to_owned(), false),
            ("second completed".to_owned(), true),
        ]
    );
    let feature_before = branch_titles(&app, "refs/heads/feature");
    app.focus = Some(Focus::Todo(first_active.id));
    app.todo_register = Some(feature_completed.clone());
    app.handle_key_event(key(KeyCode::Char(':')));
    type_text(&mut app, " group ");
    app.focus = Some(Focus::Todo(feature_active.id));

    app.handle_key_event(key(KeyCode::Enter));

    let expected_main = vec![
        ("first active".to_owned(), false),
        ("second active".to_owned(), false),
        ("first completed".to_owned(), true),
        ("second completed".to_owned(), true),
    ];
    assert_eq!(branch_titles(&app, "refs/heads/main"), expected_main);
    assert_eq!(branch_titles(&app, "refs/heads/feature"), feature_before);
    assert_eq!(app.store.load_all().unwrap(), app.todos);
    assert_eq!(app.focus, Some(Focus::Todo(feature_active.id)));
    assert_eq!(
        app.todo_register.as_ref().map(|todo| todo.id),
        Some(feature_completed.id)
    );
    assert_eq!(app.error.as_deref(), Some("group: grouped 4 items"));
    assert!(matches!(&app.mode, Mode::Normal));

    app.reload();
    assert_eq!(branch_titles(&app, "refs/heads/main"), expected_main);
    assert_eq!(branch_titles(&app, "refs/heads/feature"), feature_before);
}

#[test]
fn group_without_focus_or_persistence_fails_without_reordering() {
    let mut app = app_with_sections(vec![section("main")]);
    app.store
        .insert_todo("refs/heads/main", "first", None)
        .unwrap();
    app.store
        .insert_todo("refs/heads/main", "second", None)
        .unwrap();
    app.reload();
    let before = app.todos.clone();
    app.focus = None;

    run_command(&mut app, "group");

    assert_eq!(app.error.as_deref(), Some("group: no focused branch"));
    assert_eq!(app.store.load_all().unwrap(), before);
    assert!(matches!(&app.mode, Mode::Normal));

    app.focus = Some(Focus::Branch("refs/heads/main".to_owned()));
    app.persistence_available = false;
    run_command(&mut app, "group");

    assert_eq!(app.error.as_deref(), Some("group: persistence unavailable"));
    assert_eq!(app.store.load_all().unwrap(), before);
    assert!(matches!(&app.mode, Mode::Normal));
}

static TEMP_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let unique = TEMP_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "refdo-command-dispatch-{}-{nanos}-{unique}",
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

fn insert_and_focus_todo(app: &mut App, branch_ref: &str, title: &str) -> TodoId {
    let todo = app.store.insert_todo(branch_ref, title, None).unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(todo.id));
    todo.id
}

fn wait_for_dispatch_footer(app: &mut App, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        app.refresh_dispatch();
        if app.error.as_deref() == Some(expected) {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(app.error.as_deref(), Some(expected));
}

#[test]
fn dispatch_uses_todo_captured_when_command_line_opened() {
    let directory = TestDirectory::new();
    let mut main = section("main");
    main.worktree_path = directory.path().to_owned();
    let mut app = app_with_dispatch(
        vec![main],
        "record",
        "printf '%s' {{CONTENT}} > dispatch-content",
    );
    let selected = insert_and_focus_todo(&mut app, "refs/heads/main", "captured todo");
    let other = app
        .store
        .insert_todo("refs/heads/main", "later focus", None)
        .unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(selected));

    app.handle_key_event(key(KeyCode::Char(':')));
    assert_eq!(command_line(&app).target_todos, vec![selected]);
    app.focus = Some(Focus::Todo(other.id));
    type_text(&mut app, "dispatch record");
    app.handle_key_event(key(KeyCode::Enter));

    assert_eq!(app.error.as_deref(), Some("dispatch: running 'record'"));
    wait_for_dispatch_footer(&mut app, "dispatch: 'record' completed");
    assert_eq!(
        fs::read_to_string(directory.path().join("dispatch-content")).unwrap(),
        "captured todo"
    );
}

#[test]
fn dispatch_requires_exactly_one_name_and_rejects_unknown_names() {
    let directory = TestDirectory::new();
    let mut main = section("main");
    main.worktree_path = directory.path().to_owned();
    let mut app = app_with_dispatch(vec![main], "known", "true");
    insert_and_focus_todo(&mut app, "refs/heads/main", "selected");

    for malformed in ["dispatch", "dispatch known extra"] {
        run_command(&mut app, malformed);
        assert_eq!(
            app.error.as_deref(),
            Some("dispatch: expected :dispatch <name>")
        );
    }

    run_command(&mut app, "dispatch missing");
    assert_eq!(
        app.error.as_deref(),
        Some("dispatch: unknown dispatch 'missing'")
    );
}

#[test]
fn dispatch_requires_a_currently_existing_selected_todo() {
    let directory = TestDirectory::new();
    let mut main = section("main");
    main.worktree_path = directory.path().to_owned();
    let mut app = app_with_dispatch(vec![main], "known", "true");

    run_command(&mut app, "dispatch known");
    assert_eq!(app.error.as_deref(), Some("dispatch: no todo selected"));

    app.focus = None;
    run_command(&mut app, "dispatch known");
    assert_eq!(app.error.as_deref(), Some("dispatch: no todo selected"));

    let selected = insert_and_focus_todo(&mut app, "refs/heads/main", "selected");
    app.handle_key_event(key(KeyCode::Char(':')));
    app.todos.retain(|todo| todo.id != selected);
    type_text(&mut app, "dispatch known");
    app.handle_key_event(key(KeyCode::Enter));
    assert_eq!(
        app.error.as_deref(),
        Some("dispatch: selected todo no longer exists")
    );
}

#[test]
fn dispatch_rejects_stored_only_missing_and_empty_worktrees() {
    let directory = TestDirectory::new();

    let mut stored = section("main");
    stored.worktree_path = directory.path().to_owned();
    stored.is_stored_only = true;
    let mut stored_app = app_with_dispatch(vec![stored], "known", "true");
    insert_and_focus_todo(&mut stored_app, "refs/heads/main", "stored");
    run_command(&mut stored_app, "dispatch known");
    assert_eq!(
        stored_app.error.as_deref(),
        Some("dispatch: selected todo has no worktree")
    );

    let mut missing_app = app_with_dispatch(vec![section("main")], "known", "true");
    insert_and_focus_todo(&mut missing_app, "refs/heads/main", "missing");
    missing_app.repository.sections.clear();
    run_command(&mut missing_app, "dispatch known");
    assert_eq!(
        missing_app.error.as_deref(),
        Some("dispatch: selected todo has no worktree")
    );

    let mut empty = section("main");
    empty.worktree_path = PathBuf::new();
    let mut empty_app = app_with_dispatch(vec![empty], "known", "true");
    insert_and_focus_todo(&mut empty_app, "refs/heads/main", "empty");
    run_command(&mut empty_app, "dispatch known");
    assert_eq!(
        empty_app.error.as_deref(),
        Some("dispatch: selected todo has no worktree")
    );
}

#[test]
fn dispatch_reports_running_immediately_and_async_success() {
    let directory = TestDirectory::new();
    let mut main = section("main");
    main.worktree_path = directory.path().to_owned();
    let mut app = app_with_dispatch(vec![main], "slow", "sleep 0.1");
    insert_and_focus_todo(&mut app, "refs/heads/main", "selected");

    run_command(&mut app, "dispatch slow");

    assert_eq!(app.error.as_deref(), Some("dispatch: running 'slow'"));
    wait_for_dispatch_footer(&mut app, "dispatch: 'slow' completed");
}

#[test]
fn dispatch_surfaces_async_failure_in_footer() {
    let directory = TestDirectory::new();
    let mut main = section("main");
    main.worktree_path = directory.path().to_owned();
    let mut app = app_with_dispatch(vec![main], "fail", "printf 'fixture failed\n' >&2; exit 7");
    insert_and_focus_todo(&mut app, "refs/heads/main", "selected");

    run_command(&mut app, "dispatch fail");

    assert_eq!(app.error.as_deref(), Some("dispatch: running 'fail'"));
    wait_for_dispatch_footer(&mut app, "dispatch: 'fail' failed: fixture failed");
}

#[test]
fn dispatch_loads_selected_worktree_config_and_reports_missing_or_malformed_files() {
    let directory = TestDirectory::new();
    let mut main = section("main");
    main.worktree_path = directory.path().to_owned();
    let mut app = app_with_sections(vec![main]);
    insert_and_focus_todo(&mut app, "refs/heads/main", "selected");

    run_command(&mut app, "dispatch known");
    assert_eq!(
        app.error.as_deref(),
        Some("dispatch: selected todo's worktree has no .refdo.toml")
    );

    fs::write(directory.path().join(".refdo.toml"), "[dispatches.known").unwrap();
    run_command(&mut app, "dispatch known");
    assert!(
        app.error
            .as_deref()
            .is_some_and(|error| error.starts_with("dispatch: invalid configuration in "))
    );
}

#[test]
fn untrusted_dispatch_never_spawns_and_persistence_is_required_before_config_read() {
    let directory = TestDirectory::new();
    let marker = directory.path().join("spawned");
    let mut main = section("main");
    main.worktree_path = directory.path().to_owned();
    let mut app = app_with_sections(vec![main]);
    insert_and_focus_todo(&mut app, "refs/heads/main", "selected");
    write_dispatch_config(
        directory.path(),
        &[("known", &format!("touch {}", marker.display()))],
    );

    run_command(&mut app, "dispatch known");
    assert_eq!(
        app.error.as_deref(),
        Some("dispatch: repository configuration is untrusted; run :dispatch-trust")
    );
    assert!(!marker.exists());

    fs::remove_file(directory.path().join(".refdo.toml")).unwrap();
    app.persistence_available = false;
    for command in ["dispatch known", "dispatch-trust"] {
        run_command(&mut app, command);
        assert_eq!(
            app.error.as_deref(),
            Some("dispatch: repository trust unavailable")
        );
    }
}

#[test]
fn dispatch_trust_validates_arity_selection_and_nonempty_definition_map() {
    let directory = TestDirectory::new();
    let mut main = section("main");
    main.worktree_path = directory.path().to_owned();
    let mut app = app_with_sections(vec![main]);

    run_command(&mut app, "dispatch-trust extra");
    assert_eq!(
        app.error.as_deref(),
        Some("dispatch-trust: expected :dispatch-trust")
    );
    run_command(&mut app, "dispatch-trust");
    assert_eq!(app.error.as_deref(), Some("dispatch: no todo selected"));

    insert_and_focus_todo(&mut app, "refs/heads/main", "selected");
    fs::write(directory.path().join(".refdo.toml"), "").unwrap();
    run_command(&mut app, "dispatch-trust");
    assert_eq!(
        app.error.as_deref(),
        Some("dispatch: .refdo.toml defines no dispatches")
    );
}

#[test]
fn dispatch_trust_confirmation_is_mode_owned_and_cancel_keys_do_not_trust() {
    let directory = TestDirectory::new();
    let mut main = section("main");
    main.worktree_path = directory.path().to_owned();
    let mut app = app_with_sections(vec![main]);
    insert_and_focus_todo(&mut app, "refs/heads/main", "selected");
    let digest = write_dispatch_config(directory.path(), &[("known", "true")]);

    for cancel in [
        KeyCode::Char('n'),
        KeyCode::Char('N'),
        KeyCode::Enter,
        KeyCode::Esc,
    ] {
        run_command(&mut app, "dispatch-trust");
        let Mode::ConfirmDispatchTrust(confirmation) = &app.mode else {
            panic!("expected dispatch trust confirmation");
        };
        assert_eq!(
            confirmation.prompt,
            "dispatch: trust .refdo.toml from main? [y/N]"
        );
        app.error = Some("deferred notification".to_owned());
        let backend = TestBackend::new(70, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(row_text(&terminal, 5).contains("dispatch: trust .refdo.toml from main? [y/N]"));
        app.handle_key_event(key(cancel));
        assert!(matches!(app.mode, Mode::Normal));
        assert!(!app.store.is_dispatch_config_trusted(&digest).unwrap());
    }

    run_command(&mut app, "dispatch-trust");
    app.handle_key_event(modified_key(KeyCode::Char('y'), KeyModifiers::CONTROL));
    assert!(matches!(app.mode, Mode::ConfirmDispatchTrust(_)));
}

#[test]
fn dispatch_trust_persists_after_revalidation_and_reports_already_trusted() {
    let directory = TestDirectory::new();
    let mut main = section("main");
    main.worktree_path = directory.path().to_owned();
    let mut app = app_with_sections(vec![main]);
    insert_and_focus_todo(&mut app, "refs/heads/main", "selected");
    let digest = write_dispatch_config(directory.path(), &[("known", "true")]);

    run_command(&mut app, "dispatch-trust");
    app.handle_key_event(key(KeyCode::Char('y')));

    assert!(matches!(app.mode, Mode::Normal));
    assert_eq!(
        app.error.as_deref(),
        Some("dispatch: trusted .refdo.toml for main")
    );
    assert!(app.store.is_dispatch_config_trusted(&digest).unwrap());

    run_command(&mut app, "dispatch-trust");
    assert_eq!(
        app.error.as_deref(),
        Some("dispatch: .refdo.toml is already trusted for main")
    );
}

#[test]
fn dispatch_trust_revalidation_rejects_changed_config_and_trusts_neither_digest() {
    let directory = TestDirectory::new();
    let mut main = section("main");
    main.worktree_path = directory.path().to_owned();
    let mut app = app_with_sections(vec![main]);
    insert_and_focus_todo(&mut app, "refs/heads/main", "selected");
    let captured = write_dispatch_config(directory.path(), &[("known", "true")]);

    run_command(&mut app, "dispatch-trust");
    let replacement = write_dispatch_config(directory.path(), &[("known", "printf changed")]);
    app.handle_key_event(key(KeyCode::Char('y')));

    assert_eq!(
        app.error.as_deref(),
        Some("dispatch: .refdo.toml changed; run :dispatch-trust again")
    );
    assert!(!app.store.is_dispatch_config_trusted(&captured).unwrap());
    assert!(!app.store.is_dispatch_config_trusted(&replacement).unwrap());
}
#[test]
fn dispatch_trust_revalidation_rejects_missing_invalid_and_empty_configs() {
    let directory = TestDirectory::new();
    let mut main = section("main");
    main.worktree_path = directory.path().to_owned();
    let mut app = app_with_sections(vec![main]);
    insert_and_focus_todo(&mut app, "refs/heads/main", "selected");

    for replacement in [None, Some("[dispatches.known"), Some("")] {
        let captured = write_dispatch_config(directory.path(), &[("known", "true")]);
        run_command(&mut app, "dispatch-trust");
        match replacement {
            Some(contents) => {
                fs::write(directory.path().join(".refdo.toml"), contents).unwrap();
            }
            None => fs::remove_file(directory.path().join(".refdo.toml")).unwrap(),
        }
        app.handle_key_event(key(KeyCode::Char('y')));

        let error = app.error.as_deref().unwrap();
        match replacement {
            None => assert_eq!(
                error,
                "dispatch: selected todo's worktree has no .refdo.toml"
            ),
            Some("") => assert_eq!(error, "dispatch: .refdo.toml defines no dispatches"),
            Some(_) => assert!(error.starts_with("dispatch: invalid configuration in ")),
        }
        assert!(!app.store.is_dispatch_config_trusted(&captured).unwrap());
    }
}

#[test]
fn every_config_byte_change_requires_new_trust_before_dispatch() {
    let directory = TestDirectory::new();
    let marker = directory.path().join("ran");
    let mut main = section("main");
    main.worktree_path = directory.path().to_owned();
    let mut app = app_with_dispatch(vec![main], "known", &format!("touch {}", marker.display()));
    insert_and_focus_todo(&mut app, "refs/heads/main", "selected");
    let mut contents = fs::read_to_string(directory.path().join(".refdo.toml")).unwrap();
    contents.push_str("\n# changed bytes\n");
    fs::write(directory.path().join(".refdo.toml"), contents).unwrap();

    run_command(&mut app, "dispatch known");

    assert_eq!(
        app.error.as_deref(),
        Some("dispatch: repository configuration is untrusted; run :dispatch-trust")
    );
    assert!(!marker.exists());

    run_command(&mut app, "dispatch-trust");
    app.handle_key_event(key(KeyCode::Char('y')));
    run_command(&mut app, "dispatch known");
    wait_for_dispatch_footer(&mut app, "dispatch: 'known' completed");
    assert!(marker.exists());
}

#[test]
fn dispatch_completion_does_not_hide_or_disarm_trust_confirmation() {
    let directory = TestDirectory::new();
    let mut main = section("main");
    main.worktree_path = directory.path().to_owned();
    let mut app = app_with_dispatch(vec![main], "slow", "sleep 0.1");
    insert_and_focus_todo(&mut app, "refs/heads/main", "selected");
    run_command(&mut app, "dispatch slow");

    write_dispatch_config(directory.path(), &[("next", "true")]);
    run_command(&mut app, "dispatch-trust");
    assert!(matches!(app.mode, Mode::ConfirmDispatchTrust(_)));
    wait_for_dispatch_footer(&mut app, "dispatch: 'slow' completed");

    let backend = TestBackend::new(70, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(row_text(&terminal, 5).contains("dispatch: trust .refdo.toml from main? [y/N]"));
    app.handle_key_event(key(KeyCode::Char('n')));
    assert!(matches!(app.mode, Mode::Normal));
}

#[test]
fn selected_worktree_owns_its_dispatch_definition() {
    let main_directory = TestDirectory::new();
    let feature_directory = TestDirectory::new();
    let mut main = section("main");
    main.worktree_path = main_directory.path().to_owned();
    let mut feature = section("feature");
    feature.worktree_path = feature_directory.path().to_owned();
    let mut app = app_with_sections(vec![main, feature]);
    let main_digest = write_dispatch_config(
        main_directory.path(),
        &[("record", "printf main > dispatch-owner")],
    );
    let feature_digest = write_dispatch_config(
        feature_directory.path(),
        &[("record", "printf feature > dispatch-owner")],
    );
    trust_dispatch_config(&app, &main_digest);
    trust_dispatch_config(&app, &feature_digest);
    let main_todo = insert_and_focus_todo(&mut app, "refs/heads/main", "main todo");
    let feature_todo = app
        .store
        .insert_todo("refs/heads/feature", "feature todo", None)
        .unwrap();
    app.reload();

    for (todo, expected, directory) in [
        (main_todo, "main", main_directory.path()),
        (feature_todo.id, "feature", feature_directory.path()),
    ] {
        app.focus = Some(Focus::Todo(todo));
        run_command(&mut app, "dispatch record");
        wait_for_dispatch_footer(&mut app, "dispatch: 'record' completed");
        assert_eq!(
            fs::read_to_string(directory.join("dispatch-owner")).unwrap(),
            expected
        );
    }
}

#[test]
fn command_line_open_from_select_mode_captures_selected_todos_in_display_order_and_exits_select_mode()
 {
    let mut app = app_with_sections(vec![section("main")]);
    let t1 = app
        .store
        .insert_todo("refs/heads/main", "first", None)
        .unwrap();
    let t2 = app
        .store
        .insert_todo("refs/heads/main", "second", Some(t1.id))
        .unwrap();
    let t3 = app
        .store
        .insert_todo("refs/heads/main", "third", Some(t2.id))
        .unwrap();
    app.reload();

    // Enter select mode on t1, navigate to t3 and select it
    app.focus = Some(Focus::Todo(t1.id));
    app.handle_key_event(key(KeyCode::Char('v')));
    app.handle_key_event(key(KeyCode::Char('j')));
    app.handle_key_event(key(KeyCode::Char('j')));
    app.handle_key_event(key(KeyCode::Char(' ')));

    // Open command line with ':'
    app.handle_key_event(key(KeyCode::Char(':')));

    assert!(matches!(&app.mode, Mode::Command(_)));
    assert_eq!(
        command_line(&app).target_branch.as_deref(),
        Some("refs/heads/main")
    );
    assert_eq!(command_line(&app).target_todos, vec![t1.id, t3.id]);
}

#[test]
fn dispatch_multiple_selected_todos_formats_as_markdown_list_in_displayed_order() {
    let directory = TestDirectory::new();
    let mut main = section("main");
    main.worktree_path = directory.path().to_owned();
    let mut app = app_with_dispatch(
        vec![main],
        "record",
        "printf '%s' {{CONTENT}} > dispatch-content",
    );
    let t1 = app
        .store
        .insert_todo("refs/heads/main", "first item\ncontinuation", None)
        .unwrap();
    let t2 = app
        .store
        .insert_todo("refs/heads/main", "second item", Some(t1.id))
        .unwrap();
    let t3 = app
        .store
        .insert_todo("refs/heads/main", "third item", Some(t2.id))
        .unwrap();
    app.reload();

    // Enter select mode on t3 first, then toggle t1 (selecting {t1, t3} non-contiguously)
    app.focus = Some(Focus::Todo(t3.id));
    app.handle_key_event(key(KeyCode::Char('v')));
    app.handle_key_event(key(KeyCode::Char('k')));
    app.handle_key_event(key(KeyCode::Char('k')));
    app.handle_key_event(key(KeyCode::Char(' ')));

    run_command(&mut app, "dispatch record");
    assert_eq!(app.error.as_deref(), Some("dispatch: running 'record'"));
    wait_for_dispatch_footer(&mut app, "dispatch: 'record' completed");

    assert_eq!(
        fs::read_to_string(directory.path().join("dispatch-content")).unwrap(),
        "- first item\n  continuation\n- third item"
    );
}

#[test]
fn dispatch_single_selected_todo_preserves_plain_title_byte_for_byte() {
    let directory = TestDirectory::new();
    let mut main = section("main");
    main.worktree_path = directory.path().to_owned();
    let mut app = app_with_dispatch(
        vec![main],
        "record",
        "printf '%s' {{CONTENT}} > dispatch-content",
    );
    let t1 = app
        .store
        .insert_todo("refs/heads/main", "single plain title", None)
        .unwrap();
    app.reload();

    app.focus = Some(Focus::Todo(t1.id));
    app.handle_key_event(key(KeyCode::Char('v')));

    run_command(&mut app, "dispatch record");
    assert_eq!(app.error.as_deref(), Some("dispatch: running 'record'"));
    wait_for_dispatch_footer(&mut app, "dispatch: 'record' completed");

    assert_eq!(
        fs::read_to_string(directory.path().join("dispatch-content")).unwrap(),
        "single plain title"
    );
}

#[test]
fn dispatch_empty_selection_fails_with_no_todo_selected() {
    let directory = TestDirectory::new();
    let mut main = section("main");
    main.worktree_path = directory.path().to_owned();
    let mut app = app_with_dispatch(vec![main], "record", "true");
    let t1 = app
        .store
        .insert_todo("refs/heads/main", "only item", None)
        .unwrap();
    app.reload();

    app.focus = Some(Focus::Todo(t1.id));
    app.handle_key_event(key(KeyCode::Char('v')));
    // Toggle off t1 -> empty selection
    app.handle_key_event(key(KeyCode::Char(' ')));

    run_command(&mut app, "dispatch record");
    assert_eq!(app.error.as_deref(), Some("dispatch: no todo selected"));
}

#[test]
fn dispatch_fails_atomically_if_any_captured_selected_todo_disappears() {
    let directory = TestDirectory::new();
    let mut main = section("main");
    main.worktree_path = directory.path().to_owned();
    let mut app = app_with_dispatch(
        vec![main],
        "record",
        "printf '%s' {{CONTENT}} > dispatch-content",
    );
    let t1 = app
        .store
        .insert_todo("refs/heads/main", "first item", None)
        .unwrap();
    let t2 = app
        .store
        .insert_todo("refs/heads/main", "second item", Some(t1.id))
        .unwrap();
    app.reload();

    // Select both t1 and t2
    app.focus = Some(Focus::Todo(t1.id));
    app.handle_key_event(key(KeyCode::Char('v')));
    app.handle_key_event(key(KeyCode::Char('j')));
    app.handle_key_event(key(KeyCode::Char(' ')));

    // Open command line (captures t1 and t2)
    app.handle_key_event(key(KeyCode::Char(':')));
    assert_eq!(command_line(&app).target_todos, vec![t1.id, t2.id]);

    // Delete t2 from todos
    app.todos.retain(|todo| todo.id != t2.id);

    type_text(&mut app, "dispatch record");
    app.handle_key_event(key(KeyCode::Enter));

    assert_eq!(
        app.error.as_deref(),
        Some("dispatch: selected todo no longer exists")
    );
    assert!(!directory.path().join("dispatch-content").exists());
}

#[test]
fn dispatch_trust_with_multiple_selected_todos_uses_common_section() {
    let directory = TestDirectory::new();
    let mut main = section("main");
    main.worktree_path = directory.path().to_owned();
    let mut app = app_with_sections(vec![main]);
    let digest = write_dispatch_config(
        directory.path(),
        &[("record", "printf test > dispatch-content")],
    );
    let t1 = app
        .store
        .insert_todo("refs/heads/main", "item 1", None)
        .unwrap();
    let _t2 = app
        .store
        .insert_todo("refs/heads/main", "item 2", Some(t1.id))
        .unwrap();
    app.reload();

    app.focus = Some(Focus::Todo(t1.id));
    app.handle_key_event(key(KeyCode::Char('v')));
    app.handle_key_event(key(KeyCode::Char('j')));
    app.handle_key_event(key(KeyCode::Char(' ')));

    run_command(&mut app, "dispatch-trust");
    assert!(matches!(
        &app.mode,
        Mode::ConfirmDispatchTrust(confirmation)
            if confirmation.digest == digest
                && confirmation.display_name == "main"
                && confirmation.prompt == "dispatch: trust .refdo.toml from main? [y/N]"
    ));
    app.handle_key_event(key(KeyCode::Char('y')));
    assert_eq!(
        app.error.as_deref(),
        Some("dispatch: trusted .refdo.toml for main")
    );
}
