//! Synchronous `Store` Jobs and Atlas-metadata entry points on SurrealDB.
//!
//! When the process selected SurrealDB ([`brain_store::on`]), the `Store`
//! methods in `crate::jobs_view` and `crate::atlas_metadata` dispatch here
//! instead of touching the SQLite connection. Every function blocks on
//! [`runtime::blocking`], so callers must be plain threads, never async code.
//! Reads and writes reuse `storage::jobs_view`, `storage::jobs`,
//! `storage::event_log` and `storage::atlas_metadata`.
//!
//! [`brain_store::on`]: super::brain_store::on

use anyhow::{anyhow, Result};
use chrono::Utc;

use super::runtime;
use super::tx::TxError;
use crate::atlas_metadata::ArticleMetadata;
use crate::events::{EventFilter, EventRow, NewEvent};
use crate::job_registry::CancelRequest;
use crate::jobs_view::EventCounts;
use crate::jobs_view::{JobCounts, JobDetail, JobFilter, JobRow};

fn tx_err(e: TxError) -> anyhow::Error {
    match e {
        TxError::Abort(m) => anyhow!(m),
        e => e.into(),
    }
}

// ---- Jobs dashboard ----

/// `Store::list_jobs`.
pub fn list_jobs(filter: &JobFilter, limit: usize) -> Result<Vec<JobRow>> {
    let filter = filter.clone();
    runtime::blocking(
        |repo| async move { super::jobs_view::list_jobs(repo.db(), &filter, limit).await },
    )
}

/// `Store::job_counts`.
pub fn job_counts() -> Result<JobCounts> {
    runtime::blocking(|repo| async move { super::jobs_view::job_counts(repo.db()).await })
}

/// `Store::job_detail`.
pub fn job_detail(id: &str, task_limit: usize) -> Result<Option<JobDetail>> {
    let id = id.to_string();
    runtime::blocking(|repo| async move {
        super::jobs_view::job_detail(repo.db(), &id, task_limit).await
    })
}

/// `Store::get_job`.
pub fn get_job(id: &str) -> Result<Option<JobRow>> {
    let id = id.to_string();
    runtime::blocking(|repo| async move { super::jobs_view::get_job(repo.db(), &id).await })
}

/// `Store::retry_failed_tasks`.
pub fn retry_failed_tasks(id: &str) -> Result<usize> {
    let id = id.to_string();
    runtime::blocking(
        |repo| async move { super::jobs_view::retry_failed_tasks(repo.db(), &id).await },
    )
}

/// `Store::job_apps`.
pub fn job_apps() -> Result<Vec<String>> {
    runtime::blocking(|repo| async move { super::jobs_view::job_apps(repo.db()).await })
}

/// `Store::request_job_cancel`: task jobs are fenced, task-less jobs get a
/// cooperative cancel request (`storage::jobs_view::cancel_job`).
pub fn request_job_cancel(id: &str) -> Result<CancelRequest> {
    let id = id.to_string();
    runtime::blocking(|repo| async move { super::jobs_view::cancel_job(repo.db(), &id).await })
}

/// `Store::set_job_stage_coverage`.
pub fn set_job_stage_coverage(job_id: &str, coverage_json: &str) -> Result<()> {
    let (id, json) = (job_id.to_string(), coverage_json.to_string());
    runtime::blocking(|repo| async move {
        repo.db()
            .query("UPDATE $j SET stage_coverage_json = $c RETURN NONE")
            .bind(("j", super::jobs::rid("job", &id)))
            .bind(("c", json))
            .await?
            .check()?;
        Ok(())
    })
}

/// `Store::register_job`: create `job:id` once, then merge dashboard meta.
/// Idempotent on `job.id`, like `tasks::enqueue_job_with`.
pub fn register_job(job: &crate::tasks::NewJob, meta: &crate::tasks::JobMeta) -> Result<()> {
    let (job, meta) = (job.clone(), meta.clone());
    runtime::blocking(|repo| async move {
        let db = repo.db();
        let mut r = db
            .query("RETURN record::exists($j)")
            .bind(("j", super::jobs::rid("job", &job.id)))
            .await?
            .check()?;
        let exists: Option<bool> = r.take(0usize)?;
        if exists != Some(true) {
            let operation = if meta.operation.is_empty() {
                job.kind.as_str()
            } else {
                meta.operation.as_str()
            };
            let new = super::jobs::NewJob {
                id: &job.id,
                kind: &job.kind,
                operation,
                parent: (!meta.parent_id.is_empty()).then_some(meta.parent_id.as_str()),
                created_at: Utc::now(),
            };
            super::jobs::create_job(db, &new, &[])
                .await
                .map_err(tx_err)?;
        }
        let some = |s: &str| (!s.is_empty()).then(|| s.to_string());
        let view = super::jobs_view::JobMeta {
            app: some(&meta.app),
            title: some(&meta.title),
            run_ref: some(&meta.run_ref),
            resource_ref: some(&meta.resource_ref),
            correlation_id: some(&meta.correlation_id),
            ..Default::default()
        };
        super::jobs_view::set_job_meta(db, &job.id, &view).await
    })
}

/// `Store::set_job_progress` for a task-less job.
pub fn set_job_progress(
    id: &str,
    state: &str,
    phase: &str,
    done: i64,
    total: Option<i64>,
    error_summary: &str,
) -> Result<()> {
    let (id, state, phase, error) = (
        id.to_string(),
        state.to_string(),
        phase.to_string(),
        error_summary.to_string(),
    );
    runtime::blocking(|repo| async move {
        let terminal = matches!(
            state.as_str(),
            "completed" | "partial" | "succeeded" | "failed" | "cancelled"
        );
        repo.db()
            .query(
                "UPDATE $j SET state = $state, phase = $phase, progress_done = $done, \
                 progress_total = $total, error_summary = $error, updated_at = time::now(), \
                 finished_at = IF $terminal { finished_at ?? time::now() } ELSE { finished_at } \
                 RETURN NONE",
            )
            .bind(("j", super::jobs::rid("job", &id)))
            .bind(("state", state))
            .bind(("phase", phase))
            .bind(("done", done))
            .bind(("total", total))
            .bind(("error", error))
            .bind(("terminal", terminal))
            .await?
            .check()?;
        Ok(())
    })
}

// ---- Event log ----

/// `Store::record_event`.
pub fn record_event(event: &NewEvent) -> Result<String> {
    let event = event.clone();
    runtime::blocking(|repo| async move { super::event_log::record_event(repo.db(), &event).await })
}

/// `Store::list_events`.
pub fn list_events(filter: &EventFilter, limit: usize) -> Result<Vec<EventRow>> {
    let filter = filter.clone();
    runtime::blocking(|repo| async move {
        super::event_log::list_events(repo.db(), &filter, limit).await
    })
}

/// `Store::get_event`.
pub fn get_event(id: &str) -> Result<Option<EventRow>> {
    let id = id.to_string();
    runtime::blocking(|repo| async move { super::event_log::get_event(repo.db(), &id).await })
}

/// `Store::event_counts`.
pub fn event_counts() -> Result<EventCounts> {
    runtime::blocking(|repo| async move { super::event_log::event_counts(repo.db()).await })
}

/// `Store::event_apps`.
pub fn event_apps() -> Result<Vec<String>> {
    runtime::blocking(|repo| async move { super::event_log::event_apps(repo.db()).await })
}

/// `Store::prune_events`.
pub fn prune_events(hours: i64) -> Result<usize> {
    runtime::blocking(|repo| async move { super::event_log::prune_events(repo.db(), hours).await })
}

/// `Store::clear_events`.
pub fn clear_events() -> Result<usize> {
    runtime::blocking(|repo| async move { super::event_log::clear_events(repo.db()).await })
}

// ---- Atlas article metadata ----

/// `Store::atlas_metadata`.
pub fn atlas_metadata(run_id: &str, article_id: &str) -> Result<Option<ArticleMetadata>> {
    let (run, article) = (run_id.to_string(), article_id.to_string());
    runtime::blocking(|repo| async move {
        super::atlas_metadata::metadata(repo.db(), &run, &article)
            .await
            .map_err(tx_err)
    })
}

/// `Store::atlas_save_metadata`.
pub fn atlas_save_metadata(metadata: &ArticleMetadata) -> Result<bool> {
    let metadata = metadata.clone();
    runtime::blocking(|repo| async move {
        super::atlas_metadata::save_metadata(repo.db(), &metadata)
            .await
            .map_err(tx_err)
    })
}

/// `Store::atlas_set_classification`.
pub fn atlas_set_classification(
    run_id: &str,
    article_id: &str,
    captured_hash: &str,
    status: &str,
    receipt_json: &str,
    category: Option<&str>,
) -> Result<bool> {
    let (run, article, hash, status, receipt) = (
        run_id.to_string(),
        article_id.to_string(),
        captured_hash.to_string(),
        status.to_string(),
        receipt_json.to_string(),
    );
    let category = category.map(str::to_string);
    runtime::blocking(|repo| async move {
        super::atlas_metadata::set_classification(
            repo.db(),
            &run,
            &article,
            &hash,
            &status,
            &receipt,
            category.as_deref(),
        )
        .await
        .map_err(tx_err)
    })
}
