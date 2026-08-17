use super::support::*;
use super::*;

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

#[test]
fn hover_background_applies_to_unselected_todo_and_branch_rows() {
    let mut app = app_with_sections(vec![section("main"), section("topic")]);
    app.store
        .insert_todo("refs/heads/main", "hover me", None)
        .unwrap();
    app.reload();
    let theme = app.theme;
    let backend = TestBackend::new(40, 8);
    let mut terminal = Terminal::new(backend).unwrap();

    app.handle_mouse_event(mouse(MouseEventKind::Moved, 2, 3));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(row_text(&terminal, 3).contains("󰄱 hover me"));
    assert_eq!(terminal.backend().buffer()[(5, 3)].symbol(), "󰄱");
    assert_eq!(terminal.backend().buffer()[(7, 3)].symbol(), "h");
    assert_eq!(
        terminal.backend().buffer()[(2, 3)].bg,
        theme.hover_background
    );

    app.handle_mouse_event(mouse(MouseEventKind::Moved, 2, 4));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(row_text(&terminal, 4).contains("topic"));
    assert_eq!(
        terminal.backend().buffer()[(2, 4)].bg,
        theme.hover_background
    );
}

#[test]
fn unfocused_rows_render_without_selection() {
    let mut app = app_with_sections(vec![section("main")]);
    app.focus = None;
    let theme = app.theme;
    let backend = TestBackend::new(40, 6);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|frame| app.draw(frame)).unwrap();

    assert_eq!(terminal.backend().buffer()[(2, 2)].bg, theme.background);
}

#[test]
fn focus_repair_preserves_explicit_deselection() {
    let mut app = app_with_sections(vec![section("main")]);
    app.focus = None;

    app.repair_focus();

    assert_eq!(app.focus, None);
}

#[test]
fn selected_background_takes_precedence_over_hover() {
    let mut app = app_with_sections(vec![section("main")]);
    let theme = app.theme;
    let backend = TestBackend::new(40, 6);
    let mut terminal = Terminal::new(backend).unwrap();

    app.handle_mouse_event(mouse(MouseEventKind::Moved, 2, 2));
    terminal.draw(|frame| app.draw(frame)).unwrap();

    assert_eq!(
        terminal.backend().buffer()[(2, 2)].bg,
        theme.selection_background
    );
    assert_ne!(
        terminal.backend().buffer()[(2, 2)].bg,
        theme.hover_background
    );
}

#[test]
fn normal_left_click_selects_the_rendered_row() {
    let mut app = app_with_sections(vec![section("main")]);
    let todo = app
        .store
        .insert_todo("refs/heads/main", "click me", None)
        .unwrap();
    app.reload();
    let backend = TestBackend::new(40, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();

    app.handle_mouse_event(mouse(MouseEventKind::Down(MouseButton::Left), 2, 3));

    assert_eq!(app.focus, Some(Focus::Todo(todo.id)));
}

#[test]
fn insert_mode_click_does_not_change_focus() {
    let mut app = app_with_sections(vec![section("main"), section("topic")]);
    app.handle_key_event(key(KeyCode::Char('o')));
    let original_focus = app.focus.clone();
    let backend = TestBackend::new(40, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(row_text(&terminal, 4).contains("topic"));

    app.handle_mouse_event(mouse(MouseEventKind::Down(MouseButton::Left), 2, 4));

    assert!(matches!(&app.mode, Mode::Insert(_)));
    assert_eq!(app.focus, original_focus);
}

#[test]
fn clicks_on_empty_rows_and_outside_the_viewport_deselect() {
    let mut app = app_with_sections(vec![section("main"), section("topic")]);
    let topic = Focus::Branch("refs/heads/topic".to_owned());
    let backend = TestBackend::new(40, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(row_text(&terminal, 3).contains("No todos"));

    for (column, row) in [(2, 3), (0, 2), (39, 2), (2, 6)] {
        app.focus = Some(topic.clone());
        app.handle_mouse_event(mouse(MouseEventKind::Down(MouseButton::Left), column, row));
        assert_eq!(app.focus, None);
    }
}

#[test]
fn click_hit_testing_uses_the_focus_driven_scroll_offset() {
    let mut app = app_with_sections(vec![section("main"), section("topic"), section("third")]);
    for branch in ["main", "topic", "third"] {
        app.store
            .insert_todo(
                &format!("refs/heads/{branch}"),
                &format!("{branch} todo"),
                None,
            )
            .unwrap();
    }
    app.reload();
    app.focus = Some(Focus::Branch("refs/heads/third".to_owned()));
    let backend = TestBackend::new(40, 7);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(row_text(&terminal, 2).contains("topic"));
    assert!(row_text(&terminal, 4).contains("third"));

    app.handle_mouse_event(mouse(MouseEventKind::Down(MouseButton::Left), 2, 2));

    assert_eq!(
        app.focus,
        Some(Focus::Branch("refs/heads/topic".to_owned()))
    );
}

#[test]
fn keyboard_target_already_visible_does_not_scroll() {
    let mut app = app_with_sections(vec![section("main")]);
    let first = app
        .store
        .insert_todo("refs/heads/main", "first", None)
        .unwrap();
    app.store
        .insert_todo("refs/heads/main", "second", Some(first.id))
        .unwrap();
    app.reload();
    let backend = TestBackend::new(40, 7);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();

    app.handle_key_event(key(KeyCode::Down));
    app.handle_key_event(key(KeyCode::Down));
    terminal.draw(|frame| app.draw(frame)).unwrap();

    assert_eq!(app.focus, Some(Focus::Todo(app.todos[1].id)));
    assert!(row_text(&terminal, 2).contains("main"));
    assert!(row_text(&terminal, 3).contains("first"));
    assert!(row_text(&terminal, 4).contains("second"));
}

#[test]
fn keyboard_boundary_crossing_scrolls_minimally_without_snapping_on_reverse() {
    let mut app = app_with_sections(vec![section("main")]);
    let wrapped = app
        .store
        .insert_todo("refs/heads/main", "one two three", None)
        .unwrap();
    let trailing = app
        .store
        .insert_todo("refs/heads/main", "trailing", Some(wrapped.id))
        .unwrap();
    app.reload();
    let backend = TestBackend::new(20, 7);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();

    app.handle_key_event(key(KeyCode::Down));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(row_text(&terminal, 2).contains("main"));
    assert!(row_text(&terminal, 3).contains("one two"));
    assert!(row_text(&terminal, 4).contains("three"));

    app.handle_key_event(key(KeyCode::Down));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert_eq!(app.focus, Some(Focus::Todo(trailing.id)));
    assert!(row_text(&terminal, 2).contains("one two"));
    assert!(row_text(&terminal, 3).contains("three"));
    assert!(row_text(&terminal, 4).contains("trailing"));

    app.handle_key_event(key(KeyCode::Up));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert_eq!(app.focus, Some(Focus::Todo(wrapped.id)));
    assert!(row_text(&terminal, 2).contains("one two"));
    assert!(row_text(&terminal, 3).contains("three"));
    assert!(row_text(&terminal, 4).contains("trailing"));
}

#[test]
fn wheel_events_scroll_inside_viewport_one_row_and_preserve_focus() {
    let mut app = app_with_sections(vec![section("main")]);
    let first = app
        .store
        .insert_todo("refs/heads/main", "first", None)
        .unwrap();
    let second = app
        .store
        .insert_todo("refs/heads/main", "second", Some(first.id))
        .unwrap();
    app.store
        .insert_todo("refs/heads/main", "third", Some(second.id))
        .unwrap();
    app.reload();
    let original_focus = app.focus.clone();
    let backend = TestBackend::new(40, 7);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();

    app.handle_mouse_event(mouse(MouseEventKind::ScrollDown, 2, 2));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert_eq!(app.focus, original_focus);
    assert!(row_text(&terminal, 2).contains("first"));
    assert!(row_text(&terminal, 4).contains("third"));

    app.handle_mouse_event(mouse(MouseEventKind::ScrollUp, 2, 2));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert_eq!(app.focus, original_focus);
    assert!(row_text(&terminal, 2).contains("main"));
    assert!(row_text(&terminal, 4).contains("second"));
}

#[test]
fn wheel_outside_todo_viewport_is_a_no_op() {
    let mut app = app_with_sections(vec![section("main")]);
    let first = app
        .store
        .insert_todo("refs/heads/main", "first", None)
        .unwrap();
    let second = app
        .store
        .insert_todo("refs/heads/main", "second", Some(first.id))
        .unwrap();
    let third = app
        .store
        .insert_todo("refs/heads/main", "third", Some(second.id))
        .unwrap();
    app.store
        .insert_todo("refs/heads/main", "fourth", Some(third.id))
        .unwrap();
    app.reload();
    let backend = TestBackend::new(40, 7);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    app.handle_mouse_event(mouse(MouseEventKind::ScrollDown, 2, 2));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(row_text(&terminal, 2).contains("first"));

    app.handle_mouse_event(mouse(MouseEventKind::ScrollDown, 0, 2));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(row_text(&terminal, 2).contains("first"));

    app.handle_mouse_event(mouse(MouseEventKind::ScrollUp, 0, 2));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(row_text(&terminal, 2).contains("first"));
}

#[test]
fn click_hit_testing_follows_manual_scroll() {
    let mut app = app_with_sections(vec![section("main")]);
    let first = app
        .store
        .insert_todo("refs/heads/main", "first", None)
        .unwrap();
    let second = app
        .store
        .insert_todo("refs/heads/main", "second", Some(first.id))
        .unwrap();
    app.store
        .insert_todo("refs/heads/main", "third", Some(second.id))
        .unwrap();
    app.reload();
    let backend = TestBackend::new(40, 7);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();

    app.handle_mouse_event(mouse(MouseEventKind::ScrollDown, 2, 2));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(row_text(&terminal, 2).contains("first"));
    app.handle_mouse_event(mouse(MouseEventKind::Down(MouseButton::Left), 2, 2));

    assert_eq!(app.focus, Some(Focus::Todo(first.id)));
}

#[test]
fn narrow_header_hides_tag_and_stored_scope_shows_branch_tag() {
    let theme = test_theme();
    let normal = section("very-long-branch");
    let stored = BranchSection {
        full_ref_name: "refs/heads/archive".to_owned(),
        display_name: "archive".to_owned(),
        worktree_path: Default::default(),
        is_current: false,
        is_locked: false,
        is_stored_only: true,
    };
    let mut app = app_with_sections(vec![normal, stored]);
    let backend = TestBackend::new(23, 9);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(row_text(&terminal, 1).starts_with('┌'));
    assert_eq!(
        terminal.backend().buffer()[(0, 1)].fg,
        theme.mode_background
    );
    assert!(!row_text(&terminal, 2).contains("WORKTREE"));
    assert!(row_text(&terminal, 2).starts_with("│ 󰍝 very-long-branch"));
    assert!(row_text(&terminal, 3).contains("No todos"));
    assert!(row_text(&terminal, 4).contains("BRANCH"));
    assert!(row_text(&terminal, 7).starts_with('└'));
    assert_eq!(terminal.backend().buffer()[(1, 2)].bg, theme.background);
    assert_eq!(
        terminal.backend().buffer()[(2, 2)].bg,
        theme.selection_background
    );
}

#[test]
fn long_titles_wrap_at_words_and_shift_following_rows() {
    let mut app = app_with_sections(vec![section("main")]);
    let first = app
        .store
        .insert_todo("refs/heads/main", "alpha beta gamma delta", None)
        .unwrap();
    app.store
        .insert_todo("refs/heads/main", "following", Some(first.id))
        .unwrap();
    app.reload();
    let backend = TestBackend::new(26, 9);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|frame| app.draw(frame)).unwrap();

    assert!(row_text(&terminal, 3).starts_with("│    󰄱 alpha beta gamma"));
    assert!(row_text(&terminal, 4).starts_with("│      delta"));
    assert!(row_text(&terminal, 5).starts_with("│    󰄱 following"));
}

#[test]
fn editor_wraps_as_text_is_typed_and_shifts_following_rows() {
    let mut app = app_with_sections(vec![section("main")]);
    app.store
        .insert_todo("refs/heads/main", "following", None)
        .unwrap();
    app.reload();
    app.focus = Some(Focus::Branch("refs/heads/main".to_owned()));
    app.handle_key_event(key(KeyCode::Char('o')));
    type_text(&mut app, "alpha beta gamma delta");
    let backend = TestBackend::new(26, 9);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|frame| app.draw(frame)).unwrap();

    assert!(row_text(&terminal, 3).starts_with("│    󰄱 alpha beta gamma"));
    assert!(row_text(&terminal, 4).starts_with("│      delta"));
    assert!(row_text(&terminal, 5).starts_with("│    󰄱 following"));
    assert_eq!(
        terminal.backend_mut().get_cursor_position().unwrap(),
        Position::new(12, 4)
    );

    for expected in [
        Position::new(7, 4),
        Position::new(18, 3),
        Position::new(13, 3),
    ] {
        app.handle_key_event(modified_key(KeyCode::Left, KeyModifiers::CONTROL));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert_eq!(
            terminal.backend_mut().get_cursor_position().unwrap(),
            expected
        );
    }
}

#[test]
fn unicode_overlong_words_wrap_without_splitting_graphemes() {
    let mut app = app_with_sections(vec![section("main")]);
    app.store
        .insert_todo("refs/heads/main", "ab👩‍🔬cd界ef\nnext", None)
        .unwrap();
    app.reload();
    let backend = TestBackend::new(18, 9);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|frame| app.draw(frame)).unwrap();

    assert!(row_text(&terminal, 3).starts_with("│    󰄱 ab"));
    assert!(row_text(&terminal, 4).starts_with("│      ef"));
    assert!(row_text(&terminal, 5).starts_with("│      next"));
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(9, 3)].symbol(), "👩‍🔬");
    assert_eq!(buffer[(11, 3)].symbol(), "c");
    assert_eq!(buffer[(12, 3)].symbol(), "d");
    assert_eq!(buffer[(13, 3)].symbol(), "界");
    assert_eq!(buffer[(7, 4)].symbol(), "e");
    assert_eq!(buffer[(8, 4)].symbol(), "f");
}

#[test]
fn selection_and_hover_cover_every_wrapped_line_and_continuations_hit_test() {
    let mut app = app_with_sections(vec![section("main")]);
    let todo = app
        .store
        .insert_todo("refs/heads/main", "alpha beta gamma delta", None)
        .unwrap();
    app.reload();
    let theme = app.theme;
    let backend = TestBackend::new(26, 8);
    let mut terminal = Terminal::new(backend).unwrap();

    app.focus = Some(Focus::Todo(todo.id));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    for row in [3, 4] {
        assert_eq!(terminal.backend().buffer()[(1, row)].bg, theme.background);
        assert_eq!(terminal.backend().buffer()[(24, row)].bg, theme.background);
        for column in 2..24 {
            assert_eq!(
                terminal.backend().buffer()[(column, row)].bg,
                theme.selection_background
            );
        }
    }

    app.focus = None;
    app.handle_mouse_event(mouse(MouseEventKind::Moved, 10, 4));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    for row in [3, 4] {
        assert_eq!(terminal.backend().buffer()[(1, row)].bg, theme.background);
        assert_eq!(terminal.backend().buffer()[(24, row)].bg, theme.background);
        for column in 2..24 {
            assert_eq!(
                terminal.backend().buffer()[(column, row)].bg,
                theme.hover_background
            );
        }
    }

    app.handle_mouse_event(mouse(MouseEventKind::Down(MouseButton::Left), 10, 4));
    assert_eq!(app.focus, Some(Focus::Todo(todo.id)));
}

#[test]
fn scrolling_keeps_a_wrapped_focus_visible_and_preserves_hit_testing() {
    let mut app = app_with_sections(vec![section("main")]);
    let leading = app
        .store
        .insert_todo("refs/heads/main", "leading", None)
        .unwrap();
    let wrapped = app
        .store
        .insert_todo("refs/heads/main", "one two three", Some(leading.id))
        .unwrap();
    app.reload();
    app.focus = Some(Focus::Todo(wrapped.id));
    let theme = app.theme;
    let backend = TestBackend::new(20, 7);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|frame| app.draw(frame)).unwrap();

    assert!(row_text(&terminal, 2).starts_with("│    󰄱 leading"));
    assert!(row_text(&terminal, 3).starts_with("│    󰄱 one two"));
    assert!(row_text(&terminal, 4).starts_with("│      three"));
    for row in [3, 4] {
        assert_eq!(
            terminal.backend().buffer()[(2, row)].bg,
            theme.selection_background
        );
    }

    app.handle_mouse_event(mouse(MouseEventKind::Down(MouseButton::Left), 2, 2));
    assert_eq!(app.focus, Some(Focus::Todo(leading.id)));
}

#[test]
fn very_narrow_terminals_render_wrapped_todos_without_panicking() {
    let mut app = app_with_sections(vec![section("main")]);
    app.store
        .insert_todo("refs/heads/main", "👩‍🔬overlong", None)
        .unwrap();
    app.reload();

    for width in 1..=9 {
        let backend = TestBackend::new(width, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
    }
}
