//! Read-only exact receipt reconciliation after an uncertain network result.

use axum::extract::Query;
use axum::extract::State;
use axum::Extension;
use axum::Json;
use fleetworks_contracts::context::Permission;
use serde::Deserialize;

use crate::request_context::ContextFailure;
use crate::request_context::ResolvedRequest;
use crate::ApiResult;
use crate::ApiState;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReceiptQuery {
    request_id: String,
    operation: String,
    original_registration_id: Option<String>,
}

pub(crate) async fn read(
    State(state): State<ApiState>,
    Extension(request): Extension<ResolvedRequest>,
    Query(query): Query<ReceiptQuery>,
) -> ApiResult {
    if !request.permits(Permission::ReadWorkstationActivity) {
        return Err(ContextFailure::forbidden().into());
    }
    if !matches!(
        query.operation.as_str(),
        "atlas_cycle"
            | "manual_tool"
            | "intel_brief"
            | "actor_seed"
            | "classifier_train"
            | "classifier_activate"
            | "actor_reindex"
    ) {
        return Err(ContextFailure::invalid().into());
    }
    request.revalidate(&state.repo).await?;
    let context = request.registered()?;
    let registration = query
        .original_registration_id
        .as_deref()
        .unwrap_or(&context.registration_id);
    let receipt = argos_osint_core::storage::job_context::accepted_request(
        &state.repo,
        request.authority(),
        context,
        registration,
        &query.operation,
        &query.request_id,
    )
    .await
    .map_err(crate::internal)?
    .ok_or_else(crate::missing)?;
    request.revalidate(&state.repo).await?;
    Ok(Json(
        serde_json::to_value(receipt).map_err(crate::internal)?,
    ))
}
