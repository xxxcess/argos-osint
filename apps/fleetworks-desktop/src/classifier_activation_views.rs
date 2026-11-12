//! Explicit candidate hashes, expected prior revision and separate consent.

use fleetworks_contracts::classifier_activation::ActivationState;
use fleetworks_contracts::classifier_activation::ActiveModelRevision;
use fleetworks_contracts::context::Permission;
use rune::El;
use rune_kit::Button;
use rune_kit::ButtonVariant;
use rune_kit::Checkbox;
use rune_kit::Kit;
use rune_kit::TextInput;

use crate::app::Load;
use crate::app::State;
use crate::views::heading;
use crate::views::line;
use crate::views::quiet;

pub(crate) fn panel(state: &State, kit: &Kit) -> El {
    let activation = &state.classifier_activation;
    let permitted = state.permission(Permission::MaintainBackend)
        && state.permission(Permission::ReadWorkstationActivity)
        && state.registration.context.is_some();
    let mut body = rune::col().gap(7.0).key("classifier-activation")
        .child(heading("Activate a Natural candidate",kit))
        .child(quiet("Inspect a completed training job's exact candidate. Activation changes a shared backend model; real inference readiness is checked separately.",kit))
        .child(TextInput::new(&activation.training_job).label("Completed training job ID").width(None)
            .on_edit(|state:&mut State| &mut state.classifier_activation.training_job)
            .on_change(|state:&mut State,_| state.classifier_activation.invalidate_confirmation()).build(kit).key("activation-training-job"))
        .child(TextInput::new(&activation.candidate_model).label("Candidate model ID").width(None)
            .on_edit(|state:&mut State| &mut state.classifier_activation.candidate_model)
            .on_change(|state:&mut State,_| state.classifier_activation.invalidate_confirmation()).build(kit).key("activation-candidate-model"))
        .child(Button::new("Inspect candidate").disabled(!permitted || activation.busy || activation.uncertain)
            .on_press(|state:&mut State| state.load_activation_candidate()).build(kit).key("activation-inspect"));
    if !permitted {
        body = body.child(quiet("Register a workspace with maintenance and workstation activity permissions to inspect a candidate.",kit));
        return body;
    }
    match &activation.preview {
        Load::Pending | Load::Loading => {
            body = body.child(quiet("Checking authorized training artifacts…", kit).role("status"))
        }
        Load::Failed(error) => body = body.child(line(error.clone(), kit).role("alert")),
        Load::Ready(snapshot) => {
            let preview = &snapshot.preview;
            let candidate = &preview.candidate;
            body = body
                .child(heading(format!("Candidate {}", candidate.model_id), kit))
                .child(quiet(
                    format!("Training job {}", candidate.training_job_id),
                    kit,
                ));
            for (label, hash) in [
                ("Dataset SHA-256", &candidate.dataset_sha256),
                ("Manifest SHA-256", &candidate.manifest_sha256),
                ("Model SHA-256", &candidate.model_sha256),
                ("Report SHA-256", &candidate.report_sha256),
            ] {
                body = body.child(quiet(format!("{label}: {hash}"), kit));
            }
            body = body.child(quiet(
                format!(
                    "Expected current Natural model: {}",
                    prior(&preview.expected_prior)
                ),
                kit,
            ));
            if preview.synthetic_baseline {
                body = body.child(quiet("This candidate is a synthetic baseline. Review its qualification before granting explicit activation consent.",kit))
                    .child(Checkbox::new(activation.consent).label("I explicitly consent to activate this synthetic baseline")
                        .disabled(activation.busy).on_toggle(|state:&mut State| {
                            state.classifier_activation.consent = !state.classifier_activation.consent;
                            state.classifier_activation.invalidate_confirmation();
                        }).build(kit).key("activation-synthetic-consent"));
            }
            let reason = state
                .activation_disabled_reason()
                .or_else(|| (!preview.activation_supported).then(|| preview.reason_code.clone()))
                .or_else(|| {
                    (preview.synthetic_baseline && !activation.consent)
                        .then(|| "Explicit synthetic baseline consent required".into())
                });
            body = body.child(
                Button::new("Review activation…")
                    .disabled(reason.is_some())
                    .on_press(|state: &mut State| state.preview_candidate_activation())
                    .build(kit)
                    .key("activation-review"),
            );
            if let Some(reason) = reason {
                body = body.child(quiet(reason, kit));
            }
        }
        Load::Idle => {}
    }
    if let Some(confirmation) = &activation.confirmation {
        body = body.child(rune::col().gap(7.0).key("classifier-activation-confirmation")
            .child(heading(format!("Activate {}?",confirmation.candidate.preview.candidate.model_id),kit))
            .child(line(format!("Requested from {} · backend {} · configuration revision {}",confirmation.candidate.context.connect_id,
                confirmation.candidate.context.deployment_id,confirmation.configuration_revision),kit))
            .child(quiet(format!("Replace only the confirmed prior revision: {}. This is separate from training and does not prove inference readiness.",prior(&confirmation.candidate.preview.expected_prior)),kit))
            .child(Button::new("Confirm candidate activation").disabled(activation.busy)
                .on_press(|state:&mut State| state.confirm_candidate_activation()).build(kit).key("activation-confirm"))
            .child(Button::new("Keep current model").variant(ButtonVariant::Ghost)
                .on_press(|state:&mut State| state.classifier_activation.confirmation = None).build(kit).key("activation-keep-current")));
    }
    if activation.uncertain {
        body = body.child(line("Activation acceptance is unknown. Recheck this exact request; a changed draft never replaces it.",kit).role("status"))
            .child(Button::new("Retry exact activation request").disabled(activation.busy || !permitted)
                .on_press(|state:&mut State| state.retry_exact_activation()).build(kit).key("activation-retry-exact"));
    }
    if activation.attempt.is_some() {
        body = body.child(
            Button::new("Look up original activation request")
                .disabled(activation.busy || !permitted)
                .on_press(|state: &mut State| state.resolve_activation_request())
                .build(kit)
                .key("activation-request-lookup"),
        );
    }
    if let Load::Ready(receipt) = &activation.receipt {
        body = body.child(line(
            format!(
                "{} activation job {} · accepted by backend",
                if receipt.reused {
                    "Associated with"
                } else {
                    "Created"
                },
                receipt.job_id
            ),
            kit,
        ));
    }
    if let Load::Ready(job) = &activation.job {
        let id = job.job.id.clone();
        body = body
            .child(quiet(
                format!("Job {} · last observed {}", id, job.job.state),
                kit,
            ))
            .child(
                Button::new("View activation job")
                    .on_press(move |state: &mut State| state.open_job(&id))
                    .build(kit)
                    .key("activation-view-job"),
            )
            .child(
                Button::new("Refresh activation result")
                    .disabled(activation.busy || !permitted)
                    .on_press(|state: &mut State| state.refresh_activation_result())
                    .build(kit)
                    .key("activation-result-refresh"),
            );
    }
    match &activation.result {
        Load::Pending | Load::Loading => {
            body = body
                .child(quiet("Checking the activation job's effect receipt…", kit).role("status"))
        }
        Load::Failed(error) => body = body.child(line(error.clone(), kit).role("alert")),
        Load::Ready(result) => {
            let label = match result.state {
                ActivationState::Queued => "Activation queued",
                ActivationState::Running => "Activation running",
                ActivationState::Activated => "Natural candidate activated by this operation",
                ActivationState::Rejected => "Activation rejected",
                ActivationState::Interrupted => "Activation interrupted",
                ActivationState::EffectUncertain => {
                    "Activation effect uncertain; no owned success is proven"
                }
                _ => "Activation state unknown",
            };
            body = body.child(line(label, kit).role("status"));
            if result.state == ActivationState::Activated {
                if let Some(revision) = &result.activated_revision {
                    body = body.child(quiet(prior(revision), kit));
                }
                body = body.child(quiet(
                    "Real inference readiness remains separate from this model activation.",
                    kit,
                ));
            }
            if !result.reason_code.is_empty()
                && result.reason_code.len() <= 256
                && !result.reason_code.chars().any(char::is_control)
            {
                body = body.child(quiet(result.reason_code.clone(), kit));
            }
        }
        Load::Idle => {}
    }
    if let Some(message) = &activation.message {
        body = body.child(line(message.clone(), kit).role("status"));
    }
    body
}

fn prior(revision: &ActiveModelRevision) -> String {
    match revision {
        ActiveModelRevision::NeverActivated => "No Natural model has been activated".into(),
        ActiveModelRevision::Active {
            model_id,
            generation,
            manifest_sha256,
        } => format!("{model_id} · generation {generation} · manifest SHA-256 {manifest_sha256}"),
        _ => "Unknown prior revision".into(),
    }
}
