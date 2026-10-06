use super::*;
use crate::provider_attempt::mock::*;
use crate::provider_attempt::Transport;
use crate::brain::MemorySource;

const GOOD: &str =
    "## Northwind **halted** Baltic crossings\n\nTwo articles state the halt directly.";

struct Fx {
    _dir: tempfile::TempDir,
    db: std::path::PathBuf,
    memory_id: String,
}

fn fixture() -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("argos.db");
    let store = Store::open(&db).unwrap();
    let memory = store
        .add_memory(
            "Northwind halted Baltic crossings",
            "fact",
            false,
            MemorySource {
                app: "test".into(),
                conversation_id: "c".into(),
                message_id: None,
                reference: None,
            },
        )
        .unwrap();
    Fx {
        _dir: dir,
        db,
        memory_id: memory.id,
    }
}

fn request(fx: &Fx, id: &str) -> ExplainRequest {
    ExplainRequest {
        memory_id: fx.memory_id.clone(),
        memory_text: "Northwind halted Baltic crossings".into(),
        focus: "d1".into(),
        claim: true,
        system: "Explain the claim path.".into(),
        graph_brief: "Investigation: Northwind\nClaim path".into(),
        request_id: id.into(),
        retry_of: None,
    }
}

fn opts(secret: &ProviderSecret, account: &str, first: Transport) -> ExecOptions {
    let mut o = ExecOptions::for_secret(secret);
    o.admission_account = account.into();
    o.max_backoff = Duration::from_millis(20);
    o.admission_poll = Duration::from_millis(10);
    o.first_transport = first;
    o
}

async fn run(
    fx: &Fx,
    script: Vec<Reply>,
    first: Transport,
    faults: Faults,
) -> (ExplainReport, usize) {
    let server = serve(script).await;
    let secret = secret(&server.base_url);
    let report = explain(
        &fx.db,
        &secret,
        &request(fx, "r1"),
        &opts(&secret, &format!("ge-{}", fx.memory_id), first),
        faults,
    )
    .await;
    (report, server.hits())
}

fn conn(fx: &Fx) -> Connection {
    Connection::open(&fx.db).unwrap()
}

fn count(fx: &Fx, sql: &str, arg: &str) -> i64 {
    conn(fx).query_row(sql, [arg], |r| r.get(0)).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failure_is_durable_correlated_bounded_and_harmless() {
    let fx = fixture();
    let (report, hits) = run(
        &fx,
        vec![Reply::Json(503, "{}".into())],
        Transport::NonStream,
        Faults::default(),
    )
    .await;
    let ExplainOutcome::Failed(f) = &report.outcome else {
        panic!("{:?}", report.outcome)
    };
    assert_eq!(f.category, Category::Server);
    assert_eq!((report.requests(), hits), (2, 2));
    // One job, two attempt children, all failed and linked.
    let state: String = conn(&fx)
        .query_row(
            "SELECT state FROM argos_jobs WHERE id=?1",
            [&report.job_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "failed");
    assert_eq!(
        count(
            &fx,
            "SELECT COUNT(*) FROM argos_jobs WHERE parent_id=?1 AND state='failed'",
            &report.job_id
        ),
        2
    );
    // The failure event exists before the report is returned and carries details.
    assert!(!report.event_id.is_empty());
    let details: String = conn(&fx)
        .query_row(
            "SELECT details FROM argos_events WHERE id=?1",
            [&report.event_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(details.contains("\"category\":\"server\""), "{details}");
    assert!(report.logging_error.is_none());
    // Diagnostic record survives for View details and cooldown.
    let store = Store::open(&fx.db).unwrap();
    let rec = store
        .graph_explanation_record(&fx.memory_id)
        .unwrap()
        .unwrap();
    assert_eq!((rec.state.as_str(), rec.attempts), ("failed", 2));
    let lines = record_detail_lines(&rec).join("\n");
    assert!(lines.contains("Requests: 2 of 2"), "{lines}");
    assert!(lines.contains("Attempt 2"), "{lines}");
    // Nothing destructive: memory kept, nothing published, no failed index work.
    assert!(store.get_memory(&fx.memory_id).unwrap().is_some());
    assert!(store.graph_summary_entry(&fx.memory_id).unwrap().is_none());
    assert_eq!(
        count(
            &fx,
            "SELECT COUNT(*) FROM argos_tasks WHERE state=?1",
            "failed"
        ),
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn partial_streamed_text_is_never_saved() {
    let fx = fixture();
    let partial = Reply::Sse(vec![
        delta("## Northwind halted"),
        delta(" Baltic crossings because"),
    ]);
    let (report, hits) = run(
        &fx,
        vec![partial.clone(), partial],
        Transport::Stream,
        Faults::default(),
    )
    .await;
    assert_eq!(hits, 2);
    assert!(
        matches!(report.outcome, ExplainOutcome::Failed(_)),
        "{:?}",
        report.outcome
    );
    assert_eq!(
        report.attempts[0].failure.as_ref().unwrap().category,
        Category::PrematureEof
    );
    assert!(
        report.attempts[0]
            .failure
            .as_ref()
            .unwrap()
            .stream
            .content_began
    );
    assert_eq!(report.attempts[1].transport, "non_stream");
    let store = Store::open(&fx.db).unwrap();
    assert!(store.graph_summary_entry(&fx.memory_id).unwrap().is_none());

    // SSE error event and token-limit termination: also never saved.
    let err = "event: error\ndata: {\"error\":{\"message\":\"upstream reset\"}}\n\n".to_string();
    let (report, _) = run(
        &fx,
        vec![
            Reply::Sse(vec![delta("Some text"), err]),
            ok_json("Cut off mid", "length"),
        ],
        Transport::Stream,
        Faults::default(),
    )
    .await;
    let ExplainOutcome::Failed(f) = &report.outcome else {
        panic!()
    };
    assert_eq!(f.category, Category::TokenLimit);
    assert_eq!(
        report.attempts[0].failure.as_ref().unwrap().category,
        Category::SseError
    );
    assert!(store.graph_summary_entry(&fx.memory_id).unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_and_empty_results_fail_without_publishing() {
    let fx = fixture();
    let (report, hits) = run(
        &fx,
        vec![
            Reply::Json(200, "{\"choices\":[{".into()),
            ok_json("  ", "stop"),
        ],
        Transport::NonStream,
        Faults::default(),
    )
    .await;
    assert_eq!(hits, 2);
    assert_eq!(
        report.attempts[0].failure.as_ref().unwrap().category,
        Category::MalformedPayload
    );
    let ExplainOutcome::Failed(f) = &report.outcome else {
        panic!()
    };
    assert_eq!(f.category, Category::Empty);
    assert!(Store::open(&fx.db)
        .unwrap()
        .graph_summary_entry(&fx.memory_id)
        .unwrap()
        .is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn persistence_failure_is_typed_and_logged() {
    let fx = fixture();
    let (report, hits) = run(
        &fx,
        vec![ok_json(GOOD, "stop")],
        Transport::NonStream,
        Faults { persistence: true },
    )
    .await;
    assert_eq!(hits, 1);
    let ExplainOutcome::Failed(f) = &report.outcome else {
        panic!()
    };
    assert_eq!(
        (f.stage, f.category),
        (Stage::Persistence, Category::Persistence)
    );
    assert!(
        f.causes.iter().any(|c| c.contains("disk I/O error")),
        "{:?}",
        f.causes
    );
    assert!(!report.event_id.is_empty());
    let store = Store::open(&fx.db).unwrap();
    assert!(store.get_memory(&fx.memory_id).unwrap().is_some());
    assert!(store.graph_summary_entry(&fx.memory_id).unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn success_is_keyed_and_any_input_change_invalidates_it() {
    let fx = fixture();
    let server = serve(vec![ok_json(GOOD, "stop")]).await;
    let secret = secret(&server.base_url);
    let req = request(&fx, "r1");
    let report = explain(
        &fx.db,
        &secret,
        &req,
        &opts(&secret, "ge-ok", Transport::NonStream),
        Faults::default(),
    )
    .await;
    assert_eq!(report.outcome, ExplainOutcome::Saved(GOOD.into()));
    let store = Store::open(&fx.db).unwrap();
    let key = req.key(&secret);
    assert_eq!(
        cached(&store, &fx.memory_id, &key).unwrap(),
        Cached::Valid(GOOD.into())
    );
    let stale = |mut r: ExplainRequest, s: &ProviderSecret, edit: &dyn Fn(&mut ExplainRequest)| {
        edit(&mut r);
        cached(&store, &fx.memory_id, &r.key(s)).unwrap()
    };
    let evidence = stale(req.clone(), &secret, &|r| {
        r.graph_brief.push_str("\nevidence: new article")
    });
    let text = stale(req.clone(), &secret, &|r| {
        r.memory_text.push_str(" (revised)")
    });
    let focus = stale(req.clone(), &secret, &|r| r.focus = "d2".into());
    let prompt = stale(req.clone(), &secret, &|r| r.system.push_str(" v3"));
    let mut other = secret.clone();
    other.model = "other-model".into();
    let model = stale(req.clone(), &other, &|_| {});
    for c in [evidence, text, focus, prompt, model] {
        assert_eq!(c, Cached::Stale(GOOD.into()));
    }
    // The job completed and the record says so.
    let rec = store
        .graph_explanation_record(&fx.memory_id)
        .unwrap()
        .unwrap();
    assert_eq!(rec.state, "completed");
    assert_eq!(
        store.job_state(&report.job_id).unwrap().as_deref(),
        Some("completed")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_completion_for_a_changed_or_deleted_memory_is_not_published() {
    let fx = fixture();
    let store = Store::open(&fx.db).unwrap();
    store
        .update_memory(&fx.memory_id, "Northwind resumed crossings", "fact", false)
        .unwrap();
    let (report, _) = run(
        &fx,
        vec![ok_json(GOOD, "stop")],
        Transport::NonStream,
        Faults::default(),
    )
    .await;
    assert!(
        matches!(report.outcome, ExplainOutcome::Superseded(_)),
        "{:?}",
        report.outcome
    );
    assert!(store.graph_summary_entry(&fx.memory_id).unwrap().is_none());
    store.delete_memory(&fx.memory_id).unwrap();
    let (report, _) = run(
        &fx,
        vec![ok_json(GOOD, "stop")],
        Transport::NonStream,
        Faults::default(),
    )
    .await;
    assert!(
        matches!(report.outcome, ExplainOutcome::Superseded(ref why) if why.contains("deleted"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_failure_sends_one_request_gives_guidance_and_redacts() {
    let fx = fixture();
    let body = r#"{"error":{"message":"Incorrect API key sk-mocksecretvalue12345","code":"invalid_api_key"}}"#;
    let server = serve(vec![Reply::Json(401, body.into())]).await;
    let mut secret = secret(&server.base_url);
    secret.base_url = server.base_url.replace("http://", "http://user:hunter2pw@");
    let report = explain(
        &fx.db,
        &secret,
        &request(&fx, "r1"),
        &opts(&secret, "ge-auth", Transport::NonStream),
        Faults::default(),
    )
    .await;
    assert_eq!(server.hits(), 1);
    let ExplainOutcome::Failed(f) = &report.outcome else {
        panic!()
    };
    assert!(f.needs_configuration());
    assert!(f.guidance().unwrap().contains("Providers"));
    let store = Store::open(&fx.db).unwrap();
    let rec = store
        .graph_explanation_record(&fx.memory_id)
        .unwrap()
        .unwrap();
    assert!(rec.needs_config && !rec.retryable);
    let all: String = conn(&fx)
        .query_row(
            "SELECT group_concat(message || details, '\n') FROM argos_events",
            [],
            |r| r.get(0),
        )
        .unwrap();
    for leaked in ["sk-mocksecretvalue12345", "hunter2pw"] {
        assert!(!all.contains(leaked), "leaked {leaked}");
        assert!(!rec.diagnostic_json.contains(leaked));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reopening_respects_cooldown_and_running_jobs() {
    let fx = fixture();
    let server = serve(vec![Reply::Json(503, "{}".into())]).await;
    let secret = secret(&server.base_url);
    let req = request(&fx, "r1");
    let key = req.key(&secret);
    let store = Store::open(&fx.db).unwrap();
    assert_eq!(gate(&store, &fx.memory_id, &key, false), Gate::Ready);
    explain(
        &fx.db,
        &secret,
        &req,
        &opts(&secret, "ge-gate", Transport::NonStream),
        Faults::default(),
    )
    .await;
    assert!(matches!(
        gate(&store, &fx.memory_id, &key, false),
        Gate::CoolingDown(_)
    ));
    assert_eq!(gate(&store, &fx.memory_id, &key, true), Gate::Ready);
    let mut other = req.clone();
    other.focus = "d9".into();
    assert_eq!(
        gate(&store, &fx.memory_id, &other.key(&secret), false),
        Gate::Ready
    );
    // A running execution blocks a duplicate even on explicit retry.
    let job = JobHandle::begin(
        &fx.db,
        JobSpec::new("brain", OPERATION, "Explain claim path"),
    )
    .unwrap();
    let mut rec = store
        .graph_explanation_record(&fx.memory_id)
        .unwrap()
        .unwrap();
    rec.state = "running".into();
    rec.job_id = job.id().into();
    store.put_graph_explanation_record(&rec).unwrap();
    assert_eq!(
        gate(&store, &fx.memory_id, &key, true),
        Gate::Running(job.id().into())
    );
    job.finish(Finish::completed());
}
