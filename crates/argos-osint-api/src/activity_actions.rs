//! Permission-separated action adapters over actual guarded durable mutations.

use argos_osint_core::storage::job_actions;
use argos_osint_core::storage::job_actions::Action;
use argos_osint_core::storage::tx::TxError;
use axum::extract::Extension;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use fleetworks_contracts::context::Permission;
use serde::Deserialize;
use serde_json::json;

use crate::request_context::ContextFailure;
use crate::request_context::ResolvedRequest;
use crate::ApiError;
use crate::ApiResult;
use crate::ApiState;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActionRequest {
    pub(crate) id: String,
    pub(crate) request_id: String,
}

pub(crate) async fn cancel(
    State(state): State<ApiState>,
    context: Option<Extension<ResolvedRequest>>,
    Json(input): Json<ActionRequest>,
) -> ApiResult {
    perform(state, context, input, Action::Cancel).await
}

pub(crate) async fn retry(
    State(state): State<ApiState>,
    context: Option<Extension<ResolvedRequest>>,
    Json(input): Json<ActionRequest>,
) -> ApiResult {
    perform(state, context, input, Action::Retry).await
}

async fn perform(
    state: ApiState,
    context: Option<Extension<ResolvedRequest>>,
    input: ActionRequest,
    action: Action,
) -> ApiResult {
    let Some(Extension(context)) = context else {
        return Err(ContextFailure::registration_required().into());
    };
    let permission = match action {
        Action::Cancel => Permission::CancelJob,
        Action::Retry => Permission::RetryJob,
        _ => return Err(ContextFailure::forbidden().into()),
    };
    if !context.permits(permission) || !context.permits(Permission::ReadWorkstationActivity) {
        return Err(ContextFailure::forbidden().into());
    }
    if !argos_osint_core::intel_service::valid_id(&input.id)
        || input.request_id.is_empty()
        || input.request_id.len() > 128
        || !input
            .request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
    {
        return Err(crate::bad());
    }
    let scope = crate::activity::scope_for_request(&state.repo, &context, None).await?;
    let job = crate::activity::authorized_job(&state.repo, &context, &scope, &input.id).await?;
    if !job.private_details_available {
        return Err(ContextFailure::forbidden().into());
    }
    let admission = crate::admission::capture(&context, &input.request_id).await?;
    // This is not a check followed by an unscoped operator call: proof,
    // ownership/privacy, job state, mutation and action receipt share one TX.
    let receipt = job_actions::perform(&state.repo, &admission, action, &input.id)
        .await
        .map_err(action_error)?;
    Ok(Json(
        json!({"done":action==Action::Cancel,"requeued":if action==Action::Retry {receipt.affected_tasks}else{0},"receipt":receipt}),
    ))
}

fn action_error(error: TxError) -> ApiError {
    match &error {
        TxError::Abort(code) if code == "job_action:invalid" => crate::bad(),
        TxError::Abort(code) if code == "job_action:forbidden" => {
            ContextFailure::forbidden().into()
        }
        TxError::Abort(code) if code == "job_action:unavailable" => {
            ContextFailure::missing().into()
        }
        TxError::Abort(code)
            if matches!(
                code.as_str(),
                "job_action:unsupported"
                    | "job_action:request_conflict"
                    | "job_action:context_changed"
                    | "connect:revision"
            ) =>
        {
            ApiError(
                StatusCode::CONFLICT,
                "Job action or current context changed; refresh before retrying",
            )
        }
        TxError::Abort(code) if matches!(code.as_str(), "connect:missing" | "connect:revoked") => {
            ContextFailure::registration_required().into()
        }
        _ => ContextFailure::unavailable().into(),
    }
}
