//! Authenticated wake-up deliveries from the Jobs-owned RocksDB queue.

use std::time::Duration;

use anyhow::{anyhow, ensure, Context, Result};
use fleetworks_jobs::queue::{Delivery, QueueCommand, QueueRequest, QueueResponse};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use url::Url;

const RPC_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RESPONSE_BYTES: usize = 128 * 1024;
const VISIBILITY_MS: i64 = 45_000;

#[derive(Clone)]
pub(crate) struct Client {
    http: reqwest::Client,
    endpoint: Url,
    queue: String,
}

impl Client {
    pub(crate) fn from_environment() -> Result<Self> {
        let origin = std::env::var("FLEETWORKS_JOBS_ORIGIN")
            .context("FLEETWORKS_JOBS_ORIGIN is required for Atlas dispatch")?;
        let token = crate::fleetworks::read_secret("FLEETWORKS_JOBS_TOKEN")?;
        let queue =
            std::env::var("FLEETWORKS_QUEUE_NAME").unwrap_or_else(|_| "fleetworks.atlas".into());
        Self::new(&origin, &token, queue)
    }

    pub(crate) fn new(origin: &str, token: &str, queue: String) -> Result<Self> {
        let mut endpoint = Url::parse(origin).context("invalid Jobs origin")?;
        ensure!(
            matches!(endpoint.scheme(), "http" | "https")
                && endpoint.host_str().is_some()
                && endpoint.username().is_empty()
                && endpoint.password().is_none()
                && endpoint.query().is_none()
                && endpoint.fragment().is_none()
                && matches!(endpoint.path(), "" | "/"),
            "Jobs origin must be an exact HTTP(S) origin"
        );
        endpoint.set_path("/internal/jobs/queue");
        let mut bearer =
            HeaderValue::from_str(&format!("Bearer {token}")).context("invalid Jobs bearer")?;
        bearer.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, bearer);
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(RPC_TIMEOUT)
            .build()?;
        Ok(Self {
            http,
            endpoint,
            queue,
        })
    }

    async fn execute(&self, key: String, command: QueueCommand) -> Result<QueueResponse> {
        let request = QueueRequest::new(key, 0, command);
        let mut response = self
            .http
            .post(self.endpoint.clone())
            .json(&request)
            .send()
            .await
            .map_err(|_| anyhow!("Jobs queue request failed"))?;
        ensure!(
            response.status().is_success(),
            "Jobs queue rejected the operation"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow!("Jobs queue response failed"))?
        {
            ensure!(
                bytes.len() + chunk.len() <= MAX_RESPONSE_BYTES,
                "Jobs queue response too large"
            );
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).context("invalid Jobs queue response")
    }

    pub(crate) async fn publish(&self, id: String, payload: Vec<u8>) -> Result<()> {
        let response = self
            .execute(
                format!("publish:{id}"),
                QueueCommand::Publish {
                    queue: self.queue.clone(),
                    id,
                    payload,
                },
            )
            .await?;
        ensure!(
            matches!(response, QueueResponse::Published(_)),
            "unexpected queue publication response"
        );
        Ok(())
    }

    pub(crate) async fn claim(&self, owner: &str) -> Result<Option<Received>> {
        match self
            .execute(
                operation_key(),
                QueueCommand::Claim {
                    queue: self.queue.clone(),
                    owner: owner.into(),
                    visibility_ms: VISIBILITY_MS,
                },
            )
            .await?
        {
            QueueResponse::Delivery(delivery) => Ok(delivery.map(|delivery| Received {
                client: self.clone(),
                delivery,
            })),
            _ => Err(anyhow!("unexpected queue claim response")),
        }
    }
}

fn operation_key() -> String {
    argos_osint_core::store::new_id()
}

pub(crate) enum Disposition {
    Ack,
    Requeue,
    DeadLetter,
}

#[must_use = "settle after the durable domain outcome"]
pub(crate) struct Received {
    client: Client,
    delivery: Delivery,
}

impl Received {
    pub(crate) fn payload(&self) -> &[u8] {
        &self.delivery.payload
    }

    pub(crate) async fn renew(&self) -> Result<bool> {
        match self
            .client
            .execute(
                operation_key(),
                QueueCommand::Renew {
                    token: self.delivery.token.clone(),
                    visibility_ms: VISIBILITY_MS,
                },
            )
            .await?
        {
            QueueResponse::Changed(renewed) => Ok(renewed),
            _ => Err(anyhow!("unexpected queue lease response")),
        }
    }

    pub(crate) async fn settle(self, disposition: Disposition) -> Result<()> {
        let token = self.delivery.token;
        let command = match disposition {
            Disposition::Ack => QueueCommand::Ack { token },
            Disposition::Requeue => QueueCommand::Requeue {
                token,
                delay_ms: 2000,
            },
            Disposition::DeadLetter => QueueCommand::DeadLetter { token },
        };
        let response = self.client.execute(operation_key(), command).await?;
        ensure!(
            matches!(response, QueueResponse::Changed(true)),
            "queue delivery lease was lost before settlement"
        );
        Ok(())
    }
}
