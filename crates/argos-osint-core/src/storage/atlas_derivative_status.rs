//! Incremental current-generation readiness. Counts change with their fenced
//! publication/lease transitions, not with whole-run scans on each article.
use super::classifier_admission::{DerivativeClaim, DerivativeDisposition, DerivativeStage};
use super::jobs;
use super::tx::{Tx, TxError};
use crate::atlas_classify::OccurrenceKey;
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

pub const SCHEMA: &str = "
DEFINE TABLE IF NOT EXISTS atlas_derivative_progress SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS receipt_key ON atlas_derivative_progress TYPE string;
DEFINE FIELD IF NOT EXISTS eligible ON atlas_derivative_progress TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS processed ON atlas_derivative_progress TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS failed ON atlas_derivative_progress TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS zero_actor ON atlas_derivative_progress TYPE bool DEFAULT false;
DEFINE TABLE IF NOT EXISTS atlas_derivative_summary SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS actor_eligible ON atlas_derivative_summary TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS actor_processed ON atlas_derivative_summary TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS actor_failed ON atlas_derivative_summary TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS actor_zero ON atlas_derivative_summary TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS reconcile_eligible ON atlas_derivative_summary TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS reconcile_processed ON atlas_derivative_summary TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS reconcile_failed ON atlas_derivative_summary TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS reconcile_pages ON atlas_derivative_summary TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS stale ON atlas_derivative_summary TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS retries ON atlas_derivative_summary TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS last_error ON atlas_derivative_summary TYPE string DEFAULT '';
";
fn progress_key(occurrence: &OccurrenceKey, stage: DerivativeStage) -> String {
    super::codec::encode(&[&occurrence.record_id(), stage.as_str()])
}

/// Called in the same transaction as new admission/projection publication.
/// A superseded receipt removes its old contribution before new work is queued.
pub(crate) async fn invalidate_in(
    tx: &Tx,
    key: &OccurrenceKey,
    receipt: &str,
    admitted: bool,
) -> Result<(), TxError> {
    let mut current = tx
        .query("RETURN (SELECT VALUE receipt_key FROM ONLY $key)=$receipt;")
        .bind((
            "key",
            jobs::rid("atlas_classification_projection", &key.record_id()),
        ))
        .bind(("receipt", receipt.to_owned()))
        .await?
        .check()?;
    if current.take::<Option<bool>>(0)? != Some(true) {
        return Ok(());
    }
    for (stage, prefix) in [
        (DerivativeStage::Actors, "actor"),
        (DerivativeStage::Reconcile, "reconcile"),
    ] {
        let sql=format!("LET $old=(SELECT * FROM ONLY $progress);IF $old=NONE OR $old.receipt_key!=$receipt {{
            UPSERT $summary SET {prefix}_eligible=({prefix}_eligible ?? 0)+(IF $admitted {{1}} ELSE {{0}})-(IF ($old.eligible ?? false) {{1}} ELSE {{0}}),
            {prefix}_processed=({prefix}_processed ?? 0)-(IF ($old.processed ?? false) {{1}} ELSE {{0}}),
            {prefix}_failed=({prefix}_failed ?? 0)-(IF ($old.failed ?? false) {{1}} ELSE {{0}}),
            actor_zero=(actor_zero ?? 0)-(IF ($old.zero_actor ?? false) {{1}} ELSE {{0}}),
            stale=(stale ?? 0)+(IF $old!=NONE AND $actor {{1}} ELSE {{0}}) RETURN NONE;
            UPSERT $progress SET receipt_key=$receipt,eligible=$admitted,processed=false,failed=false,zero_actor=false RETURN NONE;
        }};");
        tx.query(sql)
            .bind((
                "progress",
                jobs::rid("atlas_derivative_progress", &progress_key(key, stage)),
            ))
            .bind((
                "summary",
                jobs::rid("atlas_derivative_summary", &key.run_id),
            ))
            .bind(("receipt", receipt.to_owned()))
            .bind(("admitted", admitted))
            .bind(("actor", stage == DerivativeStage::Actors))
            .await?
            .check()?;
    }
    Ok(())
}

/// A newly claimed current revision invalidates previous successful output and
/// dependent reconciliation in the same guarded transaction as the claim.
pub(crate) async fn reset_in(
    tx: &Tx,
    key: &OccurrenceKey,
    receipt: &str,
    stages: &[DerivativeStage],
) -> Result<(), TxError> {
    for stage in stages {
        let prefix = match stage {
            DerivativeStage::Actors => "actor",
            DerivativeStage::Reconcile => "reconcile",
            _ => continue,
        };
        let sql=format!("LET $old=(SELECT * FROM ONLY $progress); IF $old!=NONE AND $old.receipt_key=$receipt {{ UPDATE $summary SET {prefix}_processed=({prefix}_processed ?? 0)-(IF $old.processed {{1}} ELSE {{0}}),{prefix}_failed=({prefix}_failed ?? 0)-(IF $old.failed {{1}} ELSE {{0}}),actor_zero=(actor_zero ?? 0)-(IF ($old.zero_actor ?? false) {{1}} ELSE {{0}}),stale=(stale ?? 0)+(IF $actor AND $old.processed {{1}} ELSE {{0}}) RETURN NONE; UPDATE $progress SET processed=false,failed=false,zero_actor=false RETURN NONE; }};");
        tx.query(sql)
            .bind((
                "progress",
                jobs::rid("atlas_derivative_progress", &progress_key(key, *stage)),
            ))
            .bind((
                "summary",
                jobs::rid("atlas_derivative_summary", &key.run_id),
            ))
            .bind(("receipt", receipt.to_owned()))
            .bind(("actor", *stage == DerivativeStage::Actors))
            .await?
            .check()?;
    }
    Ok(())
}

/// Called only after source/lease validation, inside the output/checkpoint commit.
pub(crate) async fn publish_in(
    tx: &Tx,
    claim: &DerivativeClaim,
    disposition: &DerivativeDisposition,
) -> Result<(), TxError> {
    record_in(
        tx,
        &claim.work.occurrence,
        &claim.work.receipt_key,
        claim.stage,
        disposition,
        true,
    )
    .await
}

/// Reaffirms cached success only after an actual source/receipt/output audit in
/// this same transaction. It counts neither a publication nor a continuation.
pub(crate) async fn audit_current_in(
    tx: &Tx,
    key: &OccurrenceKey,
    receipt: &str,
    stage: DerivativeStage,
) -> Result<(), TxError> {
    record_in(
        tx,
        key,
        receipt,
        stage,
        &DerivativeDisposition::Current,
        false,
    )
    .await
}
async fn record_in(
    tx: &Tx,
    key: &OccurrenceKey,
    receipt: &str,
    stage: DerivativeStage,
    disposition: &DerivativeDisposition,
    count_page: bool,
) -> Result<(), TxError> {
    let prefix = match stage {
        DerivativeStage::Actors => "actor",
        DerivativeStage::Reconcile => "reconcile",
        _ => return Ok(()),
    };
    let processed = matches!(disposition, DerivativeDisposition::Current);
    let error = match disposition {
        DerivativeDisposition::Retryable { code } => code.as_str(),
        DerivativeDisposition::StaleInput => "stale_input",
        _ => "",
    };
    let stale = matches!(disposition, DerivativeDisposition::StaleInput)
        || matches!(disposition, DerivativeDisposition::Retryable {code}
            if matches!(code.as_str(), "actor_derivative_stale" | "actor_candidates_changed"));
    let failed =
        !stale && !error.is_empty() && !error.ends_with("continuation") && error != "cancelled";
    let continuation = error.ends_with("continuation");
    let sql=format!("LET $old=(SELECT * FROM ONLY $progress);
        IF $old!=NONE AND $old.receipt_key=$receipt AND $old.eligible {{
            LET $zero=$processed AND $actor AND ((SELECT VALUE actor_coverage FROM ONLY $index) ?? '')='zero actors';
            UPSERT $summary SET {prefix}_processed=({prefix}_processed ?? 0)+(IF $processed {{1}} ELSE {{0}})-(IF $old.processed {{1}} ELSE {{0}}),
              {prefix}_failed=({prefix}_failed ?? 0)+(IF $failed {{1}} ELSE {{0}})-(IF $old.failed {{1}} ELSE {{0}}),
              actor_zero=(actor_zero ?? 0)+(IF $zero {{1}} ELSE {{0}})-(IF ($old.zero_actor ?? false) {{1}} ELSE {{0}}),
              reconcile_pages=(reconcile_pages ?? 0)+(IF $count_page AND !$actor {{1}} ELSE {{0}}),retries=(retries ?? 0)+(IF $failed OR $stale {{1}} ELSE {{0}}),stale=(stale ?? 0)+(IF $stale {{1}} ELSE {{0}}),
              last_error=IF $failed OR $stale {{$error}} ELSE {{last_error ?? ''}} RETURN NONE;
            UPDATE $progress SET processed=$processed,failed=$failed,zero_actor=$zero RETURN NONE;
        }};");
    tx.query(sql)
        .bind((
            "progress",
            jobs::rid("atlas_derivative_progress", &progress_key(key, stage)),
        ))
        .bind((
            "summary",
            jobs::rid("atlas_derivative_summary", &key.run_id),
        ))
        .bind(("receipt", receipt.to_owned()))
        .bind(("count_page", count_page))
        .bind(("processed", processed))
        .bind(("failed", failed))
        .bind(("stale", stale))
        .bind(("error", error.to_owned()))
        .bind(("actor", stage == DerivativeStage::Actors))
        .bind(("index", jobs::rid("atlas_article_index", &key.record_id())))
        .await?
        .check()?;
    tracing::debug!(target:"argos::atlas_derivatives",stage=stage.as_str(),processed,failed,stale,continuation,"derivative_checkpoint");
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
pub struct Summary {
    pub actor_eligible: i64,
    pub actor_processed: i64,
    pub actor_failed: i64,
    pub actor_zero: i64,
    pub reconcile_eligible: i64,
    pub reconcile_processed: i64,
    pub reconcile_failed: i64,
    pub reconcile_pages: i64,
    pub stale: i64,
    pub retries: i64,
    pub last_error: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageReadiness {
    pub state: String,
    pub processed: u64,
    pub eligible: u64,
    pub failed: u64,
}
impl Summary {
    pub fn actor(&self) -> StageReadiness {
        stage(self.actor_processed, self.actor_eligible, self.actor_failed)
    }
    pub fn reconciliation(&self) -> StageReadiness {
        stage(
            self.reconcile_processed,
            self.reconcile_eligible,
            self.reconcile_failed,
        )
    }
}
fn stage(processed: i64, eligible: i64, failed: i64) -> StageReadiness {
    StageReadiness {
        state: if failed > 0 {
            "failed"
        } else if processed >= eligible {
            "ready"
        } else {
            "pending"
        }
        .into(),
        processed: processed.max(0) as u64,
        eligible: eligible.max(0) as u64,
        failed: failed.max(0) as u64,
    }
}

/// One batched point-read for the visible run cohort. Cached summaries are
/// optional for pre-upgrade runs, whose historical stats remain untouched.
async fn enrich_page(
    db: &surrealdb::Surreal<surrealdb::engine::any::Any>,
    runs: &mut [crate::store::AtlasRunRow],
) -> Result<(), TxError> {
    if runs.is_empty() {
        return Ok(());
    }
    let mut pending=db.query("RETURN count(SELECT id FROM atlas_derivative_invalidation WITH INDEX atlas_derivative_invalidation_pending WHERE state='pending' LIMIT 1)>0;").await?.check()?;
    let verification_pending = pending.take::<Option<bool>>(0)? == Some(true);
    let keys: Vec<_> = runs
        .iter()
        .map(|run| jobs::rid("atlas_derivative_summary", &run.id))
        .collect();
    #[derive(SurrealValue)]
    struct Row {
        run_id: String,
        actor_eligible: i64,
        actor_processed: i64,
        actor_failed: i64,
        actor_zero: i64,
        reconcile_eligible: i64,
        reconcile_processed: i64,
        reconcile_failed: i64,
        reconcile_pages: i64,
        stale: i64,
        retries: i64,
        last_error: String,
    }
    let mut response=db.query("SELECT record::id(id) AS run_id,actor_eligible,actor_processed,actor_failed,actor_zero,reconcile_eligible,reconcile_processed,reconcile_failed,reconcile_pages,stale,retries,last_error FROM $keys;").bind(("keys",keys)).await?.check()?;
    let rows: Vec<Row> = response.take(0)?;
    let summaries: std::collections::BTreeMap<_, _> = rows
        .into_iter()
        .map(|row| {
            (
                row.run_id,
                Summary {
                    actor_eligible: row.actor_eligible,
                    actor_processed: row.actor_processed,
                    actor_failed: row.actor_failed,
                    actor_zero: row.actor_zero,
                    reconcile_eligible: row.reconcile_eligible,
                    reconcile_processed: row.reconcile_processed,
                    reconcile_failed: row.reconcile_failed,
                    reconcile_pages: row.reconcile_pages,
                    stale: row.stale,
                    retries: row.retries,
                    last_error: row.last_error,
                },
            )
        })
        .collect();
    for run in runs {
        let Some(summary) = summaries.get(&run.id) else {
            continue;
        };
        let Ok(mut stats) = serde_json::from_str::<serde_json::Value>(&run.stats_json) else {
            continue;
        };
        if !stats.is_object()
            || stats
                .get("coverage")
                .is_some_and(|value| !value.is_object())
        {
            continue;
        }
        let coverage = &mut stats["coverage"];
        coverage["actor_indexed"] = summary.actor_processed.into();
        coverage["actor_zero"] = summary.actor_zero.into();
        coverage["actor_index_failed"] = summary.actor_failed.into();
        let (mut actor, mut reconciliation) = (summary.actor(), summary.reconciliation());
        if verification_pending {
            if actor.eligible > 0 {
                actor.state = "verification_pending".into();
            }
            if reconciliation.eligible > 0 {
                reconciliation.state = "verification_pending".into();
            }
        }
        coverage["actor_readiness"] = serde_json::to_value(actor)
            .map_err(|_| TxError::Abort("derivative_summary_json".into()))?;
        coverage["reconciliation_readiness"] = serde_json::to_value(reconciliation)
            .map_err(|_| TxError::Abort("derivative_summary_json".into()))?;
        coverage["derivative_progress"] = serde_json::to_value(summary)
            .map_err(|_| TxError::Abort("derivative_summary_json".into()))?;
        run.stats_json = serde_json::to_string(&stats)
            .map_err(|_| TxError::Abort("derivative_summary_json".into()))?;
    }
    Ok(())
}

pub(crate) async fn enrich_runs(
    db: &surrealdb::Surreal<surrealdb::engine::any::Any>,
    runs: &mut [crate::store::AtlasRunRow],
) -> Result<(), TxError> {
    for page in runs.chunks_mut(64) {
        enrich_page(db, page).await?;
    }
    Ok(())
}

#[cfg(all(test, feature = "embedded"))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn superseded_preparation_is_pending_stale_work_instead_of_a_failed_current_output() {
        let directory = tempfile::tempdir().unwrap();
        let repo = super::super::repo::Repository::connect(
            &super::super::open::Target::Embedded {
                path: directory.path().join("storage"),
            },
            "argos",
            "stale_readiness",
        )
        .await
        .unwrap();
        for (code, disposition) in [
            ("stale_input", DerivativeDisposition::StaleInput),
            (
                "actor_derivative_stale",
                DerivativeDisposition::Retryable {
                    code: "actor_derivative_stale".into(),
                },
            ),
            (
                "actor_candidates_changed",
                DerivativeDisposition::Retryable {
                    code: "actor_candidates_changed".into(),
                },
            ),
        ] {
            let key = OccurrenceKey::new(code, "stale-article");
            // Aggregation-only fixture; this creates no admission, native output,
            // successful receipt or publication authority.
            repo.db().query("CREATE $summary SET actor_eligible=1; CREATE $progress SET receipt_key='fixture',eligible=true,processed=false,failed=false,zero_actor=false;")
            .bind(("summary",jobs::rid("atlas_derivative_summary",&key.run_id))).bind(("progress",jobs::rid("atlas_derivative_progress",&progress_key(&key,DerivativeStage::Actors)))).await.unwrap().check().unwrap();
            let captured = key.clone();
            super::super::tx::run_tx(repo.db(), Default::default(), move |tx| {
                let key = captured.clone();
                let disposition = disposition.clone();
                async move {
                    record_in(
                        &tx,
                        &key,
                        "fixture",
                        DerivativeStage::Actors,
                        &disposition,
                        true,
                    )
                    .await
                }
            })
            .await
            .unwrap();
            let summary: Summary = repo
                .db()
                .query("SELECT * FROM ONLY $key;")
                .bind(("key", jobs::rid("atlas_derivative_summary", &key.run_id)))
                .await
                .unwrap()
                .check()
                .unwrap()
                .take::<Option<Summary>>(0)
                .unwrap()
                .unwrap();
            assert_eq!(summary.actor().state, "pending");
            assert_eq!(summary.actor_failed, 0);
            assert_eq!(summary.actor_processed, 0);
            assert_eq!(summary.stale, 1);
            assert_eq!(summary.retries, 1);
            assert_eq!(summary.last_error, code);
        }
        repo.publisher().shutdown().await;
    }
}
