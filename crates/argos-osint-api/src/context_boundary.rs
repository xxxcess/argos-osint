//! Shared public/private request boundary before local handlers or relays.

use argos_osint_core::storage::configuration_reset;
use argos_osint_core::storage::configuration_reset::CredentialFingerprint;
use axum::extract::Request;
use axum::extract::State;
use axum::http::Method;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::Json;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::Permission;
use serde_json::json;

use crate::network::AccessMode;
use crate::request_context;
use crate::request_context::ContextRequirement;
use crate::ApiState;
use crate::AuthenticatedPrincipal;

fn requirement(request: &Request) -> Option<(Permission, ContextRequirement)> {
    let path = request.uri().path();
    if matches!(path, "/api/v1/health/ready" | "/api/v1/capabilities")
        || path.starts_with("/api/v1/connect/")
        || path.starts_with("/api/v1/admin/configuration-reset/")
        || path.starts_with("/api/v1/internal/configuration-reset/")
    {
        return None;
    }
    if matches!(
        path,
        fleetworks_contracts::classifier_activation::ACTIVATION_PATH
            | fleetworks_contracts::classifier_activation::CANDIDATE_DETAIL_PATH
            | fleetworks_contracts::classifier_activation::ACTIVATION_RESULT_PATH
    ) {
        return Some((Permission::MaintainBackend, ContextRequirement::Registered));
    }
    if path.starts_with("/api/v1/workstation/activity")
        || path.starts_with("/api/v1/workstation/requests/")
        || matches!(
            path,
            "/api/v1/jobs"
                | "/api/v1/jobs/detail"
                | "/api/v1/logs"
                | "/api/v1/brief/stages"
                | "/api/v1/tools/result"
        )
    {
        return Some((
            Permission::ReadWorkstationActivity,
            ContextRequirement::Registered,
        ));
    }
    if request.method() == Method::GET || request.method() == Method::HEAD {
        return Some((Permission::ReadBackend, ContextRequirement::GlobalRead));
    }
    let permission = if path.starts_with("/api/v1/profile/") || path.starts_with("/api/v1/models/")
    {
        Permission::ManageConfiguration
    } else if path.starts_with("/api/v1/workstation/datasets")
        || path.starts_with("/api/v1/workstation/maintenance/")
        || path == fleetworks_contracts::search::ACTOR_REINDEX_PATH
    {
        Permission::MaintainBackend
    } else if path.ends_with("/cancel") {
        Permission::CancelJob
    } else if path.ends_with("/retry") {
        Permission::RetryJob
    } else {
        Permission::SubmitInteractive
    };
    let context = if permission == Permission::ManageConfiguration {
        ContextRequirement::GlobalRead
    } else {
        ContextRequirement::Registered
    };
    Some((permission, context))
}

pub(crate) async fn gateway(
    State(state): State<ApiState>,
    mut request: Request,
    next: Next,
) -> Response {
    // The embedded operator adapter stays separate until its retirement gate.
    if request.extensions().get::<AccessMode>() == Some(&AccessMode::Loopback) {
        return next.run(request).await;
    }
    if let Some(principal) = request.extensions().get::<AuthenticatedPrincipal>() {
        let Some(fingerprint) = request.extensions().get::<CredentialFingerprint>() else {
            return denied();
        };
        if matches!(
            principal.purpose(),
            CredentialPurpose::Native | CredentialPurpose::WebRelay
        ) {
            if let Err(error) = configuration_reset::authorize_user_access(
                &state.repo,
                principal.scope_id(),
                principal.access_epoch(),
                fingerprint,
            )
            .await
            {
                return reset_denied(error);
            }
        } else if let Err(error) =
            configuration_reset::authorize_independent_access(&state.repo, fingerprint).await
        {
            return reset_denied(error);
        }
    }
    request_context::strip_untrusted_delegation(request.headers_mut());
    let Some((permission, context)) = requirement(&request) else {
        return no_store(next.run(request).await);
    };
    let Some(principal) = request.extensions().get::<AuthenticatedPrincipal>() else {
        return denied();
    };
    let response = match request_context::resolve_gateway(
        &state.repo,
        principal,
        request.headers(),
        permission,
        context,
    )
    .await
    {
        Ok(resolved) => {
            if let Some(context) = resolved.context() {
                request.extensions_mut().insert(context.clone());
            }
            request.extensions_mut().insert(resolved);
            next.run(request).await
        }
        Err(error) => error.into_response(),
    };
    no_store(response)
}

pub(crate) async fn private(
    State(state): State<ApiState>,
    mut request: Request,
    next: Next,
) -> Response {
    if let Some(fingerprint) = request.extensions().get::<CredentialFingerprint>() {
        if let Err(error) =
            configuration_reset::authorize_independent_access(&state.repo, fingerprint).await
        {
            return reset_denied(error);
        }
    } else {
        return denied();
    }
    let Some((permission, context)) = requirement(&request) else {
        return no_store(next.run(request).await);
    };
    let Some(principal) = request.extensions().get::<AuthenticatedPrincipal>() else {
        return denied();
    };
    let response = match request_context::resolve_private(
        &state.repo,
        principal,
        request.headers(),
        permission,
        context,
    )
    .await
    {
        Ok(resolved) => {
            if matches!(
                resolved.purpose(),
                CredentialPurpose::Native | CredentialPurpose::WebRelay
            ) {
                if let Err(error) = configuration_reset::authorize_delegated_access(
                    &state.repo,
                    resolved.scope_id(),
                    resolved.access_epoch(),
                )
                .await
                {
                    return reset_denied(error);
                }
            }
            if let Some(context) = resolved.context() {
                request.extensions_mut().insert(context.clone());
            }
            request.extensions_mut().insert(resolved);
            next.run(request).await
        }
        Err(error) => error.into_response(),
    };
    no_store(response)
}

fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store, private"),
    );
    response
}

fn denied() -> Response {
    no_store(
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"Purpose-scoped credential required"})),
        )
            .into_response(),
    )
}

fn reset_denied(error: configuration_reset::ResetError) -> Response {
    let status = if error == configuration_reset::ResetError::Forbidden {
        StatusCode::UNAUTHORIZED
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    no_store(
        (
            status,
            Json(json!({"error":"Current user access is unavailable"})),
        )
            .into_response(),
    )
}
