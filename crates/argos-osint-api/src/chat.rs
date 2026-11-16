//! Assistant chat turn endpoint, V1.
//!
//! The endpoint checks the caller and the request, then answers "ok". It stores nothing, starts
//! no job and never writes an assistant reply. Only native registered callers may use it; it is
//! not on the browser relay.

use axum::extract::Extension;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::Json;
use axum::Router;
use fleetworks_contracts::chat::ChatTurnAck;
use fleetworks_contracts::chat::ChatTurnRequest;
use fleetworks_contracts::chat::CHAT_TURNS_PATH;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::Permission;

use crate::request_context::ContextFailure;
use crate::request_context::ResolvedRequest;
use crate::ApiError;
use crate::ApiState;

pub(crate) fn routes() -> Router<ApiState> {
    Router::new().route(CHAT_TURNS_PATH, post(submit))
}

/// Shared by HTTP and IPC so both transports answer the same way.
pub(crate) async fn submit_request(
    state: &ApiState,
    context: &ResolvedRequest,
    request: ChatTurnRequest,
) -> Result<ChatTurnAck, ApiError> {
    if context.purpose() != CredentialPurpose::Native
        || !context.permits(Permission::SubmitInteractive)
    {
        return Err(ContextFailure::forbidden().into());
    }
    let registered = context.registered()?;
    request.validate().map_err(|_| crate::bad())?;
    if request.context.registration_id != registered.registration_id
        || request.context.expected_registration_revision != registered.registration_revision
        || request.context.expected_context_revision != registered.context_revision
    {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Request origin changed; refresh confirmation",
        ));
    }
    context.revalidate(&state.repo).await?;
    Ok(ChatTurnAck::ok())
}

async fn submit(
    State(state): State<ApiState>,
    context: Option<Extension<ResolvedRequest>>,
    Json(request): Json<ChatTurnRequest>,
) -> Result<Json<ChatTurnAck>, ApiError> {
    let Some(Extension(context)) = context else {
        return Err(ContextFailure::registration_required().into());
    };
    Ok(Json(submit_request(&state, &context, request).await?))
}
