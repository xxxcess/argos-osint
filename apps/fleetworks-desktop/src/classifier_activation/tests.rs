use fleetworks_contracts::capabilities::Capabilities;
use fleetworks_contracts::classifier_activation::CandidateIdentity;
use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::context::JobRequestAssociation;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::readiness::ReadinessAction;
use fleetworks_contracts::readiness::ReadinessCheck;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use fleetworks_contracts::readiness::WorkstationReadiness;
use rune::Env;
use rune::Runtime;

use super::*;
use crate::app;
use crate::backend::ProfileSnapshot;
use crate::connection::ConnectionStatus;

fn candidate() -> CandidateIdentity {
    CandidateIdentity::new(
        "synthetic-candidate".into(),
        "synthetic-training".into(),
        "a".repeat(64),
        "b".repeat(64),
        "c".repeat(64),
        "d".repeat(64),
    )
    .expect("immutable candidate hashes")
}
fn context(registration: &str) -> RegisteredContext {
    RegisteredContext::new(
        RegistrationReceipt::new(
            "synthetic-deployment".into(),
            "synthetic-node".into(),
            registration.into(),
            Some("192.0.2.42".parse().expect("synthetic IP")),
            1,
        ),
        1,
        1,
        "synthetic-config".into(),
        "synthetic-access".into(),
    )
    .expect("registered synthetic context")
}
fn fixture() -> State {
    let mut state = State::fixture();
    state.page = Page::Workstation;
    state.connection.status = ConnectionStatus::Connected;
    let mut capabilities = Capabilities::default();
    capabilities.deployment_id = Some("synthetic-deployment".into());
    capabilities.permissions = vec![
        Permission::ReadBackend,
        Permission::MaintainBackend,
        Permission::ReadWorkstationActivity,
    ];
    state.connection.capabilities = Some(capabilities);
    let context = context("synthetic-registration");
    let mut check = ReadinessCheck::new(
        ReadinessCheckId::ClassifierTraining,
        ReadinessState::Ready,
        "candidate_qualified".into(),
        "Synthetic candidate available; inference remains unverified".into(),
        "2026-01-01T00:00:00Z".into(),
    );
    check.fresh_for_ms = 30_000;
    check.actions.push(ReadinessAction::new(
        ReadinessActionId::ActivateClassifier,
        Permission::MaintainBackend,
        true,
    ));
    let mut readiness = WorkstationReadiness::new(
        context.deployment_id.clone(),
        "2026-01-01T00:00:00Z".into(),
        vec![check],
    );
    readiness.registration = Some(RegistrationReceipt::new(
        context.deployment_id.clone(),
        context.workstation_id.clone(),
        context.registration_id.clone(),
        context.workstation_ip,
        context.registration_revision,
    ));
    state.readiness = Load::Ready(readiness);
    state.readiness_received_at = Some(0.0);
    state.registration.context = Some(context);
    state.backend.profile = Load::Ready(ProfileSnapshot {
        configuration_revision: 0,
        configuration_format: 1,
        accounts: Vec::new(),
        tool_credentials: Vec::new(),
        setup_required: true,
        portable_schema_version: 4,
    });
    state.classifier_activation.training_job = TextEdit::new("synthetic-training");
    state.classifier_activation.candidate_model = TextEdit::new("synthetic-candidate");
    state.fixture_effect = Some(reply);
    state
}

fn reply(state: &mut State, intent: &Intent) {
    let Intent::ClassifierActivation { action } = intent else {
        return;
    };
    let revision = state.classifier_activation.revision();
    let response = match action {
        ActivationAction::Preview => ActivationResponse::Preview(CandidatePreview::new(candidate(),ActiveModelRevision::NeverActivated,true,true,"synthetic_qualified".into())),
        ActivationAction::Prepare => {
            let Load::Ready(snapshot) = &state.classifier_activation.preview else { panic!("confirmed exact preview") };
            let context = snapshot.context.clone();
            let request = ActivationRequest::new("synthetic-original-request".into(),MutationContext::new(&context).with_configuration_revision(state.configuration_revision().expect("explicit revision0 is observed")),
                snapshot.preview.candidate.clone(),snapshot.preview.expected_prior.clone(),SyntheticBaselineConsent::ActivateSyntheticBaseline).expect("valid explicit consent request");
            ActivationResponse::Prepared(ActivationAttempt { request, context, origin_revision:state.origin_revision })
        }
        ActivationAction::Post => {
            let attempt = state.classifier_activation.attempt.as_ref().expect("same exact attempt");
            let origin = JobOrigin::new(&attempt.context,OriginSource::Native,attempt.request.request_id.clone(),"2026-01-01T00:00:00Z".into(),attempt.request.context.expected_configuration_revision.expect("observed snapshot"),Vec::new());
            ActivationResponse::Receipt(ActivationReceipt::new("opaque-server-activation-job".into(),origin,&attempt.request,false))
        }
        ActivationAction::Resolve => {
            let attempt = state.classifier_activation.attempt.as_ref().expect("immutable original lookup");
            ActivationResponse::Association(JobRequestAssociation::new("opaque-server-activation-job".into(),JobOrigin::new(&attempt.context,OriginSource::Native,attempt.request.request_id.clone(),"2026-01-01T00:00:00Z".into(),attempt.request.context.expected_configuration_revision.expect("original configuration"),Vec::new()),"2026-01-01T00:00:01Z".into()))
        }
        ActivationAction::Verify => ActivationResponse::Job(serde_json::from_value(serde_json::json!({"job":{"id":"opaque-server-activation-job","app":"workstation","title":"Synthetic explicit activation","operation":"classifier_activate","state":"completed","phase":"completed","progress_done":1,"progress_total":1,"error_summary":"","private_details_available":true,"activity_actions_available":false,"status_only":false},"stages":[],"private_details_available":true,"activity_actions_available":false})).expect("authorized actual Jobs DTO")),
        ActivationAction::Result => ActivationResponse::Result(ActivationResult::new("opaque-server-activation-job".into(),candidate(),ActivationState::Activated,
            Some(ActiveModelRevision::active("synthetic-candidate".into(),1,"b".repeat(64)).expect("new exact active revision")),"operation_owned_completion".into())),
    };
    state.complete_classifier_activation(revision, Ok(response));
}

fn prepare_and_confirm(state: &mut State) {
    state.load_activation_candidate();
    state.classifier_activation.consent = true;
    state.preview_candidate_activation();
    state.confirm_candidate_activation();
}

fn lose_post(state: &mut State, intent: &Intent) {
    if matches!(
        intent,
        Intent::ClassifierActivation {
            action: ActivationAction::Post
        }
    ) {
        state.complete_classifier_activation(
            state.classifier_activation.revision(),
            Err(ActivationFailure {
                message: "Synthetic response lost".into(),
                uncertain: true,
            }),
        );
    } else {
        reply(state, intent);
    }
}

#[test]
fn retained_activation_handler_requires_exact_consent_and_native_never_exposes_activation() {
    let mut state = fixture();
    state.load_activation_candidate();
    state.preview_candidate_activation();
    assert!(
        state.classifier_activation.confirmation.is_none(),
        "training never implies activation consent"
    );
    state.classifier_activation.consent = true;
    state.preview_candidate_activation();
    let confirmation = state
        .classifier_activation
        .confirmation
        .as_ref()
        .expect("explicit candidate confirmation");
    assert_eq!(confirmation.candidate.preview.candidate, candidate());
    assert!(!state.intents.iter().any(|intent| matches!(
        intent,
        Intent::ClassifierActivation {
            action: ActivationAction::Post
        }
    )));
    state.confirm_candidate_activation();
    assert!(
        matches!(state.classifier_activation.result,Load::Ready(ref result) if result.state==ActivationState::Activated)
    );
    assert!(
        matches!(state.classifier_activation.job,Load::Ready(ref detail) if detail.job.id=="opaque-server-activation-job")
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
        "activation-synthetic-consent",
        "activation-review",
        "activation-confirm",
    ] {
        assert!(
            runtime.find(key).is_none(),
            "activation control {key} is excluded from native UI"
        );
    }
    assert!(!runtime
        .accessibility_json()
        .to_string()
        .contains("inference: Ready"));
}

#[test]
fn uncertain_activation_retry_keeps_original_hashes_nonce_and_revision_zero() {
    let mut state = fixture();
    state.fixture_effect = Some(lose_post);
    prepare_and_confirm(&mut state);
    let original = state
        .classifier_activation
        .attempt
        .as_ref()
        .expect("retained request")
        .request
        .clone();
    assert_eq!(original.context.expected_configuration_revision, Some(0));
    state.classifier_activation.candidate_model = TextEdit::new("different-edited-draft");
    if let Load::Ready(profile) = &mut state.backend.profile {
        profile.configuration_revision = 2;
    }
    state.fixture_effect = Some(reply);
    state.retry_exact_activation();
    assert_eq!(
        state
            .classifier_activation
            .attempt
            .as_ref()
            .expect("same attempt")
            .request,
        original
    );
    assert!(matches!(
        state.classifier_activation.receipt,
        Load::Ready(_)
    ));
}

#[test]
fn changed_registration_uses_read_only_original_request_lookup() {
    let mut state = fixture();
    state.fixture_effect = Some(lose_post);
    prepare_and_confirm(&mut state);
    let posts = state
        .intents
        .iter()
        .filter(|intent| {
            matches!(
                intent,
                Intent::ClassifierActivation {
                    action: ActivationAction::Post
                }
            )
        })
        .count();
    state.invalidate_origin();
    state.registration.context = Some(context("replacement-registration"));
    state.retry_exact_activation();
    assert_eq!(
        state
            .intents
            .iter()
            .filter(|intent| matches!(
                intent,
                Intent::ClassifierActivation {
                    action: ActivationAction::Post
                }
            ))
            .count(),
        posts
    );
    state.fixture_effect = Some(reply);
    state.resolve_activation_request();
    assert!(matches!(
        state.classifier_activation.association,
        Load::Ready(_)
    ));
    assert!(
        matches!(state.classifier_activation.receipt, Load::Idle),
        "read lookup never fabricates a lost POST receipt"
    );
}

#[test]
fn matching_current_pointer_does_not_turn_effect_uncertain_into_owned_success() {
    fn uncertain_effect(state: &mut State, intent: &Intent) {
        if matches!(
            intent,
            Intent::ClassifierActivation {
                action: ActivationAction::Result
            }
        ) {
            state.complete_classifier_activation(
                state.classifier_activation.revision(),
                Ok(ActivationResponse::Result(ActivationResult::new(
                    "opaque-server-activation-job".into(),
                    candidate(),
                    ActivationState::EffectUncertain,
                    Some(
                        ActiveModelRevision::active(
                            "synthetic-candidate".into(),
                            1,
                            "b".repeat(64),
                        )
                        .expect("matching observed pointer"),
                    ),
                    "owned_completion_not_proven".into(),
                ))),
            );
        } else {
            reply(state, intent);
        }
    }
    let mut state = fixture();
    state.fixture_effect = Some(uncertain_effect);
    prepare_and_confirm(&mut state);
    let mut runtime = Runtime::new(
        state,
        app::view,
        (1280.0, 3200.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    runtime.frame(0.0);
    let tree = runtime.accessibility_json().to_string();
    assert!(
        matches!(&runtime.state.classifier_activation.result,Load::Ready(result) if result.state==ActivationState::EffectUncertain),
        "matching active pointer cannot turn uncertain operation ownership into success"
    );
    assert!(!tree.contains("activated by this operation"));
}

#[test]
fn stale_configuration_and_permission_fence_new_activation() {
    let mut state = fixture();
    state.load_activation_candidate();
    state.classifier_activation.consent = true;
    state.preview_candidate_activation();
    if let Load::Ready(profile) = &mut state.backend.profile {
        profile.configuration_revision = 1;
    }
    state.confirm_candidate_activation();
    assert!(!state.intents.iter().any(|intent| matches!(
        intent,
        Intent::ClassifierActivation {
            action: ActivationAction::Post
        }
    )));
    state
        .connection
        .capabilities
        .as_mut()
        .expect("fixture permissions")
        .permissions
        .clear();
    assert!(state.activation_disabled_reason().is_some());
}

#[test]
fn mismatched_origin_receipt_and_nonadvancing_activation_revision_are_rejected() {
    let mut state = fixture();
    state.fixture_effect = Some(lose_post);
    prepare_and_confirm(&mut state);
    let attempt = state
        .classifier_activation
        .attempt
        .as_ref()
        .expect("original attempt")
        .clone();
    let mut origin = JobOrigin::new(
        &attempt.context,
        OriginSource::Native,
        attempt.request.request_id.clone(),
        "2026-01-01T00:00:00Z".into(),
        0,
        Vec::new(),
    );
    origin.workstation_id = "another-node".into();
    state.complete_classifier_activation(
        state.classifier_activation.revision(),
        Ok(ActivationResponse::Receipt(ActivationReceipt::new(
            "opaque-other-job".into(),
            origin,
            &attempt.request,
            false,
        ))),
    );
    assert!(!matches!(
        state.classifier_activation.receipt,
        Load::Ready(_)
    ));
    let prior =
        ActiveModelRevision::active("prior".into(), 5, "e".repeat(64)).expect("exact prior");
    let result = ActivationResult::new(
        "opaque-server-activation-job".into(),
        candidate(),
        ActivationState::Activated,
        Some(
            ActiveModelRevision::active("synthetic-candidate".into(), 5, "b".repeat(64))
                .expect("nonadvancing pointer"),
        ),
        "not_new".into(),
    );
    assert!(!result_matches(&result, &prior));
}
