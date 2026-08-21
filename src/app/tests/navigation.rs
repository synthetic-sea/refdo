use super::support::*;
use super::*;

#[test]
fn unselected_navigation_starts_at_directional_edge() {
    let build_app = || {
        let mut app = app_with_sections(vec![section("main"), section("topic")]);
        app.store
            .insert_todo("refs/heads/main", "top", None)
            .unwrap();
        let bottom = app
            .store
            .insert_todo("refs/heads/topic", "bottom", None)
            .unwrap();
        app.reload();
        app.focus = None;
        (app, bottom.id)
    };

    for code in [KeyCode::Char('j'), KeyCode::Down] {
        let (mut app, _) = build_app();
        app.handle_key_event(key(code));
        assert_eq!(app.focus, Some(Focus::Branch("refs/heads/main".to_owned())));
    }

    for code in [KeyCode::Char('k'), KeyCode::Up] {
        let (mut app, bottom_id) = build_app();
        app.handle_key_event(key(code));
        assert_eq!(app.focus, Some(Focus::Todo(bottom_id)));
    }
}

#[test]
fn flattened_navigation_visits_headers_and_todos() {
    let mut app = app_with_sections(vec![section("main"), section("topic")]);
    let todo = app
        .store
        .insert_todo("refs/heads/main", "one", None)
        .unwrap();
    app.reload();
    app.handle_key_event(key(KeyCode::Char('j')));
    assert_eq!(app.focus, Some(Focus::Todo(todo.id)));
    app.handle_key_event(key(KeyCode::Char('j')));
    assert_eq!(
        app.focus,
        Some(Focus::Branch("refs/heads/topic".to_owned()))
    );
    app.handle_key_event(key(KeyCode::Char('k')));
    assert_eq!(app.focus, Some(Focus::Todo(todo.id)));
}

#[test]
fn bracket_navigation_moves_between_section_headers() {
    let mut app = app_with_sections(vec![section("main"), section("topic"), section("third")]);

    app.handle_key_event(key(KeyCode::Char(']')));
    assert_eq!(
        app.focus,
        Some(Focus::Branch("refs/heads/topic".to_owned()))
    );
    app.handle_key_event(key(KeyCode::Char(']')));
    assert_eq!(
        app.focus,
        Some(Focus::Branch("refs/heads/third".to_owned()))
    );
    app.handle_key_event(key(KeyCode::Char(']')));
    assert_eq!(
        app.focus,
        Some(Focus::Branch("refs/heads/third".to_owned()))
    );

    app.handle_key_event(key(KeyCode::Char('[')));
    assert_eq!(
        app.focus,
        Some(Focus::Branch("refs/heads/topic".to_owned()))
    );
    app.handle_key_event(key(KeyCode::Char('[')));
    assert_eq!(app.focus, Some(Focus::Branch("refs/heads/main".to_owned())));
    app.handle_key_event(key(KeyCode::Char('[')));
    assert_eq!(app.focus, Some(Focus::Branch("refs/heads/main".to_owned())));
}

#[test]
fn bracket_navigation_uses_nearest_header_and_directional_edge() {
    let mut app = app_with_sections(vec![section("main"), section("topic")]);
    let topic_todo = app
        .store
        .insert_todo("refs/heads/topic", "topic todo", None)
        .unwrap();
    app.reload();

    app.focus = Some(Focus::Todo(topic_todo.id));
    app.handle_key_event(key(KeyCode::Char('[')));
    assert_eq!(
        app.focus,
        Some(Focus::Branch("refs/heads/topic".to_owned()))
    );

    app.focus = None;
    app.handle_key_event(key(KeyCode::Char(']')));
    assert_eq!(app.focus, Some(Focus::Branch("refs/heads/main".to_owned())));
    app.focus = None;
    app.handle_key_event(key(KeyCode::Char('[')));
    assert_eq!(
        app.focus,
        Some(Focus::Branch("refs/heads/topic".to_owned()))
    );
}

#[test]
fn arrow_navigation_visits_headers_and_todos() {
    let mut app = app_with_sections(vec![section("main"), section("topic")]);
    let todo = app
        .store
        .insert_todo("refs/heads/main", "one", None)
        .unwrap();
    app.reload();

    app.handle_key_event(key(KeyCode::Down));
    assert_eq!(app.focus, Some(Focus::Todo(todo.id)));
    app.handle_key_event(key(KeyCode::Down));
    assert_eq!(
        app.focus,
        Some(Focus::Branch("refs/heads/topic".to_owned()))
    );
    app.handle_key_event(key(KeyCode::Up));
    assert_eq!(app.focus, Some(Focus::Todo(todo.id)));
}

#[test]
fn focused_todo_and_unicode_cursor_remain_visible() {
    let mut app = app_with_sections(vec![section("main"), section("topic")]);
    let todo = app
        .store
        .insert_todo("refs/heads/topic", "visible", None)
        .unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(todo.id));
    let backend = TestBackend::new(18, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(row_text(&terminal, 2).contains("visible"));

    app.handle_key_event(key(KeyCode::Char('o')));
    type_text(&mut app, "ab界cd界ef");
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let cursor = terminal.backend_mut().get_cursor_position().unwrap();
    assert_eq!(cursor.y, 2);
    assert!(cursor.x < 17);
}

#[test]
fn exact_width_editor_keeps_a_full_grapheme_visible() {
    let mut app = app_with_sections(vec![section("main")]);
    app.handle_key_event(key(KeyCode::Char('o')));
    type_text(&mut app, "👩‍🔬");
    let backend = TestBackend::new(14, 5);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|frame| app.draw(frame)).unwrap();

    assert_eq!(
        terminal.backend_mut().get_cursor_position().unwrap(),
        Position::new(9, 2)
    );
    assert_eq!(terminal.backend().buffer()[(7, 2)].symbol(), "👩‍🔬");
}

#[test]
fn select_mode_entry_with_no_items_selected_or_focused_todo() {
    let mut app = app_with_sections(vec![section("main"), section("topic")]);
    let todo = app
        .store
        .insert_todo("refs/heads/main", "first", None)
        .unwrap();
    app.reload();

    // 1. When focus is on a branch header (no items selected), pressing v enables select mode with no items selected
    app.focus = Some(Focus::Branch("refs/heads/main".to_owned()));
    app.handle_key_event(key(KeyCode::Char('v')));
    let Mode::Select(select_state) = &app.mode else {
        panic!("expected select mode from branch header");
    };
    assert_eq!(select_state.branch_ref, "refs/heads/main");
    assert!(select_state.selected_todo_ids.is_empty());
    assert_eq!(app.focus, Some(Focus::Todo(todo.id)));

    // Exit back to normal mode
    app.handle_key_event(key(KeyCode::Esc));
    assert!(matches!(&app.mode, Mode::Normal));

    // 2. When focus is None (no items selected), pressing v enables select mode on first section with no items selected
    app.focus = None;
    app.handle_key_event(key(KeyCode::Char('v')));
    let Mode::Select(select_state) = &app.mode else {
        panic!("expected select mode from None focus");
    };
    assert_eq!(select_state.branch_ref, "refs/heads/main");
    assert!(select_state.selected_todo_ids.is_empty());
    assert_eq!(app.focus, Some(Focus::Todo(todo.id)));

    // Exit back to normal mode
    app.handle_key_event(key(KeyCode::Esc));

    // 3. When focus is on a branch with no todos, v is a no-op
    app.focus = Some(Focus::Branch("refs/heads/topic".to_owned()));
    app.handle_key_event(key(KeyCode::Char('v')));
    assert!(matches!(&app.mode, Mode::Normal));

    // 4. When focus is on a valid todo, pressing v enters select mode and seeds selection with that todo
    app.focus = Some(Focus::Todo(todo.id));
    app.pending_operator = Some(PendingOperator::Cut);
    app.error = Some("stale error".to_owned());
    app.handle_key_event(key(KeyCode::Char('v')));

    let Mode::Select(select_state) = &app.mode else {
        panic!("expected select mode from todo focus");
    };
    assert_eq!(select_state.branch_ref, "refs/heads/main");
    assert_eq!(select_state.selected_todo_ids.len(), 1);
    assert!(select_state.selected_todo_ids.contains(&todo.id));
    assert_eq!(app.pending_operator, None);
    assert_eq!(app.error, None);
}

#[test]
fn select_mode_entry_with_no_focus_uses_first_displayed_section() {
    let mut current = section("z-current");
    current.is_current = true;
    let mut app = app_with_sections(vec![current, section("a-feature")]);
    let current_todo = app
        .store
        .insert_todo("refs/heads/z-current", "current", None)
        .unwrap();
    app.store
        .insert_todo("refs/heads/a-feature", "feature", None)
        .unwrap();
    app.reload();

    app.focus = None;
    app.handle_key_event(key(KeyCode::Char('v')));

    let Mode::Select(select_state) = &app.mode else {
        panic!("expected select mode from None focus");
    };
    assert_eq!(select_state.branch_ref, "refs/heads/z-current");
    assert!(select_state.selected_todo_ids.is_empty());
    assert_eq!(app.focus, Some(Focus::Todo(current_todo.id)));
}

#[test]
fn select_mode_toggling_supports_arbitrary_and_empty_selection() {
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

    app.focus = Some(Focus::Todo(t1.id));
    app.handle_key_event(key(KeyCode::Char('v')));

    // Move to t2 and toggle it on
    app.handle_key_event(key(KeyCode::Char('j')));
    assert_eq!(app.focus, Some(Focus::Todo(t2.id)));
    app.handle_key_event(key(KeyCode::Char(' ')));

    // Move to t3 and toggle it on
    app.handle_key_event(key(KeyCode::Char('j')));
    assert_eq!(app.focus, Some(Focus::Todo(t3.id)));
    app.handle_key_event(key(KeyCode::Char(' ')));

    {
        let Mode::Select(select_state) = &app.mode else {
            panic!("expected select mode");
        };
        assert_eq!(select_state.selected_todo_ids.len(), 3);
        assert!(select_state.selected_todo_ids.contains(&t1.id));
        assert!(select_state.selected_todo_ids.contains(&t2.id));
        assert!(select_state.selected_todo_ids.contains(&t3.id));
    }

    // Move back to t2 and toggle it off
    app.handle_key_event(key(KeyCode::Char('k')));
    assert_eq!(app.focus, Some(Focus::Todo(t2.id)));
    app.handle_key_event(key(KeyCode::Char(' ')));

    // Move back to t1 and toggle it off
    app.handle_key_event(key(KeyCode::Char('k')));
    assert_eq!(app.focus, Some(Focus::Todo(t1.id)));
    app.handle_key_event(key(KeyCode::Char(' ')));

    {
        let Mode::Select(select_state) = &app.mode else {
            panic!("expected select mode");
        };
        assert_eq!(select_state.selected_todo_ids.len(), 1);
        assert!(!select_state.selected_todo_ids.contains(&t1.id));
        assert!(!select_state.selected_todo_ids.contains(&t2.id));
        assert!(select_state.selected_todo_ids.contains(&t3.id));
    }

    // Move to t3 and toggle it off -> empty selection
    app.handle_key_event(key(KeyCode::Char('j')));
    app.handle_key_event(key(KeyCode::Char('j')));
    assert_eq!(app.focus, Some(Focus::Todo(t3.id)));
    app.handle_key_event(key(KeyCode::Char(' ')));

    let Mode::Select(select_state) = &app.mode else {
        panic!("expected select mode");
    };
    assert!(select_state.selected_todo_ids.is_empty());
}

#[test]
fn select_mode_navigation_clamps_to_branch_boundaries() {
    let mut app = app_with_sections(vec![section("prev"), section("main"), section("next")]);
    app.store
        .insert_todo("refs/heads/prev", "prev todo", None)
        .unwrap();
    let t1 = app
        .store
        .insert_todo("refs/heads/main", "main 1", None)
        .unwrap();
    let t2 = app
        .store
        .insert_todo("refs/heads/main", "main 2", Some(t1.id))
        .unwrap();
    app.store
        .insert_todo("refs/heads/next", "next todo", None)
        .unwrap();
    app.reload();

    app.focus = Some(Focus::Todo(t1.id));
    app.handle_key_event(key(KeyCode::Char('v')));

    // Up / k at the top of the branch clamps to t1
    app.handle_key_event(key(KeyCode::Char('k')));
    assert_eq!(app.focus, Some(Focus::Todo(t1.id)));
    app.handle_key_event(key(KeyCode::Up));
    assert_eq!(app.focus, Some(Focus::Todo(t1.id)));

    // Down / j moves to t2
    app.handle_key_event(key(KeyCode::Char('j')));
    assert_eq!(app.focus, Some(Focus::Todo(t2.id)));

    // Down / j at the bottom of the branch clamps to t2
    app.handle_key_event(key(KeyCode::Char('j')));
    assert_eq!(app.focus, Some(Focus::Todo(t2.id)));
    app.handle_key_event(key(KeyCode::Down));
    assert_eq!(app.focus, Some(Focus::Todo(t2.id)));
}

#[test]
fn select_mode_exit_on_escape_preserves_focus_and_drops_selection() {
    let mut app = app_with_sections(vec![section("main")]);
    let t1 = app
        .store
        .insert_todo("refs/heads/main", "one", None)
        .unwrap();
    let t2 = app
        .store
        .insert_todo("refs/heads/main", "two", Some(t1.id))
        .unwrap();
    app.reload();

    app.focus = Some(Focus::Todo(t1.id));
    app.handle_key_event(key(KeyCode::Char('v')));
    app.handle_key_event(key(KeyCode::Char('j')));
    app.handle_key_event(key(KeyCode::Char(' ')));
    assert_eq!(app.focus, Some(Focus::Todo(t2.id)));

    // Esc exits to Normal mode, preserving focus on t2
    app.handle_key_event(key(KeyCode::Esc));
    assert!(matches!(&app.mode, Mode::Normal));
    assert_eq!(app.focus, Some(Focus::Todo(t2.id)));

    // Re-entering select mode seeds only t2
    app.handle_key_event(key(KeyCode::Char('v')));
    let Mode::Select(select_state) = &app.mode else {
        panic!("expected select mode");
    };
    assert_eq!(select_state.selected_todo_ids.len(), 1);
    assert!(select_state.selected_todo_ids.contains(&t2.id));
    assert!(!select_state.selected_todo_ids.contains(&t1.id));
}

#[test]
fn select_mode_ignores_normal_mode_operations() {
    let mut app = app_with_sections(vec![section("main"), section("topic")]);
    let t1 = app
        .store
        .insert_todo("refs/heads/main", "one", None)
        .unwrap();
    app.reload();

    app.focus = Some(Focus::Todo(t1.id));
    app.handle_key_event(key(KeyCode::Char('v')));

    // Normal keys are ignored
    app.handle_key_event(key(KeyCode::Char('x')));
    assert!(!app.todos[0].completed);

    app.handle_key_event(key(KeyCode::Char('o')));
    assert!(matches!(&app.mode, Mode::Select(_)));

    app.handle_key_event(key(KeyCode::Char('i')));
    assert!(matches!(&app.mode, Mode::Select(_)));

    app.handle_key_event(key(KeyCode::Char('d')));
    app.handle_key_event(key(KeyCode::Char('d')));
    assert_eq!(app.todos.len(), 1);

    app.handle_key_event(key(KeyCode::Char('y')));
    app.handle_key_event(key(KeyCode::Char('y')));
    assert_eq!(app.todo_register, None);

    app.handle_key_event(key(KeyCode::Char('p')));
    app.handle_key_event(key(KeyCode::Char('P')));
    assert_eq!(app.todos.len(), 1);

    app.handle_key_event(key(KeyCode::Char(']')));
    app.handle_key_event(key(KeyCode::Char('[')));
    assert_eq!(app.focus, Some(Focus::Todo(t1.id)));

    app.handle_key_event(key(KeyCode::Char('q')));
    assert!(!app.exit);
}
