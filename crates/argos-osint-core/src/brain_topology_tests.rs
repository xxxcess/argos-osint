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
    assert_eq!(snap.root_kind, Some(TopologyNodeKind::Entity));
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
