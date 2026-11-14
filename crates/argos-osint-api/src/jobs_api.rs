//! Private Jobs REST routes. Live subscriptions stay at the gateway.

use axum::extract::{Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
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

/// Keep bounded Jobs reads alive after a client disconnect or the outer HTTP
/// deadline so their in-flight SurrealDB replies can still be drained.
pub(crate) async fn drain_read_on_disconnect(request: Request, next: Next) -> Response {
    // Ready shields its own storage probe, and follow shields each authority
    // read. Nesting would consume two permits from the shared bounded pool.
    let already_shielded = matches!(
        request.uri().path(),
        "/api/v1/health/ready" | "/api/v1/logs/follow"
    );
    if matches!(request.method(), &Method::GET | &Method::HEAD) && !already_shielded {
        match crate::run_shielded(async move { next.run(request).await }).await {
            Ok(response) => response,
            Err(error) => error.into_response(),
        }
    } else {
        // Mutations retain normal cancellation and explicit operation-key
        // replay semantics. This layer must never retry an ambiguous write.
        next.run(request).await
    }
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use axum::body::Body;
    use axum::http::StatusCode;
    use axum::middleware;
    use tokio::sync::Notify;
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn jobs_read_finishes_after_client_disconnect() {
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let finished = Arc::new(Notify::new());
        let app = Router::new()
            .route(
                JOBS_PATH,
                get({
                    let started = started.clone();
                    let release = release.clone();
                    let finished = finished.clone();
                    move || {
                        let started = started.clone();
                        let release = release.clone();
                        let finished = finished.clone();
                        async move {
                            started.notify_one();
                            release.notified().await;
                            finished.notify_one();
                            StatusCode::OK
                        }
                    }
                }),
            )
            .layer(middleware::from_fn(drain_read_on_disconnect));
        let request = Request::builder()
            .uri(JOBS_PATH)
            .body(Body::empty())
            .expect("valid Jobs read request");
        let client = tokio::spawn(async move { app.oneshot(request).await });
        tokio::time::timeout(Duration::from_secs(2), started.notified())
            .await
            .expect("Jobs handler started");
        client.abort();
        assert!(client
            .await
            .expect_err("client task was aborted")
            .is_cancelled());
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(2), finished.notified())
            .await
            .expect("in-flight Jobs read drained after disconnect");
    }
}
