//! Read models and safe controls for the Jobs and Logs dashboards.
//!
//! Jobs is a view over `argos_jobs` / `argos_tasks` / `argos_attempts`, not a
//! new executor. Every query is bounded. Unknown historic timing stays `None`
//! ("Unavailable"), never zero. Retry only requeues failed tasks whose
//! operation is owned by a durable worker pool, so successful children are
//! never repeated and unsupported operations expose no Retry.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::events::{self, EventFilter, EventRow, NewEvent};
use crate::store::Store;
use crate::tasks;

/// Status filter for the Jobs table. `Default` lists active work first, then
/// recent history.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobStatusFilter {
    #[default]
    All,
    Active,
    Retrying,
    Failed,
    Completed,
}

impl JobStatusFilter {
    pub const ALL: [Self; 5] = [
        Self::All,
        Self::Active,
        Self::Retrying,
        Self::Failed,
        Self::Completed,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Active => "Active",
            Self::Retrying => "Retrying",
            Self::Failed => "Failed",
            Self::Completed => "Completed",
        }
    }

    fn states(self) -> &'static [&'static str] {
        match self {
            Self::All => &[],
            Self::Active => &["running", "queued", "retry_scheduled", "paused"],
            Self::Retrying => &["retry_scheduled"],
            Self::Failed => &["failed", "partial", "cancelled"],
            Self::Completed => &["completed"],
        }
    }
}

/// Filters for [`list_jobs`]. Empty strings do not filter.
#[derive(Clone, Debug, Default)]
pub struct JobFilter {
    pub status: JobStatusFilter,
    pub app: String,
    pub operation: String,
    pub text: String,
    /// RFC 3339 lower bound on `created_at` (time range).
    pub since: String,
}

/// One row of the Jobs table (and the head of the detail panel).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRow {
    pub id: String,
    pub parent_id: String,
    pub kind: String,
    pub app: String,
    pub operation: String,
    pub title: String,
    pub state: String,
    pub phase: String,
    pub progress_done: Option<i64>,
    pub progress_total: Option<i64>,
    pub created_at: String,
    pub queued_at: String,
    pub started_at: String,
    pub heartbeat_at: String,
    pub finished_at: String,
    pub active_ms: Option<i64>,
    pub queue_ms: Option<i64>,
    pub retry_wait_ms: Option<i64>,
    pub attempts_used: i64,
    pub attempt_cap: i64,
    pub provider: String,
    pub model: String,
    pub tool: String,
    pub worker_owner: String,
    pub run_ref: String,
    pub resource_ref: String,
    pub result_ref: String,
    pub error_category: String,
    pub error_summary: String,
    pub correlation_id: String,
    pub stage_coverage_json: String,
    /// Child jobs directly under this one.
    pub children: i64,
    /// Durable events correlated with this job or its descendants.
    pub events: i64,
    /// Attempts recorded for this job's tasks (open or closed).
    pub attempt_rows: i64,
    /// Earliest start of an attempt that is still open (`""` when none).
    pub open_attempt_started: String,
    /// Registry jobs: start of the running span ('' when not running).
    pub active_since: String,
    pub cancel_requested: bool,
    pub cancellable: bool,
}

impl JobRow {
    pub fn is_active(&self) -> bool {
        matches!(
            self.state.as_str(),
            "running" | "queued" | "retry_scheduled" | "paused"
        )
    }

    pub fn is_service(&self) -> bool {
        self.kind == "service"
    }

    /// Display label for the state; retry wait is distinct from running.
    pub fn state_label(&self) -> &'static str {
        match self.state.as_str() {
            "running" if self.is_service() => "service",
            "running" => "running",
            "queued" => "queued",
            "retry_scheduled" => "retry wait",
            "paused" => "paused",
            "completed" => "completed",
            "partial" => "partial",
            "failed" => "failed",
            "cancelled" => "cancelled",
            "blocked" => "blocked",
            _ => "unknown",
        }
    }

    /// Active (working) time as of `now`.
    ///
    /// Jobs whose work runs as task attempts sum the closed attempts' durable
    /// `active_ms` and add the live span of any attempt still open. A job that
    /// runs in-process without task attempts counts its whole run
    /// (start → finish, or `now` while running). `None` ("Unavailable") only
    /// when there is no start timestamp at all.
    pub fn active_now(&self, now: chrono::DateTime<chrono::Utc>) -> Option<i64> {
        if self.attempt_rows > 0 {
            let open = parse(&self.open_attempt_started)
                .map(|start| (now - start).num_milliseconds().max(0))
                .unwrap_or(0);
            return Some(self.active_ms.unwrap_or(0) + open);
        }
        // Registry jobs: durable active time of earlier spans plus the live span.
        if let Some(since) = parse(&self.active_since) {
            return Some(self.active_ms.unwrap_or(0) + (now - since).num_milliseconds().max(0));
        }
        if self.kind == crate::job_registry::KIND {
            if let Some(stored) = self.active_ms {
                return Some(stored);
            }
        }
        if let Some(stored) = self.active_ms.filter(|ms| *ms > 0) {
            return Some(stored);
        }
        let start = parse(&self.started_at)?;
        let end = parse(&self.finished_at).unwrap_or(now);
        Some((end - start).num_milliseconds().max(0))
    }

    /// Total elapsed time from start (or queue) to finish (or `now`). `None`
    /// when the job never recorded a start ("Unavailable").
    pub fn elapsed_ms(&self, now: chrono::DateTime<chrono::Utc>) -> Option<i64> {
        let start = parse(&self.started_at).or_else(|| parse(&self.queued_at))?;
        let end = parse(&self.finished_at).unwrap_or(now);
        Some((end - start).num_milliseconds().max(0))
    }
}

fn parse(ts: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    if ts.is_empty() {
        return None;
    }
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|t| t.with_timezone(&chrono::Utc))
}

/// Human duration: `Unavailable` for unknown timing, never zero-filled.
pub fn format_duration(ms: Option<i64>) -> String {
    let Some(ms) = ms else {
        return "Unavailable".into();
    };
    let secs = ms / 1000;
    if secs < 1 {
        format!("{ms}ms")
    } else if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Header counts for the Jobs dashboard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobCounts {
    pub active: i64,
    pub queued: i64,
    pub retrying: i64,
    pub failed: i64,
    /// Completed within the last 24 hours.
    pub completed_recent: i64,
}

/// One task under a job (detail task tree).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRow {
    pub id: String,
    pub operation: String,
    pub pool: String,
    pub state: String,
    pub attempts: i64,
    pub max_attempts: i64,
    pub active_ms: i64,
    pub queue_ms: Option<i64>,
    pub retry_wait_ms: i64,
    pub next_eligible_at: String,
    pub error_category: String,
    pub error_message: String,
    pub result_ref: String,
    pub updated_at: String,
}

/// One attempt (detail attempt history).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptRow {
    pub id: String,
    pub task_id: String,
    pub attempt_number: i64,
    pub owner: String,
    pub started_at: String,
    pub finished_at: String,
    pub duration_ms: Option<i64>,
    pub outcome: String,
    pub error_category: String,
    pub error_message: String,
}

/// Detail panel model.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobDetail {
    pub job: JobRow,
    pub children: Vec<JobRow>,
    /// Most recent tasks first, bounded.
    pub tasks: Vec<TaskRow>,
    /// Total tasks (the list above may be truncated).
    pub task_total: i64,
    pub attempts: Vec<AttemptRow>,
    /// Failed tasks a durable pool can safely rerun.
    pub retryable: i64,
    /// The job finished more than the event retention ago and no events remain.
    pub events_expired: bool,
}

const JOB_COLUMNS: &str =
    "j.id, j.parent_id, j.kind, j.app, j.operation, j.title, j.state, j.phase,
    j.progress_done, j.progress_total, j.created_at, j.queued_at, j.started_at, j.heartbeat_at,
    j.finished_at, j.active_ms, j.queue_ms, j.retry_wait_ms, j.attempts_used, j.attempt_cap,
    j.provider, j.model, j.tool, j.worker_owner, j.run_ref, j.resource_ref, j.result_ref,
    j.error_category, j.error_summary, j.correlation_id, j.stage_coverage_json,
    (SELECT COUNT(*) FROM argos_jobs c WHERE c.parent_id = j.id),
    (SELECT COUNT(*) FROM argos_events e WHERE e.job_id = j.id
        OR e.job_id IN (SELECT c.id FROM argos_jobs c WHERE c.parent_id = j.id)),
    (SELECT COUNT(*) FROM argos_attempts a JOIN argos_tasks t ON t.id = a.task_id
        WHERE t.job_id = j.id),
    IFNULL((SELECT MIN(a.started_at) FROM argos_attempts a JOIN argos_tasks t ON t.id = a.task_id
        WHERE t.job_id = j.id AND a.finished_at = ''), ''),
    j.active_since, j.cancel_requested, j.cancellable";

fn job_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<JobRow> {
    Ok(JobRow {
        id: r.get(0)?,
        parent_id: r.get(1)?,
        kind: r.get(2)?,
        app: r.get(3)?,
        operation: r.get(4)?,
        title: r.get(5)?,
        state: r.get(6)?,
        phase: r.get(7)?,
        progress_done: r.get(8)?,
        progress_total: r.get(9)?,
        created_at: r.get(10)?,
        queued_at: r.get(11)?,
        started_at: r.get(12)?,
        heartbeat_at: r.get(13)?,
        finished_at: r.get(14)?,
        active_ms: r.get(15)?,
        queue_ms: r.get(16)?,
        retry_wait_ms: r.get(17)?,
        attempts_used: r.get(18)?,
        attempt_cap: r.get(19)?,
        provider: r.get(20)?,
        model: r.get(21)?,
        tool: r.get(22)?,
        worker_owner: r.get(23)?,
        run_ref: r.get(24)?,
        resource_ref: r.get(25)?,
        result_ref: r.get(26)?,
        error_category: r.get(27)?,
        error_summary: r.get(28)?,
        correlation_id: r.get(29)?,
        stage_coverage_json: r.get(30)?,
        children: r.get(31)?,
        events: r.get(32)?,
        attempt_rows: r.get(33)?,
        open_attempt_started: r.get(34)?,
        active_since: r.get(35)?,
        cancel_requested: r.get(36)?,
        cancellable: r.get(37)?,
    })
}

/// Top-level jobs, active work first, then most recent. Bounded by `limit`.
pub fn list_jobs(conn: &Connection, filter: &JobFilter, limit: usize) -> Result<Vec<JobRow>> {
    let mut sql = format!("SELECT {JOB_COLUMNS} FROM argos_jobs j WHERE j.parent_id = ''");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    let states = filter.status.states();
    if !states.is_empty() {
        let marks = vec!["?"; states.len()].join(",");
        sql.push_str(&format!(" AND j.state IN ({marks})"));
        for state in states {
            args.push(Box::new(state.to_string()));
        }
    }
    if !filter.app.is_empty() {
        sql.push_str(" AND j.app = ?");
        args.push(Box::new(filter.app.clone()));
    }
    if !filter.operation.is_empty() {
        sql.push_str(" AND j.operation = ?");
        args.push(Box::new(filter.operation.clone()));
    }
    if !filter.text.trim().is_empty() {
        sql.push_str(
            " AND (j.title LIKE ? OR j.id LIKE ? OR j.operation LIKE ? OR j.error_summary LIKE ?)",
        );
        let needle = format!("%{}%", filter.text.trim().replace('%', ""));
        for _ in 0..4 {
            args.push(Box::new(needle.clone()));
        }
    }
    if !filter.since.is_empty() {
        sql.push_str(" AND j.created_at >= ?");
        args.push(Box::new(filter.since.clone()));
    }
    sql.push_str(
        " ORDER BY CASE WHEN j.state IN ('running','queued','retry_scheduled','paused') AND j.kind <> 'service' THEN 0
                        WHEN j.kind = 'service' THEN 2 ELSE 1 END,
                   j.updated_at DESC, j.id
          LIMIT ?",
    );
    args.push(Box::new(limit.max(1) as i64));
    let mut stmt = conn.prepare(&sql)?;
    let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
    let rows = stmt
        .query_map(refs.as_slice(), job_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Header counts (top-level, non-service jobs).
pub fn job_counts(conn: &Connection) -> Result<JobCounts> {
    let since = (chrono::Utc::now() - chrono::Duration::hours(24)).to_rfc3339();
    Ok(conn.query_row(
        "SELECT
            IFNULL(SUM(state IN ('running','paused')),0),
            IFNULL(SUM(state='queued'),0),
            IFNULL(SUM(state='retry_scheduled'),0),
            IFNULL(SUM(state IN ('failed','partial')),0),
            IFNULL(SUM(state='completed' AND finished_at >= ?1),0)
         FROM argos_jobs WHERE parent_id='' AND kind<>'service'",
        [since],
        |r| {
            Ok(JobCounts {
                active: r.get(0)?,
                queued: r.get(1)?,
                retrying: r.get(2)?,
                failed: r.get(3)?,
                completed_recent: r.get(4)?,
            })
        },
    )?)
}

/// One job by id (any depth), regardless of filters.
pub fn get_job(conn: &Connection, id: &str) -> Result<Option<JobRow>> {
    Ok(conn
        .query_row(
            &format!("SELECT {JOB_COLUMNS} FROM argos_jobs j WHERE j.id = ?1"),
            [id],
            job_row,
        )
        .optional()?)
}

fn retryable_sql() -> String {
    let ops = tasks::INDEX_OPERATIONS
        .iter()
        .chain(tasks::SUMMARY_OPERATIONS.iter())
        .map(|op| format!("'{op}'"))
        .collect::<Vec<_>>()
        .join(",");
    format!("state='failed' AND operation IN ({ops})")
}

/// Detail panel: children, bounded task tree and attempt history.
pub fn job_detail(conn: &Connection, id: &str, task_limit: usize) -> Result<Option<JobDetail>> {
    let Some(job) = get_job(conn, id)? else {
        return Ok(None);
    };
    let children = {
        let mut stmt = conn.prepare(&format!(
            "SELECT {JOB_COLUMNS} FROM argos_jobs j WHERE j.parent_id = ?1 ORDER BY j.created_at, j.id LIMIT 50"
        ))?;
        let rows = stmt
            .query_map([id], job_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let task_total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM argos_tasks WHERE job_id=?1",
        [id],
        |r| r.get(0),
    )?;
    let tasks = {
        let mut stmt = conn.prepare(
            "SELECT id, operation, pool, state, attempts, max_attempts, active_ms, queue_ms, retry_wait_ms,
                    next_eligible_at, error_category, error_message, result_ref, updated_at
             FROM argos_tasks WHERE job_id=?1
             ORDER BY CASE WHEN state IN ('running','queued','retry_scheduled','failed') THEN 0 ELSE 1 END,
                      updated_at DESC, id
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![id, task_limit.max(1) as i64], |r| {
                Ok(TaskRow {
                    id: r.get(0)?,
                    operation: r.get(1)?,
                    pool: r.get(2)?,
                    state: r.get(3)?,
                    attempts: r.get(4)?,
                    max_attempts: r.get(5)?,
                    active_ms: r.get(6)?,
                    queue_ms: r.get(7)?,
                    retry_wait_ms: r.get(8)?,
                    next_eligible_at: r.get(9)?,
                    error_category: r.get(10)?,
                    error_message: r.get(11)?,
                    result_ref: r.get(12)?,
                    updated_at: r.get(13)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let attempts = {
        let mut stmt = conn.prepare(
            "SELECT a.id, a.task_id, a.attempt_number, a.owner, a.started_at, a.finished_at, a.duration_ms,
                    a.outcome, a.error_category, a.error_message
             FROM argos_attempts a JOIN argos_tasks t ON t.id = a.task_id
             WHERE t.job_id = ?1 ORDER BY a.started_at DESC, a.id LIMIT 20",
        )?;
        let rows = stmt
            .query_map([id], |r| {
                Ok(AttemptRow {
                    id: r.get(0)?,
                    task_id: r.get(1)?,
                    attempt_number: r.get(2)?,
                    owner: r.get(3)?,
                    started_at: r.get(4)?,
                    finished_at: r.get(5)?,
                    duration_ms: r.get(6)?,
                    outcome: r.get(7)?,
                    error_category: r.get(8)?,
                    error_message: r.get(9)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let retryable: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM argos_tasks WHERE job_id=?1 AND {}",
            retryable_sql()
        ),
        [id],
        |r| r.get(0),
    )?;
    let cutoff = chrono::Utc::now() - chrono::Duration::hours(events::DEFAULT_RETENTION_HOURS);
    let events_expired = job.events == 0
        && parse(&job.finished_at)
            .or_else(|| parse(&job.created_at))
            .is_some_and(|t| t < cutoff);
    Ok(Some(JobDetail {
        job,
        children,
        tasks,
        task_total,
        attempts,
        retryable,
        events_expired,
    }))
}

/// Requeue a job's failed tasks that a durable worker pool owns (index and
/// summary work). Completed tasks are untouched, so successful work is never
/// repeated. Returns how many tasks were requeued.
pub fn retry_failed_tasks(conn: &Connection, id: &str) -> Result<usize> {
    let now = chrono::Utc::now().to_rfc3339();
    let n = conn.execute(
        &format!(
            "UPDATE argos_tasks SET state='queued', attempts=0, next_eligible_at='', lease_owner='',
                lease_until='', error_category='', error_message='', finished_at='', updated_at=?2
             WHERE job_id=?1 AND {}",
            retryable_sql()
        ),
        params![id, now],
    )?;
    if n > 0 {
        conn.execute(
            "UPDATE OR IGNORE argos_index_changes SET state='pending', error_category='', error_message='', updated_at=?2
             WHERE state='failed' AND task_id IN (SELECT id FROM argos_tasks WHERE job_id=?1 AND state='queued')",
            params![id, now],
        )?;
        tasks::refresh_job_state(conn, id, &now)?;
        let _ = events::record_event(
            conn,
            &NewEvent {
                app: "jobs".into(),
                event_type: "job.retry".into(),
                message: format!("Retry requested for {n} failed task(s)"),
                job_id: id.into(),
                ..Default::default()
            },
        );
    }
    Ok(n)
}

/// Bounded `(app, count)` list of apps that own jobs, for the app filter.
pub fn job_apps(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT app FROM argos_jobs WHERE app<>'' AND parent_id='' ORDER BY app LIMIT 32",
    )?;
    let rows = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Severity counts and recent failures for the Logs header.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventCounts {
    pub error: i64,
    pub warn: i64,
    pub info: i64,
    pub debug: i64,
    /// Errors in the last hour.
    pub recent_failures: i64,
}

pub fn event_counts(conn: &Connection) -> Result<EventCounts> {
    let mut counts = EventCounts::default();
    for (severity, n) in events::severity_counts(conn)? {
        match severity.as_str() {
            "error" => counts.error += n,
            "warn" => counts.warn += n,
            "debug" => counts.debug += n,
            _ => counts.info += n,
        }
    }
    let hour = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
    counts.recent_failures = conn.query_row(
        "SELECT COUNT(*) FROM argos_events WHERE severity='error' AND ts >= ?1",
        [hour],
        |r| r.get(0),
    )?;
    Ok(counts)
}

/// Distinct apps that wrote events, for the Logs app filter.
pub fn event_apps(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT DISTINCT app FROM argos_events WHERE app<>'' ORDER BY app LIMIT 32")?;
    let rows = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Store-level access for the TUI (which has no raw connection).
impl Store {
    pub fn list_jobs(&self, filter: &JobFilter, limit: usize) -> Result<Vec<JobRow>> {
        list_jobs(&self.conn, filter, limit)
    }

    pub fn job_counts(&self) -> Result<JobCounts> {
        job_counts(&self.conn)
    }

    pub fn job_detail(&self, id: &str, task_limit: usize) -> Result<Option<JobDetail>> {
        job_detail(&self.conn, id, task_limit)
    }

    pub fn get_job(&self, id: &str) -> Result<Option<JobRow>> {
        get_job(&self.conn, id)
    }

    pub fn retry_failed_tasks(&self, id: &str) -> Result<usize> {
        retry_failed_tasks(&self.conn, id)
    }
    pub fn set_job_stage_coverage(&self, job_id: &str, coverage_json: &str) -> Result<()> {
        crate::tasks::set_job_stage_coverage(&self.conn, job_id, coverage_json)
    }


    pub fn job_apps(&self) -> Result<Vec<String>> {
        job_apps(&self.conn)
    }

    /// Jobs → Cancel: cooperative stop request (see [`crate::job_registry::request_cancel`]).
    pub fn request_job_cancel(&self, id: &str) -> Result<crate::job_registry::CancelRequest> {
        crate::job_registry::request_cancel(&self.conn, id)
    }

    /// Register a user-visible job before launching its work (idempotent).
    pub fn register_job(&self, job: &tasks::NewJob, meta: &tasks::JobMeta) -> Result<()> {
        tasks::enqueue_job_with(&self.conn, job, meta, &chrono::Utc::now().to_rfc3339())
    }

    /// Lifecycle/progress for a task-less job (see [`tasks::set_job_progress`]).
    pub fn set_job_progress(
        &self,
        id: &str,
        state: &str,
        phase: &str,
        done: i64,
        total: Option<i64>,
        error_summary: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        tasks::set_job_progress(
            &self.conn,
            id,
            state,
            phase,
            done,
            total,
            error_summary,
            &now,
        )
    }

    pub fn record_event(&self, event: &NewEvent) -> Result<String> {
        events::record_event(&self.conn, event)
    }

    pub fn list_events(&self, filter: &EventFilter, limit: usize) -> Result<Vec<EventRow>> {
        events::list_events(&self.conn, filter, limit)
    }

    pub fn get_event(&self, id: &str) -> Result<Option<EventRow>> {
        events::get_event(&self.conn, id)
    }

    pub fn event_counts(&self) -> Result<EventCounts> {
        event_counts(&self.conn)
    }

    pub fn event_apps(&self) -> Result<Vec<String>> {
        event_apps(&self.conn)
    }

    /// Delete events past retention. Touches only `argos_events`.
    pub fn prune_events(&self, hours: i64) -> Result<usize> {
        events::prune_events(&self.conn, hours)
    }

    /// Logs "Clear events". Touches only `argos_events`: jobs, task results and
    /// memories are kept.
    pub fn clear_events(&self) -> Result<usize> {
        events::clear_events(&self.conn)
    }
}

/// Fixtures for the binary's tests and screenshot dumps (feature `fixtures`).
#[cfg(any(test, feature = "fixtures"))]
impl Store {
    /// Enqueue one task under `job_id` and record its attempt as started
    /// `started_secs_ago`; the attempt stays open unless `ran_secs` is given.
    pub fn fixture_task_attempt(
        &self,
        job_id: &str,
        task_id: &str,
        operation: &str,
        started_secs_ago: i64,
        ran_secs: Option<i64>,
    ) -> Result<()> {
        let start = chrono::Utc::now() - chrono::Duration::seconds(started_secs_ago);
        tasks::enqueue_task(
            &self.conn,
            &tasks::NewTask {
                id: task_id.into(),
                job_id: job_id.into(),
                operation: operation.into(),
                dedupe_key: String::new(),
                priority: 100,
                input_ref: String::new(),
                input_hash: String::new(),
                source_revision: String::new(),
                role_snapshot: String::new(),
                max_attempts: 3,
            },
            &start.to_rfc3339(),
        )?;
        let pool = tasks::pool_for_operation(operation);
        let claimed =
            tasks::claim_next_in(&self.conn, &[pool], "fixture", 3600, &start.to_rfc3339())?
                .ok_or_else(|| anyhow::anyhow!("fixture task was not claimable"))?;
        if let Some(ran) = ran_secs {
            let end = start + chrono::Duration::seconds(ran);
            tasks::complete_claimed(&self.conn, &claimed, "ok", &end.to_rfc3339())?;
        }
        tasks::refresh_job_state(&self.conn, job_id, &chrono::Utc::now().to_rfc3339())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::{ErrorCategory, JobMeta, NewJob, NewTask, OperationKind, TaskError};

    fn now() -> String {
        chrono::Utc::now().to_rfc3339()
    }

    fn job(conn: &Connection, id: &str, parent: &str, app: &str, title: &str) {
        tasks::enqueue_job_with(
            conn,
            &NewJob {
                id: id.into(),
                kind: "user".into(),
                owner_scope: String::new(),
                input_revision: String::new(),
                deadline_at: String::new(),
            },
            &JobMeta {
                parent_id: parent.into(),
                app: app.into(),
                operation: "test_op".into(),
                title: title.into(),
                ..Default::default()
            },
            &now(),
        )
        .unwrap();
    }

    /// Enqueue one single-attempt task and drive it to `outcome`
    /// ("completed", "failed" or "queued").
    fn task(conn: &Connection, job_id: &str, id: &str, operation: &str, outcome: &str) {
        tasks::enqueue_task(
            conn,
            &NewTask {
                id: id.into(),
                job_id: job_id.into(),
                operation: operation.into(),
                dedupe_key: String::new(),
                priority: 100,
                input_ref: String::new(),
                input_hash: String::new(),
                source_revision: String::new(),
                role_snapshot: String::new(),
                max_attempts: 1,
            },
            &now(),
        )
        .unwrap();
        if outcome == "queued" {
            tasks::refresh_job_state(conn, job_id, &now()).unwrap();
            return;
        }
        let pool = tasks::pool_for_operation(operation);
        let claimed = tasks::claim_next_in(conn, &[pool], "worker", 30, &now())
            .unwrap()
            .expect("claimed");
        assert_eq!(claimed.id, id);
        if outcome == "completed" {
            tasks::complete_claimed(conn, &claimed, "ok", &now()).unwrap();
        } else {
            tasks::fail_claimed(
                conn,
                &claimed,
                OperationKind::Summarization,
                &TaskError::new(ErrorCategory::AuthOrQuota, "provider said no"),
                &now(),
            )
            .unwrap();
        }
    }

    #[test]
    fn jobs_list_active_first_with_counts_filters_detail_and_retry() {
        let store = Store::memory().unwrap();
        let conn = &store.conn;
        job(conn, "j-done", "", "atlas", "Done cycle");
        task(conn, "j-done", "t-done", "summarization", "completed");
        job(conn, "j-part", "", "brain", "Graph summary");
        task(conn, "j-part", "t-ok", "summarization", "completed");
        task(conn, "j-part", "t-bad", "summarization", "failed");
        job(conn, "j-net", "", "tools", "Lookup");
        task(conn, "j-net", "t-net", "osint_collect", "failed");
        job(conn, "j-wait", "", "atlas", "Queued cycle");
        task(conn, "j-wait", "t-wait", "summarization", "queued");
        tasks::ensure_service_job(conn, "svc-x", "index", "Local index", &now()).unwrap();
        job(conn, "j-child", "j-part", "brain", "Phase");

        let rows = store.list_jobs(&JobFilter::default(), 50).unwrap();
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids[0], "j-wait", "active work first: {ids:?}");
        assert_eq!(*ids.last().unwrap(), "svc-x", "service health last");
        assert!(
            !ids.contains(&"j-child"),
            "children live under their parent"
        );
        let part = rows.iter().find(|r| r.id == "j-part").unwrap();
        assert_eq!(part.state, "partial");
        assert_eq!(part.children, 1);

        let counts = store.job_counts().unwrap();
        assert_eq!(counts.queued, 1);
        assert_eq!(counts.failed, 2);
        assert_eq!(counts.completed_recent, 1);

        let failed = store
            .list_jobs(
                &JobFilter {
                    status: JobStatusFilter::Failed,
                    ..Default::default()
                },
                50,
            )
            .unwrap();
        assert_eq!(failed.len(), 2);
        let text = store
            .list_jobs(
                &JobFilter {
                    text: "graph".into(),
                    ..Default::default()
                },
                50,
            )
            .unwrap();
        assert_eq!(text.len(), 1);

        let detail = store.job_detail("j-part", 10).unwrap().unwrap();
        assert_eq!(detail.task_total, 2);
        assert_eq!(detail.attempts.len(), 2);
        assert_eq!(detail.children.len(), 1);
        assert_eq!(detail.retryable, 1);
        assert!(detail.job.error_summary.contains("provider said no"));
        // A network lookup has no durable worker: no misleading Retry.
        assert_eq!(store.job_detail("j-net", 10).unwrap().unwrap().retryable, 0);
        assert_eq!(store.retry_failed_tasks("j-net").unwrap(), 0);

        // Retry requeues only the failed child task; completed work stays.
        assert_eq!(store.retry_failed_tasks("j-part").unwrap(), 1);
        let states: Vec<(String, String, i64)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, state, attempts FROM argos_tasks WHERE job_id='j-part' ORDER BY id",
                )
                .unwrap();
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            rows
        };
        assert_eq!(
            states,
            vec![
                ("t-bad".to_string(), "queued".to_string(), 0),
                ("t-ok".to_string(), "completed".to_string(), 1)
            ]
        );
        assert_eq!(store.get_job("j-part").unwrap().unwrap().state, "queued");
        // The retry is visible in Logs, correlated with the job.
        let events = store
            .list_events(
                &EventFilter {
                    job_id: "j-part".into(),
                    ..Default::default()
                },
                10,
            )
            .unwrap();
        assert!(events.iter().any(|e| e.event_type == "job.retry"));
    }

    #[test]
    fn active_time_is_live_for_open_attempts_and_unavailable_only_without_a_start() {
        let store = Store::memory().unwrap();
        let conn = &store.conn;
        let t0 = chrono::Utc::now() - chrono::Duration::seconds(90);
        job(conn, "j-live", "", "atlas", "Atlas news cycle");
        tasks::enqueue_task(
            conn,
            &NewTask {
                id: "t-live".into(),
                job_id: "j-live".into(),
                operation: "index_upsert".into(),
                dedupe_key: String::new(),
                priority: 100,
                input_ref: String::new(),
                input_hash: String::new(),
                source_revision: String::new(),
                role_snapshot: String::new(),
                max_attempts: 3,
            },
            &t0.to_rfc3339(),
        )
        .unwrap();
        let pool = tasks::pool_for_operation("index_upsert");
        let claimed = tasks::claim_next_in(conn, &[pool], "worker", 600, &t0.to_rfc3339())
            .unwrap()
            .expect("claimed");
        tasks::refresh_job_state(conn, "j-live", &t0.to_rfc3339()).unwrap();
        let row = store.get_job("j-live").unwrap().unwrap();
        assert_eq!(row.attempt_rows, 1);
        assert!(!row.open_attempt_started.is_empty());
        let live = row
            .active_now(chrono::Utc::now())
            .expect("live active time");
        assert!((89_000..120_000).contains(&live), "live span {live}");
        // Closing the attempt freezes the durable duration.
        let t1 = (t0 + chrono::Duration::seconds(30)).to_rfc3339();
        tasks::complete_claimed(conn, &claimed, "ok", &t1).unwrap();
        tasks::refresh_job_state(conn, "j-live", &t1).unwrap();
        let row = store.get_job("j-live").unwrap().unwrap();
        assert_eq!(row.active_now(chrono::Utc::now()), Some(30_000));
        // In-process job without task attempts: its run is the active span.
        job(conn, "j-inproc", "", "brain", "Graph summary");
        store
            .set_job_progress("j-inproc", "running", "explanation", 0, None, "")
            .unwrap();
        let row = store.get_job("j-inproc").unwrap().unwrap();
        assert!(row.active_now(chrono::Utc::now()).is_some());
        // Never started: Unavailable, not zero.
        job(conn, "j-queued", "", "intel", "Queued");
        let row = store.get_job("j-queued").unwrap().unwrap();
        assert_eq!(row.active_now(chrono::Utc::now()), None);
        assert_eq!(
            format_duration(row.active_now(chrono::Utc::now())),
            "Unavailable"
        );
    }

    #[test]
    fn timing_is_unavailable_not_zero_and_expired_event_detail_is_explained() {
        let store = Store::memory().unwrap();
        let conn = &store.conn;
        let old = (chrono::Utc::now() - chrono::Duration::hours(72)).to_rfc3339();
        tasks::enqueue_job_with(
            conn,
            &NewJob {
                id: "j-old".into(),
                kind: "user".into(),
                owner_scope: String::new(),
                input_revision: String::new(),
                deadline_at: String::new(),
            },
            &JobMeta {
                app: "atlas".into(),
                title: "Old cycle".into(),
                ..Default::default()
            },
            &old,
        )
        .unwrap();
        let row = store.get_job("j-old").unwrap().unwrap();
        assert_eq!(row.active_ms, None);
        assert_eq!(format_duration(row.active_ms), "Unavailable");
        assert_eq!(format_duration(row.queue_ms), "Unavailable");
        assert_eq!(format_duration(Some(65_000)), "1m 05s");
        let detail = store.job_detail("j-old", 10).unwrap().unwrap();
        assert!(
            detail.events_expired,
            "no events past retention is explained"
        );

        job(conn, "j-new", "", "atlas", "New cycle");
        job(conn, "j-new-phase", "j-new", "atlas", "Phase");
        store
            .record_event(&NewEvent {
                app: "atlas".into(),
                message: "phase note".into(),
                job_id: "j-new-phase".into(),
                ..Default::default()
            })
            .unwrap();
        let fresh = store.job_detail("j-new", 10).unwrap().unwrap();
        assert_eq!(
            fresh.job.events, 1,
            "descendant events count toward the job"
        );
        assert!(!fresh.events_expired);
        let counts = store.event_counts().unwrap();
        assert_eq!(counts.info, 1);
        assert_eq!(store.event_apps().unwrap(), vec!["atlas".to_string()]);
        // Clearing events never deletes jobs.
        assert_eq!(store.clear_events().unwrap(), 1);
        assert!(store.get_job("j-new").unwrap().is_some());
    }
}
