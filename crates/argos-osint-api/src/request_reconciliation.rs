//! Read-only exact receipt reconciliation after an uncertain network result.

use axum::extract::Query;
use axum::extract::State;
use axum::Extension;
use axum::Json;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::context::WorkstationRequestDetailQuery;

use crate::request_context::ContextFailure;
use crate::request_context::ResolvedRequest;
use crate::ApiError;
use crate::ApiResult;
use crate::ApiState;

pub(crate) async fn read_value(
    state: &ApiState,
    request: &ResolvedRequest,
    query: &WorkstationRequestDetailQuery,
) -> Result<serde_json::Value, ApiError> {
    if !request.permits(Permission::ReadWorkstationActivity) {
        return Err(ContextFailure::forbidden().into());
    }
    if query.validate().is_err() {
        return Err(ContextFailure::invalid().into());
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
            | "atlas.cycle_history.delete"
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
    serde_json::to_value(receipt).map_err(crate::internal)
}

pub(crate) async fn read(
    State(state): State<ApiState>,
    Extension(request): Extension<ResolvedRequest>,
    Query(query): Query<WorkstationRequestDetailQuery>,
) -> ApiResult {
    Ok(Json(read_value(&state, &request, &query).await?))
}
