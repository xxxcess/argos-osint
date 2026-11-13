//! Workstation-owned cycle history associations; shared jobs and evidence are retained.
use fleetworks_contracts::context::ActivityScope;
use fleetworks_contracts::cycle_history::CycleHistory;
use fleetworks_contracts::cycle_history::CycleSummary;
use fleetworks_contracts::cycle_history::DeleteCycleReceipt;
use fleetworks_contracts::cycle_history::DeleteCycleRequest;
use sha2::Digest;
use surrealdb::types::SurrealValue;

use super::job_context::InteractiveAdmission;
use super::jobs;
use super::jobs_view;
use super::repo::Repository;
use super::tx;
use super::tx::TxError;

pub const DELETE_OPERATION: &str = "atlas.cycle_history.delete";
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS workstation_cycle_tombstone SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS scope_id ON workstation_cycle_tombstone TYPE string;
DEFINE FIELD IF NOT EXISTS deployment_id ON workstation_cycle_tombstone TYPE string;
DEFINE FIELD IF NOT EXISTS workstation_id ON workstation_cycle_tombstone TYPE string;
DEFINE FIELD IF NOT EXISTS job_id ON workstation_cycle_tombstone TYPE string;
DEFINE FIELD IF NOT EXISTS hidden_at ON workstation_cycle_tombstone TYPE datetime;
DEFINE FIELD IF NOT EXISTS revision ON workstation_cycle_tombstone TYPE int;
DEFINE INDEX IF NOT EXISTS workstation_cycle_hidden ON workstation_cycle_tombstone FIELDS scope_id,deployment_id,workstation_id,job_id UNIQUE;
DEFINE TABLE IF NOT EXISTS workstation_cycle_delete SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS fingerprint ON workstation_cycle_delete TYPE string;
DEFINE FIELD IF NOT EXISTS receipt_json ON workstation_cycle_delete TYPE string;";

#[derive(SurrealValue)]
struct SummaryRow {
    job_id: String,
    state: String,
    created_at: String,
    finished_at: Option<String>,
}

/// Authorize membership before selecting payload and applying the bounded limit.
pub async fn list(
    repo: &Repository,
    scope_id: &str,
    scope: &ActivityScope,
    limit: usize,
) -> Result<CycleHistory, TxError> {
    if !(1..=100).contains(&limit) {
        return Err(TxError::Abort("invalid cycle history bound".into()));
    }
    jobs_view::validate_activity_scope(scope_id, scope)
        .map_err(|_| TxError::Abort("invalid cycle scope".into()))?;
    let query = format!("SELECT record::id(id) AS job_id,state,<string>created_at AS created_at,IF finished_at != NONE {{ <string>finished_at }} ELSE {{ NONE }} AS finished_at FROM job WHERE operation=$operation AND ({membership}) AND array::len((SELECT VALUE job_id FROM workstation_cycle_tombstone WHERE scope_id=$activity_scope AND deployment_id=$activity_deployment AND workstation_id=$activity_node AND job_id=record::id($parent.id) LIMIT 1))=0 ORDER BY created_at DESC LIMIT $limit",membership=jobs_view::activity_membership());
    let mut response = repo
        .db()
        .query(query)
        .bind(("operation", fleetworks_contracts::ATLAS_CYCLE_OPERATION))
        .bind(("activity_scope", scope_id.to_owned()))
        .bind(("activity_deployment", scope.deployment_id.clone()))
        .bind(("activity_node", scope.workstation_id.clone()))
        .bind(("limit", limit as i64))
        .await?
        .check()?;
    let rows: Vec<SummaryRow> = response.take(0)?;
    Ok(CycleHistory::new(
        rows.into_iter()
            .filter_map(|row| {
                let cycle = fleetworks_contracts::AtlasCycleReceipt::from_job_id(&row.job_id)?;
                Some(CycleSummary::new(
                    row.job_id,
                    cycle.run_id,
                    row.state,
                    row.created_at,
                    row.finished_at,
                ))
            })
            .collect(),
    ))
}

fn digest(parts: &[&str]) -> String {
    let mut digest = sha2::Sha256::new();
    for value in parts {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

#[derive(SurrealValue)]
struct SavedDelete {
    fingerprint: String,
    receipt_json: String,
}

/// A tombstone affects only this workstation's history association. It never cancels work.
pub async fn delete(
    repo: &Repository,
    admission: &InteractiveAdmission,
    request: &DeleteCycleRequest,
) -> Result<DeleteCycleReceipt, TxError> {
    if request.expected_revision != 1
        || fleetworks_contracts::AtlasCycleReceipt::from_job_id(&request.job_id).is_none()
    {
        return Err(TxError::Abort("cycle:invalid".into()));
    }
    let context = admission.context();
    let key = digest(&[
        &context.deployment_id,
        &context.registration_id,
        &request.request_id,
    ]);
    let association = digest(&[
        admission.authority().scope_id(),
        &context.deployment_id,
        &context.workstation_id,
        &request.job_id,
    ]);
    let fingerprint = digest(&[
        &serde_json::to_string(request).map_err(|_| TxError::Abort("cycle:invalid".into()))?
    ]);
    let result=tx::run_tx(repo.db(),tx::RetryPolicy::default(),|transaction| {
        let (key,association,fingerprint)=(key.clone(),association.clone(),fingerprint.clone());
        async move {
            // This public admission proof rechecks registration/session authority under the registry guard.
            admission.record_in(&transaction,&request.job_id,DELETE_OPERATION,false).await?;
            let mut response=jobs::run(&transaction,transaction.query("SELECT fingerprint,receipt_json FROM ONLY $record").bind(("record",jobs::rid("workstation_cycle_delete",&key)))).await?;
            if let Some(saved)=jobs::take::<Option<SavedDelete>>(&mut response,0)? {
                if saved.fingerprint!=fingerprint { return Err(TxError::Abort("cycle:request_conflict".into())); }
                return serde_json::from_str(&saved.receipt_json).map_err(|_|TxError::Abort("cycle:receipt_invalid".into()));
            }
            // The original cycle association must predate this deletion's own request association.
            let mut membership=jobs::run(&transaction,transaction.query("SELECT VALUE job_id FROM job_request WHERE job_id=$job AND operation=$operation AND scope_id=$scope AND deployment_id=$deployment AND workstation_id=$node LIMIT 1;")
                .bind(("job",request.job_id.clone())).bind(("operation",fleetworks_contracts::ATLAS_CYCLE_OPERATION)).bind(("scope",admission.authority().scope_id().to_owned())).bind(("deployment",context.deployment_id.clone())).bind(("node",context.workstation_id.clone()))).await?;
            if jobs::take::<Vec<String>>(&mut membership,0)?.is_empty() { return Err(TxError::Abort("cycle:unavailable".into())); }
            let mut prior=jobs::run(&transaction,transaction.query("RETURN record::exists($record)").bind(("record",jobs::rid("workstation_cycle_tombstone",&association)))).await?;
            if jobs::take::<Option<bool>>(&mut prior,0)?==Some(true) { return Err(TxError::Abort("cycle:revision_changed".into())); }
            let receipt=DeleteCycleReceipt::hidden(request.job_id.clone());
            jobs::run(&transaction,transaction.query("CREATE $tombstone SET scope_id=$scope,deployment_id=$deployment,workstation_id=$node,job_id=$job,hidden_at=time::now(),revision=2 RETURN NONE; CREATE $request SET fingerprint=$fingerprint,receipt_json=$receipt RETURN NONE")
                .bind(("tombstone",jobs::rid("workstation_cycle_tombstone",&association))).bind(("scope",admission.authority().scope_id().to_owned())).bind(("deployment",context.deployment_id.clone())).bind(("node",context.workstation_id.clone())).bind(("job",request.job_id.clone()))
                .bind(("request",jobs::rid("workstation_cycle_delete",&key))).bind(("fingerprint",fingerprint)).bind(("receipt",serde_json::to_string(&receipt).map_err(|_|TxError::Abort("cycle:receipt_invalid".into()))?))).await?;
            Ok(receipt)
        }
    }).await?;
    Ok(result.value)
}
