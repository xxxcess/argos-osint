//! Private Fleetworks transport for the existing native annotation engine.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{HeaderValue, AUTHORIZATION};
use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;
use url::Url;

use super::cache;
use super::{Annotation, ParseContext, Service, Source, Status};

/// Private NLP transport version, separate from the engine's annotation schema.
pub const VERSION: u16 = 1;
/// Service readiness route.
pub const READY_PATH: &str = "/api/v1/health/ready";
/// Ordered batch annotation route.
pub const ANNOTATE_PATH: &str = "/api/v1/nlp/annotate";
/// Maximum wire JSON bytes before extraction.
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
/// Maximum JSON bytes sent for one annotation batch.
pub const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_ITEMS: usize = 8;
const MAX_ITEM_TEXT: usize = 64 * 1024;
const MAX_BATCH_TEXT: usize = 128 * 1024;

/// Ordered annotations requested from one engine process.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BatchRequest {
    version: u16,
    items: Vec<Item>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Item {
    prose: String,
    prose_hash: String,
    language_hint: Option<String>,
    reference_date: String,
    timezone: String,
    date_order: String,
}

impl Item {
    fn from_source(source: &Source) -> Self {
        let context = source.context();
        Self {
            prose: source.prose.clone(),
            prose_hash: source.prose_hash.clone(),
            language_hint: source.language_hint.clone(),
            reference_date: context.reference_date,
            timezone: context.timezone,
            date_order: context.date_order,
        }
    }

    fn into_source(self) -> (Source, ParseContext) {
        (
            Source {
                source_id: "fleetworks-nlp".into(),
                revision: self.prose_hash.clone(),
                attempt_id: "remote-annotation".into(),
                raw_hash: None,
                prose_hash: self.prose_hash,
                prose: self.prose,
                language_hint: self.language_hint,
                published_at: None,
                retrieved_at: self.reference_date.clone(),
            },
            ParseContext {
                reference_date: self.reference_date,
                timezone: self.timezone,
                date_order: self.date_order,
            },
        )
    }

    fn valid(&self) -> bool {
        fn valid_field(value: &str, limit: usize) -> bool {
            value.len() <= limit && !value.chars().any(char::is_control)
        }
        self.prose.len() <= MAX_ITEM_TEXT
            && self.prose_hash.len() == 64
            && cache::hash(&self.prose) == self.prose_hash
            && self
                .language_hint
                .as_deref()
                .is_none_or(|value| valid_field(value, 128))
            && valid_field(&self.reference_date, 40)
            && valid_field(&self.timezone, 96)
            && valid_field(&self.date_order, 24)
    }
}

/// Response items have exactly the order of the request.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BatchResponse {
    version: u16,
    annotations: Vec<Annotation>,
}

impl BatchRequest {
    fn new(sources: &[&Source]) -> Result<Self, RemoteError> {
        if sources.is_empty()
            || sources.len() > MAX_ITEMS
            || sources
                .iter()
                .any(|source| source.prose.len() > MAX_ITEM_TEXT)
            || sources
                .iter()
                .map(|source| source.prose.len())
                .sum::<usize>()
                > MAX_BATCH_TEXT
        {
            return Err(RemoteError::Input);
        }
        Ok(Self {
            version: VERSION,
            items: sources
                .iter()
                .map(|source| Item::from_source(source))
                .collect(),
        })
    }

    fn validate(&self) -> Result<(), RemoteError> {
        if self.version != VERSION
            || self.items.is_empty()
            || self.items.len() > MAX_ITEMS
            || self.items.iter().any(|item| !item.valid())
            || self
                .items
                .iter()
                .map(|item| item.prose.len())
                .sum::<usize>()
                > MAX_BATCH_TEXT
        {
            return Err(RemoteError::Input);
        }
        Ok(())
    }

    /// Runs the batch through one already warm local engine. A request whose
    /// metadata or hashes are invalid never enters the worker queue.
    pub async fn annotate(
        self,
        service: &Service,
        cancel: Arc<AtomicBool>,
    ) -> Result<BatchResponse, RemoteError> {
        self.validate()?;
        let mut annotations = Vec::with_capacity(self.items.len());
        for item in self.items {
            let (source, context) = item.into_source();
            let mut annotation = if cancel.load(Ordering::Acquire) {
                Annotation::terminal(&source, &context, Status::Cancelled)
            } else {
                let failed = Annotation::terminal(&source, &context, Status::Failed);
                match service.try_submit(source, context, cancel.clone()) {
                    Ok(receiver) => receiver.await.unwrap_or(failed),
                    Err(status) => Annotation { status, ..failed },
                }
            };
            if annotation.status != Status::Complete {
                // Internal interpreter failures can contain article prose.
                annotation.flags.clear();
            }
            annotations.push(annotation);
        }
        Ok(BatchResponse {
            version: VERSION,
            annotations,
        })
    }
}

/// Stable transport failure without URLs, credentials, or response contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RemoteError {
    Configuration,
    Input,
    Cancelled,
    Busy,
    Timeout,
    Unavailable,
    Authentication,
    InvalidResponse,
}

impl RemoteError {
    fn diagnostic(self) -> &'static str {
        match self {
            Self::Configuration => "remote_nlp_configuration",
            Self::Input => "remote_nlp_invalid_input",
            Self::Cancelled => "remote_nlp_cancelled",
            Self::Busy => "remote_nlp_busy",
            Self::Timeout => "remote_nlp_timeout",
            Self::Unavailable => "remote_nlp_unavailable",
            Self::Authentication => "remote_nlp_authentication",
            Self::InvalidResponse => "remote_nlp_invalid_response",
        }
    }

    fn status(self) -> Status {
        match self {
            Self::Cancelled => Status::Cancelled,
            Self::Busy => Status::Busy,
            Self::Timeout => Status::Timeout,
            _ => Status::Failed,
        }
    }
}

/// One direct, authenticated connection pool for the private service.
pub struct Client {
    endpoint: Url,
    authorization: HeaderValue,
    http: reqwest::Client,
}

impl Client {
    /// Validates a fixed HTTP(S) origin and keeps credentials on that origin.
    pub fn new(origin: &str, token: &str) -> Result<Self, RemoteError> {
        let origin = Url::parse(origin).map_err(|_| RemoteError::Configuration)?;
        if !matches!(origin.scheme(), "http" | "https")
            || origin.host_str().is_none()
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.path() != "/"
            || origin.query().is_some()
            || origin.fragment().is_some()
            || !(32..=256).contains(&token.len())
            || token.trim_end_matches('=').is_empty()
            || !token.trim_end_matches('=').bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/')
            })
        {
            return Err(RemoteError::Configuration);
        }
        let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| RemoteError::Configuration)?;
        authorization.set_sensitive(true);
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(40))
            .build()
            .map_err(|_| RemoteError::Configuration)?;
        Ok(Self {
            endpoint: origin
                .join(ANNOTATE_PATH)
                .map_err(|_| RemoteError::Configuration)?,
            authorization,
            http,
        })
    }

    /// Fetches one ordered batch, verifying identity and span offsets before use.
    pub async fn annotate(
        &self,
        sources: &[&Source],
        cancel: &AtomicBool,
    ) -> Result<Vec<Annotation>, RemoteError> {
        if cancel.load(Ordering::Acquire) {
            return Err(RemoteError::Cancelled);
        }
        let request = BatchRequest::new(sources)?;
        tokio::select! {
            biased;
            _ = wait_for_cancel(cancel) => Err(RemoteError::Cancelled),
            result = self.request(&request, sources) => result,
        }
    }

    async fn request(
        &self,
        request: &BatchRequest,
        sources: &[&Source],
    ) -> Result<Vec<Annotation>, RemoteError> {
        let mut reply = self
            .http
            .post(self.endpoint.clone())
            .header(AUTHORIZATION, self.authorization.clone())
            .json(request)
            .send()
            .await
            .map_err(transport)?;
        match reply.status().as_u16() {
            200 => {}
            400 | 413 | 422 => return Err(RemoteError::Input),
            401 | 403 => return Err(RemoteError::Authentication),
            429 => return Err(RemoteError::Busy),
            408 => return Err(RemoteError::Timeout),
            _ => return Err(RemoteError::Unavailable),
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = reply.chunk().await.map_err(transport)? {
            if chunk.len() > MAX_RESPONSE_BYTES - bytes.len() {
                return Err(RemoteError::InvalidResponse);
            }
            bytes.extend_from_slice(&chunk);
        }
        let batch: BatchResponse =
            serde_json::from_slice(&bytes).map_err(|_| RemoteError::InvalidResponse)?;
        if batch.version != VERSION || batch.annotations.len() != sources.len() {
            return Err(RemoteError::InvalidResponse);
        }
        for (annotation, source) in batch.annotations.iter().zip(sources) {
            if !annotation.validate(source)
                || annotation.feature_key != cache::feature_key(source, &source.context())
            {
                return Err(RemoteError::InvalidResponse);
            }
        }
        Ok(batch.annotations)
    }
}

fn transport(error: reqwest::Error) -> RemoteError {
    if error.is_timeout() {
        RemoteError::Timeout
    } else {
        RemoteError::Unavailable
    }
}

async fn wait_for_cancel(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Whether the operator explicitly selected the private NLP service.
pub fn is_configured() -> bool {
    std::env::var_os("FLEETWORKS_NLP_ORIGIN").is_some()
}

fn client_from_environment() -> Result<Client, RemoteError> {
    use std::io::Read;
    let origin = std::env::var("FLEETWORKS_NLP_ORIGIN").map_err(|_| RemoteError::Configuration)?;
    let token = if let Some(value) = std::env::var_os("FLEETWORKS_INTERNAL_TOKEN") {
        value
            .into_string()
            .map_err(|_| RemoteError::Configuration)?
    } else {
        let path =
            std::env::var_os("FLEETWORKS_INTERNAL_TOKEN_FILE").ok_or(RemoteError::Configuration)?;
        let file = std::fs::File::open(path).map_err(|_| RemoteError::Configuration)?;
        let mut token = String::new();
        file.take(259)
            .read_to_string(&mut token)
            .map_err(|_| RemoteError::Configuration)?;
        if token.len() > 258 {
            return Err(RemoteError::Configuration);
        }
        token.trim_end_matches(['\r', '\n']).to_owned()
    };
    Client::new(&origin, &token)
}

/// Uses the configured HTTP engine for Atlas cache misses. A misconfigured
/// endpoint or failed request yields terminal metadata and never triggers local parsing.
pub(crate) async fn annotate_configured(
    sources: &[&Source],
    cancel: &AtomicBool,
) -> Vec<Annotation> {
    if cancel.load(Ordering::Acquire) {
        return failures(sources, RemoteError::Cancelled);
    }
    let contexts = sources
        .iter()
        .map(|source| source.context())
        .collect::<Vec<_>>();
    let mut annotations = Vec::with_capacity(sources.len());
    let mut misses = Vec::new();
    for (source, context) in sources.iter().zip(&contexts) {
        let cached = super::surreal_get(source, context)
            .await
            .filter(|annotation| {
                annotation.status == Status::Complete && annotation.validate(source)
            });
        if cached.is_none() {
            misses.push(*source);
        }
        annotations.push(cached);
    }
    if misses.is_empty() {
        return annotations.into_iter().flatten().collect();
    }

    static CLIENT: OnceCell<Client> = OnceCell::const_new();
    let client = CLIENT
        .get_or_try_init(|| async {
            tokio::task::spawn_blocking(client_from_environment)
                .await
                .map_err(|_| RemoteError::Configuration)?
        })
        .await;
    let result = match client {
        Ok(client) => client.annotate(&misses, cancel).await,
        Err(error) => Err(error),
    };
    let mut responses = result
        .unwrap_or_else(|error| failures(&misses, error))
        .into_iter();
    for ((annotation, source), context) in annotations.iter_mut().zip(sources).zip(&contexts) {
        if annotation.is_some() {
            continue;
        }
        let next = responses.next().expect("one annotation per cache miss");
        if next.status == Status::Complete {
            super::surreal_put(source, context, &next).await;
        }
        *annotation = Some(next);
    }
    annotations.into_iter().flatten().collect()
}

fn failures(sources: &[&Source], error: RemoteError) -> Vec<Annotation> {
    sources
        .iter()
        .map(|source| {
            let mut annotation = Annotation::terminal(source, &source.context(), error.status());
            annotation.flags.push(error.diagnostic().into());
            annotation
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_hash_and_excessive_text_before_parsing() {
        let source = Source::derived("article", "1", "New York");
        let mut batch = BatchRequest::new(&[&source]).unwrap();
        assert!(batch.validate().is_ok());
        batch.items[0].prose_hash = "0".repeat(64);
        assert_eq!(batch.validate(), Err(RemoteError::Input));
        let oversized = Source::derived("article", "1", &"x".repeat(MAX_ITEM_TEXT + 1));
        assert!(matches!(
            BatchRequest::new(&[&oversized]),
            Err(RemoteError::Input)
        ));
    }
}
