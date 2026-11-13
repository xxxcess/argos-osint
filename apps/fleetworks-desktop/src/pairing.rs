//! Native pairing progression. Secret values live only in Keychain or effect workers.

use crate::connection::RequestTask;
use fleetworks_contracts::pairing::PairingChallenge;

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
    CatalogUnavailable,
    Network,
    Authentication,
    Unsupported,
    Denied,
    Expired,
}
impl PairingFailure {
    pub fn message(self) -> &'static str {
        match self {
            Self::InvalidOrigin => "Enter a gateway HTTPS URL or a local loopback HTTP URL.",
            Self::KeychainMissing => "Gateway access is missing from Keychain. Pair this workstation again.",
            Self::KeychainDenied => "Unlock Keychain or allow Fleetworks access, then try again.",
            Self::KeychainUnavailable => "Keychain is unavailable. Your access has not been saved; try again.",
            Self::CatalogUnavailable => "Protected pairing settings are unavailable. Restore access to this installation, then retry.",
            Self::Network => "Could not reach the gateway. Check its URL and connection, then retry.",
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
    /// Enables pairing handlers in synthetic Runtime fixtures, without effects.
    pub fixture_enabled: bool,
    pub(crate) request: Option<RequestTask>,
    pub(crate) startup_loaded: bool,
    pub(crate) paused: bool,
}
impl Pairing {
    pub fn busy(&self) -> bool {
        matches!(
            self.stage,
            PairingStage::Working(_) | PairingStage::Waiting(_)
        ) && !self.paused
    }
    pub(crate) fn cancel(&mut self) {
        self.request = None;
        self.paused = false;
        self.stage = PairingStage::Idle;
    }
}

use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use fleetworks_contracts::pairing::PairingPollRequest;
use fleetworks_contracts::pairing::PairingStartRequest;
use fleetworks_contracts::pairing::PairingStatus;
use fleetworks_contracts::Client;
use fleetworks_contracts::ClientError;
use fleetworks_contracts::PairingClient;
use fleetworks_workstation::native::keychain::KeychainError;
use fleetworks_workstation::native::keychain::KeychainStore;
use fleetworks_workstation::native::keychain::SecretToken;
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
        ClientError::Status(404 | 405 | 426) | ClientError::UnsupportedVersion(_) => {
            PairingFailure::Unsupported
        }
        ClientError::Transport | ClientError::Timeout => PairingFailure::Network,
        _ => PairingFailure::Unsupported,
    }
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

    pub(crate) fn connect_pairing(&mut self, restart: bool) {
        self.connection_profiles.initial_selection_applied = true;
        self.disconnect();
        self.connection.status = ConnectionStatus::Connecting;
        self.pairing.stage = PairingStage::Working("Checking saved gateway access…".into());
        self.record(if restart {
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
            restart,
        );
    }

    pub(crate) fn pairing_progress(&mut self, epoch: u64, stage: PairingStage) {
        if self.connection.epoch == epoch {
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
                self.connection.status = ConnectionStatus::Failed(error.message().into());
                self.pairing.stage = PairingStage::Failed(error);
            }
        }
    }

    /// Restore only the explicitly selected origin. An empty installation never
    /// starts a pairing request or adopts a token file automatically.
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

    fn restored_pairing(&mut self, epoch: u64, result: Result<Option<String>, PairingFailure>) {
        if self.connection.epoch != epoch {
            return;
        }
        self.pairing.request = None;
        match result {
            Ok(Some(origin)) => {
                self.connection.origin = rune::TextEdit::new(origin);
                self.connect_pairing(false);
            }
            Ok(None) => {}
            Err(error) => self.pairing.stage = PairingStage::Failed(error),
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
            self.connect_pairing(false);
        }
    }
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
                if let Some(requested) = requested_origin {
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
                    Ok::<_, PairingError>(exists.then_some(requested))
                } else {
                    Ok::<_, PairingError>(catalog.selected_origin.or_else(|| {
                        (catalog.pending_pairings.len() == 1)
                            .then(|| catalog.pending_pairings[0].origin.clone())
                    }))
                }
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
        restart: bool,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let keychain = self.keychain.clone();
        let task = self.runtime.spawn(async move {
            let progress_ui = ui.clone();
            let result = connect_or_pair(container, origin, keychain, restart, move |stage| {
                progress_ui.update(move |state, _| state.pairing_progress(epoch, stage));
            })
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
            _ => PairingFailure::Unsupported,
        })
}

async fn connect_or_pair(
    container: Arc<NativeContainer>,
    origin: String,
    store: Arc<dyn KeychainStore>,
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
                progress(PairingStage::Working(
                    "Saving approved gateway access in Keychain…".into(),
                ));
                let token =
                    SecretToken::new(access_token).map_err(|_| PairingFailure::Authentication)?;
                let connected = authenticated(&origin, &token).await?;
                let deployment = connected
                    .capabilities
                    .deployment_id
                    .clone()
                    .ok_or(PairingFailure::Unsupported)?;
                let pair = PairedGateway::new(&origin, credential_id, deployment)
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
