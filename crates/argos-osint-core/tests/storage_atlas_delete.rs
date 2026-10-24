//! Atlas delete and retention tests (migration phase 3, step 3) against a
//! real SurrealDB 3.3.4 disk server or the embedded RocksDB engine (see
//! `support`). Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::storage::insight::{self, publish_atlas_insights};
use argos_osint_core::storage::publisher::Publisher;
use argos_osint_core::storage::{atlas, atlas_delete, intel, lifecycle, memory, outbox};
use argos_osint_core::store::{AtlasArticleRow, AtlasInsightClaim, PublishOptions};
use support::*;
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

async fn fresh(server: &Server, name: &str) -> (Surreal<Any>, Publisher) {
    let db = server.root("argos", name).await;
    outbox::apply_schema(&db).await.unwrap();
    memory::apply_schema(&db).await.unwrap();
    insight::apply_schema(&db).await.unwrap();
    atlas::apply_schema(&db).await.unwrap();
    intel::apply_schema(&db).await.unwrap();
    lifecycle::apply_schema(&db).await.unwrap();
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

async fn article(db: &Surreal<Any>, run: &str, id: &str, seen: &str) {
    atlas::upsert_article(
        db,
        &AtlasArticleRow {
            run_id: run.into(),
            id: id.into(),
            seen_at: seen.into(),
            category: "unk".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
}

async fn texts(db: &Surreal<Any>) -> Vec<String> {
    let mut r = db
        .query("SELECT VALUE text FROM memory ORDER BY text")
        .await
        .unwrap();
    r.take(0).unwrap()
}

/// r1 holds a1 and a2, r2 holds a1 again; r1 published Paris@a1 and Lyon@a2.
async fn two_runs(db: &Surreal<Any>, p: &Publisher) {
    atlas::insert_run(db, "r1", "", "").await.unwrap();
    atlas::insert_run(db, "r2", "", "").await.unwrap();
    article(db, "r1", "a1", "1").await;
    article(db, "r1", "a2", "1").await;
    article(db, "r2", "a1", "2").await;
    publish_atlas_insights(
        p,
        "r1",
        vec![claim("Paris", "a1"), claim("Lyon", "a2")],
        vec![],
        "Brief.",
        PublishOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(texts(db).await.len(), 3, "two claims and the brief");
}

#[tokio::test]
async fn deleting_a_run_rehomes_shared_articles_and_drops_the_rest() {
    let Some(server) = Server::start() else {
        return;
    };
    let (db, p) = fresh(&server, "del_run").await;
    two_runs(&db, &p).await;
    let before: i64 = scalar(&db, "RETURN counter:memories_changed.n ?? 0").await;
    assert!(atlas_delete::delete_run(&p, "r1").await.unwrap());
    assert_eq!(texts(&db).await, vec!["Acme is in Paris.".to_string()]);
    let homes: Vec<Vec<String>> = {
        let mut r = db
            .query("SELECT VALUE [run_id, answer_id] FROM insight_source")
            .await
            .unwrap();
        r.take(0).unwrap()
    };
    assert_eq!(homes, vec![vec!["r2".to_string(), "atlas-r2".to_string()]]);
    let left: i64 = scalar(
        &db,
        "RETURN count(SELECT id FROM atlas_article WHERE run_id = 'r1') + count(SELECT id FROM atlas_run:r1) + count(SELECT id FROM insight_claim)",
    )
    .await;
    assert_eq!(left, 1, "only Paris's claim remains");
    let removals: i64 = scalar(
        &db,
        "RETURN count(SELECT id FROM index_change WHERE operation = 'index_remove')",
    )
    .await;
    assert_eq!(removals, 2, "Lyon and the brief queue index removal");
    let after: i64 = scalar(&db, "RETURN counter:memories_changed.n ?? 0").await;
    assert!(after > before);
    assert!(
        !atlas_delete::delete_run(&p, "r1").await.unwrap(),
        "already gone"
    );
    let again: i64 = scalar(&db, "RETURN counter:memories_changed.n ?? 0").await;
    assert_eq!(
        again, after,
        "a no-op delete does not bump memories_changed"
    );
}

#[tokio::test]
async fn deleting_article_insights_drops_only_that_articles_claims() {
    let Some(server) = Server::start() else {
        return;
    };
    let (db, p) = fresh(&server, "del_art_ins").await;
    two_runs(&db, &p).await;
    assert_eq!(
        atlas_delete::delete_article_insights(&p, "r1", "a2")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        texts(&db).await,
        vec!["Acme is in Paris.".to_string(), "Brief.".to_string()]
    );
    assert_eq!(
        atlas_delete::delete_article_insights(&p, "r1", "a2")
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn deleting_an_article_marks_its_owner_previous() {
    let Some(server) = Server::start() else {
        return;
    };
    let (db, p) = fresh(&server, "del_art").await;
    article(&db, "r1", "a9", "1").await;
    db.query(
        "CREATE insight_owner_state:['article', 'a9'] SET owner_kind = 'article', owner_id = 'a9',
           desired_generation = 1, active_snapshot_id = 's9', lifecycle = 'active', updated_at = '';
         CREATE insight_derivation_link:l SET snapshot_id = 's9', left_kind = 'claim', left_id = 'x',
           right_kind = 'claim', right_id = 'y', relation = 'supports', rationale = '',
           assessment_json = '{}', evidence_json = '[]', status = 'active', revision = '1';",
    )
    .await
    .unwrap()
    .check()
    .unwrap();
    atlas_delete::delete_article(&p, "r1", "a9").await.unwrap();
    let state: Vec<String> = {
        let mut r = db
            .query("SELECT VALUE [lifecycle, last_error, <string>desired_generation] FROM ONLY insight_owner_state:['article', 'a9']; SELECT VALUE status FROM ONLY insight_derivation_link:l;")
            .await
            .unwrap();
        let mut v: Vec<String> = r.take::<Vec<String>>(0).unwrap();
        v.push(r.take::<Option<String>>(1).unwrap().unwrap());
        v
    };
    assert_eq!(
        state,
        vec!["previous", "origin article removed", "2", "origin_deleted"]
    );
}

#[tokio::test]
async fn pruning_keeps_investigated_articles_and_running_runs() {
    let Some(server) = Server::start() else {
        return;
    };
    let (db, p) = fresh(&server, "del_prune").await;
    for run in ["old", "live", "new"] {
        atlas::insert_run(&db, run, "", "").await.unwrap();
    }
    db.query(
        "UPDATE atlas_run:old SET state = 'done', started_at = '2026-01-01T00:00:00+00:00';
         UPDATE atlas_run:live SET state = 'running', started_at = '2026-01-01T00:00:00+00:00';
         UPDATE atlas_run:new SET state = 'done';",
    )
    .await
    .unwrap()
    .check()
    .unwrap();
    article(&db, "old", "kept", "1").await;
    article(&db, "old", "gone", "1").await;
    article(&db, "live", "x", "1").await;
    intel::ensure_investigation(&db, "kept", "old", "https://k", "{}")
        .await
        .unwrap();
    let pruned = atlas_delete::prune_expired(&p, chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!(pruned, vec!["old".to_string()]);
    let mut r = db
        .query("SELECT VALUE article_id FROM atlas_article ORDER BY article_id; RETURN record::exists(atlas_run:old);")
        .await
        .unwrap();
    assert_eq!(
        r.take::<Vec<String>>(0).unwrap(),
        vec!["kept".to_string(), "x".to_string()]
    );
    assert_eq!(
        r.take::<Option<bool>>(1).unwrap(),
        Some(true),
        "a run holding a protected article stays"
    );
}
