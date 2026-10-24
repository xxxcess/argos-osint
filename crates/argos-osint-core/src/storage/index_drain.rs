//! The index pool on SurrealDB: drain pending `index_change` rows for
//! memories into `memory_embeddings` (the SQLite `drain_index` counterpart).
//! An upsert embeds the memory into the serving generation (skipped when no
//! generation serves this embedder's model, or the memory is inactive); a
//! remove deletes its vector. Each change is acknowledged at its revision
//! and generation, so a stale or repeated ack changes nothing. Other record
//! kinds stay pending for their own consumers.

use anyhow::Result;
use chrono::Utc;

use super::jobs::rid;
use super::outbox::{self, normalized_operation, Change};
use super::repo::Repository;
use super::vectors::{self, Embedder};

/// Drained counts for one pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Drained {
    pub completed: usize,
    pub failed: usize,
}

/// Process up to `limit` pending memory changes in `seq` order.
pub async fn drain_index(repo: &Repository, e: &Embedder, limit: i64) -> Result<Drained> {
    let mut out = Drained::default();
    let changes = outbox::pending(repo.db(), 0, limit.max(1)).await?;
    for change in changes.into_iter().filter(|c| c.record_kind == "memory") {
        let (state, outcome) = match apply(repo, e, &change).await {
            Ok(outcome) => ("completed", outcome),
            Err(err) => ("failed", format!("{err:#}")),
        };
        outbox::ack(
            repo.db(),
            &change.key,
            &change.revision,
            &change.generation,
            state,
            &outcome,
            Utc::now(),
        )
        .await?;
        if state == "completed" {
            out.completed += 1;
        } else {
            out.failed += 1;
        }
    }
    Ok(out)
}

async fn apply(repo: &Repository, e: &Embedder, change: &Change) -> Result<String> {
    match normalized_operation(&change.operation) {
        "index_remove" => {
            repo.db()
                .query("DELETE $r RETURN NONE")
                .bind(("r", rid("memory_embeddings", &change.record_id)))
                .await?
                .check()?;
            Ok("removed".into())
        }
        _ => {
            let n = vectors::index_new(repo, e, std::slice::from_ref(&change.record_id)).await?;
            Ok(if n == 0 { "skipped" } else { "indexed" }.into())
        }
    }
}
