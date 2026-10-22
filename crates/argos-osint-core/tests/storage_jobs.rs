//! Job-system repository tests (migration phase 1) against a real SurrealDB
//! 3.3.4 disk server or the embedded RocksDB engine (see `support`).
//! Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use std::collections::HashSet;

use argos_osint_core::storage::jobs::{self, ClaimedTask, NewJob, NewTask, Publish, StageSpec};
use argos_osint_core::storage::tx::{run_tx, RetryPolicy, TxError};
use argos_osint_core::storage::{outbox, processes};
use chrono::{DateTime, Duration, Utc};
use support::*;
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

const LEASE: i64 = 30;
/// Per pool, one claim in eight (jobs::HEAD_EVERY) starts at the head.
const HEAD_BOUND: usize = 8;

async fn fresh(server: &Server, name: &str) -> Surreal<Any> {
    let db = server.root("argos", name).await;
    jobs::apply_schema(&db).await.expect("schema");
    db
}

fn job<'a>(id: &'a str, parent: Option<&'a str>, at: DateTime<Utc>) -> NewJob<'a> {
    NewJob {
        id,
        kind: "task",
        operation: "test",
        parent,
        created_at: at,
    }
}

fn task<'a>(
    id: &'a str,
    job: &'a str,
    pool: &'a str,
    priority: i64,
    at: DateTime<Utc>,
) -> NewTask<'a> {
    NewTask {
        id,
        job,
        pool,
        operation: "op",
        priority,
        deps: vec![],
        input_hash: "h",
        created_at: at,
    }
}

async fn scalar<T: SurrealValue>(db: &Surreal<Any>, sql: &str) -> T {
    let mut r = db.query(sql).await.unwrap().check().unwrap();
    r.take::<Option<T>>(0usize)
        .unwrap()
        .expect("scalar query returned NONE")
}

async fn claim(
    db: &Surreal<Any>,
    pool: &str,
    owner: &str,
    now: DateTime<Utc>,
) -> Option<ClaimedTask> {
    let key = format!("claim:{owner}:{}", nanos());
    jobs::claim_next(db, pool, owner, LEASE, now, &key)
        .await
        .expect("claim")
        .value
}

// --- index check (replaces EXPLAIN QUERY PLAN in tasks.rs) -----------------

#[tokio::test]
async fn claim_scan_uses_task_claim_order_index() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "index").await;
    let info: String = scalar(&db, "RETURN <string>(INFO FOR TABLE task).indexes").await;
    assert!(info.contains("task_claim_order"), "{info}");
    assert!(
        info.contains("FIELDS state, pool, priority, created_at"),
        "index fields drifted: {info}"
    );
    let now = Utc::now();
    let mut tasks = Vec::new();
    let ids: Vec<String> = (0..1000).map(|i| format!("t{i:04}")).collect();
    for (i, id) in ids.iter().enumerate() {
        let pool = if i % 4 == 0 { "intel_brief" } else { "other" };
        tasks.push(task(
            id,
            "j",
            pool,
            (i % 7) as i64,
            now - Duration::seconds(i as i64),
        ));
    }
    jobs::create_job(&db, &job("j", None, now), &tasks)
        .await
        .unwrap();
    let plan: String = {
        let mut r = db
            .query(format!("{} EXPLAIN FULL", jobs::CLAIM_CANDIDATES_SURQL))
            .bind(("pool", "intel_brief"))
            .bind(("now", now.to_rfc3339()))
            .bind(("fresh_after", (now - Duration::hours(72)).to_rfc3339()))
            .bind(("k", 8i64))
            .bind(("start", 0i64))
            .await
            .unwrap()
            .check()
            .unwrap();
        let v: surrealdb::types::Value = r.take(0).unwrap();
        format!("{v:?}")
    };
    eprintln!("claim plan: {plan}");
    // Relies on SurrealDB 3.3.4 EXPLAIN FULL output naming the iterator and
    // index; recheck when the pin moves.
    assert!(
        plan.contains("task_claim_order"),
        "no index iterator: {plan}"
    );
    assert!(
        plan.contains(r#""operator": String("IndexScan")"#),
        "no index iterator: {plan}"
    );
    assert!(!plan.contains("TableScan"), "table scan: {plan}");
    // Order is asserted on results, not on the plan (approved fallback).
    let mut r = db
        .query(jobs::CLAIM_CANDIDATES_SURQL.replace("LIMIT $k START $start", "LIMIT 300"))
        .bind(("pool", "intel_brief"))
        .bind(("now", now.to_rfc3339()))
        .bind(("fresh_after", (now - Duration::hours(72)).to_rfc3339()))
        .await
        .unwrap()
        .check()
        .unwrap();
    #[derive(SurrealValue)]
    struct Row {
        id: String,
        priority: i64,
        created_at: surrealdb::types::Datetime,
    }
    let rows: Vec<Row> = r.take(0).unwrap();
    assert_eq!(rows.len(), 250);
    for w in rows.windows(2) {
        let a = (w[0].priority, w[0].created_at);
        let b = (w[1].priority, w[1].created_at);
        assert!(a <= b, "{} before {} out of order", w[0].id, w[1].id);
    }
}

// --- run_tx contract --------------------------------------------------------

#[tokio::test]
async fn run_tx_retries_conflicts_then_reports_contention() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "txcontract").await;
    let other = server.root("argos", "txcontract").await;
    // Interfere on the first attempt only: one retry, then commit.
    let mut calls = 0;
    let out = run_tx(&db, RetryPolicy::default(), |tx| {
        calls += 1;
        let interfere = calls == 1;
        let other = other.clone();
        async move {
            tx.query("UPSERT g:one SET rev = (rev ?? 0) + 1 RETURN NONE")
                .await?
                .check()?;
            if interfere {
                other
                    .query("UPSERT g:one SET rev = (rev ?? 0) + 100 RETURN NONE")
                    .await?
                    .check()?;
            }
            Ok(())
        }
    })
    .await
    .expect("commits after a retry");
    assert_eq!(out.retries, 1);
    assert_eq!(
        scalar::<Option<i64>>(&db, "SELECT VALUE rev FROM ONLY g:one").await,
        Some(101)
    );
    // Interfere every time: bounded, typed contention.
    let policy = RetryPolicy {
        max_attempts: 3,
        ..RetryPolicy::default()
    };
    let err = run_tx(&db, policy, |tx| {
        let other = other.clone();
        async move {
            tx.query("UPSERT g:one SET rev = (rev ?? 0) + 1 RETURN NONE")
                .await?
                .check()?;
            other
                .query("UPSERT g:one SET rev = (rev ?? 0) + 100 RETURN NONE")
                .await?
                .check()?;
            Ok(())
        }
    })
    .await
    .expect_err("never commits");
    assert!(matches!(err, TxError::Contention { attempts: 3 }), "{err}");
    // Abort writes nothing.
    let err = run_tx(&db, RetryPolicy::default(), |tx| async move {
        tx.query("CREATE g:aborted").await?.check()?;
        Err::<(), _>(TxError::Abort("validation failed".into()))
    })
    .await
    .expect_err("aborts");
    assert!(matches!(err, TxError::Abort(_)));
    assert_eq!(count(&db, "g").await, 1);
}

// --- claims ----------------------------------------------------------------

fn pct(sorted: &[u32], p: f64) -> u32 {
    sorted[((sorted.len() as f64 - 1.0) * p).round() as usize]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn eight_clients_never_double_claim_and_report_retry_stats() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "race").await;
    let now = Utc::now();
    let ids: Vec<String> = (0..200).map(|i| format!("r{i:03}")).collect();
    let tasks: Vec<NewTask> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| task(id, "race", "p", (i % 5) as i64, now))
        .collect();
    jobs::create_job(&db, &job("race", None, now), &tasks)
        .await
        .unwrap();
    let mut handles = Vec::new();
    for c in 0..8 {
        let client = server.root("argos", "race").await;
        handles.push(tokio::spawn(async move {
            let owner = format!("w{c}");
            let mut got = Vec::new();
            let mut exhausted = 0u32;
            for n in 0.. {
                let key = format!("claim:{owner}:{n}");
                match jobs::claim_next(&client, "p", &owner, LEASE, now, &key).await {
                    Ok(c) => match c.value {
                        Some(t) => got.push((t.id, t.lease_epoch, c.retries)),
                        None => break,
                    },
                    // Recorded, then retried with a fresh key like a worker
                    // would on its next poll.
                    Err(TxError::Contention { .. }) => exhausted += 1,
                    Err(e) => panic!("claim failed: {e}"),
                }
            }
            (got, exhausted)
        }));
    }
    let mut all = Vec::new();
    let mut exhausted = 0;
    for h in handles {
        let (got, e) = h.await.unwrap();
        all.extend(got);
        exhausted += e;
    }
    let unique: HashSet<_> = all.iter().map(|(id, _, _)| id.clone()).collect();
    assert_eq!(all.len(), 200, "every task claimed once");
    assert_eq!(unique.len(), 200, "no task claimed twice");
    assert!(all.iter().all(|(_, epoch, _)| *epoch == 1));
    let mut retries: Vec<u32> = all.iter().map(|(_, _, r)| *r).collect();
    retries.sort_unstable();
    eprintln!(
        "RETRY-STATS mode={} claims={} p50={} p99={} max={} exhausted={exhausted}",
        if server.embedded {
            "embedded"
        } else {
            "server"
        },
        retries.len(),
        pct(&retries, 0.5),
        pct(&retries, 0.99),
        retries.last().unwrap()
    );
    assert_eq!(count(&db, "attempt").await, 200);
    // Review decision 2: split the pool guard if p99 > 3 or any claim
    // exhausts its attempts.
    assert!(
        pct(&retries, 0.99) <= 3 && exhausted == 0,
        "pool guard too hot: p99={} exhausted={exhausted}",
        pct(&retries, 0.99)
    );
}

#[tokio::test]
async fn claim_receipt_resolves_an_unknown_commit_and_eligibility_holds() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "claims").await;
    let now = Utc::now();
    let mut dep = task("b", "j", "p", 0, now);
    dep.deps = vec!["a"];
    jobs::create_job(
        &db,
        &job("j", None, now),
        &[dep, task("a", "j", "p", 1, now)],
    )
    .await
    .unwrap();
    let first = jobs::claim_next(&db, "p", "w", LEASE, now, "op1")
        .await
        .unwrap()
        .value
        .unwrap();
    // `b` is first in priority but waits for `a`: no head-of-queue starvation.
    assert_eq!(first.id, "a");
    // Same key again (lost response): same claim, no second lease.
    let replay = jobs::claim_next(&db, "p", "w", LEASE, now, "op1")
        .await
        .unwrap()
        .value
        .unwrap();
    assert_eq!(replay, first);
    assert!(claim(&db, "p", "w", now).await.is_none(), "b still blocked");
    // Old jobs (outside 72 h) and cancelled jobs are not claimable.
    let old = now - Duration::hours(80);
    jobs::create_job(
        &db,
        &job("old", None, old),
        &[task("o", "old", "q", 0, old)],
    )
    .await
    .unwrap();
    assert!(claim(&db, "q", "w", now).await.is_none());
}

#[tokio::test]
async fn intel_brief_dependent_stage_runs_alone_in_its_job() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "brief_excl").await;
    let now = Utc::now();
    let chain = jobs::ensure_chain(
        &db,
        "art",
        "rev1",
        &[
            StageSpec {
                operation: "r1",
                deps: vec![],
            },
            StageSpec {
                operation: "r2",
                deps: vec![],
            },
            StageSpec {
                operation: "d",
                deps: vec![0],
            },
        ],
        now,
    )
    .await
    .unwrap()
    .value;
    let a = claim(&db, "intel_brief", "w", now).await.unwrap();
    let b = claim(&db, "intel_brief", "w", now).await.unwrap();
    assert_ne!(a.id, b.id, "independent roots run together");
    let r1 = if a.operation == "r1" { &a } else { &b };
    let out = jobs::commit_stage(&db, r1, "{}", "h1", now, "stage:r1")
        .await
        .unwrap()
        .value;
    assert_eq!(out, Publish::Committed);
    // `d` depends on r1 (done) but r2 is still running in the same job.
    assert!(claim(&db, "intel_brief", "w", now).await.is_none());
    let r2 = if a.operation == "r2" { &a } else { &b };
    jobs::commit_stage(&db, r2, "{}", "h2", now, "stage:r2")
        .await
        .unwrap();
    let d = claim(&db, "intel_brief", "w", now).await.unwrap();
    assert_eq!(d.operation, "d");
    assert_eq!(d.job_id, chain.job_id);
}

// --- site 1: reaper (tasks.rs:1385) ------------------------------------------

#[tokio::test]
async fn renewal_racing_the_reaper_is_never_reaped_and_replay_is_harmless() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "reaper").await;
    let other = server.root("argos", "reaper").await;
    let now = Utc::now();
    jobs::create_job(&db, &job("j", None, now), &[task("t", "j", "p", 0, now)])
        .await
        .unwrap();
    let c = claim(&db, "p", "w", now).await.unwrap();
    let later = now + Duration::seconds(LEASE + 5);
    // Reaper selects the expired lease; the owner renews before it commits.
    let tx = std::sync::Arc::new(db.clone().begin().await.unwrap());
    let reaped = jobs::interrupt_expired_leases_in(&tx, later).await;
    let renewed = jobs::renew_lease(&other, &c, LEASE * 3, later)
        .await
        .unwrap()
        .value;
    let tx = std::sync::Arc::try_unwrap(tx).unwrap();
    let commit = match reaped {
        Ok(_) => tx.commit().await.map_err(TxError::from),
        Err(e) => Err(e),
    };
    if renewed {
        assert!(
            commit.as_ref().err().is_some_and(|e| e.is_conflict()),
            "stale reaper committed: {commit:?}"
        );
    }
    // Retried reaper sees the renewal; replay also recovers nothing.
    for _ in 0..2 {
        assert_eq!(
            jobs::interrupt_expired_leases(&db, later)
                .await
                .unwrap()
                .value,
            0
        );
    }
    assert!(
        jobs::renew_lease(&db, &c, LEASE, later)
            .await
            .unwrap()
            .value
    );
    // Genuinely expired: recovered once, epoch advanced, attempt closed.
    let much_later = later + Duration::seconds(LEASE * 4);
    assert_eq!(
        jobs::interrupt_expired_leases(&db, much_later)
            .await
            .unwrap()
            .value,
        1
    );
    assert_eq!(
        jobs::interrupt_expired_leases(&db, much_later)
            .await
            .unwrap()
            .value,
        0
    );
    assert!(
        !jobs::renew_lease(&db, &c, LEASE, much_later)
            .await
            .unwrap()
            .value,
        "old epoch fenced"
    );
    let outcome: Option<String> = scalar(
        &db,
        "SELECT VALUE outcome FROM ONLY attempt WHERE task = task:t LIMIT 1",
    )
    .await;
    assert_eq!(outcome.as_deref(), Some("lease_expired"));
    // Fails after MAX_WORKER_RECOVERIES infrastructure recoveries.
    let mut t = much_later;
    for _ in 1..jobs::MAX_WORKER_RECOVERIES {
        let c = claim(&db, "p", "w", t).await.expect("requeued");
        assert!(c.lease_epoch > 1);
        t += Duration::seconds(LEASE * 2);
        jobs::interrupt_expired_leases(&db, t).await.unwrap();
    }
    let state: String = scalar(&db, "SELECT VALUE state FROM ONLY task:t").await;
    assert_eq!(state, "failed");
}

// --- sites 2 and 4: cleanup (job_cleanup.rs:113, :429) -----------------------

#[tokio::test]
async fn cleanup_never_removes_running_work_and_replay_is_harmless() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "cleanup").await;
    let now = Utc::now();
    let old = now - Duration::hours(80);
    jobs::create_job(
        &db,
        &job("idle", None, old),
        &[task("i", "idle", "p", 0, old)],
    )
    .await
    .unwrap();
    jobs::create_job(
        &db,
        &job("busy", None, old),
        &[task("b", "busy", "q", 0, old)],
    )
    .await
    .unwrap();
    jobs::create_job(&db, &job("child", Some("busy"), old), &[])
        .await
        .unwrap();
    // Claim `b` with a clock inside its window so it is running.
    let c = claim(&db, "q", "w", old + Duration::hours(1))
        .await
        .unwrap();
    assert_eq!(c.id, "b");
    let first = jobs::cleanup_batch(&db, now).await.unwrap().value;
    assert_eq!(first.jobs, 1);
    let replay = jobs::cleanup_batch(&db, now).await.unwrap().value;
    assert_eq!(replay.jobs, 0);
    assert_eq!(count(&db, "job").await, 2, "busy tree kept");
    assert_eq!(count(&db, "job_retirement").await, 1);
    assert!(
        claim(&db, "p", "w", now).await.is_none(),
        "retired job never claimable"
    );
    // Concurrent cleanup and claims: no running task ever loses its job.
    for round in 0..5 {
        let jid = format!("race{round}");
        let at = now - Duration::hours(71);
        jobs::create_job(
            &db,
            &job(&jid, None, at),
            &[task(&format!("{jid}t"), &jid, "z", 0, at)],
        )
        .await
        .unwrap();
        let later = now + Duration::hours(2);
        let key = format!("k{round}");
        let (a, b) = tokio::join!(
            jobs::cleanup_batch(&db, later),
            jobs::claim_next(&db, "z", "w", LEASE, at, &key)
        );
        a.unwrap();
        if let Some(t) = b.unwrap().value {
            let orphan: i64 = scalar(
                &db,
                &format!(
                    "RETURN count(SELECT id FROM task WHERE id = task:`{}` AND job.id = NONE)",
                    t.id
                ),
            )
            .await;
            assert_eq!(orphan, 0, "claimed task's job was deleted");
        }
    }
}

#[tokio::test]
async fn clear_history_skips_running_trees_and_replays_to_zero() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "clear").await;
    let now = Utc::now();
    for i in 0..150 {
        let id = format!("done{i:03}");
        jobs::create_job(&db, &job(&id, None, now), &[])
            .await
            .unwrap();
        jobs::finalize_job(&db, &id, None, "completed", now)
            .await
            .unwrap();
    }
    jobs::create_job(
        &db,
        &job("live", None, now),
        &[task("l", "live", "p", 0, now)],
    )
    .await
    .unwrap();
    jobs::finalize_job(&db, "live", None, "completed", now)
        .await
        .unwrap();
    claim(&db, "p", "w", now).await; // terminal job: not claimable, stays queued
    db.query("UPDATE task:l SET state = 'running'")
        .await
        .unwrap()
        .check()
        .unwrap();
    let out = jobs::clear_jobs_by_status(&db, &["completed"], now)
        .await
        .unwrap();
    assert_eq!(out.jobs, 150);
    assert_eq!(
        jobs::clear_jobs_by_status(&db, &["completed"], now)
            .await
            .unwrap()
            .jobs,
        0
    );
    assert_eq!(count(&db, "job").await, 1);
    assert!(jobs::clear_jobs_by_status(&db, &["running"], now)
        .await
        .is_err());
}

// --- site 3: stop (job_cleanup.rs:290) ----------------------------------------

#[tokio::test]
async fn stopping_a_large_tree_resumes_after_a_crash_and_fences_workers() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "stop").await;
    let now = Utc::now();
    jobs::create_job(
        &db,
        &job("root", None, now),
        &[task("rt", "root", "p", 0, now)],
    )
    .await
    .unwrap();
    for i in 0..250 {
        let id = format!("c{i:03}");
        jobs::create_job(
            &db,
            &job(&id, Some("root"), now),
            &[task(&format!("{id}t"), &id, "p", 1, now)],
        )
        .await
        .unwrap();
    }
    let running = claim(&db, "p", "w", now).await.unwrap();
    assert!(jobs::should_continue(&db, &running).await.unwrap());
    // Crash after one batch of ~100.
    let partial = jobs::stop_job(&db, "root", now, jobs::STOP_BATCH, Some(1))
        .await
        .unwrap();
    assert!(!partial.done);
    assert_eq!(partial.jobs_stopped, 100);
    // The root flag alone already stops claims and fences workers.
    assert!(
        claim(&db, "p", "w2", now).await.is_none(),
        "no claims under a cancelled root"
    );
    assert!(!jobs::should_continue(&db, &running).await.unwrap());
    let out = jobs::commit_stage(&db, &running, "{}", "h", now, "stage:x")
        .await
        .unwrap()
        .value;
    assert_ne!(out, Publish::Committed);
    assert_eq!(count(&db, "stage_output").await, 0);
    // Resume, then replay.
    let rest = jobs::stop_job(&db, "root", now, jobs::STOP_BATCH, None)
        .await
        .unwrap();
    assert!(rest.done);
    assert_eq!(partial.jobs_stopped + rest.jobs_stopped, 251);
    let replay = jobs::stop_job(&db, "root", now, jobs::STOP_BATCH, None)
        .await
        .unwrap();
    assert!(replay.done);
    assert_eq!(replay.jobs_stopped, 1, "only the root is revisited");
    let open: i64 = scalar(
        &db,
        "RETURN count(SELECT id FROM task WHERE state IN ['queued','running'])",
    )
    .await;
    assert_eq!(open, 0);
    let unstopped: i64 = scalar(
        &db,
        "RETURN count(SELECT id FROM job WHERE state != 'cancelled')",
    )
    .await;
    assert_eq!(unstopped, 0);
    let unclosed: i64 = scalar(
        &db,
        "RETURN count(SELECT id FROM attempt WHERE finished_at = NONE)",
    )
    .await;
    assert_eq!(unclosed, 0);
}

// --- site 5: finalize (intel_recon/worker.rs:838) -----------------------------

#[tokio::test]
async fn finalize_refuses_a_lost_lease_and_replay_is_a_no_op() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "finalize").await;
    let now = Utc::now();
    jobs::create_job(&db, &job("j", None, now), &[task("t", "j", "p", 0, now)])
        .await
        .unwrap();
    let c = claim(&db, "p", "w", now).await.unwrap();
    let expired = now + Duration::seconds(LEASE * 2);
    jobs::interrupt_expired_leases(&db, expired).await.unwrap();
    let lost = jobs::finalize_job(&db, "j", Some(&c), "completed", expired)
        .await
        .unwrap()
        .value;
    assert_eq!(lost, jobs::Finalize::LeaseLost);
    let c2 = claim(&db, "p", "w", expired).await.unwrap();
    let ok = jobs::finalize_job(&db, "j", Some(&c2), "partial", expired)
        .await
        .unwrap()
        .value;
    assert_eq!(ok, jobs::Finalize::Finalized);
    let again = jobs::finalize_job(&db, "j", Some(&c2), "completed", expired)
        .await
        .unwrap()
        .value;
    assert_eq!(again, jobs::Finalize::AlreadyFinal);
    let state: String = scalar(&db, "SELECT VALUE state FROM ONLY job:j").await;
    assert_eq!(state, "partial");
}

// --- site 6: brief chain (intel_recon/brief_pipeline.rs:102) ------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_ensure_chain_allocates_one_generation() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "chain").await;
    let now = Utc::now();
    let stages = vec![
        StageSpec {
            operation: "a",
            deps: vec![],
        },
        StageSpec {
            operation: "b",
            deps: vec![0],
        },
    ];
    let mut handles = Vec::new();
    for _ in 0..4 {
        let client = server.root("argos", "chain").await;
        let stages = stages.clone();
        let stages: Vec<StageSpec<'static>> = stages
            .into_iter()
            .map(|s| StageSpec {
                operation: if s.operation == "a" { "a" } else { "b" },
                deps: s.deps,
            })
            .collect();
        handles.push(tokio::spawn(async move {
            jobs::ensure_chain(&client, "art", "rev1", &stages, now)
                .await
                .unwrap()
                .value
        }));
    }
    let mut chains = Vec::new();
    for h in handles {
        chains.push(h.await.unwrap());
    }
    assert_eq!(chains.iter().filter(|c| c.created).count(), 1);
    assert!(chains
        .iter()
        .all(|c| c.job_id == chains[0].job_id && c.generation == 1));
    assert_eq!(count(&db, "brief_run").await, 1);
    // Replay attaches; a new revision supersedes and cancels the old tasks.
    let replay = jobs::ensure_chain(&db, "art", "rev1", &stages, now)
        .await
        .unwrap()
        .value;
    assert!(!replay.created);
    let next = jobs::ensure_chain(&db, "art", "rev2", &stages, now)
        .await
        .unwrap()
        .value;
    assert!(next.created && next.generation == 2);
    let old_open: i64 = scalar(&db, &format!(
        "RETURN count(SELECT id FROM task WHERE job = type::record('job', '{}') AND state = 'queued')",
        chains[0].job_id
    )).await;
    assert_eq!(old_open, 0);
}

// --- site 7: stage commit (intel_recon/brief_pipeline.rs:451) -----------------

#[tokio::test]
async fn stage_commit_after_epoch_bump_publishes_nothing_and_receipt_replays() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "stage").await;
    let now = Utc::now();
    jobs::create_job(&db, &job("j", None, now), &[task("t", "j", "p", 0, now)])
        .await
        .unwrap();
    let stale = claim(&db, "p", "w", now).await.unwrap();
    let later = now + Duration::seconds(LEASE * 2);
    jobs::interrupt_expired_leases(&db, later).await.unwrap();
    let fresh_claim = claim(&db, "p", "w2", later).await.unwrap();
    assert_eq!(fresh_claim.lease_epoch, stale.lease_epoch + 2);
    let out = jobs::commit_stage(&db, &stale, "{\"x\":1}", "h", later, "stage:t:1")
        .await
        .unwrap()
        .value;
    assert_eq!(out, Publish::LeaseLost);
    assert_eq!(count(&db, "stage_output").await, 0);
    let key = format!("stage:t:{}", fresh_claim.lease_epoch);
    for _ in 0..2 {
        let out = jobs::commit_stage(&db, &fresh_claim, "{\"x\":2}", "h2", later, &key)
            .await
            .unwrap()
            .value;
        assert_eq!(out, Publish::Committed);
    }
    assert_eq!(count(&db, "stage_output").await, 1);
    let state: String = scalar(&db, "SELECT VALUE state FROM ONLY task:t").await;
    assert_eq!(state, "completed");
}

// --- brain_neighborhood_brief_exec.rs:660 -----------------------------------

#[tokio::test]
async fn neighborhood_publication_is_fenced_and_replay_is_harmless() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "neighborhood").await;
    let now = Utc::now();
    jobs::create_job(&db, &job("j", None, now), &[task("t", "j", "p", 0, now)])
        .await
        .unwrap();
    let c = claim(&db, "p", "w", now).await.unwrap();
    let mut wrong = c.clone();
    wrong.lease_epoch += 1;
    assert_eq!(
        jobs::publish_neighborhood(&db, &wrong, "b", "r", "h", now)
            .await
            .unwrap()
            .value,
        Publish::LeaseLost
    );
    for _ in 0..2 {
        assert_eq!(
            jobs::publish_neighborhood(&db, &c, "b", "r", "h", now)
                .await
                .unwrap()
                .value,
            Publish::Committed
        );
    }
    assert_eq!(count(&db, "neighborhood_brief").await, 1);
}

// --- scheduler.rs:157 try_elect ----------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn exactly_one_scheduler_wins_and_takes_over_after_expiry() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "sched").await;
    let now = Utc::now();
    let mut handles = Vec::new();
    for i in 0..4 {
        let client = server.root("argos", "sched").await;
        handles.push(tokio::spawn(async move {
            jobs::try_elect(&client, &format!("p{i}"), 30, now)
                .await
                .unwrap()
                .value
        }));
    }
    let mut winners = 0;
    for h in handles {
        winners += h.await.unwrap().elected as usize;
    }
    assert_eq!(winners, 1);
    let holder: String = scalar(
        &db,
        "SELECT VALUE owner FROM ONLY scheduler_lease:singleton",
    )
    .await;
    let renew = jobs::try_elect(&db, &holder, 30, now + Duration::seconds(10))
        .await
        .unwrap()
        .value;
    assert!(renew.elected && renew.epoch == 1);
    let other = if holder == "p0" { "p1" } else { "p0" };
    assert!(
        !jobs::try_elect(&db, other, 30, now + Duration::seconds(20))
            .await
            .unwrap()
            .value
            .elected
    );
    let take = jobs::try_elect(&db, other, 30, now + Duration::seconds(45))
        .await
        .unwrap()
        .value;
    assert!(take.elected && take.epoch == 2);
}

// --- claim window: approximately FIFO, never across a priority level ---------

#[tokio::test]
async fn claim_window_never_crosses_a_priority_level() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "prio").await;
    let now = Utc::now();
    // Buckets smaller and larger than the 32-task window, interleaved by age.
    let mut specs = Vec::new();
    for i in 0..60 {
        let priority = match i % 6 {
            0 => 0,
            1 | 2 => 2,
            _ => 1,
        };
        specs.push((
            format!("p{i:03}"),
            priority,
            now - Duration::seconds(600 - i),
        ));
    }
    let tasks: Vec<NewTask> = specs
        .iter()
        .map(|(id, p, at)| task(id, "prio", "p", *p, *at))
        .collect();
    jobs::create_job(&db, &job("prio", None, now), &tasks)
        .await
        .unwrap();
    // Every shard takes turns, so every offset in the window is exercised.
    let mut seen = Vec::new();
    for n in 0.. {
        let owner = format!("w{}", n % 8);
        match claim(&db, "p", &owner, now).await {
            Some(t) => seen.push(specs.iter().find(|(id, _, _)| *id == t.id).unwrap().1),
            None => break,
        }
    }
    assert_eq!(seen.len(), 60);
    assert!(
        seen.windows(2).all(|w| w[0] <= w[1]),
        "a lower-priority task was claimed while a higher one was queued: {seen:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn oldest_queued_task_is_claimed_within_the_window() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "fifo").await;
    let now = Utc::now();
    let ids: Vec<String> = (0..200).map(|i| format!("f{i:03}")).collect();
    // f000 is the oldest; ids are inserted newest first so storage order
    // does not favour it.
    let tasks: Vec<NewTask> = ids
        .iter()
        .enumerate()
        .rev()
        .map(|(i, id)| task(id, "fifo", "p", 0, now - Duration::seconds(1000 - i as i64)))
        .collect();
    jobs::create_job(&db, &job("fifo", None, now), &tasks)
        .await
        .unwrap();
    let seq = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut handles = Vec::new();
    for c in 0..8 {
        let client = server.root("argos", "fifo").await;
        let seq = seq.clone();
        handles.push(tokio::spawn(async move {
            let owner = format!("w{c}");
            let mut got = Vec::new();
            for n in 0.. {
                let key = format!("claim:{owner}:{n}");
                match jobs::claim_next(&client, "p", &owner, LEASE, now, &key).await {
                    Ok(c) => match c.value {
                        Some(t) => {
                            let at = seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            got.push((at, t.id));
                        }
                        None => break,
                    },
                    Err(TxError::Contention { .. }) => {}
                    Err(e) => panic!("claim failed: {e}"),
                }
            }
            got
        }));
    }
    let mut all = Vec::new();
    for h in handles {
        all.extend(h.await.unwrap());
    }
    assert_eq!(all.len(), 200);
    let (at, _) = all.iter().find(|(_, id)| id == "f000").unwrap();
    assert!(*at < 32, "oldest task claimed only at claim {at}");
}

#[tokio::test]
async fn same_owner_reclaim_under_a_new_key_adds_no_attempt() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "reclaim").await;
    let now = Utc::now();
    jobs::create_job(&db, &job("j", None, now), &[task("t", "j", "p", 0, now)])
        .await
        .unwrap();
    let first = jobs::claim_next(&db, "p", "w", LEASE, now, "k1")
        .await
        .unwrap()
        .value
        .expect("claimed");
    let again = jobs::claim_task(&db, "p", "t", "w", LEASE, now, "k2")
        .await
        .unwrap()
        .value;
    assert!(again.is_none(), "a running task is not claimed again");
    assert!(jobs::claim_next(&db, "p", "w", LEASE, now, "k3")
        .await
        .unwrap()
        .value
        .is_none());
    assert_eq!(count(&db, "attempt").await, 1, "no duplicate attempt");
    let replay = jobs::claim_next(&db, "p", "w", LEASE, now, "k1")
        .await
        .unwrap()
        .value
        .expect("receipt replay");
    assert_eq!(
        (replay.id, replay.lease_epoch),
        (first.id, first.lease_epoch)
    );
    assert_eq!(count(&db, "attempt").await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn alternating_two_pools_keeps_each_pools_starvation_bound() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "alt").await;
    let now = Utc::now();
    let pools = ["alt_a", "alt_b"];
    let ids: Vec<(String, &str, i64)> = pools
        .iter()
        .flat_map(|p| (0..100).map(move |i| (format!("{p}{i:03}"), *p, i)))
        .collect();
    // Oldest first by created_at, inserted newest first.
    let tasks: Vec<NewTask> = ids
        .iter()
        .rev()
        .map(|(id, p, i)| task(id, "alt", p, 0, now - Duration::seconds(1000 - i)))
        .collect();
    jobs::create_job(&db, &job("alt", None, now), &tasks)
        .await
        .unwrap();
    let seqs = std::sync::Arc::new([
        std::sync::atomic::AtomicUsize::new(0),
        std::sync::atomic::AtomicUsize::new(0),
    ]);
    let mut handles = Vec::new();
    for c in 0..8 {
        let client = server.root("argos", "alt").await;
        let seqs = seqs.clone();
        handles.push(tokio::spawn(async move {
            let owner = format!("w{c}");
            let mut got = Vec::new();
            let mut empty = [false, false];
            for n in 0.. {
                let which = n % 2;
                if empty[0] && empty[1] {
                    break;
                }
                if empty[which] {
                    continue;
                }
                let key = format!("claim:{owner}:{n}");
                match jobs::claim_next(&client, pools[which], &owner, LEASE, now, &key).await {
                    Ok(r) => match r.value {
                        Some(t) => {
                            let at = seqs[which].fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            got.push((which, at, t.id));
                        }
                        None => empty[which] = true,
                    },
                    Err(TxError::Contention { .. }) => {}
                    Err(e) => panic!("claim failed: {e}"),
                }
            }
            got
        }));
    }
    let mut all = Vec::new();
    for h in handles {
        all.extend(h.await.unwrap());
    }
    assert_eq!(all.len(), 200);
    for (which, pool) in pools.iter().enumerate() {
        let oldest = format!("{pool}000");
        let (_, at, _) = all.iter().find(|(_, _, id)| *id == oldest).unwrap();
        eprintln!("ALT-POOL {pool} oldest_at={at}");
        assert!(*at < 32, "{pool}: oldest task claimed only at claim {at}");
        assert_eq!(all.iter().filter(|(w, _, _)| *w == which).count(), 100);
    }
}

/// Deterministic form: one worker strictly alternating two pools. With a
/// process-wide head counter every head claim lands on one pool, and the
/// other pool's oldest task waits until the queue drains to it.
#[tokio::test]
async fn one_worker_alternating_two_pools_reaches_both_heads() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "alt1").await;
    let now = Utc::now();
    let pools = ["alt1_a", "alt1_b"];
    let ids: Vec<(String, &str, i64)> = pools
        .iter()
        .flat_map(|p| (0..64).map(move |i| (format!("{p}{i:03}"), *p, i)))
        .collect();
    let tasks: Vec<NewTask> = ids
        .iter()
        .rev()
        .map(|(id, p, i)| task(id, "alt1", p, 0, now - Duration::seconds(1000 - i)))
        .collect();
    jobs::create_job(&db, &job("alt1", None, now), &tasks)
        .await
        .unwrap();
    let mut first_at = [None, None];
    for n in 0..64usize {
        for (which, pool) in pools.iter().enumerate() {
            let key = format!("claim:w:{which}:{n}");
            let t = jobs::claim_next(&db, pool, "w", LEASE, now, &key)
                .await
                .unwrap()
                .value
                .expect("claimed");
            if t.id == format!("{pool}000") {
                first_at[which] = Some(n);
            }
        }
    }
    eprintln!("ALT1 oldest_at={first_at:?}");
    for (which, pool) in pools.iter().enumerate() {
        let at = first_at[which].expect("oldest claimed");
        assert!(at < HEAD_BOUND, "{pool}: oldest claimed only at claim {at}");
    }
}

// ---------------------------------------------------------------------------
// Counters
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn counter_allocations_are_unique_gapless_and_replayable() {
    use argos_osint_core::storage::counters;
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "counters").await;
    counters::apply_schema(&db).await.unwrap();
    assert_eq!(counters::current(&db, "c").await.unwrap(), 0);
    let first = counters::allocate(&db, "c", "alloc:k1")
        .await
        .unwrap()
        .value;
    // Replay of the same key (an unknown commit result) returns the same value.
    assert_eq!(
        counters::allocate(&db, "c", "alloc:k1")
            .await
            .unwrap()
            .value,
        first
    );
    assert_eq!(first, 1);
    // A rolled-back allocation gives its value back.
    let aborted = run_tx(&db, RetryPolicy::default(), |tx| async move {
        counters::next_in(&tx, "c").await?;
        Err::<(), _>(TxError::Abort("rollback".into()))
    })
    .await;
    assert!(matches!(aborted, Err(TxError::Abort(_))));
    assert_eq!(counters::current(&db, "c").await.unwrap(), 1);

    let mut handles = Vec::new();
    for w in 0..8 {
        let client = server.root("argos", "counters").await;
        handles.push(tokio::spawn(async move {
            let mut got = Vec::new();
            for i in 0..10 {
                let r = counters::allocate(&client, "c", &format!("alloc:{w}:{i}"))
                    .await
                    .unwrap_or_else(|e| panic!("allocate: {e}"));
                got.push((r.value, r.retries));
            }
            got
        }));
    }
    let mut all = vec![first];
    let mut max_retries = 0;
    for h in handles {
        for (v, retries) in h.await.unwrap() {
            all.push(v);
            max_retries = max_retries.max(retries);
        }
    }
    eprintln!("COUNTER-RACE max_retries={max_retries}");
    all.sort();
    assert_eq!(all, (1..=81).collect::<Vec<i64>>(), "unique and gapless");
    assert_eq!(counters::current(&db, "c").await.unwrap(), 81);
}

// ---------------------------------------------------------------------------
// Index change outbox
// ---------------------------------------------------------------------------

fn change<'a>(id: &'a str, rev: &'a str, generation: &'a str) -> outbox::NewChange<'a> {
    outbox::NewChange {
        record_kind: "memory",
        record_id: id,
        revision: rev,
        operation: "index_upsert",
        generation,
    }
}

#[tokio::test]
async fn outbox_rows_commit_with_their_change_and_replay_by_op_key() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "outbox").await;
    outbox::apply_schema(&db).await.unwrap();
    db.query("DEFINE TABLE note SCHEMALESS")
        .await
        .unwrap()
        .check()
        .unwrap();
    let now = Utc::now();

    // The change and its row commit together, or neither does.
    let aborted = run_tx(&db, RetryPolicy::default(), |tx| async move {
        tx.query("UPSERT note:m0 SET body = 'x'").await?.check()?;
        outbox::enqueue_in(&tx, &change("m0", "r1", "g1"), "ix:m0:r1", now).await?;
        Err::<(), _>(TxError::Abort("rollback".into()))
    })
    .await;
    assert!(matches!(aborted, Err(TxError::Abort(_))));
    assert_eq!(count(&db, "index_change").await, 0);
    assert_eq!(count(&db, "note").await, 0);

    let a = outbox::enqueue(&db, &change("m1", "r1", "g1"), "ix:m1:r1", now)
        .await
        .unwrap()
        .value;
    let b = outbox::enqueue(&db, &change("m2", "r1", "g1"), "ix:m2:r1", now)
        .await
        .unwrap()
        .value;
    assert_eq!((a, b), (1, 2), "the rolled-back change left no seq gap");
    // Replay after an unknown commit result: same row, same seq, no new row.
    let replay = outbox::enqueue(&db, &change("m1", "r1", "g1"), "ix:m1:r1", now)
        .await
        .unwrap()
        .value;
    assert_eq!(replay, a);
    assert_eq!(count(&db, "index_change").await, 2);

    let pending = outbox::pending(&db, 0, 10).await.unwrap();
    assert_eq!(
        pending.iter().map(|c| c.seq).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(
        outbox::pending(&db, 1, 10).await.unwrap().len(),
        1,
        "cursor"
    );

    // A stale worker's ack is rejected; a repeated ack is harmless.
    let ack = |rev: &'static str, generation: &'static str| {
        let db = db.clone();
        async move {
            outbox::ack(&db, "ix:m1:r1", rev, generation, "completed", "", now)
                .await
                .unwrap()
                .value
        }
    };
    assert_eq!(ack("r0", "g1").await, outbox::Ack::Stale);
    assert_eq!(ack("r1", "g0").await, outbox::Ack::Stale);
    assert_eq!(ack("r1", "g1").await, outbox::Ack::Done);
    assert_eq!(ack("r1", "g1").await, outbox::Ack::AlreadyDone);
    assert_eq!(
        outbox::get(&db, "ix:m1:r1").await.unwrap().unwrap().state,
        "completed"
    );
    assert_eq!(
        outbox::pending(&db, 0, 10)
            .await
            .unwrap()
            .iter()
            .map(|c| c.seq)
            .collect::<Vec<_>>(),
        vec![2]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_outbox_enqueues_get_distinct_ordered_seqs() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "outbox_race").await;
    outbox::apply_schema(&db).await.unwrap();
    let now = Utc::now();
    let mut handles = Vec::new();
    for w in 0..8 {
        let client = server.root("argos", "outbox_race").await;
        handles.push(tokio::spawn(async move {
            for i in 0..10 {
                let id = format!("w{w}i{i}");
                outbox::enqueue(&client, &change(&id, "r", "g"), &format!("ix:{id}"), now)
                    .await
                    .unwrap_or_else(|e| panic!("enqueue: {e}"));
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    let seqs: Vec<i64> = outbox::pending(&db, 0, 200)
        .await
        .unwrap()
        .iter()
        .map(|c| c.seq)
        .collect();
    assert_eq!(seqs, (1..=80).collect::<Vec<i64>>());
}

// ---------------------------------------------------------------------------
// Process registry
// ---------------------------------------------------------------------------

#[tokio::test]
async fn gone_owners_tasks_are_recovered_once_and_fenced() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "processes").await;
    processes::apply_schema(&db).await.unwrap();
    let t0 = Utc::now();
    jobs::create_job(
        &db,
        &job("pj", None, t0),
        &[
            task("pa", "pj", "p", 0, t0),
            task("pb", "pj", "p", 0, t0 + Duration::seconds(1)),
            task("pc", "pj", "p", 0, t0 + Duration::seconds(2)),
        ],
    )
    .await
    .unwrap();
    for owner in ["me", "dead", "alive"] {
        processes::beat(&db, owner, 1, t0).await.unwrap();
    }
    // Long leases, so only the registry (not lease expiry) can recover them.
    let dead = jobs::claim_task(&db, "p", "pa", "dead", 3600, t0, "c:pa")
        .await
        .unwrap()
        .value
        .unwrap();
    let alive = jobs::claim_task(&db, "p", "pb", "alive", 3600, t0, "c:pb")
        .await
        .unwrap()
        .value
        .unwrap();
    // A task held by an owner that never registered is also orphaned.
    let ghost = jobs::claim_task(&db, "p", "pc", "ghost", 3600, t0, "c:pc")
        .await
        .unwrap()
        .value
        .unwrap();

    let t1 = t0 + Duration::seconds(processes::PROCESS_STALE_SECS + 5);
    // My own beat is stale too: a database outage, not dead owners.
    let r = processes::recover_orphans(&db, "me", t1)
        .await
        .unwrap()
        .value;
    assert!(r.skipped_self_stale);
    assert_eq!(r.tasks, 0);

    processes::beat(&db, "me", 1, t1).await.unwrap();
    processes::beat(&db, "alive", 1, t1).await.unwrap();
    // A replayed old beat never moves the heartbeat back.
    processes::beat(&db, "alive", 1, t0).await.unwrap();
    let r = processes::recover_orphans(&db, "me", t1)
        .await
        .unwrap()
        .value;
    assert_eq!((r.tasks, r.processes, r.skipped_self_stale), (2, 1, false));
    // Replay: a second pass finds nothing more.
    let r = processes::recover_orphans(&db, "me", t1)
        .await
        .unwrap()
        .value;
    assert_eq!((r.tasks, r.processes), (0, 0));

    let state = |id: &'static str| {
        let db = db.clone();
        async move { scalar::<String>(&db, &format!("SELECT VALUE state FROM ONLY task:{id}")).await }
    };
    assert_eq!(state("pa").await, "queued");
    assert_eq!(state("pc").await, "queued");
    assert_eq!(state("pb").await, "running", "a live owner keeps its task");
    let epoch: i64 = scalar(&db, "SELECT VALUE lease_epoch FROM ONLY task:pa").await;
    assert_eq!(epoch, dead.lease_epoch + 1, "recovered exactly once");
    // The old owners are fenced: their renewals fail.
    assert!(!jobs::renew_lease(&db, &dead, 3600, t1).await.unwrap().value);
    assert!(
        !jobs::renew_lease(&db, &ghost, 3600, t1)
            .await
            .unwrap()
            .value
    );
    assert!(
        jobs::renew_lease(&db, &alive, 3600, t1)
            .await
            .unwrap()
            .value
    );
    let open: i64 = scalar(
        &db,
        "RETURN count(SELECT * FROM attempt WHERE outcome = 'owner_gone')",
    )
    .await;
    assert_eq!(open, 2);
    assert_eq!(count(&db, "process").await, 2, "the dead row is removed");
}
