//! Protected folder state and worker effects, shared by production and fixtures.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use fleetworks_workspace::native::BackendBinding;
use fleetworks_workspace::native::NativeContainer;
use fleetworks_workspace::native::NativeWorkspace;
use fleetworks_workspace::native::RebindAttempt;
use fleetworks_workspace::native::RegistrationAttempt;
use fleetworks_workspace::native::RemovalKind;
use fleetworks_workspace::native::WorkspaceOperation;
use fleetworks_workspace::native::WorkspaceState;
use rune::TextEdit;
use rune::Tx;

use crate::app::Intent;
use crate::app::Load;
use crate::app::State;
use crate::connection::GatewayEffects;
use crate::connection::RequestTask;
use crate::platform;
use crate::workspace_lifecycle;
use crate::workspace_lifecycle::RetirementGateway;
use fleetworks_contracts::context::Permission;

/// Presentation snapshot only. Every filesystem effect reselects via the manager.
#[derive(Clone, Debug)]
pub struct WorkspaceRow {
    pub name: String,
    pub path: PathBuf,
    pub marker: String,
    pub protected_default: bool,
    pub state: WorkspaceState,
    pub bindings: Vec<BackendBinding>,
}

impl From<NativeWorkspace> for WorkspaceRow {
    fn from(workspace: NativeWorkspace) -> Self {
        Self {
            name: workspace.name().as_str().to_owned(),
            path: workspace.path().to_path_buf(),
            marker: workspace.marker().to_owned(),
            protected_default: workspace.is_default(),
            state: workspace.state(),
            bindings: workspace.bindings().to_vec(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct WorkspaceSnapshot {
    pub root: PathBuf,
    pub installation_marker: String,
    pub rows: Vec<WorkspaceRow>,
    pub operations: Vec<WorkspaceOperation>,
    pub registration_attempts: Vec<RegistrationAttempt>,
    pub rebind_attempts: Vec<RebindAttempt>,
}

#[derive(Clone, Debug)]
pub struct RemovalConfirmation {
    pub name: String,
    pub marker: String,
    pub kind: RemovalKind,
    pub origin_revision: u64,
    pub path: PathBuf,
    pub bindings: Vec<BackendBinding>,
}

#[derive(Default)]
pub struct Workspaces {
    pub snapshot: Load<WorkspaceSnapshot>,
    pub selected: Option<WorkspaceRow>,
    pub create_name: TextEdit,
    pub attach_name: TextEdit,
    pub attach_marker: TextEdit,
    pub message: Option<String>,
    pub busy: bool,
    pub confirmation: Option<RemovalConfirmation>,
    pub(crate) revision: u64,
    pub(crate) request: Option<RequestTask>,
    pub(crate) container: Option<Arc<NativeContainer>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceCommand {
    Refresh,
    Select(String),
    Create(String),
    Attach { name: String, marker: String },
    Reveal(String),
    Remove { name: String, kind: RemovalKind },
    Resume(String),
}

pub(crate) struct WorkspaceUpdate {
    pub(crate) container: Arc<NativeContainer>,
    pub(crate) snapshot: WorkspaceSnapshot,
    pub(crate) selected: Option<WorkspaceRow>,
    pub(crate) message: Option<String>,
}

impl State {
    pub fn copy_workspace_path(&mut self, name: &str, tx: &mut Tx) {
        let Load::Ready(snapshot) = &self.workspaces.snapshot else {
            return;
        };
        let Some(row) = snapshot.rows.iter().find(|row| row.name == name) else {
            return;
        };
        let path = row.path.display().to_string();
        if self.fixture {
            self.record(Intent::CopyWorkspacePath {
                name: name.to_owned(),
            });
        } else {
            tx.copy(path);
        }
    }
    pub fn initialize_workspaces(&mut self, selection: Option<PathBuf>) {
        if self.fixture || self.workspaces.busy {
            return;
        }
        self.workspaces.busy = true;
        self.workspaces.snapshot = Load::Loading;
        self.workspaces.revision = self.workspaces.revision.wrapping_add(1);
        if let Some(services) = &self.services {
            self.workspaces.request =
                services.initialize_workspaces(selection, self.workspaces.revision);
        }
    }

    /// Folder-only changes preserve backend drafts and Brain search/dossier state.
    pub fn change_workspace(&mut self, workspace: WorkspaceRow) {
        let changed = self
            .workspaces
            .selected
            .as_ref()
            .is_none_or(|old| old.marker != workspace.marker);
        self.selected_workspace = Some(workspace.path.clone());
        self.workspaces.selected = Some(workspace);
        if changed {
            self.invalidate_origin();
        }
    }

    pub fn workspace_command(&mut self, command: WorkspaceCommand) {
        if self.workspaces.busy {
            return;
        }
        self.record(Intent::Workspace(command.clone()));
        if self.fixture {
            return;
        }
        let gateway = if self.permission(Permission::RebindWorkstation) {
            self.client.as_ref().map(|client| RetirementGateway {
                client: client.clone(),
                deployment: self.connection.deployment_label().to_owned(),
            })
        } else {
            None
        };
        let (Some(services), Some(container)) = (&self.services, &self.workspaces.container) else {
            self.workspaces.message = Some("Open the protected workspace container first".into());
            return;
        };
        self.workspaces.busy = true;
        self.workspaces.message = None;
        self.workspaces.revision = self.workspaces.revision.wrapping_add(1);
        self.workspaces.request = services.workspace_command(
            container.clone(),
            command,
            gateway,
            self.workspaces.revision,
        );
    }

    pub fn preview_removal(&mut self, name: &str, kind: RemovalKind) {
        let Load::Ready(snapshot) = &self.workspaces.snapshot else {
            return;
        };
        let Some(row) = snapshot.rows.iter().find(|row| row.name == name) else {
            return;
        };
        if row.protected_default {
            self.workspaces.message = Some("The default workspace is protected".into());
            return;
        }
        if row
            .bindings
            .iter()
            .any(|binding| !binding.retired && (binding.stale || binding.receipt.is_none()))
        {
            self.workspaces.message =
                Some("Refresh this folder's backend registrations before removal".into());
            return;
        }
        self.workspaces.confirmation = Some(RemovalConfirmation {
            name: row.name.clone(),
            marker: row.marker.clone(),
            kind,
            origin_revision: self.origin_revision,
            path: row.path.clone(),
            bindings: row.bindings.clone(),
        });
        self.reveal_control("workspace-removal-confirmation");
    }

    pub fn confirm_removal(&mut self) {
        let Some(confirmation) = self.workspaces.confirmation.take() else {
            return;
        };
        if confirmation.origin_revision != self.origin_revision {
            self.workspaces.message =
                Some("Workspace context changed; review the removal again".into());
            return;
        }
        let Load::Ready(snapshot) = &self.workspaces.snapshot else {
            return;
        };
        if !snapshot.rows.iter().any(|row| {
            row.name == confirmation.name
                && row.marker == confirmation.marker
                && !row.protected_default
                && row.path == confirmation.path
                && row.bindings == confirmation.bindings
        }) {
            self.workspaces.message =
                Some("Workspace identity changed; refresh before removal".into());
            return;
        }
        self.invalidate_origin();
        self.workspace_command(WorkspaceCommand::Remove {
            name: confirmation.name,
            kind: confirmation.kind,
        });
    }

    pub(crate) fn complete_workspaces(
        &mut self,
        revision: u64,
        result: Result<WorkspaceUpdate, String>,
    ) {
        if revision != self.workspaces.revision {
            return;
        }
        self.workspaces.request = None;
        self.workspaces.busy = false;
        match result {
            Ok(update) => {
                self.workspaces.container = Some(update.container);
                self.workspaces.snapshot = Load::Ready(update.snapshot);
                self.workspaces.message = update.message;
                if let Some(selected) = update.selected {
                    self.change_workspace(selected);
                } else if let Some(old) = &self.workspaces.selected {
                    let Load::Ready(snapshot) = &self.workspaces.snapshot else {
                        return;
                    };
                    let current = snapshot
                        .rows
                        .iter()
                        .find(|row| row.marker == old.marker)
                        .cloned();
                    if let Some(row) = current {
                        self.change_workspace(row);
                    } else {
                        self.workspaces.selected = None;
                        self.selected_workspace = None;
                        self.invalidate_origin();
                    }
                }
            }
            Err(error) => {
                if matches!(self.workspaces.snapshot, Load::Loading) {
                    self.workspaces.snapshot = Load::Failed(error.clone());
                }
                self.workspaces.message = Some(error);
            }
        }
        if matches!(self.connection_profiles.catalog, Load::Idle)
            && self.workspaces.container.is_some()
        {
            self.initialize_connection_profiles();
        }
    }
}

impl GatewayEffects {
    fn initialize_workspaces(
        &self,
        selection: Option<PathBuf>,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                anyhow::ensure!(selection.is_none(), "workspace selection is retired");
                let container = Arc::new(NativeContainer::open_settings(&platform::home()?)?);
                Ok(WorkspaceUpdate {
                    snapshot: WorkspaceSnapshot {
                        root: container.root().to_path_buf(),
                        installation_marker: container.installation_marker().to_owned(),
                        rows: Vec::new(),
                        operations: Vec::new(),
                        registration_attempts: Vec::new(),
                        rebind_attempts: Vec::new(),
                    },
                    container,
                    selected: None,
                    message: None,
                })
            })
            .await
            .map_err(|_| "Workspace worker stopped".to_owned())
            .and_then(|result: Result<WorkspaceUpdate>| result.map_err(|error| error.to_string()));
            ui.update(move |state, _| state.complete_workspaces(revision, result));
        });
        Some(RequestTask(task.abort_handle()))
    }

    fn workspace_command(
        &self,
        container: Arc<NativeContainer>,
        command: WorkspaceCommand,
        gateway: Option<RetirementGateway>,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result = if matches!(
                command,
                WorkspaceCommand::Remove { .. } | WorkspaceCommand::Resume(_)
            ) {
                workspace_lifecycle::run_removal(container, command, gateway).await
            } else {
                tokio::task::spawn_blocking(move || run_command(container, command))
                    .await
                    .map_err(|_| "Workspace worker stopped".to_owned())
                    .and_then(|result| result.map_err(|error| error.to_string()))
            };
            ui.update(move |state, _| state.complete_workspaces(revision, result));
        });
        Some(RequestTask(task.abort_handle()))
    }
}

pub(crate) fn snapshot(container: &NativeContainer) -> Result<WorkspaceSnapshot> {
    Ok(WorkspaceSnapshot {
        root: container.root().to_path_buf(),
        installation_marker: container.installation_marker().to_owned(),
        rows: container
            .list()?
            .into_iter()
            .map(WorkspaceRow::from)
            .collect(),
        operations: container.pending_operations()?,
        registration_attempts: container.pending_registration_attempts()?,
        rebind_attempts: container.pending_rebind_attempts()?,
    })
}

fn run_command(
    container: Arc<NativeContainer>,
    command: WorkspaceCommand,
) -> Result<WorkspaceUpdate> {
    let mut selected = None;
    let message = match command {
        WorkspaceCommand::Refresh => None,
        WorkspaceCommand::Select(name) => {
            selected = Some(container.select(Some(name.as_ref()))?.into());
            None
        }
        WorkspaceCommand::Create(name) => {
            selected = Some(container.create(&name)?.into());
            Some(
                "Workspace created locally; registration is a separate authenticated action".into(),
            )
        }
        WorkspaceCommand::Attach { name, marker } => {
            selected = Some(container.attach_owned_child(&name, &marker)?.into());
            Some("Existing owned child attached without importing settings or keys".into())
        }
        WorkspaceCommand::Reveal(name) => {
            let workspace = container.select(Some(name.as_ref()))?;
            platform::reveal(workspace.path())?;
            Some("Requested Finder reveal for the verified child".into())
        }
        WorkspaceCommand::Remove { .. } | WorkspaceCommand::Resume(_) => {
            anyhow::bail!("use the confirmed folder lifecycle worker")
        }
    };
    Ok(WorkspaceUpdate {
        snapshot: snapshot(&container)?,
        container,
        selected,
        message,
    })
}
