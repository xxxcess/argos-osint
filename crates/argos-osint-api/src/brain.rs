//! Private Brain and Intel HTTP operations and the public API relay.

use std::time::Duration;

use anyhow::ensure;
use axum::body::Bytes;
use axum::extract::{FromRequest, Request, State};
use axum::http::header;
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use url::Url;

use crate::network;
use crate::{ApiError, ApiResult, ApiState};

const MAX_QUERY_BYTES: usize = 8 * 1024;
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

pub(crate) fn routes() -> Router<ApiState> {
    Router::new()
        .route("/api/v1/actors/detail", get(crate::actor_dossier::read))
        .route("/api/v1/articles/detail", get(crate::detail))
        .route("/api/v1/articles/related", get(crate::related))
        .route("/api/v1/brief/ensure", post(crate::ensure))
        .route("/api/v1/brief/reload", post(crate::reload))
        .route("/api/v1/brief/stages", get(crate::stages))
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
        )
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
pub struct BrainClient {
    origin: Url,
    authorization: HeaderValue,
    http: reqwest::Client,
}

impl BrainClient {
    /// Configures an exact HTTP(S) origin and a dedicated internal bearer token.
    pub fn new(origin: &str, token: &str) -> anyhow::Result<Self> {
        let origin = network::validate_origin(origin)?;
        ensure!(
            network::valid_token(token.as_bytes()),
            "invalid Brain bearer token"
        );
        let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))?;
        authorization.set_sensitive(true);
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(50))
            .build()?;
        Ok(Self {
            origin,
            authorization,
            http,
        })
    }
}

pub(crate) async fn forward(
    State(client): State<BrainClient>,
    request: Request,
    _next: Next,
) -> Result<Response, Response> {
    // The router selects the operation. Only the path and query from the
    // request are applied to the configured origin.
    let uri = request.uri();
    if uri
        .query()
        .is_some_and(|query| query.len() > MAX_QUERY_BYTES)
    {
        return Err(ApiError(
            StatusCode::URI_TOO_LONG,
            "Brain query exceeds the size limit",
        )
        .into_response());
    }
    let mut url = client.origin.clone();
    url.set_path(uri.path());
    url.set_query(uri.query());
    let method = request.method().clone();
    let content_type = request.headers().get(header::CONTENT_TYPE).cloned();
    let payload = Bytes::from_request(request, &())
        .await
        .map_err(IntoResponse::into_response)?;
    // Public Authorization, Host, Origin and cookies never leave the gateway.
    let mut upstream = client
        .http
        .request(method, url)
        .header(header::AUTHORIZATION, client.authorization)
        .body(payload);
    if let Some(value) = content_type {
        upstream = upstream.header(header::CONTENT_TYPE, value);
    }
    let mut upstream = upstream
        .send()
        .await
        .map_err(|_| unavailable("request failed"))?;
    let status = upstream.status();
    if status.is_redirection() || matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
    {
        return Err(unavailable("Brain service refused the internal request"));
    }
    let content_type = upstream.headers().get(header::CONTENT_TYPE).cloned();
    let mut bytes = Vec::new();
    while let Some(chunk) = upstream
        .chunk()
        .await
        .map_err(|_| unavailable("response interrupted"))?
    {
        if chunk.len() > MAX_RESPONSE_BYTES - bytes.len() {
            return Err(unavailable("Brain response exceeds the size limit"));
        }
        bytes.extend_from_slice(&chunk);
    }
    let mut response = (status, bytes).into_response();
    if let Some(value) = content_type {
        response.headers_mut().insert(header::CONTENT_TYPE, value);
    }
    Ok(response)
}

fn unavailable(reason: &'static str) -> Response {
    eprintln!("Brain service relay failed: {reason}");
    ApiError(StatusCode::BAD_GATEWAY, "Brain service unavailable").into_response()
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use axum::middleware;
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
        let gateway = Router::new()
            .route(PATH, post(|| async { StatusCode::IM_A_TEAPOT }))
            .route_layer(middleware::from_fn_with_state(client, forward));
        let request = Request::post(format!("{PATH}?{QUERY}"))
            .header(header::AUTHORIZATION, "Bearer public-credential")
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
