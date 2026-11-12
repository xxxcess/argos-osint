use rune::El;
use rune_kit::Button;
use rune_kit::ButtonVariant;
use rune_kit::Kit;

use crate::app::Load;
use crate::app::State;
use crate::job_actions::JobAction;
use crate::views::heading;
use crate::views::line;
use crate::views::quiet;

pub(crate) fn actions(state: &State, kit: &Kit) -> El {
    let mut body = rune::col().gap(8.0);
    for (action, label, key) in [
        (
            JobAction::Cancel,
            "Cancel queued job…",
            "job-cancel-preview",
        ),
        (JobAction::Retry, "Retry failed job…", "job-retry-preview"),
    ] {
        let reason = state.job_action_disabled_reason(action);
        body = body.child(
            Button::new(label)
                .disabled(reason.is_some())
                .on_press(move |state: &mut State| state.preview_job_action(action))
                .build(kit)
                .key(key),
        );
        if let Some(reason) = reason {
            body = body.child(quiet(reason, kit));
        }
    }
    if let Some(confirmation) = &state.job_actions.confirmation {
        if !matches!(state.job_actions.response, Load::Loading) {
            body = body.child(rune::col().gap(8.0).key("job-action-confirmation")
                .child(heading(match confirmation.action { JobAction::Cancel => "Cancel this queued job?", JobAction::Retry => "Retry this failed job?" }, kit))
                .child(line(confirmation.title.clone(), kit))
                .child(quiet(format!("Job {} · {}", confirmation.job_id, confirmation.context.connect_id), kit))
                .child(quiet(match confirmation.action { JobAction::Cancel => "The backend checks that execution has not started before cancelling.", JobAction::Retry => "The backend queues only the supported failed task. Successful work is preserved." }, kit))
                .child(Button::new("Confirm job action").on_press(|state: &mut State| state.confirm_job_action()).build(kit).key("job-action-confirm"))
                .child(Button::new("Keep job as it is").variant(ButtonVariant::Ghost).on_press(|state: &mut State| state.job_actions.confirmation = None).build(kit)));
        }
    }
    let current = state
        .job_actions
        .attempt
        .as_ref()
        .is_some_and(|(confirmation, _)| {
            state.registration.context.as_ref() == Some(&confirmation.context)
                && state.origin_revision() == confirmation.origin_revision
        });
    match &state.job_actions.response {
        Load::Idle => {}
        Load::Loading | Load::Pending => {
            body = body.child(quiet("Waiting for the job action receipt…", kit).role("status"))
        }
        Load::Failed(error) => {
            body = body.child(line(error.clone(), kit).role("alert"));
            if current {
                body = body.child(
                    Button::new("Recheck exact action")
                        .on_press(|state: &mut State| state.retry_exact_job_action())
                        .build(kit)
                        .key("job-action-recheck"),
                );
            }
        }
        Load::Ready(response) if current => {
            body = body.child(
                line(
                    format!(
                        "Job {} · {}",
                        response.receipt.job_id,
                        match response.receipt.action {
                            JobAction::Cancel => "cancelled before execution",
                            JobAction::Retry => "retry queued; check job status for progress",
                        }
                    ),
                    kit,
                )
                .role("status")
                .key("job-action-receipt"),
            )
        }
        Load::Ready(_) => {
            body = body.child(quiet(
                "A prior job action has a receipt in its original workstation context.",
                kit,
            ))
        }
    }
    if let Some(message) = &state.job_actions.message {
        body = body.child(quiet(message.clone(), kit).role("status"));
    }
    body
}
