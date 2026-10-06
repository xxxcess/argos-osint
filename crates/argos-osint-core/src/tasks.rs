//! Shared durable task service for Argos workflows.
//!
//! One scheduler owns admission, retries, leases and lifecycle. Workflow code
//! (Recon, Atlas, Intel Recon, Summarization) submits jobs; this module records
//! attempts and enforces the unified retry policy (Summarization: 2 total
//! attempts; other LLM ops: 3) and the shared two-concurrent-LLM-requests cap
//! per provider/account.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Default total provider attempts for Summarization tasks (first try counts).
pub const SUMMARIZATION_ATTEMPTS: u32 = 2;
/// Default total provider attempts for other retry-eligible LLM operations.
pub const DEFAULT_LLM_ATTEMPTS: u32 = 3;
/// Shared concurrent LLM requests per provider/account across all roles.
pub const DEFAULT_PROVIDER_CONCURRENCY: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Queued,
    Running,
    RetryScheduled,
    Paused,
    Completed,
    Failed,
    Cancelled,
    Superseded,
}

impl TaskState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::RetryScheduled => "retry_scheduled",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Superseded => "superseded",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw {
            "running" => Self::Running,
            "retry_scheduled" => Self::RetryScheduled,
            "paused" => Self::Paused,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "superseded" => Self::Superseded,
            _ => Self::Queued,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    RateLimit,
    TemporaryNetwork,
    Timeout,
    ContextLimit,
    InvalidResult,
    AuthOrQuota,
    UnsupportedOption,
    InterruptedStream,
    CancelledOrStale,
    /// Local vector index write/verification failed (retryable).
    IndexFailure,
    /// A required configuration is missing (e.g. embeddings disabled). Blocks, never retries blindly.
    ConfigurationMissing,
    /// Lease expired before the worker acknowledged the attempt.
    LeaseExpired,
    Unknown,
}

impl ErrorCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RateLimit => "rate_limit",
            Self::TemporaryNetwork => "temporary_network",
            Self::Timeout => "timeout",
            Self::ContextLimit => "context_limit",
            Self::InvalidResult => "invalid_result",
            Self::AuthOrQuota => "auth_or_quota",
            Self::UnsupportedOption => "unsupported_option",
            Self::InterruptedStream => "interrupted_stream",
            Self::CancelledOrStale => "cancelled_or_stale",
            Self::IndexFailure => "index_failure",
            Self::ConfigurationMissing => "configuration_missing",
            Self::LeaseExpired => "lease_expired",
            Self::Unknown => "unknown",
        }
    }

    pub fn retryable(self) -> bool {
        matches!(
            self,
            Self::RateLimit
                | Self::TemporaryNetwork
                | Self::Timeout
                | Self::InterruptedStream
                | Self::InvalidResult
                | Self::IndexFailure
                | Self::LeaseExpired
        )
    }

    pub fn parse(raw: &str) -> Self {
        match raw {
            "rate_limit" => Self::RateLimit,
            "temporary_network" => Self::TemporaryNetwork,
            "timeout" => Self::Timeout,
            "context_limit" => Self::ContextLimit,
            "invalid_result" => Self::InvalidResult,
            "auth_or_quota" => Self::AuthOrQuota,
            "unsupported_option" => Self::UnsupportedOption,
            "interrupted_stream" => Self::InterruptedStream,
            "cancelled_or_stale" => Self::CancelledOrStale,
            "index_failure" => Self::IndexFailure,
            "configuration_missing" => Self::ConfigurationMissing,
            "lease_expired" => Self::LeaseExpired,
            _ => Self::Unknown,
        }
    }
}

/// Typed worker failure. Error text is never stored as a successful completion result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskError {
    pub category: ErrorCategory,
    pub message: String,
}

impl TaskError {
    pub fn new(category: ErrorCategory, message: impl Into<String>) -> Self {
        Self {
            category,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for TaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.category.as_str(), self.message)
    }
}

impl std::error::Error for TaskError {}

/// Worker pool names. Each pool claims only the operations routed to it, so a
/// summary worker can never claim (or complete) Atlas publication or index work.
pub const POOL_SUMMARY: &str = "summary";
pub const POOL_INDEX: &str = "index";
pub const POOL_LLM: &str = "llm";
pub const POOL_NETWORK: &str = "network";
pub const POOL_ATLAS: &str = "atlas";

/// Summarization operation names (one per summary mode, plus the generic flush).
pub const SUMMARY_OPERATIONS: &[&str] = &[
    "page_evidence",
    "graph_explanation",
    "follow_up_context",
    "tool_observation",
    "investigation_title",
    "report_context",
    "section_digest",
    "atlas_brief",
    "article_description",
    "summarization",
];

/// Local index operations executed by the index pool.
pub const INDEX_OPERATIONS: &[&str] = &[
    "index_upsert",
    "index_remove",
    "index_rebuild",
    "upsert",
    "remove",
    "rebuild",
    "generation_rebuild",
];

/// Route an operation to exactly one worker pool.
pub fn pool_for_operation(operation: &str) -> &'static str {
    if SUMMARY_OPERATIONS.contains(&operation) {
        POOL_SUMMARY
    } else if INDEX_OPERATIONS.contains(&operation) {
        POOL_INDEX
    } else if operation == "osint_collect" {
        POOL_NETWORK
    } else if operation.starts_with("atlas_") {
        POOL_ATLAS
    } else {
        POOL_LLM
    }
}

/// Map an operation name onto its attempt policy kind.
pub fn operation_kind(operation: &str) -> OperationKind {
    match pool_for_operation(operation) {
        POOL_SUMMARY => OperationKind::Summarization,
        POOL_INDEX => OperationKind::LocalIndex,
        POOL_NETWORK => OperationKind::NetworkCollect,
        _ => OperationKind::OtherLlm,
    }
}

/// Operation family used to pick the attempt budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationKind {
    Summarization,
    OtherLlm,
    LocalIndex,
    NetworkCollect,
}

impl OperationKind {
    pub fn attempt_cap(self) -> u32 {
        match self {
            Self::Summarization => SUMMARIZATION_ATTEMPTS,
            Self::OtherLlm => DEFAULT_LLM_ATTEMPTS,
            Self::LocalIndex | Self::NetworkCollect => 3,
        }
    }
}

/// Process-local admission control for provider/account slots.
#[derive(Default)]
pub struct ProviderAdmission {
    /// key: provider kind / account id → active count
    active: HashMap<String, usize>,
    /// Shared cooldown after a rate-limit / 429 (spec §19). Blocks new acquires
    /// for the account until `Instant` elapses.
    cooldown_until: HashMap<String, Instant>,
}

impl ProviderAdmission {
    pub fn global() -> &'static Mutex<ProviderAdmission> {
        static SLOT: OnceLock<Mutex<ProviderAdmission>> = OnceLock::new();
        SLOT.get_or_init(|| Mutex::new(ProviderAdmission::default()))
    }

    pub fn try_acquire(&mut self, account: &str, limit: usize) -> bool {
        if let Some(until) = self.cooldown_until.get(account) {
            if Instant::now() < *until {
                return false;
            }
            self.cooldown_until.remove(account);
        }
        let slot = self.active.entry(account.to_string()).or_insert(0);
        if *slot >= limit {
            return false;
        }
        *slot += 1;
        true
    }

    pub fn release(&mut self, account: &str) {
        if let Some(slot) = self.active.get_mut(account) {
            *slot = slot.saturating_sub(1);
        }
    }

    pub fn active(&self, account: &str) -> usize {
        self.active.get(account).copied().unwrap_or(0)
    }

    /// Record a shared cooldown for this account (e.g. after HTTP 429).
    pub fn note_rate_limit(&mut self, account: &str, cooldown: Duration) {
        let until = Instant::now() + cooldown;
        let slot = self.cooldown_until.entry(account.to_string()).or_insert(until);
        if until > *slot {
            *slot = until;
        }
    }

    pub fn cooling_down(&self, account: &str) -> bool {
        self.cooldown_until
            .get(account)
            .is_some_and(|until| Instant::now() < *until)
    }
}

/// Guard that releases a provider slot when dropped.
pub struct AdmissionGuard {
    account: String,
}

impl AdmissionGuard {
    pub fn try_enter(account: &str) -> Option<Self> {
        let mut gate = ProviderAdmission::global().lock().ok()?;
        if gate.try_acquire(account, DEFAULT_PROVIDER_CONCURRENCY) {
            Some(Self {
                account: account.to_string(),
            })
        } else {
            None
        }
    }
}

impl Drop for AdmissionGuard {
    fn drop(&mut self) {
        if let Ok(mut gate) = ProviderAdmission::global().lock() {
            gate.release(&self.account);
        }
    }
}

/// Apply a process-wide cooldown for `account` after a rate-limit response.
pub fn note_shared_rate_limit(account: &str, cooldown: Duration) {
    if let Ok(mut gate) = ProviderAdmission::global().lock() {
        gate.note_rate_limit(account, cooldown);
    }
}

/// Decide whether another provider attempt is allowed.
pub fn can_retry(kind: OperationKind, attempts_used: u32, category: ErrorCategory) -> bool {
    category.retryable() && attempts_used < kind.attempt_cap()
}

/// Jittered backoff for retry_scheduled tasks (2s base, 30s cap).
pub fn backoff_delay(attempt: u32) -> Duration {
    let exp = 2u64.saturating_pow(attempt.saturating_sub(1).min(4));
    let secs = (2 * exp).min(30);
    Duration::from_secs(secs)
}

/// Apply the shared durable-task schema (versioned by the caller via store migrate).
pub fn migrate_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS argos_jobs (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            owner_scope TEXT NOT NULL DEFAULT '',
            input_revision TEXT NOT NULL DEFAULT '',
            state TEXT NOT NULL,
            reason TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            deadline_at TEXT NOT NULL DEFAULT '',
            resource_json TEXT NOT NULL DEFAULT '{}'
        );
        CREATE TABLE IF NOT EXISTS argos_tasks (
            id TEXT PRIMARY KEY,
            job_id TEXT NOT NULL,
            operation TEXT NOT NULL,
            dedupe_key TEXT NOT NULL DEFAULT '',
            priority INTEGER NOT NULL DEFAULT 100,
            input_ref TEXT NOT NULL DEFAULT '',
            input_hash TEXT NOT NULL DEFAULT '',
            source_revision TEXT NOT NULL DEFAULT '',
            role_snapshot TEXT NOT NULL DEFAULT '',
            state TEXT NOT NULL,
            next_eligible_at TEXT NOT NULL DEFAULT '',
            attempts INTEGER NOT NULL DEFAULT 0,
            max_attempts INTEGER NOT NULL DEFAULT 3,
            result_ref TEXT NOT NULL DEFAULT '',
            error_category TEXT NOT NULL DEFAULT '',
            error_message TEXT NOT NULL DEFAULT '',
            lease_owner TEXT NOT NULL DEFAULT '',
            lease_until TEXT NOT NULL DEFAULT '',
            lease_epoch INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY(job_id) REFERENCES argos_jobs(id)
        );
        CREATE INDEX IF NOT EXISTS argos_tasks_ready
            ON argos_tasks(state, next_eligible_at, priority, created_at);
        CREATE TABLE IF NOT EXISTS argos_attempts (
            id TEXT PRIMARY KEY,
            task_id TEXT NOT NULL,
            attempt_number INTEGER NOT NULL,
            worker_epoch INTEGER NOT NULL DEFAULT 0,
            provider_request_id TEXT NOT NULL DEFAULT '',
            started_at TEXT NOT NULL,
            finished_at TEXT NOT NULL DEFAULT '',
            error_category TEXT NOT NULL DEFAULT '',
            http_status INTEGER NOT NULL DEFAULT 0,
            retry_decision TEXT NOT NULL DEFAULT '',
            usage_json TEXT NOT NULL DEFAULT '{}',
            FOREIGN KEY(task_id) REFERENCES argos_tasks(id)
        );
        CREATE TABLE IF NOT EXISTS argos_index_changes (
            seq INTEGER PRIMARY KEY AUTOINCREMENT,
            record_kind TEXT NOT NULL,
            record_id TEXT NOT NULL,
            revision TEXT NOT NULL DEFAULT '',
            operation TEXT NOT NULL,
            generation TEXT NOT NULL DEFAULT '',
            state TEXT NOT NULL DEFAULT 'pending',
            outcome TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS argos_derived_summaries (
            id TEXT PRIMARY KEY,
            mode TEXT NOT NULL,
            source_id TEXT NOT NULL,
            source_revision TEXT NOT NULL DEFAULT '',
            source_hash TEXT NOT NULL DEFAULT '',
            focus_hash TEXT NOT NULL DEFAULT '',
            budget INTEGER NOT NULL DEFAULT 0,
            model TEXT NOT NULL DEFAULT '',
            provider TEXT NOT NULL DEFAULT '',
            prompt_version TEXT NOT NULL DEFAULT '',
            content TEXT NOT NULL,
            source_refs_json TEXT NOT NULL DEFAULT '[]',
            coverage_json TEXT NOT NULL DEFAULT '{}',
            fallback INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS argos_derived_summaries_key
            ON argos_derived_summaries(mode, source_id, source_revision, focus_hash, budget, model, prompt_version);
        ",
    )?;
    migrate_additive(conn)?;
    Ok(())
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn add_missing_columns(conn: &Connection, table: &str, columns: &[(&str, &str)]) -> Result<()> {
    let have = table_columns(conn, table)?;
    for (name, decl) in columns {
        if !have.iter().any(|c| c == name) {
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {name} {decl}"))?;
        }
    }
    Ok(())
}

/// Additive, idempotent columns and tables for job timing, pool routing, leased
/// index work, durable events, Atlas publication receipts/checkpoints and
/// deletion tombstones. Defaults keep older rows readable; NULL timing means
/// "Unavailable", never zero.
pub fn migrate_additive(conn: &Connection) -> Result<()> {
    add_missing_columns(
        conn,
        "argos_jobs",
        &[
            ("parent_id", "TEXT NOT NULL DEFAULT ''"),
            ("app", "TEXT NOT NULL DEFAULT ''"),
            ("operation", "TEXT NOT NULL DEFAULT ''"),
            ("title", "TEXT NOT NULL DEFAULT ''"),
            ("run_ref", "TEXT NOT NULL DEFAULT ''"),
            ("resource_ref", "TEXT NOT NULL DEFAULT ''"),
            ("phase", "TEXT NOT NULL DEFAULT ''"),
            ("progress_done", "INTEGER"),
            ("progress_total", "INTEGER"),
            ("queued_at", "TEXT NOT NULL DEFAULT ''"),
            ("started_at", "TEXT NOT NULL DEFAULT ''"),
            ("heartbeat_at", "TEXT NOT NULL DEFAULT ''"),
            ("finished_at", "TEXT NOT NULL DEFAULT ''"),
            ("active_ms", "INTEGER"),
            ("queue_ms", "INTEGER"),
            ("retry_wait_ms", "INTEGER"),
            ("attempts_used", "INTEGER NOT NULL DEFAULT 0"),
            ("attempt_cap", "INTEGER NOT NULL DEFAULT 0"),
            ("provider", "TEXT NOT NULL DEFAULT ''"),
            ("model", "TEXT NOT NULL DEFAULT ''"),
            ("tool", "TEXT NOT NULL DEFAULT ''"),
            ("worker_owner", "TEXT NOT NULL DEFAULT ''"),
            ("worker_epoch", "INTEGER NOT NULL DEFAULT 0"),
            ("result_ref", "TEXT NOT NULL DEFAULT ''"),
            ("error_category", "TEXT NOT NULL DEFAULT ''"),
            ("error_summary", "TEXT NOT NULL DEFAULT ''"),
            ("correlation_id", "TEXT NOT NULL DEFAULT ''"),
        ],
    )?;
    add_missing_columns(
        conn,
        "argos_tasks",
        &[
            ("pool", "TEXT NOT NULL DEFAULT ''"),
            ("parent_task_id", "TEXT NOT NULL DEFAULT ''"),
            ("started_at", "TEXT NOT NULL DEFAULT ''"),
            ("heartbeat_at", "TEXT NOT NULL DEFAULT ''"),
            ("finished_at", "TEXT NOT NULL DEFAULT ''"),
            ("active_ms", "INTEGER NOT NULL DEFAULT 0"),
            ("queue_ms", "INTEGER"),
            ("retry_wait_ms", "INTEGER NOT NULL DEFAULT 0"),
            ("retry_since", "TEXT NOT NULL DEFAULT ''"),
            ("provider", "TEXT NOT NULL DEFAULT ''"),
            ("model", "TEXT NOT NULL DEFAULT ''"),
            ("tool", "TEXT NOT NULL DEFAULT ''"),
            ("blocked_reason", "TEXT NOT NULL DEFAULT ''"),
            ("correlation_id", "TEXT NOT NULL DEFAULT ''"),
        ],
    )?;
    add_missing_columns(
        conn,
        "argos_attempts",
        &[
            ("owner", "TEXT NOT NULL DEFAULT ''"),
            ("duration_ms", "INTEGER"),
            ("outcome", "TEXT NOT NULL DEFAULT ''"),
            ("error_message", "TEXT NOT NULL DEFAULT ''"),
            ("stage", "TEXT NOT NULL DEFAULT ''"),
            ("details_json", "TEXT NOT NULL DEFAULT '{}'"),
        ],
    )?;
    add_missing_columns(
        conn,
        "argos_index_changes",
        &[
            ("task_id", "TEXT NOT NULL DEFAULT ''"),
            ("job_id", "TEXT NOT NULL DEFAULT ''"),
            ("work_key", "TEXT NOT NULL DEFAULT ''"),
            ("content_hash", "TEXT NOT NULL DEFAULT ''"),
            ("fingerprint", "TEXT NOT NULL DEFAULT ''"),
            ("generation_served", "TEXT NOT NULL DEFAULT ''"),
            ("error_category", "TEXT NOT NULL DEFAULT ''"),
            ("error_message", "TEXT NOT NULL DEFAULT ''"),
        ],
    )?;
    // Backfill pool routing for rows written before pools existed.
    let summary_list = SUMMARY_OPERATIONS
        .iter()
        .map(|op| format!("'{op}'"))
        .collect::<Vec<_>>()
        .join(",");
    let index_list = INDEX_OPERATIONS
        .iter()
        .map(|op| format!("'{op}'"))
        .collect::<Vec<_>>()
        .join(",");
    conn.execute_batch(&format!(
        "UPDATE argos_tasks SET pool = CASE
            WHEN operation IN ({summary_list}) THEN '{POOL_SUMMARY}'
            WHEN operation IN ({index_list}) THEN '{POOL_INDEX}'
            WHEN operation = 'osint_collect' THEN '{POOL_NETWORK}'
            WHEN substr(operation,1,6) = 'atlas_' THEN '{POOL_ATLAS}'
            ELSE '{POOL_LLM}' END
         WHERE pool = '';"
    ))?;
    conn.execute_batch(
        "
        CREATE INDEX IF NOT EXISTS argos_tasks_pool_ready
            ON argos_tasks(pool, state, next_eligible_at, priority, created_at);
        CREATE INDEX IF NOT EXISTS argos_tasks_job ON argos_tasks(job_id);
        CREATE INDEX IF NOT EXISTS argos_attempts_task ON argos_attempts(task_id, attempt_number);
        CREATE INDEX IF NOT EXISTS argos_jobs_parent ON argos_jobs(parent_id);
        CREATE INDEX IF NOT EXISTS argos_jobs_recent ON argos_jobs(state, updated_at);
        CREATE INDEX IF NOT EXISTS argos_index_changes_task ON argos_index_changes(task_id);
        CREATE INDEX IF NOT EXISTS argos_index_changes_record
            ON argos_index_changes(record_kind, record_id, state);
        CREATE UNIQUE INDEX IF NOT EXISTS argos_index_changes_active_work
            ON argos_index_changes(work_key)
            WHERE work_key <> '' AND state IN ('pending','running','blocked');
        CREATE TABLE IF NOT EXISTS argos_events (
            id TEXT PRIMARY KEY,
            seq INTEGER NOT NULL DEFAULT 0,
            ts TEXT NOT NULL,
            severity TEXT NOT NULL,
            app TEXT NOT NULL DEFAULT '',
            event_type TEXT NOT NULL DEFAULT '',
            message TEXT NOT NULL,
            details TEXT NOT NULL DEFAULT '',
            job_id TEXT NOT NULL DEFAULT '',
            task_id TEXT NOT NULL DEFAULT '',
            attempt_id TEXT NOT NULL DEFAULT '',
            run_id TEXT NOT NULL DEFAULT '',
            resource_ref TEXT NOT NULL DEFAULT '',
            correlation_id TEXT NOT NULL DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS argos_events_ts ON argos_events(ts);
        CREATE INDEX IF NOT EXISTS argos_events_job ON argos_events(job_id, ts);
        CREATE TABLE IF NOT EXISTS argos_atlas_checkpoints (
            run_id TEXT NOT NULL,
            revision TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            accepted INTEGER NOT NULL DEFAULT 0,
            rejected INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            PRIMARY KEY(run_id, revision)
        );
        CREATE TABLE IF NOT EXISTS argos_atlas_publications (
            run_id TEXT NOT NULL,
            revision TEXT NOT NULL,
            state TEXT NOT NULL,
            receipt_json TEXT NOT NULL DEFAULT '{}',
            job_id TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY(run_id, revision)
        );
        CREATE TABLE IF NOT EXISTS argos_memory_tombstones (
            memory_id TEXT NOT NULL,
            fingerprint TEXT NOT NULL DEFAULT '',
            reason TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL,
            PRIMARY KEY(memory_id, fingerprint)
        );
        ",
    )?;
    Ok(())
}

#[derive(Clone, Debug)]
pub struct NewJob {
    pub id: String,
    pub kind: String,
    pub owner_scope: String,
    pub input_revision: String,
    pub deadline_at: String,
}

#[derive(Clone, Debug)]
pub struct NewTask {
    pub id: String,
    pub job_id: String,
    pub operation: String,
    pub dedupe_key: String,
    pub priority: i64,
    pub input_ref: String,
    pub input_hash: String,
    pub source_revision: String,
    pub role_snapshot: String,
    pub max_attempts: u32,
}

/// Optional user-visible job metadata (Jobs dashboard). All fields default empty.
#[derive(Clone, Debug, Default)]
pub struct JobMeta {
    pub parent_id: String,
    pub app: String,
    pub operation: String,
    pub title: String,
    pub run_ref: String,
    pub resource_ref: String,
    pub correlation_id: String,
}

pub fn enqueue_job(conn: &Connection, job: &NewJob, now: &str) -> Result<()> {
    enqueue_job_with(conn, job, &JobMeta::default(), now)
}

/// Register a job before launching work. Idempotent on `job.id`.
pub fn enqueue_job_with(conn: &Connection, job: &NewJob, meta: &JobMeta, now: &str) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO argos_jobs(id,kind,owner_scope,input_revision,state,created_at,updated_at,deadline_at,
            parent_id,app,operation,title,run_ref,resource_ref,correlation_id,queued_at)
         VALUES (?1,?2,?3,?4,'queued',?5,?5,?6,?7,?8,?9,?10,?11,?12,?13,?5)",
        params![
            job.id,
            job.kind,
            job.owner_scope,
            job.input_revision,
            now,
            job.deadline_at,
            meta.parent_id,
            meta.app,
            meta.operation,
            meta.title,
            meta.run_ref,
            meta.resource_ref,
            meta.correlation_id,
        ],
    )?;
    Ok(())
}

pub fn enqueue_task(conn: &Connection, task: &NewTask, now: &str) -> Result<bool> {
    if !task.dedupe_key.is_empty() {
        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM argos_tasks WHERE dedupe_key=?1 AND state IN ('queued','running','retry_scheduled','paused') LIMIT 1",
                [&task.dedupe_key],
                |row| row.get(0),
            )
            .optional()?;
        if existing.is_some() {
            return Ok(false);
        }
    }
    let inserted = conn.execute(
        "INSERT INTO argos_tasks(
            id,job_id,operation,dedupe_key,priority,input_ref,input_hash,source_revision,
            role_snapshot,state,attempts,max_attempts,created_at,updated_at,pool
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'queued',0,?10,?11,?11,?12)
         ON CONFLICT(id) DO NOTHING",
        params![
            task.id,
            task.job_id,
            task.operation,
            task.dedupe_key,
            task.priority,
            task.input_ref,
            task.input_hash,
            task.source_revision,
            task.role_snapshot,
            task.max_attempts as i64,
            now,
            pool_for_operation(&task.operation),
        ],
    )?;
    Ok(inserted > 0)
}

/// A task leased to one worker owner at one epoch. Completion, failure and
/// renewal are guarded by `(owner, epoch)` so a stale worker cannot publish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimedTask {
    pub id: String,
    pub job_id: String,
    pub operation: String,
    pub owner: String,
    pub epoch: i64,
    pub attempt: u32,
    pub max_attempts: u32,
    pub input_ref: String,
    pub input_hash: String,
    pub source_revision: String,
    pub attempt_id: String,
    pub started_at: String,
}

fn parse_ts(raw: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|d| d.with_timezone(&chrono::Utc))
}

fn ms_between(start: &str, end: &str) -> Option<i64> {
    let (a, b) = (parse_ts(start)?, parse_ts(end)?);
    Some((b - a).num_milliseconds().max(0))
}

fn lease_deadline(now: &str, lease_secs: i64) -> String {
    let now_dt = parse_ts(now).unwrap_or_else(chrono::Utc::now);
    (now_dt + chrono::Duration::seconds(lease_secs)).to_rfc3339()
}

/// Atomically claim the next ready task routed to one of `pools`.
///
/// One `UPDATE … WHERE id=(SELECT …) RETURNING` statement both selects and
/// leases the row, so two processes cannot own the same attempt; only a row this
/// call actually changed is returned. An empty `pools` slice claims from any pool
/// (legacy/test use only — production workers always pass their pool).
pub fn claim_next_in(
    conn: &Connection,
    pools: &[&str],
    owner: &str,
    lease_secs: i64,
    now: &str,
) -> Result<Option<ClaimedTask>> {
    let lease_until = lease_deadline(now, lease_secs);
    let pool_filter = if pools.is_empty() {
        String::new()
    } else {
        let list = pools
            .iter()
            .map(|p| format!("'{}'", p.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",");
        format!("AND pool IN ({list})")
    };
    let sql = format!(
        "UPDATE argos_tasks SET state='running', lease_owner=?1, lease_until=?2,
            lease_epoch=lease_epoch+1, attempts=attempts+1, updated_at=?3, heartbeat_at=?3,
            started_at=CASE WHEN started_at='' THEN ?3 ELSE started_at END,
            blocked_reason=''
         WHERE id = (
            SELECT id FROM argos_tasks
            WHERE state IN ('queued','retry_scheduled')
              AND (next_eligible_at='' OR next_eligible_at<=?3)
              {pool_filter}
            ORDER BY priority ASC, created_at ASC LIMIT 1)
           AND state IN ('queued','retry_scheduled')
         RETURNING id, job_id, operation, lease_epoch, attempts, max_attempts,
                   input_ref, input_hash, source_revision, created_at, retry_since, queue_ms"
    );
    let row = conn
        .query_row(&sql, params![owner, lease_until, now], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, Option<i64>>(11)?,
            ))
        })
        .optional()?;
    let Some((id, job_id, operation, epoch, attempts, max_attempts, input_ref, input_hash, source_revision, created_at, retry_since, queue_ms)) =
        row
    else {
        return Ok(None);
    };
    // Restart-safe timing: queue wait once (first claim), retry wait per retry.
    let queue = if queue_ms.is_none() {
        ms_between(&created_at, now)
    } else {
        None
    };
    let retry_wait = if retry_since.is_empty() {
        0
    } else {
        ms_between(&retry_since, now).unwrap_or(0)
    };
    conn.execute(
        "UPDATE argos_tasks SET queue_ms=COALESCE(queue_ms, ?1), retry_wait_ms=retry_wait_ms+?2, retry_since=''
         WHERE id=?3 AND lease_epoch=?4",
        params![queue, retry_wait, id, epoch],
    )?;
    let attempt_id = format!("{id}-a{attempts}-e{epoch}");
    conn.execute(
        "INSERT OR REPLACE INTO argos_attempts(id,task_id,attempt_number,worker_epoch,started_at,owner)
         VALUES (?1,?2,?3,?4,?5,?6)",
        params![attempt_id, id, attempts, epoch, now, owner],
    )?;
    let claimed = ClaimedTask {
        id,
        job_id,
        operation,
        owner: owner.to_string(),
        epoch,
        attempt: attempts as u32,
        max_attempts: max_attempts as u32,
        input_ref,
        input_hash,
        source_revision,
        attempt_id,
        started_at: now.to_string(),
    };
    let _ = refresh_job_state(conn, &claimed.job_id, now);
    Ok(Some(claimed))
}

/// Claim the next ready task from any pool, returning its id (legacy helper).
pub fn claim_next(conn: &Connection, owner: &str, lease_secs: i64, now: &str) -> Result<Option<String>> {
    Ok(claim_next_in(conn, &[], owner, lease_secs, now)?.map(|c| c.id))
}

fn finish_attempt(
    conn: &Connection,
    claimed: &ClaimedTask,
    outcome: &str,
    category: &str,
    message: &str,
    now: &str,
) -> Result<i64> {
    let duration = ms_between(&claimed.started_at, now).unwrap_or(0);
    conn.execute(
        "UPDATE argos_attempts SET finished_at=?1, duration_ms=?2, outcome=?3, error_category=?4, error_message=?5
         WHERE id=?6",
        params![now, duration, outcome, category, message, claimed.attempt_id],
    )?;
    Ok(duration)
}

/// Complete a claimed task. Returns false (and changes nothing) when the lease
/// was lost to another owner/epoch.
pub fn complete_claimed(conn: &Connection, claimed: &ClaimedTask, result_ref: &str, now: &str) -> Result<bool> {
    let duration = ms_between(&claimed.started_at, now).unwrap_or(0);
    let n = conn.execute(
        "UPDATE argos_tasks SET state='completed', result_ref=?1, lease_owner='', lease_until='',
            error_category='', error_message='', updated_at=?2, finished_at=?2, active_ms=active_ms+?3
         WHERE id=?4 AND lease_owner=?5 AND lease_epoch=?6 AND state='running'",
        params![result_ref, now, duration, claimed.id, claimed.owner, claimed.epoch],
    )?;
    if n == 0 {
        return Ok(false);
    }
    finish_attempt(conn, claimed, "completed", "", "", now)?;
    let _ = refresh_job_state(conn, &claimed.job_id, now);
    Ok(true)
}

/// Fail a claimed attempt under the shared retry policy. Returns None when the
/// lease was lost (stale worker), otherwise the resulting state.
pub fn fail_claimed(
    conn: &Connection,
    claimed: &ClaimedTask,
    kind: OperationKind,
    error: &TaskError,
    now: &str,
) -> Result<Option<TaskState>> {
    let cap = kind.attempt_cap().min(claimed.max_attempts.max(1));
    let duration = ms_between(&claimed.started_at, now).unwrap_or(0);
    let retry = can_retry(kind, claimed.attempt, error.category) && claimed.attempt < cap;
    let message: String = error.message.chars().take(2000).collect();
    let n = if retry {
        let next = (parse_ts(now).unwrap_or_else(chrono::Utc::now)
            + chrono::Duration::from_std(backoff_delay(claimed.attempt)).unwrap_or_default())
        .to_rfc3339();
        conn.execute(
            "UPDATE argos_tasks SET state='retry_scheduled', next_eligible_at=?1, error_category=?2,
                error_message=?3, lease_owner='', lease_until='', updated_at=?4, retry_since=?4,
                active_ms=active_ms+?5
             WHERE id=?6 AND lease_owner=?7 AND lease_epoch=?8 AND state='running'",
            params![next, error.category.as_str(), message, now, duration, claimed.id, claimed.owner, claimed.epoch],
        )?
    } else {
        conn.execute(
            "UPDATE argos_tasks SET state='failed', error_category=?1, error_message=?2,
                lease_owner='', lease_until='', updated_at=?3, finished_at=?3, active_ms=active_ms+?4
             WHERE id=?5 AND lease_owner=?6 AND lease_epoch=?7 AND state='running'",
            params![error.category.as_str(), message, now, duration, claimed.id, claimed.owner, claimed.epoch],
        )?
    };
    if n == 0 {
        return Ok(None);
    }
    let state = if retry {
        TaskState::RetryScheduled
    } else {
        TaskState::Failed
    };
    conn.execute(
        "UPDATE argos_attempts SET retry_decision=?1 WHERE id=?2",
        params![if retry { "retry" } else { "give_up" }, claimed.attempt_id],
    )?;
    finish_attempt(conn, claimed, "failed", error.category.as_str(), &message, now)?;
    let _ = refresh_job_state(conn, &claimed.job_id, now);
    Ok(Some(state))
}

/// Park a claimed task because required configuration is missing (e.g.
/// embeddings disabled). The attempt is refunded: nothing was tried.
/// [`resume_blocked`] requeues it once the configuration is available.
pub fn block_claimed(conn: &Connection, claimed: &ClaimedTask, reason: &str, now: &str) -> Result<bool> {
    let n = conn.execute(
        "UPDATE argos_tasks SET state='paused', blocked_reason=?1, attempts=MAX(attempts-1,0),
            lease_owner='', lease_until='', updated_at=?2
         WHERE id=?3 AND lease_owner=?4 AND lease_epoch=?5 AND state='running'",
        params![reason, now, claimed.id, claimed.owner, claimed.epoch],
    )?;
    if n == 0 {
        return Ok(false);
    }
    finish_attempt(conn, claimed, "blocked", ErrorCategory::ConfigurationMissing.as_str(), reason, now)?;
    let _ = refresh_job_state(conn, &claimed.job_id, now);
    Ok(true)
}

/// Requeue a claimed task that made progress but is not done (e.g. one rebuild
/// batch). Progress is not failure: the attempt is refunded.
pub fn requeue_claimed(conn: &Connection, claimed: &ClaimedTask, delay: Duration, note: &str, now: &str) -> Result<bool> {
    let duration = ms_between(&claimed.started_at, now).unwrap_or(0);
    let next = (parse_ts(now).unwrap_or_else(chrono::Utc::now)
        + chrono::Duration::from_std(delay).unwrap_or_default())
    .to_rfc3339();
    let n = conn.execute(
        "UPDATE argos_tasks SET state='queued', next_eligible_at=?1, attempts=MAX(attempts-1,0),
            lease_owner='', lease_until='', updated_at=?2, active_ms=active_ms+?3, result_ref=?4
         WHERE id=?5 AND lease_owner=?6 AND lease_epoch=?7 AND state='running'",
        params![next, now, duration, note, claimed.id, claimed.owner, claimed.epoch],
    )?;
    if n == 0 {
        return Ok(false);
    }
    finish_attempt(conn, claimed, "progress", "", note, now)?;
    let _ = refresh_job_state(conn, &claimed.job_id, now);
    Ok(true)
}

/// Requeue tasks parked for `reason` (e.g. `embeddings_disabled` after enablement).
pub fn resume_blocked(conn: &Connection, reason: &str, now: &str) -> Result<usize> {
    let n = conn.execute(
        "UPDATE argos_tasks SET state='queued', blocked_reason='', next_eligible_at='', updated_at=?1
         WHERE state='paused' AND blocked_reason=?2",
        params![now, reason],
    )?;
    if n > 0 {
        conn.execute(
            "UPDATE argos_index_changes SET state='pending', updated_at=?1
             WHERE state='blocked' AND task_id IN (SELECT id FROM argos_tasks WHERE state='queued')",
            [now],
        )?;
    }
    Ok(n)
}

/// Unguarded completion (legacy). Production workers use [`complete_claimed`].
pub fn complete_task(conn: &Connection, id: &str, result_ref: &str, now: &str) -> Result<()> {
    conn.execute(
        "UPDATE argos_tasks SET state='completed', result_ref=?1, lease_owner='', lease_until='',
            error_category='', error_message='', updated_at=?2, finished_at=?2 WHERE id=?3",
        params![result_ref, now, id],
    )?;
    Ok(())
}

pub fn fail_or_retry(
    conn: &Connection,
    id: &str,
    kind: OperationKind,
    category: ErrorCategory,
    message: &str,
    now: &str,
) -> Result<TaskState> {
    let (attempts, max_attempts): (u32, u32) = conn.query_row(
        "SELECT attempts, max_attempts FROM argos_tasks WHERE id=?1",
        [id],
        |row| Ok((row.get::<_, i64>(0)? as u32, row.get::<_, i64>(1)? as u32)),
    )?;
    let cap = kind.attempt_cap().min(max_attempts);
    if can_retry(kind, attempts, category) && attempts < cap {
        let delay = backoff_delay(attempts);
        let next = (chrono::Utc::now() + chrono::Duration::from_std(delay).unwrap_or_default())
            .to_rfc3339();
        conn.execute(
            "UPDATE argos_tasks SET state='retry_scheduled', next_eligible_at=?1,
                error_category=?2, error_message=?3, lease_owner='', lease_until='', updated_at=?4, retry_since=?4
             WHERE id=?5",
            params![next, category.as_str(), message, now, id],
        )?;
        Ok(TaskState::RetryScheduled)
    } else {
        conn.execute(
            "UPDATE argos_tasks SET state='failed', error_category=?1, error_message=?2,
                lease_owner='', lease_until='', updated_at=?3, finished_at=?3 WHERE id=?4",
            params![category.as_str(), message, now, id],
        )?;
        Ok(TaskState::Failed)
    }
}

pub fn renew_lease(conn: &Connection, id: &str, owner: &str, epoch: i64, lease_secs: i64, now: &str) -> Result<bool> {
    let lease_until = lease_deadline(now, lease_secs);
    let n = conn.execute(
        "UPDATE argos_tasks SET lease_until=?1, updated_at=?2, heartbeat_at=?2
         WHERE id=?3 AND lease_owner=?4 AND lease_epoch=?5 AND state='running'",
        params![lease_until, now, id, owner, epoch],
    )?;
    Ok(n > 0)
}

/// Recover running tasks (general and index) whose lease expired: the open
/// attempt is closed as `lease_expired`; the task requeues while attempts remain
/// and fails otherwise, so nothing stays "running" forever after a crash.
pub fn interrupt_expired_leases(conn: &Connection, now: &str) -> Result<usize> {
    let expired: Vec<(String, String, i64, i64, i64, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, job_id, lease_epoch, attempts, max_attempts, operation FROM argos_tasks
             WHERE state='running' AND lease_until<>'' AND lease_until<?1",
        )?;
        let rows = stmt
            .query_map([now], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let mut recovered = 0usize;
    for (id, job_id, epoch, attempts, max_attempts, operation) in expired {
        let cap = operation_kind(&operation).attempt_cap().min(max_attempts.max(1) as u32) as i64;
        let next_state = if attempts >= cap { "failed" } else { "queued" };
        let n = conn.execute(
            "UPDATE argos_tasks SET state=?1, lease_owner='', lease_until='', updated_at=?2,
                error_category='lease_expired', error_message='worker lease expired before acknowledgement',
                finished_at=CASE WHEN ?1='failed' THEN ?2 ELSE finished_at END
             WHERE id=?3 AND lease_epoch=?4 AND state='running'",
            params![next_state, now, id, epoch],
        )?;
        if n == 0 {
            continue;
        }
        conn.execute(
            "UPDATE argos_attempts SET finished_at=?1, outcome='lease_expired', error_category='lease_expired'
             WHERE task_id=?2 AND worker_epoch=?3 AND finished_at=''",
            params![now, id, epoch],
        )?;
        conn.execute(
            "UPDATE argos_index_changes SET state=CASE WHEN ?1='failed' THEN 'failed' ELSE 'pending' END, updated_at=?2
             WHERE task_id=?3 AND state='running'",
            params![next_state, now, id],
        )?;
        let _ = refresh_job_state(conn, &job_id, now);
        recovered += 1;
    }
    Ok(recovered)
}

/// Derive a parent job's aggregate state and timing from its durable tasks.
/// Service jobs (long-lived workers) keep their own state. Parent success
/// requires every task to succeed; a mix of completed and failed is `partial`.
pub fn refresh_job_state(conn: &Connection, job_id: &str, now: &str) -> Result<()> {
    let kind: Option<String> = conn
        .query_row("SELECT kind FROM argos_jobs WHERE id=?1", [job_id], |r| r.get(0))
        .optional()?;
    let Some(kind) = kind else {
        return Ok(());
    };
    if kind == "service" {
        conn.execute(
            "UPDATE argos_jobs SET heartbeat_at=?1, updated_at=?1 WHERE id=?2",
            params![now, job_id],
        )?;
        return Ok(());
    }
    let (total, running, queued, retrying, paused, completed, failed, cancelled): (i64, i64, i64, i64, i64, i64, i64, i64) =
        conn.query_row(
            "SELECT COUNT(*),
                SUM(state='running'), SUM(state='queued'), SUM(state='retry_scheduled'),
                SUM(state='paused'), SUM(state='completed'), SUM(state IN ('failed')),
                SUM(state IN ('cancelled','superseded'))
             FROM argos_tasks WHERE job_id=?1",
            [job_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                    r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                    r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                    r.get::<_, Option<i64>>(5)?.unwrap_or(0),
                    r.get::<_, Option<i64>>(6)?.unwrap_or(0),
                    r.get::<_, Option<i64>>(7)?.unwrap_or(0),
                ))
            },
        )?;
    if total == 0 {
        return Ok(());
    }
    let state = if running > 0 {
        "running"
    } else if retrying > 0 {
        "retry_scheduled"
    } else if queued > 0 {
        "queued"
    } else if paused > 0 {
        "paused"
    } else if failed > 0 && completed > 0 {
        "partial"
    } else if failed > 0 {
        "failed"
    } else if cancelled > 0 && completed == 0 {
        "cancelled"
    } else {
        "completed"
    };
    let terminal = matches!(state, "completed" | "failed" | "partial" | "cancelled");
    let (started, active, queue, retry_wait, attempts, latest_err_cat, latest_err): (
        Option<String>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<String>,
        Option<String>,
    ) = conn.query_row(
        "SELECT MIN(NULLIF(started_at,'')), SUM(active_ms), MAX(queue_ms), SUM(retry_wait_ms), SUM(attempts),
            (SELECT error_category FROM argos_tasks WHERE job_id=?1 AND error_category<>'' ORDER BY updated_at DESC LIMIT 1),
            (SELECT error_message FROM argos_tasks WHERE job_id=?1 AND error_message<>'' ORDER BY updated_at DESC LIMIT 1)
         FROM argos_tasks WHERE job_id=?1",
        [job_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
    )?;
    conn.execute(
        "UPDATE argos_jobs SET state=?1, updated_at=?2, heartbeat_at=?2,
            started_at=CASE WHEN started_at='' THEN IFNULL(?3,'') ELSE started_at END,
            finished_at=CASE WHEN ?4 THEN CASE WHEN finished_at='' THEN ?2 ELSE finished_at END ELSE '' END,
            active_ms=?5, queue_ms=?6, retry_wait_ms=?7, attempts_used=IFNULL(?8,0),
            progress_done=?9, progress_total=?10,
            error_category=IFNULL(?11,''), error_summary=substr(IFNULL(?12,''),1,500)
         WHERE id=?13",
        params![
            state,
            now,
            started,
            terminal,
            active,
            queue,
            retry_wait,
            attempts,
            completed + failed + cancelled,
            total,
            if state == "completed" { None } else { latest_err_cat },
            if state == "completed" { None } else { latest_err },
            job_id
        ],
    )?;
    Ok(())
}

/// Typed result of applying one index change. Only `Ready` acknowledges work.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum IndexOutcome {
    /// Target revision is available to the active index.
    Ready {
        revision: String,
        fingerprint: String,
        generation: String,
    },
    /// Progress was made but the work is not complete (e.g. one rebuild batch).
    Pending { reason: String },
    /// Semantic indexing is intentionally disabled (`ARGOS_EMBED=0`) or unavailable for this store.
    Disabled { reason: String },
    RetryableFailure { message: String },
    PermanentFailure { message: String },
}

impl IndexOutcome {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Ready { .. } => "ready",
            Self::Pending { .. } => "pending",
            Self::Disabled { .. } => "disabled",
            Self::RetryableFailure { .. } => "retryable_failure",
            Self::PermanentFailure { .. } => "permanent_failure",
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| format!("{{\"status\":\"{}\"}}", self.label()))
    }
}

/// Long-lived service job that owns local index work without an explicit parent.
pub const INDEX_SERVICE_JOB: &str = "svc-local-index";

/// Ensure a service-health job row exists for a long-lived worker.
pub fn ensure_service_job(conn: &Connection, id: &str, app: &str, title: &str, now: &str) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO argos_jobs(id,kind,owner_scope,input_revision,state,created_at,updated_at,deadline_at,app,operation,title,queued_at)
         VALUES (?1,'service',?2,'','running',?3,?3,'',?2,'service',?4,?3)",
        params![id, app, now, title],
    )?;
    Ok(())
}

/// One durable index-outbox row and its leased task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexWork {
    pub seq: i64,
    pub task_id: String,
    pub record_kind: String,
    pub record_id: String,
    pub revision: String,
    pub content_hash: String,
    pub operation: String,
}

/// Result of [`enqueue_index_work`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexEnqueue {
    pub seq: i64,
    pub task_id: String,
    /// False when an equivalent active request already existed (coalesced).
    pub created: bool,
}

/// Effective work key: record kind + record id + revision + operation.
pub fn index_work_key(kind: &str, id: &str, revision: &str, operation: &str) -> String {
    format!("{kind}\u{1f}{id}\u{1f}{revision}\u{1f}{operation}")
}

fn normalized_index_operation(operation: &str) -> &'static str {
    match operation {
        "upsert" | "index_upsert" => "index_upsert",
        "remove" | "index_remove" => "index_remove",
        _ => "index_rebuild",
    }
}

/// Enqueue (or coalesce) durable index work with one leased task. Safe inside a
/// caller's open transaction: it issues plain statements only, so publication and
/// its outbox rows commit or roll back together.
#[allow(clippy::too_many_arguments)]
pub fn enqueue_index_work(
    conn: &Connection,
    kind: &str,
    id: &str,
    revision: &str,
    content_hash: &str,
    operation: &str,
    parent_job: Option<&str>,
    now: &str,
) -> Result<IndexEnqueue> {
    let op = normalized_index_operation(operation);
    let key = index_work_key(kind, id, revision, op);
    if let Some((seq, task_id)) = conn
        .query_row(
            "SELECT seq, task_id FROM argos_index_changes
             WHERE work_key=?1 AND state IN ('pending','running','blocked') LIMIT 1",
            [&key],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?
    {
        return Ok(IndexEnqueue {
            seq,
            task_id,
            created: false,
        });
    }
    let job_id = match parent_job {
        Some(job) if !job.is_empty() => job.to_string(),
        _ => {
            ensure_service_job(conn, INDEX_SERVICE_JOB, "brain", "Local index worker", now)?;
            INDEX_SERVICE_JOB.to_string()
        }
    };
    conn.execute(
        "INSERT INTO argos_index_changes(record_kind,record_id,revision,operation,state,created_at,updated_at,work_key,content_hash,job_id)
         VALUES (?1,?2,?3,?4,'pending',?5,?5,?6,?7,?8)",
        params![kind, id, revision, op, now, key, content_hash, job_id],
    )?;
    let seq = conn.last_insert_rowid();
    let task_id = format!("idx-{seq}");
    enqueue_task(
        conn,
        &NewTask {
            id: task_id.clone(),
            job_id: job_id.clone(),
            operation: op.into(),
            dedupe_key: String::new(),
            priority: if op == "index_rebuild" { 80 } else { 60 },
            input_ref: format!("{kind}/{id}"),
            input_hash: content_hash.into(),
            source_revision: revision.into(),
            role_snapshot: String::new(),
            max_attempts: OperationKind::LocalIndex.attempt_cap(),
        },
        now,
    )?;
    conn.execute(
        "UPDATE argos_index_changes SET task_id=?1 WHERE seq=?2",
        params![task_id, seq],
    )?;
    Ok(IndexEnqueue {
        seq,
        task_id,
        created: true,
    })
}

/// Enqueue an index mutation to be processed after the SQLite commit.
pub fn enqueue_index_change(
    conn: &Connection,
    kind: &str,
    id: &str,
    revision: &str,
    operation: &str,
    now: &str,
) -> Result<()> {
    enqueue_index_work(conn, kind, id, revision, revision, operation, None, now)?;
    Ok(())
}

/// Give pre-existing pending outbox rows (written before rows carried a task)
/// a leased task so one lease/retry mechanism covers every index change.
pub fn adopt_untracked_index_changes(conn: &Connection, now: &str) -> Result<usize> {
    let rows: Vec<(i64, String, String, String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT seq, record_kind, record_id, revision, operation FROM argos_index_changes
             WHERE task_id='' AND state IN ('pending','running') ORDER BY seq LIMIT 256",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let mut adopted = 0usize;
    for (seq, kind, id, revision, operation) in rows {
        let op = normalized_index_operation(&operation);
        let key = index_work_key(&kind, &id, &revision, op);
        let duplicate: i64 = conn.query_row(
            "SELECT COUNT(*) FROM argos_index_changes WHERE work_key=?1 AND seq<>?2 AND state IN ('pending','running','blocked')",
            params![key, seq],
            |r| r.get(0),
        )?;
        if duplicate > 0 {
            conn.execute(
                "UPDATE argos_index_changes SET state='superseded', outcome='coalesced', updated_at=?1 WHERE seq=?2",
                params![now, seq],
            )?;
            continue;
        }
        ensure_service_job(conn, INDEX_SERVICE_JOB, "brain", "Local index worker", now)?;
        let task_id = format!("idx-{seq}");
        enqueue_task(
            conn,
            &NewTask {
                id: task_id.clone(),
                job_id: INDEX_SERVICE_JOB.into(),
                operation: op.into(),
                dedupe_key: String::new(),
                priority: 70,
                input_ref: format!("{kind}/{id}"),
                input_hash: revision.clone(),
                source_revision: revision.clone(),
                role_snapshot: String::new(),
                max_attempts: OperationKind::LocalIndex.attempt_cap(),
            },
            now,
        )?;
        conn.execute(
            "UPDATE argos_index_changes SET task_id=?1, work_key=?2, operation=?3, state='pending', job_id=?4, updated_at=?5 WHERE seq=?6",
            params![task_id, key, op, INDEX_SERVICE_JOB, now, seq],
        )?;
        adopted += 1;
    }
    Ok(adopted)
}

/// Claim one leased index task from the index pool together with its outbox row.
pub fn claim_index_work(
    conn: &Connection,
    owner: &str,
    lease_secs: i64,
    now: &str,
) -> Result<Option<(ClaimedTask, IndexWork)>> {
    loop {
        let Some(claimed) = claim_next_in(conn, &[POOL_INDEX], owner, lease_secs, now)? else {
            return Ok(None);
        };
        let work = conn
            .query_row(
                "SELECT seq, task_id, record_kind, record_id, revision, content_hash, operation
                 FROM argos_index_changes WHERE task_id=?1",
                [&claimed.id],
                |r| {
                    Ok(IndexWork {
                        seq: r.get(0)?,
                        task_id: r.get(1)?,
                        record_kind: r.get(2)?,
                        record_id: r.get(3)?,
                        revision: r.get(4)?,
                        content_hash: r.get(5)?,
                        operation: r.get(6)?,
                    })
                },
            )
            .optional()?;
        match work {
            Some(work) => {
                conn.execute(
                    "UPDATE argos_index_changes SET state='running', updated_at=?1 WHERE seq=?2",
                    params![now, work.seq],
                )?;
                return Ok(Some((claimed, work)));
            }
            None => {
                // An index task without an outbox row cannot be executed truthfully.
                fail_claimed(
                    conn,
                    &claimed,
                    OperationKind::LocalIndex,
                    &TaskError::new(ErrorCategory::Unknown, "index task has no outbox row"),
                    now,
                )?;
            }
        }
    }
}

/// Acknowledge index work according to its typed outcome. Returns false when the
/// lease was lost (a stale worker must not publish).
pub fn finish_index_work(
    conn: &Connection,
    claimed: &ClaimedTask,
    work: &IndexWork,
    outcome: &IndexOutcome,
    now: &str,
) -> Result<bool> {
    let json = outcome.to_json();
    let (row_state, owned) = match outcome {
        IndexOutcome::Ready {
            fingerprint,
            generation,
            ..
        } => {
            let ok = complete_claimed(conn, claimed, &json, now)?;
            if ok {
                conn.execute(
                    "UPDATE argos_index_changes SET fingerprint=?1, generation_served=?2 WHERE seq=?3",
                    params![fingerprint, generation, work.seq],
                )?;
            }
            ("completed", ok)
        }
        IndexOutcome::Pending { reason } => (
            "pending",
            requeue_claimed(conn, claimed, Duration::from_secs(1), reason, now)?,
        ),
        IndexOutcome::Disabled { reason } => ("blocked", block_claimed(conn, claimed, reason_key(reason), now)?),
        IndexOutcome::RetryableFailure { message } => {
            let state = fail_claimed(
                conn,
                claimed,
                OperationKind::LocalIndex,
                &TaskError::new(ErrorCategory::IndexFailure, message.clone()),
                now,
            )?;
            match state {
                Some(TaskState::RetryScheduled) => ("pending", true),
                Some(_) => ("failed", true),
                None => ("", false),
            }
        }
        IndexOutcome::PermanentFailure { message } => {
            let state = fail_claimed(
                conn,
                claimed,
                OperationKind::LocalIndex,
                &TaskError::new(ErrorCategory::Unknown, message.clone()),
                now,
            )?;
            ("failed", state.is_some())
        }
    };
    if !owned {
        return Ok(false);
    }
    let (cat, msg) = match outcome {
        IndexOutcome::RetryableFailure { message } => ("index_failure", message.as_str()),
        IndexOutcome::PermanentFailure { message } => ("permanent", message.as_str()),
        IndexOutcome::Disabled { reason } => ("configuration_missing", reason.as_str()),
        _ => ("", ""),
    };
    conn.execute(
        "UPDATE argos_index_changes SET state=?1, outcome=?2, error_category=?3, error_message=?4, updated_at=?5 WHERE seq=?6",
        params![row_state, json, cat, msg, now, work.seq],
    )?;
    Ok(true)
}

/// Blocked tasks for disabled embeddings share one reason key so enablement can resume them.
pub const EMBEDDINGS_DISABLED: &str = "embeddings_disabled";

fn reason_key(reason: &str) -> &str {
    if reason.contains("ARGOS_EMBED") || reason.contains("disabled") {
        EMBEDDINGS_DISABLED
    } else {
        reason
    }
}

/// Legacy batch claim: atomically leases pending rows one at a time and returns
/// only rows this call actually claimed.
pub fn claim_index_changes(conn: &Connection, limit: usize) -> Result<Vec<(i64, String, String, String)>> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut out = Vec::new();
    for _ in 0..limit {
        let row = conn
            .query_row(
                "UPDATE argos_index_changes SET state='running', updated_at=?1
                 WHERE seq = (SELECT seq FROM argos_index_changes WHERE state='pending' ORDER BY seq ASC LIMIT 1)
                   AND state='pending'
                 RETURNING seq, record_kind, record_id, operation",
                [&now],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        match row {
            Some(row) => out.push(row),
            None => break,
        }
    }
    Ok(out)
}

pub fn complete_index_change(conn: &Connection, seq: i64, outcome: &str) -> Result<()> {
    conn.execute(
        "UPDATE argos_index_changes SET state='completed', outcome=?1, updated_at=?2 WHERE seq=?3",
        params![outcome, chrono::Utc::now().to_rfc3339(), seq],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn mem() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate_tables(&conn).unwrap();
        conn
    }

    #[test]
    fn rate_limit_cooldown_blocks_acquire() {
        let mut gate = ProviderAdmission::default();
        assert!(gate.try_acquire("acct", 2));
        gate.release("acct");
        gate.note_rate_limit("acct", Duration::from_secs(30));
        assert!(gate.cooling_down("acct"));
        assert!(!gate.try_acquire("acct", 2));
    }

    #[test]
    fn summarization_gets_two_attempts_other_llm_three() {
        assert_eq!(OperationKind::Summarization.attempt_cap(), 2);
        assert_eq!(OperationKind::OtherLlm.attempt_cap(), 3);
        assert!(can_retry(OperationKind::Summarization, 1, ErrorCategory::RateLimit));
        assert!(!can_retry(OperationKind::Summarization, 2, ErrorCategory::RateLimit));
        assert!(can_retry(OperationKind::OtherLlm, 2, ErrorCategory::TemporaryNetwork));
        assert!(!can_retry(OperationKind::OtherLlm, 3, ErrorCategory::TemporaryNetwork));
        assert!(!can_retry(OperationKind::OtherLlm, 1, ErrorCategory::AuthOrQuota));
    }

    #[test]
    fn index_change_claim_and_complete() {
        let conn = mem();
        let now = chrono::Utc::now().to_rfc3339();
        enqueue_index_change(&conn, "memory_index", "generation", "count=99", "rebuild", &now).unwrap();
        let batch = claim_index_changes(&conn, 10).unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].3, "index_rebuild");
        complete_index_change(&conn, batch[0].0, "enqueued").unwrap();
        let state: String = conn
            .query_row(
                "SELECT state FROM argos_index_changes WHERE seq=?1",
                [batch[0].0],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "completed");
    }

    #[test]
    fn provider_admission_caps_at_two() {
        let mut gate = ProviderAdmission::default();
        assert!(gate.try_acquire("openrouter", 2));
        assert!(gate.try_acquire("openrouter", 2));
        assert!(!gate.try_acquire("openrouter", 2));
        gate.release("openrouter");
        assert!(gate.try_acquire("openrouter", 2));
    }

    #[test]
    fn claim_complete_and_retry_round_trip() {
        let conn = mem();
        let now = chrono::Utc::now().to_rfc3339();
        enqueue_job(
            &conn,
            &NewJob {
                id: "job-1".into(),
                kind: "summarization".into(),
                owner_scope: "test".into(),
                input_revision: "r1".into(),
                deadline_at: String::new(),
            },
            &now,
        )
        .unwrap();
        assert!(enqueue_task(
            &conn,
            &NewTask {
                id: "task-1".into(),
                job_id: "job-1".into(),
                operation: "graph_explanation".into(),
                dedupe_key: "g1".into(),
                priority: 10,
                input_ref: "mem-1".into(),
                input_hash: "h".into(),
                source_revision: "1".into(),
                role_snapshot: "summarization".into(),
                max_attempts: 2,
            },
            &now,
        )
        .unwrap());
        // Dedupe coalesces.
        assert!(!enqueue_task(
            &conn,
            &NewTask {
                id: "task-2".into(),
                job_id: "job-1".into(),
                operation: "graph_explanation".into(),
                dedupe_key: "g1".into(),
                priority: 10,
                input_ref: "mem-1".into(),
                input_hash: "h".into(),
                source_revision: "1".into(),
                role_snapshot: "summarization".into(),
                max_attempts: 2,
            },
            &now,
        )
        .unwrap());
        let id = claim_next(&conn, "worker-a", 30, &now).unwrap().unwrap();
        assert_eq!(id, "task-1");
        let state = fail_or_retry(
            &conn,
            &id,
            OperationKind::Summarization,
            ErrorCategory::TemporaryNetwork,
            "timeout",
            &now,
        )
        .unwrap();
        assert_eq!(state, TaskState::RetryScheduled);
        // Force eligible.
        conn.execute("UPDATE argos_tasks SET next_eligible_at='' WHERE id=?1", [&id])
            .unwrap();
        let id2 = claim_next(&conn, "worker-a", 30, &now).unwrap().unwrap();
        assert_eq!(id2, "task-1");
        let state = fail_or_retry(
            &conn,
            &id2,
            OperationKind::Summarization,
            ErrorCategory::TemporaryNetwork,
            "timeout again",
            &now,
        )
        .unwrap();
        assert_eq!(state, TaskState::Failed);
    }

    #[test]
    fn stale_lease_renewal_is_rejected() {
        let conn = mem();
        let now = chrono::Utc::now().to_rfc3339();
        enqueue_job(
            &conn,
            &NewJob {
                id: "job-1".into(),
                kind: "test".into(),
                owner_scope: "".into(),
                input_revision: "".into(),
                deadline_at: String::new(),
            },
            &now,
        )
        .unwrap();
        enqueue_task(
            &conn,
            &NewTask {
                id: "task-1".into(),
                job_id: "job-1".into(),
                operation: "x".into(),
                dedupe_key: "".into(),
                priority: 1,
                input_ref: "".into(),
                input_hash: "".into(),
                source_revision: "".into(),
                role_snapshot: "".into(),
                max_attempts: 3,
            },
            &now,
        )
        .unwrap();
        claim_next(&conn, "owner-a", 30, &now).unwrap();
        let epoch: i64 = conn
            .query_row(
                "SELECT lease_epoch FROM argos_tasks WHERE id='task-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(renew_lease(&conn, "task-1", "owner-a", epoch, 30, &now).unwrap());
        assert!(!renew_lease(&conn, "task-1", "owner-b", epoch, 30, &now).unwrap());
        assert!(!renew_lease(&conn, "task-1", "owner-a", epoch + 99, 30, &now).unwrap());
    }

    fn job(conn: &Connection, id: &str, now: &str) {
        enqueue_job(
            conn,
            &NewJob {
                id: id.into(),
                kind: "test".into(),
                owner_scope: String::new(),
                input_revision: String::new(),
                deadline_at: String::new(),
            },
            now,
        )
        .unwrap();
    }

    fn task(conn: &Connection, id: &str, job_id: &str, operation: &str, now: &str) {
        assert!(enqueue_task(
            conn,
            &NewTask {
                id: id.into(),
                job_id: job_id.into(),
                operation: operation.into(),
                dedupe_key: String::new(),
                priority: 10,
                input_ref: String::new(),
                input_hash: String::new(),
                source_revision: String::new(),
                role_snapshot: String::new(),
                max_attempts: operation_kind(operation).attempt_cap(),
            },
            now,
        )
        .unwrap());
    }

    #[test]
    fn summary_pool_never_claims_index_or_atlas_work() {
        let conn = mem();
        let now = chrono::Utc::now().to_rfc3339();
        job(&conn, "j", &now);
        task(&conn, "t-atlas", "j", "atlas_publish", &now);
        enqueue_index_change(&conn, "memory", "m1", "r1", "upsert", &now).unwrap();
        assert!(claim_next_in(&conn, &[POOL_SUMMARY], "sum", 30, &now).unwrap().is_none());
        task(&conn, "t-sum", "j", "atlas_brief", &now);
        let got = claim_next_in(&conn, &[POOL_SUMMARY], "sum", 30, &now).unwrap().unwrap();
        assert_eq!(got.id, "t-sum");
        // The other rows are untouched and still claimable by their own pools.
        let states: Vec<String> = conn
            .prepare("SELECT state FROM argos_tasks WHERE id<>'t-sum' ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(states.iter().all(|s| s == "queued"), "{states:?}");
        assert!(claim_index_work(&conn, "idx", 30, &now).unwrap().is_some());
        assert_eq!(
            claim_next_in(&conn, &[POOL_ATLAS], "atl", 30, &now).unwrap().unwrap().id,
            "t-atlas"
        );
    }

    #[test]
    fn two_processes_cannot_own_the_same_attempt_and_stale_owner_cannot_publish() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let a = Connection::open(&path).unwrap();
        migrate_tables(&a).unwrap();
        let b = Connection::open(&path).unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        job(&a, "j", &now);
        task(&a, "t1", "j", "synthesis", &now);
        let first = claim_next_in(&a, &[POOL_LLM], "proc-a", 30, &now).unwrap();
        let second = claim_next_in(&b, &[POOL_LLM], "proc-b", 30, &now).unwrap();
        assert!(first.is_some() ^ second.is_some());
        let claimed = first.unwrap();
        // Lease expires; another process recovers and re-claims at a new epoch.
        let later = (chrono::Utc::now() + chrono::Duration::seconds(120)).to_rfc3339();
        assert_eq!(interrupt_expired_leases(&b, &later).unwrap(), 1);
        let reclaimed = claim_next_in(&b, &[POOL_LLM], "proc-b", 30, &later).unwrap().unwrap();
        assert!(reclaimed.epoch > claimed.epoch);
        assert!(!complete_claimed(&a, &claimed, "stale", &later).unwrap());
        assert!(fail_claimed(&a, &claimed, OperationKind::OtherLlm, &TaskError::new(ErrorCategory::Timeout, "x"), &later)
            .unwrap()
            .is_none());
        assert!(complete_claimed(&b, &reclaimed, "ok", &later).unwrap());
        let (state, result): (String, String) = a
            .query_row("SELECT state, result_ref FROM argos_tasks WHERE id='t1'", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!((state.as_str(), result.as_str()), ("completed", "ok"));
        let expired: String = a
            .query_row("SELECT outcome FROM argos_attempts WHERE task_id='t1' AND worker_epoch=?1", [claimed.epoch], |r| r.get(0))
            .unwrap();
        assert_eq!(expired, "lease_expired");
    }

    #[test]
    fn expired_index_lease_recovers_and_exhausted_work_fails_not_running_forever() {
        let conn = mem();
        let now = chrono::Utc::now().to_rfc3339();
        enqueue_index_change(&conn, "memory", "m1", "r1", "upsert", &now).unwrap();
        let mut t = chrono::Utc::now();
        for round in 0..3 {
            let ts = t.to_rfc3339();
            let (claimed, _work) = claim_index_work(&conn, "w", 30, &ts).unwrap().expect("claimable");
            assert_eq!(claimed.attempt, round + 1);
            t += chrono::Duration::seconds(120);
            assert_eq!(interrupt_expired_leases(&conn, &t.to_rfc3339()).unwrap(), 1);
        }
        let (task_state, row_state): (String, String) = conn
            .query_row(
                "SELECT t.state, c.state FROM argos_tasks t JOIN argos_index_changes c ON c.task_id=t.id",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((task_state.as_str(), row_state.as_str()), ("failed", "failed"));
    }

    #[test]
    fn index_outbox_coalesces_and_rolls_back_with_its_transaction() {
        let conn = mem();
        let now = chrono::Utc::now().to_rfc3339();
        let a = enqueue_index_work(&conn, "memory", "m1", "r1", "h1", "upsert", None, &now).unwrap();
        let b = enqueue_index_work(&conn, "memory", "m1", "r1", "h1", "index_upsert", None, &now).unwrap();
        assert!(a.created && !b.created);
        assert_eq!(a.seq, b.seq);
        let c = enqueue_index_work(&conn, "memory", "m1", "r2", "h2", "upsert", None, &now).unwrap();
        assert!(c.created, "a new revision is new work");
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        enqueue_index_work(&conn, "memory", "m9", "r1", "h", "upsert", None, &now).unwrap();
        conn.execute_batch("ROLLBACK").unwrap();
        let (rows, tasks): (i64, i64) = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM argos_index_changes), (SELECT COUNT(*) FROM argos_tasks)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((rows, tasks), (2, 2));
    }

    #[test]
    fn disabled_outcome_blocks_without_spending_attempts_and_resumes() {
        let conn = mem();
        let now = chrono::Utc::now().to_rfc3339();
        enqueue_index_change(&conn, "memory", "m1", "r1", "upsert", &now).unwrap();
        let (claimed, work) = claim_index_work(&conn, "w", 30, &now).unwrap().unwrap();
        let outcome = IndexOutcome::Disabled {
            reason: "semantic indexing disabled (ARGOS_EMBED=0)".into(),
        };
        assert!(finish_index_work(&conn, &claimed, &work, &outcome, &now).unwrap());
        let (state, attempts, reason, row): (String, i64, String, String) = conn
            .query_row(
                "SELECT t.state, t.attempts, t.blocked_reason, c.state FROM argos_tasks t JOIN argos_index_changes c ON c.task_id=t.id",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!((state.as_str(), attempts, reason.as_str(), row.as_str()), ("paused", 0, EMBEDDINGS_DISABLED, "blocked"));
        assert!(claim_index_work(&conn, "w", 30, &now).unwrap().is_none());
        assert_eq!(resume_blocked(&conn, EMBEDDINGS_DISABLED, &now).unwrap(), 1);
        let (claimed, work) = claim_index_work(&conn, "w", 30, &now).unwrap().unwrap();
        let ready = IndexOutcome::Ready {
            revision: "r1".into(),
            fingerprint: "fp".into(),
            generation: "gen".into(),
        };
        assert!(finish_index_work(&conn, &claimed, &work, &ready, &now).unwrap());
        let (state, fp): (String, String) = conn
            .query_row("SELECT state, fingerprint FROM argos_index_changes", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!((state.as_str(), fp.as_str()), ("completed", "fp"));
    }

    #[test]
    fn retryable_index_failure_is_not_a_completion() {
        let conn = mem();
        let now = chrono::Utc::now().to_rfc3339();
        enqueue_index_change(&conn, "memory", "m1", "r1", "upsert", &now).unwrap();
        let (claimed, work) = claim_index_work(&conn, "w", 30, &now).unwrap().unwrap();
        let outcome = IndexOutcome::RetryableFailure {
            message: "lance write failed".into(),
        };
        assert!(finish_index_work(&conn, &claimed, &work, &outcome, &now).unwrap());
        let (state, result, cat, row): (String, String, String, String) = conn
            .query_row(
                "SELECT t.state, t.result_ref, t.error_category, c.state FROM argos_tasks t JOIN argos_index_changes c ON c.task_id=t.id",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(state, "retry_scheduled");
        assert!(result.is_empty(), "error text must not be a completion result: {result}");
        assert_eq!(cat, "index_failure");
        assert_eq!(row, "pending");
    }

    #[test]
    fn job_timing_and_partial_aggregate_come_from_durable_tasks() {
        let conn = mem();
        let t0 = chrono::Utc::now();
        let created = t0.to_rfc3339();
        job(&conn, "j", &created);
        task(&conn, "a", "j", "synthesis", &created);
        task(&conn, "b", "j", "synthesis", &created);
        let start = (t0 + chrono::Duration::milliseconds(1500)).to_rfc3339();
        let end = (t0 + chrono::Duration::milliseconds(4000)).to_rfc3339();
        let ca = claim_next_in(&conn, &[POOL_LLM], "w", 30, &start).unwrap().unwrap();
        assert!(complete_claimed(&conn, &ca, "ok", &end).unwrap());
        let cb = claim_next_in(&conn, &[POOL_LLM], "w", 30, &start).unwrap().unwrap();
        fail_claimed(&conn, &cb, OperationKind::OtherLlm, &TaskError::new(ErrorCategory::AuthOrQuota, "401"), &end).unwrap();
        let (state, active, queue, finished, err): (String, Option<i64>, Option<i64>, String, String) = conn
            .query_row(
                "SELECT state, active_ms, queue_ms, finished_at, error_category FROM argos_jobs WHERE id='j'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(state, "partial");
        assert_eq!(active, Some(5000));
        assert_eq!(queue, Some(1500));
        assert!(!finished.is_empty());
        assert_eq!(err, "auth_or_quota");
        let durations: Vec<Option<i64>> = conn
            .prepare("SELECT duration_ms FROM argos_attempts ORDER BY task_id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(durations, vec![Some(2500), Some(2500)]);
    }

    #[test]
    fn legacy_pending_rows_are_adopted_into_leased_tasks() {
        let conn = mem();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO argos_index_changes(record_kind,record_id,revision,operation,state,created_at,updated_at)
             VALUES ('memory_index','generation','count=9','rebuild','pending',?1,?1)",
            [&now],
        )
        .unwrap();
        assert_eq!(adopt_untracked_index_changes(&conn, &now).unwrap(), 1);
        let (claimed, work) = claim_index_work(&conn, "w", 30, &now).unwrap().unwrap();
        assert_eq!(work.operation, "index_rebuild");
        assert_eq!(claimed.operation, "index_rebuild");
    }
}
