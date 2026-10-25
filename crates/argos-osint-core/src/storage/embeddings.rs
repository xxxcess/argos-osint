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
//!
//! Phase-6 note: chunking does not weaken that rule. [`put_multi`] writes a
//! whole chunk inside one `run_tx` attempt, so a DISKANN busy-conflict still
//! aborts the transaction and retries the whole chunk through the same
//! bounded-retry runner; the per-row UPSERTs are idempotent, so the replay
//! rewrites the same rows.

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

/// One UPSERT statement per row, `{i}`-suffixed bindings, concatenated into
/// one query (the `outbox::enqueue_many_in` shape), so a chunk commits in a
/// single round trip inside one transaction.
fn put_multi_sql(n: usize) -> String {
    const STMT: &str = "UPSERT $r{i} CONTENT { key: $key{i}, kind: $kind{i}, \
        generation: $generation{i}, revision: $revision{i}, model: $model{i}, \
        embedding: $embedding{i} } RETURN NONE;";
    let mut sql = String::new();
    for i in 0..n {
        sql.push_str(&STMT.replace("{i}", &i.to_string()));
        sql.push(' ');
    }
    sql
}

/// Write (or replace) a whole chunk of embeddings in one transaction: one
/// UPSERT per row, all committed together, still through the bounded-retry
/// runner (a DISKANN busy-conflict aborts and replays the chunk; the writes
/// are idempotent). The statements, table and record keys match [`put`].
///
/// A row-level failure aborts the chunk before it starts and carries the
/// same `embedding {id}: {err}` shape `vectors::put_all` produced per row;
/// failures that only surface at run time (the database, exhausted
/// conflict retries) are not attributable to one row and propagate as-is.
pub async fn put_multi(
    db: &Surreal<Any>,
    rows: &[Embedding<'_>],
) -> Result<Committed<()>, TxError> {
    if rows.is_empty() {
        return Ok(Committed {
            value: (),
            retries: 0,
        });
    }
    for row in rows {
        if row.embedding.len() != DIMENSION {
            return Err(TxError::Abort(format!(
                "embedding {}: embedding has {} dimensions, expected {DIMENSION}",
                row.key,
                row.embedding.len()
            )));
        }
    }
    // Owned copies: the retryable body replays from them on every attempt.
    struct Owned {
        key: String,
        kind: String,
        generation: i64,
        revision: String,
        model: String,
        embedding: Vec<f32>,
    }
    let owned: Vec<Owned> = rows
        .iter()
        .map(|r| Owned {
            key: r.key.to_string(),
            kind: r.kind.to_string(),
            generation: r.generation,
            revision: r.revision.to_string(),
            model: r.model.to_string(),
            embedding: r.embedding.to_vec(),
        })
        .collect();
    let sql = put_multi_sql(rows.len());
    run_tx(db, RetryPolicy::default(), |tx| {
        let sql = sql.clone();
        let owned = &owned;
        async move {
            let mut q = tx.query(sql);
            for (i, r) in owned.iter().enumerate() {
                q = q
                    .bind((format!("r{i}"), rid("memory_embeddings", &r.key)))
                    .bind((format!("key{i}"), r.key.clone()))
                    .bind((format!("kind{i}"), r.kind.clone()))
                    .bind((format!("generation{i}"), r.generation))
                    .bind((format!("revision{i}"), r.revision.clone()))
                    .bind((format!("model{i}"), r.model.clone()))
                    .bind((format!("embedding{i}"), r.embedding.clone()));
            }
            run(&tx, q).await?;
            Ok(())
        }
    })
    .await
}
