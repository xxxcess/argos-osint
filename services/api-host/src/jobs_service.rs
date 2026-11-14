//! Private persistence boundary owned by the Jobs process.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use argos_osint_core::storage::jobs_client;
use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use fleetworks_jobs::queue::{Queue, QueueRequest};
use fleetworks_jobs::{Request, Store};
use serde_json::json;
use tokio::sync::Semaphore;

const MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct Jobs {
    store: Store,
    queue: Queue,
    slots: Arc<Semaphore>,
}

pub(crate) async fn open_queue(workspace: PathBuf) -> anyhow::Result<Queue> {
    let path = std::env::var_os("FLEETWORKS_JOBS_QUEUE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join("storage/jobs/queue"));
    anyhow::ensure!(
        path.is_absolute(),
        "FLEETWORKS_JOBS_QUEUE_PATH must be absolute"
    );
    Queue::open(path)
        .await
        .context("open the Jobs RocksDB queue")
}

pub(crate) async fn open(workspace: PathBuf) -> anyhow::Result<Store> {
    let path = std::env::var_os("FLEETWORKS_JOBS_SQLITE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join("storage/jobs/jobs.sqlite3"));
    anyhow::ensure!(
        path.is_absolute(),
        "FLEETWORKS_JOBS_SQLITE_PATH must be absolute"
    );
    Store::open(path)
        .await
        .context("open the Jobs SQLite datastore")
}

/// The caller applies the private Jobs host and bearer policy to these routes.
pub(crate) fn router(store: Store, queue: Queue) -> Router {
    Router::new()
        .route("/internal/jobs/store", post(command))
        .route("/internal/jobs/logs", post(logs))
        .route("/internal/jobs/queue", post(queue_command))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .with_state(Jobs {
            store,
            queue,
            slots: Arc::new(Semaphore::new(32)),
        })
}

async fn queue_command(
    State(jobs): State<Jobs>,
    payload: Result<Json<QueueRequest>, JsonRejection>,
) -> Response {
    let mut request = match payload {
        Ok(Json(request)) => request,
        Err(reason) => return rejected(reason),
    };
    let Ok(_slot) = jobs.slots.try_acquire() else {
        return error(StatusCode::TOO_MANY_REQUESTS, "jobs_busy");
    };
    request.now_ms = chrono::Utc::now().timestamp_millis();
    match tokio::time::timeout(REQUEST_TIMEOUT, jobs.queue.execute(request)).await {
        Ok(Ok(response)) => Json(response).into_response(),
        Ok(Err(reason)) => {
            tracing::warn!(error = %reason, "Jobs RocksDB queue command failed");
            error(StatusCode::SERVICE_UNAVAILABLE, "jobs_queue_unavailable")
        }
        Err(_) => error(StatusCode::GATEWAY_TIMEOUT, "jobs_queue_timeout"),
    }
}

async fn command(
    State(jobs): State<Jobs>,
    payload: Result<Json<Request>, JsonRejection>,
) -> Response {
    let mut request = match payload {
        Ok(Json(request)) => request,
        Err(reason) => return rejected(reason),
    };
    let Ok(_slot) = jobs.slots.try_acquire() else {
        return error(StatusCode::TOO_MANY_REQUESTS, "jobs_busy");
    };
    // Lease authority comes from the owning service clock, never its clients.
    request.now_ms = chrono::Utc::now().timestamp_millis();
    match tokio::time::timeout(REQUEST_TIMEOUT, jobs.store.execute(request)).await {
        Ok(Ok(response)) => Json(response).into_response(),
        Ok(Err(reason)) => {
            tracing::warn!(error = %reason, "Jobs SQLite command failed");
            error(StatusCode::SERVICE_UNAVAILABLE, "jobs_store_unavailable")
        }
        Err(_) => error(StatusCode::GATEWAY_TIMEOUT, "jobs_store_timeout"),
    }
}

async fn logs(
    State(jobs): State<Jobs>,
    payload: Result<Json<jobs_client::LogCommand>, JsonRejection>,
) -> Response {
    let request = match payload {
        Ok(Json(request)) => request,
        Err(reason) => return rejected(reason),
    };
    let Ok(_slot) = jobs.slots.try_acquire() else {
        return error(StatusCode::TOO_MANY_REQUESTS, "jobs_busy");
    };
    match tokio::time::timeout(
        REQUEST_TIMEOUT,
        jobs_client::execute_local(&jobs.store, request),
    )
    .await
    {
        Ok(Ok(response)) => Json(response).into_response(),
        Ok(Err(reason)) => {
            tracing::warn!(error = %reason, "Jobs SQLite log command failed");
            error(StatusCode::SERVICE_UNAVAILABLE, "jobs_store_unavailable")
        }
        Err(_) => error(StatusCode::GATEWAY_TIMEOUT, "jobs_store_timeout"),
    }
}

fn rejected(reason: JsonRejection) -> Response {
    let status = if reason.status() == StatusCode::PAYLOAD_TOO_LARGE {
        StatusCode::PAYLOAD_TOO_LARGE
    } else {
        StatusCode::BAD_REQUEST
    };
    error(status, "jobs_invalid_request")
}

fn error(status: StatusCode, code: &'static str) -> Response {
    (status, Json(json!({"error": code}))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fleetworks_jobs::queue::{QueueCommand, QueueResponse};
    use fleetworks_jobs::{Command, Response as StoreResponse};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn private_queue_delivery_is_leased_settled_and_retained_on_reopen() {
        const TOKEN: &str = "01234567890123456789012345678901";
        let directory = tempfile::tempdir().unwrap();
        let queue_path = directory.path().join("queue");
        let store = Store::open(directory.path().join("jobs.sqlite3"))
            .await
            .unwrap();
        let queue = Queue::open(&queue_path).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let policy = argos_osint_api::NetworkPolicy::new(&[address.to_string()], TOKEN).unwrap();
        let app = argos_osint_api::guard_network(
            router(store, queue.clone()),
            policy,
            Arc::new(Vec::new()),
            address.port(),
        );
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .unwrap()
        });
        let client =
            crate::queue_client::Client::new(&format!("http://{address}"), TOKEN, "fixture".into())
                .unwrap();
        client
            .publish("first".into(), b"first payload".to_vec())
            .await
            .unwrap();
        // An ambiguous publish retry must not enqueue a duplicate dispatch.
        client
            .publish("first".into(), b"first payload".to_vec())
            .await
            .unwrap();
        let delivery = client.claim("worker-one").await.unwrap().unwrap();
        assert_eq!(delivery.payload(), b"first payload");
        assert!(delivery.renew().await.unwrap());
        assert!(client.claim("worker-two").await.unwrap().is_none());
        delivery
            .settle(crate::queue_client::Disposition::Ack)
            .await
            .unwrap();
        assert!(client.claim("worker-two").await.unwrap().is_none());
        client
            .publish("survives-restart".into(), b"persisted payload".to_vec())
            .await
            .unwrap();
        drop(client);
        stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
        drop(queue);
        let reopened = Queue::open(&queue_path).await.unwrap();
        let response = reopened
            .execute(QueueRequest::new(
                "reopen-claim",
                chrono::Utc::now().timestamp_millis(),
                QueueCommand::Claim {
                    queue: "fixture".into(),
                    owner: "after-restart".into(),
                    visibility_ms: 45_000,
                },
            ))
            .await
            .unwrap();
        assert!(
            matches!(response, QueueResponse::Delivery(Some(ref delivery)) if delivery.payload == b"persisted payload")
        );
    }

    #[tokio::test]
    async fn sqlite_commands_require_the_private_jobs_bearer() {
        const TOKEN: &str = "01234567890123456789012345678901";
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().join("jobs.sqlite3"))
            .await
            .unwrap();
        let queue = Queue::open(directory.path().join("queue")).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let policy = argos_osint_api::NetworkPolicy::new(&[address.to_string()], TOKEN).unwrap();
        let app = argos_osint_api::guard_network(
            router(store.clone(), queue),
            policy,
            Arc::new(Vec::new()),
            address.port(),
        );
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let payload = json!({
            "operation_key": "fixture-submit", "now_ms": 1000,
            "command": {
                "type": "submit",
                "job": {"id": "fixture-job", "parent_id": null, "operation": "fixture"},
                "tasks": [{"id": "fixture-task", "operation": "fixture", "pool": "fixture"}],
            },
        })
        .to_string();
        for (authorization, expected, exists) in [
            ("", "HTTP/1.1 401", false),
            (
                "Authorization: Bearer 01234567890123456789012345678901\r\n",
                "HTTP/1.1 200",
                true,
            ),
        ] {
            let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
            socket
                .write_all(
                    format!(
                        "POST /internal/jobs/store HTTP/1.1\r\nHost: {address}\r\n{authorization}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                        payload.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            let mut response = String::new();
            socket.read_to_string(&mut response).await.unwrap();
            assert!(response.starts_with(expected), "{response}");
            assert!(
                !response.contains(TOKEN),
                "response leaked the private bearer"
            );
            let stored = store
                .execute(Request::new(
                    "",
                    1000,
                    Command::Get {
                        job_id: "fixture-job".into(),
                    },
                ))
                .await
                .unwrap();
            assert!(matches!(stored, StoreResponse::Job(ref job) if job.is_some() == exists));
        }
        let log = json!({"command":"append", "event":{
            "id":"smoke-log", "seq":0, "ts":"2026-11-10T00:00:00Z",
            "severity":"info", "app":"jobs", "event_type":"smoke",
            "message":"persisted", "details":"Authorization: Bearer supersecretvalue123", "job_id":"", "task_id":"",
            "attempt_id":"", "run_id":"", "resource_ref":"", "correlation_id":""
        }})
        .to_string();
        let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
        socket.write_all(format!("POST /internal/jobs/logs HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{log}", log.len()).as_bytes()).await.unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).await.unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert_eq!(
            store.log_get("smoke-log".into()).await.unwrap().unwrap()["message"],
            "persisted"
        );
        let log = store.log_get("smoke-log".into()).await.unwrap().unwrap();
        assert!(!log["details"]
            .as_str()
            .unwrap()
            .contains("supersecretvalue123"));
        task.abort();
        let _ = task.await;
        drop(store);
        let reopened = Store::open(directory.path().join("jobs.sqlite3"))
            .await
            .unwrap();
        assert!(reopened
            .log_get("smoke-log".into())
            .await
            .unwrap()
            .is_some());
    }
}
