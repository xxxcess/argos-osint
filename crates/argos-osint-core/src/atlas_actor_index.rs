//! Atlas phase 6, "Index actors and articles" (contract `atlas-collection-v2`).
//!
//! Every retained article occurrence is bound through Brain (actors, mention
//! receipts, `mentioned_in` Links, ≤ [`crate::brain_actors::RELATED_PER_ARTICLE`]
//! Related records) and receives an `atlas_article_index` receipt in the same
//! transaction. Transactions hold at most [`BATCH`] occurrences; a failed unit
//! rolls back alone and leaves no receipt, so the run cannot report phase 6
//! complete and a resume redoes only the missing occurrences.
use anyhow::Result;
use rusqlite::params;

use crate::brain_actors::IDENTITY_INDEX_VERSION;
use crate::store::Store;

pub const BATCH: usize = 100;
pub const INDEX_CONTRACT: &str = "atlas-actor-index-v1";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexReport {
    pub total: u32,
    /// Receipts already current before this pass (resume / rerun).
    pub reused: u32,
    pub indexed: u32,
    /// Indexed occurrences with no bindable actor (still receipted).
    pub zero_actor: u32,
    pub failed: u32,
    pub errors: Vec<String>,
    /// Stopped early by pause/cancel; nothing after the last batch ran.
    pub stopped: bool,
}

impl IndexReport {
    pub fn complete(&self) -> bool {
        !self.stopped && self.failed == 0 && self.reused + self.indexed == self.total
    }
}

fn checksum(revision: &str, coverage: &str) -> String {
    crate::nlp::cache::hash(&format!(
        "{INDEX_CONTRACT}|{IDENTITY_INDEX_VERSION}|{revision}|{coverage}"
    ))
}

/// Current, checksum-valid receipts for a run, keyed by article id.
pub fn receipts(store: &Store, run_id: &str) -> Result<std::collections::HashMap<String, String>> {
    let mut stmt = store.conn.prepare(
        "SELECT article_id,input_revision,index_checksum,actor_coverage FROM atlas_article_index WHERE run_id=?1",
    )?;
    let rows = stmt
        .query_map([run_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter(|(_, rev, sum, cov)| *sum == checksum(rev, cov))
        .map(|(id, rev, _, _)| (id, rev))
        .collect())
}

/// Index every occurrence of `run_id` lacking a current receipt. `stop` is
/// polled between batches (pause/cancel).
pub fn index_run(store: &Store, run_id: &str, stop: &dyn Fn() -> bool) -> Result<IndexReport> {
    let units = crate::brain_backfill::atlas_run_units(&store.conn, run_id)?;
    let have = receipts(store, run_id)?;
    let mut report = IndexReport {
        total: units.len() as u32,
        ..IndexReport::default()
    };
    let pending: Vec<_> = units
        .into_iter()
        .filter(|(unit, rev)| {
            let article = unit.split_once('/').map(|(_, a)| a).unwrap_or(unit);
            let current = have.get(article) == Some(rev);
            if current {
                report.reused += 1;
            }
            !current
        })
        .collect();
    for chunk in pending.chunks(BATCH) {
        if stop() {
            report.stopped = true;
            break;
        }
        let now = chrono::Utc::now().to_rfc3339();
        store.write_then_index(|| {
            for (unit, rev) in chunk {
                let article = unit.split_once('/').map(|(_, a)| a).unwrap_or(unit);
                store.conn.execute_batch("SAVEPOINT occurrence")?;
                let result = crate::brain_backfill::index_article(store, unit, &now).and_then(
                    |coverage| {
                        store.conn.execute(
                            "INSERT INTO atlas_article_index(run_id,article_id,input_revision,index_checksum,actor_coverage,indexed_at)
                             VALUES(?1,?2,?3,?4,?5,?6)
                             ON CONFLICT(run_id,article_id) DO UPDATE SET input_revision=excluded.input_revision,
                               index_checksum=excluded.index_checksum, actor_coverage=excluded.actor_coverage, indexed_at=excluded.indexed_at",
                            params![run_id, article, rev, checksum(rev, &coverage), coverage, now],
                        )?;
                        // The same revision satisfies the historical backfill.
                        store.conn.execute(
                            "INSERT INTO brain_backfill_receipts(source,unit_id,revision,coverage,completed_at) VALUES('atlas_articles',?1,?2,?3,?4)
                             ON CONFLICT(source,unit_id) DO UPDATE SET revision=excluded.revision, coverage=excluded.coverage, completed_at=excluded.completed_at",
                            params![unit, rev, coverage, now],
                        )?;
                        Ok(coverage)
                    },
                );
                match result {
                    Ok(coverage) => {
                        store.conn.execute_batch("RELEASE occurrence")?;
                        report.indexed += 1;
                        if coverage == "zero actors" || coverage == "metadata missing" {
                            report.zero_actor += 1;
                        }
                    }
                    Err(err) => {
                        store
                            .conn
                            .execute_batch("ROLLBACK TO occurrence; RELEASE occurrence")?;
                        report.failed += 1;
                        report.errors.push(format!("{unit}: {err}"));
                    }
                }
            }
            crate::brain_actors::reconcile_lifecycle(&store.conn, &now)?;
            crate::store::bump_memories_changed(&store.conn)?;
            Ok(((), vec![]))
        })?;
    }
    Ok(report)
}

#[cfg(test)]
#[path = "atlas_actor_index_tests.rs"]
pub(crate) mod tests;
