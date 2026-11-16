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

fn format_metrics_age(seconds: f64) -> String {
    let seconds = seconds as u64;
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
    }
}

#[derive(Default)]
pub struct Cycles {
    pub history: Load<CycleHistory>,
    pub page_index: usize,
    page_cursors: Vec<Option<String>>,
    pending_page: Option<CyclePageRequest>,
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct CyclePageRequest {
    cursor: Option<String>,
    index: usize,
    epoch: u64,
    origin_revision: u64,
    revision: u64,
    /// A discovery poll refreshes the viewed page in place. It must not reset the viewed page or
    /// the selected cycle.
    keep_selection: bool,
}

impl Cycles {
    pub(crate) fn busy(&self) -> bool {
        self.pending_page.is_some()
            || self.read_request.is_some()
            || self.journal_request.is_some()
            || self.delete_request.is_some()
            || matches!(self.history, Load::Loading)
            || matches!(self.deletion, Load::Loading)
    }

    pub(crate) fn invalidate(&mut self) {
        self.read_request = None;
        self.pending_page = None;
        self.journal_request = None;
        self.delete_request = None;
        self.revision = self.revision.wrapping_add(1);
        self.history = Load::Idle;
        self.page_index = 0;
        self.page_cursors.clear();
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

    /// Restores the protected deletion journal. Returns `true` when a read was started; that read
    /// then starts the actor index journal restore when it finishes, so the two protected-journal
    /// reads never contend for the nonblocking catalog lock. Returns `false` when nothing was
    /// started and the caller must start the next restore itself.
    pub(crate) fn restore_cycle_deletion(&mut self) -> bool {
        if self.cycles.retained_delete.is_some() || self.cycles.journal_request.is_some() {
            return false;
        }
        let (Some(services), Some(container), Some(context)) = (
            &self.services,
            &self.workspaces.container,
            self.registration.context.clone(),
        ) else {
            return false;
        };
        let Some(ui) = services.ui.get().cloned() else {
            return false;
        };
        let container = container.clone();
        let epoch = self.connection.epoch;
        let origin_revision = self.origin_revision;
        let task = services.runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || container.pending_cycle_deletions()).await;
            ui.update(move |state, _| {
                // `cycles.revision` also advances on every history page load, so it cannot guard
                // this read. Invalidation drops the request task, which aborts it.
                if state.connection.epoch != epoch || state.origin_revision != origin_revision {
                    return;
                }
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
                state.restore_actor_reindex();
            });
        });
        self.cycles.journal_request = Some(RequestTask(task.abort_handle()));
        true
    }

    pub fn refresh_cycles(&mut self) {
        self.load_cycles_page(None, 0, false);
    }

    /// Discovers new cycles for the registered workstation by re-reading the page being viewed.
    /// The cursor, page number and selected cycle stay as they are.
    pub fn poll_cycles(&mut self) {
        let index = self.cycles.page_index;
        let cursor = self.cycles.page_cursors.get(index).cloned().flatten();
        self.load_cycles_page(cursor, index, true);
    }

    pub fn next_cycles_page(&mut self) {
        let Load::Ready(history) = &self.cycles.history else {
            return;
        };
        if let Some(cursor) = history.next_cursor.clone() {
            self.load_cycles_page(Some(cursor), self.cycles.page_index + 1, false);
        }
    }

    pub fn previous_cycles_page(&mut self) {
        if self.cycles.page_index > 0 {
            let previous = self.cycles.page_index - 1;
            if let Some(cursor) = self.cycles.page_cursors.get(previous).cloned() {
                self.load_cycles_page(cursor, previous, false);
            }
        }
    }

    fn load_cycles_page(&mut self, cursor: Option<String>, index: usize, keep_selection: bool) {
        let Some(context) = self.registration.context.clone() else {
            return;
        };
        if self.cycles.pending_page.is_some() {
            return;
        }
        let effect = match (&self.services, &self.client) {
            (Some(services), Some(client)) => {
                let Ok(client) = client.with_registered_context(&context) else {
                    return;
                };
                let Some(ui) = services.ui.get().cloned() else {
                    return;
                };
                Some((services.runtime.clone(), client, ui))
            }
            _ if self.fixture => None,
            _ => return,
        };
        self.cycles.revision = self.cycles.revision.wrapping_add(1);
        let request = CyclePageRequest {
            cursor,
            index,
            epoch: self.connection.epoch,
            origin_revision: self.origin_revision,
            revision: self.cycles.revision,
            keep_selection,
        };
        self.cycles.pending_page = Some(request.clone());
        if !matches!(self.cycles.history, Load::Ready(_)) {
            self.cycles.history = Load::Loading;
        }
        let Some((runtime, client, ui)) = effect else {
            return;
        };
        let task = runtime.spawn(async move {
            let result = client
                .cycle_history_page(40, request.cursor.as_deref())
                .await
                .map_err(|error| error.to_string());
            ui.update(move |state, _| state.complete_cycles_page(request, result));
        });
        self.cycles.read_request = Some(RequestTask(task.abort_handle()));
    }

    fn complete_cycles_page(
        &mut self,
        request: CyclePageRequest,
        result: Result<CycleHistory, String>,
    ) {
        if self.connection.epoch != request.epoch
            || self.origin_revision != request.origin_revision
            || self.cycles.revision != request.revision
            || self.cycles.pending_page.as_ref() != Some(&request)
        {
            return;
        }
        self.cycles.read_request = None;
        self.cycles.pending_page = None;
        match result {
            Ok(history)
                if history.rows.len() <= 40
                    && history
                        .next_cursor
                        .as_deref()
                        .is_none_or(|next| !next.is_empty() && next.len() <= 256) =>
            {
                self.cycles.page_index = request.index;
                self.cycles.page_cursors.truncate(request.index);
                self.cycles.page_cursors.push(request.cursor);
                let keep = request.keep_selection
                    && self
                        .cycles
                        .selected
                        .as_ref()
                        .is_some_and(|id| history.rows.iter().any(|row| &row.job_id == id));
                if !keep {
                    self.cycles.selected = None;
                }
                self.cycles.history = Load::Ready(history);
            }
            Ok(_) => {
                if !matches!(self.cycles.history, Load::Ready(_)) {
                    self.cycles.history =
                        Load::Failed("Gateway cycle history exceeded its requested bound".into());
                } else {
                    self.cycles.message =
                        Some("Gateway cycle history exceeded its requested bound".into());
                }
            }
            Err(error) => {
                if matches!(self.cycles.history, Load::Ready(_)) {
                    self.cycles.message = Some(error);
                } else {
                    self.cycles.history = Load::Failed(error);
                }
            }
        }
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
        let Some(container) = self.workspaces.container.clone() else {
            return;
        };
        let epoch = self.connection.epoch;
        let origin = self.origin_revision;
        let revision = self.cycles.revision;
        self.cycles.retained_delete = Some(request.clone());
        self.cycles.retained_context = Some(context.clone());
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
                if state.connection.epoch != epoch
                    || state.origin_revision != origin
                    || state.cycles.revision != revision
                {
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
        let revision = self.cycles.revision;
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
                if state.connection.epoch != epoch
                    || state.origin_revision != origin_revision
                    || state.cycles.revision != revision
                {
                    return;
                }
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

    pub(crate) fn apply_host_metrics(&mut self, result: Result<HostMetrics, String>) {
        match result {
            Ok(metrics) => {
                self.host_metrics = Load::Ready(metrics);
                self.host_metrics_observed_at = Some(self.clock);
                self.host_metrics_error = None;
            }
            Err(error) => {
                // Keep the last good values, but remember that they are no longer current.
                if matches!(self.host_metrics, Load::Ready(_)) {
                    self.host_metrics_error = Some(error);
                } else {
                    self.host_metrics = Load::Failed(error);
                }
            }
        }
    }

    /// Text for the stale marker, or `None` while the tiles are current.
    pub fn host_metrics_stale_notice(&self) -> Option<String> {
        let error = self.host_metrics_error.as_ref()?;
        let age = self
            .host_metrics_observed_at
            .map(|at| format_metrics_age((self.clock - at).max(0.0)));
        Some(match age {
            Some(age) => format!("Stale: last read {age} ago. Latest refresh failed: {error}"),
            None => format!("Stale: latest refresh failed: {error}"),
        })
    }

    pub fn refresh_host_metrics(&mut self) {
        if !self.connected()
            || !self.permission(fleetworks_contracts::context::Permission::ReadBackend)
            || self.host_metrics_request.is_some()
        {
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
        self.host_metrics_revision = self.host_metrics_revision.wrapping_add(1);
        let revision = self.host_metrics_revision;
        if !matches!(self.host_metrics, Load::Ready(_)) {
            self.host_metrics = Load::Loading;
        }
        let task = services.runtime.spawn(async move {
            let result: Result<HostMetrics, _> = client.get("/api/v1/host/metrics").await;
            ui.update(move |state, _| {
                if state.connection.epoch != epoch || state.host_metrics_revision != revision {
                    return;
                }
                state.host_metrics_request = None;
                state.apply_host_metrics(result.map_err(|error| error.to_string()));
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

    fn cycle_page(job_id: &str, next_cursor: Option<String>) -> CycleHistory {
        let mut page = CycleHistory::new(vec![
            fleetworks_contracts::cycle_history::CycleSummary::new(
                job_id.into(),
                format!("run-{job_id}"),
                "completed".into(),
                "2026-11-12T12:00:00Z".into(),
                None,
            ),
        ]);
        page.next_cursor = next_cursor;
        page
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cycle_page_controls_wait_for_delayed_results_and_preserve_current_page_on_error() {
        use rune::Env;
        use rune::Runtime;

        let cursor = format!("{:064x}", 2);
        let first = cycle_page("synthetic-cycle-1", Some(cursor.clone()));
        let mut state = State::fixture();
        state.page = crate::app::Page::Atlas;
        state.connection.status = crate::connection::ConnectionStatus::Connected;
        let receipt = fleetworks_contracts::connect::RegistrationReceipt::new(
            "synthetic-deployment".into(),
            "synthetic-workstation".into(),
            "synthetic-registration".into(),
            None,
            1,
        );
        state.registration.context = Some(
            RegisteredContext::new(
                receipt,
                1,
                1,
                "synthetic-config".into(),
                "synthetic-access".into(),
            )
            .expect("synthetic registration"),
        );
        state.cycles.history = Load::Ready(first.clone());
        state.cycles.page_cursors.push(None);
        let mut runtime = Runtime::new(
            state,
            crate::app::view,
            (1180.0, 780.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        let mut clock = 0.0;
        runtime.frame(clock);
        assert!(runtime.find("cycle-synthetic-cycle-1").is_some());

        runtime.update(|_, tx| tx.scroll_into_view("cycles-page-next"));
        clock += 1.0 / 60.0;
        runtime.frame(clock);
        let (x, y) = runtime
            .find("cycles-page-next")
            .expect("visible next-page button");
        assert!(
            (0.0..1180.0).contains(&x) && (50.0..780.0).contains(&y),
            "next-page button must be inside the cycle scroll viewport"
        );
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        clock += 1.0 / 60.0;
        runtime.frame(clock);
        let next = runtime
            .state
            .cycles
            .pending_page
            .clone()
            .expect("real Next page control started a request");
        assert_eq!(next.cursor.as_deref(), Some(cursor.as_str()));
        assert_eq!(next.index, 1);
        assert!(runtime.state.cycles.busy());
        assert!(runtime.find("cycle-synthetic-cycle-1").is_some());
        assert!(runtime.find("cycle-synthetic-cycle-2").is_none());
        assert!(
            !runtime
                .elements()
                .iter()
                .find(|element| element.key.as_deref() == Some("cycles-page-next"))
                .expect("next-page control remains visible")
                .focusable
        );

        let (release, response) = tokio::sync::oneshot::channel();
        let mock_gateway =
            tokio::spawn(async move { response.await.expect("mock cycle page released") });
        assert!(release.send(cycle_page("synthetic-cycle-2", None)).is_ok());
        let second = mock_gateway.await.expect("mock cycle gateway task");
        runtime.update(move |state, _| state.complete_cycles_page(next, Ok(second)));
        clock += 1.0 / 60.0;
        runtime.frame(clock);
        assert_eq!(runtime.state.cycles.page_index, 1);
        assert!(runtime.find("cycle-synthetic-cycle-2").is_some());
        assert!(runtime.find("cycle-synthetic-cycle-1").is_none());

        runtime.update(|_, tx| tx.scroll_into_view("cycles-page-previous"));
        clock += 1.0 / 60.0;
        runtime.frame(clock);
        let (x, y) = runtime
            .find("cycles-page-previous")
            .expect("visible previous-page button");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        clock += 1.0 / 60.0;
        runtime.frame(clock);
        let previous = runtime
            .state
            .cycles
            .pending_page
            .clone()
            .expect("real Previous page control started a request");
        assert_eq!(previous.cursor, None);
        assert_eq!(previous.index, 0);
        assert!(runtime.state.cycles.busy());
        let (release, response) = tokio::sync::oneshot::channel();
        let mock_gateway =
            tokio::spawn(async move { response.await.expect("mock first page released") });
        assert!(release.send(first).is_ok());
        let first = mock_gateway.await.expect("mock cycle gateway task");
        runtime.update(move |state, _| state.complete_cycles_page(previous, Ok(first)));
        clock += 1.0 / 60.0;
        runtime.frame(clock);
        assert_eq!(runtime.state.cycles.page_index, 0);
        assert!(runtime.find("cycle-synthetic-cycle-1").is_some());
        assert!(runtime.find("cycle-synthetic-cycle-2").is_none());

        runtime.update(|_, tx| tx.scroll_into_view("cycles-page-next"));
        clock += 1.0 / 60.0;
        runtime.frame(clock);
        let (x, y) = runtime
            .find("cycles-page-next")
            .expect("visible retry button");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        clock += 1.0 / 60.0;
        runtime.frame(clock);
        let failed = runtime
            .state
            .cycles
            .pending_page
            .clone()
            .expect("next page retry started");
        let (release, response) = tokio::sync::oneshot::channel();
        let mock_gateway =
            tokio::spawn(async move { response.await.expect("mock error released") });
        assert!(release
            .send(Err::<CycleHistory, _>("Synthetic gateway timeout".into()))
            .is_ok());
        let error = mock_gateway.await.expect("mock cycle gateway task");
        runtime.update(move |state, _| state.complete_cycles_page(failed, error));
        clock += 1.0 / 60.0;
        runtime.frame(clock);
        assert_eq!(runtime.state.cycles.page_index, 0);
        assert!(runtime.find("cycle-synthetic-cycle-1").is_some());
        assert_eq!(
            runtime.state.cycles.message.as_deref(),
            Some("Synthetic gateway timeout")
        );
        assert!(!runtime.state.cycles.busy());

        runtime.update(|_, tx| tx.scroll_into_view("cycles-page-next"));
        clock += 1.0 / 60.0;
        runtime.frame(clock);
        let (x, y) = runtime
            .find("cycles-page-next")
            .expect("visible next-page button");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        clock += 1.0 / 60.0;
        runtime.frame(clock);
        let stale = runtime
            .state
            .cycles
            .pending_page
            .clone()
            .expect("new delayed page request");
        runtime.update(|state, _| {
            state.cycles.invalidate();
            state.connection.epoch = state.connection.epoch.wrapping_add(1);
        });
        runtime.update(move |state, _| {
            state.complete_cycles_page(stale, Ok(cycle_page("stale-cycle", None)));
        });
        clock += 1.0 / 60.0;
        runtime.frame(clock);
        assert!(matches!(&runtime.state.cycles.history, Load::Idle));
        assert!(runtime.find("cycle-stale-cycle").is_none());
    }

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

    fn cycle_row(job_id: &str) -> fleetworks_contracts::cycle_history::CycleSummary {
        fleetworks_contracts::cycle_history::CycleSummary::new(
            job_id.into(),
            format!("run-{job_id}"),
            "completed".into(),
            "2026-11-12T12:00:00Z".into(),
            None,
        )
    }

    #[test]
    fn discovery_poll_finds_new_cycle_from_an_empty_page() {
        let mut state = crate::tests::registered_fixture();
        state.cycles.history = Load::Ready(CycleHistory::new(Vec::new()));
        state.cycles.page_cursors.push(None);

        state.poll_cycles();
        let request = state
            .cycles
            .pending_page
            .clone()
            .expect("an empty registered page still starts a discovery read");
        assert_eq!((request.index, request.cursor.as_deref()), (0, None));
        state.complete_cycles_page(request, Ok(cycle_page("new-cycle", None)));

        let Load::Ready(history) = &state.cycles.history else {
            panic!("discovery read must replace the empty page");
        };
        assert_eq!(history.rows.len(), 1);
        assert_eq!(history.rows[0].job_id, "new-cycle");
    }

    #[test]
    fn discovery_poll_keeps_page_two_cursor_and_selection() {
        let cursor = format!("{:064x}", 2);
        let mut state = crate::tests::registered_fixture();
        state.cycles.history = Load::Ready(cycle_page("cycle-3", None));
        state.cycles.page_index = 1;
        state.cycles.page_cursors = vec![None, Some(cursor.clone())];
        state.cycles.selected = Some("cycle-3".into());

        state.poll_cycles();
        let request = state
            .cycles
            .pending_page
            .clone()
            .expect("discovery read started on page two");
        assert_eq!(request.index, 1);
        assert_eq!(request.cursor.as_deref(), Some(cursor.as_str()));
        let refreshed = CycleHistory::new(vec![cycle_row("cycle-4"), cycle_row("cycle-3")]);
        state.complete_cycles_page(request, Ok(refreshed));

        assert_eq!(
            state.cycles.page_index, 1,
            "polling must not return to page one"
        );
        assert_eq!(state.cycles.page_cursors, vec![None, Some(cursor)]);
        assert_eq!(state.cycles.selected.as_deref(), Some("cycle-3"));
        let Load::Ready(history) = &state.cycles.history else {
            panic!("page two must stay loaded");
        };
        assert_eq!(history.rows.len(), 2);
    }

    #[test]
    fn discovery_poll_drops_selection_that_left_the_page() {
        let mut state = crate::tests::registered_fixture();
        state.cycles.history = Load::Ready(cycle_page("cycle-1", None));
        state.cycles.page_cursors.push(None);
        state.cycles.selected = Some("cycle-1".into());

        state.poll_cycles();
        let request = state.cycles.pending_page.clone().expect("read started");
        state.complete_cycles_page(request, Ok(cycle_page("cycle-2", None)));
        assert_eq!(state.cycles.selected, None);
    }

    #[test]
    fn discovery_result_from_a_prior_registration_is_dropped() {
        let mut state = crate::tests::registered_fixture();
        state.cycles.history = Load::Ready(CycleHistory::new(Vec::new()));
        state.cycles.page_cursors.push(None);
        state.poll_cycles();
        let request = state.cycles.pending_page.clone().expect("read started");

        // A registration change invalidates the Atlas history before the reply arrives.
        state.cycles.invalidate();
        state.complete_cycles_page(request, Ok(cycle_page("old-registration-cycle", None)));
        assert!(matches!(state.cycles.history, Load::Idle));
    }

    #[test]
    fn host_metrics_keep_last_values_and_flag_them_stale_until_recovery() {
        let metrics = || HostMetrics {
            cpu_percent: Some(10.0),
            ..HostMetrics::default()
        };
        let mut state = State::fixture();

        state.clock = 10.0;
        state.apply_host_metrics(Ok(metrics()));
        assert!(state.host_metrics_stale_notice().is_none());

        state.clock = 75.0;
        state.apply_host_metrics(Err("gateway timed out".into()));
        assert!(matches!(&state.host_metrics, Load::Ready(m) if m.cpu_percent == Some(10.0)));
        let notice = state
            .host_metrics_stale_notice()
            .expect("stale after failure");
        assert!(notice.contains("1m 5s"), "{notice}");
        assert!(notice.contains("gateway timed out"), "{notice}");

        state.clock = 80.0;
        state.apply_host_metrics(Ok(metrics()));
        assert!(state.host_metrics_stale_notice().is_none());
        assert_eq!(state.host_metrics_observed_at, Some(80.0));
    }

    #[test]
    fn first_host_metrics_failure_is_a_failed_read_not_stale_values() {
        let mut state = State::fixture();
        state.apply_host_metrics(Err("unreachable".into()));
        assert!(matches!(&state.host_metrics, Load::Failed(_)));
        assert!(state.host_metrics_stale_notice().is_none());
    }
}
