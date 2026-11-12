//! Private Jobs REST routes. Live subscriptions stay at the gateway.

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use fleetworks_contracts::{JOBS_PATH, JOB_CANCEL_PATH, JOB_DETAIL_PATH, JOB_RETRY_PATH};

use crate::private_relay::{PrivateClient, Service};
use crate::{ApiResult, ApiState};

pub(crate) fn read_and_retry_routes() -> Router<ApiState> {
    Router::new()
        .route(JOBS_PATH, get(crate::jobs))
        .route(JOB_DETAIL_PATH, get(crate::job))
        .route(JOB_RETRY_PATH, post(crate::retry))
        .route("/api/v1/logs", get(crate::operations::logs))
}

pub(crate) fn routes() -> Router<ApiState> {
    Router::new()
        .merge(crate::activity_paging::routes())
        .route(JOB_DETAIL_PATH, get(crate::activity::job))
        .route(JOB_RETRY_PATH, post(crate::activity_actions::retry))
        .route(JOB_CANCEL_PATH, post(crate::activity_actions::cancel))
        .merge(crate::operational_logs::routes())
}

pub(crate) async fn ready(State(state): State<ApiState>) -> ApiResult {
    if argos_osint_core::storage::jobs_client::configured() {
        argos_osint_core::storage::jobs_client::execute(
            argos_osint_core::storage::jobs_client::LogCommand::Ready,
        )
        .await
        .map_err(crate::internal)?;
    }
    crate::private_ready(state, "fleetworks-jobs").await
}

/// Authenticated connection to the private Jobs process.
#[derive(Clone)]
pub struct JobsClient(PrivateClient);

impl JobsClient {
    /// Configures an exact HTTP(S) origin and its dedicated bearer token.
    pub fn new(origin: &str, token: &str) -> anyhow::Result<Self> {
        Ok(Self(PrivateClient::new(origin, token, Service::Jobs)?))
    }
}

pub(crate) async fn forward(
    State(client): State<JobsClient>,
    request: Request,
    _next: Next,
) -> Result<Response, Response> {
    client.0.forward(request).await
}
