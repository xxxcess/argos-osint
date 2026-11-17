//! Supervised local embedded companion backend lifecycle and strict executable discovery.
//!
//! Communicates exclusively over framed stdio IPC via `LocalIpcClient` and `Client::from_local_ipc`.
//! Never searches bare `PATH` and never falls back to HTTP, TCP, or Unix sockets.

use std::fmt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use fleetworks_contracts::capabilities::Capabilities;
use fleetworks_contracts::capabilities::CapabilityError;
use fleetworks_contracts::ipc::IpcError;
use fleetworks_contracts::ipc::IpcEvent;
use fleetworks_contracts::ipc::LocalIpcClient;
use fleetworks_contracts::ipc::ShutdownAck;
use fleetworks_contracts::Client;
use fleetworks_contracts::ClientError;
use fleetworks_contracts::Ready;
use tokio::process::Command;
use tokio::sync::broadcast;

use crate::connection::RequestTask;

/// Companion binary filename resolved adjacent to the desktop executable.
pub const COMPANION_BINARY_NAME: &str = "argos-api-host";
/// Optional environment override pointing directly to an explicit companion executable.
pub const LOCAL_BACKEND_BIN_ENV: &str = "FLEETWORKS_LOCAL_BACKEND_BIN";
const MAX_STDERR_EXCERPT_BYTES: usize = 4096;

/// Explicit discovery state of the companion `argos-api-host` executable.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum BinaryDiscoveryStatus {
    /// Companion binary discovery has not been evaluated yet.
    #[default]
    Unknown,
    /// Validated absolute executable path ready for local supervision.
    Ready(PathBuf),
    /// Companion executable is missing or failed validation.
    Missing(String),
}

/// Recent unsolicited job progress emitted by the local companion backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalJobProgress {
    pub job_id: String,
    pub phase: String,
    pub summary: String,
}

/// Optional capability assessment derived strictly from `Capabilities` advertised by the backend.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LocalCapabilityAssessment {
    pub nlp_available: bool,
    pub classifier_available: bool,
    pub actor_search_available: bool,
    pub warnings: Vec<String>,
}

/// Inspect advertised `Capabilities` without synthesizing unsupported features.
#[must_use]
pub fn assess_local_capabilities(capabilities: &Capabilities) -> LocalCapabilityAssessment {
    let nlp_available = capabilities.features.iter().any(|feature| feature == "nlp")
        || capabilities
            .categories
            .iter()
            .any(|category| category == "nlp");
    let classifier_available = capabilities
        .features
        .iter()
        .any(|feature| feature == "classifier")
        || capabilities
            .categories
            .iter()
            .any(|category| category == "classifier");
    let actor_search_available = capabilities.require_actor_search().is_ok();

    let mut warnings = Vec::new();
    if !nlp_available {
        warnings.push("The local backend is not advertising NLP support.".into());
    }
    if !classifier_available {
        warnings.push(
            "The local backend is not advertising Natural classifier support. Training and activation are unavailable.".into(),
        );
    }
    if !actor_search_available {
        warnings.push(
            "Actor search capability (`tantivy`) is absent in this local companion binary.".into(),
        );
    }
    LocalCapabilityAssessment {
        nlp_available,
        classifier_available,
        actor_search_available,
        warnings,
    }
}

/// Errors produced while locating, validating, spawning, or communicating with the local backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocalBackendError {
    /// Companion binary was not found at any trusted location or failed validation.
    BinaryUnavailable(String),
    /// Local backend state directory failed private ownership or symlink checks.
    InvalidStateDir(String),
    /// Companion handshake omitted required capabilities or advertised an incompatible contract.
    IncompatibleCapabilities(String),
    /// Local IPC transport failed or pipe closed without HTTP fallback.
    Transport(String),
}

impl fmt::Display for LocalBackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BinaryUnavailable(message)
            | Self::InvalidStateDir(message)
            | Self::IncompatibleCapabilities(message)
            | Self::Transport(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for LocalBackendError {}

impl From<CapabilityError> for LocalBackendError {
    fn from(error: CapabilityError) -> Self {
        Self::IncompatibleCapabilities(error.to_string())
    }
}

impl From<IpcError> for LocalBackendError {
    fn from(error: IpcError) -> Self {
        Self::Transport(error.to_string())
    }
}

impl From<ClientError> for LocalBackendError {
    fn from(error: ClientError) -> Self {
        Self::Transport(error.to_string())
    }
}

/// Supervised local companion backend process attached over framed stdio IPC.
pub struct LocalBackendProcess {
    binary_path: Option<PathBuf>,
    state_dir: PathBuf,
    ipc: Arc<LocalIpcClient>,
    client: Client,
    capabilities: Capabilities,
    ready: Ready,
}

impl fmt::Debug for LocalBackendProcess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalBackendProcess")
            .field("binary_path", &self.binary_path)
            .field("state_dir", &self.state_dir)
            .field("closed", &self.is_closed())
            .finish()
    }
}

impl LocalBackendProcess {
    /// Spawn `argos-api-host --role local-ipc --state-dir <dir>` with cleaned environment and stdio IPC.
    pub async fn spawn(binary: &Path, state_dir: &Path) -> Result<Self, LocalBackendError> {
        let validated_bin = validate_companion_executable(binary)?;
        let validated_dir = validate_private_state_dir(state_dir)?;
        let mut command = build_companion_command(&validated_bin, &validated_dir)?;
        let ipc = Arc::new(
            LocalIpcClient::spawn_command(
                &mut command,
                "fleetworks-desktop",
                env!("CARGO_PKG_VERSION"),
            )
            .await
            .map_err(|error| {
                LocalBackendError::Transport(format!(
                    "failed to start local companion backend over stdio IPC: {error}"
                ))
            })?,
        );
        Self::finish_attach(Some(validated_bin), validated_dir, ipc).await
    }

    /// Wrap an already-handshaken `LocalIpcClient` without spawning an external OS process.
    pub async fn from_ipc(
        state_dir: PathBuf,
        ipc: Arc<LocalIpcClient>,
    ) -> Result<Self, LocalBackendError> {
        Self::finish_attach(None, state_dir, ipc).await
    }

    async fn finish_attach(
        binary_path: Option<PathBuf>,
        state_dir: PathBuf,
        ipc: Arc<LocalIpcClient>,
    ) -> Result<Self, LocalBackendError> {
        let capabilities = match ipc.capabilities() {
            Some(capabilities) => capabilities.clone(),
            None => {
                let _ = ipc.shutdown("missing_capabilities").await;
                return Err(LocalBackendError::IncompatibleCapabilities(
                    "local companion backend did not advertise capabilities during IPC handshake"
                        .into(),
                ));
            }
        };
        if let Err(error) = capabilities.require_native_contract() {
            let _ = ipc.shutdown("incompatible_capabilities").await;
            return Err(LocalBackendError::IncompatibleCapabilities(
                error.to_string(),
            ));
        }
        let client = Client::from_local_ipc(Arc::clone(&ipc));
        let ready = match ipc.ready_snapshot() {
            Some(ready) => {
                if ready.api_version != fleetworks_contracts::API_VERSION {
                    let _ = ipc.shutdown("unsupported_api_version").await;
                    return Err(LocalBackendError::IncompatibleCapabilities(format!(
                        "unsupported local backend API version {}",
                        ready.api_version
                    )));
                }
                ready.clone()
            }
            None => client.ready().await.map_err(|error| {
                let stderr = sanitize_stderr_excerpt(&ipc.captured_stderr());
                if stderr.is_empty() {
                    LocalBackendError::Transport(format!(
                        "local backend readiness handshake failed: {error}"
                    ))
                } else {
                    LocalBackendError::Transport(format!(
                        "local backend readiness handshake failed: {error} ({stderr})"
                    ))
                }
            })?,
        };
        Ok(Self {
            binary_path,
            state_dir,
            ipc,
            client,
            capabilities,
            ready,
        })
    }

    /// Validated executable path when launched via subprocess supervision.
    #[must_use]
    pub fn binary_path(&self) -> Option<&Path> {
        self.binary_path.as_deref()
    }

    /// Private `0o700` state directory passed to `--state-dir`.
    #[must_use]
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// Underlying framed IPC client.
    #[must_use]
    pub fn ipc(&self) -> &Arc<LocalIpcClient> {
        &self.ipc
    }

    /// Transport-neutral `Client` bound to this local IPC pipe (no HTTP fallback).
    #[must_use]
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Capabilities advertised in the IPC `HelloAck`.
    #[must_use]
    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    /// Initial readiness state observed from the local backend.
    #[must_use]
    pub fn ready(&self) -> &Ready {
        &self.ready
    }

    /// Whether the underlying local IPC pipe has closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.ipc.is_closed()
    }

    /// Subscribe to unsolicited `IpcEvent` notifications from the local backend.
    #[must_use]
    pub fn subscribe_events(&self) -> broadcast::Receiver<IpcEvent> {
        self.ipc.subscribe_events()
    }

    /// Sanitized bounded tail of captured companion stderr bytes (no control sequences).
    #[must_use]
    pub fn captured_stderr_lossy(&self) -> String {
        sanitize_stderr_excerpt(&self.ipc.captured_stderr())
    }

    /// Request graceful `Shutdown` and await `ShutdownAck` from the local backend.
    pub async fn shutdown(&self, reason: &str) -> Result<ShutdownAck, IpcError> {
        self.ipc.shutdown(reason).await
    }
}

/// UI-owned local backend supervision and event diagnostics state.
#[derive(Default)]
pub struct LocalBackendState {
    pub cli_binary_override: Option<PathBuf>,
    pub state_dir: Option<PathBuf>,
    pub binary_status: BinaryDiscoveryStatus,
    pub session: Option<Arc<LocalBackendProcess>>,
    pub(crate) event_task: Option<RequestTask>,
    pub last_job_progress: Option<LocalJobProgress>,
    pub pressure_paused: Option<String>,
    pub last_shutdown_ack: Option<ShutdownAck>,
    pub shutdown_message: Option<String>,
    pub pipe_closed_error: Option<String>,
    pub stderr_excerpt: Option<String>,
    pub stopping: bool,
}

impl LocalBackendState {
    /// Whether a local companion process session is currently attached and open.
    #[must_use]
    pub fn active(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|process| !process.is_closed())
    }

    /// Refresh companion binary discovery status from `--local-backend-bin`, env, or exe sibling.
    pub fn refresh_binary_status(&mut self) {
        self.binary_status = match resolve_companion_binary(self.cli_binary_override.as_deref()) {
            Ok(path) => BinaryDiscoveryStatus::Ready(path),
            Err(error) => BinaryDiscoveryStatus::Missing(error.to_string()),
        };
    }

    /// Reset transient status/warnings before starting a fresh local connection.
    pub fn clear_runtime_notices(&mut self) {
        self.last_job_progress = None;
        self.pressure_paused = None;
        self.pipe_closed_error = None;
        self.stderr_excerpt = None;
        self.shutdown_message = None;
    }
}

/// Locate the companion `argos-api-host` binary without searching bare `PATH`.
pub fn resolve_companion_binary(explicit_cli: Option<&Path>) -> Result<PathBuf, LocalBackendError> {
    if let Some(path) = explicit_cli {
        return validate_companion_executable(path);
    }
    if let Some(raw) = std::env::var_os(LOCAL_BACKEND_BIN_ENV) {
        if !raw.is_empty() {
            return validate_companion_executable(Path::new(&raw));
        }
    }
    let current_exe = std::env::current_exe().map_err(|error| {
        LocalBackendError::BinaryUnavailable(format!(
            "cannot resolve current desktop executable location: {error}"
        ))
    })?;
    resolve_companion_near_exe(&current_exe)
}

pub(crate) fn resolve_companion_near_exe(current_exe: &Path) -> Result<PathBuf, LocalBackendError> {
    let exe_dir = current_exe.parent().ok_or_else(|| {
        LocalBackendError::BinaryUnavailable(
            "current executable has no parent directory for sibling companion discovery".into(),
        )
    })?;
    let mut candidates = vec![exe_dir.join(COMPANION_BINARY_NAME)];
    // Support `cargo test` executables located under `target/<profile>/deps/<bin>`.
    if exe_dir.file_name().is_some_and(|name| name == "deps") {
        if let Some(profile_dir) = exe_dir.parent() {
            candidates.push(profile_dir.join(COMPANION_BINARY_NAME));
        }
    }
    let mut failures = Vec::new();
    for candidate in &candidates {
        match validate_companion_executable(candidate) {
            Ok(valid) => return Ok(valid),
            Err(error) => failures.push(format!("{}: {error}", candidate.display())),
        }
    }
    Err(LocalBackendError::BinaryUnavailable(format!(
        "companion executable `{COMPANION_BINARY_NAME}` was not found or failed verification ({}); pass `--local-backend-bin <PATH>` or set `{LOCAL_BACKEND_BIN_ENV}`",
        failures.join("; ")
    )))
}

/// Verify an explicit companion binary path is canonical, regular, owned, non-symlink, and executable.
pub fn validate_companion_executable(path: &Path) -> Result<PathBuf, LocalBackendError> {
    let text = path.to_str().ok_or_else(|| {
        LocalBackendError::BinaryUnavailable("companion binary path must be valid UTF-8".into())
    })?;
    if text.is_empty() || text.chars().any(char::is_control) || !path.is_absolute() {
        return Err(LocalBackendError::BinaryUnavailable(
            "companion binary path must be a non-empty absolute path".into(),
        ));
    }
    let symlink_meta = std::fs::symlink_metadata(path).map_err(|error| {
        LocalBackendError::BinaryUnavailable(format!(
            "companion binary `{}` is unavailable: {error}",
            path.display()
        ))
    })?;
    if symlink_meta.file_type().is_symlink() {
        return Err(LocalBackendError::BinaryUnavailable(format!(
            "companion binary `{}` must not be a symlink",
            path.display()
        )));
    }
    verify_executable_metadata(path, &symlink_meta)?;
    let canonical = std::fs::canonicalize(path).map_err(|error| {
        LocalBackendError::BinaryUnavailable(format!(
            "cannot canonicalize companion binary `{}`: {error}",
            path.display()
        ))
    })?;
    let canonical_meta = std::fs::symlink_metadata(&canonical).map_err(|error| {
        LocalBackendError::BinaryUnavailable(format!(
            "cannot stat canonical companion binary `{}`: {error}",
            canonical.display()
        ))
    })?;
    if canonical_meta.file_type().is_symlink() {
        return Err(LocalBackendError::BinaryUnavailable(format!(
            "canonical companion binary `{}` cannot be a symlink",
            canonical.display()
        )));
    }
    verify_executable_metadata(&canonical, &canonical_meta)?;
    Ok(canonical)
}

fn verify_executable_metadata(
    path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<(), LocalBackendError> {
    if !metadata.file_type().is_file() {
        return Err(LocalBackendError::BinaryUnavailable(format!(
            "companion binary `{}` must be a regular file",
            path.display()
        )));
    }
    let mode = metadata.permissions().mode();
    if mode & 0o111 == 0 {
        return Err(LocalBackendError::BinaryUnavailable(format!(
            "companion binary `{}` is not executable",
            path.display()
        )));
    }
    if mode & 0o022 != 0 {
        return Err(LocalBackendError::BinaryUnavailable(format!(
            "companion binary `{}` must not be group- or world-writable",
            path.display()
        )));
    }
    let current_euid = unsafe { libc::geteuid() };
    let owner = metadata.uid();
    if owner != current_euid && owner != 0 {
        return Err(LocalBackendError::BinaryUnavailable(format!(
            "companion binary `{}` is not owned by the current user or root",
            path.display()
        )));
    }
    Ok(())
}

/// Verify that the local backend state directory exists as a private `0o700` real directory.
pub fn validate_private_state_dir(path: &Path) -> Result<PathBuf, LocalBackendError> {
    let text = path.to_str().ok_or_else(|| {
        LocalBackendError::InvalidStateDir(
            "local backend state directory must be valid UTF-8".into(),
        )
    })?;
    if !path.is_absolute() || text.chars().any(char::is_control) {
        return Err(LocalBackendError::InvalidStateDir(
            "local backend state directory must be an absolute path".into(),
        ));
    }
    let meta = std::fs::symlink_metadata(path).map_err(|error| {
        LocalBackendError::InvalidStateDir(format!(
            "local backend state directory `{}` is missing or inaccessible: {error}",
            path.display()
        ))
    })?;
    if meta.file_type().is_symlink() || !meta.file_type().is_dir() {
        return Err(LocalBackendError::InvalidStateDir(format!(
            "local backend state directory `{}` must be a real directory without symlinks",
            path.display()
        )));
    }
    let mode = meta.permissions().mode();
    if mode & 0o077 != 0 {
        return Err(LocalBackendError::InvalidStateDir(format!(
            "local backend state directory `{}` must use private 0700 permissions",
            path.display()
        )));
    }
    let current_euid = unsafe { libc::geteuid() };
    if meta.uid() != current_euid {
        return Err(LocalBackendError::InvalidStateDir(format!(
            "local backend state directory `{}` owner does not match the current user",
            path.display()
        )));
    }
    std::fs::canonicalize(path).map_err(|error| {
        LocalBackendError::InvalidStateDir(format!(
            "cannot canonicalize local backend state directory `{}`: {error}",
            path.display()
        ))
    })
}

/// Build the cleaned-environment command for spawning `argos-api-host --role local-ipc`.
pub fn build_companion_command(
    binary: &Path,
    state_dir: &Path,
) -> Result<Command, LocalBackendError> {
    let mut command = Command::new(binary);
    command.env_clear();
    for key in [
        "HOME",
        "TMPDIR",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .arg("--role")
        .arg("local-ipc")
        .arg("--state-dir")
        .arg(state_dir);
    Ok(command)
}

/// Removes terminal escape sequences so only readable text is left: CSI (`ESC [ ... final`, the
/// colors and cursor moves a logger writes), OSC and the other string sequences (`ESC ] ...` up
/// to BEL or `ESC \`), two-byte and intermediate escapes, and a stray `ESC`. An unterminated
/// string sequence ends at the next newline, so one bad line never hides the diagnostics after it.
#[must_use]
pub fn strip_terminal_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }
        match chars.peek().copied() {
            Some('[') => {
                chars.next();
                // Parameters and intermediates, then one final byte.
                while let Some(&c) = chars.peek() {
                    if ('\u{20}'..='\u{3f}').contains(&c) {
                        chars.next();
                    } else {
                        if ('\u{40}'..='\u{7e}').contains(&c) {
                            chars.next();
                        }
                        break;
                    }
                }
            }
            Some(']' | 'P' | 'X' | '^' | '_') => {
                chars.next();
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' {
                        if chars.peek() == Some(&'\\') {
                            chars.next();
                        }
                        break;
                    }
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            Some(c) if ('\u{20}'..='\u{2f}').contains(&c) => {
                // Intermediates (such as the charset selector `ESC ( B`), then a final byte.
                while chars
                    .peek()
                    .is_some_and(|c| ('\u{20}'..='\u{2f}').contains(c))
                {
                    chars.next();
                }
                if chars
                    .peek()
                    .is_some_and(|c| ('\u{30}'..='\u{7e}').contains(c))
                {
                    chars.next();
                }
            }
            Some(c) if ('\u{30}'..='\u{7e}').contains(&c) => {
                chars.next();
            }
            _ => {}
        }
    }
    out
}

/// Bounded lossy stderr sanitization for diagnostics display without control codes. Terminal
/// escape sequences are removed whole before the tail is taken, so a cut never leaves a
/// fragment such as `[0m`.
#[must_use]
pub fn sanitize_stderr_excerpt(bytes: &[u8]) -> String {
    let text = strip_terminal_escapes(&String::from_utf8_lossy(bytes));
    let mut start = text.len().saturating_sub(MAX_STDERR_EXCERPT_BYTES);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..]
        .chars()
        .filter(|ch| *ch == '\n' || !ch.is_control())
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests;
