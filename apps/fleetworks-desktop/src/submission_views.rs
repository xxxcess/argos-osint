//! Confirmed backend-wide actions with a separately displayed request origin.

use fleetworks_contracts::context::Permission;
use rune::El;
use rune_kit::Button;
use rune_kit::ButtonVariant;
use rune_kit::Kit;

use crate::app::Load;
use crate::app::State;
use crate::submission::AcceptedReceipt;
use crate::submission::SubmissionInput;
use crate::submission::SubmissionKind;
use crate::submission::SubmissionPhase;
use crate::views::heading;
use crate::views::line;
use crate::views::quiet;

pub(crate) fn atlas(state: &State, kit: &Kit) -> El {
    panel(state, SubmissionKind::Atlas, kit)
}
pub(crate) fn tools(state: &State, kit: &Kit) -> El {
    panel(state, SubmissionKind::Tool, kit)
}

fn panel(state: &State, kind: SubmissionKind, kit: &Kit) -> El {
    let lane = state.submissions.lane(kind);
    let disabled = state.submission_disabled_reason(kind);
    let mut body = rune::col().gap(10.0).child(heading(kind.label(), kit))
        .child(quiet("This operation changes a shared backend resource. The selected workstation and workspace identify your request, while the backend owns the accepted job and its lifecycle.", kit));
    let label = match kind {
        SubmissionKind::Atlas => "Review collection cycle",
        SubmissionKind::Tool => "Review tool run",
    };
    body = body.child(
        Button::new(label)
            .disabled(disabled.is_some())
            .on_press(move |state: &mut State| match kind {
                SubmissionKind::Atlas => state.preview_atlas_cycle(),
                SubmissionKind::Tool => state.preview_tool_run(),
            })
            .build(kit)
            .key(match kind {
                SubmissionKind::Atlas => "atlas-submit-preview",
                SubmissionKind::Tool => "tool-submit-preview",
            }),
    );
    if let Some(reason) = disabled {
        body = body.child(quiet(reason, kit));
    }
    if !lane.busy() {
        if let Some(snapshot) = &lane.confirmation {
            let mut confirmation = rune::col()
                .gap(8.0)
                .key(match kind {
                    SubmissionKind::Atlas => "atlas-submit-confirmation",
                    SubmissionKind::Tool => "tool-submit-confirmation",
                })
                .child(heading("Confirm backend operation", kit))
                .child(line(
                    format!(
                        "Backend {} · request origin {} · configuration revision {}",
                        snapshot.context.deployment_id,
                        snapshot.context.connect_id,
                        snapshot.configuration_revision
                    ),
                    kit,
                ));
            match &snapshot.input {
                SubmissionInput::Atlas => confirmation = confirmation.child(quiet("Queue one Atlas collection cycle. A receipt means durable acceptance; articles and downstream work may still be pending or fail.", kit)),
                SubmissionInput::Tool { tool_id, inputs } => {
                    confirmation = confirmation.child(line(format!("Tool: {tool_id}"), kit))
                        .child(line(serde_json::to_string_pretty(inputs).unwrap_or_else(|_| "Input display unavailable".into()), kit).key("tool-confirmed-inputs"))
                        .child(quiet("Provider calls may consume quota. This submits the exact displayed inputs once under a stable request identity; uncertain delivery retries keep that identity.", kit));
                }
            }
            confirmation = confirmation
                .child(
                    Button::new("Confirm interactive request")
                        .variant(ButtonVariant::Submit)
                        .on_press(move |state: &mut State| state.confirm_submission(kind))
                        .build(kit)
                        .key(match kind {
                            SubmissionKind::Atlas => "atlas-submit-confirm",
                            SubmissionKind::Tool => "tool-submit-confirm",
                        }),
                )
                .child(
                    Button::new("Cancel preview")
                        .variant(ButtonVariant::Ghost)
                        .on_press(move |state: &mut State| {
                            state.submissions.lane_mut(kind).confirmation = None
                        })
                        .build(kit),
                );
            body = body.child(confirmation);
        }
    }
    match lane.phase {
        SubmissionPhase::Preparing => body = body.child(quiet("Preparing the stable request identity…", kit).role("status")),
        SubmissionPhase::Posting => body = body.child(quiet("Waiting for a durable acceptance response…", kit).role("status")),
        SubmissionPhase::OutcomeUnknown => body = body.child(line("The prior acceptance outcome is unknown. No replacement request identity or cancellation has been sent.", kit).role("status")),
        _ => {}
    }
    if let Some(attempt) = &lane.attempt {
        body = body.child(quiet(
            format!("Request identity: {}", attempt.request_id),
            kit,
        ));
    }
    if let Some(receipt) = &lane.receipt {
        body = body.child(quiet(
            match receipt {
                AcceptedReceipt::Atlas(receipt) => format!(
                    "Gateway reported job {} · run {}",
                    receipt.job_id, receipt.run_id
                ),
                AcceptedReceipt::Tool(receipt) => format!(
                    "Gateway reported call {}{}",
                    receipt.call_id,
                    receipt
                        .job_id
                        .as_ref()
                        .map_or(String::new(), |id| format!(" · job {id}"))
                ),
            },
            kit,
        ));
    }
    if let Load::Ready(association) = &lane.resolution {
        body = body.child(quiet(format!("Resolved request association: job {} · original request origin {} · configuration revision {}", association.job_id,
            association.origin.connect_id_at_submission, association.origin.configuration_revision), kit));
    }
    if lane.job_candidate().is_some() {
        body = body.child(
            Button::new("Recheck registered job status")
                .disabled(
                    lane.busy()
                        || !state.permission(Permission::ReadWorkstationActivity)
                        || matches!(lane.verification, Load::Loading),
                )
                .on_press(move |state: &mut State| state.verify_submission(kind))
                .build(kit)
                .key(match kind {
                    SubmissionKind::Atlas => "atlas-submit-verify",
                    SubmissionKind::Tool => "tool-submit-verify",
                }),
        );
    } else if lane.attempt.is_some() && !lane.busy() {
        body = body.child(
            Button::new("Retry exact request")
                .disabled(!state.permission(Permission::SubmitInteractive))
                .on_press(move |state: &mut State| state.retry_submission(kind))
                .build(kit)
                .key(match kind {
                    SubmissionKind::Atlas => "atlas-submit-retry",
                    SubmissionKind::Tool => "tool-submit-retry",
                }),
        );
    }
    if lane.attempt.is_some() {
        body = body.child(
            Button::new("Resolve exact prior request (read-only)")
                .disabled(
                    lane.busy()
                        || state.registration.context.is_none()
                        || !state.permission(Permission::ReadWorkstationActivity)
                        || matches!(lane.resolution, Load::Loading),
                )
                .on_press(move |state: &mut State| state.resolve_submission(kind))
                .build(kit)
                .key(match kind {
                    SubmissionKind::Atlas => "atlas-submit-resolve",
                    SubmissionKind::Tool => "tool-submit-resolve",
                }),
        );
    }
    match &lane.resolution {
        Load::Loading | Load::Pending => body = body.child(quiet("Looking up the original immutable request association under the current registered workspace…", kit).role("status")),
        Load::Failed(error) => body = body.child(line(format!("Request lookup unavailable: {error}. Select the original registered workspace before resolving its request."), kit).role("alert")),
        _ => {},
    }
    match &lane.verification {
        Load::Idle => {}
        Load::Pending | Load::Loading => {
            body = body.child(
                quiet(
                    "Checking the reported identity through registered workstation Jobs…",
                    kit,
                )
                .role("status"),
            )
        }
        Load::Failed(error) => {
            body = body
                .child(line(format!("Job verification unavailable: {error}"), kit).role("alert"))
        }
        Load::Ready(detail) => {
            let id = detail.job.id.clone();
            body = body
                .child(
                    line(
                        format!(
                            "Verified workstation job {} · {} · {}",
                            detail.job.id, detail.job.state, detail.job.phase
                        ),
                        kit,
                    )
                    .key("interactive-job-status")
                    .role("status"),
                )
                .child(
                    Button::new("View verified workstation job")
                        .on_press(move |state: &mut State| state.open_job(&id))
                        .build(kit)
                        .key(match kind {
                            SubmissionKind::Atlas => "atlas-submit-view-job",
                            SubmissionKind::Tool => "tool-submit-view-job",
                        }),
                );
            if detail.job.status_only || !detail.private_details_available {
                body = body.child(quiet("This associated job exposes status only. Its private details and results are unavailable.", kit));
            }
        }
    }
    if kind == SubmissionKind::Tool {
        let private = state.registration.context.is_some()
            && state.permission(Permission::ReadWorkstationActivity)
            && lane.private_result_available();
        body = body.child(
            Button::new("Refresh authorized tool result")
                .disabled(!private || matches!(lane.result, Load::Loading))
                .on_press(|state: &mut State| state.refresh_tool_result())
                .build(kit)
                .key("tool-result-refresh"),
        );
        if private {
            match &lane.result {
                Load::Idle => body = body.child(quiet("The private result has not been read. Acceptance never means the provider call completed.", kit)),
                Load::Pending | Load::Loading => body = body.child(quiet("Reading the current authorized tool result…", kit).role("status")),
                Load::Failed(_) => {},
                Load::Ready(status) => {
                    body = body.child(heading(format!("Backend tool status: {}", status.status), kit));
                    if let Some(result) = &status.result {
                        body = body.child(quiet(format!("{} · {} · observed {}{}", result.tool_id, result.status, result.observed_at,
                            if result.cached { " · cached result" } else { "" }), kit))
                            .child(line(result.observations.clone(), kit).key("tool-result-observations"));
                        if let Some(error) = &result.error { body = body.child(line(error.clone(), kit).role("alert")); }
                        if result.truncated { body = body.child(quiet("This structured result is bounded; additional provider content is not displayed.", kit)); }
                        if let Some(source) = &result.source { body = body.child(quiet(format!("Source: {source}"), kit)); }
                    } else { body = body.child(quiet("No completed structured result is available yet.", kit)); }
                }
            }
        }
        if let Load::Failed(error) = &lane.result {
            body = body.child(line(format!("Tool result unavailable: {error}"), kit).role("alert"));
        }
    }
    if let Some(message) = &lane.message {
        body = body.child(quiet(message.clone(), kit).role("status"));
    }
    body
}
