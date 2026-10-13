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
