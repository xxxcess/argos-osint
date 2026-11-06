//! Bounded native Atlas backfill with independent coverage.
//!
//! Only authoritative current admissions can create new actor material. Legacy
//! and rejected history remains readable through its original records/evidence.
//! Each occurrence uses the same scoped actor/reconcile publisher as ingestion;
//! no redundant global lifecycle sweep follows a successful scoped unit.
use anyhow::Result;

use super::repo::Repository;
use crate::atlas_actor_index::surreal::{index_occurrence, receipts_many, run_units_page};
use crate::atlas_classify::OccurrenceKey;
use crate::brain_backfill::{BackfillReport, SourceCoverage};

const SOURCE: &str = "atlas_articles";

/// The write budget limits attempted new work. Coverage still audits all
/// eligible occurrences in bounded pages, preserving this explicit backfill's
/// independent coverage contract without retaining a whole run's payloads.
pub async fn backfill(repo: &Repository, max_units: usize) -> Result<BackfillReport> {
    let mut report = BackfillReport::default();
    let mut runs = super::atlas::list_runs(repo.db())
        .await
        .map_err(|_| anyhow::anyhow!("backfill run inventory unavailable"))?;
    runs.sort_by(|left, right| left.id.cmp(&right.id));
    let (mut done, mut total, mut budget) = (0usize, 0usize, max_units);
    for run in runs {
        let mut after = None;
        loop {
            let units = run_units_page(repo, &run.id, after.as_ref()).await?;
            if units.is_empty() {
                break;
            }
            after = units
                .last()
                .map(|(article, _, _)| OccurrenceKey::new(&article.run_id, &article.id));
            let keys: Vec<_> = units
                .iter()
                .map(|(article, _, _)| OccurrenceKey::new(&article.run_id, &article.id))
                .collect();
            let have = receipts_many(repo, &keys).await?;
            total = total
                .checked_add(units.len())
                .ok_or_else(|| anyhow::anyhow!("backfill coverage overflow"))?;
            for (article, metadata, revision) in units {
                let key = OccurrenceKey::new(&article.run_id, &article.id);
                if have.get(&key) == Some(&revision) {
                    done += 1;
                    continue;
                }
                if budget == 0 {
                    continue;
                }
                budget -= 1;
                match index_occurrence(
                    repo,
                    &article,
                    metadata.as_ref(),
                    &revision,
                    &chrono::Utc::now().to_rfc3339(),
                )
                .await
                {
                    Ok(_) => {
                        report.processed += 1;
                        done += 1;
                    }
                    Err(_) => {
                        report.failed += 1;
                        report.errors.push(format!(
                            "{SOURCE}/{}:native_backfill_unavailable",
                            crate::nlp::cache::hash(&key.record_id())
                        ));
                    }
                }
            }
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
