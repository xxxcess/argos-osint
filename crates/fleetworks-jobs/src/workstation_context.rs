//! SQLite mechanics for the Embedded companion's separate observation journal.

use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::ensure;
use anyhow::Context;
use anyhow::Result;
use rusqlite::params;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::Transaction;
use rusqlite::TransactionBehavior;
use tokio::sync::mpsc;
use tokio::sync::oneshot;

const MAX_QUEUE: usize = 256;
const MAX_BATCH: usize = 32;
const MAX_PAYLOAD: usize = 2_048;
const MAX_EVENTS: i64 = 10_000;
const MAX_CANDIDATES: usize = 20;
const RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1_000;
const HIGH_WATER_BYTES: u64 = 64 * 1024 * 1024;
const DB_BYTES: i64 = 48 * 1024 * 1024;
const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(300);
const QUERY_BUDGET: Duration = Duration::from_millis(100);
const BUSY_BUDGET: Duration = Duration::from_millis(15);
const APPLICATION_ID: i64 = 0x4657_4358;
const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
    CREATE TABLE meta (
        singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
        deployment TEXT NOT NULL,
        started_at_ms INTEGER NOT NULL,
        continuous_since_ms INTEGER NOT NULL,
        last_activity_ms INTEGER NOT NULL,
        last_gap_from_ms INTEGER,
        last_gap_until_ms INTEGER,
        dropped INTEGER NOT NULL DEFAULT 0
    ) STRICT;
    CREATE TABLE events (
        sequence INTEGER PRIMARY KEY AUTOINCREMENT,
        scope TEXT NOT NULL,
        family TEXT NOT NULL,
        resource TEXT NOT NULL,
        generation INTEGER NOT NULL,
        revision INTEGER NOT NULL,
        observed_at_ms INTEGER NOT NULL,
        recorded_at_ms INTEGER NOT NULL,
        payload TEXT NOT NULL CHECK (length(CAST(payload AS BLOB)) <= 2048)
    ) STRICT;
    CREATE INDEX events_lookup ON events (scope, family, observed_at_ms DESC, sequence DESC);
    CREATE INDEX events_resource ON events (scope, family, resource, observed_at_ms DESC, sequence DESC);
    CREATE INDEX events_expiry ON events (observed_at_ms);
    CREATE TABLE current_state (
        scope TEXT NOT NULL,
        family TEXT NOT NULL,
        resource TEXT NOT NULL,
        generation INTEGER NOT NULL,
        revision INTEGER NOT NULL,
        observed_at_ms INTEGER NOT NULL,
        recorded_at_ms INTEGER NOT NULL,
        payload TEXT NOT NULL CHECK (length(CAST(payload AS BLOB)) <= 2048),
        terminal INTEGER NOT NULL CHECK (terminal IN (0, 1)),
        changed_at_ms INTEGER NOT NULL,
        sequence INTEGER NOT NULL,
        PRIMARY KEY (scope, family, resource)
    ) STRICT, WITHOUT ROWID;
    CREATE INDEX current_expiry ON current_state (observed_at_ms);
";

/// Companion-validated data. Index keys and provenance remain outside the payload.
pub struct Record {
    pub scope: String,
    pub family: &'static str,
    pub resource: String,
    pub generation: i64,
    pub revision: i64,
    pub observed_at_ms: i64,
    pub payload: String,
    pub terminal: bool,
}

/// Read current observations or original events within the specified interval.
#[derive(Clone, Copy)]
pub enum Range {
    Current,
    History { since_ms: i64, until_ms: i64 },
}

/// Lookup constrained to a single authorized scope and one observation family.
pub struct Lookup {
    pub scope: String,
    pub family: &'static str,
    pub resource: Option<String>,
    pub range: Range,
}

/// Retained provenance of one observation.
pub struct Entry {
    pub sequence: i64,
    pub observed_at_ms: i64,
    pub recorded_at_ms: i64,
    pub payload: String,
}

/// Bounded rows and conservative coverage from one SQLite read.
pub struct Page {
    pub entries: Vec<Entry>,
    pub coverage_started_at_ms: i64,
    pub retrieved_at_ms: i64,
    pub incomplete: bool,
}

struct Query {
    lookup: Lookup,
    reply: oneshot::Sender<Option<Page>>,
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
}

enum Message {
    Observe(Record),
    Query(Query),
    Shutdown(oneshot::Sender<()>),
}

struct QueryGuard(Arc<AtomicBool>);

impl Drop for QueryGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// Nonblocking producer and bounded asynchronous reader for one blocking SQLite owner.
#[derive(Clone)]
pub struct Journal {
    queue: mpsc::Sender<Message>,
    dropped: Arc<AtomicU64>,
}

impl Journal {
    /// Open a separate disk journal under an existing exclusive companion state lock.
    /// The worker holds its own copy of the lock until the SQLite connection closes.
    pub async fn open(
        path: PathBuf,
        deployment: String,
        state_lock: impl Send + 'static,
    ) -> Result<Self> {
        let (queue, messages) = mpsc::channel(MAX_QUEUE);
        let (opened, status) = oneshot::channel();
        let dropped = Arc::new(AtomicU64::new(0));
        let owner_dropped = Arc::clone(&dropped);
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let _state_lock = state_lock;
            match Worker::open(path, &deployment, owner_dropped) {
                Ok(worker) => {
                    if opened.send(Ok(())).is_ok() {
                        worker.run(messages, &runtime);
                    }
                }
                Err(error) => {
                    let _ = opened.send(Err(error));
                }
            }
        });
        tokio::time::timeout(QUERY_BUDGET, status)
            .await
            .context("open workstation journal budget")?
            .context("open workstation journal worker")??;
        Ok(Self { queue, dropped })
    }

    /// Enqueue one observation without blocking the existing gateway read.
    pub fn record(&self, record: Record) {
        if !valid_record(&record) || self.queue.try_send(Message::Observe(record)).is_err() {
            count_dropped(&self.dropped, 1);
        }
    }

    /// Read with a bounded queue deadline. Dropping the caller cancels queued or running SQL.
    pub async fn query(&self, lookup: Lookup) -> Option<Page> {
        let (reply, receiver) = oneshot::channel();
        let guard = QueryGuard(Arc::new(AtomicBool::new(false)));
        let deadline = Instant::now() + QUERY_BUDGET;
        self.queue
            .try_send(Message::Query(Query {
                lookup,
                reply,
                cancelled: Arc::clone(&guard.0),
                deadline,
            }))
            .ok()?;
        tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), receiver)
            .await
            .ok()?
            .ok()?
    }

    /// Drain the bounded queue and close the database before releasing the state lock.
    pub async fn shutdown(&self) {
        let (reply, done) = oneshot::channel();
        if self.queue.send(Message::Shutdown(reply)).await.is_ok() {
            let _ = done.await;
        }
    }
}

fn valid_record(record: &Record) -> bool {
    (1..=512).contains(&record.scope.len())
        && (1..=128).contains(&record.resource.len())
        && (1..=MAX_PAYLOAD).contains(&record.payload.len())
        && record.generation >= 0
        && record.revision >= 0
        && record.observed_at_ms > 0
}

fn count_dropped(dropped: &AtomicU64, count: u64) {
    let _ = dropped.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        Some(current.saturating_add(count))
    });
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

struct Worker {
    connection: Connection,
    path: PathBuf,
    dropped: Arc<AtomicU64>,
    suspended: bool,
    write_unavailable: bool,
}

impl Worker {
    fn open(path: PathBuf, deployment: &str, dropped: Arc<AtomicU64>) -> Result<Self> {
        ensure!(
            rusqlite::version_number() >= 3_051_003,
            "unsupported SQLite version"
        );
        let mut connection =
            Connection::open(&path).context("open workstation observation database")?;
        connection.busy_timeout(BUSY_BUDGET)?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let application: i64 =
            connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
        ensure!(
            (version == 0 && application == 0)
                || (version == SCHEMA_VERSION && application == APPLICATION_ID),
            "unsupported workstation observation database schema"
        );
        if version == 0 {
            let tables: i64 = connection.query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get(0),
            )?;
            ensure!(
                tables == 0,
                "unexpected schema in workstation observation database"
            );
            connection.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
        } else {
            let owner: String = connection.query_row(
                "SELECT deployment FROM meta WHERE singleton=1",
                [],
                |row| row.get(0),
            )?;
            ensure!(
                owner == deployment,
                "workstation observation identity changed"
            );
        }
        let mode: String = connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
        ensure!(
            mode == "wal",
            "workstation observation database requires WAL"
        );
        connection.pragma_update(None, "synchronous", "FULL")?;
        connection.pragma_update(None, "wal_autocheckpoint", 64)?;
        connection.pragma_update(None, "journal_size_limit", 512 * 1024)?;
        let page_bytes: i64 = connection.pragma_query_value(None, "page_size", |row| row.get(0))?;
        ensure!(page_bytes > 0, "invalid workstation observation page size");
        connection.pragma_update(None, "max_page_count", DB_BYTES / page_bytes)?;
        #[cfg(target_os = "macos")]
        connection.execute_batch("PRAGMA fullfsync=ON; PRAGMA checkpoint_fullfsync=ON;")?;
        let now = now_ms();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if version == 0 {
            transaction.execute_batch(SCHEMA)?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
            transaction.execute(
                "INSERT INTO meta(singleton,deployment,started_at_ms,continuous_since_ms,last_activity_ms)
                 VALUES(1,?1,?2,?2,?2)",
                params![deployment, now],
            )?;
        } else {
            // A restart is a known coverage break even after graceful shutdown.
            note_gap(&transaction, 0, now)?;
        }
        prune_expired(&transaction, now)?;
        limit_events(&transaction)?;
        transaction.commit()?;
        let mut worker = Self {
            connection,
            path,
            dropped,
            suspended: false,
            write_unavailable: false,
        };
        worker.check_size();
        Ok(worker)
    }

    fn run(mut self, mut messages: mpsc::Receiver<Message>, runtime: &tokio::runtime::Handle) {
        let mut next = None;
        let mut maintenance = Instant::now() + MAINTENANCE_INTERVAL;
        let shutdown = loop {
            if Instant::now() >= maintenance {
                self.store(&[]);
                let _ = self
                    .connection
                    .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()));
                maintenance = Instant::now() + MAINTENANCE_INTERVAL;
            }
            let message = if let Some(message) = next.take() {
                Some(message)
            } else {
                let deadline = tokio::time::Instant::from_std(maintenance);
                match runtime
                    .block_on(async { tokio::time::timeout_at(deadline, messages.recv()).await })
                {
                    Ok(message) => message,
                    Err(_) => continue,
                }
            };
            match message {
                Some(Message::Observe(first)) => {
                    let mut batch = Vec::with_capacity(MAX_BATCH);
                    batch.push(first);
                    while batch.len() < MAX_BATCH {
                        match messages.try_recv() {
                            Ok(Message::Observe(record)) => batch.push(record),
                            Ok(other) => {
                                next = Some(other);
                                break;
                            }
                            Err(_) => break,
                        }
                    }
                    self.store(&batch);
                }
                Some(Message::Query(query)) => self.answer(query),
                Some(Message::Shutdown(reply)) => break Some(reply),
                None => break None,
            }
        };
        self.store(&[]);
        drop(self);
        if let Some(reply) = shutdown {
            let _ = reply.send(());
        }
    }

    fn check_size(&mut self) {
        self.suspended = !total_size(&self.path).is_ok_and(|size| size < HIGH_WATER_BYTES);
        if self.suspended {
            // Reclaim free pages in short increments and give the connection no time
            // to wait for an unrelated reader during the truncating checkpoint.
            let _ = self.connection.busy_timeout(Duration::ZERO);
            let _ = self
                .connection
                .execute_batch("PRAGMA incremental_vacuum(128);");
            let _ = self
                .connection
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
            let _ = self.connection.busy_timeout(BUSY_BUDGET);
            self.suspended = !total_size(&self.path).is_ok_and(|size| size < HIGH_WATER_BYTES);
        }
    }

    fn store(&mut self, records: &[Record]) {
        if self.suspended {
            self.check_size();
        }
        let committed = !self.suspended && self.commit(records, now_ms()).is_ok();
        if !committed {
            count_dropped(&self.dropped, records.len() as u64);
        }
        self.check_size();
        self.write_unavailable = !committed || self.suspended;
    }

    fn commit(&mut self, records: &[Record], now: i64) -> Result<()> {
        let dropped = self.dropped.swap(0, Ordering::AcqRel);
        let result = self.commit_observations(records, now, dropped);
        if result.is_err() {
            count_dropped(&self.dropped, dropped);
        }
        result
    }

    fn commit_observations(&mut self, records: &[Record], now: i64, dropped: u64) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if dropped > 0 || self.write_unavailable {
            note_gap(&tx, dropped, now)?;
        }
        prune_expired(&tx, now)?;
        for record in records {
            apply(&tx, record, now)?;
        }
        limit_events(&tx)?;
        tx.execute(
            "UPDATE meta SET last_activity_ms=MAX(last_activity_ms, ?1) WHERE singleton=1",
            [now],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn answer(&mut self, query: Query) {
        if query.reply.is_closed() || Instant::now() >= query.deadline {
            return;
        }
        let deadline = query.deadline;
        let cancelled = query.cancelled;
        if self
            .connection
            .progress_handler(
                1_000,
                Some(move || cancelled.load(Ordering::Acquire) || Instant::now() >= deadline),
            )
            .is_err()
        {
            return;
        }
        let result = self.read(&query.lookup, now_ms()).ok();
        let _ = self.connection.progress_handler(0, None::<fn() -> bool>);
        let _ = query.reply.send(result);
    }

    fn read(&self, lookup: &Lookup, now: i64) -> Result<Page> {
        let cutoff = now.saturating_sub(RETENTION_MS);
        let (started, continuous): (i64, i64) = self.connection.query_row(
            "SELECT started_at_ms, continuous_since_ms FROM meta WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let after_now = now.saturating_add(1);
        let (table, since, until) = match lookup.range {
            Range::Current => ("current_state", cutoff, after_now),
            Range::History { since_ms, until_ms } => {
                ("events", since_ms.max(cutoff), until_ms.min(after_now))
            }
        };
        let match_resource = if lookup.resource.is_some() {
            "resource=?3"
        } else {
            "?3 IS NULL"
        };
        let sql = format!(
            "SELECT sequence,observed_at_ms,recorded_at_ms,payload FROM {table}
             WHERE scope=?1 AND family=?2 AND {match_resource}
               AND observed_at_ms >= ?4 AND observed_at_ms < ?5
             ORDER BY observed_at_ms DESC, sequence DESC LIMIT ?6"
        );
        let mut statement = self.connection.prepare_cached(&sql)?;
        let entries = statement
            .query_map(
                params![
                    lookup.scope,
                    lookup.family,
                    lookup.resource,
                    since,
                    until,
                    MAX_CANDIDATES as i64
                ],
                |row| {
                    Ok(Entry {
                        sequence: row.get(0)?,
                        observed_at_ms: row.get(1)?,
                        recorded_at_ms: row.get(2)?,
                        payload: row.get(3)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let coverage_started_at_ms = started.max(cutoff);
        let incomplete = self.suspended
            || self.write_unavailable
            || self.dropped.load(Ordering::Acquire) > 0
            || entries.len() == MAX_CANDIDATES
            || matches!(
                lookup.range,
                Range::History { since_ms, .. } if since_ms < continuous.max(coverage_started_at_ms)
            );
        Ok(Page {
            entries,
            coverage_started_at_ms,
            retrieved_at_ms: now,
            incomplete,
        })
    }
}

fn note_gap(tx: &Transaction<'_>, dropped: u64, now: i64) -> Result<()> {
    tx.execute(
        "UPDATE meta
         SET last_gap_from_ms=last_activity_ms,
             last_gap_until_ms=MAX(last_activity_ms,?1),
             continuous_since_ms=MAX(continuous_since_ms,?1),
             last_activity_ms=MAX(last_activity_ms,?1),
             dropped=MIN(dropped, 9223372036854775807 - ?2) + ?2
         WHERE singleton=1",
        params![now, i64::try_from(dropped).unwrap_or(i64::MAX)],
    )?;
    Ok(())
}

fn total_size(path: &Path) -> std::io::Result<u64> {
    let mut total = std::fs::metadata(path)?.len();
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(suffix);
        match std::fs::metadata(sidecar) {
            Ok(metadata) => total = total.saturating_add(metadata.len()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(total)
}

fn apply(tx: &Transaction<'_>, record: &Record, recorded_at_ms: i64) -> Result<()> {
    if record.observed_at_ms < recorded_at_ms.saturating_sub(RETENTION_MS)
        || record.observed_at_ms > recorded_at_ms
    {
        return Ok(());
    }
    type Prior = (i64, i64, i64, String, i64, i64);
    let prior: Option<Prior> = tx
        .query_row(
            "SELECT generation,revision,observed_at_ms,payload,sequence,changed_at_ms
             FROM current_state WHERE scope=?1 AND family=?2 AND resource=?3",
            params![record.scope, record.family, record.resource],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?;
    let latest = (record.generation, record.revision, record.observed_at_ms);
    if prior
        .as_ref()
        .is_some_and(|(generation, revision, at, _, _, _)| latest <= (*generation, *revision, *at))
    {
        return Ok(());
    }

    let (sequence, changed_at_ms) = match prior {
        Some((_, _, _, payload, sequence, at)) if payload == record.payload => (sequence, at),
        _ => {
            tx.execute(
                "INSERT INTO events(scope,family,resource,generation,revision,observed_at_ms,recorded_at_ms,payload)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    record.scope, record.family, record.resource, record.generation, record.revision,
                    record.observed_at_ms, recorded_at_ms, record.payload,
                ],
            )?;
            (tx.last_insert_rowid(), record.observed_at_ms)
        }
    };
    tx.execute(
        "INSERT INTO current_state(scope,family,resource,generation,revision,observed_at_ms,recorded_at_ms,payload,terminal,sequence,changed_at_ms)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
         ON CONFLICT(scope,family,resource) DO UPDATE SET
           generation=excluded.generation,revision=excluded.revision,observed_at_ms=excluded.observed_at_ms,
           recorded_at_ms=excluded.recorded_at_ms,payload=excluded.payload,terminal=excluded.terminal,
           sequence=excluded.sequence,changed_at_ms=excluded.changed_at_ms",
        params![
            record.scope, record.family, record.resource, record.generation, record.revision,
            record.observed_at_ms, recorded_at_ms, record.payload, record.terminal, sequence, changed_at_ms,
        ],
    )?;
    Ok(())
}

fn prune_expired(tx: &Transaction<'_>, now: i64) -> Result<()> {
    let cutoff = now.saturating_sub(RETENTION_MS);
    tx.execute("DELETE FROM events WHERE observed_at_ms < ?1", [cutoff])?;
    tx.execute(
        "DELETE FROM current_state WHERE observed_at_ms < ?1",
        [cutoff],
    )?;
    tx.execute(
        "DELETE FROM current_state WHERE terminal=1 AND changed_at_ms < ?1",
        [cutoff],
    )?;
    Ok(())
}

fn limit_events(tx: &Transaction<'_>) -> Result<()> {
    let count_cutoff: Option<i64> = tx
        .query_row(
            "SELECT sequence FROM events ORDER BY sequence DESC LIMIT 1 OFFSET ?1",
            [MAX_EVENTS],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(sequence) = count_cutoff {
        // Report conservative coverage when late observations were pruned by sequence.
        tx.execute(
            "UPDATE meta SET continuous_since_ms=MAX(continuous_since_ms,
             (SELECT MAX(observed_at_ms) + 1 FROM events WHERE sequence <= ?1))
             WHERE singleton=1",
            [sequence],
        )?;
        tx.execute("DELETE FROM events WHERE sequence <= ?1", [sequence])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(scope: &str, at: i64, revision: i64, payload: &str) -> Record {
        Record {
            scope: scope.to_owned(),
            family: "readiness",
            resource: "actor_search".to_owned(),
            generation: 0,
            revision,
            observed_at_ms: at,
            payload: payload.to_owned(),
            terminal: false,
        }
    }

    fn lookup(scope: &str, range: Range) -> Lookup {
        Lookup {
            scope: scope.to_owned(),
            family: "readiness",
            resource: Some("actor_search".to_owned()),
            range,
        }
    }

    fn history(since_ms: i64, until_ms: i64) -> Lookup {
        lookup("scope-a", Range::History { since_ms, until_ms })
    }

    fn owner(path: &Path) -> Worker {
        Worker::open(
            path.to_path_buf(),
            "deployment-a",
            Arc::new(AtomicU64::new(0)),
        )
        .expect("open observation journal")
    }

    #[test]
    fn observations_keep_original_events_across_reordered_polls_and_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("context.sqlite3");
        let mut worker = owner(&path);
        let now = now_ms();
        worker
            .commit(&[observation("scope-a", now, 2, "paused")], now)
            .unwrap();
        worker
            .commit(
                &[
                    observation("scope-a", now + 1, 3, "paused"),
                    observation("scope-a", now + 2, 1, "ready"),
                    observation("scope-b", now + 2, 3, "private"),
                ],
                now + 3,
            )
            .unwrap();
        let current = worker
            .read(&lookup("scope-a", Range::Current), now + 3)
            .unwrap();
        assert_eq!(current.entries.len(), 1);
        assert_eq!(current.entries[0].observed_at_ms, now + 1);
        let page = worker.read(&history(now, now + 3), now + 3).unwrap();
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].payload, "paused");
        assert_eq!(page.entries[0].observed_at_ms, now);
        assert_eq!(page.entries[0].sequence, current.entries[0].sequence);
        let previous_sequence = page.entries[0].sequence;
        drop(worker);

        assert!(Worker::open(
            path.clone(),
            "other-deployment",
            Arc::new(AtomicU64::new(0))
        )
        .is_err());
        let mut worker = owner(&path);
        let later = now.max(now_ms()) + 4;
        worker
            .commit(&[observation("scope-a", later, 4, "ready")], later)
            .unwrap();
        let before_boundary = worker.read(&history(now - 1, later), later).unwrap();
        assert_eq!(
            before_boundary.entries.len(),
            1,
            "the upper interval boundary is exclusive"
        );
        let reopened = worker.read(&history(now - 1, later + 1), later).unwrap();
        assert!(
            reopened.incomplete,
            "the restart must report a coverage gap"
        );
        assert_eq!(reopened.entries.len(), 2);
        assert_eq!(reopened.entries[0].payload, "ready");
        assert!(reopened.entries[0].sequence > previous_sequence);
    }

    #[test]
    fn event_count_time_and_terminal_retention_have_independent_bounds() {
        let dir = tempfile::tempdir().unwrap();
        let mut worker = owner(&dir.path().join("context.sqlite3"));
        let now = now_ms();
        let first = now - RETENTION_MS - 5;
        let mut terminal = observation("scope-a", first, 1, "completed");
        terminal.resource = "finished-job".to_owned();
        terminal.terminal = true;
        worker.commit(&[terminal], first).unwrap();
        worker
            .connection
            .execute(
                "UPDATE current_state SET observed_at_ms=?1 WHERE resource='finished-job'",
                [now],
            )
            .unwrap();
        worker.connection.execute(
            "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<?1)
             INSERT INTO events(scope,family,resource,generation,revision,observed_at_ms,recorded_at_ms,payload)
             SELECT 'scope-a','readiness','actor_search',0,x,?2,?2,'paused' FROM n",
            params![MAX_EVENTS + 1, now],
        ).unwrap();
        worker.commit(&[], now).unwrap();
        let total: i64 = worker
            .connection
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(total, MAX_EVENTS);
        let terminal_count: i64 = worker
            .connection
            .query_row(
                "SELECT COUNT(*) FROM current_state WHERE resource='finished-job'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(terminal_count, 0);
        let page = worker.read(&history(first, now + 1), now + 1).unwrap();
        assert_eq!(page.entries.len(), MAX_CANDIDATES);
        assert!(
            page.incomplete,
            "age and count pruning limit the available history"
        );
    }

    #[test]
    fn repeated_live_observations_checkpoint_wal_within_the_file_budget() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("context.sqlite3");
        let mut worker = owner(&path);
        let started = now_ms();
        for revision in 1..=96 {
            let at = started + revision;
            worker
                .commit(&[observation("scope-a", at, revision, "paused")], at)
                .unwrap();
            worker.check_size();
            assert!(!worker.suspended);
        }
        let size = total_size(&path).unwrap();
        assert!(
            size < 2 * 1024 * 1024,
            "regular checkpoints bound file growth: {size}"
        );
        let synchronous: i64 = worker
            .connection
            .pragma_query_value(None, "synchronous", |row| row.get(0))
            .unwrap();
        assert_eq!(synchronous, 2, "journal requires FULL synchronization");
        let page = worker
            .read(&history(started, started + 96), started + 96)
            .unwrap();
        assert_eq!(
            page.entries.len(),
            1,
            "identical polling does not append events"
        );
    }

    #[test]
    fn write_failure_leaves_previous_evidence_and_marks_gap() {
        let dir = tempfile::tempdir().unwrap();
        let mut worker = owner(&dir.path().join("context.sqlite3"));
        let now = now_ms();
        worker
            .commit(&[observation("scope-a", now, 1, "ready")], now)
            .unwrap();
        worker
            .connection
            .pragma_update(None, "query_only", true)
            .unwrap();
        worker.store(&[observation("scope-a", now, 2, "paused")]);
        let page = worker.read(&history(now, now + 1), now + 1).unwrap();
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].payload, "ready");
        assert!(
            page.incomplete,
            "readable rows do not imply collection was available"
        );
        worker
            .connection
            .pragma_update(None, "query_only", false)
            .unwrap();
        worker.store(&[]);
        let lost: i64 = worker
            .connection
            .query_row("SELECT dropped FROM meta", [], |row| row.get(0))
            .unwrap();
        assert_eq!(lost, 1);
    }

    #[test]
    fn producer_drops_at_capacity_and_rejects_oversized_payloads() {
        let (queue, mut receiver) = mpsc::channel(MAX_QUEUE);
        let journal = Journal {
            queue,
            dropped: Arc::new(AtomicU64::new(0)),
        };
        let now = now_ms();
        journal.record(observation("scope-a", now, 1, &"x".repeat(MAX_PAYLOAD + 1)));
        for _ in 0..MAX_QUEUE + 1 {
            journal.record(observation("scope-a", now, 1, "paused"));
        }
        assert_eq!(journal.dropped.load(Ordering::Acquire), 2);
        let mut accepted = 0;
        while let Ok(Message::Observe(_)) = receiver.try_recv() {
            accepted += 1;
        }
        assert_eq!(accepted, MAX_QUEUE);
    }

    #[tokio::test]
    async fn dropped_query_cancels_queued_sql() {
        let (queue, mut receiver) = mpsc::channel(MAX_QUEUE);
        let journal = Journal {
            queue,
            dropped: Arc::new(AtomicU64::new(0)),
        };
        let caller =
            tokio::spawn(async move { journal.query(lookup("scope-a", Range::Current)).await });
        let Some(Message::Query(query)) = receiver.recv().await else {
            panic!("expected query")
        };
        caller.abort();
        assert!(matches!(caller.await, Err(error) if error.is_cancelled()));
        assert!(query.cancelled.load(Ordering::Acquire));
        assert!(query.reply.is_closed());
    }

    #[test]
    fn damaged_file_is_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("context.sqlite3");
        std::fs::write(&path, b"damaged journal").unwrap();
        assert!(Worker::open(path.clone(), "deployment-a", Arc::new(AtomicU64::new(0))).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"damaged journal");
    }
}
