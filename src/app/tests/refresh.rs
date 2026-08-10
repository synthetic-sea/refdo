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
