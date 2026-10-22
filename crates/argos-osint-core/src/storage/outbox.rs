//! Index change outbox (replaces `argos_index_changes`).
//!
//! A change row and its `seq` commit with the logical change that caused it
//! (storage-audit.md:164): callers enqueue with [`enqueue_in`] inside their
//! own transaction, and `seq` comes from the `index_change` counter in the
//! same transaction, so a rolled-back change leaves neither a row nor a gap.
//! The row id is the caller's stable `op_key`, so replaying an enqueue after
//! an unknown commit result returns the first row instead of adding one.
//! Consumers read pending rows in `seq` order from a persisted cursor
//! (storage-audit.md:174), and an acknowledgment names the exact revision and
//! generation it indexed: a stale worker's ack is rejected, and a repeated
//! ack is harmless.

use chrono::{DateTime, Utc};
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use super::counters;
use super::jobs::{rid, run, take, ts};
use super::tx::{run_tx, Committed, RetryPolicy, Tx, TxError};

/// Counter that orders outbox rows.
pub const SEQ_COUNTER: &str = "index_change";

/// Outbox schema (idempotent).
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS index_change SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS seq ON index_change TYPE int READONLY;
DEFINE FIELD IF NOT EXISTS record_kind ON index_change TYPE string;
DEFINE FIELD IF NOT EXISTS record_id ON index_change TYPE string;
DEFINE FIELD IF NOT EXISTS revision ON index_change TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS operation ON index_change TYPE string;
DEFINE FIELD IF NOT EXISTS generation ON index_change TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS state ON index_change TYPE string DEFAULT 'pending';
DEFINE FIELD IF NOT EXISTS outcome ON index_change TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS created_at ON index_change TYPE datetime READONLY;
DEFINE FIELD IF NOT EXISTS updated_at ON index_change TYPE datetime;
DEFINE INDEX IF NOT EXISTS index_change_seq ON index_change FIELDS seq UNIQUE;
DEFINE INDEX IF NOT EXISTS index_change_pending ON index_change FIELDS state, seq;
DEFINE INDEX IF NOT EXISTS index_change_record ON index_change FIELDS record_kind, record_id, state;";

/// Apply the outbox schema and the counter schema it uses (idempotent).
pub async fn apply_schema(db: &Surreal<Any>) -> Result<(), TxError> {
    counters::apply_schema(db).await?;
    db.query(SCHEMA).await?.check()?;
    Ok(())
}

/// A change to enqueue.
#[derive(Debug, Clone)]
pub struct NewChange<'a> {
    pub record_kind: &'a str,
    pub record_id: &'a str,
    pub revision: &'a str,
    pub operation: &'a str,
    pub generation: &'a str,
}

/// A stored change row.
#[derive(Debug, Clone, PartialEq, Eq, SurrealValue)]
pub struct Change {
    pub key: String,
    pub seq: i64,
    pub record_kind: String,
    pub record_id: String,
    pub revision: String,
    pub operation: String,
    pub generation: String,
    pub state: String,
    pub outcome: String,
}

const CHANGE_FIELDS: &str = "record::id(id) AS key, seq, record_kind, record_id, revision, \
     operation, generation, state, outcome";

/// Enqueue `change` inside the caller's transaction and return its `seq`.
/// Replaying the same `op_key` returns the existing row's `seq`.
pub async fn enqueue_in(
    tx: &Tx,
    change: &NewChange<'_>,
    op_key: &str,
    now: DateTime<Utc>,
) -> Result<i64, TxError> {
    let mut res = run(
        tx,
        tx.query("SELECT VALUE seq FROM ONLY $r")
            .bind(("r", rid("index_change", op_key))),
    )
    .await?;
    if let Some(seq) = take::<Option<i64>>(&mut res, 0)? {
        return Ok(seq);
    }
    let seq = counters::next_in(tx, SEQ_COUNTER).await?;
    run(
        tx,
        tx.query(
            "CREATE $r SET seq = $seq, record_kind = $kind, record_id = $id, revision = $rev, \
             operation = $op, generation = $gen, state = 'pending', \
             created_at = <datetime>$now, updated_at = <datetime>$now RETURN NONE",
        )
        .bind(("r", rid("index_change", op_key)))
        .bind(("seq", seq))
        .bind(("kind", change.record_kind.to_string()))
        .bind(("id", change.record_id.to_string()))
        .bind(("rev", change.revision.to_string()))
        .bind(("op", change.operation.to_string()))
        .bind(("gen", change.generation.to_string()))
        .bind(("now", ts(now))),
    )
    .await?;
    Ok(seq)
}

/// Enqueue `change` in a transaction of its own. Every enqueue allocates
/// from one counter, so it uses the counter's hot-record policy.
pub async fn enqueue(
    db: &Surreal<Any>,
    change: &NewChange<'_>,
    op_key: &str,
    now: DateTime<Utc>,
) -> Result<Committed<i64>, TxError> {
    run_tx(db, counters::HOT_RECORD_POLICY, |tx| async move {
        enqueue_in(&tx, change, op_key, now).await
    })
    .await
}

/// Pending changes after `after_seq` (the consumer's persisted cursor), in
/// `seq` order.
pub async fn pending(
    db: &Surreal<Any>,
    after_seq: i64,
    limit: i64,
) -> Result<Vec<Change>, TxError> {
    let mut res = db
        .query(format!(
            "SELECT {CHANGE_FIELDS} FROM index_change \
             WHERE state = 'pending' AND seq > $after ORDER BY seq LIMIT $n"
        ))
        .bind(("after", after_seq))
        .bind(("n", limit))
        .await?
        .check()?;
    Ok(res.take(0)?)
}

/// Load one change by its op_key.
pub async fn get(db: &Surreal<Any>, op_key: &str) -> Result<Option<Change>, TxError> {
    let mut res = db
        .query(format!("SELECT {CHANGE_FIELDS} FROM ONLY $r"))
        .bind(("r", rid("index_change", op_key)))
        .await?
        .check()?;
    Ok(res.take(0)?)
}

/// Result of an acknowledgment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ack {
    /// The row was pending at this revision and generation; it is now done.
    Done,
    /// Already acknowledged (a repeated ack); nothing changed.
    AlreadyDone,
    /// The row names another revision or generation; the ack is rejected.
    Stale,
    /// No such row.
    Missing,
}

#[derive(Debug, SurrealValue)]
struct AckRow {
    revision: String,
    generation: String,
    state: String,
}

/// Acknowledge change `op_key`, indexed at `revision` / `generation`, with
/// final `state` (`completed` or `failed`) and `outcome`.
pub async fn ack(
    db: &Surreal<Any>,
    op_key: &str,
    revision: &str,
    generation: &str,
    state: &str,
    outcome: &str,
    now: DateTime<Utc>,
) -> Result<Committed<Ack>, TxError> {
    if state != "completed" && state != "failed" {
        return Err(TxError::Abort(format!("invalid ack state {state}")));
    }
    run_tx(db, RetryPolicy::default(), |tx| async move {
        let mut res = run(
            &tx,
            tx.query("SELECT revision, generation, state FROM ONLY $r")
                .bind(("r", rid("index_change", op_key))),
        )
        .await?;
        let Some(row) = take::<Option<AckRow>>(&mut res, 0)? else {
            return Ok(Ack::Missing);
        };
        if row.revision != revision || row.generation != generation {
            return Ok(Ack::Stale);
        }
        if row.state != "pending" {
            return Ok(Ack::AlreadyDone);
        }
        run(
            &tx,
            tx.query(
                "UPDATE $r SET state = $state, outcome = $outcome, updated_at = <datetime>$now \
                 WHERE state = 'pending' RETURN NONE",
            )
            .bind(("r", rid("index_change", op_key)))
            .bind(("state", state.to_string()))
            .bind(("outcome", outcome.to_string()))
            .bind(("now", ts(now))),
        )
        .await?;
        Ok(Ack::Done)
    })
    .await
}
