//! Real Jobs consumer: durable index progress owns completion, not HTTP return.

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use argos_osint_core::osint::manual::DurableToolOutcome;
use argos_osint_core::storage::actor_search_outbox;
use argos_osint_core::storage::jobs::ClaimedTask;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::setup_jobs;

pub(super) async fn execute(
    repo: &Repository,
    claimed: &ClaimedTask,
    cancel: &AtomicBool,
) -> anyhow::Result<DurableToolOutcome> {
    actor_search_outbox::request_rebuild(repo, claimed).await?;
    loop {
        if cancel.load(Ordering::Acquire) {
            return Ok(DurableToolOutcome::Failed {
                reason: "actor_reindex_interrupted",
            });
        }
        let progress = actor_search_outbox::observe_rebuild(repo, &claimed.job_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("durable actor reindex request missing"))?;
        if progress.phase == "completed" {
            setup_jobs::record_result(repo,claimed,&serde_json::json!({
                "operation":"actor_reindex","original_job_id":progress.original_job_id,
                "generation":progress.replacement_generation,"visible_checkpoint":progress.barrier,
                "scanned":progress.scanned,"reader_visible":true,
            })).await?;
            return Ok(DurableToolOutcome::Complete {
                result_ref: format!("actor-index:{}", progress.replacement_generation),
            });
        }
        if !progress.reason_code.is_empty() {
            return Ok(DurableToolOutcome::Failed {
                reason: "actor_reindex_correction_required",
            });
        }
        let phase = if progress.reason_code.is_empty() {
            format!("actor index {}", progress.phase)
        } else {
            "actor index paused for correction".into()
        };
        setup_jobs::record_progress_unknown_total(repo, claimed, &phase, progress.scanned as u64)
            .await?;
        // The outer durable delivery owns queue/job renewals and cancellation.
        // No source transaction or independent lease is held during this wait.
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}
