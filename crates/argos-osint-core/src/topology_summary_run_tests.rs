use super::*;
use crate::brain_topology::*;
use crate::provider_attempt::mock::{ok_json, secret, serve, Reply};
use crate::topology_summary::ScopeFilters;

fn input(seed: &str, seq: u64, label: &str) -> ScopeInput {
    let node = |id: &str, root: bool, hops, label: &str| TopologyNode {
        id: id.into(),
        kind: TopologyNodeKind::Claim,
        label: label.into(),
        detail: String::new(),
        subtype: String::new(),
        status: "active".into(),
        is_root: root,
        article_id: String::new(),
        run_id: String::new(),
        published_at: String::new(),
        cycle: None,
        member_ids: vec![],
        evidence_count: 0,
        passages: vec![],
        min_hops: hops,
        revision: "r1".into(),
        predecessor: None,
        topology_seed: None,
    };
    ScopeInput {
        spec: ScopeSpec {
            seed_key: format!("claim:{seed}"),
            depth: 2,
            cycle: CycleScope::AllRetained,
            filters: ScopeFilters::default(),
        },
        snapshot: TopologySnapshot {
            seed: Some(TopologySeed::Claim(seed.into())),
            root_id: seed.into(),
            root_label: label.into(),
            nodes: vec![node(seed, true, 0, label), node("n2", false, 1, "Second")],
            edges: vec![TopologyEdge {
                id: "e1".into(),
                from: seed.into(),
                to: "n2".into(),
                kind: TopologyEdgeKind::Premise,
                label: "premise".into(),
                status: "recorded".into(),
                member_ids: vec![],
                receipts: vec![],
                rationale: String::new(),
            }],
            exploration_exhausted: true,
            memory_changed_seq: seq,
            ..Default::default()
        },
    }
}

fn store() -> (tempfile::TempDir, Store, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("argos.db");
    let store = Store::open(&path).unwrap();
    (dir, store, path)
}

fn route() -> ProviderSecret {
    secret("http://127.0.0.1:9")
}

fn save(store: &Store, plan: &Plan, input: &ScopeInput, age_secs: i64, text: &str) {
    let s = route();
    store
        .save_topology_summary(&TopologySummaryRow {
            digest: plan.digest.clone(),
            scope_key: plan.scope_key.clone(),
            seed_key: input.spec.seed_key.clone(),
            seed_ref: plan.seed_ref.clone(),
            graph_version: plan.graph_version.clone(),
            mode: MODE_AI.into(),
            summary: text.into(),
            provider: crate::provider::effective_kind(&s),
            model: s.model,
            prompt_version: PROMPT_VERSION.into(),
            changed_seq: plan.changed_seq,
            ..Default::default()
        })
        .unwrap();
    // Backdate so the age rules can be exercised.
    let at = (Utc::now() - chrono::Duration::seconds(age_secs)).to_rfc3339();
    rusqlite::Connection::open(store_path(store))
        .unwrap()
        .execute(
            "UPDATE topology_scope_summaries SET created_at=?1 WHERE digest=?2",
            [&at, &plan.digest],
        )
        .unwrap();
}

fn store_path(_: &Store) -> std::path::PathBuf {
    PATH.with(|p| p.borrow().clone())
}
thread_local! { static PATH: std::cell::RefCell<std::path::PathBuf> = const { std::cell::RefCell::new(std::path::PathBuf::new()) }; }

fn setup() -> (tempfile::TempDir, Store, std::path::PathBuf) {
    let (d, s, p) = store();
    PATH.with(|c| *c.borrow_mut() = p.clone());
    (d, s, p)
}

#[test]
fn first_visit_generates_then_the_saved_summary_is_reused_without_a_model_call() {
    let (_d, store, _p) = setup();
    let i = input("c1", 1, "Seed");
    let s = route();
    let plan1 = plan(&store, &i, Ok(&s), Utc::now(), false).unwrap();
    assert_eq!(
        plan1.action,
        Action::Generate {
            why: "scope not summarized yet"
        }
    );
    assert!(plan1.saved.is_none());
    save(&store, &plan1, &i, 5, "## In scope\nsaved");
    let plan2 = plan(&store, &i, Ok(&s), Utc::now(), false).unwrap();
    assert_eq!(plan2.action, Action::None);
    let saved = plan2.saved.unwrap();
    assert!(saved.fresh && saved.text.contains("saved"));
}

#[test]
fn recenter_away_and_back_reuses_the_first_scopes_summary() {
    let (_d, store, _p) = setup();
    let s = route();
    let a = input("c1", 1, "A");
    let b = input("c2", 1, "B");
    let pa = plan(&store, &a, Ok(&s), Utc::now(), false).unwrap();
    save(&store, &pa, &a, 5, "## A");
    // Recenter: a different scope has nothing saved, so it generates.
    let pb = plan(&store, &b, Ok(&s), Utc::now(), false).unwrap();
    assert!(matches!(pb.action, Action::Generate { .. }));
    assert!(
        pb.saved.is_none(),
        "another scope's text is not shown as this scope's"
    );
    save(&store, &pb, &b, 5, "## B");
    // Back to A: exact digest hit, no regeneration.
    let back = plan(&store, &a, Ok(&s), Utc::now(), false).unwrap();
    assert_eq!(back.action, Action::None);
    assert_eq!(back.saved.unwrap().text, "## A");
}

#[test]
fn graph_change_regenerates_but_unrelated_churn_inside_the_throttle_waits() {
    let (_d, store, _p) = setup();
    let s = route();
    let i = input("c1", 1, "Seed");
    let p = plan(&store, &i, Ok(&s), Utc::now(), false).unwrap();
    save(&store, &p, &i, 60, "## old");
    // Same content, only the brain-wide counter moved: exact digest still valid.
    let same = input("c1", 9, "Seed");
    assert_eq!(
        plan(&store, &same, Ok(&s), Utc::now(), false)
            .unwrap()
            .action,
        Action::None
    );
    // Content changed and the counter moved, saved row is 1 minute old: throttled, old text kept as stale.
    let mut changed = input("c1", 9, "Seed");
    changed.snapshot.nodes[1].revision = "r2".into();
    let t = plan(&store, &changed, Ok(&s), Utc::now(), false).unwrap();
    match t.action {
        Action::Throttled { retry_in } => {
            assert!(retry_in <= RECHECK_THROTTLE && retry_in > Duration::from_secs(400))
        }
        other => panic!("{other:?}"),
    }
    let stale = t.saved.unwrap();
    assert!(!stale.fresh && stale.text == "## old");
    // An explicit retry bypasses the throttle.
    assert!(matches!(
        plan(&store, &changed, Ok(&s), Utc::now(), true)
            .unwrap()
            .action,
        Action::Generate { .. }
    ));
    // After the throttle has elapsed it regenerates.
    let later = Utc::now() + chrono::Duration::minutes(11);
    assert_eq!(
        plan(&store, &changed, Ok(&s), later, false).unwrap().action,
        Action::Generate {
            why: "graph changed"
        }
    );
}

#[test]
fn a_saved_narrative_expires_after_seven_days_even_if_nothing_changed() {
    let (_d, store, _p) = setup();
    let s = route();
    let i = input("c1", 1, "Seed");
    let p = plan(&store, &i, Ok(&s), Utc::now(), false).unwrap();
    save(&store, &p, &i, 8 * 24 * 3600, "## ancient");
    let again = plan(&store, &i, Ok(&s), Utc::now(), false).unwrap();
    assert_eq!(
        again.action,
        Action::Generate {
            why: "saved summary expired"
        }
    );
    assert!(!again.saved.unwrap().fresh);
}

#[test]
fn no_summarization_route_keeps_the_text_and_never_runs() {
    let (_d, store, _p) = setup();
    let s = route();
    let i = input("c1", 1, "Seed");
    let p = plan(&store, &i, Ok(&s), Utc::now(), false).unwrap();
    save(&store, &p, &i, 5, "## earlier");
    let none = plan(
        &store,
        &i,
        Err("Summarization is not configured".into()),
        Utc::now(),
        false,
    )
    .unwrap();
    assert_eq!(
        none.action,
        Action::Unavailable("Summarization is not configured".into())
    );
    assert_eq!(none.saved.unwrap().text, "## earlier");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn generation_saves_markdown_runs_one_job_and_a_failure_cools_down() {
    let (_d, store, path) = setup();
    let server = serve(vec![ok_json(
        "## In scope\n\n**Seed** links to *Second*.\n\n## Connections\n- premise",
        "stop",
    )])
    .await;
    let s = secret(&server.base_url);
    let i = input("c1", 1, "Seed");
    let p = plan(&store, &i, Ok(&s), Utc::now(), false).unwrap();
    let mut opts = ExecOptions::for_secret(&s);
    opts.admission_account = "topology-ok".into();
    let report = generate(
        &path,
        &s,
        &i,
        &p,
        &opts,
        "req-1",
        Arc::new(AtomicBool::new(false)),
    )
    .await;
    match &report.outcome {
        ExplainOutcome::Saved(text) => assert!(text.starts_with("## In scope")),
        other => panic!("{other:?}"),
    }
    assert_eq!(server.hits(), 1);
    let again = plan(&store, &i, Ok(&s), Utc::now(), false).unwrap();
    assert_eq!(again.action, Action::None, "the saved digest is reused");
    // The prompt is Markdown-directed and fences the data.
    let sent = server.requests.lock().unwrap().join("\n");
    assert!(
        sent.contains("Write Markdown only") && sent.contains("BEGIN_SCOPE_DATA"),
        "{sent}"
    );

    // A failing run records a failure, and the same inputs cool down.
    let bad = serve(vec![Reply::Json(
        401,
        r#"{"error":{"message":"bad key"}}"#.into(),
    )])
    .await;
    let bs = secret(&bad.base_url);
    let j = input("c2", 1, "Other");
    let pj = plan(&store, &j, Ok(&bs), Utc::now(), false).unwrap();
    let mut o2 = ExecOptions::for_secret(&bs);
    o2.admission_account = "topology-bad".into();
    let failed = generate(
        &path,
        &bs,
        &j,
        &pj,
        &o2,
        "req-2",
        Arc::new(AtomicBool::new(false)),
    )
    .await;
    assert!(matches!(failed.outcome, ExplainOutcome::Failed(_)));
    let cool = plan(&store, &j, Ok(&bs), Utc::now(), false).unwrap();
    assert!(
        matches!(cool.action, Action::Cooling(_)),
        "{:?}",
        cool.action
    );
    assert!(matches!(
        plan(&store, &j, Ok(&bs), Utc::now(), true).unwrap().action,
        Action::Generate { .. }
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_late_result_for_an_old_scope_is_saved_under_its_own_digest() {
    let (_d, store, path) = setup();
    let server = serve(vec![ok_json("## In scope\n\nold scope text", "stop")]).await;
    let s = secret(&server.base_url);
    let old = input("c1", 1, "Old");
    let p_old = plan(&store, &old, Ok(&s), Utc::now(), false).unwrap();
    // The user recentered before the run finished; the run still completes.
    let new = input("c2", 1, "New");
    let p_new = plan(&store, &new, Ok(&s), Utc::now(), false).unwrap();
    assert_ne!(p_old.digest, p_new.digest);
    let mut opts = ExecOptions::for_secret(&s);
    opts.admission_account = "topology-late".into();
    let report = generate(
        &path,
        &s,
        &old,
        &p_old,
        &opts,
        "req-late",
        Arc::new(AtomicBool::new(false)),
    )
    .await;
    assert!(matches!(report.outcome, ExplainOutcome::Saved(_)));
    assert_eq!(
        report.cache_key, p_old.digest,
        "the caller compares this to the current digest"
    );
    assert!(store.topology_summary(&p_old.digest).unwrap().is_some());
    assert!(store.topology_summary(&p_new.digest).unwrap().is_none());
}

#[test]
fn the_brief_is_bounded_and_fenced() {
    let mut i = input("c1", 1, "Seed");
    for k in 0..200 {
        let mut n = i.snapshot.nodes[1].clone();
        n.id = format!("x{k}");
        i.snapshot.nodes.push(n);
        let mut e = i.snapshot.edges[0].clone();
        e.id = format!("ex{k}");
        e.to = format!("x{k}");
        i.snapshot.edges.push(e);
    }
    let b = brief(&i);
    assert!(b.starts_with("BEGIN_SCOPE_DATA") && b.ends_with("END_SCOPE_DATA"));
    assert!(b.contains("more nodes") && b.contains("more relationships"));
    assert!(b.lines().count() < 330);
}

/// One real request through the installed Summarization route. Run manually:
/// `ARGOS_LIVE_TOPOLOGY=1 cargo test -p argos-osint-core live_topology -- --ignored --nocapture`
/// Prints only the route (provider kind, model, host) and the outcome, never a key.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn live_topology_summary_through_the_installed_summarization_route() {
    if std::env::var("ARGOS_LIVE_TOPOLOGY").is_err() {
        return;
    }
    let auth = crate::secrets::AuthFile::load().unwrap();
    let settings = crate::provider::SettingsFile::load().unwrap();
    let secret = crate::provider::role_secret(&auth, &settings, "summarization").unwrap();
    let host = url::Url::parse(&secret.base_url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default();
    println!(
        "route: kind={} model={} host={host} key_present={}",
        crate::provider::effective_kind(&secret),
        secret.model,
        secret.api_key.as_deref().is_some_and(|k| !k.is_empty())
    );
    let (_d, store, path) = setup();
    let i = input("live1", 1, "Harbor contract shifted");
    let p = plan(&store, &i, Ok(&secret), Utc::now(), false).unwrap();
    let mut opts = ExecOptions::for_secret(&secret);
    opts.admission_account = "topology-live".into();
    let report = generate(
        &path,
        &secret,
        &i,
        &p,
        &opts,
        "req-live",
        Arc::new(AtomicBool::new(false)),
    )
    .await;
    println!("requests: {}", report.attempts.len());
    match &report.outcome {
        ExplainOutcome::Saved(text) => {
            println!("outcome: saved {} chars\n{text}", text.len());
            assert!(
                plan(&store, &i, Ok(&secret), Utc::now(), false)
                    .unwrap()
                    .action
                    == Action::None
            );
        }
        ExplainOutcome::Failed(f) => {
            println!("outcome: failed · {} · {}", f.category.reason(), f.message)
        }
        ExplainOutcome::Superseded(why) => println!("outcome: superseded {why}"),
    }
}
