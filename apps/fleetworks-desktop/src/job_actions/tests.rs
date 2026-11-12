use fleetworks_contracts::capabilities::Capabilities;
use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::context::ActivityScope;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::context::RegisteredContext;
use rune::Env;
use rune::Runtime;

use super::JobAction;
use super::JobActionReceipt;
use super::JobActionRequest;
use super::JobActionResponse;
use crate::app;
use crate::app::Intent;
use crate::app::Load;
use crate::app::Page;
use crate::app::State;
use crate::backend::ActivityJob;
use crate::backend::ActivityJobs;
use crate::backend::JobDetail;
use crate::connection::ConnectionStatus;

fn fixture() -> State {
    let mut state = State::fixture();
    state.page = Page::Jobs;
    state.connection.status = ConnectionStatus::Connected;
    let mut capabilities = Capabilities::default();
    capabilities.permissions = vec![
        Permission::ReadBackend,
        Permission::ReadWorkstationActivity,
        Permission::CancelJob,
        Permission::RetryJob,
    ];
    state.connection.capabilities = Some(capabilities);
    let registration = RegistrationReceipt::new(
        "synthetic-deployment".into(),
        "synthetic-node".into(),
        "synthetic-workspace".into(),
        "synthetic-registration".into(),
        "192.0.2.42:default".parse().expect("synthetic Connect ID"),
        1,
    );
    let context = RegisteredContext::new(
        registration,
        1,
        1,
        "synthetic-config".into(),
        "synthetic-access".into(),
    )
    .expect("synthetic registered proof");
    state.registration.context = Some(context.clone());
    state.sync_jobs_binding();
    let job = ActivityJob {
        id: "synthetic-job".into(),
        app: "atlas".into(),
        title: "Synthetic queued collection".into(),
        operation: "atlas_cycle".into(),
        state: "queued".into(),
        phase: "queued".into(),
        progress_done: None,
        progress_total: None,
        error_summary: String::new(),
        private_details_available: true,
        activity_actions_available: true,
        status_only: false,
        can_cancel: true,
        can_retry: false,
        action_reason_code: "supported".into(),
    };
    state.backend.jobs = Load::Ready(ActivityJobs {
        rows: vec![job.clone()],
        activity_scope: ActivityScope::for_workstation(&context),
        relation_counts_available: false,
        activity_actions_available: true,
    });
    state.backend.detail = Load::Ready(JobDetail {
        job,
        maintenance_result: None,
        stages: Vec::new(),
        private_details_available: true,
        activity_actions_available: true,
    });
    state.backend.selected_job = Some("synthetic-job".into());
    state.registration.context = Some(context);
    state.fixture_effect = Some(reply);
    state
}

fn reply(state: &mut State, intent: &Intent) {
    match intent {
        Intent::PrepareJobAction { .. } => {
            let confirmation = state
                .job_actions
                .confirmation
                .clone()
                .expect("confirmed action");
            let request = JobActionRequest {
                id: confirmation.job_id.clone(),
                request_id: "synthetic-action-request".into(),
            };
            state.complete_job_action_preparation(
                state.job_actions.revision(),
                Ok((confirmation, request)),
            );
        }
        Intent::PostJobAction { action } => state.complete_job_action(
            state.job_actions.revision(),
            Ok(JobActionResponse {
                done: *action == JobAction::Cancel,
                requeued: u64::from(*action == JobAction::Retry),
                receipt: JobActionReceipt {
                    action_id: "synthetic-action-receipt".into(),
                    job_id: "synthetic-job".into(),
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
        ),
        _ => {}
    }
}

#[test]
fn queued_state_does_not_grant_an_action_without_server_availability_and_permission() {
    let mut state = fixture();
    if let Load::Ready(detail) = &mut state.backend.detail {
        detail.job.can_cancel = false;
    }
    state.preview_job_action(JobAction::Cancel);
    assert!(state.job_actions.confirmation.is_none());
    if let Load::Ready(detail) = &mut state.backend.detail {
        detail.job.can_cancel = true;
    }
    state
        .connection
        .capabilities
        .as_mut()
        .expect("fixture permissions")
        .permissions
        .retain(|permission| *permission != Permission::CancelJob);
    state.preview_job_action(JobAction::Cancel);
    assert!(
        state.job_actions.confirmation.is_none(),
        "mere job visibility cannot grant cancellation"
    );
    assert!(state.intents.is_empty());
}

#[test]
fn real_cancel_controls_require_confirmation_and_keep_status_read_authoritative() {
    let mut runtime = Runtime::new(
        fixture(),
        app::view,
        (1280.0, 2400.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    runtime.frame(0.0);
    for (index, key) in ["job-cancel-preview", "job-action-confirm"]
        .into_iter()
        .enumerate()
    {
        let (x, y) = runtime.find(key).expect("shared production job control");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame((index + 1) as f64 / 60.0);
        if index == 0 {
            assert!(!runtime
                .state
                .intents
                .iter()
                .any(|intent| matches!(intent, Intent::PostJobAction { .. })));
        }
    }
    assert!(matches!(runtime.state.job_actions.response, Load::Ready(_)));
    assert!(
        matches!(&runtime.state.backend.detail, Load::Ready(detail) if detail.job.state == "queued"),
        "a receipt does not optimistically overwrite the separately observed job row"
    );
}

#[test]
fn uncertain_job_action_replays_the_exact_request_identity() {
    fn lose_response(state: &mut State, intent: &Intent) {
        if matches!(intent, Intent::PostJobAction { .. }) {
            state.complete_job_action(
                state.job_actions.revision(),
                Err("Synthetic response lost".into()),
            );
        } else {
            reply(state, intent);
        }
    }
    let mut state = fixture();
    state.fixture_effect = Some(lose_response);
    state.preview_job_action(JobAction::Cancel);
    state.confirm_job_action();
    let request_id = state
        .job_actions
        .attempt
        .as_ref()
        .expect("retained action")
        .1
        .request_id
        .clone();
    state.fixture_effect = Some(reply);
    state.retry_exact_job_action();
    assert_eq!(
        state
            .job_actions
            .attempt
            .as_ref()
            .expect("same action")
            .1
            .request_id,
        request_id
    );
    assert!(matches!(state.job_actions.response, Load::Ready(_)));
}

#[test]
fn closing_ephemeral_requests_never_invokes_a_job_action() {
    let mut state = fixture();
    state.cancel_requests();
    assert!(state.intents.is_empty());
    assert!(matches!(&state.backend.detail,Load::Ready(detail) if detail.job.state == "queued"));
}
