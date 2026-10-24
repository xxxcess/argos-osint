//! Process registry (replaces `argos_processes`, job_registry.rs).
//!
//! Each process beats every [`BEAT_INTERVAL`] into `process:[owner]`, where
//! `owner` is `scheduler::process_owner()`. A claim stamps that same id into
//! `task.process_owner`; `task.lease_owner` names the worker (w0..w7) and is
//! never compared with process ids. A process whose last beat is older than
//! [`PROCESS_STALE_SECS`] is gone, and the running tasks it stamped are
//! recovered with a new lease epoch, which fences any
//! late write from the old owner (storage-audit.md:174).
//!
//! Service unavailability is not a dead owner. A beat that cannot reach the
//! database returns an error and writes nothing, so no process is ever
//! judged by a beat it failed to write while the database was down. And a
//! recovering process must itself have a fresh beat: if its own beat is
//! stale, the gap is more likely a database outage than every other owner
//! dying, so it recovers nothing until it has beaten again.

use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use super::jobs::{self, rid, run, take, ts};
use super::tx::{run_tx, Committed, RetryPolicy, TxError};

/// Running tasks whose owner process is not fresh. Reads the task table
/// through the `state` prefix of `task_claim_order` (the plan 3.3.4 picks;
/// the test pins an IndexScan), so it costs the running set
/// (bounded by workers x processes), not all tasks. The `process` scan above
/// it is a full read of a table with one row per live process.
pub const ORPHANS_SURQL: &str = "SELECT record::id(id) AS id, record::id(root) AS root_id, \
     lease_epoch, recoveries, process_owner FROM task \
     WHERE state = 'running' AND process_owner != '' \
     AND process_owner NOT IN $fresh LIMIT $n";

/// Seconds after the last beat at which a process counts as gone.
pub const PROCESS_STALE_SECS: i64 = 30;
/// Heartbeat interval.
pub const BEAT_INTERVAL: Duration = Duration::from_secs(5);
/// Tasks recovered per transaction.
pub const RECOVER_BATCH: i64 = 64;

/// Registry schema (idempotent).
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS process SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS pid ON process TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS started_at ON process TYPE datetime READONLY;
DEFINE FIELD IF NOT EXISTS heartbeat_at ON process TYPE datetime;
DEFINE INDEX IF NOT EXISTS process_heartbeat ON process FIELDS heartbeat_at;";

/// Apply the registry schema (idempotent).
pub async fn apply_schema(db: &Surreal<Any>) -> Result<(), TxError> {
    db.query(SCHEMA).await?.check()?;
    Ok(())
}

/// Record a beat for `owner`. The first beat sets `started_at`. Idempotent:
/// replaying a beat only moves `heartbeat_at` forward, never back.
pub async fn beat(
    db: &Surreal<Any>,
    owner: &str,
    pid: u32,
    now: DateTime<Utc>,
) -> Result<Committed<()>, TxError> {
    run_tx(db, RetryPolicy::default(), |tx| async move {
        run(
            &tx,
            tx.query(
                "UPSERT $p SET pid = $pid, started_at = started_at ?? <datetime>$now, \
                 heartbeat_at = IF heartbeat_at > <datetime>$now THEN heartbeat_at \
                 ELSE <datetime>$now END RETURN NONE",
            )
            .bind(("p", rid("process", owner)))
            .bind(("pid", i64::from(pid)))
            .bind(("now", ts(now))),
        )
        .await?;
        Ok(())
    })
    .await
}

/// Outcome of a recovery pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Recovery {
    /// Running tasks of gone owners requeued (or failed) with a new epoch.
    pub tasks: usize,
    /// Stale process rows removed.
    pub processes: usize,
    /// The caller's own beat was stale, so nothing was recovered.
    pub skipped_self_stale: bool,
}

#[derive(Debug, SurrealValue)]
struct Orphan {
    id: String,
    root_id: String,
    lease_epoch: i64,
    recoveries: i64,
    process_owner: String,
}

/// Recover running tasks owned by gone processes (stale beat, or no row),
/// other than `me`, then drop stale process rows. Run it again until
/// `tasks` is below [`RECOVER_BATCH`].
pub async fn recover_orphans(
    db: &Surreal<Any>,
    me: &str,
    now: DateTime<Utc>,
) -> Result<Committed<Recovery>, TxError> {
    let now_s = ts(now);
    let stale_s = ts(now - ChronoDuration::seconds(PROCESS_STALE_SECS));
    run_tx(db, RetryPolicy::default(), |tx| {
        let (now_s, stale_s) = (now_s.clone(), stale_s.clone());
        async move {
            let mut res = run(
                &tx,
                tx.query(
                    "SELECT VALUE record::id(id) FROM process \
                     WHERE heartbeat_at >= <datetime>$stale",
                )
                .bind(("stale", stale_s.clone())),
            )
            .await?;
            let fresh: Vec<String> = take(&mut res, 0)?;
            if !fresh.iter().any(|o| o == me) {
                return Ok(Recovery {
                    skipped_self_stale: true,
                    ..Recovery::default()
                });
            }
            // Shares the reaper's guard: both fence running tasks.
            jobs::bump(&tx, "lease_sweep_guard", "singleton").await?;
            let mut res = run(
                &tx,
                tx.query(ORPHANS_SURQL)
                    .bind(("fresh", fresh.clone()))
                    .bind(("n", RECOVER_BATCH)),
            )
            .await?;
            let orphans: Vec<Orphan> = take(&mut res, 0)?;
            for o in &orphans {
                debug_assert!(o.process_owner != me);
                jobs::requeue_running_in(
                    &tx,
                    &o.id,
                    &o.root_id,
                    o.lease_epoch,
                    o.recoveries,
                    "owner_gone",
                    &now_s,
                    false,
                )
                .await?;
            }
            let mut res = run(
                &tx,
                tx.query(
                    "LET $gone = SELECT VALUE id FROM process \
                     WHERE heartbeat_at < <datetime>$stale AND record::id(id) != $me;
                     DELETE $gone RETURN NONE;
                     RETURN count($gone);",
                )
                .bind(("stale", stale_s))
                .bind(("me", me.to_string())),
            )
            .await?;
            let processes: Option<i64> = take(&mut res, 2)?;
            Ok(Recovery {
                tasks: orphans.len(),
                processes: processes.unwrap_or(0) as usize,
                skipped_self_stale: false,
            })
        }
    })
    .await
}
