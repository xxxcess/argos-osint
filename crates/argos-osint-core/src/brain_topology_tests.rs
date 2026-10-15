use super::*;
use tempfile::tempdir;

use crate::brain::MemorySource;
use crate::brain_actors::{self as actors, BindRequest};
use crate::memory_metadata::{EndpointRef, LinkPayload, MemoryKind};
use crate::store::Store;

fn test_store() -> (tempfile::TempDir, Store) {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.db");
    let store = Store::open(&path).unwrap();
    (dir, store)
}

#[test]
fn topology_snapshot_claim_seed_and_associations() {
    let (_dir, store) = test_store();

    // 1. Add user claim
    let claim = store
        .add_user_claim(
            "Acme Corp approved the project in Chicago",
            "fact",
            MemorySource {
                app: "test".into(),
                conversation_id: "c1".into(),
                message_id: None,
                reference: None,
            },
        )
        .unwrap();

    // 2. Bind an actor to the claim
    let now = "2026-10-14T20:00:00Z";
    let (binding, _) = actors::bind(
        &store.conn,
        &BindRequest {
            name: "Acme Corp",
            subtype: actors::parse_subtype("organization"),
            scope: "global",
            identifiers: &[],
            external: None,
            provenance: serde_json::json!({"source": "test"}),
        },
        now,
    )
    .unwrap();

    store
        .conn
        .execute(
            "INSERT INTO brain_memory_actor_refs(memory_id, actor_id, role, binding_state) VALUES(?1, ?2, 'subject', 'resolved')",
            rusqlite::params![claim.id, binding.actor_id],
        )
        .unwrap();

    // 3. Query topology snapshot
    let snap = store
        .topology_snapshot(
            &TopologySeed::Claim(claim.id.clone()),
            &TopologyOptions::default(),
        )
        .unwrap();

    assert_eq!(snap.root_id, claim.id);
    assert_eq!(snap.root_kind, Some(TopologyNodeKind::Claim));
    assert!(snap
        .nodes
        .iter()
        .any(|n| n.kind == TopologyNodeKind::Claim && n.id == claim.id));
    assert!(snap
        .nodes
        .iter()
        .any(|n| n.kind == TopologyNodeKind::Entity && n.label == "Acme Corp"));
    assert!(snap
        .edges
        .iter()
        .any(|e| e.kind == TopologyEdgeKind::Concerns));
}

#[test]
fn topology_snapshot_actor_group_preserves_member_bindings_without_identity() {
    let (_dir, store) = test_store();
    let now = "2026-10-14T20:00:00Z";

    let (binding1, _) = actors::bind(
        &store.conn,
        &BindRequest {
            name: "Northstar Labs",
            subtype: actors::parse_subtype("organization"),
            scope: "global",
            identifiers: &[],
            external: None,
            provenance: serde_json::json!({}),
        },
        now,
    )
    .unwrap();

    let (binding2, _) = actors::bind(
        &store.conn,
        &BindRequest {
            name: "Northstar Labs",
            subtype: actors::parse_subtype("organization"),
            scope: "cycle-12",
            identifiers: &[],
            external: None,
            provenance: serde_json::json!({}),
        },
        now,
    )
    .unwrap();

    let groups = store.actor_groups().unwrap();
    let group = groups
        .into_iter()
        .find(|g| g.display_name == "Northstar Labs")
        .unwrap();
    assert!(group.actor_ids.contains(&binding1.actor_id));
    assert!(group.actor_ids.contains(&binding2.actor_id));

    let snap = store
        .topology_snapshot(
            &TopologySeed::ActorGroup {
                name_key: group.name_key.clone(),
                member_ids: group.actor_ids.clone(),
            },
            &TopologyOptions::default(),
        )
        .unwrap();

    assert_eq!(snap.root_id, group.key());
    assert_eq!(snap.root_kind, Some(TopologyNodeKind::ActorGroup));
    let root = snap.nodes.iter().find(|n| n.is_root).unwrap();
    assert_eq!(root.member_ids.len(), 2);
    // Aggregate key never stored in memories table
    assert!(store.get_memory(&group.key()).unwrap().is_none());
}

#[test]
fn topology_snapshot_dispatches_memory_endpoints_without_miscasting() {
    let (_dir, store) = test_store();

    // Claim 1
    let c1 = store
        .add_user_claim(
            "Approval confirmed for rollout",
            "fact",
            MemorySource {
                app: "test".into(),
                conversation_id: "c1".into(),
                message_id: None,
                reference: None,
            },
        )
        .unwrap();

    // Claim 2
    let c2 = store
        .add_user_claim(
            "Pilot approved by board",
            "fact",
            MemorySource {
                app: "test".into(),
                conversation_id: "c2".into(),
                message_id: None,
                reference: None,
            },
        )
        .unwrap();

    // Link between C1 and C2
    let now = "2026-10-14T20:00:00Z";
    let (link_id, _) = actors::write_link(
        &store.conn,
        &LinkPayload {
            source: EndpointRef::Memory {
                memory_id: c1.id.clone(),
                kind: MemoryKind::Claim,
            },
            target: EndpointRef::Memory {
                memory_id: c2.id.clone(),
                kind: MemoryKind::Claim,
            },
            relation: "supports".into(),
            rationale: "The board approved the pilot.".into(),
            receipts: vec![],
            promoted_from: None,
        },
        now,
    )
    .unwrap();

    // Query topology snapshot for the link
    let snap = store
        .topology_snapshot(
            &TopologySeed::Link(link_id.clone()),
            &TopologyOptions::default(),
        )
        .unwrap();

    assert_eq!(snap.edges.len(), 1);
    let edge = &snap.edges[0];
    assert_eq!(edge.id, link_id);
    assert_eq!(edge.kind, TopologyEdgeKind::Link);
    assert_eq!(edge.label, "supports");

    // Both endpoints are Claims, NOT miscast as Actors
    let s_node = snap.nodes.iter().find(|n| n.id == c1.id).unwrap();
    let t_node = snap.nodes.iter().find(|n| n.id == c2.id).unwrap();
    assert_eq!(s_node.kind, TopologyNodeKind::Claim);
    assert_eq!(t_node.kind, TopologyNodeKind::Claim);
}

#[test]
fn topology_snapshot_depth_and_continuation_bounds() {
    let (_dir, store) = test_store();

    let c1 = store
        .add_user_claim(
            "Root claim node",
            "fact",
            MemorySource {
                app: "test".into(),
                conversation_id: "c1".into(),
                message_id: None,
                reference: None,
            },
        )
        .unwrap();

    let options = TopologyOptions {
        depth: 2,
        cycle_scope: CycleScope::AllRetained,
        max_nodes: 1, // Cap to 1 to force continuation
        max_edges: 10,
        continuation_token: None,
    };

    let snap = store
        .topology_snapshot(&TopologySeed::Claim(c1.id.clone()), &options)
        .unwrap();

    assert_eq!(snap.nodes.len(), 1);
}

fn claim(store: &Store, text: &str) -> String {
    store
        .add_user_claim(
            text,
            "fact",
            MemorySource {
                app: "fixture".into(),
                conversation_id: text.into(),
                message_id: None,
                reference: None,
            },
        )
        .unwrap()
        .id
}

fn actor(store: &Store, name: &str, scope: &str) -> String {
    store
        .bind_actor(&BindRequest {
            name,
            subtype: actors::parse_subtype("organization"),
            scope,
            identifiers: &[],
            external: None,
            provenance: serde_json::json!({"fixture":scope}),
        })
        .unwrap()
        .actor_id
}

fn connect(store: &Store, source: &str, target: &str) -> String {
    store
        .write_link_memory(&LinkPayload {
            source: EndpointRef::Memory {
                memory_id: source.into(),
                kind: MemoryKind::Claim,
            },
            target: EndpointRef::Memory {
                memory_id: target.into(),
                kind: MemoryKind::Claim,
            },
            relation: "supports".into(),
            rationale: "Fixture assessed relation".into(),
            receipts: vec!["exact-fixture-receipt".into()],
            promoted_from: None,
        })
        .unwrap()
}

fn article(store: &Store, run: &str, id: &str) -> crate::store::AtlasArticleRow {
    let row = crate::store::AtlasArticleRow {
        run_id: run.into(),
        id: id.into(),
        title: format!("Article {id}"),
        url: format!("https://fixture.test/{id}"),
        category: "unk".into(),
        ..Default::default()
    };
    store.atlas_upsert_article(&row).unwrap();
    row
}

fn contains(snapshot: &TopologySnapshot, id: &str) -> bool {
    snapshot.nodes.iter().any(|n| n.id == id)
}

fn assert_closed(snapshot: &TopologySnapshot) {
    for e in &snapshot.edges {
        assert!(contains(snapshot, &e.from), "missing source {}", e.id);
        assert!(contains(snapshot, &e.to), "missing target {}", e.id);
    }
    for n in &snapshot.nodes {
        if let Some(parent) = &n.predecessor {
            assert!(contains(snapshot, parent));
            assert!(snapshot.edges.iter().any(
                |e| (e.from == n.id && e.to == *parent) || (e.to == n.id && e.from == *parent)
            ));
        }
    }
}

#[test]
fn topology_two_hops_are_real_scoped_paths_and_an_orphan_stays_an_orphan() {
    let s = Store::memory().unwrap();
    let a = claim(&s, "Alpha root");
    let b = claim(&s, "Alpha first neighbour");
    let c = claim(&s, "Alpha second neighbour");
    let d = claim(&s, "Alpha third-hop decoy");
    let orphan = claim(&s, "Separate isolated root");
    connect(&s, &a, &b);
    connect(&s, &b, &c);
    connect(&s, &c, &d);
    let snap = s
        .topology_snapshot(&TopologySeed::Claim(a.clone()), &TopologyOptions::default())
        .unwrap();
    assert_eq!(TopologyOptions::default().depth, 2);
    assert!(contains(&snap, &a) && contains(&snap, &b) && contains(&snap, &c));
    assert!(!contains(&snap, &d) && !contains(&snap, &orphan));
    assert_eq!(snap.nodes.iter().find(|n| n.id == c).unwrap().min_hops, 2);
    assert_closed(&snap);
    let one = s
        .topology_snapshot(
            &TopologySeed::Claim(a.clone()),
            &TopologyOptions {
                depth: 1,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(!contains(&one, &c));
    let three = s
        .topology_snapshot(
            &TopologySeed::Claim(a),
            &TopologyOptions {
                depth: 3,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(contains(&three, &d));
    let isolated = s
        .topology_snapshot(
            &TopologySeed::Claim(orphan.clone()),
            &TopologyOptions::default(),
        )
        .unwrap();
    assert_eq!(isolated.nodes.len(), 1);
    assert_eq!(isolated.root_id, orphan);
    assert!(isolated.edges.is_empty());
    assert!(s
        .topology_snapshot(
            &TopologySeed::Claim("deleted-or-never-existed".into()),
            &TopologyOptions::default()
        )
        .unwrap()
        .nodes
        .is_empty());
}

#[test]
fn topology_exact_actor_and_explicit_group_do_not_union_namesakes() {
    let s = Store::memory().unwrap();
    let a = actor(&s, "Same Name", "scope-a");
    let b = actor(&s, "Same Name", "scope-b");
    let ca = claim(&s, "Only actor A said this");
    let cb = claim(&s, "Only actor B said this");
    for (mid, aid) in [(&ca, &a), (&cb, &b)] {
        actors::add_memory_actor_ref(
            &s.conn,
            mid,
            aid,
            "subject",
            crate::memory_metadata::IdentityState::Provisional,
            &serde_json::json!({"binding":aid}),
        )
        .unwrap();
    }
    let exact = s
        .topology_snapshot(&TopologySeed::Actor(a.clone()), &TopologyOptions::default())
        .unwrap();
    assert_eq!(exact.root_id, a);
    assert!(contains(&exact, &ca));
    assert!(!contains(&exact, &b) && !contains(&exact, &cb));
    let group = s
        .topology_snapshot(
            &TopologySeed::ActorGroup {
                name_key: "same name".into(),
                member_ids: vec![a.clone()],
            },
            &TopologyOptions::default(),
        )
        .unwrap();
    assert_eq!(group.root_kind, Some(TopologyNodeKind::ActorGroup));
    assert_eq!(group.nodes[0].member_ids, vec![a]);
    assert!(!contains(&group, &b) && !contains(&group, &cb));
    assert_closed(&group);
}

#[test]
fn topology_direct_mentions_and_typed_article_evidence_need_no_related_pairs() {
    let s = Store::memory().unwrap();
    let a = actor(&s, "Isolated witness", "article-scope");
    let c = claim(&s, "A claim with direct retained article evidence");
    let row = article(&s, "cycle-a", "article-a");
    let revision = crate::atlas_metadata::input_hash(&row);
    actors::record_mention(
        &s.conn,
        &a,
        "article",
        "cycle-a",
        "article-a",
        &revision,
        &serde_json::json!([{"start":0}]),
        "2026-10-15",
    )
    .unwrap();
    let lid = s
        .write_link_memory(&LinkPayload {
            source: EndpointRef::Article {
                run_id: row.run_id.clone(),
                article_id: row.id.clone(),
                revision: revision.clone(),
            },
            target: EndpointRef::Memory {
                memory_id: c.clone(),
                kind: MemoryKind::Claim,
            },
            relation: "supports".into(),
            rationale: "Exact retained article supports extraction".into(),
            receipts: vec!["passage:3@revision".into()],
            promoted_from: None,
        })
        .unwrap();
    let snap = s
        .topology_snapshot(&TopologySeed::Actor(a), &TopologyOptions::default())
        .unwrap();
    assert!(contains(&snap, "article:cycle-a/article-a") && contains(&snap, &c));
    let e = snap.edges.iter().find(|e| e.id == lid).unwrap();
    assert_eq!(e.from, "article:cycle-a/article-a");
    assert_eq!(e.to, c);
    assert!(e.receipts.contains(&"passage:3@revision".into()));
    assert_closed(&snap);
    assert_eq!(
        s.conn
            .query_row("SELECT COUNT(*) FROM atlas_related_articles", [], |r| r
                .get::<_, usize>(
                0
            ))
            .unwrap(),
        0
    );
    let wrong = s
        .topology_snapshot(
            &TopologySeed::Article {
                run_id: row.run_id,
                article_id: row.id,
                revision: "obsolete".into(),
                originating_memory: None,
            },
            &TopologyOptions::default(),
        )
        .unwrap();
    assert!(wrong.nodes.is_empty());
    assert!(!wrong.unresolved_endpoints.is_empty());
}

#[test]
fn topology_inference_premises_and_topic_attributes_are_visible_and_reversible() {
    let s = Store::memory().unwrap();
    let c = claim(&s, "The shipment was delayed");
    let inf = claim(&s, "The delivery could miss its deadline");
    let payload = crate::memory_metadata::MemoryPayload::Inference(
        crate::memory_metadata::InferencePayload {
            premise_claims: vec![c.clone()],
            reasoning: "Delay may affect delivery".into(),
            limitations: "Schedule may recover".into(),
            support_state: "assessed".into(),
            ..Default::default()
        },
    );
    s.conn.execute("UPDATE memory_metadata SET memory_kind='inference',payload_json=?2,payload_schema=1 WHERE memory_id=?1",rusqlite::params![inf,serde_json::to_string(&payload).unwrap()]).unwrap();
    s.conn.execute("INSERT INTO insight_claims(fingerprint,memory_id,entity_id,predicate,object_value,topic,classification,confidence,created_at,updated_at) VALUES('topic-fp',?1,'','p','o','Supply chain','claim',0,'fixture','fixture')",[&c]).unwrap();
    let snap = s
        .topology_snapshot(&TopologySeed::Claim(c.clone()), &TopologyOptions::default())
        .unwrap();
    let i = snap.nodes.iter().find(|n| n.id == inf).unwrap();
    assert_eq!(i.kind, TopologyNodeKind::Inference);
    assert!(i.detail.contains("Schedule may recover"));
    assert!(snap
        .edges
        .iter()
        .any(|e| e.kind == TopologyEdgeKind::Premise && e.from == c && e.to == inf));
    let topic = snap
        .nodes
        .iter()
        .find(|n| n.kind == TopologyNodeKind::Topic)
        .unwrap();
    assert!(snap
        .edges
        .iter()
        .any(|e| e.to == topic.id && e.receipts.iter().any(|r| r.contains("Supply chain"))));
    let reverse = s
        .topology_snapshot(
            topic.topology_seed.as_ref().unwrap(),
            &TopologyOptions::default(),
        )
        .unwrap();
    assert!(contains(&reverse, &c) && contains(&reverse, &inf));
    assert_closed(&reverse);
}

fn snapshot_receipt(s: &Store, snapshot: &str, owner: &str) {
    let input = if let Some(article) = s.atlas_latest_article_by_id(owner).unwrap() {
        let artifact = crate::nlp::cache::artifact(
            &s.conn,
            &format!("{}\n{}", article.title, article.description),
        )
        .unwrap();
        serde_json::json!({"source_refs":[{"source_id":format!("article:{owner}"),"artifact_hash":artifact,"revision":artifact,"url":article.url,"published_at":article.published_at}]})
    } else {
        serde_json::json!({})
    };
    s.conn.execute("INSERT INTO insight_snapshots(id,owner_kind,owner_id,generation,input_key,inputs_json,entities_json,coverage_json,readiness_json,contract_version,policy_version,created_at) VALUES(?1,'article',?2,1,?1,?3,'[]','[]','{}','fixture','fixture','fixture')",rusqlite::params![snapshot,owner,input.to_string()]).unwrap();
}

fn derivation(
    s: &Store,
    snapshot: &str,
    local: &str,
    memory: &str,
    kind: &str,
    item: serde_json::Value,
) {
    s.conn.execute("INSERT INTO insight_derivations(id,snapshot_id,memory_id,insight_kind,fingerprint,item_json,assessment_json,status,revision) VALUES(?1,?2,?3,?4,?1,?5,'{}','assessed','fixture-revision')",rusqlite::params![format!("{snapshot}:{local}"),snapshot,memory,kind,item.to_string()]).unwrap();
}

#[test]
fn topology_derivation_premises_resolve_snapshot_local_ids_without_claim_casts() {
    let s = Store::memory().unwrap();
    let c1 = claim(&s, "Snapshot A premise");
    let c2 = claim(&s, "Snapshot B namesake premise");
    let i = claim(&s, "Snapshot A derived inference");
    s.conn.execute("UPDATE memory_metadata SET memory_kind='inference',payload_schema=0 WHERE memory_id=?1",[&i]).unwrap();
    snapshot_receipt(&s, "snapshot-a", "a");
    snapshot_receipt(&s, "snapshot-b", "b");
    derivation(
        &s,
        "snapshot-a",
        "c1",
        &c1,
        "claim",
        serde_json::json!({"id":"c1"}),
    );
    derivation(
        &s,
        "snapshot-b",
        "c1",
        &c2,
        "claim",
        serde_json::json!({"id":"c1"}),
    );
    derivation(
        &s,
        "snapshot-a",
        "i1",
        &i,
        "inference",
        serde_json::json!({"id":"i1","premise_claim_ids":["c1"],"rationale":"Correct snapshot rationale","limitations":["one source"]}),
    );
    let snap = s
        .topology_snapshot(
            &TopologySeed::Inference(i.clone()),
            &TopologyOptions::default(),
        )
        .unwrap();
    assert!(contains(&snap, &c1) && !contains(&snap, &c2));
    assert!(snap.nodes[0].detail.contains("Correct snapshot rationale"));
    assert!(snap
        .edges
        .iter()
        .any(|e| e.kind == TopologyEdgeKind::Premise && e.from == c1 && e.to == i));
    let reverse = s
        .topology_snapshot(&TopologySeed::Claim(c1), &TopologyOptions::default())
        .unwrap();
    assert!(contains(&reverse, &i));
    assert_closed(&snap);
}

#[test]
fn topology_limits_preserve_paths_and_report_truthful_omission_and_change_sequence() {
    let s = Store::memory().unwrap();
    let root = claim(&s, "High degree root");
    for index in 0..45 {
        let c = claim(&s, &format!("Bounded neighbour {index:03}"));
        connect(&s, &root, &c);
    }
    let options = TopologyOptions {
        max_nodes: 4,
        max_edges: 2,
        ..Default::default()
    };
    let a = s
        .topology_snapshot(&TopologySeed::Claim(root.clone()), &options)
        .unwrap();
    let b = s
        .topology_snapshot(&TopologySeed::Claim(root.clone()), &options)
        .unwrap();
    assert_eq!(a.nodes, b.nodes);
    assert_eq!(a.edges, b.edges);
    assert!(a.omitted_nodes > 0 && a.omitted_edges > 0);
    assert!(!a.exploration_exhausted);
    assert!(!a.has_continuation && a.continuation_token.is_none());
    assert_eq!(
        a.memory_changed_seq,
        s.memories_changed_seq().unwrap() as u64
    );
    assert_closed(&a);
    let before = a.memory_changed_seq;
    claim(&s, "A committed independent change");
    let changed = s
        .topology_snapshot(&TopologySeed::Claim(root.clone()), &options)
        .unwrap();
    assert!(changed.memory_changed_seq > before);
    assert!(s
        .topology_snapshot(
            &TopologySeed::Claim(root),
            &TopologyOptions {
                continuation_token: Some("nodes:4".into()),
                ..options
            }
        )
        .is_err());
}

#[test]
fn topology_retained_intel_items_keep_exact_source_scope_without_writes() {
    let s = Store::memory().unwrap();
    let a = article(&s, "cycle-a", "a");
    let b = article(&s, "cycle-b", "b");
    let before = s.memories_changed_seq().unwrap();
    let seed = TopologySeed::RetainedClaim {
        run_id: a.run_id.clone(),
        article_id: a.id.clone(),
        revision: crate::atlas_metadata::input_hash(&a),
        local_id: "c1".into(),
        text: "Unindexed visible source claim".into(),
        job_id: "job-a".into(),
    };
    let snap = s
        .topology_snapshot(&seed, &TopologyOptions::default())
        .unwrap();
    assert_eq!(snap.root_id, seed.key());
    assert!(snap.nodes[0].status.contains("not indexed"));
    assert!(contains(&snap, "article:cycle-a/a"));
    assert!(!contains(&snap, "article:cycle-b/b"));
    assert_eq!(s.memories_changed_seq().unwrap(), before);
    let actor_seed = TopologySeed::RetainedActor {
        run_id: b.run_id.clone(),
        article_id: b.id.clone(),
        revision: crate::atlas_metadata::input_hash(&b),
        actor_ref: "organization:metadata actor".into(),
        name: "Metadata actor".into(),
    };
    let actor = s
        .topology_snapshot(&actor_seed, &TopologyOptions::default())
        .unwrap();
    assert_eq!(actor.root_id, actor_seed.key());
    assert_eq!(actor.nodes[0].label, "Metadata actor");
    assert!(actor.nodes[0].member_ids.is_empty());
    assert_eq!(s.memories_changed_seq().unwrap(), before);
}

#[test]
fn topology_intel_canonical_claim_resolution_fences_owner_local_id_and_generation() {
    let s = Store::memory().unwrap();
    let a = article(&s, "run-a", "owner-a");
    let b = article(&s, "run-b", "owner-b");
    let ca = claim(&s, "Canonical A");
    let cb = claim(&s, "Canonical B");
    for (row, snapshot, job, memory, text) in [
        (&a, "snap-a", "job-a", &ca, "Visible claim A"),
        (&b, "snap-b", "job-b", &cb, "Visible claim B"),
    ] {
        snapshot_receipt(&s, snapshot, &row.id);
        derivation(
            &s,
            snapshot,
            "c1",
            memory,
            "claim",
            serde_json::json!({"id":"c1","text":text}),
        );
        s.conn.execute("INSERT INTO intel_brief_runs(job_id,article_id,generation,contract,input_revision,state,created_at,updated_at) VALUES(?1,?2,1,'fixture','input','completed','fixture','fixture')",rusqlite::params![job,row.id]).unwrap();
        s.conn.execute("INSERT INTO intel_brief_publications(article_id,job_id,generation,contract,body_hash,snapshot_hash,claims_json,inferences_json,leads_json,published_at) VALUES(?1,?2,1,'fixture','','snapshot',?3,'{}','[]','fixture')",rusqlite::params![row.id,job,serde_json::json!({"claims":[{"id":"c1","text":text}]}).to_string()]).unwrap();
        s.conn.execute("INSERT INTO app_state(key,value) VALUES(?1,?2)",rusqlite::params![crate::intel_recon::brief_canonical::state_key(&row.id),serde_json::json!({"publication_version":2,"state":"indexed","snapshot_id":snapshot,"job_id":job,"reason":""}).to_string()]).unwrap();
    }
    let seed = TopologySeed::RetainedClaim {
        run_id: a.run_id.clone(),
        article_id: a.id.clone(),
        revision: crate::atlas_metadata::input_hash(&a),
        local_id: "c1".into(),
        text: "Visible claim A".into(),
        job_id: "job-a".into(),
    };
    let snapshot = s
        .topology_snapshot(&seed, &TopologyOptions::default())
        .unwrap();
    assert_eq!(snapshot.root_id, ca);
    assert!(!contains(&snapshot, &cb));
    assert_eq!(snapshot.seed, Some(seed.clone()));
    let mut revised = a.clone();
    revised.title = "Changed metadata before next pipeline run".into();
    s.atlas_upsert_article(&revised).unwrap();
    let revised_seed = TopologySeed::RetainedClaim {
        run_id: revised.run_id.clone(),
        article_id: revised.id.clone(),
        revision: crate::atlas_metadata::input_hash(&revised),
        local_id: "c1".into(),
        text: "Visible claim A".into(),
        job_id: "job-a".into(),
    };
    let changed = s
        .topology_snapshot(&revised_seed, &TopologyOptions::default())
        .unwrap();
    assert_eq!(changed.root_id, revised_seed.key());
    assert_ne!(changed.root_id, ca);
    s.atlas_upsert_article(&a).unwrap();
    s.conn
        .execute(
            "UPDATE insight_snapshots SET inputs_json='{}' WHERE id='snap-a'",
            [],
        )
        .unwrap();
    let unavailable = s
        .topology_snapshot(&seed, &TopologyOptions::default())
        .unwrap();
    assert_eq!(unavailable.root_id, seed.key());
    s.conn.execute("INSERT INTO intel_brief_runs(job_id,article_id,generation,contract,input_revision,state,created_at,updated_at) VALUES('job-a-new',?1,2,'fixture','new-input','active','fixture','fixture')",[&a.id]).unwrap();
    let old = s
        .topology_snapshot(&seed, &TopologyOptions::default())
        .unwrap();
    assert_eq!(old.root_id, seed.key());
    assert_ne!(old.root_id, ca);
    assert!(!contains(&old, &cb));
}

#[test]
fn topology_cycle_scope_uses_origins_instead_of_creation_date_substrings() {
    let s = Store::memory().unwrap();
    let a = article(&s, "cycle-actual-a", "a");
    let b = article(&s, "cycle-actual-b", "b");
    let c = claim(&s, "Shared claim");
    let only_b = claim(&s, "Only cycle B");
    for (mid, row, fp) in [(&c, &a, "a-fp"), (&only_b, &b, "b-fp")] {
        s.conn.execute("INSERT INTO insight_claims(fingerprint,memory_id,entity_id,predicate,object_value,topic,classification,confidence,created_at,updated_at) VALUES(?1,?2,'','p','o','investigation','claim',0,'2026-10-15T00:00:00Z','fixture')",rusqlite::params![fp,mid]).unwrap();
        s.conn.execute("INSERT INTO insight_sources(fingerprint,run_id,answer_id,call_id) VALUES(?1,?2,'fixture',?3)",rusqlite::params![fp,row.run_id,row.id]).unwrap();
    }
    connect(&s, &c, &only_b);
    let scoped = s
        .topology_snapshot(
            &TopologySeed::Claim(c.clone()),
            &TopologyOptions {
                cycle_scope: CycleScope::ThisCycle(a.run_id.clone()),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(contains(&scoped, &format!("article:{}/{}", a.run_id, a.id)));
    assert!(!contains(&scoped, &only_b));
    assert!(!contains(
        &scoped,
        &format!("article:{}/{}", b.run_id, b.id)
    ));
    assert_closed(&scoped);
}

#[test]
fn topology_native_metadata_actor_resolves_existing_scoped_binding_without_claims() {
    use crate::atlas_metadata::{ArticleMention, ArticleMetadata, MetadataState};
    let s = Store::memory().unwrap();
    let row = article(&s, "native-run", "native-article");
    let mut metadata = ArticleMetadata::pending(&row);
    metadata.entity_state = MetadataState::Complete;
    metadata.mentions.push(ArticleMention {
        type_hint: "organization".into(),
        comparison_key: row.title.to_lowercase(),
        field: "title".into(),
        start_byte: 0,
        end_byte: row.title.len(),
        original_text: row.title.clone(),
    });
    assert!(s.atlas_save_metadata(&metadata).unwrap());
    crate::brain_backfill::index_article(&s, "native-run/native-article", "2026-10-15T00:00:00Z")
        .unwrap();
    let native = crate::atlas_metadata::article_actors(&row, &metadata).remove(0);
    let group = s.actor_groups().unwrap().remove(0);
    let before = s.memories_changed_seq().unwrap();
    let snapshot = s
        .topology_snapshot(
            &TopologySeed::RetainedActor {
                run_id: row.run_id.clone(),
                article_id: row.id.clone(),
                revision: crate::atlas_metadata::input_hash(&row),
                actor_ref: native.id,
                name: native.display_name,
            },
            &TopologyOptions::default(),
        )
        .unwrap();
    assert!(group.actor_ids.contains(&snapshot.root_id));
    assert_eq!(snapshot.nodes[0].subtype, "organization");
    assert!(contains(&snapshot, "article:native-run/native-article"));
    assert_eq!(s.memories_changed_seq().unwrap(), before);
    assert!(!snapshot
        .nodes
        .iter()
        .any(|n| n.kind == TopologyNodeKind::Claim));
    assert_closed(&snapshot);
}

#[test]
fn topology_revised_article_does_not_inherit_historical_derivation_evidence() {
    let s = Store::memory().unwrap();
    let mut row = article(&s, "retained-run", "revision-owner");
    let memory = claim(&s, "Original metadata assertion");
    snapshot_receipt(&s, "revision-snapshot", &row.id);
    derivation(
        &s,
        "revision-snapshot",
        "c1",
        &memory,
        "claim",
        serde_json::json!({"id":"c1","text":"Original metadata assertion"}),
    );
    let old_artifact =
        crate::nlp::cache::artifact(&s.conn, &format!("{}\n{}", row.title, row.description))
            .unwrap();
    s.conn.execute("INSERT INTO insight_derivation_evidence(derivation_id,source_id,source_revision,evidence_json) VALUES('revision-snapshot:c1',?1,?2,?3)",rusqlite::params![format!("article:{}",row.id),old_artifact,serde_json::json!({"source_id":format!("article:{}",row.id),"revision":old_artifact,"artifact_hash":old_artifact}).to_string()]).unwrap();
    let current = s
        .topology_snapshot(
            &TopologySeed::Claim(memory.clone()),
            &TopologyOptions::default(),
        )
        .unwrap();
    assert!(contains(&current, "article:retained-run/revision-owner"));
    row.title = "Replacement source text".into();
    s.atlas_upsert_article(&row).unwrap();
    let revised = s
        .topology_snapshot(
            &TopologySeed::Claim(memory.clone()),
            &TopologyOptions::default(),
        )
        .unwrap();
    assert!(!contains(&revised, "article:retained-run/revision-owner"));
    assert!(revised
        .nodes
        .iter()
        .any(|n| n.kind == TopologyNodeKind::Source && n.revision == old_artifact));
    let article_view = s
        .topology_snapshot(
            &TopologySeed::Article {
                run_id: row.run_id.clone(),
                article_id: row.id.clone(),
                revision: crate::atlas_metadata::input_hash(&row),
                originating_memory: None,
            },
            &TopologyOptions::default(),
        )
        .unwrap();
    assert!(!contains(&article_view, &memory));
}

#[test]
fn topology_cycle_filters_precede_topic_fanout_limits() {
    let s = Store::memory().unwrap();
    for i in 0..40 {
        let mut row = article(&s, &format!("aaa-{i:02}"), &format!("article-{i:02}"));
        row.category = "Scoped topic".into();
        s.atlas_upsert_article(&row).unwrap();
    }
    let mut target = article(&s, "zzz-selected", "target");
    target.category = "Scoped topic".into();
    s.atlas_upsert_article(&target).unwrap();
    let options = TopologyOptions {
        max_nodes: 8,
        max_edges: 16,
        cycle_scope: CycleScope::ThisCycle(target.run_id.clone()),
        ..Default::default()
    };
    let snapshot = s
        .topology_snapshot(&TopologySeed::Topic("Scoped topic".into()), &options)
        .unwrap();
    assert!(contains(&snapshot, "article:zzz-selected/target"));
    assert_eq!(snapshot.nodes.len(), 2);
    assert!(snapshot.exploration_exhausted);
    let mut claims: Vec<_> = (0..41)
        .map(|i| claim(&s, &format!("Topic member {i:02}")))
        .collect();
    claims.sort();
    let selected = claims.last().unwrap().clone();
    for (i, id) in claims.iter().enumerate() {
        let run = if id == &selected {
            "zzz-selected"
        } else {
            "other-cycle"
        };
        let fp = format!("topic-row-{i}");
        s.conn.execute("INSERT INTO insight_claims(fingerprint,memory_id,entity_id,predicate,object_value,topic,classification,confidence,created_at,updated_at) VALUES(?1,?2,'','p','o','Claim topic','claim',0,'fixture','fixture')",rusqlite::params![fp,id]).unwrap();
        s.conn.execute("INSERT INTO insight_sources(fingerprint,run_id,answer_id,call_id) VALUES(?1,?2,'fixture','no-retained-article')",rusqlite::params![fp,run]).unwrap();
    }
    let snapshot = s
        .topology_snapshot(&TopologySeed::Topic("Claim topic".into()), &options)
        .unwrap();
    assert!(contains(&snapshot, &selected));
    assert_eq!(snapshot.nodes.len(), 2);
    assert!(snapshot.exploration_exhausted);
}

#[tokio::test]
async fn topology_scope_analysis_excludes_retained_projection_premises() {
    let s = Store::memory().unwrap();
    let row = article(&s, "analysis-run", "analysis-owner");
    let canonical = claim(&s, "One canonical source claim");
    snapshot_receipt(&s, "analysis-snapshot", &row.id);
    derivation(
        &s,
        "analysis-snapshot",
        "canonical",
        &canonical,
        "claim",
        serde_json::json!({"id":"canonical"}),
    );
    let seed = TopologySeed::RetainedClaim {
        run_id: row.run_id.clone(),
        article_id: row.id.clone(),
        revision: crate::atlas_metadata::input_hash(&row),
        local_id: "unindexed".into(),
        text: "A retained unindexed claim".into(),
        job_id: "unpublished-job".into(),
    };
    let snapshot = s
        .topology_snapshot(&seed, &TopologyOptions::default())
        .unwrap();
    assert_eq!(
        snapshot
            .nodes
            .iter()
            .filter(|n| n.kind == TopologyNodeKind::Claim)
            .count(),
        2
    );
    let before = s.memories_changed_seq().unwrap();
    let result = crate::brain_scope::run_scope_analysis(
        &s,
        &crate::brain_scope::ScopeAnalysisPayload {
            seed,
            depth: 2,
            cycle_scope: CycleScope::AllRetained,
            options: TopologyOptions::default(),
        },
        None,
        "2026-10-15T00:00:00Z",
    )
    .await
    .unwrap();
    assert_eq!(result.inferences_created, 0);
    assert_eq!(s.memories_changed_seq().unwrap(), before);
}

#[test]
fn topology_relationship_identity_and_direction_are_stable_across_entry_seeds() {
    let s = Store::memory().unwrap();
    let actor_id = actor(&s, "Direction witness", "direction-scope");
    let claim_id = claim(&s, "A recorded witness assertion");
    let inference_id = claim(&s, "An inference from the assertion");
    actors::add_memory_actor_ref(
        &s.conn,
        &claim_id,
        &actor_id,
        "subject",
        crate::memory_metadata::IdentityState::Provisional,
        &serde_json::json!({"record":"direction-fixture"}),
    )
    .unwrap();
    let payload = crate::memory_metadata::MemoryPayload::Inference(
        crate::memory_metadata::InferencePayload {
            premise_claims: vec![claim_id.clone()],
            reasoning: "Recorded inference reasoning".into(),
            support_state: "assessed".into(),
            ..Default::default()
        },
    );
    s.conn.execute("UPDATE memory_metadata SET memory_kind='inference',payload_json=?2,payload_schema=1 WHERE memory_id=?1",rusqlite::params![inference_id,serde_json::to_string(&payload).unwrap()]).unwrap();
    let row = article(&s, "direction-run", "direction-article");
    snapshot_receipt(&s, "direction-snapshot", &row.id);
    derivation(
        &s,
        "direction-snapshot",
        "c1",
        &claim_id,
        "claim",
        serde_json::json!({"id":"c1"}),
    );
    let artifact =
        crate::nlp::cache::artifact(&s.conn, &format!("{}\n{}", row.title, row.description))
            .unwrap();
    s.conn.execute("INSERT INTO insight_derivation_evidence(derivation_id,source_id,source_revision,evidence_json) VALUES('direction-snapshot:c1',?1,?2,?3)",rusqlite::params![format!("article:{}",row.id),artifact,serde_json::json!({"source_id":format!("article:{}",row.id),"revision":artifact,"artifact_hash":artifact}).to_string()]).unwrap();
    let link_id = s
        .write_link_memory(&LinkPayload {
            source: EndpointRef::Memory {
                memory_id: inference_id.clone(),
                kind: MemoryKind::Inference,
            },
            target: EndpointRef::Memory {
                memory_id: claim_id.clone(),
                kind: MemoryKind::Claim,
            },
            relation: "derived_from".into(),
            rationale: "Recorded directed Link payload".into(),
            receipts: vec!["directed-payload-receipt".into()],
            promoted_from: None,
        })
        .unwrap();
    let article_id = format!("article:{}/{}", row.run_id, row.id);
    let source_seed = TopologySeed::Source {
        origin: "evidence".into(),
        source_id: format!("article:{}", row.id),
        revision: artifact,
    };
    let source_id = source_seed.key();
    let expected = [
        (
            format!("actor-ref:{claim_id}:{actor_id}:subject"),
            actor_id.clone(),
            claim_id.clone(),
            "referenced by",
        ),
        (
            format!("premise:{claim_id}:{inference_id}"),
            claim_id.clone(),
            inference_id.clone(),
            "premise for",
        ),
        (
            format!("article-evidence:{article_id}:{claim_id}"),
            article_id,
            claim_id.clone(),
            "source for",
        ),
        (
            format!("evidence:{source_id}:{claim_id}"),
            source_id,
            claim_id.clone(),
            "source for",
        ),
        (
            link_id,
            inference_id.clone(),
            claim_id.clone(),
            "derived_from",
        ),
    ];
    let seeds = [
        TopologySeed::Actor(actor_id),
        TopologySeed::Claim(claim_id),
        TopologySeed::Inference(inference_id),
        TopologySeed::Article {
            run_id: row.run_id.clone(),
            article_id: row.id.clone(),
            revision: crate::atlas_metadata::input_hash(&row),
            originating_memory: None,
        },
        source_seed,
    ];
    for seed in seeds {
        let snapshot = s
            .topology_snapshot(&seed, &TopologyOptions::default())
            .unwrap();
        for (id, from, to, label) in &expected {
            let matches: Vec<_> = snapshot
                .edges
                .iter()
                .filter(|edge| &edge.id == id)
                .collect();
            assert_eq!(matches.len(), 1, "{}: relation {id}", seed.key());
            let edge = matches[0];
            assert_eq!(
                (&edge.from, &edge.to, edge.label.as_str()),
                (from, to, *label),
                "{}: relation {id}",
                seed.key()
            );
        }
        assert_closed(&snapshot);
    }
}

#[test]
fn topology_actor_entry_finds_direct_typed_inference_without_reverse_index() {
    let s = Store::memory().unwrap();
    let actor_id = actor(&s, "Typed inference actor", "typed-inference-scope");
    let inference_id = claim(&s, "A scope inference with a recorded actor");
    let payload = crate::memory_metadata::MemoryPayload::Inference(
        crate::memory_metadata::InferencePayload {
            actors: vec![crate::memory_metadata::ActorRef {
                actor_id: actor_id.clone(),
                role: "subject".into(),
                binding_state: crate::memory_metadata::IdentityState::Provisional,
            }],
            reasoning: "Exact recorded actor binding".into(),
            ..Default::default()
        },
    );
    s.conn.execute("UPDATE memory_metadata SET memory_kind='inference',payload_json=?2,payload_schema=1 WHERE memory_id=?1",rusqlite::params![inference_id,serde_json::to_string(&payload).unwrap()]).unwrap();
    assert_eq!(
        s.conn
            .query_row(
                "SELECT COUNT(*) FROM brain_memory_actor_refs WHERE memory_id=?1",
                [&inference_id],
                |r| r.get::<_, usize>(0)
            )
            .unwrap(),
        0
    );
    let options = TopologyOptions {
        depth: 1,
        ..Default::default()
    };
    let edge_id = format!("actor-ref:{inference_id}:{actor_id}:subject");
    for seed in [
        TopologySeed::Actor(actor_id.clone()),
        TopologySeed::Inference(inference_id.clone()),
    ] {
        let snapshot = s.topology_snapshot(&seed, &options).unwrap();
        assert!(contains(&snapshot, &actor_id) && contains(&snapshot, &inference_id));
        let other = snapshot.nodes.iter().find(|node| !node.is_root).unwrap();
        assert_eq!(other.min_hops, 1);
        let edge = snapshot
            .edges
            .iter()
            .find(|edge| edge.id == edge_id)
            .unwrap();
        assert_eq!((&edge.from, &edge.to), (&actor_id, &inference_id));
        assert_eq!(edge.label, "referenced by");
        assert!(edge
            .receipts
            .iter()
            .any(|receipt| receipt.starts_with(&format!("memory_metadata:{inference_id}@"))));
        assert_closed(&snapshot);
    }
    assert_eq!(
        s.conn
            .query_row(
                "SELECT COUNT(*) FROM brain_memory_actor_refs WHERE memory_id=?1",
                [&inference_id],
                |r| r.get::<_, usize>(0)
            )
            .unwrap(),
        0,
        "opening topology never repairs or writes the index"
    );
}

#[test]
fn topology_user_claim_assessment_status_distinguishes_unassessed_from_receipts() {
    let s = Store::memory().unwrap();
    let id = claim(&s, "A user assertion without extraction assessment");
    let Some(crate::memory_metadata::MemoryPayload::Claim(mut payload)) =
        s.memory_payload(&id).unwrap()
    else {
        panic!("typed user Claim required");
    };
    assert_eq!(
        payload.assessment, "unassessed",
        "exercise the actual user-claim default, not an empty synthetic payload"
    );
    let seed = TopologySeed::Claim(id.clone());
    let snapshot = s
        .topology_snapshot(&seed, &TopologyOptions::default())
        .unwrap();
    assert_eq!(snapshot.nodes[0].status, "unassessed");
    payload.assessment=serde_json::json!({"fidelity":"faithful","relation":"supports","threshold_passed":true,"rationale":"Recorded extraction assessment","gaps":[]}).to_string();
    s.conn
        .execute(
            "UPDATE memory_metadata SET payload_json=?2 WHERE memory_id=?1",
            rusqlite::params![
                id,
                serde_json::to_string(&crate::memory_metadata::MemoryPayload::Claim(
                    payload.clone()
                ))
                .unwrap()
            ],
        )
        .unwrap();
    let assessed = s
        .topology_snapshot(&seed, &TopologyOptions::default())
        .unwrap();
    assert_eq!(assessed.root_id, id);
    assert_eq!(assessed.nodes[0].status, "extraction assessed");
    payload.assessment = "  UnAsSeSsEd  ".into();
    s.conn
        .execute(
            "UPDATE memory_metadata SET payload_json=?2 WHERE memory_id=?1",
            rusqlite::params![
                id,
                serde_json::to_string(&crate::memory_metadata::MemoryPayload::Claim(payload))
                    .unwrap()
            ],
        )
        .unwrap();
    assert_eq!(
        s.topology_snapshot(&seed, &TopologyOptions::default())
            .unwrap()
            .nodes[0]
            .status,
        "unassessed"
    );
}
