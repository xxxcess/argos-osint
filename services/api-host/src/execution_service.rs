//! Guarded HTTP boundary for one retained model attempt.

use std::path::PathBuf;
use std::sync::Arc;

use argos_osint_core::provider_attempt::remote::{self, ChatAttempt};
use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use tokio::sync::Semaphore;

#[derive(Clone)]
struct Execution {
    workspace: PathBuf,
    slots: Arc<Semaphore>,
}

pub(crate) fn router(workspace: PathBuf) -> Router {
    Router::new()
        .route(remote::READY_PATH, get(ready))
        .route(remote::ATTEMPT_PATH, post(attempt))
        .layer(DefaultBodyLimit::max(remote::MAX_REQUEST_BYTES))
        .with_state(Execution {
            workspace,
            slots: Arc::new(Semaphore::new(8)),
        })
}

async fn ready() -> Json<serde_json::Value> {
    Json(json!({"service": "fleetworks-execution", "api_version": remote::VERSION, "ready": true}))
}

async fn attempt(
    State(execution): State<Execution>,
    payload: Result<Json<ChatAttempt>, JsonRejection>,
) -> Response {
    let request = match payload {
        Ok(Json(request)) if request.validate() => request,
        Err(reason) if reason.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            return error(StatusCode::PAYLOAD_TOO_LARGE, "execution_request_too_large");
        }
        _ => return error(StatusCode::BAD_REQUEST, "execution_invalid_request"),
    };
    let Ok(slot) = execution.slots.try_acquire_owned() else {
        return error(StatusCode::TOO_MANY_REQUESTS, "execution_busy");
    };
    let stream = remote::serve_attempt(execution.workspace, request, slot);
    (
        [(header::CONTENT_TYPE, remote::MEDIA_TYPE)],
        Body::from_stream(stream),
    )
        .into_response()
}

fn error(status: StatusCode, code: &'static str) -> Response {
    (status, Json(json!({"error": code}))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn private_attempt_rejects_unauthorized_and_reports_missing_route() {
        const TOKEN: &str = "01234567890123456789012345678901";
        let workspace = std::env::temp_dir().join(format!(
            "fleetworks-execution-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir(&workspace).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let policy = argos_osint_api::NetworkPolicy::new(&[address.to_string()], TOKEN).unwrap();
        let app = argos_osint_api::guard_network(
            router(workspace.clone()),
            policy,
            Arc::new(Vec::new()),
            address.port(),
        );
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let deadline = json!({"secs": 1, "nanos": 0});
        let payload = json!({
            "version": 1,
            "alias": {
                "role": "atlas_analyst",
                "operation": "intel_brief_claims.generate",
                "index": 0,
                "provider": "local",
                "account": "local",
                "model": "fixture-model",
                "revision": "a".repeat(64),
            },
            "messages": [{"role": "user", "content": "fixture", "tool_call_id": null, "tool_calls": []}],
            "transport": "stream",
            "deadlines": {
                "connect": deadline, "first_response": deadline, "idle": deadline, "total": deadline,
                "first_output": null, "model_inactivity": null,
            },
            "policy": argos_osint_core::request_policy::RequestPolicy::default(),
            "contract": {"kind": "prose"},
            "capabilities": null,
        }).to_string();
        for (authorization, expected) in [
            ("", "HTTP/1.1 401"),
            (
                "Authorization: Bearer 01234567890123456789012345678901\r\n",
                "HTTP/1.1 200",
            ),
        ] {
            let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
            socket.write_all(format!(
                "POST {} HTTP/1.1\r\nHost: {address}\r\n{authorization}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                remote::ATTEMPT_PATH, payload.len()
            ).as_bytes()).await.unwrap();
            let mut response = String::new();
            socket.read_to_string(&mut response).await.unwrap();
            assert!(response.starts_with(expected), "{response}");
            if authorization.is_empty() {
                assert!(!response.contains("execution route"));
            } else {
                assert!(
                    response.contains("execution route unavailable")
                        || response.contains("execution route changed")
                );
            }
        }
        task.abort();
        let _ = task.await;
        std::fs::remove_dir_all(workspace).unwrap();
    }
}
