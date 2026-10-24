//! Insight refresh execution and recovery on SurrealDB (migration phase
//! 4d). Both modes via `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use argos_osint_core::investigation::insights::{
    snapshot, EvidenceLocation, EvidenceRef, PipelineInput, PipelineOutput, RetainedEvidence,
};
use argos_osint_core::provider::SettingsFile;
use argos_osint_core::storage::derivations::DerivationContext;
use argos_osint_core::storage::insight::SnapshotStep;
use argos_osint_core::storage::refresh::schedule_article_refresh;
use argos_osint_core::storage::refresh_exec::*;
use support::*;
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

async fn scalar<T: SurrealValue>(db: &Surreal<Any>, sql: &str) -> T {
    let mut r = db.query(sql).await.unwrap().check().unwrap();
    r.take::<Option<T>>(0usize)
        .unwrap()
        .unwrap_or_else(|| panic!("NONE: {sql}"))
}

fn input(article: &str) -> PipelineInput {
    let evidence = vec![RetainedEvidence {
        reference: EvidenceRef {
            source_id: format!("body:{article}"),
            revision: "r1".into(),
            artifact_hash: "hash".into(),
            acquisition_id: "acq".into(),
            origin_root: String::new(),
            url: "https://example.test/a".into(),
            published_at: None,
            retrieved_at: "2026-10-01".into(),
            location: EvidenceLocation::JsonPointer(String::new()),
        },
        text: "Body.".into(),
        native: None,
        substantive: true,
        relevant: true,
        complete_story: true,
        known_truncation: false,
    }];
    PipelineInput {
        snapshot: snapshot("article", article, 0, &evidence),
        evidence,
        objective: "Extract attributed assertions.".into(),
        surface: "article".into(),
        required_tasks_complete: true,
        save_insights: false,
        recall_enabled: false,
    }
}

fn ok_validate() -> SnapshotStep {
    Arc::new(|_, _| Box::pin(async { Ok(()) }))
}

fn ctx() -> DerivationContext {
    DerivationContext {
        save_insights: false,
        recon_thread: None,
    }
}

fn eligible() -> Execute {
    Box::new(|input| {
        Box::pin(async move {
            let mut out = PipelineOutput::default();
            out.readiness.publication_eligible = true;
            Ok((input, out))
        })
    })
}

fn failing() -> Execute {
    Box::new(|_| Box::pin(async { Err(anyhow::anyhow!("provider exploded")) }))
}

async fn schedule(db: &Surreal<Any>, article: &str, s: &SettingsFile) -> String {
    schedule_article_refresh(db, article, Some(&input(article)), s, true)
        .await
        .unwrap()
        .expect("task")
}

async fn state(db: &Surreal<Any>, task: &str) -> String {
    let mut r = db
        .query("SELECT VALUE state FROM ONLY type::record('task', $t)")
        .bind(("t", task.to_string()))
        .await
        .unwrap()
        .check()
        .unwrap();
    r.take::<Option<String>>(0usize)
        .unwrap()
        .unwrap_or_default()
}

#[tokio::test]
async fn slice_publishes_fails_and_blocks_under_its_lease() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "refresh_exec").await;
    let db = repo.db().clone();
    let s = SettingsFile::default();
    let calm = Arc::new(AtomicBool::new(false));

    let t1 = schedule(&db, "a1", &s).await;
    let out = run_refresh_slice(&repo, &t1, &s, &calm, ok_validate(), ctx(), eligible())
        .await
        .unwrap();
    assert!(out.is_some());
    assert_eq!(state(&db, &t1).await, "completed");
    let life: String = scalar(
        &db,
        "RETURN (SELECT VALUE lifecycle FROM ONLY insight_owner_state:['article', 'a1'])",
    )
    .await;
    assert_eq!(life, "active");
    let active: String = scalar(
        &db,
        "RETURN (SELECT VALUE active_snapshot_id FROM ONLY insight_owner_state:['article', 'a1'])",
    )
    .await;
    assert!(active.starts_with("isnap:article:a1:1:"), "{active}");
    // Not claimable again.
    assert!(
        run_refresh_slice(&repo, &t1, &s, &calm, ok_validate(), ctx(), eligible())
            .await
            .unwrap()
            .is_none()
    );

    let t2 = schedule(&db, "a2", &s).await;
    let err = run_refresh_slice(&repo, &t2, &s, &calm, ok_validate(), ctx(), failing())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("provider exploded"));
    assert_eq!(state(&db, &t2).await, "failed");
    let row: (String, String) = {
        let mut r = db
            .query("RETURN (SELECT VALUE [lifecycle, last_error] FROM ONLY insight_owner_state:['article', 'a2'])")
            .await
            .unwrap()
            .check()
            .unwrap();
        let v: Vec<String> = r.take(0usize).unwrap();
        (v[0].clone(), v[1].clone())
    };
    assert_eq!(row, ("previous".into(), "provider exploded".into()));

    // Ineligible output fails with the readiness gaps.
    let t3 = schedule(&db, "a3", &s).await;
    let thin: Execute = Box::new(|input| {
        Box::pin(async move {
            let mut out = PipelineOutput::default();
            out.readiness.gaps = vec!["no claims".into()];
            Ok((input, out))
        })
    });
    let err = run_refresh_slice(&repo, &t3, &s, &calm, ok_validate(), ctx(), thin)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no claims"));
    assert_eq!(state(&db, &t3).await, "failed");

    // Cancellation pauses with evidence dirty and refunds the attempt.
    let t4 = schedule(&db, "a4", &s).await;
    let cancelled = Arc::new(AtomicBool::new(true));
    assert!(
        run_refresh_slice(&repo, &t4, &s, &cancelled, ok_validate(), ctx(), failing())
            .await
            .is_err()
    );
    assert_eq!(state(&db, &t4).await, "paused");
    let reason: String = scalar(
        &db,
        &format!("RETURN (SELECT VALUE blocked_reason FROM ONLY type::record('insight_refresh_task', '{t4}'))"),
    )
    .await;
    assert_eq!(reason, "cancelled; evidence dirty");

    // Live-evidence validation failure fails the refresh before execution.
    let t5 = schedule(&db, "a5", &s).await;
    let invalid: SnapshotStep = Arc::new(|_, _| {
        Box::pin(async {
            Err(argos_osint_core::storage::tx::TxError::Abort(
                "evidence changed".into(),
            ))
        })
    });
    let err = run_refresh_slice(&repo, &t5, &s, &calm, invalid, ctx(), eligible())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("evidence changed"), "{err}");
    assert_eq!(state(&db, &t5).await, "failed");
}

#[tokio::test]
async fn recovery_lists_queued_refreshes_and_skips_missing_articles() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "refresh_recover").await;
    let db = repo.db().clone();
    let s = SettingsFile::default();
    let a = schedule(&db, "a1", &s).await;
    let b = schedule(&db, "a2", &s).await;
    // The intents' articles are not in Atlas: rescheduling skips them.
    let (ids, errors) = recover_refreshes(&repo, &s).await.unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(ids, vec![a.clone(), b]);
    let captured = captured_settings(&repo, &a, &s).await.unwrap();
    assert_eq!(
        serde_json::to_string(&captured.defaults).unwrap(),
        serde_json::to_string(&s.defaults).unwrap()
    );
}

#[tokio::test]
async fn retry_extraction_requeues_a_failed_refresh_with_an_exhausted_stage() {
    use argos_osint_core::storage::insight_retry::retry_extraction;
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "refresh_retry_extraction").await;
    let db = repo.db().clone();
    let s = SettingsFile::default();
    let calm = Arc::new(AtomicBool::new(false));

    // Live owner and evidence, so admission passes.
    let text = "Body.";
    let hash = argos_osint_core::nlp::cache::hash(text);
    db.query(
        "CREATE atlas_article SET run_id = 'run', article_id = 'r1', title = 'T',
           description = '', url = 'https://example.test/a', country = '', source_name = '',
           source_domain = '', published_at = '', provider = '', temperature = 0.0,
           seen_at = '2026-10-01';
         CREATE article_body:r1 SET article_id = 'r1', run_id = 'run', original_url = '',
           resolved_url = '', source_domain = '', source_name = '', body_markdown = 'Body.',
           content_hash = '', quality = '', quality_rationale = '', body_version = 1,
           fetch_tool = '', fetched_at = '', retry_after = '', state = 'ready',
           created_at = '', updated_at = '';
         CREATE type::record('nlp_artifact', $h) SET text = $t, byte_len = 5, created_at = '';",
    )
    .bind(("h", hash.clone()))
    .bind(("t", text.to_string()))
    .await
    .unwrap()
    .check()
    .unwrap();
    let mut live = input("r1");
    live.evidence[0].reference.revision = "1".into();
    live.evidence[0].reference.artifact_hash = hash;
    live.snapshot = snapshot("article", "r1", 0, &live.evidence);
    let t = schedule_article_refresh(&db, "r1", Some(&live), &s, true)
        .await
        .unwrap()
        .expect("task");
    run_refresh_slice(&repo, &t, &s, &calm, ok_validate(), ctx(), failing())
        .await
        .unwrap_err();
    assert_eq!(state(&db, &t).await, "failed");
    // No exhausted stage yet: nothing to retry.
    assert!(!retry_extraction(&db, &t).await.unwrap());

    let generation: i64 = scalar(
        &db,
        "RETURN (SELECT VALUE desired_generation FROM ONLY insight_owner_state:['article', 'r1'])",
    )
    .await;
    db.query(
        "CREATE insight_stage_checkpoint SET owner_kind = 'article', owner_id = 'r1',
           generation = $g, stage = 'claims', window_id = '', state = 'exhausted',
           updated_at = '2026-10-24T00:00:00Z';
         CREATE insight_stage_checkpoint SET owner_kind = 'article', owner_id = 'r1',
           generation = $g, stage = 'body', window_id = '', state = 'completed',
           updated_at = '2026-10-24T00:00:00Z';",
    )
    .bind(("g", generation))
    .await
    .unwrap()
    .check()
    .unwrap();
    let attempts: i64 = scalar(
        &db,
        &format!("RETURN (SELECT VALUE attempts FROM ONLY task:`{t}`)"),
    )
    .await;

    assert!(retry_extraction(&db, &t).await.unwrap());
    assert_eq!(state(&db, &t).await, "queued");
    let max: i64 = scalar(
        &db,
        &format!("RETURN (SELECT VALUE max_attempts FROM ONLY task:`{t}`)"),
    )
    .await;
    assert_eq!(max, attempts + 1);
    let states: Vec<String> = {
        let mut r = db
            .query("SELECT VALUE state FROM insight_stage_checkpoint WHERE owner_id = 'r1' ORDER BY stage")
            .await
            .unwrap()
            .check()
            .unwrap();
        r.take(0usize).unwrap()
    };
    assert_eq!(
        states,
        vec!["completed".to_string(), "retry_admitted".to_string()]
    );
    let epoch: i64 = scalar(
        &db,
        &format!("RETURN (SELECT VALUE retry_epoch FROM ONLY insight_refresh_execution:`{t}`)"),
    )
    .await;
    assert_eq!(epoch, 1);
    let life: String = scalar(
        &db,
        "RETURN (SELECT VALUE lifecycle FROM ONLY insight_owner_state:['article', 'r1'])",
    )
    .await;
    assert_eq!(life, "pending");
    let events: i64 = scalar(
        &db,
        "RETURN count(SELECT id FROM job_event WHERE event_type = 'insight.retry')",
    )
    .await;
    assert_eq!(events, 1);
    // Queued now: a second retry is refused.
    assert!(!retry_extraction(&db, &t).await.unwrap());
}

#[tokio::test]
async fn intel_brief_parts_read_the_published_owner() {
    use argos_osint_core::storage::intel_brief_view::brief_parts;
    use argos_osint_core::store::AtlasArticleRow;
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "intel_brief_parts").await;
    let db = repo.db().clone();
    let s = SettingsFile::default();
    let calm = Arc::new(AtomicBool::new(false));
    let article = AtlasArticleRow {
        run_id: "run".into(),
        id: "b1".into(),
        ..Default::default()
    };
    let none = brief_parts(&repo, &article).await.unwrap();
    assert_eq!(none.lifecycle, "legacy/unassessed");
    assert_eq!(none.status, "Waiting · complete article details required");
    assert!(none.output.is_none() && none.claims.is_empty() && none.body.is_none());

    let t = schedule(&db, "b1", &s).await;
    let queued = brief_parts(&repo, &article).await.unwrap();
    assert_eq!(queued.status, "Queued");
    run_refresh_slice(&repo, &t, &s, &calm, ok_validate(), ctx(), eligible())
        .await
        .unwrap()
        .expect("published");
    let parts = brief_parts(&repo, &article).await.unwrap();
    assert_eq!(parts.lifecycle, "active");
    assert_eq!(parts.status, "Published");
    assert!(parts.output.is_some());
}
