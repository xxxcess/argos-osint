//! Transport-neutral actor search service boundary.
//!
//! Enforces bounded admission, reader/writer concurrency, pacing, pressure
//! hysteresis, timeouts, and reader-drained retirement before touching [`Engine`].

use std::fmt;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;
use std::time::Duration;
use std::time::Instant;

use fleetworks_contracts::actor_index::ActorIndexBatch;
use fleetworks_contracts::actor_index::ActorIndexGeneration;
use fleetworks_contracts::actor_index::ActorIndexLimits;
use fleetworks_contracts::actor_index::ActorIndexPromote;
use fleetworks_contracts::actor_index::ActorIndexQuery;
use fleetworks_contracts::actor_index::ActorIndexQueryResponse;
use fleetworks_contracts::actor_index::ActorIndexRebuildStart;
use fleetworks_contracts::actor_index::ActorIndexReceipt;
use fleetworks_contracts::actor_index::ActorIndexRepair;
use fleetworks_contracts::actor_index::ActorIndexRetire;
use fleetworks_contracts::actor_index::ActorIndexStatus;
use tokio::sync::OwnedSemaphorePermit;
use tokio::sync::Semaphore;

use crate::config::Config;
use crate::config::LocalEngineConfig;
use crate::engine::Engine;
use crate::pressure::Meter;
use crate::ErrorKind;
use crate::IndexError;

const REQUEST_CAPACITY: usize = 8;
const WRITE_CAPACITY: usize = 1;
const QUERY_TIMEOUT: Duration = Duration::from_secs(2);
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// Controlled write pacing window without catch-up bursts.
#[derive(Debug, Default)]
pub struct Pace {
    last: Option<Instant>,
}

impl Pace {
    /// No catch-up burst; callers use controlled Instants in policy tests.
    pub fn admit(&mut self, now: Instant) -> Result<(), Duration> {
        self.admit_after(now, Duration::from_millis(500))
    }

    pub fn admit_after(&mut self, now: Instant, spacing: Duration) -> Result<(), Duration> {
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

    pub fn snapshot(&self) -> Result<Option<Arc<Engine>>, IndexError> {
        self.0
            .read()
            .map(|engine| engine.clone())
            .map_err(|_| IndexError::unavailable("engine_slot_poisoned"))
    }

    pub fn install(&self, engine: Engine) -> Result<(), IndexError> {
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

/// Transport-neutral service failure carrying HTTP-compatible status and retry hints.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub struct ServiceError {
    pub status: u16,
    pub code: &'static str,
    pub retry_after_secs: Option<u64>,
}

impl ServiceError {
    pub fn invalid(code: &'static str) -> Self {
        Self {
            status: 400,
            code,
            retry_after_secs: None,
        }
    }

    pub fn conflict(code: &'static str) -> Self {
        Self {
            status: 409,
            code,
            retry_after_secs: None,
        }
    }

    pub fn payload_too_large(code: &'static str) -> Self {
        Self {
            status: 413,
            code,
            retry_after_secs: None,
        }
    }

    pub fn paced(code: &'static str) -> Self {
        Self {
            status: 429,
            code,
            retry_after_secs: Some(1),
        }
    }

    pub fn unavailable(code: &'static str) -> Self {
        Self {
            status: 503,
            code,
            retry_after_secs: None,
        }
    }

    pub fn busy(code: &'static str) -> Self {
        Self {
            status: 503,
            code,
            retry_after_secs: Some(1),
        }
    }

    pub fn pressure_paused(code: &'static str) -> Self {
        Self {
            status: 503,
            code,
            retry_after_secs: Some(5),
        }
    }

    pub fn from_index_error(error: IndexError) -> Self {
        if error.code == "resource_pressure_paused" {
            return Self::pressure_paused("resource_pressure_paused");
        }
        match error.kind {
            ErrorKind::Invalid => Self::invalid(error.code),
            ErrorKind::Conflict => Self::conflict(error.code),
            ErrorKind::Unavailable => Self::unavailable(error.code),
        }
    }
}

impl From<IndexError> for ServiceError {
    fn from(error: IndexError) -> Self {
        Self::from_index_error(error)
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for ServiceError {}

/// Transport-neutral facade over [`EngineSlot`] enforcing shared capacity,
/// pacing, resource pressure, timeouts, and reader-drained retirement.
#[derive(Clone)]
pub struct SearchService {
    config: LocalEngineConfig,
    engine: Arc<EngineSlot>,
    unavailable: &'static str,
    meter: Arc<Meter>,
    pace: Arc<Mutex<Pace>>,
    requests: Arc<Semaphore>,
    writes: Arc<Semaphore>,
    queries: Arc<Semaphore>,
    query_capacity: usize,
}

impl SearchService {
    pub fn new(
        config: LocalEngineConfig,
        engine: Arc<EngineSlot>,
        unavailable: &'static str,
    ) -> Result<Self, IndexError> {
        let meter = Arc::new(Meter::new(
            config.engine.directory.clone(),
            config.limits.clone(),
        )?);
        let query_capacity = config.query_capacity.clamp(1, 8);
        Ok(Self {
            config,
            engine,
            unavailable,
            meter,
            pace: Arc::new(Mutex::new(Pace::default())),
            requests: Arc::new(Semaphore::new(REQUEST_CAPACITY)),
            writes: Arc::new(Semaphore::new(WRITE_CAPACITY)),
            queries: Arc::new(Semaphore::new(query_capacity)),
            query_capacity,
        })
    }

    pub fn from_config(
        config: &Config,
        engine: Arc<EngineSlot>,
        unavailable: &'static str,
    ) -> Result<Self, IndexError> {
        Self::new(LocalEngineConfig::from(config), engine, unavailable)
    }

    pub fn with_meter(mut self, meter: Arc<Meter>) -> Self {
        self.meter = meter;
        self
    }

    pub fn meter(&self) -> &Arc<Meter> {
        &self.meter
    }

    pub fn limits(&self) -> &ActorIndexLimits {
        &self.config.limits
    }

    pub fn engine(&self) -> &Arc<EngineSlot> {
        &self.engine
    }

    pub(crate) fn try_acquire_request(&self) -> Result<OwnedSemaphorePermit, ServiceError> {
        Arc::clone(&self.requests)
            .try_acquire_owned()
            .map_err(|_| ServiceError::busy("request_capacity"))
    }

    pub(crate) fn try_acquire_write(&self) -> Result<OwnedSemaphorePermit, ServiceError> {
        Arc::clone(&self.writes)
            .try_acquire_owned()
            .map_err(|_| ServiceError::busy("writer_busy"))
    }

    pub async fn status(&self) -> Result<ActorIndexStatus, ServiceError> {
        let _request = self.try_acquire_request()?;
        self.status_reserved().await
    }

    pub(crate) async fn status_reserved(&self) -> Result<ActorIndexStatus, ServiceError> {
        let service = self.clone();
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
            Ok(Ok(status)) => Ok(status),
            Ok(Err(error)) => Err(ServiceError::from_index_error(error)),
            Err(_) => Err(ServiceError::busy("status_worker_unavailable")),
        }
    }

    pub async fn query(
        &self,
        request: &ActorIndexQuery,
    ) -> Result<ActorIndexQueryResponse, ServiceError> {
        let _request = self.try_acquire_request()?;
        self.query_reserved(request).await
    }

    pub(crate) async fn query_reserved(
        &self,
        request: &ActorIndexQuery,
    ) -> Result<ActorIndexQueryResponse, ServiceError> {
        let engine = self
            .engine
            .snapshot()
            .map_err(ServiceError::from_index_error)?
            .ok_or_else(|| ServiceError::unavailable(self.unavailable))?;
        let permit = Arc::clone(&self.queries)
            .try_acquire_owned()
            .map_err(|_| ServiceError::paced("query_capacity"))?;
        let request = request.clone();
        let meter = Arc::clone(&self.meter);
        let task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let at = Instant::now();
            let result = engine.query(&request);
            meter.record_query(at.elapsed())?;
            result
        });
        match tokio::time::timeout(QUERY_TIMEOUT, task).await {
            Ok(Ok(Ok(response))) => Ok(response),
            Ok(Ok(Err(error))) => Err(ServiceError::from_index_error(error)),
            _ => Err(ServiceError::busy("query_unavailable")),
        }
    }

    pub async fn apply_batch(
        &self,
        batch: &ActorIndexBatch,
        raw_len: Option<usize>,
    ) -> Result<ActorIndexReceipt, ServiceError> {
        let _request = self.try_acquire_request()?;
        let write = self.try_acquire_write()?;
        self.apply_batch_reserved(Arc::new(write), batch, raw_len)
            .await
    }

    pub(crate) async fn apply_batch_reserved(
        &self,
        permit: Arc<OwnedSemaphorePermit>,
        batch: &ActorIndexBatch,
        raw_len: Option<usize>,
    ) -> Result<ActorIndexReceipt, ServiceError> {
        if let Some(len) = raw_len {
            if len > self.config.limits.batch_bytes as usize {
                return Err(ServiceError::payload_too_large(
                    "configured_batch_byte_limit",
                ));
            }
        }
        let batch = batch.clone();
        self.run_write(permit, move |engine, limits| {
            if batch.mutations.len() > usize::from(limits.batch_documents) {
                return Err(IndexError::invalid("configured_batch_document_limit"));
            }
            engine.apply(&batch)
        })
        .await
    }

    pub async fn start_rebuild(
        &self,
        request: &ActorIndexRebuildStart,
    ) -> Result<ActorIndexGeneration, ServiceError> {
        let _request = self.try_acquire_request()?;
        let write = self.try_acquire_write()?;
        self.start_rebuild_reserved(Arc::new(write), request).await
    }

    pub(crate) async fn start_rebuild_reserved(
        &self,
        permit: Arc<OwnedSemaphorePermit>,
        request: &ActorIndexRebuildStart,
    ) -> Result<ActorIndexGeneration, ServiceError> {
        let request = request.clone();
        self.run_write(permit, move |engine, _| engine.start_rebuild(&request))
            .await
    }

    pub async fn promote(
        &self,
        request: &ActorIndexPromote,
    ) -> Result<ActorIndexGeneration, ServiceError> {
        let _request = self.try_acquire_request()?;
        let write = self.try_acquire_write()?;
        self.promote_reserved(Arc::new(write), request).await
    }

    pub(crate) async fn promote_reserved(
        &self,
        permit: Arc<OwnedSemaphorePermit>,
        request: &ActorIndexPromote,
    ) -> Result<ActorIndexGeneration, ServiceError> {
        let request = request.clone();
        self.run_write(permit, move |engine, _| engine.promote(&request))
            .await
    }

    pub async fn repair(
        &self,
        request: &ActorIndexRepair,
    ) -> Result<ActorIndexStatus, ServiceError> {
        let _request = self.try_acquire_request()?;
        let write = self.try_acquire_write()?;
        self.repair_reserved(Arc::new(write), request).await
    }

    pub(crate) async fn repair_reserved(
        &self,
        permit: Arc<OwnedSemaphorePermit>,
        request: &ActorIndexRepair,
    ) -> Result<ActorIndexStatus, ServiceError> {
        self.admit_pace()?;
        let request = request.clone();
        let service = self.clone();
        let task = tokio::task::spawn_blocking(move || {
            // Retain the write permit inside the blocking worker so a timed-out
            // caller cannot admit a competing mutation before repair completes.
            let _permit = permit;
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
        match tokio::time::timeout(WRITE_TIMEOUT, task).await {
            Ok(Ok(Ok(status))) => Ok(status),
            Ok(Ok(Err(error))) => Err(ServiceError::from_index_error(error)),
            _ => Err(ServiceError::busy("repair_outcome_uncertain")),
        }
    }

    pub async fn retire(
        &self,
        request: &ActorIndexRetire,
    ) -> Result<ActorIndexStatus, ServiceError> {
        let _request = self.try_acquire_request()?;
        let write = self.try_acquire_write()?;
        self.retire_reserved(Arc::new(write), request).await
    }

    pub(crate) async fn retire_reserved(
        &self,
        permit: Arc<OwnedSemaphorePermit>,
        request: &ActorIndexRetire,
    ) -> Result<ActorIndexStatus, ServiceError> {
        self.admit_pace()?;
        let queries = Arc::clone(&self.queries)
            .try_acquire_many_owned(self.query_capacity as u32)
            .map_err(|_| ServiceError::busy("retirement_waits_for_readers"))?;
        // Explicit cleanup can free disk under pressure. The same write/query
        // permits survive a caller timeout until the guarded unlink finishes.
        let request = request.clone();
        let service = self.clone();
        let task = tokio::task::spawn_blocking(move || {
            let (_permit, _queries) = (permit, queries);
            service
                .engine
                .snapshot()?
                .ok_or_else(|| IndexError::unavailable(service.unavailable))?
                .retire(&request)
        });
        match tokio::time::timeout(WRITE_TIMEOUT, task).await {
            Ok(Ok(Ok(status))) => Ok(status),
            Ok(Ok(Err(error))) => Err(ServiceError::from_index_error(error)),
            _ => Err(ServiceError::busy("retirement_outcome_uncertain")),
        }
    }

    async fn run_write<T: Send + 'static>(
        &self,
        permit: Arc<OwnedSemaphorePermit>,
        operation: impl FnOnce(&Engine, &ActorIndexLimits) -> Result<T, IndexError> + Send + 'static,
    ) -> Result<T, ServiceError> {
        let engine = self
            .engine
            .snapshot()
            .map_err(ServiceError::from_index_error)?
            .ok_or_else(|| ServiceError::unavailable(self.unavailable))?;
        self.admit_pace()?;
        let meter = Arc::clone(&self.meter);
        let limits = self.config.limits.clone();
        let task = tokio::task::spawn_blocking(move || {
            // Keep the write permit alive inside the blocking worker across
            // caller timeouts so a competing write cannot interleave.
            let _permit = permit;
            meter.admit_write()?;
            let at = Instant::now();
            let result = operation(&engine, &limits);
            if !matches!(
                &result,
                Err(error) if matches!(error.kind, ErrorKind::Invalid | ErrorKind::Conflict)
            ) {
                meter.record_commit(at.elapsed())?;
            } else {
                meter.cancel_probe()?;
            }
            result
        });
        match tokio::time::timeout(WRITE_TIMEOUT, task).await {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(error))) => Err(ServiceError::from_index_error(error)),
            _ => Err(ServiceError::busy(
                "write_outcome_uncertain_replay_same_batch",
            )),
        }
    }

    fn admit_pace(&self) -> Result<(), ServiceError> {
        let mut pace = self
            .pace
            .lock()
            .map_err(|_| ServiceError::unavailable("pace_state_poisoned"))?;
        if pace
            .admit_after(
                Instant::now(),
                Duration::from_millis(u64::from(self.config.limits.min_start_ms)),
            )
            .is_err()
        {
            return Err(ServiceError::paced("write_paced"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use fleetworks_contracts::actor_index::ActorIndexAlias;
    use fleetworks_contracts::actor_index::ActorIndexMutation;
    use fleetworks_contracts::actor_index::ActorProjection;
    use fleetworks_contracts::actor_index::ACTOR_INDEX_PROTOCOL;
    use tempfile::TempDir;

    use super::*;

    fn local_config(limits: ActorIndexLimits) -> (TempDir, LocalEngineConfig) {
        let temporary = TempDir::new().expect("isolated fixture root");
        let directory = temporary
            .path()
            .canonicalize()
            .expect("canonical fixture root")
            .join("index-volume");
        let config = LocalEngineConfig::for_identity(
            directory,
            "synthetic-deployment".into(),
            "synthetic-source".into(),
            limits,
        )
        .expect("valid local config");
        (temporary, config)
    }

    fn actor(id: &str, revision: u64, name: &str) -> ActorIndexMutation {
        ActorIndexMutation::new(
            id.into(),
            revision,
            Some(ActorProjection {
                display_name: name.into(),
                aliases: vec![ActorIndexAlias {
                    value: "Harbor".into(),
                    strength: "catalog".into(),
                    provenance: Some("synthetic-fixture".into()),
                }],
                summary: None,
                actor_type: Some("synthetic organization".into()),
                countries: Vec::new(),
                source_id: Some("fixture-catalog".into()),
                authority_scopes: vec!["synthetic-reader".into()],
                eligible: true,
            }),
        )
        .expect("valid synthetic projection")
    }

    fn batch(
        id: &str,
        generation: &str,
        from: u64,
        writer_epoch: u64,
        mutations: Vec<ActorIndexMutation>,
    ) -> ActorIndexBatch {
        ActorIndexBatch {
            protocol: ACTOR_INDEX_PROTOCOL,
            deployment_id: "synthetic-deployment".into(),
            source_epoch: "synthetic-source".into(),
            index_generation: generation.into(),
            batch_id: id.into(),
            writer_epoch,
            from_checkpoint: from,
            through_checkpoint: from + 1,
            source_complete: true,
            mutations,
        }
    }

    fn reset_pace(service: &SearchService) {
        *service.pace.lock().expect("unpoisoned pace") = Pace::default();
    }

    #[test]
    fn service_error_maps_index_errors_and_retry_after_consistently() {
        assert_eq!(
            ServiceError::from_index_error(IndexError::invalid("bad_input")),
            ServiceError::invalid("bad_input")
        );
        assert_eq!(ServiceError::invalid("bad_input").status, 400);
        assert_eq!(ServiceError::invalid("bad_input").retry_after_secs, None);

        assert_eq!(
            ServiceError::from_index_error(IndexError::conflict("stale")),
            ServiceError::conflict("stale")
        );
        assert_eq!(ServiceError::conflict("stale").status, 409);

        assert_eq!(
            ServiceError::from_index_error(IndexError::unavailable("resource_pressure_paused")),
            ServiceError::pressure_paused("resource_pressure_paused")
        );
        assert_eq!(
            ServiceError::pressure_paused("resource_pressure_paused").status,
            503
        );
        assert_eq!(
            ServiceError::pressure_paused("resource_pressure_paused").retry_after_secs,
            Some(5)
        );

        assert_eq!(
            ServiceError::from_index_error(IndexError::unavailable("down")),
            ServiceError::unavailable("down")
        );
        assert_eq!(ServiceError::unavailable("down").retry_after_secs, None);
        assert_eq!(ServiceError::payload_too_large("too_big").status, 413);
        assert_eq!(ServiceError::paced("write_paced").status, 429);
        assert_eq!(ServiceError::paced("write_paced").retry_after_secs, Some(1));
        assert_eq!(ServiceError::busy("writer_busy").status, 503);
        assert_eq!(ServiceError::busy("writer_busy").retry_after_secs, Some(1));
    }

    #[tokio::test]
    async fn status_reports_paused_overlay_only_when_active_generation_exists() {
        let (_temporary, config) = local_config(ActorIndexLimits::default());
        let empty_service = SearchService::new(
            config.clone(),
            Arc::new(EngineSlot::new(None)),
            "synthetic_unavailable",
        )
        .expect("empty service");
        let empty_status = empty_service.status().await.expect("recovery status");
        assert!(empty_status.active.is_none());
        assert_ne!(empty_status.state, "paused");
        assert_eq!(
            empty_status.reason_code.as_deref(),
            Some("synthetic_unavailable")
        );

        let engine = Arc::new(Engine::open(config.engine.clone()).expect("opened engine"));
        let shared_meter = Arc::new(
            Meter::new(config.engine.directory.clone(), config.limits.clone()).expect("meter"),
        );
        shared_meter
            .record_commit(Duration::from_secs(10))
            .expect("trip commit latency high water");
        let service = SearchService::new(
            config,
            Arc::new(EngineSlot::new(Some(engine))),
            "synthetic_unavailable",
        )
        .expect("service")
        .with_meter(Arc::clone(&shared_meter));
        assert!(Arc::ptr_eq(service.meter(), &shared_meter));
        let status = service.status().await.expect("paused status");
        assert!(status.active.is_some());
        assert_eq!(status.state, "paused");
        assert!(status.reason_code.is_some());
        assert!(status.pressure.is_some_and(|pressure| pressure.paused));
    }

    #[tokio::test]
    async fn service_enforces_limits_pacing_concurrency_and_reader_drained_retirement() {
        let limits = ActorIndexLimits {
            batch_documents: 1,
            query_capacity: 2,
            ..ActorIndexLimits::default()
        };
        let (_temporary, config) = local_config(limits);
        let engine = Arc::new(Engine::open(config.engine.clone()).expect("opened engine"));
        let service = SearchService::new(
            config.clone(),
            Arc::new(EngineSlot::new(Some(engine))),
            "synthetic_unavailable",
        )
        .expect("service");

        let oversized = batch(
            "oversized-bytes",
            "g1",
            0,
            1,
            vec![actor("actor:a", 1, "Harbor Union")],
        );
        assert_eq!(
            service
                .apply_batch(&oversized, Some(config.limits.batch_bytes as usize + 1))
                .await
                .expect_err("byte limit")
                .code,
            "configured_batch_byte_limit"
        );

        let too_many_docs = batch(
            "too-many-docs",
            "g1",
            0,
            1,
            vec![
                actor("actor:a", 1, "Harbor Union"),
                actor("actor:b", 1, "Harbor Dock"),
            ],
        );
        assert_eq!(
            service
                .apply_batch(&too_many_docs, None)
                .await
                .expect_err("document limit")
                .code,
            "configured_batch_document_limit"
        );

        reset_pace(&service);
        let first = batch(
            "seed-1",
            "g1",
            0,
            1,
            vec![actor("actor:a", 1, "Harbor Union")],
        );
        let receipt = service
            .apply_batch(&first, None)
            .await
            .expect("commit first batch");
        assert_eq!(receipt.visible_checkpoint, 1);

        let second = batch(
            "paced-2",
            "g1",
            1,
            1,
            vec![actor("actor:b", 1, "Harbor Two")],
        );
        let paced_err = service
            .apply_batch(&second, None)
            .await
            .expect_err("write pacing");
        assert_eq!(paced_err.status, 429);
        assert_eq!(paced_err.code, "write_paced");
        assert_eq!(paced_err.retry_after_secs, Some(1));

        let write_guard = service
            .try_acquire_write()
            .expect("acquire exclusive write");
        assert_eq!(
            service
                .apply_batch(&second, None)
                .await
                .expect_err("busy writer")
                .code,
            "writer_busy"
        );
        drop(write_guard);

        let hits = service
            .query(&ActorIndexQuery {
                query: "Harbor".into(),
                limit: 10,
            })
            .await
            .expect("query succeeds");
        assert_eq!(hits.hits.len(), 1);
        assert_eq!(hits.hits[0].actor_id, "actor:a");

        let q1 = Arc::clone(&service.queries)
            .try_acquire_owned()
            .expect("query slot 1");
        let q2 = Arc::clone(&service.queries)
            .try_acquire_owned()
            .expect("query slot 2");
        let query_busy = service
            .query(&ActorIndexQuery {
                query: "Harbor".into(),
                limit: 10,
            })
            .await
            .expect_err("query capacity enforced");
        assert_eq!(query_busy.status, 429);
        assert_eq!(query_busy.code, "query_capacity");
        drop((q1, q2));

        reset_pace(&service);
        let rebuilt = service
            .start_rebuild(&ActorIndexRebuildStart {
                deployment_id: config.engine.deployment_id.clone(),
                source_epoch: config.engine.source_epoch.clone(),
                request_id: "rebuild-1".into(),
                expected_generation: "g1".into(),
                writer_epoch: 2,
                source_checkpoint: 1,
            })
            .await
            .expect("start rebuild");
        assert_eq!(rebuilt.generation, "g2");

        reset_pace(&service);
        let mut replacement = batch(
            "replace-1",
            "g2",
            1,
            2,
            vec![actor("actor:new", 1, "New Harbor")],
        );
        replacement.writer_epoch = 2;
        service
            .apply_batch(&replacement, None)
            .await
            .expect("seed replacement generation");

        reset_pace(&service);
        let promoted = service
            .promote(&ActorIndexPromote {
                deployment_id: config.engine.deployment_id.clone(),
                source_epoch: config.engine.source_epoch.clone(),
                expected_generation: "g1".into(),
                replacement_generation: "g2".into(),
                writer_epoch: 2,
                catchup_checkpoint: 2,
            })
            .await
            .expect("promote g2");
        assert_eq!(promoted.generation, "g2");

        let status = service.status().await.expect("status before retire");
        let retire = ActorIndexRetire {
            deployment_id: config.engine.deployment_id.clone(),
            source_epoch: config.engine.source_epoch.clone(),
            request_id: "retire-g1".into(),
            expected_generation: "g2".into(),
            generation: "g1".into(),
            expected_manifest_checksum: status.manifest_checksum.expect("manifest checksum"),
            writer_epoch: 2,
        };

        reset_pace(&service);
        let active_reader = Arc::clone(&service.queries)
            .try_acquire_owned()
            .expect("active reader");
        let waiting = service
            .retire(&retire)
            .await
            .expect_err("retire must wait for active readers");
        assert_eq!(waiting.status, 503);
        assert_eq!(waiting.code, "retirement_waits_for_readers");
        assert_eq!(waiting.retry_after_secs, Some(1));
        drop(active_reader);

        reset_pace(&service);
        let retired = service
            .retire(&retire)
            .await
            .expect("retire drains all query permits and succeeds");
        assert_eq!(retired.retained_generations, 0);
    }

    #[tokio::test]
    async fn repair_installs_recovered_engine_when_slot_is_empty() {
        let (_temporary, config) = local_config(ActorIndexLimits::default());
        drop(Engine::open(config.engine.clone()).expect("owned root"));
        let scope = fs::read_dir(&config.engine.directory)
            .expect("scope directory")
            .next()
            .expect("scope entry")
            .expect("entry")
            .path();
        fs::write(scope.join("g1/meta.json"), b"corrupted metadata").expect("corrupt g1");
        assert!(Engine::open(config.engine.clone()).is_err());

        let service = SearchService::new(
            config.clone(),
            Arc::new(EngineSlot::new(None)),
            "index_storage_unavailable",
        )
        .expect("service");
        let pre_status = service.status().await.expect("recovery status");
        assert!(pre_status.recovery_available);

        let repaired = service
            .repair(&ActorIndexRepair {
                deployment_id: config.engine.deployment_id.clone(),
                source_epoch: config.engine.source_epoch.clone(),
                request_id: "repair-1".into(),
                expected_manifest_checksum: pre_status
                    .manifest_checksum
                    .expect("manifest checksum"),
                writer_epoch: 2,
            })
            .await
            .expect("repair installs engine");
        assert_eq!(repaired.active.expect("active generation").generation, "g2");
        assert!(service.engine().snapshot().expect("slot").is_some());
    }
}
