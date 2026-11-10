//! Authenticated standalone tool execution.

use argos_osint_core::osint::manual::{self, Receipt, SubmitError, Tools};
use axum::extract::{Query, Request, State};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use fleetworks_contracts::{ToolRunRequest, TOOL_RESULT_PATH, TOOL_RUN_PATH};
use serde::Deserialize;
use serde_json::json;

use crate::private_relay::{PrivateClient, Service};
use crate::{bad, internal, missing, ApiError, ApiResult, ApiState};

pub(crate) fn routes() -> Router<ApiState> {
    Router::new()
        .route("/api/v1/tools", get(crate::operations::tools))
        .route(TOOL_RUN_PATH, post(submit))
        .route(TOOL_RESULT_PATH, get(result))
        .layer(Extension(Tools::default()))
}

pub(crate) async fn ready(State(state): State<ApiState>) -> ApiResult {
    crate::storage_ready(&state.repo).await?;
    Ok(Json(json!({
        "service": "fleetworks-tools", "api_version": 1, "ready": true,
        "storage": true, "instance_id": state.instance_id,
    })))
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
    let allowed = matches!(
        (request.method(), request.uri().path()),
        (&Method::GET, "/api/v1/tools" | TOOL_RESULT_PATH) | (&Method::POST, TOOL_RUN_PATH)
    );
    if !allowed {
        return Err(StatusCode::METHOD_NOT_ALLOWED.into_response());
    }
    client.0.forward(request).await
}

async fn submit(
    State(state): State<ApiState>,
    Extension(tools): Extension<Tools>,
    Json(request): Json<ToolRunRequest>,
) -> Result<(StatusCode, Json<Receipt>), ApiError> {
    let receipt = tools
        .submit(
            state.repo,
            &request.request_id,
            &request.tool_id,
            request.inputs,
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
            SubmitError::Unavailable(error) => internal(error),
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
    Query(query): Query<ResultQuery>,
) -> Result<Json<manual::Status>, ApiError> {
    if !manual::valid_call_id(&query.id) {
        return Err(bad());
    }
    let result = manual::status(&state.repo, &query.id)
        .await
        .map_err(internal)?
        .ok_or_else(missing)?;
    Ok(Json(result))
}
