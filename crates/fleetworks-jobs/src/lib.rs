//! Disk-backed operational job state owned exclusively by the Jobs service.
//!
//! All SQLite work runs on Tokio blocking workers. A single permit bounds
//! in-flight database work per handle; transactions serialize writes across
//! independently opened handles too. Domain publication still requires a
//! domain-owned fencing/idempotency protocol; an outbox entry alone cannot
//! make a SQLite/SurrealDB write atomic.

pub mod logs;
mod operations;
pub mod queue;
mod queue_types;
mod read;
pub mod scoped_logs;
mod types;

pub use types::{
    Attempt, Command, Event, Job, Lease, NewJob, NewTask, OutboxEntry, Request, Response, Task,
};

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, ensure, Context, Result};
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::TransactionBehavior;
use tokio::sync::Semaphore;

struct Inner {
    connection: Mutex<Connection>,
    permit: Arc<Semaphore>,
}

/// Cloneable asynchronous handle to one persisted SQLite job database.
#[derive(Clone)]
pub struct Store(Arc<Inner>);

impl Store {
    pub(crate) async fn with_connection<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let permit = self.0.permit.clone().acquire_owned().await?;
        let inner = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut connection = inner
                .connection
                .lock()
                .map_err(|_| anyhow!("Jobs database mutex poisoned"))?;
            operation(&mut connection)
        })
        .await
        .context("execute Jobs database worker")?
    }

    /// Opens a local disk database, applies its schema, and verifies durability.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let connection = tokio::task::spawn_blocking(move || -> Result<Connection> {
            ensure!(
                !path.as_os_str().is_empty() && path != Path::new(":memory:"),
                "Jobs requires a disk-backed path"
            );
            ensure!(
                rusqlite::version_number() >= 3_051_003,
                "SQLite 3.51.3 or newer is required"
            );
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent).context("create Jobs data directory")?;
            }
            let mut connection = Connection::open(&path).context("open Jobs SQLite database")?;
            connection.busy_timeout(Duration::from_secs(5))?;
            connection.pragma_update(None, "foreign_keys", true)?;
            let mode: String =
                connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
            ensure!(mode == "wal", "Jobs database did not enable WAL mode");
            connection.pragma_update(None, "synchronous", "FULL")?;
            #[cfg(target_os = "macos")]
            connection.execute_batch("PRAGMA fullfsync=ON; PRAGMA checkpoint_fullfsync=ON;")?;
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let version: i64 =
                transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
            ensure!(
                version <= 1,
                "Jobs database schema {version} is newer than this executable"
            );
            if version == 0 {
                transaction.execute_batch(include_str!("schema.sql"))?;
                transaction.pragma_update(None, "user_version", 1)?;
            }
            transaction.commit()?;
            Ok(connection)
        })
        .await
        .context("open Jobs database worker")??;
        Ok(Self(Arc::new(Inner {
            connection: Mutex::new(connection),
            permit: Arc::new(Semaphore::new(1)),
        })))
    }

    /// Executes a bounded typed operation, atomically retaining mutation receipts.
    /// Reusing a key for a different command is an error. Retrying a command
    /// returns its original result even when the service clock has advanced.
    pub async fn execute(&self, request: Request) -> Result<Response> {
        let permit = self.0.permit.clone().acquire_owned().await?;
        let inner = self.0.clone();
        tokio::task::spawn_blocking(move || -> Result<Response> {
            let _permit = permit;
            let mut connection = inner.connection.lock().map_err(|_| anyhow!("Jobs database mutex poisoned"))?;
            if request.command.is_read() {
                return read::execute(&connection, &request.command);
            }
            ensure!(!request.operation_key.trim().is_empty(), "mutation requires an operation key");
            ensure!(request.operation_key.len() <= 512, "operation key exceeds 512 bytes");
            let serialized = serde_json::to_string(&request.command)?;
            let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let previous: Option<(String, String)> = transaction.query_row(
                "SELECT request, response FROM receipts WHERE operation_key=?1", [&request.operation_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).optional()?;
            if let Some((previous_request, response)) = previous {
                ensure!(previous_request == serialized, "operation key was already used for another request");
                return serde_json::from_str(&response).context("decode Jobs receipt");
            }
            let (response, job_id) = operations::execute(&transaction, &request.command, request.now_ms)?;
            transaction.execute("INSERT INTO receipts(operation_key, job_id, request, response) VALUES(?1,?2,?3,?4)",
                rusqlite::params![request.operation_key, job_id, serialized, serde_json::to_string(&response)?])?;
            transaction.commit()?;
            Ok(response)
        }).await.context("execute Jobs database worker")?
    }
}
