//! Private, database-free inference over the existing warm Rust-BERT worker.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use argos_osint_core::classifier::readiness::{ClassifierHome, PinnedClassifier};
use argos_osint_core::classifier::remote::{
    self, InferRequest, InferResponse, PinRequest, PinResponse, ValidateRequest,
};
use argos_osint_core::classifier::{ClassifierError, WorkControl};
use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use tokio::sync::{Mutex, Semaphore};

#[derive(Clone)]
struct Engine {
    home: ClassifierHome,
    warm: Arc<Mutex<Option<WarmPin>>>,
}

const MAX_ACTIVATIONS: usize = 32;

struct WarmPin {
    classifier: PinnedClassifier,
    // Several resumable cycles may name the same bundle with distinct times.
    // Keep a bounded set of exact snapshots accepted by the pin route.
    activations: VecDeque<String>,
}

impl WarmPin {
    fn remember(&mut self, activation: String) {
        self.activations.retain(|saved| saved != &activation);
        self.activations.push_back(activation);
        if self.activations.len() > MAX_ACTIVATIONS {
            self.activations.pop_front();
        }
    }
}

/// Build the internal service router. The host must wrap it in its network
/// guard before serving. Model and worker assets are loaded only by pinning.
pub(crate) fn router(home: ClassifierHome) -> Router {
    let work = Router::new()
        .route(remote::PIN_PATH, post(pin))
        .route(remote::VALIDATE_PATH, post(validate_pin))
        .route(remote::INFER_PATH, post(infer))
        .route_layer(middleware::from_fn_with_state(
            Arc::new(Semaphore::new(1)),
            limit_work,
        ));
    Router::new()
        .route(remote::READY_PATH, get(ready))
        .merge(work)
        .layer(DefaultBodyLimit::max(remote::MAX_REQUEST_BYTES))
        .with_state(Engine {
            home,
            warm: Arc::new(Mutex::new(None)),
        })
}

async fn ready(State(engine): State<Engine>) -> Response {
    let ready = cfg!(feature = "classifier");
    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let warm = engine.warm.lock().await;
    let active_model = warm.as_ref().map(|warm| {
        json!({
            "snapshot": warm.classifier.snapshot,
            "manifest_sha256": warm.classifier.manifest_sha256,
        })
    });
    let model_state = if active_model.is_some() {
        "verified"
    } else {
        "unverified"
    };
    (
        status,
        Json(json!({
            "service": "fleetworks-classifier",
            "api_version": remote::VERSION,
            "ready": ready,
            "model_state": model_state,
            "active_model": active_model,
        })),
    )
        .into_response()
}

async fn limit_work(State(slot): State<Arc<Semaphore>>, request: Request, next: Next) -> Response {
    let Ok(_slot) = slot.try_acquire() else {
        return error(StatusCode::TOO_MANY_REQUESTS, "classifier_busy");
    };
    next.run(request).await
}

async fn pin(
    State(engine): State<Engine>,
    payload: Result<Json<PinRequest>, JsonRejection>,
) -> Response {
    let request = match payload {
        Ok(Json(request)) if request.validate().is_ok() => request,
        Ok(_) => return invalid_request(),
        Err(reason) => return invalid_payload(reason),
    };
    // Local explicitly bypasses environment-selected remote dispatch.
    let pinned = match engine
        .home
        .pin_atlas_local(request.requested.as_ref())
        .await
    {
        Ok(pinned) => pinned,
        Err(reason) => return worker_error(reason),
    };
    let response = match PinResponse::from_local(&pinned) {
        Ok(response) => response,
        Err(reason) => return worker_error(reason),
    };
    let mut warm = engine.warm.lock().await;
    match warm.as_mut() {
        Some(warm) if response.identity.matches_bundle(&warm.classifier) => {
            warm.remember(pinned.snapshot.activated_at);
        }
        _ => {
            *warm = Some(WarmPin {
                activations: VecDeque::from([pinned.snapshot.activated_at.clone()]),
                classifier: pinned,
            });
        }
    }
    Json(response).into_response()
}

async fn validate_pin(
    State(engine): State<Engine>,
    payload: Result<Json<ValidateRequest>, JsonRejection>,
) -> Response {
    let request = match payload {
        Ok(Json(request)) if request.validate().is_ok() => request,
        Ok(_) => return invalid_request(),
        Err(reason) => return invalid_payload(reason),
    };
    match engine
        .home
        .validate_local_saved_pin_integrity(
            &request.identity.snapshot,
            &request.identity.manifest_sha256,
        )
        .await
    {
        Ok(()) => Json(request).into_response(),
        Err(reason) => worker_error(reason),
    }
}

struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

async fn infer(
    State(engine): State<Engine>,
    payload: Result<Json<InferRequest>, JsonRejection>,
) -> Response {
    let request = match payload {
        Ok(Json(request)) if request.validate().is_ok() => request,
        Ok(_) => return invalid_request(),
        Err(reason) => return invalid_payload(reason),
    };
    let cached = engine.warm.lock().await.as_ref().and_then(|warm| {
        (request.identity.matches_bundle(&warm.classifier)
            && warm
                .activations
                .contains(&request.identity.snapshot.activated_at))
        .then(|| warm.classifier.clone())
    });
    let Some(pin) = cached else {
        return error(StatusCode::PRECONDITION_FAILED, "classifier_pin_required");
    };
    let cancel = CancelOnDrop(Arc::new(AtomicBool::new(false)));
    let control = WorkControl {
        cancel: cancel.0.clone(),
        // Bound one native microbatch and one possible worker restart.
        overall_deadline: Instant::now() + Duration::from_secs(75),
        ..WorkControl::new()
    };
    let InferRequest {
        identity, inputs, ..
    } = request;
    match tokio::task::spawn_blocking(move || {
        pin.service.classify_batch_with_control(inputs, &control)
    })
    .await
    {
        Ok(Ok(receipts)) => Json(InferResponse {
            version: remote::VERSION,
            identity,
            receipts,
        })
        .into_response(),
        Ok(Err(reason)) => worker_error(reason),
        Err(_) => error(StatusCode::SERVICE_UNAVAILABLE, "classifier_unavailable"),
    }
}

fn invalid_payload(reason: JsonRejection) -> Response {
    if reason.status() == StatusCode::PAYLOAD_TOO_LARGE {
        error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "classifier_payload_too_large",
        )
    } else {
        invalid_request()
    }
}

fn invalid_request() -> Response {
    error(StatusCode::BAD_REQUEST, "classifier_invalid_request")
}

fn worker_error(reason: ClassifierError) -> Response {
    match reason {
        ClassifierError::Busy => error(StatusCode::TOO_MANY_REQUESTS, "classifier_busy"),
        ClassifierError::Timeout | ClassifierError::Cancelled => {
            error(StatusCode::GATEWAY_TIMEOUT, "classifier_timeout")
        }
        ClassifierError::Incompatible(_) => error(StatusCode::CONFLICT, "classifier_pin_mismatch"),
        ClassifierError::MissingZeroShotConfig(_) => error(
            StatusCode::FAILED_DEPENDENCY,
            "classifier_model_not_configured",
        ),
        _ => error(StatusCode::SERVICE_UNAVAILABLE, "classifier_unavailable"),
    }
}

fn error(status: StatusCode, code: &'static str) -> Response {
    (status, Json(json!({"error": code}))).into_response()
}

#[cfg(all(test, feature = "classifier", unix))]
mod tests {
    use super::*;
    use argos_osint_core::classifier::{ClassifyInput, RejectionReason};
    use std::fs;
    use std::path::{Path, PathBuf};

    const TOKEN: &str = "00001111222233334444555566667777";

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let home = std::env::temp_dir().join(format!(
                "fleetworks-classifier-{}-{}",
                std::process::id(),
                chrono::Utc::now().timestamp_nanos_opt().unwrap()
            ));
            fs::create_dir(&home).unwrap();
            let fixture = Self(home.canonicalize().unwrap());
            let assets = fixture.0.join("model");
            fs::create_dir(&assets).unwrap();
            fs::write(
                assets.join("config.json"),
                br#"{"routes":{"policy":[0.95,0.9,0.1,0.1,0.1,0.1]}}"#,
            )
            .unwrap();
            fs::write(assets.join("vocab.txt"), "fixture-vocab").unwrap();
            fs::write(assets.join("rust_model.ot"), "fixture-model").unwrap();
            let classifier = fixture.0.join("classifier");
            fs::create_dir(&classifier).unwrap();
            let worker = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../scripts/fixtures/atlas_bert")
                .canonicalize()
                .unwrap();
            fs::write(
                classifier.join("zero-shot.json"),
                serde_json::to_vec(&json!({
                    "worker_path": worker,
                    "model_dir": assets,
                }))
                .unwrap(),
            )
            .unwrap();
            fixture
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn authenticated_batch_uses_exact_warm_pin() {
        let fixture = Fixture::new();
        let home = ClassifierHome::new(fixture.0.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let origin = format!("http://{address}");
        let policy = argos_osint_api::NetworkPolicy::new(&[address.to_string()], TOKEN).unwrap();
        let app = argos_osint_api::guard_network(
            router(home.clone()),
            policy,
            Arc::new(Vec::new()),
            address.port(),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let wrong = Arc::new(remote::Client::new(&origin, &"x".repeat(32)).unwrap());
        assert!(
            matches!(wrong.pin(None).await, Err(ClassifierError::BridgeProtocol { code, .. }) if code == "remote_classifier_authentication")
        );
        let client = Arc::new(remote::Client::new(&origin, TOKEN).unwrap());
        let pinned = client.pin(None).await.unwrap();
        let mut saved = pinned.snapshot.clone();
        saved.activated_at = "2026-01-01T00:00:00+00:00".into();
        let exact = client.pin(Some(&saved)).await.unwrap();
        assert_eq!(saved, exact.snapshot);
        assert_eq!(pinned.manifest_sha256, exact.manifest_sha256);

        // Inference retains the verified model while later pins recheck files.
        fs::write(fixture.0.join("model/rust_model.ot"), "changed-model").unwrap();
        let first = ClassifyInput::new("first", "National policy", "Article", None);
        let second = ClassifyInput::new("second", "Other event", "Article", None);
        let receipts = tokio::task::spawn_blocking(move || {
            pinned.service.classify_batch(vec![first, second]).unwrap()
        })
        .await
        .unwrap();
        assert_eq!(receipts[0].occurrence_id, "first");
        assert_eq!(receipts[0].selected_domains.len(), 2);
        assert_eq!(receipts[1].occurrence_id, "second");
        assert_eq!(
            receipts[1].rejection_reason,
            Some(RejectionReason::OutOfScope)
        );
        assert!(matches!(
            client.pin(Some(&exact.snapshot)).await,
            Err(ClassifierError::Incompatible(_))
        ));
        server.abort();
        let _ = server.await;
        home.shutdown().await.unwrap();
    }
}
