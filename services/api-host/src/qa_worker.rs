//! Private connection to one Inbox QA model process.

#[path = "qa_sandbox.rs"]
mod sandbox;

use std::collections::BTreeMap;
use std::env;
use std::fmt;
use std::fs;
use std::future;
use std::io;
use std::io::Read;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

use anyhow::anyhow;
use anyhow::ensure;
use anyhow::Result;
use argos_osint_api::inbox_qa::CancellationToken;
use argos_osint_api::inbox_qa::QaInference;
use argos_osint_api::inbox_qa::QaInferenceFuture;
use fleetworks_contracts::chat::QaAvailabilityReason;
use fleetworks_contracts::chat::QaStartupStage;
use fleetworks_contracts::qa_worker::qa_frame_len;
use fleetworks_contracts::qa_worker::QaContextUsage;
use fleetworks_contracts::qa_worker::QaDialogueKind;
use fleetworks_contracts::qa_worker::QaWorkerCommand;
use fleetworks_contracts::qa_worker::QaWorkerEvent;
use fleetworks_contracts::qa_worker::QaWorkerOutcome;
use fleetworks_contracts::qa_worker::QaWorkerReply;
use fleetworks_contracts::qa_worker::QaWorkerRequest;
use fleetworks_contracts::qa_worker::QaWorkerSequence;
use fleetworks_contracts::qa_worker::QA_CHAT_MODEL_REVISION;
use fleetworks_contracts::qa_worker::QA_CONTEXT_USAGE_ENV;
use fleetworks_contracts::qa_worker::QA_CONTEXT_WINDOW_TOKENS;
use fleetworks_contracts::qa_worker::QA_DIALOGUE_ENV;
use fleetworks_contracts::qa_worker::QA_GENERATOR_FILES;
use fleetworks_contracts::qa_worker::QA_MODEL_FILES;
use fleetworks_contracts::qa_worker::QA_MODEL_REVISION;
use fleetworks_contracts::qa_worker::QA_WORKER_PROTOCOL;
use serde::Deserialize;
use sha2::Digest;
use sha2::Sha256;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::process::Child;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::watch;
use tokio::sync::Mutex;
use tokio::time::timeout;

const PACK_ROOT_ENV: &str = "FLEETWORKS_QA_PACK_ROOT";
const PACK_DIGEST_ENV: &str = "FLEETWORKS_QA_MANIFEST_SHA256";
const SOCKET_ENV: &str = "FLEETWORKS_QA_SOCKET";
const MANIFEST: &str = "manifest.json";
const WORKER_BINARY: &str = "bin/fleetworks-assistant-bert";
const SOCKET_LISTENER_BINARY: &str = "bin/fleetworks-assistant-bert-listener";
const MAX_POINTER_BYTES: u64 = 4 * 1024;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_PACK_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_GENERATOR_BYTES: u64 = 310_000_000;
const MAX_PACK_FILES: usize = 32;
const WARM_LIMIT: Duration = Duration::from_secs(45);
const TURN_LIMIT: Duration = Duration::from_secs(25);
const PING_LIMIT: Duration = Duration::from_secs(2);
const UNLOAD_LIMIT: Duration = Duration::from_secs(3);
const MAX_RESIDENT_BYTES: u64 = 1536 * 1024 * 1024;

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
    let reason = if error
        .downcast_ref::<io::Error>()
        .is_some_and(|error| error.kind() == io::ErrorKind::NotFound)
    {
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

enum WorkerTarget {
    EmbeddedState(PathBuf),
    Distributed(PathBuf),
}

/// The selected transport is fixed for the lifetime of the backend service.
pub(crate) struct QaWorkerSupervisor {
    target: WorkerTarget,
    session: Mutex<Option<WorkerSession>>,
    stopping: watch::Sender<()>,
}

impl QaWorkerSupervisor {
    fn new(target: WorkerTarget) -> Self {
        Self {
            target,
            session: Mutex::new(None),
            stopping: watch::channel(()).0,
        }
    }

    /// Discover an offline pack installed in this companion's private state directory.
    pub(crate) fn embedded_for_state(state_dir: PathBuf) -> Self {
        Self::new(WorkerTarget::EmbeddedState(state_dir))
    }

    /// Read the optional Distributed socket address. It can only select a backend sidecar.
    pub(crate) fn distributed_from_environment() -> Result<Option<Self>> {
        env::var_os(SOCKET_ENV)
            .map(|path| {
                let path = PathBuf::from(path);
                ensure!(path.is_absolute(), "invalid QA socket configuration");
                Ok(Self::new(WorkerTarget::Distributed(path)))
            })
            .transpose()
    }

    async fn connect(&self, progress: &(dyn Fn(QaStartupStage) + Sync)) -> Result<WorkerSession> {
        match &self.target {
            WorkerTarget::Distributed(socket) => {
                WorkerSession::open_socket(socket, QaWorkerCommand::OpenWithDialogue).await
            }
            WorkerTarget::EmbeddedState(state_dir) => {
                let state_dir = state_dir.clone();
                let (pointer_loaded, pointer_observed) = oneshot::channel();
                let mut stage = WarmStage::new("pack_verification");
                let verification = tokio::task::spawn_blocking(move || {
                    let pointer = match override_pack().map_err(pack_pointer_error)? {
                        Some(pointer) => pointer,
                        None => load_installed_pointer(&state_dir).map_err(pack_pointer_error)?,
                    };
                    let _ = pointer_loaded.send(());
                    let invalid_pack = || WarmFailure(QaAvailabilityReason::PackInvalid);
                    let pack = verify_pack(&pointer).map_err(|error| {
                        warm_error("pack_verification", error.context(invalid_pack()))
                    })?;
                    sandbox::protect_state(&state_dir, &pack).map_err(|error| {
                        warm_error("sandbox_state_boundary", error.context(invalid_pack()))
                    })?;
                    Ok::<_, anyhow::Error>(pack)
                });
                if pointer_observed.await.is_ok() {
                    progress(QaStartupStage::VerifyingPack);
                }
                let verified = verification.await;
                stage.finish();
                let pack = verified.map_err(|_| {
                    warm_error(
                        "pack_verification",
                        anyhow!("QA pack verification did not finish"),
                    )
                })??;
                progress(QaStartupStage::LoadingModel);
                WorkerSession::child(&pack)
            }
        }
    }

    async fn connect_ready(
        &self,
        stopped: &mut watch::Receiver<()>,
        cancel: &CancellationToken,
        progress: &(dyn Fn(QaStartupStage) + Sync),
    ) -> Result<WorkerSession> {
        let mut session = tokio::select! {
            biased;
            _ = stopped.changed() => return Err(anyhow!("QA worker is stopping")),
            _ = cancel.cancelled() => return Err(anyhow!("QA request was canceled")),
            result = timeout(WARM_LIMIT, self.connect(progress)) => result??,
        };
        let child_pid = session.child_id();
        let mut stage = WarmStage::new("worker_ready_frame");
        let readiness = async {
            if let WorkerTarget::Distributed(socket) = &self.target {
                for fallback in [QaWorkerCommand::OpenWithContext, QaWorkerCommand::Open] {
                    if session.ready_after_context_open().await? {
                        return Ok(());
                    }
                    session = WorkerSession::open_socket(socket, fallback).await?;
                }
            }
            session.ready().await
        };
        let (ready, failure_stage) = tokio::select! {
            biased;
            _ = stopped.changed() => (Err(anyhow!("QA worker is stopping")), None),
            _ = cancel.cancelled() => (Err(anyhow!("QA request was canceled")), None),
            error = guard_resident(child_pid) => (Err(error), Some("worker_memory_limit")),
            result = timeout(WARM_LIMIT, readiness) => match result {
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
            let _ = session.stop().await;
            return Err(error);
        }
        Ok(session)
    }

    async fn ensure_ready(
        &self,
        slot: &mut Option<WorkerSession>,
        stopped: &mut watch::Receiver<()>,
        cancel: &CancellationToken,
        progress: &(dyn Fn(QaStartupStage) + Sync),
    ) -> Result<()> {
        if let Some(session) = slot.as_mut() {
            let child_pid = session.child_id();
            let ping = tokio::select! {
                biased;
                _ = stopped.changed() => return Err(anyhow!("QA worker is stopping")),
                _ = cancel.cancelled() => return Err(anyhow!("QA request was canceled")),
                error = guard_resident(child_pid) => Ok(Err(error)),
                result = timeout(PING_LIMIT, session.ping()) => result,
            };
            if matches!(ping, Ok(Ok(()))) {
                return Ok(());
            }
        }
        if let Some(old) = slot.take() {
            let _ = old.stop().await;
        }
        *slot = Some(self.connect_ready(stopped, cancel, progress).await?);
        Ok(())
    }

    pub(crate) async fn warm_with_progress(
        &self,
        progress: &(dyn Fn(QaStartupStage) + Sync),
    ) -> Result<()> {
        let mut stopped = self.stopping.subscribe();
        let mut slot = self.session.lock().await;
        let cancel = CancellationToken::new();
        self.ensure_ready(&mut slot, &mut stopped, &cancel, progress)
            .await
    }
}

impl QaInference for QaWorkerSupervisor {
    fn warm(&self) -> QaInferenceFuture<'_> {
        Box::pin(async move { self.warm_with_progress(&|_| {}).await })
    }

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
            self.ensure_ready(&mut slot, &mut stopped, &cancel, &|_| {})
                .await?;
            let session = slot
                .as_mut()
                .ok_or_else(|| anyhow!("QA worker unavailable"))?;
            let child_pid = session.child_id();
            let (result, failure_stage) = tokio::select! {
                biased;
                _ = stopped.changed() => (Err(anyhow!("QA worker is stopping")), None),
                _ = cancel.cancelled() => (Err(anyhow!("QA request was canceled")), None),
                error = guard_resident(child_pid) => (Err(error), Some("worker_memory_limit")),
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
                if let Some(session) = slot.take() {
                    let _ = session.stop().await;
                }
            }
            result
        })
    }

    fn shutdown(&self) -> QaInferenceFuture<'_> {
        Box::pin(async move {
            self.stopping.send_modify(|_| {});
            let mut slot = self.session.lock().await;
            if let Some(session) = slot.take() {
                let stopped = session.stop().await;
                if stopped.is_ok() || matches!(self.target, WorkerTarget::EmbeddedState(_)) {
                    return stopped;
                }
            }
            if let WorkerTarget::Distributed(socket) = &self.target {
                // Query the Torch-free listener. It can confirm unload even after the original
                // executor connection was lost, without starting another inference child.
                let session = WorkerSession::socket(socket).await?;
                return session.stop().await;
            }
            Ok(())
        })
    }
}

struct WorkerSession {
    input: Box<dyn AsyncWrite + Send + Unpin>,
    output: Box<dyn AsyncRead + Send + Unpin>,
    child: Option<Child>,
    dialogue_allowed: bool,
    dialogue: bool,
}

impl WorkerSession {
    fn child(pack: &Path) -> Result<Self> {
        let model = pack.join("model");
        let mut command = sandbox::command(pack, &pack.join(WORKER_BINARY))
            .map_err(|error| warm_error("sandbox_setup", error))?;
        command
            .arg("--model-dir")
            .arg(&model)
            .current_dir(&model)
            .env(QA_CONTEXT_USAGE_ENV, "1")
            .env(QA_DIALOGUE_ENV, "1")
            .env("OMP_NUM_THREADS", "1")
            .env("MKL_NUM_THREADS", "1")
            .env("RAYON_NUM_THREADS", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .kill_on_drop(true);
        limit_embedded_memory(&mut command);
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
            input: Box::new(input),
            output: Box::new(output),
            child: Some(child),
            dialogue_allowed: true,
            dialogue: false,
        })
    }

    fn child_id(&self) -> Option<u32> {
        self.child.as_ref().and_then(Child::id)
    }

    async fn open_socket(socket: &Path, command: QaWorkerCommand) -> Result<Self> {
        let mut session = Self::socket(socket)
            .await
            .map_err(|error| warm_error("worker_socket", error))?;
        session.dialogue_allowed = matches!(&command, QaWorkerCommand::OpenWithDialogue);
        session
            .send(&command)
            .await
            .map_err(|error| warm_error("worker_socket_open", error))?;
        Ok(session)
    }

    async fn socket(socket: &Path) -> Result<Self> {
        let parent = socket
            .parent()
            .ok_or_else(|| anyhow!("invalid QA worker socket"))?;
        let directory = tokio::fs::symlink_metadata(parent).await?;
        let metadata = tokio::fs::symlink_metadata(socket).await?;
        ensure!(
            directory.is_dir()
                && directory.mode() & 0o777 == 0o700
                && directory.uid() == uid()
                && metadata.file_type().is_socket()
                && metadata.mode() & 0o777 == 0o600
                && metadata.uid() == uid(),
            "QA worker socket must be private"
        );
        let stream = timeout(PING_LIMIT, UnixStream::connect(socket)).await??;
        let (output, input) = stream.into_split();
        Ok(Self {
            input: Box::new(input),
            output: Box::new(output),
            child: None,
            dialogue_allowed: false,
            dialogue: false,
        })
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

    async fn ready_after_context_open(&mut self) -> Result<bool> {
        let mut header = [0; 4];
        // Older listeners close the socket as soon as they reject the unknown command.
        // Once a peer sends any frame bytes, handle it as an ordinary readiness reply.
        let received = match self.output.read(&mut header).await {
            Ok(0) => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::ConnectionReset => return Ok(false),
            read => read?,
        };
        self.output.read_exact(&mut header[received..]).await?;
        let reply = self.receive_payload(header).await?;
        self.validate_ready(reply)?;
        Ok(true)
    }

    async fn ready(&mut self) -> Result<()> {
        let reply = self.receive().await?;
        self.validate_ready(reply)
    }

    fn validate_ready(&mut self, reply: QaWorkerReply) -> Result<()> {
        match reply {
            QaWorkerReply::Ready {
                protocol,
                model_revision,
            } if protocol == QA_WORKER_PROTOCOL && model_revision == QA_MODEL_REVISION => {
                self.dialogue = false;
                Ok(())
            }
            QaWorkerReply::ReadyWithDialogue {
                protocol,
                model_revision,
            } if self.dialogue_allowed
                && protocol == QA_WORKER_PROTOCOL
                && matches!(
                    model_revision.as_str(),
                    QA_MODEL_REVISION | QA_CHAT_MODEL_REVISION
                ) =>
            {
                self.dialogue = true;
                Ok(())
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
                Err(anyhow!("QA model process panicked during startup"))
            }
            _ => Err(anyhow!(
                "QA worker could not confirm the expected checkpoint"
            )),
        }
    }

    async fn ping(&mut self) -> Result<()> {
        self.send(&QaWorkerCommand::Ping).await?;
        self.ready().await
    }

    async fn ask(
        &mut self,
        mut request: QaWorkerRequest,
        events: mpsc::Sender<QaWorkerEvent>,
    ) -> Result<()> {
        if let Some(dialogue) = request.dialogue.as_ref().filter(|_| !self.dialogue) {
            if dialogue.kind != QaDialogueKind::Operational {
                let event = QaWorkerEvent::Complete {
                    turn_id: request.turn_id,
                    sequence: 1,
                    outcome: QaWorkerOutcome::InsufficientContext,
                    context: Some(QaContextUsage::new(QA_CONTEXT_WINDOW_TOKENS, 0, 0)),
                    reply: None,
                };
                return events
                    .send(event)
                    .await
                    .map_err(|_| anyhow!("QA event receiver closed"));
            }
            request.dialogue = None;
        }
        self.send(&QaWorkerCommand::Ask {
            request: request.clone(),
        })
        .await?;
        let mut validation = QaWorkerSequence::default();
        loop {
            let QaWorkerReply::Event { event } = self.receive().await? else {
                return Err(anyhow!("QA worker stopped before its terminal result"));
            };
            let terminal = validation.accept(&request, &event)?;
            events
                .send(event)
                .await
                .map_err(|_| anyhow!("QA event receiver closed"))?;
            if terminal {
                return Ok(());
            }
        }
    }

    async fn stop(mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            if matches!(
                timeout(UNLOAD_LIMIT, async {
                    self.send(&QaWorkerCommand::Shutdown).await?;
                    child.wait().await?;
                    Ok::<_, anyhow::Error>(())
                })
                .await,
                Ok(Ok(()))
            ) {
                return Ok(());
            }
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

async fn guard_resident(pid: Option<u32>) -> anyhow::Error {
    let Some(pid) = pid else {
        return future::pending().await;
    };
    loop {
        if resident_bytes(pid)
            .await
            .is_some_and(|bytes| bytes > MAX_RESIDENT_BYTES)
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

fn limit_embedded_memory(command: &mut Command) {
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

fn load_installed_pointer(state_dir: &Path) -> Result<PackPointer> {
    let directory = fs::symlink_metadata(state_dir)?;
    ensure!(
        directory.is_dir() && directory.uid() == uid() && directory.mode() & 0o777 == 0o700,
        "QA state directory must be private"
    );
    let path = state_dir.join("qa-pack.json");
    let mut pointer = open_pack_file(&path, MAX_POINTER_BYTES)?;
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

fn verify_pack(pointer: &PackPointer) -> Result<PathBuf> {
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
    ensure!(
        manifest.protocol == QA_WORKER_PROTOCOL
            && matches!(
                manifest.model_revision.as_str(),
                QA_MODEL_REVISION | QA_CHAT_MODEL_REVISION
            )
            && manifest.target == host_target()
            && manifest.files.len() <= MAX_PACK_FILES
            && manifest.files.contains_key(WORKER_BINARY)
            && manifest.files.keys().all(|name| allowed_pack_file(name)),
        "QA pack is incompatible with this companion"
    );
    let legacy_model = manifest.model_revision == QA_MODEL_REVISION;
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
    if !legacy_model
        || manifest
            .files
            .keys()
            .any(|name| name.starts_with("model/generator/"))
    {
        for (name, digest) in QA_GENERATOR_FILES {
            ensure!(
                manifest
                    .files
                    .get(&format!("model/generator/{name}"))
                    .map(String::as_str)
                    == Some(digest),
                "QA pack contains an incomplete or incompatible generator"
            );
        }
    }

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
                (&mut generator_bytes, MAX_GENERATOR_BYTES)
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
    Ok(root)
}

fn allowed_pack_file(name: &str) -> bool {
    matches!(name, WORKER_BINARY | SOCKET_LISTENER_BINARY)
        || QA_MODEL_FILES
            .iter()
            .any(|(file, _)| name.strip_prefix("model/") == Some(*file))
        || QA_GENERATOR_FILES
            .iter()
            .any(|(file, _)| name.strip_prefix("model/generator/") == Some(*file))
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
    use std::os::unix::fs::PermissionsExt;

    use fleetworks_contracts::qa_worker::QaPassage;
    use tokio::net::UnixListener;
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
        let reason = || match load_installed_pointer(state.path()).map_err(pack_pointer_error) {
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

    #[tokio::test]
    async fn opens_a_legacy_listener_after_an_empty_context_handshake() {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let socket = directory.path().join("qa.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();

        let peer = async {
            let (mut unsupported, _) = listener.accept().await.unwrap();
            assert!(matches!(
                read_command(&mut unsupported).await,
                QaWorkerCommand::OpenWithDialogue
            ));
            drop(unsupported);
            let (mut unsupported, _) = listener.accept().await.unwrap();
            assert!(matches!(
                read_command(&mut unsupported).await,
                QaWorkerCommand::OpenWithContext
            ));
            drop(unsupported);

            let (mut legacy, _) = listener.accept().await.unwrap();
            assert!(matches!(
                read_command(&mut legacy).await,
                QaWorkerCommand::Open
            ));
            write_reply(
                &mut legacy,
                QaWorkerReply::Ready {
                    protocol: QA_WORKER_PROTOCOL,
                    model_revision: QA_MODEL_REVISION.into(),
                },
            )
            .await;
            assert!(matches!(
                read_command(&mut legacy).await,
                QaWorkerCommand::Shutdown
            ));
            write_reply(&mut legacy, QaWorkerReply::Stopped).await;
        };
        let supervisor = QaWorkerSupervisor::new(WorkerTarget::Distributed(socket));
        let executor = async {
            supervisor.warm().await.unwrap();
            supervisor.shutdown().await.unwrap();
        };
        timeout(PING_LIMIT, async { tokio::join!(peer, executor) })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn streams_first_finding_before_next_batch_and_confirms_stop() {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let socket = directory.path().join("qa.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        let (release, next_batch) = oneshot::channel();
        let peer = tokio::spawn(async move {
            let (mut peer, _) = listener.accept().await.unwrap();
            assert!(matches!(
                read_command(&mut peer).await,
                QaWorkerCommand::OpenWithDialogue
            ));
            let ready = || QaWorkerReply::Ready {
                protocol: QA_WORKER_PROTOCOL,
                model_revision: QA_MODEL_REVISION.into(),
            };
            write_reply(&mut peer, ready()).await;
            assert!(matches!(
                read_command(&mut peer).await,
                QaWorkerCommand::Ping
            ));
            write_reply(&mut peer, ready()).await;
            let QaWorkerCommand::Ask { request } = read_command(&mut peer).await else {
                panic!("missing prediction request");
            };
            let turn_id = request.turn_id;
            write_reply(
                &mut peer,
                QaWorkerReply::Event {
                    event: QaWorkerEvent::Finding {
                        turn_id: turn_id.clone(),
                        sequence: 1,
                        source_id: "check-a".into(),
                        start: 16,
                        end: 21,
                        text: "ready".into(),
                    },
                },
            )
            .await;
            next_batch.await.unwrap();
            write_reply(
                &mut peer,
                QaWorkerReply::Event {
                    event: QaWorkerEvent::Finding {
                        turn_id: turn_id.clone(),
                        sequence: 2,
                        source_id: "check-b".into(),
                        start: 11,
                        end: 18,
                        text: "healthy".into(),
                    },
                },
            )
            .await;
            write_reply(
                &mut peer,
                QaWorkerReply::Event {
                    event: QaWorkerEvent::Complete {
                        turn_id,
                        sequence: 3,
                        outcome: QaWorkerOutcome::Answered,
                        context: Some(QaContextUsage::new(QA_CONTEXT_WINDOW_TOKENS, 58, 2)),
                        reply: None,
                    },
                },
            )
            .await;
            assert!(matches!(
                read_command(&mut peer).await,
                QaWorkerCommand::Shutdown
            ));
            write_reply(&mut peer, QaWorkerReply::Stopped).await;
        });
        let supervisor = QaWorkerSupervisor::new(WorkerTarget::Distributed(socket));
        supervisor.warm().await.unwrap();
        let request = QaWorkerRequest::new(
            "turn-a".into(),
            "snapshot-a".into(),
            "How are the workstation dependencies?".into(),
            vec![
                QaPassage::new("check-a".into(), "Actor search is ready.".into()),
                QaPassage::new("check-b".into(), "Storage is healthy.".into()),
            ],
        );
        let (events, mut stream) = mpsc::channel(1);
        let ask = supervisor.ask(request, events, CancellationToken::new());
        let consume = async {
            assert!(matches!(
                stream.recv().await,
                Some(QaWorkerEvent::Finding { sequence: 1, .. })
            ));
            release.send(()).unwrap();
            assert!(matches!(
                stream.recv().await,
                Some(QaWorkerEvent::Finding { sequence: 2, .. })
            ));
            assert!(matches!(
                stream.recv().await,
                Some(QaWorkerEvent::Complete {
                    sequence: 3,
                    context: Some(context),
                    ..
                }) if context == QaContextUsage::new(QA_CONTEXT_WINDOW_TOKENS, 58, 2)
            ));
        };
        let (result, ()) = timeout(PING_LIMIT, async { tokio::join!(ask, consume) })
            .await
            .unwrap();
        result.unwrap();
        supervisor.shutdown().await.unwrap();
        peer.await.unwrap();
    }
}
