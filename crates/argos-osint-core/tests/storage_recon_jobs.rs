//! The Recon job writers on SurrealDB (`storage::recon_jobs`): model
//! operation `task_epoch` and attempts, coverage records, Intel report
//! jobs and tasks, and the Recon job registry rows, on the full bootstrap
//! schema plus `recon_jobs::SCHEMA`. Both modes via `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::recon::diversity::CoverageRecord;
use argos_osint_core::storage::bootstrap::ensure_schema;
use argos_osint_core::storage::recon_jobs as rj;
use argos_osint_core::storage::repo::Repository;
use support::*;

async fn setup(server: &Server, db: &str) -> Repository {
    let repo = server.repo("argos", db).await;
    ensure_schema(repo.db()).await.unwrap();
    rj::apply_schema(repo.db()).await.unwrap();
    repo
}

#[tokio::test]
async fn operations_keep_task_epoch_and_attempts_upsert_by_key() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = setup(&server, "jobs_ops").await;
    rj::ensure_operation(&repo, "op1", "run1", "task1", "brief", 1, "", Some(7))
        .await
        .unwrap();
    // A later ensure without an epoch keeps the stored one.
    rj::ensure_operation(&repo, "op1", "run1", "task1", "brief", 2, "d", None)
        .await
        .unwrap();
    assert_eq!(
        rj::operation_task_epoch(&repo, "op1").await.unwrap(),
        Some(7)
    );
    assert_eq!(
        rj::operations_for_task_epoch(&repo, "task1", 7)
            .await
            .unwrap(),
        vec!["op1".to_string()]
    );
    assert_eq!(
        rj::operation_task_epoch(&repo, "missing").await.unwrap(),
        None
    );

    let start = rj::AttemptStart {
        id: "a1",
        operation_id: "op1",
        generation: 2,
        route_index: 0,
        attempt: 1,
        provider: "p",
        account: "acct",
        model: "m",
        transport: "http",
        queued_at: "2026-10-23T00:00:00Z",
        declaration_json: "{}",
    };
    rj::record_attempt_start(&repo, &start).await.unwrap();
    rj::record_attempt_finish(
        &repo,
        "a1",
        &rj::AttemptFinish {
            outcome: "completed".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    // Replaying the start resets the same row to running.
    rj::record_attempt_start(&repo, &start).await.unwrap();
    let rows = rj::attempts(&repo, "op1").await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, "running");
}

#[tokio::test]
async fn coverage_records_round_trip_by_scope_and_generation() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = setup(&server, "jobs_cov").await;
    for (d, cat) in [(1, "b"), (0, "a"), (0, "b")] {
        let rec = CoverageRecord {
            scope: "s".into(),
            generation: 1,
            directive_index: d,
            category: cat.into(),
            ..Default::default()
        };
        rj::save_coverage_record(&repo, &rec).await.unwrap();
        rj::save_coverage_record(&repo, &rec).await.unwrap();
    }
    let got = rj::load_coverage_records(&repo, "s", 1).await.unwrap();
    let keys: Vec<_> = got
        .iter()
        .map(|r| (r.directive_index, r.category.as_str()))
        .collect();
    assert_eq!(keys, vec![(0, "a"), (0, "b"), (1, "b")]);
    assert!(rj::load_coverage_records(&repo, "s", 2)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn report_tasks_claim_in_dependency_order_once() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = setup(&server, "jobs_report").await;
    let job = rj::insert_report_job(
        &repo,
        &rj::NewReportJob {
            id: "j1",
            investigation_id: "inv",
            article_id: "art",
            mode: "brief",
            revision: rj::next_job_revision(&repo, "art", "brief").await.unwrap(),
            settings_json: "{}",
            budget_json: "{}",
            sections_total: 2,
            elements_total: 0,
            tool_calls_allowance: 5,
            parent_job_id: None,
        },
    )
    .await
    .unwrap();
    assert_eq!((job.state.as_str(), job.revision), ("queued", 1));
    assert_eq!(
        rj::next_job_revision(&repo, "art", "brief").await.unwrap(),
        2
    );
    rj::insert_report_task(&repo, "t1", "j1", "section", "a", "[]", "pending")
        .await
        .unwrap();
    rj::insert_report_task(&repo, "t2", "j1", "section", "b", "[\"t1\"]", "pending")
        .await
        .unwrap();

    let first = rj::claim_report_task(&repo, "j1", "w", 60).await.unwrap();
    assert_eq!(first.as_ref().map(|t| t.id.as_str()), Some("t1"));
    // t1 is leased and t2 waits on it.
    assert!(rj::claim_report_task(&repo, "j1", "w", 60)
        .await
        .unwrap()
        .is_none());
    rj::complete_report_task(&repo, "t1", "ref").await.unwrap();
    let second = rj::claim_report_task(&repo, "j1", "w", 60).await.unwrap();
    assert_eq!(second.map(|t| t.id), Some("t2".to_string()));

    rj::update_report_job(&repo, "j1", "completed", "done", 2, 0, "", "")
        .await
        .unwrap();
    let job = rj::report_job(&repo, "j1").await.unwrap().unwrap();
    assert_eq!(job.state, "completed");
    assert_eq!(
        rj::report_jobs_for_article(&repo, "art")
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn recon_job_rows_begin_resume_and_finish() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = setup(&server, "jobs_registry").await;
    let spec = rj::ReconJob {
        id: None,
        title: "who",
        run_ref: "run1",
        resource_ref: "thread1",
    };
    let id = rj::begin_recon_job(&repo, &spec).await.unwrap();
    assert!(!rj::set_recon_phase(&repo, &id, "tools").await.unwrap());
    rj::finish_recon_job(&repo, &id, "paused", None, "", "", "", 5)
        .await
        .unwrap();
    let found = rj::job_for_run(&repo, rj::RECON_OPERATION, "run1")
        .await
        .unwrap();
    assert_eq!(found.as_deref(), Some(id.as_str()));
    let again = rj::begin_recon_job(
        &repo,
        &rj::ReconJob {
            id: Some(&id),
            ..spec
        },
    )
    .await
    .unwrap();
    assert_eq!(again, id);
    rj::finish_recon_job(&repo, &id, "completed", Some("run1"), "", "", "ans", 9)
        .await
        .unwrap();
    let mut r = repo
        .db()
        .query("SELECT VALUE [state, attempts_used] FROM ONLY type::record('job', $k)")
        .bind(("k", id.clone()))
        .await
        .unwrap();
    let v: Option<serde_json::Value> = r.take(0usize).unwrap();
    assert_eq!(v, Some(serde_json::json!(["completed", 2])));
    let rows = rj::profile_rows(repo.db()).await.unwrap();
    assert!(rows.report_jobs.is_empty());
}
