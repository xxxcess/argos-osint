//! Private one-attempt execution transport for retained Focus Brief chat.

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::Stream;
use reqwest::header::{HeaderValue, AUTHORIZATION};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Notify, OnceCell, OwnedSemaphorePermit};
use url::Url;

use super::{AttemptProgress, AttemptReport, AttemptSpec, Deadlines, Transport};
use crate::provider::{self, ChatMessage, Completion, SettingsFile, ToolCall, Usage};
use crate::provider_adapters::{EffectivePolicy, ResponseContract};
use crate::provider_capabilities::ResolvedCapabilities;
use crate::provider_chain::Route;
use crate::provider_diag::{self, Category, ProviderFailure, Stage};
use crate::request_policy::RequestPolicy;
use crate::secrets::{AuthFile, ProviderSecret};

pub const READY_PATH: &str = "/api/v1/health/ready";
pub const ATTEMPT_PATH: &str = "/api/v1/execution/attempt";
pub const MEDIA_TYPE: &str = "application/x-ndjson";
pub const VERSION: u16 = 1;
pub const MAX_REQUEST_BYTES: usize = 512 * 1024;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_FRAME_BYTES: usize = 512 * 1024;
const MAX_PROVIDER_DEADLINE: Duration = Duration::from_secs(600);
const HTTP_DEADLINE: Duration = Duration::from_secs(630);
const FINAL_FRAME_DEADLINE: Duration = Duration::from_secs(5);

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RouteAlias {
    role: String,
    operation: Option<String>,
    index: usize,
    provider: String,
    account: String,
    model: String,
    revision: String,
}

impl RouteAlias {
    fn new(
        role: &str,
        operation: Option<&str>,
        index: usize,
        route: &Route,
        secret: &ProviderSecret,
    ) -> Self {
        Self {
            role: provider::pipeline_role_name(role).unwrap_or(role).into(),
            operation: operation.map(str::to_owned),
            index,
            provider: route.provider.clone(),
            account: route.account.clone(),
            model: route.model.clone(),
            revision: route_revision(secret),
        }
    }

    fn resolve(&self, workspace: &Path) -> Result<ProviderSecret, ProviderFailure> {
        let auth_path = config_path(workspace, "auth.json")?;
        let settings_path = config_path(workspace, "config.toml")?;
        let auth = AuthFile::load_from(&auth_path)
            .map_err(|_| configuration("execution accounts unavailable"))?;
        let settings = SettingsFile::load_from(&settings_path)
            .map_err(|_| configuration("execution routes unavailable"))?;
        let (routes, secrets) = crate::model_exec::resolve_operation_routes(
            &auth,
            &settings,
            &self.role,
            self.operation.as_deref(),
        );
        let (route, secret) = routes
            .get(self.index)
            .zip(secrets.into_iter().nth(self.index))
            .ok_or_else(|| configuration("execution route unavailable"))?;
        if route.skip_reason.is_some()
            || route.provider != self.provider
            || route.account != self.account
            || route.model != self.model
            || route_revision(&secret) != self.revision
            || super::is_subscription(&secret)
        {
            return Err(configuration("execution route changed or unavailable"));
        }
        Ok(secret)
    }
}

fn route_revision(secret: &ProviderSecret) -> String {
    let descriptor = serde_json::json!([
        provider::effective_kind(secret),
        provider_diag::sanitize_endpoint(&secret.base_url),
        secret.model,
    ]);
    crate::classifier::artifact::sha256_hex(descriptor.to_string().as_bytes())
}

fn config_path(workspace: &Path, name: &str) -> Result<PathBuf, ProviderFailure> {
    let path = workspace.join(name);
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() => Ok(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(path),
        _ => Err(configuration("execution configuration unavailable")),
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChatAttempt {
    version: u16,
    alias: RouteAlias,
    messages: Vec<ChatMessage>,
    transport: Transport,
    deadlines: Deadlines,
    policy: RequestPolicy,
    contract: Option<ResponseContract>,
    capabilities: Option<ResolvedCapabilities>,
}

impl ChatAttempt {
    /// Checks the bounded Focus Brief chat contract before admitting work.
    pub fn validate(&self) -> bool {
        let alias = &self.alias;
        let times = [
            Some(self.deadlines.connect),
            Some(self.deadlines.first_response),
            Some(self.deadlines.idle),
            Some(self.deadlines.total),
            self.deadlines.first_output,
            self.deadlines.model_inactivity,
        ];
        self.version == VERSION
            && alias.role == "atlas_analyst"
            && supports_operation(alias.operation.as_deref())
            && alias.index < 32
            && valid_label(&alias.provider)
            && valid_label(&alias.account)
            && valid_label(&alias.model)
            && alias.revision.len() == 64
            && alias.revision.bytes().all(|byte| byte.is_ascii_hexdigit())
            && !self.messages.is_empty()
            && self.messages.len() <= 64
            && self.messages.iter().all(|message| {
                matches!(message.role.as_str(), "system" | "user" | "assistant")
                    && message.tool_call_id.is_none()
                    && message.tool_calls.is_empty()
            })
            && times
                .into_iter()
                .flatten()
                .all(|value| !value.is_zero() && value <= MAX_PROVIDER_DEADLINE)
    }
}

pub(crate) fn supports_operation(operation: Option<&str>) -> bool {
    matches!(
        operation,
        Some(
            provider::OP_CLAIMS_GENERATE
                | provider::OP_INFERENCES_GENERATE
                | provider::OP_LINKS_GENERATE
        )
    )
}

fn valid_label(label: &str) -> bool {
    !label.is_empty() && label.len() <= 256 && !label.chars().any(char::is_control)
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "frame", rename_all = "snake_case", deny_unknown_fields)]
enum Frame {
    Progress { progress: AttemptProgress },
    End { report: Box<WireReport> },
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireReport {
    transport: Transport,
    endpoint: String,
    elapsed_ms: u64,
    outcome: Result<Final, ProviderFailure>,
    policy_diagnostics: Vec<String>,
    effective: Option<EffectivePolicy>,
    recovery: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Final {
    content: String,
    tool_calls: Vec<ToolCall>,
    finish_reason: Option<String>,
    usage: Option<Usage>,
}

impl From<AttemptReport> for WireReport {
    fn from(report: AttemptReport) -> Self {
        Self {
            transport: report.transport,
            endpoint: report.endpoint,
            elapsed_ms: report.elapsed_ms,
            outcome: report
                .outcome
                .map(|completion| Final {
                    content: completion.content,
                    tool_calls: completion.tool_calls,
                    finish_reason: completion.finish_reason,
                    usage: completion.usage,
                })
                .map_err(|mut failure| {
                    // Provider bodies and request IDs can contain echoed credentials.
                    failure.causes.clear();
                    failure.provider_code = None;
                    failure.provider_message = None;
                    failure.request_id = None;
                    failure
                }),
            policy_diagnostics: report.policy_diagnostics,
            effective: report.effective,
            recovery: report.recovery,
        }
    }
}

impl From<WireReport> for AttemptReport {
    fn from(report: WireReport) -> Self {
        Self {
            transport: report.transport,
            endpoint: report.endpoint,
            elapsed_ms: report.elapsed_ms,
            outcome: report.outcome.map(|final_value| Completion {
                content: final_value.content,
                tool_calls: final_value.tool_calls,
                finish_reason: final_value.finish_reason,
                usage: final_value.usage,
                ..Completion::default()
            }),
            policy_diagnostics: report.policy_diagnostics,
            effective: report.effective,
            recovery: report.recovery,
        }
    }
}

fn failed(transport: Transport, failure: ProviderFailure) -> AttemptReport {
    AttemptReport {
        transport,
        endpoint: String::new(),
        elapsed_ms: 0,
        outcome: Err(failure),
        policy_diagnostics: Vec::new(),
        effective: None,
        recovery: None,
    }
}

fn configuration(detail: &str) -> ProviderFailure {
    ProviderFailure::new(Stage::Configuration, Category::Configuration, detail)
}

fn wire_error(category: Category, detail: &str) -> ProviderFailure {
    ProviderFailure::new(Stage::Stream, category, detail)
}

fn encode(frame: &Frame, key: Option<&str>) -> Option<Vec<u8>> {
    let mut bytes = serde_json::to_vec(frame).ok()?;
    if bytes.len() >= MAX_FRAME_BYTES {
        return None;
    }
    if let Some(key) = key.filter(|key| !key.is_empty()) {
        let escaped = serde_json::to_string(key).ok()?;
        let encoded_key = &escaped.as_bytes()[1..escaped.len() - 1];
        if bytes
            .windows(encoded_key.len())
            .any(|window| window == encoded_key)
        {
            return None;
        }
    }
    bytes.push(b'\n');
    Some(bytes)
}

/// Runs one attempt and returns a bounded stream. The guarded caller owns
/// the shared account lease. This process owns provider auth and pacing.
pub fn serve_attempt(
    workspace: PathBuf,
    request: ChatAttempt,
    slot: OwnedSemaphorePermit,
) -> impl Stream<Item = io::Result<Vec<u8>>> {
    let (tx, rx) = mpsc::channel::<Vec<u8>>(32);
    tokio::spawn(async move {
        let _slot = slot;
        let transport = request.transport;
        let alias = request.alias;
        let resolved = tokio::task::spawn_blocking(move || alias.resolve(&workspace));
        let report = match resolved.await {
            Ok(Ok(secret)) => {
                let key = provider::resolved_key(&secret);
                let overflow = Arc::new(Notify::new());
                let progress_key = key.clone();
                let progress_tx = tx.clone();
                let progress_overflow = overflow.clone();
                let progress = move |progress| {
                    if let Some(bytes) =
                        encode(&Frame::Progress { progress }, progress_key.as_deref())
                    {
                        if progress_tx.try_send(bytes).is_ok() {
                            return;
                        }
                    }
                    progress_overflow.notify_one();
                };
                let attempt = Box::pin(super::attempt_spec_inner(
                    &secret,
                    &request.messages,
                    transport,
                    request.deadlines,
                    &request.policy,
                    AttemptSpec {
                        progress: Some(&progress),
                        contract: request.contract.as_ref(),
                        caps: request.capabilities.as_ref(),
                        ..AttemptSpec::default()
                    },
                ));
                let report = tokio::select! {
                    biased;
                    _ = tx.closed() => return,
                    _ = overflow.notified() => failed(transport, wire_error(Category::StreamInterrupted, "execution progress exceeded its channel bound")),
                    result = tokio::time::timeout(HTTP_DEADLINE, attempt) => {
                        result.unwrap_or_else(|_| failed(transport, wire_error(Category::Timeout, "execution attempt deadline exceeded")))
                    }
                };
                send_report(&tx, report, key.as_deref()).await;
                return;
            }
            Ok(Err(failure)) => failed(transport, failure),
            Err(_) => failed(
                transport,
                configuration("execution configuration task failed"),
            ),
        };
        send_report(&tx, report, None).await;
    });
    futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|bytes| (Ok(bytes), rx))
    })
}

async fn send_report(tx: &mpsc::Sender<Vec<u8>>, report: AttemptReport, key: Option<&str>) {
    let transport = report.transport;
    let end = Frame::End {
        report: Box::new(report.into()),
    };
    let bytes = encode(&end, key).or_else(|| {
        let report = failed(
            transport,
            wire_error(
                Category::InvalidResult,
                "execution result was unsafe or exceeded its size bound",
            ),
        );
        encode(
            &Frame::End {
                report: Box::new(report.into()),
            },
            None,
        )
    });
    if let Some(bytes) = bytes {
        let _ = tokio::time::timeout(FINAL_FRAME_DEADLINE, tx.send(bytes)).await;
    }
}

struct Client {
    endpoint: Url,
    auth: HeaderValue,
    http: reqwest::Client,
}

impl Client {
    fn new(origin: &str, token: &str) -> Result<Self, ProviderFailure> {
        let invalid = || configuration("execution service configuration invalid");
        let origin = Url::parse(origin).map_err(|_| invalid())?;
        if !matches!(origin.scheme(), "http" | "https")
            || origin.host().is_none()
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.path() != "/"
            || origin.query().is_some()
            || origin.fragment().is_some()
            || !(32..=256).contains(&token.len())
        {
            return Err(invalid());
        }
        let mut auth = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| invalid())?;
        auth.set_sensitive(true);
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(HTTP_DEADLINE)
            .build()
            .map_err(|_| invalid())?;
        Ok(Self {
            endpoint: origin.join(ATTEMPT_PATH).map_err(|_| invalid())?,
            auth,
            http,
        })
    }

    async fn attempt(
        &self,
        request: ChatAttempt,
        progress: Option<&super::ProgressFn>,
    ) -> Result<AttemptReport, ProviderFailure> {
        let invalid = || wire_error(Category::MalformedPayload, "invalid execution stream");
        let transport = request.transport;
        if !request.validate() {
            return Err(configuration("invalid execution attempt"));
        }
        let body = serde_json::to_vec(&request).map_err(|_| invalid())?;
        if body.len() > MAX_REQUEST_BYTES {
            return Err(configuration("execution attempt exceeds byte bound"));
        }
        let mut response = self
            .http
            .post(self.endpoint.clone())
            .header(AUTHORIZATION, self.auth.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(transport_error)?;
        match response.status().as_u16() {
            200 => {}
            400 | 413 | 422 => return Err(configuration("execution service refused the attempt")),
            401 | 403 => return Err(configuration("execution service authentication failed")),
            429 => return Err(wire_error(Category::Server, "execution service is busy")),
            _ => {
                return Err(wire_error(
                    Category::Network,
                    "execution service unavailable",
                ))
            }
        }
        if response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .is_none_or(|value| value != MEDIA_TYPE)
        {
            return Err(invalid());
        }
        let mut total = 0usize;
        let mut line = Vec::new();
        let mut terminal = None;
        while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
            total = total.saturating_add(chunk.len());
            if total > MAX_RESPONSE_BYTES {
                return Err(invalid());
            }
            for part in chunk.split_inclusive(|byte| *byte == b'\n') {
                if terminal.is_some() || line.len() + part.len() > MAX_FRAME_BYTES {
                    return Err(invalid());
                }
                line.extend_from_slice(part);
                if line.last() == Some(&b'\n') {
                    match serde_json::from_slice(&line).map_err(|_| invalid())? {
                        Frame::Progress { progress: event } => {
                            if let Some(sink) = progress {
                                sink(event);
                            }
                        }
                        Frame::End { report } if report.transport == transport => {
                            terminal = Some(AttemptReport::from(*report));
                        }
                        Frame::End { .. } => return Err(invalid()),
                    }
                    line.clear();
                }
            }
        }
        terminal.ok_or_else(|| {
            wire_error(
                Category::PrematureEof,
                "execution stream ended without a final report",
            )
        })
    }
}

fn transport_error(error: reqwest::Error) -> ProviderFailure {
    let category = if error.is_timeout() {
        Category::Timeout
    } else {
        Category::Network
    };
    wire_error(category, "execution service request failed")
}

pub(crate) fn configured() -> bool {
    std::env::var_os("FLEETWORKS_EXECUTION_ORIGIN").is_some()
}

fn client_from_environment() -> Result<Client, ProviderFailure> {
    let invalid = || configuration("execution service configuration invalid");
    let origin = std::env::var("FLEETWORKS_EXECUTION_ORIGIN").map_err(|_| invalid())?;
    let path = std::env::var_os("FLEETWORKS_EXECUTION_TOKEN_FILE").ok_or_else(invalid)?;
    let mut token = String::new();
    std::fs::File::open(path)
        .map_err(|_| invalid())?
        .take(259)
        .read_to_string(&mut token)
        .map_err(|_| invalid())?;
    if token.len() > 258 {
        return Err(invalid());
    }
    Client::new(&origin, token.trim_end_matches(['\r', '\n']))
}

/// Dispatch one routed chat request through the explicitly configured service.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn routed_attempt(
    role: &str,
    operation: Option<&str>,
    route_index: usize,
    route: &Route,
    secret: &ProviderSecret,
    messages: &[ChatMessage],
    transport: Transport,
    deadlines: Deadlines,
    policy: &RequestPolicy,
    spec: AttemptSpec<'_>,
) -> AttemptReport {
    static CLIENT: OnceCell<Client> = OnceCell::const_new();
    let client = CLIENT
        .get_or_try_init(|| async {
            tokio::task::spawn_blocking(client_from_environment)
                .await
                .map_err(|_| configuration("execution configuration task failed"))?
        })
        .await;
    let report = match client {
        Ok(client) => {
            client
                .attempt(
                    ChatAttempt {
                        version: VERSION,
                        alias: RouteAlias::new(role, operation, route_index, route, secret),
                        messages: messages.to_vec(),
                        transport,
                        deadlines,
                        policy: *policy,
                        contract: spec.contract.cloned(),
                        capabilities: spec.caps.cloned(),
                    },
                    spec.progress,
                )
                .await
        }
        Err(failure) => Err(failure),
    };
    report.unwrap_or_else(|failure| failed(transport, failure))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ModelAssignment;
    use crate::provider_attempt::mock;
    use futures_util::StreamExt;

    const FIXTURE_KEY: &str = "fixture-model-secret-for-wire-test";

    fn setup(workspace: &Path, base_url: &str) -> ChatAttempt {
        let secret = ProviderSecret {
            kind: "local".into(),
            base_url: base_url.into(),
            model: "fixture-model".into(),
            api_key: Some(FIXTURE_KEY.into()),
            stt_model: None,
            device: None,
        };
        let mut auth = AuthFile::default();
        auth.set_account(secret.clone());
        std::fs::write(
            workspace.join("auth.json"),
            serde_json::to_vec(&auth).unwrap(),
        )
        .unwrap();
        let mut settings = SettingsFile::default();
        *settings.defaults.role_mut("atlas_analyst").unwrap() = ModelAssignment {
            provider: "local".into(),
            model: secret.model.clone(),
            ..ModelAssignment::default()
        };
        settings.save_to(&workspace.join("config.toml")).unwrap();
        let (routes, secrets) = crate::model_exec::resolve_operation_routes(
            &auth,
            &settings,
            "atlas_analyst",
            Some(provider::OP_CLAIMS_GENERATE),
        );
        ChatAttempt {
            version: VERSION,
            alias: RouteAlias::new(
                "atlas_analyst",
                Some(provider::OP_CLAIMS_GENERATE),
                0,
                &routes[0],
                &secrets[0],
            ),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: "Read the fixture packet.".into(),
                tool_call_id: None,
                tool_calls: Vec::new(),
            }],
            transport: Transport::Stream,
            deadlines: Deadlines::default(),
            policy: RequestPolicy::default(),
            contract: Some(ResponseContract::Prose),
            capabilities: None,
        }
    }

    #[tokio::test]
    async fn streams_progress_then_final_with_server_owned_authorization() {
        let answer = mock::delta("Fixture answer");
        let finish = mock::finish("stop");
        let upstream = mock::serve(vec![mock::sse_reply(&[&answer, &finish])]).await;
        let workspace = tempfile::tempdir().unwrap();
        let request = setup(workspace.path(), &upstream.base_url);
        assert!(request.validate());
        assert!(!serde_json::to_string(&request)
            .unwrap()
            .contains(FIXTURE_KEY));
        let slot = std::sync::Arc::new(tokio::sync::Semaphore::new(1))
            .acquire_owned()
            .await
            .unwrap();
        let stream = serve_attempt(workspace.path().into(), request, slot);
        futures_util::pin_mut!(stream);
        assert!(matches!(
            serde_json::from_slice::<Frame>(&stream.next().await.unwrap().unwrap()).unwrap(),
            Frame::Progress {
                progress: AttemptProgress::Dispatched
            }
        ));
        let mut saw_output = false;
        while let Some(frame) = stream.next().await {
            let bytes = frame.unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains(FIXTURE_KEY));
            match serde_json::from_slice::<Frame>(&bytes).unwrap() {
                Frame::Progress {
                    progress: AttemptProgress::FirstModelOutput,
                } => saw_output = true,
                Frame::End { report } => {
                    assert!(saw_output);
                    assert_eq!(report.outcome.unwrap().content, "Fixture answer");
                    assert!(stream.next().await.is_none());
                    assert_eq!(upstream.hits(), 1);
                    assert!(upstream.requests.lock().unwrap()[0].contains(FIXTURE_KEY));
                    return;
                }
                _ => {}
            }
        }
        panic!("execution never sent a final report");
    }

    #[tokio::test(start_paused = true)]
    async fn a_full_response_queue_cannot_hold_the_attempt_slot() {
        let (tx, _stalled_reader) = mpsc::channel(1);
        tx.try_send(Vec::new()).unwrap();
        let report = failed(Transport::Stream, configuration("fixture"));
        tokio::time::timeout(
            FINAL_FRAME_DEADLINE + Duration::from_secs(1),
            send_report(&tx, report, None),
        )
        .await
        .expect("terminal frame must yield when the response reader stalls");
    }

    #[tokio::test]
    async fn configured_transport_outage_returns_typed_failure() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let workspace = tempfile::tempdir().unwrap();
        let request = setup(workspace.path(), &format!("http://{address}/v1"));
        let client = Client::new(&format!("http://{address}"), &"x".repeat(32)).unwrap();
        assert_eq!(
            client.attempt(request, None).await.unwrap_err().category,
            Category::Network
        );
    }

    #[tokio::test]
    async fn dropping_response_stream_cancels_upstream_attempt() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let upstream = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = [0u8; 8192];
            assert!(socket.read(&mut bytes).await.unwrap() > 0);
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 4096\r\n\r\n").await.unwrap();
            while socket.read(&mut bytes).await.unwrap_or(0) != 0 {}
        });
        let workspace = tempfile::tempdir().unwrap();
        let request = setup(workspace.path(), &format!("http://{address}/v1"));
        let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
        let slot = slots.clone().acquire_owned().await.unwrap();
        let mut stream = Box::pin(serve_attempt(workspace.path().into(), request, slot));
        while let Some(frame) = stream.next().await {
            if matches!(
                serde_json::from_slice::<Frame>(&frame.unwrap()).unwrap(),
                Frame::Progress {
                    progress: AttemptProgress::HeadersReceived { .. }
                }
            ) {
                break;
            }
        }
        drop(stream);
        tokio::time::timeout(Duration::from_secs(5), upstream)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(slots.available_permits(), 1);
    }
}
