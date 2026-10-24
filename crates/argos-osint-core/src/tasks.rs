//! Shared durable task service for Argos workflows.
//!
//! One scheduler owns admission, retries, leases and lifecycle. Workflow code
//! (Recon, Atlas, Intel Recon, Summarization) submits jobs; this module records
//! attempts and enforces the unified retry policy (primary: 4 total attempts)
//! and the shared two-concurrent-LLM-requests cap per provider/account.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Default total provider attempts for Summarization tasks (first try counts).
/// Summary-specific cap, including the first request.
pub const SUMMARIZATION_ATTEMPTS: u32 = 2;
/// Default total provider attempts for other retry-eligible LLM operations.
pub const DEFAULT_LLM_ATTEMPTS: u32 = crate::provider_chain::PRIMARY_ATTEMPTS;
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

    /// Map a provider/executor failure category to the task category, so a
    /// stage that died on a timeout or 503 is never reported as a bad answer.
    pub fn from_provider(category: crate::provider_diag::Category) -> Self {
        use crate::provider_diag::Category as C;
        match category {
            C::RateLimited => Self::RateLimit,
            C::Timeout => Self::Timeout,
            C::Server | C::Network => Self::TemporaryNetwork,
            C::StreamInterrupted | C::PrematureEof | C::SseError => Self::InterruptedStream,
            C::Auth | C::Permission => Self::AuthOrQuota,
            C::InvalidModel | C::Unsupported | C::MalformedRequest => Self::UnsupportedOption,
            C::TokenLimit => Self::ContextLimit,
            C::Configuration => Self::ConfigurationMissing,
            C::Cancelled => Self::CancelledOrStale,
            C::MalformedPayload | C::Empty | C::InvalidResult | C::Refused => Self::InvalidResult,
            C::Persistence => Self::Unknown,
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
/// Dedicated pool for dependent Intel brief preparation stages; never
/// claimed by generic/legacy drainers.
pub const POOL_INTEL_BRIEF: &str = "intel_brief";
/// Dedicated pool for durable Brain topology scope analysis.
pub const POOL_BRAIN_SCOPE: &str = "brain_scope";
/// Worker crash/lease-expiry recoveries allowed per task before it fails as an
/// infrastructure fault. Independent of provider attempts.
pub const MAX_WORKER_RECOVERIES: i64 = 3;

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
    "topology_neighborhood",
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
    } else if operation.starts_with("intel_brief_") {
        POOL_INTEL_BRIEF
    } else if operation.starts_with("brain_scope") {
        POOL_BRAIN_SCOPE
    } else {
        POOL_LLM
    }
}

/// Map an operation name onto its attempt policy kind.
pub fn operation_kind(operation: &str) -> OperationKind {
    match operation {
        "atlas_entities" | "atlas_related" | "atlas_metadata" => return OperationKind::LocalIndex,
        "atlas_publishers" | "atlas_collect" | "atlas_collection" => {
            return OperationKind::NetworkCollect
        }
        _ => {}
    }
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
    /// Interactive callers currently waiting per account. While any wait,
    /// background callers do not take a freed slot (no preemption of running
    /// requests; the per-account limit is unchanged).
    interactive_waiting: HashMap<String, usize>,
}

impl ProviderAdmission {
    /// Acquire for an interactive or background caller.
    pub fn try_acquire_class(&mut self, account: &str, limit: usize, interactive: bool) -> bool {
        if !interactive && self.interactive_waiting.get(account).copied().unwrap_or(0) > 0 {
            return false;
        }
        self.try_acquire(account, limit)
    }

    pub fn interactive_waiting(&self, account: &str) -> usize {
        self.interactive_waiting.get(account).copied().unwrap_or(0)
    }

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
        let slot = self
            .cooldown_until
            .entry(account.to_string())
            .or_insert(until);
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
        Self::try_enter_class(account, false)
    }

    /// Enter as an interactive (user-waiting) or background caller.
    pub fn try_enter_class(account: &str, interactive: bool) -> Option<Self> {
        let mut gate = ProviderAdmission::global().lock().ok()?;
        if gate.try_acquire_class(account, DEFAULT_PROVIDER_CONCURRENCY, interactive) {
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

/// Registration of an interactive caller waiting for an account slot.
pub struct InteractiveWait {
    account: String,
}

impl InteractiveWait {
    pub fn register(account: &str) -> Self {
        if let Ok(mut gate) = ProviderAdmission::global().lock() {
            *gate
                .interactive_waiting
                .entry(account.to_string())
                .or_insert(0) += 1;
        }
        Self {
            account: account.to_string(),
        }
    }
}

impl Drop for InteractiveWait {
    fn drop(&mut self) {
        if let Ok(mut gate) = ProviderAdmission::global().lock() {
            if let Some(n) = gate.interactive_waiting.get_mut(&self.account) {
                *n = n.saturating_sub(1);
            }
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
/// Single source of truth for whether a queued task may be leased, written
/// against alias `t` for `argos_tasks` with `?3` bound to "now". Both the
/// candidate selector and the claim `UPDATE` embed it so they cannot drift.
/// - the job is inside its 3-day window, not cancel-requested, not retired;
/// - every dependency has completed;
/// - Intel brief chains: independent roots run together while a dependent
///   stage stays exclusive, and no stage dispatches from an unbound input.
const CLAIM_ELIGIBLE: &str = "(
    EXISTS(SELECT 1 FROM argos_jobs j WHERE j.id=t.job_id AND julianday(j.created_at)>=julianday(?3)-3 AND j.cancel_requested=0)
    AND NOT EXISTS(SELECT 1 FROM argos_job_retirements r WHERE r.job_id=t.job_id)
    AND NOT EXISTS(SELECT 1 FROM argos_task_dependencies d JOIN argos_tasks p ON p.id=d.depends_on
                   WHERE d.task_id=t.id AND p.state<>'completed')
    AND NOT (t.pool='intel_brief' AND EXISTS(SELECT 1 FROM argos_tasks o
             WHERE o.job_id=t.job_id AND o.id<>t.id AND o.state='running'
               AND (EXISTS(SELECT 1 FROM argos_task_dependencies d WHERE d.task_id=t.id)
                    OR EXISTS(SELECT 1 FROM argos_task_dependencies d WHERE d.task_id=o.id))))
    AND (t.pool<>'intel_brief' OR t.input_hash<>'')
)";

/// Candidate selector for [`claim_next_in`]: the top-ranked queued row that
/// passes the same `CLAIM_ELIGIBLE` predicate as the claim `UPDATE`, so the
/// head can never be a row the UPDATE would reject (which would starve every
/// eligible task behind it). `pool_filter` is an `AND pool IN (...)` clause or
/// empty. Binds `?3` as "now".
fn claim_selector_sql(pool_filter: &str) -> String {
    format!(
        "(SELECT t.id FROM argos_tasks t
            WHERE t.state IN ('queued','retry_scheduled')
              AND (t.next_eligible_at='' OR julianday(t.next_eligible_at)<=julianday(?3))
              {pool_filter}
              AND {CLAIM_ELIGIBLE}
            ORDER BY t.priority ASC, t.created_at ASC LIMIT 1)"
    )
}

/// Aggregated task timing for one job: started, active, queue, retry wait,
/// attempts, latest error category, latest error message.
type JobTimingRow = (
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<String>,
    Option<String>,
);

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
    Pending {
        reason: String,
    },
    /// Semantic indexing is intentionally disabled (`ARGOS_EMBED=0`) or unavailable for this store.
    Disabled {
        reason: String,
    },
    RetryableFailure {
        message: String,
    },
    PermanentFailure {
        message: String,
    },
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
        serde_json::to_string(self)
            .unwrap_or_else(|_| format!("{{\"status\":\"{}\"}}", self.label()))
    }
}

/// Long-lived service job that owns local index work without an explicit parent.
pub const INDEX_SERVICE_JOB: &str = "svc-local-index";

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

/// Blocked tasks for disabled embeddings share one reason key so enablement can resume them.
pub const EMBEDDINGS_DISABLED: &str = "embeddings_disabled";

fn reason_key(reason: &str) -> &str {
    if reason.contains("ARGOS_EMBED") || reason.contains("disabled") {
        EMBEDDINGS_DISABLED
    } else {
        reason
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn claim_eligibility_const_names_the_brief_pool() {
        assert!(super::CLAIM_ELIGIBLE.contains(&format!("'{}'", super::POOL_INTEL_BRIEF)));
    }

    use super::*;

    #[test]
    fn interactive_waiters_take_freed_slots_before_background_work() {
        let mut gate = ProviderAdmission::default();
        assert!(gate.try_acquire_class("acct", 1, false));
        // Slot busy: an interactive caller waits.
        *gate.interactive_waiting.entry("acct".into()).or_default() += 1;
        assert!(!gate.try_acquire_class("acct", 1, true));
        gate.release("acct");
        // Freed slot: background is held back, interactive gets it.
        assert!(!gate.try_acquire_class("acct", 1, false));
        assert!(gate.try_acquire_class("acct", 1, true));
        *gate.interactive_waiting.get_mut("acct").unwrap() -= 1;
        gate.release("acct");
        assert!(
            gate.try_acquire_class("acct", 1, false),
            "no waiter: background proceeds"
        );
        // Other accounts are unaffected.
        *gate.interactive_waiting.entry("acct".into()).or_default() += 1;
        assert!(gate.try_acquire_class("other", 1, false));
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
    fn summarization_gets_two_attempts_other_llm_four() {
        assert_eq!(OperationKind::Summarization.attempt_cap(), 2);
        assert_eq!(OperationKind::OtherLlm.attempt_cap(), 4);
        assert!(can_retry(
            OperationKind::Summarization,
            1,
            ErrorCategory::RateLimit
        ));
        assert!(!can_retry(
            OperationKind::Summarization,
            4,
            ErrorCategory::RateLimit
        ));
        assert!(can_retry(
            OperationKind::OtherLlm,
            2,
            ErrorCategory::TemporaryNetwork
        ));
        assert!(!can_retry(
            OperationKind::OtherLlm,
            4,
            ErrorCategory::TemporaryNetwork
        ));
        assert!(!can_retry(
            OperationKind::OtherLlm,
            1,
            ErrorCategory::AuthOrQuota
        ));
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
    fn atlas_native_metadata_uses_native_attempt_policies() {
        for operation in ["atlas_entities", "atlas_related"] {
            assert_eq!(pool_for_operation(operation), POOL_ATLAS);
            assert_eq!(operation_kind(operation), OperationKind::LocalIndex);
        }
        assert_eq!(
            operation_kind("atlas_publishers"),
            OperationKind::NetworkCollect
        );
        assert_eq!(
            operation_kind("atlas_classification"),
            OperationKind::OtherLlm
        );
    }
}
