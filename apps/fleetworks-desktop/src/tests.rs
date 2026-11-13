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
    let default_state = State::default();
    assert_eq!(default_state.page, Page::Workstation);
    assert!(!default_state.setup_gateway);

    let mut disconnected = Runtime::new(
        State::fixture(),
        app::view,
        (1180.0, 780.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    disconnected.frame(0.0);
    assert_eq!(disconnected.state.page, Page::Workstation);
    assert!(!disconnected.state.setup_gateway);
    assert!(disconnected.find("sidebar-context-footer").is_none());
    assert!(disconnected.find("nav-workstation").is_some());
    assert!(disconnected.find("nav-setup").is_some());
    assert!(disconnected.find("workstation-setup-gateway").is_some());
    assert!(disconnected.find("host-resources-card").is_none());
    assert!(disconnected.find("local-client-scope").is_some());
    assert!(disconnected.find("resource-scope").is_none());
    assert!(disconnected.find("deployment").is_none());
    let disconnected_tree = disconnected.accessibility_json().to_string();
    assert!(!disconnected_tree.contains("Host Resources"));
    assert!(!disconnected_tree.contains("Workstation activity: registration required"));
    assert!(!disconnected_tree.contains("6 checks"));
    for page in Page::ALL {
        assert!(
            disconnected.find(&format!("nav-{}", page.key())).is_none(),
            "deployment destination {page:?} stays hidden until gateway is connected"
        );
    }

    let (x, y) = disconnected
        .find("workstation-setup-gateway")
        .expect("Workstation Status Setup Gateway CTA");
    disconnected.pointer_down(x, y);
    disconnected.pointer_up(x, y);
    disconnected.frame(1.0 / 60.0);
    assert!(disconnected.state.setup_gateway);

    let (x, y) = disconnected
        .find("nav-workstation")
        .expect("Workstation Status sidebar item");
    disconnected.pointer_down(x, y);
    disconnected.pointer_up(x, y);
    disconnected.frame(2.0 / 60.0);
    assert!(!disconnected.state.setup_gateway);
    assert_eq!(disconnected.state.page, Page::Workstation);

    disconnected.update(|state, _| state.navigate(Page::Brain));
    disconnected.frame(3.0 / 60.0);
    assert_eq!(
        disconnected.state.page,
        Page::Workstation,
        "disconnected navigation cannot enter hidden deployment pages"
    );
    assert_eq!(
        disconnected.state.connection.status,
        ConnectionStatus::Disconnected
    );
    assert!(disconnected.state.intents.is_empty());
    assert!(disconnected.find("fixture-marker").is_some());

    let mut connected = Runtime::new(
        registered_fixture(),
        app::view,
        (1180.0, 780.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    connected.frame(0.0);
    assert!(connected.find("sidebar-context-footer").is_none());
    assert!(connected.find("deployment").is_none());
    assert!(connected.find("gateway-status-header").is_some());
    assert!(connected.find("connect-id").is_none());
    assert!(connected.find("workstation-registered").is_some());
    let connected_tree = connected.accessibility_json().to_string();
    assert!(!connected_tree.contains("Connect ID"));
    assert!(connected_tree.contains("Gateway connected"));
    assert!(connected_tree.contains("synthetic-deployment"));
    let hidden_connect_id = connected
        .state
        .registration
        .context
        .as_ref()
        .expect("registered fixture context")
        .connect_id
        .to_string();
    assert!(!connected_tree.contains(&hidden_connect_id));
    assert!(connected.find("nav-workstation").is_some());
    assert!(connected.find("nav-setup").is_none());
    for page in Page::ALL {
        let key = format!("nav-{}", page.key());
        let (x, y) = connected
            .find(&key)
            .expect("each connected page has a real sidebar control");
        connected.pointer_down(x, y);
        connected.pointer_up(x, y);
        connected.frame(1.0 / 60.0);
        assert_eq!(connected.state.page, page);
        if page != Page::Workstation {
            assert!(
                !connected
                    .accessibility_json()
                    .to_string()
                    .contains("synthetic-deployment"),
                "deployment ID stays restricted to Workstation Status Gateway Backend"
            );
        }
    }
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

pub(crate) fn registered_fixture() -> State {
    let mut state = State::fixture();
    state.connection.status = ConnectionStatus::Connected;
    let mut capabilities = Capabilities::default();
    capabilities.deployment_id = Some("synthetic-deployment".into());
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

#[test]
fn setup_advanced_toggle_switch_reveals_and_hides_readiness_checks() {
    use fleetworks_contracts::readiness::ReadinessCheck;
    use fleetworks_contracts::readiness::ReadinessCheckId;
    use fleetworks_contracts::readiness::ReadinessState;
    use fleetworks_contracts::readiness::WorkstationReadiness;

    let mut state = registered_fixture();
    state.setup_gateway = true;
    state.readiness = Load::Ready(WorkstationReadiness::new(
        "synthetic-deployment".into(),
        "2026-01-01T00:00:00Z".into(),
        vec![ReadinessCheck::new(
            ReadinessCheckId::Gateway,
            ReadinessState::Ready,
            "ok".into(),
            "Synthetic gateway probe".into(),
            "2026-01-01T00:00:00Z".into(),
        )],
    ));
    state.readiness_received_at = Some(0.0);
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
    assert!(!runtime.state.setup_advanced_open);
    assert!(
        !runtime
            .accessibility_json()
            .to_string()
            .contains("Gateway: Ready"),
        "readiness checks stay collapsed behind the Switch until toggled"
    );

    runtime.update(|_, tx| tx.scroll_into_view("setup-advanced-toggle"));
    runtime.frame(1.0 / 60.0);
    let (x, y) = runtime
        .find("setup-advanced-toggle")
        .expect("setup advanced readiness Switch");
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(2.0 / 60.0);
    assert!(runtime.state.setup_advanced_open);
    assert!(runtime
        .accessibility_json()
        .to_string()
        .contains("Gateway: Ready"));

    let (x, y) = runtime
        .find("setup-advanced-toggle")
        .expect("setup advanced readiness Switch remains reachable");
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(3.0 / 60.0);
    assert!(!runtime.state.setup_advanced_open);
    assert!(!runtime
        .accessibility_json()
        .to_string()
        .contains("Gateway: Ready"));
}

#[test]
fn workstation_metric_tile_opens_and_closes_telemetry_modal() {
    use crate::app::WorkstationInspector;
    use crate::workstation_dashboard::HostMetrics;

    let mut state = registered_fixture();
    state.page = Page::Workstation;
    state.host_metrics = Load::Ready(HostMetrics {
        cpu_percent: Some(42.5),
        memory_percent: Some(61.0),
        disk_percent: Some(73.2),
        network_kbps: Some(128.0),
        platform: Some("linux/x86_64".into()),
        chipset: Some("synthetic-x64".into()),
    });
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
    assert!(!runtime
        .accessibility_json()
        .to_string()
        .contains("CPU Telemetry"));

    runtime.update(|_, tx| tx.scroll_into_view("workstation-tile-cpu"));
    runtime.frame(1.0 / 60.0);
    let (x, y) = runtime
        .find("workstation-tile-cpu")
        .expect("clickable CPU metric ring tile");
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(2.0 / 60.0);
    assert_eq!(
        runtime.state.workstation_inspector,
        Some(WorkstationInspector::Cpu)
    );
    let tree = runtime.accessibility_json().to_string();
    assert!(tree.contains("CPU Telemetry"));
    assert!(tree.contains("Observed reading: 42.5%"));
    assert!(tree.contains("Measurements describe the API host"));

    runtime.update(|state, _| state.workstation_inspector = None);
    runtime.frame(3.0 / 60.0);
    assert!(!runtime
        .accessibility_json()
        .to_string()
        .contains("CPU Telemetry"));
}

#[test]
fn seed_preflight_checklist_and_progressive_disclosure_reflect_state() {
    use crate::app::ModelsTab;
    use crate::app::SeedStage;
    use fleetworks_contracts::readiness::ReadinessCheck;
    use fleetworks_contracts::readiness::ReadinessCheckId;
    use fleetworks_contracts::readiness::ReadinessState;
    use fleetworks_contracts::readiness::WorkstationReadiness;

    let mut state = registered_fixture();
    state.page = Page::Seed;
    state.readiness = Load::Ready(WorkstationReadiness::new(
        "synthetic-deployment".into(),
        "2026-01-01T00:00:00Z".into(),
        vec![
            ReadinessCheck::new(
                ReadinessCheckId::Gateway,
                ReadinessState::Ready,
                "ok".into(),
                "Gateway ok".into(),
                "2026-01-01T00:00:00Z".into(),
            ),
            ReadinessCheck::new(
                ReadinessCheckId::Datastore,
                ReadinessState::Unknown,
                "pending".into(),
                "Datastore check pending".into(),
                "2026-01-01T00:00:00Z".into(),
            ),
        ],
    ));
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
    assert!(runtime.find("preflight-gateway").is_some());
    assert!(runtime.find("preflight-datastore").is_some());
    assert!(runtime.find("preflight-dataset").is_some());
    assert!(runtime.find("maintenance-seed").is_some());
    assert!(runtime.find("maintenance-train").is_some());
    assert!(runtime.find("maintenance-reindex").is_some());
    let seed_tree = runtime.accessibility_json().to_string();
    assert!(seed_tree.contains("Choose a CSV to add or update actors."));
    assert!(!seed_tree.contains("Choose a training CSV for a Natural candidate."));

    runtime.update(|state, _| state.seed_stage = SeedStage::Classifier);
    runtime.frame(1.0 / 60.0);
    let classifier_tree = runtime.accessibility_json().to_string();
    assert!(classifier_tree.contains("Choose a training CSV for a Natural candidate."));
    assert!(!classifier_tree.contains("Choose a CSV to add or update actors."));

    runtime.update(|state, _| state.seed_stage = SeedStage::Maintenance);
    runtime.frame(2.0 / 60.0);
    let reindex_tree = runtime.accessibility_json().to_string();
    assert!(reindex_tree.contains("Build the actor-search projection from the backend catalog."));
    assert!(!reindex_tree.contains("Choose a training CSV for a Natural candidate."));

    runtime.update(|state, _| {
        state.page = Page::Models;
        state.backend.models = Load::Idle;
        state.backend.profile = Load::Idle;
    });
    runtime.frame(3.0 / 60.0);
    assert!(runtime
        .accessibility_json()
        .to_string()
        .contains("Role assignments have not been observed."));

    runtime.update(|state, _| state.models_tab = ModelsTab::Providers);
    runtime.frame(4.0 / 60.0);
    let providers_tree = runtime.accessibility_json().to_string();
    assert!(providers_tree.contains("Provider metadata has not been observed."));
    assert!(!providers_tree.contains("Role assignments have not been observed."));
}
