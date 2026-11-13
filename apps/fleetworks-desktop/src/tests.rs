use fleetworks_contracts::search::ActorSearchResponse;
use fleetworks_contracts::search::IndexStatus;
use rune::Env;
use rune::Runtime;
use rune::TextEdit;

use crate::app;
use crate::app::Intent;
use crate::app::Load;
use crate::backend::ReadKind;
use crate::brain::Brain;
use crate::connection::ConnectionStatus;
use crate::workspaces::WorkspaceRow;
use crate::Page;
use crate::State;
use fleetworks_contracts::capabilities::Capabilities;
use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_workstation::native::WorkspaceState;

#[test]
fn real_shell_navigation_works_offline_without_boundary_effects() {
    let mut runtime = Runtime::new(
        State::fixture(),
        app::view,
        (1180.0, 780.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    runtime.frame(0.0);
    for page in Page::ALL {
        let key = format!("nav-{}", page.key());
        let (x, y) = runtime
            .find(&key)
            .expect("each maintained page has a real sidebar control");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(1.0 / 60.0);
        assert_eq!(runtime.state.page, page);
    }
    assert_eq!(
        runtime.state.connection.status,
        ConnectionStatus::Disconnected
    );
    assert!(runtime.state.intents.is_empty());
    assert!(runtime.find("fixture-marker").is_some());
}

#[test]
fn destination_change_clears_old_gateway_data_and_rejects_queued_reply() {
    let mut state = State::fixture();
    state.connection.status = ConnectionStatus::Connected;
    state.brain.query = TextEdit::new("sensitive old query");
    let previous_epoch = state.connection.epoch;
    state.disconnect();
    state.complete_connection(previous_epoch, Err("stale reply".into()));
    assert_eq!(state.connection.status, ConnectionStatus::Disconnected);
    assert!(state.brain.query.text.is_empty());
    assert!(state.client.is_none());
    assert!(state.connection.capabilities.is_none());
}

#[test]
fn search_coalesces_edits_rejects_stale_reply_and_never_searches_empty_input() {
    let mut brain = Brain {
        query: TextEdit::new("Harbor"),
        ..Brain::default()
    };
    brain.edited(0.0);
    let stale = brain.query_revision();
    brain.in_flight = true;
    brain.query = TextEdit::new("Harbor Union");
    brain.edited(0.1);
    brain.complete_search(
        stale,
        Ok(ActorSearchResponse::new(
            Vec::new(),
            IndexStatus::unknown("fixture".into()),
        )),
    );
    assert!(matches!(brain.results, Load::Pending));
    assert!(!brain.in_flight);
    assert_eq!(brain.pending_at, Some(0.30000000000000004));
    brain.query = TextEdit::new("");
    brain.edited(0.2);
    assert!(brain.pending_at.is_none());
    assert!(matches!(brain.results, Load::Idle));
}

#[test]
fn newer_selection_fences_old_dossier_completion() {
    let mut brain = Brain {
        detail_revision: 3,
        dossier: Load::Loading,
        ..Brain::default()
    };
    brain.complete_dossier(2, Err("old actor".into()));
    assert!(matches!(brain.dossier, Load::Loading));
}

fn workspace(name: &str) -> WorkspaceRow {
    WorkspaceRow {
        name: name.into(),
        path: format!("/fixture/.fleetworks/{name}").into(),
        marker: format!("synthetic-{name}"),
        protected_default: name == "default",
        state: WorkspaceState::Local,
        bindings: Vec::new(),
    }
}

fn registered_fixture() -> State {
    let mut state = State::fixture();
    state.connection.status = ConnectionStatus::Connected;
    let mut capabilities = Capabilities::default();
    capabilities.permissions = vec![
        Permission::ReadBackend,
        Permission::ReadWorkstationActivity,
        Permission::MaintainBackend,
        Permission::ManageConfiguration,
    ];
    state.connection.capabilities = Some(capabilities);
    state.change_workspace(workspace("default"));
    let receipt = RegistrationReceipt::new(
        "synthetic-deployment".into(),
        "synthetic-node".into(),
        "synthetic-registration".into(),
        Some("192.0.2.42".parse().expect("fixture IP")),
        1,
    );
    state.registration.context = Some(
        RegisteredContext::new(
            receipt,
            1,
            1,
            "synthetic-config".into(),
            "synthetic-access".into(),
        )
        .expect("fixture context"),
    );
    state
}

#[test]
fn folder_only_switch_keeps_global_drafts_and_fences_future_origin() {
    let mut state = registered_fixture();
    state.brain.query = TextEdit::new("global actor draft");
    state.backend.article_search = TextEdit::new("global article draft");
    state.backend.tool_inputs = TextEdit::new("{\"email\":\"fixture@example.test\"}");
    let old_revision = state.origin_revision;
    state.change_workspace(workspace("research"));
    assert_eq!(state.brain.query.text, "global actor draft");
    assert_eq!(state.backend.article_search.text, "global article draft");
    assert_eq!(
        state.backend.tool_inputs.text,
        "{\"email\":\"fixture@example.test\"}"
    );
    assert!(state.registration.context.is_none());
    assert!(state.origin_revision > old_revision);
    assert!(matches!(state.backend.jobs, Load::Idle));
    assert!(state.maintenance.confirmation.is_none());
}

#[test]
fn global_reads_do_not_require_folder_but_activity_never_falls_back_to_global() {
    let mut state = registered_fixture();
    state.invalidate_origin();
    state.page = Page::Models;
    state.refresh_page();
    assert_eq!(
        state.intents,
        vec![
            Intent::BackendRead {
                kind: ReadKind::Models
            },
            Intent::BackendRead {
                kind: ReadKind::Profile
            }
        ]
    );
    state.page = Page::Jobs;
    state.refresh_page();
    assert!(matches!(state.backend.jobs, Load::Failed(_)));
    assert_eq!(
        state.intents.len(),
        2,
        "missing registration cannot issue an unscoped activity request"
    );
}

#[test]
fn cross_page_job_link_uses_registered_activity_and_exact_job_detail() {
    let mut state = registered_fixture();
    state.open_job("synthetic-job");
    assert_eq!(state.page, Page::Jobs);
    assert_eq!(state.backend.selected_job.as_deref(), Some("synthetic-job"));
    assert_eq!(
        state.intents,
        vec![
            Intent::BackendRead {
                kind: ReadKind::Jobs
            },
            Intent::BackendRead {
                kind: ReadKind::JobDetail
            }
        ]
    );
    state.invalidate_origin();
    let requests = state.intents.len();
    state.open_job("synthetic-other-node-job");
    assert_eq!(
        state.intents.len(),
        requests,
        "a link cannot manufacture an origin after context invalidation"
    );
}

#[test]
fn provider_key_is_masked_in_real_accessibility_tree_and_confirmation() {
    let mut state = registered_fixture();
    state.page = Page::Models;
    state.backend.profile = Load::Ready(crate::backend::ProfileSnapshot {
        configuration_revision: 1,
        configuration_format: 1,
        accounts: vec![crate::backend::AccountMetadata {
            id: "synthetic-account".into(),
            provider: "local".into(),
            base_url: "http://127.0.0.1:11434".into(),
            model: "synthetic-model".into(),
            has_api_key: true,
            credential_reference: "account:synthetic-account".into(),
        }],
        tool_credentials: Vec::new(),
        setup_required: true,
        portable_schema_version: 4,
    });
    state.configuration.account_id = TextEdit::new("synthetic-account");
    state.configuration.account_provider = TextEdit::new("local");
    state.configuration.account_base_url = TextEdit::new("http://127.0.0.1:11434");
    state
        .configuration
        .account_api_key
        .replace_text("synthetic-secret-that-must-stay-masked");
    state.preview_provider_account();
    let confirmation = state
        .configuration
        .confirmation
        .as_ref()
        .expect("real provider form confirmation");
    assert!(!confirmation
        .change
        .summary()
        .contains("synthetic-secret-that-must-stay-masked"));
    let mut runtime = Runtime::new(
        state,
        app::view,
        (1180.0, 2600.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    runtime.frame(0.0);
    assert!(
        runtime.find("account-api-key/value/input").is_some(),
        "the shared production input participates in the rendered view"
    );
    fn named_secure_editor(node: &serde_json::Value) -> bool {
        (node["role"] == "password-input" && node["name"] == "API key")
            || node["children"]
                .as_array()
                .is_some_and(|children| children.iter().any(named_secure_editor))
    }
    let tree = runtime.accessibility_json();
    assert!(
        named_secure_editor(&tree),
        "the production password editor must retain its API key accessible name: {tree}"
    );
    let accessibility = tree.to_string();
    assert!(
        !accessibility.contains("synthetic-secret-that-must-stay-masked"),
        "assistive output must not expose the entered key"
    );
    runtime.state.invalidate_origin();
    assert!(
        runtime.state.configuration.confirmation.is_none(),
        "authority changes invalidate pending confirmations"
    );
    assert!(
        runtime.state.configuration.account_api_key.text
            == "synthetic-secret-that-must-stay-masked",
        "authority invalidation preserves the unsaved provider draft while clearing consent"
    );
}

#[test]
fn direct_cycles_navigation_observes_configuration_before_any_mutation() {
    let mut state = registered_fixture();
    assert!(
        state.configuration_revision().is_none(),
        "fresh connection has no previous model/profile snapshot"
    );
    state.intents.clear();
    state.navigate(Page::Atlas);
    assert!(
        state.intents.iter().any(|intent| matches!(
            intent,
            Intent::BackendRead {
                kind: ReadKind::Models
            }
        )),
        "cycles must request its required configuration without a prior Models visit"
    );
    assert!(
        !state.intents.iter().any(|intent| matches!(
            intent,
            Intent::SubmitInteractive { .. }
                | Intent::PostInteractive { .. }
                | Intent::PostJobAction { .. }
        )),
        "view entry observes state and never submits work"
    );
}

#[test]
fn changing_model_after_variant_edit_prunes_only_inactive_pending_updates() {
    let mut state = registered_fixture();
    state.backend.models = Load::Ready(serde_json::from_value(serde_json::json!({
        "configuration_revision":7,"roles":["brain_summary"],
        "defaults":{"brain_summary":{"provider":"local","model":"model-a","account":"saved","fallbacks":[{"provider":"local","model":"fallback-c","account":"saved"}]}},
        "catalog":[{"provider":"local","model":"model-a","account":"saved","efforts":["high"]},{"provider":"local","model":"model-b","account":"saved","efforts":[]},{"provider":"local","model":"fallback-c","account":"saved","efforts":["low"]}],
        "model_variants":{"atlas_analyst|local|unrelated":"medium"}
    })).expect("backend model snapshot"));
    state.edit_model_role("brain_summary");
    state.select_model_variant("high");
    state
        .configuration
        .variant_updates
        .insert("brain_summary|local|fallback-c".into(), Some("low".into()));
    assert!(state
        .configuration
        .variant_updates
        .contains_key("brain_summary|local|model-a"));
    state.select_catalog_model("model-b · saved");
    state.preview_model_role(false);
    let confirmation = state
        .configuration
        .confirmation
        .as_ref()
        .expect("valid final-route confirmation");
    let crate::configuration::ConfigurationChange::ModelRole(request) = &confirmation.change else {
        panic!("model role change")
    };
    assert_eq!(request.primary.as_ref().expect("primary").model, "model-b");
    assert!(!request
        .variant_updates
        .contains_key("brain_summary|local|model-a"));
    assert_eq!(
        request
            .variant_updates
            .get("brain_summary|local|fallback-c"),
        Some(&Some("low".into()))
    );
    let Load::Ready(defaults) = &state.backend.models else {
        panic!("unchanged snapshot")
    };
    assert_eq!(
        defaults
            .model_variants
            .get("atlas_analyst|local|unrelated")
            .map(String::as_str),
        Some("medium")
    );
    state.confirm_configuration();
    assert!(state.intents.iter().any(|intent| matches!(
        intent,
        Intent::Configuration {
            action: crate::configuration::ConfigurationAction::ModelRole
        }
    )));
}
