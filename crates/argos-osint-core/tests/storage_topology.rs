//! Brain topology on SurrealDB (migration phase 5, first item): the
//! Actor-group seed of `Store::topology_snapshot`. Members resolve through
//! redirects; the group's neighbours are its members' typed payload actors,
//! actor refs, native mentions and active Links, all re-pointed at the group;
//! the cycle scope, the depth gap and refused seeds. Both modes via
//! `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::brain::MemorySource;
use argos_osint_core::brain_topology::{
    CycleScope, TopologyEdgeKind, TopologyNodeKind, TopologyOptions, TopologySeed,
};
use argos_osint_core::memory_metadata::{
    ActorRef, ClaimPayload, EndpointRef, IdentityState, LinkPayload, MemoryKind, MemoryPayload,
};
use argos_osint_core::recon::Thread;
use argos_osint_core::storage::recon::{self, Endpoint};
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::{atlas, memories, topology};
use argos_osint_core::store::AtlasArticleRow;
use support::*;

fn source() -> MemorySource {
    MemorySource {
        app: "cli".into(),
        conversation_id: "c1".into(),
        message_id: None,
        reference: None,
    }
}

fn article(run: &str, id: &str) -> AtlasArticleRow {
    AtlasArticleRow {
        run_id: run.into(),
        id: id.into(),
        title: format!("Title {id}"),
        url: format!("https://ex.com/{id}"),
        category: "unk".into(),
        seen_at: "2026-10-23T00:00:00Z".into(),
        ..Default::default()
    }
}

fn group() -> TopologySeed {
    TopologySeed::ActorGroup {
        name_key: "acme corp".into(),
        member_ids: vec!["act1".into(), "act2".into(), "missing".into()],
    }
}

fn options(depth: usize, cycle_scope: CycleScope) -> TopologyOptions {
    TopologyOptions {
        depth,
        cycle_scope,
        ..Default::default()
    }
}

/// act1 (with act3 redirected into it) and act2 share the name Acme Corp.
/// c1 names act3 in its typed payload; c2 has an act2 actor ref and an
/// insight source in r1; act1 is mentioned in thread t1 and Linked to
/// r2/a2; act2 is mentioned in r1/a1.
async fn seed(repo: &Repository) -> (String, String) {
    let db = repo.db().clone();
    for run in ["r1", "r2"] {
        atlas::insert_run(&db, run, "{}", "{}").await.unwrap();
    }
    atlas::upsert_article(&db, &article("r1", "a1"))
        .await
        .unwrap();
    atlas::upsert_article(&db, &article("r2", "a2"))
        .await
        .unwrap();
    recon::put_thread(
        repo,
        &Thread {
            id: "t1".into(),
            title: "Thread One".into(),
            created_at: "2026-10-23T00:00:00Z".into(),
            updated_at: "2026-10-23T00:00:00Z".into(),
            draft: String::new(),
            scroll: 0,
            recall_insights: true,
        },
    )
    .await
    .unwrap();
    let link = serde_json::to_string(&MemoryPayload::Link(LinkPayload {
        source: EndpointRef::Memory {
            memory_id: "act1".into(),
            kind: MemoryKind::Actor,
        },
        target: EndpointRef::Article {
            run_id: "r2".into(),
            article_id: "a2".into(),
            revision: String::new(),
        },
        relation: "supports".into(),
        rationale: "because".into(),
        receipts: vec![],
        promoted_from: None,
    }))
    .unwrap();
    db.query(
        "CREATE memory:act1 SET text = 'Acme Corp', memory_kind = 'actor';
         CREATE memory:act2 SET text = 'ACME corp', memory_kind = 'actor';
         CREATE memory:act3 SET text = 'Acme', memory_kind = 'actor', lifecycle = 'redirected';
         CREATE brain_actor:act1 SET display_name = 'Acme Corp', created_at = '', updated_at = '';
         CREATE brain_actor:act2 SET display_name = 'ACME corp', created_at = '', updated_at = '';
         CREATE brain_actor:act3 SET display_name = 'Acme', created_at = '', updated_at = '';
         CREATE brain_actor_redirect:act3 SET to_actor = 'act1', created_at = '';
         CREATE memory:l1 SET text = 'link', memory_kind = 'link', payload_json = $link;
         CREATE brain_actor_mention SET actor_id = 'act1', origin_kind = 'thread',
           origin_id = 't1', created_at = '';
         CREATE brain_actor_mention SET actor_id = 'act2', origin_kind = 'article',
           origin_scope = 'r1', origin_id = 'a1', created_at = '';",
    )
    .bind(("link", link))
    .await
    .unwrap()
    .check()
    .unwrap();
    let c1 = memories::add_memory(repo, "Acme bought Widgets", "fact", false, source(), None)
        .await
        .unwrap();
    let c2 = memories::add_memory(repo, "Acme sued Gadgets", "fact", false, source(), None)
        .await
        .unwrap();
    let payload = serde_json::to_string(&MemoryPayload::Claim(ClaimPayload {
        actors: vec![ActorRef {
            actor_id: "act3".into(),
            role: "subject".into(),
            binding_state: IdentityState::default(),
        }],
        ..Default::default()
    }))
    .unwrap();
    db.query(
        "UPDATE type::record('memory', $c1) SET payload_json = $p RETURN NONE;
         CREATE insight_claim:fp2 SET memory_id = $c2, entity_id = 'e', predicate = 'p',
           object_value = 'o', topic = 't', classification = 'fact', confidence = 0.5,
           created_at = '', updated_at = '';
         CREATE insight_source SET fingerprint = 'fp2', run_id = 'r1', answer_id = 'x',
           call_id = 'article:a1';",
    )
    .bind(("c1", c1.id.clone()))
    .bind(("c2", c2.id.clone()))
    .bind(("p", payload))
    .await
    .unwrap()
    .check()
    .unwrap();
    recon::put_actor_ref(repo, &c2.id, "act2", "object")
        .await
        .unwrap();
    for (side, kind, r, o) in [
        ("source", "memory", "act1", ""),
        ("target", "article", "a2", "r2"),
    ] {
        recon::put_endpoint(
            repo,
            &Endpoint {
                memory_id: "l1",
                side,
                endpoint_kind: kind,
                ref_id: r,
                origin: o,
                revision: "",
            },
        )
        .await
        .unwrap();
    }
    (c1.id, c2.id)
}

fn ids<T>(items: &[T], f: impl Fn(&T) -> String) -> Vec<String> {
    let mut out: Vec<String> = items.iter().map(f).collect();
    out.sort();
    out
}

#[tokio::test]
async fn actor_group_topology_collects_member_neighbours() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "topology_group").await;
    let (c1, c2) = seed(&repo).await;
    let snap = topology::topology_snapshot(&repo, &group(), &options(1, CycleScope::AllRetained))
        .await
        .unwrap();

    assert_eq!(snap.root_id, "actor-group:acme corp");
    assert_eq!(snap.root_kind, Some(TopologyNodeKind::ActorGroup));
    let root = &snap.nodes[0];
    assert!(root.is_root);
    assert_eq!(root.member_ids, vec!["act1", "act2"], "missing is dropped");
    assert_eq!(root.status, "display group · identity unmerged");

    let mut want = vec![
        c1.clone(),
        c2.clone(),
        "article:r1/a1".to_string(),
        "article:r2/a2".to_string(),
        "source:thread:t1@".to_string(),
        snap.root_id.clone(),
    ];
    want.sort();
    assert_eq!(ids(&snap.nodes, |n| n.id.clone()), want);
    let thread = snap.nodes.iter().find(|n| n.id == "source:thread:t1@");
    assert_eq!(thread.unwrap().label, "Thread One");

    let mut edges = vec![
        format!("actor-ref:{c1}:act3:subject"),
        format!("actor-ref:{c2}:act2:object"),
        "l1".to_string(),
        "mention:act1:thread::t1:".to_string(),
        "mention:act2:article:r1:a1:".to_string(),
    ];
    edges.sort();
    assert_eq!(ids(&snap.edges, |e| e.id.clone()), edges);
    for e in &snap.edges {
        assert!(
            e.from == snap.root_id || e.to == snap.root_id,
            "{} is re-pointed at the group",
            e.id
        );
    }
    let l1 = snap.edges.iter().find(|e| e.id == "l1").unwrap();
    assert_eq!(
        (l1.kind, l1.label.as_str()),
        (TopologyEdgeKind::Link, "supports")
    );
    let typed = snap
        .edges
        .iter()
        .find(|e| e.id == format!("actor-ref:{c1}:act3:subject"))
        .unwrap();
    assert_eq!(typed.kind, TopologyEdgeKind::Concerns);
    assert_eq!(typed.member_ids, vec!["act3"]);

    assert_eq!(ids(&snap.associations.claims, |m| m.id.clone()), {
        let mut c = vec![c1, c2];
        c.sort();
        c
    });
    assert_eq!(
        snap.associations.threads,
        vec![("t1".to_string(), "Thread One".to_string())]
    );
    assert_eq!(snap.associations.articles.len(), 2);
    assert!(snap.exploration_exhausted);
    assert!(snap.unresolved_endpoints.is_empty());
    assert!(snap.neighborhood_brief.is_some());
    assert!(snap.neighborhood_input.is_some());
}

#[tokio::test]
async fn actor_group_topology_scopes_depth_and_seeds() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "topology_scope").await;
    let (_, c2) = seed(&repo).await;

    // This cycle: r1's article and the claim sourced in r1 only.
    let snap = topology::topology_snapshot(
        &repo,
        &group(),
        &options(1, CycleScope::ThisCycle("r1".into())),
    )
    .await
    .unwrap();
    let mut want = vec![c2, "article:r1/a1".into(), snap.root_id.clone()];
    want.sort();
    assert_eq!(ids(&snap.nodes, |n| n.id.clone()), want);

    // Deeper walks are bounded with an explicit gap.
    let snap = topology::topology_snapshot(&repo, &group(), &options(2, CycleScope::AllRetained))
        .await
        .unwrap();
    assert!(!snap.exploration_exhausted);
    assert!(snap
        .unresolved_endpoints
        .contains(&topology::DEPTH_GAP.to_string()));

    // A group with no live member has no root.
    let empty = TopologySeed::ActorGroup {
        name_key: "nobody".into(),
        member_ids: vec!["missing".into()],
    };
    let snap = topology::topology_snapshot(&repo, &empty, &options(1, CycleScope::AllRetained))
        .await
        .unwrap();
    assert!(snap.nodes.is_empty());
    assert!(snap.root_id.is_empty());
    assert!(snap.neighborhood_brief.is_some());

    // Other seeds are refused, never answered from partial data.
    let err = topology::topology_snapshot(
        &repo,
        &TopologySeed::Actor("act1".into()),
        &options(1, CycleScope::AllRetained),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("isn't available on SurrealDB yet"));
}
