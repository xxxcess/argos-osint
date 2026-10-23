//! Memory embedding writes.
//!
//! A write to a DISKANN index can fail to commit with a retryable
//! `TransactionConflict` ("Resource busy" on RocksDB) when the engine's
//! background compaction of that index runs at the same time. That holds in
//! server mode (RocksDB storage) and embedded mode alike, and the server never
//! retries user transactions (SurrealDB 3.2 upgrade notes, still true on
//! 3.3.4). It is inherent to the index, not to Argos contention, so every
//! embedding write goes through the bounded-retry runner. Do not write
//! `memory_embeddings` with a bare `db.query`.

use surrealdb::engine::any::Any;
use surrealdb::Surreal;

use super::jobs::{rid, run};
use super::tx::{run_tx, Committed, RetryPolicy, TxError};

/// Embedding schema (idempotent).
pub const SCHEMA: &str = include_str!("schema/embeddings.surql");

/// Vector dimension of `memory_embeddings.embedding`.
pub const DIMENSION: usize = 384;

/// Apply the embedding schema (idempotent).
pub async fn apply_schema(db: &Surreal<Any>) -> Result<(), TxError> {
    db.query(SCHEMA).await?.check()?;
    Ok(())
}

/// One embedding row. `key` is the record key, so a rewrite of the same key
/// replaces the vector instead of adding a second row.
#[derive(Debug, Clone)]
pub struct Embedding<'a> {
    pub key: &'a str,
    pub kind: &'a str,
    pub generation: i64,
    /// `memory::memory_revision` of the text the vector was computed from.
    pub revision: &'a str,
    /// Embedding model fingerprint.
    pub model: &'a str,
    pub embedding: &'a [f32],
}

/// Write (or replace) one embedding, retrying index-compaction conflicts.
/// Replaying the same row is idempotent, so a retry after an unknown commit
/// result needs no receipt.
pub async fn put(db: &Surreal<Any>, row: &Embedding<'_>) -> Result<Committed<()>, TxError> {
    if row.embedding.len() != DIMENSION {
        return Err(TxError::Abort(format!(
            "embedding has {} dimensions, expected {DIMENSION}",
            row.embedding.len()
        )));
    }
    let vector = row.embedding.to_vec();
    run_tx(db, RetryPolicy::default(), |tx| {
        let vector = vector.clone();
        async move {
            run(
                &tx,
                tx.query(
                    "UPSERT $r CONTENT { key: $key, kind: $kind, generation: $generation, \
                     revision: $revision, model: $model, embedding: $embedding } RETURN NONE",
                )
                .bind(("r", rid("memory_embeddings", row.key)))
                .bind(("key", row.key.to_string()))
                .bind(("kind", row.kind.to_string()))
                .bind(("generation", row.generation))
                .bind(("revision", row.revision.to_string()))
                .bind(("model", row.model.to_string()))
                .bind(("embedding", vector)),
            )
            .await?;
            Ok(())
        }
    })
    .await
}
