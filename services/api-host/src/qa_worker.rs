//! Private connection to one Inbox QA model process.

#[path = "qa_sandbox.rs"]
mod sandbox;

#[cfg(feature = "embedded")]
pub(crate) fn decision_command(
    runtime_dir: &Path,
    model: &Path,
    worker: &Path,
    socket_dir: &Path,
) -> Result<Command> {
    sandbox::decision_command(runtime_dir, model, worker, socket_dir)
}

use std::collections::BTreeMap;
use std::env;
use std::fmt;
use std::fs;
use std::future;
use std::io;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use anyhow::anyhow;
use anyhow::ensure;
use anyhow::Result;
use argos_osint_api::inbox_qa::CancellationToken;
use argos_osint_api::inbox_qa::QaInference;
use argos_osint_api::inbox_qa::QaInferenceFuture;
use argos_osint_core::hardware;
use fleetworks_contracts::chat::QaAvailabilityReason;
use fleetworks_contracts::chat::QaGeneratorPreference;
use fleetworks_contracts::chat::QaModelTarget;
use fleetworks_contracts::chat::QaStartupStage;
use fleetworks_contracts::qa_worker::qa_frame_len;
use fleetworks_contracts::qa_worker::QaWorkerCommand;
use fleetworks_contracts::qa_worker::QaWorkerEvent;
use fleetworks_contracts::qa_worker::QaWorkerReply;
use fleetworks_contracts::qa_worker::QaWorkerRequest;
use fleetworks_contracts::qa_worker::QaWorkerSequence;
use fleetworks_contracts::qa_worker::QA_CONTEXT_USAGE_ENV;
use fleetworks_contracts::qa_worker::QA_DECODED_CLARIFICATION_ENV;
use fleetworks_contracts::qa_worker::QA_DIALOGUE_ENV;
use fleetworks_contracts::qa_worker::QA_GENERAL_DIALOGUE_ENV;
use fleetworks_contracts::qa_worker::QA_JOB_LOG_CONTEXT_ENV;
use fleetworks_contracts::qa_worker::QA_LEGACY_STREAMING_WINDOW_TOKENS;
use fleetworks_contracts::qa_worker::QA_MODEL_CONTEXT_ENV;
use fleetworks_contracts::qa_worker::QA_MODEL_FILES;
use fleetworks_contracts::qa_worker::QA_MODEL_REVISION;
use fleetworks_contracts::qa_worker::QA_MODEL_TARGET_ENV;
use fleetworks_contracts::qa_worker::QA_RESPONSE_PLAN_ENV;
use fleetworks_contracts::qa_worker::QA_STREAMING_ENV;
use fleetworks_contracts::qa_worker::QA_WORKER_PROTOCOL;
use serde::Deserialize;
use sha2::Digest;
use sha2::Sha256;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;
use tokio::process::Child;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::watch;
use tokio::sync::Mutex;
use tokio::time::timeout;

const PACK_ROOT_ENV: &str = "FLEETWORKS_QA_PACK_ROOT";
const PACK_DIGEST_ENV: &str = "FLEETWORKS_QA_MANIFEST_SHA256";
const MANIFEST: &str = "manifest.json";
const WORKER_BINARY: &str = "bin/fleetworks-assistant-bert";
const SOCKET_LISTENER_BINARY: &str = "bin/fleetworks-assistant-bert-listener";
const MAX_POINTER_BYTES: u64 = 4 * 1024;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_PACK_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_PACK_FILES: usize = 32;
const WARM_LIMIT: Duration = Duration::from_secs(45);
const TURN_LIMIT: Duration = Duration::from_secs(25);
const PING_LIMIT: Duration = Duration::from_secs(2);
const UNLOAD_LIMIT: Duration = Duration::from_secs(3);
const MIB: u64 = 1024 * 1024;
pub(crate) const WORKSTATION_RESERVE: u64 = 1024 * MIB;
const SMALL_RESIDENT_LIMIT: u64 = 1536 * MIB;
const BASE_RESIDENT_LIMIT: u64 = 3584 * MIB;
const GPT2_SMALL_RESIDENT_LIMIT: u64 = 2048 * MIB;
const GPT2_MEDIUM_RESIDENT_LIMIT: u64 = 4608 * MIB;

fn configured_u16(name: &str) -> Option<u16> {
    env::var(name).ok()?.parse().ok()
}

fn select_target(
    preference: QaGeneratorPreference,
    available_memory: u64,
    operator_limit: u64,
) -> Option<QaModelTarget> {
    let limit = available_memory
        .saturating_sub(WORKSTATION_RESERVE)
        .min(operator_limit);
    let target = match preference {
        QaGeneratorPreference::Automatic if limit >= BASE_RESIDENT_LIMIT => {
            QaModelTarget::FlanT5Base
        }
        QaGeneratorPreference::Automatic | QaGeneratorPreference::FlanT5Small => {
            QaModelTarget::FlanT5Small
        }
        QaGeneratorPreference::FlanT5Base => QaModelTarget::FlanT5Base,
        QaGeneratorPreference::Gpt2Small => QaModelTarget::Gpt2Small,
        QaGeneratorPreference::Gpt2Medium => QaModelTarget::Gpt2Medium,
        _ => return None,
    };
    (limit >= resident_limit(target)).then_some(target)
}

fn resident_limit(target: QaModelTarget) -> u64 {
    match target {
        QaModelTarget::FlanT5Base => BASE_RESIDENT_LIMIT,
        QaModelTarget::Gpt2Small => GPT2_SMALL_RESIDENT_LIMIT,
        QaModelTarget::Gpt2Medium => GPT2_MEDIUM_RESIDENT_LIMIT,
        _ => SMALL_RESIDENT_LIMIT,
    }
}

async fn select_workstation_model(preference: QaGeneratorPreference) -> Result<QaModelTarget> {
    let available = tokio::task::spawn_blocking(hardware::available_memory_bytes).await?;
    let operator_limit = configured_u16("FLEETWORKS_QA_MEMORY_MIB")
        .filter(|n| (512..=6144).contains(n))
        .map_or(u64::MAX, |limit| u64::from(limit) * MIB);
    select_target(preference, available, operator_limit)
        .ok_or_else(|| WarmFailure(QaAvailabilityReason::InsufficientResources).into())
}

struct WorkerLimits {
    threads: u16,
    resident_bytes: u64,
}

impl WorkerLimits {
    fn for_target(target: QaModelTarget) -> Self {
        Self {
            threads: configured_u16("FLEETWORKS_QA_CPU_THREADS")
                .filter(|n| (1..=4).contains(n))
                .unwrap_or(1),
            resident_bytes: resident_limit(target),
        }
    }
}

#[derive(Debug)]
struct WarmFailure(QaAvailabilityReason);

impl fmt::Display for WarmFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Embedded Inbox QA worker unavailable")
    }
}

impl std::error::Error for WarmFailure {}

pub(crate) fn availability_reason(error: &anyhow::Error) -> QaAvailabilityReason {
    error
        .downcast_ref::<WarmFailure>()
        .map_or(QaAvailabilityReason::WorkerStartFailed, |failure| failure.0)
}

fn pack_pointer_error(error: anyhow::Error) -> anyhow::Error {
    let reason = if pointer_missing(&error) {
        QaAvailabilityReason::PackMissing
    } else {
        QaAvailabilityReason::PackInvalid
    };
    warm_error("pack_pointer", error.context(WarmFailure(reason)))
}

fn warm_error(stage: &'static str, error: anyhow::Error) -> anyhow::Error {
    tracing::warn!(
        qa_warm_stage = stage,
        sanitized_os_error = ?error.downcast_ref::<io::Error>().and_then(io::Error::raw_os_error),
        "Inbox QA worker warmup failed"
    );
    error
}

/// The executor may drop a warmup future before the supervisor's own deadline.
struct WarmStage {
    name: &'static str,
    started: Instant,
    completed: bool,
}

impl WarmStage {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            started: Instant::now(),
            completed: false,
        }
    }

    fn finish(&mut self) {
        self.completed = true;
    }
}

impl Drop for WarmStage {
    fn drop(&mut self) {
        if !self.completed {
            tracing::warn!(
                qa_warm_stage = self.name,
                qa_warm_reason = "abandoned",
                qa_warm_elapsed_ms = self.started.elapsed().as_millis() as u64,
                "Inbox QA worker warmup ended before the stage completed"
            );
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PackPointer {
    root: PathBuf,
    manifest_sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    protocol: u16,
    model_revision: String,
    target: String,
    files: BTreeMap<String, String>,
}

/// Owns the only isolated model child for this Embedded companion.
pub(crate) struct QaWorkerSupervisor {
    state_dir: PathBuf,
    model_admission: Arc<Mutex<()>>,
    session: Mutex<Option<WorkerSession>>,
    stopping: watch::Sender<()>,
}

impl QaWorkerSupervisor {
    pub(crate) fn embedded_for_state(state_dir: PathBuf) -> Self {
        Self {
            state_dir,
            model_admission: Arc::new(Mutex::new(())),
            session: Mutex::new(None),
            stopping: watch::channel(()).0,
        }
    }

    pub(crate) fn with_model_admission(mut self, admission: Arc<Mutex<()>>) -> Self {
        self.model_admission = admission;
        self
    }

    async fn connect(
        &self,
        mut target: QaModelTarget,
        preference: QaGeneratorPreference,
        selected_model: &(dyn Fn(QaModelTarget) + Sync),
        progress: &(dyn Fn(QaStartupStage) + Sync),
    ) -> Result<WorkerSession> {
        let state_dir = self.state_dir.clone();
        let (pointer_loaded, pointer_observed) = oneshot::channel();
        let mut stage = WarmStage::new("pack_verification");
        let verification = tokio::task::spawn_blocking(move || {
            let pointer = worker_pack_pointer(&state_dir, target).map_err(pack_pointer_error)?;
            let _ = pointer_loaded.send(());
            let invalid_pack = || WarmFailure(QaAvailabilityReason::PackInvalid);
            let (pack, packed_target) = verify_pack(&pointer)
                .map_err(|error| warm_error("pack_verification", error.context(invalid_pack())))?;
            ensure!(
                packed_target.is_some(),
                "QA pack requires a verified generation runtime"
            );
            sandbox::protect_state(&state_dir, &pack).map_err(|error| {
                warm_error("sandbox_state_boundary", error.context(invalid_pack()))
            })?;
            Ok::<_, anyhow::Error>((pack, packed_target))
        });
        if pointer_observed.await.is_ok() {
            progress(QaStartupStage::VerifyingPack);
        }
        let verified = timeout(WARM_LIMIT, verification).await;
        stage.finish();
        let (pack, packed_target) = verified?.map_err(|_| {
            warm_error(
                "pack_verification",
                anyhow!("QA pack verification did not finish"),
            )
        })??;
        let model = if packed_target == Some(target) {
            pack.join("model")
        } else {
            progress(QaStartupStage::DownloadingPack);
            match crate::qa_model_assets::provision(&self.state_dir, target).await {
                Ok(model) => model,
                Err(error)
                    if preference == QaGeneratorPreference::Automatic
                        && target == QaModelTarget::FlanT5Base
                        && packed_target == Some(QaModelTarget::FlanT5Small) =>
                {
                    let _ = warm_error("base_model_provision", error);
                    target = QaModelTarget::FlanT5Small;
                    selected_model(target);
                    pack.join("model")
                }
                Err(error) => {
                    return Err(warm_error(
                        "model_provision",
                        error.context(WarmFailure(QaAvailabilityReason::ModelProvisionFailed)),
                    ));
                }
            }
        };
        let model = tokio::fs::canonicalize(model).await?;
        sandbox::protect_state(&self.state_dir, &model)?;
        progress(QaStartupStage::LoadingModel);
        WorkerSession::child(&pack, &model, target)
    }

    async fn connect_ready(
        &self,
        slot: &mut Option<WorkerSession>,
        target: QaModelTarget,
        preference: QaGeneratorPreference,
        stopped: &mut watch::Receiver<()>,
        selected_model: &(dyn Fn(QaModelTarget) + Sync),
        progress: &(dyn Fn(QaStartupStage) + Sync),
    ) -> Result<()> {
        let session = slot.insert(tokio::select! {
            biased;
            _ = stopped.changed() => return Err(anyhow!("QA worker is stopping")),
            result = self.connect(target, preference, selected_model, progress) => result?,
        });
        let child_pid = session.child_id();
        let mut stage = WarmStage::new("worker_ready_frame");
        let resident_limit = session.resident_limit;
        let (ready, failure_stage) = tokio::select! {
            biased;
            _ = stopped.changed() => (Err(anyhow!("QA worker is stopping")), None),
            error = guard_resident(child_pid, resident_limit) => (Err(error), Some("worker_memory_limit")),
            result = timeout(WARM_LIMIT, session.ready()) => match result {
                Ok(result) => (result, Some("worker_ready_frame")),
                Err(_) => (Err(anyhow!("QA worker did not load in time")), Some("worker_ready_timeout")),
            },
        };
        stage.finish();
        if let Err(error) = ready {
            if let Some(stage) = failure_stage {
                let status = session
                    .child
                    .as_mut()
                    .and_then(|child| child.try_wait().ok().flatten());
                tracing::warn!(
                    qa_warm_stage = stage,
                    sanitized_os_error = ?error.downcast_ref::<io::Error>().and_then(io::Error::raw_os_error),
                    worker_exit_code = ?status.and_then(|status| status.code()),
                    worker_signal = ?status.and_then(|status| status.signal()),
                    "Inbox QA worker warmup failed"
                );
            }
            let _ = stop_worker(slot).await;
            return Err(error);
        }
        session.lifecycle = WorkerLifecycle::Ready;
        Ok(())
    }

    pub(crate) async fn warm_with_progress(
        &self,
        preference: QaGeneratorPreference,
        selected_model: &(dyn Fn(QaModelTarget) + Sync),
        progress: &(dyn Fn(QaStartupStage) + Sync),
    ) -> Result<()> {
        let mut stopped = self.stopping.subscribe();
        let _admission = self.model_admission.lock().await;
        let mut slot = self.session.lock().await;
        stop_worker(&mut slot).await?;
        let target = select_workstation_model(preference).await?;
        selected_model(target);
        self.connect_ready(
            &mut slot,
            target,
            preference,
            &mut stopped,
            selected_model,
            progress,
        )
        .await
    }

    pub(crate) async fn active_model(&self) -> Option<QaModelTarget> {
        self.session
            .lock()
            .await
            .as_ref()
            .filter(|session| session.lifecycle == WorkerLifecycle::Ready)
            .map(|session| session.model_target)
    }

    pub(crate) async fn response_plan_support(
        &self,
        needs_decoded_clarification: bool,
        needs_general_dialogue: bool,
    ) -> (bool, bool) {
        let session = self.session.lock().await;
        let Some(session) = session
            .as_ref()
            .filter(|session| session.lifecycle == WorkerLifecycle::Ready)
        else {
            return (false, false);
        };
        let support = session.response_plan;
        (
            support.supports(needs_decoded_clarification)
                && (!needs_general_dialogue || support.supports_general_dialogue()),
            support == ResponsePlanSupport::JobLogContext,
        )
    }

    pub(crate) async fn check_ready(&self) -> Result<()> {
        let mut slot = self.session.lock().await;
        let session = slot
            .as_mut()
            .filter(|session| session.lifecycle == WorkerLifecycle::Ready)
            .ok_or_else(|| anyhow!("QA worker unavailable"))?;
        let child_pid = session.child_id();
        let resident_limit = session.resident_limit;
        tokio::select! {
            error = guard_resident(child_pid, resident_limit) => Err(error),
            result = timeout(PING_LIMIT, session.ping()) => result?,
        }
    }
}

impl QaInference for QaWorkerSupervisor {
    fn ask(
        &self,
        request: QaWorkerRequest,
        events: mpsc::Sender<QaWorkerEvent>,
        cancel: CancellationToken,
    ) -> QaInferenceFuture<'_> {
        Box::pin(async move {
            request.validate()?;
            let mut stopped = self.stopping.subscribe();
            let mut slot = self.session.lock().await;
            let session = slot
                .as_mut()
                .filter(|session| session.lifecycle == WorkerLifecycle::Ready)
                .ok_or_else(|| anyhow!("QA worker unavailable"))?;
            ensure!(
                (session.response_plan.supports_general_dialogue()
                    || !request
                        .dialogue
                        .as_ref()
                        .is_some_and(|dialogue| dialogue.kind.needs_general_support()))
                    && request.plan.as_ref().is_none_or(|plan| session
                        .response_plan
                        .supports(plan.clarification.is_some())),
                "QA response plan support was not negotiated"
            );
            let child_pid = session.child_id();
            let resident_limit = session.resident_limit;
            let (result, failure_stage) = tokio::select! {
                biased;
                _ = stopped.changed() => (Err(anyhow!("QA worker is stopping")), None),
                _ = cancel.cancelled() => (Err(anyhow!("QA request was canceled")), None),
                error = guard_resident(child_pid, resident_limit) => (Err(error), Some("worker_memory_limit")),
                result = timeout(TURN_LIMIT, session.ask(request, events)) => match result {
                    Ok(Ok(())) => (Ok(()), None),
                    Ok(Err(error)) => {
                        let stage = if error.downcast_ref::<io::Error>().is_some_and(|error| {
                            matches!(error.kind(), io::ErrorKind::UnexpectedEof | io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset)
                        }) {
                            "worker_eof"
                        } else {
                            "worker_frame"
                        };
                        (Err(error), Some(stage))
                    }
                    Err(_) => (Err(anyhow!("QA request exceeded its execution budget")), Some("worker_turn_timeout")),
                },
            };
            if let Err(error) = &result {
                if let Some(stage) = failure_stage {
                    let status = session
                        .child
                        .as_mut()
                        .and_then(|child| child.try_wait().ok().flatten());
                    tracing::warn!(
                        qa_turn_stage = stage,
                        sanitized_os_error = ?error.downcast_ref::<io::Error>().and_then(io::Error::raw_os_error),
                        worker_exit_code = ?status.and_then(|status| status.code()),
                        worker_signal = ?status.and_then(|status| status.signal()),
                        "Inbox QA worker could not finish the turn"
                    );
                }
                let _ = stop_worker(&mut slot).await;
            }
            result
        })
    }

    fn shutdown(&self) -> QaInferenceFuture<'_> {
        Box::pin(async move {
            self.stopping.send_modify(|_| {});
            let mut slot = self.session.lock().await;
            stop_worker(&mut slot).await
        })
    }
}

async fn stop_worker(slot: &mut Option<WorkerSession>) -> Result<()> {
    if let Some(session) = slot.as_mut() {
        session.stop().await?;
        *slot = None;
    }
    Ok(())
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum WorkerLifecycle {
    Starting,
    Ready,
    Stopping,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ResponsePlanSupport {
    Unavailable,
    Legacy,
    DecodedClarification,
    GeneralDialogue,
    JobLogContext,
}

impl ResponsePlanSupport {
    fn supports(self, needs_decoded_clarification: bool) -> bool {
        match self {
            Self::Unavailable => false,
            Self::Legacy => !needs_decoded_clarification,
            Self::DecodedClarification | Self::GeneralDialogue | Self::JobLogContext => true,
        }
    }

    fn supports_general_dialogue(self) -> bool {
        matches!(self, Self::GeneralDialogue | Self::JobLogContext)
    }
}

struct WorkerSession {
    lifecycle: WorkerLifecycle,
    input: Box<dyn AsyncWrite + Send + Unpin>,
    output: Box<dyn AsyncRead + Send + Unpin>,
    child: Option<Child>,
    streaming_window: Option<u16>,
    response_plan: ResponsePlanSupport,
    model_target: QaModelTarget,
    resident_limit: u64,
}

impl WorkerSession {
    fn child(pack: &Path, model: &Path, target: QaModelTarget) -> Result<Self> {
        let limits = WorkerLimits::for_target(target);
        let threads = limits.threads.to_string();
        let mut command = sandbox::command(pack, model, &pack.join(WORKER_BINARY))
            .map_err(|error| warm_error("sandbox_setup", error))?;
        command
            .arg("--model-dir")
            .arg(model)
            .current_dir(model)
            .env(QA_CONTEXT_USAGE_ENV, "1")
            .env(QA_DIALOGUE_ENV, "1")
            .env(QA_STREAMING_ENV, "1")
            .env(QA_MODEL_CONTEXT_ENV, "1")
            .env(QA_RESPONSE_PLAN_ENV, "1")
            .env(QA_DECODED_CLARIFICATION_ENV, "1")
            .env(QA_GENERAL_DIALOGUE_ENV, "1")
            .env(QA_JOB_LOG_CONTEXT_ENV, "1")
            .env(QA_MODEL_TARGET_ENV, target.slug())
            .env("OMP_NUM_THREADS", &threads)
            .env("MKL_NUM_THREADS", &threads)
            .env("RAYON_NUM_THREADS", &threads)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .kill_on_drop(true);
        limit_embedded_worker(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| warm_error("worker_spawn", error.into()))?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| warm_error("worker_spawn", anyhow!("missing QA worker input")))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| warm_error("worker_spawn", anyhow!("missing QA worker output")))?;
        Ok(Self {
            lifecycle: WorkerLifecycle::Starting,
            input: Box::new(input),
            output: Box::new(output),
            child: Some(child),
            streaming_window: None,
            response_plan: ResponsePlanSupport::Unavailable,
            model_target: target,
            resident_limit: limits.resident_bytes,
        })
    }

    fn child_id(&self) -> Option<u32> {
        self.child.as_ref().and_then(Child::id)
    }

    async fn send(&mut self, command: &QaWorkerCommand) -> Result<()> {
        let bytes =
            serde_json::to_vec(command).map_err(|_| anyhow!("invalid QA worker command"))?;
        let header = u32::try_from(bytes.len())?.to_be_bytes();
        qa_frame_len(header)?;
        self.input.write_all(&header).await?;
        self.input.write_all(&bytes).await?;
        self.input.flush().await?;
        Ok(())
    }

    async fn receive(&mut self) -> Result<QaWorkerReply> {
        let mut header = [0; 4];
        self.output.read_exact(&mut header).await?;
        self.receive_payload(header).await
    }

    async fn receive_payload(&mut self, header: [u8; 4]) -> Result<QaWorkerReply> {
        let mut payload = vec![0; qa_frame_len(header)?];
        self.output.read_exact(&mut payload).await?;
        serde_json::from_slice(&payload).map_err(|_| anyhow!("invalid QA worker response"))
    }

    async fn ready(&mut self) -> Result<()> {
        let reply = self.receive().await?;
        self.validate_ready(reply)
    }

    fn validate_ready(&mut self, reply: QaWorkerReply) -> Result<()> {
        let (streaming_window, response_plan) = match reply {
            QaWorkerReply::ReadyWithDialogue {
                protocol,
                model_revision,
            } if protocol == QA_WORKER_PROTOCOL
                && model_revision == self.model_target.revision() =>
            {
                (None, ResponsePlanSupport::Unavailable)
            }
            QaWorkerReply::ReadyWithStreaming {
                protocol,
                model_revision,
            } if protocol == QA_WORKER_PROTOCOL
                && model_revision == self.model_target.revision() =>
            {
                (
                    Some(QA_LEGACY_STREAMING_WINDOW_TOKENS),
                    ResponsePlanSupport::Unavailable,
                )
            }
            QaWorkerReply::ReadyWithStreamingContext {
                protocol,
                model_revision,
                window_tokens,
            } if protocol == QA_WORKER_PROTOCOL
                && model_revision == self.model_target.revision()
                && window_tokens == self.model_target.input_tokens()
                && window_tokens > QA_LEGACY_STREAMING_WINDOW_TOKENS =>
            {
                (Some(window_tokens), ResponsePlanSupport::Unavailable)
            }
            QaWorkerReply::ReadyWithResponsePlan {
                protocol,
                model_revision,
                window_tokens,
            } if protocol == QA_WORKER_PROTOCOL
                && model_revision == self.model_target.revision()
                && (window_tokens == self.model_target.input_tokens()
                    || window_tokens == QA_LEGACY_STREAMING_WINDOW_TOKENS) =>
            {
                (Some(window_tokens), ResponsePlanSupport::Legacy)
            }
            QaWorkerReply::ReadyWithDecodedClarification {
                protocol,
                model_revision,
                window_tokens,
            } if protocol == QA_WORKER_PROTOCOL
                && model_revision == self.model_target.revision()
                && (window_tokens == self.model_target.input_tokens()
                    || window_tokens == QA_LEGACY_STREAMING_WINDOW_TOKENS) =>
            {
                (
                    Some(window_tokens),
                    ResponsePlanSupport::DecodedClarification,
                )
            }
            QaWorkerReply::ReadyWithGeneralDialogue {
                protocol,
                model_revision,
                window_tokens,
            } if protocol == QA_WORKER_PROTOCOL
                && model_revision == self.model_target.revision()
                && window_tokens == self.model_target.input_tokens() =>
            {
                (Some(window_tokens), ResponsePlanSupport::GeneralDialogue)
            }
            QaWorkerReply::ReadyWithJobLogContext {
                protocol,
                model_revision,
                window_tokens,
            } if protocol == QA_WORKER_PROTOCOL
                && model_revision == self.model_target.revision()
                && window_tokens == self.model_target.input_tokens() =>
            {
                (Some(window_tokens), ResponsePlanSupport::JobLogContext)
            }
            QaWorkerReply::StartupFailed {
                source,
                line,
                cause,
            } => {
                tracing::warn!(
                    qa_warm_stage = "worker_startup_panic",
                    worker_panic_source = source.code(),
                    worker_panic_line = line,
                    worker_panic_cause = cause.code(),
                    "Inbox QA worker reported a startup panic"
                );
                return Err(anyhow!("QA model process panicked during startup"));
            }
            _ => {
                return Err(anyhow!(
                    "QA worker could not confirm the expected checkpoint"
                ))
            }
        };
        ensure!(
            self.lifecycle != WorkerLifecycle::Ready
                || (self.streaming_window == streaming_window
                    && self.response_plan == response_plan),
            "QA worker changed its negotiated context"
        );
        self.streaming_window = streaming_window;
        self.response_plan = response_plan;
        Ok(())
    }

    async fn ping(&mut self) -> Result<()> {
        self.send(&QaWorkerCommand::Ping).await?;
        self.ready().await
    }

    async fn ask(
        &mut self,
        request: QaWorkerRequest,
        events: mpsc::Sender<QaWorkerEvent>,
    ) -> Result<()> {
        ensure!(
            self.response_plan == ResponsePlanSupport::JobLogContext
                || !request.passages.iter().any(|passage| passage.job_log_tail),
            "QA job log context support was not negotiated"
        );
        self.send(&QaWorkerCommand::Ask {
            request: Box::new(request.clone()),
        })
        .await?;
        let mut validation = QaWorkerSequence::default();
        loop {
            let QaWorkerReply::Event { event } = self.receive().await? else {
                return Err(anyhow!("QA worker stopped before its terminal result"));
            };
            let terminal = validation.accept(&request, &event)?;
            match &event {
                QaWorkerEvent::Delta { .. } => {
                    ensure!(self.streaming_window.is_some(), "unexpected QA stream");
                }
                QaWorkerEvent::Complete {
                    context: Some(context),
                    ..
                } => {
                    let window_matches = match self.streaming_window {
                        Some(window) => context.window_tokens == window,
                        None => context.window_tokens <= QA_LEGACY_STREAMING_WINDOW_TOKENS,
                    };
                    ensure!(window_matches, "unexpected QA context window");
                }
                _ => {}
            }
            events
                .send(event)
                .await
                .map_err(|_| anyhow!("QA event receiver closed"))?;
            if terminal {
                return Ok(());
            }
        }
    }

    async fn stop(&mut self) -> Result<()> {
        let try_graceful = self.lifecycle != WorkerLifecycle::Stopping;
        self.lifecycle = WorkerLifecycle::Stopping;
        if self.child.is_some() {
            if try_graceful
                && matches!(
                    timeout(UNLOAD_LIMIT, async {
                        self.send(&QaWorkerCommand::Shutdown).await?;
                        self.child
                            .as_mut()
                            .expect("owned worker child")
                            .wait()
                            .await?;
                        Ok::<_, anyhow::Error>(())
                    })
                    .await,
                    Ok(Ok(()))
                )
            {
                return Ok(());
            }
            // Keep the handle in the supervisor slot if this wait is interrupted.
            let child = self.child.as_mut().expect("owned worker child");
            if let Some(pid) = child.id().and_then(|pid| i32::try_from(pid).ok()) {
                // SAFETY: An unreaped child owns its PID and was spawned as process group leader.
                unsafe { libc::kill(-pid, libc::SIGKILL) };
            }
            // Tokio Child::kill waits for the subprocess to be reaped.
            timeout(UNLOAD_LIMIT, child.kill()).await??;
            return Ok(());
        }
        timeout(UNLOAD_LIMIT, async {
            self.send(&QaWorkerCommand::Shutdown).await?;
            // A canceled request can have at most the bounded set of events left in the pipe.
            for _ in 0..=fleetworks_contracts::qa_worker::QA_MAX_FINDINGS + 2 {
                if matches!(self.receive().await?, QaWorkerReply::Stopped) {
                    return Ok(());
                }
            }
            Err(anyhow!("QA worker unload was not confirmed"))
        })
        .await
        .map_err(|_| anyhow!("QA worker unload was not confirmed"))?
    }
}

pub(crate) async fn guard_resident(pid: Option<u32>, resident_limit: u64) -> anyhow::Error {
    let Some(pid) = pid else {
        return future::pending().await;
    };
    loop {
        if resident_bytes(pid)
            .await
            .is_some_and(|bytes| bytes > resident_limit)
        {
            return anyhow!("QA worker exceeded its resident memory budget");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[cfg(target_os = "linux")]
async fn resident_bytes(pid: u32) -> Option<u64> {
    let status = tokio::fs::read_to_string(format!("/proc/{pid}/status"))
        .await
        .ok()?;
    let resident = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?;
    resident
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?
        .checked_mul(1024)
}

#[cfg(target_os = "macos")]
async fn resident_bytes(pid: u32) -> Option<u64> {
    let mut info = std::mem::MaybeUninit::<libc::proc_taskinfo>::uninit();
    let size = i32::try_from(std::mem::size_of::<libc::proc_taskinfo>()).ok()?;
    let pid = i32::try_from(pid).ok()?;
    // SAFETY: proc_pidinfo fills the correctly sized output buffer for this process-info flavor.
    let written = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTASKINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if written != size {
        return None;
    }
    // SAFETY: The kernel reported that it wrote a complete proc_taskinfo value.
    Some(unsafe { info.assume_init() }.pti_resident_size)
}

pub(crate) fn limit_embedded_worker(command: &mut Command) {
    // SAFETY: setpriority applies only to the forked worker before exec.
    unsafe {
        command.pre_exec(|| {
            if libc::setpriority(libc::PRIO_PROCESS, 0, 10) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    #[cfg(target_os = "linux")]
    // SAFETY: The hook makes only async-signal-safe getrlimit/setrlimit calls before exec.
    unsafe {
        command.pre_exec(|| {
            const MAX_ADDRESS_BYTES: libc::rlim_t = 8 * 1024 * 1024 * 1024;
            let mut current = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            if libc::getrlimit(libc::RLIMIT_AS, &mut current) != 0 {
                return Err(io::Error::last_os_error());
            }
            current.rlim_cur = current.rlim_cur.min(MAX_ADDRESS_BYTES);
            if libc::setrlimit(libc::RLIMIT_AS, &current) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    #[cfg(not(target_os = "linux"))]
    let _ = command;
}

fn uid() -> u32 {
    // SAFETY: geteuid has no inputs and always returns the effective user ID.
    unsafe { libc::geteuid() }
}

fn override_pack() -> Result<Option<PackPointer>> {
    match (env::var_os(PACK_ROOT_ENV), env::var(PACK_DIGEST_ENV)) {
        (None, Err(env::VarError::NotPresent)) => Ok(None),
        (Some(root), Ok(manifest_sha256)) => Ok(Some(PackPointer {
            root: root.into(),
            manifest_sha256,
        })),
        _ => Err(anyhow!("incomplete QA pack configuration")),
    }
}

fn protect_private_directory(path: &Path) -> Result<()> {
    let directory = fs::symlink_metadata(path)?;
    ensure!(
        directory.is_dir() && directory.uid() == uid() && directory.mode() & 0o777 == 0o700,
        "QA state directory must be private"
    );
    Ok(())
}

fn pointer_missing(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<io::Error>()
        .is_some_and(|error| error.kind() == io::ErrorKind::NotFound)
}

fn worker_pack_pointer(state_dir: &Path, target: QaModelTarget) -> Result<PackPointer> {
    if let Some(pointer) = override_pack()? {
        return Ok(pointer);
    }
    load_installed_pointer(state_dir, target)
}

fn load_installed_pointer(state_dir: &Path, target: QaModelTarget) -> Result<PackPointer> {
    protect_private_directory(state_dir)?;
    let targets = state_dir.join("qa-model-packs");
    let named = match protect_private_directory(&targets) {
        Ok(()) => true,
        Err(error) if pointer_missing(&error) => false,
        Err(error) => return Err(error),
    };
    if named {
        match read_pointer(&targets.join(format!("{}.json", target.slug()))) {
            Err(error) if pointer_missing(&error) => {}
            other => return other,
        }
    }
    match read_pointer(&state_dir.join("qa-pack.json")) {
        Err(error)
            if pointer_missing(&error)
                && named
                && matches!(
                    target,
                    QaModelTarget::FlanT5Base
                        | QaModelTarget::Gpt2Small
                        | QaModelTarget::Gpt2Medium
                ) =>
        {
            // The standard Small pack also contains the shared verified generator runtime.
            read_pointer(&targets.join("flan-t5-small.json"))
        }
        other => other,
    }
}

fn read_pointer(path: &Path) -> Result<PackPointer> {
    let mut pointer = open_pack_file(path, MAX_POINTER_BYTES)?;
    ensure!(
        pointer.metadata()?.mode() & 0o777 == 0o600,
        "QA pack pointer must be private"
    );
    let mut bytes = Vec::new();
    pointer
        .by_ref()
        .take(MAX_POINTER_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_POINTER_BYTES,
        "QA pack pointer exceeds its limit"
    );
    serde_json::from_slice(&bytes).map_err(|_| anyhow!("invalid QA pack pointer"))
}

fn open_pack_file(path: &Path, max_size: u64) -> Result<fs::File> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.nlink() == 1
            && metadata.uid() == uid()
            && metadata.mode() & 0o022 == 0
            && (1..=max_size).contains(&metadata.len()),
        "QA pack file must have a protected identity"
    );
    Ok(file)
}

fn verify_pack(pointer: &PackPointer) -> Result<(PathBuf, Option<QaModelTarget>)> {
    ensure!(
        pointer.root.is_absolute(),
        "QA pack directory must be absolute"
    );
    ensure!(
        valid_sha(&pointer.manifest_sha256),
        "invalid QA pack manifest digest"
    );
    let metadata = fs::symlink_metadata(&pointer.root)?;
    ensure!(
        metadata.is_dir() && metadata.uid() == uid() && metadata.mode() & 0o022 == 0,
        "QA pack directory must be protected"
    );
    let root = pointer.root.canonicalize()?;
    let manifest = open_pack_file(&root.join(MANIFEST), MAX_MANIFEST_BYTES)?;
    let mut bytes = Vec::new();
    manifest
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_MANIFEST_BYTES,
        "QA pack manifest exceeds its limit"
    );
    ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == pointer.manifest_sha256,
        "QA pack manifest digest mismatch"
    );
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|_| anyhow!("invalid QA pack manifest"))?;
    let legacy_model = manifest.model_revision == QA_MODEL_REVISION;
    let mut packed_target = QaModelTarget::ALL
        .into_iter()
        .find(|model| model.revision() == manifest.model_revision);
    let has_generator = manifest
        .files
        .keys()
        .any(|name| name.starts_with("model/generator/"));
    if legacy_model && has_generator {
        packed_target = Some(QaModelTarget::FlanT5Small);
    }
    ensure!(
        manifest.protocol == QA_WORKER_PROTOCOL
            && (legacy_model || packed_target.is_some())
            && manifest.target == host_target()
            && manifest.files.len() <= MAX_PACK_FILES
            && manifest.files.contains_key(WORKER_BINARY)
            && manifest.files.keys().all(|name| allowed_pack_file(name)),
        "QA pack is incompatible with this companion"
    );
    for (name, digest) in QA_MODEL_FILES {
        let entry = manifest
            .files
            .get(&format!("model/{name}"))
            .map(String::as_str);
        ensure!(
            if legacy_model {
                entry == Some(digest)
            } else {
                entry.is_none()
            },
            "QA pack contains the wrong model"
        );
    }
    if let Some(target) = packed_target {
        let files = crate::qa_model_assets::expected_files(target)?;
        ensure!(
            manifest
                .files
                .iter()
                .filter(|(name, _)| name.starts_with("model/generator/"))
                .count()
                == files.len(),
            "QA pack has unexpected generator assets"
        );
        for (name, digest) in files {
            ensure!(
                manifest.files.get(&format!("model/generator/{name}")) == Some(&digest),
                "QA pack contains an incomplete or incompatible generator"
            );
        }
    }
    let max_generator_bytes = packed_target.map_or(0, |target| target.weight_bytes() + 4 * MIB);

    // The directory inventory rejects an unverified file anywhere in the loader path.
    let mut pending = vec![root.clone()];
    let mut count = 0;
    let mut total_bytes = 0;
    let mut generator_bytes = 0;
    let mut seen = 0;
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            count += 1;
            ensure!(
                count <= 2 * MAX_PACK_FILES,
                "QA pack directory is too large"
            );
            let path = entry.path();
            let relative = path.strip_prefix(&root)?;
            ensure!(
                relative
                    .components()
                    .all(|c| matches!(c, Component::Normal(_))),
                "invalid QA pack path"
            );
            let relative = relative
                .to_str()
                .ok_or_else(|| anyhow!("invalid QA pack encoding"))?;
            if relative == MANIFEST {
                continue;
            }
            let kind = entry.file_type()?;
            if kind.is_dir() {
                let metadata = entry.metadata()?;
                ensure!(
                    metadata.uid() == uid() && metadata.mode() & 0o022 == 0,
                    "QA pack directory must be protected"
                );
                pending.push(path);
                continue;
            }
            let expected = manifest
                .files
                .get(relative)
                .ok_or_else(|| anyhow!("unverified QA pack file"))?;
            ensure!(valid_sha(expected), "invalid QA pack file digest");
            let (used, limit) = if relative.starts_with("model/generator/") {
                (&mut generator_bytes, max_generator_bytes)
            } else {
                (&mut total_bytes, MAX_PACK_BYTES)
            };
            let file = open_pack_file(&path, limit - *used)?;
            if matches!(relative, WORKER_BINARY | SOCKET_LISTENER_BINARY) {
                ensure!(
                    file.metadata()?.mode() & 0o100 != 0,
                    "QA worker is not executable"
                );
            }
            let mut digest = Sha256::new();
            *used += io::copy(&mut file.take(limit - *used + 1), &mut digest)?;
            ensure!(*used <= limit, "QA pack exceeds its size limit");
            ensure!(
                format!("{:x}", digest.finalize()) == *expected,
                "QA pack digest mismatch"
            );
            seen += 1;
        }
    }
    ensure!(
        seen == manifest.files.len() && root.join("lib").is_dir(),
        "QA pack is incomplete"
    );
    Ok((root, packed_target))
}

fn allowed_pack_file(name: &str) -> bool {
    matches!(name, WORKER_BINARY | SOCKET_LISTENER_BINARY)
        || QA_MODEL_FILES
            .iter()
            .any(|(file, _)| name.strip_prefix("model/") == Some(*file))
        || name.strip_prefix("model/generator/").is_some_and(|file| {
            matches!(
                file,
                "config.json"
                    | "spiece.model"
                    | "model.safetensors"
                    | "rust_model.ot"
                    | "merges.txt"
                    | "vocab.json"
            )
        })
        || name
            .strip_prefix("lib/")
            .is_some_and(|file| !file.contains('/'))
        || name == "licenses/torch-LICENSE"
        || name
            .strip_prefix("licenses/torch-NOTICE")
            .is_some_and(|suffix| !suffix.contains('/'))
}

fn valid_sha(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn host_target() -> String {
    let arch = env::consts::ARCH;
    if cfg!(target_os = "macos") {
        format!("{arch}-apple-darwin")
    } else {
        format!("{arch}-unknown-linux-gnu")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn selects_largest_model_with_memory_left_for_the_workstation() {
        use super::{
            select_target, BASE_RESIDENT_LIMIT, SMALL_RESIDENT_LIMIT, WORKSTATION_RESERVE,
        };
        use fleetworks_contracts::chat::QaModelTarget;

        let base_available = BASE_RESIDENT_LIMIT + WORKSTATION_RESERVE;
        let small_available = SMALL_RESIDENT_LIMIT + WORKSTATION_RESERVE;
        assert_eq!(
            select_target(QaGeneratorPreference::Automatic, base_available, u64::MAX),
            Some(QaModelTarget::FlanT5Base)
        );
        assert_eq!(
            select_target(
                QaGeneratorPreference::Automatic,
                base_available - 1,
                u64::MAX
            ),
            Some(QaModelTarget::FlanT5Small)
        );
        assert_eq!(
            select_target(
                QaGeneratorPreference::Automatic,
                base_available,
                SMALL_RESIDENT_LIMIT
            ),
            Some(QaModelTarget::FlanT5Small)
        );
        assert_eq!(
            select_target(QaGeneratorPreference::Automatic, small_available, u64::MAX),
            Some(QaModelTarget::FlanT5Small)
        );
        assert_eq!(
            select_target(
                QaGeneratorPreference::Automatic,
                small_available - 1,
                u64::MAX
            ),
            None
        );
        assert_eq!(
            select_target(
                QaGeneratorPreference::Automatic,
                base_available,
                SMALL_RESIDENT_LIMIT - 1
            ),
            None
        );
    }

    #[test]
    fn explicit_model_preferences_require_their_own_memory_budget() {
        let base_available = BASE_RESIDENT_LIMIT + WORKSTATION_RESERVE;
        let small_available = SMALL_RESIDENT_LIMIT + WORKSTATION_RESERVE;
        assert_eq!(
            select_target(QaGeneratorPreference::FlanT5Small, base_available, u64::MAX),
            Some(QaModelTarget::FlanT5Small),
            "Explicit Small must remain Small on a Base-capable host"
        );
        assert_eq!(
            select_target(QaGeneratorPreference::FlanT5Base, base_available, u64::MAX),
            Some(QaModelTarget::FlanT5Base)
        );
        assert_eq!(
            select_target(
                QaGeneratorPreference::FlanT5Base,
                base_available,
                SMALL_RESIDENT_LIMIT
            ),
            None,
            "An operator cap must not silently substitute Small for explicit Base"
        );
        assert_eq!(
            select_target(
                QaGeneratorPreference::FlanT5Base,
                base_available - 1,
                u64::MAX
            ),
            None
        );
        assert_eq!(
            select_target(
                QaGeneratorPreference::FlanT5Small,
                small_available - 1,
                u64::MAX
            ),
            None
        );
    }

    #[test]
    fn explicit_gpt2_preferences_admit_only_the_requested_target() {
        for (preference, target, resident) in [
            (
                QaGeneratorPreference::Gpt2Small,
                QaModelTarget::Gpt2Small,
                GPT2_SMALL_RESIDENT_LIMIT,
            ),
            (
                QaGeneratorPreference::Gpt2Medium,
                QaModelTarget::Gpt2Medium,
                GPT2_MEDIUM_RESIDENT_LIMIT,
            ),
        ] {
            let available = resident + WORKSTATION_RESERVE;
            assert_eq!(select_target(preference, available, resident), Some(target));
            assert_eq!(select_target(preference, available, resident - 1), None);
            assert_eq!(select_target(preference, available - 1, resident), None);
            assert_eq!(WorkerLimits::for_target(target).resident_bytes, resident);
        }
    }

    use std::os::unix::fs::PermissionsExt;

    use fleetworks_contracts::qa_worker::{
        QaContextUsage, QaDialogueKind, QaWorkerDialogue, QaWorkerOutcome,
    };
    use tokio::net::UnixStream;
    use tokio::sync::oneshot;

    use super::*;

    async fn read_command(peer: &mut UnixStream) -> QaWorkerCommand {
        let mut header = [0; 4];
        peer.read_exact(&mut header).await.unwrap();
        let mut bytes = vec![0; qa_frame_len(header).unwrap()];
        peer.read_exact(&mut bytes).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn write_reply(peer: &mut UnixStream, reply: QaWorkerReply) {
        let bytes = serde_json::to_vec(&reply).unwrap();
        let header = u32::try_from(bytes.len()).unwrap().to_be_bytes();
        peer.write_all(&header).await.unwrap();
        peer.write_all(&bytes).await.unwrap();
    }

    #[test]
    fn classifies_missing_and_invalid_local_pack_and_startup_failure() {
        let state = tempfile::tempdir().unwrap();
        fs::set_permissions(state.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let reason = || match load_installed_pointer(state.path(), QaModelTarget::FlanT5Small)
            .map_err(pack_pointer_error)
        {
            Ok(_) => panic!("fixture must not load a pack"),
            Err(error) => availability_reason(&error),
        };
        assert_eq!(reason(), QaAvailabilityReason::PackMissing);
        let pointer = state.path().join("qa-pack.json");
        fs::write(&pointer, b"malformed pointer").unwrap();
        fs::set_permissions(&pointer, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(reason(), QaAvailabilityReason::PackInvalid);
        assert_eq!(
            availability_reason(&anyhow!("private synthetic worker failure")),
            QaAvailabilityReason::WorkerStartFailed
        );
    }

    #[test]
    fn alternate_models_find_named_small_runtime_when_preferred_pointers_are_missing() {
        let state = tempfile::tempdir().unwrap();
        fs::set_permissions(state.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let targets = state.path().join("qa-model-packs");
        fs::create_dir(&targets).unwrap();
        fs::set_permissions(&targets, fs::Permissions::from_mode(0o700)).unwrap();
        let target = select_target(
            QaGeneratorPreference::Automatic,
            BASE_RESIDENT_LIMIT + WORKSTATION_RESERVE,
            u64::MAX,
        )
        .unwrap();
        assert_eq!(target, QaModelTarget::FlanT5Base);
        assert!(!state.path().join("qa-model-assets").exists());

        let small = targets.join("flan-t5-small.json");
        let generic = state.path().join("qa-pack.json");
        let write = |path: &Path, root: &str| {
            let pointer = serde_json::json!({
                "root": root,
                "manifest_sha256": "a".repeat(64)
            });
            fs::write(path, pointer.to_string()).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        };
        write(&small, "/named-small");
        for target in [target, QaModelTarget::Gpt2Small, QaModelTarget::Gpt2Medium] {
            let named = targets.join(format!("{}.json", target.slug()));
            let found = || load_installed_pointer(state.path(), target).map(|pointer| pointer.root);
            assert_eq!(found().unwrap(), Path::new("/named-small"));

            write(&generic, "/default-pack");
            assert_eq!(found().unwrap(), Path::new("/default-pack"));
            write(&named, "/named-exact");
            assert_eq!(found().unwrap(), Path::new("/named-exact"));

            fs::write(&named, b"malformed pointer").unwrap();
            assert!(!pointer_missing(&found().unwrap_err()));
            fs::remove_file(&named).unwrap();
            fs::write(&generic, b"malformed pointer").unwrap();
            assert!(!pointer_missing(&found().unwrap_err()));
            fs::remove_file(&generic).unwrap();
        }
    }

    #[tokio::test]
    async fn retry_reaps_an_interrupted_shutdown_without_reusing_the_worker() {
        let (io, mut peer) = UnixStream::pair().unwrap();
        let (reader, writer) = io.into_split();
        let child = Command::new("/bin/cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        let target = QaModelTarget::FlanT5Small;
        let supervisor = QaWorkerSupervisor::embedded_for_state(PathBuf::new());
        *supervisor.session.lock().await = Some(WorkerSession {
            lifecycle: WorkerLifecycle::Ready,
            input: Box::new(writer),
            output: Box::new(reader),
            child: Some(child),
            streaming_window: Some(QA_LEGACY_STREAMING_WINDOW_TOKENS),
            response_plan: ResponsePlanSupport::Unavailable,
            model_target: target,
            resident_limit: WorkerLimits::for_target(target).resident_bytes,
        });
        {
            let shutdown = supervisor.shutdown();
            tokio::pin!(shutdown);
            let command = timeout(PING_LIMIT, async {
                tokio::select! {
                    result = &mut shutdown => panic!("worker exited before interrupted wait: {result:?}"),
                    command = read_command(&mut peer) => command,
                }
            })
            .await
            .unwrap();
            assert!(matches!(command, QaWorkerCommand::Shutdown));
        }
        assert_eq!(
            supervisor
                .session
                .lock()
                .await
                .as_ref()
                .and_then(WorkerSession::child_id),
            Some(pid)
        );
        assert_eq!(supervisor.active_model().await, None);
        assert!(supervisor.check_ready().await.is_err());

        supervisor.shutdown().await.unwrap();
        assert!(supervisor.session.lock().await.is_none());
    }

    #[tokio::test]
    async fn gpt2_streaming_receipts_use_the_negotiated_window() {
        let legacy = QA_LEGACY_STREAMING_WINDOW_TOKENS;
        for target in [QaModelTarget::Gpt2Small, QaModelTarget::Gpt2Medium] {
            let extended = target.input_tokens();
            let legacy_ready = || QaWorkerReply::ReadyWithStreaming {
                protocol: QA_WORKER_PROTOCOL,
                model_revision: target.revision().into(),
            };
            let model_ready = |window_tokens| QaWorkerReply::ReadyWithStreamingContext {
                protocol: QA_WORKER_PROTOCOL,
                model_revision: target.revision().into(),
                window_tokens,
            };
            for (advertised, received) in [
                (legacy, legacy),
                (legacy, extended),
                (extended, extended),
                (extended, legacy),
            ] {
                let (io, mut peer) = UnixStream::pair().unwrap();
                let (reader, writer) = io.into_split();
                let mut session = WorkerSession {
                    lifecycle: WorkerLifecycle::Starting,
                    input: Box::new(writer),
                    output: Box::new(reader),
                    child: None,
                    streaming_window: None,
                    response_plan: ResponsePlanSupport::Unavailable,
                    model_target: target,
                    resident_limit: WorkerLimits::for_target(target).resident_bytes,
                };
                assert!(session.validate_ready(model_ready(legacy)).is_err());

                let usage = QaContextUsage::new(received, 82, 1);
                let child = async {
                    let ready = if advertised == legacy {
                        legacy_ready()
                    } else {
                        model_ready(advertised)
                    };
                    write_reply(&mut peer, ready).await;
                    let QaWorkerCommand::Ask { request } = read_command(&mut peer).await else {
                        panic!("missing inference request");
                    };
                    write_reply(
                        &mut peer,
                        QaWorkerReply::Event {
                            event: QaWorkerEvent::Complete {
                                turn_id: request.turn_id,
                                sequence: 1,
                                outcome: QaWorkerOutcome::Answered,
                                context: Some(usage),
                                reply: Some("Hello!".into()),
                            },
                        },
                    )
                    .await;
                };
                let executor = async {
                    session.ready().await.unwrap();
                    session.lifecycle = WorkerLifecycle::Ready;
                    assert_eq!(session.streaming_window, Some(advertised));
                    let other_ready = if advertised == legacy {
                        model_ready(extended)
                    } else {
                        legacy_ready()
                    };
                    assert!(session.validate_ready(other_ready).is_err());

                    let mut request = QaWorkerRequest::new(
                        "turn-a".into(),
                        "snapshot-a".into(),
                        "Hi!".into(),
                        Vec::new(),
                    );
                    request.dialogue = Some(QaWorkerDialogue {
                        kind: QaDialogueKind::Greeting,
                        history: Vec::new(),
                    });
                    let (events, mut stream) = mpsc::channel(1);
                    let result = session.ask(request, events).await;
                    if advertised == received {
                        result.unwrap();
                        assert!(matches!(
                            stream.recv().await,
                            Some(QaWorkerEvent::Complete { context: Some(context), .. }) if context == usage
                        ));
                    } else {
                        assert_eq!(
                            result.unwrap_err().to_string(),
                            "unexpected QA context window"
                        );
                        assert!(stream.recv().await.is_none());
                    }
                };
                timeout(PING_LIMIT, async { tokio::join!(child, executor) })
                    .await
                    .unwrap();
            }
        }
    }

    #[tokio::test]
    async fn reply_plan_negotiation_is_fixed_at_startup() {
        for target in QaModelTarget::ALL {
            let (io, _peer) = UnixStream::pair().unwrap();
            let (reader, writer) = io.into_split();
            let mut session = WorkerSession {
                lifecycle: WorkerLifecycle::Starting,
                input: Box::new(writer),
                output: Box::new(reader),
                child: None,
                streaming_window: None,
                response_plan: ResponsePlanSupport::Unavailable,
                model_target: target,
                resident_limit: WorkerLimits::for_target(target).resident_bytes,
            };
            let plan = |window_tokens| QaWorkerReply::ReadyWithResponsePlan {
                protocol: QA_WORKER_PROTOCOL,
                model_revision: target.revision().into(),
                window_tokens,
            };
            let decoded = |window_tokens| QaWorkerReply::ReadyWithDecodedClarification {
                protocol: QA_WORKER_PROTOCOL,
                model_revision: target.revision().into(),
                window_tokens,
            };
            let general = |window_tokens| QaWorkerReply::ReadyWithGeneralDialogue {
                protocol: QA_WORKER_PROTOCOL,
                model_revision: target.revision().into(),
                window_tokens,
            };
            let job_log_context = QaWorkerReply::ReadyWithJobLogContext {
                protocol: QA_WORKER_PROTOCOL,
                model_revision: target.revision().into(),
                window_tokens: target.input_tokens(),
            };
            assert!(session.validate_ready(plan(256)).is_err());
            session.validate_ready(plan(target.input_tokens())).unwrap();
            session.lifecycle = WorkerLifecycle::Ready;
            session.validate_ready(plan(target.input_tokens())).unwrap();
            assert!(session.response_plan.supports(false));
            assert!(!session.response_plan.supports(true));
            assert!(!session.response_plan.supports_general_dialogue());
            assert!(session
                .validate_ready(decoded(target.input_tokens()))
                .is_err());
            assert!(session
                .validate_ready(QaWorkerReply::ReadyWithStreaming {
                    protocol: QA_WORKER_PROTOCOL,
                    model_revision: target.revision().into(),
                })
                .is_err());
            session.lifecycle = WorkerLifecycle::Starting;
            assert!(session.validate_ready(decoded(256)).is_err());
            session
                .validate_ready(decoded(target.input_tokens()))
                .unwrap();
            session.lifecycle = WorkerLifecycle::Ready;
            session
                .validate_ready(decoded(target.input_tokens()))
                .unwrap();
            assert!(session.response_plan.supports(true));
            assert!(!session.response_plan.supports_general_dialogue());
            assert!(session.validate_ready(plan(target.input_tokens())).is_err());
            session.lifecycle = WorkerLifecycle::Starting;
            if target.input_tokens() != QA_LEGACY_STREAMING_WINDOW_TOKENS {
                assert!(session
                    .validate_ready(general(QA_LEGACY_STREAMING_WINDOW_TOKENS))
                    .is_err());
                session
                    .validate_ready(decoded(QA_LEGACY_STREAMING_WINDOW_TOKENS))
                    .unwrap();
            }
            session
                .validate_ready(general(target.input_tokens()))
                .unwrap();
            assert_eq!(session.streaming_window, Some(target.input_tokens()));
            session.lifecycle = WorkerLifecycle::Ready;
            session
                .validate_ready(general(target.input_tokens()))
                .unwrap();
            assert!(session
                .validate_ready(decoded(target.input_tokens()))
                .is_err());
            assert!(session.response_plan.supports_general_dialogue());
            session.lifecycle = WorkerLifecycle::Starting;
            session.validate_ready(job_log_context).unwrap();
            assert!(session.response_plan.supports_general_dialogue());
        }
    }

    #[tokio::test]
    async fn streaming_draft_arrives_before_the_canonical_completion() {
        let (io, mut peer) = UnixStream::pair().unwrap();
        let (input, output) = io.into_split();
        let target = QaModelTarget::FlanT5Small;
        let usage = QaContextUsage::new(target.input_tokens(), 82, 1);
        let mut session = WorkerSession {
            lifecycle: WorkerLifecycle::Starting,
            input: Box::new(output),
            output: Box::new(input),
            child: None,
            streaming_window: None,
            response_plan: ResponsePlanSupport::Unavailable,
            model_target: target,
            resident_limit: WorkerLimits::for_target(target).resident_bytes,
        };
        let (release, next) = oneshot::channel();
        let child = async {
            write_reply(
                &mut peer,
                QaWorkerReply::ReadyWithStreaming {
                    protocol: QA_WORKER_PROTOCOL,
                    model_revision: target.revision().into(),
                },
            )
            .await;
            let QaWorkerCommand::Ask { request } = read_command(&mut peer).await else {
                panic!("missing inference request");
            };
            let turn_id = request.turn_id;
            write_reply(
                &mut peer,
                QaWorkerReply::Event {
                    event: QaWorkerEvent::Delta {
                        turn_id: turn_id.clone(),
                        sequence: 1,
                        text: "Hello".into(),
                    },
                },
            )
            .await;
            next.await.unwrap();
            write_reply(
                &mut peer,
                QaWorkerReply::Event {
                    event: QaWorkerEvent::Complete {
                        turn_id,
                        sequence: 2,
                        outcome: QaWorkerOutcome::Answered,
                        context: Some(usage),
                        reply: Some("Hello!".into()),
                    },
                },
            )
            .await;
            assert!(matches!(
                read_command(&mut peer).await,
                QaWorkerCommand::Shutdown
            ));
            write_reply(&mut peer, QaWorkerReply::Stopped).await;
        };
        let executor = async {
            session.ready().await.unwrap();
            let mut request = QaWorkerRequest::new(
                "turn-a".into(),
                "snapshot-a".into(),
                "Hi!".into(),
                Vec::new(),
            );
            request.dialogue = Some(QaWorkerDialogue {
                kind: QaDialogueKind::Greeting,
                history: Vec::new(),
            });
            let (events, mut stream) = mpsc::channel(1);
            let consume = async {
                assert!(matches!(stream.recv().await,
                    Some(QaWorkerEvent::Delta { sequence: 1, text, .. }) if text == "Hello"));
                release.send(()).unwrap();
                assert!(matches!(stream.recv().await,
                    Some(QaWorkerEvent::Complete { sequence: 2, context: Some(context), .. }) if context == usage));
            };
            let (result, ()) = tokio::join!(session.ask(request, events), consume);
            result.unwrap();
            session.stop().await.unwrap();
        };
        timeout(PING_LIMIT, async { tokio::join!(child, executor) })
            .await
            .unwrap();
    }
}
