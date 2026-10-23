//! SurrealDB schema bootstrap (P3 step 5), the counterpart of
//! `Store::ensure_schema`.
//!
//! [`ensure_schema`] applies every storage module's schema, in dependency
//! order, in one transaction, and records the schema version on
//! `storage_meta:schema`. As in SQLite (`user_version`), a database written
//! by a newer build is refused and the transaction rolls back. Every definition
//! is `IF NOT EXISTS`, so a re-run (or a concurrent bootstrap, which retries
//! on conflict) is a no-op. There is no in-place migration ladder: the
//! SurrealDB database starts fresh (wipe, no import; acceptance.md P4).

use surrealdb::engine::any::Any;
use surrealdb::Surreal;

use super::jobs::{run, take};
use super::tx::{run_tx, RetryPolicy, TxError};

/// SurrealDB schema version this build writes.
pub const SCHEMA_VERSION: i64 = 1;

/// Module schemas in apply order: counters before the outbox; the graph
/// before lifecycle (which indexes `brain_graph_node`); Atlas before its
/// metadata (whose event is on `atlas_article`); jobs, insight and intel
/// before refresh.
pub fn schemas() -> [(&'static str, &'static str); 14] {
    [
        ("counters", super::counters::SCHEMA),
        ("outbox", super::outbox::SCHEMA),
        ("memory", super::memory::SCHEMA),
        ("graph", super::graph::SCHEMA),
        ("embeddings", super::embeddings::SCHEMA),
        ("processes", super::processes::SCHEMA),
        ("jobs", super::jobs::SCHEMA),
        ("insight", super::insight::SCHEMA),
        ("atlas", super::atlas::SCHEMA),
        ("atlas_metadata", super::atlas_metadata::SCHEMA),
        ("intel", super::intel::SCHEMA),
        ("lifecycle", super::lifecycle::SCHEMA),
        ("refresh", super::refresh::SCHEMA),
        ("meta", META_SCHEMA),
    ]
}

/// The version record's own table.
pub const META_SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS storage_meta SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS version ON storage_meta TYPE int;
DEFINE FIELD IF NOT EXISTS applied_at ON storage_meta TYPE string;";

/// What [`ensure_schema`] found and did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bootstrap {
    /// Version recorded before this call (0: fresh database).
    pub previous: i64,
    pub version: i64,
}

/// Apply every schema and record [`SCHEMA_VERSION`] in one transaction.
pub async fn ensure_schema(db: &Surreal<Any>) -> Result<Bootstrap, TxError> {
    let mut sql = String::new();
    for (_, schema) in schemas() {
        sql.push_str(schema.trim().trim_end_matches(';'));
        sql.push_str(";\n");
    }
    let sql = std::sync::Arc::new(sql);
    let out = run_tx(db, RetryPolicy::default(), |tx| {
        let sql = sql.clone();
        async move {
            // Definitions first (tables must exist to be read); a refusal
            // below aborts the transaction, so they roll back with it.
            run(&tx, tx.query(sql.as_str())).await?;
            let mut r = run(
                &tx,
                tx.query("SELECT VALUE version FROM storage_meta WHERE id = storage_meta:schema"),
            )
            .await?;
            let previous = take::<Vec<i64>>(&mut r, 0)?.into_iter().next().unwrap_or(0);
            if previous > SCHEMA_VERSION {
                return Err(TxError::Abort(format!(
                    "database schema version {previous} is newer than this Argos build"
                )));
            }
            run(
                &tx,
                tx.query(
                    "UPSERT storage_meta:schema SET version = $v, applied_at = $now RETURN NONE",
                )
                .bind(("v", SCHEMA_VERSION))
                .bind(("now", chrono::Utc::now().to_rfc3339())),
            )
            .await?;
            Ok(Bootstrap {
                previous,
                version: SCHEMA_VERSION,
            })
        }
    })
    .await?;
    Ok(out.value)
}
