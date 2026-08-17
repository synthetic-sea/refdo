use super::super::PendingOperator;
use super::support::*;
use super::*;

fn editor(app: &App) -> &Editor {
    let Mode::Insert(editor) = &app.mode else {
        panic!("expected insert mode");
    };
    editor
}

#[test]
fn yy_queues_exact_unicode_title_and_replaces_todo_register_without_mutating_todos() {
    let mut app = app_with_sections(vec![section("main")]);
    let previous = app
        .store
        .insert_todo("refs/heads/main", "previous register", None)
        .unwrap();
    let todo = app
        .store
        .insert_todo("refs/heads/main", "Ship 🚀 café", Some(previous.id))
        .unwrap();
    app.store.toggle_todo(todo.id).unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(todo.id));
    app.todo_register = Some(previous.clone());
    let todos_before = app.todos.clone();
    let stored_before = app.store.load_all().unwrap();
    let focus_before = app.focus.clone();

    app.handle_key_event(key(KeyCode::Char('y')));

    assert_eq!(app.pending_operator, Some(PendingOperator::Yank));
    assert!(app.clipboard_request.is_none());
    assert_eq!(app.todo_register, Some(previous));
    assert_eq!(app.todos, todos_before);
    assert_eq!(app.store.load_all().unwrap(), stored_before);
    assert_eq!(app.focus, focus_before);

    app.handle_key_event(key(KeyCode::Char('y')));

    assert_eq!(app.pending_operator, None);
    assert_eq!(app.clipboard_request.as_deref(), Some("Ship 🚀 café"));
    assert_eq!(
        app.todo_register.as_ref().map(|registered| (
            registered.id,
            registered.title.as_str(),
            registered.completed
        )),
        Some((todo.id, "Ship 🚀 café", true))
    );
    assert_eq!(app.todos, todos_before);
    assert_eq!(app.store.load_all().unwrap(), stored_before);
    assert_eq!(app.focus, focus_before);
}

#[test]
fn yanked_todo_can_be_pasted_without_removing_the_original() {
    let mut app = app_with_sections(vec![section("main")]);
    let original = app
        .store
        .insert_todo("refs/heads/main", "duplicate me", None)
        .unwrap();
    app.store.toggle_todo(original.id).unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(original.id));

    app.handle_key_event(key(KeyCode::Char('y')));
    app.handle_key_event(key(KeyCode::Char('y')));
    app.handle_key_event(key(KeyCode::Char('p')));

    assert_eq!(
        branch_titles(&app, "refs/heads/main"),
        vec![
            ("duplicate me".to_owned(), true),
            ("duplicate me".to_owned(), true)
        ]
    );
    assert!(app.todos.iter().any(|todo| todo.id == original.id));
    assert_eq!(
        app.todo_register.as_ref().map(|registered| registered.id),
        Some(original.id)
    );
    assert_eq!(app.store.load_all().unwrap(), app.todos);
}

#[test]
fn normal_key_between_yanks_cancels_yank_and_performs_its_action() {
    let mut app = app_with_sections(vec![section("main")]);
    let first = app
        .store
        .insert_todo("refs/heads/main", "first", None)
        .unwrap();
    let second = app
        .store
        .insert_todo("refs/heads/main", "second", Some(first.id))
        .unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(first.id));

    app.handle_key_event(key(KeyCode::Char('y')));
    app.handle_key_event(key(KeyCode::Char('j')));

    assert_eq!(app.pending_operator, None);
    assert_eq!(app.focus, Some(Focus::Todo(second.id)));
    assert!(app.clipboard_request.is_none());

    app.handle_key_event(key(KeyCode::Char('y')));

    assert_eq!(app.pending_operator, Some(PendingOperator::Yank));
    assert!(app.clipboard_request.is_none());
}

#[test]
fn yy_without_todo_focus_queues_nothing() {
    let mut app = app_with_sections(vec![section("main")]);
    app.focus = Some(Focus::Branch("refs/heads/main".to_owned()));

    app.handle_key_event(key(KeyCode::Char('y')));
    app.handle_key_event(key(KeyCode::Char('y')));
    assert_eq!(app.pending_operator, None);
    assert!(app.clipboard_request.is_none());

    app.focus = None;
    app.handle_key_event(key(KeyCode::Char('y')));
    app.handle_key_event(key(KeyCode::Char('y')));
    assert_eq!(app.pending_operator, None);
    assert!(app.clipboard_request.is_none());
}

#[test]
fn clipboard_flush_emits_and_consumes_osc52_request() {
    let mut app = app_with_sections(vec![section("main")]);
    app.clipboard_request = Some("Ship 🚀 café".to_owned());
    let mut output = Vec::new();

    app.flush_clipboard_request(&mut output);

    assert!(app.clipboard_request.is_none());
    assert!(!output.is_empty());
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("U2hpcCDwn5qAIGNhZsOp")
    );
    assert_eq!(app.error.as_deref(), Some("Copied todo text"));
}

#[test]
fn clipboard_flush_consumes_request_and_reports_writer_failure() {
    struct FailingWriter;

    impl std::io::Write for FailingWriter {
        fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("deliberate failure"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let mut app = app_with_sections(vec![section("main")]);
    app.clipboard_request = Some("selected".to_owned());

    app.flush_clipboard_request(&mut FailingWriter);

    assert!(app.clipboard_request.is_none());
    assert!(
        app.error
            .as_deref()
            .is_some_and(|error| error.starts_with("copy: "))
    );
}

#[test]
fn escape_clears_normal_mode_row_selection() {
    let mut app = app_with_sections(vec![section("main")]);
    let todo = app
        .store
        .insert_todo("refs/heads/main", "selected", None)
        .unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(todo.id));

    app.handle_key_event(key(KeyCode::Char('d')));
    assert_eq!(app.pending_operator, Some(PendingOperator::Cut));
    app.handle_key_event(key(KeyCode::Esc));

    assert_eq!(app.focus, None);
    assert_eq!(app.pending_operator, None);
    assert_eq!(app.store.load_all().unwrap(), app.todos);

    app.focus = Some(Focus::Branch("refs/heads/main".to_owned()));
    app.handle_key_event(key(KeyCode::Esc));
    assert_eq!(app.focus, None);
}

#[test]
fn dd_requires_consecutive_keys_and_focuses_the_next_row_after_cut() {
    let mut app = app_with_sections(vec![section("main")]);
    let first = app
        .store
        .insert_todo("refs/heads/main", "first", None)
        .unwrap();
    let second = app
        .store
        .insert_todo("refs/heads/main", "second", Some(first.id))
        .unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(first.id));

    app.handle_key_event(key(KeyCode::Char('d')));
    assert_eq!(app.pending_operator, Some(PendingOperator::Cut));
    assert_eq!(app.store.load_all().unwrap().len(), 2);

    app.handle_key_event(key(KeyCode::Char('j')));
    assert_eq!(app.pending_operator, None);
    assert_eq!(app.focus, Some(Focus::Todo(second.id)));
    assert_eq!(app.store.load_all().unwrap().len(), 2);

    app.handle_key_event(key(KeyCode::Char('d')));
    app.handle_key_event(key(KeyCode::Char('k')));
    assert_eq!(app.focus, Some(Focus::Todo(first.id)));
    assert_eq!(app.store.load_all().unwrap().len(), 2);

    app.handle_key_event(key(KeyCode::Char('d')));
    app.handle_key_event(key(KeyCode::Char('d')));

    assert_eq!(app.pending_operator, None);
    assert_eq!(app.focus, Some(Focus::Todo(second.id)));
    assert_eq!(
        branch_titles(&app, "refs/heads/main"),
        vec![("second".to_owned(), false)]
    );
    assert_eq!(app.store.load_all().unwrap(), app.todos);
    assert_eq!(
        app.todo_register.as_ref().map(|todo| todo.title.as_str()),
        Some("first")
    );
}

#[test]
fn paste_register_persists_and_preserves_completion() {
    let mut app = app_with_sections(vec![section("main")]);
    let completed = app
        .store
        .insert_todo("refs/heads/main", "repeat me", None)
        .unwrap();
    app.store.toggle_todo(completed.id).unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(completed.id));

    app.handle_key_event(key(KeyCode::Char('d')));
    app.handle_key_event(key(KeyCode::Char('d')));
    let registered_id = app.todo_register.as_ref().unwrap().id;
    assert!(app.store.load_all().unwrap().is_empty());

    app.handle_key_event(key(KeyCode::Char('p')));
    let first_paste = match app.focus {
        Some(Focus::Todo(id)) => id,
        _ => panic!("pasted todo must receive focus"),
    };
    app.handle_key_event(key(KeyCode::Char('p')));
    let second_paste = match app.focus {
        Some(Focus::Todo(id)) => id,
        _ => panic!("pasted todo must receive focus"),
    };

    assert_ne!(first_paste, second_paste);
    assert_eq!(app.todo_register.as_ref().unwrap().id, registered_id);
    assert_eq!(
        branch_titles(&app, "refs/heads/main"),
        vec![
            ("repeat me".to_owned(), true),
            ("repeat me".to_owned(), true)
        ]
    );
    assert_eq!(app.store.load_all().unwrap(), app.todos);
}

#[test]
fn todo_paste_positions_above_and_below_in_another_section() {
    let mut app = app_with_sections(vec![section("main"), section("topic")]);
    let first = app
        .store
        .insert_todo("refs/heads/main", "first", None)
        .unwrap();
    let second = app
        .store
        .insert_todo("refs/heads/main", "second", Some(first.id))
        .unwrap();
    let source = app
        .store
        .insert_todo("refs/heads/topic", "source", None)
        .unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(source.id));
    app.handle_key_event(key(KeyCode::Char('d')));
    app.handle_key_event(key(KeyCode::Char('d')));

    app.focus = Some(Focus::Todo(second.id));
    app.handle_key_event(key(KeyCode::Char('P')));
    let above = app.focus.clone();
    assert_eq!(
        branch_titles(&app, "refs/heads/main"),
        vec![
            ("first".to_owned(), false),
            ("source".to_owned(), false),
            ("second".to_owned(), false)
        ]
    );

    app.handle_key_event(key(KeyCode::Char('p')));
    assert_ne!(app.focus, above);
    assert_eq!(
        branch_titles(&app, "refs/heads/main"),
        vec![
            ("first".to_owned(), false),
            ("source".to_owned(), false),
            ("source".to_owned(), false),
            ("second".to_owned(), false)
        ]
    );
    assert!(branch_titles(&app, "refs/heads/topic").is_empty());
    assert_eq!(app.store.load_all().unwrap(), app.todos);
}

#[test]
fn header_paste_uses_section_top_and_bottom() {
    let mut app = app_with_sections(vec![section("main"), section("topic")]);
    let first = app
        .store
        .insert_todo("refs/heads/main", "first", None)
        .unwrap();
    app.store
        .insert_todo("refs/heads/main", "second", Some(first.id))
        .unwrap();
    let source = app
        .store
        .insert_todo("refs/heads/topic", "source", None)
        .unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(source.id));
    app.handle_key_event(key(KeyCode::Char('d')));
    app.handle_key_event(key(KeyCode::Char('d')));

    app.focus = Some(Focus::Branch("refs/heads/main".to_owned()));
    app.handle_key_event(key(KeyCode::Char('P')));
    assert_eq!(
        branch_titles(&app, "refs/heads/main"),
        vec![
            ("source".to_owned(), false),
            ("first".to_owned(), false),
            ("second".to_owned(), false)
        ]
    );

    app.focus = Some(Focus::Branch("refs/heads/main".to_owned()));
    app.handle_key_event(key(KeyCode::Char('p')));
    assert_eq!(
        branch_titles(&app, "refs/heads/main"),
        vec![
            ("source".to_owned(), false),
            ("first".to_owned(), false),
            ("second".to_owned(), false),
            ("source".to_owned(), false)
        ]
    );
    assert!(matches!(app.focus, Some(Focus::Todo(_))));
    assert_eq!(app.store.load_all().unwrap(), app.todos);
}

#[test]
fn register_paste_no_ops_and_failures_preserve_state() {
    let mut app = app_with_sections(vec![section("main")]);
    app.handle_key_event(key(KeyCode::Char('p')));
    app.handle_key_event(key(KeyCode::Char('P')));
    app.handle_key_event(key(KeyCode::Char('d')));
    app.handle_key_event(key(KeyCode::Char('d')));
    assert!(app.todos.is_empty());
    assert!(app.todo_register.is_none());

    let retained = app
        .store
        .insert_todo("refs/heads/main", "retained", None)
        .unwrap();
    app.reload();
    app.todo_register = Some(retained.clone());
    app.focus = Some(Focus::Todo(i64::MAX));
    app.handle_key_event(key(KeyCode::Char('d')));
    app.handle_key_event(key(KeyCode::Char('d')));

    assert_eq!(app.todo_register, Some(retained.clone()));
    assert_eq!(app.store.load_all().unwrap(), app.todos);
    assert!(app.error.is_some());

    let mut invalid_register = retained;
    invalid_register.title = "   ".to_owned();
    app.todo_register = Some(invalid_register.clone());
    app.focus = Some(Focus::Branch("refs/heads/main".to_owned()));
    let todos_before_failed_paste = app.todos.clone();
    app.handle_key_event(key(KeyCode::Char('p')));

    assert_eq!(app.todo_register, Some(invalid_register));
    assert_eq!(app.todos, todos_before_failed_paste);
    assert_eq!(app.store.load_all().unwrap(), app.todos);
    assert!(app.error.is_some());
}

#[test]
fn insert_mode_treats_cut_and_paste_keys_as_text() {
    let mut app = app_with_sections(vec![section("main")]);
    app.handle_key_event(key(KeyCode::Char('o')));

    app.handle_key_event(key(KeyCode::Char('d')));
    app.handle_key_event(key(KeyCode::Char('d')));
    app.handle_key_event(key(KeyCode::Char('p')));
    app.handle_key_event(key(KeyCode::Char('P')));

    assert_eq!(editor(&app).text, "ddpP");
    assert_eq!(app.pending_operator, None);
    assert!(app.todo_register.is_none());
    assert!(app.store.load_all().unwrap().is_empty());
}

#[test]
fn header_creation_inserts_first_and_chains_entries() {
    let mut app = app_with_sections(vec![section("main")]);
    let existing_first = app
        .store
        .insert_todo("refs/heads/main", "existing first", None)
        .unwrap();
    app.store
        .insert_todo(
            "refs/heads/main",
            "existing second",
            Some(existing_first.id),
        )
        .unwrap();
    app.reload();
    app.focus = Some(Focus::Branch("refs/heads/main".to_owned()));

    app.handle_key_event(key(KeyCode::Char('o')));
    type_text(&mut app, "first");
    assert_eq!(
        app.store
            .load_all()
            .unwrap()
            .iter()
            .map(|todo| todo.title.as_str())
            .collect::<Vec<_>>(),
        ["existing first", "existing second"]
    );
    let backend = TestBackend::new(40, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(row_text(&terminal, 3).contains("󰄱 first"));
    assert!(row_text(&terminal, 4).contains("󰄱 existing first"));

    app.handle_key_event(key(KeyCode::Enter));
    assert_eq!(
        app.todos
            .iter()
            .map(|todo| todo.title.as_str())
            .collect::<Vec<_>>(),
        ["first", "existing first", "existing second"]
    );
    assert!(matches!(&app.mode, Mode::Insert(_)));
    type_text(&mut app, "second");
    app.handle_key_event(key(KeyCode::Enter));
    assert_eq!(
        app.todos
            .iter()
            .map(|todo| todo.title.as_str())
            .collect::<Vec<_>>(),
        ["first", "second", "existing first", "existing second"]
    );
}

#[test]
fn opening_on_todo_inserts_directly_below_it() {
    let mut app = app_with_sections(vec![section("main")]);
    let first = app
        .store
        .insert_todo("refs/heads/main", "first", None)
        .unwrap();
    app.store
        .insert_todo("refs/heads/main", "third", Some(first.id))
        .unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(first.id));
    app.handle_key_event(key(KeyCode::Char('o')));
    type_text(&mut app, "second");
    app.handle_key_event(key(KeyCode::Enter));
    assert_eq!(
        app.todos
            .iter()
            .map(|todo| todo.title.as_str())
            .collect::<Vec<_>>(),
        ["first", "second", "third"]
    );
}

#[test]
fn blank_enter_and_escape_discard_without_writing() {
    let mut app = app_with_sections(vec![section("main")]);
    app.handle_key_event(key(KeyCode::Char('o')));
    type_text(&mut app, "   ");
    app.handle_key_event(key(KeyCode::Enter));
    assert!(matches!(&app.mode, Mode::Normal));
    assert!(app.store.load_all().unwrap().is_empty());
    app.handle_key_event(key(KeyCode::Char('o')));
    type_text(&mut app, "discard me");
    app.handle_key_event(key(KeyCode::Esc));
    assert!(matches!(&app.mode, Mode::Normal));
    assert!(app.store.load_all().unwrap().is_empty());
}

#[test]
fn unavailable_persistence_does_not_open_an_editor() {
    let mut app = app_with_sections(vec![section("main")]);
    app.persistence_available = false;
    app.error = Some("database unavailable".to_owned());

    app.handle_key_event(key(KeyCode::Char('o')));

    assert!(matches!(&app.mode, Mode::Normal));
    assert_eq!(app.error.as_deref(), Some("database unavailable"));
}

#[test]
fn edit_save_updates_title_and_returns_to_normal_with_focus() {
    let mut app = app_with_sections(vec![section("main")]);
    let todo = app
        .store
        .insert_todo("refs/heads/main", "old", None)
        .unwrap();
    app.store.toggle_todo(todo.id).unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(todo.id));

    app.handle_key_event(key(KeyCode::Char('i')));
    assert_eq!(editor(&app).text, "old");
    assert_eq!(editor(&app).cursor, "old".len());
    app.handle_key_event(key(KeyCode::Home));
    for _ in 0..3 {
        app.handle_key_event(key(KeyCode::Delete));
    }
    type_text(&mut app, "new title");
    app.handle_key_event(key(KeyCode::Enter));

    assert!(matches!(&app.mode, Mode::Normal));
    assert_eq!(app.focus, Some(Focus::Todo(todo.id)));
    let persisted = app.store.load_all().unwrap();
    assert_eq!(persisted[0].title, "new title");
    assert!(persisted[0].completed);
    assert_eq!(app.todos[0].title, "new title");
    assert!(app.todos[0].completed);
}

#[test]
fn edit_escape_cancels_without_changing_the_todo() {
    let mut app = app_with_sections(vec![section("main")]);
    let todo = app
        .store
        .insert_todo("refs/heads/main", "unchanged", None)
        .unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(todo.id));

    app.handle_key_event(key(KeyCode::Char('i')));
    type_text(&mut app, " addition");
    app.handle_key_event(key(KeyCode::Esc));

    assert!(matches!(&app.mode, Mode::Normal));
    assert_eq!(app.focus, Some(Focus::Todo(todo.id)));
    assert_eq!(app.store.load_all().unwrap()[0].title, "unchanged");
    assert_eq!(app.todos[0].title, "unchanged");
}

#[test]
fn blank_edit_is_rejected_and_retains_the_editor_buffer() {
    let mut app = app_with_sections(vec![section("main")]);
    let todo = app
        .store
        .insert_todo("refs/heads/main", "keep", None)
        .unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(todo.id));

    app.handle_key_event(key(KeyCode::Char('i')));
    app.handle_key_event(key(KeyCode::Home));
    for _ in 0..4 {
        app.handle_key_event(key(KeyCode::Delete));
    }
    app.handle_key_event(key(KeyCode::Enter));

    assert!(matches!(&app.mode, Mode::Insert(_)));
    assert!(editor(&app).text.is_empty());
    assert_eq!(editor(&app).cursor, 0);
    assert!(app.error.is_some());
    assert_eq!(app.store.load_all().unwrap()[0].title, "keep");
    assert_eq!(app.todos[0].title, "keep");
}

#[test]
fn failed_edit_keeps_the_editor_buffer_and_original_todo() {
    let mut app = app_with_sections(vec![section("main")]);
    let todo = app
        .store
        .insert_todo("refs/heads/main", "original", None)
        .unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(todo.id));
    app.handle_key_event(key(KeyCode::Char('i')));
    type_text(&mut app, " retained");
    let Mode::Insert(active_editor) = &mut app.mode else {
        panic!("expected insert mode");
    };
    active_editor.target = EditorTarget::Update { id: i64::MAX };

    app.handle_key_event(key(KeyCode::Enter));

    assert!(matches!(&app.mode, Mode::Insert(_)));
    assert_eq!(editor(&app).text, "original retained");
    assert!(app.error.is_some());
    assert_eq!(app.store.load_all().unwrap()[0].title, "original");
    assert_eq!(app.todos[0].title, "original");
}

#[test]
fn branch_i_is_a_no_op() {
    let mut app = app_with_sections(vec![section("main")]);
    let focus = app.focus.clone();
    app.error = Some("existing error".to_owned());

    app.handle_key_event(key(KeyCode::Char('i')));

    assert!(matches!(&app.mode, Mode::Normal));
    assert_eq!(app.focus, focus);
    assert_eq!(app.error.as_deref(), Some("existing error"));
}

#[test]
fn edit_renders_in_place_with_the_original_completion_marker() {
    let mut app = app_with_sections(vec![section("main")]);
    let first = app
        .store
        .insert_todo("refs/heads/main", "first", None)
        .unwrap();
    let second = app
        .store
        .insert_todo("refs/heads/main", "second", Some(first.id))
        .unwrap();
    app.store.toggle_todo(second.id).unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(second.id));
    app.handle_key_event(key(KeyCode::Char('i')));
    let backend = TestBackend::new(40, 8);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|frame| app.draw(frame)).unwrap();

    assert!(row_text(&terminal, 3).contains("󰄱 first"));
    assert!(row_text(&terminal, 4).contains("󰄲 second"));
    assert!(!row_text(&terminal, 5).contains("󰄱"));
    assert!(!row_text(&terminal, 5).contains("󰄲"));
    assert_eq!(
        terminal.backend_mut().get_cursor_position().unwrap(),
        Position::new(14, 4)
    );
    assert_eq!(app.focus, Some(Focus::Todo(second.id)));
}

#[test]
fn normal_mode_toggles_focused_todo_and_persists_each_state() {
    let mut app = app_with_sections(vec![section("main")]);
    let todo = app
        .store
        .insert_todo("refs/heads/main", "toggle me", None)
        .unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(todo.id));
    app.error = Some("old error".to_owned());

    app.handle_key_event(key(KeyCode::Char('x')));

    assert!(
        app.todos
            .iter()
            .find(|item| item.id == todo.id)
            .unwrap()
            .completed
    );
    assert!(app.store.load_all().unwrap()[0].completed);
    assert_eq!(app.focus, Some(Focus::Todo(todo.id)));
    assert!(app.error.is_none());
    let backend = TestBackend::new(40, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(row_text(&terminal, 3).contains("󰄲 toggle me"));
    assert_eq!(
        terminal.backend().buffer()[(5, 3)].fg,
        app.theme.foreground_muted
    );
    assert_eq!(
        terminal.backend().buffer()[(7, 3)].fg,
        app.theme.foreground_muted
    );

    app.handle_key_event(key(KeyCode::Char(' ')));

    assert!(
        !app.todos
            .iter()
            .find(|item| item.id == todo.id)
            .unwrap()
            .completed
    );
    assert!(!app.store.load_all().unwrap()[0].completed);
    assert_eq!(app.focus, Some(Focus::Todo(todo.id)));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(row_text(&terminal, 3).contains("󰄱 toggle me"));
    assert_eq!(terminal.backend().buffer()[(5, 3)].fg, app.theme.foreground);
    assert_eq!(terminal.backend().buffer()[(7, 3)].fg, app.theme.foreground);
}

#[test]
fn toggle_keys_are_no_ops_on_branch_headers_or_without_persistence() {
    let mut app = app_with_sections(vec![section("main")]);
    let todo = app
        .store
        .insert_todo("refs/heads/main", "unchanged", None)
        .unwrap();
    app.reload();
    app.error = Some("existing error".to_owned());

    app.handle_key_event(key(KeyCode::Char('x')));
    app.handle_key_event(key(KeyCode::Char(' ')));

    assert!(!app.store.load_all().unwrap()[0].completed);
    assert_eq!(app.focus, Some(Focus::Branch("refs/heads/main".to_owned())));
    assert_eq!(app.error.as_deref(), Some("existing error"));

    app.focus = Some(Focus::Todo(todo.id));
    app.persistence_available = false;
    app.handle_key_event(key(KeyCode::Char('x')));
    assert!(!app.store.load_all().unwrap()[0].completed);
    assert_eq!(app.focus, Some(Focus::Todo(todo.id)));
    assert_eq!(app.error.as_deref(), Some("existing error"));
}

#[test]
fn insert_mode_accepts_x_and_space_as_text() {
    let mut app = app_with_sections(vec![section("main")]);
    app.handle_key_event(key(KeyCode::Char('o')));

    app.handle_key_event(key(KeyCode::Char('x')));
    app.handle_key_event(key(KeyCode::Char(' ')));

    assert!(matches!(&app.mode, Mode::Insert(_)));
    assert_eq!(editor(&app).text, "x ");
    assert!(app.store.load_all().unwrap().is_empty());
}

#[test]
fn failed_toggle_preserves_focus_and_in_memory_state() {
    let mut app = app_with_sections(vec![section("main")]);
    app.focus = Some(Focus::Todo(i64::MAX));

    app.handle_key_event(key(KeyCode::Char('x')));

    assert_eq!(app.focus, Some(Focus::Todo(i64::MAX)));
    assert!(app.todos.is_empty());
    assert!(app.error.is_some());
}

#[test]
fn failed_commit_keeps_the_editor_and_insert_mode() {
    let mut app = app_with_sections(vec![section("main")]);
    app.handle_key_event(key(KeyCode::Char('o')));
    type_text(&mut app, "keep me");
    let Mode::Insert(active_editor) = &mut app.mode else {
        panic!("expected insert mode");
    };
    let EditorTarget::Create { after, .. } = &mut active_editor.target else {
        panic!("expected create editor");
    };
    *after = Some(i64::MAX);

    app.handle_key_event(key(KeyCode::Enter));

    assert!(matches!(&app.mode, Mode::Insert(_)));
    assert_eq!(editor(&app).text, "keep me");
    assert!(app.error.is_some());
    assert!(app.store.load_all().unwrap().is_empty());
}

#[test]
fn unicode_editing_uses_grapheme_boundaries() {
    let mut app = app_with_sections(vec![section("main")]);
    app.handle_key_event(key(KeyCode::Char('o')));
    type_text(&mut app, "a界é");
    app.handle_key_event(key(KeyCode::Left));
    app.handle_key_event(key(KeyCode::Backspace));
    app.handle_key_event(key(KeyCode::Char('好')));
    app.handle_key_event(key(KeyCode::Home));
    app.handle_key_event(key(KeyCode::Right));
    app.handle_key_event(key(KeyCode::Left));
    app.handle_key_event(key(KeyCode::Delete));
    app.handle_key_event(key(KeyCode::End));
    assert_eq!(editor(&app).text, "好é");
    assert_eq!(editor(&app).cursor, "好é".len());
}

#[test]
fn modified_arrows_move_between_unicode_word_boundaries() {
    let mut app = app_with_sections(vec![section("main")]);
    app.handle_key_event(key(KeyCode::Char('o')));
    type_text(&mut app, "alpha βeta gamma");

    app.handle_key_event(modified_key(KeyCode::Left, KeyModifiers::CONTROL));
    assert_eq!(editor(&app).cursor, "alpha βeta ".len());
    app.handle_key_event(modified_key(KeyCode::Left, KeyModifiers::SHIFT));
    assert_eq!(editor(&app).cursor, "alpha ".len());
    app.handle_key_event(modified_key(KeyCode::Left, KeyModifiers::CONTROL));
    assert_eq!(editor(&app).cursor, 0);
    app.handle_key_event(modified_key(KeyCode::Left, KeyModifiers::CONTROL));
    assert_eq!(editor(&app).cursor, 0);

    app.handle_key_event(modified_key(KeyCode::Right, KeyModifiers::SHIFT));
    assert_eq!(editor(&app).cursor, "alpha ".len());
    app.handle_key_event(modified_key(KeyCode::Right, KeyModifiers::CONTROL));
    assert_eq!(editor(&app).cursor, "alpha βeta ".len());
    app.handle_key_event(modified_key(KeyCode::Right, KeyModifiers::SHIFT));
    assert_eq!(editor(&app).cursor, "alpha βeta gamma".len());
    app.handle_key_event(modified_key(KeyCode::Right, KeyModifiers::CONTROL));
    assert_eq!(editor(&app).cursor, "alpha βeta gamma".len());
}

#[test]
fn insertion_keeps_cursor_on_a_full_string_grapheme_boundary() {
    let mut app = app_with_sections(vec![section("main")]);
    app.handle_key_event(key(KeyCode::Char('o')));
    type_text(&mut app, "👩🔬");
    app.handle_key_event(key(KeyCode::Left));

    app.handle_key_event(key(KeyCode::Char('\u{200d}')));

    let input = editor(&app);
    assert_eq!(input.text, "👩‍🔬");
    assert_eq!(input.cursor, input.text.len());
    app.handle_key_event(key(KeyCode::Backspace));
    assert!(editor(&app).text.is_empty());
}
