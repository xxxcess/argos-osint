//! Stable installation-backed workstation registration and authority fencing.

use std::sync::Arc;

use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::connect::RegistrationStatus;
use fleetworks_contracts::connect::REGISTRATION_ENSURE_PATH;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::Client;
use fleetworks_workstation::native::NativeContainer;
use rune::Tx;
use serde::Deserialize;
use serde::Serialize;

use crate::app::{Intent, Load, State};
use crate::connection::{GatewayEffects, RequestTask};
use crate::platform;
pub use crate::platform::{InterfaceSnapshot, NativeInterface};

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
    pub fn reconcile_registration_attempt(&mut self, _request_id: &str) {
        // The protected workstation journal reuses its unresolved request automatically.
        self.ensure_registration();
    }

    pub(crate) fn invalidate_origin(&mut self) {
        self.origin_revision = self.origin_revision.wrapping_add(1);
        self.reveal = None;
        self.tool_key_dialog = false;
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
        // Interface addresses are optional mutable metadata, never identity or authority.
    }

    pub fn choose_interface(&mut self, index: usize) {
        if let Load::Ready(snapshot) = &self.registration.interfaces {
            self.registration.selected_interface = snapshot.interfaces.get(index).cloned();
        }
    }

    pub fn can_register(&self) -> bool {
        self.permission(Permission::RegisterWorkstation)
            && (self.fixture || self.workspaces.container.is_some())
            && !matches!(self.registration.receipt, Load::Loading)
    }

    pub fn ensure_registration(&mut self) {
        if !self.can_register() {
            return;
        }
        self.record(Intent::Register);
        if self.fixture {
            return;
        }
        let (Some(services), Some(client), Some(container)) =
            (&self.services, &self.client, &self.workspaces.container)
        else {
            return;
        };
        self.registration.receipt = Load::Loading;
        self.registration.message = None;
        self.registration.request = services.ensure_workstation(
            client.clone(),
            container.clone(),
            self.connection.origin.text.clone(),
            self.connection.deployment_label().to_owned(),
            self.connection
                .capabilities
                .as_ref()
                .and_then(|value| value.configuration_epoch.clone()),
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
                self.registration.message = Some("Workstation registration verified".into());
                self.restore_cycle_deletion();
                self.refresh_readiness();
                self.refresh_page();
            }
            Err(error) => {
                self.registration.context = None;
                self.registration.receipt = Load::Failed(error);
            }
        }
    }

    pub fn open_intel_web(&mut self, tx: &mut Tx) {
        if self.registration.context.is_none() {
            return;
        }
        let Some(capabilities) = &self.connection.capabilities else {
            return;
        };
        let result = capabilities
            .links
            .intel_web
            .as_ref()
            .ok_or_else(|| "Gateway has not advertised Intel Web".to_owned())
            .and_then(|links| {
                links
                    .validated_connect_url(&self.connection.origin.text)
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok(url) if self.fixture => {
                let _ = url;
                self.record(Intent::OpenIntel);
            }
            Ok(url) => {
                if let Err(error) = tx.open_url(url.as_str()) {
                    self.registration.message = Some(error.to_string());
                }
            }
            Err(error) => self.registration.message = Some(error),
        }
    }
}

impl GatewayEffects {
    #[allow(clippy::too_many_arguments)]
    fn ensure_workstation(
        &self,
        client: Client,
        container: Arc<NativeContainer>,
        origin: String,
        deployment: String,
        configuration_epoch: Option<String>,
        epoch: u64,
        origin_revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let prepared = tokio::task::spawn_blocking({
                let container = container.clone();
                let deployment = deployment.clone();
                move || -> anyhow::Result<RegistrationEnsureRequest> {
                    let ip = platform::interfaces(&origin)
                        .ok()
                        .and_then(|snapshot| snapshot.route_suggestion);
                    let request = RegistrationEnsureRequest::new(
                        platform::request_id()?,
                        ip,
                        container.installation_marker().to_owned(),
                    );
                    container.begin_workstation_registration(&deployment, request)
                }
            })
            .await
            .map_err(|_| "Workstation journal worker stopped".to_owned())
            .and_then(|result| result.map_err(|error| error.to_string()));
            let request_id = prepared
                .as_ref()
                .ok()
                .map(|request| request.request_id.clone());
            let result = async {
                let request = prepared?;
                let receipt: RegistrationReceipt = client
                    .post(REGISTRATION_ENSURE_PATH, &request)
                    .await
                    .map_err(|error| error.to_string())?;
                if !receipt.is_consistent()
                    || receipt.status != RegistrationStatus::Active
                    || receipt.deployment_id != deployment
                {
                    return Err(
                        "Gateway returned an inconsistent or inactive workstation registration"
                            .into(),
                    );
                }
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
                    || context.context_revision == 0
                    || context.scope_revision == 0
                    || context.access_epoch.is_empty()
                    || configuration_epoch.as_deref() != Some(context.configuration_epoch.as_str())
                {
                    return Err("Gateway context changed; reconnect before submitting work".into());
                }
                tokio::task::spawn_blocking(move || {
                    container
                        .record_workstation_registration(&request, &receipt.clone())
                        .map(|_| receipt)
                })
                .await
                .map_err(|_| "Workstation journal worker stopped".to_owned())?
                .map(|receipt| Registered { receipt, context })
                .map_err(|error| error.to_string())
            }
            .await;
            ui.update(move |state, _| {
                if state.connection.epoch == epoch && state.origin_revision == origin_revision {
                    state.complete_registration(request_id, result);
                }
            });
        });
        Some(RequestTask(task.abort_handle()))
    }
}
