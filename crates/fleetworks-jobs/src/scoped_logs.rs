//! Operational SQLite log pages for an already authorized COMPLETE job cohort.
//! Empty cohorts are rejected, never interpreted as the legacy global filter.

use std::collections::BTreeSet;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use anyhow::ensure;
use anyhow::Context;
use anyhow::Result;
use rusqlite::params;
use rusqlite::Connection;
use serde::Deserialize;
use serde::Serialize;

use crate::Store;

pub const MAX_COHORT: usize = 512;
pub const MAX_PAGE: usize = 100;
const SCAN_BUDGET: Duration = Duration::from_millis(750);
const OWNER_BUDGET: Duration = Duration::from_millis(900);

/// Cancellation reaches the blocking SQLite VM, rather than only dropping its
/// async reply. The deadline also applies when a remote caller disconnects
/// without immediately cancelling the private HTTP handler.
struct CancelRead(Arc<AtomicBool>);
impl Drop for CancelRead {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
fn bounded_read<T>(
    connection: &mut Connection,
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
    operation: impl FnOnce(&mut Connection) -> Result<T>,
) -> Result<T> {
    ensure!(
        !cancelled.load(Ordering::Acquire) && Instant::now() < deadline,
        "scoped log read deadline expired"
    );
    let previous_busy: i64 =
        connection.pragma_query_value(None, "busy_timeout", |row| row.get(0))?;
    let previous_busy = u64::try_from(previous_busy).context("negative SQLite busy timeout")?;
    let result = (|| {
        // Lock contention cannot consume the public follow's whole deadline.
        connection.busy_timeout(Duration::from_millis(25))?;
        connection.progress_handler(
            1000,
            Some(move || cancelled.load(Ordering::Acquire) || Instant::now() >= deadline),
        )?;
        operation(connection)
    })();
    // Always remove per-read state before the sole connection/permit is reused.
    let progress = connection.progress_handler(0, None::<fn() -> bool>);
    let busy = connection.busy_timeout(Duration::from_millis(previous_busy));
    progress?;
    busy?;
    result
}
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct LogFilters {
    pub min_rank: Option<i64>,
    pub app: String,
    pub text: String,
    pub since: String,
}
impl LogFilters {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.min_rank.is_none_or(|rank| (0..=3).contains(&rank)),
            "invalid log severity"
        );
        for (value, bound) in [(&self.app, 64), (&self.text, 256), (&self.since, 64)] {
            ensure!(
                value.len() <= bound && !value.chars().any(char::is_control),
                "invalid bounded log filter"
            );
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LogDirection {
    Older,
    Newer,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ScopedLogRead {
    pub job_ids: Vec<String>,
    pub filters: LogFilters,
    pub direction: LogDirection,
    pub upper_seq: Option<i64>,
    pub boundary: Option<i64>,
    pub limit: usize,
}
impl ScopedLogRead {
    pub fn new(
        job_ids: Vec<String>,
        filters: LogFilters,
        direction: LogDirection,
        upper_seq: Option<i64>,
        boundary: Option<i64>,
        limit: usize,
    ) -> Result<Self> {
        let read = Self {
            job_ids,
            filters,
            direction,
            upper_seq,
            boundary,
            limit,
        };
        read.validate()?;
        Ok(read)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.job_ids.is_empty() && self.job_ids.len() <= MAX_COHORT,
            "complete nonempty log cohort required"
        );
        let mut unique = BTreeSet::new();
        for id in &self.job_ids {
            ensure!(
                !id.is_empty()
                    && id.len() <= 512
                    && !id.chars().any(char::is_control)
                    && unique.insert(id),
                "invalid unique scoped job identity"
            );
        }
        self.filters.validate()?;
        ensure!(
            (1..=MAX_PAGE).contains(&self.limit)
                && self.upper_seq.is_none_or(|seq| seq >= 0)
                && self.boundary.is_none_or(|seq| seq >= 0),
            "invalid log page boundary"
        );
        ensure!(
            self.direction != LogDirection::Newer || self.boundary.is_some(),
            "follow boundary required"
        );
        Ok(())
    }
}

/// Safe summary shape shared by pages/export. No details, prompts or raw seq.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct OperationalLog {
    pub id: String,
    pub ts: String,
    pub severity: String,
    pub app: String,
    pub event_type: String,
    pub message: String,
    pub job_id: String,
    pub task_id: String,
    pub attempt_id: String,
    pub run_id: String,
}
impl OperationalLog {
    fn validate(&self) -> Result<()> {
        for (value, bound) in [
            (&self.id, 512),
            (&self.ts, 64),
            (&self.severity, 16),
            (&self.app, 64),
            (&self.event_type, 128),
            (&self.job_id, 512),
            (&self.task_id, 512),
            (&self.attempt_id, 512),
            (&self.run_id, 512),
        ] {
            ensure!(
                value.len() <= bound && !value.chars().any(char::is_control),
                "invalid bounded operational log metadata"
            );
        }
        ensure!(
            !self.id.is_empty() && !self.job_id.is_empty() && self.message.chars().count() <= 1000,
            "invalid scoped log identity or summary"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct SequencedLog {
    pub seq: i64,
    pub row: OperationalLog,
}
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ScopedLogCounts {
    pub total: u64,
    pub error: u64,
    pub warn: u64,
    pub info: u64,
    pub debug: u64,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ScopedLogPage {
    pub rows: Vec<SequencedLog>,
    pub counts: ScopedLogCounts,
    pub apps: Vec<String>,
    /// False when additional scoped apps exceed the 32-entry facet bound.
    pub apps_complete: bool,
    pub upper_seq: i64,
    pub next_boundary: Option<i64>,
    pub has_more: bool,
}
impl ScopedLogPage {
    pub fn empty() -> Self {
        Self {
            rows: Vec::new(),
            counts: ScopedLogCounts::default(),
            apps: Vec::new(),
            apps_complete: true,
            upper_seq: 0,
            next_boundary: None,
            has_more: false,
        }
    }
}

// The authorization cohort is always the first mandatory predicate. The public
// text filter searches only the projected summary, never hidden raw details.
const FILTER:&str="json_extract(body,'$.job_id') IN (SELECT value FROM json_each(?1))
    AND (?2 IS NULL OR CASE json_extract(body,'$.severity') WHEN 'debug' THEN 0 WHEN 'warn' THEN 2 WHEN 'error' THEN 3 ELSE 1 END>=?2)
    AND (?3='' OR json_extract(body,'$.app')=?3)
    AND (?4='' OR instr(substr(COALESCE(json_extract(body,'$.message'),''),1,1000),?4)>0)
    AND (?5='' OR ts>=?5)";
const FIELDS: &str = "seq,id,ts,
    COALESCE(json_extract(body,'$.severity'),'info'),
    COALESCE(json_extract(body,'$.app'),''),
    COALESCE(json_extract(body,'$.event_type'),''),
    substr(COALESCE(json_extract(body,'$.message'),''),1,1000),
    COALESCE(json_extract(body,'$.job_id'),''),
    COALESCE(json_extract(body,'$.task_id'),''),
    COALESCE(json_extract(body,'$.attempt_id'),''),
    COALESCE(json_extract(body,'$.run_id'),'')";
impl Store {
    /// One SQLite read transaction binds rows/counts/facets to the same scoped
    /// high-water snapshot. Filtering precedes every limit/count/order.
    pub async fn scoped_log_page(&self, read: ScopedLogRead) -> Result<ScopedLogPage> {
        read.validate()?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancel_on_drop = CancelRead(cancelled.clone());
        let deadline = Instant::now() + SCAN_BUDGET;
        let operation = self.with_connection(move |connection| {
            bounded_read(connection, cancelled, deadline, |connection| {
                let transaction = connection.transaction()?;
                let jobs = serde_json::to_string(&read.job_ids)?;
                let filters = &read.filters;
                let captured: i64 = transaction.query_row(
                    &format!("SELECT COALESCE(MAX(seq),0) FROM logs WHERE {FILTER}"),
                    params![
                        jobs,
                        filters.min_rank,
                        filters.app,
                        filters.text,
                        filters.since
                    ],
                    |row| row.get(0),
                )?;
                let upper = read.upper_seq.unwrap_or(captured);
                let (comparison, order) = match read.direction {
                    LogDirection::Older => ("<", "DESC"),
                    LogDirection::Newer => (">", "ASC"),
                };
                let mut query = transaction.prepare(&format!(
                    "SELECT {FIELDS} FROM logs WHERE {FILTER} AND seq<=?6 \
                 AND (?7 IS NULL OR seq {comparison} ?7) ORDER BY seq {order} LIMIT ?8"
                ))?;
                let rows = query.query_map(
                    params![
                        jobs,
                        filters.min_rank,
                        filters.app,
                        filters.text,
                        filters.since,
                        upper,
                        read.boundary,
                        read.limit as i64 + 1
                    ],
                    |row| {
                        Ok(SequencedLog {
                            seq: row.get(0)?,
                            row: OperationalLog {
                                id: row.get(1)?,
                                ts: row.get(2)?,
                                severity: row.get(3)?,
                                app: row.get(4)?,
                                event_type: row.get(5)?,
                                message: row.get(6)?,
                                job_id: row.get(7)?,
                                task_id: row.get(8)?,
                                attempt_id: row.get(9)?,
                                run_id: row.get(10)?,
                            },
                        })
                    },
                )?;
                let mut rows: Vec<SequencedLog> = rows.collect::<rusqlite::Result<_>>()?;
                for row in &rows {
                    row.row.validate()?;
                }
                let has_more = rows.len() > read.limit;
                rows.truncate(read.limit);
                let next_boundary = rows.last().map(|row| row.seq);
                drop(query);
                let mut query = transaction.prepare(&format!(
                    "SELECT CASE json_extract(body,'$.severity') WHEN 'error' THEN 'error' \
                 WHEN 'warn' THEN 'warn' WHEN 'debug' THEN 'debug' ELSE 'info' END,count(*) \
                 FROM logs WHERE {FILTER} AND seq<=?6 GROUP BY 1"
                ))?;
                let severities = query.query_map(
                    params![
                        jobs,
                        filters.min_rank,
                        filters.app,
                        filters.text,
                        filters.since,
                        upper
                    ],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                )?;
                let mut counts = ScopedLogCounts::default();
                for row in severities {
                    let (severity, count) = row?;
                    let count = u64::try_from(count).context("negative SQLite log count")?;
                    counts.total += count;
                    match severity.as_str() {
                        "error" => counts.error = count,
                        "warn" => counts.warn = count,
                        "debug" => counts.debug = count,
                        _ => counts.info = count,
                    }
                }
                drop(query);
                let mut query = transaction.prepare(&format!(
                    "SELECT DISTINCT json_extract(body,'$.app') AS app FROM logs WHERE {FILTER} \
                 AND seq<=?6 AND json_extract(body,'$.app')!='' ORDER BY app LIMIT 33"
                ))?;
                let mut apps = query
                    .query_map(
                        params![
                            jobs,
                            filters.min_rank,
                            filters.app,
                            filters.text,
                            filters.since,
                            upper
                        ],
                        |row| row.get::<_, String>(0),
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                for app in &apps {
                    ensure!(
                        app.len() <= 64 && !app.chars().any(char::is_control),
                        "invalid bounded log app facet"
                    );
                }
                let apps_complete = apps.len() <= 32;
                apps.truncate(32);
                drop(query);
                transaction.commit()?;
                Ok(ScopedLogPage {
                    rows,
                    counts,
                    apps,
                    apps_complete,
                    upper_seq: upper,
                    next_boundary,
                    has_more,
                })
            })
        });
        tokio::time::timeout(OWNER_BUDGET, operation)
            .await
            .context("scoped log owner deadline expired")?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn expensive_read(connection: &mut Connection) -> Result<()> {
        let _: i64 = connection.query_row(
            "WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<100000000) SELECT sum(x) FROM n",
            [],
            |row| row.get(0),
        )?;
        Ok(())
    }
    fn assert_interrupted(result: Result<()>) {
        let error = result.expect_err("the SQLite VM must actually stop");
        assert!(
            matches!(error.downcast_ref::<rusqlite::Error>(), Some(rusqlite::Error::SqliteFailure(code, _)) if code.code == rusqlite::ErrorCode::OperationInterrupted),
            "actual SQLite interruption: {error}"
        );
    }

    #[tokio::test]
    async fn scoped_app_facet_marks_overflow_without_truncating_counts_or_log_pages() -> Result<()>
    {
        let temporary = tempfile::tempdir()?;
        let store = Store::open(temporary.path().join("logs.sqlite3")).await?;
        for index in 0..32 {
            store.log_append(json!({"id":format!("allowed-{index}"),"ts":"2026-11-12T12:00:00Z","job_id":"allowed","severity":"info","app":format!("app-{index:02}"),"message":"safe summary"})).await?;
        }
        store.log_append(json!({"id":"foreign-facet","ts":"2026-11-12T12:00:00Z","job_id":"foreign","severity":"info","app":"foreign-app","message":"private summary"})).await?;
        let read = || {
            ScopedLogRead::new(
                vec!["allowed".into()],
                LogFilters::default(),
                LogDirection::Older,
                None,
                None,
                1,
            )
        };
        let complete = store.scoped_log_page(read()?).await?;
        assert_eq!(complete.counts.total, 32);
        assert_eq!(complete.rows.len(), 1);
        assert_eq!(complete.apps.len(), 32);
        assert!(
            complete.apps_complete,
            "foreign apps cannot change facet completeness"
        );
        store.log_append(json!({"id":"allowed-32","ts":"2026-11-12T12:00:00Z","job_id":"allowed","severity":"info","app":"app-32","message":"safe summary"})).await?;
        let overflow = store.scoped_log_page(read()?).await?;
        assert_eq!(
            overflow.counts.total, 33,
            "facet cap never truncates the scoped count"
        );
        assert_eq!(overflow.rows.len(), 1);
        assert!(overflow.has_more);
        assert_eq!(overflow.apps.len(), 32);
        assert!(!overflow.apps_complete);
        assert_eq!(overflow.apps.last().map(String::as_str), Some("app-31"));
        let mut filter = LogFilters::default();
        filter.app = "app-32".into();
        let narrowed = store
            .scoped_log_page(ScopedLogRead::new(
                vec!["allowed".into()],
                filter,
                LogDirection::Older,
                None,
                None,
                100,
            )?)
            .await?;
        assert_eq!(narrowed.counts.total, 1);
        assert_eq!(narrowed.rows[0].row.app, "app-32");
        assert_eq!(narrowed.apps, vec!["app-32"]);
        assert!(narrowed.apps_complete);
        Ok(())
    }

    #[tokio::test]
    async fn sqlite_scan_deadline_and_dropped_read_guard_interrupt_vm_and_restore_connection(
    ) -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let store = Store::open(temporary.path().join("logs.sqlite3")).await?;
        let expired = store
            .with_connection(|connection| {
                bounded_read(
                    connection,
                    Arc::new(AtomicBool::new(false)),
                    Instant::now() + Duration::from_millis(10),
                    expensive_read,
                )
            })
            .await;
        assert_interrupted(expired);
        let cancelled = Arc::new(AtomicBool::new(false));
        let guard = CancelRead(cancelled.clone());
        let (started, ready) = tokio::sync::oneshot::channel();
        let interrupt = store
            .with_connection(|connection| Ok(connection.get_interrupt_handle()))
            .await?;
        let worker = store.clone();
        let mut running = tokio::spawn(async move {
            worker
                .with_connection(move |connection| {
                    bounded_read(
                        connection,
                        cancelled,
                        Instant::now() + Duration::from_secs(30),
                        |connection| {
                            let _ = started.send(());
                            expensive_read(connection)
                        },
                    )
                })
                .await
        });
        ready.await?;
        drop(guard);
        let interrupted = match tokio::time::timeout(Duration::from_secs(2), &mut running).await {
            Ok(result) => result?,
            Err(_) => {
                // A regression must not strand its diagnostic query in the
                // blocking pool. This independent fallback is test-only.
                interrupt.interrupt();
                let _ = running.await;
                anyhow::bail!("dropped read guard did not stop the SQLite VM");
            }
        };
        assert_interrupted(interrupted);
        store.log_append(json!({"id":"after-interruption","ts":"2026-11-12T12:00:00Z","job_id":"allowed","message":"ordinary store work continues"})).await?;
        assert!(store.log_get("after-interruption".into()).await?.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn actual_sqlite_scope_precedes_count_limit_snapshot_follow_and_safe_export_projection(
    ) -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let path = temporary.path().join("logs.sqlite3");
        let store = Store::open(&path).await?;
        for (id, job, app, message) in [
            ("a-root", "a-root", "brain", "allowed older"),
            ("foreign", "b-root", "foreign-app", "foreign private"),
            ("system", "", "system", "global daemon"),
            ("a-child", "a-child", "tools", "allowed child"),
            ("shared", "mixed-root", "private-app", "shared private"),
            ("legacy", "legacy-unassigned", "legacy", "unassigned"),
        ] {
            store.log_append(json!({"id":id,"ts":"2026-11-12T12:00:00Z","job_id":job,"app":app,"severity":"info","message":message,"details":"hidden raw prompt"})).await?;
        }
        let ids = vec!["a-root".into(), "a-child".into()];
        let page = store
            .scoped_log_page(ScopedLogRead::new(
                ids.clone(),
                LogFilters::default(),
                LogDirection::Older,
                None,
                None,
                1,
            )?)
            .await?;
        assert_eq!(page.rows[0].row.id, "a-child");
        assert_eq!(page.counts.total, 2);
        assert!(page.has_more);
        assert_eq!(page.apps, vec!["brain", "tools"]);
        let projection = serde_json::to_value(&page.rows[0].row)?;
        assert!(projection.get("seq").is_none());
        assert!(projection.get("details").is_none());
        store.log_append(json!({"id":"new-a","ts":"2026-11-12T12:01:00Z","job_id":"a-root","app":"brain","severity":"error","message":"allowed new"})).await?;
        drop(store);
        let store = Store::open(&path).await?;
        let older = store
            .scoped_log_page(ScopedLogRead::new(
                ids.clone(),
                LogFilters::default(),
                LogDirection::Older,
                Some(page.upper_seq),
                page.next_boundary,
                1,
            )?)
            .await?;
        assert_eq!(older.rows[0].row.id, "a-root");
        assert_eq!(older.counts.total, 2);
        assert!(!older.has_more);
        let follow = store
            .scoped_log_page(ScopedLogRead::new(
                ids.clone(),
                LogFilters::default(),
                LogDirection::Newer,
                None,
                Some(page.upper_seq),
                100,
            )?)
            .await?;
        assert_eq!(follow.rows.len(), 1);
        assert_eq!(follow.rows[0].row.id, "new-a");
        assert_eq!(follow.counts.total, 3);
        let mut filter = LogFilters::default();
        filter.text = "foreign private".into();
        let private = store
            .scoped_log_page(ScopedLogRead::new(
                ids,
                filter,
                LogDirection::Older,
                None,
                None,
                100,
            )?)
            .await?;
        assert!(private.rows.is_empty());
        assert_eq!(private.counts.total, 0);
        assert!(private.apps.is_empty());
        // Retained/direct-store rows can exceed the normal EventRow append
        // bound. Their unprojected suffix cannot influence public search.
        store.log_append(json!({"id":"oversized-summary","ts":"2026-11-12T12:02:00Z","job_id":"a-root","app":"brain","severity":"info","message":format!("{}nonprojected-suffix", "a".repeat(1000))})).await?;
        let mut suffix_filter = LogFilters::default();
        suffix_filter.text = "nonprojected-suffix".into();
        let hidden_suffix = store
            .scoped_log_page(ScopedLogRead::new(
                vec!["a-root".into()],
                suffix_filter,
                LogDirection::Older,
                None,
                None,
                100,
            )?)
            .await?;
        assert!(hidden_suffix.rows.is_empty());
        assert_eq!(hidden_suffix.counts.total, 0);
        assert!(hidden_suffix.apps.is_empty());
        let projected = store
            .scoped_log_page(ScopedLogRead::new(
                vec!["a-root".into()],
                LogFilters::default(),
                LogDirection::Older,
                None,
                None,
                1,
            )?)
            .await?;
        assert_eq!(projected.rows[0].row.id, "oversized-summary");
        assert_eq!(projected.rows[0].row.message, "a".repeat(1000));
        assert!(ScopedLogRead::new(
            Vec::new(),
            LogFilters::default(),
            LogDirection::Older,
            None,
            None,
            100
        )
        .is_err());
        Ok(())
    }
}
