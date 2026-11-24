use fleetworks_contracts::capabilities::Capabilities;
use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::context::JobOrigin;
use fleetworks_contracts::context::JobRequestAssociation;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::readiness::AtlasCyclePreflight;
use fleetworks_contracts::readiness::AtlasKeyPresence;
use fleetworks_contracts::readiness::ReadinessState;
use fleetworks_contracts::readiness::WorkstationReadiness;
use fleetworks_workstation::native::BackendKind;
use rune::Env;
use rune::Runtime;
use rune::TextEdit;

use super::AcceptedReceipt;
use super::SubmissionAttempt;
use super::SubmissionFailure;
use super::SubmissionInput;
use super::SubmissionKind;
use super::SubmissionLane;
use super::SubmissionPhase;
use super::ToolReceipt;
use super::ToolResultView;
use super::ToolStatusView;
use crate::app;
use crate::app::Intent;
use crate::app::Load;
use crate::app::Page;
use crate::app::State;
use crate::backend::ActivityJob;
use crate::backend::JobDetail;
use crate::backend::ProfileSnapshot;
use crate::backend::ToolCatalog;
use crate::backend::ToolCredentialMetadata;
use crate::backend::ToolDefinition;
use crate::configuration::ConfigurationAction;
use crate::configuration::ConfigurationSaved;
use crate::connection::ConnectionStatus;

fn fixture() -> State {
    let mut state = State::fixture();
    state.page = Page::Tools;
    state.connection.status = ConnectionStatus::Connected;
    let mut capabilities = Capabilities::default();
    capabilities.permissions = vec![
        Permission::ReadBackend,
        Permission::SubmitInteractive,
        Permission::ReadWorkstationActivity,
    ];
    capabilities.features = vec!["atlas-cycles".into(), "tool-execution".into()];
    capabilities.deployment_id = Some("synthetic-deployment".into());
    capabilities.configuration_epoch = Some("synthetic-config".into());
    state.connection.capabilities = Some(capabilities);
    let receipt = RegistrationReceipt::new(
        "synthetic-deployment".into(),
        "synthetic-node".into(),
        "synthetic-registration".into(),
        Some("192.0.2.42".parse().expect("synthetic IP")),
        1,
    );
    state.registration.context = Some(
        RegisteredContext::new(
            receipt.clone(),
            1,
            1,
            "synthetic-config".into(),
            "synthetic-access".into(),
        )
        .expect("synthetic registered context"),
    );
    let mut readiness = WorkstationReadiness::new(
        "synthetic-deployment".into(),
        "2026-01-01T00:00:00Z".into(),
        Vec::new(),
    );
    readiness.registration = Some(receipt);
    let mut preflight = AtlasCyclePreflight::new(
        ReadinessState::Ready,
        ReadinessState::Ready,
        AtlasKeyPresence::Saved,
        AtlasKeyPresence::Saved,
        AtlasKeyPresence::Saved,
        AtlasKeyPresence::Saved,
    );
    preflight.configuration_revision = Some(0);
    readiness.atlas_preflight = Some(preflight);
    state.complete_readiness(Ok(readiness));
    state.backend.profile = Load::Ready(ProfileSnapshot {
        configuration_revision: 0,
        configuration_format: 1,
        accounts: Vec::new(),
        tool_credentials: Vec::new(),
        setup_required: true,
        portable_schema_version: 4,
    });
    state.backend.tools = Load::Ready(ToolCatalog {
        tools: vec![ToolDefinition {
            id: "synthetic-tool".into(),
            name: "Synthetic lookup".into(),
            category: "Fixture".into(),
            description: "No provider call".into(),
            inputs: vec!["email".into()],
            restrictions: "Fixture only".into(),
            timeout_seconds: 30,
        }],
    });
    state.backend.selected_tool = Some("synthetic-tool".into());
    state.backend.tool_inputs = TextEdit::new("{\"email\":\"fixture@example.test\"}");
    state.fixture_effect = Some(reply);
    state
}

fn detail(id: &str, kind: SubmissionKind, private: bool) -> JobDetail {
    JobDetail {
        job: ActivityJob {
            id: id.into(),
            app: "synthetic".into(),
            title: "Synthetic server-owned job".into(),
            operation: kind.operation().into(),
            state: "queued".into(),
            phase: "queued".into(),
            progress_done: None,
            progress_total: None,
            error_summary: String::new(),
            private_details_available: private,
            activity_actions_available: false,
            status_only: !private,
            can_cancel: false,
            can_retry: false,
            action_reason_code: "not_observed".into(),
        },
        maintenance_result: None,
        stages: Vec::new(),
        private_details_available: private,
        activity_actions_available: false,
    }
}

fn receipt(kind: SubmissionKind) -> AcceptedReceipt {
    match kind {
        SubmissionKind::Atlas => AcceptedReceipt::Atlas(serde_json::from_value(serde_json::json!({"job_id":"opaque-server-atlas-job","run_id":"opaque-server-run"})).expect("reported fixture receipt")),
        SubmissionKind::Tool => AcceptedReceipt::Tool(ToolReceipt { call_id: "opaque-server-tool-job".into(), job_id: Some("opaque-server-tool-job".into()) }),
    }
}

fn reply(state: &mut State, intent: &Intent) {
    match intent {
        Intent::SubmitInteractive { kind } => {
            let lane = state.submissions.lane(*kind);
            let snapshot = lane
                .confirmation
                .clone()
                .expect("same-handler confirmed snapshot");
            let revision = lane.revision();
            let attempt = SubmissionAttempt::new("synthetic-client-request".into(), snapshot)
                .expect("synthetic request identity");
            state.complete_submission_preparation(*kind, revision, Ok(attempt));
        }
        Intent::PostInteractive { kind } => {
            let revision = state.submissions.lane(*kind).revision();
            state.complete_submission_post(*kind, revision, Ok(receipt(*kind)));
        }
        Intent::VerifyInteractive { kind } => {
            let lane = state.submissions.lane(*kind);
            let id = lane
                .job_candidate()
                .expect("reported or resolved fixture job")
                .to_owned();
            let revision = lane.revision();
            state.complete_submission_verification(*kind, revision, Ok(detail(&id, *kind, true)));
        }
        Intent::InteractiveToolResult => {
            let revision = state.submissions.tools.revision();
            state.complete_interactive_tool_result(
                revision,
                Ok(ToolStatusView {
                    status: "succeeded".into(),
                    result: Some(ToolResultView {
                        tool_id: "synthetic-tool".into(),
                        status: "ok".into(),
                        observed_at: "2026-01-01T00:00:00Z".into(),
                        observations: "Synthetic structured observation".into(),
                        error: None,
                        cached: false,
                        truncated: false,
                        source: None,
                    }),
                }),
            );
        }
        Intent::ResolveInteractive { kind } => {
            let lane = state.submissions.lane(*kind);
            let revision = lane.revision();
            let attempt = lane.attempt.as_ref().expect("original request reference");
            let origin = JobOrigin::new(
                &attempt.snapshot.context,
                OriginSource::Native,
                attempt.request_id.clone(),
                "2026-01-01T00:00:00Z".into(),
                attempt.snapshot.configuration_revision,
                Vec::new(),
            );
            let association = JobRequestAssociation::new(
                receipt(*kind).job_candidate().to_owned(),
                origin,
                "2026-01-01T00:00:00Z".into(),
            );
            state.complete_submission_resolution(*kind, revision, Ok(association));
        }
        _ => {}
    }
}

fn uncertain_once(state: &mut State, intent: &Intent) {
    if let Intent::PostInteractive { kind } = intent {
        let posts = state
            .intents
            .iter()
            .filter(|intent| matches!(intent, Intent::PostInteractive { .. }))
            .count();
        if posts == 1 {
            let revision = state.submissions.lane(*kind).revision();
            state.complete_submission_post(
                *kind,
                revision,
                Err(SubmissionFailure::Uncertain(
                    "Synthetic response lost".into(),
                )),
            );
            return;
        }
    }
    reply(state, intent);
}

#[test]
fn explicit_server_revision_zero_allows_fresh_setup_but_missing_revision_does_not() {
    let mut state = fixture();
    assert_eq!(state.configuration_revision(), Some(0));
    state.preview_atlas_cycle();
    assert!(state.submissions.atlas.confirmation.is_some());
    state.submissions.atlas.confirmation = None;
    state.backend.profile = Load::Idle;
    assert_eq!(state.configuration_revision(), None);
    state.preview_atlas_cycle();
    assert!(state.submissions.atlas.confirmation.is_none());
    assert!(
        state.intents.is_empty(),
        "unknown configuration cannot create a request"
    );
}

#[test]
fn uncertain_retry_keeps_identity_inputs_and_original_config_after_draft_changes() {
    let mut state = fixture();
    state.fixture_effect = Some(uncertain_once);
    state.preview_tool_run();
    state.confirm_submission(SubmissionKind::Tool);
    assert_eq!(
        state.submissions.tools.phase,
        SubmissionPhase::OutcomeUnknown
    );
    let request_id = state
        .submissions
        .tools
        .attempt
        .as_ref()
        .expect("retained uncertain request")
        .request_id
        .clone();
    state.backend.tool_inputs = TextEdit::new("{\"email\":\"changed@example.test\"}");
    state.interactive_draft_edited(SubmissionKind::Tool);
    if let Load::Ready(profile) = &mut state.backend.profile {
        profile.configuration_revision = 1;
    }
    state.retry_submission(SubmissionKind::Tool);
    let attempt = state
        .submissions
        .tools
        .attempt
        .as_ref()
        .expect("same reconciled request");
    assert_eq!(attempt.request_id, request_id);
    assert_eq!(
        attempt.mutation_context().expected_configuration_revision,
        Some(0)
    );
    assert!(
        matches!(&attempt.snapshot.input, SubmissionInput::Tool { inputs, .. } if inputs.get("email").and_then(serde_json::Value::as_str) == Some("fixture@example.test")),
        "replay cannot read an edited draft"
    );
    assert_eq!(state.submissions.tools.phase, SubmissionPhase::Verified);
    assert!(matches!(
        state.submissions.tools.verification,
        Load::Ready(_)
    ));
}

#[test]
fn later_rejection_does_not_erase_an_earlier_uncertain_outcome() {
    let mut state = fixture();
    state.preview_tool_run();
    let snapshot = state
        .submissions
        .tools
        .confirmation
        .take()
        .expect("confirmed draft");
    let mut lane = SubmissionLane {
        attempt: Some(
            SubmissionAttempt::new("synthetic-client-request".into(), snapshot)
                .expect("request identity"),
        ),
        ..SubmissionLane::default()
    };
    lane.observe_post(Err(SubmissionFailure::Uncertain(
        "Synthetic timeout".into(),
    )))
    .expect("record uncertainty");
    lane.observe_post(Err(SubmissionFailure::Rejected(
        "Synthetic forbidden replay".into(),
    )))
    .expect("record retry rejection");
    assert!(lane.delivery_uncertain);
    assert_eq!(lane.phase, SubmissionPhase::OutcomeUnknown);
    assert!(lane.receipt.is_none());
}

#[test]
fn receipt_requires_matching_authorized_job_and_private_eligibility_for_results() {
    let mut state = fixture();
    state.preview_tool_run();
    state.confirm_submission(SubmissionKind::Tool);
    let lane = &mut state.submissions.tools;
    lane.observe_verification(Ok(detail("different-job", SubmissionKind::Tool, true)));
    assert!(matches!(lane.verification, Load::Failed(_)));
    assert!(!lane.private_result_available());
    lane.observe_verification(Ok(detail(
        "opaque-server-tool-job",
        SubmissionKind::Tool,
        false,
    )));
    assert!(!lane.private_result_available());
    let before = state.intents.len();
    state.refresh_tool_result();
    assert_eq!(
        state.intents.len(),
        before,
        "shared private results must not be requested"
    );
}

#[test]
fn production_cycle_confirmation_uses_shared_handlers_and_navigation_never_resubmits() {
    let mut state = fixture();
    state.page = Page::Atlas;
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
    assert!(runtime.find("tool-submit-preview").is_none());
    assert!(runtime.find("atlas-submit-confirm").is_none());
    for (step, key) in ["atlas-submit-preview", "atlas-submit-confirm"]
        .into_iter()
        .enumerate()
    {
        runtime.update(|_, tx| tx.scroll_into_view(key));
        runtime.frame((step * 2 + 1) as f64 / 60.0);
        let (x, y) = runtime
            .find(key)
            .expect("allowlisted production cycle control");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame((step * 2 + 2) as f64 / 60.0);
        if step == 0 {
            assert!(
                !runtime
                    .state
                    .intents
                    .iter()
                    .any(|intent| matches!(intent, Intent::PostInteractive { .. })),
                "review is not consent to submit"
            );
        }
    }
    assert_eq!(
        runtime.state.submissions.atlas.phase,
        SubmissionPhase::Verified
    );
    assert!(matches!(
        runtime.state.submissions.atlas.receipt,
        Some(AcceptedReceipt::Atlas(_))
    ));
    assert!(
        runtime
            .accessibility_json()
            .to_string()
            .contains("Completion pending"),
        "accepted receipts never invent completion"
    );
    runtime.update(|state, _| state.navigate(Page::Tools));
    runtime.frame(6.0 / 60.0);
    runtime.update(|state, _| state.navigate(Page::Atlas));
    runtime.frame(7.0 / 60.0);
    let posts = runtime
        .state
        .intents
        .iter()
        .filter(|intent| matches!(intent, Intent::PostInteractive { .. }))
        .count();
    assert_eq!(
        posts, 1,
        "revisiting the destination never resubmits accepted work"
    );
}

#[test]
fn distributed_atlas_prerequisites_gate_creation_and_open_the_missing_key_editor() {
    let mut state = fixture();
    state.page = Page::Atlas;
    state
        .connection
        .capabilities
        .as_mut()
        .unwrap()
        .permissions
        .push(Permission::ManageConfiguration);
    let Load::Ready(readiness) = &mut state.readiness else {
        panic!("registered readiness fixture");
    };
    let preflight = readiness.atlas_preflight.as_mut().unwrap();
    preflight.rust_bert = ReadinessState::NotReady;
    preflight.currents = AtlasKeyPresence::Missing;
    let mut runtime = Runtime::new(state, app::view, (640.0, 480.0), Env::default());
    runtime.reduced_motion = true;
    runtime.frame(0.0);
    let tree = runtime.accessibility_json().to_string();
    assert!(tree.contains("Ask the gateway operator to check the Rust-BERT worker and checkpoint"));
    assert!(tree.contains("Configure the Currents API key in Tool Box"));
    assert!(tree.contains("Saved keys still need provider authentication"));

    runtime.update(|state, _| {
        let Load::Ready(readiness) = &mut state.readiness else {
            panic!("registered readiness fixture");
        };
        let preflight = readiness.atlas_preflight.as_mut().unwrap();
        preflight.rust_bert = ReadinessState::Ready;
    });
    runtime.frame(0.021);
    assert!(runtime
        .accessibility_json()
        .to_string()
        .contains("Add a Currents API key using Configure API keys"));
    runtime.update(|state, tx| {
        state.preview_atlas_cycle();
        tx.scroll_into_view("atlas-key-open-currents");
    });
    runtime.frame(0.042);
    assert!(runtime.state.submissions.atlas.confirmation.is_none());
    assert!(!runtime.state.intents.iter().any(|intent| matches!(
        intent,
        Intent::SubmitInteractive { .. } | Intent::PostInteractive { .. }
    )));

    let (x, y) = runtime
        .find("atlas-key-open-currents")
        .expect("missing key link");
    assert!((0.0..640.0).contains(&x) && (0.0..480.0).contains(&y));
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(0.063);
    assert_eq!(runtime.state.page, Page::Tools);
    assert!(runtime.state.tool_key_dialog);
    assert_eq!(runtime.state.configuration.tool_provider.text, "currents");
}

#[test]
fn embedded_atlas_links_to_setup_without_stopping_the_companion() {
    let mut state = fixture();
    state.page = Page::Atlas;
    state.backend_kind = BackendKind::Local;
    state.connection.origin = TextEdit::new("local://embedded");
    let mut runtime = Runtime::new(state, app::view, (640.0, 480.0), Env::default());
    runtime.reduced_motion = true;
    runtime.frame(0.0);
    assert!(runtime
        .accessibility_json()
        .to_string()
        .contains(super::ATLAS_DISTRIBUTED_REQUIREMENT));
    assert!(runtime.find("atlas-prerequisites").is_none());
    let (x, y) = runtime
        .find("atlas-open-distributed-setup")
        .expect("Distributed setup link");
    assert!((0.0..640.0).contains(&x) && (0.0..480.0).contains(&y));
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(0.021);
    assert!(runtime.state.settings_open);
    assert!(!runtime.state.setup_gateway);
    assert_eq!(runtime.state.backend_kind, BackendKind::Local);
    assert!(runtime.state.connected());
    assert!(runtime.find("switch-remote-gateway").is_some());
    assert!(runtime
        .accessibility_json()
        .to_string()
        .contains(super::ATLAS_DISTRIBUTED_REQUIREMENT));
}

#[test]
fn atlas_provider_keys_save_in_news_cycles_and_wait_for_current_readiness() {
    let mut state = fixture();
    state.page = Page::Atlas;
    state.clock = 0.0;
    state
        .connection
        .capabilities
        .as_mut()
        .unwrap()
        .permissions
        .push(Permission::ManageConfiguration);
    let Load::Ready(profile) = &mut state.backend.profile else {
        panic!("configuration fixture")
    };
    profile.tool_credentials = ["gnews", "newsdata", "newsapi", "currents"]
        .into_iter()
        .map(|provider| ToolCredentialMetadata {
            provider: provider.into(),
            primary_present: provider != "currents",
            fallback_present: false,
        })
        .collect();
    let Load::Ready(ready) = &mut state.readiness else {
        panic!("readiness fixture")
    };
    ready.atlas_preflight.as_mut().unwrap().currents = AtlasKeyPresence::Missing;

    let mut runtime = Runtime::new(state, app::view, (640.0, 480.0), Env::default());
    runtime.reduced_motion = true;
    runtime.frame(0.0);
    runtime.frame(0.01);
    assert!(runtime
        .state
        .submission_disabled_reason(SubmissionKind::Atlas)
        .is_some());

    let opener = "atlas-configure-provider-keys";
    for key in [opener, "atlas-preflight-refresh", "atlas-submit-preview"] {
        let visible = runtime
            .elements()
            .into_iter()
            .find(|element| element.key.as_deref() == Some(key))
            .unwrap();
        assert_eq!(
            visible.visible, visible.rect,
            "{key} must fit the compact News Cycles viewport"
        );
    }
    runtime.update(|_, tx| tx.focus(opener, true));
    runtime.frame(0.021);
    runtime.key(rune::KeyEvent {
        key: rune::Key::Text(" ".into()),
        mods: rune::Mods::default(),
    });
    runtime.frame(0.042);
    assert!(runtime.state.tool_key_dialog);
    for key in ["gnews", "newsdata", "newsapi", "currents"] {
        let editor = format!("atlas-key-edit-{key}");
        let element = runtime
            .elements()
            .into_iter()
            .find(|el| el.key.as_deref() == Some(editor.as_str()))
            .expect("provider edit action");
        assert_eq!(
            element.visible, element.rect,
            "{key} editor action must fit the compact dialog"
        );
        assert!(element.focusable);
    }

    fn click(runtime: &mut Runtime<State>, key: &str, at: f64) {
        runtime.update(|_, tx| tx.scroll_into_view(key));
        runtime.frame(at);
        let (x, y) = runtime.find(key).expect("visible credential control");
        assert!((0.0..640.0).contains(&x) && (0.0..480.0).contains(&y));
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(at + 0.021);
    }
    click(&mut runtime, "atlas-key-edit-currents", 0.063);
    runtime.update(|state, _| {
        state
            .configuration
            .tool_fallback
            .replace_text("fixture-key-never-visible");
        state.configuration_edited();
    });
    runtime.frame(0.105);
    let tree = runtime.accessibility_json().to_string();
    assert!(tree.contains("Primary API key") && tree.contains("Fallback API key"));
    assert!(!tree.contains("fixture-key-never-visible"));
    click(&mut runtime, "tool-credentials-preview", 0.126);
    assert!(runtime.state.configuration.confirmation.is_some());
    assert!(!runtime
        .state
        .intents
        .iter()
        .any(|intent| matches!(intent, Intent::Configuration { .. })));
    click(&mut runtime, "configuration-confirm", 0.168);
    assert!(runtime.state.intents.contains(&Intent::Configuration {
        action: ConfigurationAction::ToolCredentials
    }));

    runtime.update(|state, _| {
        state.intents.clear();
        state.complete_configuration(
            state.connection.epoch,
            state.configuration.revision,
            state.configuration.draft_revision,
            0,
            ConfigurationAction::ToolCredentials,
            Ok(ConfigurationSaved {
                saved: true,
                configuration_revision: 1,
            }),
        );
        let Load::Ready(profile) = &mut state.backend.profile else {
            panic!("profile fixture")
        };
        profile.configuration_revision = 1;
    });
    runtime.frame(0.21);
    assert_eq!(runtime.state.page, Page::Atlas);
    assert!(runtime.state.tool_key_dialog);
    assert!(runtime.state.configuration.tool_fallback.text.is_empty());
    assert!(runtime.find("atlas-key-sources").is_some());
    assert!(runtime.state.intents.contains(&Intent::Readiness));
    assert!(runtime
        .state
        .submission_disabled_reason(SubmissionKind::Atlas)
        .is_some());

    runtime.update(|state, _| {
        let Load::Ready(readiness) = &state.readiness else {
            panic!("readiness fixture")
        };
        let mut refreshed = readiness.clone();
        let preflight = refreshed.atlas_preflight.as_mut().unwrap();
        preflight.configuration_revision = Some(1);
        preflight.currents = AtlasKeyPresence::Saved;
        state.complete_readiness(Ok(refreshed));
    });
    runtime.frame(0.231);
    assert!(runtime
        .state
        .submission_disabled_reason(SubmissionKind::Atlas)
        .is_none());
    runtime.key(rune::KeyEvent {
        key: rune::Key::Escape,
        mods: rune::Mods::default(),
    });
    runtime.frame(0.252);
    assert!(!runtime.state.tool_key_dialog);
    assert!(runtime
        .elements()
        .into_iter()
        .any(|el| el.key.as_deref() == Some(opener) && el.focused));
    assert!(!runtime
        .state
        .intents
        .iter()
        .any(|intent| matches!(intent, Intent::PostInteractive { .. })));
}

#[test]
fn changed_origin_retains_unknown_identity_but_cannot_replay_its_body() {
    let mut state = fixture();
    state.fixture_effect = Some(uncertain_once);
    state.preview_tool_run();
    state.confirm_submission(SubmissionKind::Tool);
    let id = state
        .submissions
        .tools
        .attempt
        .as_ref()
        .expect("uncertain request retained")
        .request_id
        .clone();
    state.invalidate_origin();
    assert!(state.submissions.tools.delivery_uncertain);
    assert_eq!(
        state
            .submissions
            .tools
            .attempt
            .as_ref()
            .expect("origin switch keeps lookup handle")
            .request_id,
        id
    );
    assert!(!state.submissions.tools.private_result_available());
    let receipt = RegistrationReceipt::new(
        "synthetic-deployment".into(),
        "synthetic-node".into(),
        "synthetic-other-registration".into(),
        Some("192.0.2.42".parse().expect("new synthetic IP")),
        1,
    );
    state.registration.context = Some(
        RegisteredContext::new(
            receipt,
            2,
            1,
            "synthetic-config".into(),
            "synthetic-access".into(),
        )
        .expect("new synthetic context"),
    );
    let before = state.intents.len();
    state.retry_submission(SubmissionKind::Tool);
    assert_eq!(
        state.intents.len(),
        before,
        "a new origin cannot POST the old request"
    );
    assert_eq!(
        state.submissions.tools.phase,
        SubmissionPhase::OutcomeUnknown
    );
}

#[test]
fn structured_result_projection_discards_raw_wire_and_query_credentials() {
    let wire: super::ToolStatusWire = serde_json::from_value(serde_json::json!({
        "call_id":"opaque-server-tool-job","job_id":"opaque-server-tool-job","tool_id":"synthetic-tool","status":"succeeded",
        "result":{"tool_id":"synthetic-tool","status":"ok","source_url":"https://example.test/evidence?api_key=synthetic-secret",
            "retrieved_at":"2026-01-01T00:00:00Z","observations":{"evidence":"synthetic observation"},"raw":"synthetic-secret-provider-wire",
            "inputs":{"api_key":"synthetic-secret"},"error":null,"cached":false,"truncated":false}
    })).expect("safe response projection");
    let view = super::format_tool_status(wire).expect("bounded native view");
    let result = view.result.expect("structured result");
    assert!(!result.observations.contains("synthetic-secret"));
    assert!(result.source.as_deref() == Some("https://example.test/evidence"));
    let (_, truncated) =
        super::bounded_json(&serde_json::json!({"large":"x".repeat(super::MAX_RESULT_TEXT * 2)}));
    assert!(
        truncated,
        "large provider observations cannot expand the native text unboundedly"
    );
}

#[test]
fn read_only_resolution_recovers_exact_association_without_another_post() {
    let mut state = fixture();
    state.fixture_effect = Some(uncertain_once);
    state.preview_tool_run();
    state.confirm_submission(SubmissionKind::Tool);
    assert!(state.submissions.tools.delivery_uncertain);
    state.invalidate_origin();
    let receipt = RegistrationReceipt::new(
        "synthetic-deployment".into(),
        "synthetic-node".into(),
        "synthetic-new-registration".into(),
        Some("192.0.2.42".parse().expect("same synthetic IP")),
        2,
    );
    state.registration.context = Some(
        RegisteredContext::new(
            receipt,
            2,
            1,
            "synthetic-config".into(),
            "synthetic-access".into(),
        )
        .expect("new current proof"),
    );
    state.resolve_submission(SubmissionKind::Tool);
    assert!(matches!(state.submissions.tools.resolution, Load::Ready(_)));
    assert_eq!(state.submissions.tools.phase, SubmissionPhase::Verified);
    assert!(!state.submissions.tools.delivery_uncertain);
    assert!(
        state.submissions.tools.receipt.is_none(),
        "a lookup never fabricates the lost POST receipt"
    );
    let posts = state
        .intents
        .iter()
        .filter(|intent| matches!(intent, Intent::PostInteractive { .. }))
        .count();
    assert_eq!(
        posts, 1,
        "reconciliation is read-only and never submits replacement work"
    );
}
