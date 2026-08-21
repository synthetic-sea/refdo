use std::{error::Error, fmt, fs, path::Path, time::Duration};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

pub type TodoId = i64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Todo {
    pub id: TodoId,
    pub branch_ref: String,
    pub title: String,
    pub body: String,
    pub completed: bool,
    pub sort_order: i64,
}

#[derive(Debug)]
pub enum StoreError {
    Database(rusqlite::Error),
    CreateDirectory(std::io::Error),
    EmptyTitle,
    TodoNotFound(TodoId),
    AnchorBranchMismatch {
        id: TodoId,
        expected_branch_ref: String,
        actual_branch_ref: String,
    },
    OrderingOverflow,
    UnsupportedSchemaVersion(i64),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(f, "todo database error: {error}"),
            Self::CreateDirectory(error) => {
                write!(f, "failed to create todo database directory: {error}")
            }
            Self::EmptyTitle => f.write_str("todo title must not be empty"),
            Self::TodoNotFound(id) => write!(f, "todo {id} does not exist"),
            Self::AnchorBranchMismatch {
                id,
                expected_branch_ref,
                actual_branch_ref,
            } => write!(
                f,
                "todo anchor {id} belongs to {actual_branch_ref:?}, not {expected_branch_ref:?}",
            ),
            Self::OrderingOverflow => f.write_str("todo ordering exceeds the supported range"),
            Self::UnsupportedSchemaVersion(version) => {
                write!(f, "unsupported todo database schema version {version}")
            }
        }
    }
}

impl Error for StoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::CreateDirectory(error) => Some(error),
            Self::EmptyTitle
            | Self::TodoNotFound(_)
            | Self::AnchorBranchMismatch { .. }
            | Self::OrderingOverflow
            | Self::UnsupportedSchemaVersion(_) => None,
        }
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

pub struct TodoStore {
    connection: Connection,
}

impl TodoStore {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(StoreError::CreateDirectory)?;
        }

        let mut connection = Connection::open(path)?;
        configure_connection(&connection, true)?;
        migrate(&mut connection)?;
        Ok(Self { connection })
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        let mut connection = Connection::open_in_memory()?;
        configure_connection(&connection, false)?;
        migrate(&mut connection)?;
        Ok(Self { connection })
    }

    pub fn load_all(&self) -> Result<Vec<Todo>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT id, branch_ref, title, body, completed, sort_order
             FROM todos
             ORDER BY branch_ref, sort_order, id",
        )?;
        let todos = statement
            .query_map([], |row| {
                Ok(Todo {
                    id: row.get(0)?,
                    branch_ref: row.get(1)?,
                    title: row.get(2)?,
                    body: row.get(3)?,
                    completed: row.get(4)?,
                    sort_order: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(todos)
    }

    pub fn insert_todo(
        &mut self,
        branch_ref: &str,
        title: &str,
        after: Option<TodoId>,
    ) -> Result<Todo, StoreError> {
        self.insert_todo_with_completion(branch_ref, title, "", false, after)
    }

    pub fn insert_todo_with_completion(
        &mut self,
        branch_ref: &str,
        title: &str,
        body: &str,
        completed: bool,
        after: Option<TodoId>,
    ) -> Result<Todo, StoreError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(StoreError::EmptyTitle);
        }

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let maximum_order = transaction.query_row(
            "SELECT COALESCE(MAX(sort_order), -1) FROM todos WHERE branch_ref = ?1",
            [branch_ref],
            |row| row.get::<_, i64>(0),
        )?;

        let sort_order = match after {
            None => 0,
            Some(anchor_id) => {
                let anchor = transaction
                    .query_row(
                        "SELECT branch_ref, sort_order FROM todos WHERE id = ?1",
                        [anchor_id],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                    )
                    .optional()?
                    .ok_or(StoreError::TodoNotFound(anchor_id))?;
                if anchor.0 != branch_ref {
                    return Err(StoreError::AnchorBranchMismatch {
                        id: anchor_id,
                        expected_branch_ref: branch_ref.to_owned(),
                        actual_branch_ref: anchor.0,
                    });
                }
                anchor
                    .1
                    .checked_add(1)
                    .ok_or(StoreError::OrderingOverflow)?
            }
        };
        if sort_order <= maximum_order {
            let offset = maximum_order
                .checked_add(2)
                .ok_or(StoreError::OrderingOverflow)?;
            maximum_order
                .checked_add(offset)
                .ok_or(StoreError::OrderingOverflow)?;
            transaction.execute(
                "UPDATE todos
                 SET sort_order = sort_order + ?1
                 WHERE branch_ref = ?2 AND sort_order >= ?3",
                params![offset, branch_ref, sort_order],
            )?;
            transaction.execute(
                "UPDATE todos
                 SET sort_order = sort_order - ?1 + 1
                 WHERE branch_ref = ?2 AND sort_order >= ?1",
                params![offset, branch_ref],
            )?;
        }

        transaction.execute(
            "INSERT INTO todos (branch_ref, title, body, completed, sort_order)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![branch_ref, title, body, completed, sort_order],
        )?;
        let id = transaction.last_insert_rowid();
        transaction.commit()?;

        Ok(Todo {
            id,
            branch_ref: branch_ref.to_owned(),
            title: title.to_owned(),
            body: body.to_owned(),
            completed,
            sort_order,
        })
    }

    pub fn delete_todo(&mut self, id: TodoId) -> Result<Todo, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let todo = transaction
            .query_row(
                "DELETE FROM todos
                 WHERE id = ?1
                 RETURNING id, branch_ref, title, body, completed, sort_order",
                [id],
                |row| {
                    Ok(Todo {
                        id: row.get(0)?,
                        branch_ref: row.get(1)?,
                        title: row.get(2)?,
                        body: row.get(3)?,
                        completed: row.get(4)?,
                        sort_order: row.get(5)?,
                    })
                },
            )
            .optional()?
            .ok_or(StoreError::TodoNotFound(id))?;
        transaction.commit()?;
        Ok(todo)
    }

    pub fn delete_completed_todos(&mut self, branch_ref: &str) -> Result<usize, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let deleted = transaction.execute(
            "DELETE FROM todos WHERE branch_ref = ?1 AND completed = 1",
            [branch_ref],
        )?;
        transaction.commit()?;
        Ok(deleted)
    }

    pub fn delete_all_todos(&mut self, branch_ref: &str) -> Result<usize, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let deleted =
            transaction.execute("DELETE FROM todos WHERE branch_ref = ?1", [branch_ref])?;
        transaction.commit()?;
        Ok(deleted)
    }

    pub fn sort_todos(&mut self, branch_ref: &str) -> Result<usize, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ordered_todos = {
            let mut statement = transaction.prepare(
                "SELECT id, sort_order
                 FROM todos
                 WHERE branch_ref = ?1
                 ORDER BY completed ASC, created_at ASC, id ASC",
            )?;
            let rows = statement.query_map([branch_ref], |row| {
                Ok((row.get::<_, TodoId>(0)?, row.get::<_, i64>(1)?))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        write_todo_order(&transaction, &ordered_todos)?;
        transaction.commit()?;
        Ok(ordered_todos.len())
    }

    pub fn group_todos(&mut self, branch_ref: &str) -> Result<usize, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ordered_todos = {
            let mut statement = transaction.prepare(
                "SELECT id, sort_order
                 FROM todos
                 WHERE branch_ref = ?1
                 ORDER BY completed ASC, sort_order ASC, id ASC",
            )?;
            let rows = statement.query_map([branch_ref], |row| {
                Ok((row.get::<_, TodoId>(0)?, row.get::<_, i64>(1)?))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        write_todo_order(&transaction, &ordered_todos)?;
        transaction.commit()?;
        Ok(ordered_todos.len())
    }

    pub fn update_todo(&mut self, id: TodoId, title: &str, body: &str) -> Result<Todo, StoreError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(StoreError::EmptyTitle);
        }

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let todo = transaction
            .query_row(
                "UPDATE todos
                 SET title = ?1, body = ?2
                 WHERE id = ?3
                 RETURNING id, branch_ref, title, body, completed, sort_order",
                params![title, body, id],
                |row| {
                    Ok(Todo {
                        id: row.get(0)?,
                        branch_ref: row.get(1)?,
                        title: row.get(2)?,
                        body: row.get(3)?,
                        completed: row.get(4)?,
                        sort_order: row.get(5)?,
                    })
                },
            )
            .optional()?
            .ok_or(StoreError::TodoNotFound(id))?;
        transaction.commit()?;
        Ok(todo)
    }

    pub fn toggle_todo(&mut self, id: TodoId) -> Result<Todo, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let todo = transaction
            .query_row(
                "UPDATE todos
                 SET completed = 1 - completed
                 WHERE id = ?1
                 RETURNING id, branch_ref, title, body, completed, sort_order",
                [id],
                |row| {
                    Ok(Todo {
                        id: row.get(0)?,
                        branch_ref: row.get(1)?,
                        title: row.get(2)?,
                        body: row.get(3)?,
                        completed: row.get(4)?,
                        sort_order: row.get(5)?,
                    })
                },
            )
            .optional()?
            .ok_or(StoreError::TodoNotFound(id))?;
        transaction.commit()?;
        Ok(todo)
    }
    pub fn is_dispatch_config_trusted(&self, digest: &[u8; 32]) -> Result<bool, StoreError> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM trusted_dispatch_configs WHERE digest = ?1
            )",
            [digest.as_slice()],
            |row| row.get(0),
        )?)
    }

    pub fn trust_dispatch_config(&self, digest: &[u8; 32]) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT OR IGNORE INTO trusted_dispatch_configs (digest) VALUES (?1)",
            [digest.as_slice()],
        )?;
        Ok(())
    }

    pub fn data_version(&self) -> Result<i64, StoreError> {
        Ok(self
            .connection
            .query_row("PRAGMA data_version", [], |row| row.get(0))?)
    }
}

fn write_todo_order(
    transaction: &rusqlite::Transaction<'_>,
    ordered_todos: &[(TodoId, i64)],
) -> Result<(), StoreError> {
    let Some(maximum_order) = ordered_todos
        .iter()
        .map(|(_, sort_order)| *sort_order)
        .max()
    else {
        return Ok(());
    };
    let temporary_start = maximum_order
        .checked_add(1)
        .ok_or(StoreError::OrderingOverflow)?;
    for (index, (id, _)) in ordered_todos.iter().enumerate() {
        let index = i64::try_from(index).map_err(|_| StoreError::OrderingOverflow)?;
        let temporary_order = temporary_start
            .checked_add(index)
            .ok_or(StoreError::OrderingOverflow)?;
        transaction.execute(
            "UPDATE todos SET sort_order = ?1 WHERE id = ?2",
            params![temporary_order, id],
        )?;
    }

    for (sort_order, (id, _)) in ordered_todos.iter().enumerate() {
        let sort_order = i64::try_from(sort_order).map_err(|_| StoreError::OrderingOverflow)?;
        transaction.execute(
            "UPDATE todos SET sort_order = ?1 WHERE id = ?2",
            params![sort_order, id],
        )?;
    }
    Ok(())
}

fn configure_connection(connection: &Connection, file_backed: bool) -> Result<(), StoreError> {
    connection.busy_timeout(Duration::from_secs(1))?;
    connection.pragma_update(None, "foreign_keys", true)?;
    if file_backed {
        connection.pragma_update(None, "journal_mode", "WAL")?;
    }
    Ok(())
}

fn migrate(connection: &mut Connection) -> Result<(), StoreError> {
    const SCHEMA_VERSION: i64 = 3;

    let version = connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
    match version {
        SCHEMA_VERSION => return Ok(()),
        0..=2 => {}
        version => return Err(StoreError::UnsupportedSchemaVersion(version)),
    }

    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let locked_version =
        transaction.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
    match locked_version {
        SCHEMA_VERSION => {}
        0 => {
            transaction.execute_batch(
                "CREATE TABLE todos (
                    id INTEGER PRIMARY KEY,
                    branch_ref TEXT NOT NULL,
                    title TEXT NOT NULL CHECK (length(title) > 0 AND title = trim(title)),
                    body TEXT NOT NULL DEFAULT '',
                    completed INTEGER NOT NULL DEFAULT 0 CHECK (completed IN (0, 1)),
                    sort_order INTEGER NOT NULL CHECK (sort_order >= 0),
                    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                    UNIQUE (branch_ref, sort_order)
                );
                CREATE TABLE trusted_dispatch_configs (
                    digest BLOB PRIMARY KEY CHECK (length(digest) = 32),
                    trusted_at INTEGER NOT NULL DEFAULT (unixepoch())
                );
                PRAGMA user_version = 3;",
            )?;
        }
        1 => {
            transaction.execute_batch(
                "CREATE TABLE trusted_dispatch_configs (
                    digest BLOB PRIMARY KEY CHECK (length(digest) = 32),
                    trusted_at INTEGER NOT NULL DEFAULT (unixepoch())
                );
                ALTER TABLE todos ADD COLUMN body TEXT NOT NULL DEFAULT '';
                PRAGMA user_version = 3;",
            )?;
        }
        2 => {
            transaction.execute_batch(
                "ALTER TABLE todos ADD COLUMN body TEXT NOT NULL DEFAULT '';
                PRAGMA user_version = 3;",
            )?;
        }
        version => return Err(StoreError::UnsupportedSchemaVersion(version)),
    }
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        process,
        sync::{
            Arc, Barrier,
            atomic::{AtomicU64, Ordering},
        },
        thread,
    };

    use rusqlite::params;

    use super::{StoreError, TodoStore};

    static NEXT_DATABASE: AtomicU64 = AtomicU64::new(0);

    struct TemporaryDatabase {
        path: PathBuf,
    }

    impl TemporaryDatabase {
        fn new() -> Self {
            let sequence = NEXT_DATABASE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("refdo-storage-{}-{sequence}", process::id()))
                .join("nested")
                .join("data.db");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TemporaryDatabase {
        fn drop(&mut self) {
            if let Some(root) = self.path.parent().and_then(Path::parent) {
                let _ = fs::remove_dir_all(root);
            }
        }
    }

    #[test]
    fn inserts_first_and_after_stable_id() {
        let mut store = TodoStore::open_in_memory().unwrap();
        let second = store
            .insert_todo("refs/heads/main", "second", None)
            .unwrap();
        let third = store
            .insert_todo("refs/heads/main", "third", Some(second.id))
            .unwrap();
        let first = store.insert_todo("refs/heads/main", "first", None).unwrap();
        store
            .insert_todo("refs/heads/other", "other", None)
            .unwrap();

        let todos = store.load_all().unwrap();
        assert_eq!(
            todos
                .iter()
                .map(|todo| (
                    todo.branch_ref.as_str(),
                    todo.title.as_str(),
                    todo.sort_order
                ))
                .collect::<Vec<_>>(),
            vec![
                ("refs/heads/main", "first", 0),
                ("refs/heads/main", "second", 1),
                ("refs/heads/main", "third", 2),
                ("refs/heads/other", "other", 0),
            ]
        );
        assert_eq!(todos[0].id, first.id);
        assert_eq!(todos[1].id, second.id);
        assert_eq!(todos[2].id, third.id);
    }

    #[test]
    fn inserts_with_completion_after_the_requested_todo() {
        let mut store = TodoStore::open_in_memory().unwrap();
        let first = store.insert_todo("refs/heads/main", "first", None).unwrap();
        let third = store
            .insert_todo("refs/heads/main", "third", Some(first.id))
            .unwrap();

        let second = store
            .insert_todo_with_completion("refs/heads/main", "second", "", false, Some(first.id))
            .unwrap();

        assert_eq!(
            store
                .load_all()
                .unwrap()
                .iter()
                .map(|todo| todo.id)
                .collect::<Vec<_>>(),
            vec![first.id, second.id, third.id]
        );
    }

    #[test]
    fn inserts_with_the_requested_completion_state() {
        let mut store = TodoStore::open_in_memory().unwrap();

        let completed = store
            .insert_todo_with_completion("refs/heads/main", "done", "", true, None)
            .unwrap();

        assert!(completed.completed);
        assert_eq!(store.load_all().unwrap(), vec![completed]);
    }

    #[test]
    fn deletes_and_returns_the_full_todo() {
        let mut store = TodoStore::open_in_memory().unwrap();
        let first = store.insert_todo("refs/heads/main", "first", None).unwrap();
        let completed = store
            .insert_todo_with_completion("refs/heads/main", "done", "", true, Some(first.id))
            .unwrap();

        let deleted = store.delete_todo(completed.id).unwrap();

        assert_eq!(deleted, completed);
        assert_eq!(store.load_all().unwrap(), vec![first]);
    }

    #[test]
    fn deletes_only_completed_todos_in_the_requested_branch() {
        let mut store = TodoStore::open_in_memory().unwrap();
        let main_incomplete = store
            .insert_todo("refs/heads/main", "main incomplete", None)
            .unwrap();
        let main_completed_first = store
            .insert_todo_with_completion(
                "refs/heads/main",
                "main completed first",
                "",
                true,
                Some(main_incomplete.id),
            )
            .unwrap();
        let _main_completed_second = store
            .insert_todo_with_completion(
                "refs/heads/main",
                "main completed second",
                "",
                true,
                Some(main_completed_first.id),
            )
            .unwrap();
        let feature_completed = store
            .insert_todo_with_completion("refs/heads/feature", "feature completed", "", true, None)
            .unwrap();
        let feature_incomplete = store
            .insert_todo(
                "refs/heads/feature",
                "feature incomplete",
                Some(feature_completed.id),
            )
            .unwrap();
        let expected = vec![feature_completed, feature_incomplete, main_incomplete];

        let deleted = store.delete_completed_todos("refs/heads/main").unwrap();

        assert_eq!(deleted, 2);
        assert_eq!(store.load_all().unwrap(), expected);
        assert_eq!(store.delete_completed_todos("refs/heads/main").unwrap(), 0);
        assert_eq!(store.load_all().unwrap(), expected);
    }

    #[test]
    fn deletes_all_todos_only_in_the_requested_branch() {
        let mut store = TodoStore::open_in_memory().unwrap();
        store
            .insert_todo("refs/heads/main", "main incomplete", None)
            .unwrap();
        store
            .insert_todo_with_completion("refs/heads/main", "main completed", "", true, None)
            .unwrap();
        let feature_completed = store
            .insert_todo_with_completion("refs/heads/feature", "feature completed", "", true, None)
            .unwrap();
        let feature_incomplete = store
            .insert_todo(
                "refs/heads/feature",
                "feature incomplete",
                Some(feature_completed.id),
            )
            .unwrap();
        let expected = vec![feature_completed, feature_incomplete];

        let deleted = store.delete_all_todos("refs/heads/main").unwrap();

        assert_eq!(deleted, 2);
        assert_eq!(store.load_all().unwrap(), expected);
        assert_eq!(store.delete_all_todos("refs/heads/main").unwrap(), 0);
        assert_eq!(store.load_all().unwrap(), expected);
    }

    #[test]
    fn sorts_requested_branch_by_completion_creation_and_id() {
        let mut store = TodoStore::open_in_memory().unwrap();
        let completed_newer = store
            .insert_todo_with_completion("refs/heads/main", "completed newer", "", true, None)
            .unwrap();
        let incomplete_newer = store
            .insert_todo("refs/heads/main", "incomplete newer", None)
            .unwrap();
        let completed_older = store
            .insert_todo_with_completion("refs/heads/main", "completed older", "", true, None)
            .unwrap();
        let incomplete_older = store
            .insert_todo("refs/heads/main", "incomplete older", None)
            .unwrap();
        let equal_timestamp_first = store
            .insert_todo("refs/heads/main", "equal timestamp first", None)
            .unwrap();
        let equal_timestamp_second = store
            .insert_todo("refs/heads/main", "equal timestamp second", None)
            .unwrap();
        let other_first = store
            .insert_todo("refs/heads/other", "other first", None)
            .unwrap();
        let other_second = store
            .insert_todo("refs/heads/other", "other second", Some(other_first.id))
            .unwrap();
        store
            .connection
            .execute(
                "UPDATE todos
                 SET created_at = CASE id
                     WHEN ?1 THEN 40
                     WHEN ?2 THEN 30
                     WHEN ?3 THEN 5
                     WHEN ?4 THEN 10
                     WHEN ?5 THEN 20
                     WHEN ?6 THEN 20
                     ELSE created_at
                 END
                 WHERE branch_ref = 'refs/heads/main'",
                params![
                    completed_newer.id,
                    incomplete_newer.id,
                    completed_older.id,
                    incomplete_older.id,
                    equal_timestamp_first.id,
                    equal_timestamp_second.id,
                ],
            )
            .unwrap();
        let other_before = store
            .load_all()
            .unwrap()
            .into_iter()
            .filter(|todo| todo.branch_ref == "refs/heads/other")
            .collect::<Vec<_>>();

        assert_eq!(store.sort_todos("refs/heads/main").unwrap(), 6);

        let all = store.load_all().unwrap();
        let main = all
            .iter()
            .filter(|todo| todo.branch_ref == "refs/heads/main")
            .collect::<Vec<_>>();
        assert_eq!(
            main.iter().map(|todo| todo.id).collect::<Vec<_>>(),
            vec![
                incomplete_older.id,
                equal_timestamp_first.id,
                equal_timestamp_second.id,
                incomplete_newer.id,
                completed_older.id,
                completed_newer.id,
            ]
        );
        assert_eq!(
            main.iter().map(|todo| todo.sort_order).collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4, 5]
        );
        assert_eq!(
            all.into_iter()
                .filter(|todo| todo.branch_ref == "refs/heads/other")
                .collect::<Vec<_>>(),
            other_before
        );

        let once_sorted = store.load_all().unwrap();
        assert_eq!(store.sort_todos("refs/heads/main").unwrap(), 6);
        assert_eq!(store.load_all().unwrap(), once_sorted);
        assert_eq!(other_before, vec![other_first, other_second]);
    }

    #[test]
    fn sorting_an_empty_branch_returns_zero_without_modifying_data() {
        let mut store = TodoStore::open_in_memory().unwrap();
        let existing = store
            .insert_todo("refs/heads/main", "existing", None)
            .unwrap();

        assert_eq!(store.sort_todos("refs/heads/missing").unwrap(), 0);
        assert_eq!(store.load_all().unwrap(), vec![existing]);
    }

    #[test]
    fn groups_only_the_requested_branch_without_reordering_within_completion_states() {
        let mut store = TodoStore::open_in_memory().unwrap();
        let completed_first = store
            .insert_todo_with_completion("refs/heads/main", "completed first", "", true, None)
            .unwrap();
        let incomplete_first = store
            .insert_todo(
                "refs/heads/main",
                "incomplete first",
                Some(completed_first.id),
            )
            .unwrap();
        let completed_second = store
            .insert_todo_with_completion(
                "refs/heads/main",
                "completed second",
                "",
                true,
                Some(incomplete_first.id),
            )
            .unwrap();
        let incomplete_second = store
            .insert_todo(
                "refs/heads/main",
                "incomplete second",
                Some(completed_second.id),
            )
            .unwrap();
        let other_completed = store
            .insert_todo_with_completion("refs/heads/other", "other completed", "", true, None)
            .unwrap();
        let other_incomplete = store
            .insert_todo(
                "refs/heads/other",
                "other incomplete",
                Some(other_completed.id),
            )
            .unwrap();
        store
            .connection
            .execute(
                "UPDATE todos
                 SET created_at = CASE id
                     WHEN ?1 THEN 30
                     WHEN ?2 THEN 40
                     WHEN ?3 THEN 5
                     WHEN ?4 THEN 10
                     ELSE created_at
                 END
                 WHERE branch_ref = 'refs/heads/main'",
                params![
                    completed_first.id,
                    incomplete_first.id,
                    completed_second.id,
                    incomplete_second.id,
                ],
            )
            .unwrap();
        let other_before = store
            .load_all()
            .unwrap()
            .into_iter()
            .filter(|todo| todo.branch_ref == "refs/heads/other")
            .collect::<Vec<_>>();

        assert_eq!(store.group_todos("refs/heads/main").unwrap(), 4);

        let grouped = store.load_all().unwrap();
        let main = grouped
            .iter()
            .filter(|todo| todo.branch_ref == "refs/heads/main")
            .collect::<Vec<_>>();
        assert_eq!(
            main.iter().map(|todo| todo.id).collect::<Vec<_>>(),
            vec![
                incomplete_first.id,
                incomplete_second.id,
                completed_first.id,
                completed_second.id,
            ]
        );
        assert_eq!(
            main.iter().map(|todo| todo.sort_order).collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
        assert_eq!(
            grouped
                .iter()
                .filter(|todo| todo.branch_ref == "refs/heads/other")
                .cloned()
                .collect::<Vec<_>>(),
            other_before
        );

        assert_eq!(store.group_todos("refs/heads/main").unwrap(), 4);
        assert_eq!(store.load_all().unwrap(), grouped);
        assert_eq!(other_before, vec![other_completed, other_incomplete]);
    }

    #[test]
    fn grouping_an_empty_branch_returns_zero_without_modifying_data() {
        let mut store = TodoStore::open_in_memory().unwrap();
        let existing = store
            .insert_todo("refs/heads/main", "existing", None)
            .unwrap();

        assert_eq!(store.group_todos("refs/heads/missing").unwrap(), 0);
        assert_eq!(store.load_all().unwrap(), vec![existing]);
    }

    #[test]
    fn sorting_reports_ordering_overflow_without_modifying_data() {
        let mut store = TodoStore::open_in_memory().unwrap();
        let first = store.insert_todo("refs/heads/main", "first", None).unwrap();
        let second = store
            .insert_todo("refs/heads/main", "second", Some(first.id))
            .unwrap();
        store
            .connection
            .execute(
                "UPDATE todos SET sort_order = ?1 WHERE id = ?2",
                params![i64::MAX - 1, first.id],
            )
            .unwrap();
        let before = store.load_all().unwrap();

        assert!(matches!(
            store.sort_todos("refs/heads/main"),
            Err(StoreError::OrderingOverflow)
        ));
        assert_eq!(store.load_all().unwrap(), before);
        assert_eq!(
            before
                .iter()
                .map(|todo| (todo.id, todo.sort_order))
                .collect::<Vec<_>>(),
            vec![(second.id, 1), (first.id, i64::MAX - 1)]
        );
    }

    #[test]
    fn deleting_a_missing_todo_returns_not_found_without_modifying_data() {
        let mut store = TodoStore::open_in_memory().unwrap();
        let existing = store
            .insert_todo("refs/heads/main", "keep this", None)
            .unwrap();

        assert!(matches!(
            store.delete_todo(existing.id + 1),
            Err(StoreError::TodoNotFound(id)) if id == existing.id + 1
        ));
        assert_eq!(store.load_all().unwrap(), vec![existing]);
    }

    #[test]
    fn trims_titles_and_rejects_empty_input() {
        let mut store = TodoStore::open_in_memory().unwrap();
        let todo = store
            .insert_todo("refs/heads/main", "  keep this  ", None)
            .unwrap();
        assert_eq!(todo.title, "keep this");

        assert!(matches!(
            store.insert_todo("refs/heads/main", " \t\n ", None),
            Err(StoreError::EmptyTitle)
        ));
        assert_eq!(store.load_all().unwrap().len(), 1);
    }

    #[test]
    fn toggles_todo_completion_without_changing_other_fields() {
        let mut store = TodoStore::open_in_memory().unwrap();
        let inserted = store
            .insert_todo("refs/heads/feature", "persistent identity", None)
            .unwrap();

        let completed = store.toggle_todo(inserted.id).unwrap();
        assert_eq!(
            completed,
            super::Todo {
                completed: true,
                ..inserted.clone()
            }
        );
        assert_eq!(store.load_all().unwrap(), vec![completed]);

        let incomplete = store.toggle_todo(inserted.id).unwrap();
        assert_eq!(incomplete, inserted);
        assert_eq!(store.load_all().unwrap(), vec![incomplete]);
    }

    #[test]
    fn updates_a_trimmed_title_without_changing_other_fields() {
        let mut store = TodoStore::open_in_memory().unwrap();
        let first = store
            .insert_todo("refs/heads/feature", "first", None)
            .unwrap();
        let target = store
            .insert_todo_with_completion(
                "refs/heads/feature",
                "old title",
                "line one\n\nline three\n",
                false,
                Some(first.id),
            )
            .unwrap();
        let completed = store.toggle_todo(target.id).unwrap();

        let updated = store
            .update_todo(completed.id, "  new title \t", &completed.body)
            .unwrap();

        assert_eq!(
            updated,
            super::Todo {
                title: "new title".to_owned(),
                ..completed
            }
        );
        assert_eq!(store.load_all().unwrap(), vec![first, updated]);
    }

    #[test]
    fn rejects_an_empty_updated_title_without_changing_the_todo() {
        let mut store = TodoStore::open_in_memory().unwrap();
        let inserted = store
            .insert_todo("refs/heads/main", "keep this", None)
            .unwrap();

        assert!(matches!(
            store.update_todo(inserted.id, " \t\n ", &inserted.body),
            Err(StoreError::EmptyTitle)
        ));
        assert_eq!(store.load_all().unwrap(), vec![inserted]);
    }

    #[test]
    fn updating_a_missing_todo_returns_not_found() {
        let mut store = TodoStore::open_in_memory().unwrap();

        assert!(matches!(
            store.update_todo(42, "new title", "body"),
            Err(StoreError::TodoNotFound(42))
        ));
        assert!(store.load_all().unwrap().is_empty());
    }

    #[test]
    fn toggling_a_missing_todo_returns_not_found() {
        let mut store = TodoStore::open_in_memory().unwrap();

        assert!(matches!(
            store.toggle_todo(42),
            Err(StoreError::TodoNotFound(42))
        ));
        assert!(store.load_all().unwrap().is_empty());
    }

    #[test]
    fn persists_todos_in_a_new_parent_directory() {
        let database = TemporaryDatabase::new();
        let inserted = {
            let mut store = TodoStore::open(database.path()).unwrap();
            store
                .insert_todo("refs/heads/feature", "persistent", None)
                .unwrap()
        };

        let reopened = TodoStore::open(database.path()).unwrap();
        let todos = reopened.load_all().unwrap();
        assert_eq!(todos, vec![inserted]);
    }

    #[test]
    fn dispatch_config_trust_is_separated_by_digest() {
        let store = TodoStore::open_in_memory().unwrap();
        let trusted_digest = [0x11; 32];
        let other_digest = [0x22; 32];

        assert!(!store.is_dispatch_config_trusted(&trusted_digest).unwrap());
        assert!(!store.is_dispatch_config_trusted(&other_digest).unwrap());

        store.trust_dispatch_config(&trusted_digest).unwrap();

        assert!(store.is_dispatch_config_trusted(&trusted_digest).unwrap());
        assert!(!store.is_dispatch_config_trusted(&other_digest).unwrap());
    }

    #[test]
    fn trusting_a_dispatch_config_is_idempotent() {
        let store = TodoStore::open_in_memory().unwrap();
        let digest = [0x33; 32];

        store.trust_dispatch_config(&digest).unwrap();
        store.trust_dispatch_config(&digest).unwrap();

        let count = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM trusted_dispatch_configs WHERE digest = ?1",
                [digest.as_slice()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn dispatch_config_trust_persists_when_the_database_is_reopened() {
        let database = TemporaryDatabase::new();
        let digest = [0x44; 32];
        {
            let store = TodoStore::open(database.path()).unwrap();
            store.trust_dispatch_config(&digest).unwrap();
        }

        let reopened = TodoStore::open(database.path()).unwrap();
        assert!(reopened.is_dispatch_config_trusted(&digest).unwrap());
    }

    #[test]
    fn migrates_schema_v1_without_losing_todos() {
        let database = TemporaryDatabase::new();
        fs::create_dir_all(database.path().parent().unwrap()).unwrap();
        {
            let connection = rusqlite::Connection::open(database.path()).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE todos (
                        id INTEGER PRIMARY KEY,
                        branch_ref TEXT NOT NULL,
                        title TEXT NOT NULL CHECK (length(title) > 0 AND title = trim(title)),
                        completed INTEGER NOT NULL DEFAULT 0 CHECK (completed IN (0, 1)),
                        sort_order INTEGER NOT NULL CHECK (sort_order >= 0),
                        created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                        UNIQUE (branch_ref, sort_order)
                    );
                    INSERT INTO todos (id, branch_ref, title, completed, sort_order)
                    VALUES (17, 'refs/heads/legacy', 'preserved', 1, 0);
                    PRAGMA user_version = 1;",
                )
                .unwrap();
        }

        let store = TodoStore::open(database.path()).unwrap();

        let todos = store.load_all().unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].id, 17);
        assert_eq!(todos[0].branch_ref, "refs/heads/legacy");
        assert_eq!(todos[0].title, "preserved");
        assert!(todos[0].completed);
        assert_eq!(todos[0].body, "");
        assert_eq!(todos[0].sort_order, 0);
        assert!(!store.is_dispatch_config_trusted(&[0x55; 32]).unwrap());
        assert_eq!(
            store
                .connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            3
        );
    }

    #[test]
    fn migrates_schema_v2_and_reopens_with_a_multiline_body() {
        let database = TemporaryDatabase::new();
        fs::create_dir_all(database.path().parent().unwrap()).unwrap();
        {
            let connection = rusqlite::Connection::open(database.path()).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE todos (
                        id INTEGER PRIMARY KEY,
                        branch_ref TEXT NOT NULL,
                        title TEXT NOT NULL CHECK (length(title) > 0 AND title = trim(title)),
                        completed INTEGER NOT NULL DEFAULT 0 CHECK (completed IN (0, 1)),
                        sort_order INTEGER NOT NULL CHECK (sort_order >= 0),
                        created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                        UNIQUE (branch_ref, sort_order)
                    );
                    CREATE TABLE trusted_dispatch_configs (
                        digest BLOB PRIMARY KEY CHECK (length(digest) = 32),
                        trusted_at INTEGER NOT NULL DEFAULT (unixepoch())
                    );
                    INSERT INTO todos (id, branch_ref, title, completed, sort_order)
                    VALUES (23, 'refs/heads/legacy', 'preserved', 0, 0);
                    PRAGMA user_version = 2;",
                )
                .unwrap();
        }

        {
            let mut store = TodoStore::open(database.path()).unwrap();
            let mut todo = store.load_all().unwrap().pop().unwrap();
            assert_eq!(todo.body, "");
            todo = store
                .update_todo(todo.id, &todo.title, "first line\n\nlast line\n")
                .unwrap();
            assert_eq!(todo.body, "first line\n\nlast line\n");
            assert_eq!(
                store
                    .connection
                    .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                3
            );
        }

        let reopened = TodoStore::open(database.path()).unwrap();
        let todos = reopened.load_all().unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].id, 23);
        assert_eq!(todos[0].title, "preserved");
        assert_eq!(todos[0].body, "first line\n\nlast line\n");
    }

    #[test]
    fn sorted_orders_persist_when_the_database_is_reopened() {
        let database = TemporaryDatabase::new();
        let (incomplete_id, completed_id) = {
            let mut store = TodoStore::open(database.path()).unwrap();
            let completed = store
                .insert_todo_with_completion("refs/heads/main", "completed", "", true, None)
                .unwrap();
            let incomplete = store
                .insert_todo("refs/heads/main", "incomplete", Some(completed.id))
                .unwrap();

            assert_eq!(store.sort_todos("refs/heads/main").unwrap(), 2);
            (incomplete.id, completed.id)
        };

        let reopened = TodoStore::open(database.path()).unwrap();
        assert_eq!(
            reopened
                .load_all()
                .unwrap()
                .iter()
                .map(|todo| (todo.id, todo.sort_order))
                .collect::<Vec<_>>(),
            vec![(incomplete_id, 0), (completed_id, 1)]
        );
    }

    #[test]
    fn concurrent_first_opens_share_one_migration() {
        let database = TemporaryDatabase::new();
        let gate = Arc::new(Barrier::new(3));
        let handles = (0..2)
            .map(|_| {
                let path = database.path().to_path_buf();
                let gate = Arc::clone(&gate);
                thread::spawn(move || {
                    gate.wait();
                    TodoStore::open(&path)
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                })
            })
            .collect::<Vec<_>>();

        gate.wait();
        for handle in handles {
            handle.join().unwrap().unwrap();
        }
        assert!(TodoStore::open(database.path()).is_ok());
    }

    #[test]
    fn data_version_detects_another_connections_commit() {
        let database = TemporaryDatabase::new();
        let first = TodoStore::open(database.path()).unwrap();
        let mut second = TodoStore::open(database.path()).unwrap();
        let before = first.data_version().unwrap();

        second
            .insert_todo("refs/heads/main", "from another connection", None)
            .unwrap();

        assert_ne!(first.data_version().unwrap(), before);
        assert_eq!(first.load_all().unwrap().len(), 1);
    }
}
