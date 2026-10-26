//! Index change outbox (replaces `argos_index_changes`).
//!
//! A change row and its `seq` commit with the logical change that caused it
//! (storage-audit.md:164): callers enqueue with [`enqueue_in`] inside their
//! own transaction, and `seq` comes from the `index_change` counter in the
//! same transaction, so a rolled-back change leaves neither a row nor a gap.
//! The row id is the caller's stable `op_key`, so replaying an enqueue after
//! an unknown commit result returns the first row instead of adding one.
//! Equivalent work coalesces (the old `argos_index_changes_active_work`
//! partial unique index, tasks.rs): while a change for the same work key
//! (record kind, record id, revision, normalized operation) is pending,
//! running or blocked, `index_work:[work_key]` names it,
//! and another caller's enqueue returns that row instead of adding one. The
//! lock is a record id, so two concurrent first enqueues conflict and the
//! retry sees the winner. A terminal ack releases it in the same transaction.
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
DEFINE FIELD IF NOT EXISTS work_key ON index_change TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS content_hash ON index_change TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS task_id ON index_change TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS job_id ON index_change TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS fingerprint ON index_change TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS generation_served ON index_change TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS error_category ON index_change TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS error_message ON index_change TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS created_at ON index_change TYPE datetime READONLY;
DEFINE FIELD IF NOT EXISTS updated_at ON index_change TYPE datetime;
DEFINE INDEX IF NOT EXISTS index_change_seq ON index_change FIELDS seq UNIQUE;
DEFINE INDEX IF NOT EXISTS index_change_pending ON index_change FIELDS state, seq;
DEFINE INDEX IF NOT EXISTS index_change_record ON index_change FIELDS record_kind, record_id, state;
DEFINE INDEX IF NOT EXISTS index_change_task ON index_change FIELDS task_id;
DEFINE TABLE IF NOT EXISTS index_work SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS change ON index_work TYPE string;";

/// States in which a change holds its work key.
pub const ACTIVE_STATES: [&str; 3] = ["pending", "running", "blocked"];

/// Normalized index operation (tasks.rs `normalized_index_operation`).
pub fn normalized_operation(operation: &str) -> &'static str {
    match operation {
        "upsert" | "index_upsert" => "index_upsert",
        "remove" | "index_remove" => "index_remove",
        _ => "index_rebuild",
    }
}

/// Work key string: kind, id, revision and normalized operation
/// (tasks.rs `index_work_key`).
pub fn work_key(kind: &str, id: &str, revision: &str, operation: &str) -> String {
    let op = normalized_operation(operation);
    format!("{kind}\u{1f}{id}\u{1f}{revision}\u{1f}{op}")
}

fn work_rid(c: &NewChange<'_>) -> surrealdb::types::RecordId {
    rid(
        "index_work",
        &work_key(c.record_kind, c.record_id, c.revision, c.operation),
    )
}

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
    pub content_hash: &'a str,
    pub task_id: &'a str,
    pub job_id: &'a str,
}

/// Result of an enqueue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Enqueued {
    pub seq: i64,
    /// op_key of the row that carries the work.
    pub key: String,
    /// False when the work coalesced into an existing active row (or the
    /// same op_key was replayed).
    pub created: bool,
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
    pub work_key: String,
    pub content_hash: String,
    pub task_id: String,
    pub job_id: String,
}

const CHANGE_FIELDS: &str = "record::id(id) AS key, seq, record_kind, record_id, revision, \
     operation, generation, state, outcome, work_key, content_hash, task_id, job_id";

/// Enqueue `change` inside the caller's transaction. Replaying the same
/// `op_key` returns that row (and is an error if it names other work); equivalent active work returns the active row.
pub async fn enqueue_in(
    tx: &Tx,
    change: &NewChange<'_>,
    op_key: &str,
    now: DateTime<Utc>,
) -> Result<Enqueued, TxError> {
    let mut res = run(
        tx,
        tx.query("SELECT seq, record_kind, record_id, revision, operation FROM ONLY $r")
            .bind(("r", rid("index_change", op_key))),
    )
    .await?;
    #[derive(SurrealValue)]
    struct Existing {
        seq: i64,
        record_kind: String,
        record_id: String,
        revision: String,
        operation: String,
    }
    if let Some(e) = take::<Option<Existing>>(&mut res, 0)? {
        // A replay must name the same work; reusing an op_key for other
        // work would silently drop it.
        let op = normalized_operation(change.operation);
        if (
            e.record_kind.as_str(),
            e.record_id.as_str(),
            e.revision.as_str(),
            e.operation.as_str(),
        ) != (change.record_kind, change.record_id, change.revision, op)
        {
            return Err(TxError::Abort(format!(
                "op_key {op_key} already names {}/{}@{} {}",
                e.record_kind, e.record_id, e.revision, e.operation
            )));
        }
        return Ok(Enqueued {
            seq: e.seq,
            key: op_key.to_string(),
            created: false,
        });
    }
    let lock = work_rid(change);
    let mut res = run(
        tx,
        tx.query(
            "LET $k = (SELECT VALUE change FROM ONLY $w); \
             SELECT record::id(id) AS key, seq FROM ONLY type::record('index_change', $k) \
             WHERE state IN $active",
        )
        .bind(("w", lock.clone()))
        .bind(("active", ACTIVE_STATES.to_vec())),
    )
    .await?;
    #[derive(SurrealValue)]
    struct Active {
        key: String,
        seq: i64,
    }
    if let Some(active) = take::<Option<Active>>(&mut res, 1)? {
        return Ok(Enqueued {
            seq: active.seq,
            key: active.key,
            created: false,
        });
    }
    let seq = counters::next_in(tx, SEQ_COUNTER).await?;
    let op = normalized_operation(change.operation);
    run(
        tx,
        tx.query(
            "CREATE $r SET seq = $seq, record_kind = $kind, record_id = $id, revision = $rev, \
             operation = $op, generation = $gen, state = 'pending', work_key = $wk, \
             content_hash = $hash, task_id = $task, job_id = $job, \
             created_at = <datetime>$now, updated_at = <datetime>$now RETURN NONE; \
             UPSERT $w SET change = $key RETURN NONE;",
        )
        .bind(("r", rid("index_change", op_key)))
        .bind(("w", lock))
        .bind(("key", op_key.to_string()))
        .bind(("seq", seq))
        .bind(("kind", change.record_kind.to_string()))
        .bind(("id", change.record_id.to_string()))
        .bind(("rev", change.revision.to_string()))
        .bind(("op", op.to_string()))
        .bind(("gen", change.generation.to_string()))
        .bind((
            "wk",
            work_key(change.record_kind, change.record_id, change.revision, op),
        ))
        .bind(("hash", change.content_hash.to_string()))
        .bind(("task", change.task_id.to_string()))
        .bind(("job", change.job_id.to_string()))
        .bind(("now", ts(now))),
    )
    .await?;
    Ok(Enqueued {
        seq,
        key: op_key.to_string(),
        created: true,
    })
}

/// One change's enqueue as a single SurrealQL block, same rules as
/// [`enqueue_in`]: op_key replay, active work-key coalescing, then a fresh
/// seq. `{i}` suffixes the bindings.
const ENQUEUE_BLOCK: &str = "{
    LET $e = (SELECT seq, record_kind, record_id, revision, operation FROM ONLY $r{i});
    IF $e != NONE {
        IF $e.record_kind != $kind{i} OR $e.record_id != $id{i} OR $e.revision != $rev{i}
            OR $e.operation != $op{i} {
            RETURN { conflict: 'op_key ' + $key{i} + ' already names ' + $e.record_kind + '/'
                + $e.record_id + '@' + $e.revision + ' ' + $e.operation };
        };
        RETURN { seq: $e.seq, key: $key{i}, created: false };
    };
    LET $k = (SELECT VALUE change FROM ONLY $w{i});
    IF $k != NONE {
        LET $a = (SELECT record::id(id) AS key, seq FROM ONLY type::record('index_change', $k)
            WHERE state IN $active);
        IF $a != NONE { RETURN { seq: $a.seq, key: $a.key, created: false }; };
    };
    LET $seq = (UPSERT $seq_counter SET n = (n ?? 0) + 1 RETURN VALUE n)[0];
    CREATE $r{i} SET seq = $seq, record_kind = $kind{i}, record_id = $id{i}, revision = $rev{i},
        operation = $op{i}, generation = $gen{i}, state = 'pending', work_key = $wk{i},
        content_hash = $hash{i}, task_id = $task{i}, job_id = $job{i},
        created_at = <datetime>$now, updated_at = <datetime>$now RETURN NONE;
    UPSERT $w{i} SET change = $key{i} RETURN NONE;
    RETURN { seq: $seq, key: $key{i}, created: true };
};
";

/// Enqueue many changes inside the caller's transaction in one round trip,
/// in order, then (if `bump` names a counter) allocate one value from it.
/// Equivalent to calling [`enqueue_in`] per change.
pub async fn enqueue_many_in(
    tx: &Tx,
    changes: &[(NewChange<'_>, &str)],
    bump: Option<&str>,
    now: DateTime<Utc>,
) -> Result<(Vec<Enqueued>, Option<i64>), TxError> {
    if changes.is_empty() && bump.is_none() {
        return Ok((Vec::new(), None));
    }
    let mut sql = String::new();
    for i in 0..changes.len() {
        sql.push_str(&ENQUEUE_BLOCK.replace("{i}", &i.to_string()));
    }
    if bump.is_some() {
        sql.push_str("UPSERT $bump SET n = (n ?? 0) + 1 RETURN VALUE n;");
    }
    let mut q = tx
        .query(sql)
        .bind(("seq_counter", rid("counter", SEQ_COUNTER)))
        .bind(("active", ACTIVE_STATES.to_vec()))
        .bind(("now", ts(now)));
    if let Some(name) = bump {
        q = q.bind(("bump", rid("counter", name)));
    }
    for (i, (c, op_key)) in changes.iter().enumerate() {
        let op = normalized_operation(c.operation);
        q = q
            .bind((format!("r{i}"), rid("index_change", op_key)))
            .bind((format!("w{i}"), work_rid(c)))
            .bind((format!("key{i}"), op_key.to_string()))
            .bind((format!("kind{i}"), c.record_kind.to_string()))
            .bind((format!("id{i}"), c.record_id.to_string()))
            .bind((format!("rev{i}"), c.revision.to_string()))
            .bind((format!("op{i}"), op.to_string()))
            .bind((format!("gen{i}"), c.generation.to_string()))
            .bind((
                format!("wk{i}"),
                work_key(c.record_kind, c.record_id, c.revision, op),
            ))
            .bind((format!("hash{i}"), c.content_hash.to_string()))
            .bind((format!("task{i}"), c.task_id.to_string()))
            .bind((format!("job{i}"), c.job_id.to_string()));
    }
    let mut res = run(tx, q).await?;
    #[derive(SurrealValue)]
    struct Row {
        seq: Option<i64>,
        key: Option<String>,
        created: Option<bool>,
        conflict: Option<String>,
    }
    let mut out = Vec::with_capacity(changes.len());
    for i in 0..changes.len() {
        let row: Option<Row> = take(&mut res, i)?;
        let row = row.ok_or_else(|| TxError::Abort(format!("enqueue {i} returned nothing")))?;
        if let Some(why) = row.conflict {
            return Err(TxError::Abort(why));
        }
        match (row.seq, row.key, row.created) {
            (Some(seq), Some(key), Some(created)) => out.push(Enqueued { seq, key, created }),
            _ => {
                return Err(TxError::Abort(format!(
                    "enqueue {i} returned a partial row"
                )))
            }
        }
    }
    let bumped = if bump.is_some() {
        let v: Vec<i64> = take(&mut res, changes.len())?;
        v.first().copied()
    } else {
        None
    };
    Ok((out, bumped))
}

/// Enqueue `change` in a transaction of its own. Every enqueue allocates
/// from one counter, so it uses the counter's hot-record policy.
pub async fn enqueue(
    db: &Surreal<Any>,
    change: &NewChange<'_>,
    op_key: &str,
    now: DateTime<Utc>,
) -> Result<Committed<Enqueued>, TxError> {
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
    pending_filtered(db, None, after_seq, limit).await
}

/// Pending changes of one `record_kind` after `after_seq`, in `seq` order.
/// The kind filter is pushed into SQL, so rows another consumer owns never
/// consume this caller's `limit` (or reach its loop to be filtered out).
pub async fn pending_of_kind(
    db: &Surreal<Any>,
    kind: &str,
    after_seq: i64,
    limit: i64,
) -> Result<Vec<Change>, TxError> {
    pending_filtered(db, Some(kind), after_seq, limit).await
}

/// [`pending`] and [`pending_of_kind`] share one query builder; `kind` is the
/// optional `record_kind = $kind` clause. `seq` ordering is kept either way.
async fn pending_filtered(
    db: &Surreal<Any>,
    kind: Option<&str>,
    after_seq: i64,
    limit: i64,
) -> Result<Vec<Change>, TxError> {
    let sql = format!(
        "SELECT {CHANGE_FIELDS} FROM index_change WHERE {}state = 'pending' \
         AND seq > $after ORDER BY seq LIMIT $n",
        if kind.is_some() {
            "record_kind = $kind AND "
        } else {
            ""
        }
    );
    let mut q = db.query(sql).bind(("after", after_seq)).bind(("n", limit));
    if let Some(kind) = kind {
        q = q.bind(("kind", kind.to_string()));
    }
    let mut res = q.await?.check()?;
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
    record_kind: String,
    record_id: String,
    revision: String,
    operation: String,
    generation: String,
    state: String,
}

impl AckRow {
    fn lock(&self) -> surrealdb::types::RecordId {
        work_rid(&NewChange {
            record_kind: &self.record_kind,
            record_id: &self.record_id,
            revision: &self.revision,
            operation: &self.operation,
            generation: "",
            content_hash: "",
            task_id: "",
            job_id: "",
        })
    }
}

const ACK_FIELDS: &str = "record_kind, record_id, revision, operation, generation, state";

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
            tx.query(format!("SELECT {ACK_FIELDS} FROM ONLY $r"))
                .bind(("r", rid("index_change", op_key))),
        )
        .await?;
        let Some(row) = take::<Option<AckRow>>(&mut res, 0)? else {
            return Ok(Ack::Missing);
        };
        if row.revision != revision || row.generation != generation {
            return Ok(Ack::Stale);
        }
        if !ACTIVE_STATES.contains(&row.state.as_str()) {
            return Ok(Ack::AlreadyDone);
        }
        run(
            &tx,
            tx.query(
                "UPDATE $r SET state = $state, outcome = $outcome, updated_at = <datetime>$now \
                 WHERE state IN $active RETURN NONE; \
                 DELETE $w WHERE change = $key RETURN NONE;",
            )
            .bind(("r", rid("index_change", op_key)))
            .bind(("w", row.lock()))
            .bind(("key", op_key.to_string()))
            .bind(("active", ACTIVE_STATES.to_vec()))
            .bind(("state", state.to_string()))
            .bind(("outcome", outcome.to_string()))
            .bind(("now", ts(now))),
        )
        .await?;
        Ok(Ack::Done)
    })
    .await
}

/// One acknowledgment inside [`ack_many`]. `work_key` is the `index_work`
/// record key released when the row is actually updated.
pub struct AckItem {
    pub op_key: String,
    pub revision: String,
    pub generation: String,
    pub state: String,
    pub outcome: String,
    pub work_key: String,
}

/// One change's ack as a SurrealQL block, same rules as [`ack`]: a row that
/// is missing, stale, or already terminal is left alone, and the work lock
/// is released only when this revision moves to a terminal state. `{i}`
/// suffixes the bindings. A chunk of these is one round trip.
const ACK_BLOCK: &str = "{
    LET $e = (SELECT revision, generation, state FROM ONLY $r{i});
    IF $e != NONE AND $e.revision = $rev{i} AND $e.generation = $gen{i} AND $e.state IN $active {
        UPDATE $r{i} SET state = $state{i}, outcome = $outcome{i}, updated_at = <datetime>$now \
            WHERE state IN $active RETURN NONE;
        DELETE $w{i} WHERE change = $key{i} RETURN NONE;
    };
    RETURN NONE;
};
";

fn ack_many_sql(n: usize) -> String {
    let mut sql = String::new();
    for i in 0..n {
        sql.push_str(&ACK_BLOCK.replace("{i}", &i.to_string()));
    }
    sql
}

/// Acknowledge every item in one transaction. Each item keeps [`ack`]'s
/// revision and generation check, so a stale row in the chunk does not
/// reject the others and does not release their work locks.
pub async fn ack_many(
    db: &Surreal<Any>,
    items: &[AckItem],
    now: DateTime<Utc>,
) -> Result<Committed<()>, TxError> {
    if items.is_empty() {
        return Ok(Committed {
            value: (),
            retries: 0,
        });
    }
    for item in items {
        if item.state != "completed" && item.state != "failed" {
            return Err(TxError::Abort(format!("invalid ack state {}", item.state)));
        }
    }
    let sql = ack_many_sql(items.len());
    let owned: Vec<AckItem> = items
        .iter()
        .map(|item| AckItem {
            op_key: item.op_key.clone(),
            revision: item.revision.clone(),
            generation: item.generation.clone(),
            state: item.state.clone(),
            outcome: item.outcome.clone(),
            work_key: item.work_key.clone(),
        })
        .collect();
    run_tx(db, RetryPolicy::default(), |tx| {
        let sql = sql.clone();
        let owned = &owned;
        async move {
            let mut q = tx
                .query(sql)
                .bind(("active", ACTIVE_STATES.to_vec()))
                .bind(("now", ts(now)));
            for (i, item) in owned.iter().enumerate() {
                q = q
                    .bind((format!("r{i}"), rid("index_change", &item.op_key)))
                    .bind((format!("w{i}"), rid("index_work", &item.work_key)))
                    .bind((format!("key{i}"), item.op_key.clone()))
                    .bind((format!("rev{i}"), item.revision.clone()))
                    .bind((format!("gen{i}"), item.generation.clone()))
                    .bind((format!("state{i}"), item.state.clone()))
                    .bind((format!("outcome{i}"), item.outcome.clone()));
            }
            run(&tx, q).await?;
            Ok(())
        }
    })
    .await
}

/// Move an active change to `running` or `blocked` (it keeps its work key).
/// Returns false if the row is missing or already terminal.
pub async fn set_active_state(
    db: &Surreal<Any>,
    op_key: &str,
    state: &str,
    now: DateTime<Utc>,
) -> Result<Committed<bool>, TxError> {
    if !ACTIVE_STATES.contains(&state) {
        return Err(TxError::Abort(format!("invalid active state {state}")));
    }
    run_tx(db, RetryPolicy::default(), |tx| async move {
        let mut res = run(
            &tx,
            tx.query(
                "UPDATE $r SET state = $state, updated_at = <datetime>$now \
                 WHERE state IN $active RETURN VALUE record::id(id)",
            )
            .bind(("r", rid("index_change", op_key)))
            .bind(("state", state.to_string()))
            .bind(("active", ACTIVE_STATES.to_vec()))
            .bind(("now", ts(now))),
        )
        .await?;
        Ok(!take::<Vec<String>>(&mut res, 0)?.is_empty())
    })
    .await
}
