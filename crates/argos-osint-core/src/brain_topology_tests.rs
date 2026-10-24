use super::*;
use tempfile::tempdir;

use crate::brain::MemorySource;
use crate::brain_actors::{self as actors, BindRequest};
use crate::memory_metadata::{EndpointRef, LinkPayload, MemoryKind};
use crate::store::Store;

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
