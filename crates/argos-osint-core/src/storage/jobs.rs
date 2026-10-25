//! Job-system repository on SurrealDB (migration phase 1).
//!
//! Ports the scheduler election, task claim, lease renewal, lease reaper,
//! cleanup, stop, finalize, brief-chain and stage/neighborhood publication
//! fences. Each SQLite `BEGIN IMMEDIATE` boundary becomes a [`run_tx`]
//! transaction that writes a guard record first (see `tx`):
//!
//! | Guard | Serializes |
//! |---|---|
//! | `pool_guard:[pool, shard]` | claims by one shard of a pool's workers; the task row and `job_guard` catch cross-shard races |
//! | `job_guard:[root]` | Intel brief claims, stop, finalize, stage commit, cleanup of one job tree |
//! | `lease_sweep_guard:singleton` | the expired-lease reaper |
//! | `cleanup_guard:singleton` | retention cleanup and history clearing |
//! | `brief_chain_guard:[article]` | brief-chain generation allocation |
//! | `scheduler_lease:singleton` | scheduler election (the lease is the guard) |
//!
//! Claims and stage commits write a `tx_receipt` keyed by the caller's stable
//! operation key, so a retry after an unknown commit result returns the first
//! result. Every other write is idempotent: replaying it changes nothing.
//!
//! Times are RFC 3339 strings bound as parameters and cast with `<datetime>`.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use surrealdb::engine::any::Any;
use surrealdb::types::{RecordId, SurrealValue};
use surrealdb::{IndexedResults, Surreal};

use super::codec;
use super::tx::{run_tx, Committed, RetryPolicy, Tx, TxError};

/// The SurrealQL schema for the job system.
pub const SCHEMA: &str = include_str!("schema/jobs.surql");
/// Pool whose dependent stages run exclusively within one job.
pub const POOL_INTEL_BRIEF: &str = "intel_brief";
/// Infrastructure recoveries before a task fails (tasks.rs parity).
pub const MAX_WORKER_RECOVERIES: i64 = 3;
/// Execution age limit for claims and retention (72 hours).
pub const EXECUTION_WINDOW_HOURS: i64 = 72;
/// Candidates read per page. Each worker starts at its shard's offset in the
/// page (see [`claim_shard`]), so concurrent workers rarely pick the same task
/// row: with 8-row pages the embedded race still hit p99 4 and one exhaustion.
/// The cost is that a claim may take any of the first 32 queued tasks in
/// claim order rather than strictly the first.
pub const CLAIM_PAGE: i64 = 32;
/// Pages read per claim before it gives up (ineligible rows are skipped).
pub const CLAIM_MAX_PAGES: i64 = 2;
/// Claim guard shards per pool. One guard per pool exhausted 23 of 200 claims
/// in the 8-client race (p99 5 retries), over the review threshold (p99 > 3 or
/// any exhaustion), so each worker takes the guard of its own shard. Two
/// shards racing for one task still conflict on the task row, and two claims
/// in one job tree conflict on `job_guard:[root]`, so exclusivity holds.
pub const POOL_GUARD_SHARDS: u64 = 8;

/// One claim in this many (per pool, per process) starts at the queue head.
const HEAD_EVERY: u64 = 8;
/// Claim counters per pool. Per pool, so claims in one pool cannot absorb
/// every head turn of another (a process-wide counter alternating between
/// two pools could land every head claim on one of them).
static CLAIM_SEQ: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, u64>>> =
    std::sync::LazyLock::new(Default::default);

fn next_claim_seq(pool: &str) -> u64 {
    let mut seqs = CLAIM_SEQ.lock().unwrap_or_else(|e| e.into_inner());
    let n = seqs.entry(pool.to_string()).or_insert(0);
    let seq = *n;
    *n += 1;
    seq
}

/// Stable shard of `owner` (FNV-1a), used for its pool guard and to rotate
/// its candidate order so concurrent workers try different queue heads.
pub fn claim_shard(owner: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in owner.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h % POOL_GUARD_SHARDS
}
/// Approved bounded batch size for stopping a large job tree.
pub const STOP_BATCH: usize = 100;
/// Jobs or tasks handled per reaper / cleanup transaction.
pub const SWEEP_BATCH: i64 = 64;

const TERMINAL: &[&str] = &["completed", "failed", "partial", "cancelled", "superseded"];

/// Candidate scan for a claim: queued rows of one pool in priority order. The
/// index test runs this exact text under `EXPLAIN FULL`.
pub const CLAIM_CANDIDATES_SURQL: &str = "SELECT record::id(id) AS id, priority, created_at, \
     record::id(job) AS job_id, record::id(root) AS root_id, operation, pool, input_hash, \
     array::len(deps) AS dep_count, deps.state AS dep_states, \
     (job.cancel_requested OR array::any(job.ancestors.cancel_requested)) AS cancelled, \
     job.created_at >= <datetime>$fresh_after AS fresh, \
     record::exists(type::record('job_retirement', record::id(root))) AS retired \
     FROM task WHERE state = 'queued' AND pool = $pool AND next_eligible_at <= <datetime>$now \
     ORDER BY priority, created_at LIMIT $k START $start";

/// Order in which one worker tries a page of candidates: eligible rows only,
/// approximately FIFO. Only the leading run of eligible rows that share the
/// best priority is rotated by the claim's slot offset (`None` = the head), so the window never
/// crosses a priority level: a lower-priority task is tried only after every
/// eligible higher-priority row in the page. Rows blocked on dependencies are
/// not eligible, so they cannot hold back the work they wait on.
fn claim_order(candidates: Vec<Candidate>, slot: Option<u64>) -> Vec<Candidate> {
    let mut eligible: Vec<Candidate> = candidates.into_iter().filter(|c| c.eligible()).collect();
    let Some(top) = eligible.first().map(|c| c.priority) else {
        return eligible;
    };
    let bucket = eligible.iter().take_while(|c| c.priority == top).count();
    // Worker slots map into 1..bucket, never 0 (the oldest), for any bucket
    // size above 1. Only head claims (`None`) start at the oldest.
    let turn = match slot {
        Some(s) if bucket > 1 => 1 + (s as usize * (bucket - 1)) / POOL_GUARD_SHARDS as usize,
        _ => 0,
    };
    eligible[..bucket].rotate_left(turn);
    eligible
}

pub(crate) fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339()
}

pub(crate) fn rid(table: &str, key: &str) -> RecordId {
    RecordId::new(table, key.to_string())
}

pub(crate) async fn run(
    tx: &Tx,
    query: surrealdb::method::Query<'_, Any>,
) -> Result<IndexedResults, TxError> {
    let _ = tx;
    super::tx::QUERY_ROUND_TRIPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(query.await?.check()?)
}

pub(crate) fn take<T: SurrealValue>(res: &mut IndexedResults, i: usize) -> Result<T, TxError>
where
    usize: surrealdb::opt::QueryResult<T>,
{
    res.take::<T>(i).map_err(TxError::from)
}

pub(crate) async fn bump(tx: &Tx, table: &str, key: &str) -> Result<(), TxError> {
    run(
        tx,
        tx.query("UPSERT $g SET rev = (rev ?? 0) + 1 RETURN NONE")
            .bind(("g", rid(table, key))),
    )
    .await?;
    Ok(())
}

/// Apply the job-system schema (idempotent).
pub async fn apply_schema(db: &Surreal<Any>) -> Result<(), TxError> {
    db.query(SCHEMA).await?.check()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Jobs and tasks (creation helpers used by callers and tests)
// ---------------------------------------------------------------------------

/// A new job. `parent` makes it a child of an existing job tree.
#[derive(Debug, Clone)]
pub struct NewJob<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub operation: &'a str,
    pub parent: Option<&'a str>,
    pub created_at: DateTime<Utc>,
}

/// A new task in `job`.
#[derive(Debug, Clone)]
pub struct NewTask<'a> {
    pub id: &'a str,
    pub job: &'a str,
    pub pool: &'a str,
    pub operation: &'a str,
    pub priority: i64,
    pub deps: Vec<&'a str>,
    pub input_hash: &'a str,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, SurrealValue)]
struct Lineage {
    root_id: String,
    ancestors: Vec<RecordId>,
}

/// Dashboard app of a job, from its operation (`argos_jobs.app` parity).
pub fn job_app(operation: &str) -> &'static str {
    match operation {
        op if op.starts_with("intel_brief") || op.starts_with("insight") => "intel",
        op if op.starts_with("atlas") => "atlas",
        op if op.starts_with("recon") => "recon",
        op if op.starts_with("brain") || op.starts_with("memory") || op.starts_with("index") => {
            "brain"
        }
        "summaries" | "summary" => "intel",
        _ => "jobs",
    }
}

/// Dashboard title of a job: the operation in words.
pub fn job_title(operation: &str) -> String {
    let words = operation.replace('_', " ");
    let mut chars = words.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// Durable rollup written when a job reaches a terminal state: attempts
/// used and cap, active time (sum of closed attempt durations), queue time
/// (creation to the first task start; NONE when no task started) and the
/// failure summary from failed tasks.
pub(crate) async fn rollup_in(tx: &Tx, job_id: &str) -> Result<(), TxError> {
    run(
        tx,
        tx.query(
            "LET $tasks = SELECT VALUE id FROM task WHERE job = $j;
             LET $att = SELECT VALUE duration_ms FROM attempt WHERE task INSIDE $tasks;
             LET $first = (SELECT VALUE started_at FROM task WHERE job = $j AND started_at != NONE \
               ORDER BY started_at LIMIT 1)[0];
             LET $failed = SELECT VALUE error_category FROM task WHERE job = $j AND state = 'failed';
             UPDATE $j SET attempts_used = count($att),
               attempt_cap = math::sum((SELECT VALUE max_attempts FROM task WHERE job = $j)),
               active_ms = IF count($att) > 0 { math::sum($att.map(|$d| $d ?? 0)) } ELSE { active_ms },
               queue_ms = IF $first != NONE { duration::millis($first - created_at) } ELSE { queue_ms },
               error_summary = IF count($failed) > 0 { <string>count($failed) + ' task(s) failed' }
                 ELSE { error_summary },
               error_category = IF count($failed) > 0 { $failed.find(|$c| $c != '') ?? 'task_failed' }
                 ELSE { error_category }
             RETURN NONE;",
        )
        .bind(("j", rid("job", job_id))),
    )
    .await?;
    Ok(())
}

pub(crate) async fn create_job_in(tx: &Tx, job: &NewJob<'_>) -> Result<(), TxError> {
    let (root, ancestors) = match job.parent {
        None => (rid("job", job.id), Vec::new()),
        Some(parent) => {
            let mut res = run(
                tx,
                tx.query("SELECT record::id(root) AS root_id, ancestors FROM ONLY $p")
                    .bind(("p", rid("job", parent))),
            )
            .await?;
            let lineage: Option<Lineage> = take(&mut res, 0)?;
            let lineage =
                lineage.ok_or_else(|| TxError::Abort(format!("parent job {parent} missing")))?;
            let mut ancestors = lineage.ancestors;
            ancestors.push(rid("job", parent));
            (rid("job", &lineage.root_id), ancestors)
        }
    };
    run(
        tx,
        tx.query(
            "CREATE $id SET kind = $kind, operation = $op, parent = $parent, root = $root, \
             ancestors = $ancestors, state = 'running', app = $app, title = $title, \
             created_at = <datetime>$at, \
             updated_at = <datetime>$at RETURN NONE;
             FOR $a IN $ancestors {
                 CREATE type::record('job_ancestor', [record::id($a), record::id($id)])
                     SET ancestor = $a, job = $id, root = $root RETURN NONE;
             };",
        )
        .bind(("id", rid("job", job.id)))
        .bind(("kind", job.kind.to_string()))
        .bind(("op", job.operation.to_string()))
        .bind(("app", job_app(job.operation).to_string()))
        .bind(("title", job_title(job.operation)))
        .bind(("parent", job.parent.map(|p| rid("job", p))))
        .bind(("root", root))
        .bind(("ancestors", ancestors))
        .bind(("at", ts(job.created_at))),
    )
    .await?;
    Ok(())
}

pub(crate) async fn create_task_in(tx: &Tx, task: &NewTask<'_>) -> Result<(), TxError> {
    run(
        tx,
        tx.query(
            "CREATE $id SET job = $job, root = $job.root, pool = $pool, operation = $op, \
             state = 'queued', priority = $priority, deps = $deps, input_hash = $input, \
             created_at = <datetime>$at, next_eligible_at = <datetime>$at, \
             updated_at = <datetime>$at RETURN NONE",
        )
        .bind(("id", rid("task", task.id)))
        .bind(("job", rid("job", task.job)))
        .bind(("pool", task.pool.to_string()))
        .bind(("op", task.operation.to_string()))
        .bind(("priority", task.priority))
        .bind((
            "deps",
            task.deps.iter().map(|d| rid("task", d)).collect::<Vec<_>>(),
        ))
        .bind(("input", task.input_hash.to_string()))
        .bind(("at", ts(task.created_at))),
    )
    .await?;
    Ok(())
}

/// Create a job and its tasks in one transaction.
pub async fn create_job(
    db: &Surreal<Any>,
    job: &NewJob<'_>,
    tasks: &[NewTask<'_>],
) -> Result<(), TxError> {
    run_tx(db, RetryPolicy::default(), |tx| async move {
        create_job_in(&tx, job).await?;
        for task in tasks {
            create_task_in(&tx, task).await?;
        }
        Ok(())
    })
    .await
    .map(|_| ())
}

// ---------------------------------------------------------------------------
// Scheduler election (scheduler.rs:157 try_elect)
// ---------------------------------------------------------------------------

#[derive(Debug, SurrealValue)]
struct LeaseRow {
    owner: String,
    epoch: i64,
    live: bool,
}

/// Election result: whether `owner` holds the scheduler lease, and its epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Election {
    pub elected: bool,
    pub epoch: i64,
}

/// Take or renew the single scheduler lease. SQLite ran the read and the
/// conditional write as separate autocommit statements; here they share one
/// transaction on `scheduler_lease:singleton`, so concurrent candidates
/// conflict and the loser re-reads the winner. Server ownership is separate.
pub async fn try_elect(
    db: &Surreal<Any>,
    owner: &str,
    lease_secs: i64,
    now: DateTime<Utc>,
) -> Result<Committed<Election>, TxError> {
    let until = ts(now + ChronoDuration::seconds(lease_secs));
    let now_s = ts(now);
    run_tx(db, RetryPolicy::default(), |tx| {
        let (until, now_s) = (until.clone(), now_s.clone());
        async move {
            let key = rid("scheduler_lease", "singleton");
            let mut res = run(
                &tx,
                tx.query("SELECT owner, epoch, lease_until > <datetime>$now AS live FROM ONLY $k")
                    .bind(("k", key.clone()))
                    .bind(("now", now_s.clone())),
            )
            .await?;
            let current: Option<LeaseRow> = take(&mut res, 0)?;
            let epoch = match current {
                Some(l) if l.owner != owner && l.live => {
                    return Ok(Election {
                        elected: false,
                        epoch: l.epoch,
                    })
                }
                Some(l) if l.owner == owner => l.epoch,
                Some(l) => l.epoch + 1,
                None => 1,
            };
            run(
                &tx,
                tx.query(
                    "UPSERT $k SET owner = $owner, epoch = $epoch, \
                     lease_until = <datetime>$until, updated_at = <datetime>$now RETURN NONE",
                )
                .bind(("k", key))
                .bind(("owner", owner.to_string()))
                .bind(("epoch", epoch))
                .bind(("until", until))
                .bind(("now", now_s)),
            )
            .await?;
            Ok(Election {
                elected: true,
                epoch,
            })
        }
    })
    .await
}

// ---------------------------------------------------------------------------
// Claims (tasks.rs:996 claim_with, :960 claim_next_in, :986 claim_task)
// ---------------------------------------------------------------------------

/// A task leased to one owner at one epoch.
#[derive(Debug, Clone, PartialEq, Eq, SurrealValue)]
pub struct ClaimedTask {
    pub id: String,
    pub job_id: String,
    pub root_id: String,
    pub operation: String,
    pub pool: String,
    pub input_hash: String,
    pub lease_owner: String,
    pub lease_epoch: i64,
    pub attempts: i64,
    pub state: String,
}

#[derive(Debug, SurrealValue)]
struct Candidate {
    priority: i64,
    id: String,
    job_id: String,
    pool: String,
    input_hash: String,
    dep_count: i64,
    dep_states: Vec<String>,
    cancelled: bool,
    fresh: bool,
    retired: bool,
}

impl Candidate {
    fn eligible(&self) -> bool {
        self.fresh
            && !self.cancelled
            && !self.retired
            && self.dep_states.iter().all(|s| s == "completed")
            && (self.pool != POOL_INTEL_BRIEF || !self.input_hash.is_empty())
    }
}

#[derive(Debug, SurrealValue)]
struct Receipt {
    kind: String,
    task: String,
    epoch: i64,
}

const LOAD_CLAIMED: &str = "SELECT record::id(id) AS id, record::id(job) AS job_id, \
     record::id(root) AS root_id, operation, pool, input_hash, lease_owner, lease_epoch, \
     attempts, state FROM ONLY $t";

async fn load_task(tx: &Tx, id: &str) -> Result<Option<ClaimedTask>, TxError> {
    let mut res = run(tx, tx.query(LOAD_CLAIMED).bind(("t", rid("task", id)))).await?;
    take(&mut res, 0)
}

async fn receipt(tx: &Tx, op_key: &str) -> Result<Option<Receipt>, TxError> {
    let mut res = run(
        tx,
        tx.query("SELECT kind, task, epoch FROM ONLY $r")
            .bind(("r", rid("tx_receipt", op_key))),
    )
    .await?;
    take(&mut res, 0)
}

async fn write_receipt(
    tx: &Tx,
    op_key: &str,
    kind: &str,
    task: &str,
    epoch: i64,
    now: &str,
) -> Result<(), TxError> {
    run(
        tx,
        tx.query(
            "CREATE $r SET kind = $kind, task = $task, epoch = $epoch, at = <datetime>$now \
             RETURN NONE",
        )
        .bind(("r", rid("tx_receipt", op_key)))
        .bind(("kind", kind.to_string()))
        .bind(("task", task.to_string()))
        .bind(("epoch", epoch))
        .bind(("now", now.to_string())),
    )
    .await?;
    Ok(())
}

/// Claim the next eligible queued task of `pool` for `owner`.
///
/// `op_key` is the caller's stable operation key: retrying with the same key
/// after an unknown commit result returns the original claim.
pub async fn claim_next(
    db: &Surreal<Any>,
    pool: &str,
    owner: &str,
    lease_secs: i64,
    now: DateTime<Utc>,
    op_key: &str,
) -> Result<Committed<Option<ClaimedTask>>, TxError> {
    claim(db, pool, None, owner, lease_secs, now, op_key).await
}

/// Claim one specific task (ignores `next_eligible_at`, still checks
/// eligibility). `None` when another owner holds it or it is not queued.
pub async fn claim_task(
    db: &Surreal<Any>,
    pool: &str,
    task_id: &str,
    owner: &str,
    lease_secs: i64,
    now: DateTime<Utc>,
    op_key: &str,
) -> Result<Committed<Option<ClaimedTask>>, TxError> {
    claim(db, pool, Some(task_id), owner, lease_secs, now, op_key).await
}

async fn claim(
    db: &Surreal<Any>,
    pool: &str,
    task_id: Option<&str>,
    owner: &str,
    lease_secs: i64,
    now: DateTime<Utc>,
    op_key: &str,
) -> Result<Committed<Option<ClaimedTask>>, TxError> {
    // One claim in HEAD_EVERY (per pool) starts at the queue head; the
    // others start at the worker's own slot, and no slot is the head. So the
    // oldest task is reached within a bounded number of claims whichever
    // workers are alive or fast, and head claims rarely meet each other.
    let seq = next_claim_seq(pool);
    let head = seq.is_multiple_of(HEAD_EVERY);
    let tries = std::sync::atomic::AtomicU32::new(0);
    run_tx(db, RetryPolicy::default(), |tx| {
        // A head claim that lost a conflict retries from its own slot, so
        // retries never pile up on the head.
        let first = tries.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0;
        let slot = (!(head && first)).then(|| claim_shard(owner));
        async move { claim_in(&tx, pool, task_id, owner, lease_secs, now, op_key, slot).await }
    })
    .await
}

#[allow(clippy::too_many_arguments)]
async fn claim_in(
    tx: &Tx,
    pool: &str,
    task_id: Option<&str>,
    owner: &str,
    lease_secs: i64,
    now: DateTime<Utc>,
    op_key: &str,
    slot: Option<u64>,
) -> Result<Option<ClaimedTask>, TxError> {
    let now_s = ts(now);
    let shard = claim_shard(owner);
    bump(
        tx,
        "pool_guard",
        &codec::encode(&[pool, &shard.to_string()]),
    )
    .await?;
    if let Some(r) = receipt(tx, op_key).await? {
        if r.kind == "claim" {
            return Ok(load_task(tx, &r.task)
                .await?
                .filter(|t| t.lease_epoch == r.epoch));
        }
        return Err(TxError::Abort(format!(
            "operation key {op_key} names a {}",
            r.kind
        )));
    }
    let fresh_after = ts(now - ChronoDuration::hours(EXECUTION_WINDOW_HOURS));
    let pages = if task_id.is_some() {
        1
    } else {
        CLAIM_MAX_PAGES
    };
    for page in 0..pages {
        let candidates: Vec<Candidate> = match task_id {
            Some(id) => {
                let sql = CLAIM_CANDIDATES_SURQL
                    .replace(
                        "WHERE state = 'queued' AND pool = $pool AND next_eligible_at <= <datetime>$now",
                        "WHERE id = $id AND state = 'queued' AND pool = $pool",
                    );
                let mut res = run(
                    tx,
                    tx.query(sql)
                        .bind(("id", rid("task", id)))
                        .bind(("pool", pool.to_string()))
                        .bind(("now", now_s.clone()))
                        .bind(("fresh_after", fresh_after.clone()))
                        .bind(("k", 1i64))
                        .bind(("start", 0i64)),
                )
                .await?;
                take(&mut res, 0)?
            }
            None => {
                let mut res = run(
                    tx,
                    tx.query(CLAIM_CANDIDATES_SURQL)
                        .bind(("pool", pool.to_string()))
                        .bind(("now", now_s.clone()))
                        .bind(("fresh_after", fresh_after.clone()))
                        .bind(("k", CLAIM_PAGE))
                        .bind(("start", page * CLAIM_PAGE)),
                )
                .await?;
                take(&mut res, 0)?
            }
        };
        if candidates.is_empty() {
            break;
        }
        let ordered = claim_order(candidates, slot);
        // A candidate that cannot be claimed (an Intel brief already running
        // in its job, or a lost UPDATE race) is skipped and the scan keeps
        // paging, so one blocked row never hides eligible work behind it.
        for c in ordered.iter() {
            if c.pool == POOL_INTEL_BRIEF {
                // A dependent stage runs alone in its job; independent roots
                // run together (CLAIM_ELIGIBLE parity).
                let mut res = run(
                    tx,
                    tx.query(
                        "RETURN count(SELECT id FROM task WHERE job = $job AND state = 'running' \
                         AND id != $id AND ($mine OR array::len(deps) > 0))",
                    )
                    .bind(("job", rid("job", &c.job_id)))
                    .bind(("id", rid("task", &c.id)))
                    .bind(("mine", c.dep_count > 0)),
                )
                .await?;
                let busy: Option<i64> = take(&mut res, 0)?;
                if busy.unwrap_or(0) > 0 {
                    continue;
                }
            }
            // Confirm on the rows this UPDATE changed, not on a reread: a task
            // already running under the same owner (a reclaim under a new
            // op_key) changes no row and must not get a second attempt.
            let mut res = run(
                tx,
                tx.query(
                    "UPDATE $t SET state = 'running', lease_owner = $owner, process_owner = $proc, \
                     lease_until = <datetime>$until, lease_epoch += 1, attempts += 1, \
                     heartbeat_at = <datetime>$now, started_at = started_at ?? <datetime>$now, \
                     updated_at = <datetime>$now WHERE state = 'queued' RETURN AFTER",
                )
                .bind(("t", rid("task", &c.id)))
                .bind(("owner", owner.to_string()))
                .bind(("proc", crate::scheduler::process_owner()))
                .bind(("until", ts(now + ChronoDuration::seconds(lease_secs))))
                .bind(("now", now_s.clone())),
            )
            .await?;
            let changed: Vec<surrealdb::types::Value> = take(&mut res, 0)?;
            if changed.len() != 1 {
                continue;
            }
            let Some(task) = load_task(tx, &c.id).await? else {
                continue;
            };
            run(
                tx,
                tx.query(
                    "CREATE attempt SET task = $t, root = $root, number = $n, epoch = $e, \
                     owner = $owner, started_at = <datetime>$now RETURN NONE",
                )
                .bind(("t", rid("task", &task.id)))
                .bind(("root", rid("job", &task.root_id)))
                .bind(("n", task.attempts))
                .bind(("e", task.lease_epoch))
                .bind(("owner", owner.to_string()))
                .bind(("now", now_s.clone())),
            )
            .await?;
            if task.pool == POOL_INTEL_BRIEF {
                // Two brief claims in one tree must conflict for exclusivity.
                // Other claims race stop, cleanup and the reaper on the task
                // row itself, which they all write.
                bump(tx, "job_guard", &task.root_id).await?;
            }
            write_receipt(tx, op_key, "claim", &task.id, task.lease_epoch, &now_s).await?;
            return Ok(Some(task));
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Lease renewal (tasks.rs:1356) and the reaper (tasks.rs:1385)
// ---------------------------------------------------------------------------

/// Extend `claimed`'s lease. False when the lease was lost (epoch, owner or
/// state changed). Idempotent.
pub async fn renew_lease(
    db: &Surreal<Any>,
    claimed: &ClaimedTask,
    lease_secs: i64,
    now: DateTime<Utc>,
) -> Result<Committed<bool>, TxError> {
    run_tx(db, RetryPolicy::default(), |tx| async move {
        run(
            &tx,
            tx.query(
                "UPDATE $t SET lease_until = <datetime>$until, heartbeat_at = <datetime>$now, \
                 updated_at = <datetime>$now WHERE state = 'running' AND lease_owner = $owner \
                 AND lease_epoch = $e RETURN NONE",
            )
            .bind(("t", rid("task", &claimed.id)))
            .bind(("until", ts(now + ChronoDuration::seconds(lease_secs))))
            .bind(("now", ts(now)))
            .bind(("owner", claimed.lease_owner.clone()))
            .bind(("e", claimed.lease_epoch)),
        )
        .await?;
        let task = load_task(&tx, &claimed.id).await?;
        Ok(task.is_some_and(|t| {
            t.state == "running"
                && t.lease_owner == claimed.lease_owner
                && t.lease_epoch == claimed.lease_epoch
        }))
    })
    .await
}

#[derive(Debug, SurrealValue)]
struct Expired {
    id: String,
    root_id: String,
    lease_epoch: i64,
    recoveries: i64,
}

/// Recover expired leases: advance the epoch (fencing the stale worker),
/// close its attempt as `lease_expired`, and requeue, or fail after
/// [`MAX_WORKER_RECOVERIES`]. A lease renewed concurrently conflicts with
/// this transaction (both write the task), so it is never reaped.
pub async fn interrupt_expired_leases(
    db: &Surreal<Any>,
    now: DateTime<Utc>,
) -> Result<Committed<usize>, TxError> {
    run_tx(db, RetryPolicy::default(), |tx| async move {
        interrupt_expired_leases_in(&tx, now).await
    })
    .await
}

/// Reaper body inside an open transaction (exposed for interleaving tests).
pub async fn interrupt_expired_leases_in(tx: &Tx, now: DateTime<Utc>) -> Result<usize, TxError> {
    let now_s = ts(now);
    bump(tx, "lease_sweep_guard", "singleton").await?;
    let mut res = run(
        tx,
        tx.query(
            "SELECT record::id(id) AS id, record::id(root) AS root_id, lease_epoch, recoveries \
             FROM task WHERE state = 'running' AND lease_until <= <datetime>$now LIMIT $n",
        )
        .bind(("now", now_s.clone()))
        .bind(("n", SWEEP_BATCH)),
    )
    .await?;
    let expired: Vec<Expired> = take(&mut res, 0)?;
    let mut recovered = 0;
    for e in expired {
        requeue_running_in(
            tx,
            &e.id,
            &e.root_id,
            e.lease_epoch,
            e.recoveries,
            "lease_expired",
            &now_s,
            true,
        )
        .await?;
        recovered += 1;
    }
    Ok(recovered)
}

/// Fence a running task held at `epoch`: advance the epoch (a late write
/// from the old owner then fails its fence), close its open attempt with
/// `outcome`, and requeue it, or fail it after [`MAX_WORKER_RECOVERIES`].
/// Shared by the lease reaper (`only_expired`: the lease must also have run
/// out by now) and the process registry. A no-op if the task moved past
/// `epoch` or is no longer running.
pub(crate) async fn requeue_running_in(
    tx: &Tx,
    id: &str,
    root_id: &str,
    epoch: i64,
    recoveries: i64,
    outcome: &str,
    now_s: &str,
    only_expired: bool,
) -> Result<(), TxError> {
    let failed = recoveries + 1 >= MAX_WORKER_RECOVERIES;
    run(
        tx,
        tx.query(
            "UPDATE $t SET state = $next, lease_epoch += 1, lease_owner = '', process_owner = '', \
             lease_until = NONE, recoveries += 1, next_eligible_at = <datetime>$now, \
             finished_at = IF $failed THEN <datetime>$now ELSE NONE END, updated_at = <datetime>$now \
             WHERE state = 'running' AND lease_epoch = $e \
             AND (!$only_expired OR lease_until <= <datetime>$now) RETURN NONE;
             LET $open = SELECT VALUE id FROM attempt \
             WHERE task = $t AND epoch = $e AND finished_at = NONE;
             UPDATE $open SET finished_at = <datetime>$now, duration_ms = duration::millis(<datetime>$now - started_at), outcome = $outcome RETURN NONE;",
        )
        .bind(("t", rid("task", id)))
        .bind(("next", if failed { "failed" } else { "queued" }))
        .bind(("failed", failed))
        .bind(("now", now_s.to_string()))
        .bind(("e", epoch))
        .bind(("outcome", outcome.to_string()))
        .bind(("only_expired", only_expired)),
    )
    .await?;
    bump(tx, "job_guard", root_id).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Publication fences: stage commit (brief_pipeline.rs:451) and neighborhood
// brief (brain_neighborhood_brief_exec.rs:660)
// ---------------------------------------------------------------------------

/// Result of a fenced publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Publish {
    Committed,
    /// Owner, epoch, state or lease expiry no longer match; nothing written.
    LeaseLost,
    /// The job or an ancestor was cancelled; nothing written.
    Cancelled,
}

#[derive(Debug, SurrealValue)]
pub(crate) struct FenceRow {
    root_id: String,
    state: String,
    lease_owner: String,
    lease_epoch: i64,
    output_hash: String,
    live: bool,
    pub(crate) cancelled: bool,
}

pub(crate) async fn fence(
    tx: &Tx,
    claimed: &ClaimedTask,
    now: &str,
) -> Result<Option<FenceRow>, TxError> {
    let mut res = run(
        tx,
        tx.query(
            "SELECT record::id(root) AS root_id, state, lease_owner, lease_epoch, output_hash, \
             (lease_until ?? d'1970-01-01T00:00:00Z') > <datetime>$now AS live, \
             (job.cancel_requested OR array::any(job.ancestors.cancel_requested)) AS cancelled \
             FROM ONLY $t",
        )
        .bind(("t", rid("task", &claimed.id)))
        .bind(("now", now.to_string())),
    )
    .await?;
    take(&mut res, 0)
}

pub(crate) fn holds(f: &FenceRow, claimed: &ClaimedTask) -> bool {
    f.state == "running"
        && f.lease_owner == claimed.lease_owner
        && f.lease_epoch == claimed.lease_epoch
        && f.live
}

pub(crate) async fn complete_in(
    tx: &Tx,
    claimed: &ClaimedTask,
    hash: &str,
    now: &str,
) -> Result<(), TxError> {
    run(
        tx,
        tx.query(
            "UPDATE $t SET state = 'completed', output_hash = $hash, lease_owner = '', process_owner = '', \
             lease_until = NONE, finished_at = <datetime>$now, updated_at = <datetime>$now \
             RETURN NONE;
             LET $open = SELECT VALUE id FROM attempt \
             WHERE task = $t AND epoch = $e AND finished_at = NONE;
             UPDATE $open SET finished_at = <datetime>$now, duration_ms = duration::millis(<datetime>$now - started_at), outcome = 'completed' RETURN NONE;
             UPDATE $t SET active_ms = math::sum((SELECT VALUE duration_ms ?? 0 FROM attempt WHERE task = $t)) RETURN NONE;",
        )
        .bind(("t", rid("task", &claimed.id)))
        .bind(("hash", hash.to_string()))
        .bind(("now", now.to_string()))
        .bind(("e", claimed.lease_epoch)),
    )
    .await?;
    Ok(())
}

/// Commit a completed brief stage: lease fence, stage output and task
/// completion in one transaction. Dependents become claimable through the
/// claim's dependency check. `op_key` (stable per task and epoch) receipts it.
pub async fn commit_stage(
    db: &Surreal<Any>,
    claimed: &ClaimedTask,
    output_json: &str,
    output_hash: &str,
    now: DateTime<Utc>,
    op_key: &str,
) -> Result<Committed<Publish>, TxError> {
    let now_s = ts(now);
    run_tx(db, RetryPolicy::default(), |tx| {
        let now_s = now_s.clone();
        async move {
            bump(&tx, "job_guard", &claimed.root_id).await?;
            if let Some(r) = receipt(&tx, op_key).await? {
                if r.kind == "stage" && r.task == claimed.id && r.epoch == claimed.lease_epoch {
                    return Ok(Publish::Committed);
                }
                return Err(TxError::Abort(format!("operation key {op_key} reused")));
            }
            let Some(f) = fence(&tx, claimed, &now_s).await? else {
                return Ok(Publish::LeaseLost);
            };
            if !holds(&f, claimed) {
                return Ok(Publish::LeaseLost);
            }
            if f.cancelled {
                return Ok(Publish::Cancelled);
            }
            run(
                &tx,
                tx.query(
                    "UPSERT $o SET task = $t, root = $root, stage = $stage, \
                     output_hash = $hash, output_json = $json, committed_at = <datetime>$now \
                     RETURN NONE",
                )
                .bind(("o", rid("stage_output", &claimed.id)))
                .bind(("t", rid("task", &claimed.id)))
                .bind(("root", rid("job", &f.root_id)))
                .bind(("stage", claimed.operation.clone()))
                .bind(("hash", output_hash.to_string()))
                .bind(("json", output_json.to_string()))
                .bind(("now", now_s.clone())),
            )
            .await?;
            complete_in(&tx, claimed, output_hash, &now_s).await?;
            write_receipt(
                &tx,
                op_key,
                "stage",
                &claimed.id,
                claimed.lease_epoch,
                &now_s,
            )
            .await?;
            Ok(Publish::Committed)
        }
    })
    .await
}

/// Fenced neighborhood-brief publication: lease check, cache write and task
/// completion commit together. A replay of the same completed publication
/// (same epoch and output hash) reports `Committed` and writes nothing.
pub async fn publish_neighborhood(
    db: &Surreal<Any>,
    claimed: &ClaimedTask,
    body: &str,
    route: &str,
    output_hash: &str,
    now: DateTime<Utc>,
) -> Result<Committed<Publish>, TxError> {
    let now_s = ts(now);
    run_tx(db, RetryPolicy::default(), |tx| {
        let now_s = now_s.clone();
        async move {
            bump(&tx, "job_guard", &claimed.root_id).await?;
            let Some(f) = fence(&tx, claimed, &now_s).await? else {
                return Ok(Publish::LeaseLost);
            };
            if f.state == "completed"
                && f.lease_epoch == claimed.lease_epoch
                && f.output_hash == output_hash
            {
                return Ok(Publish::Committed);
            }
            if !holds(&f, claimed) {
                return Ok(Publish::LeaseLost);
            }
            if f.cancelled {
                return Ok(Publish::Cancelled);
            }
            run(
                &tx,
                tx.query(
                    "UPSERT $n SET task = $t, body = $body, route = $route, \
                     generated_at = <datetime>$now RETURN NONE",
                )
                .bind(("n", rid("neighborhood_brief", &claimed.id)))
                .bind(("t", rid("task", &claimed.id)))
                .bind(("body", body.to_string()))
                .bind(("route", route.to_string()))
                .bind(("now", now_s.clone())),
            )
            .await?;
            complete_in(&tx, claimed, output_hash, &now_s).await?;
            Ok(Publish::Committed)
        }
    })
    .await
}

/// Worker check before each stage: false once the job or an ancestor has
/// `cancel_requested`, or the task is no longer running.
pub async fn should_continue(db: &Surreal<Any>, claimed: &ClaimedTask) -> Result<bool, TxError> {
    let mut res = db
        .query(
            "SELECT state, (job.cancel_requested OR array::any(job.ancestors.cancel_requested)) \
             AS cancelled FROM ONLY $t",
        )
        .bind(("t", rid("task", &claimed.id)))
        .await?
        .check()?;
    #[derive(SurrealValue)]
    struct Row {
        state: String,
        cancelled: bool,
    }
    let row: Option<Row> = res.take(0)?;
    Ok(row.is_some_and(|r| r.state == "running" && !r.cancelled))
}

// ---------------------------------------------------------------------------
// Report finalization (intel_recon/worker.rs:838)
// ---------------------------------------------------------------------------

/// Result of [`finalize_job`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finalize {
    Finalized,
    /// Already terminal; replay is a no-op.
    AlreadyFinal,
    LeaseLost,
    Retired,
}

/// Set a job's terminal state under an optional task lease. The caller
/// computes `state` from its report rows (Intel repository, phase 2).
pub async fn finalize_job(
    db: &Surreal<Any>,
    job_id: &str,
    lease: Option<&ClaimedTask>,
    state: &str,
    now: DateTime<Utc>,
) -> Result<Committed<Finalize>, TxError> {
    let now_s = ts(now);
    run_tx(db, RetryPolicy::default(), |tx| {
        let now_s = now_s.clone();
        async move {
            #[derive(SurrealValue)]
            struct Row {
                root_id: String,
                state: String,
                retired: bool,
            }
            let mut res = run(
                &tx,
                tx.query(
                    "SELECT record::id(root) AS root_id, state, \
                     record::exists(type::record('job_retirement', record::id(root))) AS retired \
                     FROM ONLY $j",
                )
                .bind(("j", rid("job", job_id))),
            )
            .await?;
            let job: Option<Row> = take(&mut res, 0)?;
            let job = job.ok_or_else(|| TxError::Abort(format!("job {job_id} missing")))?;
            bump(&tx, "job_guard", &job.root_id).await?;
            if TERMINAL.contains(&job.state.as_str()) {
                return Ok(Finalize::AlreadyFinal);
            }
            if let Some(claimed) = lease {
                match fence(&tx, claimed, &now_s).await? {
                    Some(f) if holds(&f, claimed) => {}
                    _ => return Ok(Finalize::LeaseLost),
                }
            }
            if job.retired {
                return Ok(Finalize::Retired);
            }
            run(
                &tx,
                tx.query(
                    "UPDATE $j SET state = $state, finished_at = <datetime>$now, \
                     updated_at = <datetime>$now WHERE state NOTINSIDE $terminal RETURN NONE",
                )
                .bind(("j", rid("job", job_id)))
                .bind(("state", state.to_string()))
                .bind(("now", now_s.clone()))
                .bind((
                    "terminal",
                    TERMINAL.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                )),
            )
            .await?;
            rollup_in(&tx, job_id).await?;
            Ok(Finalize::Finalized)
        }
    })
    .await
}

// ---------------------------------------------------------------------------
// Brief chains (intel_recon/brief_pipeline.rs:102 ensure_chain)
// ---------------------------------------------------------------------------

/// One stage of a brief chain; `deps` index earlier stages.
#[derive(Debug, Clone)]
pub struct StageSpec<'a> {
    pub operation: &'a str,
    pub deps: Vec<usize>,
}

/// The chain `ensure_chain` returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chain {
    pub job_id: String,
    pub generation: i64,
    pub created: bool,
}

/// Attach to the article's current chain when its inputs are unchanged,
/// otherwise supersede it and create the next generation. Generation numbers
/// come from `brief_chain_guard:[article]` (no `MAX + 1`), and
/// `brief_run_generation` is unique as a backstop. Replay is a no-op attach.
pub async fn ensure_chain(
    db: &Surreal<Any>,
    article_id: &str,
    input_revision: &str,
    stages: &[StageSpec<'_>],
    now: DateTime<Utc>,
) -> Result<Committed<Chain>, TxError> {
    let now_s = ts(now);
    run_tx(db, RetryPolicy::default(), |tx| {
        let now_s = now_s.clone();
        async move {
            let guard = rid("brief_chain_guard", article_id);
            let mut res = run(
                &tx,
                tx.query(
                    "UPSERT $g SET rev = (rev ?? 0) + 1, generation = generation ?? 0 RETURN NONE;
                     SELECT VALUE generation FROM ONLY $g;
                     SELECT record::id(job) AS job_id, input_revision, state, generation \
                     FROM brief_run WHERE article_id = $a ORDER BY generation DESC LIMIT 1;",
                )
                .bind(("g", guard.clone()))
                .bind(("a", article_id.to_string())),
            )
            .await?;
            #[derive(SurrealValue)]
            struct Run {
                job_id: String,
                input_revision: String,
                state: String,
                generation: i64,
            }
            let counter: Option<i64> = take(&mut res, 1)?;
            let latest: Vec<Run> = take(&mut res, 2)?;
            if let Some(run_row) = latest.first() {
                if run_row.input_revision == input_revision && run_row.state != "superseded" {
                    return Ok(Chain {
                        job_id: run_row.job_id.clone(),
                        generation: run_row.generation,
                        created: false,
                    });
                }
                if run_row.state == "active" {
                    bump(&tx, "job_guard", &run_row.job_id).await?;
                    run(
                        &tx,
                        tx.query(
                            "UPDATE $j SET cancel_requested = true, state = 'superseded', \
                             finished_at = <datetime>$now, updated_at = <datetime>$now RETURN NONE;
                             UPDATE task SET state = 'cancelled', lease_epoch += 1, \
                             lease_owner = '', process_owner = '', lease_until = NONE, finished_at = <datetime>$now, \
                             updated_at = <datetime>$now WHERE job = $j \
                             AND state IN ['queued', 'running'] RETURN NONE;
                             UPDATE brief_run SET state = 'superseded', updated_at = <datetime>$now \
                             WHERE job = $j RETURN NONE;",
                        )
                        .bind(("j", rid("job", &run_row.job_id)))
                        .bind(("now", now_s.clone())),
                    )
                    .await?;
                }
            }
            let generation = counter.unwrap_or(0) + 1;
            let gen_s = generation.to_string();
            let job_id = codec::encode(&["brief", article_id, &gen_s]);
            create_job_in(
                &tx,
                &NewJob {
                    id: &job_id,
                    kind: "task",
                    operation: POOL_INTEL_BRIEF,
                    parent: None,
                    created_at: now,
                },
            )
            .await?;
            let ids: Vec<String> = stages
                .iter()
                .map(|s| codec::encode(&[&job_id, s.operation]))
                .collect();
            for (i, stage) in stages.iter().enumerate() {
                create_task_in(
                    &tx,
                    &NewTask {
                        id: &ids[i],
                        job: &job_id,
                        pool: POOL_INTEL_BRIEF,
                        operation: stage.operation,
                        priority: i as i64,
                        deps: stage.deps.iter().map(|d| ids[*d].as_str()).collect(),
                        input_hash: input_revision,
                        created_at: now,
                    },
                )
                .await?;
            }
            run(
                &tx,
                tx.query(
                    "CREATE $r SET article_id = $a, generation = $gen, job = $j, \
                     input_revision = $rev, state = 'active', updated_at = <datetime>$now \
                     RETURN NONE;
                     UPDATE $g SET generation = $gen RETURN NONE;",
                )
                .bind(("r", rid("brief_run", &codec::encode(&[article_id, &gen_s]))))
                .bind(("a", article_id.to_string()))
                .bind(("gen", generation))
                .bind(("j", rid("job", &job_id)))
                .bind(("rev", input_revision.to_string()))
                .bind(("now", now_s.clone()))
                .bind(("g", guard)),
            )
            .await?;
            Ok(Chain {
                job_id,
                generation,
                created: true,
            })
        }
    })
    .await
}

// ---------------------------------------------------------------------------
// Stop (job_cleanup.rs:290 stop_task_job)
// ---------------------------------------------------------------------------

/// Progress of [`stop_job`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StopProgress {
    pub jobs_stopped: usize,
    pub batches: usize,
    pub done: bool,
}

/// Unstopped descendants of `$j`, at most `$n`. Reads `job_ancestor` by its
/// `ancestor` index, so a batch costs the job's descendants, not all jobs.
pub const STOP_DESCENDANTS_SURQL: &str = "SELECT VALUE record::id(job) FROM job_ancestor \
     WHERE ancestor = $j AND job.stopped = false AND job.kind != 'service' LIMIT $n";

/// Stop a job and its descendants. First sets `cancel_requested` on the job
/// (claims and fences stop immediately), then cancels descendants in batches
/// of `batch` jobs per transaction, then the job itself. Resumable and
/// idempotent: calling it again after a crash continues where it stopped.
/// `max_batches` bounds the work of one call (tests use it to crash midway).
pub async fn stop_job(
    db: &Surreal<Any>,
    job_id: &str,
    now: DateTime<Utc>,
    batch: usize,
    max_batches: Option<usize>,
) -> Result<StopProgress, TxError> {
    let now_s = ts(now);
    let root_id = run_tx(db, RetryPolicy::default(), |tx| {
        let now_s = now_s.clone();
        async move {
            let mut res = run(
                &tx,
                tx.query("SELECT VALUE record::id(root) FROM ONLY $j")
                    .bind(("j", rid("job", job_id))),
            )
            .await?;
            let root: Option<String> = take(&mut res, 0)?;
            let root = root.ok_or_else(|| TxError::Abort(format!("job {job_id} missing")))?;
            bump(&tx, "job_guard", &root).await?;
            run(
                &tx,
                tx.query(
                    "UPDATE $j SET cancel_requested = true, phase = 'Stopped by user', \
                     updated_at = <datetime>$now WHERE kind != 'service' RETURN NONE",
                )
                .bind(("j", rid("job", job_id)))
                .bind(("now", now_s)),
            )
            .await?;
            Ok(root)
        }
    })
    .await?
    .value;
    let mut progress = StopProgress::default();
    loop {
        if max_batches.is_some_and(|m| progress.batches >= m) {
            return Ok(progress);
        }
        let stopped = run_tx(db, RetryPolicy::default(), |tx| {
            let (now_s, root_id) = (now_s.clone(), root_id.clone());
            async move {
                bump(&tx, "job_guard", &root_id).await?;
                let mut res = run(
                    &tx,
                    tx.query(STOP_DESCENDANTS_SURQL)
                        .bind(("j", rid("job", job_id)))
                        .bind(("n", batch as i64)),
                )
                .await?;
                let mut ids: Vec<String> = take(&mut res, 0)?;
                let last = ids.is_empty();
                if last {
                    ids.push(job_id.to_string());
                }
                for id in &ids {
                    cancel_one_in(&tx, id, &now_s).await?;
                }
                Ok((ids.len(), last))
            }
        })
        .await?
        .value;
        progress.batches += 1;
        progress.jobs_stopped += stopped.0;
        if stopped.1 {
            progress.done = true;
            return Ok(progress);
        }
    }
}

// Attempts are closed by selecting their ids and then updating those records:
// in SurrealDB 3.3.4 an `UPDATE attempt ... WHERE task ...` inside an
// interactive transaction matched no rows that the same `SELECT` found.
async fn cancel_one_in(tx: &Tx, job_id: &str, now: &str) -> Result<(), TxError> {
    run(
        tx,
        tx.query(
            "LET $tasks = SELECT VALUE id FROM task WHERE job = $j;
             LET $open = SELECT VALUE id FROM attempt WHERE task INSIDE $tasks AND finished_at = NONE;
             UPDATE $open SET finished_at = <datetime>$now, duration_ms = duration::millis(<datetime>$now - started_at), outcome = 'cancelled' RETURN NONE;
             UPDATE task SET state = 'cancelled', lease_epoch += 1, lease_owner = '', process_owner = '', \
             lease_until = NONE, finished_at = <datetime>$now, updated_at = <datetime>$now \
             WHERE job = $j AND state IN ['queued', 'running'] RETURN NONE;
             UPDATE $j SET state = 'cancelled', finished_at = <datetime>$now \
             WHERE state NOTINSIDE $terminal RETURN NONE;
             UPDATE $j SET cancel_requested = true, stopped = true, phase = 'Stopped by user', \
             updated_at = <datetime>$now RETURN NONE;
             UPDATE brief_run SET state = 'cancelled', updated_at = <datetime>$now \
             WHERE job = $j AND state = 'active' RETURN NONE;",
        )
        .bind(("j", rid("job", job_id)))
        .bind(("now", now.to_string()))
        .bind((
            "terminal",
            TERMINAL.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        )),
    )
    .await?;
    rollup_in(tx, job_id).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Retention and history (job_cleanup.rs:113 cleanup_batch, :429
// clear_jobs_by_status)
// ---------------------------------------------------------------------------

/// Rows removed by one cleanup call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CleanupCounts {
    pub jobs: usize,
}

async fn tree_busy(tx: &Tx, root: &str) -> Result<bool, TxError> {
    let mut res = run(
        tx,
        tx.query(
            "RETURN count(SELECT id FROM task WHERE root = $r AND state = 'running') \
             + count(SELECT id FROM job WHERE root = $r AND kind = 'service')",
        )
        .bind(("r", rid("job", root))),
    )
    .await?;
    let n: Option<i64> = take(&mut res, 0)?;
    Ok(n.unwrap_or(0) > 0)
}

async fn purge_tree_in(tx: &Tx, root: &str, reason: &str, now: &str) -> Result<(), TxError> {
    bump(tx, "job_guard", root).await?;
    run(
        tx,
        tx.query(
            "DELETE attempt WHERE root = $r;
             DELETE stage_output WHERE root = $r;
             DELETE neighborhood_brief WHERE task.root = $r;
             DELETE task WHERE root = $r;
             DELETE brief_run WHERE job.root = $r;
             DELETE job WHERE root = $r;
             DELETE job_ancestor WHERE root = $r;
             UPSERT $m SET retired_at = <datetime>$now, reason = $reason RETURN NONE;",
        )
        .bind(("r", rid("job", root)))
        .bind(("m", rid("job_retirement", root)))
        .bind(("reason", reason.to_string()))
        .bind(("now", now.to_string())),
    )
    .await?;
    Ok(())
}

/// One bounded retention batch: delete root job trees older than the 72-hour
/// execution window (from immutable `created_at`, any status) that have no
/// running task, leaving a retirement marker. Idempotent.
pub async fn cleanup_batch(
    db: &Surreal<Any>,
    now: DateTime<Utc>,
) -> Result<Committed<CleanupCounts>, TxError> {
    let cutoff = ts(now - ChronoDuration::hours(EXECUTION_WINDOW_HOURS));
    let now_s = ts(now);
    run_tx(db, RetryPolicy::default(), |tx| {
        let (cutoff, now_s) = (cutoff.clone(), now_s.clone());
        async move {
            bump(&tx, "cleanup_guard", "singleton").await?;
            let mut res = run(
                &tx,
                tx.query(
                    "SELECT VALUE record::id(id) FROM job WHERE parent = NONE \
                     AND kind != 'service' AND created_at < <datetime>$cutoff \
                     ORDER BY created_at LIMIT $n",
                )
                .bind(("cutoff", cutoff))
                .bind(("n", SWEEP_BATCH)),
            )
            .await?;
            let roots: Vec<String> = take(&mut res, 0)?;
            let mut counts = CleanupCounts::default();
            for root in roots {
                if tree_busy(&tx, &root).await? {
                    continue;
                }
                purge_tree_in(&tx, &root, "retention", &now_s).await?;
                counts.jobs += 1;
            }
            Ok(counts)
        }
    })
    .await
}

/// Delete terminal root job trees in `states` (Completed, Stopped, Failed
/// history), batch by batch, skipping trees with running work or services.
pub async fn clear_jobs_by_status(
    db: &Surreal<Any>,
    states: &[&str],
    now: DateTime<Utc>,
) -> Result<CleanupCounts, TxError> {
    for s in states {
        if !TERMINAL.contains(s) {
            return Err(TxError::Abort(format!("{s} is not a terminal job state")));
        }
    }
    let states: Vec<String> = states.iter().map(|s| s.to_string()).collect();
    let now_s = ts(now);
    let mut total = CleanupCounts::default();
    let mut skip = 0i64;
    loop {
        let (deleted, skipped, seen) = run_tx(db, RetryPolicy::default(), |tx| {
            let (states, now_s) = (states.clone(), now_s.clone());
            async move {
                bump(&tx, "cleanup_guard", "singleton").await?;
                let mut res = run(
                    &tx,
                    tx.query(
                        "SELECT VALUE record::id(id) FROM job WHERE parent = NONE \
                         AND kind != 'service' AND state IN $states \
                         ORDER BY created_at, id LIMIT $n START $skip",
                    )
                    .bind(("states", states))
                    .bind(("n", SWEEP_BATCH))
                    .bind(("skip", skip)),
                )
                .await?;
                let roots: Vec<String> = take(&mut res, 0)?;
                let (mut deleted, mut skipped) = (0usize, 0i64);
                for root in &roots {
                    if tree_busy(&tx, root).await? {
                        skipped += 1;
                        continue;
                    }
                    purge_tree_in(&tx, root, "cleared", &now_s).await?;
                    deleted += 1;
                }
                Ok((deleted, skipped, roots.len()))
            }
        })
        .await?
        .value;
        total.jobs += deleted;
        skip += skipped;
        if seen == 0 {
            return Ok(total);
        }
    }
}
