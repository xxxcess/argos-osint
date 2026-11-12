//! Safe registered-workstation log pages; operational storage stays server-only.

use serde::Deserialize;
use serde::Serialize;

/// Scope and bounded filters, never a caller-selected workstation or job cohort.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct LogQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descendants: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_rank: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait_ms: Option<u64>,
}

fn bounded(value: &str, limit: usize) -> bool {
    value.len() <= limit && !value.chars().any(char::is_control)
}

pub(crate) fn cursor_valid(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

impl LogQuery {
    /// Validate transport bounds. The server independently resolves authority,
    /// parses timestamps and checks membership before querying any log owner.
    pub fn validate(&self) -> Result<(), LogInputError> {
        if self.limit.is_some_and(|value| !(1..=100).contains(&value))
            || self.min_rank.is_some_and(|rank| !(0..=3).contains(&rank))
            || self.wait_ms.is_some_and(|value| value > 4000)
        {
            return Err(LogInputError);
        }
        for value in [&self.job_id, &self.workspace_id].into_iter().flatten() {
            if value.is_empty() || !bounded(value, 512) {
                return Err(LogInputError);
            }
        }
        for (value, limit) in [(&self.app, 64), (&self.text, 256), (&self.since, 64)] {
            if value.as_ref().is_some_and(|value| !bounded(value, limit)) {
                return Err(LogInputError);
            }
        }
        if self
            .cursor
            .as_ref()
            .is_some_and(|value| !cursor_valid(value))
        {
            return Err(LogInputError);
        }
        Ok(())
    }
}

/// Summary fields only. Raw sequence numbers, details and provider payloads
/// cannot be deserialized into an exported public row.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct LogEntry {
    pub id: String,
    pub ts: String,
    pub severity: String,
    pub app: String,
    pub event_type: String,
    pub message: String,
    pub job_id: String,
    pub task_id: String,
    pub attempt_id: String,
    pub run_id: String,
}

impl LogEntry {
    /// Check the same public summary bounds used by the operational owner.
    pub fn validate(&self) -> Result<(), LogInputError> {
        for (value, limit) in [
            (&self.id, 512),
            (&self.ts, 64),
            (&self.severity, 16),
            (&self.app, 64),
            (&self.event_type, 128),
            (&self.job_id, 512),
            (&self.task_id, 512),
            (&self.attempt_id, 512),
            (&self.run_id, 512),
        ] {
            if !bounded(value, limit) {
                return Err(LogInputError);
            }
        }
        if self.id.is_empty() || self.job_id.is_empty() || self.message.chars().count() > 1000 {
            return Err(LogInputError);
        }
        Ok(())
    }
}

/// Counts cover the same authorized cohort and filters as the returned page.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct LogCounts {
    pub total: u64,
    pub error: u64,
    pub warn: u64,
    pub info: u64,
    pub debug: u64,
}

/// Public cursors are opaque, short-lived and bound to proof, scope and filters.
/// Missing counts and incomplete app facets are distinct from zero results.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct LogPage {
    pub rows: Vec<LogEntry>,
    pub counts: Option<LogCounts>,
    pub apps: Vec<String>,
    pub apps_complete: bool,
    pub next_cursor: Option<String>,
    pub follow_cursor: Option<String>,
    pub available: bool,
    pub private_logs_only: bool,
    /// True when this cohort has no mixed-origin private-log exclusions.
    /// This is completeness metadata, never permission to reveal shared rows.
    pub shared_private_logs_available: bool,
    pub reason_code: Option<String>,
    pub narrowing_hint: Option<String>,
    pub has_more: bool,
}

impl LogPage {
    /// Reject inconsistent or unbounded wire summaries before displaying them.
    pub fn validate(&self) -> Result<(), LogInputError> {
        if !self.private_logs_only
            || self.rows.len() > 100
            || self.apps.len() > 32
            || self.apps.iter().any(|app| !bounded(app, 64))
            || [&self.next_cursor, &self.follow_cursor]
                .into_iter()
                .flatten()
                .any(|value| !cursor_valid(value))
            || self
                .reason_code
                .as_ref()
                .is_some_and(|value| !bounded(value, 128))
            || self
                .narrowing_hint
                .as_ref()
                .is_some_and(|value| !bounded(value, 1024))
        {
            return Err(LogInputError);
        }
        for row in &self.rows {
            row.validate()?;
        }
        if !self.available
            && (!self.rows.is_empty()
                || self.counts.is_some()
                || !self.apps.is_empty()
                || self.next_cursor.is_some()
                || self.follow_cursor.is_some()
                || self.has_more)
        {
            return Err(LogInputError);
        }
        if let Some(counts) = &self.counts {
            let sum = counts
                .error
                .checked_add(counts.warn)
                .and_then(|n| n.checked_add(counts.info))
                .and_then(|n| n.checked_add(counts.debug));
            if sum != Some(counts.total) {
                return Err(LogInputError);
            }
        }
        Ok(())
    }
}

/// Static diagnostics contain neither filters nor private log contents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LogInputError;
impl std::fmt::Display for LogInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Invalid bounded workstation log data")
    }
}
impl std::error::Error for LogInputError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_cannot_supply_authority_or_an_operational_cohort() {
        for field in [
            "job_ids",
            "scope_id",
            "workstation_id",
            "upper_seq",
            "details",
        ] {
            assert!(
                serde_json::from_value::<LogQuery>(serde_json::json!({(field): "untrusted"}))
                    .is_err()
            );
        }
        let query = LogQuery {
            cursor: Some("not-an-owner-cursor".into()),
            ..Default::default()
        };
        assert_eq!(query.validate(), Err(LogInputError));
    }

    #[test]
    fn public_rows_reject_raw_details_and_pages_preserve_unknown_counts() {
        let raw = serde_json::json!({"id":"event","ts":"2026-01-01T00:00:00Z","severity":"info","app":"tools","event_type":"phase","message":"safe summary","job_id":"job","task_id":"","attempt_id":"","run_id":""});
        let row: LogEntry = serde_json::from_value(raw.clone()).expect("public summary");
        row.validate().expect("bounded summary");
        let mut private = raw;
        private["details"] = serde_json::json!({"secret":"synthetic-never-exported"});
        assert!(serde_json::from_value::<LogEntry>(private).is_err());
        let page: LogPage = serde_json::from_value(serde_json::json!({"rows":[],"counts":null,"apps":[],"apps_complete":false,"next_cursor":null,"follow_cursor":null,"available":false,"private_logs_only":true,"shared_private_logs_available":false,"reason_code":"cohort_over_budget","narrowing_hint":"Select a child or a job","has_more":false})).expect("explicit unavailable");
        page.validate().expect("unknown is not zero");
        assert!(page.counts.is_none() && !page.apps_complete);
        let clean: LogPage = serde_json::from_value(serde_json::json!({"rows":[],"counts":{"total":0,"error":0,"warn":0,"info":0,"debug":0},"apps":[],"apps_complete":true,"next_cursor":null,"follow_cursor":null,"available":true,"private_logs_only":true,"shared_private_logs_available":true,"reason_code":null,"narrowing_hint":null,"has_more":false})).expect("clean authorized cohort");
        clean.validate().expect("complete private cohort is valid");
    }
}
