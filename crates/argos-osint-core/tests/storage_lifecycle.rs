//! Actor and source lifecycle tests (migration phase 3) against a real
//! SurrealDB 3.3.4 disk server or the embedded RocksDB engine (see
//! `support`). Run locally through `python3 scripts/test_surreal.py --mode embedded`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::storage::tx::{run_tx, RetryPolicy, TxError};
use argos_osint_core::storage::{atlas, codec, counters, graph, insight, lifecycle, memory};
use support::*;
use surrealdb::engine::any::Any;
use surrealdb::Surreal;

async fn fresh(server: &Server, name: &str) -> Surreal<Any> {
    let db = server.root("argos", name).await;
    memory::apply_schema(&db).await.unwrap();
    insight::apply_schema(&db).await.unwrap();
    atlas::apply_schema(&db).await.unwrap();
    lifecycle::apply_schema(&db).await.unwrap();
    counters::apply_schema(&db).await.unwrap();
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
         UPSERT type::record('brain_graph_node', $node) SET kind = 'memory', ref_id = $id, memory_kind = 'actor', lifecycle = 'active';",
    )
    .bind(("id", id.to_string()))
    .bind(("scope", scope.to_string()))
    .bind(("pin", pinned))
    .bind(("node", graph::memory_node(id)))
    .await
    .unwrap()
    .check()
    .unwrap();
}

/// An active Link memory `id` from node `from` to node `to` (node keys).
async fn link(db: &Surreal<Any>, id: &str, from: &str, to: &str) {
    db.query(
        "UPSERT type::record('memory', $id) SET text = $id, memory_kind = 'link', lifecycle = 'active';
         UPSERT type::record('brain_graph_node', $node) SET kind = 'memory', ref_id = $id, memory_kind = 'link', lifecycle = 'active';
         RELATE (type::record('brain_graph_node', $from))->(type::record('brain_graph_edge', $id))->(type::record('brain_graph_node', $to))
           SET meaning = 'link', owner = $id;",
    )
    .bind(("id", id.to_string()))
    .bind(("from", from.to_string()))
    .bind(("to", to.to_string()))
    .bind(("node", graph::memory_node(id)))
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
        let covering =
            q == lifecycle::ORIGIN_NODES_SURQL && p.contains("brain_graph_node_occurrence");
        assert!(p.contains(index) || covering, "{q}: {p}");
        assert!(
            !p.contains("Iterate Table") && !p.contains("TableScan"),
            "{q}: {p}"
        );
    }
}

async fn support_epoch(db: &Surreal<Any>) -> i64 {
    db.query("RETURN (SELECT VALUE revision FROM ONLY lifecycle_support_epoch:current) ?? 0")
        .await
        .unwrap()
        .take::<Option<i64>>(0)
        .unwrap()
        .unwrap()
}

async fn drain_affected(
    db: &Surreal<Any>,
    scope: &lifecycle::ReconcileScope,
    mut cursor: Option<lifecycle::ReconcileCursor>,
) -> (usize, Vec<String>) {
    let mut changed = std::collections::BTreeSet::new();
    for step in 1..=2048 {
        let scope = scope.clone();
        let next = cursor.clone();
        let progress = in_tx(db, move |tx| {
            let (scope, next) = (scope.clone(), next.clone());
            async move { lifecycle::reconcile_affected_in(&tx, &scope, next.as_ref()).await }
        })
        .await
        .expect("bounded affected continuation");
        assert!(progress.examined <= 256, "step {step}: {progress:?}");
        changed.extend(progress.changed_memory_ids);
        cursor = progress.next.map(|next| {
            // A continuation contains no process-local state and is usable by
            // another transaction after a durable serialization round trip.
            serde_json::from_str(&serde_json::to_string(&next).unwrap()).unwrap()
        });
        if cursor.is_none() {
            return (step, changed.into_iter().collect());
        }
    }
    panic!("affected frontier failed to converge within its fixture bound");
}

#[tokio::test]
async fn scoped_closure_crosses_256_relationships_without_retiring_unrelated_siblings() {
    let Some(mut server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "life_bounded_chain").await;
    sql(
        &db,
        "UPSERT memory:gone SET text='gone',memory_kind='claim',lifecycle='retired';
        UPSERT memory:kept SET text='kept',memory_kind='claim',lifecycle='active';",
    )
    .await;
    for (id, state) in [("gone", "retired"), ("kept", "active")] {
        db.query("UPSERT type::record('brain_graph_node',$id) SET kind='memory',ref_id=$ref,memory_kind='claim',lifecycle=$state")
            .bind(("id",graph::memory_node(id))).bind(("ref",id.to_string()))
            .bind(("state",state.to_string())).await.unwrap().check().unwrap();
    }
    let mut previous = graph::memory_node("gone");
    for index in 0..270 {
        let id = format!("chain-{index:04}");
        link(&db, &id, &previous, &graph::memory_node("kept")).await;
        previous = graph::memory_node(&id);
    }
    link(
        &db,
        "unrelated",
        &graph::memory_node("kept"),
        &graph::memory_node("kept"),
    )
    .await;
    let scope = lifecycle::ReconcileScope {
        keys: vec![lifecycle::ReconcileKey::Relationship {
            memory_id: "gone".into(),
        }],
    };
    let first = in_tx(&db, |tx| {
        let scope = scope.clone();
        async move { lifecycle::reconcile_affected_in(&tx, &scope, None).await }
    })
    .await
    .unwrap();
    if first.next.is_none() {
        let mut diagnostic = db.query("SELECT id,owner,record::id(in) AS input FROM brain_graph_edge WHERE in=$node ORDER BY id LIMIT 2;
            SELECT id,owner,record::id(in) AS input FROM brain_graph_edge WITH INDEX brain_graph_edge_input WHERE in=$node AND meaning INSIDE ['link','related'] ORDER BY id LIMIT 2;
            SELECT phase,priority,state,payload_json FROM lifecycle_reconcile_frontier ORDER BY phase,priority LIMIT 12;")
            .bind(("node",surrealdb::types::RecordId::new("brain_graph_node",graph::memory_node("gone"))))
            .await.unwrap().check().unwrap();
        for index in 0..3 {
            eprintln!(
                "closure diagnostic {index}: {:?}",
                diagnostic.take::<surrealdb::types::Value>(index).unwrap()
            );
        }
    }
    assert!(first.examined <= 256);
    assert!(
        first.next.is_some(),
        "270 dependent identities cannot finish in one bounded step"
    );
    assert_eq!(
        lc(&db, "chain-0269").await,
        "active",
        "overflow is deferred rather than silently discarded"
    );
    drop(db);
    server.restart();
    let db = fresh(&server, "life_bounded_chain").await;
    let (steps, mut changed) = drain_affected(&db, &scope, first.next).await;
    changed.extend(first.changed_memory_ids);
    changed.sort();
    changed.dedup();
    assert!(steps > 1);
    assert_eq!(changed.len(), 270);
    for index in 0..270 {
        assert_eq!(lc(&db, &format!("chain-{index:04}")).await, "retired");
    }
    assert_eq!(lc(&db, "unrelated").await, "active");
}

#[tokio::test]
async fn support_batch_is_once_per_actual_transaction_and_cannot_escape_commit() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "life_batch_owner").await;
    actor(&db, "a", "atlas:r", false).await;
    let escaped = in_tx(&db, |tx| async move {
        let batch = lifecycle::SupportBatch::new(&tx);
        for revision in ["one", "two"] {
            lifecycle::record_mention_in_batch(
                &tx,
                "a",
                "article",
                "r",
                "x",
                revision,
                "[]",
                "fixed",
                &batch.clone(),
            )
            .await?;
        }
        Ok(batch)
    })
    .await
    .unwrap();
    assert_eq!(
        support_epoch(&db).await,
        1,
        "clones in the actual transaction share one confirmed bump"
    );
    let wrong_tx = in_tx(&db, |tx| {
        let escaped = escaped.clone();
        async move {
            lifecycle::record_mention_in_batch(
                &tx, "a", "article", "r", "x", "bad", "[]", "fixed", &escaped,
            )
            .await
        }
    })
    .await
    .unwrap_err();
    assert!(matches!(wrong_tx, TxError::Abort(_)), "{wrong_tx:?}");
    assert_eq!(support_epoch(&db).await, 1);
    in_tx(&db, |tx| async move {
        let batch = lifecycle::SupportBatch::new(&tx);
        lifecycle::record_mention_in_batch(
            &tx, "a", "article", "r", "x", "three", "[]", "fixed", &batch,
        )
        .await
    })
    .await
    .unwrap();
    assert_eq!(
        support_epoch(&db).await,
        2,
        "a later real mutation cannot inherit a prior transaction's once gate"
    );
    let mut result = db
        .query("SELECT VALUE origin_revision FROM brain_actor_mention ORDER BY origin_revision")
        .await
        .unwrap();
    assert_eq!(
        result.take::<Vec<String>>(0).unwrap(),
        vec!["one", "three", "two"]
    );
}

#[tokio::test]
async fn bounded_capture_rejects_a_mention_only_support_change_and_a_foreign_transaction_permit() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "life_capture_fence").await;
    actor(&db, "a", "atlas:r", false).await;
    // Reverse discovery itself spans pages, even though all references belong
    // to one actor. No canonical memory write accompanies the later mention.
    let mut fixture = String::new();
    for index in 0..270 {
        fixture.push_str(&format!("CREATE brain_actor_mention:m{index:04} SET actor_id='a',origin_kind='article',origin_scope='r',origin_id='x',origin_revision='{index}',created_at='';"));
    }
    sql(&db, &fixture).await;
    let scope = lifecycle::ReconcileScope {
        keys: vec![lifecycle::ReconcileKey::Source {
            run_id: "r".into(),
            article_id: "x".into(),
        }],
    };
    let first = in_tx(&db, |tx| {
        let scope = scope.clone();
        async move { lifecycle::capture_affected_in(&tx, &scope, None).await }
    })
    .await
    .unwrap();
    assert!(!first.ready);
    assert!(first.examined <= 256);
    in_tx(&db, |tx| async move {
        lifecycle::record_mention_in(
            &tx,
            "a",
            "article",
            "r",
            "new-support",
            "new",
            "[]",
            "fixed",
        )
        .await
    })
    .await
    .unwrap();
    let error = in_tx(&db, |tx| {
        let (scope, cursor) = (scope.clone(), first.cursor.clone());
        async move { lifecycle::capture_affected_in(&tx, &scope, Some(&cursor)).await }
    })
    .await
    .unwrap_err();
    assert!(
        format!("{error}").contains("capture_conflict_restart_required"),
        "{error:?}"
    );
    assert_eq!(
        lc(&db, "a").await,
        "active",
        "conflict must not continue using stale absence of support"
    );
    let replacement = in_tx(&db, |tx| {
        let scope = scope.clone();
        async move { lifecycle::capture_affected_in(&tx, &scope, None).await }
    })
    .await
    .unwrap();
    assert!(!replacement.ready);
    in_tx(&db, |tx| async move {
        lifecycle::retire_origin_in(&tx, "article", "r").await
    })
    .await
    .unwrap();
    let removed = in_tx(&db, |tx| {
        let (scope, cursor) = (scope.clone(), replacement.cursor.clone());
        async move { lifecycle::capture_affected_in(&tx, &scope, Some(&cursor)).await }
    })
    .await
    .unwrap_err();
    assert!(
        format!("{removed}").contains("capture_conflict_restart_required"),
        "removing mention-only support must invalidate a saved capture: {removed:?}"
    );
    let small = lifecycle::ReconcileScope {
        keys: vec![lifecycle::ReconcileKey::Actor {
            actor_id: "a".into(),
        }],
    };
    let ready = in_tx(&db, |tx| {
        let small = small.clone();
        async move { lifecycle::capture_affected_in(&tx, &small, None).await }
    })
    .await
    .unwrap();
    assert!(ready.ready);
    let error = in_tx(&db, |tx| {
        let ready = ready.clone();
        async move { lifecycle::SupportBatch::for_capture(&tx, &ready) }
    })
    .await
    .unwrap_err();
    assert!(
        matches!(error, TxError::Abort(_)),
        "a ready value from a committed transaction is not a mutation capability"
    );
}

#[tokio::test]
async fn scoped_rehome_preserves_the_existing_winner_and_last_shared_evidence() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "life_scoped_rehome").await;
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
    // Classification state is deliberately non-current: retention is about
    // the occurrence's existence, never inferred admission authority.
    db.query("UPDATE type::record('atlas_article',$id) SET admission_state='rejected';")
        .bind(("id", codec::encode(&["r2", "art"])))
        .await
        .unwrap()
        .check()
        .unwrap();
    let old = codec::encode(&["shared", "atlas-r1", "art"]);
    let winner = codec::encode(&["shared", "atlas-r2", "art"]);
    db.query("UPSERT memory:shared SET text='shared',memory_kind='claim',lifecycle='active',source_json=$source;
        CREATE insight_claim:shared SET memory_id='shared',entity_id='e',predicate='p',object_value='o',topic='t',classification='fact',confidence=0.5,created_at='',updated_at='';
        CREATE type::record('insight_source',$old) SET fingerprint='shared',run_id='r1',answer_id='atlas-r1',call_id='art',source_url='https://fixture.invalid/old';
        CREATE type::record('insight_source',$winner) SET fingerprint='shared',run_id='r2',answer_id='atlas-r2',call_id='art',source_url='https://fixture.invalid/winner',published_at='fixed';")
        .bind(("old",old.clone())).bind(("winner",winner.clone()))
        .bind(("source",r#"{"app":"atlas"}"#.to_string())).await.unwrap().check().unwrap();
    let snapshot = |db: Surreal<Any>, winner: String| async move {
        db.query("RETURN type::string((SELECT * FROM ONLY type::record('insight_source',$winner)))")
            .bind(("winner", winner))
            .await
            .unwrap()
            .take::<Option<String>>(0)
            .unwrap()
            .unwrap()
    };
    let before = snapshot(db.clone(), winner.clone()).await;
    let scope = lifecycle::ReconcileScope {
        keys: vec![lifecycle::ReconcileKey::Source {
            run_id: "r1".into(),
            article_id: "art".into(),
        }],
    };
    drain_affected(&db, &scope, None).await;
    assert_eq!(
        snapshot(db.clone(), winner).await,
        before,
        "an equivalent target collision must not overwrite the winning receipt"
    );
    let exists = db
        .query("RETURN record::exists(type::record('insight_source',$old))")
        .bind(("old", old))
        .await
        .unwrap()
        .take::<Option<bool>>(0)
        .unwrap();
    assert_eq!(
        exists,
        Some(false),
        "the redundant old key is removed even when the target already exists"
    );
    assert_eq!(
        lc(&db, "shared").await,
        "active",
        "retained rejected articles are still historical evidence, never deletion"
    );
    sql(&db, "DELETE atlas_article; ").await;
    let scope = lifecycle::ReconcileScope {
        keys: vec![lifecycle::ReconcileKey::Source {
            run_id: "r2".into(),
            article_id: "art".into(),
        }],
    };
    drain_affected(&db, &scope, None).await;
    assert_eq!(
        lc(&db, "shared").await,
        "retired",
        "only removing the final retained origin retires its eligible claim"
    );
}

#[tokio::test]
async fn paged_refresh_preserves_same_id_republication_and_confirmed_siblings() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "life_refresh_stamp").await;
    actor(&db, "a", "atlas:r", false).await;
    let article = argos_osint_core::store::AtlasArticleRow {
        run_id: "r".into(),
        id: "x".into(),
        title: "Fixture port closure".into(),
        url: "https://fixture.invalid/x".into(),
        ..Default::default()
    };
    let source_revision = argos_osint_core::atlas_metadata::input_hash(&article);
    atlas::upsert_article(&db, &article).await.unwrap();
    in_tx(&db, |tx| async move {
        let batch = lifecycle::SupportBatch::new(&tx);
        for index in 0..270 {
            lifecycle::record_mention_in_batch(
                &tx,
                "a",
                "article",
                "r",
                "x",
                &format!("revision-{index:04}"),
                "[]",
                "fixed",
                &batch,
            )
            .await?;
        }
        Ok(())
    })
    .await
    .unwrap();
    let article_node = codec::encode(&["article", "r", "x", &source_revision]);
    db.query("UPSERT type::record('brain_graph_node',$node) SET kind='article',origin='r',ref_id='x',revision=$revision;")
        .bind(("node",article_node.clone())).bind(("revision",source_revision.clone())).await.unwrap().check().unwrap();
    link(
        &db,
        "confirmed-sibling",
        &graph::memory_node("a"),
        &article_node,
    )
    .await;
    for id in ["ingest-old", "ingest-republished"] {
        link(&db, id, &graph::memory_node("a"), &article_node).await;
        db.query("UPDATE type::record('memory',$id) SET source_json=$source,payload_json=$payload;")
            .bind(("id",id.to_string()))
            .bind(("source",r#"{"app":"atlas_ingest"}"#.to_string()))
            .bind(("payload",serde_json::json!({"kind":"link","payload":{
                "source":{"type":"memory","memory_id":"a","kind":"actor"},
                "target":{"type":"article","run_id":"r","article_id":"x","revision":source_revision},
                "relation":"mentioned_in","receipts":["atlas_article_mentions:r/x:fixture"]
            }}).to_string())).await.unwrap().check().unwrap();
    }
    db.query("UPDATE memory:`confirmed-sibling` SET source_json=$source,payload_json=$payload;")
        .bind(("source",r#"{"app":"intel"}"#.to_string()))
        .bind(("payload",serde_json::json!({"kind":"link","payload":{
            "source":{"type":"memory","memory_id":"a","kind":"actor"},
            "target":{"type":"article","run_id":"r","article_id":"x","revision":source_revision},
            "relation":"supports","rationale":"confirmed historical fixture","receipts":["actual-prior-receipt"]
        }}).to_string())).await.unwrap().check().unwrap();
    let scope = lifecycle::ReconcileScope {
        keys: vec![lifecycle::ReconcileKey::Source {
            run_id: "r".into(),
            article_id: "x".into(),
        }],
    };
    let mut cursor = None;
    let mut captures = 0;
    let progress = loop {
        let output = in_tx(&db, |tx| {
            let (scope, cursor, revision) =
                (scope.clone(), cursor.clone(), source_revision.clone());
            async move {
                let capture = lifecycle::capture_affected_in(&tx, &scope, cursor.as_ref()).await?;
                if !capture.ready {
                    return Ok((Some(capture.cursor), None, capture.examined));
                }
                let batch = lifecycle::SupportBatch::for_capture(&tx, &capture)?;
                lifecycle::refresh_article_occurrence_in_batch(&tx, "r", "x", &revision, &batch)
                    .await?;
                lifecycle::record_mention_in_batch(
                    &tx,
                    "a",
                    "article",
                    "r",
                    "x",
                    "revision-0000",
                    "[]",
                    "new-publication",
                    &batch,
                )
                .await?;
                // Re-publication deliberately keeps the same canonical
                // material. Only the operational batch stamp distinguishes
                // this newer owner from captured obsolete ingest inventory.
                lifecycle::mark_ingest_memory_in(&tx, "ingest-republished", &batch).await?;
                let progress = lifecycle::reconcile_captured_in(&tx, &scope, &capture).await?;
                let examined = progress.examined;
                Ok((None, Some(progress), examined))
            }
        })
        .await
        .unwrap();
        captures += 1;
        assert!(output.2 <= 256);
        if let Some(progress) = output.1 {
            break progress;
        }
        cursor = output.0;
        assert!(captures < 32, "bounded capture must progress");
    };
    assert!(
        captures > 1,
        "270 old mentions must defer mutation across capture pages"
    );
    drain_affected(&db, &scope, progress.next).await;
    let mut result=db.query("SELECT VALUE origin_revision FROM brain_actor_mention WHERE origin_scope='r' AND origin_id='x';").await.unwrap();
    assert_eq!(
        result.take::<Vec<String>>(0).unwrap(),
        vec!["revision-0000"],
        "new same-ID publication survives the old inventory's later cleanup"
    );
    assert_eq!(lc(&db, "a").await, "active");
    assert_eq!(lc(&db, "ingest-old").await, "retired");
    assert_eq!(
        lc(&db, "ingest-republished").await,
        "active",
        "same-material newer memory publication must survive old inventory cleanup"
    );
    assert_eq!(
        lc(&db, "confirmed-sibling").await,
        "active",
        "non-ingest confirmed historical relationships are not blanket-retired by refresh"
    );
}

#[tokio::test]
async fn bounded_reverse_redirect_family_preserves_a_pinned_member_and_catalog_actor() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "life_scoped_family").await;
    actor(&db, "root", "atlas:r", false).await;
    actor(&db, "catalog", "catalog:fixture", false).await;
    let mut fixture = String::new();
    for index in 0..270 {
        fixture.push_str(&format!("CREATE type::record('memory','member-{index:04}') SET text='member',memory_kind='actor',lifecycle='redirected',pinned={};
            CREATE type::record('brain_actor_redirect','member-{index:04}') SET to_actor='root',created_at='fixed';",index==269));
    }
    sql(&db, &fixture).await;
    let scope = lifecycle::ReconcileScope {
        keys: vec![
            lifecycle::ReconcileKey::Actor {
                actor_id: "root".into(),
            },
            lifecycle::ReconcileKey::Actor {
                actor_id: "catalog".into(),
            },
        ],
    };
    let (steps, changed) = drain_affected(&db, &scope, None).await;
    assert!(
        steps > 1,
        "reverse redirects are discovered in bounded persisted pages"
    );
    assert!(changed.is_empty());
    assert_eq!(
        lc(&db, "root").await,
        "active",
        "the pinned member is beyond the first reverse page"
    );
    assert_eq!(lc(&db, "catalog").await, "active");
    sql(
        &db,
        "UPDATE type::record('memory','member-0269') SET pinned=false;",
    )
    .await;
    let (_, changed) = drain_affected(&db, &scope, None).await;
    assert_eq!(changed, vec!["root"]);
    assert_eq!(lc(&db, "root").await, "retired");
    assert_eq!(
        lc(&db, "member-0269").await,
        "redirected",
        "members preserve their redirect lifecycle"
    );
    assert_eq!(
        lc(&db, "catalog").await,
        "active",
        "catalog retention is independent of source support"
    );
}

#[tokio::test]
async fn concurrent_mention_only_writer_cannot_commit_behind_a_successful_capture_fence() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "life_concurrent_fence").await;
    actor(&db, "a", "atlas:r", false).await;
    let opened = std::sync::Arc::new(tokio::sync::Notify::new());
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    let task = {
        let (db, opened, release) = (db.clone(), opened.clone(), release.clone());
        tokio::spawn(async move {
            run_tx(
                &db,
                RetryPolicy {
                    max_attempts: 1,
                    ..RetryPolicy::default()
                },
                |tx| {
                    let (opened, release) = (opened.clone(), release.clone());
                    async move {
                        let scope = lifecycle::ReconcileScope {
                            keys: vec![lifecycle::ReconcileKey::Actor {
                                actor_id: "a".into(),
                            }],
                        };
                        let captured = lifecycle::capture_affected_in(&tx, &scope, None).await?;
                        opened.notify_one();
                        release.notified().await;
                        Ok(captured)
                    }
                },
            )
            .await
        })
    };
    opened.notified().await;
    let writer = run_tx(
        &db,
        RetryPolicy {
            max_attempts: 1,
            ..RetryPolicy::default()
        },
        |tx| async move {
            lifecycle::record_mention_in(&tx, "a", "article", "r", "x", "new", "[]", "fixed").await
        },
    )
    .await;
    release.notify_one();
    let capture = task.await.unwrap();
    assert!(capture.is_err() || writer.is_err(),"a capture and concurrent support mutation must conflict on the same actual support epoch; both committed");
    // Whichever transaction lost can retry without preserving an obsolete
    // transaction capability or suppressing the later writer's epoch bump.
    if writer.is_err() {
        in_tx(&db, |tx| async move {
            lifecycle::record_mention_in(&tx, "a", "article", "r", "x", "new", "[]", "fixed").await
        })
        .await
        .unwrap();
    }
    assert_eq!(support_epoch(&db).await, 1);
    assert_eq!(lc(&db, "a").await, "active");
}

#[tokio::test]
async fn publisher_counter_and_index_work_do_not_conflict_with_its_own_lifecycle_continuation() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "life_publisher_frontier").await;
    let repo = server.repo("argos", "life_publisher_frontier").await;
    sql(
        &db,
        "UPSERT memory:gone SET text='gone',memory_kind='claim',lifecycle='retired';
        UPSERT memory:kept SET text='kept',memory_kind='claim',lifecycle='active';",
    )
    .await;
    for (id, state) in [("gone", "retired"), ("kept", "active")] {
        db.query("UPSERT type::record('brain_graph_node',$node) SET kind='memory',ref_id=$id,memory_kind='claim',lifecycle=$state;")
            .bind(("node",graph::memory_node(id))).bind(("id",id.to_string()))
            .bind(("state",state.to_string())).await.unwrap().check().unwrap();
    }
    for index in 0..40 {
        link(
            &db,
            &format!("owned-{index:03}"),
            &graph::memory_node("gone"),
            &graph::memory_node("kept"),
        )
        .await;
    }
    let scope = lifecycle::ReconcileScope {
        keys: vec![lifecycle::ReconcileKey::Relationship {
            memory_id: "gone".into(),
        }],
    };
    let mut changed = std::collections::BTreeSet::new();
    let mut completed = false;
    let mut calls = 0;
    for step in 0..256 {
        let progress = lifecycle::reconcile_affected(&repo, &scope, "publisher-owned-operation")
            .await
            .unwrap();
        calls = step + 1;
        assert!(progress.examined <= 256);
        changed.extend(progress.changed_memory_ids);
        if progress.next.is_none() {
            completed = true;
            break;
        }
    }
    assert!(completed,"own publisher counter increments must not cause a false support conflict or endless restart");
    assert!(
        calls > 1,
        "real publication crosses a continuation boundary"
    );
    assert_eq!(changed.len(), 40);
    assert!(counters::current(&db, "memories_changed").await.unwrap() > 0);
    let mut outbox=db.query("SELECT VALUE record_id FROM index_change WHERE record_kind='memory' ORDER BY record_id;").await.unwrap();
    assert_eq!(
        outbox.take::<Vec<String>>(0).unwrap(),
        (0..40)
            .map(|index| format!("owned-{index:03}"))
            .collect::<Vec<_>>(),
        "exact lifecycle changes and index work commit through the actual publisher"
    );
    for index in 0..40 {
        assert_eq!(lc(&db, &format!("owned-{index:03}")).await, "retired");
    }
    repo.publisher().shutdown().await;
}

/// Canonical publications use array source keys while older sources use string
/// keys. A durable capture must preserve their key types across every page.
#[tokio::test]
async fn scoped_source_capture_pages_string_and_array_ids_and_rehomes_in_place() {
    use std::collections::BTreeSet;
    use surrealdb::types::{Array, RecordId, SurrealValue};

    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "life_source_key_types").await;
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
    let ids: Vec<_> = (0..140)
        .map(|index| {
            if index < 70 {
                RecordId::new("insight_source", format!("legacy-{index:03}"))
            } else {
                RecordId::new(
                    "insight_source",
                    Array::from(vec![format!("canonical-{index:03}"), "escaped:|é".into()]),
                )
            }
        })
        .collect();
    db.query("FOR $source IN $sources { CREATE $source SET fingerprint=type::string($source),run_id='r1',answer_id='snapshot-fixed',call_id='article:art',source_url='',published_at='' RETURN NONE; };")
        .bind(("sources", ids.clone())).await.unwrap().check().unwrap();
    let rekey_ids = vec![
        RecordId::new("insight_source", "legacy-rekey"),
        RecordId::new(
            "insight_source",
            Array::from(vec!["canonical-rekey", "escaped:|é"]),
        ),
    ];
    let mut rekey_targets = Vec::new();
    for (index, source) in rekey_ids.iter().enumerate() {
        let fingerprint = format!("rekey-{index}");
        rekey_targets.push(RecordId::new(
            "insight_source",
            codec::encode(&[&fingerprint, "atlas-r2", "article:art"]),
        ));
        db.query("CREATE $source SET fingerprint=$fingerprint,run_id='r1',answer_id='atlas-r1',call_id='article:art',source_url=$url,published_at=$published;")
            .bind(("source", source.clone()))
            .bind(("fingerprint", fingerprint))
            .bind(("url", format!("https://fixture.invalid/rekey-{index}")))
            .bind(("published", format!("published-{index}")))
            .await.unwrap().check().unwrap();
    }
    let other = RecordId::new("insight_source", Array::from(vec!["other", "article"]));
    let recon = RecordId::new("insight_source", Array::from(vec!["recon", "thread"]));
    db.query("CREATE $other SET fingerprint='other',run_id='r1',answer_id='snapshot-fixed',call_id='article:other',source_url='',published_at='';
        CREATE $recon SET fingerprint='recon',thread_id='thread',run_id='r1',answer_id='snapshot-fixed',call_id='article:art',source_url='',published_at='';")
        .bind(("other",other.clone())).bind(("recon",recon.clone())).await.unwrap().check().unwrap();
    let scope = lifecycle::ReconcileScope {
        keys: vec![lifecycle::ReconcileKey::Source {
            run_id: "r1".into(),
            article_id: "art".into(),
        }],
    };
    let first = in_tx(&db, |tx| {
        let scope = scope.clone();
        async move { lifecycle::capture_affected_in(&tx, &scope, None).await }
    })
    .await
    .unwrap();
    assert!(
        !first.ready,
        "140 source identities require capture continuation"
    );
    assert!(first.examined <= 256);
    let (steps, _) = drain_affected(&db, &scope, Some(first.cursor)).await;
    assert!(steps > 1);

    #[derive(SurrealValue)]
    struct SourceState {
        id: RecordId,
        run_id: String,
        answer_id: String,
    }
    let mut result = db
        .query("SELECT id,run_id,answer_id FROM $sources;")
        .bind(("sources", ids.clone()))
        .await
        .unwrap()
        .check()
        .unwrap();
    let rows: Vec<SourceState> = result.take(0).unwrap();
    assert_eq!(
        rows.len(),
        ids.len(),
        "no source may be skipped or re-keyed"
    );
    assert!(rows
        .iter()
        .all(|row| row.run_id == "r2" && row.answer_id == "snapshot-fixed"));
    assert_eq!(
        rows.into_iter().map(|row| row.id).collect::<BTreeSet<_>>(),
        ids.into_iter().collect::<BTreeSet<_>>(),
        "run-only rehome preserves exact string and array record IDs"
    );
    let mut old_sources = db
        .query("SELECT VALUE id FROM $sources;")
        .bind(("sources", rekey_ids))
        .await
        .unwrap()
        .check()
        .unwrap();
    assert!(
        old_sources.take::<Vec<RecordId>>(0).unwrap().is_empty(),
        "answer-changing rehome removes the exact original string and array IDs"
    );
    #[derive(SurrealValue)]
    struct RekeyedSource {
        id: RecordId,
        fingerprint: String,
        run_id: String,
        answer_id: String,
        call_id: String,
        source_url: String,
        published_at: String,
    }
    let mut rekeyed = db
        .query(
            "SELECT id,fingerprint,run_id,answer_id,call_id,source_url,published_at FROM $sources;",
        )
        .bind(("sources", rekey_targets.clone()))
        .await
        .unwrap()
        .check()
        .unwrap();
    let rekeyed: Vec<RekeyedSource> = rekeyed.take(0).unwrap();
    assert_eq!(rekeyed.len(), 2, "both no-winner targets are created");
    assert_eq!(
        rekeyed
            .iter()
            .map(|row| row.id.clone())
            .collect::<BTreeSet<_>>(),
        rekey_targets.into_iter().collect::<BTreeSet<_>>(),
        "answer-changing rehome creates the exact codec target IDs"
    );
    for row in rekeyed {
        let index = row.fingerprint.strip_prefix("rekey-").unwrap();
        assert_eq!(row.run_id, "r2");
        assert_eq!(row.answer_id, "atlas-r2");
        assert_eq!(row.call_id, "article:art");
        assert_eq!(
            row.source_url,
            format!("https://fixture.invalid/rekey-{index}")
        );
        assert_eq!(row.published_at, format!("published-{index}"));
    }
    let mut excluded = db
        .query("SELECT VALUE run_id FROM $sources;")
        .bind(("sources", vec![other, recon]))
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(excluded.take::<Vec<String>>(0).unwrap(), vec!["r1", "r1"]);
}
