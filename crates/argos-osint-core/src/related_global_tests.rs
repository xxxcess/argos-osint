use super::*;
use crate::atlas_actor_index::tests::seed;

#[test]
fn browsing_requires_a_recorded_ingest_pair_and_never_adds_name_suggestions() {
    let store = Store::memory().unwrap();
    seed(&store, "rA", "a1", "Ada meets Acme", &["Ada", "Acme"]);
    seed(&store, "rB", "b1", "Ada joins Acme", &["Ada", "Acme"]);
    assert!(store
        .related_articles_global("rA", "a1", RelatedScope::AllRetained, None, 20)
        .unwrap()
        .rows
        .is_empty());
    index(&store, "rA");
    index(&store, "rB");
    assert_eq!(
        store
            .related_articles_global("rA", "a1", RelatedScope::AllRetained, None, 20)
            .unwrap()
            .total,
        1
    );
    seed(&store, "rB", "b2", "Ada repeats Ada", &["Ada"]);
    index(&store, "rB");
    let page = store
        .related_articles_global("rA", "a1", RelatedScope::AllRetained, None, 20)
        .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.rows[0].article.id, "b1");
    // Editing content while preserving both names must invalidate old receipts.
    let mut candidate = store.atlas_article("rB", "b1").unwrap().unwrap();
    candidate.title.push_str(" updated");
    store.atlas_upsert_article(&candidate).unwrap();
    assert!(store
        .related_articles_global("rA", "a1", RelatedScope::AllRetained, None, 20)
        .unwrap()
        .rows
        .is_empty());
    assert!(!store
        .has_ingest_actor_association(&candidate, "ada")
        .unwrap());
}

fn index(store: &Store, run: &str) {
    assert!(crate::atlas_actor_index::index_run(store, run, &|| false)
        .unwrap()
        .complete());
}

fn world() -> Store {
    let store = Store::memory().unwrap();
    seed(
        &store,
        "rA",
        "a1",
        "Jane Doe visits Kyiv with Acme",
        &["Jane Doe", "Acme"],
    );
    seed(
        &store,
        "rA",
        "a2",
        "Jane Doe meets envoy at Acme",
        &["Jane Doe", "Acme"],
    );
    seed(
        &store,
        "rB",
        "b1",
        "Jane Doe resigns from Acme",
        &["Jane Doe", "Acme"],
    );
    // b-cycle retains a2 again (same URL).
    seed(
        &store,
        "rB",
        "a2",
        "Jane Doe meets envoy at Acme",
        &["Jane Doe", "Acme"],
    );
    crate::related_articles::build(&store, "rA").unwrap();
    crate::related_articles::build(&store, "rB").unwrap();
    index(&store, "rA");
    index(&store, "rB");
    store
}

#[test]
fn all_retained_is_default_one_row_per_url_with_every_origin() {
    assert_eq!(RelatedScope::default(), RelatedScope::AllRetained);
    let store = world();
    let page = store
        .related_articles_global("rA", "a1", RelatedScope::AllRetained, None, 20)
        .unwrap();
    let keys: Vec<_> = page.rows.iter().map(|r| r.url_key.as_str()).collect();
    assert!(keys.contains(&"https://x.test/a2"), "{keys:?}");
    assert!(keys.contains(&"https://x.test/b1"), "{keys:?}");
    assert!(!keys.contains(&"https://x.test/a1"), "source excluded");
    let mut unique = keys.clone();
    unique.dedup();
    assert_eq!(unique.len(), keys.len(), "one row per URL");
    let a2 = page
        .rows
        .iter()
        .find(|r| r.url_key.ends_with("/a2"))
        .unwrap();
    assert_eq!(a2.cycles, 2, "{a2:?}");
    assert_eq!(a2.origins.len(), 2);
    // Different provisional actors → name match only, never established.
    assert!(page.rows.iter().all(|r| r.tentative));
    assert!(a2.summary().contains("2026-10-12 · 2 cycles"));
    assert!(a2.summary().ends_with("· tentative"));
}

#[test]
fn this_cycle_scope_stays_inside_the_source_cycle() {
    let store = world();
    let page = store
        .related_articles_global("rA", "a1", RelatedScope::ThisCycle, None, 20)
        .unwrap();
    assert!(page
        .rows
        .iter()
        .all(|r| r.origins.iter().all(|o| o.run_id == "rA")));
    assert!(!page.rows.iter().any(|r| r.url_key.ends_with("/b1")));
}

#[test]
fn merged_canonical_actor_establishes_link_and_actor_articles_spans_cycles() {
    let store = world();
    let actor_of = |id: &str| -> String {
        store
            .conn
            .query_row(
                "SELECT m.actor_id FROM brain_actor_mentions m JOIN brain_actors a ON a.actor_id=m.actor_id WHERE m.origin_id=?1 AND a.display_name='Jane Doe' LIMIT 1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    };
    let (a1, b1) = (actor_of("a1"), actor_of("b1"));
    assert_ne!(a1, b1);
    store
        .merge_actors(&b1, &a1, &serde_json::json!({"analyst":"fixture"}))
        .unwrap();
    let page = store
        .related_articles_global("rA", "a1", RelatedScope::AllRetained, None, 20)
        .unwrap();
    let b = page
        .rows
        .iter()
        .find(|r| r.url_key.ends_with("/b1"))
        .unwrap();
    assert!(
        b.tentative,
        "shared mentions remain tentative even after identity resolution"
    );
    let articles = store.actor_articles(&b1, None, 20).unwrap();
    let keys: Vec<_> = articles.rows.iter().map(|r| r.url_key.clone()).collect();
    assert!(keys.contains(&"https://x.test/a1".to_string()));
    assert!(keys.contains(&"https://x.test/b1".to_string()));
}

#[test]
fn cursor_pages_without_overlap() {
    let store = world();
    let first = store
        .related_articles_global("rA", "a1", RelatedScope::AllRetained, None, 1)
        .unwrap();
    assert_eq!(first.rows.len(), 1);
    let next = first.next_cursor.clone().expect("more");
    let second = store
        .related_articles_global("rA", "a1", RelatedScope::AllRetained, Some(&next), 1)
        .unwrap();
    assert_ne!(first.rows[0].url_key, second.rows[0].url_key);
    assert_eq!(first.total, second.total);
}

#[test]
fn actor_graph_has_one_node_per_canonical_actor_and_distinct_edge_kinds() {
    use crate::recon::{GraphEdgeKind, GraphNodeKind};
    let store = world();
    let actor_of = |id: &str| -> String {
        store
            .conn
            .query_row(
                "SELECT actor_id FROM brain_actor_mentions WHERE origin_id=?1 AND origin_scope='rA' LIMIT 1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    };
    let (a1, a2) = (actor_of("a1"), actor_of("a2"));
    store
        .merge_actors(&a2, &a1, &serde_json::json!({"analyst":"fixture"}))
        .unwrap();
    // Opening the merged-away actor resolves to the canonical node.
    let graph = store.graph_for_memory(&a2).unwrap();
    let actors: Vec<_> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == GraphNodeKind::Actor)
        .collect();
    assert_eq!(actors.len(), 1);
    assert_eq!(actors[0].id, format!("actor:{a1}"));
    let links = graph
        .edges
        .iter()
        .filter(|e| e.kind == GraphEdgeKind::Link)
        .count();
    assert!(links >= 2, "{graph:?}");
    assert!(graph
        .edges
        .iter()
        .filter(|e| e.kind == GraphEdgeKind::Link)
        .all(|e| e.directive.as_deref() == Some("mentioned_in")));
    let related: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.kind == GraphEdgeKind::Related)
        .collect();
    let mut pairs: Vec<_> = related
        .iter()
        .map(|e| {
            let mut k = [e.from.clone(), e.to.clone()];
            k.sort();
            k
        })
        .collect();
    let n = pairs.len();
    pairs.sort();
    pairs.dedup();
    assert_eq!(pairs.len(), n, "one Related edge per unordered pair");
    assert_ne!(GraphEdgeKind::Link.verb(), GraphEdgeKind::Related.verb());
}
