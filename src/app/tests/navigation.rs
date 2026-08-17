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
