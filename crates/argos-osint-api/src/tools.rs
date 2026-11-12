//! Authenticated standalone tool execution.

use argos_osint_core::osint::manual::{self, Receipt, SubmitError, Tools};
use axum::extract::{Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::{ToolRunRequest, TOOL_RESULT_PATH, TOOL_RUN_PATH};
use serde::Deserialize;

use crate::private_relay::{PrivateClient, Service};
use crate::request_context::ContextFailure;
use crate::request_context::ResolvedRequest;
use crate::{bad, internal, missing, ApiError, ApiResult, ApiState};

pub(crate) fn routes() -> Router<ApiState> {
    Router::new()
        .route("/api/v1/tools", get(crate::operations::tools))
        .route(TOOL_RUN_PATH, post(submit))
        .route(TOOL_RESULT_PATH, get(result))
        .layer(Extension(Tools::default()))
}

pub(crate) async fn ready(State(state): State<ApiState>) -> ApiResult {
    crate::private_ready(state, "fleetworks-tools").await
}

/// Authenticated connection to the private Tools process.
#[derive(Clone)]
pub struct ToolsClient(PrivateClient);

impl ToolsClient {
    /// Configures an exact HTTP(S) origin and its dedicated bearer token.
    pub fn new(origin: &str, token: &str) -> anyhow::Result<Self> {
        Ok(Self(PrivateClient::new(origin, token, Service::Tools)?))
    }
}

pub(crate) async fn forward(
    State(client): State<ToolsClient>,
    request: Request,
    _next: Next,
) -> Result<Response, Response> {
    client.0.forward(request).await
}

async fn submit(
    State(state): State<ApiState>,
    Extension(tools): Extension<Tools>,
    context: Option<Extension<ResolvedRequest>>,
    Json(request): Json<ToolRunRequest>,
) -> Result<(StatusCode, Json<Receipt>), ApiError> {
    let Some(Extension(context)) = context else {
        return Err(ContextFailure::registration_required().into());
    };
    if !context.permits(Permission::SubmitInteractive) {
        return Err(ContextFailure::forbidden().into());
    }
    let admission = crate::admission::capture_checked(
        &state.repo,
        &context,
        &request.request_id,
        manual::TOOLS_OPERATION,
        request.context.as_ref(),
    )
    .await?;
    let receipt = tools
        .submit_interactive(
            &state.repo,
            &request.request_id,
            &request.tool_id,
            request.inputs,
            &admission,
        )
        .await
        .map_err(|error| match error {
            SubmitError::Invalid => bad(),
            SubmitError::Conflict => {
                ApiError(StatusCode::CONFLICT, "Tool request ID is already in use")
            }
            SubmitError::Busy => {
                ApiError(StatusCode::TOO_MANY_REQUESTS, "Too many pending tool calls")
            }
            SubmitError::ContextChanged => ContextFailure::changed().into(),
            SubmitError::Unavailable(error) => {
                tracing::warn!(error=%argos_osint_core::provider_diag::sanitize(&error.to_string()),"Durable Tools admission unavailable");
                ApiError(StatusCode::SERVICE_UNAVAILABLE, "Durable Tools admission unavailable")
            }
            _ => internal("Unknown tool submission failure"),
        })?;
    Ok((StatusCode::ACCEPTED, Json(receipt)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultQuery {
    id: String,
}

async fn result(
    State(state): State<ApiState>,
    context: Option<Extension<ResolvedRequest>>,
    Query(query): Query<ResultQuery>,
) -> Result<Json<manual::Status>, ApiError> {
    if !manual::valid_call_id(&query.id) {
        return Err(bad());
    }
    let Some(Extension(context)) = context else {
        return Err(ContextFailure::registration_required().into());
    };
    let scope = crate::activity::scope_for_request(&state.repo, &context, None).await?;
    let job = crate::activity::authorized_job(&state.repo, &context, &scope, &query.id).await?;
    if !job.private_details_available || job.job.operation != manual::TOOLS_OPERATION {
        return Err(missing());
    }
    let result = manual::durable_status(&state.repo, &query.id)
        .await
        .map_err(internal)?
        .ok_or_else(missing)?;
    if !crate::activity::authorized_job(&state.repo, &context, &scope, &query.id)
        .await?
        .private_details_available
    {
        return Err(ContextFailure::changed().into());
    }
    Ok(Json(result))
}
