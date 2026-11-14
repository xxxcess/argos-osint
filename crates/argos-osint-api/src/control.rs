//! Global configuration service and authenticated gateway relay.

use axum::extract::Request;
use axum::extract::State;
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::get;
use axum::routing::post;
use axum::Router;

use crate::configuration;
use crate::private_relay::PrivateClient;
use crate::private_relay::Service;
use crate::ApiResult;
use crate::ApiState;

/// The retained embedded operator adapter is retired only after native parity.
pub(crate) fn read_routes() -> Router<ApiState> {
    Router::new()
        .route("/api/v1/profile", get(crate::operations::profile))
        .route(
            "/api/v1/models/defaults",
            get(crate::operations::model_defaults),
        )
}

pub(crate) fn routes() -> Router<ApiState> {
    Router::new()
        .route("/api/v1/profile", get(configuration::profile))
        .route("/api/v1/models/defaults", get(configuration::models))
        .route(
            "/api/v1/profile/accounts/upsert",
            post(configuration::upsert_account),
        )
        .route(
            "/api/v1/profile/accounts/delete",
            post(configuration::delete_account),
        )
        .route(
            "/api/v1/profile/tools/set",
            post(configuration::save_tool_credential),
        )
        .route(
            "/api/v1/models/defaults/set",
            post(configuration::save_role),
        )
        .route("/api/v1/profile/export", post(configuration::export))
        .route(
            "/api/v1/profile/import",
            post(configuration::import)
                .layer(axum::extract::DefaultBodyLimit::max(2 * 1024 * 1024)),
        )
}

pub(crate) async fn ready(State(state): State<ApiState>) -> ApiResult {
    crate::private_ready(state, "fleetworks-control").await
}

/// Authenticated connection to the private global Control process.
#[derive(Clone)]
pub struct ControlClient(PrivateClient);

impl ControlClient {
    pub fn new(origin: &str, token: &str) -> anyhow::Result<Self> {
        Ok(Self(PrivateClient::new(origin, token, Service::Control)?))
    }
}

pub(crate) async fn forward(
    State(client): State<ControlClient>,
    request: Request,
    _next: Next,
) -> Result<Response, Response> {
    client.0.forward(request).await
}
