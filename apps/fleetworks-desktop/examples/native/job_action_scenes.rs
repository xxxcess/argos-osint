use fleetworks_contracts::context::Permission;
use fleetworks_desktop::app::Intent;
use fleetworks_desktop::app::Load;
use fleetworks_desktop::job_actions::JobAction;
use fleetworks_desktop::job_actions::JobActionReceipt;
use fleetworks_desktop::job_actions::JobActionRequest;
use fleetworks_desktop::job_actions::JobActionResponse;
use fleetworks_desktop::State;

fn setup(state: &mut State, action: JobAction) {
    super::jobs_ready(state);
    state
        .connection
        .capabilities
        .as_mut()
        .expect("synthetic permissions")
        .permissions
        .extend([Permission::CancelJob, Permission::RetryJob]);
    if let Load::Ready(detail) = &mut state.backend.detail {
        detail.job.operation = "atlas_cycle".into();
        detail.job.title = "Synthetic collection job".into();
        detail.job.state = if action == JobAction::Cancel {
            "queued"
        } else {
            "failed"
        }
        .into();
        detail.job.phase = detail.job.state.clone();
        detail.job.progress_done = None;
        detail.job.progress_total = None;
        detail.job.can_cancel = action == JobAction::Cancel;
        detail.job.can_retry = action == JobAction::Retry;
        detail.job.action_reason_code = "supported".into();
        detail.job.activity_actions_available = true;
        detail.activity_actions_available = true;
    }
    align_selected_row(state);
    state.backend.logs = Load::Idle;
}

fn align_selected_row(state: &mut State) {
    if let (Load::Ready(detail), Load::Ready(jobs)) =
        (&state.backend.detail, &mut state.backend.jobs)
    {
        if let Some(row) = jobs.rows.iter_mut().find(|row| row.id == detail.job.id) {
            *row = detail.job.clone();
        }
        jobs.activity_actions_available = detail.activity_actions_available;
    }
}

pub fn cancel_preview(state: &mut State) {
    setup(state, JobAction::Cancel);
    state.preview_job_action(JobAction::Cancel);
}
pub fn retry_preview(state: &mut State) {
    setup(state, JobAction::Retry);
    state.preview_job_action(JobAction::Retry);
}
pub fn cancel_receipt(state: &mut State) {
    cancel_preview(state);
    state.confirm_job_action();
}
pub fn retry_receipt(state: &mut State) {
    retry_preview(state);
    state.confirm_job_action();
}

pub fn reply(state: &mut State, intent: &Intent) -> bool {
    match intent {
        Intent::PrepareJobAction { .. } => {
            let confirmation = state
                .job_actions
                .confirmation
                .clone()
                .expect("same-handler confirmation");
            let request = JobActionRequest {
                id: confirmation.job_id.clone(),
                request_id: "synthetic-job-action".into(),
            };
            state.complete_job_action_preparation(
                state.job_actions.revision(),
                Ok((confirmation, request)),
            );
        }
        Intent::PostJobAction { action } => {
            let id = state
                .job_actions
                .attempt
                .as_ref()
                .expect("original action")
                .1
                .id
                .clone();
            state.complete_job_action(
                state.job_actions.revision(),
                Ok(JobActionResponse {
                    done: *action == JobAction::Cancel,
                    requeued: u64::from(*action == JobAction::Retry),
                    receipt: JobActionReceipt {
                        action_id: "synthetic-action-receipt".into(),
                        job_id: id,
                        action: *action,
                        state: if *action == JobAction::Cancel {
                            "cancelled"
                        } else {
                            "queued"
                        }
                        .into(),
                        affected_tasks: 1,
                    },
                }),
            );
        }
        Intent::BackendRead {
            kind: fleetworks_desktop::backend::ReadKind::JobDetail,
        } if state.job_actions.attempt.is_some() => {
            if let (Load::Ready(response), Load::Ready(detail)) =
                (&state.job_actions.response, &mut state.backend.detail)
            {
                detail.job.state = response.receipt.state.clone();
                detail.job.phase = detail.job.state.clone();
                detail.job.can_cancel = false;
                detail.job.can_retry = false;
                detail.job.activity_actions_available = false;
                detail.activity_actions_available = false;
                detail.job.action_reason_code = "operation_or_state_not_supported".into();
            }
            align_selected_row(state);
        }
        _ => return false,
    }
    true
}
