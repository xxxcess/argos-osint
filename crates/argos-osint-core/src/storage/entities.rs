//! Recon entities and the actor receipts behind the repository (P3 item 2,
//! part 1): `recon_entities`, `brain_backfill_receipts` and
//! `brain_actor_dedup_receipts`.
//!
//! Record keys: `recon_entity` by entity id; `brain_backfill_receipt` by
//! `codec::encode([source, unit_id])`; `brain_actor_dedup_receipt` by the
//! sorted `left:right` pair key, as the legacy store. `recon_entity`'s
//! `(kind, namespace, canonical)` is unique, and [`ensure_recon_entity_in`]
//! ignores a row that would collide on it or on the id (`INSERT OR IGNORE`).

use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use super::codec::encode;
use super::jobs::{rid, run, take};
use super::tx::{run_tx, RetryPolicy, Tx, TxError};

/// Entity and receipt schema (idempotent).
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS recon_entity SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS kind ON recon_entity TYPE string;
DEFINE FIELD IF NOT EXISTS namespace ON recon_entity TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS canonical ON recon_entity TYPE string;
DEFINE FIELD IF NOT EXISTS label ON recon_entity TYPE string;
DEFINE INDEX IF NOT EXISTS recon_entity_key ON recon_entity FIELDS kind, namespace, canonical UNIQUE;

DEFINE TABLE IF NOT EXISTS brain_backfill_receipt SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS source ON brain_backfill_receipt TYPE string;
DEFINE FIELD IF NOT EXISTS unit_id ON brain_backfill_receipt TYPE string;
DEFINE FIELD IF NOT EXISTS revision ON brain_backfill_receipt TYPE string;
DEFINE FIELD IF NOT EXISTS coverage ON brain_backfill_receipt TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS completed_at ON brain_backfill_receipt TYPE string;
DEFINE INDEX IF NOT EXISTS brain_backfill_receipt_source ON brain_backfill_receipt FIELDS source;

DEFINE TABLE IF NOT EXISTS brain_actor_dedup_receipt SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS input_hash ON brain_actor_dedup_receipt TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS decision ON brain_actor_dedup_receipt TYPE string DEFAULT 'pending';
DEFINE FIELD IF NOT EXISTS lease_owner ON brain_actor_dedup_receipt TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS lease_until ON brain_actor_dedup_receipt TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS receipt_json ON brain_actor_dedup_receipt TYPE string DEFAULT '{}';
DEFINE FIELD IF NOT EXISTS updated_at ON brain_actor_dedup_receipt TYPE string;
DEFINE INDEX IF NOT EXISTS brain_actor_dedup_receipt_decision ON brain_actor_dedup_receipt FIELDS decision;";

/// One `recon_entities` row.
#[derive(Debug, Clone, PartialEq, Eq, SurrealValue)]
pub struct ReconEntity {
    pub id: String,
    pub kind: String,
    pub namespace: String,
    pub canonical: String,
    pub label: String,
}

/// `INSERT OR IGNORE INTO recon_entities`: create the entity unless its id
/// or its `(kind, namespace, canonical)` already exists. Returns whether a
/// row was created.
pub async fn ensure_recon_entity_in(tx: &Tx, e: &ReconEntity) -> Result<bool, TxError> {
    let mut r = run(
        tx,
        tx.query(
            "IF record::exists($r) OR (SELECT VALUE id FROM recon_entity
                 WHERE kind = $kind AND namespace = $ns AND canonical = $canon LIMIT 1) != [] {
               RETURN false;
             } ELSE {
               CREATE $r SET kind = $kind, namespace = $ns, canonical = $canon, label = $label
                 RETURN NONE;
               RETURN true;
             };",
        )
        .bind(("r", rid("recon_entity", &e.id)))
        .bind(("kind", e.kind.clone()))
        .bind(("ns", e.namespace.clone()))
        .bind(("canon", e.canonical.clone()))
        .bind(("label", e.label.clone())),
    )
    .await?;
    Ok(take::<Option<bool>>(&mut r, 0)? == Some(true))
}

/// One entity by id.
pub async fn recon_entity_in(tx: &Tx, id: &str) -> Result<Option<ReconEntity>, TxError> {
    let mut r = run(
        tx,
        tx.query("SELECT record::id(id) AS id, kind, namespace, canonical, label FROM ONLY $r")
            .bind(("r", rid("recon_entity", id))),
    )
    .await?;
    take::<Option<ReconEntity>>(&mut r, 0)
}

/// The entity's display label: `COALESCE(NULLIF(label,''), NULLIF(canonical,''))`.
pub async fn entity_label_in(tx: &Tx, id: &str) -> Result<Option<String>, TxError> {
    Ok(recon_entity_in(tx, id)
        .await?
        .and_then(|e| [e.label, e.canonical].into_iter().find(|s| !s.is_empty())))
}

/// Upsert one backfill receipt (`ON CONFLICT(source, unit_id) DO UPDATE`).
pub async fn put_backfill_receipt_in(
    tx: &Tx,
    source: &str,
    unit_id: &str,
    revision: &str,
    coverage: &str,
    now: &str,
) -> Result<(), TxError> {
    run(
        tx,
        tx.query(
            "UPSERT $r SET source = $source, unit_id = $unit, revision = $rev,
               coverage = $coverage, completed_at = $now RETURN NONE;",
        )
        .bind((
            "r",
            rid("brain_backfill_receipt", &encode(&[source, unit_id])),
        ))
        .bind(("source", source.to_string()))
        .bind(("unit", unit_id.to_string()))
        .bind(("rev", revision.to_string()))
        .bind(("coverage", coverage.to_string()))
        .bind(("now", now.to_string())),
    )
    .await?;
    Ok(())
}

/// `(unit_id, revision)` for every receipt of `source`, ordered by unit.
pub async fn backfill_receipts(
    db: &Surreal<Any>,
    source: &str,
) -> Result<Vec<(String, String)>, TxError> {
    #[derive(SurrealValue)]
    struct Row {
        unit_id: String,
        revision: String,
    }
    let mut r = db
        .query("SELECT unit_id, revision FROM brain_backfill_receipt WHERE source = $s ORDER BY unit_id")
        .bind(("s", source.to_string()))
        .await?
        .check()?;
    let rows: Vec<Row> = r.take(0usize)?;
    Ok(rows.into_iter().map(|r| (r.unit_id, r.revision)).collect())
}

/// The sorted pair key the legacy store uses.
pub fn pair_key(a: &str, b: &str) -> String {
    let mut pair = [a, b];
    pair.sort();
    format!("{}:{}", pair[0], pair[1])
}

/// Record an analyst's explicit split, so the dedup job never merges the
/// pair again; clears any lease, as `brain_actors::split` does.
pub async fn mark_separate_in(
    tx: &Tx,
    a: &str,
    b: &str,
    receipt_json: &str,
    now: &str,
) -> Result<(), TxError> {
    run(
        tx,
        tx.query(
            "UPSERT $r SET decision = 'separate', receipt_json = $receipt, lease_owner = '',
               lease_until = '', input_hash = input_hash ?? '', updated_at = $now RETURN NONE;",
        )
        .bind(("r", rid("brain_actor_dedup_receipt", &pair_key(a, b))))
        .bind(("receipt", receipt_json.to_string()))
        .bind(("now", now.to_string())),
    )
    .await?;
    Ok(())
}

/// Raw pair keys of every explicit separation (unresolved, as stored).
pub async fn separate_pair_keys(db: &Surreal<Any>) -> Result<Vec<String>, TxError> {
    let mut r = db
        .query(
            "SELECT VALUE record::id(id) FROM brain_actor_dedup_receipt
             WHERE decision = 'separate' ORDER BY id",
        )
        .await?
        .check()?;
    Ok(r.take(0usize)?)
}

/// [`ensure_recon_entity_in`] in its own transaction.
pub async fn ensure_recon_entity(db: &Surreal<Any>, e: &ReconEntity) -> Result<bool, TxError> {
    let e = e.clone();
    Ok(run_tx(db, RetryPolicy::default(), |tx| {
        let e = e.clone();
        async move { ensure_recon_entity_in(&tx, &e).await }
    })
    .await?
    .value)
}
