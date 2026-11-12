//! Explicit workstation IP changes with a durable exact-request retry journal.

use std::sync::Arc;

use anyhow::Context;
use anyhow::Result;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::Client;
use fleetworks_workspace::native::NativeContainer;
use fleetworks_workspace::native::RebindAttempt;
use serde::Serialize;

use crate::app::Intent;
use crate::app::Load;
use crate::app::State;
use crate::connection::GatewayEffects;
use crate::connection::RequestTask;
use crate::platform;
use crate::platform::NativeInterface;
use crate::workspaces;
use crate::workspaces::WorkspaceRow;
use crate::workspaces::WorkspaceUpdate;

#[derive(Clone)]
pub struct RebindConfirmation {
    pub workspace: WorkspaceRow,
    pub prior: RegistrationReceipt,
    pub interface: NativeInterface,
    pub origin_revision: u64,
    pub epoch: u64,
}

#[derive(Default)]
pub struct Rebind {
    pub confirmation: Option<RebindConfirmation>,
    pub(crate) request: Option<RequestTask>,
}

#[derive(Serialize)]
struct RebindRequest<'a> {
    request_id: &'a str,
    registration_id: &'a str,
    expected_revision: u64,
    workstation_ip: std::net::IpAddr,
    workspace_name: &'a fleetworks_contracts::connect::WorkspaceName,
    installation_marker: &'a str,
    workspace_marker: &'a str,
}

impl State {
    pub fn rebind_confirmation(&self) -> Result<RebindConfirmation, String> {
        if !self.permission(Permission::RebindWorkstation) || self.workspaces.busy {
            return Err("Connect with folder management permission first".into());
        }
        if matches!(&self.workspaces.snapshot, Load::Ready(snapshot) if snapshot.rebind_attempts.iter()
            .any(|attempt| attempt.prior.deployment_id == self.connection.deployment_label()))
        {
            return Err("Recheck the pending IP change in Workspaces first".into());
        }
        let workspace = self
            .workspaces
            .selected
            .as_ref()
            .ok_or("Select an owned workspace first")?;
        let binding = workspace
            .bindings
            .iter()
            .find(|binding| {
                binding.deployment_id == self.connection.deployment_label()
                    && !binding.retired
                    && !binding.stale
            })
            .ok_or("Refresh this workspace's backend registration first")?;
        let prior = binding
            .receipt
            .as_ref()
            .ok_or("Refresh this workspace's complete registration receipt first")?;
        let interface = self.registration.selected_interface.as_ref().filter(|interface|
            matches!(&self.registration.interfaces, Load::Ready(snapshot) if snapshot.interfaces.contains(interface)))
            .ok_or("Select the new current LAN or VPN interface first")?;
        if interface.ip == prior.workstation_ip {
            return Err("Choose a different current interface to change the workstation IP".into());
        }
        Ok(RebindConfirmation {
            workspace: workspace.clone(),
            prior: prior.clone(),
            interface: interface.clone(),
            origin_revision: self.origin_revision,
            epoch: self.connection.epoch,
        })
    }

    pub fn preview_rebind(&mut self) {
        match self.rebind_confirmation() {
            Ok(confirmation) => {
                self.rebind.confirmation = Some(confirmation);
                self.reveal_control("workspace-rebind-confirmation");
            }
            Err(error) => self.workspaces.message = Some(error),
        }
    }

    pub fn confirm_rebind(&mut self) {
        let Some(confirmation) = self.rebind.confirmation.take() else {
            return;
        };
        let Ok(current) = self.rebind_confirmation() else {
            return;
        };
        if current.origin_revision != confirmation.origin_revision
            || current.epoch != confirmation.epoch
            || current.workspace.marker != confirmation.workspace.marker
            || current.prior != confirmation.prior
            || current.interface != confirmation.interface
        {
            self.workspaces.message =
                Some("The workspace or interface changed. Review the IP change again.".into());
            return;
        }
        self.record(Intent::RebindWorkspace {
            name: confirmation.workspace.name.clone(),
            ip: confirmation.interface.ip.to_string(),
        });
        self.invalidate_origin();
        if self.fixture {
            return;
        }
        let (Some(services), Some(client), Some(container)) =
            (&self.services, &self.client, &self.workspaces.container)
        else {
            return;
        };
        self.workspaces.busy = true;
        self.workspaces.revision = self.workspaces.revision.wrapping_add(1);
        self.rebind.request = services.rebind_workspace(
            client.clone(),
            container.clone(),
            RebindOperation::New(Box::new(confirmation)),
            self.connection.origin.text.clone(),
            self.connection.deployment_label().to_owned(),
            self.connection.epoch,
            self.workspaces.revision,
        );
    }

    pub fn reconcile_rebind(&mut self, request_id: &str) {
        if self.workspaces.busy || !self.permission(Permission::RebindWorkstation) {
            return;
        }
        self.record(Intent::ReconcileRebind {
            request_id: request_id.to_owned(),
        });
        self.invalidate_origin();
        if self.fixture {
            return;
        }
        let (Some(services), Some(client), Some(container)) =
            (&self.services, &self.client, &self.workspaces.container)
        else {
            return;
        };
        self.workspaces.busy = true;
        self.workspaces.revision = self.workspaces.revision.wrapping_add(1);
        self.rebind.request = services.rebind_workspace(
            client.clone(),
            container.clone(),
            RebindOperation::Retry(request_id.to_owned()),
            self.connection.origin.text.clone(),
            self.connection.deployment_label().to_owned(),
            self.connection.epoch,
            self.workspaces.revision,
        );
    }
}

enum RebindOperation {
    New(Box<RebindConfirmation>),
    Retry(String),
}

impl GatewayEffects {
    #[allow(clippy::too_many_arguments)]
    fn rebind_workspace(
        &self,
        client: Client,
        container: Arc<NativeContainer>,
        operation: RebindOperation,
        origin: String,
        deployment: String,
        epoch: u64,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let prepared = tokio::task::spawn_blocking({ let container = container.clone(); let deployment = deployment.clone();
                move || -> Result<RebindAttempt> {
                    if let RebindOperation::Retry(request_id) = operation {
                        return container.pending_rebind_attempts()?.into_iter().find(|attempt| attempt.request.request_id == request_id
                            && attempt.prior.deployment_id == deployment).context("Reconnect to the recorded backend before retrying this IP change");
                    }
                    let RebindOperation::New(confirmation) = operation else { unreachable!("retry returned above") };
                    anyhow::ensure!(confirmation.prior.deployment_id == deployment, "backend changed before IP confirmation");
                    let interfaces = platform::interfaces(&origin)?;
                    anyhow::ensure!(interfaces.interfaces.contains(&confirmation.interface), "the confirmed interface is no longer available");
                    let workspace = container.select(Some(confirmation.workspace.name.as_ref()))?;
                    anyhow::ensure!(workspace.marker() == confirmation.workspace.marker, "workspace identity changed before IP confirmation");
                    let request = RegistrationEnsureRequest::new(platform::request_id()?, confirmation.interface.ip,
                        workspace.name().clone(), container.installation_marker().to_owned(), workspace.marker().to_owned());
                    container.begin_rebind_attempt(&workspace, &confirmation.prior, request)
                }
            }).await.map_err(|_| "IP change preparation worker stopped".to_owned())
                .and_then(|result| result.map_err(|error| error.to_string()));
            let result = async {
                let attempt = prepared?;
                let receipt: RegistrationReceipt = client.post("/api/v1/connect/registrations/rebind", &RebindRequest {
                    request_id: &attempt.request.request_id, registration_id: &attempt.prior.registration_id,
                    expected_revision: attempt.prior.revision, workstation_ip: attempt.request.workstation_ip,
                    workspace_name: &attempt.request.workspace_name, installation_marker: &attempt.request.installation_marker,
                    workspace_marker: &attempt.request.workspace_marker,
                }).await.map_err(|error| error.to_string())?;
                let container = container.clone();
                tokio::task::spawn_blocking(move || -> Result<WorkspaceUpdate> {
                    container.complete_rebind_attempt(&attempt.request.request_id, &receipt)?;
                    Ok(WorkspaceUpdate { snapshot: workspaces::snapshot(&container)?, container, selected: None,
                        message: Some(format!("IP changed. New Connect ID: {}. Refresh other workspace registrations before using them; register this workspace to resume requests.", receipt.connect_id)) })
                }).await.map_err(|_| "IP change journal worker stopped".to_owned())?
                    .map_err(|error| error.to_string())
            }.await;
            let result = match result {
                Ok(update) => Ok(update),
                Err(error) => crate::workspace_lifecycle::refreshed_error(container, error).await,
            };
            ui.update(move |state, _| {
                if state.connection.epoch == epoch {
                    state.rebind.request = None;
                    state.complete_workspaces(revision, result);
                } else if state.workspaces.revision == revision {
                    state.workspaces.busy = false; state.rebind.request = None;
                }
            });
        });
        Some(RequestTask(task.abort_handle()))
    }
}
