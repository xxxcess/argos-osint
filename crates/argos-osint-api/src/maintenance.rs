//! Explicit registered maintenance admission into durable Jobs/outbox storage.

use argos_osint_core::storage::setup_jobs;
use argos_osint_core::storage::tx;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Extension;
use axum::Json;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::MaintenanceReceipt;
use fleetworks_contracts::readiness::MaintenanceRequest;

use crate::request_context::ResolvedRequest;
use crate::ApiError;
use crate::ApiState;

async fn admit(
    state: ApiState,
    context: ResolvedRequest,
    request: MaintenanceRequest,
    operation: MaintenanceOperation,
    owners: Option<crate::workstation::ReadinessOwners>,
) -> Result<(StatusCode, Json<MaintenanceReceipt>), ApiError> {
    if !matches!(
        context.purpose(),
        CredentialPurpose::Native | CredentialPurpose::Operator
    ) {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Native backend maintenance authority required",
        ));
    }
    let configured = owners
        .as_ref()
        .and_then(|owners| owners.configured_input(operation))
        .ok_or(ApiError(
            StatusCode::CONFLICT,
            "Maintenance input is not configured",
        ))?;
    if request.dataset != configured.dataset
        || request.context.expected_configuration_revision
            != Some(configured.configuration_revision)
    {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Configured input or configuration revision changed; refresh before retrying",
        ));
    }
    let admission = crate::admission::capture_checked(
        &state.repo,
        &context,
        &request.request_id,
        setup_jobs::operation_name(operation),
        Some(&request.context),
    )
    .await?;
    let accepted = tx::run_tx(state.repo.db(), tx::RetryPolicy::default(), |tx| {
        let admission = &admission;
        let request = &request;
        async move {
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
        tx::TxError::Abort(_) => ApiError(
            StatusCode::CONFLICT,
            "Maintenance context or resource changed; refresh and retry",
        ),
        other => crate::internal(other),
    })?;
    Ok((StatusCode::ACCEPTED, Json(accepted.value)))
}

pub(crate) async fn seed(
    State(state): State<ApiState>,
    Extension(context): Extension<ResolvedRequest>,
    owners: Option<Extension<crate::workstation::ReadinessOwners>>,
    Json(request): Json<MaintenanceRequest>,
) -> Result<(StatusCode, Json<MaintenanceReceipt>), ApiError> {
    admit(
        state,
        context,
        request,
        MaintenanceOperation::SeedActors,
        owners.map(|Extension(value)| value),
    )
    .await
}
pub(crate) async fn train(
    State(state): State<ApiState>,
    Extension(context): Extension<ResolvedRequest>,
    owners: Option<Extension<crate::workstation::ReadinessOwners>>,
    Json(request): Json<MaintenanceRequest>,
) -> Result<(StatusCode, Json<MaintenanceReceipt>), ApiError> {
    admit(
        state,
        context,
        request,
        MaintenanceOperation::TrainClassifier,
        owners.map(|Extension(value)| value),
    )
    .await
}

pub(crate) async fn reindex(
    State(state): State<ApiState>,
    Extension(context): Extension<ResolvedRequest>,
    owners: Option<Extension<crate::workstation::ReadinessOwners>>,
    Json(request): Json<MaintenanceRequest>,
) -> Result<(StatusCode, Json<MaintenanceReceipt>), ApiError> {
    admit(
        state,
        context,
        request,
        MaintenanceOperation::ReindexActors,
        owners.map(|Extension(value)| value),
    )
    .await
}
