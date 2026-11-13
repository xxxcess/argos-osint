//! Gateway-only effects. No application operation starts a process or service.

use std::sync::Arc;
use std::sync::OnceLock;

use fleetworks_contracts::capabilities::Capabilities;
use fleetworks_contracts::readiness::WorkstationReadiness;
use fleetworks_contracts::search::ActorSearchQuery;
use fleetworks_contracts::search::ActorSearchResponse;
use fleetworks_contracts::Client;
use fleetworks_contracts::ClientError;
use fleetworks_contracts::Ready;
use fleetworks_workstation::native::CredentialReference;
use fleetworks_workstation::native::NativeContainer;
use rune::AppHandle;
use rune::TextEdit;
use tokio::runtime::Handle;
use tokio::task::AbortHandle;

use crate::app::State;
use crate::brain::ActorDossier;

const TOKEN_LIMIT: u64 = 4096;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum ConnectionStatus {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Failed(String),
}

pub struct Connection {
    pub origin: TextEdit,
    pub token_file: TextEdit,
    pub status: ConnectionStatus,
    pub ready: Option<Ready>,
    pub capabilities: Option<Capabilities>,
    pub(crate) epoch: u64,
}

impl Default for Connection {
    fn default() -> Self {
        Self {
            origin: TextEdit::new(fleetworks_contracts::DEFAULT_API_ORIGIN),
            token_file: TextEdit::new(""),
            status: ConnectionStatus::Disconnected,
            ready: None,
            capabilities: None,
            epoch: 0,
        }
    }
}

impl Connection {
    pub fn deployment_label(&self) -> &str {
        self.capabilities
            .as_ref()
            .and_then(|capabilities| capabilities.deployment_id.as_deref())
            .unwrap_or("No connected deployment")
    }
}

/// Finite client requests stop when their owning state is replaced or closes.
pub(crate) struct RequestTask(pub(crate) AbortHandle);

impl Drop for RequestTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(crate) struct Connected {
    pub client: Client,
    pub ready: Ready,
    pub capabilities: Capabilities,
    pub credential: Option<CredentialReference>,
}

#[derive(Clone)]
pub(crate) enum ConnectionCredential {
    None,
    Reference(CredentialReference),
    Provisioned(Arc<NativeContainer>),
    Explicit(std::path::PathBuf),
    Unavailable,
}
impl ConnectionCredential {
    pub(crate) fn reference(&self, origin: &str) -> anyhow::Result<Option<CredentialReference>> {
        match self {
            Self::None => Ok(None),
            Self::Provisioned(container) => container.credential_binding(origin).map(Some),
            Self::Explicit(path) => {
                let reference = CredentialReference::capture(path, origin)?;
                drop(reference.open_verified(origin)?);
                Ok(Some(reference))
            }
            Self::Reference(reference) => {
                drop(reference.open_verified(origin)?);
                Ok(Some(reference.clone()))
            }
            Self::Unavailable => anyhow::bail!(
                "refresh saved connection metadata before using its credential reference"
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionTestPhase {
    Origin,
    CredentialReference,
    Reachability,
    Authentication,
    Compatibility,
    Readiness,
    GatewayAvailability,
}
#[derive(Clone, Debug)]
pub struct ConnectionTestFailure {
    pub phase: ConnectionTestPhase,
    pub message: String,
}
#[derive(Clone, Debug)]
pub struct ConnectionTestSummary {
    pub origin: String,
    pub deployment_id: String,
    pub api_version: u16,
    pub ready: bool,
    pub storage: bool,
    pub credential: Option<CredentialReference>,
}

/// One app-owned Tokio executor sends results back through Rune's UI-thread handle.
pub struct GatewayEffects {
    pub(crate) runtime: Handle,
    pub(crate) ui: OnceLock<AppHandle<State>>,
    pub(crate) keychain: Arc<dyn fleetworks_workstation::native::keychain::KeychainStore>,
}

impl GatewayEffects {
    pub fn new(runtime: Handle) -> Self {
        Self {
            runtime,
            ui: OnceLock::new(),
            keychain: Arc::new(fleetworks_workstation::native::keychain::SystemKeychain),
        }
    }

    pub fn install_ui(&self, handle: AppHandle<State>) -> Result<(), &'static str> {
        self.ui
            .set(handle)
            .map_err(|_| "UI handle already installed")
    }

    pub(crate) fn connect(
        &self,
        origin: String,
        credential: ConnectionCredential,
        epoch: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result = handshake(&origin, credential)
                .await
                .map_err(|error| error.message);
            ui.update(move |state, _| state.complete_connection(epoch, result));
        });
        Some(RequestTask(task.abort_handle()))
    }

    pub(crate) fn test_profile(
        &self,
        origin: String,
        credential: ConnectionCredential,
        epoch: u64,
        draft: u64,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result =
                handshake(&origin, credential)
                    .await
                    .map(|connected| ConnectionTestSummary {
                        origin: Client::canonical_origin(&origin)
                            .expect("handshake validated the origin"),
                        deployment_id: connected
                            .capabilities
                            .deployment_id
                            .unwrap_or(connected.ready.instance_id),
                        api_version: connected.ready.api_version,
                        ready: connected.ready.ready,
                        storage: connected.ready.storage,
                        credential: connected.credential,
                    });
            // The test's client is dropped here. Only Connect may install an
            // authenticated client or change the active connection status.
            ui.update(move |state, _| {
                if state.connection.epoch == epoch
                    && state.connection_profiles.draft_revision == draft
                {
                    state.complete_profile_test(revision, result);
                }
            });
        });
        Some(RequestTask(task.abort_handle()))
    }

    pub(crate) fn readiness(
        &self,
        client: Client,
        epoch: u64,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result: Result<WorkstationReadiness, String> = client
                .get("/api/v1/workstation/readiness")
                .await
                .map_err(|error| error.to_string());
            ui.update(move |state, tx| {
                if state.connection.epoch == epoch && state.readiness_revision == revision {
                    state.readiness_received_at = Some(tx.clock());
                    state.complete_readiness(result);
                }
            });
        });
        Some(RequestTask(task.abort_handle()))
    }

    pub(crate) fn search(
        &self,
        client: Client,
        query: ActorSearchQuery,
        epoch: u64,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result: Result<ActorSearchResponse, String> = client
                .get_with_query("/api/v1/actors/search", &query)
                .await
                .map_err(|error| error.to_string());
            ui.update(move |state, tx| {
                if state.connection.epoch == epoch {
                    state.brain.complete_search(revision, result);
                    state.search_request = None;
                    if let Some(deadline) = state.brain.pending_at {
                        tx.wake_after(std::time::Duration::from_secs_f64(
                            (deadline - tx.clock()).max(0.0),
                        ));
                    }
                }
            });
        });
        Some(RequestTask(task.abort_handle()))
    }

    pub(crate) fn dossier(
        &self,
        client: Client,
        id: String,
        epoch: u64,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result: Result<ActorDossier, String> = client
                .get_with_query("/api/v1/actors/detail", &[("id", id)])
                .await
                .map_err(|error| error.to_string());
            ui.update(move |state, _| {
                if state.connection.epoch == epoch && state.brain.detail_revision == revision {
                    state.brain.complete_dossier(revision, result);
                    state.dossier_request = None;
                }
            });
        });
        Some(RequestTask(task.abort_handle()))
    }
}

async fn handshake(
    origin: &str,
    credential: ConnectionCredential,
) -> Result<Connected, ConnectionTestFailure> {
    let origin = Client::canonical_origin(origin).map_err(|error| ConnectionTestFailure {
        phase: ConnectionTestPhase::Origin,
        message: error.to_string(),
    })?;
    if origin.len() > 2048 {
        return Err(ConnectionTestFailure {
            phase: ConnectionTestPhase::Origin,
            message: "Gateway origin exceeds the saved-connection size limit".into(),
        });
    }
    let (token, credential) = read_private_token(origin.clone(), credential).await?;
    handshake_token(&origin, token.as_deref(), credential).await
}

pub(crate) async fn handshake_token(
    origin: &str,
    token: Option<&str>,
    credential: Option<CredentialReference>,
) -> Result<Connected, ConnectionTestFailure> {
    let origin = Client::canonical_origin(origin).map_err(|error| ConnectionTestFailure {
        phase: ConnectionTestPhase::Origin,
        message: error.to_string(),
    })?;
    if origin.len() > 2048 {
        return Err(ConnectionTestFailure {
            phase: ConnectionTestPhase::Origin,
            message: "Gateway URL is too long".into(),
        });
    }
    let client = Client::new(&origin, token)
        .map_err(|error| client_failure(error, ConnectionTestPhase::Authentication))?;
    let ready = client
        .ready()
        .await
        .map_err(|error| client_failure(error, ConnectionTestPhase::Readiness))?;
    let capabilities: Capabilities = client
        .get(fleetworks_contracts::CAPABILITIES_PATH)
        .await
        .map_err(|error| client_failure(error, ConnectionTestPhase::Compatibility))?;
    capabilities
        .require_native_contract()
        .map_err(|error| ConnectionTestFailure {
            phase: ConnectionTestPhase::Compatibility,
            message: error.to_string(),
        })?;
    Ok(Connected {
        client,
        ready,
        capabilities,
        credential,
    })
}

async fn read_private_token(
    origin: String,
    source: ConnectionCredential,
) -> Result<(Option<String>, Option<CredentialReference>), ConnectionTestFailure> {
    tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        use std::io::Read;
        let Some(reference) = source.reference(&origin)? else {
            return Ok((None, None));
        };
        let file = reference.open_verified(&origin)?;
        let mut bytes = Vec::new();
        file.take(TOKEN_LIMIT + 1).read_to_end(&mut bytes)?;
        anyhow::ensure!(
            bytes.len() as u64 <= TOKEN_LIMIT,
            "credential file exceeds bound"
        );
        let value = String::from_utf8(bytes)?;
        Ok((
            Some(value.trim_end_matches(['\r', '\n']).to_owned()),
            Some(reference),
        ))
    })
    .await
    .map_err(|_| ConnectionTestFailure {
        phase: ConnectionTestPhase::CredentialReference,
        message: "Credential reference worker stopped".into(),
    })?
    .map_err(|_| ConnectionTestFailure {
        phase: ConnectionTestPhase::CredentialReference,
        message:
            "The explicitly configured gateway credential file is missing, changed or unavailable"
                .into(),
    })
}

fn client_failure(error: ClientError, phase: ConnectionTestPhase) -> ConnectionTestFailure {
    let phase = match error {
        ClientError::Transport | ClientError::Timeout => ConnectionTestPhase::Reachability,
        ClientError::Status(401 | 403) | ClientError::MissingToken | ClientError::InvalidToken => {
            ConnectionTestPhase::Authentication
        }
        ClientError::UnsupportedVersion(_) | ClientError::Status(404 | 405 | 426) => {
            ConnectionTestPhase::Compatibility
        }
        ClientError::Status(_) | ClientError::InvalidResponse | ClientError::ResponseTooLarge => {
            ConnectionTestPhase::GatewayAvailability
        }
        _ => phase,
    };
    ConnectionTestFailure {
        phase,
        message: error.to_string(),
    }
}
