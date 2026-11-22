//! Explicit registered maintenance admission into durable Jobs/outbox storage.

use argos_osint_core::storage::setup_jobs;
use argos_osint_core::storage::tx;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Extension;
use axum::Json;
use fleetworks_contracts::actor_index::ActorIndexStatus;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::MaintenanceReceipt;
use fleetworks_contracts::readiness::MaintenanceRequest;

use crate::request_context::ResolvedRequest;
use crate::ApiError;
use crate::ApiState;

#[derive(Clone, Copy)]
pub(crate) enum SetupIngress<'a> {
    Network,
    Embedded(Option<&'a ActorIndexStatus>),
}

pub(crate) async fn admit_request(
    state: &ApiState,
    context: &ResolvedRequest,
    request: MaintenanceRequest,
    operation: MaintenanceOperation,
    ingress: SetupIngress<'_>,
) -> Result<MaintenanceReceipt, ApiError> {
    if !matches!(
        context.purpose(),
        CredentialPurpose::Native | CredentialPurpose::Operator
    ) || !context.permits(Permission::MaintainBackend)
    {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Native backend maintenance authority required",
        ));
    }
    if operation == MaintenanceOperation::TrainClassifier {
        return Err(ApiError(
            StatusCode::GONE,
            "Natural classifier training has been retired",
        ));
    }
    let admission = crate::admission::capture_checked(
        state,
        context,
        &request.request_id,
        setup_jobs::operation_name(operation),
        Some(&request.context),
    )
    .await?;
    // Accepted request replay preserves its receipt even if a later seed is
    // incomplete/empty. The operation owner still checks the exact payload.
    let accepted_replay = matches!(ingress, SetupIngress::Network)
        && admission
            .replay_job(&state.repo, setup_jobs::operation_name(operation))
            .await
            .map_err(crate::internal)?
            .is_some();
    let accepted = tx::run_tx(state.repo.db(), tx::RetryPolicy::default(), |tx| {
        let admission = &admission;
        let request = &request;
        async move {
            match ingress {
                SetupIngress::Network => {
                    if operation == MaintenanceOperation::ReindexActors && !accepted_replay {
                        setup_jobs::require_seeded_actors_in(&tx).await?;
                    }
                    setup_jobs::admit_in(
                        &tx,
                        admission.authority(),
                        admission.context(),
                        admission.origin(),
                        operation,
                        request,
                    )
                    .await
                }
                SetupIngress::Embedded(index_status) => {
                    setup_jobs::admit_embedded_in(
                        &tx,
                        admission.authority(),
                        admission.context(),
                        admission.origin(),
                        operation,
                        request,
                        index_status,
                    )
                    .await
                }
            }
        }
    })
    .await
    .map_err(|error| match error {
        tx::TxError::Abort(ref code)
            if code == "setup:dispatch_unavailable" || code == "dispatch_pool_disabled" =>
        {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "Durable maintenance consumer is not configured",
            )
        }
        tx::TxError::Abort(ref code) if code == "maintenance:actors_not_seeded" => ApiError(
            StatusCode::CONFLICT,
            "Complete actor seeding and retain actors in the database before building the index",
        ),
        tx::TxError::Abort(ref code) if code == "setup:actor_index_observation_pending" => ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Actor index status is still checking or catching up. Refresh status before building.",
        ),
        tx::TxError::Abort(_) => ApiError(
            StatusCode::CONFLICT,
            "Maintenance context or resource changed; refresh and retry",
        ),
        other => crate::internal(other),
    })?;
    Ok(accepted.value)
}

async fn admit(
    state: ApiState,
    context: ResolvedRequest,
    request: MaintenanceRequest,
    operation: MaintenanceOperation,
) -> Result<(StatusCode, Json<MaintenanceReceipt>), ApiError> {
    let receipt =
        admit_request(&state, &context, request, operation, SetupIngress::Network).await?;
    Ok((StatusCode::ACCEPTED, Json(receipt)))
}

pub(crate) async fn seed(
    State(state): State<ApiState>,
    Extension(context): Extension<ResolvedRequest>,
    Json(request): Json<MaintenanceRequest>,
) -> Result<(StatusCode, Json<MaintenanceReceipt>), ApiError> {
    admit(state, context, request, MaintenanceOperation::SeedActors).await
}
pub(crate) async fn train(
    State(state): State<ApiState>,
    Extension(context): Extension<ResolvedRequest>,
    Json(request): Json<MaintenanceRequest>,
) -> Result<(StatusCode, Json<MaintenanceReceipt>), ApiError> {
    admit(
        state,
        context,
        request,
        MaintenanceOperation::TrainClassifier,
    )
    .await
}

pub(crate) async fn reindex(
    State(state): State<ApiState>,
    Extension(context): Extension<ResolvedRequest>,
    Json(request): Json<MaintenanceRequest>,
) -> Result<(StatusCode, Json<MaintenanceReceipt>), ApiError> {
    admit(state, context, request, MaintenanceOperation::ReindexActors).await
}

#[cfg(test)]
mod tests;
