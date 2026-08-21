use std::{
    cell::Cell,
    path::PathBuf,
    time::{Duration, Instant},
};

use ratatui::{Terminal, backend::TestBackend};

use super::support::*;
use super::*;

#[test]
fn refresh_loads_other_process_commits_and_adds_their_branch() {
    let directory = std::env::temp_dir().join(format!(
        "refdo-main-refresh-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let database = directory.join("data.db");
    let store = TodoStore::open(&database).unwrap();
    let mut app = app_with_sections(vec![section("main")]);
    app.store = store;
    app.data_version = app.store.data_version().unwrap();
    let mut other = TodoStore::open(&database).unwrap();

    other
        .insert_todo("refs/heads/stored-only", "from another process", None)
        .unwrap();
    app.refresh_external();

    assert_eq!(app.todos[0].title, "from another process");
    assert!(app.repository.sections.iter().any(|section| {
        section.full_ref_name == "refs/heads/stored-only" && section.is_stored_only
    }));
    drop(other);
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn refresh_retries_an_unknown_snapshot_without_a_new_commit() {
    let directory = std::env::temp_dir().join(format!(
        "refdo-main-unknown-version-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let database = directory.join("data.db");
    let mut writer = TodoStore::open(&database).unwrap();
    writer
        .insert_todo("refs/heads/main", "already committed", None)
        .unwrap();
    let store = TodoStore::open(&database).unwrap();
    let mut app = app_with_sections(vec![section("main")]);
    app.store = store;
    app.data_version = UNKNOWN_DATA_VERSION;

    app.refresh_external();

    assert_eq!(app.todos[0].title, "already committed");
    drop(writer);
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn reload_removes_stored_only_section_when_external_process_deletes_its_todos() {
    let directory = std::env::temp_dir().join(format!(
        "refdo-reload-remove-stored-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let database = directory.join("data.db");
    let store = TodoStore::open(&database).unwrap();
    let mut app = app_with_sections(vec![section("main")]);
    app.store = store;
    let mut other = TodoStore::open(&database).unwrap();
    let todo = other
        .insert_todo("refs/heads/stored-only", "to be deleted", None)
        .unwrap();
    assert!(app.reload());
    assert_eq!(app.repository.sections.len(), 2);
    assert!(app.repository.sections[1].is_stored_only);

    other.delete_todo(todo.id).unwrap();
    assert!(app.reload());
    assert_eq!(app.repository.sections.len(), 1);
    assert_eq!(app.repository.sections[0].display_name, "main");
    assert!(app.todos.is_empty());

    drop(other);
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn startup_discovery_failure_uses_in_memory_store_and_sets_repository_error() {
    let calls = Cell::new(0);
    let mut app =
        App::new_with_repository_discovery(test_theme(), DispatchController::default(), || {
            calls.set(calls.get() + 1);
            Err("injected discovery failure".to_owned())
        });

    assert_eq!(
        app.repository_error.as_deref(),
        Some("repository: injected discovery failure")
    );
    assert_eq!(app.error, None);
    assert!(!app.persistence_available);
    assert!(app.repository.common_git_dir.as_os_str().is_empty());
    assert!(app.repository.sections.is_empty());
    assert_eq!(app.repository.head_label, "unknown");

    let refresh_calls = Cell::new(0);
    app.refresh_repository_with(Instant::now() + Duration::from_secs(5), || {
        refresh_calls.set(refresh_calls.get() + 1);
        Ok(RepositoryContext::default())
    });
    assert_eq!(refresh_calls.get(), 0);
}

#[test]
fn footer_rendering_precedence_confirmation_prompt_over_repository_error_over_generic_error() {
    let mut app = app_with_sections(vec![section("main")]);
    app.repository_error = Some("repository: temporary failure".to_owned());
    app.error = Some("generic command error".to_owned());

    let backend = TestBackend::new(80, 8);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|frame| app.draw(frame)).unwrap();
    let footer = row_text(&terminal, 7);
    assert!(footer.contains("repository: temporary failure"));
    assert!(!footer.contains("generic command error"));

    app.repository_error = None;
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let footer = row_text(&terminal, 7);
    assert!(footer.contains("generic command error"));

    app.repository_error = Some("repository: temporary failure".to_owned());
    app.mode = Mode::ConfirmClear(ClearConfirmation {
        target_branch: "refs/heads/main".to_owned(),
        prompt: "clear: remove all items from main? [y/N]".to_owned(),
    });
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let footer = row_text(&terminal, 7);
    assert!(footer.contains("clear: remove all items from main? [y/N]"));
    assert!(!footer.contains("repository: temporary failure"));
}

#[test]
fn periodic_repository_refresh_replaces_state_converts_removed_worktree_and_repairs_focus() {
    let common_dir = PathBuf::from("/test/repo/.git");
    let mut app = app_with_sections(vec![
        section("main"),
        section("feature"),
        section("ephemeral"),
    ]);
    app.repository.common_git_dir = common_dir.clone();
    let feature_todo = app
        .store
        .insert_todo("refs/heads/feature", "feature task", None)
        .unwrap();
    app.todos = vec![feature_todo];
    app.focus = Some(Focus::Branch("refs/heads/ephemeral".to_owned()));

    let started = Instant::now();
    app.last_repository_refresh = started;

    let discovered_context = RepositoryContext {
        head_label: "feature".to_owned(),
        common_git_dir: common_dir,
        sections: vec![section("main"), section("new-live")],
    };

    app.refresh_repository_with(started + Duration::from_secs(1), || Ok(discovered_context));

    assert_eq!(app.repository.head_label, "feature");
    assert_eq!(app.repository.sections.len(), 3);
    assert_eq!(app.repository.sections[0].display_name, "main");
    assert!(!app.repository.sections[0].is_stored_only);
    assert_eq!(app.repository.sections[1].display_name, "new-live");
    assert!(!app.repository.sections[1].is_stored_only);
    assert_eq!(app.repository.sections[2].display_name, "feature");
    assert!(app.repository.sections[2].is_stored_only);
    assert_eq!(app.repository_error, None);
    assert!(app.focus.is_some());
    assert_ne!(
        app.focus,
        Some(Focus::Branch("refs/heads/ephemeral".to_owned()))
    );
}

#[test]
fn discovery_failure_and_common_dir_mismatch_retain_state_and_later_success_clears_error() {
    let common_dir = PathBuf::from("/test/repo/.git");
    let mut app = app_with_sections(vec![section("main")]);
    app.repository.common_git_dir = common_dir.clone();
    let todo = app
        .store
        .insert_todo("refs/heads/main", "task", None)
        .unwrap();
    app.todos = vec![todo.clone()];
    app.focus = Some(Focus::Todo(todo.id));
    let original_version = app.data_version;

    let started = Instant::now();
    app.last_repository_refresh = started;
    let calls = Cell::new(0);

    // 1. Injected discovery failure at 1s consumes the interval
    app.refresh_repository_with(started + Duration::from_secs(1), || {
        calls.set(calls.get() + 1);
        Err("temporary disk error".to_owned())
    });
    assert_eq!(calls.get(), 1);
    assert_eq!(
        app.repository_error.as_deref(),
        Some("repository: temporary disk error")
    );
    assert_eq!(app.repository.head_label, "main");
    assert_eq!(app.repository.sections.len(), 1);
    assert_eq!(app.todos.len(), 1);
    assert_eq!(app.focus, Some(Focus::Todo(todo.id)));
    assert_eq!(app.data_version, original_version);
    assert!(app.persistence_available);

    // Sub-interval attempt at 1.5s is not invoked
    app.refresh_repository_with(started + Duration::from_millis(1500), || {
        calls.set(calls.get() + 1);
        Ok(RepositoryContext::default())
    });
    assert_eq!(calls.get(), 1);
    assert_eq!(
        app.repository_error.as_deref(),
        Some("repository: temporary disk error")
    );

    // 2. Mismatched common_git_dir at 2s consumes the interval
    app.refresh_repository_with(started + Duration::from_secs(2), || {
        calls.set(calls.get() + 1);
        Ok(RepositoryContext {
            head_label: "other".to_owned(),
            common_git_dir: PathBuf::from("/different/repo/.git"),
            sections: vec![section("other")],
        })
    });
    assert_eq!(calls.get(), 2);
    assert_eq!(
        app.repository_error.as_deref(),
        Some("repository: discovered repository has a different common Git directory")
    );
    assert_eq!(app.repository.head_label, "main");
    assert_eq!(app.repository.sections.len(), 1);
    assert_eq!(app.todos.len(), 1);
    assert_eq!(app.focus, Some(Focus::Todo(todo.id)));

    // Sub-interval attempt at 2.5s is not invoked
    app.refresh_repository_with(started + Duration::from_millis(2500), || {
        calls.set(calls.get() + 1);
        Ok(RepositoryContext::default())
    });
    assert_eq!(calls.get(), 2);
    assert_eq!(
        app.repository_error.as_deref(),
        Some("repository: discovered repository has a different common Git directory")
    );

    // 3. Later eligible success at 3s clears error and applies state
    app.refresh_repository_with(started + Duration::from_secs(3), || {
        calls.set(calls.get() + 1);
        Ok(RepositoryContext {
            head_label: "main-updated".to_owned(),
            common_git_dir: common_dir,
            sections: vec![section("main")],
        })
    });
    assert_eq!(calls.get(), 3);
    assert_eq!(app.repository_error, None);
    assert_eq!(app.repository.head_label, "main-updated");
}

#[test]
fn refresh_repository_enforces_interval_and_guards_empty_common_dir() {
    let common_dir = PathBuf::from("/test/repo/.git");
    let mut app = app_with_sections(vec![section("main")]);
    app.repository.common_git_dir = common_dir.clone();

    let started = Instant::now();
    app.last_repository_refresh = started;

    let calls = Cell::new(0);

    app.refresh_repository_with(started + Duration::from_millis(999), || {
        calls.set(calls.get() + 1);
        Ok(RepositoryContext {
            head_label: "main".to_owned(),
            common_git_dir: common_dir.clone(),
            sections: vec![section("main")],
        })
    });
    assert_eq!(calls.get(), 0);

    app.refresh_repository_with(started + Duration::from_secs(1), || {
        calls.set(calls.get() + 1);
        Ok(RepositoryContext {
            head_label: "main".to_owned(),
            common_git_dir: common_dir.clone(),
            sections: vec![section("main")],
        })
    });
    assert_eq!(calls.get(), 1);

    app.repository.common_git_dir = PathBuf::new();
    app.refresh_repository_with(started + Duration::from_secs(10), || {
        calls.set(calls.get() + 1);
        Ok(RepositoryContext::default())
    });
    assert_eq!(calls.get(), 1);
}

#[test]
fn refresh_select_mode_prunes_deleted_ids_and_preserves_empty_selection_when_todos_remain() {
    let directory = std::env::temp_dir().join(format!(
        "refdo-select-prune-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let database = directory.join("data.db");
    let store = TodoStore::open(&database).unwrap();
    let mut other = TodoStore::open(&database).unwrap();
    let t1 = other.insert_todo("refs/heads/main", "first", None).unwrap();
    let t2 = other
        .insert_todo("refs/heads/main", "second", Some(t1.id))
        .unwrap();

    let mut app = app_with_sections(vec![section("main")]);
    app.store = store;
    app.data_version = app.store.data_version().unwrap();
    app.reload();

    app.focus = Some(Focus::Todo(t1.id));
    app.handle_key_event(key(KeyCode::Char('v')));

    // External process deletes the selected and focused todo t1
    other.delete_todo(t1.id).unwrap();
    app.refresh_external();

    let Mode::Select(select_state) = &app.mode else {
        panic!("expected select mode to remain active while section has todos");
    };
    assert!(select_state.selected_todo_ids.is_empty());
    assert_eq!(app.focus, Some(Focus::Todo(t2.id)));

    drop(other);
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn refresh_select_mode_exits_to_normal_mode_when_section_has_no_remaining_todos() {
    let directory = std::env::temp_dir().join(format!(
        "refdo-select-exit-empty-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let database = directory.join("data.db");
    let store = TodoStore::open(&database).unwrap();
    let mut other = TodoStore::open(&database).unwrap();
    let t1 = other
        .insert_todo("refs/heads/topic", "sole todo", None)
        .unwrap();

    let mut app = app_with_sections(vec![section("main"), section("topic")]);
    app.store = store;
    app.data_version = app.store.data_version().unwrap();
    app.reload();

    app.focus = Some(Focus::Todo(t1.id));
    app.handle_key_event(key(KeyCode::Char('v')));

    // External process deletes the only todo in the branch
    other.delete_todo(t1.id).unwrap();
    app.refresh_external();

    assert!(matches!(&app.mode, Mode::Normal));
    assert_eq!(
        app.focus,
        Some(Focus::Branch("refs/heads/topic".to_owned()))
    );

    drop(other);
    drop(app);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn refresh_preserves_repository_error_over_generic_error_in_select_mode() {
    let mut app = app_with_sections(vec![section("main")]);
    app.repository.common_git_dir = PathBuf::from("/test/repo/.git");
    let t1 = app
        .store
        .insert_todo("refs/heads/main", "main todo", None)
        .unwrap();
    app.reload();

    app.focus = Some(Focus::Todo(t1.id));
    app.handle_key_event(key(KeyCode::Char('v')));

    app.error = Some("generic database error".to_owned());

    let started = Instant::now();
    app.last_repository_refresh = started;
    app.refresh_repository_with(started + Duration::from_secs(2), || {
        Err("detached worktree".to_owned())
    });

    assert_eq!(
        app.repository_error.as_deref(),
        Some("repository: detached worktree")
    );
    assert_eq!(app.error.as_deref(), Some("generic database error"));

    let backend = TestBackend::new(60, 6);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();

    let footer = row_text(&terminal, 5);
    assert!(
        footer.starts_with(" SELECT · 1 selected repository: detached worktree"),
        "unexpected footer: {footer}"
    );
}
