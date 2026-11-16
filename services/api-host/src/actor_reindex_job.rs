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
    let mut reported = None;
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
            let result = fleetworks_contracts::readiness::ActorReindexResult::new(
                (!progress.original_job_id.is_empty()).then(|| progress.original_job_id.clone()),
                (!progress.replacement_generation.is_empty())
                    .then(|| progress.replacement_generation.clone()),
                u64::try_from(progress.barrier).ok(),
                u64::try_from(progress.scanned).unwrap_or(0),
                true,
            );
            setup_jobs::record_result(repo, claimed, &serde_json::to_value(result)?).await?;
            return Ok(DurableToolOutcome::Complete {
                result_ref: format!("actor-index:{}", progress.replacement_generation),
            });
        }
        if !progress.reason_code.is_empty() {
            return Ok(DurableToolOutcome::Failed {
                reason: "actor_reindex_correction_required",
            });
        }
        let current = (
            format!("actor index {}", progress.phase),
            u64::try_from(progress.scanned).unwrap_or(0),
        );
        if reported.as_ref() != Some(&current) {
            setup_jobs::record_progress_unknown_total(repo, claimed, &current.0, current.1).await?;
            reported = Some(current);
        }
        // The outer durable delivery owns queue/job renewals and cancellation.
        // No source transaction or independent lease is held during this wait.
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}
