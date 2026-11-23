//! Native pairing progression. Secret values live only in Keychain or effect workers.

use crate::connection::RequestTask;
use fleetworks_contracts::pairing::PairingChallenge;
use std::collections::VecDeque;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum PairingStage {
    #[default]
    Idle,
    Working(String),
    Waiting(PairingChallenge),
    Failed(PairingFailure),
    Connected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PairingFailure {
    InvalidOrigin,
    KeychainMissing,
    KeychainDenied,
    KeychainUnavailable,
    SessionEnded,
    CatalogUnavailable,
    Network,
    GatewayUnavailable,
    Authentication,
    Unsupported,
    Denied,
    Expired,
}
impl PairingFailure {
    pub fn message(self) -> &'static str {
        match self {
            Self::InvalidOrigin => "Enter a gateway HTTPS URL or a local loopback HTTP URL.",
            Self::KeychainMissing => {
                if cfg!(target_os = "macos") {
                    "Gateway access is missing from Keychain. Pair this workstation again."
                } else {
                    "Gateway access is missing from the system keyring. Pair this workstation again."
                }
            }
            Self::KeychainDenied => {
                if cfg!(target_os = "macos") {
                    "Unlock Keychain or allow Fleetworks access, then try again."
                } else {
                    "Unlock your system keyring or allow Fleetworks access, then retry or use session-only mode."
                }
            }
            Self::KeychainUnavailable => {
                if cfg!(target_os = "macos") {
                    "Keychain is unavailable. Your access has not been saved; try again or use this session only."
                } else {
                    "System keyring (Secret Service) is unavailable. Your access has not been saved; retry or use session-only mode."
                }
            }
            Self::SessionEnded => "Session ended. Pair again.",
            Self::CatalogUnavailable => "Protected pairing settings are unavailable. Restore access to this installation, then retry.",
            Self::Network => "Could not reach the gateway. Check its URL and connection, then retry.",
            Self::GatewayUnavailable => "The gateway could not complete setup. Retry; if this continues, ask your gateway operator to check service configuration and health.",
            Self::Authentication => "This workstation's gateway access was rejected. Pair again after approval.",
            Self::Unsupported => "This gateway does not support pairing yet. Update the gateway before continuing.",
            Self::Denied => "Pairing was declined. Ask your gateway operator before starting again.",
            Self::Expired => "Pairing expired. Start again to request a new approval.",
        }
    }
}

#[derive(Default)]
pub struct Pairing {
    pub stage: PairingStage,
    /// Explicit per-session storage choice (`SystemKeyring` default, or opt-in `SessionOnly`).
    pub storage_mode: fleetworks_workstation::native::CredentialStorage,
    /// Enables pairing handlers in synthetic Runtime fixtures, without effects.
    pub fixture_enabled: bool,
    pub(crate) request: Option<RequestTask>,
    pub(crate) startup_loaded: bool,
    pub(crate) paused: bool,
    saved_startup_origin: Option<(String, bool)>,
    local_review_announced: VecDeque<String>,
}
impl Pairing {
    fn record_local_review_announcement(&mut self, pairing_id: &str) -> bool {
        if self
            .local_review_announced
            .iter()
            .any(|id| id == pairing_id)
        {
            return false;
        }
        if self.local_review_announced.len() == 32 {
            self.local_review_announced.pop_front();
        }
        self.local_review_announced.push_back(pairing_id.to_owned());
        true
    }

    pub fn busy(&self) -> bool {
        matches!(
            self.stage,
            PairingStage::Working(_) | PairingStage::Waiting(_)
        ) && !self.paused
    }
    pub(crate) fn cancel(&mut self) {
        if self.request.is_some() && self.stage == PairingStage::Idle {
            self.startup_loaded = false;
        }
        self.request = None;
        self.paused = false;
        self.stage = PairingStage::Idle;
    }
}

use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use url::Host;

use fleetworks_contracts::pairing::PairingPollRequest;
use fleetworks_contracts::pairing::PairingStartRequest;
use fleetworks_contracts::pairing::PairingStatus;
use fleetworks_contracts::Client;
use fleetworks_contracts::ClientError;
use fleetworks_contracts::PairingClient;
use fleetworks_workstation::native::keychain::KeychainError;
use fleetworks_workstation::native::keychain::KeychainStore;
use fleetworks_workstation::native::keychain::SecretToken;
use fleetworks_workstation::native::CredentialStorage;
use fleetworks_workstation::native::NativeContainer;
use fleetworks_workstation::native::PairedGateway;
use fleetworks_workstation::native::PairingError;
use fleetworks_workstation::native::PendingPairing;

use crate::app::Intent;
use crate::app::State;
use crate::connection::Connected;
use crate::connection::ConnectionStatus;
use crate::connection::GatewayEffects;

impl From<PairingError> for PairingFailure {
    fn from(error: PairingError) -> Self {
        match error {
            PairingError::Keychain(KeychainError::Missing) => Self::KeychainMissing,
            PairingError::Keychain(KeychainError::LockedDenied) => Self::KeychainDenied,
            PairingError::Keychain(_) => Self::KeychainUnavailable,
            PairingError::InvalidInput => Self::InvalidOrigin,
            _ => Self::CatalogUnavailable,
        }
    }
}
fn transport_failure(error: ClientError) -> PairingFailure {
    match error {
        ClientError::InvalidOrigin => PairingFailure::InvalidOrigin,
        ClientError::Status(401 | 403) => PairingFailure::Authentication,
        ClientError::Status(410) => PairingFailure::Expired,
        ClientError::Status(404 | 405 | 426) | ClientError::UnsupportedVersion(_) => {
            PairingFailure::Unsupported
        }
        ClientError::Transport | ClientError::Timeout => PairingFailure::Network,
        _ => PairingFailure::GatewayUnavailable,
    }
}

/// The review socket is an opt-in local handoff. Only its private same-user
/// listener receives the approval code; polling secrets and tokens stay out.
fn local_review_destination(origin: &str, socket: &Path) -> Option<(PathBuf, String)> {
    if origin.len() > 512 || !socket.is_absolute() || socket.as_os_str().as_bytes().len() > 103 {
        return None;
    }
    let origin = Client::canonical_origin(origin).ok()?;
    if origin.len() > 512 {
        return None;
    }
    let url = url::Url::parse(&origin).ok()?;
    if url.scheme() != "http" {
        return None;
    }
    let loopback = match url.host()? {
        Host::Domain(name) => name.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(address) => address.is_loopback(),
        Host::Ipv6(address) => address.is_loopback(),
    };
    loopback.then(|| (socket.to_path_buf(), origin))
}

pub(crate) fn local_review_handoff_configured(origin: &str) -> bool {
    std::env::var_os("FLEETWORKS_LOCAL_REVIEW_APPROVAL_SOCKET")
        .is_some_and(|socket| local_review_destination(origin, Path::new(&socket)).is_some())
}

async fn send_local_review_handoff(
    socket: PathBuf,
    origin: String,
    pairing_id: String,
    approval_code: String,
    expires_at_ms: u64,
) {
    let Ok(mut line) = serde_json::to_vec(&serde_json::json!({
        "version": 1,
        "origin": origin,
        "pairing_id": pairing_id,
        "approval_code": approval_code,
        "expires_at_ms": expires_at_ms,
    })) else {
        return;
    };
    if line.len() > 768 {
        return;
    }
    line.push(b'\n');
    let _ = tokio::time::timeout(Duration::from_millis(250), async {
        let metadata = tokio::fs::symlink_metadata(&socket).await?;
        if !metadata.file_type().is_socket()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o600
        {
            return Ok(());
        }
        let mut stream = UnixStream::connect(socket).await?;
        stream.write_all(&line).await?;
        stream.shutdown().await
    })
    .await;
}

impl State {
    pub(crate) fn uses_native_pairing(&self) -> bool {
        !self.connection_profiles.operator_token_override
            && (!self.fixture || self.pairing.fixture_enabled)
    }

    /// Explicitly abandon a pending approval and request a new one. Active access
    /// is kept until its replacement has authenticated and reached Keychain.
    pub fn restart_pairing(&mut self) {
        self.connect_pairing(true);
    }

    /// Explicitly opt into RAM-only session storage without saving secrets to disk.
    /// Any existing durable pairing remains untouched until this pairing authenticates.
    pub fn use_session_only_pairing(&mut self) {
        self.connect_pairing_with_storage(CredentialStorage::SessionOnly, true);
    }

    pub(crate) fn connect_pairing(&mut self, restart: bool) {
        self.connect_pairing_with_storage(CredentialStorage::SystemKeyring, restart);
    }

    fn connect_pairing_with_storage(&mut self, storage_mode: CredentialStorage, restart: bool) {
        self.start_pairing_request(storage_mode, restart, false);
    }

    fn resume_paused_pairing(&mut self) {
        let storage_mode = self.pairing.storage_mode;
        self.start_pairing_request(storage_mode, false, true);
    }

    fn start_pairing_request(
        &mut self,
        storage_mode: CredentialStorage,
        restart: bool,
        preserve_session_secrets: bool,
    ) {
        self.connection_profiles.initial_selection_applied = true;
        if preserve_session_secrets {
            self.disconnect_keeping_session_secrets();
        } else {
            self.disconnect();
        }
        self.pairing.storage_mode = storage_mode;
        self.connection.status = ConnectionStatus::Connecting;
        self.pairing.stage =
            PairingStage::Working(if storage_mode == CredentialStorage::SessionOnly {
                "Starting session-only gateway pairing…".into()
            } else {
                "Checking saved gateway access…".into()
            });
        self.record(if storage_mode == CredentialStorage::SessionOnly {
            Intent::PairingSessionOnly
        } else if restart {
            Intent::PairingRestart
        } else {
            Intent::PairingConnect
        });
        if self.fixture {
            return;
        }
        let (Some(services), Some(container)) = (&self.services, &self.workspaces.container) else {
            self.pairing.stage = PairingStage::Failed(PairingFailure::CatalogUnavailable);
            self.connection.status = ConnectionStatus::Disconnected;
            return;
        };
        self.pairing.request = services.pairing_connect(
            container.clone(),
            self.connection.origin.text.clone(),
            self.connection.epoch,
            storage_mode,
            restart,
        );
    }

    pub(crate) fn pairing_progress(&mut self, epoch: u64, origin: &str, stage: PairingStage) {
        if self.connection.epoch == epoch {
            if !self.fixture {
                if let PairingStage::Waiting(challenge) = &stage {
                    let destination = std::env::var_os("FLEETWORKS_LOCAL_REVIEW_APPROVAL_SOCKET")
                        .and_then(|socket| local_review_destination(origin, Path::new(&socket)));
                    if let (Some(services), Some((socket, origin))) = (&self.services, destination)
                    {
                        if self
                            .pairing
                            .record_local_review_announcement(&challenge.pairing_id)
                        {
                            services.runtime.spawn(send_local_review_handoff(
                                socket,
                                origin,
                                challenge.pairing_id.clone(),
                                challenge.approval_code.clone(),
                                challenge.expires_at_ms,
                            ));
                        }
                    }
                }
            }
            self.pairing.stage = stage;
        }
    }

    pub(crate) fn complete_pairing_connection(
        &mut self,
        epoch: u64,
        result: Result<Connected, PairingFailure>,
    ) {
        if self.connection.epoch != epoch {
            return;
        }
        self.pairing.request = None;
        match result {
            Ok(connected) => {
                self.complete_connection(epoch, Ok(connected));
                self.pairing.stage = PairingStage::Connected;
            }
            Err(error) => {
                if matches!(
                    error,
                    PairingFailure::Denied
                        | PairingFailure::Expired
                        | PairingFailure::SessionEnded
                        | PairingFailure::Authentication
                ) {
                    self.clear_session_secrets();
                }
                self.connection.status = ConnectionStatus::Failed(error.message().into());
                self.pairing.stage = PairingStage::Failed(error);
            }
        }
    }

    /// Preload only the saved pairing origin from local settings. Startup never
    /// dispatches a connection or pairing request before the user explicitly clicks Connect.
    pub(crate) fn initialize_pairing_connection(&mut self) {
        if self.fixture || self.pairing.startup_loaded || !self.uses_native_pairing() {
            return;
        }
        let (Some(services), Some(container)) = (&self.services, &self.workspaces.container) else {
            return;
        };
        self.pairing.startup_loaded = true;
        self.pairing.request = services.restore_pairing(
            container.clone(),
            self.connection.epoch,
            self.connection_profiles
                .launch_override
                .then(|| self.connection.origin.text.clone()),
        );
    }

    pub(crate) fn prefill_startup_pairing_origin(&mut self) {
        if self.connection.origin.text != fleetworks_contracts::DEFAULT_API_ORIGIN {
            return;
        }
        if let Some((origin, session_ended)) = &self.pairing.saved_startup_origin {
            self.connection.origin = rune::TextEdit::new(origin);
            if *session_ended {
                self.pairing.stage = PairingStage::Failed(PairingFailure::SessionEnded);
            }
        } else {
            self.initialize_pairing_connection();
        }
    }

    fn restored_pairing(&mut self, epoch: u64, result: Result<RestoreOutcome, PairingFailure>) {
        if self.connection.epoch != epoch {
            return;
        }
        self.pairing.request = None;
        let (origin, session_ended) = match result {
            Ok(RestoreOutcome::DurableOrPending(origin)) => (origin, false),
            Ok(RestoreOutcome::SessionEnded(origin)) => (origin, true),
            Ok(RestoreOutcome::None) => return,
            Err(error) => {
                self.pairing.stage = PairingStage::Failed(error);
                return;
            }
        };
        self.pairing.saved_startup_origin = Some((origin.clone(), session_ended));
        if self.backend_kind == fleetworks_workstation::native::BackendKind::Remote
            && (self.connection.origin.text == fleetworks_contracts::DEFAULT_API_ORIGIN
                || Client::canonical_origin(&self.connection.origin.text).as_ref() == Ok(&origin))
        {
            self.connection.origin = rune::TextEdit::new(origin);
            if session_ended {
                self.pairing.stage = PairingStage::Failed(PairingFailure::SessionEnded);
            }
        }
    }

    pub(crate) fn pairing_focus(&mut self, focused: bool) {
        if !focused && self.pairing.busy() {
            self.pairing.request = None;
            self.pairing.paused = true;
            self.connection.epoch = self.connection.epoch.wrapping_add(1);
            self.connection.status = ConnectionStatus::Disconnected;
        } else if focused && self.pairing.paused && self.setup_gateway {
            self.pairing.paused = false;
            self.resume_paused_pairing();
        }
    }
}

enum RestoreOutcome {
    None,
    DurableOrPending(String),
    SessionEnded(String),
}

impl GatewayEffects {
    fn restore_pairing(
        &self,
        container: Arc<NativeContainer>,
        epoch: u64,
        requested_origin: Option<String>,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let catalog = container.pairing_catalog()?;
                let candidate = if let Some(requested) = requested_origin {
                    let requested = Client::canonical_origin(&requested)
                        .map_err(|_| PairingError::InvalidInput)?;
                    let exists = catalog
                        .paired_gateways
                        .iter()
                        .any(|pair| pair.origin == requested)
                        || catalog
                            .pending_pairings
                            .iter()
                            .any(|pending| pending.origin == requested);
                    exists.then_some(requested)
                } else {
                    catalog.selected_origin.clone().or_else(|| {
                        (catalog.pending_pairings.len() == 1)
                            .then(|| catalog.pending_pairings[0].origin.clone())
                    })
                };
                let Some(origin) = candidate else {
                    return Ok::<_, PairingError>(RestoreOutcome::None);
                };
                let has_pending = catalog
                    .pending_pairings
                    .iter()
                    .any(|pending| pending.origin == origin);
                if !has_pending
                    && catalog
                        .paired_gateways
                        .iter()
                        .find(|pair| pair.origin == origin)
                        .is_some_and(|pair| !pair.is_durable())
                {
                    return Ok(RestoreOutcome::SessionEnded(origin));
                }
                Ok(RestoreOutcome::DurableOrPending(origin))
            })
            .await
            .map_err(|_| PairingFailure::CatalogUnavailable)
            .and_then(|result| result.map_err(PairingFailure::from));
            ui.update(move |state, _| state.restored_pairing(epoch, result));
        });
        Some(RequestTask(task.abort_handle()))
    }

    fn pairing_connect(
        &self,
        container: Arc<NativeContainer>,
        origin: String,
        epoch: u64,
        storage_mode: CredentialStorage,
        restart: bool,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let store: Arc<dyn KeychainStore> = match storage_mode {
            CredentialStorage::SystemKeyring => self.keychain.clone(),
            CredentialStorage::SessionOnly => self.session_keychain.clone(),
            _ => self.keychain.clone(),
        };
        let task = self.runtime.spawn(async move {
            let progress_ui = ui.clone();
            let progress_origin = origin.clone();
            let result = connect_or_pair_with_storage(
                container,
                origin,
                store,
                storage_mode,
                restart,
                move |stage| {
                    let origin = progress_origin.clone();
                    progress_ui
                        .update(move |state, _| state.pairing_progress(epoch, &origin, stage));
                },
            )
            .await;
            ui.update(move |state, _| state.complete_pairing_connection(epoch, result));
        });
        Some(RequestTask(task.abort_handle()))
    }
}

#[derive(Clone)]
struct LocalPairing {
    container: Arc<NativeContainer>,
    origin: String,
    store: Arc<dyn KeychainStore>,
}
impl LocalPairing {
    async fn run<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&NativeContainer, &str, &dyn KeychainStore) -> Result<T, PairingError>
            + Send
            + 'static,
    ) -> Result<T, PairingFailure> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            operation(&this.container, &this.origin, this.store.as_ref())
        })
        .await
        .map_err(|_| PairingFailure::CatalogUnavailable)?
        .map_err(PairingFailure::from)
    }
}

async fn authenticated(origin: &str, secret: &SecretToken) -> Result<Connected, PairingFailure> {
    crate::connection::handshake_token(origin, Some(secret.expose_secret()), None)
        .await
        .map_err(|failure| match failure.phase {
            crate::connection::ConnectionTestPhase::Authentication => {
                PairingFailure::Authentication
            }
            crate::connection::ConnectionTestPhase::Reachability => PairingFailure::Network,
            crate::connection::ConnectionTestPhase::GatewayAvailability => {
                PairingFailure::GatewayUnavailable
            }
            _ => PairingFailure::Unsupported,
        })
}

#[cfg(test)]
async fn connect_or_pair(
    container: Arc<NativeContainer>,
    origin: String,
    store: Arc<dyn KeychainStore>,
    restart: bool,
    progress: impl FnMut(PairingStage),
) -> Result<Connected, PairingFailure> {
    connect_or_pair_with_storage(
        container,
        origin,
        store,
        CredentialStorage::SystemKeyring,
        restart,
        progress,
    )
    .await
}

async fn connect_or_pair_with_storage(
    container: Arc<NativeContainer>,
    origin: String,
    store: Arc<dyn KeychainStore>,
    storage_mode: CredentialStorage,
    restart: bool,
    mut progress: impl FnMut(PairingStage),
) -> Result<Connected, PairingFailure> {
    let origin = Client::canonical_origin(&origin).map_err(transport_failure)?;
    let local = LocalPairing {
        container,
        origin: origin.clone(),
        store,
    };
    local
        .run(|container, origin, store| container.recover_pairing(origin, store))
        .await?;
    let catalog = local
        .run(|container, _, _| container.pairing_catalog())
        .await?;
    if restart {
        if let Some(pending) = catalog.pending_pairings.iter().find(|p| p.origin == origin) {
            let request_id = pending.request_id.clone();
            local
                .run(move |c, o, s| c.clear_pending_pairing(catalog.revision, o, &request_id, s))
                .await?;
        }
    } else if !catalog.pending_pairings.iter().any(|p| p.origin == origin) {
        if let Some(pair) = catalog
            .paired_gateways
            .iter()
            .find(|p| p.origin == origin)
            .cloned()
        {
            if storage_mode == CredentialStorage::SystemKeyring && !pair.is_durable() {
                return Err(PairingFailure::SessionEnded);
            }
            let secret = local.run(|c, o, s| c.paired_credential(o, s)).await?;
            let connected = authenticated(&origin, &secret).await?;
            if connected.capabilities.deployment_id.as_deref() != Some(pair.deployment_id.as_str())
            {
                return Err(PairingFailure::Authentication);
            }
            local
                .run(move |c, o, _| {
                    let snapshot = c.pairing_catalog()?;
                    if c.paired_gateway(o)?.as_ref() != Some(&pair) {
                        return Err(PairingError::RevisionConflict);
                    }
                    c.select_paired_gateway(snapshot.revision, o)
                })
                .await?;
            return Ok(connected);
        }
    }
    let catalog = local.run(|c, _, _| c.pairing_catalog()).await?;
    if !catalog.pending_pairings.iter().any(|p| p.origin == origin) {
        let request_id =
            crate::platform::request_id().map_err(|_| PairingFailure::KeychainUnavailable)?;
        let mut entropy = [0_u8; 32];
        getrandom::fill(&mut entropy).map_err(|_| PairingFailure::KeychainUnavailable)?;
        let secret = SecretToken::new(entropy.iter().map(|byte| format!("{byte:02x}")).collect())
            .map_err(|_| PairingFailure::KeychainUnavailable)?;
        let pending = PendingPairing::new(&origin, request_id).map_err(PairingFailure::from)?;
        local
            .run(move |c, _, s| c.begin_pairing(catalog.revision, pending, &secret, s))
            .await?;
    }
    let (pending, secret, installation) = local
        .run(|c, o, s| {
            let snapshot = c.pairing_catalog()?;
            let pending = snapshot
                .pending_pairings
                .into_iter()
                .find(|p| p.origin == o)
                .ok_or(PairingError::RevisionConflict)?;
            let secret = c.pending_pairing_secret(o, s)?;
            Ok((pending, secret, c.installation_marker().to_owned()))
        })
        .await?;
    let transport = PairingClient::new(&origin).map_err(transport_failure)?;
    let challenge = transport
        .start(&PairingStartRequest {
            request_id: pending.request_id.clone(),
            installation_marker: installation,
            polling_secret: secret.expose_secret().to_owned(),
        })
        .await
        .map_err(transport_failure)?;
    if pending.issued_credential.is_none() {
        let request_id = pending.request_id.clone();
        let recorded = challenge.clone();
        local
            .run(move |c, o, _| {
                let snapshot = c.pairing_catalog()?;
                c.record_pairing_challenge(
                    snapshot.revision,
                    o,
                    &request_id,
                    recorded.pairing_id,
                    recorded.expires_at_ms,
                )
            })
            .await?;
    }
    progress(PairingStage::Waiting(challenge.clone()));
    let deadline = Instant::now() + Duration::from_secs(10 * 60);
    let poll = PairingPollRequest {
        pairing_id: challenge.pairing_id,
        polling_secret: secret.expose_secret().to_owned(),
    };
    loop {
        if Instant::now() >= deadline {
            return Err(PairingFailure::Expired);
        }
        match transport.poll(&poll).await.map_err(transport_failure)? {
            PairingStatus::Pending { poll_after_ms } => {
                tokio::time::sleep(Duration::from_millis(poll_after_ms.clamp(1000, 60_000))).await
            }
            PairingStatus::Denied => return Err(PairingFailure::Denied),
            PairingStatus::Expired => return Err(PairingFailure::Expired),
            PairingStatus::Approved {
                credential_id,
                access_token,
                ..
            } => {
                progress(PairingStage::Working(match storage_mode {
                    CredentialStorage::SessionOnly => {
                        "Completing session-only gateway access…".into()
                    }
                    _ if cfg!(target_os = "macos") => {
                        "Saving approved gateway access in Keychain…".into()
                    }
                    _ => "Saving approved gateway access in system keyring…".into(),
                }));
                let token =
                    SecretToken::new(access_token).map_err(|_| PairingFailure::Authentication)?;
                let connected = authenticated(&origin, &token).await?;
                let deployment = connected
                    .capabilities
                    .deployment_id
                    .clone()
                    .ok_or(PairingFailure::Unsupported)?;
                let pair = match storage_mode {
                    CredentialStorage::SessionOnly => {
                        PairedGateway::new_session_only(&origin, credential_id, deployment)
                    }
                    _ => PairedGateway::new(&origin, credential_id, deployment),
                }
                .map_err(PairingFailure::from)?;
                let request_id = pending.request_id.clone();
                local
                    .run(move |c, _, s| {
                        let snapshot = c.pairing_catalog()?;
                        c.complete_pairing(snapshot.revision, &request_id, pair, &token, s)
                    })
                    .await?;
                return Ok(connected);
            }
            _ => return Err(PairingFailure::Unsupported),
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod restore_tests {
    use fleetworks_workstation::native::BackendKind;

    use super::*;

    #[test]
    fn saved_pairing_restore_preloads_origin_without_dispatching_connect() {
        let mut state = State::fixture();
        state.pairing.fixture_enabled = true;
        state.startup = crate::startup::Startup::cold_launch();
        state.backend_kind = BackendKind::Remote;
        let epoch = state.connection.epoch;

        state.restored_pairing(
            epoch,
            Ok(RestoreOutcome::DurableOrPending(
                "https://gateway.example.test/".into(),
            )),
        );

        assert_eq!(
            state.connection.origin.text,
            "https://gateway.example.test/"
        );
        assert_eq!(state.connection.status, ConnectionStatus::Disconnected);
        assert_eq!(state.pairing.stage, PairingStage::Idle);
        assert!(state.pairing.request.is_none());
        assert!(
            state.intents.is_empty(),
            "saved-pairing restore must not dispatch Connect or PairingConnect: {:?}",
            state.intents
        );

        // Selecting Distributed from startup keeps the preloaded origin and still never connects.
        state.choose_startup_destination(BackendKind::Remote);
        assert_eq!(
            state.connection.origin.text,
            "https://gateway.example.test/"
        );
        assert_eq!(state.connection.status, ConnectionStatus::Disconnected);
        assert_eq!(state.pairing.stage, PairingStage::Idle);
        assert!(state.setup_gateway);
        assert!(
            !state
                .intents
                .iter()
                .any(|intent| matches!(intent, Intent::Connect | Intent::PairingConnect)),
            "choosing Distributed must not auto-connect: {:?}",
            state.intents
        );
    }

    #[test]
    fn saved_pairing_restore_does_not_overwrite_local_backend_or_stale_epoch() {
        let mut state = State::fixture();
        state.backend_kind = BackendKind::Local;
        state.connection.origin = rune::TextEdit::new("local://embedded");
        let epoch = state.connection.epoch;

        state.restored_pairing(
            epoch,
            Ok(RestoreOutcome::DurableOrPending(
                "https://gateway.example.test/".into(),
            )),
        );
        assert_eq!(state.connection.origin.text, "local://embedded");
        assert!(state.intents.is_empty());

        state.backend_kind = BackendKind::Remote;
        state.restored_pairing(
            epoch.wrapping_add(1),
            Ok(RestoreOutcome::DurableOrPending(
                "https://stale.example.test/".into(),
            )),
        );
        assert_eq!(state.connection.origin.text, "local://embedded");
        assert!(state.intents.is_empty());
    }

    #[test]
    fn choosing_distributed_after_embedded_restores_the_saved_url() {
        let mut state = State::fixture();
        state.startup = crate::startup::Startup::cold_launch();
        state.backend_kind = BackendKind::Local;
        state.connection.origin = rune::TextEdit::new("local://embedded");
        state.restored_pairing(
            state.connection.epoch,
            Ok(RestoreOutcome::DurableOrPending(
                "https://gateway.example.test/".into(),
            )),
        );

        state.choose_startup_destination(BackendKind::Remote);

        assert_eq!(
            state.connection.origin.text,
            "https://gateway.example.test/"
        );
        assert_eq!(state.connection.status, ConnectionStatus::Disconnected);
        assert!(!state
            .intents
            .iter()
            .any(|intent| matches!(intent, Intent::Connect | Intent::PairingConnect)));
    }
}
