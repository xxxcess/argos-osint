//! Bounded private HTTP transport. No browser or public gateway token can write.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;
use std::time::Duration;
use std::time::Instant;

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
use tokio::sync::Semaphore;

use crate::config;
use crate::config::Config;
use crate::engine::Engine;
use crate::pressure::Meter;
use crate::ErrorKind;
use crate::IndexError;

const BATCH_PATH: &str = "/internal/v1/actors/batches";
const QUERY_PATH: &str = "/internal/v1/actors/query";
const STATUS_PATH: &str = "/internal/v1/actors/status";
const START_PATH: &str = "/internal/v1/actors/rebuild/start";
const PROMOTE_PATH: &str = "/internal/v1/actors/rebuild/promote";
const REPAIR_PATH: &str = "/internal/v1/actors/rebuild/repair";
const RETIRE_PATH: &str = "/internal/v1/actors/generations/retire";

#[derive(Default)]
pub struct Pace {
    last: Option<Instant>,
}

impl Pace {
    /// No catch-up burst; callers use controlled Instants in policy tests.
    pub fn admit(&mut self, now: Instant) -> Result<(), Duration> {
        self.admit_after(now, Duration::from_millis(500))
    }

    fn admit_after(&mut self, now: Instant, spacing: Duration) -> Result<(), Duration> {
        if let Some(last) = self.last {
            let elapsed = now.checked_duration_since(last).unwrap_or(Duration::ZERO);
            if elapsed < spacing {
                return Err(spacing - elapsed);
            }
        }
        self.last = Some(now);
        Ok(())
    }
}

/// Production main retains this slot outside Tokio, including repaired engines,
/// so the final mmap/writer destruction happens on the plain owning thread.
pub struct EngineSlot(RwLock<Option<Arc<Engine>>>);

impl EngineSlot {
    pub fn new(engine: Option<Arc<Engine>>) -> Self {
        Self(RwLock::new(engine))
    }
    fn snapshot(&self) -> Result<Option<Arc<Engine>>, IndexError> {
        self.0
            .read()
            .map(|engine| engine.clone())
            .map_err(|_| IndexError::unavailable("engine_slot_poisoned"))
    }
    fn install(&self, engine: Engine) -> Result<(), IndexError> {
        let mut current = self
            .0
            .write()
            .map_err(|_| IndexError::unavailable("engine_slot_poisoned"))?;
        if current.is_some() {
            return Err(IndexError::conflict("healthy_engine_repair_not_allowed"));
        }
        *current = Some(Arc::new(engine));
        Ok(())
    }
}

#[derive(Clone)]
struct Service {
    config: Arc<Config>,
    engine: Arc<EngineSlot>,
    unavailable: &'static str,
    requests: Arc<Semaphore>,
    queries: Arc<Semaphore>,
    writes: Arc<Semaphore>,
    pace: Arc<Mutex<Pace>>,
    meter: Arc<Meter>,
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
    let meter = Arc::new(Meter::new(
        config.engine.directory.clone(),
        config.limits.clone(),
    )?);
    let service = Service {
        queries: Arc::new(Semaphore::new(config.query_capacity)),
        config: Arc::new(config),
        engine,
        unavailable,
        requests: Arc::new(Semaphore::new(8)),
        writes: Arc::new(Semaphore::new(1)),
        pace: Arc::new(Mutex::new(Pace::default())),
        meter,
    };
    Ok(Router::new()
        .route(STATUS_PATH, get(status))
        .route(QUERY_PATH, post(query).layer(DefaultBodyLimit::max(4096)))
        .route(BATCH_PATH, post(batch))
        .route(START_PATH, post(start))
        .route(PROMOTE_PATH, post(promote))
        .route(REPAIR_PATH, post(repair))
        .route(RETIRE_PATH, post(retire))
        .layer(DefaultBodyLimit::max(MAX_BATCH_BYTES))
        .layer(middleware::from_fn_with_state(service.clone(), ingress))
        .with_state(service))
}

async fn ingress(State(service): State<Service>, mut request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok());
    if !host.is_some_and(|host| {
        service
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
    let write = bool::from(digest.ct_eq(&service.config.write_digest));
    let read = bool::from(digest.ct_eq(&service.config.read_digest));
    let mutating = matches!(
        request.uri().path(),
        BATCH_PATH | START_PATH | PROMOTE_PATH | REPAIR_PATH | RETIRE_PATH
    );
    if (!read && !write) || (mutating && !write) {
        return failure(StatusCode::FORBIDDEN, "private_permission_denied");
    }
    let Ok(_permit) = Arc::clone(&service.requests).try_acquire_owned() else {
        return retry(StatusCode::SERVICE_UNAVAILABLE, "request_capacity");
    };
    if mutating {
        let Ok(permit) = Arc::clone(&service.writes).try_acquire_owned() else {
            return retry(StatusCode::SERVICE_UNAVAILABLE, "writer_busy");
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

async fn status(State(service): State<Service>) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let mut status = match service.engine.snapshot()? {
            Some(engine) => engine.status()?,
            None => Engine::recovery_status(&service.config.engine, service.unavailable),
        };
        status.limits = service.config.limits.clone();
        status.pressure = Some(service.meter.snapshot()?);
        if status
            .pressure
            .as_ref()
            .is_some_and(|pressure| pressure.paused)
            && status.active.is_some()
        {
            status.state = "paused".into();
            status.reason_code = status
                .pressure
                .as_ref()
                .and_then(|pressure| pressure.reason_code.clone());
        }
        Ok::<_, IndexError>(status)
    })
    .await;
    match result {
        Ok(result) => result_response(result),
        Err(_) => retry(StatusCode::SERVICE_UNAVAILABLE, "status_worker_unavailable"),
    }
}

async fn query(State(service): State<Service>, body: Bytes) -> Response {
    if body.len() > 4096 {
        return failure(StatusCode::PAYLOAD_TOO_LARGE, "query_body_limit");
    }
    let request: ActorIndexQuery = match decode(&body) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let engine = match service.engine.snapshot() {
        Ok(Some(engine)) => engine,
        Ok(None) => return failure(StatusCode::SERVICE_UNAVAILABLE, service.unavailable),
        Err(error) => return index_failure(error),
    };
    let Ok(permit) = service.queries.try_acquire_owned() else {
        return retry(StatusCode::TOO_MANY_REQUESTS, "query_capacity");
    };
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let at = Instant::now();
        let result = engine.query(&request);
        service.meter.record_query(at.elapsed())?;
        result
    });
    match tokio::time::timeout(Duration::from_secs(2), task).await {
        Ok(Ok(result)) => result_response(result),
        _ => retry(StatusCode::SERVICE_UNAVAILABLE, "query_unavailable"),
    }
}

async fn batch(
    State(service): State<Service>,
    Extension(permit): Extension<Arc<OwnedSemaphorePermit>>,
    body: Bytes,
) -> Response {
    if body.len() > service.config.limits.batch_bytes as usize {
        return failure(StatusCode::PAYLOAD_TOO_LARGE, "configured_batch_byte_limit");
    }
    let limit = service.config.limits.batch_documents;
    write(service, permit, body, move |engine, bytes| {
        let batch: ActorIndexBatch =
            serde_json::from_slice(bytes).map_err(|_| IndexError::invalid("invalid_batch_json"))?;
        if batch.mutations.len() > usize::from(limit) {
            return Err(IndexError::invalid("configured_batch_document_limit"));
        }
        engine.apply(&batch)
    })
    .await
}

async fn start(
    State(service): State<Service>,
    Extension(permit): Extension<Arc<OwnedSemaphorePermit>>,
    body: Bytes,
) -> Response {
    if body.len() > 4096 {
        return failure(StatusCode::PAYLOAD_TOO_LARGE, "control_body_limit");
    }
    write(service, permit, body, |engine, bytes| {
        let request: ActorIndexRebuildStart = serde_json::from_slice(bytes)
            .map_err(|_| IndexError::invalid("invalid_rebuild_json"))?;
        engine.start_rebuild(&request)
    })
    .await
}

async fn promote(
    State(service): State<Service>,
    Extension(permit): Extension<Arc<OwnedSemaphorePermit>>,
    body: Bytes,
) -> Response {
    if body.len() > 4096 {
        return failure(StatusCode::PAYLOAD_TOO_LARGE, "control_body_limit");
    }
    write(service, permit, body, |engine, bytes| {
        let request: ActorIndexPromote = serde_json::from_slice(bytes)
            .map_err(|_| IndexError::invalid("invalid_promote_json"))?;
        engine.promote(&request)
    })
    .await
}

async fn repair(
    State(service): State<Service>,
    Extension(permit): Extension<Arc<OwnedSemaphorePermit>>,
    body: Bytes,
) -> Response {
    if body.len() > 4096 {
        return failure(StatusCode::PAYLOAD_TOO_LARGE, "control_body_limit");
    }
    if let Err(response) = admit_pace(&service) {
        return response;
    }
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let request: ActorIndexRepair = serde_json::from_slice(&body)
            .map_err(|_| IndexError::invalid("invalid_repair_json"))?;
        request.validate().map_err(IndexError::protocol)?;
        service.meter.admit_write()?;
        let started = Instant::now();
        let result = (|| {
            if let Some(engine) = service.engine.snapshot()? {
                return engine.repair_shadow(&request);
            }
            service
                .engine
                .install(Engine::repair(&service.config.engine, &request)?)?;
            service
                .engine
                .snapshot()?
                .ok_or_else(|| IndexError::unavailable("repaired_engine_missing"))?
                .status()
        })();
        if result.is_ok() {
            service.meter.record_commit(started.elapsed())?;
        } else {
            service.meter.cancel_probe()?;
        }
        result
    });
    match tokio::time::timeout(Duration::from_secs(30), task).await {
        Ok(Ok(result)) => result_response(result),
        _ => retry(StatusCode::SERVICE_UNAVAILABLE, "repair_outcome_uncertain"),
    }
}

async fn retire(
    State(service): State<Service>,
    Extension(permit): Extension<Arc<OwnedSemaphorePermit>>,
    body: Bytes,
) -> Response {
    if body.len() > 4096 {
        return failure(StatusCode::PAYLOAD_TOO_LARGE, "control_body_limit");
    }
    if let Err(response) = admit_pace(&service) {
        return response;
    }
    let Ok(queries) =
        Arc::clone(&service.queries).try_acquire_many_owned(service.config.query_capacity as u32)
    else {
        return retry(
            StatusCode::SERVICE_UNAVAILABLE,
            "retirement_waits_for_readers",
        );
    };
    // Explicit cleanup can free disk under pressure. The same write/query
    // permits survive an HTTP timeout until the guarded unlink finishes.
    let task = tokio::task::spawn_blocking(move || {
        let (_permit, _queries) = (permit, queries);
        let request: ActorIndexRetire = serde_json::from_slice(&body)
            .map_err(|_| IndexError::invalid("invalid_retirement_json"))?;
        service
            .engine
            .snapshot()?
            .ok_or_else(|| IndexError::unavailable(service.unavailable))?
            .retire(&request)
    });
    match tokio::time::timeout(Duration::from_secs(30), task).await {
        Ok(Ok(result)) => result_response(result),
        _ => retry(
            StatusCode::SERVICE_UNAVAILABLE,
            "retirement_outcome_uncertain",
        ),
    }
}

async fn write<T: Serialize + Send + 'static>(
    service: Service,
    permit: Arc<OwnedSemaphorePermit>,
    body: Bytes,
    operation: impl FnOnce(&Engine, &[u8]) -> Result<T, IndexError> + Send + 'static,
) -> Response {
    let engine = match service.engine.snapshot() {
        Ok(Some(engine)) => engine,
        Ok(None) => return failure(StatusCode::SERVICE_UNAVAILABLE, service.unavailable),
        Err(error) => return index_failure(error),
    };
    if let Err(response) = admit_pace(&service) {
        return response;
    }
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        service.meter.admit_write()?;
        let at = Instant::now();
        let result = operation(&engine, &body);
        if !matches!(&result,Err(error) if matches!(error.kind,ErrorKind::Invalid|ErrorKind::Conflict))
        {
            service.meter.record_commit(at.elapsed())?;
        } else {
            service.meter.cancel_probe()?;
        }
        result
    });
    match tokio::time::timeout(Duration::from_secs(30), task).await {
        Ok(Ok(result)) => result_response(result),
        _ => retry(
            StatusCode::SERVICE_UNAVAILABLE,
            "write_outcome_uncertain_replay_same_batch",
        ),
    }
}

fn admit_pace(service: &Service) -> Result<(), Response> {
    match service.pace.lock() {
        Ok(mut pace) => {
            if pace
                .admit_after(
                    Instant::now(),
                    Duration::from_millis(u64::from(service.config.limits.min_start_ms)),
                )
                .is_err()
            {
                return Err(retry(StatusCode::TOO_MANY_REQUESTS, "write_paced"));
            }
        }
        Err(_) => {
            return Err(failure(
                StatusCode::SERVICE_UNAVAILABLE,
                "pace_state_poisoned",
            ))
        }
    }
    Ok(())
}

fn decode<T: DeserializeOwned>(body: &[u8]) -> Result<T, Response> {
    serde_json::from_slice(body).map_err(|_| failure(StatusCode::BAD_REQUEST, "invalid_json"))
}
fn result_response<T: Serialize>(result: Result<T, IndexError>) -> Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(error) => index_failure(error),
    }
}
fn index_failure(error: IndexError) -> Response {
    if error.code == "resource_pressure_paused" {
        let mut response = retry(StatusCode::SERVICE_UNAVAILABLE, "resource_pressure_paused");
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
        return response;
    }
    let status = match error.kind {
        ErrorKind::Invalid => StatusCode::BAD_REQUEST,
        ErrorKind::Conflict => StatusCode::CONFLICT,
        ErrorKind::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
    };
    failure(status, error.code)
}
fn failure(status: StatusCode, code: &'static str) -> Response {
    (status, Json(serde_json::json!({"error":{"code":code}}))).into_response()
}
fn retry(status: StatusCode, code: &'static str) -> Response {
    let mut response = failure(status, code);
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
    response
}
