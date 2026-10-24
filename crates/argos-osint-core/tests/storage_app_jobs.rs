//! Contract for `storage::recon_jobs::begin_app_job` / `set_job_progress`
//! (the SurrealDB job rows Atlas cycles and their phases write) against a
//! real SurrealDB 3.3.4 disk server or the embedded RocksDB engine (see
//! `support`). Not yet listed in `scripts/test_surreal.py`; run with
//! `cargo test --features surreal --test storage_app_jobs`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::storage::bootstrap;
use argos_osint_core::storage::recon_jobs::{begin_app_job, set_job_progress, AppJob};
use argos_osint_core::storage::repo::Repository;
use support::*;

fn spec<'a>(id: &'a str, parent: Option<&'a str>) -> AppJob<'a> {
    AppJob {
        id,
        operation: "atlas.cycle",
        app: "atlas",
        title: "Atlas cycle",
        run_ref: "run-1",
        resource_ref: "",
        parent,
    }
}

async fn row(repo: &Repository, id: &str) -> serde_json::Value {
    let mut r = repo
        .db()
        .query(
            "SELECT state, attempts_used, phase, progress_done, progress_total, app,
               operation, run_ref, cancel_requested,
               IF parent != NONE { record::id(parent) } ELSE { NONE } AS parent,
               record::id(root) AS root
             FROM type::record('job', $id)",
        )
        .bind(("id", id.to_string()))
        .await
        .unwrap()
        .check()
        .unwrap();
    let rows: Vec<surrealdb::types::Value> = r.take(0usize).unwrap();
    rows.into_iter()
        .next()
        .map(|v| v.into_json_value())
        .unwrap_or(serde_json::Value::Null)
}

async fn fresh(server: &Server, name: &str) -> Repository {
    let repo = server.repo("argos", name).await;
    bootstrap::ensure_schema(repo.db()).await.unwrap();
    repo
}

#[tokio::test]
async fn begin_app_job_creates_then_restarts_with_one_more_attempt() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = fresh(&server, "app_jobs_begin").await;
    begin_app_job(&repo, &spec("atlas-1", None)).await.unwrap();
    let first = row(&repo, "atlas-1").await;
    assert_eq!(first["state"], "running");
    assert_eq!(first["attempts_used"], 1);
    assert_eq!(first["app"], "atlas");
    assert_eq!(first["operation"], "atlas.cycle");
    assert_eq!(first["run_ref"], "run-1");
    assert_eq!(first["root"], "atlas-1");
    assert!(first["parent"].is_null());

    repo.db()
        .query(
            "UPDATE type::record('job', 'atlas-1') SET state = 'failed', cancel_requested = true",
        )
        .await
        .unwrap()
        .check()
        .unwrap();
    begin_app_job(&repo, &spec("atlas-1", None)).await.unwrap();
    let again = row(&repo, "atlas-1").await;
    assert_eq!(again["state"], "running");
    assert_eq!(again["attempts_used"], 2);
    assert_eq!(again["cancel_requested"], false);
}

#[tokio::test]
async fn begin_app_job_links_a_phase_child_and_rejects_an_empty_id() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = fresh(&server, "app_jobs_child").await;
    begin_app_job(&repo, &spec("atlas-2", None)).await.unwrap();
    begin_app_job(&repo, &spec("atlas-2-extract", Some("atlas-2")))
        .await
        .unwrap();
    let child = row(&repo, "atlas-2-extract").await;
    assert_eq!(child["parent"], "atlas-2");
    assert_eq!(child["root"], "atlas-2");
    assert!(begin_app_job(&repo, &spec("", None)).await.is_err());
}

#[tokio::test]
async fn set_job_progress_records_counts_and_reports_cancellation() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = fresh(&server, "app_jobs_progress").await;
    begin_app_job(&repo, &spec("atlas-3", None)).await.unwrap();
    let cancel = set_job_progress(&repo, "atlas-3", "extract", Some(3), Some(10))
        .await
        .unwrap();
    assert!(!cancel);
    let progressed = row(&repo, "atlas-3").await;
    assert_eq!(progressed["phase"], "extract");
    assert_eq!(progressed["progress_done"], 3);
    assert_eq!(progressed["progress_total"], 10);

    repo.db()
        .query("UPDATE type::record('job', 'atlas-3') SET cancel_requested = true")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert!(set_job_progress(&repo, "atlas-3", "publish", None, None)
        .await
        .unwrap());
    let cleared = row(&repo, "atlas-3").await;
    assert_eq!(cleared["phase"], "publish");
    assert!(cleared["progress_done"].is_null());

    // An empty id is a no-op, never a cancellation.
    assert!(!set_job_progress(&repo, "", "x", None, None).await.unwrap());
}
