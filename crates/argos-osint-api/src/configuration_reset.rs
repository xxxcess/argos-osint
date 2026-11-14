//! Independent Operator reset endpoint; never a Native/WebRelay recovery bypass.
//!
//! The Coordinator extension is installed by deployment startup with actual
//! policy/configuration owners. Unsupported builds refuse prepare before writes.

mod owners;

pub use owners::ResetOwner;
pub use owners::ResetOwnerClient;
pub use owners::ResetPolicyObserver;

use std::sync::Arc;

use argos_osint_core::storage::configuration_reset::CompletionRequest;
use argos_osint_core::storage::configuration_reset::Coordinator;
use argos_osint_core::storage::configuration_reset::ResetAuthority;
use argos_osint_core::storage::configuration_reset::ResetError;
use argos_osint_core::storage::configuration_reset::ResetReceipt;
use argos_osint_core::storage::configuration_reset::ResetRequest;
use axum::extract::Extension;
use axum::extract::Query;
use axum::extract::State;
use axum::http::header;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use axum::routing::post;
use axum::Json;
use axum::Router;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::Permission;
use serde_json::json;

use crate::ApiState;
use crate::AuthenticatedPrincipal;

pub(crate) fn routes() -> Router<ApiState> {
    Router::new()
        .route("/api/v1/admin/configuration-reset/prepare", post(prepare))
        .route("/api/v1/admin/configuration-reset/status", get(status))
        .route("/api/v1/admin/configuration-reset/complete", post(complete))
        .route("/api/v1/admin/configuration-reset/finalize", post(finalize))
}

/// Merge only into authenticated private routers with an actual owner extension.
pub fn owner_routes() -> Router<ApiState> {
    Router::new()
        .route(owners::OBSERVE_PATH, get(owners::observe))
        .route(owners::CLEAR_PATH, post(owners::clear))
}

fn authority(
    principal: Option<Extension<AuthenticatedPrincipal>>,
) -> Result<ResetAuthority, Failure> {
    let Some(Extension(principal)) = principal else {
        return Err(Failure(
            StatusCode::UNAUTHORIZED,
            "Independent scoped Operator credential required",
        ));
    };
    if principal.purpose() != CredentialPurpose::Operator
        || !principal.permits(Permission::ManageConfiguration)
        || !principal.permits(Permission::MaintainBackend)
    {
        return Err(Failure(
            StatusCode::FORBIDDEN,
            "Independent Operator configuration and maintenance authority required",
        ));
    }
    ResetAuthority::new(principal.scope_id(), principal.credential_id()).map_err(Failure::from)
}
fn coordinator(
    extension: Option<Extension<Arc<Coordinator>>>,
) -> Result<Arc<Coordinator>, Failure> {
    extension.map(|Extension(value)| value).ok_or(Failure(
        StatusCode::SERVICE_UNAVAILABLE,
        "Reset owner is not provisioned",
    ))
}

async fn prepare(
    State(state): State<ApiState>,
    principal: Option<Extension<AuthenticatedPrincipal>>,
    owner: Option<Extension<Arc<Coordinator>>>,
    Json(request): Json<ResetRequest>,
) -> Result<Response, Failure> {
    let authority = authority(principal)?;
    let receipt = coordinator(owner)?
        .prepare(&state.repo, &authority, &request)
        .await?;
    Ok(protected(receipt))
}
async fn status(
    State(state): State<ApiState>,
    principal: Option<Extension<AuthenticatedPrincipal>>,
    owner: Option<Extension<Arc<Coordinator>>>,
    Query(request): Query<ResetRequest>,
) -> Result<Response, Failure> {
    let authority = authority(principal)?;
    let receipt = coordinator(owner)?
        .status(&state.repo, &authority, &request)
        .await?;
    Ok(protected(receipt))
}
async fn complete(
    State(state): State<ApiState>,
    principal: Option<Extension<AuthenticatedPrincipal>>,
    owner: Option<Extension<Arc<Coordinator>>>,
    Json(request): Json<CompletionRequest>,
) -> Result<Response, Failure> {
    let authority = authority(principal)?;
    let receipt = coordinator(owner)?
        .complete(&state.repo, &authority, &request)
        .await?;
    Ok(protected(receipt))
}
async fn finalize(
    State(state): State<ApiState>,
    principal: Option<Extension<AuthenticatedPrincipal>>,
    owner: Option<Extension<Arc<Coordinator>>>,
    Json(request): Json<CompletionRequest>,
) -> Result<Response, Failure> {
    let authority = authority(principal)?;
    let receipt = coordinator(owner)?
        .finalize(&state.repo, &authority, &request)
        .await?;
    Ok(protected(receipt))
}
fn protected(receipt: ResetReceipt) -> Response {
    let mut response = Json(receipt).into_response();
    private_headers(&mut response);
    response
}
fn private_headers(response: &mut Response) {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
}

pub(crate) struct Failure(StatusCode, &'static str);
impl From<ResetError> for Failure {
    fn from(error: ResetError) -> Self {
        let status = match error {
            ResetError::Invalid => StatusCode::BAD_REQUEST,
            ResetError::Forbidden => StatusCode::FORBIDDEN,
            ResetError::Missing => StatusCode::NOT_FOUND,
            ResetError::Conflict | ResetError::UnsafeFiles | ResetError::UnreviewedSources => {
                StatusCode::CONFLICT
            }
            _ => StatusCode::SERVICE_UNAVAILABLE,
        };
        Self(
            status,
            match error {
                ResetError::UnsupportedBuild => {
                    "Full current-only reset support is not installed; no cutover was performed"
                }
                ResetError::Draining => "Credential-consuming work is still draining",
                ResetError::ReprovisioningRequired => {
                    "Actual fresh user policy and private owner epoch reload are required"
                }
                ResetError::Invalid => "Invalid bounded reset identity",
                ResetError::Forbidden => "Reset authority is not permitted",
                ResetError::Missing => "Reset record unavailable",
                ResetError::Conflict | ResetError::UnsafeFiles | ResetError::UnreviewedSources => {
                    "Reset inventory or state requires operator review"
                }
                _ => "Reset authority unavailable",
            },
        )
    }
}
impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let mut response = (self.0, Json(json!({"error":self.1}))).into_response();
        private_headers(&mut response);
        response
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    use super::*;
    use crate::guard_network;
    use crate::NetworkPolicy;
    use crate::ScopedCredential;

    async fn authorized(
        principal: Option<Extension<AuthenticatedPrincipal>>,
    ) -> Result<StatusCode, Failure> {
        authority(principal)?;
        Ok(StatusCode::NO_CONTENT)
    }

    async fn call(policy: NetworkPolicy) -> StatusCode {
        let app = guard_network(
            Router::new().route("/admin-probe", post(authorized)),
            policy,
            Arc::new(Vec::new()),
            8080,
        );
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin-probe")
                .header("host", "reset.test:8080")
                .header(
                    "authorization",
                    "Bearer fixture-reset-operator-token-0123456789abcdef",
                )
                .body(Body::empty())
                .expect("synthetic request"),
        )
        .await
        .expect("finite actual network boundary")
        .status()
    }

    #[tokio::test]
    async fn only_independent_operator_with_both_permissions_passes_actual_network_policy() {
        let token = "fixture-reset-operator-token-0123456789abcdef";
        let hosts = vec!["reset.test:8080".into()];
        for (purpose, permissions, expected) in [
            (
                CredentialPurpose::Operator,
                BTreeSet::from([Permission::ManageConfiguration, Permission::MaintainBackend]),
                StatusCode::NO_CONTENT,
            ),
            (
                CredentialPurpose::Operator,
                BTreeSet::from([Permission::ManageConfiguration]),
                StatusCode::FORBIDDEN,
            ),
            (
                CredentialPurpose::Native,
                BTreeSet::from([Permission::ManageConfiguration, Permission::MaintainBackend]),
                StatusCode::FORBIDDEN,
            ),
            (
                CredentialPurpose::WebRelay,
                BTreeSet::from([Permission::ResolveWebSession]),
                StatusCode::FORBIDDEN,
            ),
            (
                CredentialPurpose::InternalService,
                BTreeSet::from([Permission::ManageConfiguration, Permission::MaintainBackend]),
                StatusCode::FORBIDDEN,
            ),
        ] {
            let credential = ScopedCredential::new(
                "fixture-operator".into(),
                "fixture-scope".into(),
                "fixture-access".into(),
                purpose,
                permissions,
                token,
            )
            .expect("explicit synthetic scoped credential");
            let policy = NetworkPolicy::with_credentials(&hosts, vec![credential])
                .expect("actual scoped policy");
            assert_eq!(call(policy).await, expected);
        }
        assert_eq!(
            call(NetworkPolicy::new(&hosts, token).expect("legacy scoped-less bearer")).await,
            StatusCode::UNAUTHORIZED,
            "a valid legacy bearer supplies no Operator principal"
        );
    }

    #[test]
    fn caller_json_cannot_attest_drain_revocation_or_legacy_retirement() {
        let base = json!({"version":1,"reset_id":"fixture-reset","plan_sha256":"a".repeat(64),"deployment_id":"fixture-deployment","expected_configuration_epoch":"old-config","expected_access_epoch":"old-access"});
        for field in [
            "work_drained",
            "legacy_reload_disabled",
            "user_access_revoked",
            "configuration_root",
            "operator_authorized",
        ] {
            let mut forged = base.clone();
            forged[field] = json!(true);
            assert!(serde_json::from_value::<ResetRequest>(forged).is_err());
        }
        assert!(serde_json::from_value::<ResetRequest>(base).is_ok());
    }
}
