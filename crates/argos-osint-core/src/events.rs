//! Durable structured event sink (`argos_events`).
//!
//! Every background worker writes through [`record_event`], even when its source
//! app is not open. Messages and details are redacted before persistence. The
//! TUI keeps only a bounded page cache; this table is the historical source of
//! truth, retained for [`DEFAULT_RETENTION_HOURS`]. Pruning events never deletes
//! jobs, task results or memories.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Default event-detail retention (matches the previous in-session log pruning).
pub const DEFAULT_RETENTION_HOURS: i64 = 24;
/// Bound on persisted detail text.
pub const MAX_DETAIL_CHARS: usize = 16_000;
pub(crate) const MAX_MESSAGE_CHARS: usize = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Debug,
    Info,
    Warn,
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw {
            "debug" => Self::Debug,
            "warn" | "warning" => Self::Warn,
            "error" => Self::Error,
            _ => Self::Info,
        }
    }
}

/// An event to persist. Empty strings mean "no reference".
#[derive(Clone, Debug, Default)]
pub struct NewEvent {
    pub severity: Option<Severity>,
    pub app: String,
    pub event_type: String,
    pub message: String,
    pub details: String,
    pub job_id: String,
    pub task_id: String,
    pub attempt_id: String,
    pub run_id: String,
    pub resource_ref: String,
    pub correlation_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRow {
    pub id: String,
    pub seq: i64,
    pub ts: String,
    pub severity: String,
    pub app: String,
    pub event_type: String,
    pub message: String,
    pub details: String,
    pub job_id: String,
    pub task_id: String,
    pub attempt_id: String,
    pub run_id: String,
    pub resource_ref: String,
    pub correlation_id: String,
}

static SEQ: AtomicU64 = AtomicU64::new(1);

pub(crate) fn new_event_id(ts_ms: i64) -> String {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("evt-{ts_ms}-{}-{n}", std::process::id())
}

fn patterns() -> &'static [(Regex, &'static str)] {
    static P: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    P.get_or_init(|| {
        vec![
            // Authorization / API-key headers.
            (
                Regex::new(r"(?i)\b(authorization|proxy-authorization)\s*[:=]\s*(bearer|basic|token)?\s*[A-Za-z0-9._~+/=\-]+").unwrap(),
                "$1: [redacted]",
            ),
            (
                Regex::new(r"(?i)\b(x-api-key|api-key|x-goog-api-key|x-subscription-token|x-access-token)\s*[:=]\s*[^\s,;]+").unwrap(),
                "$1: [redacted]",
            ),
            (Regex::new(r"(?i)\bbearer\s+[A-Za-z0-9._~+/=\-]{8,}").unwrap(), "Bearer [redacted]"),
            // Sensitive query parameters in URLs.
            (
                Regex::new(r"(?i)([?&](?:api_?key|apikey|key|token|access_token|auth|secret|password|client_secret|sig|signature)=)[^&\s#]+").unwrap(),
                "${1}[redacted]",
            ),
            // JSON-ish "api_key": "..." pairs.
            (
                Regex::new(r#"(?i)("(?:api_?key|apikey|token|access_token|refresh_token|secret|password|client_secret|authorization)"\s*:\s*")[^"]*(")"#).unwrap(),
                "${1}[redacted]${2}",
            ),
            // Well-known key prefixes.
            (Regex::new(r"\b(sk-[A-Za-z0-9_\-]{12,}|xai-[A-Za-z0-9_\-]{12,}|sk-ant-[A-Za-z0-9_\-]{12,}|gh[pousr]_[A-Za-z0-9]{20,}|AIza[0-9A-Za-z_\-]{20,})").unwrap(), "[redacted]"),
            // Credentials embedded in URLs.
            (Regex::new(r"(?i)(https?://)[^/\s:@]+:[^/\s@]+@").unwrap(), "${1}[redacted]@"),
        ]
    })
}

/// Redact secrets (API keys, authorization headers, sensitive query parameters,
/// credentialed URLs) from text before it is persisted or displayed.
pub fn redact(text: &str) -> String {
    let mut out = text.to_string();
    for (re, rep) in patterns() {
        out = re.replace_all(&out, *rep).into_owned();
    }
    if let Ok(auth) = crate::secrets::AuthFile::load() {
        out = auth.redact(&out);
    }
    out
}

pub(crate) fn bound(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max).collect();
    s.push_str(" …[truncated]");
    s
}

/// Filters for [`list_events`]. Empty fields do not filter.
#[derive(Clone, Debug, Default)]
pub struct EventFilter {
    pub min_severity: Option<Severity>,
    pub app: String,
    /// Matches the job and its descendant jobs (via `argos_jobs.parent_id`).
    pub job_id: String,
    pub text: String,
    pub since: String,
    pub before_seq: Option<i64>,
}

fn severity_rank_sql() -> &'static str {
    "CASE severity WHEN 'debug' THEN 0 WHEN 'info' THEN 1 WHEN 'warn' THEN 2 WHEN 'error' THEN 3 ELSE 1 END"
}

pub(crate) fn severity_rank(sev: Severity) -> i64 {
    match sev {
        Severity::Debug => 0,
        Severity::Info => 1,
        Severity::Warn => 2,
        Severity::Error => 3,
    }
}

#[cfg(test)]
mod tests {}
