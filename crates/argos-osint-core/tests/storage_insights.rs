//! Insight publication tests (migration phase 3) against a real SurrealDB
//! 3.3.4 disk server or the embedded RocksDB engine (see `support`).
//! Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::storage::insight::{self, commit_article_insight_replacement};
use argos_osint_core::storage::publisher::Publisher;
use argos_osint_core::storage::tx::TxError;
use argos_osint_core::storage::{memory, outbox};
use argos_osint_core::store::AtlasInsightClaim;
use support::*;
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

async fn fresh(server: &Server, name: &str) -> (Surreal<Any>, Publisher) {
    let db = server.root("argos", name).await;
    outbox::apply_schema(&db).await.unwrap();
    memory::apply_schema(&db).await.unwrap();
    insight::apply_schema(&db).await.unwrap();
    (
        db.clone(),
        Publisher::spawn(server.root("argos", name).await),
    )
}

async fn scalar<T: SurrealValue>(db: &Surreal<Any>, sql: &str) -> T {
    let mut r = db.query(sql).await.unwrap().check().unwrap();
    r.take::<Option<T>>(0usize).unwrap().expect("NONE")
}

fn claim(object: &str, article: &str) -> AtlasInsightClaim {
    AtlasInsightClaim {
        fingerprint: String::new(),
        entity: "Acme".into(),
        namespace: "org".into(),
        predicate: "located_in".into(),
        object: object.into(),
        topic: "geo".into(),
        claim: format!("Acme is in {object}."),
        classification: "fact".into(),
        confidence: 0.7,
        article_id: article.into(),
        source_url: "https://example.test/a".into(),
        published_at: "2026-10-01".into(),
        reliability: "B".into(),
        info_credibility: 2,
        admiralty: "B2".into(),
        rsp_status: "ok".into(),
    }
}

async fn counts(db: &Surreal<Any>) -> (i64, i64, i64, i64) {
    (
        scalar(db, "RETURN count(SELECT id FROM insight_claim)").await,
        scalar(db, "RETURN count(SELECT id FROM insight_source)").await,
        scalar(db, "RETURN count(SELECT id FROM memory)").await,
        scalar(db, "RETURN count(SELECT id FROM insight_relation)").await,
    )
}

#[tokio::test]
async fn replacement_swaps_claims_and_queues_index_work_atomically() {
    let Some(server) = Server::start() else {
        return;
    };
    let (db, p) = fresh(&server, "insight_replace").await;
    let a = vec![claim("Paris", "art1"), claim("Lyon", "art1")];
    let out = commit_article_insight_replacement(&p, "run1", "art1", a, vec![])
        .await
        .unwrap();
    assert_eq!((out.claims, out.created.len()), (2, 2));
    let fps = out.created.clone();
    let rel = vec![(fps[0].clone(), fps[1].clone(), "contradicts".into())];
    // Same claims again plus a relation. As in SQLite, the article's old
    // sources go first, so its claims are orphaned and re-created.
    let out = commit_article_insight_replacement(
        &p,
        "run1",
        "art1",
        vec![claim("Paris", "art1"), claim("Lyon", "art1")],
        rel,
    )
    .await
    .unwrap();
    assert_eq!((out.claims, out.created.len()), (2, 2));
    assert_eq!(counts(&db).await, (2, 2, 2, 1));
    let pending: i64 = scalar(&db, "RETURN count(SELECT id FROM index_work)").await;
    assert!(pending >= 2, "index work for created memories is queued");

    // Replace with one claim: Lyon is orphaned, its memory and relation go.
    let out = commit_article_insight_replacement(
        &p,
        "run1",
        "art1",
        vec![claim("Paris", "art1")],
        vec![],
    )
    .await
    .unwrap();
    assert_eq!(out.claims, 1);
    assert_eq!(counts(&db).await, (1, 1, 1, 0));
}

#[tokio::test]
async fn a_user_edited_orphan_is_preserved() {
    let Some(server) = Server::start() else {
        return;
    };
    let (db, p) = fresh(&server, "insight_user_edit").await;
    commit_article_insight_replacement(&p, "r", "art", vec![claim("Oslo", "art")], vec![])
        .await
        .unwrap();
    db.query("CREATE type::record('insight_user_edit', (SELECT VALUE memory_id FROM ONLY insight_claim LIMIT 1))")
        .await
        .unwrap()
        .check()
        .unwrap();
    let out = commit_article_insight_replacement(&p, "r", "art", vec![], vec![])
        .await
        .unwrap();
    assert_eq!(out.preserved_user_edits, 1);
    assert_eq!(counts(&db).await, (1, 0, 1, 0));
}

#[tokio::test]
async fn an_invalid_staged_claim_aborts_the_whole_replacement() {
    let Some(server) = Server::start() else {
        return;
    };
    let (db, p) = fresh(&server, "insight_invalid").await;
    commit_article_insight_replacement(&p, "r", "art", vec![claim("Rome", "art")], vec![])
        .await
        .unwrap();
    let before = counts(&db).await;
    let bad = vec![claim("Milan", "art"), claim("Turin", "other-article")];
    let err = commit_article_insight_replacement(&p, "r", "art", bad, vec![])
        .await
        .unwrap_err();
    assert!(matches!(err, TxError::Abort(_)), "{err:?}");
    assert_eq!(
        counts(&db).await,
        before,
        "nothing from the aborted batch is written"
    );
}
