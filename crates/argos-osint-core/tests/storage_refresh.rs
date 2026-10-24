//! Article insight-refresh admission (migration phase 3) against a real
//! SurrealDB 3.3.4 disk server or the embedded RocksDB engine (see
//! `support`). Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::investigation::insights::{
    snapshot, EvidenceLocation, EvidenceRef, PipelineInput, RetainedEvidence,
};
use argos_osint_core::provider::SettingsFile;
use argos_osint_core::storage::{insight, intel, jobs, refresh};
use support::*;
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

async fn fresh(server: &Server, name: &str) -> Surreal<Any> {
    let db = server.root("argos", name).await;
    jobs::apply_schema(&db).await.unwrap();
    insight::apply_schema(&db).await.unwrap();
    intel::apply_schema(&db).await.unwrap();
    refresh::apply_schema(&db).await.unwrap();
    db
}

async fn scalar<T: SurrealValue>(db: &Surreal<Any>, sql: &str) -> T {
    let mut r = db.query(sql).await.unwrap().check().unwrap();
    r.take::<Option<T>>(0usize).unwrap().expect("NONE")
}

fn input(article: &str, body: &str, complete: bool) -> PipelineInput {
    let evidence = vec![RetainedEvidence {
        reference: EvidenceRef {
            source_id: format!("body:{article}"),
            revision: "r1".into(),
            artifact_hash: format!("hash-{body}"),
            acquisition_id: "acq".into(),
            origin_root: String::new(),
            url: "https://example.test/a".into(),
            published_at: None,
            retrieved_at: "2026-10-01".into(),
            location: EvidenceLocation::JsonPointer(String::new()),
        },
        text: body.into(),
        native: None,
        substantive: true,
        relevant: true,
        complete_story: complete,
        known_truncation: false,
    }];
    PipelineInput {
        snapshot: snapshot("article", article, 0, &evidence),
        evidence,
        objective: "Extract attributed assertions.".into(),
        surface: "article".into(),
        required_tasks_complete: true,
        save_insights: true,
        recall_enabled: false,
    }
}

async fn task_state(db: &Surreal<Any>, id: &str) -> String {
    let mut r = db
        .query("SELECT VALUE state FROM ONLY type::record('task', $t)")
        .bind(("t", id.to_string()))
        .await
        .unwrap()
        .check()
        .unwrap();
    r.take::<Option<String>>(0usize)
        .unwrap()
        .unwrap_or_default()
}

#[tokio::test]
async fn refresh_admission_dedupes_supersedes_and_respects_intent() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "refresh_admit").await;
    let s = SettingsFile::default();
    let v1 = input("a1", "Body one.", true);
    // No intent and not a visit: nothing.
    assert_eq!(
        refresh::schedule_article_refresh(&db, "a1", Some(&v1), &s, false)
            .await
            .unwrap(),
        None
    );
    // Ineligible input (incomplete story): the visit records the intent only.
    let partial = input("a1", "Body one.", false);
    assert_eq!(
        refresh::schedule_article_refresh(&db, "a1", Some(&partial), &s, true)
            .await
            .unwrap(),
        None
    );
    assert!(intel::has_refresh_intent(&db, "a1").await.unwrap());

    let t1 = refresh::schedule_article_refresh(&db, "a1", Some(&v1), &s, false)
        .await
        .unwrap()
        .expect("first task");
    assert!(t1.starts_with("insight-refresh:article:a1:1:"), "{t1}");
    assert_eq!(task_state(&db, &t1).await, "queued");
    let job: String = scalar(
        &db,
        "RETURN (SELECT VALUE refresh_job_id FROM ONLY insight_owner_state:['article', 'a1'])",
    )
    .await;
    assert_eq!(job, "insight-refresh:article:a1:1");

    // Same input: attach to the live task, no new generation.
    let again = refresh::schedule_article_refresh(&db, "a1", Some(&v1), &s, false)
        .await
        .unwrap();
    assert_eq!(again.as_deref(), Some(t1.as_str()));
    assert_eq!(
        scalar::<i64>(&db, "RETURN count(SELECT id FROM task)").await,
        1
    );

    // Changed source: generation 2 supersedes the queued task and its job.
    let v2 = input("a1", "Body two.", true);
    let t2 = refresh::schedule_article_refresh(&db, "a1", Some(&v2), &s, false)
        .await
        .unwrap()
        .expect("second task");
    assert!(t2.starts_with("insight-refresh:article:a1:2:"), "{t2}");
    assert_eq!(task_state(&db, &t1).await, "superseded");
    assert_eq!(task_state(&db, &t2).await, "queued");
    let old_job: String = scalar(
        &db,
        "RETURN (SELECT VALUE state FROM ONLY job:⟨insight-refresh:article:a1:1⟩)",
    )
    .await;
    assert_eq!(old_job, "cancelled");
    let (by, reason) = refresh::supersession(&db, &t1).await.unwrap().unwrap();
    assert_eq!(by, "insight-refresh:article:a1:2");
    assert_eq!(reason, "source changed");

    // A retired input key is never re-admitted.
    let v3 = input("a1", "Body three.", true);
    let mut keyed = v3.clone();
    keyed.snapshot.generation = 0;
    let identity = argos_osint_core::investigation::insights::input_key(&keyed, &s);
    intel::retire_refresh_input(&db, "a1", &identity)
        .await
        .unwrap();
    assert_eq!(
        refresh::schedule_article_refresh(&db, "a1", Some(&v3), &s, false)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn running_refresh_is_recorded_but_left_to_its_fence() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "refresh_running").await;
    let s = SettingsFile::default();
    intel::add_refresh_intent(&db, "a2").await.unwrap();
    let t1 = refresh::schedule_article_refresh(&db, "a2", Some(&input("a2", "x", true)), &s, false)
        .await
        .unwrap()
        .unwrap();
    db.query("UPDATE type::record('task', $t) SET state = 'running'")
        .bind(("t", t1.clone()))
        .await
        .unwrap()
        .check()
        .unwrap();
    let t2 = refresh::schedule_article_refresh(&db, "a2", Some(&input("a2", "y", true)), &s, false)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(t1, t2);
    assert_eq!(task_state(&db, &t1).await, "running");
    assert!(refresh::supersession(&db, &t1).await.unwrap().is_some());
}

/// `retire_atlas_insight_work` cancels only Atlas-scoped live refresh work,
/// fences its lease and is idempotent.
#[tokio::test]
async fn atlas_insight_work_is_retired_and_articles_are_kept() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "refresh_retire_atlas").await;
    let s = SettingsFile::default();
    intel::add_refresh_intent(&db, "a3").await.unwrap();
    intel::add_refresh_intent(&db, "a4").await.unwrap();
    let atlas =
        refresh::schedule_article_refresh(&db, "a3", Some(&input("a3", "x", true)), &s, false)
            .await
            .unwrap()
            .unwrap();
    let kept =
        refresh::schedule_article_refresh(&db, "a4", Some(&input("a4", "x", true)), &s, false)
            .await
            .unwrap()
            .unwrap();
    db.query("UPDATE type::record('insight_refresh_task', $t) SET owner_scope = 'atlas:a3'")
        .bind(("t", atlas.clone()))
        .await
        .unwrap()
        .check()
        .unwrap();
    let epoch = |t: String| {
        let db = db.clone();
        async move {
            let mut r = db
                .query("SELECT VALUE lease_epoch FROM ONLY type::record('task', $t)")
                .bind(("t", t))
                .await
                .unwrap()
                .check()
                .unwrap();
            r.take::<Option<i64>>(0usize).unwrap().unwrap_or(-1)
        }
    };
    let before = epoch(atlas.clone()).await;
    assert_eq!(refresh::retire_atlas_insight_work(&db).await.unwrap(), 1);
    assert_eq!(task_state(&db, &atlas).await, "cancelled");
    assert_eq!(epoch(atlas.clone()).await, before + 1);
    assert_eq!(task_state(&db, &kept).await, "queued");
    assert_eq!(refresh::retire_atlas_insight_work(&db).await.unwrap(), 0);
}
