//! Bounded private HTTP transport. No browser or public gateway token can write.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::DefaultBodyLimit;
use axum::extract::Request;
use axum::extract::State;
use axum::http::header;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::middleware;
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use axum::routing::post;
use axum::Extension;
use axum::Json;
use axum::Router;
use fleetworks_contracts::actor_index::ActorIndexBatch;
use fleetworks_contracts::actor_index::ActorIndexPromote;
use fleetworks_contracts::actor_index::ActorIndexQuery;
use fleetworks_contracts::actor_index::ActorIndexRebuildStart;
use fleetworks_contracts::actor_index::ActorIndexRepair;
use fleetworks_contracts::actor_index::ActorIndexRetire;
use fleetworks_contracts::actor_index::MAX_BATCH_BYTES;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sha2::Digest;
use subtle::ConstantTimeEq;
use tokio::sync::OwnedSemaphorePermit;

use crate::config;
use crate::config::Config;
use crate::engine::Engine;
pub use crate::service::EngineSlot;
pub use crate::service::Pace;
use crate::service::SearchService;
use crate::service::ServiceError;
use crate::IndexError;

const BATCH_PATH: &str = "/internal/v1/actors/batches";
const QUERY_PATH: &str = "/internal/v1/actors/query";
const STATUS_PATH: &str = "/internal/v1/actors/status";
const START_PATH: &str = "/internal/v1/actors/rebuild/start";
const PROMOTE_PATH: &str = "/internal/v1/actors/rebuild/promote";
const REPAIR_PATH: &str = "/internal/v1/actors/rebuild/repair";
const RETIRE_PATH: &str = "/internal/v1/actors/generations/retire";
const CONTROL_BODY_LIMIT: usize = 4096;

#[derive(Clone)]
struct HttpState {
    config: Arc<Config>,
    service: SearchService,
}

pub fn router(
    config: Config,
    engine: Option<Arc<Engine>>,
    unavailable: &'static str,
) -> Result<Router, IndexError> {
    router_with_slot(config, Arc::new(EngineSlot::new(engine)), unavailable)
}

pub fn router_with_slot(
    config: Config,
    engine: Arc<EngineSlot>,
    unavailable: &'static str,
) -> Result<Router, IndexError> {
    let service = SearchService::from_config(&config, engine, unavailable)?;
    Ok(router_with_service(config, service))
}

pub fn router_with_service(config: Config, service: SearchService) -> Router {
    let state = HttpState {
        config: Arc::new(config),
        service,
    };
    Router::new()
        .route(STATUS_PATH, get(status))
        .route(
            QUERY_PATH,
            post(query).layer(DefaultBodyLimit::max(CONTROL_BODY_LIMIT)),
        )
        .route(BATCH_PATH, post(batch))
        .route(START_PATH, post(start))
        .route(PROMOTE_PATH, post(promote))
        .route(REPAIR_PATH, post(repair))
        .route(RETIRE_PATH, post(retire))
        .layer(DefaultBodyLimit::max(MAX_BATCH_BYTES))
        .layer(middleware::from_fn_with_state(state.clone(), ingress))
        .with_state(state)
}

async fn ingress(State(state): State<HttpState>, mut request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok());
    if !host.is_some_and(|host| {
        state
            .config
            .allowed_hosts
            .contains(&host.to_ascii_lowercase())
    }) {
        return failure(StatusCode::BAD_REQUEST, "invalid_host");
    }
    if request.headers().contains_key(header::ORIGIN) {
        return failure(StatusCode::FORBIDDEN, "browser_origin_not_allowed");
    }
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| config::valid_token(value.as_bytes()));
    let Some(token) = token else {
        return failure(StatusCode::UNAUTHORIZED, "private_credential_required");
    };
    let digest: [u8; 32] = sha2::Sha256::digest(token.as_bytes()).into();
    let write = bool::from(digest.ct_eq(&state.config.write_digest));
    let read = bool::from(digest.ct_eq(&state.config.read_digest));
    let mutating = matches!(
        request.uri().path(),
        BATCH_PATH | START_PATH | PROMOTE_PATH | REPAIR_PATH | RETIRE_PATH
    );
    if (mutating || !read) && !write {
        return failure(StatusCode::FORBIDDEN, "private_permission_denied");
    }
    let Ok(_permit) = state.service.try_acquire_request() else {
        return service_failure(ServiceError::busy("request_capacity"));
    };
    if mutating {
        let Ok(permit) = state.service.try_acquire_write() else {
            return service_failure(ServiceError::busy("writer_busy"));
        };
        // Reserve before buffering a batch. The handler moves this same permit
        // into its blocking task, so an uncertain HTTP timeout cannot admit a
        // competing write while the original commit is still running.
        request.extensions_mut().insert(Arc::new(permit));
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn status(State(state): State<HttpState>) -> Response {
    result_response(state.service.status_reserved().await)
}

async fn query(State(state): State<HttpState>, body: Bytes) -> Response {
    let request: ActorIndexQuery = match decode_bounded(&body, "query_body_limit", "invalid_json") {
        Ok(value) => value,
        Err(response) => return *response,
    };
    result_response(state.service.query_reserved(&request).await)
}

async fn batch(
    State(state): State<HttpState>,
    Extension(permit): Extension<Arc<OwnedSemaphorePermit>>,
    body: Bytes,
) -> Response {
    let raw_len = body.len();
    if raw_len > state.service.limits().batch_bytes as usize {
        return failure(StatusCode::PAYLOAD_TOO_LARGE, "configured_batch_byte_limit");
    }
    let batch = match tokio::task::spawn_blocking(move || {
        serde_json::from_slice::<ActorIndexBatch>(&body)
            .map_err(|_| ServiceError::invalid("invalid_batch_json"))
    })
    .await
    {
        Ok(Ok(batch)) => batch,
        Ok(Err(error)) => return service_failure(error),
        Err(_) => return service_failure(ServiceError::busy("batch_decode_unavailable")),
    };
    result_response(
        state
            .service
            .apply_batch_reserved(permit, &batch, Some(raw_len))
            .await,
    )
}

async fn start(
    State(state): State<HttpState>,
    Extension(permit): Extension<Arc<OwnedSemaphorePermit>>,
    body: Bytes,
) -> Response {
    let request: ActorIndexRebuildStart =
        match decode_bounded(&body, "control_body_limit", "invalid_rebuild_json") {
            Ok(value) => value,
            Err(response) => return *response,
        };
    result_response(state.service.start_rebuild_reserved(permit, &request).await)
}

async fn promote(
    State(state): State<HttpState>,
    Extension(permit): Extension<Arc<OwnedSemaphorePermit>>,
    body: Bytes,
) -> Response {
    let request: ActorIndexPromote =
        match decode_bounded(&body, "control_body_limit", "invalid_promote_json") {
            Ok(value) => value,
            Err(response) => return *response,
        };
    result_response(state.service.promote_reserved(permit, &request).await)
}

async fn repair(
    State(state): State<HttpState>,
    Extension(permit): Extension<Arc<OwnedSemaphorePermit>>,
    body: Bytes,
) -> Response {
    let request: ActorIndexRepair =
        match decode_bounded(&body, "control_body_limit", "invalid_repair_json") {
            Ok(value) => value,
            Err(response) => return *response,
        };
    result_response(state.service.repair_reserved(permit, &request).await)
}

async fn retire(
    State(state): State<HttpState>,
    Extension(permit): Extension<Arc<OwnedSemaphorePermit>>,
    body: Bytes,
) -> Response {
    let request: ActorIndexRetire =
        match decode_bounded(&body, "control_body_limit", "invalid_retirement_json") {
            Ok(value) => value,
            Err(response) => return *response,
        };
    result_response(state.service.retire_reserved(permit, &request).await)
}

fn decode_bounded<T: DeserializeOwned>(
    body: &[u8],
    limit_code: &'static str,
    invalid_code: &'static str,
) -> Result<T, Box<Response>> {
    if body.len() > CONTROL_BODY_LIMIT {
        return Err(Box::new(failure(StatusCode::PAYLOAD_TOO_LARGE, limit_code)));
    }
    serde_json::from_slice(body)
        .map_err(|_| Box::new(failure(StatusCode::BAD_REQUEST, invalid_code)))
}

fn result_response<T: Serialize>(result: Result<T, ServiceError>) -> Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(error) => service_failure(error),
    }
}

fn service_failure(error: ServiceError) -> Response {
    let status = StatusCode::from_u16(error.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = failure(status, error.code);
    if let Some(seconds) = error.retry_after_secs {
        if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
    }
    response
}

fn failure(status: StatusCode, code: &'static str) -> Response {
    (status, Json(serde_json::json!({"error":{"code":code}}))).into_response()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::time::Duration;

    use axum::body::Body;
    use fleetworks_contracts::actor_index::ActorIndexAlias;
    use fleetworks_contracts::actor_index::ActorIndexMutation;
    use fleetworks_contracts::actor_index::ActorProjection;
    use fleetworks_contracts::actor_index::ACTOR_INDEX_PROTOCOL;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use super::*;
    use crate::engine::EngineConfig;
    use crate::pressure::Meter;

    fn test_config(directory: std::path::PathBuf) -> Config {
        Config {
            bind: "127.0.0.1:0".parse().expect("loopback"),
            allowed_hosts: BTreeSet::from(["search.test".into()]),
            read_digest: sha2::Sha256::digest(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").into(),
            write_digest: sha2::Sha256::digest(b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").into(),
            query_capacity: 4,
            limits: Default::default(),
            engine: EngineConfig {
                directory,
                deployment_id: "synthetic-deployment".into(),
                source_epoch: "synthetic-source".into(),
                writer_memory_bytes: 64 * 1024 * 1024,
                writer_threads: 1,
                merge_threads: 1,
            },
        }
    }

    #[test]
    fn service_failure_preserves_status_codes_and_retry_after_headers() {
        let paused = service_failure(ServiceError::pressure_paused("resource_pressure_paused"));
        assert_eq!(paused.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            paused
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok()),
            Some("5")
        );

        let paced = service_failure(ServiceError::paced("write_paced"));
        assert_eq!(paced.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            paced
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok()),
            Some("1")
        );
    }

    #[tokio::test]
    async fn http_adapter_preserves_body_limits_retry_headers_and_paused_status() {
        let temporary = TempDir::new().expect("tempdir");
        let dir = temporary
            .path()
            .canonicalize()
            .expect("canonical")
            .join("index-volume");
        let config = test_config(dir);
        let engine = Arc::new(Engine::open(config.engine.clone()).expect("engine"));
        let router = router(config.clone(), Some(engine), "synthetic_unavailable").expect("router");

        let oversized_query = Request::builder()
            .method("POST")
            .uri(QUERY_PATH)
            .header("Host", "search.test")
            .header("Authorization", "Bearer aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .body(Body::from(vec![b'x'; CONTROL_BODY_LIMIT + 1]))
            .expect("oversized query");
        let response = router
            .clone()
            .oneshot(oversized_query)
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            response
                .headers()
                .get(header::CACHE_CONTROL)
                .and_then(|value| value.to_str().ok()),
            Some("no-store")
        );

        let oversized_control = Request::builder()
            .method("POST")
            .uri(START_PATH)
            .header("Host", "search.test")
            .header("Authorization", "Bearer bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
            .body(Body::from(vec![b'x'; CONTROL_BODY_LIMIT + 1]))
            .expect("oversized control");
        assert_eq!(
            router
                .clone()
                .oneshot(oversized_control)
                .await
                .expect("response")
                .status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );

        let mutation = ActorIndexMutation::new(
            "actor:a".into(),
            1,
            Some(ActorProjection {
                display_name: "Harbor Union".into(),
                aliases: vec![ActorIndexAlias {
                    value: "Harbor".into(),
                    strength: "catalog".into(),
                    provenance: Some("synthetic-fixture".into()),
                }],
                summary: None,
                actor_type: None,
                countries: Vec::new(),
                source_id: None,
                authority_scopes: vec!["reader".into()],
                eligible: true,
            }),
        )
        .expect("mutation");
        let batch_body = serde_json::to_vec(&ActorIndexBatch {
            protocol: ACTOR_INDEX_PROTOCOL,
            deployment_id: config.engine.deployment_id.clone(),
            source_epoch: config.engine.source_epoch.clone(),
            index_generation: "g1".into(),
            batch_id: "http-batch-1".into(),
            writer_epoch: 1,
            from_checkpoint: 0,
            through_checkpoint: 1,
            source_complete: true,
            mutations: vec![mutation],
        })
        .expect("batch json");

        let write_req = |body: Vec<u8>| {
            Request::builder()
                .method("POST")
                .uri(BATCH_PATH)
                .header("Host", "search.test")
                .header("Authorization", "Bearer bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
                .body(Body::from(body))
                .expect("batch request")
        };

        let first = router
            .clone()
            .oneshot(write_req(batch_body.clone()))
            .await
            .expect("first batch");
        assert_eq!(first.status(), StatusCode::OK);

        let second = router
            .clone()
            .oneshot(write_req(batch_body.clone()))
            .await
            .expect("paced batch");
        assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            second
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok()),
            Some("1")
        );

        let meter = Arc::new(
            Meter::new(config.engine.directory.clone(), config.limits.clone()).expect("meter"),
        );
        meter
            .record_commit(Duration::from_secs(10))
            .expect("trip pressure");
        let paused_engine = Arc::new(EngineSlot::new(Some(Arc::new(
            Engine::open(EngineConfig {
                directory: temporary
                    .path()
                    .canonicalize()
                    .expect("canonical")
                    .join("paused-volume"),
                ..config.engine.clone()
            })
            .expect("paused engine"),
        ))));
        let paused_service = SearchService::from_config(&config, paused_engine, "unavailable")
            .expect("service")
            .with_meter(meter);
        let paused_router = router_with_service(config, paused_service);
        let invalid_json = paused_router
            .clone()
            .oneshot(write_req(b"{}".to_vec()))
            .await
            .expect("invalid json response");
        assert_eq!(invalid_json.status(), StatusCode::BAD_REQUEST);

        let paused_response = paused_router
            .oneshot(write_req(batch_body))
            .await
            .expect("paused response");
        assert_eq!(paused_response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            paused_response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok()),
            Some("5")
        );
    }
}
