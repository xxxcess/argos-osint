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
use rusqlite::OptionalExtension;
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
    fn rollup_key(&self) -> String {
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

/// Opens a short-lived connection for a write. Mirrors how `model_exec` records
/// progress with a raw `db_path` instead of a pooled `Store`.
fn write_connection(db_path: &Path) -> Result<rusqlite::Connection> {
    let conn =
        rusqlite::Connection::open(db_path).map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    Ok(conn)
}

fn rusqlite_result<T>(value: Result<T, rusqlite::Error>) -> anyhow::Result<T> {
    value.map_err(|err| anyhow::anyhow!(err.to_string()))
}

/// Persists one event and refreshes its rollups.
pub fn record(db_path: &Path, event: &TelemetryEvent) -> anyhow::Result<()> {
    let conn = write_connection(db_path)?;
    insert(&conn, event)
}

/// Writes an event on an existing connection.
pub fn insert(conn: &rusqlite::Connection, event: &TelemetryEvent) -> anyhow::Result<()> {
    let id = if event.id.is_empty() {
        next_id()
    } else {
        event.id.clone()
    };
    let occurred = &event.occurred_at;
    let day = occurred.chars().take(10).collect::<String>();
    let hour = occurred.chars().take(13).collect::<String>();
    let already = conn
        .query_row(
            "SELECT 1 FROM telemetry_events WHERE id = ?1",
            [&id],
            |_| Ok(true),
        )
        .optional()
        .map_err(|err| anyhow::anyhow!(err.to_string()))?
        .unwrap_or(false);
    rusqlite_result(conn.execute(
        "INSERT OR REPLACE INTO telemetry_events
         (id, event_type, occurred_at, day, hour, app, trigger, tool_id, category, provider, role,
          model, mode, engine, outcome, reason, job_id, run_id, thread_id, turn_id, article_id,
          origin, call_id, canonical_ref, metric_ms, count, payload_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
                 ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27)",
        rusqlite::params![
            id,
            event.event_type,
            occurred,
            day,
            hour,
            event.app,
            event.trigger,
            event.tool_id,
            event.category,
            event.provider,
            event.role,
            event.model,
            event.mode,
            event.engine,
            event.outcome,
            event.reason,
            event.job_id,
            event.run_id,
            event.thread_id,
            event.turn_id,
            event.article_id,
            event.origin,
            event.call_id,
            event.canonical_ref,
            event.metric_ms,
            event.count,
            event.payload_json,
        ],
    ))?;
    if already {
        return Ok(());
    }
    rollup(conn, event, &hour, &day)
}

/// Folds an event into the hourly and daily rollups. Duration histograms are
/// merged by bin counts, never by averaging averages.
fn rollup(
    conn: &rusqlite::Connection,
    event: &TelemetryEvent,
    hour: &str,
    day: &str,
) -> Result<()> {
    let metric = event.metric_ms.unwrap_or(0).max(0) as u64;
    let bins = event_bins(metric);
    let bins_json = serde_json::to_string(&bins).unwrap_or_else(|_| "[]".into());
    for (bucket, table) in [(hour, "telemetry_hourly"), (day, "telemetry_daily")] {
        let sql = format!(
            "INSERT INTO {table} (bucket, event_type, dim_key, events, total_ms, max_ms, bins_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(bucket, event_type, dim_key) DO UPDATE SET
               events = events + excluded.events,
               total_ms = total_ms + excluded.total_ms,
               max_ms = MAX(max_ms, excluded.max_ms),
               bins_json = ?7"
        );
        rusqlite_result(conn.execute(
            &sql,
            rusqlite::params![
                bucket,
                event.event_type,
                event.rollup_key(),
                event.count.max(0),
                metric,
                metric,
                bins_json,
            ],
        ))?;
    }
    Ok(())
}

fn event_bins(metric_ms: u64) -> Vec<i64> {
    let mut bins = vec![0i64; DURATION_BINS_MS.len() + 1];
    if metric_ms == 0 {
        return bins;
    }
    bins[bin_index(metric_ms)] += 1;
    bins
}

static ID_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_id() -> String {
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
pub fn prune(db_path: &Path, now: chrono::DateTime<chrono::Utc>) -> Result<()> {
    let conn = write_connection(db_path)?;
    if !should_prune(&conn, now)? {
        return Ok(());
    }
    let raw_cutoff = (now - chrono::Duration::days(RAW_RETENTION_DAYS)).to_rfc3339();
    let rollup_cutoff = now - chrono::Duration::days(ROLLUP_RETENTION_DAYS);
    let _ = rusqlite_result(conn.execute(
        "DELETE FROM telemetry_events WHERE occurred_at < ?1",
        rusqlite::params![raw_cutoff],
    ))?;
    let hourly_cutoff = raw_cutoff.chars().take(13).collect::<String>();
    let _ = rusqlite_result(conn.execute(
        "DELETE FROM telemetry_hourly WHERE bucket < ?1",
        rusqlite::params![hourly_cutoff],
    ))?;
    let _ = rusqlite_result(conn.execute(
        "DELETE FROM telemetry_daily WHERE bucket < ?1",
        rusqlite::params![rollup_cutoff.format("%Y-%m-%d").to_string()],
    ))?;
    set_meta(&conn, "raw_pruned_at", &now.to_rfc3339())?;
    Ok(())
}

fn should_prune(
    conn: &rusqlite::Connection,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<bool> {
    let last = get_meta(conn, "raw_pruned_at")?;
    match last {
        None => Ok(true),
        Some(value) => {
            let parsed = chrono::DateTime::parse_from_rfc3339(&value)
                .map(|ts| ts.with_timezone(&chrono::Utc))
                .unwrap_or(now);
            Ok((now - parsed).num_seconds() >= PRUNE_INTERVAL_SECS)
        }
    }
}

pub fn get_meta(conn: &rusqlite::Connection, key: &str) -> anyhow::Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM telemetry_meta WHERE key = ?1",
        rusqlite::params![key],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(|err| anyhow::anyhow!(err.to_string()))
}

pub fn set_meta(conn: &rusqlite::Connection, key: &str, value: &str) -> anyhow::Result<()> {
    rusqlite_result(conn.execute(
        "INSERT INTO telemetry_meta (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        rusqlite::params![key, value, chrono::Utc::now().to_rfc3339()],
    ))?;
    Ok(())
}

/// Marks the first moment this installation recorded telemetry.
pub fn ensure_observed_since(conn: &rusqlite::Connection) -> anyhow::Result<String> {
    let existing = get_meta(conn, "observed_since")?;
    if let Some(value) = existing {
        return Ok(value);
    }
    let now = chrono::Utc::now().to_rfc3339();
    set_meta(conn, "observed_since", &now)?;
    Ok(now)
}

/// Sample counts read straight from raw events, used for the coverage and N/A
/// markers the dashboard renders.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Coverage {
    pub raw_events: i64,
    pub rollup_events: i64,
    pub observed_since: String,
}

pub fn coverage(conn: &rusqlite::Connection) -> anyhow::Result<Coverage> {
    let mut stmt = rusqlite_result(conn.prepare("SELECT COUNT(*) FROM telemetry_events"))?;
    let raw_events =
        rusqlite_result(stmt.query_row([], |row| row.get::<_, i64>(0))).unwrap_or_default();
    let mut stmt =
        rusqlite_result(conn.prepare("SELECT COALESCE(SUM(events), 0) FROM telemetry_hourly"))?;
    let rollup_events =
        rusqlite_result(stmt.query_row([], |row| row.get::<_, i64>(0))).unwrap_or_default();
    let observed_since = get_meta(conn, "observed_since")?.unwrap_or_default();
    Ok(Coverage {
        raw_events,
        rollup_events,
        observed_since,
    })
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

    fn db() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("argos.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(crate::store::SCHEMA_TELEMETRY_SQL)
            .unwrap();
        (dir, path)
    }

    #[test]
    fn events_persist_and_roll_up_once() {
        let (_dir, path) = db();
        let event = TelemetryEvent::new(EventKind::ToolInvocation)
            .tool("firecrawl_google_search")
            .outcome("verified_zero")
            .trigger(Trigger::ManualTool)
            .duration_ms(Some(420));
        record(&path, &event).unwrap();
        record(&path, &event.clone().with_id("fixed")).unwrap();

        let conn = rusqlite::Connection::open(&path).unwrap();
        let mut stmt = conn
            .prepare("SELECT COUNT(*) FROM telemetry_events")
            .unwrap();
        assert_eq!(stmt.query_row([], |r| r.get::<_, i64>(0)).unwrap(), 2);
        let mut stmt = conn
            .prepare("SELECT SUM(events) FROM telemetry_hourly")
            .unwrap();
        assert_eq!(stmt.query_row([], |r| r.get::<_, i64>(0)).unwrap(), 2);
    }

    #[test]
    fn id_is_stable_so_replay_never_double_counts() {
        let (_dir, path) = db();
        let event = TelemetryEvent::new(EventKind::ReconRun)
            .with_id("recon-run-1234")
            .outcome("completed_with_evidence");
        for _ in 0..3 {
            record(&path, &event).unwrap();
        }
        let conn = rusqlite::Connection::open(&path).unwrap();
        let mut stmt = conn
            .prepare("SELECT COUNT(*) FROM telemetry_events WHERE id = 'recon-run-1234'")
            .unwrap();
        assert_eq!(stmt.query_row([], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }

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
    fn coverage_reports_observed_since_and_counts() {
        let (_dir, path) = db();
        let conn = rusqlite::Connection::open(&path).unwrap();
        let since = ensure_observed_since(&conn).unwrap();
        assert!(!since.is_empty());
        record(
            &path,
            &TelemetryEvent::new(EventKind::AtlasCycle).outcome("completed"),
        )
        .unwrap();
        let conn = rusqlite::Connection::open(&path).unwrap();
        let report = coverage(&conn).unwrap();
        assert_eq!(report.raw_events, 1);
        assert_eq!(report.observed_since, since);
    }

    #[test]
    fn measured_elapsed_is_never_negative() {
        let clock = Measured::start();
        assert!(clock.elapsed_ms() >= 0);
    }

    #[test]
    fn logical_attempt_and_cache_rows_stay_distinct_under_replay() {
        let (_dir, path) = db();
        let logical = TelemetryEvent::new(EventKind::ToolInvocation)
            .with_id("invoke-call-1")
            .tool("firecrawl_google_search")
            .outcome(ToolOutcome::CompletedNonEmpty.as_str())
            .trigger(Trigger::ReconPrompt)
            .mode("remote");
        let wire = TelemetryEvent::new(EventKind::ToolWireRequest)
            .with_id("wire-call-1")
            .tool("firecrawl_google_search")
            .canonical("invoke-call-1");
        let cached = TelemetryEvent::new(EventKind::ToolInvocation)
            .with_id("invoke-call-2")
            .tool("firecrawl_google_search")
            .outcome(ToolOutcome::CompletedNonEmpty.as_str())
            .mode("cache");
        for _ in 0..2 {
            record(&path, &logical).unwrap();
            record(&path, &wire).unwrap();
            record(&path, &cached).unwrap();
        }
        let conn = rusqlite::Connection::open(&path).unwrap();
        let count = |kind: &str| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM telemetry_events WHERE event_type = ?1",
                [kind],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(count("tool_invocation"), 2);
        assert_eq!(count("tool_wire_request"), 1);
        let rolled: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(events), 0) FROM telemetry_hourly WHERE event_type = 'tool_invocation'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rolled, 2);
    }
}
