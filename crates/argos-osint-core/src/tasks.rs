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
        )
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

pub fn enqueue_job(conn: &Connection, job: &NewJob, now: &str) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO argos_jobs(id,kind,owner_scope,input_revision,state,created_at,updated_at,deadline_at)
         VALUES (?1,?2,?3,?4,'queued',?5,?5,?6)",
        params![
            job.id,
            job.kind,
            job.owner_scope,
            job.input_revision,
            now,
            job.deadline_at
        ],
    )?;
    Ok(())
}

pub fn enqueue_task(conn: &Connection, task: &NewTask, now: &str) -> Result<bool> {
    if !task.dedupe_key.is_empty() {
        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM argos_tasks WHERE dedupe_key=?1 AND state IN ('queued','running','retry_scheduled') LIMIT 1",
                [&task.dedupe_key],
                |row| row.get(0),
            )
            .optional()?;
        if existing.is_some() {
            return Ok(false);
        }
    }
    conn.execute(
        "INSERT INTO argos_tasks(
            id,job_id,operation,dedupe_key,priority,input_ref,input_hash,source_revision,
            role_snapshot,state,attempts,max_attempts,created_at,updated_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'queued',0,?10,?11,?11)",
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
            now
        ],
    )?;
    Ok(true)
}

/// Claim the next ready task for `owner`, returning its id.
pub fn claim_next(conn: &Connection, owner: &str, lease_secs: i64, now: &str) -> Result<Option<String>> {
    let now_dt = chrono::DateTime::parse_from_rfc3339(now)
        .map(|d| d.with_timezone(&chrono::Utc))
        .unwrap_or_else(|_| chrono::Utc::now());
    let lease_until = (now_dt + chrono::Duration::seconds(lease_secs)).to_rfc3339();
    let id: Option<String> = conn
        .query_row(
            "SELECT id FROM argos_tasks
             WHERE state IN ('queued','retry_scheduled')
               AND (next_eligible_at='' OR next_eligible_at<=?1)
             ORDER BY priority ASC, created_at ASC LIMIT 1",
            [now],
            |row| row.get(0),
        )
        .optional()?;
    let Some(id) = id else {
        return Ok(None);
    };
    let changed = conn.execute(
        "UPDATE argos_tasks SET state='running', lease_owner=?1, lease_until=?2,
            lease_epoch=lease_epoch+1, attempts=attempts+1, updated_at=?3
         WHERE id=?4 AND state IN ('queued','retry_scheduled')",
        params![owner, lease_until, now, id],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    Ok(Some(id))
}

pub fn complete_task(conn: &Connection, id: &str, result_ref: &str, now: &str) -> Result<()> {
    conn.execute(
        "UPDATE argos_tasks SET state='completed', result_ref=?1, lease_owner='', lease_until='',
            error_category='', error_message='', updated_at=?2 WHERE id=?3",
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
                error_category=?2, error_message=?3, lease_owner='', lease_until='', updated_at=?4
             WHERE id=?5",
            params![next, category.as_str(), message, now, id],
        )?;
        Ok(TaskState::RetryScheduled)
    } else {
        conn.execute(
            "UPDATE argos_tasks SET state='failed', error_category=?1, error_message=?2,
                lease_owner='', lease_until='', updated_at=?3 WHERE id=?4",
            params![category.as_str(), message, now, id],
        )?;
        Ok(TaskState::Failed)
    }
}

pub fn renew_lease(conn: &Connection, id: &str, owner: &str, epoch: i64, lease_secs: i64, now: &str) -> Result<bool> {
    let now_dt = chrono::DateTime::parse_from_rfc3339(now)
        .map(|d| d.with_timezone(&chrono::Utc))
        .unwrap_or_else(|_| chrono::Utc::now());
    let lease_until = (now_dt + chrono::Duration::seconds(lease_secs)).to_rfc3339();
    let n = conn.execute(
        "UPDATE argos_tasks SET lease_until=?1, updated_at=?2
         WHERE id=?3 AND lease_owner=?4 AND lease_epoch=?5 AND state='running'",
        params![lease_until, now, id, owner, epoch],
    )?;
    Ok(n > 0)
}

pub fn interrupt_expired_leases(conn: &Connection, now: &str) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE argos_tasks SET state='queued', lease_owner='', updated_at=?1
         WHERE state='running' AND lease_until<>'' AND lease_until<?1",
        [now],
    )? )
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
    conn.execute(
        "INSERT INTO argos_index_changes(record_kind,record_id,revision,operation,state,created_at,updated_at)
         VALUES (?1,?2,?3,?4,'pending',?5,?5)",
        params![kind, id, revision, operation, now],
    )?;
    Ok(())
}


/// Claim a batch of pending index changes for a worker.
pub fn claim_index_changes(conn: &Connection, limit: usize) -> Result<Vec<(i64, String, String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT seq, record_kind, record_id, operation FROM argos_index_changes
         WHERE state='pending' ORDER BY seq ASC LIMIT ?1",
    )?;
    let rows = stmt
        .query_map([limit as i64], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (seq, _, _, _) in &rows {
        conn.execute(
            "UPDATE argos_index_changes SET state='running', updated_at=?1 WHERE seq=?2 AND state='pending'",
            params![chrono::Utc::now().to_rfc3339(), seq],
        )?;
    }
    Ok(rows)
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
        assert_eq!(batch[0].3, "rebuild");
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
}
