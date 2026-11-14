//! Explicit actions over server-advertised queued/failed job availability.

use fleetworks_contracts::context::Permission;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::Client;
use fleetworks_contracts::JOB_CANCEL_PATH;
use fleetworks_contracts::JOB_RETRY_PATH;
use serde::Deserialize;
use serde::Serialize;

use crate::app::Intent;
use crate::app::Load;
use crate::app::State;
use crate::connection::GatewayEffects;
use crate::connection::RequestTask;
use crate::jobs_query::JobsEvent;
use crate::platform;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobAction {
    Cancel,
    Retry,
}

impl JobAction {
    fn permission(self) -> Permission {
        match self {
            Self::Cancel => Permission::CancelJob,
            Self::Retry => Permission::RetryJob,
        }
    }
    fn route(self) -> &'static str {
        match self {
            Self::Cancel => JOB_CANCEL_PATH,
            Self::Retry => JOB_RETRY_PATH,
        }
    }
}

#[derive(Clone)]
pub struct JobActionConfirmation {
    pub action: JobAction,
    pub job_id: String,
    pub title: String,
    pub context: RegisteredContext,
    pub origin_revision: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct JobActionRequest {
    pub id: String,
    pub request_id: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct JobActionReceipt {
    pub action_id: String,
    pub job_id: String,
    pub action: JobAction,
    pub state: String,
    pub affected_tasks: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct JobActionResponse {
    pub done: bool,
    pub requeued: u64,
    pub receipt: JobActionReceipt,
}

#[derive(Default)]
pub struct JobActions {
    pub confirmation: Option<JobActionConfirmation>,
    pub attempt: Option<(JobActionConfirmation, JobActionRequest)>,
    pub response: Load<JobActionResponse>,
    pub message: Option<String>,
    pub(crate) revision: u64,
    pub(crate) request: Option<RequestTask>,
}

impl JobActions {
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub(crate) fn invalidate_origin(&mut self) {
        self.confirmation = None;
        self.request = None;
        self.revision = self.revision.wrapping_add(1);
        if self.attempt.is_some() && matches!(self.response, Load::Loading) {
            self.response = Load::Failed("Origin changed before the action receipt was received. Refresh the original job; no replacement action was sent.".into());
        }
    }
}

impl State {
    pub fn job_action_disabled_reason(&self, action: JobAction) -> Option<String> {
        if !self.permission(Permission::ReadWorkstationActivity)
            || !self.permission(action.permission())
        {
            return Some("Permission required for this job action".into());
        }
        if self.registration.context.is_none() {
            return Some("Register this workspace first".into());
        }
        if matches!(self.job_actions.response, Load::Loading) {
            return Some("Waiting for the previous action receipt".into());
        }
        let Load::Ready(detail) = &self.backend.detail else {
            return Some("Refresh the selected job first".into());
        };
        if self.backend.selected_job.as_deref() != Some(detail.job.id.as_str())
            || !detail.private_details_available
            || !detail.job.private_details_available
            || detail.job.status_only
        {
            return Some("This job is available as status only".into());
        }
        let allowed = detail.activity_actions_available
            && detail.job.activity_actions_available
            && match action {
                JobAction::Cancel => detail.job.can_cancel,
                JobAction::Retry => detail.job.can_retry,
            };
        if !allowed {
            return Some(
                match detail.job.action_reason_code.as_str() {
                    "action_permission_required" => "Permission required for this job action",
                    "shared_or_unowned_work" => {
                        "This action is unavailable for shared work or another workspace"
                    }
                    _ => "The backend does not offer this action for the current job state",
                }
                .into(),
            );
        }
        None
    }

    pub fn preview_job_action(&mut self, action: JobAction) {
        self.job_actions.message = None;
        if let Some(reason) = self.job_action_disabled_reason(action) {
            self.job_actions.message = Some(reason);
            return;
        }
        let (Some(context), Load::Ready(detail)) =
            (&self.registration.context, &self.backend.detail)
        else {
            return;
        };
        self.job_actions.response = Load::Idle;
        self.job_actions.attempt = None;
        self.job_actions.confirmation = Some(JobActionConfirmation {
            action,
            job_id: detail.job.id.clone(),
            title: detail.job.title.clone(),
            context: context.clone(),
            origin_revision: self.origin_revision,
        });
        self.reveal_control("job-action-confirmation");
    }

    pub fn confirm_job_action(&mut self) {
        let Some(confirmation) = self.job_actions.confirmation.take() else {
            return;
        };
        if !self.job_action_context_current(&confirmation) {
            self.job_actions.message =
                Some("Origin or selected job changed; review the action again".into());
            return;
        }
        if let Some(reason) = self.job_action_disabled_reason(confirmation.action) {
            self.job_actions.message = Some(reason);
            return;
        }
        self.job_actions.confirmation = Some(confirmation.clone());
        self.job_actions.response = Load::Loading;
        self.job_actions.revision = self.job_actions.revision.wrapping_add(1);
        let revision = self.job_actions.revision;
        self.record(Intent::PrepareJobAction {
            action: confirmation.action,
        });
        if self.fixture {
            return;
        }
        if let Some(services) = &self.services {
            self.job_actions.request =
                services.prepare_job_action(confirmation, self.connection.epoch, revision);
        } else {
            self.job_actions.response =
                Load::Failed("Job action preparation service is unavailable".into());
        }
    }

    pub fn complete_job_action_preparation(
        &mut self,
        revision: u64,
        result: Result<(JobActionConfirmation, JobActionRequest), String>,
    ) {
        if self.job_actions.revision != revision {
            return;
        }
        self.job_actions.request = None;
        let (confirmation, request) = match result {
            Ok(prepared) => prepared,
            Err(error) => {
                self.job_actions.response = Load::Failed(error);
                return;
            }
        };
        if !self.job_action_context_current(&confirmation) {
            self.job_actions.response =
                Load::Failed("Origin changed before the action was sent".into());
            return;
        }
        self.job_actions.confirmation = None;
        self.job_actions.attempt = Some((confirmation, request));
        self.send_job_action();
    }

    pub fn retry_exact_job_action(&mut self) {
        self.send_job_action();
    }

    fn job_action_context_current(&self, confirmation: &JobActionConfirmation) -> bool {
        self.origin_revision == confirmation.origin_revision
            && self.registration.context.as_ref() == Some(&confirmation.context)
            && self.backend.selected_job.as_deref() == Some(confirmation.job_id.as_str())
    }

    fn send_job_action(&mut self) {
        if self.job_actions.request.is_some() && matches!(self.job_actions.response, Load::Loading)
        {
            return;
        }
        let Some((confirmation, request)) = self.job_actions.attempt.clone() else {
            return;
        };
        if !self.job_action_context_current(&confirmation)
            || !self.permission(Permission::ReadWorkstationActivity)
            || !self.permission(confirmation.action.permission())
        {
            self.job_actions.message =
                Some("Select the original registered job context to reconcile this action".into());
            return;
        }
        self.job_actions.response = Load::Loading;
        self.job_actions.revision = self.job_actions.revision.wrapping_add(1);
        let revision = self.job_actions.revision;
        self.record(Intent::PostJobAction {
            action: confirmation.action,
        });
        if self.fixture {
            return;
        }
        let (Some(services), Some(client)) = (&self.services, &self.client) else {
            self.job_actions.response =
                Load::Failed("Gateway job action services are unavailable".into());
            return;
        };
        let client = match client.with_registered_context(&confirmation.context) {
            Ok(client) => client,
            Err(error) => {
                self.job_actions.response = Load::Failed(error.to_string());
                return;
            }
        };
        self.job_actions.request = services.job_action(
            client,
            confirmation,
            request,
            self.connection.epoch,
            revision,
        );
    }

    pub fn complete_job_action(
        &mut self,
        revision: u64,
        result: Result<JobActionResponse, String>,
    ) {
        if self.job_actions.revision != revision {
            return;
        }
        self.job_actions.request = None;
        let Some((confirmation, _)) = &self.job_actions.attempt else {
            return;
        };
        self.job_actions.response = match result {
            Ok(response) if response.receipt.job_id == confirmation.job_id && response.receipt.action == confirmation.action
                && !response.receipt.action_id.is_empty() && response.receipt.affected_tasks == 1
                && match confirmation.action {
                    JobAction::Cancel => response.done && response.requeued == 0 && response.receipt.state == "cancelled",
                    JobAction::Retry => !response.done && response.requeued == 1 && response.receipt.state == "queued",
                } => Load::Ready(response),
            Ok(_) => Load::Failed("Backend returned an unexpected action receipt; refresh the job before any new action".into()),
            Err(error) => Load::Failed(format!("Action outcome unconfirmed: {error}. Recheck this exact action or refresh the job.")),
        };
        let id = confirmation.job_id.clone();
        if matches!(self.job_actions.response, Load::Ready(_))
            && self.backend.selected_job.as_deref() == Some(id.as_str())
        {
            self.select_job(&id);
            self.request_jobs(JobsEvent::Poll);
        }
    }
}

impl GatewayEffects {
    fn prepare_job_action(
        &self,
        confirmation: JobActionConfirmation,
        epoch: u64,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let origin_revision = confirmation.origin_revision;
        let task = self.runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                platform::request_id()
                    .map(|request_id| {
                        let request = JobActionRequest {
                            id: confirmation.job_id.clone(),
                            request_id,
                        };
                        (confirmation, request)
                    })
                    .map_err(|error| error.to_string())
            })
            .await
            .map_err(|_| "Job action identity worker stopped".to_owned())
            .and_then(|result| result);
            ui.update(move |state, _| {
                if state.connection.epoch == epoch && state.origin_revision == origin_revision {
                    state.complete_job_action_preparation(revision, result);
                }
            });
        });
        Some(RequestTask(task.abort_handle()))
    }

    fn job_action(
        &self,
        client: Client,
        confirmation: JobActionConfirmation,
        request: JobActionRequest,
        epoch: u64,
        revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let result = client
                .post(confirmation.action.route(), &request)
                .await
                .map_err(|error| error.to_string());
            ui.update(move |state, _| {
                if state.connection.epoch == epoch
                    && state.origin_revision == confirmation.origin_revision
                {
                    state.complete_job_action(revision, result);
                }
            });
        });
        Some(RequestTask(task.abort_handle()))
    }
}

#[cfg(test)]
mod tests;
