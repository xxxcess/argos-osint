//! Durable structured event sink (`argos_events`).
//!
//! Every background worker writes through [`record_event`], even when its source
//! app is not open. Messages and details are redacted before persistence. The
//! TUI keeps only a bounded page cache; this table is the historical source of
//! truth, retained for [`DEFAULT_RETENTION_HOURS`]. Pruning events never deletes
//! jobs, task results or memories.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use anyhow::Result;
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Default event-detail retention (matches the previous in-session log pruning).
pub const DEFAULT_RETENTION_HOURS: i64 = 24;
/// Bound on persisted detail text.
pub const MAX_DETAIL_CHARS: usize = 16_000;
const MAX_MESSAGE_CHARS: usize = 1_000;

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

fn new_event_id(ts_ms: i64) -> String {
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

fn bound(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max).collect();
    s.push_str(" …[truncated]");
    s
}

/// Persist one redacted event, returning its stable id.
pub fn record_event(conn: &Connection, event: &NewEvent) -> Result<String> {
    let now = chrono::Utc::now();
    let id = new_event_id(now.timestamp_millis());
    let seq: i64 = conn
        .query_row("SELECT IFNULL(MAX(seq),0)+1 FROM argos_events", [], |r| {
            r.get(0)
        })
        .unwrap_or(1);
    conn.execute(
        "INSERT INTO argos_events(id,seq,ts,severity,app,event_type,message,details,job_id,task_id,attempt_id,run_id,resource_ref,correlation_id)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![
            id,
            seq,
            now.to_rfc3339(),
            event.severity.unwrap_or(Severity::Info).as_str(),
            event.app,
            event.event_type,
            bound(&redact(&event.message), MAX_MESSAGE_CHARS),
            bound(&redact(&event.details), MAX_DETAIL_CHARS),
            event.job_id,
            event.task_id,
            event.attempt_id,
            event.run_id,
            event.resource_ref,
            event.correlation_id,
        ],
    )?;
    Ok(id)
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

fn severity_rank(sev: Severity) -> i64 {
    match sev {
        Severity::Debug => 0,
        Severity::Info => 1,
        Severity::Warn => 2,
        Severity::Error => 3,
    }
}

/// Newest-first bounded page of events.
pub fn list_events(conn: &Connection, filter: &EventFilter, limit: usize) -> Result<Vec<EventRow>> {
    let mut sql = String::from(
        "SELECT id,seq,ts,severity,app,event_type,message,details,job_id,task_id,attempt_id,run_id,resource_ref,correlation_id
         FROM argos_events WHERE 1=1",
    );
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(sev) = filter.min_severity {
        sql.push_str(&format!(" AND {} >= ?", severity_rank_sql()));
        args.push(Box::new(severity_rank(sev)));
    }
    if !filter.app.is_empty() {
        sql.push_str(" AND app = ?");
        args.push(Box::new(filter.app.clone()));
    }
    if !filter.job_id.is_empty() {
        sql.push_str(
            " AND job_id IN (WITH RECURSIVE tree(id) AS (SELECT ? UNION SELECT j.id FROM argos_jobs j JOIN tree t ON j.parent_id = t.id) SELECT id FROM tree)",
        );
        args.push(Box::new(filter.job_id.clone()));
    }
    if !filter.text.is_empty() {
        sql.push_str(" AND (message LIKE ? OR details LIKE ?)");
        let needle = format!("%{}%", filter.text.replace('%', ""));
        args.push(Box::new(needle.clone()));
        args.push(Box::new(needle));
    }
    if !filter.since.is_empty() {
        sql.push_str(" AND ts >= ?");
        args.push(Box::new(filter.since.clone()));
    }
    if let Some(before) = filter.before_seq {
        sql.push_str(" AND seq < ?");
        args.push(Box::new(before));
    }
    sql.push_str(" ORDER BY seq DESC LIMIT ?");
    args.push(Box::new(limit.max(1) as i64));
    let mut stmt = conn.prepare(&sql)?;
    let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
    let rows = stmt
        .query_map(refs.as_slice(), event_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn event_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<EventRow> {
    Ok(EventRow {
        id: r.get(0)?,
        seq: r.get(1)?,
        ts: r.get(2)?,
        severity: r.get(3)?,
        app: r.get(4)?,
        event_type: r.get(5)?,
        message: r.get(6)?,
        details: r.get(7)?,
        job_id: r.get(8)?,
        task_id: r.get(9)?,
        attempt_id: r.get(10)?,
        run_id: r.get(11)?,
        resource_ref: r.get(12)?,
        correlation_id: r.get(13)?,
    })
}

/// Look up one event by id. `None` means it expired (or never existed): callers
/// must say so instead of showing an empty page.
pub fn get_event(conn: &Connection, id: &str) -> Result<Option<EventRow>> {
    Ok(conn
        .query_row(
            "SELECT id,seq,ts,severity,app,event_type,message,details,job_id,task_id,attempt_id,run_id,resource_ref,correlation_id
             FROM argos_events WHERE id=?1",
            [id],
            event_row,
        )
        .optional()?)
}

/// Delete events older than `hours`. Touches only `argos_events`.
pub fn prune_events(conn: &Connection, hours: i64) -> Result<usize> {
    let cutoff = (chrono::Utc::now() - chrono::Duration::hours(hours)).to_rfc3339();
    Ok(conn.execute("DELETE FROM argos_events WHERE ts < ?1", [cutoff])?)
}

/// Clear all events (Logs "Clear log"). Touches only `argos_events`.
pub fn clear_events(conn: &Connection) -> Result<usize> {
    Ok(conn.execute("DELETE FROM argos_events", [])?)
}

/// Severity counts for the Logs header.
pub fn severity_counts(conn: &Connection) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare("SELECT severity, COUNT(*) FROM argos_events GROUP BY severity")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::tasks::migrate_tables(&conn).unwrap();
        conn
    }

    #[test]
    fn secrets_are_redacted_before_persistence() {
        let conn = mem();
        let id = record_event(
            &conn,
            &NewEvent {
                severity: Some(Severity::Error),
                app: "brain".into(),
                event_type: "graph_explanation_failed".into(),
                message: "POST https://api.example.com/v1/chat?api_key=SECRETVALUE123&x=1 failed".into(),
                details: "Authorization: Bearer abcdefghijklmnop1234\ncaused by: {\"api_key\": \"sk-abcdefghijklmnopqrstuv\"}\nhttps://user:pass@host.example/path".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let row = get_event(&conn, &id).unwrap().unwrap();
        assert!(!row.message.contains("SECRETVALUE123"), "{}", row.message);
        assert!(row.message.contains("api_key=[redacted]"));
        assert!(
            !row.details.contains("abcdefghijklmnop1234"),
            "{}",
            row.details
        );
        assert!(
            !row.details.contains("sk-abcdefghijklmnopqrstuv"),
            "{}",
            row.details
        );
        assert!(!row.details.contains("user:pass"), "{}", row.details);
    }

    #[test]
    fn job_filter_includes_descendants_and_prune_keeps_jobs() {
        let conn = mem();
        let now = chrono::Utc::now().to_rfc3339();
        for (id, parent) in [("job-p", ""), ("job-c", "job-p"), ("job-x", "")] {
            crate::tasks::enqueue_job_with(
                &conn,
                &crate::tasks::NewJob {
                    id: id.into(),
                    kind: "t".into(),
                    owner_scope: String::new(),
                    input_revision: String::new(),
                    deadline_at: String::new(),
                },
                &crate::tasks::JobMeta {
                    parent_id: parent.into(),
                    ..Default::default()
                },
                &now,
            )
            .unwrap();
        }
        for job in ["job-p", "job-c", "job-x", ""] {
            record_event(
                &conn,
                &NewEvent {
                    message: format!("evt {job}"),
                    job_id: job.into(),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        let rows = list_events(
            &conn,
            &EventFilter {
                job_id: "job-p".into(),
                ..Default::default()
            },
            50,
        )
        .unwrap();
        let jobs: Vec<_> = rows.iter().map(|r| r.job_id.as_str()).collect();
        assert_eq!(rows.len(), 2, "{jobs:?}");
        assert!(jobs.contains(&"job-p") && jobs.contains(&"job-c"));
        assert_eq!(
            list_events(&conn, &EventFilter::default(), 50)
                .unwrap()
                .len(),
            4
        );
        conn.execute("UPDATE argos_events SET ts='2000-01-01T00:00:00+00:00'", [])
            .unwrap();
        assert_eq!(prune_events(&conn, DEFAULT_RETENTION_HOURS).unwrap(), 4);
        let jobs_left: i64 = conn
            .query_row("SELECT COUNT(*) FROM argos_jobs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(jobs_left, 3, "pruning events must not delete jobs");
    }
}
