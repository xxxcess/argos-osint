//! The per-memory graph summary cache on SurrealDB: the repository side of
//! `Store::graph_summary` / `Store::save_graph_summary`
//! (`memory_graph_summaries`).
//!
//! Rows live in `memory_graph_summary`, keyed by memory id (table defined by
//! `storage::insight`, whose delete paths already drop the row with its
//! memory). [`save_graph_summary`] checks the memory exists and writes the
//! summary in one transaction, so a summary never outlives a concurrent
//! delete. No new tables (bootstrap count unchanged).

use anyhow::{anyhow, ensure, Result};
use surrealdb::types::SurrealValue;

use super::jobs::{rid, run, take};
use super::repo::Repository;
use super::tx::{run_tx, RetryPolicy};
use crate::store::GraphSummary;

#[derive(SurrealValue)]
struct Row {
    summary: String,
    focus: Option<String>,
}

/// `Store::graph_summary`: the cached summary for `memory_id`, if any.
pub async fn graph_summary(repo: &Repository, memory_id: &str) -> Result<Option<GraphSummary>> {
    let mut r = repo
        .db()
        .query("SELECT summary, focus FROM ONLY $s")
        .bind(("s", rid("memory_graph_summary", memory_id)))
        .await?
        .check()?;
    let row: Option<Row> = r.take(0usize)?;
    Ok(row.map(|row| GraphSummary {
        summary: row.summary,
        focus: row.focus.unwrap_or_default(),
    }))
}

/// `Store::save_graph_summary`: store the synthesis explanation for one
/// memory (`focus` is the directive id). Returns false when the memory is
/// already gone. An empty summary is an error, as on SQLite.
pub async fn save_graph_summary(
    repo: &Repository,
    memory_id: &str,
    summary: &str,
    focus: &str,
) -> Result<bool> {
    let summary = summary.trim().to_string();
    ensure!(!summary.is_empty(), "graph summary is empty");
    let memory_id = memory_id.to_string();
    let focus = focus.to_string();
    let out = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (memory_id, summary, focus) = (memory_id.clone(), summary.clone(), focus.clone());
        async move {
            let mut r = run(
                &tx,
                tx.query(
                    "IF record::exists($m) {
                       UPSERT $s SET summary = $summary, focus = $focus, created_at = $now
                         RETURN NONE;
                       RETURN true;
                     } ELSE {
                       RETURN false;
                     };",
                )
                .bind(("m", rid("memory", &memory_id)))
                .bind(("s", rid("memory_graph_summary", &memory_id)))
                .bind(("summary", summary))
                .bind(("focus", focus))
                .bind(("now", chrono::Utc::now().to_rfc3339())),
            )
            .await?;
            Ok(take::<Option<bool>>(&mut r, 0)?.unwrap_or(false))
        }
    })
    .await
    .map_err(|e| anyhow!("save graph summary: {e}"))?;
    Ok(out.value)
}
