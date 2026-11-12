//! Explicit local-interface choice and authoritative registered request context.

use std::net::IpAddr;
use std::sync::Arc;

use anyhow::Result;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::connect::RegistrationStatus;
use fleetworks_contracts::connect::REGISTRATION_ENSURE_PATH;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::Client;
use fleetworks_workspace::native::NativeContainer;
use fleetworks_workspace::native::WorkspaceState;
use rune::Tx;
use serde::Deserialize;
use serde::Serialize;

use crate::app::Intent;
use crate::app::Load;
use crate::app::State;
use crate::connection::GatewayEffects;
use crate::connection::RequestTask;
use crate::platform;
pub use crate::platform::InterfaceSnapshot;
pub use crate::platform::NativeInterface;
use crate::workspaces;
use crate::workspaces::WorkspaceCommand;
use crate::workspaces::WorkspaceRow;
use crate::workspaces::WorkspaceUpdate;

#[derive(Default)]
pub struct Registration {
    pub interfaces: Load<InterfaceSnapshot>,
    pub selected_interface: Option<NativeInterface>,
    pub receipt: Load<RegistrationReceipt>,
    pub context: Option<RegisteredContext>,
    pub message: Option<String>,
    pub(crate) request_id: Option<String>,
    pub(crate) request: Option<RequestTask>,
    pub(crate) interface_request: Option<RequestTask>,
    pub(crate) interface_revision: u64,
}

#[derive(Serialize)]
struct ContextRequest<'a> {
    registration_id: &'a str,
    expected_revision: u64,
}

#[derive(Deserialize)]
struct ContextResponse {
    context: RegisteredContext,
}

pub(crate) struct Registered {
    pub receipt: RegistrationReceipt,
    pub context: RegisteredContext,
}

impl State {
    /// Replay an unknown outcome with its original durable request. This only
    /// records the authoritative binding; it never selects a stale IP as origin.
    pub fn reconcile_registration_attempt(&mut self, request_id: &str) {
        if self.workspaces.busy || !self.permission(Permission::RegisterWorkstation) {
            return;
        }
        self.record(Intent::ReconcileRegistration {
            request_id: request_id.to_owned(),
        });
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
        self.workspaces.request = services.reconcile_registration_attempt(
            client.clone(),
            container.clone(),
            request_id.to_owned(),
            self.connection.deployment_label().to_owned(),
            self.connection.epoch,
            self.workspaces.revision,
        );
    }
    /// Invalidate only future request attribution, never backend-wide drafts/data.
    pub(crate) fn invalidate_origin(&mut self) {
        self.origin_revision = self.origin_revision.wrapping_add(1);
        self.reveal = None;
        self.registration.request = None;
        self.registration.receipt = Load::Idle;
        self.registration.context = None;
        self.registration.request_id = None;
        self.registration.message = None;
        self.workspaces.confirmation = None;
        self.rebind.confirmation = None;
        self.configuration.confirmation = None;
        self.transfer.confirmation = None;
        self.submissions.invalidate_origin();
        self.job_actions.invalidate_origin();
        self.classifier_activation.invalidate_origin();
        self.scoped_logs.invalidate_origin();
        self.scoped_jobs.invalidate_origin();
        self.backend.invalidate_activity();
        self.maintenance.invalidate_origin();
        self.readiness_revision = self.readiness_revision.wrapping_add(1);
        if let Load::Ready(readiness) = &mut self.readiness {
            readiness.registration = None;
            for check in &mut readiness.checks {
                for action in &mut check.actions {
                    action.allowed = false;
                    action.disabled_reason = Some("registered_origin_required".into());
                }
            }
        }
    }

    pub fn permission(&self, permission: Permission) -> bool {
        self.connected()
            && self
                .connection
                .capabilities
                .as_ref()
                .is_some_and(|capabilities| capabilities.permissions.contains(&permission))
    }

    pub fn refresh_interfaces(&mut self) {
        self.record(Intent::Interfaces);
        if self.fixture {
            return;
        }
        self.registration.interfaces = Load::Loading;
        self.registration.interface_request = None;
        self.registration.interface_revision = self.registration.interface_revision.wrapping_add(1);
        if let Some(services) = &self.services {
            self.registration.interface_request = services.interfaces(
                self.connection.origin.text.clone(),
                self.connection.epoch,
                self.registration.interface_revision,
            );
        }
    }

    pub fn choose_interface(&mut self, index: usize) {
        let Load::Ready(snapshot) = &self.registration.interfaces else {
            return;
        };
        let Some(choice) = snapshot.interfaces.get(index).cloned() else {
            return;
        };
        if self.registration.selected_interface.as_ref() != Some(&choice) {
            self.invalidate_origin();
            self.registration.selected_interface = Some(choice);
        }
    }

    pub fn can_register(&self) -> bool {
        self.permission(Permission::RegisterWorkstation)
            && !matches!(&self.workspaces.snapshot, Load::Ready(snapshot) if snapshot.rebind_attempts.iter()
                .any(|attempt| attempt.prior.deployment_id == self.connection.deployment_label()))
            && self.workspaces.selected.as_ref().is_some_and(|workspace|
                matches!(workspace.state, WorkspaceState::Local | WorkspaceState::Registered))
            && self.registration.selected_interface.as_ref().is_some_and(|selected|
                matches!(&self.registration.interfaces, Load::Ready(snapshot) if snapshot.interfaces.contains(selected)))
            && !matches!(self.registration.receipt, Load::Loading)
            && !self.workspaces.busy
    }

    pub fn ensure_registration(&mut self) {
        if !self.can_register() {
            self.registration.message = Some("Select an owned workspace and a current LAN/VPN interface, then connect with registration permission".into());
            return;
        }
        self.record(Intent::Register);
        if self.fixture {
            return;
        }
        let (Some(services), Some(client), Some(container), Some(workspace), Some(interface)) = (
            &self.services,
            &self.client,
            &self.workspaces.container,
            &self.workspaces.selected,
            &self.registration.selected_interface,
        ) else {
            return;
        };
        self.registration.receipt = Load::Loading;
        self.registration.message = None;
        self.registration.request = services.ensure_registration(
            client.clone(),
            container.clone(),
            workspace.clone(),
            interface.clone(),
            self.connection.origin.text.clone(),
            self.connection.deployment_label().to_owned(),
            self.connection
                .capabilities
                .as_ref()
                .and_then(|capabilities| capabilities.configuration_epoch.clone()),
            self.registration.request_id.clone(),
            self.connection.epoch,
            self.origin_revision,
        );
    }

    pub(crate) fn complete_registration(
        &mut self,
        request_id: Option<String>,
        result: Result<Registered, String>,
    ) {
        self.registration.request = None;
        self.registration.request_id = request_id;
        match result {
            Ok(registered) => {
                self.registration.context = Some(registered.context);
                self.registration.receipt = Load::Ready(registered.receipt);
                self.registration.message =
                    Some("Registered context verified. Copy Connect ID or open Intel Web.".into());
            }
            Err(error) => {
                self.registration.context = None;
                self.registration.receipt = Load::Failed(error);
            }
        }
        if !self.fixture && !self.workspaces.busy {
            self.workspace_command(WorkspaceCommand::Refresh);
        }
    }

    pub fn copy_connect_id(&mut self, tx: &mut Tx) {
        let Some(context) = &self.registration.context else {
            return;
        };
        if self.fixture {
            self.record(Intent::CopyConnectId);
        } else {
            tx.copy(context.connect_id.to_string());
        }
        self.registration.message = Some("Copied Connect ID".into());
    }

    pub fn open_intel_web(&mut self, tx: &mut Tx) {
        if self.registration.context.is_none() {
            return;
        }
        let Some(capabilities) = &self.connection.capabilities else {
            return;
        };
        match capabilities
            .links
            .intel_web
            .as_ref()
            .ok_or_else(|| "Gateway has not advertised Intel Web".to_owned())
            .and_then(|links| {
                links
                    .validated_connect_url(&self.connection.origin.text)
                    .map_err(|error| error.to_string())
            }) {
            Ok(url) => {
                if self.fixture {
                    self.record(Intent::OpenIntel);
                } else if let Err(error) = tx.open_url(url.as_str()) {
                    self.registration.message = Some(error.to_string());
                }
            }
            Err(error) => self.registration.message = Some(error),
        }
    }
}

impl GatewayEffects {
    fn reconcile_registration_attempt(
        &self,
        client: Client,
        container: Arc<NativeContainer>,
        request_id: String,
        deployment: String,
        epoch: u64,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let request = tokio::task::spawn_blocking({ let container = container.clone(); let deployment = deployment.clone(); move || -> Result<RegistrationEnsureRequest> {
                container.pending_registration_attempts()?.into_iter().find(|attempt|
                    attempt.deployment_id == deployment && attempt.request.request_id == request_id)
                    .map(|attempt| attempt.request).ok_or_else(|| anyhow::anyhow!("Reconnect to the attempt's recorded deployment before reconciliation"))
            }}).await.map_err(|_| "Registration reconciliation worker stopped".to_owned())
                .and_then(|result| result.map_err(|error| error.to_string()));
            let result = async {
                let request = request?;
                reconcile_receipt(&client, &container, &request, &deployment).await?;
                tokio::task::spawn_blocking(move || -> Result<WorkspaceUpdate> {
                    Ok(WorkspaceUpdate { snapshot: workspaces::snapshot(&container)?, container, selected: None,
                        message: Some("Registration outcome reconciled and binding recorded. Select the current interface before using a request origin.".into()) })
                }).await.map_err(|_| "Workspace refresh worker stopped".to_owned())?
                    .map_err(|error| error.to_string())
            }.await;
            ui.update(move |state, _| {
                if state.connection.epoch == epoch { state.complete_workspaces(revision, result); }
                else if state.workspaces.revision == revision { state.workspaces.busy = false; state.workspaces.request = None; }
            });
        });
        Some(RequestTask(task.abort_handle()))
    }
    fn interfaces(&self, origin: String, epoch: u64, revision: u64) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || platform::interfaces(&origin))
                .await
                .map_err(|_| "Interface worker stopped".to_owned())
                .and_then(|result| result.map_err(|error| error.to_string()));
            ui.update(move |state, _| {
                if state.connection.epoch != epoch
                    || state.registration.interface_revision != revision
                {
                    return;
                }
                state.registration.interface_request = None;
                if let (Ok(snapshot), Some(selected)) =
                    (&result, &state.registration.selected_interface)
                {
                    if !snapshot.interfaces.contains(selected) {
                        state.invalidate_origin();
                        state.registration.selected_interface = None;
                    }
                }
                state.registration.interfaces = match result {
                    Ok(snapshot) => Load::Ready(snapshot),
                    Err(error) => Load::Failed(error),
                };
            });
        });
        Some(RequestTask(task.abort_handle()))
    }

    #[allow(clippy::too_many_arguments)]
    fn ensure_registration(
        &self,
        client: Client,
        container: Arc<NativeContainer>,
        workspace: WorkspaceRow,
        interface: NativeInterface,
        origin: String,
        deployment: String,
        configuration_epoch: Option<String>,
        previous_request_id: Option<String>,
        epoch: u64,
        origin_revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let preparation = tokio::task::spawn_blocking({
                let container = container.clone();
                let workspace = workspace.clone();
                let deployment = deployment.clone();
                move || -> Result<RegistrationEnsureRequest> {
                    let current = container.select(Some(workspace.name.as_ref()))?;
                    anyhow::ensure!(
                        current.marker() == workspace.marker,
                        "Workspace identity changed; refresh before registration"
                    );
                    let interfaces = platform::interfaces(&origin)?;
                    anyhow::ensure!(
                        interfaces.interfaces.contains(&interface),
                        "The selected interface is no longer available"
                    );
                    if let Some(pending) = container.pending_registration_attempts()?.into_iter().find(|attempt|
                        attempt.deployment_id == deployment && attempt.request.workspace_marker == current.marker()) {
                        anyhow::ensure!(pending.request.workstation_ip == interface.ip,
                            "Reconcile the earlier registration attempt in Workspaces before changing its IP");
                        return Ok(pending.request);
                    }
                    let request = RegistrationEnsureRequest::new(
                        previous_request_id.map_or_else(platform::request_id, Ok)?,
                        interface.ip,
                        current.name().clone(),
                        container.installation_marker().to_owned(),
                        current.marker().to_owned(),
                    );
                    container.begin_registration_attempt(&current, &deployment, request.clone())?;
                    Ok(request)
                }
            })
            .await
            .map_err(|_| "Registration worker stopped".to_owned())
            .and_then(|result| result.map_err(|error| error.to_string()));
            let request_id = preparation
                .as_ref()
                .ok()
                .map(|request| request.request_id.clone());
            let result = match preparation {
                Ok(request) => {
                    register(
                        &client,
                        &container,
                        &request,
                        &deployment,
                        configuration_epoch.as_deref(),
                    )
                    .await
                }
                Err(error) => Err(error),
            };
            ui.update(move |state, _| {
                if state.connection.epoch == epoch && state.origin_revision == origin_revision {
                    state.complete_registration(request_id, result);
                }
            });
        });
        Some(RequestTask(task.abort_handle()))
    }
}

async fn register(
    client: &Client,
    container: &Arc<NativeContainer>,
    request: &RegistrationEnsureRequest,
    deployment: &str,
    configuration_epoch: Option<&str>,
) -> Result<Registered, String> {
    let receipt = reconcile_receipt(client, container, request, deployment).await?;
    let response: ContextResponse = client
        .post(
            "/api/v1/connect/registrations/context",
            &ContextRequest {
                registration_id: &receipt.registration_id,
                expected_revision: receipt.revision,
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    let context = response.context;
    if context.registration_id != receipt.registration_id
        || context.registration_revision != receipt.revision
        || context.connect_id != receipt.connect_id
        || context.deployment_id != receipt.deployment_id
        || context.workstation_id != receipt.workstation_id
        || context.workspace_id != receipt.workspace_id
        || context.workstation_ip != receipt.workstation_ip
        || context.workspace_name != receipt.workspace_name
        || context.context_revision == 0
        || context.scope_revision == 0
        || context.access_epoch.is_empty()
        || configuration_epoch != Some(context.configuration_epoch.as_str())
    {
        return Err("Gateway context changed; reconnect and register again".into());
    }
    Ok(Registered { receipt, context })
}

async fn reconcile_receipt(
    client: &Client,
    container: &Arc<NativeContainer>,
    request: &RegistrationEnsureRequest,
    deployment: &str,
) -> Result<RegistrationReceipt, String> {
    let receipt: RegistrationReceipt = client
        .post(REGISTRATION_ENSURE_PATH, request)
        .await
        .map_err(|error| error.to_string())?;
    if !valid_receipt(
        &receipt,
        request.workstation_ip,
        request.workspace_name.as_str(),
        deployment,
    ) {
        return Err("Gateway returned an inconsistent or inactive registration".into());
    }
    let persisted_receipt = receipt.clone();
    let name = request.workspace_name.as_str().to_owned();
    let marker = request.workspace_marker.clone();
    let container = container.clone();
    tokio::task::spawn_blocking(move || {
        workspaces::persist_registration(&container, &name, &marker, &persisted_receipt)
    })
    .await
    .map_err(|_| "Registration persistence worker stopped".to_owned())?
    .map_err(|error| error.to_string())?;
    Ok(receipt)
}

fn valid_receipt(receipt: &RegistrationReceipt, ip: IpAddr, name: &str, deployment: &str) -> bool {
    receipt.is_consistent()
        && receipt.status == RegistrationStatus::Active
        && receipt.workstation_ip == ip
        && receipt.workspace_name.as_str() == name
        && receipt.deployment_id == deployment
}
