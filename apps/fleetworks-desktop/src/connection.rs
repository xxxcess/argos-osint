//! Gateway-only effects. No application operation starts a process or service.

use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;

use fleetworks_contracts::capabilities::Capabilities;
use fleetworks_contracts::readiness::WorkstationReadiness;
use fleetworks_contracts::search::ActorSearchQuery;
use fleetworks_contracts::search::ActorSearchResponse;
use fleetworks_contracts::Client;
use fleetworks_contracts::Ready;
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
}

/// One app-owned Tokio executor sends results back through Rune's UI-thread handle.
pub struct GatewayEffects {
    pub(crate) runtime: Handle,
    pub(crate) ui: OnceLock<AppHandle<State>>,
}

impl GatewayEffects {
    pub fn new(runtime: Handle) -> Self {
        Self {
            runtime,
            ui: OnceLock::new(),
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
        token_file: Option<PathBuf>,
        epoch: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result = connect(&origin, token_file.as_deref()).await;
            ui.update(move |state, _| state.complete_connection(epoch, result));
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

async fn connect(origin: &str, token_file: Option<&Path>) -> Result<Connected, String> {
    let token = match token_file {
        Some(path) => Some(read_private_token(path).await?),
        None => None,
    };
    let client = Client::new(origin, token.as_deref()).map_err(|error| error.to_string())?;
    let ready = client.ready().await.map_err(|error| error.to_string())?;
    let capabilities: Capabilities = client
        .get(fleetworks_contracts::CAPABILITIES_PATH)
        .await
        .map_err(|error| error.to_string())?;
    capabilities
        .require_native_contract()
        .map_err(|error| error.to_string())?;
    Ok(Connected {
        client,
        ready,
        capabilities,
    })
}

async fn read_private_token(path: &Path) -> Result<String, String> {
    let metadata = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|_| "Cannot read the selected protected credential file".to_owned())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > TOKEN_LIMIT {
        return Err("Credential file must be a bounded regular file, not a symbolic link".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("Credential file must be readable only by its owner (mode 0600)".into());
        }
    }
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| "Cannot open the selected protected credential file".to_owned())?;
    let opened = file
        .metadata()
        .await
        .map_err(|_| "Cannot inspect the selected protected credential file".to_owned())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.dev() != opened.dev() || metadata.ino() != opened.ino() {
            return Err("Credential file changed while it was being opened".into());
        }
    }
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    file.take(TOKEN_LIMIT + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| "Cannot read the selected protected credential file".to_owned())?;
    if bytes.len() as u64 > TOKEN_LIMIT {
        return Err("Credential file exceeds its size limit".into());
    }
    let value = String::from_utf8(bytes).map_err(|_| "Invalid credential file".to_owned())?;
    Ok(value.trim_end_matches(['\r', '\n']).to_owned())
}
