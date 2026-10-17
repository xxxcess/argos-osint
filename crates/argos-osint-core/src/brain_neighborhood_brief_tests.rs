use super::*;
use crate::memory_metadata::{
    EndpointRef, InferencePayload, LinkPayload, MemoryKind, RelatedPayload,
};

fn node(id: &str, kind: TopologyNodeKind, label: &str, hops: usize) -> TopologyNode {
    TopologyNode {
        id: id.into(),
        kind,
        label: label.into(),
        detail: String::new(),
        subtype: String::new(),
        status: "recorded".into(),
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
        predecessor: None,
        topology_seed: None,
    }
}

fn edge(id: &str, from: &str, to: &str, kind: TopologyEdgeKind) -> TopologyEdge {
    TopologyEdge {
        id: id.into(),
        from: from.into(),
        to: to.into(),
        kind,
        label: kind.label().into(),
        status: "recorded".into(),
        member_ids: vec![],
        receipts: vec![format!("receipt-{id}")],
        rationale: String::new(),
    }
}

fn mem(id: &str) -> EndpointRef {
    EndpointRef::Memory {
        memory_id: id.into(),
        kind: MemoryKind::Claim,
    }
}

fn link(source: &str, target: &str, relation: &str, rationale: &str) -> MemoryPayload {
    MemoryPayload::Link(LinkPayload {
        source: mem(source),
        target: mem(target),
        relation: relation.into(),
        rationale: rationale.into(),
        receipts: vec![],
        promoted_from: None,
    })
}

fn inference(state: &str) -> MemoryPayload {
    MemoryPayload::Inference(InferencePayload {
        premise_claims: vec!["c1".into(), "c2".into(), "c9".into()],
        actors: vec![],
        reasoning: "Both claims name the same harbor operator".into(),
        limitations: "Only two retained articles; no shipping data".into(),
        support_state: state.into(),
        coverage_receipt: "coverage-1".into(),
    })
}

pub(crate) struct Fixture {
    pub(crate) snapshot: TopologySnapshot,
    pub(crate) payloads: BTreeMap<String, MemoryPayload>,
    pub(crate) seed: TopologySeed,
}

pub(crate) fn rich() -> Fixture {
    let mut c1 = node(
        "c1",
        TopologyNodeKind::Claim,
        "Port closed after the attack",
        0,
    );
    c1.status = "extraction assessed".into();
    let c2 = node(
        "c2",
        TopologyNodeKind::Claim,
        "Harbor traffic was rerouted",
        1,
    );
    let a1 = node("a1", TopologyNodeKind::Entity, "Harbor Authority", 1);
    let mut s1 = node("s1", TopologyNodeKind::Article, "Port Daily report", 1);
    s1.article_id = "art-1".into();
    s1.run_id = "run-1".into();
    let i1 = node(
        "i1",
        TopologyNodeKind::Inference,
        "Shipping will reroute",
        1,
    );
    let snapshot = TopologySnapshot {
        root_id: "c1".into(),
        root_label: c1.label.clone(),
        root_kind: Some(TopologyNodeKind::Claim),
        nodes: vec![c1, c2, a1, s1, i1],
        edges: vec![
            edge("act1", "a1", "c1", TopologyEdgeKind::Concerns),
            edge("act2", "a1", "c2", TopologyEdgeKind::Concerns),
            edge("src1", "s1", "c1", TopologyEdgeKind::Source),
            edge("src2", "s1", "c2", TopologyEdgeKind::Source),
            edge("p1", "c1", "i1", TopologyEdgeKind::Premise),
            edge("p2", "c2", "i1", TopologyEdgeKind::Premise),
            edge("l1", "c2", "c1", TopologyEdgeKind::Link),
        ],
        discovered_nodes: 5,
        discovered_edges: 7,
        total_nodes: 5,
        total_edges: 7,
        exploration_exhausted: true,
        ..Default::default()
    };
    let mut payloads = BTreeMap::new();
    payloads.insert("i1".into(), inference("supported"));
    payloads.insert(
        "l1".into(),
        link("c2", "c1", "contradicts", "Dates disagree"),
    );
    Fixture {
        snapshot,
        payloads,
        seed: TopologySeed::Claim("c1".into()),
    }
}

pub(crate) fn input_of(f: &Fixture) -> NeighborhoodInput {
    let b = BriefInput {
        snapshot: &f.snapshot,
        seed: &f.seed,
        depth: 2,
        cycle: &CycleScope::AllRetained,
        payloads: f.payloads.clone(),
        payload_reads_skipped: 0,
    };
    build_input(&b, &build(&b).basis_key)
}

pub(crate) fn brief_of(f: &Fixture) -> NeighborhoodBrief {
    build(&BriefInput {
        snapshot: &f.snapshot,
        seed: &f.seed,
        depth: 2,
        cycle: &CycleScope::AllRetained,
        payloads: f.payloads.clone(),
        payload_reads_skipped: 0,
    })
}

#[test]
fn rich_neighborhood_has_the_four_blocks_header_and_footer() {
    let f = rich();
    let b = brief_of(&f);
    assert_eq!(
        b.header(),
        "Port closed after the attack · Neighborhood summary"
    );
    assert_eq!(
        b.footer(),
        "Within 2 hops · all retained cycles · complete within bounds"
    );
    let names: Vec<_> = b.blocks().into_iter().map(|(n, _)| n).collect();
    assert_eq!(
        names,
        ["Overview", "Connections", "Interpretation", "Limits"]
    );
    assert!(b.connections.len() <= 2 && b.interpretation.len() == 1 && b.limits.len() == 1);
    let text = b.to_text();
    assert!(text.contains("connects to"), "{text}");
    // The recorded contradiction is pinned first, with its exact predicate.
    assert!(
        b.connections[0].text.starts_with("Recorded link: “Harbor traffic was rerouted” contradicts “Port closed after the attack”"),
        "{text}"
    );
    assert!(b.connections[0].text.contains("Dates disagree"), "{text}");
    // Existing inference: stored interpretation with reasoning and limitation.
    let i = &b.interpretation[0];
    assert_eq!(i.epistemic, Epistemic::StoredInference);
    assert!(i.text.contains("not a verified conclusion"), "{text}");
    assert!(
        i.text
            .contains("Reasoning: Both claims name the same harbor operator"),
        "{text}"
    );
    assert!(
        i.text.contains("Limitation: Only two retained articles"),
        "{text}"
    );
    assert_eq!(i.receipts, vec!["coverage-1".to_string()]);
    // Premise grounding: c9 is outside the loaded neighborhood.
    assert!(
        b.limits[0]
            .text
            .contains("lists 3 premise claims; 1 is outside"),
        "{text}"
    );
    assert!(b.word_count() <= 180, "{} words\n{text}", b.word_count());
    assert!(b.word_count() >= 60, "{text}");
}

#[test]
fn every_reference_is_typed_numbered_and_resolves_in_the_snapshot() {
    let f = rich();
    let b = brief_of(&f);
    assert_eq!(b.verify(&f.snapshot), Vec::<String>::new());
    assert!(!b.references.is_empty());
    for (i, r) in b.references.iter().enumerate() {
        assert_eq!(r.n, i + 1);
        match &r.target {
            BasisTarget::Node(id) => assert!(f.snapshot.nodes.iter().any(|n| &n.id == id)),
            BasisTarget::Edge(id) => assert!(f.snapshot.edges.iter().any(|e| &e.id == id)),
        }
    }
    for it in b.items() {
        assert!(!it.basis.is_empty(), "{}", it.text);
        assert_eq!(it.basis.len(), it.node_ids.len() + it.edge_ids.len());
    }
    assert!(b.to_text().contains("[1"), "{}", b.to_text());
    assert_eq!(refs_text(&[1, 2, 3, 5]), " [1–3, 5]");
    assert_eq!(refs_text(&[2, 1]), " [1, 2]");
    // A broken item is detected, not rendered silently.
    let mut broken = b.clone();
    broken.connections[0].node_ids.push("ghost".into());
    broken.connections[0].basis.push(99);
    assert_eq!(broken.verify(&f.snapshot).len(), 2);
}

#[test]
fn shared_actor_and_shared_source_are_explained_without_claiming_independence() {
    let mut f = rich();
    f.payloads.clear();
    f.snapshot
        .edges
        .retain(|e| e.kind != TopologyEdgeKind::Link);
    let b = brief_of(&f);
    let text = b.to_text();
    let all: Vec<&str> = b.connections.iter().map(|c| c.text.as_str()).collect();
    assert!(
        all.iter()
            .any(|t| t.contains("share actor “Harbor Authority”")),
        "{text}"
    );
    assert!(
        all.iter()
            .any(|t| t.contains("2 claims trace to retained article “Port Daily report”"))
            || b.connections.len() == 2,
        "{text}"
    );
    assert!(
        !text.to_lowercase().contains("independent"),
        "one article proves no independence: {text}"
    );
}

#[test]
fn duplicate_article_occurrences_merge_and_distinct_records_are_not_independence() {
    let mut f = rich();
    f.payloads.clear();
    f.snapshot
        .edges
        .retain(|e| e.kind == TopologyEdgeKind::Source);
    // A second revision of the same article is the same source path.
    let mut s1b = node(
        "s1b",
        TopologyNodeKind::Article,
        "Port Daily report (revised)",
        1,
    );
    s1b.article_id = "art-1".into();
    s1b.run_id = "run-1".into();
    f.snapshot.nodes.push(s1b);
    f.snapshot.edges.extend([
        edge("src3", "s1b", "c1", TopologyEdgeKind::Source),
        edge("src4", "s1b", "c2", TopologyEdgeKind::Source),
    ]);
    let b = brief_of(&f);
    let article: Vec<&BriefItem> = b
        .connections
        .iter()
        .filter(|c| c.text.contains("retained article"))
        .collect();
    assert_eq!(article.len(), 1, "{}", b.to_text());
    assert!(article[0]
        .text
        .starts_with("2 claims trace to retained article"));
    assert!(
        article[0].receipts.contains(&"receipt-src3".to_string()),
        "all occurrences keep their receipts"
    );
    // Different records, one claim each: counted as retained records, never as independence.
    let mut g = rich();
    g.payloads.clear();
    g.snapshot.edges = vec![
        edge("src1", "s1", "c1", TopologyEdgeKind::Source),
        edge("src5", "s2", "c2", TopologyEdgeKind::Source),
    ];
    g.snapshot
        .nodes
        .push(node("s2", TopologyNodeKind::Source, "Harbor wire", 1));
    let b = brief_of(&g);
    assert!(
        b.connections.iter().any(|c| c.text
            == "2 claims trace to 2 retained records, which are not independent confirmations."),
        "{}",
        b.to_text()
    );
}

#[test]
fn tentative_related_lead_is_shown_only_with_its_basis_and_gap() {
    let mut f = rich();
    f.snapshot
        .nodes
        .retain(|n| n.kind != TopologyNodeKind::Inference);
    f.snapshot
        .edges
        .retain(|e| e.kind != TopologyEdgeKind::Premise && e.kind != TopologyEdgeKind::Link);
    let mut r = edge("r1", "c1", "c2", TopologyEdgeKind::Related);
    r.status = "tentative".into();
    f.snapshot.edges.push(r);
    f.payloads.clear();
    f.payloads.insert(
        "r1".into(),
        MemoryPayload::Related(RelatedPayload {
            source: mem("c1"),
            target: mem("c2"),
            shared_actors: vec!["act-9".into()],
            basis: "Same operator named in both texts".into(),
            interpretation: "Possibly the same event".into(),
            verification_gap: "Operator identity is unverified".into(),
            source_revisions: vec![],
        }),
    );
    let b = brief_of(&f);
    assert_eq!(b.interpretation.len(), 1);
    let t = &b.interpretation[0];
    assert_eq!(t.epistemic, Epistemic::Tentative);
    assert!(
        t.text.contains("basis: Same operator named in both texts"),
        "{}",
        t.text
    );
    assert!(
        t.text
            .contains("Verification gap: Operator identity is unverified"),
        "{}",
        t.text
    );
    assert!(t.text.contains("not corroboration"), "{}", t.text);
    assert_eq!(t.member_ids, vec!["act-9".to_string()]);
    // Without a recorded basis or gap, nothing is invented.
    f.payloads.insert(
        "r1".into(),
        MemoryPayload::Related(RelatedPayload {
            source: mem("c1"),
            target: mem("c2"),
            shared_actors: vec![],
            basis: String::new(),
            interpretation: String::new(),
            verification_gap: String::new(),
            source_revisions: vec![],
        }),
    );
    assert!(brief_of(&f).interpretation.is_empty());
}

#[test]
fn no_inference_means_structural_connections_not_conclusions() {
    let mut f = rich();
    f.snapshot
        .nodes
        .retain(|n| n.kind != TopologyNodeKind::Inference);
    f.snapshot
        .edges
        .retain(|e| e.kind != TopologyEdgeKind::Premise);
    f.payloads.clear();
    let b = brief_of(&f);
    assert!(b.interpretation.is_empty());
    assert!(
        b.limits[0]
            .text
            .contains("No stored inference covers this neighborhood"),
        "{}",
        b.to_text()
    );
    for banned in [
        "coordinated",
        "controls",
        "responsible",
        "intends",
        "caused",
        "influence",
    ] {
        assert!(!b.to_text().to_lowercase().contains(banned), "{banned}");
    }
}

#[test]
fn isolated_seed_omits_every_inapplicable_block() {
    let f = Fixture {
        snapshot: TopologySnapshot {
            root_id: "c1".into(),
            root_label: "Lonely claim".into(),
            root_kind: Some(TopologyNodeKind::Claim),
            nodes: vec![node("c1", TopologyNodeKind::Claim, "Lonely claim", 0)],
            exploration_exhausted: true,
            ..Default::default()
        },
        payloads: BTreeMap::new(),
        seed: TopologySeed::Claim("c1".into()),
    };
    let b = brief_of(&f);
    assert_eq!(b.coverage.status, CoverageStatus::Isolated);
    assert!(b.connections.is_empty() && b.interpretation.is_empty() && b.limits.is_empty());
    let names: Vec<_> = b.blocks().into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["Overview"]);
    assert!(b
        .overview
        .as_ref()
        .unwrap()
        .text
        .contains("no recorded neighbors"));
    assert!(b.word_count() < 25);
    assert!(b.footer().ends_with("isolated"));
    let lower = b.to_text().to_lowercase();
    for word in ["unavailable", "unknown", "n/a", "none", "not recorded"] {
        assert!(!lower.contains(word), "{word}: {lower}");
    }
}

#[test]
fn missing_seed_says_so_and_nothing_else() {
    let snap = TopologySnapshot::default();
    let seed = TopologySeed::Claim("gone".into());
    let b = missing(&snap, &seed, 2, &CycleScope::AllRetained);
    assert_eq!(b.coverage.status, CoverageStatus::Missing);
    assert!(b.overview.is_none() && b.connections.is_empty());
    assert!(b.limits[0]
        .text
        .contains("missing, revised, or has no supported typed endpoint"));
    assert!(b.footer().ends_with("seed not found"));
}

#[test]
fn bounded_coverage_is_a_limit_and_never_a_claim_of_absence() {
    let mut f = rich();
    f.payloads.clear();
    f.snapshot
        .nodes
        .retain(|n| n.kind != TopologyNodeKind::Inference);
    f.snapshot
        .edges
        .retain(|e| e.kind != TopologyEdgeKind::Premise);
    f.snapshot.omitted_nodes = 4;
    f.snapshot.omitted_edges = 1;
    f.snapshot.unresolved_endpoints = vec!["x".into(), "y".into()];
    let b = brief_of(&f);
    assert_eq!(b.coverage.status, CoverageStatus::Limited);
    let l = &b.limits[0];
    assert_eq!(l.epistemic, Epistemic::Coverage);
    assert!(
        l.text
            .contains("4 nodes and 1 relationship omitted; 2 unresolved endpoints"),
        "{}",
        l.text
    );
    assert!(l.text.contains("not evidence of absence"), "{}", l.text);
    assert!(b.footer().ends_with("limited coverage"));
    assert_eq!(b.coverage.omitted_nodes, 4);
}

#[test]
fn identity_caveats_outrank_other_limits_and_names_never_merge_actors() {
    let mut f = rich();
    f.snapshot.nodes[2].status = "ambiguous".into();
    let b = brief_of(&f);
    assert!(b.limits[0].text.contains("Identity of actor “Harbor Authority” is ambiguous; equal names do not establish identity."), "{}", b.limits[0].text);
    assert!(
        b.connections
            .iter()
            .any(|c| c.text.contains("identity ambiguous")),
        "{}",
        b.to_text()
    );
    // An actor group is a display group with separate members.
    let mut g = node("actor-group:acme", TopologyNodeKind::ActorGroup, "Acme", 0);
    g.member_ids = vec!["m1".into(), "m2".into()];
    let f = Fixture {
        snapshot: TopologySnapshot {
            root_id: g.id.clone(),
            root_label: "Acme".into(),
            root_kind: Some(TopologyNodeKind::ActorGroup),
            nodes: vec![
                g,
                node("c1", TopologyNodeKind::Claim, "Acme won the bid", 1),
                node("c2", TopologyNodeKind::Claim, "Acme lost the bid", 1),
            ],
            edges: vec![
                edge("e1", "actor-group:acme", "c1", TopologyEdgeKind::Concerns),
                edge("e2", "actor-group:acme", "c2", TopologyEdgeKind::Concerns),
            ],
            exploration_exhausted: true,
            ..Default::default()
        },
        payloads: BTreeMap::new(),
        seed: TopologySeed::ActorGroup {
            name_key: "acme".into(),
            member_ids: vec!["m1".into(), "m2".into()],
        },
    };
    let b = brief_of(&f);
    assert!(
        b.connections[0]
            .text
            .contains("a name group, members stay unmerged"),
        "{}",
        b.to_text()
    );
    assert_eq!(
        b.connections[0].member_ids,
        vec!["m1".to_string(), "m2".to_string()]
    );
    assert!(
        b.limits[0]
            .text
            .contains("display group of separate actor bindings"),
        "{}",
        b.to_text()
    );
}

#[test]
fn shared_topic_is_not_agreement() {
    let mut f = rich();
    f.payloads.clear();
    f.snapshot.edges.clear();
    f.snapshot
        .nodes
        .retain(|n| matches!(n.kind, TopologyNodeKind::Claim));
    f.snapshot
        .nodes
        .push(node("t1", TopologyNodeKind::Topic, "Supply chain", 1));
    f.snapshot.edges = vec![
        edge("t-a", "c1", "t1", TopologyEdgeKind::Concerns),
        edge("t-b", "c2", "t1", TopologyEdgeKind::Concerns),
    ];
    let b = brief_of(&f);
    assert!(b.connections[0].text.contains("share stored topic “Supply chain”; topic membership does not establish agreement or a relationship"), "{}", b.to_text());
}

#[test]
fn link_seed_leads_with_the_relationship_and_both_endpoints() {
    let mut f = rich();
    f.snapshot.nodes[1].is_root = true;
    f.snapshot.nodes[1].min_hops = 0;
    f.seed = TopologySeed::Link("l1".into());
    let b = brief_of(&f);
    assert_eq!(
        b.header(),
        "Port closed after the attack → Harbor traffic was rerouted · Neighborhood summary"
    );
    let o = b.overview.as_ref().unwrap();
    assert_eq!(
        o.text,
        "The recorded link says “Harbor traffic was rerouted” contradicts “Port closed after the attack”."
    );
    assert_eq!(o.edge_ids, vec!["l1".to_string()]);
    assert_eq!(o.node_ids.len(), 2);
    // `mentions` is never presented as `supports`.
    f.payloads
        .insert("l1".into(), link("c2", "c1", "mentions", ""));
    let b = brief_of(&f);
    assert!(b.overview.unwrap().text.contains("mentions"));
}

#[test]
fn related_seed_is_potential_context_only() {
    let mut f = rich();
    f.snapshot
        .nodes
        .retain(|n| n.kind == TopologyNodeKind::Claim);
    f.snapshot.nodes[1].is_root = true;
    f.snapshot.nodes[1].min_hops = 0;
    let mut r = edge("r1", "c1", "c2", TopologyEdgeKind::Related);
    r.status = "tentative".into();
    f.snapshot.edges = vec![r];
    f.payloads.clear();
    f.seed = TopologySeed::Related("r1".into());
    let b = brief_of(&f);
    let o = b.overview.unwrap();
    assert_eq!(o.epistemic, Epistemic::Tentative);
    assert!(o.text.contains("potential context only"), "{}", o.text);
}

#[test]
fn long_text_is_shortened_to_the_word_target_and_references_survive() {
    let mut f = rich();
    let long = "reasoning words ".repeat(60);
    f.payloads.insert(
        "i1".into(),
        MemoryPayload::Inference(InferencePayload {
            premise_claims: vec!["c1".into(), "c2".into()],
            reasoning: long.clone(),
            limitations: long,
            support_state: "supported".into(),
            ..Default::default()
        }),
    );
    let b = brief_of(&f);
    assert!(
        b.word_count() <= 180,
        "{} words\n{}",
        b.word_count(),
        b.to_text()
    );
    assert!(!b.interpretation[0].basis.is_empty());
    assert_eq!(b.verify(&f.snapshot), Vec::<String>::new());
}

#[test]
fn equal_inputs_give_equal_text_and_basis_key_and_changes_move_it() {
    let f = rich();
    let a = brief_of(&f);
    assert_eq!(a, brief_of(&f));
    let mut shuffled = rich();
    shuffled.snapshot.nodes.reverse();
    shuffled.snapshot.edges.reverse();
    assert_eq!(a, brief_of(&shuffled), "input order never matters");
    assert!(!a.basis_key.is_empty());
    let mut renamed = rich();
    renamed.snapshot.nodes[1].label = "Harbor traffic rerouted".into();
    assert_ne!(a.basis_key, brief_of(&renamed).basis_key);
    let mut cov = rich();
    cov.snapshot.omitted_nodes = 1;
    assert_ne!(a.basis_key, brief_of(&cov).basis_key);
    let mut payload = rich();
    payload.payloads.insert("i1".into(), inference("rejected"));
    assert_ne!(a.basis_key, brief_of(&payload).basis_key);
    // Depth is part of the basis; selection and viewport are not inputs at all.
    let deeper = build(&BriefInput {
        snapshot: &f.snapshot,
        seed: &f.seed,
        depth: 3,
        cycle: &CycleScope::AllRetained,
        payloads: f.payloads.clone(),
        payload_reads_skipped: 0,
    });
    assert_ne!(a.basis_key, deeper.basis_key);
}

#[test]
fn contradiction_and_identity_are_pinned_ahead_of_supportive_detail() {
    let mut f = rich();
    // Many supportive patterns exist, yet the contradiction stays first.
    f.snapshot
        .nodes
        .push(node("t1", TopologyNodeKind::Topic, "Supply chain", 1));
    f.snapshot
        .edges
        .push(edge("t-a", "c1", "t1", TopologyEdgeKind::Concerns));
    f.snapshot
        .edges
        .push(edge("t-b", "c2", "t1", TopologyEdgeKind::Concerns));
    let b = brief_of(&f);
    assert_eq!(b.connections.len(), 2);
    assert!(
        b.connections[0].text.contains("contradicts"),
        "{}",
        b.to_text()
    );
}

// ------------------------------------------------- generated output validation

fn good_output() -> String {
    serde_json::json!({
        "overview": [{"text": "This claim concerns a port closure and sits among related claims, an actor and a source.", "basis": ["N1"]}],
        "connections": [{"text": "The Harbor Authority is referenced by both claims, and a recorded link says the second claim contradicts the first.", "basis": ["N2", "E3"]}],
        "interpretation": [{"text": "A stored inference with support state supported reads the two claims together; it is a stored interpretation, not a verified conclusion.", "basis": ["N4"]}],
        "limits": [{"text": "One premise of the stored inference is outside this neighborhood, and links not shown are not evidence of absence.", "basis": []}]
    })
    .to_string()
}

fn with(block: &str, text: &str, basis: &[&str]) -> String {
    let mut v: serde_json::Value = serde_json::from_str(&good_output()).unwrap();
    v[block] = serde_json::json!([{"text": text, "basis": basis}]);
    v.to_string()
}

fn check(raw: &str) -> Result<Validated, String> {
    let f = rich();
    validate_generated(&input_of(&f), &brief_of(&f), raw)
}

#[test]
fn packet_refs_are_stable_typed_and_frozen() {
    let f = rich();
    let a = input_of(&f);
    let b = input_of(&f);
    assert_eq!(a, b);
    assert_eq!(a.nodes[0].reference, "N1");
    assert!(a.nodes[0].node.is_root);
    assert_eq!(a.edges[2].edge.id, "l1");
    assert_eq!(a.edges[2].reference, "E3");
    assert!(a.worth_generating());
    let json = a.prompt_json();
    assert!(json.contains("\"ref\":\"E3\"") && json.contains("contradicts"));
}

#[test]
fn a_faithful_answer_validates_with_typed_references_and_host_owned_fields() {
    let f = rich();
    let v = check(&good_output()).unwrap();
    let fb = brief_of(&f);
    assert_eq!(v.brief.basis_key, fb.basis_key);
    assert_eq!(v.brief.coverage, fb.coverage);
    assert_eq!(v.brief.seed, fb.seed);
    assert_eq!(
        v.brief.interpretation[0].epistemic,
        Epistemic::StoredInference
    );
    assert!(!v.brief.references.is_empty());
    assert!(v.brief.connections[0].edge_ids.contains(&"l1".to_string()));
    assert!(v.host_notes.is_empty());
    assert!(v.brief.verify(&f.snapshot).is_empty());
}

#[test]
fn empty_malformed_and_unknown_shapes_are_rejected() {
    assert!(check("").unwrap_err().contains("empty"));
    assert!(check("not json").unwrap_err().contains("malformed"));
    assert!(check("{\"extra\": []}").is_err());
    assert!(
        check("{\"overview\":[],\"connections\":[],\"interpretation\":[],\"limits\":[]}")
            .unwrap_err()
            .contains("no meaningful content")
    );
    assert!(check(&with("connections", "ok", &["N2"]))
        .unwrap_err()
        .contains("length"));
}

#[test]
fn unknown_references_and_unreferenced_claims_are_rejected() {
    let bad = with(
        "connections",
        "The Harbor Authority is referenced by both claims here.",
        &["N99"],
    );
    assert!(check(&bad).unwrap_err().contains("unknown reference"));
    let none = with(
        "connections",
        "The Harbor Authority is referenced by both claims here.",
        &[],
    );
    assert!(check(&none).unwrap_err().contains("without a basis"));
}

#[test]
fn unsupported_assertions_numbers_and_quotes_are_rejected() {
    let a = with(
        "connections",
        "The Harbor Authority coordinated the closure of the port.",
        &["N2"],
    );
    assert!(check(&a).unwrap_err().contains("unsupported assertion"));
    let n = with(
        "connections",
        "The Harbor Authority appears across 47 separate claims here.",
        &["N2"],
    );
    assert!(check(&n).unwrap_err().contains("number 47"));
    let q = with(
        "connections",
        "The Harbor Authority issued a “moon base order” in this period.",
        &["N2"],
    );
    assert!(check(&q).unwrap_err().contains("quoted text"));
    // The same word negated is a caveat, not an assertion.
    let ok = with(
        "connections",
        "Shared references do not prove coordination between the two claims.",
        &["N2"],
    );
    assert!(check(&ok).is_ok());
}

#[test]
fn inferences_must_be_attributed_and_interpretation_needs_one() {
    let unattributed = with(
        "connections",
        "The two claims point toward one shipping outcome overall.",
        &["N4"],
    );
    assert!(check(&unattributed).unwrap_err().contains("not attributed"));
    let wrong = with(
        "interpretation",
        "The Harbor Authority is referenced by both claims here.",
        &["N2"],
    );
    assert!(check(&wrong).unwrap_err().contains("interpretation"));
}

#[test]
fn relation_direction_is_checked() {
    let mut f = rich();
    f.payloads
        .insert("l1".into(), link("c2", "c1", "supports", "Dates agree"));
    let input = input_of(&f);
    let fb = brief_of(&f);
    let reversed = with(
        "connections",
        "Port closed after the attack supports Harbor traffic was rerouted.",
        &["E3"],
    );
    assert!(validate_generated(&input, &fb, &reversed)
        .unwrap_err()
        .contains("direction reversed"));
    let right = with(
        "connections",
        "Harbor traffic was rerouted supports Port closed after the attack.",
        &["E3"],
    );
    assert!(validate_generated(&input, &fb, &right).is_ok());
    let passive = with(
        "connections",
        "Port closed after the attack is supported by Harbor traffic was rerouted.",
        &["E3"],
    );
    assert!(validate_generated(&input, &fb, &passive).is_ok());
}

#[test]
fn pinned_contradiction_and_required_limit_are_restored_by_the_host() {
    let v = check(&with(
        "connections",
        "The Harbor Authority is referenced by both claims in this neighborhood.",
        &["N2"],
    ))
    .unwrap();
    assert!(v
        .brief
        .connections
        .iter()
        .any(|i| i.edge_ids.contains(&"l1".to_string())));
    assert!(v.host_notes.iter().any(|n| n.contains("contradiction")));
    let v = check(&with(
        "limits",
        "Nothing further needs to be said about this area.",
        &[],
    ))
    .unwrap();
    assert!(v.host_notes.iter().any(|n| n.contains("required limit")));
    assert!(!v.brief.limits[0].text.contains("Nothing further"));
}

#[test]
fn fenced_json_is_accepted_and_overlong_output_is_not() {
    assert!(check(&format!("```json\n{}\n```", good_output())).is_ok());
    let long = "word ".repeat(70);
    let mut v: serde_json::Value = serde_json::from_str(&good_output()).unwrap();
    v["overview"] = serde_json::json!([{"text": long.clone(), "basis": ["N1"]}]);
    v["connections"] = serde_json::json!([
        {"text": long.clone(), "basis": ["N2"]},
        {"text": long.clone(), "basis": ["N3"]}
    ]);
    v["interpretation"] =
        serde_json::json!([{"text": format!("stored inference {long}"), "basis": ["N4"]}]);
    assert!(check(&v.to_string()).unwrap_err().contains("too long"));
}
