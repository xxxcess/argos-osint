//! Structured activity telemetry for the Profile dashboard.
//!
//! Rules enforced by this module:
//!   * lifecycle writes, never log scraping or model-generated statistics;
//!   * exactly one authoritative row per logical fact, so replaying a resume
//!     never double counts;
//!   * bounded dimensions only. Prompts, article bodies, model outputs and
//!     credentials are rejected before they reach storage;
//!   * raw events live 90 days, hourly/daily rollups 365 days, and every
//!     aggregate is derived here rather than authored by a writer.

use std::path::Path;
use std::time::Instant;

use anyhow::Result;
use serde_json::Value;

#[cfg(test)]
use serde_json::json;

/// Raw operational events are retained for 90 days by default.
pub const RAW_RETENTION_DAYS: i64 = 90;
/// Hourly and daily rollups are retained for 365 days by default.
pub const ROLLUP_RETENTION_DAYS: i64 = 365;
/// Retention sweeps run at most this often.
pub const PRUNE_INTERVAL_SECS: i64 = 15 * 60;
/// Maximum payload characters kept per event. Prevents runaway dimension growth.
pub const MAX_PAYLOAD_CHARS: usize = 4_000;
/// Duration bin edges shared by every mergeable histogram.
pub const DURATION_BINS_MS: [u64; 15] = [
    0, 5, 10, 25, 50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000, 30_000, 60_000, 300_000,
];

/// Returns the bin index for `ms`; the last bin is unbounded.
pub fn bin_index(ms: u64) -> usize {
    DURATION_BINS_MS
        .iter()
        .rposition(|edge| *edge <= ms)
        .unwrap_or(DURATION_BINS_MS.len() - 1)
}

/// Fixed label for a bin index, used when a histogram is rendered.
pub fn bin_label(index: usize) -> &'static str {
    const LABELS: [&str; 16] = [
        "<5ms",
        "5-10ms",
        "10-25ms",
        "25-50ms",
        "50-100ms",
        "100-250ms",
        "250-500ms",
        "0.5-1s",
        "1-2.5s",
        "2.5-5s",
        "5-10s",
        "10-30s",
        "30-60s",
        "1-5m",
        "5-10m",
        ">5m",
    ];
    LABELS.get(index).copied().unwrap_or(">5m")
}

/// Bounded vocabulary of activity event types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventKind {
    /// Terminal state of one logical model operation.
    ModelOperation,
    /// One wire attempt of a model operation.
    ModelAttempt,
    /// One logical tool invocation (cache hits included).
    ToolInvocation,
    /// One remote tool request or poll.
    ToolWireRequest,
    /// One named search-engine query with its typed SERP outcome.
    ToolEngineQuery,
    /// One recon stage completion with its measured duration.
    ReconStage,
    /// Terminal state of one recon run.
    ReconRun,
    /// One Brain recall query with candidate and accepted hit counts.
    RecallQuery,
    /// One directive's terminal assessment.
    DirectiveAssessed,
    /// Terminal state of one Atlas cycle.
    AtlasCycle,
    /// One origin observation inside a cycle snapshot.
    AtlasOriginSnapshot,
    /// One candidate article occurrence with its collection disposition.
    AtlasCandidate,
    /// One Atlas stage transition.
    AtlasStage,
    /// First time a canonical article was seen.
    IntelArticle,
    /// Terminal revision of one Intel report per mode.
    IntelReport,
    /// Per-article Brief rating observation.
    IntelBriefRating,
    /// One accepted evidence item and its provenance.
    EvidenceItem,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ModelOperation => "model_operation",
            Self::ModelAttempt => "model_attempt",
            Self::ToolInvocation => "tool_invocation",
            Self::ToolWireRequest => "tool_wire_request",
            Self::ToolEngineQuery => "tool_engine_query",
            Self::ReconStage => "recon_stage",
            Self::ReconRun => "recon_run",
            Self::RecallQuery => "recall_query",
            Self::DirectiveAssessed => "directive_assessed",
            Self::AtlasCycle => "atlas_cycle",
            Self::AtlasOriginSnapshot => "atlas_origin_snapshot",
            Self::AtlasCandidate => "atlas_candidate",
            Self::AtlasStage => "atlas_stage",
            Self::IntelArticle => "intel_article",
            Self::IntelReport => "intel_report",
            Self::IntelBriefRating => "intel_brief_rating",
            Self::EvidenceItem => "evidence_item",
        }
    }

    /// Every event kind, so retention and rollup cover the full vocabulary.
    pub const ALL: [Self; 17] = [
        Self::ModelOperation,
        Self::ModelAttempt,
        Self::ToolInvocation,
        Self::ToolWireRequest,
        Self::ToolEngineQuery,
        Self::ReconStage,
        Self::ReconRun,
        Self::RecallQuery,
        Self::DirectiveAssessed,
        Self::AtlasCycle,
        Self::AtlasOriginSnapshot,
        Self::AtlasCandidate,
        Self::AtlasStage,
        Self::IntelArticle,
        Self::IntelReport,
        Self::IntelBriefRating,
        Self::EvidenceItem,
    ];
}

/// Terminal tool outcome classes. Exactly one applies to an invocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolOutcome {
    /// Completed with at least one usable item.
    CompletedNonEmpty,
    /// Completed and the engine/provider explicitly reported zero results.
    CompletedVerifiedZero,
    /// Completed with some but not all requested items.
    CompletedPartial,
    /// The tool failed (transport, auth, configuration, quota, invalid input).
    Failed,
    /// The engine or provider served a challenge, consent or block page.
    Blocked,
    /// The response could not be classified by the parser.
    ParserMismatch,
    /// The invocation was cancelled by the user or a deadline.
    Cancelled,
}

impl ToolOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CompletedNonEmpty => "completed_nonempty",
            Self::CompletedVerifiedZero => "verified_zero",
            Self::CompletedPartial => "partial",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
            Self::ParserMismatch => "parser_mismatch",
            Self::Cancelled => "cancelled",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "completed_nonempty" => Self::CompletedNonEmpty,
            "verified_zero" => Self::CompletedVerifiedZero,
            "partial" => Self::CompletedPartial,
            "failed" => Self::Failed,
            "blocked" => Self::Blocked,
            "parser_mismatch" => Self::ParserMismatch,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }

    /// True when the invocation reached a provider or engine successfully.
    pub fn reached_source(&self) -> bool {
        matches!(
            self,
            Self::CompletedNonEmpty | Self::CompletedVerifiedZero | Self::CompletedPartial
        )
    }
}

/// Bounded trigger attribution. Never inferred from prompt text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Trigger {
    ReconPrompt,
    IntelBrief,
    AtlasCycle,
    ManualTool,
    Scheduled,
    Repair,
    #[default]
    Unknown,
}

impl Trigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReconPrompt => "recon_prompt",
            Self::IntelBrief => "intel_brief",
            Self::AtlasCycle => "atlas_cycle",
            Self::ManualTool => "manual_tool",
            Self::Scheduled => "scheduled",
            Self::Repair => "repair",
            Self::Unknown => "unknown",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Self {
        match value {
            "recon_prompt" => Self::ReconPrompt,
            "intel_brief" => Self::IntelBrief,
            "atlas_cycle" => Self::AtlasCycle,
            "manual_tool" => Self::ManualTool,
            "scheduled" => Self::Scheduled,
            "repair" => Self::Repair,
            _ => Self::Unknown,
        }
    }
}

/// One telemetry row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TelemetryEvent {
    pub id: String,
    pub event_type: String,
    pub occurred_at: String,
    pub app: String,
    pub trigger: String,
    pub tool_id: String,
    pub category: String,
    pub provider: String,
    pub role: String,
    pub model: String,
    pub mode: String,
    pub engine: String,
    pub outcome: String,
    pub reason: String,
    pub job_id: String,
    pub run_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub article_id: String,
    pub origin: String,
    pub call_id: String,
    /// Link to an existing durable id (a schema-26 model attempt id, for
    /// instance). Empty when this event is the authoritative row.
    pub canonical_ref: String,
    pub metric_ms: Option<i64>,
    pub count: i64,
    pub payload_json: String,
}

impl TelemetryEvent {
    pub fn new(kind: EventKind) -> Self {
        Self {
            id: String::new(),
            event_type: kind.as_str().to_string(),
            occurred_at: chrono::Utc::now().to_rfc3339(),
            count: 1,
            ..Self::default()
        }
    }

    pub fn at(mut self, at: impl Into<String>) -> Self {
        self.occurred_at = at.into();
        self
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    pub fn app(mut self, value: impl Into<String>) -> Self {
        self.app = clamp(value.into());
        self
    }
    pub fn trigger(mut self, value: Trigger) -> Self {
        self.trigger = value.as_str().to_string();
        self
    }
    pub fn tool(mut self, value: impl Into<String>) -> Self {
        self.tool_id = clamp(value.into());
        self
    }
    pub fn category(mut self, value: impl Into<String>) -> Self {
        self.category = clamp(value.into());
        self
    }
    pub fn provider(mut self, value: impl Into<String>) -> Self {
        self.provider = clamp(value.into());
        self
    }
    pub fn role(mut self, value: impl Into<String>) -> Self {
        self.role = clamp(value.into());
        self
    }
    pub fn model(mut self, value: impl Into<String>) -> Self {
        self.model = clamp(value.into());
        self
    }
    pub fn mode(mut self, value: impl Into<String>) -> Self {
        self.mode = clamp(value.into());
        self
    }
    pub fn engine(mut self, value: impl Into<String>) -> Self {
        self.engine = clamp(value.into());
        self
    }
    pub fn outcome(mut self, value: impl Into<String>) -> Self {
        self.outcome = clamp(value.into());
        self
    }
    pub fn reason(mut self, value: impl Into<String>) -> Self {
        self.reason = clamp(value.into());
        self
    }
    pub fn job(mut self, value: impl Into<String>) -> Self {
        self.job_id = clamp(value.into());
        self
    }
    pub fn run(mut self, value: impl Into<String>) -> Self {
        self.run_id = clamp(value.into());
        self
    }
    pub fn thread(mut self, value: impl Into<String>) -> Self {
        self.thread_id = clamp(value.into());
        self
    }
    pub fn turn(mut self, value: impl Into<String>) -> Self {
        self.turn_id = clamp(value.into());
        self
    }
    pub fn article(mut self, value: impl Into<String>) -> Self {
        self.article_id = clamp(value.into());
        self
    }
    pub fn origin(mut self, value: impl Into<String>) -> Self {
        self.origin = clamp(value.into());
        self
    }
    pub fn call(mut self, value: impl Into<String>) -> Self {
        self.call_id = clamp(value.into());
        self
    }
    pub fn canonical(mut self, value: impl Into<String>) -> Self {
        self.canonical_ref = clamp(value.into());
        self
    }
    pub fn duration_ms(mut self, ms: Option<i64>) -> Self {
        self.metric_ms = ms.filter(|v| *v >= 0);
        self
    }
    pub fn count(mut self, value: i64) -> Self {
        self.count = value.max(0);
        self
    }
    pub fn payload(mut self, value: Value) -> Self {
        let mut redacted = value;
        crate::investigation::trace::redact_secrets(&mut redacted);
        self.payload_json = safe_payload(&redacted);
        self
    }

    /// The dimension subset used by the hourly/daily rollups. A single
    /// `dim_key` per (bucket, event_type) keeps counts mergeable.
    pub(crate) fn rollup_key(&self) -> String {
        let fields = [
            self.app.as_str(),
            self.trigger.as_str(),
            self.tool_id.as_str(),
            self.category.as_str(),
            self.provider.as_str(),
            self.role.as_str(),
            self.model.as_str(),
            self.mode.as_str(),
            self.engine.as_str(),
            self.outcome.as_str(),
            self.reason.as_str(),
        ];
        fields.join("\u{1f}")
    }
}

/// Truncates a dimension to a bounded length so one bad value cannot widen a row.
fn clamp(value: String) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= 128 {
        return trimmed.to_string();
    }
    trimmed.chars().take(128).collect()
}

/// Redacts and bounds the payload; never keeps prose, keys or model output.
fn safe_payload(value: &Value) -> String {
    let text = serde_json::to_string(value).unwrap_or_default();
    if text.chars().count() <= MAX_PAYLOAD_CHARS {
        return text;
    }
    text.chars().take(MAX_PAYLOAD_CHARS).collect()
}

/// True when `key` names something that must never be persisted.
pub fn is_forbidden_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    [
        "prompt",
        "body",
        "completion",
        "content",
        "message",
        "output",
        "text",
        "markdown",
        "raw",
        "api_key",
        "key",
        "token",
        "secret",
        "authorization",
        "password",
        "credential",
    ]
    .iter()
    .any(|bad| lower == *bad || lower.ends_with(bad))
}

/// Persists one event and refreshes its rollups.
///
/// Under a storage config (SurrealDB selected) the event goes to the
/// repository instead (`storage::telemetry::record`, started on the process
/// runtime without waiting) and SQLite is never opened: the two stores are
/// never both written.
pub fn record(_db_path: &Path, event: &TelemetryEvent) -> anyhow::Result<()> {
    let event = event.clone();
    crate::storage::runtime::spawn(async move {
        if let Ok(repo) = crate::storage::runtime::repository().await {
            let _ = crate::storage::telemetry::record(repo.db(), &event).await;
        }
    });
    Ok(())
}

pub(crate) fn event_bins(metric_ms: u64) -> Vec<i64> {
    let mut bins = vec![0i64; DURATION_BINS_MS.len() + 1];
    if metric_ms == 0 {
        return bins;
    }
    bins[bin_index(metric_ms)] += 1;
    bins
}

static ID_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub(crate) fn next_id() -> String {
    let n = ID_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!(
        "tel-{}-{n}",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    )
}

/// Bounded, monotonic clock for measured durations. Wall-clock jumps never
/// produce a negative duration.
pub struct Measured {
    start: Instant,
}

impl Measured {
    pub fn start() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    pub fn elapsed_ms(&self) -> i64 {
        self.start.elapsed().as_millis() as i64
    }
}

/// Deletes raw events past the raw retention boundary and rollups past the
/// rollup boundary. Rate limited; records when it last ran.
///
/// Under a storage config the prune runs on the repository
/// (`storage::telemetry::prune`, started without waiting) and SQLite is
/// never opened.
pub fn prune(_db_path: &Path, now: chrono::DateTime<chrono::Utc>) -> Result<()> {
    crate::storage::runtime::spawn(async move {
        if let Ok(repo) = crate::storage::runtime::repository().await {
            let _ = crate::storage::telemetry::prune(repo.db(), now).await;
        }
    });
    Ok(())
}

/// Sample counts read straight from raw events, used for the coverage and N/A
/// markers the dashboard renders.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Coverage {
    pub raw_events: i64,
    pub rollup_events: i64,
    pub observed_since: String,
}

/// Merges `bins` from a rollup row into one histogram.
pub fn merge_bins(bins: &mut [i64], incoming: &[i64]) {
    for (slot, value) in bins.iter_mut().zip(incoming.iter()) {
        *slot += *value;
    }
}

/// Reads a value from a telemetry payload without allowing prose.
pub fn payload_number(payload: &str, key: &str) -> Option<f64> {
    if is_forbidden_key(key) {
        return None;
    }
    let value: Value = serde_json::from_str(payload).ok()?;
    value.get(key).and_then(Value::as_f64)
}

/// Convenience constructor for a tool invocation's terminal row.
pub fn tool_invocation(
    tool_id: &str,
    outcome: ToolOutcome,
    trigger: Trigger,
    app: &str,
    duration_ms: Option<i64>,
) -> TelemetryEvent {
    TelemetryEvent::new(EventKind::ToolInvocation)
        .tool(tool_id)
        .outcome(outcome.as_str())
        .trigger(trigger)
        .app(app)
        .duration_ms(duration_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_shaped_keys_are_rejected_from_payloads() {
        assert!(is_forbidden_key("api_key"));
        assert!(is_forbidden_key("authorization"));
        assert!(is_forbidden_key("prompt"));
        assert!(is_forbidden_key("response_text"));
        assert!(!is_forbidden_key("duration_ms"));
        let mut payload = json!({"api_key": "sk-live-123", "duration_ms": 5});
        crate::investigation::trace::redact_secrets(&mut payload);
        let redacted = serde_json::to_string(&payload).unwrap();
        assert!(
            !redacted.contains("sk-live-123"),
            "secrets must not survive: {redacted}"
        );
    }

    #[test]
    fn duration_bins_are_bounded_and_mergeable() {
        let mut bins = vec![0i64; DURATION_BINS_MS.len() + 1];
        let mut other = vec![0i64; DURATION_BINS_MS.len() + 1];
        bins[bin_index(5)] = 1;
        other[bin_index(5)] = 2;
        other[bin_index(400)] = 3;
        merge_bins(&mut bins, &other);
        assert_eq!(bins[bin_index(5)], 3);
        assert_eq!(bins[bin_index(400)], 3);
    }

    #[test]
    fn payload_numbers_ignore_forbidden_keys() {
        assert_eq!(
            payload_number(r#"{"duration_ms": 42}"#, "duration_ms"),
            Some(42.0)
        );
        assert_eq!(payload_number(r#"{"prompt": "hi"}"#, "prompt"), None);
    }

    #[test]
    fn measured_elapsed_is_never_negative() {
        let clock = Measured::start();
        assert!(clock.elapsed_ms() >= 0);
    }
}
