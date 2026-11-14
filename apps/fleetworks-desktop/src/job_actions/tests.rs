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
        "synthetic-registration".into(),
        Some("192.0.2.42".parse().expect("synthetic IP")),
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
fn legacy_cancel_handler_requires_confirmation_while_native_jobs_remains_read_only() {
    let mut state = fixture();
    state.preview_job_action(JobAction::Cancel);
    assert!(state.job_actions.confirmation.is_some());
    assert!(!state
        .intents
        .iter()
        .any(|intent| matches!(intent, Intent::PostJobAction { .. })));
    state.confirm_job_action();
    assert!(matches!(state.job_actions.response, Load::Ready(_)));
    assert!(
        matches!(&state.backend.detail,Load::Ready(detail) if detail.job.state=="queued"),
        "a receipt does not optimistically overwrite the authoritative job row"
    );
    let mut runtime = Runtime::new(
        state,
        app::view,
        (1180.0, 780.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    runtime.frame(0.0);
    for key in [
        "job-cancel-preview",
        "job-retry-preview",
        "job-action-confirm",
    ] {
        assert!(
            runtime.find(key).is_none(),
            "queued job fixture exposes neither retired cancel nor retry controls ({key})"
        );
    }
    assert!(runtime.accessibility_json().to_string().contains("queued"));
}

fn click(runtime: &mut Runtime<State>, key: &str, clock: &mut f64) {
    runtime.update(|_, tx| tx.scroll_into_view(key));
    *clock += 1.0 / 60.0;
    runtime.frame(*clock);
    let (x, y) = runtime
        .find(key)
        .unwrap_or_else(|| panic!("{key} presented"));
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    *clock += 1.0 / 60.0;
    runtime.frame(*clock);
}

#[test]
fn failed_retryable_job_exposes_confirmed_native_retry_flow() {
    let mut state = fixture();
    if let Load::Ready(detail) = &mut state.backend.detail {
        detail.job.state = "failed".into();
        detail.job.phase = "Failed".into();
        detail.job.can_cancel = false;
        detail.job.can_retry = true;
    }
    let mut runtime = Runtime::new(
        state,
        app::view,
        (1180.0, 780.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    let mut clock = 0.0;
    runtime.frame(clock);
    assert!(
        runtime.find("job-cancel-preview").is_none(),
        "retired native cancel action remains unexposed"
    );
    assert!(
        runtime.find("job-retry-preview").is_some(),
        "failed retryable job exposes native retry button"
    );

    click(&mut runtime, "job-retry-preview", &mut clock);
    assert!(runtime.state.job_actions.confirmation.is_some());
    assert!(!runtime
        .state
        .intents
        .iter()
        .any(|intent| matches!(intent, Intent::PostJobAction { .. })));
    assert!(runtime.find("job-action-confirmation").is_some());
    assert!(runtime.find("job-action-confirm").is_some());

    click(&mut runtime, "job-action-confirm", &mut clock);
    assert!(matches!(runtime.state.job_actions.response, Load::Ready(_)));
    assert!(runtime.find("job-action-receipt").is_some());
    assert!(runtime.state.intents.iter().any(|intent| matches!(
        intent,
        Intent::PostJobAction {
            action: JobAction::Retry
        }
    )));
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
