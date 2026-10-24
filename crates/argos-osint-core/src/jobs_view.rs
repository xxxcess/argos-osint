//! Read models and safe controls for the Jobs and Logs dashboards.
//!
//! Jobs is a view over `argos_jobs` / `argos_tasks` / `argos_attempts`, not a
//! new executor. Every query is bounded. Unknown historic timing stays `None`
//! ("Unavailable"), never zero. Retry only requeues failed tasks whose
//! operation is owned by a durable worker pool, so successful children are
//! never repeated and unsupported operations expose no Retry.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::events::{EventFilter, EventRow, NewEvent};
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
    Stopped,
}

impl JobStatusFilter {
    pub const ALL: [Self; 6] = [
        Self::All,
        Self::Active,
        Self::Retrying,
        Self::Failed,
        Self::Completed,
        Self::Stopped,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Active => "Active",
            Self::Retrying => "Retrying",
            Self::Failed => "Failed",
            Self::Completed => "Completed",
            Self::Stopped => "Stopped",
        }
    }

    pub(crate) fn states(self) -> &'static [&'static str] {
        match self {
            Self::All => &[],
            Self::Active => &["running", "queued", "retry_scheduled"],
            Self::Retrying => &["retry_scheduled"],
            Self::Failed => &["failed", "partial"],
            Self::Completed => &["completed"],
            Self::Stopped => &["cancelled", "superseded"],
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
            "running" | "queued" | "retry_scheduled"
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
            "waiting" => "waiting",
            "completed" => "completed",
            "partial" => "partial",
            "failed" => "failed",
            "cancelled" => "stopped",
            "superseded" => "superseded",
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
    /// Shared Intel brief stage row ("Inferences · Failed · …"); empty for
    /// other tasks.
    #[serde(default)]
    pub stage_label: String,
}

/// Display label for a task's operation: brief stages use their stage name.
pub fn task_label(operation: &str) -> String {
    crate::intel_recon::brief_pipeline::STAGES
        .iter()
        .find(|(op, _)| *op == operation)
        .map_or_else(|| operation.to_string(), |(_, label)| (*label).to_string())
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
    /// Human label of the owning task (a brief stage or the operation).
    #[serde(default)]
    pub task_label: String,
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
    /// Provider requests actually sent for this job's tasks (a task attempt
    /// may dispatch several across retries and fallback routes).
    #[serde(default)]
    pub wire_requests: i64,
    /// Completed-with-warnings notes ("Inferences: 1 of 2 claims could not be
    /// analyzed (timeout)"): the job produced a usable partial result.
    #[serde(default)]
    pub warnings: Vec<String>,
    /// Failed tasks a durable pool can safely rerun.
    pub retryable: i64,
    /// The job finished more than the event retention ago and no events remain.
    pub events_expired: bool,
    #[serde(default)]
    pub insight_progress: Vec<crate::investigation::insight_execution::Progress>,
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
    j.active_since, j.cancel_requested,
    (j.kind<>'service' AND (j.cancellable OR EXISTS(SELECT 1 FROM argos_tasks t WHERE t.job_id=j.id)))";

fn retryable_sql() -> String {
    let ops = tasks::INDEX_OPERATIONS
        .iter()
        .chain(tasks::SUMMARY_OPERATIONS.iter())
        .map(|op| format!("'{op}'"))
        .collect::<Vec<_>>()
        .join(",");
    format!("state='failed' AND operation IN ({ops})")
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

/// Store-level access for the TUI (which has no raw connection).
impl Store {
    pub fn list_jobs(&self, filter: &JobFilter, limit: usize) -> Result<Vec<JobRow>> {
        crate::storage::jobs_store::list_jobs(filter, limit)
    }

    pub fn job_counts(&self) -> Result<JobCounts> {
        crate::storage::jobs_store::job_counts()
    }

    pub fn job_detail(&self, id: &str, task_limit: usize) -> Result<Option<JobDetail>> {
        crate::storage::jobs_store::job_detail(id, task_limit)
    }

    pub fn get_job(&self, id: &str) -> Result<Option<JobRow>> {
        crate::storage::jobs_store::get_job(id)
    }

    pub fn retry_failed_tasks(&self, id: &str) -> Result<usize> {
        crate::storage::jobs_store::retry_failed_tasks(id)
    }
    pub fn set_job_stage_coverage(&self, job_id: &str, coverage_json: &str) -> Result<()> {
        crate::storage::jobs_store::set_job_stage_coverage(job_id, coverage_json)
    }

    pub fn job_apps(&self) -> Result<Vec<String>> {
        crate::storage::jobs_store::job_apps()
    }

    /// Jobs → Stop: dequeue/fence durable work, or request cooperative
    /// cancellation from its registered owner. Service loops are excluded.
    pub fn request_job_cancel(&self, id: &str) -> Result<crate::job_registry::CancelRequest> {
        crate::storage::jobs_store::request_job_cancel(id)
    }

    /// Register a user-visible job before launching its work (idempotent).
    pub fn register_job(&self, job: &tasks::NewJob, meta: &tasks::JobMeta) -> Result<()> {
        crate::storage::jobs_store::register_job(job, meta)
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
        crate::storage::jobs_store::set_job_progress(id, state, phase, done, total, error_summary)
    }

    pub fn record_event(&self, event: &NewEvent) -> Result<String> {
        crate::storage::jobs_store::record_event(event)
    }

    pub fn list_events(&self, filter: &EventFilter, limit: usize) -> Result<Vec<EventRow>> {
        crate::storage::jobs_store::list_events(filter, limit)
    }

    pub fn get_event(&self, id: &str) -> Result<Option<EventRow>> {
        crate::storage::jobs_store::get_event(id)
    }

    pub fn event_counts(&self) -> Result<EventCounts> {
        crate::storage::jobs_store::event_counts()
    }

    pub fn event_apps(&self) -> Result<Vec<String>> {
        crate::storage::jobs_store::event_apps()
    }

    /// Delete events past retention. Touches only `argos_events`.
    pub fn prune_events(&self, hours: i64) -> Result<usize> {
        crate::storage::jobs_store::prune_events(hours)
    }

    /// Logs "Clear events". Touches only `argos_events`: jobs, task results and
    /// memories are kept.
    pub fn clear_events(&self) -> Result<usize> {
        crate::storage::jobs_store::clear_events()
    }
}

#[cfg(test)]
mod tests {

    fn now() -> String {
        chrono::Utc::now().to_rfc3339()
    }
}
