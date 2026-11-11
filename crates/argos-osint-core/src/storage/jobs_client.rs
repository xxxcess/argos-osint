//! @tag:jobs-persistence
//!
//! Authenticated operational-log storage owned by the private Jobs service.
//!
//! Network failures never fall back to writing the domain database. Legacy
//! embedded clients retain the Surreal sink until a Jobs connection is installed.

use std::io::Read;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{anyhow, ensure, Context, Result};
use fleetworks_jobs::Store;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::events::EventRow;

const RESPONSE_LIMIT: usize = 16 * 1024 * 1024;
static BACKEND: OnceLock<Backend> = OnceLock::new();
static QUEUE: OnceLock<fleetworks_jobs::queue::Queue> = OnceLock::new();

/// Installs the queue owned by this Jobs process for readiness checks.
pub fn install_queue(queue: fleetworks_jobs::queue::Queue) -> Result<()> {
    QUEUE
        .set(queue)
        .map_err(|_| anyhow!("Jobs queue already initialized"))
}

enum Backend {
    Local(Store),
    Remote {
        client: reqwest::Client,
        endpoint: Url,
    },
}

/// Private Jobs log commands. Domain lifecycle transactions stay in SurrealDB.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
#[non_exhaustive]
pub enum LogCommand {
    Ready,
    Append {
        event: EventRow,
    },
    List {
        min_rank: Option<i64>,
        app: String,
        job_ids: Vec<String>,
        text: String,
        since: String,
        before_seq: Option<i64>,
        limit: usize,
    },
    Get {
        id: String,
    },
    Counts {
        since: String,
    },
    Apps,
    Prune {
        before: String,
    },
    Clear,
}

/// Installs the sole local file owner in the Jobs process.
pub fn install_local(store: Store) -> Result<()> {
    BACKEND
        .set(Backend::Local(store))
        .map_err(|_| anyhow!("Jobs log backend already initialized"))
}

/// Configures authenticated HTTP access when the Jobs origin is present.
/// Call on a blocking startup task because token files are read synchronously.
pub fn initialize_remote_from_environment() -> Result<()> {
    let origin = match std::env::var("FLEETWORKS_JOBS_ORIGIN") {
        Ok(origin) => origin,
        Err(std::env::VarError::NotPresent) => return Ok(()),
        Err(_) => return Err(anyhow!("invalid FLEETWORKS_JOBS_ORIGIN")),
    };
    let mut endpoint = Url::parse(&origin).context("invalid Jobs origin")?;
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
    endpoint.set_path("/internal/jobs/logs");
    let token = match std::env::var("FLEETWORKS_JOBS_TOKEN") {
        Ok(token) => token,
        Err(std::env::VarError::NotPresent) => {
            let path = std::env::var_os("FLEETWORKS_JOBS_TOKEN_FILE")
                .context("FLEETWORKS_JOBS_TOKEN or FLEETWORKS_JOBS_TOKEN_FILE is required")?;
            let mut token = String::new();
            std::fs::File::open(path)
                .context("open Jobs token file")?
                .take(4097)
                .read_to_string(&mut token)
                .context("read Jobs token file")?;
            ensure!(token.len() <= 4096, "Jobs token file is too large");
            token.trim_end_matches(['\r', '\n']).to_owned()
        }
        Err(_) => return Err(anyhow!("invalid FLEETWORKS_JOBS_TOKEN")),
    };
    ensure!(
        (32..=256).contains(&token.len())
            && token
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-._~".contains(&c)),
        "invalid Jobs token"
    );
    let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))?;
    authorization.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, authorization);
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .build()?;
    BACKEND
        .set(Backend::Remote { client, endpoint })
        .map_err(|_| anyhow!("Jobs log backend already initialized"))
}

/// Whether operational logs have been assigned to the Jobs service.
pub fn configured() -> bool {
    BACKEND.get().is_some()
}

/// Executes one log command, returning only after its durable store operation.
pub async fn execute(command: LogCommand) -> Result<Value> {
    match BACKEND
        .get()
        .context("Jobs log backend is not configured")?
    {
        Backend::Local(store) => execute_local(store, command).await,
        Backend::Remote { client, endpoint } => {
            let mut response = client
                .post(endpoint.clone())
                .json(&command)
                .send()
                .await
                .context("Jobs log service unavailable")?;
            ensure!(
                response.status().is_success(),
                "Jobs log service rejected operation ({})",
                response.status()
            );
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.context("read Jobs log response")? {
                ensure!(
                    bytes.len().saturating_add(chunk.len()) <= RESPONSE_LIMIT,
                    "Jobs log response exceeds size limit"
                );
                bytes.extend_from_slice(&chunk);
            }
            serde_json::from_slice(&bytes).context("invalid Jobs log response")
        }
    }
}

/// Executes against the Jobs process-owned store without remote dispatch.
pub async fn execute_local(store: &Store, command: LogCommand) -> Result<Value> {
    use fleetworks_jobs::logs::LogFilter;
    match command {
        LogCommand::Ready => {
            store.log_get("__jobs_readiness__".into()).await?;
            if let Some(queue) = QUEUE.get() {
                use fleetworks_jobs::queue::{QueueCommand, QueueRequest};
                queue
                    .execute(QueueRequest::new(
                        "",
                        chrono::Utc::now().timestamp_millis(),
                        QueueCommand::DeadLetters {
                            queue: "fleetworks.atlas".into(),
                            limit: 1,
                        },
                    ))
                    .await?;
            }
            Ok(serde_json::json!({"ready":true}))
        }
        LogCommand::Append { mut event } => {
            event.message = crate::events::bound(
                &crate::events::redact(&event.message),
                crate::events::MAX_MESSAGE_CHARS,
            );
            event.details = crate::events::bound(
                &crate::events::redact(&event.details),
                crate::events::MAX_DETAIL_CHARS,
            );
            Ok(serde_json::to_value(
                store.log_append(serde_json::to_value(event)?).await?,
            )?)
        }
        LogCommand::List {
            min_rank,
            app,
            job_ids,
            text,
            since,
            before_seq,
            limit,
        } => {
            let mut filter = LogFilter::default();
            filter.min_rank = min_rank;
            filter.app = app;
            filter.job_ids = job_ids;
            filter.text = text;
            filter.since = since;
            filter.before_seq = before_seq;
            filter.limit = limit;
            Ok(serde_json::to_value(store.log_list(filter).await?)?)
        }
        LogCommand::Get { id } => Ok(serde_json::to_value(store.log_get(id).await?)?),
        LogCommand::Counts { since } => store.log_counts(since).await,
        LogCommand::Apps => Ok(serde_json::to_value(store.log_apps().await?)?),
        LogCommand::Prune { before } => {
            Ok(serde_json::to_value(store.log_prune(Some(before)).await?)?)
        }
        LogCommand::Clear => Ok(serde_json::to_value(store.log_prune(None).await?)?),
    }
}
