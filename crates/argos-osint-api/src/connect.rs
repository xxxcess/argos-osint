//! Purpose-authorized native registry and private web-relay session boundary.
//!
//! Connect IDs select existing registered context; they never authenticate a
//! person or confer registration/maintenance authority. No browser-supplied
//! owner, gateway, workstation ID or forwarded header can construct a principal.

use std::net::IpAddr;
use std::net::SocketAddr;

use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::connect_registry::RegistryError;
use axum::extract::ConnectInfo;
use axum::extract::State;
use axum::http::header;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::Extension;
use axum::Json;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::connect::WebSessionCreateRequest;
use fleetworks_contracts::connect::WorkspaceName;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::Permission;
use serde::Deserialize;
use serde_json::json;
use serde_json::Value;

use crate::ApiState;
use crate::AuthenticatedPrincipal;

pub(crate) struct ConnectError(StatusCode, &'static str);

impl IntoResponse for ConnectError {
    fn into_response(self) -> Response {
        let mut response = protected(self.0, json!({"error":self.1}));
        if self.0 == StatusCode::TOO_MANY_REQUESTS {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
        }
        response
    }
}

fn protected(status: StatusCode, value: Value) -> Response {
    let mut response = (status, Json(value)).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}

fn require(
    principal: Option<Extension<AuthenticatedPrincipal>>,
    permission: Permission,
    web_relay: bool,
) -> Result<RegistryAuthority, ConnectError> {
    let Some(Extension(principal)) = principal else {
        return Err(ConnectError(
            StatusCode::UNAUTHORIZED,
            "Purpose-scoped gateway credential required",
        ));
    };
    let purpose = if web_relay {
        principal.purpose() == CredentialPurpose::WebRelay
    } else {
        matches!(
            principal.purpose(),
            CredentialPurpose::Native | CredentialPurpose::Operator
        )
    };
    if !purpose || !principal.permits(permission) {
        return Err(ConnectError(
            StatusCode::FORBIDDEN,
            "Operation is not permitted for this credential purpose",
        ));
    }
    RegistryAuthority::new(
        principal.scope_id(),
        principal.credential_id(),
        principal.access_epoch(),
    )
    .map_err(|_| ConnectError(StatusCode::FORBIDDEN, "Credential authority is unavailable"))
}

fn failure(error: RegistryError, browser: bool) -> ConnectError {
    match error {
        RegistryError::Storage(_) | RegistryError::Unconfigured => ConnectError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Connection authority unavailable",
        ),
        RegistryError::AdmissionPaused => ConnectError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Registered admission paused for maintenance",
        ),
        RegistryError::RateLimited => ConnectError(
            StatusCode::TOO_MANY_REQUESTS,
            "Connection resolution rate limited",
        ),
        RegistryError::RevisionMismatch => ConnectError(
            StatusCode::CONFLICT,
            "Connection context changed; reconnect before retrying",
        ),
        RegistryError::InvalidInput => ConnectError(
            StatusCode::BAD_REQUEST,
            "Invalid registered context request",
        ),
        RegistryError::ProtectedDefault if !browser => ConnectError(
            StatusCode::CONFLICT,
            "Protected default workspace cannot be retired or renamed",
        ),
        RegistryError::Collision if !browser => ConnectError(
            StatusCode::CONFLICT,
            "Workstation IP or workspace identity collision; explicitly repair or rebind",
        ),
        RegistryError::NotFound
        | RegistryError::Revoked
        | RegistryError::Collision
        | RegistryError::ProtectedDefault => ConnectError(
            StatusCode::UNAUTHORIZED,
            "Connection context is invalid or unavailable",
        ),
        _ => ConnectError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Connection authority unavailable",
        ),
    }
}

/// Native ensure is never added to the ordinary browser relay allowlist.
pub(crate) async fn ensure(
    State(state): State<ApiState>,
    principal: Option<Extension<AuthenticatedPrincipal>>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    Json(input): Json<RegistrationEnsureRequest>,
) -> Result<Response, ConnectError> {
    let authority = require(principal, Permission::RegisterWorkstation, false)?;
    let observed_peer = peer.map(|Extension(ConnectInfo(address))| address.ip());
    let result = connect_registry::ensure_observed(&state.repo, &authority, &input, observed_peer)
        .await
        .map_err(|error| failure(error, false))?;
    let status = if result.created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    let value = serde_json::to_value(result.receipt).map_err(|_| {
        ConnectError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Registration response unavailable",
        )
    })?;
    Ok(protected(status, value))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RebindRequest {
    request_id: String,
    registration_id: String,
    expected_revision: u64,
    workstation_ip: IpAddr,
    workspace_name: WorkspaceName,
    installation_marker: String,
    workspace_marker: String,
}

pub(crate) async fn rebind(
    State(state): State<ApiState>,
    principal: Option<Extension<AuthenticatedPrincipal>>,
    Json(input): Json<RebindRequest>,
) -> Result<Response, ConnectError> {
    let authority = require(principal, Permission::RebindWorkstation, false)?;
    let request = RegistrationEnsureRequest::new(
        input.request_id,
        input.workstation_ip,
        input.workspace_name,
        input.installation_marker,
        input.workspace_marker,
    );
    let input =
        connect_registry::RebindInput::new(request, input.registration_id, input.expected_revision);
    let result = connect_registry::rebind(&state.repo, &authority, &input)
        .await
        .map_err(|error| failure(error, false))?;
    let value = serde_json::to_value(result.receipt).map_err(|_| {
        ConnectError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Registration response unavailable",
        )
    })?;
    Ok(protected(StatusCode::OK, value))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegistrationRevisionRequest {
    registration_id: String,
    expected_revision: u64,
}

pub(crate) async fn retire(
    State(state): State<ApiState>,
    principal: Option<Extension<AuthenticatedPrincipal>>,
    Json(input): Json<RegistrationRevisionRequest>,
) -> Result<Response, ConnectError> {
    let authority = require(principal, Permission::RebindWorkstation, false)?;
    let pending = connect_registry::begin_retirement(
        &state.repo,
        &authority,
        &input.registration_id,
        input.expected_revision,
    )
    .await
    .map_err(|error| failure(error, false))?;
    let result = connect_registry::finish_retirement(
        &state.repo,
        &authority,
        &pending.registration_id,
        pending.revision,
    )
    .await
    .map_err(|error| failure(error, false))?;
    Ok(match result {
        connect_registry::RetirementResult::Complete(receipt) => protected(
            StatusCode::OK,
            json!({
                "status":"retired","deletion_ready":true,"deployment_id":receipt.deployment_id,"registration_id":receipt.registration_id,
                "workspace_id":receipt.workspace_id,"registration_revision":receipt.registration_revision,"revision":receipt.registration_revision,
                "receipt_id":receipt.receipt_id,"admission_fenced":receipt.admission_fenced,"sessions_revoked":receipt.sessions_revoked,"active_work_drained":receipt.active_work_drained
            }),
        ),
        connect_registry::RetirementResult::Pending(pending) => protected(
            StatusCode::ACCEPTED,
            json!({
                "status":"retirement_pending", "deletion_ready":false,
                "registration_id":pending.registration_id, "workspace_id":pending.workspace_id,
                "revision":pending.revision,"admission_fenced":true,"sessions_revoked":true,"active_work_drained":false,"reason_code":"associated_work_active"
            }),
        ),
        _ => {
            return Err(ConnectError(
                StatusCode::SERVICE_UNAVAILABLE,
                "Retirement state unavailable",
            ))
        }
    })
}

pub(crate) async fn registration_context(
    State(state): State<ApiState>,
    principal: Option<Extension<AuthenticatedPrincipal>>,
    Json(input): Json<RegistrationRevisionRequest>,
) -> Result<Response, ConnectError> {
    let authority = require(principal, Permission::RegisterWorkstation, false)?;
    let context = connect_registry::registration_context(
        &state.repo,
        &authority,
        &input.registration_id,
        input.expected_revision,
    )
    .await
    .map_err(|error| failure(error, false))?;
    Ok(protected(StatusCode::OK, json!({"context":context})))
}

/// Token response is for the authenticated server relay, never browser JavaScript.
pub(crate) async fn create_session(
    State(state): State<ApiState>,
    principal: Option<Extension<AuthenticatedPrincipal>>,
    Json(input): Json<WebSessionCreateRequest>,
) -> Result<Response, ConnectError> {
    let authority = require(principal, Permission::ResolveWebSession, true)?;
    let session = connect_registry::create_session(&state.repo, &authority, &input.connect_id)
        .await
        .map_err(|error| failure(error, true))?;
    Ok(protected(
        StatusCode::CREATED,
        json!({
            "session_token":session.token.expose_secret(),
            "context":session.context, "expires_at":session.expires_at
        }),
    ))
}

/// This body intentionally has no Debug/Serialize implementation.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionProof {
    session_token: String,
    #[serde(default)]
    expected_context_revision: Option<u64>,
}

pub(crate) async fn resolve_session(
    State(state): State<ApiState>,
    principal: Option<Extension<AuthenticatedPrincipal>>,
    Json(input): Json<SessionProof>,
) -> Result<Response, ConnectError> {
    let authority = require(principal, Permission::ResolveWebSession, true)?;
    let session = connect_registry::validate_session(
        &state.repo,
        &authority,
        &input.session_token,
        input.expected_context_revision,
    )
    .await
    .map_err(|error| failure(error, true))?;
    Ok(protected(
        StatusCode::OK,
        json!({"context":session.context,"expires_at":session.expires_at}),
    ))
}

pub(crate) async fn logout_session(
    State(state): State<ApiState>,
    principal: Option<Extension<AuthenticatedPrincipal>>,
    Json(input): Json<SessionProof>,
) -> Result<Response, ConnectError> {
    let authority = require(principal, Permission::ResolveWebSession, true)?;
    connect_registry::revoke_session(&state.repo, &authority, &input.session_token)
        .await
        .map_err(|error| failure(error, true))?;
    Ok(protected(StatusCode::OK, json!({"revoked":true})))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::Arc;

    use axum::body::Body;
    use axum::extract::Request;
    use axum::middleware;
    use axum::routing::post;
    use axum::Router;
    use tower::ServiceExt;

    use super::*;
    use crate::network::protect;
    use crate::network::AccessPolicy;
    use crate::NetworkPolicy;
    use crate::ScopedCredential;

    const TOKEN: &str = "synthetic-connect-boundary-token-000000000000";

    async fn registration_boundary(
        principal: Option<Extension<AuthenticatedPrincipal>>,
    ) -> Result<StatusCode, ConnectError> {
        require(principal, Permission::RegisterWorkstation, false)?;
        Ok(StatusCode::NO_CONTENT)
    }

    async fn session_boundary(
        principal: Option<Extension<AuthenticatedPrincipal>>,
    ) -> Result<StatusCode, ConnectError> {
        require(principal, Permission::ResolveWebSession, true)?;
        Ok(StatusCode::NO_CONTENT)
    }

    #[tokio::test]
    async fn purpose_policy_rejects_legacy_and_cross_purpose_authority_even_with_forged_headers() {
        let cases = [
            (None, "/registration", StatusCode::UNAUTHORIZED),
            (
                Some((CredentialPurpose::WebRelay, Permission::ResolveWebSession)),
                "/registration",
                StatusCode::FORBIDDEN,
            ),
            (
                Some((CredentialPurpose::Native, Permission::RegisterWorkstation)),
                "/registration",
                StatusCode::NO_CONTENT,
            ),
            (
                Some((CredentialPurpose::Native, Permission::ResolveWebSession)),
                "/session",
                StatusCode::FORBIDDEN,
            ),
            (
                Some((CredentialPurpose::WebRelay, Permission::ResolveWebSession)),
                "/session",
                StatusCode::NO_CONTENT,
            ),
        ];
        for (scope, route, expected) in cases {
            let network = match scope {
                Some((purpose, permission)) => NetworkPolicy::with_credentials(
                    &["api:8080".into()],
                    vec![ScopedCredential::new(
                        "synthetic-credential".into(),
                        "synthetic-scope".into(),
                        "synthetic-epoch".into(),
                        purpose,
                        BTreeSet::from([permission]),
                        TOKEN,
                    )
                    .expect("synthetic purpose credential")],
                )
                .expect("synthetic ingress policy"),
                None => NetworkPolicy::new(&["api:8080".into()], TOKEN)
                    .expect("legacy synthetic policy"),
            };
            let app = Router::new()
                .route("/registration", post(registration_boundary))
                .route("/session", post(session_boundary))
                .layer(middleware::from_fn_with_state(
                    AccessPolicy {
                        port: 8080,
                        origins: Arc::new(Vec::new()),
                        network: Some(network),
                    },
                    protect,
                ));
            let request = Request::builder()
                .method("POST")
                .uri(route)
                .header(header::HOST, "api:8080")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .header("x-fleetworks-scope-id", "forged-owner")
                .header("x-fleetworks-permissions", "register_workstation")
                .body(Body::empty())
                .expect("synthetic request");
            assert_eq!(
                app.oneshot(request)
                    .await
                    .expect("boundary response")
                    .status(),
                expected
            );
        }
    }

    #[test]
    fn browser_session_payload_cannot_assign_owner_gateway_or_workstation() {
        let mut payload = json!({"session_token":"synthetic-not-a-real-session"});
        for name in [
            "gateway",
            "owner",
            "workstation_id",
            "workspace_id",
            "access_epoch",
        ] {
            payload[name] = json!("forged");
            assert!(serde_json::from_value::<SessionProof>(payload.clone()).is_err());
            payload
                .as_object_mut()
                .expect("fixture object")
                .remove(name);
        }
    }

    #[tokio::test]
    async fn browser_errors_are_non_enumerating_private_and_rate_limited() {
        use axum::body::to_bytes;
        let mut bodies = Vec::new();
        for error in [
            RegistryError::NotFound,
            RegistryError::Revoked,
            RegistryError::Collision,
        ] {
            let response = failure(error, true).into_response();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(
                response.headers()[header::CACHE_CONTROL],
                "no-store, private"
            );
            bodies.push(
                to_bytes(response.into_body(), 4096)
                    .await
                    .expect("bounded error body"),
            );
        }
        assert!(bodies.windows(2).all(|pair| pair[0] == pair[1]));
        let response = failure(RegistryError::RateLimited, true).into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()[header::RETRY_AFTER], "60");
    }
}
