use super::*;
use crate::atlas_actor_index::tests::seed;

fn world() -> Store {
    let store = Store::memory().unwrap();
    seed(&store, "rA", "a1", "Jane Doe visits Kyiv", &["Jane Doe"]);
    seed(&store, "rB", "b1", "Jane Doe resigns", &["Jane Doe"]);
    for run in ["rA", "rB"] {
        assert!(crate::atlas_actor_index::index_run(&store, run, &|| false)
            .unwrap()
            .complete());
    }
    store
        .add_memory(
            "Jane Doe resigned as envoy",
            "fact",
            false,
            crate::brain::MemorySource {
                app: "manual".into(),
                conversation_id: "c1".into(),
                message_id: None,
                reference: None,
            },
        )
        .unwrap();
    store
}

#[test]
fn kind_filters_cover_all_five_kinds_and_default_keeps_evidence_only() {
    let store = world();
    let counts: std::collections::HashMap<_, _> =
        store.memory_kind_counts().unwrap().into_iter().collect();
    assert_eq!(counts[&MemoryKind::Actor], 2);
    assert_eq!(counts[&MemoryKind::Link], 2);
    assert!(counts[&MemoryKind::Related] >= 1);
    assert_eq!(
        counts[&MemoryKind::Claim],
        1,
        "legacy untyped browses as Claim"
    );
    assert_eq!(counts[&MemoryKind::Inference], 0);
    let default = store.list_memories_by_kind(None, "").unwrap();
    assert_eq!(default.len(), 1);
    let actors = store
        .list_memories_by_kind(Some(MemoryKind::Actor), "jane")
        .unwrap();
    assert_eq!(actors.len(), 2);
    assert!(store
        .list_memories_by_kind(Some(MemoryKind::Actor), "nobody")
        .unwrap()
        .is_empty());
}

#[test]
fn typed_details_render_each_payload_kind() {
    let store = world();
    let first = |k| {
        store.list_memories_by_kind(Some(k), "").unwrap()[0]
            .id
            .clone()
    };
    let actor = typed_detail(&store, &first(MemoryKind::Actor)).unwrap();
    assert_eq!(actor[0], "Kind: Actor");
    assert!(actor.iter().any(|l| l == "Subtype: person"), "{actor:?}");
    assert!(actor
        .iter()
        .any(|l| l.starts_with("Mentions: 1 (1 article occurrence)")));
    let link = typed_detail(&store, &first(MemoryKind::Link)).unwrap();
    assert!(
        link.iter().any(|l| l == "Relation: mentioned_in"),
        "{link:?}"
    );
    assert!(link.iter().any(|l| l.starts_with("From: actor: Jane Doe")));
    assert!(link.iter().any(|l| l.starts_with("To: article: Jane Doe")));
    let related = typed_detail(&store, &first(MemoryKind::Related)).unwrap();
    assert!(
        related
            .iter()
            .any(|l| l == "Status: tentative · not evidence"),
        "{related:?}"
    );
    assert!(related
        .iter()
        .any(|l| l.starts_with("Basis: candidate name: Jane Doe")));
    let claim = typed_detail(&store, &first(MemoryKind::Claim)).unwrap();
    assert_eq!(claim, vec!["Kind: Claim", "Payload: legacy (untyped)"]);
}

#[test]
fn allowed_kinds_filter_recall_and_key_the_cache() {
    let store = world();
    let default =
        crate::brain_query::recall_candidates_for(&store, "recon", "run", "Jane Doe", 10, &[])
            .unwrap();
    assert!(default.iter().all(|h| !matches!(
        memory_kind(&store, &h.memory.id).unwrap(),
        MemoryKind::Actor
    )));
    let actors = crate::brain_query::recall_candidates_for(
        &store,
        "recon",
        "run",
        "Jane Doe",
        10,
        &[MemoryKind::Actor],
    )
    .unwrap();
    assert_eq!(actors.len(), 2, "cached default set is not reused");
    assert!(actors
        .iter()
        .all(|h| memory_kind(&store, &h.memory.id).unwrap() == MemoryKind::Actor));
    assert_eq!(
        kinds_key(&[MemoryKind::Link, MemoryKind::Actor, MemoryKind::Link]),
        "actor,link"
    );
    assert!(kind_allowed(&[], MemoryKind::Claim));
    assert!(!kind_allowed(&[], MemoryKind::Related));
}

/// TTL / retention audit (spec §5): operational cleanup touches only job
/// journals and telemetry, never Brain identities or typed memories; a
/// referenced canonical Actor cannot be deleted; deleting a memory retires
/// Link/Related records pointing at it.
#[test]
fn operational_ttl_never_removes_brain_identities_and_deletes_retire_edges() {
    let store = world();
    let count = |sql: &str| -> i64 { store.conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    let before = (
        count("SELECT COUNT(*) FROM brain_actors"),
        count("SELECT COUNT(*) FROM memories"),
        count("SELECT COUNT(*) FROM brain_actor_mentions"),
        count("SELECT COUNT(*) FROM atlas_article_index"),
    );
    let far = chrono::Utc::now() + chrono::Duration::days(3650);
    crate::job_cleanup::cleanup_batch(&store.conn, far).unwrap();
    assert_eq!(
        before,
        (
            count("SELECT COUNT(*) FROM brain_actors"),
            count("SELECT COUNT(*) FROM memories"),
            count("SELECT COUNT(*) FROM brain_actor_mentions"),
            count("SELECT COUNT(*) FROM atlas_article_index"),
        )
    );
    // A mentioned actor survives any delete path.
    let actor = store
        .list_memories_by_kind(Some(MemoryKind::Actor), "")
        .unwrap()[0]
        .id
        .clone();
    assert!(store.delete_memory(&actor).is_err());
    assert!(store
        .conn
        .execute("DELETE FROM memories WHERE id=?1", [&actor])
        .is_err());
    assert!(store.get_memory(&actor).unwrap().is_some());
    // Deleting a Link's memory endpoint retires the Link.
    let claim = store.list_memories_by_kind(None, "").unwrap()[0].id.clone();
    store
        .conn
        .execute(
            "INSERT INTO memory_metadata(memory_id,memory_kind) VALUES(?1,'claim')",
            [&claim],
        )
        .unwrap();
    let link = store
        .write_link_memory(&crate::memory_metadata::LinkPayload {
            source: crate::memory_metadata::EndpointRef::Memory {
                memory_id: actor.clone(),
                kind: MemoryKind::Actor,
            },
            target: crate::memory_metadata::EndpointRef::Memory {
                memory_id: claim.clone(),
                kind: MemoryKind::Claim,
            },
            relation: "entity_connection".into(),
            rationale: "fixture".into(),
            receipts: vec!["fixture".into()],
            promoted_from: None,
        })
        .unwrap();
    store.delete_memory(&claim).unwrap();
    let state: String = store
        .conn
        .query_row(
            "SELECT lifecycle FROM memory_metadata WHERE memory_id=?1",
            [&link],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "retired");
}
