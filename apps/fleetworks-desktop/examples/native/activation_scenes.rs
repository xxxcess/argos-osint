//! Synthetic exact-candidate effects through the production activation handlers.

use fleetworks_contracts::classifier_activation::ActivationReceipt;
use fleetworks_contracts::classifier_activation::ActivationRequest;
use fleetworks_contracts::classifier_activation::ActivationResult;
use fleetworks_contracts::classifier_activation::ActivationState;
use fleetworks_contracts::classifier_activation::ActiveModelRevision;
use fleetworks_contracts::classifier_activation::CandidateIdentity;
use fleetworks_contracts::classifier_activation::CandidatePreview;
use fleetworks_contracts::classifier_activation::SyntheticBaselineConsent;
use fleetworks_contracts::context::JobOrigin;
use fleetworks_contracts::context::JobRequestAssociation;
use fleetworks_contracts::context::MutationContext;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::readiness::ReadinessAction;
use fleetworks_contracts::readiness::ReadinessActionId;
use fleetworks_contracts::readiness::ReadinessCheck;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use fleetworks_desktop::app::Intent;
use fleetworks_desktop::app::Load;
use fleetworks_desktop::classifier_activation::ActivationAction;
use fleetworks_desktop::classifier_activation::ActivationAttempt;
use fleetworks_desktop::classifier_activation::ActivationFailure;
use fleetworks_desktop::classifier_activation::ActivationResponse;
use fleetworks_desktop::State;
use rune::TextEdit;

fn candidate() -> CandidateIdentity {
    CandidateIdentity::new(
        "synthetic-natural-candidate".into(),
        "synthetic-training-job".into(),
        "a".repeat(64),
        "b".repeat(64),
        "c".repeat(64),
        "d".repeat(64),
    )
    .expect("synthetic exact hashes")
}
fn setup(state: &mut State) {
    super::maintenance_preview(state);
    state.maintenance.confirmation = None;
    if let Load::Ready(readiness) = &mut state.readiness {
        let mut check = ReadinessCheck::new(
            ReadinessCheckId::ClassifierTraining,
            ReadinessState::Ready,
            "candidate_qualified".into(),
            "Synthetic Natural candidate qualified; real inference remains unverified".into(),
            "2026-01-01T00:00:00Z".into(),
        );
        check.fresh_for_ms = 30_000;
        check.actions.push(ReadinessAction::new(
            ReadinessActionId::ActivateClassifier,
            Permission::MaintainBackend,
            true,
        ));
        readiness.checks.push(check);
    }
    state.classifier_activation.training_job = TextEdit::new("synthetic-training-job");
    state.classifier_activation.candidate_model = TextEdit::new("synthetic-natural-candidate");
}
pub fn preview(state: &mut State) {
    setup(state);
    state.load_activation_candidate();
}
pub fn confirmation(state: &mut State) {
    preview(state);
    state.classifier_activation.consent = true;
    state.preview_candidate_activation();
}
pub fn accepted(state: &mut State) {
    confirmation(state);
    state.confirm_candidate_activation();
}
pub fn uncertain(state: &mut State) {
    fn effect(state: &mut State, intent: &Intent) {
        if matches!(
            intent,
            Intent::ClassifierActivation {
                action: ActivationAction::Result
            }
        ) {
            state.complete_classifier_activation(
                state.classifier_activation.revision(),
                Ok(ActivationResponse::Result(ActivationResult::new(
                    "synthetic-activation-job".into(),
                    candidate(),
                    ActivationState::EffectUncertain,
                    Some(
                        ActiveModelRevision::active(
                            "synthetic-natural-candidate".into(),
                            1,
                            "b".repeat(64),
                        )
                        .expect("matching observed pointer"),
                    ),
                    "owned_completion_not_proven".into(),
                ))),
            );
        } else {
            super::reply(state, intent);
        }
    }
    setup(state);
    state.fixture_effect = Some(effect);
    state.load_activation_candidate();
    state.classifier_activation.consent = true;
    state.preview_candidate_activation();
    state.confirm_candidate_activation();
}
pub fn completed(state: &mut State) {
    fn effect(state: &mut State, intent: &Intent) {
        if matches!(
            intent,
            Intent::ClassifierActivation {
                action: ActivationAction::Result
            }
        ) {
            state.complete_classifier_activation(
                state.classifier_activation.revision(),
                Ok(ActivationResponse::Result(ActivationResult::new(
                    "synthetic-activation-job".into(),
                    candidate(),
                    ActivationState::Activated,
                    Some(
                        ActiveModelRevision::active(
                            "synthetic-natural-candidate".into(),
                            1,
                            "b".repeat(64),
                        )
                        .expect("owned new revision"),
                    ),
                    "operation_owned_completion".into(),
                ))),
            );
        } else {
            super::reply(state, intent);
        }
    }
    setup(state);
    state.fixture_effect = Some(effect);
    state.load_activation_candidate();
    state.classifier_activation.consent = true;
    state.preview_candidate_activation();
    state.confirm_candidate_activation();
}
pub fn acceptance_unknown(state: &mut State) {
    fn effect(state: &mut State, intent: &Intent) {
        if matches!(
            intent,
            Intent::ClassifierActivation {
                action: ActivationAction::Post
            }
        ) {
            state.complete_classifier_activation(
                state.classifier_activation.revision(),
                Err(ActivationFailure {
                    message: "Synthetic activation response lost; exact request retained".into(),
                    uncertain: true,
                }),
            );
        } else {
            super::reply(state, intent);
        }
    }
    setup(state);
    state.fixture_effect = Some(effect);
    state.load_activation_candidate();
    state.classifier_activation.consent = true;
    state.preview_candidate_activation();
    state.confirm_candidate_activation();
}

pub fn reply(state: &mut State, intent: &Intent) -> bool {
    let Intent::ClassifierActivation { action } = intent else {
        return false;
    };
    let response = match action {
        ActivationAction::Preview => ActivationResponse::Preview(CandidatePreview::new(candidate(),ActiveModelRevision::NeverActivated,true,true,"synthetic_qualified".into())),
        ActivationAction::Prepare => {
            let Load::Ready(snapshot) = &state.classifier_activation.preview else { return true; };
            let request = ActivationRequest::new("synthetic-activation-request".into(),MutationContext::new(&snapshot.context).with_configuration_revision(7),
                snapshot.preview.candidate.clone(),snapshot.preview.expected_prior.clone(),SyntheticBaselineConsent::ActivateSyntheticBaseline).expect("synthetic explicit consent request");
            ActivationResponse::Prepared(ActivationAttempt { request,context:snapshot.context.clone(),origin_revision:state.origin_revision() })
        }
        ActivationAction::Post => {
            let attempt = state.classifier_activation.attempt.as_ref().expect("exact activation snapshot");
            let origin = JobOrigin::new(&attempt.context,OriginSource::Native,attempt.request.request_id.clone(),"2026-01-01T00:00:00Z".into(),7,Vec::new());
            ActivationResponse::Receipt(ActivationReceipt::new("synthetic-activation-job".into(),origin,&attempt.request,false))
        }
        ActivationAction::Resolve => {
            let attempt = state.classifier_activation.attempt.as_ref().expect("exact activation lookup");
            ActivationResponse::Association(JobRequestAssociation::new("synthetic-activation-job".into(),JobOrigin::new(&attempt.context,OriginSource::Native,attempt.request.request_id.clone(),"2026-01-01T00:00:00Z".into(),7,Vec::new()),"2026-01-01T00:00:01Z".into()))
        }
        ActivationAction::Verify => ActivationResponse::Job(serde_json::from_value(serde_json::json!({"job":{"id":"synthetic-activation-job","app":"workstation","title":"Synthetic exact candidate activation","operation":"classifier_activate","state":"queued","phase":"queued","progress_done":null,"progress_total":null,"error_summary":"","private_details_available":true,"activity_actions_available":false,"status_only":false},"stages":[],"private_details_available":true,"activity_actions_available":false})).expect("synthetic scoped Jobs detail")),
        ActivationAction::Result => ActivationResponse::Result(ActivationResult::new("synthetic-activation-job".into(),candidate(),ActivationState::Queued,None,"accepted_waiting_for_worker".into())),
    };
    state.complete_classifier_activation(state.classifier_activation.revision(), Ok(response));
    true
}
