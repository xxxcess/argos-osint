//! One durable database-leased coordinator for the private actor index.
//! There is no broker, CSV indexer, detached writer or application retry layer.
//! Source progress advances only after the serving reader's matching receipt.

use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;

#[path = "actor_index_rebuild.rs"]
mod rebuild;

use argos_osint_api::actor_search::ActorSearchBackend;
use argos_osint_api::actor_search::SearchFuture;
use argos_osint_core::storage::actor_search_outbox;
use argos_osint_core::storage::actor_search_outbox::Preparation;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::tx::TxError;
use chrono::Utc;
use fleetworks_contracts::actor_index::ActorIndexBatch;
use fleetworks_contracts::actor_index::ActorIndexQuery;
use fleetworks_contracts::actor_index::ActorIndexQueryResponse;
use fleetworks_contracts::actor_index::ActorIndexReceipt;
use fleetworks_contracts::actor_index::ActorIndexStatus;
use fleetworks_contracts::actor_index::ACTOR_INDEX_ANALYZER;
use fleetworks_contracts::actor_index::ACTOR_INDEX_PROTOCOL;
use fleetworks_contracts::actor_index::ACTOR_INDEX_SCHEMA;
use fleetworks_search_index::service::SearchService;
use fleetworks_search_index::service::ServiceError;
use reqwest::header;
use reqwest::header::HeaderValue;
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use url::Url;

const MAX_RESPONSE: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TickReport {
    Waiting,
    Visible { checkpoint: u64 },
    Deferred { correction_required: bool },
}

enum IndexTransport {
    Http {
        origin: Url,
        reader: HeaderValue,
        writer: HeaderValue,
        http: reqwest::Client,
    },
    Local(Arc<SearchService>),
}

/// In-process [`ActorSearchBackend`] backed directly by a [`SearchService`].
pub struct LocalActorSearchBackend(pub Arc<SearchService>);

impl ActorSearchBackend for LocalActorSearchBackend {
    fn status(&self) -> SearchFuture<'_, ActorIndexStatus> {
        Box::pin(async move {
            self.0
                .status()
                .await
                .map_err(|error| anyhow::anyhow!("actor index status error: {}", error.code))
        })
    }

    fn query<'a>(
        &'a self,
        query: &'a ActorIndexQuery,
    ) -> SearchFuture<'a, ActorIndexQueryResponse> {
        Box::pin(async move {
            query.validate()?;
            self.0
                .query(query)
                .await
                .map_err(|error| anyhow::anyhow!("actor index query error: {}", error.code))
        })
    }
}

/// The host owns this value and calls `tick` sequentially. Replica exclusion,
/// pacing, retry delay and the exact pending body are durable database state.
pub struct ActorIndexWorker {
    transport: IndexTransport,
    owner: String,
    status: Option<(Instant, ActorIndexStatus)>,
    deferred_initial_seed: bool,
}

fn validate_owner(owner: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !owner.trim().is_empty() && owner.len() <= 256 && !owner.chars().any(char::is_control),
        "invalid index coordinator identity"
    );
    Ok(())
}

impl ActorIndexWorker {
    pub fn new(
        origin: &str,
        read_token: &str,
        write_token: &str,
        owner: String,
    ) -> anyhow::Result<Self> {
        let origin = Url::parse(&argos_osint_api::network_origin(origin)?)?;
        validate_owner(&owner)?;
        anyhow::ensure!(
            read_token != write_token,
            "index read and write credentials must differ"
        );
        let reader = bearer(read_token)?;
        let writer = bearer(write_token)?;
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(35))
            .build()?;
        Ok(Self {
            transport: IndexTransport::Http {
                origin,
                reader,
                writer,
                http,
            },
            owner,
            status: None,
            deferred_initial_seed: false,
        })
    }

    /// Construct a coordinator talking directly to an in-process [`SearchService`].
    pub fn local(service: Arc<SearchService>, owner: String) -> anyhow::Result<Self> {
        validate_owner(&owner)?;
        Ok(Self {
            transport: IndexTransport::Local(service),
            owner,
            status: None,
            deferred_initial_seed: false,
        })
    }

    /// Release any held coordinator lease on clean worker shutdown.
    pub async fn release(&self, repo: &Repository) -> anyhow::Result<bool> {
        Ok(actor_search_outbox::release_lease(repo, &self.owner).await?)
    }

    async fn status(&mut self) -> Result<ActorIndexStatus, DeliveryFailure> {
        if let Some((observed, status)) = &self.status {
            if observed.elapsed() < Duration::from_secs(5) {
                return Ok(status.clone());
            }
        }
        let status: ActorIndexStatus = match &self.transport {
            IndexTransport::Http {
                origin,
                reader,
                http,
                ..
            } => {
                let mut endpoint = origin.clone();
                endpoint.set_path("/internal/v1/actors/status");
                let response = http
                    .get(endpoint)
                    .header(header::AUTHORIZATION, reader.clone())
                    .timeout(Duration::from_secs(8))
                    .send()
                    .await
                    .map_err(|_| DeliveryFailure::transient())?;
                decode(response).await?
            }
            IndexTransport::Local(service) => service.status().await.map_err(map_service_error)?,
        };
        if status.protocol != ACTOR_INDEX_PROTOCOL
            || status.schema_version != ACTOR_INDEX_SCHEMA
            || status.analyzer_version != ACTOR_INDEX_ANALYZER
        {
            return Err(DeliveryFailure::permanent());
        }
        if let Some(pressure) = status
            .pressure
            .as_ref()
            .filter(|_| matches!(&self.transport, IndexTransport::Local(_)))
        {
            let previous = self
                .status
                .as_ref()
                .and_then(|(_, status)| status.pressure.as_ref());
            if previous.is_none_or(|previous| {
                previous.paused != pressure.paused || previous.reason_code != pressure.reason_code
            }) {
                tracing::info!(
                    code = "embedded_actor_index_pressure_transition",
                    paused = pressure.paused,
                    reason = pressure.reason_code.as_deref().unwrap_or("none"),
                    resident_bytes = pressure.resident_bytes.unwrap_or(0),
                    disk_free_bytes = pressure.disk_free_bytes.unwrap_or(0),
                    "Observed an Embedded actor index pressure change"
                );
            }
        }
        self.status = Some((Instant::now(), status.clone()));
        Ok(status)
    }

    async fn deliver(&self, batch: &ActorIndexBatch) -> Result<ActorIndexReceipt, DeliveryFailure> {
        match &self.transport {
            IndexTransport::Http {
                origin,
                writer,
                http,
                ..
            } => {
                let mut endpoint = origin.clone();
                endpoint.set_path("/internal/v1/actors/batches");
                let response = http
                    .post(endpoint)
                    .header(header::AUTHORIZATION, writer.clone())
                    .json(batch)
                    .send()
                    .await
                    .map_err(|_| DeliveryFailure::transient())?;
                decode(response).await
            }
            IndexTransport::Local(service) => {
                let raw_len = serde_json::to_vec(batch).ok().map(|bytes| bytes.len());
                service
                    .apply_batch(batch, raw_len)
                    .await
                    .map_err(map_service_error)
            }
        }
    }

    /// At most one source page, one persisted prepared batch and one HTTP write
    /// in flight. The DB grants no more than two starts per second globally.
    pub async fn tick(&mut self, repo: &Repository) -> anyhow::Result<TickReport> {
        let Some(claim) = actor_search_outbox::claim(repo, &self.owner, Utc::now()).await? else {
            return Ok(TickReport::Waiting);
        };
        let status = match self.status().await {
            Ok(status) => status,
            Err(error) => {
                self.status = None;
                actor_search_outbox::defer(
                    repo,
                    &claim,
                    error.retry_after,
                    error.permanent,
                    Utc::now(),
                )
                .await?;
                return Ok(TickReport::Deferred {
                    correction_required: error.permanent,
                });
            }
        };
        actor_search_outbox::observe_resources(repo, &claim, &status, Utc::now()).await?;
        let embedded = matches!(&self.transport, IndexTransport::Local(_));
        if embedded
            && status
                .pressure
                .as_ref()
                .is_some_and(|pressure| pressure.paused)
        {
            // Preserve any pending receipt and avoid allocating a new projection
            // while the co-located writer is waiting for resources.
            return Ok(TickReport::Waiting);
        }
        let embedded_catch_up = embedded
            && status
                .active
                .as_ref()
                .is_some_and(|active| !active.source_complete);
        let defer_initial_seed = embedded
            && status.rebuilding.is_none()
            && status
                .active
                .as_ref()
                .is_some_and(|active| active.visible_checkpoint == 0 && !active.source_complete)
            && actor_search_outbox::defer_initial_seed_index(
                repo,
                status.limits.backlog_resume_events,
            )
            .await?;
        if defer_initial_seed {
            if !self.deferred_initial_seed {
                tracing::info!(
                    code = "embedded_actor_index_seed_deferred",
                    "Holding initial actor index writes while the catalog commits"
                );
            }
            self.deferred_initial_seed = true;
            return Ok(TickReport::Waiting);
        }
        self.deferred_initial_seed = false;
        match actor_search_outbox::next_rebuild(repo, &self.owner, &status, Utc::now()).await {
            Ok(Some(action)) => return self.rebuild_tick(repo, action).await,
            Ok(None) => (),
            Err(TxError::Abort(reason)) => {
                tracing::warn!(stage="rebuild_prepare",code=%safe_code(&reason).unwrap_or("actor_index_abort"),"Actor index rebuild preparation failed");
                let permanent = reason != "actor_index_service_unavailable";
                self.status = None;
                actor_search_outbox::defer(repo, &claim, None, permanent, Utc::now()).await?;
                return Ok(TickReport::Deferred {
                    correction_required: permanent,
                });
            }
            Err(error) => return Err(error.into()),
        }
        let prepare_started = Instant::now();
        let preparation = match actor_search_outbox::prepare_with_embedded_policy(
            repo,
            &self.owner,
            &status,
            Utc::now(),
            embedded,
        )
        .await
        {
            Ok(preparation) => preparation,
            Err(TxError::Abort(reason)) => {
                // Known source/schema/generation/oversize conflicts require a
                // reviewed correction. Unavailable service state is retried.
                let permanent = reason != "actor_index_service_unavailable";
                self.status = None;
                actor_search_outbox::defer(repo, &claim, None, permanent, Utc::now()).await?;
                return Ok(TickReport::Deferred {
                    correction_required: permanent,
                });
            }
            Err(error) => return Err(error.into()),
        };
        let Preparation::Batch { lease, prepared } = preparation else {
            return Ok(TickReport::Waiting);
        };
        let prepare_ms = prepare_started.elapsed().as_millis() as u64;
        let write_started = Instant::now();
        let result = {
            let delivery = self.deliver(&prepared.batch);
            tokio::pin!(delivery);
            let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
            heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            heartbeat.tick().await;
            loop {
                tokio::select! {
                    result = &mut delivery => break result,
                    _ = heartbeat.tick() => actor_search_outbox::renew(repo, &lease, Utc::now()).await?,
                }
            }
        };
        let write_ms = write_started.elapsed().as_millis() as u64;
        match result {
            Ok(receipt) => {
                let acknowledge_started = Instant::now();
                if let Err(error) = actor_search_outbox::acknowledge_with_retirement(
                    repo,
                    &lease,
                    &receipt,
                    Utc::now(),
                    embedded_catch_up,
                )
                .await
                {
                    self.status = None;
                    if matches!(&error,TxError::Abort(reason) if reason=="actor_index_receipt_mismatch" || reason=="actor_index_checksum")
                    {
                        actor_search_outbox::defer(repo, &lease, None, true, Utc::now()).await?;
                        return Ok(TickReport::Deferred {
                            correction_required: true,
                        });
                    }
                    return Err(error.into());
                }
                if embedded_catch_up {
                    tracing::info!(
                        code = "embedded_actor_index_batch_elapsed",
                        prepare_ms,
                        write_ms,
                        ack_ms = acknowledge_started.elapsed().as_millis() as u64,
                        mutations = prepared.batch.mutations.len(),
                        checkpoint = receipt.visible_checkpoint,
                        "Embedded initial actor index batch acknowledged"
                    );
                }
                if let Some((_, status)) = &mut self.status {
                    if let Some(active) = &mut status.active {
                        active.committed_checkpoint = receipt.committed_checkpoint;
                        active.visible_checkpoint = receipt.visible_checkpoint;
                        active.writer_epoch = receipt.writer_epoch;
                        active.source_complete = prepared.batch.source_complete;
                    }
                }
                Ok(TickReport::Visible {
                    checkpoint: receipt.visible_checkpoint,
                })
            }
            Err(error) => {
                self.status = None;
                actor_search_outbox::defer(
                    repo,
                    &lease,
                    error.retry_after,
                    error.permanent,
                    Utc::now(),
                )
                .await?;
                Ok(TickReport::Deferred {
                    correction_required: error.permanent,
                })
            }
        }
    }
}

fn bearer(token: &str) -> anyhow::Result<HeaderValue> {
    anyhow::ensure!(
        (32..=256).contains(&token.len())
            && token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-._~+/=".contains(&byte)),
        "invalid private index credential"
    );
    let mut value = HeaderValue::from_str(&format!("Bearer {token}"))?;
    value.set_sensitive(true);
    Ok(value)
}

struct DeliveryFailure {
    permanent: bool,
    retry_after: Option<Duration>,
    status: Option<StatusCode>,
    code: Option<String>,
}

impl DeliveryFailure {
    fn transient() -> Self {
        Self {
            permanent: false,
            retry_after: None,
            status: None,
            code: None,
        }
    }
    fn permanent() -> Self {
        Self {
            permanent: true,
            retry_after: None,
            status: None,
            code: None,
        }
    }
}

fn is_transient_status(status: StatusCode) -> bool {
    status.is_server_error()
        || matches!(
            status,
            StatusCode::TOO_MANY_REQUESTS | StatusCode::REQUEST_TIMEOUT
        )
}

fn map_service_error(error: ServiceError) -> DeliveryFailure {
    let status = StatusCode::from_u16(error.status).ok();
    let permanent = !status.is_some_and(is_transient_status);
    DeliveryFailure {
        permanent,
        retry_after: error.retry_after_secs.map(Duration::from_secs),
        status,
        code: safe_code(error.code).map(str::to_owned),
    }
}

fn safe_code(code: &str) -> Option<&str> {
    (1..=64)
        .contains(&code.len())
        .then_some(code)
        .filter(|code| {
            code.bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        })
}

fn safe_service_code(body: &[u8]) -> Option<String> {
    let body: serde_json::Value = serde_json::from_slice(body).ok()?;
    let code = body.get("error")?.get("code")?.as_str()?;
    safe_code(code).map(str::to_owned)
}

fn tx_error_code(error: &TxError) -> String {
    match error {
        TxError::Abort(reason) => safe_code(reason).unwrap_or("actor_index_abort").to_owned(),
        TxError::Contention { .. } => "storage_contention".into(),
        TxError::Db(_) => "storage_error".into(),
        TxError::PublisherStopped => "publisher_stopped".into(),
        TxError::PublisherDropped => "publisher_dropped".into(),
    }
}

fn rebuild_ack_requires_correction(error: &TxError) -> bool {
    matches!(error, TxError::Abort(reason) if reason != "actor_index_lease_lost")
}

async fn decode<T: DeserializeOwned + Send + 'static>(
    mut response: reqwest::Response,
) -> Result<T, DeliveryFailure> {
    let status = response.status();
    if !status.is_success() {
        let retry_after = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| retry_after(value, SystemTime::now()));
        let permanent = !is_transient_status(status);
        let mut body = Vec::new();
        while let Ok(Some(chunk)) = response.chunk().await {
            if chunk.len() > 1024 - body.len() {
                break;
            }
            body.extend_from_slice(&chunk);
        }
        return Err(DeliveryFailure {
            permanent,
            retry_after,
            status: Some(status),
            code: safe_service_code(&body),
        });
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| DeliveryFailure::transient())?
    {
        if chunk.len() > MAX_RESPONSE - bytes.len() {
            return Err(DeliveryFailure::permanent());
        }
        bytes.extend_from_slice(&chunk);
    }
    tokio::task::spawn_blocking(move || serde_json::from_slice(&bytes))
        .await
        .map_err(|_| DeliveryFailure::transient())?
        .map_err(|_| DeliveryFailure::permanent())
}

fn retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    httpdate::parse_http_date(value)
        .ok()
        .map(|until| until.duration_since(now).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuild_ack_retries_lost_lease_but_blocks_invalid_receipt() {
        let lost_lease = TxError::Abort("actor_index_lease_lost".into());
        let bad_receipt = TxError::Abort("actor_index_rebuild_receipt_mismatch".into());
        assert!(!rebuild_ack_requires_correction(&lost_lease));
        assert!(!rebuild_ack_requires_correction(&TxError::Contention {
            attempts: 6
        }));
        assert!(rebuild_ack_requires_correction(&bad_receipt));
        assert_eq!(tx_error_code(&lost_lease), "actor_index_lease_lost");
    }

    #[test]
    fn coordinator_log_codes_are_bounded_and_do_not_echo_untrusted_values() {
        assert_eq!(
            safe_service_code(br#"{"error":{"code":"checkpoint_gap_or_expired_receipt"}}"#),
            Some("checkpoint_gap_or_expired_receipt".into())
        );
        assert_eq!(
            safe_service_code(br#"{"error":{"code":"Bearer secret"}}"#),
            None
        );
        assert_eq!(
            tx_error_code(&TxError::Abort("secret\nvalue".into())),
            "actor_index_abort"
        );
    }

    #[test]
    fn retry_after_preserves_long_valid_delay_and_http_dates() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert_eq!(retry_after("86400", now), Some(Duration::from_secs(86_400)));
        assert_eq!(
            retry_after("18446744073709551615", now),
            Some(Duration::from_secs(u64::MAX))
        );
        let date = httpdate::fmt_http_date(now + Duration::from_secs(3600));
        assert_eq!(retry_after(&date, now), Some(Duration::from_secs(3600)));
        assert_eq!(
            retry_after(&httpdate::fmt_http_date(now - Duration::from_secs(10)), now),
            Some(Duration::ZERO)
        );
        assert_eq!(retry_after("later", now), None);
    }

    #[test]
    fn service_error_mapping_preserves_transient_retry_after_and_permanent_codes() {
        let busy = map_service_error(ServiceError::busy("writer_busy"));
        assert!(!busy.permanent);
        assert_eq!(busy.status, Some(StatusCode::SERVICE_UNAVAILABLE));
        assert_eq!(busy.retry_after, Some(Duration::from_secs(1)));
        assert_eq!(busy.code.as_deref(), Some("writer_busy"));

        let paused = map_service_error(ServiceError::pressure_paused("resource_pressure_paused"));
        assert!(!paused.permanent);
        assert_eq!(paused.status, Some(StatusCode::SERVICE_UNAVAILABLE));
        assert_eq!(paused.retry_after, Some(Duration::from_secs(5)));

        let conflict = map_service_error(ServiceError {
            status: 409,
            code: "checkpoint_gap_or_expired_receipt",
            retry_after_secs: None,
        });
        assert!(conflict.permanent);
        assert_eq!(conflict.status, Some(StatusCode::CONFLICT));
        assert_eq!(
            conflict.code.as_deref(),
            Some("checkpoint_gap_or_expired_receipt")
        );
    }
}
