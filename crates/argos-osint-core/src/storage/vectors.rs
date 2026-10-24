//! Brain vectors on the async repository: the serving embedding generation,
//! `argos memories reindex`, and the query embedding that `recall` uses, for
//! the CLI and TUI.
//!
//! `storage_meta:vectors` records the serving generation and the model
//! fingerprint that produced it. A reindex embeds every active memory into
//! the next generation and records each memory's index state. Only then
//! does it switch the serving generation and drop older rows, so recall
//! never sees a half-built generation. The fences in `storage::recall`
//! (generation, model, revision) drop stale vectors. Embedding is CPU work
//! and runs on the blocking pool, never on a runtime worker.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use surrealdb::types::SurrealValue;

use super::embeddings::{self, Embedding};
use super::jobs::run;
use super::memory::{self, memory_revision, IndexedRow};
use super::repo::Repository;
use super::tx::{run_tx, RetryPolicy};

/// Texts in, one vector per text out.
pub type EmbedFn = Arc<dyn Fn(Vec<String>) -> Result<Vec<Vec<f32>>> + Send + Sync>;

/// An embedder and the fingerprint of its model.
#[derive(Clone)]
pub struct Embedder {
    pub model: String,
    pub embed: EmbedFn,
}

impl Embedder {
    /// The local Brain model (`crate::embed`), or None when `ARGOS_EMBED` is off.
    pub fn local() -> Option<Self> {
        if !crate::embed::enabled() {
            return None;
        }
        Some(Self {
            model: crate::embed::fingerprint(),
            embed: Arc::new(|texts: Vec<String>| {
                let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
                crate::embed::embed_batch(&refs)
            }),
        })
    }

    /// Embed on the blocking pool.
    pub async fn run(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let n = texts.len();
        let f = self.embed.clone();
        let out = tokio::task::spawn_blocking(move || f(texts))
            .await
            .map_err(|e| anyhow!("embedding task: {e}"))??;
        anyhow::ensure!(
            out.len() == n,
            "embedder returned {} vectors for {n} texts",
            out.len()
        );
        Ok(out)
    }
}

/// The serving generation and its model.
#[derive(Debug, Clone, PartialEq, Eq, SurrealValue)]
pub struct VectorState {
    pub generation: i64,
    pub model: String,
}

pub async fn state(repo: &Repository) -> Result<Option<VectorState>> {
    let mut r = repo
        .db()
        .query(
            "SELECT embedding_generation AS generation, embedding_model AS model \
             FROM ONLY storage_meta:vectors WHERE embedding_generation != NONE",
        )
        .await?
        .check()?;
    Ok(r.take(0usize)?)
}

/// What a reindex did (`argos memories reindex`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReindexReport {
    pub memories: usize,
    pub generation: i64,
    pub fingerprint: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct Active {
    id: String,
    text: String,
    memory_kind: String,
}

const BATCH: usize = 32;

async fn put_all(repo: &Repository, e: &Embedder, rows: &[Active], generation: i64) -> Result<()> {
    for chunk in rows.chunks(BATCH) {
        let vectors = e
            .run(chunk.iter().map(|r| r.text.clone()).collect())
            .await?;
        for (row, v) in chunk.iter().zip(vectors) {
            let revision = memory_revision(&row.text);
            embeddings::put(
                repo.db(),
                &Embedding {
                    key: &row.id,
                    kind: &row.memory_kind,
                    generation,
                    revision: &revision,
                    model: &e.model,
                    embedding: &v,
                },
            )
            .await
            .map_err(|err| anyhow!("embedding {}: {err}", row.id))?;
        }
    }
    let indexed: Vec<IndexedRow> = rows
        .iter()
        .map(|r| IndexedRow {
            id: r.id.clone(),
            text: r.text.clone(),
        })
        .collect();
    memory::record_index_states(repo.db(), &indexed, &e.model, &generation.to_string())
        .await
        .map_err(|err| anyhow!("record index states: {err}"))?;
    Ok(())
}

/// Embed every active memory into a new generation, then serve it.
pub async fn reindex(repo: &Repository, e: &Embedder) -> Result<ReindexReport> {
    let generation = state(repo).await?.map_or(1, |s| s.generation + 1);
    let mut r = repo
        .db()
        .query(
            "SELECT meta::id(id) AS id, text, memory_kind FROM memory WHERE lifecycle = 'active'",
        )
        .await?
        .check()?;
    let rows: Vec<Active> = r.take(0usize)?;
    put_all(repo, e, &rows, generation).await?;
    let model = e.model.clone();
    run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let model = model.clone();
        async move {
            run(
                &tx,
                tx.query(
                    "UPSERT storage_meta:vectors SET embedding_generation = $g, \
                     embedding_model = $m RETURN NONE; \
                     DELETE memory_embeddings WHERE generation != $g RETURN NONE;",
                )
                .bind(("g", generation))
                .bind(("m", model)),
            )
            .await?;
            Ok(())
        }
    })
    .await
    .map_err(|err| anyhow!("switch generation: {err}"))?;
    Ok(ReindexReport {
        memories: rows.len(),
        generation,
        fingerprint: e.model.clone(),
    })
}

/// Embed the given memories into the serving generation, if one exists
/// for this embedder's model (post-commit, after `add_memory`). A memory
/// that is gone or inactive is skipped.
pub async fn index_new(repo: &Repository, e: &Embedder, ids: &[String]) -> Result<usize> {
    let Some(s) = state(repo).await? else {
        return Ok(0);
    };
    if s.model != e.model {
        return Ok(0);
    }
    let ids: Vec<_> = ids
        .iter()
        .map(|id| super::jobs::rid("memory", id))
        .collect();
    let mut r = repo
        .db()
        .query("SELECT meta::id(id) AS id, text, memory_kind FROM $ids WHERE lifecycle = 'active'")
        .bind(("ids", ids))
        .await?
        .check()?;
    let rows: Vec<Active> = r.take(0usize)?;
    put_all(repo, e, &rows, s.generation).await?;
    Ok(rows.len())
}

/// The query vector for recall: Some only when a generation is served for
/// this embedder's model. An embedding failure is an error; the caller
/// decides whether to fall back to lexical recall.
pub async fn query_vector(
    repo: &Repository,
    e: &Embedder,
    text: &str,
) -> Result<Option<(VectorState, Vec<f32>)>> {
    let Some(s) = state(repo).await? else {
        return Ok(None);
    };
    if s.model != e.model {
        return Ok(None);
    }
    let v = e
        .run(vec![text.to_string()])
        .await?
        .pop()
        .ok_or_else(|| anyhow!("no query vector"))?;
    Ok(Some((s, v)))
}
