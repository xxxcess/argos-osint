//! Exact public pairing routes and independently authenticated operator decisions.
use argos_osint_core::storage::native_pairing;
use argos_osint_core::storage::native_pairing::OperatorAuthority;
use argos_osint_core::storage::tx::TxError;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::post;
use axum::Extension;
use axum::Json;
use axum::Router;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::pairing::PairingDecisionRequest;
use fleetworks_contracts::pairing::PairingPollRequest;
use fleetworks_contracts::pairing::PairingRevokeRequest;
use fleetworks_contracts::pairing::PairingStartRequest;
use fleetworks_contracts::pairing::PairingStatus;
use serde_json::json;

use crate::network::NetworkPolicy;
use crate::ApiState;
use crate::AuthenticatedPrincipal;

pub(crate) fn routes() -> Router<ApiState> {
    Router::new()
        .route(
            fleetworks_contracts::pairing::PAIRING_START_PATH,
            post(start),
        )
        .route(fleetworks_contracts::pairing::PAIRING_POLL_PATH, post(poll))
        .route(
            fleetworks_contracts::pairing::PAIRING_APPROVE_PATH,
            post(approve),
        )
        .route(fleetworks_contracts::pairing::PAIRING_DENY_PATH, post(deny))
        .route(
            fleetworks_contracts::pairing::PAIRING_REVOKE_PATH,
            post(revoke),
        )
}

struct PairingError(StatusCode, &'static str);
impl IntoResponse for PairingError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error":self.1}))).into_response()
    }
}
fn failure(error: TxError) -> PairingError {
    match error {
        TxError::Abort(reason) => match reason.as_str() {
            "pairing:invalid" => PairingError(StatusCode::BAD_REQUEST, "Invalid pairing request"),
            "pairing:conflict" => PairingError(
                StatusCode::CONFLICT,
                "Pairing request conflicts with retained state",
            ),
            "pairing:expired" => PairingError(StatusCode::GONE, "Pairing request expired"),
            "pairing:rate" => PairingError(
                StatusCode::TOO_MANY_REQUESTS,
                "Pairing is temporarily rate limited",
            ),
            "pairing:missing" | "pairing:forbidden" | "configuration_reset:forbidden" => {
                PairingError(StatusCode::FORBIDDEN, "Pairing access is unavailable")
            }
            _ => PairingError(
                StatusCode::SERVICE_UNAVAILABLE,
                "Pairing authority is unavailable",
            ),
        },
        _ => PairingError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Pairing authority is unavailable",
        ),
    }
}
fn operator(
    principal: Option<Extension<AuthenticatedPrincipal>>,
) -> Result<OperatorAuthority, PairingError> {
    let Some(Extension(principal)) = principal else {
        return Err(PairingError(
            StatusCode::UNAUTHORIZED,
            "Operator authentication required",
        ));
    };
    if principal.purpose() != CredentialPurpose::Operator
        || ![
            Permission::ManageConfiguration,
            Permission::RegisterWorkstation,
            Permission::ReadBackend,
        ]
        .into_iter()
        .all(|p| principal.permits(p))
    {
        return Err(PairingError(
            StatusCode::FORBIDDEN,
            "Pairing approval requires operator authority",
        ));
    }
    Ok(OperatorAuthority {
        credential_id: principal.credential_id().into(),
        scope_id: principal.scope_id().into(),
        access_epoch: principal.access_epoch().into(),
        permissions: principal.permissions().collect(),
    })
}
async fn start(
    State(state): State<ApiState>,
    Json(input): Json<PairingStartRequest>,
) -> Result<Response, PairingError> {
    let challenge = native_pairing::start(&state.repo, &input)
        .await
        .map_err(failure)?;
    Ok((StatusCode::OK, Json(challenge)).into_response())
}
async fn poll(
    State(state): State<ApiState>,
    Extension(policy): Extension<NetworkPolicy>,
    Json(input): Json<PairingPollRequest>,
) -> Result<Response, PairingError> {
    let status = native_pairing::poll(&state.repo, &input)
        .await
        .map_err(failure)?;
    // Even recovery of an already-issued token is fenced by today's issuer
    // policy. Removed/relabelled/reduced operators cannot issue stale authority.
    if let PairingStatus::Approved { access_token, .. } = &status {
        let authority = native_pairing::authenticate(&state.repo, access_token)
            .await
            .map_err(failure)?;
        let allowed = if let Some(authority) = &authority {
            policy.authorizes_paired(&state.repo, authority).await
        } else {
            false
        };
        if !allowed {
            return Err(PairingError(
                StatusCode::FORBIDDEN,
                "Pairing approval is no longer current",
            ));
        }
    }
    Ok(Json(status).into_response())
}
async fn approve(
    State(state): State<ApiState>,
    principal: Option<Extension<AuthenticatedPrincipal>>,
    Json(input): Json<PairingDecisionRequest>,
) -> Result<Response, PairingError> {
    native_pairing::decide(&state.repo, &operator(principal)?, &input, true)
        .await
        .map_err(failure)?;
    Ok(Json(json!({"status":"approved"})).into_response())
}
async fn deny(
    State(state): State<ApiState>,
    principal: Option<Extension<AuthenticatedPrincipal>>,
    Json(input): Json<PairingDecisionRequest>,
) -> Result<Response, PairingError> {
    native_pairing::decide(&state.repo, &operator(principal)?, &input, false)
        .await
        .map_err(failure)?;
    Ok(Json(json!({"status":"denied"})).into_response())
}
async fn revoke(
    State(state): State<ApiState>,
    principal: Option<Extension<AuthenticatedPrincipal>>,
    Json(input): Json<PairingRevokeRequest>,
) -> Result<Response, PairingError> {
    input
        .validate()
        .map_err(|_| PairingError(StatusCode::BAD_REQUEST, "Invalid credential reference"))?;
    let authority = operator(principal)?;
    // Never take a DB guard while waiting for the configuration owner. The
    // writer performs its dynamic-authority check after taking this same lock.
    let lock = tokio::task::spawn_blocking(|| {
        let root = crate::configuration::workspace().map_err(|_| {
            PairingError(
                StatusCode::SERVICE_UNAVAILABLE,
                "Configuration owner unavailable for revocation",
            )
        })?;
        argos_osint_core::config_commit::ConfigLock::acquire_at(&root).map_err(|_| {
            PairingError(
                StatusCode::SERVICE_UNAVAILABLE,
                "Configuration owner lock unavailable for revocation",
            )
        })
    })
    .await
    .map_err(|_| {
        PairingError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Revocation task unavailable",
        )
    })??;
    native_pairing::revoke(&state.repo, &authority, &input.credential_id, &lock)
        .await
        .map_err(failure)?;
    Ok(Json(json!({"status":"revoked"})).into_response())
}

#[cfg(test)]
mod tests;
