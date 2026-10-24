//! Actor and source lifecycle tests (migration phase 3) against a real
//! SurrealDB 3.3.4 disk server or the embedded RocksDB engine (see
//! `support`). Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::storage::tx::{run_tx, RetryPolicy, TxError};
use argos_osint_core::storage::{atlas, codec, graph, insight, lifecycle, memory};
use support::*;
use surrealdb::engine::any::Any;
use surrealdb::Surreal;

async fn fresh(server: &Server, name: &str) -> Surreal<Any> {
    let db = server.root("argos", name).await;
    memory::apply_schema(&db).await.unwrap();
    insight::apply_schema(&db).await.unwrap();
    atlas::apply_schema(&db).await.unwrap();
    lifecycle::apply_schema(&db).await.unwrap();
    db
}

async fn sql(db: &Surreal<Any>, q: &str) {
    db.query(q).await.unwrap().check().unwrap();
}

async fn lc(db: &Surreal<Any>, id: &str) -> String {
    let mut r = db
        .query("SELECT VALUE lifecycle FROM ONLY type::record('memory', $id)")
        .bind(("id", id.to_string()))
        .await
        .unwrap();
    r.take::<Option<String>>(0).unwrap().unwrap()
}

/// An actor memory with its row and a graph node.
async fn actor(db: &Surreal<Any>, id: &str, scope: &str, pinned: bool) {
    db.query(
        "UPSERT type::record('memory', $id) SET text = $id, memory_kind = 'actor', lifecycle = 'active', pinned = $pin;
         UPSERT type::record('brain_actor', $id) SET display_name = $id, scope_key = $scope, created_at = '', updated_at = '';
         UPSERT type::record('brain_graph_node', ['memory', $id]) SET kind = 'memory', ref_id = $id, memory_kind = 'actor', lifecycle = 'active';",
    )
    .bind(("id", id.to_string()))
    .bind(("scope", scope.to_string()))
    .bind(("pin", pinned))
    .await
    .unwrap()
    .check()
    .unwrap();
}

/// An active Link memory `id` from node `from` to node `to` (node keys).
async fn link(db: &Surreal<Any>, id: &str, from: &str, to: &str) {
    db.query(
        "UPSERT type::record('memory', $id) SET text = $id, memory_kind = 'link', lifecycle = 'active';
         UPSERT type::record('brain_graph_node', ['memory', $id]) SET kind = 'memory', ref_id = $id, memory_kind = 'link', lifecycle = 'active';
         RELATE (type::record('brain_graph_node', $from))->(type::record('brain_graph_edge', $id))->(type::record('brain_graph_node', $to))
           SET meaning = 'link', owner = $id;",
    )
    .bind(("id", id.to_string()))
    .bind(("from", from.to_string()))
    .bind(("to", to.to_string()))
    .await
    .unwrap()
    .check()
    .unwrap();
}

async fn in_tx<T, F, Fut>(db: &Surreal<Any>, f: F) -> Result<T, TxError>
where
    F: FnMut(argos_osint_core::storage::tx::Tx) -> Fut,
    Fut: std::future::Future<Output = Result<T, TxError>>,
{
    Ok(run_tx(db, RetryPolicy::default(), f).await?.value)
}

#[tokio::test]
async fn retire_origin_retires_unsupported_actors_and_origin_links() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "life_origin").await;
    actor(&db, "a", "atlas:r1", false).await;
    actor(&db, "b", "atlas:r1", false).await;
    let art = codec::encode(&["article", "r1", "x", ""]);
    db.query(
        "UPSERT type::record('brain_graph_node', $art) SET kind = 'article', ref_id = 'x', origin = 'r1';
         CREATE brain_actor_mention:m1 SET actor_id = 'a', origin_kind = 'article', origin_scope = 'r1', origin_id = 'x', created_at = '';
         CREATE brain_actor_mention:m2 SET actor_id = 'b', origin_kind = 'article', origin_scope = 'r1', origin_id = 'x', created_at = '';
         CREATE brain_actor_mention:m3 SET actor_id = 'b', origin_kind = 'article', origin_scope = 'r2', origin_id = 'y', created_at = '';",
    )
    .bind(("art", art.clone()))
    .await
    .unwrap()
    .check()
    .unwrap();
    link(&db, "l1", &graph::memory_node("a"), &art).await;
    let (retired, changed) = in_tx(&db, |tx| async move {
        lifecycle::retire_origin_in(&tx, "article", "r1").await
    })
    .await
    .unwrap();
    assert_eq!(retired, 1);
    assert_eq!(lc(&db, "l1").await, "retired");
    assert_eq!(lc(&db, "a").await, "retired", "no support left");
    assert_eq!(lc(&db, "b").await, "active", "still mentioned by r2");
    assert!(changed.contains(&"a".to_string()) && changed.contains(&"l1".to_string()));
    let mut r = db
        .query("RETURN count(SELECT id FROM brain_graph_edge WHERE owner = 'l1')")
        .await
        .unwrap();
    assert_eq!(
        r.take::<Option<i64>>(0).unwrap(),
        Some(0),
        "retired link loses its edge"
    );
    // Nothing left to do: a second pass changes nothing.
    let (_, again) = in_tx(&db, |tx| async move {
        lifecycle::retire_origin_in(&tx, "article", "r1").await
    })
    .await
    .unwrap();
    assert!(again.is_empty());
}

#[tokio::test]
async fn redirect_families_share_support_and_cycles_abort() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "life_family").await;
    actor(&db, "c", "atlas:r", true).await;
    actor(&db, "d", "atlas:r", false).await;
    actor(&db, "e", "atlas:r", false).await;
    in_tx(&db, |tx| async move {
        lifecycle::add_redirect_in(&tx, "c", "d", "t").await
    })
    .await
    .unwrap();
    let err = in_tx(&db, |tx| async move {
        lifecycle::add_redirect_in(&tx, "d", "c", "t").await
    })
    .await
    .unwrap_err();
    assert!(matches!(err, TxError::Abort(_)), "{err:?}");
    sql(&db, "UPDATE memory:c SET lifecycle = 'redirected'").await;
    in_tx(&db, |tx| async move {
        lifecycle::reconcile_lifecycle_in(&tx).await
    })
    .await
    .unwrap();
    assert_eq!(
        lc(&db, "d").await,
        "active",
        "pinned member supports its root"
    );
    assert_eq!(
        lc(&db, "c").await,
        "redirected",
        "redirected actors keep lifecycle"
    );
    assert_eq!(lc(&db, "e").await, "retired", "unsupported actor retires");
}

#[tokio::test]
async fn reconcile_lifecycle_rehomes_sources_and_retires_orphans() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "life_sources").await;
    atlas::upsert_article(
        &db,
        &argos_osint_core::store::AtlasArticleRow {
            run_id: "r2".into(),
            id: "art".into(),
            seen_at: "2".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    sql(
        &db,
        r#"UPSERT memory:k1 SET text = 'k1', memory_kind = 'claim', lifecycle = 'active', source_json = '{"app":"atlas"}';
           UPSERT memory:k2 SET text = 'k2', memory_kind = 'claim', lifecycle = 'active', source_json = '{"app":"atlas"}';
           UPSERT memory:k3 SET text = 'k3', memory_kind = 'claim', lifecycle = 'active', source_json = '{"app":"manual"}';
           CREATE insight_claim:f1 SET memory_id = 'k1', entity_id = 'e', predicate = 'p', object_value = 'o', topic = 't', classification = 'fact', confidence = 0.5, created_at = '', updated_at = '';
           CREATE insight_claim:f2 SET memory_id = 'k2', entity_id = 'e', predicate = 'p', object_value = 'o', topic = 't', classification = 'fact', confidence = 0.5, created_at = '', updated_at = '';
           CREATE insight_claim:f3 SET memory_id = 'k3', entity_id = 'e', predicate = 'p', object_value = 'o', topic = 't', classification = 'fact', confidence = 0.5, created_at = '', updated_at = '';
           CREATE insight_source:s1 SET fingerprint = 'f1', run_id = 'r1', answer_id = 'atlas-r1', call_id = 'art';
           CREATE insight_source:s2 SET fingerprint = 'f2', run_id = 'r1', answer_id = 'atlas-r1', call_id = 'gone';"#,
    )
    .await;
    let changed = in_tx(&db, |tx| async move {
        lifecycle::reconcile_lifecycle_in(&tx).await
    })
    .await
    .unwrap();
    let mut r = db
        .query("SELECT VALUE [run_id, answer_id] FROM insight_source WHERE fingerprint = 'f1'; RETURN count(SELECT id FROM insight_source WHERE fingerprint = 'f2');")
        .await
        .unwrap();
    let homes: Vec<Vec<String>> = r.take(0).unwrap();
    assert_eq!(
        homes,
        vec![vec!["r2".to_string(), "atlas-r2".to_string()]],
        "re-homed"
    );
    assert_eq!(
        r.take::<Option<i64>>(1).unwrap(),
        Some(0),
        "gone article's source dropped"
    );
    assert_eq!(lc(&db, "k1").await, "active");
    assert_eq!(lc(&db, "k2").await, "retired");
    assert_eq!(
        lc(&db, "k3").await,
        "active",
        "non-article claims are left alone"
    );
    assert_eq!(changed, vec!["k2".to_string()]);
}

#[tokio::test]
async fn relationships_to_retired_memories_retire_transitively() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "life_links").await;
    sql(
        &db,
        "UPSERT memory:gone SET text = 'g', memory_kind = 'claim', lifecycle = 'retired';
         UPSERT memory:ok SET text = 'o', memory_kind = 'claim', lifecycle = 'active';",
    )
    .await;
    for (id, lcy) in [("gone", "retired"), ("ok", "active")] {
        db.query("UPSERT type::record('brain_graph_node', $k) SET kind = 'memory', ref_id = $id, memory_kind = 'claim', lifecycle = $lc")
            .bind(("k", graph::memory_node(id)))
            .bind(("id", id.to_string()))
            .bind(("lc", lcy.to_string()))
            .await
            .unwrap()
            .check()
            .unwrap();
    }
    let (g, o) = (graph::memory_node("gone"), graph::memory_node("ok"));
    link(&db, "l1", &g, &o).await;
    let l1 = graph::memory_node("l1");
    link(&db, "l2", &l1, &o).await;
    link(&db, "l3", &o, &o).await;
    in_tx(&db, |tx| async move {
        lifecycle::reconcile_lifecycle_in(&tx).await
    })
    .await
    .unwrap();
    assert_eq!(lc(&db, "l1").await, "retired");
    assert_eq!(
        lc(&db, "l2").await,
        "retired",
        "a link to a retired link retires"
    );
    assert_eq!(lc(&db, "l3").await, "active");
}

#[tokio::test]
async fn origin_lookups_use_their_indexes() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "life_plans").await;
    for (q, index) in [
        (
            lifecycle::ORIGIN_MENTIONS_SURQL,
            "brain_actor_mention_origin",
        ),
        (lifecycle::ORIGIN_NODES_SURQL, "brain_graph_node_origin"),
    ] {
        let mut r = db
            .query(format!("{q} EXPLAIN FULL"))
            .bind(("kind", "article".to_string()))
            .bind(("scope", "r".to_string()))
            .await
            .unwrap()
            .check()
            .unwrap();
        let p = format!("{:?}", r.take::<surrealdb::types::Value>(0).unwrap());
        assert!(p.contains(index), "{q}: {p}");
        assert!(!p.contains("Iterate Table"), "{q}: {p}");
    }
}
