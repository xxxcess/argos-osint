//! One durable database-leased coordinator for the private actor index.
//! There is no broker, CSV indexer, detached writer or application retry layer.
//! Source progress advances only after the serving reader's matching receipt.

use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;

#[path = "actor_index_rebuild.rs"]
mod rebuild;

use argos_osint_core::storage::actor_search_outbox;
use argos_osint_core::storage::actor_search_outbox::Preparation;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::tx::TxError;
use chrono::Utc;
use fleetworks_contracts::actor_index::ActorIndexBatch;
use fleetworks_contracts::actor_index::ActorIndexReceipt;
use fleetworks_contracts::actor_index::ActorIndexStatus;
use fleetworks_contracts::actor_index::ACTOR_INDEX_ANALYZER;
use fleetworks_contracts::actor_index::ACTOR_INDEX_PROTOCOL;
use fleetworks_contracts::actor_index::ACTOR_INDEX_SCHEMA;
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

/// The host owns this value and calls `tick` sequentially. Replica exclusion,
/// pacing, retry delay and the exact pending body are durable database state.
pub struct ActorIndexWorker {
    origin: Url,
    reader: HeaderValue,
    writer: HeaderValue,
    http: reqwest::Client,
    owner: String,
    status: Option<(Instant, ActorIndexStatus)>,
}

impl ActorIndexWorker {
    pub fn new(
        origin: &str,
        read_token: &str,
        write_token: &str,
        owner: String,
    ) -> anyhow::Result<Self> {
        let origin = Url::parse(&argos_osint_api::network_origin(origin)?)?;
        anyhow::ensure!(
            !owner.trim().is_empty() && owner.len() <= 256 && !owner.chars().any(char::is_control),
            "invalid index coordinator identity"
        );
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
            origin,
            reader,
            writer,
            http,
            owner,
            status: None,
        })
    }

    fn endpoint(&self, path: &str) -> Url {
        let mut endpoint = self.origin.clone();
        endpoint.set_path(path);
        endpoint
    }

    async fn status(&mut self) -> Result<ActorIndexStatus, DeliveryFailure> {
        if let Some((observed, status)) = &self.status {
            if observed.elapsed() < Duration::from_secs(5) {
                return Ok(status.clone());
            }
        }
        let response = self
            .http
            .get(self.endpoint("/internal/v1/actors/status"))
            .header(header::AUTHORIZATION, self.reader.clone())
            .timeout(Duration::from_secs(8))
            .send()
            .await
            .map_err(|_| DeliveryFailure::transient())?;
        let status: ActorIndexStatus = decode(response).await?;
        if status.protocol != ACTOR_INDEX_PROTOCOL
            || status.schema_version != ACTOR_INDEX_SCHEMA
            || status.analyzer_version != ACTOR_INDEX_ANALYZER
        {
            return Err(DeliveryFailure::permanent());
        }
        self.status = Some((Instant::now(), status.clone()));
        Ok(status)
    }

    async fn deliver(&self, batch: &ActorIndexBatch) -> Result<ActorIndexReceipt, DeliveryFailure> {
        let response = self
            .http
            .post(self.endpoint("/internal/v1/actors/batches"))
            .header(header::AUTHORIZATION, self.writer.clone())
            .json(batch)
            .send()
            .await
            .map_err(|_| DeliveryFailure::transient())?;
        decode(response).await
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
        match actor_search_outbox::next_rebuild(repo, &self.owner, &status, Utc::now()).await {
            Ok(Some(action)) => return self.rebuild_tick(repo, action).await,
            Ok(None) => (),
            Err(TxError::Abort(reason)) => {
                let permanent = reason != "actor_index_service_unavailable";
                self.status = None;
                actor_search_outbox::defer(repo, &claim, None, permanent, Utc::now()).await?;
                return Ok(TickReport::Deferred {
                    correction_required: permanent,
                });
            }
            Err(error) => return Err(error.into()),
        }
        let preparation =
            match actor_search_outbox::prepare(repo, &self.owner, &status, Utc::now()).await {
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
        match result {
            Ok(receipt) => {
                if let Err(error) =
                    actor_search_outbox::acknowledge(repo, &lease, &receipt, Utc::now()).await
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
}

impl DeliveryFailure {
    fn transient() -> Self {
        Self {
            permanent: false,
            retry_after: None,
        }
    }
    fn permanent() -> Self {
        Self {
            permanent: true,
            retry_after: None,
        }
    }
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
        let permanent = !(status.is_server_error()
            || matches!(
                status,
                StatusCode::TOO_MANY_REQUESTS | StatusCode::REQUEST_TIMEOUT
            ));
        return Err(DeliveryFailure {
            permanent,
            retry_after,
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
}
