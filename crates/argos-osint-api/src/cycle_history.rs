//! Typed cycle history reads and workstation association tombstones.
use axum::extract::Query;
use axum::extract::State;
use axum::Extension;
use axum::Json;
use fleetworks_contracts::cycle_history::CycleHistory;
use fleetworks_contracts::cycle_history::DeleteCycleReceipt;
use fleetworks_contracts::cycle_history::DeleteCycleRequest;
use serde::Deserialize;

use crate::request_context::ResolvedRequest;
use crate::ApiError;
use crate::ApiState;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HistoryQuery {
    limit: Option<usize>,
}

pub(crate) async fn list(
    State(state): State<ApiState>,
    Extension(context): Extension<ResolvedRequest>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<CycleHistory>, ApiError> {
    let scope = crate::activity::scope_for_request(&state.repo, &context, None).await?;
    let limit = query.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err(crate::bad());
    }
    let history = argos_osint_core::storage::cycle_history::list(
        &state.repo,
        context.scope_id(),
        &scope,
        limit,
    )
    .await
    .map_err(crate::internal)?;
    finish_history(&state, &context, history).await
}

/// Fence a completed read before exposing its payload after possible revocation.
async fn finish_history(
    state: &ApiState,
    context: &ResolvedRequest,
    history: CycleHistory,
) -> Result<Json<CycleHistory>, ApiError> {
    context.revalidate(&state.repo).await?;
    Ok(Json(history))
}

pub(crate) async fn delete(
    State(state): State<ApiState>,
    Extension(context): Extension<ResolvedRequest>,
    Json(request): Json<DeleteCycleRequest>,
) -> Result<Json<DeleteCycleReceipt>, ApiError> {
    // Unlike collection, hiding a history entry requires workstation activity authority.
    crate::activity::scope_for_request(&state.repo, &context, None).await?;
    let admission = crate::admission::capture_checked(
        &state.repo,
        &context,
        &request.request_id,
        argos_osint_core::storage::cycle_history::DELETE_OPERATION,
        Some(&request.context),
    )
    .await?;
    let receipt =
        argos_osint_core::storage::cycle_history::delete(&state.repo, &admission, &request)
            .await
            .map_err(|_| {
                ApiError(
                    axum::http::StatusCode::CONFLICT,
                    "Cycle history changed or is unavailable; refresh before retrying",
                )
            })?;
    Ok(Json(receipt))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use argos_osint_core::storage::bootstrap;
    use argos_osint_core::storage::connect_registry;
    use argos_osint_core::storage::connect_registry::RegistryAuthority;
    use axum::body::Body;
    use axum::extract::State;
    use axum::http::HeaderMap;
    use axum::http::Request;
    use axum::http::StatusCode;
    use axum::routing::get;
    use axum::Extension;
    use axum::Json;
    use axum::Router;
    use fleetworks_contracts::connect::RegistrationEnsureRequest;
    use fleetworks_contracts::context::CredentialPurpose;
    use fleetworks_contracts::context::Permission;
    use fleetworks_contracts::cycle_history::CycleHistory;
    use tower::ServiceExt;

    use crate::request_context;
    use crate::request_context::ContextRequirement;
    use crate::ApiError;
    use crate::ApiState;
    use crate::AuthenticatedPrincipal;
    use crate::NetworkPolicy;
    use crate::ScopedCredential;

    #[tokio::test]
    async fn completed_cycle_read_is_denied_when_registration_is_revoked_before_response() {
        #[cfg(feature = "embedded")]
        let directory = tempfile::tempdir().expect("isolated history repository");
        #[cfg(feature = "embedded")]
        let repo = argos_osint_core::storage::repo::Repository::connect(
            &argos_osint_core::storage::open::Target::Embedded {
                path: directory.path().join("db"),
            },
            "cycle_api",
            "read_revocation",
        )
        .await
        .expect("embedded history repository");
        #[cfg(not(feature = "embedded"))]
        let server = crate::storage_test_support::Server::start()
            .expect("required isolated storage harness");
        #[cfg(not(feature = "embedded"))]
        let repo = server.repo("cycle_api", "read_revocation").await;
        bootstrap::ensure_network_identity(repo.db())
            .await
            .expect("database identity");
        connect_registry::initialize_control(repo.db())
            .await
            .expect("registry control");
        let authority = RegistryAuthority::new("history-scope", "history-native", "history-access")
            .expect("authority");
        let registration = connect_registry::ensure(
            &repo,
            &authority,
            &RegistrationEnsureRequest::new(
                "ensure-history".into(),
                None,
                "history-installation-marker".into(),
            ),
        )
        .await
        .expect("registration")
        .receipt;
        const TOKEN: &str = "synthetic-history-native-0123456789abcdefghijklmnop";
        let policy = NetworkPolicy::with_credentials(
            &["history.test:8080".into()],
            vec![ScopedCredential::new(
                "history-native".into(),
                "history-scope".into(),
                "history-access".into(),
                CredentialPurpose::Native,
                [Permission::ReadWorkstationActivity].into_iter().collect(),
                TOKEN,
            )
            .expect("native credential")],
        )
        .expect("policy");
        let state = ApiState {
            repo,
            instance_id: "fixture-instance".into(),
            port: 8080,
            origins: Arc::new(vec![]),
        };
        async fn read_then_revoke(
            State(state): State<ApiState>,
            Extension(principal): Extension<AuthenticatedPrincipal>,
            headers: HeaderMap,
        ) -> Result<Json<CycleHistory>, ApiError> {
            let context = request_context::resolve_gateway(
                &state.repo,
                &principal,
                &headers,
                Permission::ReadWorkstationActivity,
                ContextRequirement::Registered,
            )
            .await?;
            let scope = crate::activity::scope_for_request(&state.repo, &context, None).await?;
            let history = argos_osint_core::storage::cycle_history::list(
                &state.repo,
                context.scope_id(),
                &scope,
                20,
            )
            .await
            .map_err(crate::internal)?;
            // Deterministically interleave the completed storage read with revocation.
            // Production and this fixture invoke the same response-boundary fence.
            state
                .repo
                .db()
                .query("UPDATE $registration SET state='revoked',revision+=1 RETURN NONE")
                .bind((
                    "registration",
                    surrealdb::types::RecordId::new(
                        "connect_registration",
                        context.registered()?.registration_id.clone(),
                    ),
                ))
                .await
                .map_err(crate::internal)?
                .check()
                .map_err(crate::internal)?;
            super::finish_history(&state, &context, history).await
        }
        let app = crate::guard_network(
            Router::new()
                .route("/fixture/read-then-revoke", get(read_then_revoke))
                .with_state(state),
            policy,
            Arc::new(vec![]),
            8080,
        );
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/fixture/read-then-revoke")
                    .header("host", "history.test:8080")
                    .header("authorization", format!("Bearer {TOKEN}"))
                    .header(
                        request_context::REGISTRATION_HEADER,
                        registration.registration_id,
                    )
                    .header(
                        request_context::REGISTRATION_REVISION_HEADER,
                        registration.revision.to_string(),
                    )
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .expect("bounded response");
        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON error");
        assert!(
            body.get("rows").is_none(),
            "a completed read must not expose rows after revocation"
        );
    }
}
