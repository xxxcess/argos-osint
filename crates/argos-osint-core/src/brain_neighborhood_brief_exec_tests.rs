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
