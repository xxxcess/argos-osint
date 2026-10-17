use super::*;
use crate::brain_topology::*;
use crate::store::{Store, TopologySummaryRow};

fn node(
    id: &str,
    kind: TopologyNodeKind,
    label: &str,
    hops: usize,
    pred: Option<&str>,
) -> TopologyNode {
    TopologyNode {
        id: id.into(),
        kind,
        label: label.into(),
        detail: String::new(),
        subtype: String::new(),
        status: "active".into(),
        is_root: hops == 0,
        article_id: String::new(),
        run_id: String::new(),
        published_at: String::new(),
        cycle: None,
        member_ids: vec![],
        evidence_count: 0,
        passages: vec![],
        min_hops: hops,
        revision: "r1".into(),
        predecessor: pred.map(str::to_string),
        topology_seed: None,
    }
}

fn edge(id: &str, from: &str, to: &str, kind: TopologyEdgeKind, label: &str) -> TopologyEdge {
    TopologyEdge {
        id: id.into(),
        from: from.into(),
        to: to.into(),
        kind,
        label: label.into(),
        status: "recorded".into(),
        member_ids: vec![],
        receipts: vec![],
        rationale: String::new(),
    }
}

fn snapshot() -> TopologySnapshot {
    let mut claim = node(
        "c1",
        TopologyNodeKind::Claim,
        "Port closed after attack",
        0,
        None,
    );
    claim.evidence_count = 3;
    TopologySnapshot {
        root_id: "c1".into(),
        root_label: "Port closed after attack".into(),
        root_kind: Some(TopologyNodeKind::Claim),
        nodes: vec![
            claim,
            node(
                "a1",
                TopologyNodeKind::Entity,
                "Harbor Authority",
                1,
                Some("c1"),
            ),
            node("s1", TopologyNodeKind::Source, "reuters.com", 1, Some("c1")),
            node(
                "i1",
                TopologyNodeKind::Inference,
                "Shipping will reroute",
                1,
                Some("c1"),
            ),
            node(
                "c2",
                TopologyNodeKind::Claim,
                "Rates rose 12 percent",
                2,
                Some("i1"),
            ),
        ],
        edges: vec![
            edge("e1", "c1", "a1", TopologyEdgeKind::Concerns, "concerns"),
            edge("e2", "c1", "s1", TopologyEdgeKind::Source, "source"),
            edge("e3", "i1", "c1", TopologyEdgeKind::Premise, "premise"),
            edge("e4", "i1", "c2", TopologyEdgeKind::Premise, "premise"),
            edge("e5", "c2", "c1", TopologyEdgeKind::Related, "related"),
        ],
        exploration_exhausted: true,
        ..Default::default()
    }
}

fn spec(filters: ScopeFilters) -> ScopeSpec {
    ScopeSpec {
        seed_key: "claim:c1".into(),
        depth: 2,
        cycle: CycleScope::AllRetained,
        filters,
    }
}

#[test]
fn summary_explains_connections_and_scope_not_a_node_list() {
    let s = deterministic_summary(&snapshot(), &spec(ScopeFilters::default()));
    let text = s.to_text();
    assert!(text.starts_with("## In scope"), "{text}");
    assert!(
        text.contains("5 nodes") && text.contains("5 relationships"),
        "{text}"
    );
    assert!(
        text.contains("2 claims") && text.contains("1 actor"),
        "{text}"
    );
    assert!(text.contains("Inference premises: 2 connections"), "{text}");
    assert!(text.contains("Related (tentative context)"), "{text}");
    assert!(
        text.contains("Longest path from the seed (2 hops)"),
        "{text}"
    );
    assert!(
        text.contains("Related links are potential context"),
        "{text}"
    );
    // Never one line per node: the 5 labels are not each listed on their own line.
    assert!(text.lines().count() < 20, "{text}");
}

#[test]
fn summary_is_deterministic_and_covers_only_visible_nodes() {
    let snap = snapshot();
    let all = deterministic_summary(&snap, &spec(ScopeFilters::default()));
    assert_eq!(
        all,
        deterministic_summary(&snap, &spec(ScopeFilters::default()))
    );
    let no_sources_or_related = ScopeFilters {
        sources: false,
        related: false,
        ..ScopeFilters::default()
    };
    let narrowed = deterministic_summary(&snap, &spec(no_sources_or_related));
    assert_eq!(narrowed.node_count, 4);
    assert_eq!(narrowed.edge_count, 3);
    let text = narrowed.to_text();
    assert!(
        !text.contains("Sourced from") && !text.contains("Related (tentative"),
        "{text}"
    );
    assert!(
        narrowed.caveat_line().contains("1 hidden by filters"),
        "{text}"
    );
}

#[test]
fn isolated_seed_and_bounded_scope_are_stated() {
    let mut snap = TopologySnapshot {
        root_id: "c1".into(),
        root_label: "Lonely".into(),
        nodes: vec![node("c1", TopologyNodeKind::Claim, "Lonely", 0, None)],
        exploration_exhausted: true,
        ..Default::default()
    };
    let s = deterministic_summary(&snap, &spec(ScopeFilters::default()));
    assert!(
        s.to_text().contains("No recorded neighbors"),
        "{}",
        s.to_text()
    );
    snap.omitted_nodes = 4;
    snap.omitted_edges = 7;
    snap.unresolved_endpoints = vec!["x".into()];
    let s = deterministic_summary(&snap, &spec(ScopeFilters::default()));
    let caveats = s.caveat_line();
    assert!(
        caveats.contains("4 nodes and 7 relationships omitted"),
        "{caveats}"
    );
    assert!(caveats.contains("1 unresolved endpoint"), "{caveats}");
}

#[test]
fn scope_key_changes_on_recenter_depth_cycle_and_filters_only() {
    let base = spec(ScopeFilters::default());
    let key = base.key();
    assert_eq!(key, spec(ScopeFilters::default()).key());
    let mut recentered = base.clone();
    recentered.seed_key = "claim:c2".into();
    assert_ne!(key, recentered.key(), "recenter is a scope change");
    let mut deeper = base.clone();
    deeper.depth = 3;
    assert_ne!(key, deeper.key());
    let mut cycle = base.clone();
    cycle.cycle = CycleScope::ThisCycle("run-9".into());
    assert_ne!(key, cycle.key());
    let mut filtered = base.clone();
    filtered.filters.link = false;
    assert_ne!(key, filtered.key());
}

#[test]
fn graph_version_ignores_order_but_not_content_or_filters() {
    let snap = snapshot();
    let f = ScopeFilters::default();
    let v = graph_version(&snap, &f);
    let mut shuffled = snap.clone();
    shuffled.nodes.reverse();
    shuffled.edges.reverse();
    assert_eq!(v, graph_version(&shuffled, &f));
    let mut changed = snap.clone();
    changed.nodes[1].revision = "r2".into();
    assert_ne!(v, graph_version(&changed, &f));
    let mut seq = snap.clone();
    seq.memory_changed_seq = 99;
    assert_eq!(
        v,
        graph_version(&seq, &f),
        "unrelated churn does not change the version"
    );
    let narrowed = ScopeFilters {
        sources: false,
        ..f
    };
    assert_ne!(v, graph_version(&snap, &narrowed));
}

#[test]
fn digest_separates_scope_version_mode_and_model() {
    let d = |a, b, m, p, mo, pv| digest(a, b, m, p, mo, pv);
    let base = d("s", "g", MODE_AI, "openrouter", "m", PROMPT_VERSION);
    assert_eq!(
        base,
        d("s", "g", MODE_AI, "openrouter", "m", PROMPT_VERSION)
    );
    for other in [
        d("s2", "g", MODE_AI, "openrouter", "m", PROMPT_VERSION),
        d("s", "g2", MODE_AI, "openrouter", "m", PROMPT_VERSION),
        d("s", "g", "deterministic", "openrouter", "m", PROMPT_VERSION),
        d("s", "g", MODE_AI, "google", "m", PROMPT_VERSION),
        d("s", "g", MODE_AI, "openrouter", "m2", PROMPT_VERSION),
        d("s", "g", MODE_AI, "openrouter", "m", "v2"),
    ] {
        assert_ne!(base, other);
    }
}

fn row(digest: &str, scope: &str, seed: &str, seed_ref: &str, text: &str) -> TopologySummaryRow {
    TopologySummaryRow {
        digest: digest.into(),
        scope_key: scope.into(),
        seed_key: seed.into(),
        seed_ref: seed_ref.into(),
        graph_version: "g".into(),
        mode: MODE_AI.into(),
        summary: text.into(),
        provider: "openrouter".into(),
        model: "m".into(),
        prompt_version: PROMPT_VERSION.into(),
        node_count: 5,
        edge_count: 5,
        changed_seq: 7,
        ..Default::default()
    }
}

#[test]
fn saved_summaries_are_reused_by_digest_pruned_and_deleted_with_the_seed() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("argos.db")).unwrap();
    store
        .save_topology_summary(&row("d1", "scope-a", "claim:c1", "c1", "## A"))
        .unwrap();
    store
        .save_topology_summary(&row("d2", "scope-b", "claim:c1", "c1", "## B"))
        .unwrap();
    // Recenter away (scope-b) and back (scope-a): scope-a is still there.
    assert_eq!(
        store.topology_summary("d1").unwrap().unwrap().summary,
        "## A"
    );
    assert_eq!(
        store
            .topology_summary_for_scope("scope-b", MODE_AI)
            .unwrap()
            .unwrap()
            .digest,
        "d2"
    );
    assert!(store.topology_summary("missing").unwrap().is_none());
    assert!(store
        .save_topology_summary(&row("d3", "s", "claim:c1", "c1", "   "))
        .is_err());
    // Same digest overwrites instead of duplicating.
    store
        .save_topology_summary(&row("d1", "scope-a", "claim:c1", "c1", "## A2"))
        .unwrap();
    assert_eq!(store.topology_summary_count("claim:c1"), 2);
    // Another seed is untouched by deleting this seed's memory.
    store
        .save_topology_summary(&row("o1", "scope-o", "claim:other", "other", "## O"))
        .unwrap();
    assert_eq!(store.delete_topology_summaries_for_memory("c1").unwrap(), 2);
    assert!(store.topology_summary("d1").unwrap().is_none());
    assert!(store.topology_summary("o1").unwrap().is_some());
}

#[test]
fn each_seed_keeps_at_most_the_newest_fifty() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("argos.db")).unwrap();
    for i in 0..60 {
        store
            .save_topology_summary(&row(
                &format!("d{i}"),
                &format!("s{i}"),
                "claim:c1",
                "c1",
                "## x",
            ))
            .unwrap();
    }
    assert_eq!(
        store.topology_summary_count("claim:c1"),
        crate::store::TOPOLOGY_SUMMARY_MAX_PER_SEED
    );
}

#[test]
fn seed_memory_ref_is_the_memory_id_for_stored_seeds_only() {
    assert_eq!(TopologySeed::Claim("c1".into()).memory_ref(), "c1");
    assert_eq!(TopologySeed::Topic("t".into()).memory_ref(), "");
}
