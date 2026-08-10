use std::{error::Error, fmt, fs, path::Path, time::Duration};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

pub type TodoId = i64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Todo {
    pub id: TodoId,
    pub branch_ref: String,
    pub title: String,
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
            "SELECT id, branch_ref, title, completed, sort_order
             FROM todos
             ORDER BY branch_ref, sort_order, id",
        )?;
        let todos = statement
            .query_map([], |row| {
                Ok(Todo {
                    id: row.get(0)?,
                    branch_ref: row.get(1)?,
                    title: row.get(2)?,
                    completed: row.get(3)?,
                    sort_order: row.get(4)?,
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
            "INSERT INTO todos (branch_ref, title, sort_order) VALUES (?1, ?2, ?3)",
            params![branch_ref, title, sort_order],
        )?;
        let id = transaction.last_insert_rowid();
        transaction.commit()?;

        Ok(Todo {
            id,
            branch_ref: branch_ref.to_owned(),
            title: title.to_owned(),
            completed: false,
            sort_order,
        })
    }

    pub fn update_todo_title(&mut self, id: TodoId, title: &str) -> Result<Todo, StoreError> {
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
                 SET title = ?1
                 WHERE id = ?2
                 RETURNING id, branch_ref, title, completed, sort_order",
                params![title, id],
                |row| {
                    Ok(Todo {
                        id: row.get(0)?,
                        branch_ref: row.get(1)?,
                        title: row.get(2)?,
                        completed: row.get(3)?,
                        sort_order: row.get(4)?,
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
                 RETURNING id, branch_ref, title, completed, sort_order",
                [id],
                |row| {
                    Ok(Todo {
                        id: row.get(0)?,
                        branch_ref: row.get(1)?,
                        title: row.get(2)?,
                        completed: row.get(3)?,
                        sort_order: row.get(4)?,
                    })
                },
            )
            .optional()?
            .ok_or(StoreError::TodoNotFound(id))?;
        transaction.commit()?;
        Ok(todo)
    }

    pub fn data_version(&self) -> Result<i64, StoreError> {
        Ok(self
            .connection
            .query_row("PRAGMA data_version", [], |row| row.get(0))?)
    }
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
    const SCHEMA_VERSION: i64 = 1;

    let version = connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
    match version {
        SCHEMA_VERSION => return Ok(()),
        0 => {}
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
                    completed INTEGER NOT NULL DEFAULT 0 CHECK (completed IN (0, 1)),
                    sort_order INTEGER NOT NULL CHECK (sort_order >= 0),
                    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                    UNIQUE (branch_ref, sort_order)
                );
                PRAGMA user_version = 1;",
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

    use super::{StoreError, TodoStore};

    static NEXT_DATABASE: AtomicU64 = AtomicU64::new(0);

    struct TemporaryDatabase {
        path: PathBuf,
    }

    impl TemporaryDatabase {
        fn new() -> Self {
            let sequence = NEXT_DATABASE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("tuido-storage-{}-{sequence}", process::id()))
                .join("nested")
                .join("todos.db");
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
            .insert_todo("refs/heads/feature", "old title", Some(first.id))
            .unwrap();
        let completed = store.toggle_todo(target.id).unwrap();

        let updated = store
            .update_todo_title(completed.id, "  new title \t")
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
            store.update_todo_title(inserted.id, " \t\n "),
            Err(StoreError::EmptyTitle)
        ));
        assert_eq!(store.load_all().unwrap(), vec![inserted]);
    }

    #[test]
    fn updating_a_missing_todo_returns_not_found() {
        let mut store = TodoStore::open_in_memory().unwrap();

        assert!(matches!(
            store.update_todo_title(42, "new title"),
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
