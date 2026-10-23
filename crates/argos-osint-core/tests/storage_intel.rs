//! Intel storage tests (migration phase 3) against a real SurrealDB 3.3.4
//! disk server or the embedded RocksDB engine (see `support`).
//! Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use std::collections::HashMap;

use argos_osint_core::storage::insight;
use argos_osint_core::storage::intel::{self, Sequence};
use support::*;
use surrealdb::engine::any::Any;
use surrealdb::Surreal;

async fn fresh(server: &Server, name: &str) -> Surreal<Any> {
    let db = server.root("argos", name).await;
    insight::apply_schema(&db).await.unwrap();
    intel::apply_schema(&db).await.unwrap();
    db
}

#[tokio::test]
async fn investigations_are_created_once_per_article() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "intel_inv").await;
    let mut hs = Vec::new();
    for _ in 0..6 {
        let db = db.clone();
        hs.push(tokio::spawn(async move {
            intel::ensure_investigation(&db, "a1", "r1", "https://x", "{}")
                .await
                .unwrap()
                .id
        }));
    }
    let mut ids = Vec::new();
    for h in hs {
        ids.push(h.await.unwrap());
    }
    ids.dedup();
    assert_eq!(ids.len(), 1, "racing creators converge: {ids:?}");
    let id = ids[0].clone();
    intel::set_investigation_thread(&db, &id, "t9")
        .await
        .unwrap();
    assert_eq!(
        intel::update_shared_assessment(&db, &id, "{\"x\":1}")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        intel::update_shared_assessment(&db, &id, "{\"x\":2}")
            .await
            .unwrap(),
        2
    );
    let row = intel::investigation_for_article(&db, "a1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.thread_id.as_deref(), Some("t9"));
    assert_eq!(row.shared_assessment_json, "{\"x\":2}");
    assert_eq!(
        intel::investigation_by_id(&db, &id).await.unwrap().unwrap(),
        row
    );
    assert!(intel::investigation_for_article(&db, "none")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        intel::investigated_articles(&db).await.unwrap(),
        vec!["a1".to_string()]
    );
}

#[tokio::test]
async fn link_explanations_prefer_the_active_snapshot() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "intel_links").await;
    let mut e = HashMap::new();
    e.insert(("l".to_string(), "r".to_string()), "saved".to_string());
    intel::save_link_explanations(&db, "a1", &e).await.unwrap();
    e.insert(("l".to_string(), "r".to_string()), "saved2".to_string());
    intel::save_link_explanations(&db, "a1", &e).await.unwrap();
    assert_eq!(intel::link_explanations(&db, "a1").await.unwrap(), e);
    db.query(
        "UPSERT insight_owner_state:['article', 'a1'] SET owner_kind = 'article', owner_id = 'a1', active_snapshot_id = 's1', lifecycle = 'active', updated_at = '';
         CREATE insight_derivation_link SET snapshot_id = 's1', left_kind = 'claim', left_id = 'x', right_kind = 'claim', right_id = 'y', relation = 'supports', rationale = 'why', assessment_json = '{}', evidence_json = '{}', status = 'active', revision = '1';",
    )
    .await
    .unwrap()
    .check()
    .unwrap();
    let live = intel::link_explanations(&db, "a1").await.unwrap();
    assert_eq!(
        live.get(&("x".to_string(), "y".to_string()))
            .map(String::as_str),
        Some("[supports · active] why")
    );
    intel::delete_link_explanations(&db, "a1").await.unwrap();
    assert!(intel::link_explanations(&db, "a2")
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn report_results_are_immutable_and_sequenced() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "intel_results").await;
    let rec = |job: &'static str, gen: i64, seq: Sequence| {
        let db = db.clone();
        async move {
            intel::record_report_result(
                &db,
                job,
                gen,
                "a",
                &format!("{{\"{job}\":{gen}}}"),
                "t",
                seq,
            )
            .await
            .unwrap()
        }
    };
    assert_eq!(rec("j1", 1, Sequence::Next).await, Some(1));
    assert_eq!(rec("j2", 1, Sequence::Next).await, Some(2));
    assert_eq!(
        rec("j2", 1, Sequence::Next).await,
        None,
        "once per (job, generation)"
    );
    assert_eq!(rec("old", 1, Sequence::Retained).await, Some(-1));
    assert!(intel::report_result_exists(&db, "j2", 1).await.unwrap());
    assert!(!intel::report_result_exists(&db, "j2", 2).await.unwrap());
    let latest = intel::latest_report_result(&db, "a")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (latest.job_id.as_str(), latest.completion_sequence),
        ("j2", 2)
    );
    assert_eq!(
        intel::job_report_details(&db, "j1")
            .await
            .unwrap()
            .as_deref(),
        Some("{\"j1\":1}")
    );
    let err = db
        .query("UPDATE intel_report_result SET details_json = 'x'")
        .await
        .unwrap()
        .check();
    assert!(err.is_err(), "completed results are immutable");
}

#[tokio::test]
async fn refresh_intents_and_retirements() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "intel_refresh").await;
    assert!(intel::add_refresh_intent(&db, "a").await.unwrap());
    assert!(!intel::add_refresh_intent(&db, "a").await.unwrap());
    assert!(intel::has_refresh_intent(&db, "a").await.unwrap());
    assert_eq!(
        intel::refresh_intents(&db).await.unwrap(),
        vec!["a".to_string()]
    );
    intel::clear_refresh_intent(&db, "a").await.unwrap();
    assert!(!intel::has_refresh_intent(&db, "a").await.unwrap());
    intel::retire_refresh_input(&db, "a", "k").await.unwrap();
    intel::retire_refresh_input(&db, "a", "k").await.unwrap();
    assert!(intel::refresh_input_retired(&db, "a", "k").await.unwrap());
    assert!(!intel::refresh_input_retired(&db, "a", "j").await.unwrap());
}

#[tokio::test]
async fn intel_lookups_use_their_indexes() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "intel_plans").await;
    for (sql, index) in [
        (
            intel::INVESTIGATION_BY_ARTICLE_SURQL,
            "intel_investigation_article",
        ),
        (intel::EXPLANATIONS_SURQL, "intel_link_explanation_article"),
        (intel::LATEST_RESULT_SURQL, "intel_report_result_sequence"),
    ] {
        let mut r = db
            .query(format!("{sql} EXPLAIN FULL"))
            .bind(("article", "a".to_string()))
            .await
            .unwrap()
            .check()
            .unwrap();
        let p = format!("{:?}", r.take::<surrealdb::types::Value>(0).unwrap());
        assert!(p.contains(index), "{sql}: {p}");
        assert!(!p.contains("Iterate Table"), "{sql}: {p}");
    }
}
