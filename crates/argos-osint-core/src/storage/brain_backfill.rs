//! The Brain backfill pool on SurrealDB (`brain_backfill` on the
//! repository).
//!
//! the legacy store's backfill migrates legacy tables (`recon_entities`,
//! `investigation_entities`, `recon_thread_entities`, insight claims,
//! derivation links, Intel leads) into the Brain registry. A SurrealDB store
//! starts empty and binds those at write time (claim subjects at insight
//! publication, `storage::actor_bind`), so the one source with work here is
//! `atlas_articles`: occurrences without a current, checksum-valid
//! `atlas_article_index` receipt (a run collected but not yet indexed, or
//! indexed under an older revision). They are indexed through the phase-6
//! path, which writes the same receipt, so the two never index one
//! occurrence twice.

use anyhow::Result;
use std::sync::Arc;

use super::lifecycle;
use super::memory::{write_then_index, MemoryWrite};
use super::repo::Repository;
use super::tx::Tx;
use crate::atlas_actor_index::surreal::{index_occurrence, receipts, run_units};
use crate::brain_backfill::{BackfillReport, SourceCoverage};

const SOURCE: &str = "atlas_articles";

/// Index up to `max_units` pending Atlas occurrences, oldest run first, in
/// batches of [`crate::brain_backfill::BATCH`], each followed by a lifecycle
/// reconcile. A failing unit is reported and retried on the next call.
pub async fn backfill(repo: &Repository, max_units: usize) -> Result<BackfillReport> {
    let mut report = BackfillReport::default();
    let mut runs = super::atlas::list_runs(repo.db())
        .await
        .map_err(|e| anyhow::anyhow!("backfill runs: {e}"))?;
    runs.sort_by(|a, b| a.id.cmp(&b.id));
    let (mut done, mut total, mut budget) = (0usize, 0usize, max_units);
    for run in runs {
        let units = run_units(repo, &run.id).await?;
        let have = receipts(repo, &run.id).await?;
        total += units.len();
        let pending: Vec<_> = units
            .into_iter()
            .filter(|(a, _, rev)| {
                let current = have.get(&a.id) == Some(rev);
                done += usize::from(current);
                !current
            })
            .collect();
        for chunk in pending.chunks(crate::brain_backfill::BATCH) {
            if budget == 0 {
                break;
            }
            let take = chunk.len().min(budget);
            budget -= take;
            let now = chrono::Utc::now().to_rfc3339();
            for (article, metadata, rev) in &chunk[..take] {
                match index_occurrence(repo, article, metadata.as_ref(), rev, &now).await {
                    Ok(_) => {
                        report.processed += 1;
                        done += 1;
                    }
                    Err(e) => {
                        report.failed += 1;
                        report
                            .errors
                            .push(format!("{SOURCE}/{}/{}: {e}", run.id, article.id));
                    }
                }
            }
            let write: MemoryWrite = Arc::new(|tx: Tx| {
                Box::pin(async move { lifecycle::reconcile_lifecycle_in(&tx).await })
            });
            write_then_index(repo.publisher(), write)
                .await
                .map_err(|e| anyhow::anyhow!("backfill reconcile: {e}"))?;
        }
    }
    report.coverage = vec![SourceCoverage {
        source: SOURCE.into(),
        done,
        total,
    }];
    report.complete = report.failed == 0 && done >= total;
    Ok(report)
}
