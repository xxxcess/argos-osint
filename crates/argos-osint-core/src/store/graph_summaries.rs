//! Revision-aware graph summary cache and the latest explanation diagnostic.
//!
//! A saved summary is valid only for the exact inputs it was written from:
//! memory text revision, graph (evidence) revision, focus, provider, model and
//! prompt version. Anything else is shown as an earlier result, never as
//! current. The diagnostic row keeps the most recent execution's outcome so
//! Brain can explain a failure after a restart and enforce the retry cooldown.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::Store;

/// Inputs a graph summary depends on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryKey {
    pub memory_revision: String,
    pub graph_revision: String,
    pub focus: String,
    pub provider: String,
    pub model: String,
    pub prompt_version: String,
}

impl SummaryKey {
    pub fn digest(&self) -> String {
        crate::evidence::content_hash(
            &[
                self.memory_revision.as_str(),
                self.graph_revision.as_str(),
                self.focus.as_str(),
                self.provider.as_str(),
                self.model.as_str(),
                self.prompt_version.as_str(),
            ]
            .join("\u{1f}"),
        )
    }
}

/// A stored summary and the inputs it was written from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphSummaryEntry {
    pub summary: String,
    pub focus: String,
    pub cache_key: String,
    pub model: String,
    pub created_at: String,
}

/// Result of a keyed save.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    Saved,
    /// The memory was deleted; nothing is published.
    MemoryGone,
    /// The memory text changed since the request started; nothing is published.
    MemoryChanged,
}

/// Latest graph explanation execution for one memory.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ExplanationRecord {
    pub memory_id: String,
    pub job_id: String,
    pub request_id: String,
    pub cache_key: String,
    /// running · completed · failed · superseded
    pub state: String,
    pub category: String,
    pub reason: String,
    pub guidance: String,
    pub needs_config: bool,
    pub retryable: bool,
    pub attempts: i64,
    pub event_id: String,
    /// Sanitized failure + attempts, as JSON.
    pub diagnostic_json: String,
    pub updated_at: String,
}

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    let columns: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(memory_graph_summaries)")?;
        let rows = stmt.query_map([], |row| row.get(1))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for column in [
        "cache_key",
        "memory_revision",
        "graph_revision",
        "provider",
        "model",
        "prompt_version",
    ] {
        if !columns.iter().any(|c| c == column) {
            conn.execute_batch(&format!(
                "ALTER TABLE memory_graph_summaries ADD COLUMN {column} TEXT NOT NULL DEFAULT ''"
            ))?;
        }
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS argos_graph_explanations (
           memory_id TEXT PRIMARY KEY,
           job_id TEXT NOT NULL DEFAULT '',
           request_id TEXT NOT NULL DEFAULT '',
           cache_key TEXT NOT NULL DEFAULT '',
           state TEXT NOT NULL DEFAULT '',
           category TEXT NOT NULL DEFAULT '',
           reason TEXT NOT NULL DEFAULT '',
           guidance TEXT NOT NULL DEFAULT '',
           needs_config INTEGER NOT NULL DEFAULT 0,
           retryable INTEGER NOT NULL DEFAULT 0,
           attempts INTEGER NOT NULL DEFAULT 0,
           event_id TEXT NOT NULL DEFAULT '',
           diagnostic_json TEXT NOT NULL DEFAULT '',
           updated_at TEXT NOT NULL DEFAULT ''
         );",
    )?;
    Ok(())
}

impl Store {
    /// Stored summary with its cache key (empty for pre-revision rows).
    pub fn graph_summary_entry(&self, memory_id: &str) -> Result<Option<GraphSummaryEntry>> {
        Ok(self
            .conn
            .query_row(
                "SELECT summary, focus, cache_key, model, created_at
                 FROM memory_graph_summaries WHERE memory_id=?1",
                [memory_id],
                |row| {
                    Ok(GraphSummaryEntry {
                        summary: row.get(0)?,
                        focus: row.get(1)?,
                        cache_key: row.get(2)?,
                        model: row.get(3)?,
                        created_at: row.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    /// Save a summary for exactly `key`. Refuses when the memory is gone or
    /// its text no longer matches the revision the summary was written from.
    pub fn save_graph_summary_keyed(
        &self,
        memory_id: &str,
        summary: &str,
        key: &SummaryKey,
    ) -> Result<SaveOutcome> {
        let summary = summary.trim();
        anyhow::ensure!(!summary.is_empty(), "graph summary is empty");
        let text: Option<String> = self
            .conn
            .query_row("SELECT text FROM memories WHERE id=?1", [memory_id], |r| {
                r.get(0)
            })
            .optional()?;
        let Some(text) = text else {
            return Ok(SaveOutcome::MemoryGone);
        };
        if super::publication::memory_revision(&text) != key.memory_revision {
            return Ok(SaveOutcome::MemoryChanged);
        }
        self.conn.execute(
            "INSERT INTO memory_graph_summaries
               (memory_id, summary, created_at, focus, cache_key, memory_revision,
                graph_revision, provider, model, prompt_version)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(memory_id) DO UPDATE SET summary=excluded.summary,
               created_at=excluded.created_at, focus=excluded.focus,
               cache_key=excluded.cache_key, memory_revision=excluded.memory_revision,
               graph_revision=excluded.graph_revision, provider=excluded.provider,
               model=excluded.model, prompt_version=excluded.prompt_version",
            params![
                memory_id,
                summary,
                chrono::Utc::now().to_rfc3339(),
                key.focus,
                key.digest(),
                key.memory_revision,
                key.graph_revision,
                key.provider,
                key.model,
                key.prompt_version
            ],
        )?;
        Ok(SaveOutcome::Saved)
    }

    pub fn graph_explanation_record(&self, memory_id: &str) -> Result<Option<ExplanationRecord>> {
        Ok(self
            .conn
            .query_row(
                "SELECT memory_id, job_id, request_id, cache_key, state, category, reason,
                        guidance, needs_config, retryable, attempts, event_id,
                        diagnostic_json, updated_at
                 FROM argos_graph_explanations WHERE memory_id=?1",
                [memory_id],
                |row| {
                    Ok(ExplanationRecord {
                        memory_id: row.get(0)?,
                        job_id: row.get(1)?,
                        request_id: row.get(2)?,
                        cache_key: row.get(3)?,
                        state: row.get(4)?,
                        category: row.get(5)?,
                        reason: row.get(6)?,
                        guidance: row.get(7)?,
                        needs_config: row.get::<_, i64>(8)? != 0,
                        retryable: row.get::<_, i64>(9)? != 0,
                        attempts: row.get(10)?,
                        event_id: row.get(11)?,
                        diagnostic_json: row.get(12)?,
                        updated_at: row.get(13)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn put_graph_explanation_record(&self, rec: &ExplanationRecord) -> Result<()> {
        put_record(&self.conn, rec)
    }

    /// Graph explanation executions for one memory, oldest first:
    /// `(job id, state, correlation id)`. Retries correlate to the failed job.
    pub fn graph_explanation_jobs(&self, memory_id: &str) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, state, correlation_id FROM argos_jobs
             WHERE operation='graph_explanation' AND run_ref=?1
             ORDER BY created_at, rowid",
        )?;
        let rows = stmt.query_map([format!("graph:{memory_id}")], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Current state of a registry job (`running`, `failed`, …).
    pub fn job_state(&self, job_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT state FROM argos_jobs WHERE id=?1", [job_id], |r| {
                r.get(0)
            })
            .optional()?)
    }
}

pub(crate) fn put_record(conn: &Connection, rec: &ExplanationRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO argos_graph_explanations
           (memory_id, job_id, request_id, cache_key, state, category, reason, guidance,
            needs_config, retryable, attempts, event_id, diagnostic_json, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
         ON CONFLICT(memory_id) DO UPDATE SET job_id=excluded.job_id,
           request_id=excluded.request_id, cache_key=excluded.cache_key,
           state=excluded.state, category=excluded.category, reason=excluded.reason,
           guidance=excluded.guidance, needs_config=excluded.needs_config,
           retryable=excluded.retryable, attempts=excluded.attempts,
           event_id=excluded.event_id, diagnostic_json=excluded.diagnostic_json,
           updated_at=excluded.updated_at",
        params![
            rec.memory_id,
            rec.job_id,
            rec.request_id,
            rec.cache_key,
            rec.state,
            rec.category,
            crate::events::redact(&rec.reason),
            rec.guidance,
            rec.needs_config as i64,
            rec.retryable as i64,
            rec.attempts,
            rec.event_id,
            crate::events::redact(&rec.diagnostic_json),
            rec.updated_at
        ],
    )?;
    Ok(())
}
