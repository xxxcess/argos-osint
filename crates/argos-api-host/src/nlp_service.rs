//! Private, database-free NLP routes over the warm Compromise worker.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::ensure;
use argos_osint_core::nlp::remote::{self, BatchRequest, RemoteError};
use argos_osint_core::nlp::Service;
use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use tokio::sync::Semaphore;

const MAX_ACTIVE_BATCHES: usize = 4;

/// Warms the existing worker. Call on a blocking thread before serving.
pub(crate) fn router() -> anyhow::Result<Router> {
    ensure!(
        cfg!(feature = "nlp"),
        "this build does not include the NLP engine"
    );
    let service = Service::shared();
    ensure!(service.readiness.is_ok(), "could not warm the NLP engine");
    let annotations = Router::new()
        .route(remote::ANNOTATE_PATH, post(annotate))
        .route_layer(middleware::from_fn_with_state(
            Arc::new(Semaphore::new(MAX_ACTIVE_BATCHES)),
            limit_batches,
        ));
    Ok(Router::new()
        .route(remote::READY_PATH, get(ready))
        .merge(annotations)
        .layer(DefaultBodyLimit::max(remote::MAX_REQUEST_BYTES))
        .with_state(service))
}

async fn ready() -> Json<serde_json::Value> {
    Json(json!({"service": "fleetworks-nlp", "api_version": remote::VERSION, "ready": true}))
}

async fn limit_batches(
    State(slots): State<Arc<Semaphore>>,
    request: Request,
    next: Next,
) -> Response {
    let Ok(_slot) = slots.try_acquire() else {
        return error(StatusCode::TOO_MANY_REQUESTS, "nlp_busy");
    };
    next.run(request).await
}

struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

async fn annotate(
    State(service): State<&'static Service>,
    payload: Result<Json<BatchRequest>, JsonRejection>,
) -> Response {
    let request = match payload {
        Ok(Json(request)) => request,
        Err(reason) if reason.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            return error(StatusCode::PAYLOAD_TOO_LARGE, "nlp_payload_too_large");
        }
        Err(_) => return error(StatusCode::BAD_REQUEST, "nlp_invalid_request"),
    };
    let cancel = CancelOnDrop(Arc::new(AtomicBool::new(false)));
    let batch = match request.annotate(service, cancel.0.clone()).await {
        Ok(batch) => batch,
        Err(RemoteError::Input) => return error(StatusCode::BAD_REQUEST, "nlp_invalid_request"),
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "nlp_unavailable"),
    };
    let Ok(bytes) = serde_json::to_vec(&batch) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "nlp_unavailable");
    };
    if bytes.len() > remote::MAX_RESPONSE_BYTES {
        return error(StatusCode::SERVICE_UNAVAILABLE, "nlp_response_too_large");
    }
    ([(header::CONTENT_TYPE, "application/json")], bytes).into_response()
}

fn error(status: StatusCode, code: &'static str) -> Response {
    (status, Json(json!({"error": code}))).into_response()
}

#[cfg(all(test, feature = "nlp"))]
mod tests {
    use super::*;
    use argos_osint_core::nlp::Source;

    #[tokio::test]
    async fn batch_preserves_order_and_uses_the_warm_engine_cache() {
        let app = tokio::task::spawn_blocking(router).await.unwrap().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async { axum::serve(listener, app).await.unwrap() });
        let client = remote::Client::new(&origin, "00001111222233334444555566667777").unwrap();
        let first = Source::derived("first", "1", "Alice Smith arrived in Denver.");
        let second = Source::derived("second", "1", "Bob Jones visited Chicago.");
        let cancel = AtomicBool::new(false);
        let annotations = client.annotate(&[&first, &second], &cancel).await.unwrap();
        for (annotation, source) in annotations.iter().zip([&first, &second]) {
            assert_eq!(annotation.status, argos_osint_core::nlp::Status::Complete);
            assert!(annotation.validate(source));
        }
        let cached = client.annotate(&[&second], &cancel).await.unwrap();
        assert!(cached[0].metrics.cache_hit);
        server.abort();
        let _ = server.await;
    }
}
