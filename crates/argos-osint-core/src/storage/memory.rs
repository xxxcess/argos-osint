//! Memory publication on SurrealDB (P2): the `write_then_index` and
//! `record_index_states` write sites of `Store`.
//!
//! A memory write and the index work it implies commit together through the
//! process [`Publisher`]: the write runs, then [`index_changes_in`] reads the
//! touched memories and queues upsert work for each active memory whose
//! revision is not recorded as indexed, and removal work for each id that is
//! gone. Vector writes happen after commit, never inside the transaction.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures_util::future::BoxFuture;
use surrealdb::engine::any::Any;
use surrealdb::types::{RecordId, SurrealValue};
use surrealdb::Surreal;

use super::jobs::{rid, run, take};
use super::publisher::{Change, Published, Publisher, Write, Wrote};
use super::tx::{run_tx, Committed, RetryPolicy, Tx, TxError};

/// Record kind for memory rows in the outbox (`store::publication`).
pub const MEMORY_RECORD: &str = "memory";
/// Revision of removal work (`store::publication`).
pub const DELETED_REVISION: &str = "deleted";

/// Memory schema (idempotent).
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS memory SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS text ON memory TYPE string;
DEFINE FIELD IF NOT EXISTS category ON memory TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS pinned ON memory TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS lifecycle ON memory TYPE string DEFAULT 'active';
DEFINE FIELD IF NOT EXISTS source_json ON memory TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS memory_kind ON memory TYPE string DEFAULT 'claim';
-- Discriminated `MemoryPayload` JSON; '' for untyped memories.
DEFINE FIELD IF NOT EXISTS payload_json ON memory TYPE string DEFAULT '';
-- Lexical recall pages active memories of the allowed kinds.
DEFINE INDEX IF NOT EXISTS memory_lifecycle_kind ON memory FIELDS lifecycle, memory_kind;
DEFINE FIELD IF NOT EXISTS created_at ON memory TYPE datetime DEFAULT time::now();
DEFINE TABLE IF NOT EXISTS memory_index_state SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS revision ON memory_index_state TYPE string;
DEFINE FIELD IF NOT EXISTS fingerprint ON memory_index_state TYPE string;
DEFINE FIELD IF NOT EXISTS generation ON memory_index_state TYPE string;
DEFINE FIELD IF NOT EXISTS indexed_at ON memory_index_state TYPE datetime;";

/// Apply the memory schema (idempotent).
pub async fn apply_schema(db: &Surreal<Any>) -> Result<(), TxError> {
    db.query(SCHEMA).await?.check()?;
    db.query(super::graph::SCHEMA).await?.check()?;
    Ok(())
}

/// Stable revision of a memory's indexed content (same as SQLite).
pub fn memory_revision(text: &str) -> String {
    crate::evidence::content_hash(text)
}

/// Unique per publication, fixed across its retries, so op_keys replay
/// within one publication and never collide with an older one.
fn nonce() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

#[derive(SurrealValue)]
struct Touched {
    id: String,
    text: Option<String>,
    lifecycle: Option<String>,
    indexed: Option<String>,
}

/// Index work for `ids` after the caller's writes, read inside `tx`
/// (`enqueue_memory_index_on`). Duplicate ids are queued once.
pub async fn index_changes_in(
    tx: &Tx,
    ids: &[String],
    nonce: &str,
) -> Result<Vec<Change>, TxError> {
    let mut seen = std::collections::HashSet::new();
    let ids: Vec<String> = ids
        .iter()
        .filter(|i| seen.insert(i.as_str()))
        .cloned()
        .collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    // One statement per id, one round trip.
    let mut sql = String::new();
    for i in 0..ids.len() {
        sql.push_str(&format!(
            "RETURN {{ id: $i{i}, text: (SELECT VALUE text FROM ONLY $m{i}), \
             lifecycle: (SELECT VALUE lifecycle FROM ONLY $m{i}), \
             indexed: (SELECT VALUE revision FROM ONLY $s{i}) }};"
        ));
    }
    let mut q = tx.query(sql);
    for (i, id) in ids.iter().enumerate() {
        q = q
            .bind((format!("i{i}"), id.clone()))
            .bind((format!("m{i}"), rid("memory", id)))
            .bind((format!("s{i}"), rid("memory_index_state", id)));
    }
    let mut res = run(tx, q).await?;
    let mut rows = Vec::with_capacity(ids.len());
    for (i, id) in ids.iter().enumerate() {
        rows.push(
            take::<Option<Touched>>(&mut res, i)?
                .ok_or_else(|| TxError::Abort(format!("memory {id} lookup returned nothing")))?,
        );
    }
    let mut out = Vec::new();
    for t in rows {
        let active = t
            .text
            .filter(|_| t.lifecycle.as_deref().unwrap_or("active") == "active");
        let (revision, operation, hash) = match active {
            Some(text) => {
                let rev = memory_revision(&text);
                if t.indexed.as_deref() == Some(rev.as_str()) {
                    continue;
                }
                (rev.clone(), "index_upsert", rev)
            }
            None => (DELETED_REVISION.to_string(), "index_remove", String::new()),
        };
        out.push(Change {
            record_kind: MEMORY_RECORD.into(),
            op_key: format!("mem:{nonce}:{}", t.id),
            record_id: t.id,
            revision,
            operation: operation.into(),
            generation: String::new(),
            content_hash: hash,
        });
    }
    Ok(out)
}

/// The caller's memory statements; returns the memory ids it touched.
pub type MemoryWrite =
    Arc<dyn Fn(Tx) -> BoxFuture<'static, Result<Vec<String>, TxError>> + Send + Sync>;

/// `Store::write_then_index`: run `write` and queue the index work it
/// implies in one publisher transaction (which also bumps
/// `memories_changed`). Index failure after commit never fails the write;
/// the queued outbox rows are the durable record.
pub async fn write_then_index(
    publisher: &Publisher,
    write: MemoryWrite,
) -> Result<Committed<Published>, TxError> {
    let nonce = nonce();
    let w: Write = Arc::new(move |tx: Tx| {
        let write = write.clone();
        let nonce = nonce.clone();
        Box::pin(async move {
            let ids = write(tx.clone()).await?;
            // As in SQLite: no touched memory, no `memories_changed` bump.
            let changed = !ids.is_empty();
            let changes = index_changes_in(&tx, &ids, &nonce).await?;
            Ok(Wrote { changes, changed })
        })
    });
    publisher.publish(w).await
}

/// One indexed memory: id and the text whose vector was written.
#[derive(Debug, Clone, SurrealValue)]
pub struct IndexedRow {
    pub id: String,
    pub text: String,
}

/// `record_index_states` inside the caller's transaction, one query.
pub async fn record_index_states_in(
    tx: &Tx,
    rows: &[IndexedRow],
    fingerprint: &str,
    generation: &str,
) -> Result<(), TxError> {
    if rows.is_empty() {
        return Ok(());
    }
    #[derive(SurrealValue)]
    struct Row {
        id: String,
        rev: String,
    }
    let rows: Vec<Row> = rows
        .iter()
        .map(|r| Row {
            id: r.id.clone(),
            rev: memory_revision(&r.text),
        })
        .collect();
    run(
        tx,
        tx.query(
            "FOR $r IN $rows { \
                 UPSERT type::record('memory_index_state', $r.id) SET revision = $r.rev, \
                 fingerprint = $fp, generation = $gen, indexed_at = time::now() RETURN NONE; \
             };",
        )
        .bind(("rows", rows))
        .bind(("fp", fingerprint.to_string()))
        .bind(("gen", generation.to_string())),
    )
    .await?;
    Ok(())
}

/// `Store::record_index_states` (rebuilds): all rows in one transaction.
pub async fn record_index_states(
    db: &Surreal<Any>,
    rows: &[IndexedRow],
    fingerprint: &str,
    generation: &str,
) -> Result<Committed<()>, TxError> {
    run_tx(db, RetryPolicy::default(), |tx| async move {
        record_index_states_in(&tx, rows, fingerprint, generation).await
    })
    .await
}

/// A typed memory to write: canonical row plus its graph projection.
#[derive(Debug, Clone)]
pub struct TypedMemory {
    pub id: String,
    pub text: String,
    pub kind: crate::memory_metadata::MemoryKind,
    pub payload: Option<crate::memory_metadata::MemoryPayload>,
    pub lifecycle: String,
}

/// Write `m` and re-project its graph edges inside the caller's
/// transaction (graph projection stays synchronous with the canonical
/// write). Use inside a [`MemoryWrite`] so index work joins the same
/// publication.
pub async fn put_typed_in(tx: &Tx, m: &TypedMemory) -> Result<(), TxError> {
    let payload_json = match &m.payload {
        Some(p) => serde_json::to_string(p)
            .map_err(|e| TxError::Abort(format!("memory {} payload: {e}", m.id)))?,
        None => String::new(),
    };
    run(
        tx,
        tx.query(
            "UPSERT $m SET text = $text, memory_kind = $kind, payload_json = $payload, \
             lifecycle = $lc RETURN NONE",
        )
        .bind(("m", rid("memory", &m.id)))
        .bind(("text", m.text.clone()))
        .bind(("kind", m.kind.as_str().to_string()))
        .bind(("payload", payload_json))
        .bind(("lc", m.lifecycle.clone())),
    )
    .await?;
    super::graph::project_memory_in(tx, &m.id, m.kind, m.payload.as_ref(), &m.lifecycle).await
}

#[derive(SurrealValue)]
struct LifecycleRow {
    id: String,
    lifecycle: Option<String>,
    memory_kind: Option<String>,
    payload_json: Option<String>,
}

/// Set `lifecycle` on each existing memory in `ids` whose lifecycle differs,
/// re-projecting its graph node and edges in the caller's transaction (a
/// memory that stops being active loses the edges its payload produced).
/// Returns the ids that changed. One read and one write round trip.
pub async fn set_lifecycle_in(
    tx: &Tx,
    ids: &[String],
    lifecycle: &str,
) -> Result<Vec<String>, TxError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let rids: Vec<RecordId> = ids.iter().map(|i| rid("memory", i)).collect();
    let mut r = run(
        tx,
        tx.query(
            "SELECT record::id(id) AS id, lifecycle, memory_kind, payload_json FROM $rids \
             WHERE lifecycle != $lc",
        )
        .bind(("rids", rids))
        .bind(("lc", lifecycle.to_string())),
    )
    .await?;
    let rows: Vec<LifecycleRow> = take(&mut r, 0)?;
    let mut changed = Vec::with_capacity(rows.len());
    for row in rows {
        let _ = row.lifecycle;
        let kind = row
            .memory_kind
            .as_deref()
            .and_then(crate::memory_metadata::MemoryKind::parse)
            .unwrap_or(crate::memory_metadata::MemoryKind::Claim);
        let payload = row
            .payload_json
            .as_deref()
            .filter(|p| !p.is_empty())
            .and_then(|p| serde_json::from_str::<crate::memory_metadata::MemoryPayload>(p).ok());
        run(
            tx,
            tx.query("UPDATE $m SET lifecycle = $lc RETURN NONE")
                .bind(("m", rid("memory", &row.id)))
                .bind(("lc", lifecycle.to_string())),
        )
        .await?;
        super::graph::project_memory_in(tx, &row.id, kind, payload.as_ref(), lifecycle).await?;
        changed.push(row.id);
    }
    Ok(changed)
}
