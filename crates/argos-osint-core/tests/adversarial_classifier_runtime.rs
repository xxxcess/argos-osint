//! Adversarial empirical stress harness for Argos Classifier QuickJS Runtime & Memory Budget.
//!
//! Verifies:
//! 1. QuickJS Memory Limit: 128 MiB heap ceiling enforced; OOM throws safely without process crash.
//! 2. QuickJS Stack Limit: 1 MiB stack ceiling enforced; deep recursion stops without segfault.
//! 3. Worker Budget Exhaustion: 3 workers fit within 512 MiB ledger; 4th fails with `RuntimeBudgetExceeded`.
//! 4. Drop Semantics & Synchronous Recovery: Dropping a worker immediately frees capacity for new reservations.

#![cfg(feature = "classifier")]

use std::sync::Arc;

use argos_osint_core::classifier::artifact::ArtifactStore;
use argos_osint_core::classifier::{
    start_candidate_worker, ClassifierError, ModelId, WorkControl, RUNTIME_MEMORY_LIMIT_BYTES,
    RUNTIME_STACK_LIMIT_BYTES,
};
use rquickjs::{Context, Runtime};

/// Adversarial Test 1: QuickJS Engine Memory Limit Enforcement (128 MiB)
///
/// Verifies that when JavaScript attempts to allocate beyond 128 MiB, QuickJS
/// triggers an out-of-memory exception and fails safely without segfaulting,
/// crashing the host process, or aborting the thread.
#[test]
fn test_adversarial_quickjs_memory_limit_enforced() {
    let runtime = Runtime::new().expect("QuickJS runtime creation must succeed");
    runtime.set_memory_limit(RUNTIME_MEMORY_LIMIT_BYTES);

    let context = Context::full(&runtime).expect("QuickJS context creation must succeed");

    let result: Result<(), rquickjs::Error> = context.with(|ctx| {
        // Attempt to allocate 140 MiB (140 * 1 MiB Uint8Arrays) against a 128 MiB limit
        ctx.eval::<(), _>(
            r#"
            const chunks = [];
            for (let i = 0; i < 140; i++) {
                chunks.push(new Uint8Array(1024 * 1024));
            }
            "#,
        )
    });

    assert!(
        result.is_err(),
        "allocating 140 MiB in a 128 MiB QuickJS runtime must fail with an allocation or exception error"
    );

    // Verify the runtime remains usable after catching the memory exception
    let recovery: Result<i32, rquickjs::Error> = context.with(|ctx| ctx.eval::<i32, _>("1 + 1"));
    assert_eq!(
        recovery.expect("runtime should still evaluate simple code after handled OOM"),
        2
    );
}

/// Adversarial Test 2: QuickJS Engine Stack Limit Enforcement (1 MiB)
///
/// Verifies that infinite recursion triggers QuickJS's stack overflow detector
/// based on `RUNTIME_STACK_LIMIT_BYTES` (1 MiB) rather than segfaulting the OS thread.
#[test]
fn test_adversarial_quickjs_stack_limit_enforced() {
    let runtime = Runtime::new().expect("QuickJS runtime creation must succeed");
    runtime.set_max_stack_size(RUNTIME_STACK_LIMIT_BYTES);

    let context = Context::full(&runtime).expect("QuickJS context creation must succeed");

    let result: Result<(), rquickjs::Error> = context.with(|ctx| {
        // Infinite recursion
        ctx.eval::<(), _>(
            r#"
            function recurse(depth) {
                return recurse(depth + 1);
            }
            recurse(0);
            "#,
        )
    });

    assert!(
        result.is_err(),
        "unbounded recursion must trigger a stack overflow error before exhausting OS thread stack"
    );

    let err_str = format!("{:?}", result.unwrap_err());
    assert!(
        err_str.to_lowercase().contains("stack")
            || err_str.to_lowercase().contains("overflow")
            || err_str.to_lowercase().contains("exception"),
        "error must indicate stack exhaustion or exception, got: {err_str}"
    );
}

/// Adversarial Test 3: Process-wide Candidate Worker Budget Exhaustion and Synchronous Recovery
///
/// Verifies that:
/// - Exactly 3 candidate workers (each 135,266,304 bytes) fit into the 512 MiB ledger.
/// - The 4th worker fails with `ClassifierError::RuntimeBudgetExceeded`.
/// - Dropping an active worker releases capacity synchronously and immediately allows a 4th worker.
#[test]
fn test_adversarial_candidate_worker_budget_exhaustion_and_recovery() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let store = ArtifactStore::new(&tmp.path().join("classifier")).expect("store");
    let lease = Arc::new(store.try_writer().expect("try_writer"));

    let m1 = ModelId::parse("candidate-m1").unwrap();
    let m2 = ModelId::parse("candidate-m2").unwrap();
    let m3 = ModelId::parse("candidate-m3").unwrap();
    let m4 = ModelId::parse("candidate-m4").unwrap();

    let control = WorkControl::new();

    // Spawn 3 candidate workers — each reserves 135,266,304 bytes
    let w1 = start_candidate_worker(&m1, lease.clone(), &control)
        .expect("worker 1 must succeed (135,266,304 <= 536,870,912)");
    let w2 = start_candidate_worker(&m2, lease.clone(), &control)
        .expect("worker 2 must succeed (270,532,608 <= 536,870,912)");
    let w3 = start_candidate_worker(&m3, lease.clone(), &control)
        .expect("worker 3 must succeed (405,798,912 <= 536,870,912)");

    // 4th worker: 405,798,912 + 135,266,304 = 541,065,216 > 536,870,912 -> must fail!
    let err4 = start_candidate_worker(&m4, lease.clone(), &control)
        .err()
        .expect("worker 4 must fail with RuntimeBudgetExceeded");

    match err4 {
        ClassifierError::RuntimeBudgetExceeded {
            requested,
            reserved,
            limit,
        } => {
            assert_eq!(
                requested, 135_266_304,
                "requested bytes must match single runtime ceiling"
            );
            assert_eq!(
                reserved, 405_798_912,
                "reserved bytes must match exactly 3 runtimes"
            );
            assert_eq!(limit, 536_870_912, "limit must match 512 MiB ceiling");
        }
        other => panic!("expected RuntimeBudgetExceeded, got: {other:?}"),
    }

    // Drop worker 1 synchronously
    drop(w1);

    // Now worker 4 must succeed immediately!
    let w4 = start_candidate_worker(&m4, lease.clone(), &control)
        .expect("worker 4 must succeed after worker 1 released its reservation");

    // Clean drop of remaining workers
    drop(w2);
    drop(w3);
    drop(w4);
}
