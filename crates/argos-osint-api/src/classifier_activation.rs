//! Explicit registered Natural candidate preview and durable activation.
//! Training-job membership/private eligibility precedes every artifact/report
//! lookup; maintenance permission is separate and does not follow membership.

use std::path::PathBuf;
use std::sync::Arc;

use argos_osint_core::classifier::artifact::ArtifactStore;
use argos_osint_core::classifier::candidate_activation;
use argos_osint_core::storage::classifier_activation;
use axum::extract::Extension;
use axum::extract::Query;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::routing::post;
use axum::Json;
use axum::Router;
use fleetworks_contracts::classifier_activation::ActivationReceipt;
use fleetworks_contracts::classifier_activation::ActivationRequest;
use fleetworks_contracts::classifier_activation::ActivationResult;
use fleetworks_contracts::classifier_activation::CandidatePreview;
use fleetworks_contracts::classifier_activation::SyntheticBaselineConsent;
use fleetworks_contracts::classifier_activation::ACTIVATION_PATH;
use fleetworks_contracts::classifier_activation::ACTIVATION_RESULT_PATH;
use fleetworks_contracts::classifier_activation::CANDIDATE_DETAIL_PATH;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::Permission;
use serde::Deserialize;

use crate::activity;
use crate::request_context::ContextFailure;
use crate::request_context::ResolvedRequest;
use crate::ApiError;
use crate::ApiState;

/// Bind only the actual server-owned artifact home during process startup.
/// No body/query path or frontend-selected folder can create this owner.
#[derive(Clone)]
pub struct ActivationArtifacts {
    store: ArtifactStore,
}
impl ActivationArtifacts {
    pub fn new(home: PathBuf) -> Result<Self, argos_osint_core::classifier::ClassifierError> {
        Ok(Self {
            store: ArtifactStore::new(&home)?,
        })
    }
}

pub(crate) fn routes() -> Router<ApiState> {
    Router::new()
        .route(CANDIDATE_DETAIL_PATH, get(preview))
        .route(ACTIVATION_PATH, post(submit))
        .route(ACTIVATION_RESULT_PATH, get(result))
}
fn authority(context: Option<Extension<ResolvedRequest>>) -> Result<ResolvedRequest, ApiError> {
    let Some(Extension(context)) = context else {
        return Err(ContextFailure::registration_required().into());
    };
    if context.purpose() != CredentialPurpose::Native
        || !context.permits(Permission::MaintainBackend)
        || !context.permits(Permission::ReadWorkstationActivity)
    {
        return Err(ContextFailure::forbidden().into());
    }
    context.registered()?;
    Ok(context)
}
fn artifacts(
    owner: Option<Extension<Arc<ActivationArtifacts>>>,
) -> Result<Arc<ActivationArtifacts>, ApiError> {
    owner.map(|Extension(owner)| owner).ok_or(ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "Natural artifact owner unavailable",
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewQuery {
    training_job_id: String,
    model_id: String,
}
async fn eligible(
    state: &ApiState,
    context: &ResolvedRequest,
    job_id: &str,
) -> Result<(), ApiError> {
    let scope = activity::scope_for_request(&state.repo, context, None).await?;
    let job = activity::authorized_job(&state.repo, context, &scope, job_id).await?;
    if !job.private_details_available {
        return Err(crate::missing());
    }
    Ok(())
}
async fn actual_preview(
    state: &ApiState,
    context: &ResolvedRequest,
    owner: &ActivationArtifacts,
    query: &PreviewQuery,
) -> Result<CandidatePreview, ApiError> {
    eligible(state, context, &query.training_job_id).await?;
    let source = classifier_activation::training_source(
        &state.repo,
        context.scope_id(),
        context.registered()?,
        &query.training_job_id,
    )
    .await
    .map_err(|_| crate::missing())?;
    if source.model_id != query.model_id {
        return Err(crate::missing());
    }
    let store = owner.store.clone();
    let job = source.training_job_id;
    let model = source.model_id;
    let preview =
        tokio::task::spawn_blocking(move || candidate_activation::preview(&store, &job, &model))
            .await
            .map_err(|_| crate::internal("Natural preview unavailable"))?
            .map_err(failure)?;
    if preview.candidate.dataset_sha256 != source.dataset_sha256 {
        return Err(crate::missing());
    }
    eligible(state, context, &query.training_job_id).await?;
    context.revalidate(&state.repo).await?;
    Ok(preview)
}
async fn preview(
    State(state): State<ApiState>,
    context: Option<Extension<ResolvedRequest>>,
    owner: Option<Extension<Arc<ActivationArtifacts>>>,
    Query(query): Query<PreviewQuery>,
) -> Result<Json<CandidatePreview>, ApiError> {
    let context = authority(context)?;
    let owner = artifacts(owner)?;
    Ok(Json(
        actual_preview(&state, &context, &owner, &query).await?,
    ))
}
async fn submit(
    State(state): State<ApiState>,
    context: Option<Extension<ResolvedRequest>>,
    owner: Option<Extension<Arc<ActivationArtifacts>>>,
    Json(request): Json<ActivationRequest>,
) -> Result<(StatusCode, Json<ActivationReceipt>), ApiError> {
    let context = authority(context)?;
    let owner = artifacts(owner)?;
    request.validate().map_err(|error| {
        if error == fleetworks_contracts::classifier_activation::ActivationInputError::MissingConfigurationRevision {
            ApiError(StatusCode::PRECONDITION_REQUIRED, "Configuration revision required")
        } else {crate::bad()}
    })?;
    let admission = crate::admission::capture_checked(
        &state.repo,
        &context,
        &request.request_id,
        classifier_activation::OPERATION,
        Some(&request.context),
    )
    .await?;
    // Exact accepted replay is reconciled before candidate/prior can change.
    // Storage still requires matching immutable input and real current proof.
    let replay = admission
        .replay_job(&state.repo, classifier_activation::OPERATION)
        .await
        .map_err(crate::internal)?
        .is_some();
    if !replay {
        let preview = actual_preview(
            &state,
            &context,
            &owner,
            &PreviewQuery {
                training_job_id: request.candidate.training_job_id.clone(),
                model_id: request.candidate.model_id.clone(),
            },
        )
        .await?;
        if !preview.activation_supported {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "Natural activation evidence unavailable",
            ));
        }
        if preview.candidate != request.candidate
            || preview.expected_prior != request.expected_prior
        {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "Candidate or prior Natural model changed; refresh confirmation",
            ));
        }
        if preview.synthetic_baseline
            && request.synthetic_baseline != SyntheticBaselineConsent::ActivateSyntheticBaseline
        {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                "Explicit synthetic baseline consent required",
            ));
        }
    }
    let receipt = classifier_activation::admit(&state.repo, &admission, &request)
        .await
        .map_err(storage_failure)?;
    Ok((StatusCode::ACCEPTED, Json(receipt)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultQuery {
    job_id: String,
}
async fn result(
    State(state): State<ApiState>,
    context: Option<Extension<ResolvedRequest>>,
    Query(query): Query<ResultQuery>,
) -> Result<Json<ActivationResult>, ApiError> {
    let context = authority(context)?;
    eligible(&state, &context, &query.job_id).await?;
    let result = classifier_activation::result_for_job(&state.repo, &query.job_id)
        .await
        .map_err(storage_failure)?
        .ok_or_else(crate::missing)?;
    eligible(&state, &context, &query.job_id).await?;
    Ok(Json(result))
}
fn storage_failure(error: argos_osint_core::storage::tx::TxError) -> ApiError {
    match error {
        argos_osint_core::storage::tx::TxError::Abort(ref code)
            if code == "classifier_activation:consumer_unavailable" =>
        {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "Natural activation consumer unavailable",
            )
        }
        argos_osint_core::storage::tx::TxError::Abort(_) => ApiError(
            StatusCode::CONFLICT,
            "Natural activation context, source or resource changed",
        ),
        _ => crate::internal("Natural activation authority unavailable"),
    }
}
fn failure(error: candidate_activation::ActivationFailure) -> ApiError {
    use candidate_activation::ActivationFailure;
    let status = match error {
        ActivationFailure::Busy | ActivationFailure::Stale => StatusCode::CONFLICT,
        ActivationFailure::Invalid | ActivationFailure::ConsentRequired => StatusCode::BAD_REQUEST,
        ActivationFailure::CandidateUnqualified => StatusCode::NOT_FOUND,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    ApiError(status, error.reason_code())
}
