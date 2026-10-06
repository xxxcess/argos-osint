//! Shared registration for in-process asynchronous operations (spec §5.1).
//!
//! Durable task workers already record jobs, tasks and attempts. Operations
//! that run in-process (Atlas cycles, Recon investigations, tool runs, graph
//! explanations, provider checks, …) register here instead. Registration is
//! persisted before the work launches, timing comes from a monotonic clock,
//! and every handle reaches a terminal or recoverable state:
//!
//! - an explicit [`JobHandle::finish`] (completed, partial, failed, paused,
//!   cancelled);
//! - `Drop` without finishing (panic, abort, early return) records a failure;
//! - a process that exits is detected by [`recover_orphans`] through the
//!   per-process heartbeat row, and its open jobs become interrupted failures.
//!
//! Cancellation is cooperative. [`request_cancel`] only sets a flag that the
//! operation observes; the job reads "cancelled" once the operation stops.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::events::{self, NewEvent, Severity};

/// Job kind for registry-owned rows (task-backed jobs keep their own kinds).
pub const KIND: &str = "inproc";
/// A process whose heartbeat is older than this is treated as gone.
pub const PROCESS_STALE_SECS: i64 = 30;
/// How often the process heartbeat runs.
pub const BEAT_INTERVAL: Duration = Duration::from_secs(5);

/// What to register. `id` defaults to a fresh unique id.
#[derive(Clone, Debug, Default)]
pub struct JobSpec {
    pub id: String,
    pub parent_id: String,
    pub app: String,
    pub operation: String,
    pub title: String,
    pub run_ref: String,
    pub resource_ref: String,
    pub provider: String,
    pub model: String,
    pub tool: String,
    pub correlation_id: String,
    /// Cancel is safe at any point (no half-committed side effects).
    pub cancellable: bool,
}

impl JobSpec {
    pub fn new(app: &str, operation: &str, title: impl Into<String>) -> Self {
        Self {
            id: new_job_id(operation),
            app: app.into(),
            operation: operation.into(),
            title: title.into(),
            ..Default::default()
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    pub fn run(mut self, run_ref: impl Into<String>) -> Self {
        self.run_ref = run_ref.into();
        self
    }

    pub fn resource(mut self, resource_ref: impl Into<String>) -> Self {
        self.resource_ref = resource_ref.into();
        self
    }

    pub fn tool(mut self, tool: impl Into<String>) -> Self {
        self.tool = tool.into();
        self
    }

    pub fn model(mut self, provider: impl Into<String>, model: impl Into<String>) -> Self {
        self.provider = provider.into();
        self.model = model.into();
        self
    }

    pub fn cancellable(mut self) -> Self {
        self.cancellable = true;
        self
    }
}

/// Fresh job id: operation, wall-clock millis and a process-local counter.
pub fn new_job_id(operation: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let op: String = operation
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("job-{op}-{}-{seq}", chrono::Utc::now().timestamp_millis())
}

/// How an operation ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Finish {
    Completed {
        result_ref: String,
    },
    /// Finished with an explicit warning (e.g. some required work incomplete).
    Partial {
        summary: String,
    },
    Failed {
        category: String,
        summary: String,
    },
    /// Stopped cooperatively and can be resumed.
    Paused {
        summary: String,
    },
    Cancelled {
        summary: String,
    },
}

impl Finish {
    pub fn completed() -> Self {
        Self::Completed {
            result_ref: String::new(),
        }
    }

    pub fn failed(category: &str, summary: impl Into<String>) -> Self {
        Self::Failed {
            category: category.into(),
            summary: summary.into(),
        }
    }

    pub fn state(&self) -> &'static str {
        match self {
            Self::Completed { .. } => "completed",
            Self::Partial { .. } => "partial",
            Self::Failed { .. } => "failed",
            Self::Paused { .. } => "paused",
            Self::Cancelled { .. } => "cancelled",
        }
    }
}

/// Live cancel flags of jobs running in this process.
fn live() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static LIVE: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    LIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn open(db: &Path) -> Result<Connection> {
    let conn = Connection::open(db)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    Ok(conn)
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// A registered in-process operation. Finish it explicitly; dropping it
/// unfinished records an interrupted failure.
pub struct JobHandle {
    db: PathBuf,
    id: String,
    app: String,
    correlation: String,
    run_ref: String,
    span: Instant,
    prior_active_ms: i64,
    cancel: Arc<AtomicBool>,
    finished: bool,
}

impl std::fmt::Debug for JobHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobHandle").field("id", &self.id).finish()
    }
}

impl JobHandle {
    /// Persist the registration and mark it running, then return the handle.
    /// An existing id (a resumed operation) is reused: its attempt count goes
    /// up, earlier active time is kept, and no second row is created.
    pub fn begin(db: &Path, spec: JobSpec) -> Result<Self> {
        Self::begin_with(db, spec, Arc::new(AtomicBool::new(false)))
    }

    /// Like [`begin`](Self::begin) but adopts an existing cancel flag, so the
    /// operation's own stop flag and Jobs → Cancel are the same switch.
    pub fn begin_with(db: &Path, spec: JobSpec, cancel: Arc<AtomicBool>) -> Result<Self> {
        let conn = open(db)?;
        let ts = now();
        let owner = crate::scheduler::process_owner();
        let correlation = if spec.correlation_id.is_empty() {
            spec.id.clone()
        } else {
            spec.correlation_id.clone()
        };
        let id = if spec.id.is_empty() {
            new_job_id(&spec.operation)
        } else {
            spec.id.clone()
        };
        let existing: Option<Option<i64>> = conn
            .query_row("SELECT active_ms FROM argos_jobs WHERE id=?1", [&id], |r| {
                r.get(0)
            })
            .optional()?;
        let prior_active_ms = existing.flatten().unwrap_or(0);
        if existing.is_some() {
            conn.execute(
                "UPDATE argos_jobs SET state='running', updated_at=?2, heartbeat_at=?2,
                    started_at=CASE WHEN started_at='' THEN ?2 ELSE started_at END,
                    active_since=?2, finished_at='', attempts_used=attempts_used+1,
                    error_category='', error_summary='', cancel_requested=0,
                    cancellable=?3, worker_owner=?4,
                    run_ref=CASE WHEN ?5<>'' THEN ?5 ELSE run_ref END
                 WHERE id=?1",
                params![id, ts, spec.cancellable, owner, spec.run_ref],
            )?;
        } else {
            conn.execute(
                "INSERT INTO argos_jobs(id,kind,owner_scope,input_revision,state,created_at,updated_at,deadline_at,
                    parent_id,app,operation,title,run_ref,resource_ref,correlation_id,queued_at,
                    started_at,heartbeat_at,active_since,active_ms,queue_ms,attempts_used,attempt_cap,
                    provider,model,tool,worker_owner,cancellable)
                 VALUES (?1,?2,?3,'','running',?4,?4,'',?5,?6,?7,?8,?9,?10,?11,?4,
                    ?4,?4,?4,0,0,1,1,?12,?13,?14,?15,?16)",
                params![
                    id,
                    KIND,
                    spec.app,
                    ts,
                    spec.parent_id,
                    spec.app,
                    spec.operation,
                    spec.title,
                    spec.run_ref,
                    spec.resource_ref,
                    correlation,
                    spec.provider,
                    spec.model,
                    spec.tool,
                    owner,
                    spec.cancellable,
                ],
            )?;
        }
        cancel.store(false, Ordering::Relaxed);
        live()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.clone(), cancel.clone());
        let _ = events::record_event(
            &conn,
            &NewEvent {
                severity: Some(Severity::Info),
                app: spec.app.clone(),
                event_type: "job.started".into(),
                message: if existing.is_some() {
                    format!("Resumed: {}", spec.title)
                } else {
                    format!("Started: {}", spec.title)
                },
                job_id: id.clone(),
                run_id: spec.run_ref.clone(),
                resource_ref: spec.resource_ref.clone(),
                correlation_id: correlation.clone(),
                ..Default::default()
            },
        );
        Ok(Self {
            db: db.to_path_buf(),
            id,
            app: spec.app,
            correlation,
            run_ref: spec.run_ref,
            span: Instant::now(),
            prior_active_ms,
            cancel,
            finished: false,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn correlation(&self) -> &str {
        &self.correlation
    }

    /// The flag the operation polls; Jobs → Cancel sets it.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Register a child (phase, tool call, follow-up) under this job.
    pub fn child(&self, mut spec: JobSpec) -> Result<JobHandle> {
        spec.parent_id = self.id.clone();
        if spec.correlation_id.is_empty() {
            spec.correlation_id = self.correlation.clone();
        }
        if spec.run_ref.is_empty() {
            spec.run_ref = self.run_ref.clone();
        }
        JobHandle::begin(&self.db, spec)
    }

    /// Link a run reference learned after launch (e.g. a new Atlas run id).
    pub fn link_run(&mut self, run_ref: &str) {
        self.run_ref = run_ref.into();
        if let Ok(conn) = open(&self.db) {
            let _ = conn.execute(
                "UPDATE argos_jobs SET run_ref=?2 WHERE id=?1",
                params![self.id, run_ref],
            );
        }
    }

    /// Current phase and progress; doubles as a heartbeat.
    pub fn phase(&self, phase: &str, done: Option<i64>, total: Option<i64>) {
        if let Ok(conn) = open(&self.db) {
            let _ = conn.execute(
                "UPDATE argos_jobs SET phase=?2, progress_done=?3, progress_total=?4,
                    heartbeat_at=?5, updated_at=?5
                 WHERE id=?1",
                params![self.id, phase, done, total, now()],
            );
        }
    }

    /// Accumulated active time including the current span (monotonic).
    pub fn active_ms(&self) -> i64 {
        self.prior_active_ms + self.span.elapsed().as_millis() as i64
    }

    /// Record the terminal (or paused) state.
    pub fn finish(mut self, outcome: Finish) {
        self.write_finish(&outcome);
    }

    fn write_finish(&mut self, outcome: &Finish) {
        if self.finished {
            return;
        }
        self.finished = true;
        live()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
        let Ok(conn) = open(&self.db) else {
            return;
        };
        let ts = now();
        let (category, summary, result_ref) = match outcome {
            Finish::Completed { result_ref } => (String::new(), String::new(), result_ref.clone()),
            Finish::Partial { summary } => ("partial".into(), summary.clone(), String::new()),
            Finish::Failed { category, summary } => {
                (category.clone(), summary.clone(), String::new())
            }
            Finish::Paused { summary } => (String::new(), summary.clone(), String::new()),
            Finish::Cancelled { summary } => ("cancelled".into(), summary.clone(), String::new()),
        };
        let summary = events::redact(&summary);
        let state = outcome.state();
        let terminal = state != "paused";
        let _ = conn.execute(
            "UPDATE argos_jobs SET state=?2, updated_at=?3, heartbeat_at=?3, active_since='',
                active_ms=?4, finished_at=CASE WHEN ?5 THEN ?3 ELSE '' END,
                error_category=?6, error_summary=substr(?7,1,500),
                result_ref=CASE WHEN ?8<>'' THEN ?8 ELSE result_ref END
             WHERE id=?1",
            params![
                self.id,
                state,
                ts,
                self.active_ms(),
                terminal,
                category,
                summary,
                result_ref
            ],
        );
        let (severity, message) = match outcome {
            Finish::Completed { .. } => (Severity::Info, "Completed".to_string()),
            Finish::Partial { summary } => (Severity::Warn, format!("Partial: {summary}")),
            Finish::Failed { summary, .. } => (Severity::Error, format!("Failed: {summary}")),
            Finish::Paused { .. } => (Severity::Info, "Paused".to_string()),
            Finish::Cancelled { .. } => (Severity::Warn, "Cancelled".to_string()),
        };
        let _ = events::record_event(
            &conn,
            &NewEvent {
                severity: Some(severity),
                app: self.app.clone(),
                event_type: format!("job.{state}"),
                message,
                job_id: self.id.clone(),
                run_id: self.run_ref.clone(),
                correlation_id: self.correlation.clone(),
                ..Default::default()
            },
        );
    }
}

impl Drop for JobHandle {
    fn drop(&mut self) {
        if !self.finished {
            let reason = if std::thread::panicking() {
                "stopped by a panic before finishing"
            } else {
                "stopped before finishing (aborted or dropped)"
            };
            self.write_finish(&Finish::failed("interrupted", reason));
        }
    }
}

/// Result of a Jobs → Cancel request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CancelRequest {
    /// The flag is set; the job reads "cancelled" once the operation stops.
    Requested,
    /// The job does not support safe cancellation.
    NotCancellable,
    /// The job is not running.
    NotRunning,
    Missing,
}

/// Ask a running job to stop cooperatively. Works across processes: the
/// owning process's heartbeat picks up `cancel_requested`.
pub fn request_cancel(conn: &Connection, job_id: &str) -> Result<CancelRequest> {
    let row: Option<(String, bool, String)> = conn
        .query_row(
            "SELECT state, cancellable, app FROM argos_jobs WHERE id=?1",
            [job_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((state, cancellable, app)) = row else {
        return Ok(CancelRequest::Missing);
    };
    if !matches!(state.as_str(), "running" | "queued") {
        return Ok(CancelRequest::NotRunning);
    }
    if !cancellable {
        return Ok(CancelRequest::NotCancellable);
    }
    conn.execute(
        "UPDATE argos_jobs SET cancel_requested=1, updated_at=?2 WHERE id=?1",
        params![job_id, now()],
    )?;
    if let Some(flag) = live().lock().unwrap_or_else(|e| e.into_inner()).get(job_id) {
        flag.store(true, Ordering::Relaxed);
    }
    events::record_event(
        conn,
        &NewEvent {
            severity: Some(Severity::Info),
            app,
            event_type: "job.cancel_requested".into(),
            message: "Cancel requested".into(),
            job_id: job_id.into(),
            correlation_id: job_id.into(),
            ..Default::default()
        },
    )?;
    Ok(CancelRequest::Requested)
}

/// Refresh this process's liveness row and apply cancel requests made by
/// other processes to jobs running here. Called by the heartbeat thread.
pub fn beat(conn: &Connection, owner: &str) -> Result<()> {
    let ts = now();
    conn.execute(
        "INSERT INTO argos_processes(owner,pid,started_at,heartbeat_at) VALUES (?1,?2,?3,?3)
         ON CONFLICT(owner) DO UPDATE SET heartbeat_at=excluded.heartbeat_at",
        params![owner, std::process::id(), ts],
    )?;
    let mut stmt = conn.prepare(
        "SELECT id FROM argos_jobs WHERE worker_owner=?1 AND cancel_requested=1
            AND state IN ('running','queued')",
    )?;
    let ids = stmt
        .query_map([owner], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let live = live().lock().unwrap_or_else(|e| e.into_inner());
    for id in ids {
        if let Some(flag) = live.get(&id) {
            flag.store(true, Ordering::Relaxed);
        }
    }
    Ok(())
}

/// Mark open in-process jobs owned by processes that are gone as
/// interrupted failures. Their active time ends at the last heartbeat seen.
pub fn recover_orphans(conn: &Connection, me: &str) -> Result<usize> {
    let now_dt = chrono::Utc::now();
    let stale = (now_dt - chrono::Duration::seconds(PROCESS_STALE_SECS)).to_rfc3339();
    let mut stmt = conn.prepare(
        "SELECT j.id, j.active_ms, j.active_since, j.heartbeat_at, IFNULL(p.heartbeat_at,''), j.app, j.run_ref
         FROM argos_jobs j LEFT JOIN argos_processes p ON p.owner = j.worker_owner
         WHERE j.kind=?1 AND j.state IN ('running','queued') AND j.worker_owner<>?2
           AND (p.owner IS NULL OR p.heartbeat_at < ?3)",
    )?;
    type Orphan = (String, Option<i64>, String, String, String, String, String);
    let rows = stmt
        .query_map(params![KIND, me, stale], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<Orphan>>>()?;
    let parse = |ts: &str| {
        chrono::DateTime::parse_from_rfc3339(ts)
            .ok()
            .map(|t| t.with_timezone(&chrono::Utc))
    };
    let ts = now_dt.to_rfc3339();
    for (id, active, since, job_beat, proc_beat, app, run_ref) in &rows {
        let last_seen = [proc_beat.as_str(), job_beat.as_str()]
            .into_iter()
            .filter_map(parse)
            .max();
        let span = match (parse(since), last_seen) {
            (Some(start), Some(end)) => (end - start).num_milliseconds().max(0),
            _ => 0,
        };
        let finished = last_seen
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| ts.clone());
        conn.execute(
            "UPDATE argos_jobs SET state='failed', error_category='interrupted',
                error_summary='Argos exited before this job finished', active_since='',
                active_ms=?2, finished_at=?3, updated_at=?4
             WHERE id=?1 AND state IN ('running','queued')",
            params![id, active.unwrap_or(0) + span, finished, ts],
        )?;
        let _ = events::record_event(
            conn,
            &NewEvent {
                severity: Some(Severity::Warn),
                app: app.clone(),
                event_type: "job.interrupted".into(),
                message: "Argos exited before this job finished".into(),
                job_id: id.clone(),
                run_id: run_ref.clone(),
                correlation_id: id.clone(),
                ..Default::default()
            },
        );
    }
    conn.execute(
        "DELETE FROM argos_processes WHERE heartbeat_at < ?1 AND owner<>?2",
        params![stale, me],
    )?;
    Ok(rows.len())
}

/// Background heartbeat for this process. Dropping it stops the thread.
pub struct ProcessBeat {
    stop: Arc<AtomicBool>,
}

impl Drop for ProcessBeat {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Write the first heartbeat, recover jobs left by exited processes, then
/// keep beating on a thread.
pub fn start_process_beat(db: &Path) -> Result<ProcessBeat> {
    let owner = crate::scheduler::process_owner();
    let conn = open(db)?;
    beat(&conn, &owner)?;
    recover_orphans(&conn, &owner)?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let db = db.to_path_buf();
    std::thread::Builder::new()
        .name("argos-job-beat".into())
        .spawn(move || {
            let mut ticks = 0u32;
            while !flag.load(Ordering::Relaxed) {
                std::thread::sleep(BEAT_INTERVAL);
                if let Ok(conn) = open(&db) {
                    let _ = beat(&conn, &owner);
                    ticks += 1;
                    if ticks % 12 == 0 {
                        let _ = recover_orphans(&conn, &owner);
                    }
                }
            }
        })?;
    Ok(ProcessBeat { stop })
}

/// Canonical job for a legacy run/job id (Atlas run, Intel Recon job): the
/// existing registry row linked to it, so resumes reuse one top-level job.
pub fn job_for_run(conn: &Connection, operation: &str, run_ref: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT id FROM argos_jobs WHERE operation=?1 AND run_ref=?2 AND parent_id=''
             ORDER BY created_at LIMIT 1",
            params![operation, run_ref],
            |r| r.get(0),
        )
        .optional()?)
}

/// Optional handle: a registry write failure (e.g. a locked database) must
/// never stop the operation itself, so callers run without tracking.
pub fn begin_optional(db: &Path, spec: JobSpec) -> Option<JobHandle> {
    JobHandle::begin(db, spec).ok()
}

/// [`begin_optional`] adopting the operation's existing stop flag.
pub fn begin_optional_with(db: &Path, spec: JobSpec, cancel: Arc<AtomicBool>) -> Option<JobHandle> {
    JobHandle::begin_with(db, spec, cancel).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("argos.db");
        let store = crate::store::Store::open(&path).unwrap();
        drop(store);
        (dir, path)
    }

    fn row(db: &Path, id: &str) -> crate::jobs_view::JobRow {
        crate::store::Store::open(db)
            .unwrap()
            .get_job(id)
            .unwrap()
            .expect("job row")
    }

    #[test]
    fn registration_is_durable_before_work_and_finish_records_monotonic_timing() {
        let (_dir, path) = db();
        let job = JobHandle::begin(
            &path,
            JobSpec::new("tools", "tool_run", "Run whois").tool("whois"),
        )
        .unwrap();
        let id = job.id().to_string();
        let running = row(&path, &id);
        assert_eq!(running.state, "running");
        assert_eq!(running.kind, KIND);
        assert!(!running.started_at.is_empty());
        assert_eq!(running.correlation_id, id);
        assert!(running.active_now(chrono::Utc::now()).is_some());
        job.phase("query", Some(1), Some(2));
        assert_eq!(row(&path, &id).phase, "query");
        std::thread::sleep(Duration::from_millis(20));
        job.finish(Finish::completed());
        let done = row(&path, &id);
        assert_eq!(done.state, "completed");
        assert!(!done.finished_at.is_empty());
        let active = done.active_now(chrono::Utc::now()).unwrap();
        assert!(active >= 20, "{active}");
        // Finished jobs do not keep growing.
        std::thread::sleep(Duration::from_millis(15));
        assert_eq!(row(&path, &id).active_now(chrono::Utc::now()), Some(active));
        let store = crate::store::Store::open(&path).unwrap();
        let events = store
            .list_events(
                &crate::events::EventFilter {
                    job_id: id.clone(),
                    ..Default::default()
                },
                10,
            )
            .unwrap();
        let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        assert!(
            types.contains(&"job.started") && types.contains(&"job.completed"),
            "{types:?}"
        );
        assert!(events.iter().all(|e| e.correlation_id == id));
    }

    #[test]
    fn dropped_or_panicking_work_reaches_a_failed_state() {
        let (_dir, path) = db();
        let id = {
            let job =
                JobHandle::begin(&path, JobSpec::new("brain", "graph_summary", "Explain")).unwrap();
            job.id().to_string()
        };
        let dropped = row(&path, &id);
        assert_eq!(dropped.state, "failed");
        assert_eq!(dropped.error_category, "interrupted");
        let path2 = path.clone();
        let handle = std::thread::spawn(move || {
            let job =
                JobHandle::begin(&path2, JobSpec::new("recon", "investigation", "Q")).unwrap();
            let id = job.id().to_string();
            let _keep = job;
            std::panic::panic_any(id);
        });
        let id = *handle.join().unwrap_err().downcast::<String>().unwrap();
        let panicked = row(&path, &id);
        assert_eq!(panicked.state, "failed");
        assert!(
            panicked.error_summary.contains("panic"),
            "{}",
            panicked.error_summary
        );
    }

    #[test]
    fn resume_reuses_the_job_and_keeps_earlier_active_time() {
        let (_dir, path) = db();
        let spec = JobSpec::new("atlas", "atlas_cycle", "Atlas cycle").run("run-1");
        let id = spec.id.clone();
        let job = JobHandle::begin(&path, spec.clone()).unwrap();
        std::thread::sleep(Duration::from_millis(15));
        job.finish(Finish::Paused {
            summary: "paused".into(),
        });
        let paused = row(&path, &id);
        assert_eq!(paused.state, "paused");
        assert!(paused.finished_at.is_empty(), "paused is not terminal");
        let first = paused.active_now(chrono::Utc::now()).unwrap();
        let conn = open(&path).unwrap();
        assert_eq!(
            job_for_run(&conn, "atlas_cycle", "run-1").unwrap(),
            Some(id.clone())
        );
        let again = JobHandle::begin(&path, spec).unwrap();
        std::thread::sleep(Duration::from_millis(15));
        again.finish(Finish::completed());
        let done = row(&path, &id);
        assert_eq!(done.attempts_used, 2);
        assert!(done.active_now(chrono::Utc::now()).unwrap() >= first + 15);
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM argos_jobs WHERE run_ref='run-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "no duplicate top-level rows");
    }

    #[test]
    fn cancel_is_cooperative_and_only_for_cancellable_running_jobs() {
        let (_dir, path) = db();
        let conn = open(&path).unwrap();
        let fixed = JobHandle::begin(&path, JobSpec::new("atlas", "repair", "Repair")).unwrap();
        assert_eq!(
            request_cancel(&conn, fixed.id()).unwrap(),
            CancelRequest::NotCancellable
        );
        assert!(!fixed.is_cancelled());
        fixed.finish(Finish::completed());

        let job = JobHandle::begin(
            &path,
            JobSpec::new("tools", "tool_run", "whois").cancellable(),
        )
        .unwrap();
        let id = job.id().to_string();
        assert_eq!(
            request_cancel(&conn, &id).unwrap(),
            CancelRequest::Requested
        );
        assert!(job.is_cancelled(), "in-process flag flips immediately");
        // Still running until the operation stops (no premature "cancelled").
        assert_eq!(row(&path, &id).state, "running");
        job.finish(Finish::Cancelled {
            summary: "stopped by user".into(),
        });
        assert_eq!(row(&path, &id).state, "cancelled");
        assert_eq!(
            request_cancel(&conn, &id).unwrap(),
            CancelRequest::NotRunning
        );
        assert_eq!(
            request_cancel(&conn, "nope").unwrap(),
            CancelRequest::Missing
        );
    }

    #[test]
    fn cross_process_cancel_reaches_the_owner_through_its_heartbeat() {
        let (_dir, path) = db();
        let job = JobHandle::begin(
            &path,
            JobSpec::new("recon", "investigation", "Q").cancellable(),
        )
        .unwrap();
        let conn = open(&path).unwrap();
        // Another process can only set the durable flag.
        conn.execute(
            "UPDATE argos_jobs SET cancel_requested=1 WHERE id=?1",
            [job.id()],
        )
        .unwrap();
        assert!(!job.is_cancelled());
        beat(&conn, &crate::scheduler::process_owner()).unwrap();
        assert!(job.is_cancelled());
        job.finish(Finish::Cancelled {
            summary: String::new(),
        });
    }

    #[test]
    fn jobs_of_exited_processes_become_interrupted_and_live_ones_stay() {
        let (_dir, path) = db();
        let conn = open(&path).unwrap();
        let me = crate::scheduler::process_owner();
        beat(&conn, &me).unwrap();
        let mine = JobHandle::begin(&path, JobSpec::new("intel", "article_body", "Body")).unwrap();
        // A job owned by a process whose last heartbeat is old.
        let gone = JobHandle::begin(&path, JobSpec::new("recon", "investigation", "Old")).unwrap();
        let gone_id = gone.id().to_string();
        let old = (chrono::Utc::now() - chrono::Duration::minutes(10)).to_rfc3339();
        let older = (chrono::Utc::now() - chrono::Duration::minutes(12)).to_rfc3339();
        conn.execute(
            "INSERT INTO argos_processes(owner,pid,started_at,heartbeat_at) VALUES ('argos-1-1',1,?1,?2)",
            params![older, old],
        )
        .unwrap();
        conn.execute(
            "UPDATE argos_jobs SET worker_owner='argos-1-1', active_since=?2, heartbeat_at=?2 WHERE id=?1",
            params![gone_id, older],
        )
        .unwrap();
        std::mem::forget(gone); // simulate the process vanishing (no Drop)
        assert_eq!(recover_orphans(&conn, &me).unwrap(), 1);
        let row_gone = row(&path, &gone_id);
        assert_eq!(row_gone.state, "failed");
        assert_eq!(row_gone.error_category, "interrupted");
        assert_eq!(
            row_gone.finished_at,
            old.parse::<chrono::DateTime<chrono::Utc>>()
                .unwrap()
                .to_rfc3339()
        );
        let active = row_gone.active_now(chrono::Utc::now()).unwrap();
        assert!(
            (119_000..=121_000).contains(&active),
            "two minutes up to the last heartbeat: {active}"
        );
        assert_eq!(
            row(&path, mine.id()).state,
            "running",
            "this process's jobs are untouched"
        );
        assert_eq!(recover_orphans(&conn, &me).unwrap(), 0, "idempotent");
        mine.finish(Finish::completed());
    }
}
