use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{self, HeaderValue};
use reqwest::{Method, RequestBuilder};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::io::AsyncSeekExt;
use url::Url;

use crate::ipc::IpcOperation;
use crate::ipc::LocalIpcClient;
use crate::ipc::ShutdownAck;
use crate::{Ready, API_VERSION, READY_PATH};

mod pairing;
pub use pairing::PairingClient;

mod log_transfer;
pub use log_transfer::LogExportPage;

mod configuration_transfer;
pub use configuration_transfer::ConfigurationTransferReceipt;

mod inbox_qa;

pub(crate) const MAX_REQUEST_BYTES: usize = 16 * 1024;
const MAX_QUERY_BYTES: usize = 8 * 1024;
pub(crate) const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_TOKEN_BYTES: usize = 4 * 1024;

/// A validated backend-owned upload. The reference contains no filesystem path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct DatasetUpload {
    pub dataset: crate::readiness::DatasetRef,
    pub expires_in_seconds: u64,
}

impl DatasetUpload {
    /// Construct a validated backend-owned upload receipt.
    #[must_use]
    pub fn new(dataset: crate::readiness::DatasetRef, expires_in_seconds: u64) -> Self {
        Self {
            dataset,
            expires_in_seconds,
        }
    }
}

/// Client failures contain neither the API's prose nor request URLs or credentials.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ClientError {
    InvalidOrigin,
    MissingToken,
    InvalidToken,
    InvalidRoute,
    InvalidRequest,
    RequestTooLarge,
    Transport,
    Timeout,
    Status(u16),
    ResponseTooLarge,
    InvalidResponse,
    UnsupportedVersion(u16),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::InvalidOrigin => {
                write!(f, "expected a loopback HTTP or authenticated HTTPS origin")
            }
            Self::MissingToken => write!(f, "a bearer token is required for a remote API origin"),
            Self::InvalidToken => write!(f, "invalid API bearer token"),
            Self::InvalidRoute => write!(f, "invalid versioned API route"),
            Self::InvalidRequest => write!(f, "invalid API request"),
            Self::RequestTooLarge => write!(f, "API request exceeds the size limit"),
            Self::Transport => write!(f, "could not reach the configured API"),
            Self::Timeout => write!(f, "API request timed out"),
            Self::Status(status) => write!(f, "API returned HTTP {status}"),
            Self::ResponseTooLarge => write!(f, "API response exceeds the size limit"),
            Self::InvalidResponse => write!(f, "invalid API response"),
            Self::UnsupportedVersion(version) => write!(f, "unsupported API version {version}"),
        }
    }
}

impl std::error::Error for ClientError {}

impl ClientError {
    fn transport(error: reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::Timeout
        } else {
            Self::Transport
        }
    }
}

#[derive(Clone)]
struct HttpClient {
    origin: Url,
    token: Option<HeaderValue>,
    http: reqwest::Client,
    registration: Option<(HeaderValue, HeaderValue)>,
}

impl HttpClient {
    fn request(&self, method: Method, route: &str) -> Result<RequestBuilder, ClientError> {
        validate_route(route)?;
        let mut url = self.origin.clone();
        url.set_path(route);
        let mut request = self
            .http
            .request(method, url)
            .header(header::ACCEPT, "application/json");
        if inbox_qa::is_qa_route(route) {
            request = request.header(
                crate::chat::QA_EVIDENCE_HEADER,
                crate::chat::QA_EVIDENCE_VERSION,
            );
        }
        if let Some((registration, revision)) = &self.registration {
            request = request
                .header("x-fleetworks-registration", registration.clone())
                .header("x-fleetworks-registration-revision", revision.clone());
        }
        Ok(match &self.token {
            Some(token) => request.header(header::AUTHORIZATION, token.clone()),
            None => request,
        })
    }
}

#[derive(Clone)]
enum ClientTransport {
    Http(Box<HttpClient>),
    LocalIpc(Arc<LocalIpcClient>),
}

/// A transport-neutral client pinned to one public API origin or local IPC stream.
///
/// For HTTP, authorization stays on that origin and redirects/proxies are disabled.
#[derive(Clone)]
pub struct Client {
    transport: ClientTransport,
    registered_context: Option<crate::context::RegisteredContext>,
}

/// Validate a literal versioned API route path without query or traversal segments.
pub(crate) fn validate_route(route: &str) -> Result<(), ClientError> {
    if inbox_qa::is_qa_route(route) {
        return Ok(());
    }
    let tail = route
        .strip_prefix("/api/v1/")
        .ok_or(ClientError::InvalidRoute)?;
    if route.len() > 128
        || !tail.split('/').all(|segment| {
            !segment.is_empty()
                && segment.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'-' | b'_')
                })
        })
    {
        return Err(ClientError::InvalidRoute);
    }
    Ok(())
}

/// Parse the shared origin policy without transport or credential side effects.
fn validated_origin(origin: &str) -> Result<(Url, bool), ClientError> {
    let origin = Url::parse(origin).map_err(|_| ClientError::InvalidOrigin)?;
    let loopback = match origin.host() {
        Some(url::Host::Domain(name)) => name == "localhost",
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    if origin.host().is_none()
        || !matches!(origin.scheme(), "http" | "https")
        || (origin.scheme() == "http" && !loopback)
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.path() != "/"
        || origin.query().is_some()
        || origin.fragment().is_some()
    {
        return Err(ClientError::InvalidOrigin);
    }
    Ok((origin, loopback))
}

impl Client {
    /// Wrap an already-handshaken local IPC transport without HTTP fallback.
    #[must_use]
    pub fn from_local_ipc(transport: Arc<LocalIpcClient>) -> Self {
        Self {
            transport: ClientTransport::LocalIpc(transport),
            registered_context: None,
        }
    }

    /// Whether this client communicates over local IPC rather than HTTP.
    #[must_use]
    pub fn is_local_ipc(&self) -> bool {
        matches!(self.transport, ClientTransport::LocalIpc(_))
    }

    /// Capabilities advertised during a local IPC handshake, if connected over local IPC.
    #[must_use]
    pub fn capabilities(&self) -> Option<&crate::capabilities::Capabilities> {
        match &self.transport {
            ClientTransport::Http(_) => None,
            ClientTransport::LocalIpc(ipc) => ipc.capabilities(),
        }
    }

    /// Gracefully shut down an attached local IPC backend process.
    pub async fn shutdown_local(&self, reason: &str) -> Result<ShutdownAck, ClientError> {
        match &self.transport {
            ClientTransport::Http(_) => Err(ClientError::InvalidRequest),
            ClientTransport::LocalIpc(ipc) => ipc.shutdown(reason).await.map_err(ClientError::from),
        }
    }

    /// Read only this registration's visible cycle associations, newest first.
    pub async fn cycle_history(
        &self,
        limit: usize,
    ) -> Result<crate::cycle_history::CycleHistory, ClientError> {
        self.cycle_history_page(limit, None).await
    }

    /// Read a bounded page of this registration's cycle associations.
    pub async fn cycle_history_page(
        &self,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<crate::cycle_history::CycleHistory, ClientError> {
        if !(1..=100).contains(&limit) {
            return Err(ClientError::InvalidRequest);
        }
        if cursor.is_some_and(|cursor| {
            cursor.is_empty() || cursor.len() > 256 || cursor.chars().any(char::is_control)
        }) {
            return Err(ClientError::InvalidRequest);
        }
        let mut query = vec![("limit", limit.to_string())];
        if let Some(cursor) = cursor {
            query.push(("cursor", cursor.to_owned()));
        }
        self.get_with_query(crate::cycle_history::CYCLE_HISTORY_PATH, &query)
            .await
    }

    /// Hide a cycle association after an in-place confirmation and revision check.
    pub async fn delete_cycle(
        &self,
        request: &crate::cycle_history::DeleteCycleRequest,
    ) -> Result<crate::cycle_history::DeleteCycleReceipt, ClientError> {
        self.post(crate::cycle_history::CYCLE_HISTORY_DELETE_PATH, request)
            .await
    }

    /// Validate and normalize an origin for a local connection profile.
    ///
    /// This uses the same origin policy as `new` without creating an HTTP
    /// client, reading a credential reference or making any network request.
    pub fn canonical_origin(origin: &str) -> Result<String, ClientError> {
        validated_origin(origin).map(|(origin, _)| origin.to_string())
    }

    /// Connect to an HTTP loopback origin or an HTTPS origin.
    ///
    /// Unauthenticated access is restricted to loopback for legacy Argos compatibility.
    pub fn new(origin: &str, token: Option<&str>) -> Result<Self, ClientError> {
        let (origin, loopback) = validated_origin(origin)?;
        if !loopback && token.is_none() {
            return Err(ClientError::MissingToken);
        }
        let token = token.map(bearer_header).transpose()?;
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            // The existing API's own deadline is 45 seconds.
            .timeout(Duration::from_secs(50))
            .build()
            .map_err(ClientError::transport)?;
        Ok(Self {
            transport: ClientTransport::Http(Box::new(HttpClient {
                origin,
                token,
                http,
                registration: None,
            })),
            registered_context: None,
        })
    }

    /// Clone the origin-bound client for one confirmed registered origin.
    ///
    /// The base client is unchanged for backend-global reads. The server must
    /// still validate current ownership, registration and permission; headers
    /// are context references, never authorization or immutable job origin.
    pub fn with_registered_context(
        &self,
        context: &crate::context::RegisteredContext,
    ) -> Result<Self, ClientError> {
        if context.registration_id.is_empty()
            || context.registration_id.len() > 128
            || context.registration_revision == 0
        {
            return Err(ClientError::InvalidRequest);
        }
        let registration = HeaderValue::from_str(&context.registration_id)
            .map_err(|_| ClientError::InvalidRequest)?;
        let revision = HeaderValue::from_str(&context.registration_revision.to_string())
            .map_err(|_| ClientError::InvalidRequest)?;
        let mut client = self.clone();
        client.registered_context = Some(context.clone());
        if let ClientTransport::Http(http) = &mut client.transport {
            http.registration = Some((registration, revision));
        }
        Ok(client)
    }

    /// Read readiness and verify the reported API version.
    pub async fn ready(&self) -> Result<Ready, ClientError> {
        let ready: Ready = self.get(READY_PATH).await?;
        if ready.api_version != API_VERSION {
            return Err(ClientError::UnsupportedVersion(ready.api_version));
        }
        Ok(ready)
    }

    /// Send a GET to a literal versioned route.
    pub async fn get<T: DeserializeOwned>(&self, route: &str) -> Result<T, ClientError> {
        match &self.transport {
            ClientTransport::Http(http) => Self::send(http.request(Method::GET, route)?).await,
            ClientTransport::LocalIpc(ipc) => {
                let op = IpcOperation::from_get(route)?;
                ipc.request_json(self.registered_context.clone(), op).await
            }
        }
    }

    /// Send a GET with encoded query fields.
    pub async fn get_with_query<T: DeserializeOwned>(
        &self,
        route: &str,
        query: &(impl Serialize + ?Sized),
    ) -> Result<T, ClientError> {
        match &self.transport {
            ClientTransport::Http(http) => {
                Self::send(http.request(Method::GET, route)?.query(query)).await
            }
            ClientTransport::LocalIpc(ipc) => {
                let op = IpcOperation::from_get_with_query(route, query)?;
                ipc.request_json(self.registered_context.clone(), op).await
            }
        }
    }

    /// Send JSON to a literal versioned route.
    pub async fn post<T: DeserializeOwned>(
        &self,
        route: &str,
        body: &(impl Serialize + ?Sized),
    ) -> Result<T, ClientError> {
        match &self.transport {
            ClientTransport::Http(http) => {
                let request = http.request(Method::POST, route)?;
                let body = serde_json::to_vec(body).map_err(|_| ClientError::InvalidRequest)?;
                if body.len() > MAX_REQUEST_BYTES {
                    return Err(ClientError::RequestTooLarge);
                }
                Self::send(
                    request
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(body),
                )
                .await
            }
            ClientTransport::LocalIpc(ipc) => {
                validate_route(route)?;
                let body = serde_json::to_vec(body).map_err(|_| ClientError::InvalidRequest)?;
                let op = IpcOperation::from_post_bytes(route, &body)?;
                ipc.request_json(self.registered_context.clone(), op).await
            }
        }
    }

    /// Read authenticated backend readiness and actor-index action permissions.
    pub async fn workstation_readiness(
        &self,
    ) -> Result<crate::readiness::WorkstationReadiness, ClientError> {
        self.get(crate::readiness::WORKSTATION_READINESS_PATH).await
    }

    /// Request or replay a durable actor search index rebuild on the registered backend.
    pub async fn reindex_actors(
        &self,
        request: &crate::readiness::MaintenanceRequest,
    ) -> Result<crate::readiness::MaintenanceReceipt, ClientError> {
        if self.registered_context.is_none()
            || request
                .validate(crate::readiness::MaintenanceOperation::ReindexActors)
                .is_err()
        {
            return Err(ClientError::InvalidRequest);
        }
        self.post(crate::search::ACTOR_REINDEX_PATH, request).await
    }

    /// Reconcile whether an uncertain request ID was durably accepted by the backend.
    pub async fn request_detail(
        &self,
        query: &crate::context::WorkstationRequestDetailQuery,
    ) -> Result<crate::context::JobRequestAssociation, ClientError> {
        if self.registered_context.is_none() || query.validate().is_err() {
            return Err(ClientError::InvalidRequest);
        }
        self.get_with_query(crate::context::WORKSTATION_REQUEST_DETAIL_PATH, query)
            .await
    }

    /// Upload an already-open, user-selected CSV with bounded memory.
    ///
    /// The server independently verifies the actual byte count, SHA-256 and CSV
    /// kind before publishing a dataset reference. No local path crosses transport.
    pub async fn upload_dataset(
        &self,
        kind: crate::readiness::DatasetKind,
        sha256: &str,
        mut file: tokio::fs::File,
        size: u64,
    ) -> Result<DatasetUpload, ClientError> {
        use crate::dataset_upload::DatasetUploadRequest;
        use crate::dataset_upload::DatasetUploadTicket;
        use crate::dataset_upload::DATASET_UPLOADS_PATH;

        if self.registered_context.is_none() {
            return Err(ClientError::InvalidRequest);
        }
        if size == 0 || size > crate::dataset_upload::dataset_upload_limit(kind) {
            return Err(ClientError::RequestTooLarge);
        }
        let upload = DatasetUploadRequest::new(kind, sha256.to_owned(), size)
            .map_err(|_| ClientError::InvalidRequest)?;
        let metadata = file
            .metadata()
            .await
            .map_err(|_| ClientError::InvalidRequest)?;
        if !metadata.is_file() || metadata.len() != size {
            return Err(ClientError::InvalidRequest);
        }
        file.rewind()
            .await
            .map_err(|_| ClientError::InvalidRequest)?;

        let response: DatasetUpload = match &self.transport {
            ClientTransport::Http(http) => {
                let ticket: DatasetUploadTicket = self.post(DATASET_UPLOADS_PATH, &upload).await?;
                ticket
                    .validate()
                    .map_err(|_| ClientError::InvalidResponse)?;
                let stream = futures_util::stream::try_unfold(
                    (file, 0u64),
                    move |(mut file, read)| async move {
                        let mut chunk = vec![0u8; 64 * 1024];
                        let length = file.read(&mut chunk).await?;
                        if length == 0 {
                            if read != size {
                                return Err(std::io::Error::other("dataset length changed"));
                            }
                            return Ok(None);
                        }
                        let read = read + length as u64;
                        if read > size {
                            return Err(std::io::Error::other("dataset length changed"));
                        }
                        chunk.truncate(length);
                        Ok(Some((chunk, (file, read))))
                    },
                );
                let request = http
                    .request(Method::PUT, &ticket.upload_path)?
                    .header(header::CONTENT_TYPE, "text/csv")
                    .header(header::CONTENT_LENGTH, size)
                    .timeout(Duration::from_secs(600))
                    .body(reqwest::Body::wrap_stream(stream));
                Self::send(request).await?
            }
            ClientTransport::LocalIpc(ipc) => {
                let ticket: DatasetUploadTicket = ipc
                    .request_json(
                        self.registered_context.clone(),
                        IpcOperation::DatasetUploadStart(upload),
                    )
                    .await?;
                ticket
                    .validate()
                    .map_err(|_| ClientError::InvalidResponse)?;
                let upload_id = ticket
                    .upload_path
                    .strip_prefix(DATASET_UPLOADS_PATH)
                    .and_then(|suffix| suffix.strip_prefix('/'))
                    .ok_or(ClientError::InvalidResponse)?
                    .to_owned();
                let mut offset = 0_u64;
                let mut buffer = vec![0_u8; crate::ipc::MAX_IPC_CHUNK_BYTES];
                loop {
                    let length = file
                        .read(&mut buffer)
                        .await
                        .map_err(|_| ClientError::Transport)?;
                    if length == 0 {
                        if offset != size {
                            return Err(ClientError::InvalidRequest);
                        }
                        break;
                    }
                    let next_offset = offset
                        .checked_add(length as u64)
                        .ok_or(ClientError::RequestTooLarge)?;
                    if next_offset > size {
                        return Err(ClientError::InvalidRequest);
                    }
                    let data_b64 = crate::ipc::base64_encode(&buffer[..length]);
                    let _: serde_json::Value = ipc
                        .request_json(
                            self.registered_context.clone(),
                            IpcOperation::DatasetUploadChunk {
                                upload_id: upload_id.clone(),
                                offset,
                                data_b64,
                            },
                        )
                        .await?;
                    offset = next_offset;
                }
                ipc.request_json(
                    self.registered_context.clone(),
                    IpcOperation::DatasetUploadFinish { upload_id },
                )
                .await?
            }
        };

        response
            .dataset
            .validate()
            .map_err(|_| ClientError::InvalidResponse)?;
        if response.dataset.kind != kind
            || response.dataset.size_bytes != size
            || response.dataset.sha256 != sha256.to_ascii_lowercase()
        {
            return Err(ClientError::InvalidResponse);
        }
        Ok(response)
    }

    async fn send<T: DeserializeOwned>(request: RequestBuilder) -> Result<T, ClientError> {
        let (http, request) = request.build_split();
        let request = request.map_err(|_| ClientError::InvalidRequest)?;
        let response_limit = if inbox_qa::is_qa_route(request.url().path()) {
            crate::chat::MAX_PAGE_BYTES
        } else {
            MAX_RESPONSE_BYTES
        };
        if request
            .url()
            .query()
            .is_some_and(|query| query.len() > MAX_QUERY_BYTES)
        {
            return Err(ClientError::RequestTooLarge);
        }
        let mut response = http
            .execute(request)
            .await
            .map_err(ClientError::transport)?;
        if !response.status().is_success() {
            return Err(ClientError::Status(response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|len| len > response_limit as u64)
        {
            return Err(ClientError::ResponseTooLarge);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(ClientError::transport)? {
            if chunk.len() > response_limit - body.len() {
                return Err(ClientError::ResponseTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(|_| ClientError::InvalidResponse)
    }
}

fn bearer_header(token: &str) -> Result<HeaderValue, ClientError> {
    let value = token.trim_end_matches('=');
    if token.len() > MAX_TOKEN_BYTES
        || value.is_empty()
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/')
        })
    {
        return Err(ClientError::InvalidToken);
    }
    let mut header =
        HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| ClientError::InvalidToken)?;
    header.set_sensitive(true);
    Ok(header)
}

#[cfg(test)]
mod tests;
