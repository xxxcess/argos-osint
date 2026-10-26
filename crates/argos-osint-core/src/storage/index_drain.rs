//! The index pool on SurrealDB: drain pending `index_change` rows for
//! memories into `memory_embeddings` (the SQLite `drain_index` counterpart).
//! An upsert embeds the memory into the serving generation (skipped when no
//! generation serves this embedder's model, or the memory is inactive); a
//! remove deletes its vector. The `record_kind` filter runs in SQL
//! (`outbox::pending_of_kind`), so other record kinds stay pending for their
//! own consumers and never consume this pass's limit. Upserts are buffered
//! into groups of up to [`CHUNK`] — one embedder run, one chunked
//! `put_multi` transaction, and one `ack_many` transaction per group.
//! Every change is still acknowledged at its own revision and generation,
//! so a stale or repeated ack changes nothing. Pending rows are read from
//! `seq` 0: an ack leaves the `pending` state, so the next pass's lowest
//! pending `seq` is the cursor.

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

/// Upserts embedded per group: one `Embedder::run` and one `put_multi` per
/// chunk (the batch size `vectors::put_all` already embeds with).
const CHUNK: usize = 32;

/// Process up to `limit` pending memory changes in `seq` order.
pub async fn drain_index(repo: &Repository, e: &Embedder, limit: i64) -> Result<Drained> {
    let mut out = Drained::default();
    let changes = outbox::pending_of_kind(repo.db(), "memory", 0, limit.max(1)).await?;
    let mut group: Vec<Change> = Vec::new();
    for change in changes {
        if normalized_operation(&change.operation) == "index_remove" {
            // `seq` order still decides the final state: flush buffered
            // upserts first, then apply the removal on top of them.
            flush(repo, e, &mut group, &mut out).await?;
            let (state, outcome) = match apply_remove(repo, &change).await {
                Ok(outcome) => ("completed", outcome),
                Err(err) => ("failed", format!("{err:#}")),
            };
            ack_all(
                repo,
                vec![outbox::AckItem {
                    op_key: change.key.clone(),
                    revision: change.revision.clone(),
                    generation: change.generation.clone(),
                    state: state.into(),
                    outcome: outcome.clone(),
                    work_key: work_key_of(&change),
                }],
                &mut out,
            )
            .await?;
        } else {
            if group.len() >= CHUNK {
                flush(repo, e, &mut group, &mut out).await?;
            }
            group.push(change);
        }
    }
    flush(repo, e, &mut group, &mut out).await?;
    Ok(out)
}

/// Embed one buffered group of upserts (at most [`CHUNK`] changes): one
/// embedder run, one chunked write, then one revision-exact `ack_many` for
/// the group. When the chunk fails it aborted as a whole, so the group is
/// retried row by row and only rows that still fail are acknowledged failed.
async fn flush(
    repo: &Repository,
    e: &Embedder,
    group: &mut Vec<Change>,
    out: &mut Drained,
) -> Result<()> {
    if group.is_empty() {
        return Ok(());
    }
    let ids: Vec<String> = group.iter().map(|c| c.record_id.clone()).collect();
    match vectors::index_new_batch(repo, e, &ids).await {
        Ok(applied) => {
            let items: Vec<outbox::AckItem> = group
                .drain(..)
                .zip(applied)
                .map(|(change, applied)| {
                    let work_key = work_key_of(&change);
                    outbox::AckItem {
                        op_key: change.key,
                        revision: change.revision,
                        generation: change.generation,
                        state: "completed".into(),
                        outcome: if applied { "indexed" } else { "skipped" }.into(),
                        work_key,
                    }
                })
                .collect();
            ack_all(repo, items, out).await?;
        }
        Err(_) => {
            // The chunk commits in one transaction, so one bad row or an
            // exhausted conflict retry aborts all of it. Retry row by row and
            // fail only the rows that still fail on their own.
            let mut items = Vec::with_capacity(group.len());
            for change in group.drain(..) {
                let ids = std::slice::from_ref(&change.record_id);
                let (state, outcome) = match vectors::index_new_batch(repo, e, ids).await {
                    Ok(applied) => (
                        "completed",
                        if applied.first().copied().unwrap_or(false) {
                            "indexed"
                        } else {
                            "skipped"
                        }
                        .to_string(),
                    ),
                    Err(err) => ("failed", format!("{err:#}")),
                };
                let work_key = work_key_of(&change);
                items.push(outbox::AckItem {
                    op_key: change.key,
                    revision: change.revision,
                    generation: change.generation,
                    state: state.into(),
                    outcome,
                    work_key,
                });
            }
            ack_all(repo, items, out).await?;
        }
    }
    Ok(())
}

fn work_key_of(change: &Change) -> String {
    if change.work_key.is_empty() {
        outbox::work_key(
            &change.record_kind,
            &change.record_id,
            &change.revision,
            &change.operation,
        )
    } else {
        change.work_key.clone()
    }
}

/// Acknowledge one group in a single transaction and count each row.
async fn ack_all(repo: &Repository, items: Vec<outbox::AckItem>, out: &mut Drained) -> Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let completed = items
        .iter()
        .filter(|item| item.state == "completed")
        .count();
    let failed = items.len() - completed;
    outbox::ack_many(repo.db(), &items, Utc::now()).await?;
    out.completed += completed;
    out.failed += failed;
    Ok(())
}

async fn apply_remove(repo: &Repository, change: &Change) -> Result<String> {
    repo.db()
        .query("DELETE $r RETURN NONE")
        .bind(("r", rid("memory_embeddings", &change.record_id)))
        .await?
        .check()?;
    Ok("removed".into())
}
