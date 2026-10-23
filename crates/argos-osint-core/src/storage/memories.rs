//! Brain memory operations on the async repository, for the CLI and TUI
//! (`argos remember|recall|memories`). Same JSON shapes as the SQLite
//! `Store::add_memory`, `list_memories` and `recall`.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use surrealdb::types::SurrealValue;

use super::jobs::{rid, run};
use super::memory::{self, MemoryWrite};
use super::recall::{self, RecallQuery};
use super::repo::Repository;
use crate::brain::{normalize_category, Memory, MemorySource};

#[derive(Debug, Clone, SurrealValue)]
struct Row {
    id: String,
    text: String,
    category: String,
    pinned: bool,
    created_at: String,
    source_json: String,
}

impl Row {
    fn into_memory(self) -> Result<Memory> {
        Ok(Memory {
            source: serde_json::from_str(&self.source_json)
                .map_err(|e| anyhow!("memory {} source: {e}", self.id))?,
            id: self.id,
            text: self.text,
            category: self.category,
            pinned: self.pinned,
            created_at: self.created_at,
        })
    }
}

const ROW: &str = "meta::id(id) AS id, text, category, pinned, \
    time::format(created_at, '%+') AS created_at, source_json";

/// Save a sourced memory and queue its index work in one publication.
pub async fn add_memory(
    repo: &Repository,
    text: &str,
    category: &str,
    pinned: bool,
    source: MemorySource,
) -> Result<Memory> {
    source.validate()?;
    let text = text.trim();
    anyhow::ensure!(!text.is_empty(), "memory text is empty");
    let created = chrono::Utc::now();
    let memory = Memory {
        id: crate::store::new_id(),
        text: text.into(),
        category: normalize_category(category).into(),
        pinned,
        created_at: created.to_rfc3339(),
        source,
    };
    let source_json = serde_json::to_string(&memory.source)?;
    let m = memory.clone();
    let write: MemoryWrite = Arc::new(move |tx| {
        let m = m.clone();
        let source_json = source_json.clone();
        Box::pin(async move {
            run(
                &tx,
                tx.query(
                    "CREATE $m SET text = $text, category = $category, pinned = $pinned, \
                     created_at = <datetime>$at, source_json = $source RETURN NONE",
                )
                .bind(("m", rid("memory", &m.id)))
                .bind(("text", m.text.clone()))
                .bind(("category", m.category.clone()))
                .bind(("pinned", m.pinned))
                .bind(("at", m.created_at.clone()))
                .bind(("source", source_json)),
            )
            .await?;
            Ok(vec![m.id.clone()])
        })
    });
    memory::write_then_index(repo.publisher(), write)
        .await
        .map_err(|e| anyhow!("save memory: {e}"))?;
    Ok(memory)
}

/// Default Brain listing: active evidence/user memories, pinned first, newest first.
pub async fn list_memories(repo: &Repository) -> Result<Vec<Memory>> {
    let mut r = repo
        .db()
        .query(format!(
            "SELECT {ROW}, created_at AS at FROM memory WHERE lifecycle = 'active' \
             AND memory_kind NOTINSIDE ['actor', 'link', 'related'] \
             ORDER BY pinned DESC, at DESC"
        ))
        .await?
        .check()?;
    let rows: Vec<Row> = r.take(0usize)?;
    rows.into_iter().map(Row::into_memory).collect()
}

/// One recall hit with its memory (`argos recall` JSON).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScoredMemory {
    pub score: f32,
    pub memory: Memory,
}

/// Fenced recall (caps: five memories, token budget) without a query
/// vector, so the lexical path ranks.
pub async fn recall(repo: &Repository, query: &str, limit: usize) -> Result<Vec<ScoredMemory>> {
    if query.trim().is_empty() || limit == 0 {
        return Ok(Vec::new());
    }
    let hits = recall::recall(
        repo.db(),
        &RecallQuery {
            text: query.into(),
            max_memories: limit,
            ..RecallQuery::default()
        },
    )
    .await
    .map_err(|e| anyhow!("recall: {e}"))?;
    let ids: Vec<_> = hits.iter().map(|h| rid("memory", &h.id)).collect();
    let mut r = repo
        .db()
        .query(format!("SELECT {ROW} FROM $ids"))
        .bind(("ids", ids))
        .await?
        .check()?;
    let rows: Vec<Row> = r.take(0usize)?;
    let mut by_id: std::collections::HashMap<String, Memory> = rows
        .into_iter()
        .map(|row| row.into_memory().map(|m| (m.id.clone(), m)))
        .collect::<Result<_>>()?;
    Ok(hits
        .into_iter()
        .filter_map(|h| {
            by_id.remove(&h.id).map(|memory| ScoredMemory {
                score: h.score,
                memory,
            })
        })
        .collect())
}
