use super::*;
use crate::memory_metadata::ActorSubtype;
use crate::store::Store;

fn req<'a>(name: &'a str, scope: &'a str) -> BindRequest<'a> {
    BindRequest {
        name,
        subtype: ActorSubtype::Person,
        scope,
        identifiers: &[],
        external: None,
        provenance: json!({}),
    }
}

fn count(store: &Store, sql: &str) -> i64 {
    store.conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn names_are_search_keys_not_identities() {
    let store = Store::memory().unwrap();
    let a = store
        .bind_actor(&req("Jane Doe", "atlas-article:a1"))
        .unwrap();
    let again = store
        .bind_actor(&req("  jane   DOE ", "atlas-article:a1"))
        .unwrap();
    assert_eq!(a.actor_id, again.actor_id, "same scope + name reuses");
    assert!(a.created && !again.created);
    let b = store
        .bind_actor(&req("Jane Doe", "recon-thread:t9"))
        .unwrap();
    assert_ne!(
        a.actor_id, b.actor_id,
        "different people with one name stay separate"
    );
    assert_eq!(
        b.candidates,
        vec![a.actor_id.clone()],
        "labelled candidate only"
    );
    assert_eq!(b.state, IdentityState::Provisional);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM brain_actors"), 2);
    assert_eq!(
        count(
            &store,
            "SELECT COUNT(*) FROM memory_metadata WHERE memory_kind='actor'"
        ),
        2
    );
    match store.memory_payload(&a.actor_id).unwrap().unwrap() {
        MemoryPayload::Actor(p) => {
            assert_eq!(p.display_name, "Jane Doe");
            assert_eq!(p.subtype, ActorSubtype::Person);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn trusted_identifiers_and_external_refs_resolve_one_actor() {
    let store = Store::memory().unwrap();
    let ids = vec!["@ada_osint".to_string()];
    let first = store
        .bind_actor(&BindRequest {
            name: "Ada",
            subtype: ActorSubtype::Account,
            scope: "recon:x",
            identifiers: &ids,
            external: Some(("recon_entities", "e1")),
            provenance: json!({}),
        })
        .unwrap();
    assert_eq!(first.state, IdentityState::Resolved);
    let second = store
        .bind_actor(&BindRequest {
            name: "Ada L.",
            subtype: ActorSubtype::Account,
            scope: "atlas-article:z",
            identifiers: &["@ADA_OSINT".to_string()],
            external: None,
            provenance: json!({}),
        })
        .unwrap();
    assert_eq!(second.actor_id, first.actor_id);
    assert_eq!(second.basis, "identifier");
    let by_ref = store
        .bind_actor(&BindRequest {
            external: Some(("recon_entities", "e1")),
            ..req("whatever", "elsewhere")
        })
        .unwrap();
    assert_eq!(by_ref.actor_id, first.actor_id);
    assert_eq!(by_ref.basis, "external_ref");
    // A person name is never a trusted identifier.
    let p1 = store
        .bind_actor(&BindRequest {
            identifiers: &ids,
            ..req("Ada", "s1")
        })
        .unwrap();
    let p2 = store
        .bind_actor(&BindRequest {
            identifiers: &ids,
            ..req("Ada", "s2")
        })
        .unwrap();
    assert_ne!(p1.actor_id, p2.actor_id);
}

#[test]
fn merge_redirects_preserve_lineage_and_split_restores() {
    let store = Store::memory().unwrap();
    let a = store.bind_actor(&req("Jane Doe", "s1")).unwrap().actor_id;
    let b = store.bind_actor(&req("J. Doe", "s2")).unwrap().actor_id;
    let now = "2026-10-13T00:00:00Z";
    record_mention(&store.conn, &a, "article", "r1", "x1", "h", &json!([]), now).unwrap();
    record_mention(&store.conn, &b, "article", "r2", "x2", "h", &json!([]), now).unwrap();
    store
        .merge_actors(&b, &a, &json!({"assessed":"same entity"}))
        .unwrap();
    assert_eq!(resolve(&store.conn, &b).unwrap(), a);
    assert_eq!(
        mentions(&store.conn, &a).unwrap().len(),
        2,
        "merged mentions read through"
    );
    // Old ID still exists (history), marked redirected; cycles are rejected.
    assert!(store.merge_actors(&a, &b, &json!({})).is_err());
    let lifecycle: String = store
        .conn
        .query_row(
            "SELECT lifecycle FROM memory_metadata WHERE memory_id=?1",
            [&b],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lifecycle, "redirected");
    // Rebinding the old source reads the survivor.
    assert_eq!(store.bind_actor(&req("J. Doe", "s2")).unwrap().actor_id, a);
    store.split_actor(&b).unwrap();
    assert_eq!(resolve(&store.conn, &b).unwrap(), b);
    assert_eq!(mentions(&store.conn, &a).unwrap().len(), 1);
}

#[test]
fn link_and_related_writers_validate_and_are_idempotent() {
    let store = Store::memory().unwrap();
    store.atlas_insert_run("r1", "{}", "{}").unwrap();
    let art = crate::store::AtlasArticleRow {
        run_id: "r1".into(),
        id: "a1".into(),
        title: "t".into(),
        ..Default::default()
    };
    store.atlas_upsert_article(&art).unwrap();
    let actor = store.bind_actor(&req("Jane Doe", "s")).unwrap().actor_id;
    let link = LinkPayload {
        source: EndpointRef::Memory {
            memory_id: actor.clone(),
            kind: MemoryKind::Actor,
        },
        target: EndpointRef::Article {
            run_id: "r1".into(),
            article_id: "a1".into(),
            revision: "h".into(),
        },
        relation: "mentioned_in".into(),
        rationale: String::new(),
        receipts: vec![],
        promoted_from: None,
    };
    let id = store.write_link_memory(&link).unwrap();
    assert_eq!(
        store.write_link_memory(&link).unwrap(),
        id,
        "repeat sighting, same memory"
    );
    assert_eq!(
        count(
            &store,
            "SELECT COUNT(*) FROM memory_metadata WHERE memory_kind='link'"
        ),
        1
    );
    let mut bad = link.clone();
    bad.relation = "same_as".into();
    assert!(
        store.write_link_memory(&bad).is_err(),
        "vocabulary enforced"
    );
    let mut bad = link.clone();
    bad.target = EndpointRef::Article {
        run_id: "r1".into(),
        article_id: "nope".into(),
        revision: String::new(),
    };
    assert!(
        store.write_link_memory(&bad).is_err(),
        "endpoint must exist"
    );
    let mut bad = link.clone();
    bad.source = EndpointRef::Memory {
        memory_id: actor.clone(),
        kind: MemoryKind::Claim,
    };
    assert!(
        store.write_link_memory(&bad).is_err(),
        "endpoint type checked"
    );
    let mut bad = link.clone();
    bad.relation = "supports".into();
    bad.promoted_from = Some(id.clone());
    assert!(
        store.write_link_memory(&bad).is_err(),
        "promotion must reference Related"
    );

    let related = RelatedPayload {
        source: link.target.clone(),
        target: link.source.clone(),
        shared_actors: vec![],
        basis: "candidate name: Jane Doe".into(),
        interpretation: String::new(),
        verification_gap: String::new(),
        source_revisions: vec![],
    };
    let rid = store.write_related_memory(&related).unwrap();
    let swapped = RelatedPayload {
        source: related.target.clone(),
        target: related.source.clone(),
        ..related.clone()
    };
    assert_eq!(
        store.write_related_memory(&swapped).unwrap(),
        rid,
        "endpoint order independent"
    );
    // Promotion retains the original Related record.
    let promoted = LinkPayload {
        relation: "supports".into(),
        promoted_from: Some(rid.clone()),
        ..link.clone()
    };
    store.write_link_memory(&promoted).unwrap();
    assert!(store.memory_payload(&rid).unwrap().is_some());
    // Structural memories never masquerade as recall evidence.
    store
        .add_memory(
            "Jane Doe testified before the committee",
            "fact",
            false,
            crate::brain::MemorySource {
                app: "manual".into(),
                conversation_id: "c".into(),
                message_id: None,
                reference: None,
            },
        )
        .unwrap();
    let hits = store.recall("Jane Doe", 10).unwrap();
    assert!(!hits.is_empty());
    let structural = store.structural_memory_ids().unwrap();
    assert!(hits.iter().all(|h| !structural.contains(&h.memory.id)));
}

#[test]
fn retiring_an_origin_keeps_actors_supported_elsewhere() {
    let store = Store::memory().unwrap();
    let now = "2026-10-13T00:00:00Z";
    let shared = store.bind_actor(&req("Acme", "s")).unwrap().actor_id;
    let lonely = store.bind_actor(&req("Solo", "s")).unwrap().actor_id;
    record_mention(
        &store.conn,
        &shared,
        "article",
        "rA",
        "x",
        "h",
        &json!([]),
        now,
    )
    .unwrap();
    record_mention(
        &store.conn,
        &shared,
        "article",
        "rB",
        "x",
        "h",
        &json!([]),
        now,
    )
    .unwrap();
    record_mention(
        &store.conn,
        &lonely,
        "article",
        "rA",
        "y",
        "h",
        &json!([]),
        now,
    )
    .unwrap();
    retire_origin(&store.conn, "article", "rA", now).unwrap();
    let state = |id: &str| -> String {
        store
            .conn
            .query_row(
                "SELECT lifecycle FROM memory_metadata WHERE memory_id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(state(&shared), "active");
    assert_eq!(state(&lonely), "retired");
    assert_eq!(mentions(&store.conn, &shared).unwrap().len(), 1);
}

fn memory(store: &Store, text: &str, conversation: &str) -> String {
    store
        .add_memory(
            text,
            "fact",
            false,
            crate::brain::MemorySource {
                app: "recon".into(),
                conversation_id: conversation.into(),
                message_id: None,
                reference: None,
            },
        )
        .unwrap()
        .id
}

fn claim(store: &Store, fp: &str, memory_id: &str, entity: &str) {
    store
        .conn
        .execute(
            "INSERT INTO insight_claims(fingerprint,memory_id,entity_id,predicate,object_value,topic,classification,confidence,created_at,updated_at)
             VALUES(?1,?2,?3,'p','o','t','claim',0.5,'x','x')",
            params![fp, memory_id, entity],
        )
        .unwrap();
    crate::brain_backfill::bind_claim_subject(&store.conn, fp).unwrap();
}

// §4 recall/graph: canonical Actor IDs decide "same actor"; names only offer
// labelled candidates; a merge makes both claims share one actor.
#[test]
fn related_memories_join_on_canonical_actor_ids() {
    use crate::related_memories::{RelatedLimits, RelationKind};
    let store = Store::memory().unwrap();
    let a = memory(&store, "Jane Doe chairs the port board", "thread-1");
    let b = memory(&store, "Jane Doe resigned from the port board", "thread-1");
    let c = memory(&store, "Jane Doe won a marathon", "thread-2");
    claim(&store, "fa", &a, "Jane Doe");
    claim(&store, "fb", &b, "jane doe");
    claim(&store, "fc", &c, "Jane Doe");
    let rows = store
        .related_memories(
            &a,
            RelatedLimits {
                explicit: 12,
                similar: 5,
            },
        )
        .unwrap();
    let rb = rows.iter().find(|r| r.memory_id == b).unwrap();
    assert_eq!(rb.kind, RelationKind::Explicit);
    assert!(rb.reason.starts_with("same actor:"), "{}", rb.reason);
    let rc = rows.iter().find(|r| r.memory_id == c).unwrap();
    assert_eq!(
        rc.kind,
        RelationKind::Similar,
        "different scope: candidate only"
    );
    assert!(rc.reason.contains("unconfirmed"));
    // An assessed merge makes the other thread's subject the same actor.
    let actor = |m: &str| -> String {
        store
            .conn
            .query_row(
                "SELECT actor_id FROM brain_memory_actor_refs WHERE memory_id=?1",
                [m],
                |r| r.get(0),
            )
            .unwrap()
    };
    store
        .merge_actors(&actor(&c), &actor(&a), &json!({"assessed": true}))
        .unwrap();
    let rows = store
        .related_memories(
            &a,
            RelatedLimits {
                explicit: 12,
                similar: 5,
            },
        )
        .unwrap();
    let rc = rows.iter().find(|r| r.memory_id == c).unwrap();
    assert_eq!(rc.kind, RelationKind::Explicit);
}
