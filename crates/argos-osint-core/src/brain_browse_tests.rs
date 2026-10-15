use super::*;
use crate::brain_actors::{add_memory_actor_ref, BindRequest};
use crate::memory_metadata::{ActorSubtype, IdentityState};
use serde_json::json;

fn actor(store: &Store, name: &str, scope: &str) -> String {
    store
        .bind_actor(&BindRequest {
            name,
            subtype: ActorSubtype::Person,
            scope,
            identifiers: &[],
            external: None,
            provenance: json!({"test":true}),
        })
        .unwrap()
        .actor_id
}

fn claim(store: &Store, text: &str) -> Memory {
    let m = store
        .add_memory(
            text,
            "fact",
            false,
            MemorySource {
                app: "user".into(),
                conversation_id: "test".into(),
                message_id: None,
                reference: None,
            },
        )
        .unwrap();
    store.mark_user_claim(&m.id).unwrap();
    m
}

#[test]
fn browsing_groups_names_before_find_without_resolving_identities() {
    let s = Store::memory().unwrap();
    let a = actor(&s, "Ada Lovelace", "article:1");
    let b = actor(&s, "ADA LOVELACE", "article:2");
    let c = actor(&s, "Ada Lovelace", "article:3");
    s.merge_actors(&b, &a, &json!({"receipt":"confirmed"}))
        .unwrap();
    let m = claim(&s, "Ada described the engine");
    s.add_memory("internal note", "fact", false, m.source.clone())
        .unwrap();
    let groups = s.actor_groups().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].actor_ids.len(), 3);
    assert_eq!(
        actors::resolve(&s.conn, &c).unwrap(),
        c,
        "grouping cannot merge identity"
    );
    assert_eq!(s.brain_browse(None, "").unwrap().len(), 2);
    assert_eq!(
        s.brain_browse(Some(MemoryKind::Actor), "ada")
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        s.brain_browse(Some(MemoryKind::Inference), "")
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn batched_browse_counts_preserve_ingest_authority_for_large_cohorts() {
    let store = Store::memory().unwrap();
    for group in 0..50 {
        for occurrence in 0..3 {
            let (council, port, witness) = (
                format!("Council{group}"),
                format!("Port{group}"),
                format!("Witness{group}x{occurrence}"),
            );
            let id = format!("a{group}x{occurrence}");
            let title = format!("{council} and {port} meet {witness}");
            crate::atlas_actor_index::tests::seed(
                &store,
                "cohort",
                &id,
                &title,
                &[&council, &port, &witness],
            );
            let mut a = store.atlas_article("cohort", &id).unwrap().unwrap();
            a.category = "economic".into();
            store.atlas_upsert_article(&a).unwrap();
        }
    }
    crate::related_articles::build(&store, "cohort").unwrap();
    assert!(
        crate::atlas_actor_index::index_run(&store, "cohort", &|| false)
            .unwrap()
            .complete()
    );
    let started = std::time::Instant::now();
    let rows = store.brain_browse_memories(None, "").unwrap();
    eprintln!(
        "batched Brain browse rows={} articles=150 elapsed_ms={}",
        rows.len(),
        started.elapsed().as_millis()
    );
    assert_eq!(rows.len(), 250);
    for key in ["council0", "port49", "witness0x0"] {
        let group = store.actor_group(key).unwrap().unwrap();
        let a = store.actor_group_associations(&group).unwrap();
        let expected = if key.starts_with("witness") { 1 } else { 3 };
        assert_eq!(a.articles.len(), expected);
        let row = rows.iter().find(|m| m.id == group.key()).unwrap();
        assert_eq!(
            row.category,
            format!("0 claims · 0 threads · {expected} articles")
        );
    }
}

#[test]
fn associations_union_all_members_two_actor_claims_and_more_than_forty_articles() {
    let mut s = Store::memory().unwrap();
    let a = actor(&s, "Ada", "article:1");
    let b = actor(&s, "Ada", "article:2");
    let other = actor(&s, "Charles", "article:3");
    let m = claim(&s, "Ada and Charles describe an engine");
    for id in [&a, &other] {
        add_memory_actor_ref(
            &s.conn,
            &m.id,
            id,
            "mentioned",
            IdentityState::Provisional,
            &json!({}),
        )
        .unwrap();
    }
    let Some(MemoryPayload::Claim(payload)) = s.memory_payload(&m.id).unwrap() else {
        panic!("typed claim")
    };
    assert_eq!(payload.actors.len(), 2);
    s.atlas_insert_run("run", "{}", "{}").unwrap();
    for n in 0..45 {
        let id = format!("article-{n}");
        crate::atlas_actor_index::tests::seed(
            &s,
            "run",
            &id,
            &format!("Ada and Acme article {n}"),
            &["Ada", "Acme"],
        );
        let mut article = s.atlas_article("run", &id).unwrap().unwrap();
        article.category = "economic".into();
        s.atlas_upsert_article(&article).unwrap();
        actors::record_mention(
            &s.conn,
            if n % 2 == 0 { &a } else { &b },
            "article",
            "run",
            &id,
            "",
            &json!([]),
            "fixture",
        )
        .unwrap();
    }
    crate::related_articles::build(&s, "run").unwrap();
    let thread = s.new_thread("Ada conversation").unwrap();
    s.record_thread_actor(&thread.id, "Ada", "person").unwrap();
    assert!(s
        .actor_group_associations(&s.actor_group("ada").unwrap().unwrap())
        .unwrap()
        .threads
        .is_empty());
    s.set_recall_insights(&thread.id, true).unwrap();
    s.record_thread_actor(&thread.id, "Ada", "person").unwrap();
    s.record_thread_actor(&thread.id, "Ada", "person").unwrap();
    s.set_recall_insights(&thread.id, false).unwrap();
    let group = s.actor_group("ada").unwrap().unwrap();
    let a = s.actor_group_associations(&group).unwrap();
    assert_eq!(a.claims.len(), 1);
    assert_eq!(a.threads.len(), 1);
    assert_eq!(a.articles.len(), 45);
    assert_eq!(s.claim_associations(&m.id).unwrap().actors.len(), 2);
    assert_eq!(s.actor_group_graph(&group).unwrap().nodes.len(), 48);
    crate::atlas_actor_index::tests::seed(&s, "duplicate", "article-0", "Ada alone", &["Ada"]);
    actors::record_mention(
        &s.conn,
        &b,
        "article",
        "duplicate",
        "article-0",
        "",
        &json!([]),
        "fixture",
    )
    .unwrap();
    let duplicate = s.actor_group_associations(&group).unwrap();
    assert_eq!(duplicate.articles.len(), 45);
    assert!(duplicate
        .articles
        .iter()
        .flat_map(|a| &a.occurrences)
        .any(|(run, _, _)| run == "duplicate"));

    s.delete_thread(&thread.id, false).unwrap();
    let a = s.actor_group_associations(&group).unwrap();
    assert!(a.threads.is_empty());
    assert_eq!(a.articles.len(), 45);
}
