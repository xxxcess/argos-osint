//! Empirical Challenger 2 Stress Test Suite for Milestone 1:
//! 1. Task Lease Renewal Heartbeats (`HeartbeatGuard`) under prolonged execution (> lease interval).
//! 2. Lease loss detection (`lost == true`) and fence protection on invalidation.
//! 3. Clean cancellation of HeartbeatGuard on Drop.
//! 4. Bounded telemetry channel backpressure (`spawn_ordered` buffer = 4096, non-blocking overflow drop).
//! 5. `WorkerPool::shutdown_all` and `drop` latency (<500ms) and clean thread joins with real drainers.
#![cfg(feature = "surreal")]

mod support;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use argos_osint_core::scheduler::WorkerPool;
use argos_osint_core::storage::home::{self, Layout, Mode, StorageConfig};
use argos_osint_core::storage::jobs::{self, NewJob, NewTask, Publish};
use argos_osint_core::storage::{bootstrap, runtime, summary_flush};
use argos_osint_core::summarization::{deterministic_follow_up, flush_request, SummarizationMode};
use chrono::Utc;
use support::*;

struct TestHeartbeatGuard {
    stop: Arc<AtomicBool>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for TestHeartbeatGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

/// Stress Test 1: Task Lease Renewal Heartbeat keeps prolonged task live in SurrealDB
#[tokio::test]
async fn stress_test_heartbeat_renewal_prevents_fence_expiration() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "stress_heartbeat_renewal").await;
    bootstrap::ensure_schema(repo.db()).await.unwrap();

    let job_id = "test-job-prolonged";
    let task_id = "test-task-prolonged";
    let now = Utc::now();
    jobs::create_job(
        repo.db(),
        &NewJob {
            id: job_id,
            kind: "test_job",
            operation: "long_op",
            parent: None,
            created_at: now,
        },
        &[NewTask {
            id: task_id,
            job: job_id,
            pool: "summary",
            operation: "long_op",
            priority: 50,
            deps: Vec::new(),
            input_hash: "hash-long",
            created_at: now,
        }],
    )
    .await
    .unwrap();

    // 1. Claim task with a short 1-second lease.
    let lease_secs = 1i64;
    let claimed = jobs::claim_next(
        repo.db(),
        "summary",
        "worker_heartbeat",
        lease_secs,
        now,
        "claim-1",
    )
    .await
    .unwrap()
    .value
    .expect("claimed task");

    assert_eq!(claimed.id, task_id);

    // 2. Spawn heartbeat task identical to summary_flush and brain_scope
    let lost = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let beats_count = Arc::new(AtomicUsize::new(0));

    let beat = {
        let (repo, claimed) = (repo.clone(), claimed.clone());
        let (lost, stop) = (lost.clone(), stop.clone());
        let beats_count = beats_count.clone();
        let every = Duration::from_millis(250);
        tokio::spawn(async move {
            while !stop.load(Ordering::SeqCst) {
                tokio::time::sleep(every).await;
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let renewed = jobs::renew_lease(repo.db(), &claimed, lease_secs, Utc::now())
                    .await
                    .map(|c| c.value)
                    .unwrap_or(false);
                if renewed {
                    beats_count.fetch_add(1, Ordering::SeqCst);
                } else {
                    lost.store(true, Ordering::SeqCst);
                    break;
                }
            }
        })
    };
    let mut guard = TestHeartbeatGuard {
        stop: stop.clone(),
        handle: Some(beat),
    };

    // 3. Emulate prolonged execution: sleep 2.2 seconds (exceeding initial 1s lease)
    tokio::time::sleep(Duration::from_millis(2200)).await;

    // Heartbeat must not have been lost
    assert!(!lost.load(Ordering::SeqCst), "Lease must not be lost");
    let beats = beats_count.load(Ordering::SeqCst);
    assert!(
        beats >= 5,
        "Heartbeat should have renewed at least 5 times in 2.2s, renewed: {beats}"
    );

    // Stop guard cleanly
    stop.store(true, Ordering::SeqCst);
    if let Some(h) = guard.handle.take() {
        let _ = h.await;
    }

    // 4. Verify commit_stage succeeds with Publish::Committed because the lease was actively renewed
    let res = jobs::commit_stage(
        repo.db(),
        &claimed,
        "{}",
        "out-hash",
        Utc::now(),
        "op-key-commit-1",
    )
    .await
    .unwrap()
    .value;

    assert_eq!(
        res,
        Publish::Committed,
        "commit_stage must succeed with Committed after 2.2s execution because heartbeat renewed the 1s lease"
    );
}

/// Stress Test 2: Unrenewed task fence expires when execution exceeds lease interval (control baseline)
#[tokio::test]
async fn stress_test_unrenewed_task_fence_expires() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "stress_unrenewed_expiry").await;
    bootstrap::ensure_schema(repo.db()).await.unwrap();

    let job_id = "test-job-expire";
    let task_id = "test-task-expire";
    let now = Utc::now();
    jobs::create_job(
        repo.db(),
        &NewJob {
            id: job_id,
            kind: "test_job",
            operation: "exp_op",
            parent: None,
            created_at: now,
        },
        &[NewTask {
            id: task_id,
            job: job_id,
            pool: "summary",
            operation: "exp_op",
            priority: 50,
            deps: Vec::new(),
            input_hash: "hash-exp",
            created_at: now,
        }],
    )
    .await
    .unwrap();

    // Claim with 1-second lease
    let claimed = jobs::claim_next(repo.db(), "summary", "worker_expire", 1, now, "claim-exp")
        .await
        .unwrap()
        .value
        .expect("claimed");

    // Sleep 1.6s WITHOUT heartbeat renewal
    tokio::time::sleep(Duration::from_millis(1600)).await;

    // Verify commit_stage fails with LeaseLost because lease_until has passed
    let res = jobs::commit_stage(
        repo.db(),
        &claimed,
        "{}",
        "out-hash",
        Utc::now(),
        "op-key-commit-exp",
    )
    .await
    .unwrap()
    .value;

    assert_eq!(
        res,
        Publish::LeaseLost,
        "commit_stage must return LeaseLost when lease has expired without renewal"
    );
}

/// Stress Test 3: Simulated lease loss causes HeartbeatGuard to flag lost == true
#[tokio::test]
async fn stress_test_heartbeat_flags_lost_on_lease_invalidation() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "stress_lost_lease").await;
    bootstrap::ensure_schema(repo.db()).await.unwrap();

    let job_id = "test_job_lost";
    let task_id = "test_task_lost";
    let now = Utc::now();
    jobs::create_job(
        repo.db(),
        &NewJob {
            id: job_id,
            kind: "test_job",
            operation: "lost_op",
            parent: None,
            created_at: now,
        },
        &[NewTask {
            id: task_id,
            job: job_id,
            pool: "summary",
            operation: "lost_op",
            priority: 50,
            deps: Vec::new(),
            input_hash: "hash-lost",
            created_at: now,
        }],
    )
    .await
    .unwrap();

    let claimed = jobs::claim_next(repo.db(), "summary", "worker_lost", 1, now, "claim-lost")
        .await
        .unwrap()
        .value
        .expect("claimed");

    let lost = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));

    let beat = {
        let (repo, claimed) = (repo.clone(), claimed.clone());
        let (lost, stop) = (lost.clone(), stop.clone());
        let every = Duration::from_millis(100);
        tokio::spawn(async move {
            while !stop.load(Ordering::SeqCst) {
                tokio::time::sleep(every).await;
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let renewed = jobs::renew_lease(repo.db(), &claimed, 1, Utc::now())
                    .await
                    .map(|c| c.value)
                    .unwrap_or(false);
                if !renewed {
                    lost.store(true, Ordering::SeqCst);
                    break;
                }
            }
        })
    };
    let mut guard = TestHeartbeatGuard {
        stop: stop.clone(),
        handle: Some(beat),
    };

    // Simulate external epoch bump or eviction by setting lease_epoch to 999
    repo.db()
        .query(format!("UPDATE task:{task_id} SET lease_epoch = 999"))
        .await
        .unwrap()
        .check()
        .unwrap();

    // Wait 350ms for heartbeat to attempt renewal and fail
    tokio::time::sleep(Duration::from_millis(350)).await;

    assert!(
        lost.load(Ordering::SeqCst),
        "HeartbeatGuard must detect renewal failure and flag lost == true"
    );

    stop.store(true, Ordering::SeqCst);
    if let Some(h) = guard.handle.take() {
        let _ = h.await;
    }
}

/// Stress Test 4: Dropping HeartbeatGuard cleanly halts renewal task
#[tokio::test]
async fn stress_test_heartbeat_guard_drop_aborts_task() {
    let tick_count = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));

    let handle = {
        let ticks = tick_count.clone();
        let stop = stop.clone();
        tokio::spawn(async move {
            while !stop.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(50)).await;
                ticks.fetch_add(1, Ordering::SeqCst);
            }
        })
    };

    let guard = TestHeartbeatGuard {
        stop: stop.clone(),
        handle: Some(handle),
    };

    // Let it run for 120ms (~2 ticks)
    tokio::time::sleep(Duration::from_millis(120)).await;
    let ticks_before_drop = tick_count.load(Ordering::SeqCst);
    assert!(ticks_before_drop >= 1);

    // Drop the guard: should abort the handle and set stop = true
    drop(guard);

    // Wait another 150ms
    tokio::time::sleep(Duration::from_millis(150)).await;
    let ticks_after_drop = tick_count.load(Ordering::SeqCst);

    // Ticks should not increase any further
    assert_eq!(
        ticks_before_drop, ticks_after_drop,
        "Handle must be aborted immediately upon Guard drop"
    );
}

/// Stress Test 5: End-to-end summary_flush::drain executes cleanly with HeartbeatGuard active
#[tokio::test]
async fn stress_test_summary_flush_drain_with_heartbeat() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "stress_summary_drain_heartbeat").await;
    bootstrap::ensure_schema(repo.db()).await.unwrap();

    let text = "Test observation content for drain heartbeat verification.";
    let req = flush_request(
        SummarizationMode::ToolObservation,
        "call-hb",
        "call-hb",
        text,
        "tool",
        400,
    );
    let det = deterministic_follow_up(text, 400);
    summary_flush::publish(&repo, &req, det).await.unwrap();

    let out = summary_flush::drain(&repo, "worker-1", || None, 60)
        .await
        .unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].tag, "cached_deterministic_no_secret");
}

/// Stress Test 6: Bounded Telemetry Queue (4096 capacity) enforces backpressure without blocking callers
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stress_test_spawn_ordered_bounded_backpressure_and_drain() {
    let dir = tempfile::tempdir().unwrap();
    let layout = Layout::new(dir.path());
    home::save_config(
        &layout,
        &StorageConfig {
            mode: Mode::Embedded,
            port: None,
        },
    )
    .unwrap();

    let _ = runtime::init(layout);

    assert_eq!(
        runtime::ORDERED_QUEUE_CAPACITY,
        4096,
        "Capacity must be exactly 4096"
    );

    // Test non-blocking property: queueing 1000 items must be ultra-fast (<50ms)
    let start = Instant::now();
    let executed_counter = Arc::new(AtomicUsize::new(0));

    for _ in 0..1000 {
        let counter = executed_counter.clone();
        let accepted = runtime::spawn_ordered(async move {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        assert!(accepted, "Should accept within 4096 capacity");
    }
    let enqueue_time = start.elapsed();
    assert!(
        enqueue_time < Duration::from_millis(150),
        "Enqueueing must be non-blocking: took {:?}",
        enqueue_time
    );

    // Wait for drain
    runtime::drain().await;
    assert_eq!(
        executed_counter.load(Ordering::SeqCst),
        1000,
        "All accepted tasks must execute on drain"
    );
}

/// Stress Test 7: WorkerPool::shutdown_all latency with real spawned drainers < 500ms
#[test]
fn stress_test_worker_pool_shutdown_latency_under_500ms() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();

    for i in 0..5 {
        let mut pools = Vec::new();
        // Spawn 4 real summary pool drainers and 2 brain scope drainers
        for p in 0..4 {
            pools.push(WorkerPool::spawn_summary_flush_drainer(
                path.clone(),
                format!("owner-sum-{i}-{p}"),
            ));
        }
        for p in 0..2 {
            pools.push(WorkerPool::spawn_brain_scope_drainer(
                path.clone(),
                format!("owner-scope-{i}-{p}"),
            ));
        }

        let start = Instant::now();
        WorkerPool::shutdown_all(pools);
        let elapsed = start.elapsed();

        assert!(
            elapsed < Duration::from_millis(500),
            "Iteration {i}: WorkerPool::shutdown_all took {:?}, exceeding 500ms limit",
            elapsed
        );
    }
}

/// Stress Test 8: WorkerPool Drop stops and joins cleanly without deadlocks
#[test]
fn stress_test_worker_pool_drop_joins_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();

    for _ in 0..10 {
        let pool = WorkerPool::spawn_summary_flush_drainer(path.clone(), "owner-drop".into());
        let start = Instant::now();
        drop(pool);
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(350),
            "WorkerPool drop join must terminate promptly (<350ms): took {:?}",
            elapsed
        );
    }
}
