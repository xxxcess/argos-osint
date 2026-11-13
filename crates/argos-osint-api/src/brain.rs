//! Private Brain and Intel HTTP operations and the public API relay.

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

use crate::private_relay::{PrivateClient, Service};
use crate::{ApiResult, ApiState};

pub(crate) fn routes() -> Router<ApiState> {
    build_routes(true)
}

pub(crate) fn operator_routes() -> Router<ApiState> {
    build_routes(false)
}

fn build_routes(scoped: bool) -> Router<ApiState> {
    let router = Router::new()
        .route("/api/v1/actors/detail", get(crate::actor_dossier::read))
        .route("/api/v1/actors/search", get(crate::actor_dossier::search))
        .route("/api/v1/articles/detail", get(crate::detail))
        .route("/api/v1/articles/related", get(crate::related))
        .route("/api/v1/brief/ensure", post(crate::ensure))
        .route("/api/v1/brief/reload", post(crate::reload))
        .route(
            "/api/v1/topology",
            get(crate::topology_web::read).post(crate::topology),
        )
        .route(
            "/api/v1/topology/resolve",
            post(crate::topology_web::resolve),
        )
        .route(
            "/api/v1/topology/summary",
            get(crate::topology_summary::read),
        )
        .route(
            "/api/v1/topology/summary/generate",
            post(crate::topology_summary::generate),
        );
    if scoped {
        router.route("/api/v1/brief/stages", get(crate::activity::stages))
    } else {
        router.route("/api/v1/brief/stages", get(crate::stages))
    }
}

pub(crate) async fn ready(State(state): State<ApiState>) -> ApiResult {
    crate::storage_ready(&state.repo).await?;
    Ok(Json(json!({
        "service": "fleetworks-brain", "api_version": 1, "ready": true,
        "storage": true, "instance_id": state.instance_id,
    })))
}

/// An authenticated connection pool to a single private Brain service.
#[derive(Clone)]
pub struct BrainClient(PrivateClient);

impl BrainClient {
    /// Configures an exact HTTP(S) origin and a dedicated internal bearer token.
    pub fn new(origin: &str, token: &str) -> anyhow::Result<Self> {
        Ok(Self(PrivateClient::new(origin, token, Service::Brain)?))
    }
}

pub(crate) async fn forward(
    State(client): State<BrainClient>,
    request: Request,
    next: Next,
) -> Result<Response, Response> {
    if request.uri().path() == "/api/v1/actors/search"
        && request.uri().query().is_some_and(|query| {
            let modes: Vec<_> = url::form_urlencoded::parse(query.as_bytes())
                .filter(|(key, _)| key == "mode")
                .collect();
            modes.len() == 1 && modes[0].1 == "discovery"
        })
    {
        return Ok(next.run(request).await);
    }
    client.0.forward(request).await
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt;

    use super::*;

    const TOKEN: &str = "brain-private-test-credential-123456789";

    #[test]
    fn rejects_origins_that_could_send_the_bearer_outside_the_configured_service() {
        for origin in [
            "file:///tmp/brain",
            "http://user:password@brain:8080",
            "http://brain:8080/api",
            "http://brain:8080/?query=1",
            "http://brain:8080/#fragment",
        ] {
            assert!(BrainClient::new(origin, TOKEN).is_err(), "{origin}");
        }
        assert!(BrainClient::new("http://brain:8080", "short-token").is_err());
    }

    #[tokio::test]
    async fn relay_replaces_public_credentials_and_preserves_the_brief_contract() {
        const PATH: &str = "/api/v1/brief/reload";
        const QUERY: &str = "id=alpha%2Fbeta%26gamma";
        const PAYLOAD: &str = r#"{"run":"run-1","id":"story-1","request_id":"test-reload"}"#;
        const PUBLIC_TOKEN: &str = "public-native-test-credential-0123456789";
        use argos_osint_core::storage::connect_registry;
        use argos_osint_core::storage::connect_registry::RegistryAuthority;
        #[cfg(feature = "embedded")]
        use argos_osint_core::storage::repo::Repository;
        use fleetworks_contracts::connect::RegistrationEnsureRequest;
        use fleetworks_contracts::context::CredentialPurpose;
        use fleetworks_contracts::context::Permission;

        #[cfg(not(feature = "embedded"))]
        let datastore = crate::storage_test_support::Server::start()
            .expect("server-only API fixture requires the pinned isolated SurrealDB harness");
        #[cfg(not(feature = "embedded"))]
        let repo = datastore.repo("argos", "api_brief_relay_context").await;
        #[cfg(feature = "embedded")]
        let directory = tempfile::tempdir().expect("isolated embedded fixture");
        #[cfg(feature = "embedded")]
        let repo = Repository::connect(
            &argos_osint_core::storage::open::Target::Embedded {
                path: directory.path().join("store"),
            },
            "argos",
            "api_brief_relay_context",
        )
        .await
        .expect("actual embedded repository");
        argos_osint_core::storage::bootstrap::ensure_network_identity(repo.db())
            .await
            .expect("actual deployment identity");
        connect_registry::initialize_control(repo.db())
            .await
            .expect("explicit fixture registry");
        let authority = RegistryAuthority::new("relay-scope", "relay-native", "relay-access")
            .expect("fixture authority");
        let receipt = connect_registry::ensure(
            &repo,
            &authority,
            &RegistrationEnsureRequest::new(
                "relay-ensure".into(),
                Some("192.0.2.42".parse().expect("synthetic native IP")),
                "synthetic-relay-installation".into(),
            ),
        )
        .await
        .expect("actual registration")
        .receipt;

        let upstream = Router::new().route(
            PATH,
            post(|request: axum::extract::Request| async move {
                assert_eq!(request.uri().query(), Some(QUERY));
                assert_eq!(
                    request.headers()[header::AUTHORIZATION],
                    format!("Bearer {TOKEN}").as_str()
                );
                assert_eq!(request.headers()[header::CONTENT_TYPE], "application/json");
                assert!(!request.headers().contains_key(header::ORIGIN));
                assert!(!request.headers().contains_key(header::COOKIE));
                assert_ne!(
                    request.headers()[crate::request_context::DELEGATION_HEADER],
                    "forged"
                );
                assert_eq!(
                    to_bytes(request.into_body(), crate::MAX_REQUEST_BYTES)
                        .await
                        .unwrap(),
                    PAYLOAD
                );
                (
                    StatusCode::CONFLICT,
                    Json(json!({"error":"fixture conflict"})),
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let client = BrainClient::new(&origin, TOKEN).unwrap();
        let server = tokio::spawn(async { axum::serve(listener, upstream).await.unwrap() });
        let policy = crate::NetworkPolicy::with_credentials(
            &["127.0.0.1:8080".into()],
            vec![crate::ScopedCredential::new(
                "relay-native".into(),
                "relay-scope".into(),
                "relay-access".into(),
                CredentialPurpose::Native,
                [Permission::SubmitInteractive].into_iter().collect(),
                PUBLIC_TOKEN,
            )
            .expect("actual native ingress authority")],
        )
        .expect("fixture policy");
        let gateway = crate::network_router_with_private_services(
            crate::ApiState {
                repo,
                instance_id: "relay-fixture".into(),
                port: 8080,
                origins: std::sync::Arc::new(vec!["https://public.example".into()]),
            },
            policy,
            crate::PrivateServices::new(Some(client), None, None, None),
        );
        let request = Request::post(format!("{PATH}?{QUERY}"))
            .header(header::HOST, "127.0.0.1:8080")
            .header(header::AUTHORIZATION, format!("Bearer {PUBLIC_TOKEN}"))
            .header(
                crate::request_context::REGISTRATION_HEADER,
                &receipt.registration_id,
            )
            .header(
                crate::request_context::REGISTRATION_REVISION_HEADER,
                receipt.revision.to_string(),
            )
            .header(crate::request_context::DELEGATION_HEADER, "forged")
            .header(header::ORIGIN, "https://public.example")
            .header(header::COOKIE, "session=public-credential")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(PAYLOAD))
            .unwrap();
        let response = gateway.oneshot(request).await.unwrap();
        server.abort();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
            json!({"error":"fixture conflict"})
        );
    }
}
