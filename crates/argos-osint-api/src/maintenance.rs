//! Explicit registered maintenance admission into durable Jobs/outbox storage.

use argos_osint_core::storage::setup_jobs;
use argos_osint_core::storage::tx;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Extension;
use axum::Json;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::Permission;
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
) -> Result<(StatusCode, Json<MaintenanceReceipt>), ApiError> {
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
    let admission = crate::admission::capture_checked(
        &state.repo,
        &context,
        &request.request_id,
        setup_jobs::operation_name(operation),
        Some(&request.context),
    )
    .await?;
    // Accepted request replay preserves its receipt even if a later seed is
    // incomplete/empty. The operation owner still checks the exact payload.
    let accepted_replay = admission
        .replay_job(&state.repo, setup_jobs::operation_name(operation))
        .await
        .map_err(crate::internal)?
        .is_some();
    let accepted = tx::run_tx(state.repo.db(), tx::RetryPolicy::default(), |tx| {
        let admission = &admission;
        let request = &request;
        async move {
            if operation == MaintenanceOperation::ReindexActors && !accepted_replay {
                require_seeded_actors(&tx).await?;
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
        tx::TxError::Abort(_) => ApiError(
            StatusCode::CONFLICT,
            "Maintenance context or resource changed; refresh and retry",
        ),
        other => crate::internal(other),
    })?;
    Ok((StatusCode::ACCEPTED, Json(accepted.value)))
}

/// Check and fence both requirements inside the actual acceptance transaction.
/// Unchanged-value writes serialize against seed replacement and actor deletion
/// without adding a second catalog counter or admitting from stale readiness.
async fn require_seeded_actors(transaction: &tx::Tx) -> Result<(), tx::TxError> {
    // Seed admission writes this same resource before queuing work. A queued
    // reseed must fence index admission even while the old catalog is complete.
    let mut seed = transaction.query("UPSERT setup_resource:actor_catalog SET revision=(revision ?? 0)+1 RETURN NONE; SELECT VALUE IF job_id != '' { ((type::record('job',job_id).state ?? '') IN ['queued','running','waiting','reconciling'] OR array::len((SELECT VALUE id FROM task WHERE job=type::record('job',$parent.job_id) AND state NOT IN ['completed','failed','cancelled','superseded','stopped','skipped'] LIMIT 1))>0) } ELSE { false } FROM ONLY setup_resource:actor_catalog;")
        .await?.check()?;
    let active: Option<bool> = seed.take(1)?;
    if active != Some(false) {
        return Err(tx::TxError::Abort("maintenance:actors_not_seeded".into()));
    }
    let mut result = transaction.query("RETURN count(UPDATE brain_actor_seed:catalog SET status='complete' WHERE status='complete' AND total>0 AND processed=total RETURN VALUE id)=1; RETURN count(UPDATE (SELECT VALUE id FROM brain_actor WHERE scope_key=$scope LIMIT 1) SET scope_key=$scope RETURN VALUE id)=1;")
        .bind(("scope", argos_osint_core::actor_catalog::SCOPE)).await?.check()?;
    let seed_ready: Option<bool> = result.take(0)?;
    let actors_present: Option<bool> = result.take(1)?;
    if seed_ready != Some(true) || actors_present != Some(true) {
        return Err(tx::TxError::Abort("maintenance:actors_not_seeded".into()));
    }
    Ok(())
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
