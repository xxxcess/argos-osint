//! Shared registration for in-process asynchronous operations (spec §5.1).
//!
//! Durable task workers already record jobs, tasks and attempts. Operations
//! that run in-process (Atlas cycles, Recon investigations, tool runs, graph
//! explanations, provider checks, …) register here instead. Registration is
//! persisted before the work launches, timing comes from a monotonic clock,
//! and every handle reaches a terminal or recoverable state:
//!
//! - an explicit [`JobHandle::finish`] (completed, partial, failed, paused,
//!   waiting, blocked, cancelled);
//! - `Drop` without finishing (panic, abort, early return) records a failure;
//! - a process that exits is detected by [`recover_orphans`] through the
//!   per-process heartbeat row, and its open jobs become interrupted failures.
//!
//! Cancellation is cooperative. [`request_cancel`] only sets a flag that the
//! operation observes; the job reads "cancelled" once the operation stops.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
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
    /// Eligible work remains (background retries, indexing). Not terminal.
    Waiting {
        summary: String,
    },
    /// Required configuration is missing. Not terminal; resume after the fix.
    Blocked {
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
            Self::Waiting { .. } => "waiting",
            Self::Blocked { .. } => "blocked",
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
    /// Active time of earlier attempts read from the SurrealDB row on a
    /// resume; filled by the ordered begin write, so `finish` sees it.
    surreal_prior: Arc<AtomicI64>,
    cancel: Arc<AtomicBool>,
    finished: bool,
    /// The row lives in the repository's `job` table (SurrealDB selected);
    /// no SQLite connection is opened for it.
    surreal: bool,
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
        if crate::recon::backend::surreal_selected() {
            return Ok(Self::begin_surreal(db, spec, cancel));
        }
        let conn = open(db)?;
        let _ = ensure_beat(db);
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
        crate::job_cleanup::register_reference(&conn, &id, &spec.resource_ref, &spec.operation)?;
        if let Err(error) = crate::job_cleanup::ensure_admissible(&conn, &id, &ts) {
            cancel.store(true, Ordering::Release);
            return Err(error);
        }
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
            surreal_prior: Arc::new(AtomicI64::new(0)),
            cancel,
            finished: false,
            surreal: false,
        })
    }

    /// [`begin_with`](Self::begin_with) on the repository: the `job` row and
    /// its `job.started` event are written in order on the storage runtime
    /// (`storage::recon_jobs::begin_app_job`, `storage::event_log`).
    fn begin_surreal(db: &Path, spec: JobSpec, cancel: Arc<AtomicBool>) -> Self {
        let id = if spec.id.is_empty() {
            new_job_id(&spec.operation)
        } else {
            spec.id.clone()
        };
        let correlation = if spec.correlation_id.is_empty() {
            id.clone()
        } else {
            spec.correlation_id.clone()
        };
        live()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.clone(), cancel.clone());
        let surreal_prior = Arc::new(AtomicI64::new(0));
        #[cfg(feature = "surreal")]
        {
            let (spec, id, correlation) = (spec.clone(), id.clone(), correlation.clone());
            let prior = surreal_prior.clone();
            crate::storage::runtime::spawn_ordered(async move {
                let Ok(repo) = crate::storage::runtime::repository().await else {
                    return;
                };
                // A resume reuses the row: keep the active time it recorded.
                if let Ok(Some(ms)) = surreal_prior_active_ms(&repo, &id).await {
                    prior.store(ms, Ordering::Release);
                }
                let job = crate::storage::recon_jobs::AppJob {
                    id: &id,
                    operation: &spec.operation,
                    app: &spec.app,
                    title: &spec.title,
                    run_ref: &spec.run_ref,
                    resource_ref: &spec.resource_ref,
                    parent: Some(spec.parent_id.as_str()),
                };
                if crate::storage::recon_jobs::begin_app_job(&repo, &job)
                    .await
                    .is_err()
                {
                    return;
                }
                let _ = surreal_set_job_meta(&repo, &id, &spec).await;
                let _ = crate::storage::event_log::record_event(
                    repo.db(),
                    &NewEvent {
                        severity: Some(Severity::Info),
                        app: spec.app.clone(),
                        event_type: "job.started".into(),
                        message: format!("Started: {}", spec.title),
                        job_id: id.clone(),
                        run_id: spec.run_ref.clone(),
                        resource_ref: spec.resource_ref.clone(),
                        correlation_id: correlation,
                        ..Default::default()
                    },
                )
                .await;
            });
        }
        Self {
            db: db.to_path_buf(),
            id,
            app: spec.app,
            correlation,
            run_ref: spec.run_ref,
            span: Instant::now(),
            prior_active_ms: 0,
            surreal_prior,
            cancel,
            finished: false,
            surreal: true,
        }
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
        if self.surreal {
            // Carried to the row by `finish` (`finish_recon_job` run_ref).
            return;
        }
        if let Ok(conn) = open(&self.db) {
            let _ = conn.execute(
                "UPDATE argos_jobs SET run_ref=?2 WHERE id=?1",
                params![self.id, run_ref],
            );
        }
    }

    /// Current phase and progress; doubles as a heartbeat.
    pub fn phase(&self, phase: &str, done: Option<i64>, total: Option<i64>) {
        if self.surreal {
            #[cfg(feature = "surreal")]
            {
                let (id, phase, cancel) = (self.id.clone(), phase.to_string(), self.cancel.clone());
                crate::storage::runtime::spawn_ordered(async move {
                    let Ok(repo) = crate::storage::runtime::repository().await else {
                        return;
                    };
                    if let Ok(true) = crate::storage::recon_jobs::set_job_progress(
                        &repo, &id, &phase, done, total,
                    )
                    .await
                    {
                        cancel.store(true, Ordering::Relaxed);
                    }
                });
            }
            return;
        }
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
        self.prior_active_ms
            + self.surreal_prior.load(Ordering::Acquire)
            + self.span.elapsed().as_millis() as i64
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
        if self.surreal {
            self.write_finish_surreal(outcome);
            return;
        }
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
            Finish::Waiting { summary } => (String::new(), summary.clone(), String::new()),
            Finish::Blocked { summary } => ("blocked".into(), summary.clone(), String::new()),
            Finish::Cancelled { summary } => ("cancelled".into(), summary.clone(), String::new()),
        };
        let summary = events::redact(&summary);
        let state = outcome.state();
        let terminal = !matches!(state, "paused" | "waiting" | "blocked");
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
            Finish::Waiting { summary } => (Severity::Info, format!("Waiting: {summary}")),
            Finish::Blocked { summary } => (Severity::Warn, format!("Blocked: {summary}")),
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

impl JobHandle {
    /// `write_finish` on the repository, queued behind the phase writes so
    /// the terminal state lands last.
    fn write_finish_surreal(&mut self, outcome: &Finish) {
        #[cfg(feature = "surreal")]
        {
            let (category, summary, result_ref) = finish_fields(outcome);
            let (severity, message) = finish_event(outcome);
            let state = outcome.state();
            let (id, app, run, corr) = (
                self.id.clone(),
                self.app.clone(),
                self.run_ref.clone(),
                self.correlation.clone(),
            );
            // This attempt's span now; earlier attempts' time is added in
            // the ordered write, after the begin write has read it.
            let span_ms = self.prior_active_ms + self.span.elapsed().as_millis() as i64;
            let prior = self.surreal_prior.clone();
            crate::storage::runtime::spawn_ordered(async move {
                let active_ms = span_ms + prior.load(Ordering::Acquire);
                let Ok(repo) = crate::storage::runtime::repository().await else {
                    return;
                };
                let _ = crate::storage::recon_jobs::finish_recon_job(
                    &repo,
                    &id,
                    state,
                    Some(run.as_str()),
                    &category,
                    &summary,
                    &result_ref,
                    active_ms,
                )
                .await;
                let _ = crate::storage::event_log::record_event(
                    repo.db(),
                    &NewEvent {
                        severity: Some(severity),
                        app,
                        event_type: format!("job.{state}"),
                        message,
                        job_id: id,
                        run_id: run,
                        correlation_id: corr,
                        ..Default::default()
                    },
                )
                .await;
            });
        }
        #[cfg(not(feature = "surreal"))]
        let _ = outcome;
    }
}

/// `active_ms` an existing SurrealDB job row recorded (a resumed job).
#[cfg(feature = "surreal")]
async fn surreal_prior_active_ms(
    repo: &crate::storage::repo::Repository,
    id: &str,
) -> Result<Option<i64>> {
    let mut r = repo
        .db()
        .query("SELECT VALUE active_ms FROM type::record('job', $id)")
        .bind(("id", id.to_string()))
        .await?
        .check()?;
    let ms: Vec<Option<i64>> = r.take(0usize)?;
    Ok(ms.into_iter().next().flatten())
}

/// The spec fields `begin_app_job` does not write: provider, model, tool
/// and whether Jobs → Cancel may stop the job.
#[cfg(feature = "surreal")]
async fn surreal_set_job_meta(
    repo: &crate::storage::repo::Repository,
    id: &str,
    spec: &JobSpec,
) -> Result<()> {
    repo.db()
        .query(
            "UPDATE type::record('job', $id) SET provider = $provider, model = $model,
               tool = $tool, cancellable = $cancellable RETURN NONE",
        )
        .bind(("id", id.to_string()))
        .bind(("provider", spec.provider.clone()))
        .bind(("model", spec.model.clone()))
        .bind(("tool", spec.tool.clone()))
        .bind(("cancellable", spec.cancellable))
        .await?
        .check()?;
    Ok(())
}

/// Error category, summary and result reference recorded for an outcome.
#[cfg_attr(not(feature = "surreal"), allow(dead_code))]
fn finish_fields(outcome: &Finish) -> (String, String, String) {
    match outcome {
        Finish::Completed { result_ref } => (String::new(), String::new(), result_ref.clone()),
        Finish::Partial { summary } => ("partial".into(), summary.clone(), String::new()),
        Finish::Failed { category, summary } => (category.clone(), summary.clone(), String::new()),
        Finish::Paused { summary } | Finish::Waiting { summary } => {
            (String::new(), summary.clone(), String::new())
        }
        Finish::Blocked { summary } => ("blocked".into(), summary.clone(), String::new()),
        Finish::Cancelled { summary } => ("cancelled".into(), summary.clone(), String::new()),
    }
}

/// Severity and message of the terminal `job.<state>` event.
#[cfg_attr(not(feature = "surreal"), allow(dead_code))]
fn finish_event(outcome: &Finish) -> (Severity, String) {
    match outcome {
        Finish::Completed { .. } => (Severity::Info, "Completed".to_string()),
        Finish::Partial { summary } => (Severity::Warn, format!("Partial: {summary}")),
        Finish::Failed { summary, .. } => (Severity::Error, format!("Failed: {summary}")),
        Finish::Paused { .. } => (Severity::Info, "Paused".to_string()),
        Finish::Waiting { summary } => (Severity::Info, format!("Waiting: {summary}")),
        Finish::Blocked { summary } => (Severity::Warn, format!("Blocked: {summary}")),
        Finish::Cancelled { .. } => (Severity::Warn, "Cancelled".to_string()),
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
    for (id, flag) in live.iter() {
        if crate::job_cleanup::retired(conn, id)? {
            flag.store(true, Ordering::Release);
        }
    }
    for id in ids {
        if let Some(flag) = live.get(&id) {
            flag.store(true, Ordering::Relaxed);
        }
    }
    Ok(())
}

/// Mark open jobs owned by processes that are gone (registry jobs and
/// owned task-less jobs such as the repair pass) as interrupted failures. Their active time ends at the last heartbeat seen.
pub fn recover_orphans(conn: &Connection, me: &str) -> Result<usize> {
    crate::job_cleanup::cleanup_batch(conn, chrono::Utc::now())?;
    let now_dt = chrono::Utc::now();
    let stale = (now_dt - chrono::Duration::seconds(PROCESS_STALE_SECS)).to_rfc3339();
    let mut stmt = conn.prepare(
        "SELECT j.id, j.active_ms, j.active_since, j.heartbeat_at, IFNULL(p.heartbeat_at,''), j.app, j.run_ref
         FROM argos_jobs j LEFT JOIN argos_processes p ON p.owner = j.worker_owner
         WHERE j.kind<>'service' AND j.worker_owner<>'' AND j.worker_owner<>?1
           AND j.state IN ('running','queued')
           AND (p.owner IS NULL OR p.heartbeat_at < ?2)",
    )?;
    type Orphan = (String, Option<i64>, String, String, String, String, String);
    let rows = stmt
        .query_map(params![me, stale], |r| {
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

/// Running heartbeat threads by database (one per process and state root).
fn beats() -> &'static Mutex<HashMap<PathBuf, Arc<AtomicBool>>> {
    static BEATS: OnceLock<Mutex<HashMap<PathBuf, Arc<AtomicBool>>>> = OnceLock::new();
    BEATS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Make sure this process heartbeats for `db`, so other processes never
/// mistake its running jobs for orphans. Registration calls this, so CLI
/// runs are covered as well as the TUI.
fn ensure_beat(db: &Path) -> Result<Arc<AtomicBool>> {
    let mut running = beats().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(flag) = running.get(db) {
        if !flag.load(Ordering::Relaxed) {
            return Ok(flag.clone());
        }
    }
    let owner = crate::scheduler::process_owner();
    beat(&open(db)?, &owner)?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let path = db.to_path_buf();
    std::thread::Builder::new()
        .name("argos-job-beat".into())
        .spawn(move || {
            let mut ticks = 0u32;
            while !flag.load(Ordering::Relaxed) {
                std::thread::sleep(BEAT_INTERVAL);
                if let Ok(conn) = open(&path) {
                    let _ = beat(&conn, &owner);
                    ticks += 1;
                    if ticks.is_multiple_of(12) {
                        let _ = recover_orphans(&conn, &owner);
                    }
                    if ticks.is_multiple_of(60) {
                        let _ = crate::job_cleanup::cleanup_batch(&conn, chrono::Utc::now());
                    }
                }
            }
        })?;
    running.insert(db.to_path_buf(), stop.clone());
    Ok(stop)
}

/// Write the first heartbeat, recover jobs left by exited processes, then
/// keep beating on a thread until the returned guard is dropped.
pub fn start_process_beat(db: &Path) -> Result<ProcessBeat> {
    if crate::recon::backend::surreal_selected() {
        return start_surreal_beat();
    }
    let stop = ensure_beat(db)?;
    recover_orphans(&open(db)?, &crate::scheduler::process_owner())?;
    Ok(ProcessBeat { stop })
}

/// [`start_process_beat`] on the repository: `process:[owner]` beats
/// (`storage::processes::beat`), cross-process cancels reach live jobs, and
/// every minute jobs and tasks of gone processes are recovered.
#[cfg(feature = "surreal")]
fn start_surreal_beat() -> Result<ProcessBeat> {
    let owner = crate::scheduler::process_owner();
    let first = owner.clone();
    crate::storage::runtime::blocking(move |repo| async move {
        surreal_beat(&repo, &first).await?;
        surreal_recover(&repo, &first).await
    })?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::Builder::new()
        .name("argos-job-beat".into())
        .spawn(move || {
            let mut ticks = 0u32;
            while !flag.load(Ordering::Relaxed) {
                std::thread::sleep(BEAT_INTERVAL);
                ticks += 1;
                let recover = ticks.is_multiple_of(12);
                let me = owner.clone();
                // A beat that cannot reach the database writes nothing;
                // the next tick tries again.
                let _ = crate::storage::runtime::blocking(move |repo| async move {
                    surreal_beat(&repo, &me).await?;
                    if recover {
                        surreal_recover(&repo, &me).await?;
                    }
                    Ok(())
                });
            }
        })?;
    Ok(ProcessBeat { stop })
}

#[cfg(not(feature = "surreal"))]
fn start_surreal_beat() -> Result<ProcessBeat> {
    anyhow::bail!("SurrealDB storage is off without the surreal feature")
}

/// One beat, then raise the stop flag of live jobs here that another
/// process asked to cancel.
#[cfg(feature = "surreal")]
async fn surreal_beat(repo: &crate::storage::repo::Repository, owner: &str) -> Result<()> {
    crate::storage::processes::beat(repo.db(), owner, std::process::id(), chrono::Utc::now())
        .await
        .map_err(|e| anyhow::anyhow!("process beat: {e}"))?;
    let mut r = repo
        .db()
        .query(
            "SELECT VALUE record::id(id) FROM job WHERE worker_owner = $owner
               AND cancel_requested = true AND state IN ['running', 'queued']",
        )
        .bind(("owner", owner.to_string()))
        .await?
        .check()?;
    let ids: Vec<String> = r.take(0usize)?;
    let live = live().lock().unwrap_or_else(|e| e.into_inner());
    for id in ids {
        if let Some(flag) = live.get(&id) {
            flag.store(true, Ordering::Relaxed);
        }
    }
    Ok(())
}

/// [`recover_orphans`] on the repository: registry jobs of processes with
/// no fresh beat become interrupted failures whose active time ends at the
/// job's last heartbeat, then `storage::processes::recover_orphans` requeues
/// their tasks and drops their process rows. Nothing is recovered while
/// this process's own beat is stale (a likely database outage).
#[cfg(feature = "surreal")]
async fn surreal_recover(repo: &crate::storage::repo::Repository, me: &str) -> Result<usize> {
    let now = chrono::Utc::now();
    let stale = now - chrono::Duration::seconds(PROCESS_STALE_SECS);
    let mut r = repo
        .db()
        .query("SELECT VALUE record::id(id) FROM process WHERE heartbeat_at >= <datetime>$stale")
        .bind(("stale", stale.to_rfc3339()))
        .await?
        .check()?;
    let fresh: Vec<String> = r.take(0usize)?;
    if !fresh.iter().any(|o| o == me) {
        return Ok(0);
    }
    use surrealdb::types::SurrealValue;
    #[derive(SurrealValue)]
    struct Orphan {
        id: String,
        app: String,
        run_ref: String,
    }
    let mut r = repo
        .db()
        .query(
            "UPDATE job SET state = 'failed', error_category = 'interrupted',
               error_summary = 'Argos exited before this job finished',
               active_ms = (active_ms ?? 0) + IF active_since != NONE AND heartbeat_at != NONE
                 AND heartbeat_at > active_since
                 { duration::millis(heartbeat_at - active_since) } ELSE { 0 },
               active_since = NONE, finished_at = heartbeat_at ?? <datetime>$now,
               updated_at = <datetime>$now
             WHERE kind = $kind AND worker_owner != '' AND worker_owner != $me
               AND state IN ['running', 'queued'] AND worker_owner NOTINSIDE $fresh
             RETURN record::id(id) AS id, app, run_ref",
        )
        .bind(("kind", KIND))
        .bind(("me", me.to_string()))
        .bind(("fresh", fresh))
        .bind(("now", now.to_rfc3339()))
        .await?
        .check()?;
    let orphans: Vec<Orphan> = r.take(0usize)?;
    for o in &orphans {
        let _ = crate::storage::event_log::record_event(
            repo.db(),
            &NewEvent {
                severity: Some(Severity::Warn),
                app: o.app.clone(),
                event_type: "job.interrupted".into(),
                message: "Argos exited before this job finished".into(),
                job_id: o.id.clone(),
                run_id: o.run_ref.clone(),
                correlation_id: o.id.clone(),
                ..Default::default()
            },
        )
        .await;
    }
    crate::storage::processes::recover_orphans(repo.db(), me, now)
        .await
        .map_err(|e| anyhow::anyhow!("task recovery: {e}"))?;
    Ok(orphans.len())
}

pub(crate) fn signal_retirement(id: &str) {
    if let Some(flag) = live().lock().unwrap_or_else(|e| e.into_inner()).get(id) {
        flag.store(true, Ordering::Release);
    }
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
}
