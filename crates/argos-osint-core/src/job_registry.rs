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

use crate::events::{NewEvent, Severity};

/// Job kind for registry-owned rows (task-backed jobs keep their own kinds).
pub const KIND: &str = "inproc";
/// A process whose heartbeat is older than this is treated as gone.
/// Three beat intervals, so one delayed beat does not look like a dead owner.
pub const PROCESS_STALE_SECS: i64 = 90;
/// How often the process heartbeat runs.
pub const BEAT_INTERVAL: Duration = Duration::from_secs(30);

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

/// Raise the stop flag of a live job in this process (a Jobs → Cancel
/// issued here reaches the operation without waiting for the next beat).
pub fn raise_local_cancel(id: &str) -> bool {
    let live = live().lock().unwrap_or_else(|e| e.into_inner());
    match live.get(id) {
        Some(flag) => {
            crate::recon::cancel_with_notify(flag);
            true
        }
        None => false,
    }
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
    /// no legacy store connection is opened for it.
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
        Ok(Self::begin_surreal(db, spec, cancel))
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

    /// Accumulated active time including the current span (monotonic).
    /// Current phase and progress; doubles as a heartbeat.
    pub fn phase(&self, phase: &str, done: Option<i64>, total: Option<i64>) {
        let (id, phase, cancel) = (self.id.clone(), phase.to_string(), self.cancel.clone());
        crate::storage::runtime::spawn_ordered(async move {
            let Ok(repo) = crate::storage::runtime::repository().await else {
                return;
            };
            if let Ok(true) =
                crate::storage::recon_jobs::set_job_progress(&repo, &id, &phase, done, total).await
            {
                crate::recon::cancel_with_notify(&cancel);
            }
        });
    }

    /// Link a run reference learned after launch (e.g. a new Atlas run id);
    /// carried to the row by `finish` (`finish_recon_job` run_ref).
    pub fn link_run(&mut self, run_ref: &str) {
        self.run_ref = run_ref.into();
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
        self.write_finish_surreal(outcome);
    }

    pub fn active_ms(&self) -> i64 {
        self.prior_active_ms
            + self.surreal_prior.load(Ordering::Acquire)
            + self.span.elapsed().as_millis() as i64
    }

    /// Record the terminal (or paused) state.
    pub fn finish(mut self, outcome: Finish) {
        self.write_finish(&outcome);
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
    // The repository heartbeat (`storage::processes::beat`) also recovers
    // orphans; keep its thread alive past this guard.
    let beat = start_surreal_beat()?;
    let stop = beat.stop.clone();
    std::mem::forget(beat);
    running.insert(db.to_path_buf(), stop.clone());
    Ok(stop)
}

/// Write the first heartbeat, recover jobs left by exited processes, then
/// keep beating on a thread until the returned guard is dropped.
pub fn start_process_beat(_db: &Path) -> Result<ProcessBeat> {
    start_surreal_beat()
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
                // Two 30s beats keep orphan recovery on the one-minute cadence.
                let recover = ticks.is_multiple_of(2);
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
             WHERE (kind = $kind OR (kind = 'service' AND operation IN ['api_server', 'web_server']))
               AND worker_owner != '' AND worker_owner != $me
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
    #[cfg(feature = "embedded")]
    #[tokio::test]
    async fn traffic_services_recover_dead_owners_without_retiring_other_services() {
        use crate::storage::{
            home::{self, Layout, Mode, StorageConfig},
            recon_jobs::{self, AppJob},
            repo::Repository,
        };
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path());
        home::save_config(
            &layout,
            &StorageConfig {
                mode: Mode::Embedded,
                port: None,
            },
        )
        .unwrap();
        let repo = Repository::open_home(&layout).await.unwrap();
        crate::storage::processes::beat(repo.db(), "fresh", std::process::id(), chrono::Utc::now())
            .await
            .unwrap();
        for (id, operation, owner) in [
            ("api", "api_server", "gone"),
            ("web", "web_server", "gone"),
            ("live", "api_server", "fresh"),
            ("index", "local_index", "gone"),
        ] {
            recon_jobs::begin_app_job(
                &repo,
                &AppJob {
                    id,
                    operation,
                    app: "system",
                    title: id,
                    run_ref: "",
                    resource_ref: "",
                    parent: None,
                },
            )
            .await
            .unwrap();
            repo.db().query("UPDATE type::record('job', $id) SET kind = 'service', worker_owner = $owner RETURN NONE").bind(("id", id.to_string())).bind(("owner", owner.to_string())).await.unwrap().check().unwrap();
        }
        assert_eq!(super::surreal_recover(&repo, "fresh").await.unwrap(), 2);
        for (id, expected) in [
            ("api", "failed"),
            ("web", "failed"),
            ("live", "running"),
            ("index", "running"),
        ] {
            assert_eq!(
                crate::storage::jobs_view::get_job(repo.db(), id)
                    .await
                    .unwrap()
                    .unwrap()
                    .state,
                expected
            );
        }
    }
}
