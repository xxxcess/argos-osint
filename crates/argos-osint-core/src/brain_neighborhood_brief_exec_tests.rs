use super::*;
use crate::brain_neighborhood_brief::tests::{brief_of, input_of, rich};
use crate::provider::{ModelAssignment, SettingsFile};
use crate::provider_attempt::mock;
use crate::secrets::{AuthFile, ProviderSecret};
use crate::store::Store;
use crate::tasks;

fn runtime(base: &str, model: &str) -> RoleRuntime {
    let mut auth = AuthFile::default();
    auth.set_account(ProviderSecret {
        kind: "local".into(),
        model: "fixture".into(),
        base_url: base.into(),
        api_key: Some("fixture-only".into()),
        stt_model: None,
        device: None,
    });
    let mut settings = SettingsFile::default();
    settings.nlp.mode = crate::nlp::Mode::Off;
    settings.defaults.summarization = ModelAssignment {
        provider: "local".into(),
        model: model.into(),
        ..Default::default()
    };
    RoleRuntime::new(auth, settings)
}

fn good() -> String {
    serde_json::json!({
        "overview": [{"text": "This claim concerns a port closure and sits among related claims, an actor and a source.", "basis": ["N1"]}],
        "connections": [{"text": "The Harbor Authority is referenced by both claims, and a recorded link says the second claim contradicts the first.", "basis": ["N2", "E3"]}],
        "interpretation": [{"text": "A stored inference with support state supported reads the two claims together; it is a stored interpretation, not a verified conclusion.", "basis": ["N4"]}],
        "limits": [{"text": "One premise of the stored inference is outside this neighborhood, and links not shown are not evidence of absence.", "basis": []}]
    })
    .to_string()
}

fn cancel() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

fn task_count(store: &Store) -> i64 {
    store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM argos_tasks WHERE operation='topology_neighborhood'",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

#[tokio::test]
async fn fallback_is_readable_first_then_a_validated_answer_replaces_it_and_hits_make_no_request() {
    let server = mock::serve(vec![mock::ok_json(&good(), "stop")]).await;
    let rt = runtime(&server.base_url, "fixture");
    let store = Store::memory().unwrap();
    let f = rich();
    let (input, fallback) = (input_of(&f), brief_of(&f));
    let fp = route_fingerprint(&rt);

    // Fallback first: pending, one queued task, no request yet.
    let (req, d) = publish(&store.conn, &input, &fallback, &fp).unwrap();
    assert_eq!(d, Delivery::Pending);
    assert_eq!(server.hits(), 0);
    // The same request is deduplicated.
    let (_, d2) = publish(&store.conn, &input, &fallback, &fp).unwrap();
    assert_eq!(d2, Delivery::Pending);
    assert_eq!(task_count(&store), 1);

    // The model answers later; the cache then holds the validated result.
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    let flush = load_flush_request(&store.conn, &req.cache_key())
        .unwrap()
        .unwrap();
    let g = generate(&rt, None, "op-1", None, &flush, &cancel())
        .await
        .unwrap();
    assert_eq!(g.route, "local/fixture");
    store_generated(&store.conn, &req, &g).unwrap();
    match poll(&store.conn, &req).unwrap() {
        Delivery::Generated(got) => {
            assert_eq!(got.brief.basis_key, fallback.basis_key);
            assert_eq!(got.route, "local/fixture");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(server.hits(), 1);
    // Exact hit: no new request, no new task.
    let (_, d3) = publish(&store.conn, &input, &fallback, &fp).unwrap();
    assert!(matches!(d3, Delivery::Generated(_)));
    assert_eq!(server.hits(), 1);
    assert_eq!(task_count(&store), 1);
}

#[tokio::test]
async fn malformed_and_empty_answers_are_failures_that_keep_the_fallback_within_the_attempt_cap() {
    for reply in ["not json", ""] {
        let server = mock::serve(vec![
            mock::ok_json(reply, "stop"),
            mock::ok_json(reply, "stop"),
            mock::ok_json(reply, "stop"),
        ])
        .await;
        let rt = runtime(&server.base_url, "fixture");
        let store = Store::memory().unwrap();
        let f = rich();
        let (input, fallback) = (input_of(&f), brief_of(&f));
        let (req, _) = publish(&store.conn, &input, &fallback, &route_fingerprint(&rt)).unwrap();
        let flush = load_flush_request(&store.conn, &req.cache_key())
            .unwrap()
            .unwrap();
        let err = generate(&rt, None, "op-bad", None, &flush, &cancel())
            .await
            .unwrap_err();
        assert!(server.hits() <= 2, "chain owns retries, capped at 2");
        assert_ne!(err.kind, FailureKind::NotConfigured);
        // Nothing generated was stored; the fallback is still what is cached.
        assert!(!matches!(
            poll(&store.conn, &req).unwrap(),
            Delivery::Generated(_)
        ));
    }
}

#[tokio::test]
async fn unknown_references_are_rejected_not_stored() {
    let bad = good().replace("\"N2\"", "\"N42\"");
    let server = mock::serve(vec![
        mock::ok_json(&bad, "stop"),
        mock::ok_json(&bad, "stop"),
    ])
    .await;
    let rt = runtime(&server.base_url, "fixture");
    let store = Store::memory().unwrap();
    let f = rich();
    let (input, fallback) = (input_of(&f), brief_of(&f));
    let (req, _) = publish(&store.conn, &input, &fallback, &route_fingerprint(&rt)).unwrap();
    let flush = load_flush_request(&store.conn, &req.cache_key())
        .unwrap()
        .unwrap();
    let err = generate(&rt, None, "op-ref", None, &flush, &cancel())
        .await
        .unwrap_err();
    assert_eq!(err.kind, FailureKind::Rejected);
    assert!(err.message.contains("unknown reference"));
}

#[tokio::test]
async fn missing_configuration_and_decisions_only_routes_spend_no_request() {
    let server = mock::serve(vec![mock::ok_json(&good(), "stop")]).await;
    let store = Store::memory().unwrap();
    let f = rich();
    let (input, fallback) = (input_of(&f), brief_of(&f));
    for rt in [
        RoleRuntime::new(AuthFile::default(), {
            let mut s = SettingsFile::default();
            s.nlp.mode = crate::nlp::Mode::Off;
            s
        }),
        runtime(&server.base_url, "typesafe/jev"),
    ] {
        let (req, _) = publish(&store.conn, &input, &fallback, &route_fingerprint(&rt)).unwrap();
        let flush = load_flush_request(&store.conn, &req.cache_key())
            .unwrap()
            .unwrap();
        let err = generate(&rt, None, "op-cfg", None, &flush, &cancel())
            .await
            .unwrap_err();
        assert_eq!(err.kind, FailureKind::NotConfigured);
        assert!(err.kind.needs_models());
    }
    assert_eq!(server.hits(), 0);
}

#[tokio::test]
async fn a_cancelled_operation_is_reported_as_cancelled() {
    let server = mock::serve(vec![mock::ok_json(&good(), "stop")]).await;
    let rt = runtime(&server.base_url, "fixture");
    let store = Store::memory().unwrap();
    let f = rich();
    let (input, fallback) = (input_of(&f), brief_of(&f));
    let (req, _) = publish(&store.conn, &input, &fallback, &route_fingerprint(&rt)).unwrap();
    let flush = load_flush_request(&store.conn, &req.cache_key())
        .unwrap()
        .unwrap();
    let stop = Arc::new(AtomicBool::new(true));
    let err = generate(&rt, None, "op-stop", None, &flush, &stop)
        .await
        .unwrap_err();
    assert_eq!(err.kind, FailureKind::Cancelled);
}

#[test]
fn terminal_failure_is_persisted_without_replay_and_retry_never_duplicates() {
    let store = Store::memory().unwrap();
    let f = rich();
    let (input, fallback) = (input_of(&f), brief_of(&f));
    let (req, _) = publish(&store.conn, &input, &fallback, "fp").unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    let claimed = tasks::claim_next_in(&store.conn, &[tasks::POOL_SUMMARY], "w", 60, &now)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.operation, "topology_neighborhood");
    // Retry while running adds nothing.
    assert!(!retry(&store.conn, &req).unwrap());
    let failure = GenerationFailure::new(FailureKind::Provider, "every route failed");
    assert!(tasks::complete_claimed(&store.conn, &claimed, &failure.tag(), &now).unwrap());
    match poll(&store.conn, &req).unwrap() {
        Delivery::Failed(got) => assert_eq!(got, failure),
        other => panic!("{other:?}"),
    }
    // Explicit Retry enqueues once; a second Retry is a no-op.
    assert!(retry(&store.conn, &req).unwrap());
    assert!(!retry(&store.conn, &req).unwrap());
    assert_eq!(poll(&store.conn, &req).unwrap(), Delivery::Pending);
}

#[test]
fn request_identity_follows_basis_options_and_role_configuration() {
    let f = rich();
    let input = input_of(&f);
    let a = build_request(&input, "fp-a").cache_key();
    assert_eq!(a, build_request(&input, "fp-a").cache_key());
    assert_ne!(a, build_request(&input, "fp-b").cache_key());
    let mut changed = rich();
    changed.snapshot.nodes[1].label = "Harbor traffic rerouted again".into();
    assert_ne!(
        a,
        build_request(&input_of(&changed), "fp-a").cache_key(),
        "changed data is a new request"
    );
    let mut rt1 = runtime("http://127.0.0.1:1", "m1");
    let rt2 = runtime("http://127.0.0.1:1", "m2");
    assert_ne!(route_fingerprint(&rt1), route_fingerprint(&rt2));
    rt1.auth.accounts.clear();
    assert!(!route_fingerprint(&rt1).contains("fixture-only"));
}

#[test]
fn isolated_neighborhoods_are_not_generated() {
    let mut f = rich();
    f.snapshot.edges.clear();
    f.snapshot.nodes.truncate(1);
    let store = Store::memory().unwrap();
    let (input, fallback) = (input_of(&f), brief_of(&f));
    let (_, d) = publish(&store.conn, &input, &fallback, "fp").unwrap();
    assert_eq!(d, Delivery::Idle);
    assert_eq!(task_count(&store), 0);
}

// ---------------------------------------------------------- scheduler delivery

use crate::scheduler::{drain_summary_flush_with_runtime, DEFAULT_LEASE_SECS};
use std::time::{Duration, Instant};

fn file_store() -> (tempfile::TempDir, std::path::PathBuf, Store) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    let store = Store::open(&path).unwrap();
    (dir, path, store)
}

fn prepared(store: &Store, rt: &RoleRuntime) -> SummaryRequest {
    let f = rich();
    publish(
        &store.conn,
        &input_of(&f),
        &brief_of(&f),
        &route_fingerprint(rt),
    )
    .unwrap()
    .0
}

#[tokio::test(flavor = "multi_thread")]
async fn worker_generates_publishes_and_completes_under_the_role_runtime() {
    let server = mock::serve(vec![mock::ok_json(&good(), "stop")]).await;
    let rt = runtime(&server.base_url, "fixture");
    let (_dir, path, store) = file_store();
    let req = prepared(&store, &rt);
    let out = drain_summary_flush_with_runtime(
        &store.conn,
        "w1",
        None,
        Some(&rt),
        Some(&path),
        DEFAULT_LEASE_SECS,
    )
    .unwrap();
    assert_eq!(out.len(), 1);
    assert!(out[0].generated && !out[0].stale);
    assert_eq!(out[0].tag, "generated:local/fixture");
    assert!(matches!(
        poll(&store.conn, &req).unwrap(),
        Delivery::Generated(_)
    ));
    // Nothing left to claim; the cache answers without a request.
    let again = drain_summary_flush_with_runtime(
        &store.conn,
        "w1",
        None,
        Some(&rt),
        Some(&path),
        DEFAULT_LEASE_SECS,
    )
    .unwrap();
    assert!(again.is_empty());
    assert_eq!(server.hits(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn delayed_reply_keeps_the_fallback_readable_until_the_watcher_sees_the_result() {
    let server = mock::serve_delayed(
        Duration::from_millis(900),
        vec![mock::ok_json(&good(), "stop")],
    )
    .await;
    let rt = runtime(&server.base_url, "fixture");
    let (_dir, path, store) = file_store();
    let req = prepared(&store, &rt);
    let stop = Arc::new(AtomicBool::new(false));
    let watcher = {
        let (p, r, s) = (path.clone(), req.clone(), stop.clone());
        std::thread::spawn(move || watch_until_settled(&p, &r, Duration::from_millis(100), &s))
    };
    let worker = {
        let (p, r) = (path.clone(), rt.clone());
        std::thread::spawn(move || {
            let conn = Connection::open(&p).unwrap();
            drain_summary_flush_with_runtime(
                &conn,
                "w1",
                None,
                Some(&r),
                Some(&p),
                DEFAULT_LEASE_SECS,
            )
            .unwrap()
        })
    };
    // While the model is still thinking, readers get Pending (fallback stays).
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(poll(&store.conn, &req).unwrap(), Delivery::Pending);
    let started = Instant::now();
    let settled = tokio::task::spawn_blocking(move || watcher.join().unwrap())
        .await
        .unwrap();
    assert!(matches!(settled, Some(Delivery::Generated(_))));
    assert!(started.elapsed() < Duration::from_secs(5));
    let out = worker.join().unwrap();
    assert!(out[0].generated);
    assert_eq!(
        server.hits(),
        1,
        "watching never runs a second model request"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn lease_is_renewed_while_generating() {
    HEARTBEAT_TRACE.lock().unwrap().clear();
    let server = mock::serve_delayed(
        Duration::from_millis(4500),
        vec![mock::ok_json(&good(), "stop")],
    )
    .await;
    let rt = runtime(&server.base_url, "fixture");
    let (_dir, path, store) = file_store();
    prepared(&store, &rt);
    // A 3s lease expires mid-request without the heartbeat; the generous
    // lease and delay keep the test tolerant of a loaded runner.
    let out = drain_summary_flush_with_runtime(&store.conn, "w1", None, Some(&rt), Some(&path), 3)
        .unwrap();
    let trace = HEARTBEAT_TRACE.lock().unwrap().clone();
    assert!(
        out[0].generated && !out[0].stale,
        "{out:?}\nheartbeat trace:\n{}",
        trace.join("\n")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_worker_that_lost_its_lease_cannot_publish_or_complete() {
    let server = mock::serve_delayed(
        Duration::from_millis(1500),
        vec![mock::ok_json(&good(), "stop")],
    )
    .await;
    let rt = runtime(&server.base_url, "fixture");
    let (_dir, path, store) = file_store();
    let req = prepared(&store, &rt);
    let thief = {
        let p = path.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            let conn = Connection::open(&p).unwrap();
            conn.execute("UPDATE argos_tasks SET lease_epoch=lease_epoch+1 WHERE operation='topology_neighborhood'", [])
                .unwrap();
        })
    };
    let out = drain_summary_flush_with_runtime(&store.conn, "w1", None, Some(&rt), Some(&path), 1)
        .unwrap();
    thief.join().unwrap();
    assert!(out[0].stale && !out[0].generated, "{out:?}");
    assert!(!matches!(
        poll(&store.conn, &req).unwrap(),
        Delivery::Generated(_)
    ));
    let state: String = store
        .conn
        .query_row(
            "SELECT state FROM argos_tasks WHERE operation='topology_neighborhood'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        state, "running",
        "the stale worker did not complete the task"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn failures_are_not_success_and_the_queue_does_not_replay_an_exhausted_chain() {
    let server = mock::serve(vec![mock::ok_json("not json", "stop")]).await;
    let rt = runtime(&server.base_url, "fixture");
    let (_dir, path, store) = file_store();
    let req = prepared(&store, &rt);
    let out = drain_summary_flush_with_runtime(
        &store.conn,
        "w1",
        None,
        Some(&rt),
        Some(&path),
        DEFAULT_LEASE_SECS,
    )
    .unwrap();
    assert!(!out[0].generated && out[0].tag.starts_with("topology_failed:rejected"));
    let hits = server.hits();
    assert!(hits <= 2);
    match poll(&store.conn, &req).unwrap() {
        Delivery::Failed(f) => assert_eq!(f.kind, FailureKind::Rejected),
        other => panic!("{other:?}"),
    }
    let again = drain_summary_flush_with_runtime(
        &store.conn,
        "w1",
        None,
        Some(&rt),
        Some(&path),
        DEFAULT_LEASE_SECS,
    )
    .unwrap();
    assert!(again.is_empty());
    assert_eq!(server.hits(), hits);
    // No runtime: not configured, not generated, fallback stays.
    let (_d2, p2, s2) = file_store();
    let req2 = prepared(&s2, &rt);
    let out = drain_summary_flush_with_runtime(&s2.conn, "w1", None, None, Some(&p2), 30).unwrap();
    assert!(!out[0].generated);
    match poll(&s2.conn, &req2).unwrap() {
        Delivery::Failed(f) => assert_eq!(f.kind, FailureKind::NotConfigured),
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cache_write_failure_publishes_nothing_and_leaves_the_task_to_the_queue_policy() {
    let server = mock::serve(vec![mock::ok_json(&good(), "stop")]).await;
    let rt = runtime(&server.base_url, "fixture");
    let (_dir, path, store) = file_store();
    let req = prepared(&store, &rt);
    store
        .conn
        .execute_batch(
            "CREATE TRIGGER no_generated BEFORE INSERT ON argos_derived_summaries
             BEGIN SELECT RAISE(ABORT,'cache full'); END;",
        )
        .unwrap();
    let out = drain_summary_flush_with_runtime(
        &store.conn,
        "w1",
        None,
        Some(&rt),
        Some(&path),
        DEFAULT_LEASE_SECS,
    )
    .unwrap();
    assert_eq!(out[0].tag, "storage_failed");
    assert!(!out[0].generated);
    assert!(!matches!(
        poll(&store.conn, &req).unwrap(),
        Delivery::Generated(_)
    ));
    let state: String = store
        .conn
        .query_row(
            "SELECT state FROM argos_tasks WHERE operation='topology_neighborhood'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(state, "completed");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_cache_table_is_a_queue_failure_not_a_success() {
    let rt = runtime("http://127.0.0.1:1", "fixture");
    let (_dir, path, store) = file_store();
    prepared(&store, &rt);
    store
        .conn
        .execute_batch("DROP TABLE argos_derived_summaries")
        .unwrap();
    let out = drain_summary_flush_with_runtime(
        &store.conn,
        "w1",
        None,
        Some(&rt),
        Some(&path),
        DEFAULT_LEASE_SECS,
    )
    .unwrap();
    assert!(!out[0].generated);
    assert_eq!(out[0].tag, "storage_failed");
}

#[test]
fn other_modes_report_a_deterministic_fallback_as_not_generated() {
    let store = Store::memory().unwrap();
    let req = crate::summarization::flush_request(
        SummarizationMode::AtlasBrief,
        "atlas-1",
        "r1",
        "Some deterministic atlas text that is long enough to keep.",
        "focus",
        400,
    );
    crate::summarization::publish_deterministic_and_enqueue(
        &store.conn,
        &req,
        crate::summarization::deterministic_follow_up("Some deterministic atlas text.", 400),
    )
    .unwrap();
    let out = drain_summary_flush_with_runtime(&store.conn, "w1", None, None, None, 30).unwrap();
    assert_eq!(out.len(), 1);
    assert!(!out[0].generated, "{out:?}");
}

#[test]
fn an_obsolete_watcher_detaches_without_a_result() {
    let (_dir, path, store) = file_store();
    let rt = runtime("http://127.0.0.1:1", "fixture");
    let req = prepared(&store, &rt);
    let stop = AtomicBool::new(true);
    assert!(watch_until_settled(&path, &req, Duration::from_millis(50), &stop).is_none());
}

#[test]
fn a_request_nobody_works_on_times_out_with_a_typed_failure() {
    let (_dir, path, store) = file_store();
    let rt = runtime("http://127.0.0.1:1", "fixture");
    let req = prepared(&store, &rt);
    let stop = AtomicBool::new(false);
    let limits = WatchLimits {
        max_poll_errors: 5,
        deadline: Duration::from_millis(200),
    };
    let began = Instant::now();
    match watch_with_limits(&path, &req, Duration::from_millis(50), &stop, limits) {
        Some(Delivery::Failed(f)) => {
            assert_eq!(f.kind, FailureKind::Timeout);
            assert!(f.message.contains("waiting for a worker"), "{f:?}");
        }
        other => panic!("{other:?}"),
    }
    assert!(began.elapsed() < Duration::from_secs(5));
}

#[test]
fn repeated_poll_errors_end_the_watch_with_a_storage_failure() {
    let (_dir, path, store) = file_store();
    let rt = runtime("http://127.0.0.1:1", "fixture");
    let req = prepared(&store, &rt);
    store
        .conn
        .execute_batch("DROP TABLE argos_tasks; DROP TABLE argos_derived_summaries;")
        .unwrap();
    let stop = AtomicBool::new(false);
    let limits = WatchLimits {
        max_poll_errors: 3,
        deadline: Duration::from_secs(30),
    };
    match watch_with_limits(&path, &req, Duration::from_millis(20), &stop, limits) {
        Some(Delivery::Failed(f)) => assert_eq!(f.kind, FailureKind::Storage),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_database_that_cannot_be_opened_is_a_storage_failure() {
    let (_dir, _path, store) = file_store();
    let rt = runtime("http://127.0.0.1:1", "fixture");
    let req = prepared(&store, &rt);
    let missing = std::path::Path::new("/nonexistent-dir/none/argos.db");
    let stop = AtomicBool::new(false);
    match watch_until_settled(missing, &req, Duration::from_millis(20), &stop) {
        Some(Delivery::Failed(f)) => assert_eq!(f.kind, FailureKind::Storage),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_standard_deadline_is_the_attempt_budget_plus_a_margin() {
    let limits = WatchLimits::standard();
    assert_eq!(limits.deadline, Duration::from_secs(90 * 2 + 60));
    assert!(limits.max_poll_errors >= 2);
}
