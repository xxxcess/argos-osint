//! Persisted transactional counters.
//!
//! Replaces the legacy store AUTOINCREMENT / `last_insert_rowid` and `MAX(seq)+1`
//! (storage-audit.md:120). A counter is a regular `counter:[name]` record, so
//! backup and restore include it. Allocation commits with the caller's
//! logical change when it runs inside the caller's transaction
//! ([`next_in`]); two concurrent allocations write the same record, so one
//! conflicts and is retried by the runner, and values are never shared.

use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use super::jobs::{rid, run, take, ts};
use super::tx::{run_tx, Committed, RetryPolicy, Tx, TxError};

/// Retry policy for allocations. Every allocator of one counter writes the
/// same record, so N concurrent allocators serialize: each conflict round
/// commits one of them, and the last can need about N attempts. The approved
/// default (6) covers the job system's sharded guards; a counter is one
/// unsharded record, so it gets 16 attempts (same backoff). With 8
/// concurrent allocators the worst case seen was 9 retries (server) and 8
/// (embedded) over 5 runs each. Callers that put
/// more concurrency on one counter must shard it or batch allocations.
pub const HOT_RECORD_POLICY: RetryPolicy = RetryPolicy {
    max_attempts: 16,
    base: std::time::Duration::from_millis(5),
    cap: std::time::Duration::from_millis(200),
};

/// Counter schema (idempotent).
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS counter SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS n ON counter TYPE int DEFAULT 0;";

/// Apply the counter schema (idempotent).
pub async fn apply_schema(db: &Surreal<Any>) -> Result<(), TxError> {
    db.query(SCHEMA).await?.check()?;
    Ok(())
}

/// Allocate the next value of `name` (1, 2, ...) inside an open transaction.
/// A rolled-back transaction gives its value back.
pub async fn next_in(tx: &Tx, name: &str) -> Result<i64, TxError> {
    let mut res = run(
        tx,
        tx.query("UPSERT $c SET n = (n ?? 0) + 1 RETURN VALUE n")
            .bind(("c", rid("counter", name))),
    )
    .await?;
    let values: Vec<i64> = take(&mut res, 0)?;
    values
        .first()
        .copied()
        .ok_or_else(|| TxError::Abort(format!("counter {name} returned no value")))
}

/// Current value of `name` (0 before the first allocation).
pub async fn current(db: &Surreal<Any>, name: &str) -> Result<i64, TxError> {
    let mut res = db
        .query("RETURN (SELECT VALUE n FROM ONLY $c) ?? 0")
        .bind(("c", rid("counter", name)))
        .await?
        .check()?;
    let v: Option<i64> = res.take(0)?;
    Ok(v.unwrap_or(0))
}

/// Receipt id for one allocation. Namespaced by counter so one caller
/// op_key used on two counters (or by another receipt kind) never collides.
fn receipt_key(name: &str, op_key: &str) -> String {
    format!("counter:{name}:{op_key}")
}

#[derive(Debug, SurrealValue)]
struct CounterReceipt {
    n: i64,
}

/// Allocate one value of `name` on its own. `op_key` is the caller's stable
/// operation key: replaying it (after an unknown commit result) returns the
/// value first allocated instead of consuming another.
pub async fn allocate(
    db: &Surreal<Any>,
    name: &str,
    op_key: &str,
) -> Result<Committed<i64>, TxError> {
    run_tx(db, HOT_RECORD_POLICY, |tx| async move {
        let mut res = run(
            &tx,
            tx.query("SELECT n FROM ONLY $r WHERE kind = 'counter' AND name = $name")
                .bind(("r", rid("tx_receipt", &receipt_key(name, op_key))))
                .bind(("name", name.to_string())),
        )
        .await?;
        if let Some(r) = take::<Option<CounterReceipt>>(&mut res, 0)? {
            return Ok(r.n);
        }
        let value = next_in(&tx, name).await?;
        run(
            &tx,
            tx.query(
                "CREATE $r SET kind = 'counter', name = $name, n = $value, \
                 at = <datetime>$now RETURN NONE",
            )
            .bind(("r", rid("tx_receipt", &receipt_key(name, op_key))))
            .bind(("name", name.to_string()))
            .bind(("value", value))
            .bind(("now", ts(chrono::Utc::now()))),
        )
        .await?;
        Ok(value)
    })
    .await
}
