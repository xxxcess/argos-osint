//! Exact private training-result eligibility before any artifact/report access.

use fleetworks_contracts::context::RegisteredContext;
use surrealdb::types::SurrealValue;

use super::super::jobs;
use super::super::tx::Tx;
use super::super::tx::TxError;

pub(super) struct TrainingSource {
    pub model_id: String,
    pub dataset_sha256: String,
}
#[derive(SurrealValue)]
struct TrainingRow {
    operation: String,
    scope_id: String,
    result_json: String,
}
#[derive(SurrealValue)]
struct Origin {
    scope_id: String,
    deployment_id: String,
    workstation_id: String,
    workspace_id: String,
}

pub(super) async fn training_in(
    tx: &Tx,
    scope: &str,
    context: &RegisteredContext,
    id: &str,
) -> Result<TrainingSource, TxError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
    {
        return Err(super::invalid("source_unavailable"));
    }
    let mut reply=jobs::run(tx,tx.query("SELECT VALUE job_id FROM job_request WHERE job_id=$job AND scope_id=$scope AND deployment_id=$deployment AND workstation_id=$node AND workspace_id=$workspace LIMIT 1;
        SELECT VALUE job_id FROM job_request WHERE job_id=$job AND (scope_id!=$scope OR deployment_id!=$deployment OR workstation_id!=$node OR workspace_id!=$workspace) LIMIT 1;
        SELECT scope_id,deployment_id,workstation_id,workspace_id FROM ONLY $origin;
        SELECT VALUE state FROM ONLY $record;")
        .bind(("job",id.to_owned())).bind(("scope",scope.to_owned())).bind(("deployment",context.deployment_id.clone())).bind(("node",context.workstation_id.clone())).bind(("workspace",context.workspace_id.clone()))
        .bind(("origin",jobs::rid("job_origin",id))).bind(("record",jobs::rid("job",id)))).await?;
    let member: Vec<String> = jobs::take(&mut reply, 0)?;
    let outside: Vec<String> = jobs::take(&mut reply, 1)?;
    let origin: Option<Origin> = jobs::take(&mut reply, 2)?;
    let state: Option<String> = jobs::take(&mut reply, 3)?;
    // Read no training/report payload until all authority conditions are true.
    if member.is_empty()
        || !outside.is_empty()
        || !origin.is_some_and(|origin| {
            origin.scope_id == scope
                && origin.deployment_id == context.deployment_id
                && origin.workstation_id == context.workstation_id
                && origin.workspace_id == context.workspace_id
        })
        || !state.is_some_and(|state| matches!(state.as_str(), "completed" | "succeeded"))
    {
        return Err(super::invalid("source_unavailable"));
    }
    let mut payload = jobs::run(
        tx,
        tx.query("SELECT operation,scope_id,result_json FROM ONLY $training")
            .bind(("training", jobs::rid("setup_operation", id))),
    )
    .await?;
    let row: TrainingRow = jobs::take::<Option<TrainingRow>>(&mut payload, 0)?
        .ok_or_else(|| super::invalid("source_unavailable"))?;
    if row.operation != super::super::setup_jobs::TRAIN_POOL
        || row.scope_id != scope
        || row.result_json.len() > 64 * 1024
    {
        return Err(super::invalid("source_unavailable"));
    }
    let result: serde_json::Value =
        serde_json::from_str(&row.result_json).map_err(|_| super::invalid("source_unavailable"))?;
    if result["operation"] != "classifier_train"
        || result["engine"] != "natural"
        || result["activation_requested"] != false
    {
        return Err(super::invalid("source_unavailable"));
    }
    let model_id = result["candidate_model_id"]
        .as_str()
        .ok_or_else(|| super::invalid("source_unavailable"))?
        .to_owned();
    let dataset_sha256 = result["dataset_sha256"]
        .as_str()
        .ok_or_else(|| super::invalid("source_unavailable"))?
        .to_owned();
    if crate::classifier::artifact::ModelId::parse(&model_id).is_none()
        || dataset_sha256.len() != 64
        || !dataset_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(super::invalid("source_unavailable"));
    }
    Ok(TrainingSource {
        model_id,
        dataset_sha256,
    })
}
