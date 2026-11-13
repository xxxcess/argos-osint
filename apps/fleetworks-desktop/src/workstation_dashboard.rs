//! Bounded, authority-fenced cycle history and optional gateway host metrics.

use fleetworks_contracts::context::JobRequestAssociation;
use fleetworks_contracts::context::MutationContext;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::cycle_history::{CycleHistory, DeleteCycleReceipt, DeleteCycleRequest};
use rune::TextEdit;
use serde::Deserialize;

use crate::app::{Load, State};
use crate::connection::RequestTask;
use crate::platform;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct HostMetrics {
    pub cpu_percent: Option<f64>,
    pub memory_percent: Option<f64>,
    pub disk_percent: Option<f64>,
    pub network_kbps: Option<f64>,
    pub platform: Option<String>,
    pub chipset: Option<String>,
}

#[derive(Default)]
pub struct Cycles {
    pub history: Load<CycleHistory>,
    pub selected: Option<String>,
    pub confirmation: Option<String>,
    confirmation_configuration_revision: Option<u64>,
    pub message: Option<String>,
    pub deletion: Load<DeleteCycleReceipt>,
    retained_delete: Option<DeleteCycleRequest>,
    retained_context: Option<RegisteredContext>,
    read_request: Option<RequestTask>,
    journal_request: Option<RequestTask>,
    delete_request: Option<RequestTask>,
    revision: u64,
}

impl Cycles {
    pub(crate) fn busy(&self) -> bool {
        self.read_request.is_some()
            || self.journal_request.is_some()
            || self.delete_request.is_some()
            || matches!(self.history, Load::Loading)
            || matches!(self.deletion, Load::Loading)
    }

    pub(crate) fn invalidate(&mut self) {
        self.read_request = None;
        self.journal_request = None;
        self.delete_request = None;
        self.revision = self.revision.wrapping_add(1);
        self.history = Load::Idle;
        self.confirmation = None;
        self.confirmation_configuration_revision = None;
        self.selected = None;
        // Preserve exact uncertain input. A different registration cannot resubmit it.
        if self.retained_delete.is_some() {
            self.deletion = Load::Failed(
                "Deletion outcome remains unresolved under its original registration".into(),
            );
        }
    }
}

impl State {
    pub fn select_cycle(&mut self, job_id: &str) {
        if matches!(&self.cycles.history, Load::Ready(history) if history.rows.iter().any(|row| row.job_id == job_id))
        {
            self.cycles.selected = Some(job_id.to_owned());
            self.cycles.confirmation = None;
        }
    }

    pub(crate) fn restore_cycle_deletion(&mut self) {
        if self.cycles.retained_delete.is_some() || self.cycles.journal_request.is_some() {
            return;
        }
        let (Some(services), Some(container), Some(context)) = (
            &self.services,
            &self.workspaces.container,
            self.registration.context.clone(),
        ) else {
            return;
        };
        let Some(ui) = services.ui.get().cloned() else {
            return;
        };
        let container = container.clone();
        let epoch = self.connection.epoch;
        let origin_revision = self.origin_revision;
        let task = services.runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || container.pending_cycle_deletions()).await;
            ui.update(move |state, _| {
                if state.connection.epoch != epoch || state.origin_revision != origin_revision { return; }
                state.cycles.journal_request = None;
                match result {
                    Ok(Ok(pending)) => {
                        if let Some(pending) = pending.into_iter().find(|pending| pending.context.deployment_id == context.deployment_id && pending.context.workstation_id == context.workstation_id) {
                            state.cycles.retained_delete = Some(pending.request);
                            state.cycles.retained_context = Some(pending.context);
                            state.cycles.deletion = Load::Failed("Deletion outcome unresolved; use read-only recovery".into());
                        }
                    }
                    _ => state.cycles.message = Some("Could not read protected deletion journal; resolve local settings before deleting history".into()),
                }
            });
        });
        self.cycles.journal_request = Some(RequestTask(task.abort_handle()));
    }

    pub fn refresh_cycles(&mut self) {
        let Some(context) = self.registration.context.clone() else {
            return;
        };
        if self.cycles.read_request.is_some() {
            return;
        }
        let (Some(services), Some(client)) = (&self.services, &self.client) else {
            return;
        };
        let Ok(client) = client.with_registered_context(&context) else {
            return;
        };
        let Some(ui) = services.ui.get().cloned() else {
            return;
        };
        self.cycles.revision = self.cycles.revision.wrapping_add(1);
        let revision = self.cycles.revision;
        let epoch = self.connection.epoch;
        let origin = self.origin_revision;
        self.cycles.history = Load::Loading;
        let task = services.runtime.spawn(async move {
            let result = client
                .cycle_history(40)
                .await
                .map_err(|error| error.to_string());
            ui.update(move |state, _| {
                if state.connection.epoch != epoch
                    || state.origin_revision != origin
                    || state.cycles.revision != revision
                {
                    return;
                }
                state.cycles.read_request = None;
                state.cycles.history = match result {
                    Ok(history) if history.rows.len() <= 40 => Load::Ready(history),
                    Ok(_) => {
                        Load::Failed("Gateway cycle history exceeded its requested bound".into())
                    }
                    Err(error) => Load::Failed(error),
                };
            });
        });
        self.cycles.read_request = Some(RequestTask(task.abort_handle()));
    }

    pub fn cycle_deletion_disabled_reason(&self) -> Option<String> {
        if !self.permission(fleetworks_contracts::context::Permission::SubmitInteractive) {
            return Some("Interactive submission permission is required".into());
        }
        if self.registration.context.is_none() {
            return Some("Register this workstation before deleting cycle history".into());
        }
        if self.configuration_revision().is_none() {
            return Some(
                "Backend configuration is unavailable; check the gateway connection".into(),
            );
        }
        if matches!(self.cycles.deletion, Load::Loading) {
            return Some("The current deletion is awaiting its outcome".into());
        }
        None
    }

    fn prepared_cycle_deletion(&self, request_id: String) -> Result<DeleteCycleRequest, String> {
        if let Some(reason) = self.cycle_deletion_disabled_reason() {
            return Err(reason);
        }
        let context = self
            .registration
            .context
            .as_ref()
            .ok_or("Workstation registration unavailable")?;
        let configuration_revision = self
            .cycles
            .confirmation_configuration_revision
            .ok_or("Review the deletion against current backend configuration")?;
        if self.configuration_revision() != Some(configuration_revision) {
            return Err("Backend configuration changed; review the deletion again".into());
        }
        let id = self
            .cycles
            .confirmation
            .as_ref()
            .ok_or("Review the selected cycle before deleting history")?;
        let Load::Ready(history) = &self.cycles.history else {
            return Err("Refresh cycle history before deleting".into());
        };
        let row = history
            .rows
            .iter()
            .find(|row| &row.job_id == id)
            .ok_or("The selected cycle is no longer in current history")?;
        Ok(DeleteCycleRequest::new(
            request_id,
            MutationContext::new(context).with_configuration_revision(configuration_revision),
            id.clone(),
            row.revision,
        ))
    }

    pub fn preview_delete_cycle(&mut self) {
        if let Some(reason) = self.cycle_deletion_disabled_reason() {
            self.cycles.message = Some(reason);
            return;
        }
        if self.cycles.retained_delete.is_some() {
            self.cycles.message =
                Some("Resolve the earlier deletion before deleting another cycle".into());
            return;
        }
        if let (Some(id), Load::Ready(history)) = (&self.cycles.selected, &self.cycles.history) {
            if history.rows.iter().any(|row| &row.job_id == id) {
                self.cycles.confirmation = Some(id.clone());
                self.cycles.confirmation_configuration_revision = self.configuration_revision();
            }
        }
    }

    pub fn confirm_delete_cycle(&mut self) {
        if self.cycles.delete_request.is_some() {
            return;
        }
        if self.cycles.retained_delete.is_some() {
            self.resolve_cycle_deletion();
            return;
        }
        let Some(context) = self.registration.context.clone() else {
            return;
        };
        let request_id = match platform::request_id() {
            Ok(value) => value,
            Err(error) => {
                self.cycles.message = Some(error.to_string());
                return;
            }
        };
        let request = self.prepared_cycle_deletion(request_id);
        self.cycles.confirmation = None;
        self.cycles.confirmation_configuration_revision = None;
        let request = match request {
            Ok(request) => request,
            Err(reason) => {
                self.cycles.message = Some(reason);
                return;
            }
        };
        let (Some(services), Some(client)) = (&self.services, &self.client) else {
            return;
        };
        let Ok(client) = client.with_registered_context(&context) else {
            return;
        };
        let Some(ui) = services.ui.get().cloned() else {
            return;
        };
        let epoch = self.connection.epoch;
        let origin = self.origin_revision;
        self.cycles.retained_delete = Some(request.clone());
        self.cycles.retained_context = Some(context.clone());
        let Some(container) = self.workspaces.container.clone() else {
            return;
        };
        self.cycles.deletion = Load::Loading;
        let task = services.runtime.spawn(async move {
            let prepared = tokio::task::spawn_blocking({
                let container = container.clone();
                let request = request.clone();
                move || container.begin_cycle_deletion(&context, &request)
            })
            .await
            .map_err(|_| "Deletion journal worker stopped".to_owned())
            .and_then(|result| result.map_err(|error| error.to_string()));
            let result = async {
                prepared?;
                let receipt = client
                    .delete_cycle(&request)
                    .await
                    .map_err(|error| error.to_string())?;
                if receipt.job_id == request.job_id
                    && receipt.hidden
                    && receipt.revision > request.expected_revision
                {
                    let saved_request = request.clone();
                    tokio::task::spawn_blocking(move || {
                        container.complete_cycle_deletion(&saved_request)
                    })
                    .await
                    .map_err(|_| "Deletion journal worker stopped".to_owned())?
                    .map_err(|error| error.to_string())?;
                }
                Ok::<_, String>(receipt)
            }
            .await;
            ui.update(move |state, _| {
                if state.connection.epoch != epoch || state.origin_revision != origin {
                    return;
                }
                state.cycles.delete_request = None;
                match result {
                    Ok(receipt)
                        if receipt.job_id == request.job_id
                            && receipt.hidden
                            && receipt.revision > request.expected_revision =>
                    {
                        state.cycles.retained_delete = None;
                        state.cycles.retained_context = None;
                        state.cycles.deletion = Load::Ready(receipt);
                        state.refresh_cycles();
                    }
                    Ok(_) => state.cycles.deletion = Load::Failed(
                        "Deletion receipt did not match the original request; outcome unresolved"
                            .into(),
                    ),
                    Err(error) => state.cycles.deletion = Load::Failed(error),
                }
            });
        });
        self.cycles.delete_request = Some(RequestTask(task.abort_handle()));
    }

    fn resolve_cycle_deletion(&mut self) {
        let (Some(request), Some(original), Some(context)) = (
            self.cycles.retained_delete.clone(),
            self.cycles.retained_context.clone(),
            self.registration.context.clone(),
        ) else {
            return;
        };
        if original.deployment_id != context.deployment_id
            || original.workstation_id != context.workstation_id
        {
            self.cycles.message =
                Some("Reconnect to the original workstation to resolve this deletion".into());
            return;
        }
        let (Some(services), Some(client)) = (&self.services, &self.client) else {
            return;
        };
        let Ok(client) = client.with_registered_context(&context) else {
            return;
        };
        let Some(ui) = services.ui.get().cloned() else {
            return;
        };
        let Some(container) = self.workspaces.container.clone() else {
            return;
        };
        let epoch = self.connection.epoch;
        let origin_revision = self.origin_revision;
        self.cycles.deletion = Load::Loading;
        let task = services.runtime.spawn(async move {
            #[derive(serde::Serialize)]
            struct Lookup<'a> { request_id: &'a str, operation: &'a str, original_registration_id: &'a str }
            let result: Result<JobRequestAssociation, _> = client.get_with_query(
                "/api/v1/workstation/requests/detail", &Lookup {
                    request_id: &request.request_id, operation: "atlas.cycle_history.delete",
                    original_registration_id: &original.registration_id,
                }).await;
            if result.as_ref().is_ok_and(|association| association.job_id == request.job_id
                && association.origin.request_id == request.request_id
                && association.origin.registration_id == original.registration_id
                && association.origin.workstation_id == original.workstation_id
                && association.origin.deployment_id == original.deployment_id) {
                let saved_request = request.clone();
                let _ = tokio::task::spawn_blocking(move || container.complete_cycle_deletion(&saved_request)).await;
            }
            ui.update(move |state, _| {
                if state.connection.epoch != epoch || state.origin_revision != origin_revision { return; }
                state.cycles.delete_request = None;
                match result {
                    Ok(association) if association.job_id == request.job_id
                        && association.origin.request_id == request.request_id
                        && association.origin.registration_id == original.registration_id
                        && association.origin.workstation_id == original.workstation_id
                        && association.origin.deployment_id == original.deployment_id => {
                        state.cycles.retained_delete = None;
                        state.cycles.retained_context = None;
                        state.cycles.deletion = Load::Idle;
                        state.cycles.message = Some("Server verified the original deletion request; refreshing cycle history".into());
                        state.refresh_cycles();
                    }
                    Ok(_) => state.cycles.deletion = Load::Failed("Request lookup returned a different association; deletion remains unresolved".into()),
                    Err(error) => state.cycles.deletion = Load::Failed(format!("Deletion outcome remains unresolved: {error}")),
                }
            });
        });
        self.cycles.delete_request = Some(RequestTask(task.abort_handle()));
    }

    pub fn refresh_host_metrics(&mut self) {
        if !self.connected() || self.host_metrics_request.is_some() {
            return;
        }
        let (Some(services), Some(client)) = (&self.services, &self.client) else {
            return;
        };
        let Some(ui) = services.ui.get().cloned() else {
            return;
        };
        let client = client.clone();
        let epoch = self.connection.epoch;
        self.host_metrics = Load::Loading;
        let task = services.runtime.spawn(async move {
            let result: Result<HostMetrics, _> = client.get("/api/v1/host/metrics").await;
            ui.update(move |state, _| {
                if state.connection.epoch != epoch {
                    return;
                }
                state.host_metrics_request = None;
                state.host_metrics = match result {
                    Ok(metrics) => Load::Ready(metrics),
                    Err(error) => Load::Failed(error.to_string()),
                };
            });
        });
        self.host_metrics_request = Some(RequestTask(task.abort_handle()));
    }

    /// A new cycle uses the existing durable submission/reconciliation controller.
    pub fn new_cycle(&mut self) {
        if self.submissions.atlas.attempt.is_some() && self.submissions.atlas.receipt.is_none() {
            return;
        }
        self.backend.article_day = TextEdit::new(chrono::Utc::now().format("%Y-%m-%d").to_string());
        self.preview_atlas_cycle();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletion_pins_reviewed_configuration_and_rejects_missing_or_changed_revision() {
        let mut state = State::fixture();
        state.connection.status = crate::connection::ConnectionStatus::Connected;
        let mut capabilities = fleetworks_contracts::capabilities::Capabilities::default();
        capabilities
            .permissions
            .push(fleetworks_contracts::context::Permission::SubmitInteractive);
        state.connection.capabilities = Some(capabilities);
        let receipt = fleetworks_contracts::connect::RegistrationReceipt::new(
            "deployment".into(),
            "workstation".into(),
            "registration".into(),
            None,
            1,
        );
        state.registration.context = Some(
            RegisteredContext::new(receipt, 1, 1, "config".into(), "access".into())
                .expect("context"),
        );
        state.cycles.history = Load::Ready(CycleHistory::new(vec![
            fleetworks_contracts::cycle_history::CycleSummary::new(
                "cycle".into(),
                "run".into(),
                "completed".into(),
                "2026-11-12T12:00:00Z".into(),
                None,
            ),
        ]));
        state.cycles.selected = Some("cycle".into());
        assert!(state.cycle_deletion_disabled_reason().is_some());
        state.preview_delete_cycle();
        assert!(state.cycles.confirmation.is_none());
        state.backend.models = Load::Ready(
            serde_json::from_value(
                serde_json::json!({"configuration_revision":0,"roles":[],"defaults":{}}),
            )
            .expect("explicit initial configuration"),
        );
        state.preview_delete_cycle();
        let request = state
            .prepared_cycle_deletion("request".into())
            .expect("reviewed delete");
        assert_eq!(request.context.expected_configuration_revision, Some(0));
        assert_eq!(request.job_id, "cycle");
        assert_eq!(request.expected_revision, 1);
        if let Load::Ready(models) = &mut state.backend.models {
            models.configuration_revision = 1;
        }
        assert!(
            state.prepared_cycle_deletion("request".into()).is_err(),
            "changed configuration cannot silently retarget consent"
        );
        assert!(
            state.cycles.retained_delete.is_none(),
            "invalid preparation never journals or posts a mutation"
        );
    }
}
