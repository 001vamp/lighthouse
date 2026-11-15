//! Durable thread and request storage on one dedicated SQLite worker thread.
//!
//! All filesystem and SQLite work, including opening and migrating the database,
//! happens on that worker. Clones share a bounded queue. Once a write is queued,
//! cancelling its caller does not cancel the write. Dropping the last handle
//! drains queued work and closes SQLite without blocking the async runtime.

#![deny(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod model;

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

pub use model::{
    AgentKind, Message, MessagePage, MessageRole, NewThread, RequestId, RequestInput, RequestState,
    SessionId, StoredRequest, Thread, ThreadId, ThreadStatus,
};

/// An input, worker, migration or database failure.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// SQLite could not complete an operation.
    #[error("SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// The dedicated worker could not be started.
    #[error("could not start SQLite worker: {0}")]
    WorkerStart(#[source] std::io::Error),
    /// The worker ended before delivering a result.
    #[error("SQLite worker is closed")]
    WorkerClosed,
    /// Input failed validation at the store boundary.
    #[error("invalid store input: {0}")]
    InvalidInput(String),
    /// An operation addressed a thread that does not exist.
    #[error("thread {0:?} does not exist")]
    ThreadNotFound(ThreadId),
    /// A thread with this Toasty identifier already exists.
    #[error("thread {0:?} already exists")]
    ThreadAlreadyExists(ThreadId),
    /// A request upsert would change its identity or a terminal answer.
    #[error("request {0:?} conflicts with its persisted identity or state")]
    RequestConflict(RequestId),
    /// The database belongs to a newer schema version.
    #[error("database schema version {found} is newer than supported version {supported}")]
    NewerSchema {
        /// Version found in SQLite's user_version.
        found: i64,
        /// Highest version understood by this crate.
        supported: i64,
    },
    /// A stored schema version is outside SQLite migration numbering.
    #[error("invalid database schema version {0}")]
    InvalidSchemaVersion(i64),
    /// The database could not enable WAL journaling.
    #[error("database must support WAL journaling; SQLite returned {0}")]
    WalUnavailable(String),
    /// JSON serialization or stored JSON decoding failed.
    #[error("request JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

/// A cloneable async handle to one SQLite connection and its worker thread.
#[derive(Clone)]
pub struct Store {
    jobs: mpsc::Sender<Job>,
}

impl Store {
    /// Open a database, enable WAL and foreign keys, and apply migrations.
    ///
    /// The parent directory must already exist. In-memory databases are rejected
    /// because they cannot provide WAL durability. Each committed write uses
    /// `synchronous=FULL`; success means its commit completed on the worker.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        let (jobs, mut receiver) = mpsc::channel::<Job>(128);
        let (ready, initialized) = oneshot::channel();
        std::thread::Builder::new()
            .name("toasty-sqlite".into())
            .spawn(move || {
                let mut connection = match open_connection(&path) {
                    Ok(connection) => connection,
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                if ready.send(Ok(())).is_err() {
                    return;
                }
                while let Some(job) = receiver.blocking_recv() {
                    job(&mut connection);
                }
            })
            .map_err(StoreError::WorkerStart)?;
        initialized.await.map_err(|_| StoreError::WorkerClosed)??;
        Ok(Self { jobs })
    }

    async fn call<T, F>(&self, operation: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, StoreError> + Send + 'static,
    {
        let (reply, result) = oneshot::channel();
        self.jobs
            .send(Box::new(move |connection| {
                let _ = reply.send(operation(connection));
            }))
            .await
            .map_err(|_| StoreError::WorkerClosed)?;
        result.await.map_err(|_| StoreError::WorkerClosed)?
    }

    /// Commit a new idle thread, including its session ID, before returning it.
    pub async fn create_thread(&self, thread: NewThread) -> Result<Thread, StoreError> {
        self.call(move |connection| {
            let cwd = thread
                .cwd
                .to_str()
                .ok_or_else(|| StoreError::InvalidInput("working directory is not UTF-8".into()))?;
            if !thread.cwd.is_absolute() || cwd.contains('\0') {
                return Err(StoreError::InvalidInput(
                    "working directory must be absolute and contain no NUL".into(),
                ));
            }
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if find_thread(&transaction, &thread.id)?.is_some() {
                return Err(StoreError::ThreadAlreadyExists(thread.id));
            }
            transaction.execute(
                "INSERT INTO threads (id, agent, session_id, cwd, title, status, config)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'idle', ?6)",
                params![
                    thread.id.as_str(),
                    thread.agent.as_str(),
                    thread.session_id.as_str(),
                    cwd,
                    thread.title,
                    serde_json::to_string(&thread.config)?
                ],
            )?;
            let saved = require_thread(&transaction, &thread.id)?;
            transaction.commit()?;
            Ok(saved)
        })
        .await
    }

    /// Durably replace the agent's session ID before any prompt uses it.
    pub async fn set_session_id(
        &self,
        thread: &ThreadId,
        session_id: SessionId,
    ) -> Result<(), StoreError> {
        let thread = thread.clone();
        self.call(move |connection| {
            update_thread(connection, &thread, "UPDATE threads SET session_id = ?2,
                updated_at = max(updated_at, cast(unixepoch('subsec') * 1000 AS INTEGER)) WHERE id = ?1", session_id.as_str())
        }).await
    }

    /// Commit a lifecycle change, failing if the thread does not exist.
    pub async fn set_status(
        &self,
        thread: &ThreadId,
        status: ThreadStatus,
    ) -> Result<(), StoreError> {
        let thread = thread.clone();
        self.call(move |connection| {
            update_thread(connection, &thread, "UPDATE threads SET status = ?2,
                updated_at = max(updated_at, cast(unixepoch('subsec') * 1000 AS INTEGER)) WHERE id = ?1", status.as_str())
        }).await
    }

    /// Save settings before a reopened session uses them.
    pub async fn set_config(
        &self,
        thread: &ThreadId,
        config: Vec<(String, String)>,
    ) -> Result<(), StoreError> {
        let config = serde_json::to_string(&config)?;
        let thread = thread.clone();
        self.call(move |connection| {
            update_thread(
                connection,
                &thread,
                "UPDATE threads SET config = ?2 WHERE id = ?1",
                &config,
            )
        })
        .await
    }

    /// Commit the current thread title.
    pub async fn set_title(&self, thread: &ThreadId, title: String) -> Result<(), StoreError> {
        let thread = thread.clone();
        self.call(move |connection| {
            update_thread(connection, &thread, "UPDATE threads SET title = ?2,
                updated_at = max(updated_at, cast(unixepoch('subsec') * 1000 AS INTEGER)) WHERE id = ?1", &title)
        }).await
    }

    /// Atomically allocate a per-thread sequence and commit a cached message.
    pub async fn append_message(
        &self,
        thread: &ThreadId,
        role: MessageRole,
        text: String,
    ) -> Result<Message, StoreError> {
        let thread = thread.clone();
        self.call(move |connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            require_thread(&transaction, &thread)?;
            let message = transaction.query_row(
                "INSERT INTO messages (thread_id, seq, role, text)
                 SELECT ?1, coalesce(max(seq), 0) + 1, ?2, ?3 FROM messages WHERE thread_id = ?1
                 RETURNING thread_id, seq, role, text, created_at",
                params![thread.as_str(), role.as_str(), text],
                read_message,
            )?;
            transaction.execute(
                "UPDATE threads SET updated_at = max(updated_at, ?2) WHERE id = ?1",
                params![thread.as_str(), message.created_at],
            )?;
            transaction.commit()?;
            Ok(message)
        })
        .await
    }

    /// Return threads ordered by most recent change, then identifier.
    pub async fn list_threads(&self) -> Result<Vec<Thread>, StoreError> {
        self.call(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, agent, session_id, cwd, title, status, created_at, updated_at, config
                 FROM threads ORDER BY updated_at DESC, id",
            )?;
            Ok(statement
                .query_map([], read_thread)?
                .collect::<Result<Vec<_>, _>>()?)
        })
        .await
    }

    /// Find a thread by its Toasty identifier.
    pub async fn get_thread(&self, thread: &ThreadId) -> Result<Option<Thread>, StoreError> {
        let thread = thread.clone();
        self.call(move |connection| find_thread(connection, &thread))
            .await
    }

    /// Read a bounded page after an exclusive sequence cursor.
    /// Pages contain at most `limit` chunks (1..=500) and aim for 256 KiB of JSON.
    /// A single larger chunk is returned alone so cursors always make progress.
    pub async fn messages_since(
        &self,
        thread: &ThreadId,
        seq: i64,
        limit: u32,
    ) -> Result<MessagePage, StoreError> {
        if seq < 0 || !(1..=500).contains(&limit) {
            return Err(StoreError::InvalidInput(
                "message cursor must be nonnegative and limit must be 1..=500".into(),
            ));
        }
        let thread = thread.clone();
        self.call(move |connection| {
            require_thread(connection, &thread)?;
            let mut statement = connection.prepare(
                "SELECT thread_id, seq, role, text, created_at FROM messages
                 WHERE thread_id = ?1 AND seq > ?2 ORDER BY seq LIMIT ?3",
            )?;
            let mut rows =
                statement.query_map(params![thread.as_str(), seq, limit + 1], read_message)?;
            let mut messages: Vec<Message> = Vec::new();
            let mut bytes = 0;
            for row in &mut rows {
                let message = row?;
                let size = serde_json::to_vec(&message)?.len();
                if !messages.is_empty()
                    && (messages.len() == limit as usize || bytes + size > 256 * 1024)
                {
                    let next_after = messages.last().map(|message| message.seq);
                    return Ok(MessagePage {
                        messages,
                        next_after,
                    });
                }
                bytes += size;
                messages.push(message);
            }
            Ok(MessagePage {
                messages,
                next_after: None,
            })
        })
        .await
    }

    /// Commit a new request or advance an open request to a terminal state.
    /// Repeating the same state is allowed; reopening or replacing an answer is not.
    pub async fn upsert_request(&self, request: RequestInput) -> Result<StoredRequest, StoreError> {
        self.call(move |connection| {
            let payload = serde_json::to_string(&request.payload)?;
            let (state, answer) = match &request.state {
                RequestState::Open => ("open", None),
                RequestState::Answered(answer) => ("answered", Some(serde_json::to_string(answer)?)),
                RequestState::Withdrawn => ("withdrawn", None),
            };
            let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            require_thread(&transaction, &request.thread_id)?;
            let changed = transaction.execute(
                "INSERT INTO requests (id, thread_id, payload, state, answer)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET state = excluded.state, answer = excluded.answer,
                   updated_at = max(requests.updated_at, cast(unixepoch('subsec') * 1000 AS INTEGER))
                 WHERE requests.thread_id = excluded.thread_id AND requests.payload = excluded.payload
                   AND (requests.state = 'open' OR (requests.state = excluded.state AND requests.answer IS excluded.answer))",
                params![request.id.as_str(), request.thread_id.as_str(), payload, state, answer],
            )?;
            if changed == 0 {
                return Err(StoreError::RequestConflict(request.id));
            }
            let saved = transaction.query_row(
                "SELECT id, thread_id, payload, state, answer, created_at, updated_at FROM requests WHERE id = ?1",
                [request.id.as_str()], read_request,
            )?;
            transaction.commit()?;
            Ok(saved)
        }).await
    }

    /// Find an open, answered or withdrawn request by its identifier.
    pub async fn get_request(
        &self,
        request: &RequestId,
    ) -> Result<Option<StoredRequest>, StoreError> {
        let request = request.clone();
        self.call(move |connection| {
            Ok(connection.query_row(
                "SELECT id, thread_id, payload, state, answer, created_at, updated_at FROM requests WHERE id = ?1",
                [request.as_str()], read_request,
            ).optional()?)
        }).await
    }

    /// Return open requests, optionally scoped to one thread, in creation order.
    pub async fn list_open_requests(
        &self,
        thread: Option<&ThreadId>,
    ) -> Result<Vec<StoredRequest>, StoreError> {
        let thread = thread.cloned();
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT id, thread_id, payload, state, answer, created_at, updated_at FROM requests
                 WHERE state = 'open' AND (?1 IS NULL OR thread_id = ?1) ORDER BY created_at, id",
            )?;
            Ok(statement
                .query_map([thread.as_ref().map(ThreadId::as_str)], read_request)?
                .collect::<Result<Vec<_>, _>>()?)
        })
        .await
    }

    /// Atomically mark running and waiting threads interrupted and return them.
    ///
    /// Returned records already have interrupted status and preserve their agent
    /// session IDs. Call at startup before accepting work. Repeated calls return
    /// an empty list until threads become running or waiting again.
    pub async fn mark_running_interrupted(&self) -> Result<Vec<Thread>, StoreError> {
        self.call(|connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let threads = {
                let mut statement = transaction.prepare(
                    "UPDATE threads SET status = 'interrupted',
                     updated_at = max(updated_at, cast(unixepoch('subsec') * 1000 AS INTEGER))
                     WHERE status IN ('running', 'waiting')
                     RETURNING id, agent, session_id, cwd, title, status, created_at, updated_at, config",
                )?;
                let mut threads = statement
                    .query_map([], read_thread)?
                    .collect::<Result<Vec<_>, _>>()?;
                threads.sort_by(|left, right| left.id.cmp(&right.id));
                threads
            };
            transaction.commit()?;
            Ok(threads)
        })
        .await
    }
}

fn open_connection(path: &Path) -> Result<Connection, StoreError> {
    let mut connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.pragma_update(None, "foreign_keys", true)?;
    let journal: String =
        connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    if !journal.eq_ignore_ascii_case("wal") {
        return Err(StoreError::WalUnavailable(journal));
    }
    connection.pragma_update(None, "synchronous", "FULL")?;
    // Serialize version checks with schema changes, including other openers.
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: i64 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version < 0 {
        return Err(StoreError::InvalidSchemaVersion(version));
    }
    if version > 2 {
        return Err(StoreError::NewerSchema {
            found: version,
            supported: 2,
        });
    }
    if version == 0 {
        transaction.execute_batch(include_str!("migration_001.sql"))?;
        transaction.pragma_update(None, "user_version", 1)?;
    }
    if version < 2 {
        transaction.execute_batch(include_str!("migration_002.sql"))?;
        transaction.pragma_update(None, "user_version", 2)?;
    }
    transaction.commit()?;
    Ok(connection)
}

fn update_thread(
    connection: &Connection,
    thread: &ThreadId,
    sql: &str,
    value: &str,
) -> Result<(), StoreError> {
    if connection.execute(sql, params![thread.as_str(), value])? == 0 {
        return Err(StoreError::ThreadNotFound(thread.clone()));
    }
    Ok(())
}

fn find_thread(connection: &Connection, thread: &ThreadId) -> Result<Option<Thread>, StoreError> {
    Ok(connection.query_row(
        "SELECT id, agent, session_id, cwd, title, status, created_at, updated_at, config FROM threads WHERE id = ?1",
        [thread.as_str()], read_thread,
    ).optional()?)
}

fn require_thread(connection: &Connection, thread: &ThreadId) -> Result<Thread, StoreError> {
    find_thread(connection, thread)?.ok_or_else(|| StoreError::ThreadNotFound(thread.clone()))
}

fn read_typed<T: DeserializeOwned>(row: &Row<'_>, index: usize) -> rusqlite::Result<T> {
    let value: String = row.get(index)?;
    serde_json::from_value(Value::String(value)).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn read_json(row: &Row<'_>, index: usize) -> rusqlite::Result<Value> {
    let value: String = row.get(index)?;
    serde_json::from_str(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn read_thread(row: &Row<'_>) -> rusqlite::Result<Thread> {
    Ok(Thread {
        id: read_typed(row, 0)?,
        agent: read_typed(row, 1)?,
        session_id: read_typed(row, 2)?,
        cwd: Path::new(&row.get::<_, String>(3)?).to_path_buf(),
        title: row.get(4)?,
        status: read_typed(row, 5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        config: serde_json::from_value(read_json(row, 8)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                8,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
    })
}

fn read_message(row: &Row<'_>) -> rusqlite::Result<Message> {
    Ok(Message {
        thread_id: read_typed(row, 0)?,
        seq: row.get(1)?,
        role: read_typed(row, 2)?,
        text: row.get(3)?,
        created_at: row.get(4)?,
    })
}

fn read_request(row: &Row<'_>) -> rusqlite::Result<StoredRequest> {
    let state: String = row.get(3)?;
    let state = match state.as_str() {
        "open" => RequestState::Open,
        "answered" => RequestState::Answered(read_json(row, 4)?),
        "withdrawn" => RequestState::Withdrawn,
        _ => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(StoreError::InvalidInput(format!(
                    "unknown stored request state {state}"
                ))),
            ));
        }
    };
    Ok(StoredRequest {
        request: RequestInput {
            id: read_typed(row, 0)?,
            thread_id: read_typed(row, 1)?,
            payload: read_json(row, 2)?,
            state,
        },
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}
