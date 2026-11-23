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
fn wide_and_compact_sidebar_headers_show_the_symbol_alone() {
    let env = Env {
        system_dark: true,
        scale: 1.0,
    };
    let mut wide = Runtime::new(State::fixture(), app::view, (1180.0, 800.0), env);
    wide.frame(0.0);
    assert!(
        wide.find("brand-mark").is_some(),
        "the approved symbol, named Fleetworks, heads the sidebar"
    );
    let mut narrow = Runtime::new(State::fixture(), app::view, (640.0, 480.0), env);
    narrow.frame(0.0);
    assert!(narrow.find("compact-icon-rail").is_some());
    assert!(narrow.find("brand-mark").is_some());
}

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
    assert!(connected.find("nav-seed").is_none());
    for page in Page::ALL.into_iter().filter(|page| *page != Page::Seed) {
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
    let mut gateway = ReadinessCheck::new(
        ReadinessCheckId::Gateway,
        ReadinessState::Ready,
        "ok".into(),
        "Synthetic gateway probe".into(),
        "2026-01-01T00:00:00Z".into(),
    );
    gateway.fresh_for_ms = 60_000;
    state.readiness = Load::Ready(WorkstationReadiness::new(
        "synthetic-deployment".into(),
        "2026-01-01T00:00:00Z".into(),
        vec![gateway],
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
        .find("setup-advanced-toggle/switch")
        .expect("setup advanced readiness Switch");
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(2.0 / 60.0);
    assert!(runtime.state.setup_advanced_open);
    assert!(runtime
        .accessibility_json()
        .to_string()
        .contains("Gateway: Ready"));

    runtime.update(|_, tx| tx.blur());
    runtime.frame(3.0 / 60.0);
    assert!(!status_check_element(&runtime, "setup-advanced-toggle/switch").focused);
    let (x, y) = runtime
        .find("setup-advanced-toggle")
        .expect("the original readiness row remains a pointer target");
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(4.0 / 60.0);
    assert!(!runtime.state.setup_advanced_open);
    assert!(status_check_element(&runtime, "setup-advanced-toggle/switch").focused);
    assert!(!runtime
        .accessibility_json()
        .to_string()
        .contains("Gateway: Ready"));
}

fn compact_connected_setup_readiness_toggle(kind: fleetworks_workstation::native::BackendKind) {
    use fleetworks_contracts::readiness::ReadinessCheck;
    use fleetworks_contracts::readiness::ReadinessCheckId;
    use fleetworks_contracts::readiness::ReadinessState;
    use fleetworks_contracts::readiness::WorkstationReadiness;
    use fleetworks_workstation::native::BackendKind;

    let mut state = registered_fixture();
    state.backend_kind = kind;
    if kind == BackendKind::Local {
        state.connection.origin = TextEdit::new("local://embedded");
    }
    let mut gateway = ReadinessCheck::new(
        ReadinessCheckId::Gateway,
        ReadinessState::Ready,
        "ok".into(),
        "Synthetic gateway probe".into(),
        "2026-01-01T00:00:00Z".into(),
    );
    gateway.fresh_for_ms = 60_000;
    state.readiness = Load::Ready(WorkstationReadiness::new(
        "synthetic-deployment".into(),
        "2026-01-01T00:00:00Z".into(),
        vec![gateway],
    ));
    state.readiness_received_at = Some(0.0);

    let mut runtime = Runtime::new(
        state,
        app::view,
        (640.0, 480.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    runtime.frame(0.0);
    runtime.frame(1.0 / 60.0);
    assert!(!runtime.state.setup_gateway);
    let (x, y) = runtime
        .find("connection-editor")
        .expect("reopen Setup Gateway");
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(2.0 / 60.0);
    assert!(runtime.state.setup_gateway);
    assert!(runtime.state.connected());
    assert!(runtime.state.registration.context.is_some());

    let key = "setup-advanced-toggle/switch";
    runtime.update(|_, tx| tx.scroll_into_view("setup-advanced-toggle"));
    runtime.frame(3.0 / 60.0);
    let elements = runtime.elements();
    let switch = elements
        .iter()
        .find(|element| element.path.ends_with(key))
        .expect("actual readiness switch");
    let copy = elements
        .iter()
        .find(|element| {
            element
                .path
                .ends_with("setup-advanced-toggle/copy/description")
        })
        .expect("visible readiness switch description");
    assert_eq!(switch.role, "switch");
    assert_eq!(switch.label.as_deref(), Some("Status checks · 1 passed"),);
    assert_eq!(
        switch.visible, switch.rect,
        "entire {kind:?} switch must fit"
    );
    assert_eq!(
        copy.visible, copy.rect,
        "entire {kind:?} description must fit"
    );
    assert!(switch.rect[2] >= 44.0 && switch.rect[3] >= 32.0);
    let [x, y, w, h] = switch.rect;
    assert!(x >= 0.0 && y >= 0.0 && x + w <= 640.0 && y + h <= 480.0);
    assert!(!runtime.state.setup_advanced_open);

    // Press the switch itself, rather than the centre of its enclosing row.
    let (x, y) = runtime.find(key).expect("switch pointer target");
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(4.0 / 60.0);
    assert!(runtime.state.setup_advanced_open);
    assert!(runtime
        .accessibility_json()
        .to_string()
        .contains("Gateway: Ready"));

    runtime.update(|_, tx| tx.focus("setup-advanced-toggle", true));
    runtime.frame(5.0 / 60.0);
    assert!(runtime
        .elements()
        .iter()
        .any(|element| element.path.ends_with(key) && element.focused));
    runtime.key(rune::KeyEvent {
        key: rune::Key::Text(" ".into()),
        mods: rune::Mods::default(),
    });
    runtime.frame(6.0 / 60.0);
    assert!(!runtime.state.setup_advanced_open);
    assert!(!runtime
        .accessibility_json()
        .to_string()
        .contains("Gateway: Ready"));
}

#[test]
fn compact_connected_embedded_setup_keeps_readiness_switch_usable() {
    compact_connected_setup_readiness_toggle(fleetworks_workstation::native::BackendKind::Local);
}

#[test]
fn compact_connected_distributed_setup_keeps_readiness_switch_usable() {
    compact_connected_setup_readiness_toggle(fleetworks_workstation::native::BackendKind::Remote);
}

#[test]
fn status_checks_share_the_compact_keyboard_disclosures_and_keep_the_blocker_visible() {
    use fleetworks_contracts::context::JobOrigin;
    use fleetworks_contracts::context::OriginSource;
    use fleetworks_contracts::readiness::MaintenanceOperation;
    use fleetworks_contracts::readiness::MaintenanceReceipt;
    use fleetworks_contracts::readiness::ReadinessCheck;
    use fleetworks_contracts::readiness::ReadinessCheckId;
    use fleetworks_contracts::readiness::ReadinessState;
    use fleetworks_contracts::readiness::WorkstationReadiness;
    use fleetworks_workstation::native::BackendKind;

    const OBSERVED: &str = "2026-01-01T00:00:00Z";
    const CRITICAL: &str = "readiness-critical-action";
    const SECTION: &str = "status-checks-toggle/trigger";
    const SETUP_SWITCH: &str = "setup-advanced-toggle/switch";
    const FAILED_ROW: &str = "readiness-trigger-jobs";
    const FAILED_DETAIL: &str = "readiness-details-jobs";
    const ACTOR_ROW: &str = "readiness-trigger-actor_search";
    const PASSED_ROW: &str = "readiness-trigger-gateway";

    let check = |id, state, reason: &str, explanation: &str| {
        let mut check = ReadinessCheck::new(
            id,
            state,
            reason.to_owned(),
            explanation.to_owned(),
            OBSERVED.to_owned(),
        );
        check.fresh_for_ms = 60_000;
        check
    };
    let mut unverified = check(
        ReadinessCheckId::Datastore,
        ReadinessState::Ready,
        "datastore_unverified",
        "An earlier gateway response marked the datastore ready.",
    );
    unverified.fresh_for_ms = 0;
    let checks = vec![
        check(
            ReadinessCheckId::Gateway,
            ReadinessState::Ready,
            "gateway_reachable",
            "The synthetic gateway accepted the workstation registration.",
        ),
        unverified,
        check(
            ReadinessCheckId::Brain,
            ReadinessState::Unknown,
            "brain_pending",
            "The query service has yet to be checked.",
        ),
        check(
            ReadinessCheckId::ActorSearch,
            ReadinessState::Running,
            "index_catching_up",
            "The index projection is converging on the current checkpoint.",
        ),
        check(
            ReadinessCheckId::Jobs,
            ReadinessState::NotReady,
            "process_not_ready",
            "The Jobs service reported that its dispatcher was not ready.",
        ),
    ];

    for (kind, setup, disclosure) in [
        (BackendKind::Remote, false, SECTION),
        (BackendKind::Local, true, SETUP_SWITCH),
        (BackendKind::Remote, true, SETUP_SWITCH),
    ] {
        let mut state = registered_fixture();
        state.backend_kind = kind;
        state.setup_gateway = setup;
        if kind == BackendKind::Local {
            state.connection.origin = TextEdit::new("local://embedded");
        }
        state.complete_readiness(Ok(WorkstationReadiness::new(
            "synthetic-deployment".into(),
            OBSERVED.into(),
            checks.clone(),
        )));
        let mut runtime = Runtime::new(
            state,
            app::view,
            (640.0, 480.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        runtime.reduced_motion = true;
        let mut frame = 0;
        tick_status_checks(&mut runtime, &mut frame);

        let summary = status_check_element(&runtime, disclosure).label.unwrap();
        assert!(summary.contains("Status checks"), "{summary}");
        assert!(summary.contains("1 passed"), "{summary}");
        assert!(summary.contains("1 failed"), "{summary}");
        assert!(runtime.find(FAILED_ROW).is_none());
        assert!(runtime.find(FAILED_DETAIL).is_none());

        // The blocker must be visible in the initial viewport on all three surfaces.
        let critical = assert_status_check_fully_visible(&runtime, CRITICAL);
        assert_eq!(critical.scrolled, [0.0, 0.0], "{kind:?}, setup={setup}");
        assert_eq!(critical.label.as_deref(), Some("Refresh checks"));
        assert!(critical.focusable);
        focus_status_check(&mut runtime, CRITICAL, &mut frame);
        runtime.state.intents.clear();
        press_status_check(&mut runtime, &mut frame);
        assert!(runtime.state.intents.contains(&Intent::Readiness));
        assert!(runtime.find(FAILED_ROW).is_none());

        focus_status_check(&mut runtime, disclosure, &mut frame);
        press_status_check(&mut runtime, &mut frame);
        assert!(runtime.find("backend-readiness-checks").is_some());
        assert!(runtime.find(FAILED_DETAIL).is_none());

        let failed = status_check_element(&runtime, FAILED_ROW);
        let actor = status_check_element(&runtime, ACTOR_ROW);
        let passed = status_check_element(&runtime, PASSED_ROW);
        assert_eq!(failed.label.as_deref(), Some("Jobs: Failed"));
        assert_eq!(
            actor.label.as_deref(),
            Some("Actor search: Background indexing")
        );
        assert_eq!(passed.label.as_deref(), Some("Gateway: Ready"));
        assert!(failed.target[1] < actor.target[1], "{kind:?}");
        assert!(actor.target[1] < passed.target[1], "{kind:?}");
        for key in ["readiness-icon-jobs", "readiness-icon-gateway"] {
            assert!(status_check_element(&runtime, key).rect[2] > 0.0);
        }
        let unverified = status_check_element(&runtime, "readiness-unsynced-datastore");
        assert!(unverified
            .label
            .as_deref()
            .is_some_and(|label| { label.to_ascii_lowercase().contains("unsynced") }));
        let tree = runtime.accessibility_json().to_string();
        assert!(tree.contains("Datastore observation unsynced."));
        assert!(!tree.contains("process_not_ready"));
        assert!(!tree.contains("The Jobs service reported that its dispatcher was not ready."));

        runtime.key(rune::KeyEvent {
            key: rune::Key::Tab,
            mods: rune::Mods::default(),
        });
        tick_status_checks(&mut runtime, &mut frame);
        assert!(assert_status_check_fully_visible(&runtime, FAILED_ROW).focused);
        press_status_check(&mut runtime, &mut frame);
        assert!(runtime.find(FAILED_DETAIL).is_some());
        assert!(status_check_element(&runtime, FAILED_ROW).focused);
        let tree = runtime.accessibility_json().to_string();
        assert!(tree.contains("process_not_ready"));
        assert!(tree.contains("The Jobs service reported that its dispatcher was not ready."));

        runtime.update(|state, _| {
            let Load::Ready(previous) = &state.readiness else {
                panic!("expected the previous observation");
            };
            let mut next = previous.clone();
            next.checks.reverse();
            let jobs = next
                .checks
                .iter_mut()
                .find(|check| check.id == ReadinessCheckId::Jobs)
                .unwrap();
            jobs.explanation = "The Jobs service still reports its dispatcher as not ready.".into();
            state.complete_readiness(Ok(next));
        });
        tick_status_checks(&mut runtime, &mut frame);
        assert!(status_check_element(&runtime, FAILED_ROW).focused);
        assert!(runtime.find(FAILED_DETAIL).is_some());
        assert!(runtime
            .accessibility_json()
            .to_string()
            .contains("The Jobs service still reports its dispatcher as not ready."));

        runtime.update(|_, tx| tx.scroll_into_view(FAILED_DETAIL));
        tick_status_checks(&mut runtime, &mut frame);
        assert_status_check_fully_visible(&runtime, FAILED_DETAIL);

        runtime.update(|state, _| {
            let origin = JobOrigin::new(
                state.registration.context.as_ref().unwrap(),
                OriginSource::Native,
                "synthetic-reindex-request".into(),
                OBSERVED.into(),
                0,
                Vec::new(),
            );
            state.maintenance.receipt = Load::Ready(MaintenanceReceipt::new(
                "synthetic-reindex".into(),
                MaintenanceOperation::ReindexActors,
                origin,
                None,
                false,
            ));
        });
        tick_status_checks(&mut runtime, &mut frame);
        assert_eq!(
            status_check_element(&runtime, ACTOR_ROW).label.as_deref(),
            Some("Actor search: Checking")
        );

        focus_status_check(&mut runtime, disclosure, &mut frame);
        press_status_check(&mut runtime, &mut frame);
        assert!(runtime.find(FAILED_ROW).is_none());
        runtime.update(|_, tx| tx.scroll_into_view(CRITICAL));
        tick_status_checks(&mut runtime, &mut frame);
        assert_status_check_fully_visible(&runtime, CRITICAL);
    }
}

#[test]
fn desktop_status_table_aligns_columns_and_exposes_the_granted_row_action() {
    use fleetworks_contracts::readiness::DatasetKind;
    use fleetworks_contracts::readiness::ReadinessAction;
    use fleetworks_contracts::readiness::ReadinessActionId;
    use fleetworks_contracts::readiness::ReadinessCheck;
    use fleetworks_contracts::readiness::ReadinessCheckId;
    use fleetworks_contracts::readiness::ReadinessState;
    use fleetworks_contracts::readiness::WorkstationReadiness;

    const OBSERVED: &str = "2026-01-01T00:00:00Z";
    const SEED: &str = "readiness-action-actor_catalog-SeedActors";
    let mut state = registered_fixture();
    state.backend.profile = Load::Ready(crate::backend::ProfileSnapshot {
        configuration_revision: 0,
        configuration_format: 1,
        accounts: vec![],
        tool_credentials: vec![],
        setup_required: false,
        portable_schema_version: 4,
    });
    let mut gateway = ReadinessCheck::new(
        ReadinessCheckId::Gateway,
        ReadinessState::Ready,
        "gateway_reachable".into(),
        "The gateway accepted the request.".into(),
        OBSERVED.into(),
    );
    gateway.fresh_for_ms = 60_000;
    let mut catalog = ReadinessCheck::new(
        ReadinessCheckId::ActorCatalog,
        ReadinessState::NotReady,
        "database_catalog_empty".into(),
        "The catalog is empty at the current checkpoint.".into(),
        OBSERVED.into(),
    );
    catalog.fresh_for_ms = 60_000;
    catalog.actions.push(ReadinessAction::new(
        ReadinessActionId::SeedActors,
        Permission::MaintainBackend,
        true,
    ));
    let mut jobs = ReadinessCheck::new(
        ReadinessCheckId::Jobs,
        ReadinessState::NotReady,
        "process_not_ready".into(),
        "The dispatcher is unavailable.".into(),
        OBSERVED.into(),
    );
    jobs.fresh_for_ms = 60_000;
    let mut readiness = WorkstationReadiness::new(
        "synthetic-deployment".into(),
        OBSERVED.into(),
        vec![gateway, catalog, jobs],
    );
    readiness.registration = Some(RegistrationReceipt::new(
        "synthetic-deployment".into(),
        "synthetic-node".into(),
        "synthetic-registration".into(),
        Some("192.0.2.42".parse().unwrap()),
        1,
    ));
    state.complete_readiness(Ok(readiness));
    let mut runtime = Runtime::new(
        state,
        app::view,
        (1280.0, 800.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    runtime.reduced_motion = true;
    let mut frame = 0;
    tick_status_checks(&mut runtime, &mut frame);
    let blocker = status_check_element(&runtime, "readiness-critical-action");
    assert_eq!(blocker.visible, blocker.rect);
    assert_eq!(blocker.label.as_deref(), Some("Refresh checks"));
    assert!(runtime.find("workstation-open-seed").is_none());
    assert!(runtime.find(SEED).is_none());
    runtime.update(|_, tx| tx.focus("status-checks-toggle", true));
    tick_status_checks(&mut runtime, &mut frame);
    press_status_check(&mut runtime, &mut frame);
    let header = status_check_element(&runtime, "readiness-status-table-header");
    let column_labels: Vec<_> = runtime
        .elements()
        .into_iter()
        .filter(|element| element.path.starts_with(&format!("{}/", header.path)))
        .filter_map(|element| element.text)
        .collect();
    assert_eq!(column_labels, ["Service", "Status", "Action"]);
    let catalog_status = status_check_element(&runtime, "readiness-status-actor_catalog");
    let gateway_status = status_check_element(&runtime, "readiness-status-gateway");
    let jobs_status = status_check_element(&runtime, "readiness-status-jobs");
    assert!(jobs_status.target[1] < catalog_status.target[1]);
    assert_eq!(catalog_status.target[0], gateway_status.target[0]);
    assert!(catalog_status.target[1] < gateway_status.target[1]);
    assert!(runtime.find("readiness-details-actor_catalog").is_none());
    let seed = status_check_element(&runtime, SEED);
    assert!(seed.focusable);
    assert!(seed.target[0] > catalog_status.target[0]);
    runtime.update(|_, tx| {
        tx.scroll_into_view(SEED);
        tx.focus(SEED, true);
    });
    tick_status_checks(&mut runtime, &mut frame);
    assert!(status_check_element(&runtime, SEED).focused);
    runtime.state.intents.clear();
    press_status_check(&mut runtime, &mut frame);
    assert!(runtime.state.intents.contains(&Intent::ChooseDataset {
        kind: DatasetKind::ActorCatalog,
    }));
    assert!(runtime.find("readiness-details-actor_catalog").is_none());

    runtime.update(|state, _| {
        let Load::Ready(readiness) = &state.readiness else {
            panic!("expected the previous observation");
        };
        let mut next = readiness.clone();
        let catalog = next
            .checks
            .iter_mut()
            .find(|check| check.id == ReadinessCheckId::ActorCatalog)
            .unwrap();
        catalog.state = ReadinessState::Ready;
        catalog.reason_code = "database_catalog_ready".into();
        catalog.explanation = "The catalog is ready at the current checkpoint.".into();
        state.complete_readiness(Ok(next));
    });
    tick_status_checks(&mut runtime, &mut frame);
    let jobs_status = status_check_element(&runtime, "readiness-status-jobs");
    let catalog_status = status_check_element(&runtime, "readiness-status-actor_catalog");
    assert!(jobs_status.target[1] < catalog_status.target[1]);
    assert!(runtime.find(SEED).is_none());
    const CATALOG_ROW: &str = "readiness-trigger-actor_catalog";
    runtime.update(|_, tx| {
        tx.scroll_into_view(CATALOG_ROW);
        tx.focus(CATALOG_ROW, true);
    });
    tick_status_checks(&mut runtime, &mut frame);
    let catalog_row = status_check_element(&runtime, CATALOG_ROW);
    assert!(catalog_row.focused);
    assert_eq!(catalog_row.visible, catalog_row.rect);
    press_status_check(&mut runtime, &mut frame);
    assert!(runtime.find("readiness-details-actor_catalog").is_some());
    assert!(status_check_element(&runtime, SEED).focusable);
}

fn tick_status_checks(runtime: &mut Runtime<State>, frame: &mut u32) {
    *frame += 1;
    runtime.frame(f64::from(*frame) / 60.0);
}

fn status_check_element(runtime: &Runtime<State>, key: &str) -> rune::ElementInfo {
    runtime
        .elements()
        .into_iter()
        .find(|element| {
            !element.exiting
                && (element.key.as_deref() == Some(key)
                    || element.path.ends_with(&format!("/{key}")))
        })
        .unwrap_or_else(|| panic!("expected element {key}"))
}

fn assert_status_check_fully_visible(runtime: &Runtime<State>, key: &str) -> rune::ElementInfo {
    let element = status_check_element(runtime, key);
    assert_eq!(element.visible, element.rect, "{key} is clipped");
    let [x, y, w, h] = element.rect;
    assert!(w > 0.0 && h > 0.0, "{key}: {element:?}");
    assert!(
        x >= 0.0 && y >= 0.0 && x + w <= 640.5 && y + h <= 480.5,
        "{key}: {element:?}",
    );
    element
}

fn focus_status_check(runtime: &mut Runtime<State>, path: &str, frame: &mut u32) {
    // Rune's focus request accepts an element key and chooses its first focusable
    // descendant. The scroll request and assertions can target the exact child path.
    let anchor = path.split_once('/').map_or(path, |(key, _)| key);
    runtime.update(move |_, tx| {
        tx.scroll_into_view(path);
        tx.focus(anchor, true);
    });
    tick_status_checks(runtime, frame);
    assert!(
        assert_status_check_fully_visible(runtime, path).focused,
        "expected keyboard focus at {path} within {anchor}",
    );
}

fn press_status_check(runtime: &mut Runtime<State>, frame: &mut u32) {
    runtime.key(rune::KeyEvent {
        key: rune::Key::Text(" ".into()),
        mods: rune::Mods::default(),
    });
    tick_status_checks(runtime, frame);
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

    assert!(runtime.find("host-resources-card").is_some());
    let (x, y) = runtime
        .find("workstation-tile-cpu")
        .expect("CPU dial is available on the dashboard without disclosure");
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(1.0 / 60.0);
    assert_eq!(
        runtime.state.workstation_inspector,
        Some(WorkstationInspector::Cpu)
    );
    let tree = runtime.accessibility_json().to_string();
    assert!(tree.contains("CPU Telemetry"));
    assert!(tree.contains("Observed reading: 42.5%"));
    assert!(tree.contains("Measurements describe the API host"));

    runtime.update(|state, _| {
        state.apply_host_metrics(Err("gateway timed out".into()));
    });
    runtime.frame(2.0 / 60.0);
    assert_eq!(
        runtime.state.workstation_inspector,
        Some(WorkstationInspector::Cpu)
    );
    assert!(runtime
        .accessibility_json()
        .to_string()
        .contains("Observed reading: 42.5%"));

    runtime.update(|state, _| state.workstation_inspector = None);
    runtime.frame(3.0 / 60.0);
    assert!(!runtime
        .accessibility_json()
        .to_string()
        .contains("CPU Telemetry"));
}

#[test]
fn seed_button_preflight_status_shimmer_and_progressive_disclosure_reflect_state() {
    use crate::app::ModelsTab;
    use crate::app::SeedStage;
    use fleetworks_contracts::readiness::ReadinessAction;
    use fleetworks_contracts::readiness::ReadinessActionId;
    use fleetworks_contracts::readiness::ReadinessCheck;
    use fleetworks_contracts::readiness::ReadinessCheckId;
    use fleetworks_contracts::readiness::ReadinessState;
    use fleetworks_contracts::readiness::WorkstationReadiness;

    let mut state = registered_fixture();
    state.page = Page::Seed;
    let mut gateway = ReadinessCheck::new(
        ReadinessCheckId::Gateway,
        ReadinessState::Ready,
        "ok".into(),
        "Gateway ok".into(),
        "2026-01-01T00:00:00Z".into(),
    );
    gateway.fresh_for_ms = 5000;
    gateway.actions = vec![ReadinessAction::new(
        ReadinessActionId::SeedActors,
        Permission::MaintainBackend,
        true,
    )];
    let mut readiness = WorkstationReadiness::new(
        "synthetic-deployment".into(),
        "2026-01-01T00:00:00Z".into(),
        vec![
            gateway,
            ReadinessCheck::new(
                ReadinessCheckId::Datastore,
                ReadinessState::Unknown,
                "pending".into(),
                "Datastore check pending".into(),
                "2026-01-01T00:00:00Z".into(),
            ),
        ],
    );
    readiness.registration = Some(RegistrationReceipt::new(
        "synthetic-deployment".into(),
        "synthetic-node".into(),
        "synthetic-registration".into(),
        Some("192.0.2.42".parse().expect("fixture IP")),
        1,
    ));
    state.readiness = Load::Ready(readiness);
    state.readiness_received_at = Some(0.0);
    state.backend.profile = Load::Ready(crate::backend::ProfileSnapshot {
        configuration_revision: 0,
        configuration_format: 1,
        accounts: vec![],
        tool_credentials: vec![],
        setup_required: false,
        portable_schema_version: 4,
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
    assert!(runtime.find("preflight-gateway").is_none());
    assert!(runtime.find("preflight-datastore").is_none());
    assert!(runtime.find("preflight-dataset").is_none());
    assert!(runtime.find("maintenance-seed").is_some());
    assert!(runtime.find("maintenance-train").is_none());
    assert!(runtime.find("maintenance-reindex").is_some());
    let seed_action = if cfg!(target_os = "linux") {
        "Enter actor CSV path"
    } else {
        "1. Seed Actors"
    };
    let seed_explanation =
        "Use a CSV to add or update actors. Existing actors outside this CSV are kept.";
    let seed_tree = runtime.accessibility_json().to_string();
    assert!(!seed_tree.contains("Pre-flight Checklist"));
    assert!(seed_tree.contains(&format!("{seed_action} (Ready)")));
    assert!(seed_tree.contains("2. Build Search Index (Unavailable)"));
    assert!(seed_tree.contains(seed_explanation));
    assert!(!seed_tree.contains("Use a training CSV for a Natural candidate."));
    assert!(runtime.find("loading-shimmer").is_none());

    runtime.update(|state, _| state.maintenance.upload = Load::Loading);
    runtime.update(|state, _| {
        state.maintenance.local_dataset = Some(crate::maintenance::LocalDataset {
            path: "/fixture/actors.csv".into(),
            kind: fleetworks_contracts::readiness::DatasetKind::ActorCatalog,
        });
    });
    runtime.frame(0.5 / 60.0);
    let uploading_tree = runtime.accessibility_json().to_string();
    assert!(uploading_tree.contains(&format!("{seed_action} (Uploading…)")));
    assert!(runtime.find("loading-shimmer").is_some());
    runtime.update(|state, _| {
        state.maintenance.upload = Load::Idle;
        state.maintenance.local_dataset = None;
    });

    runtime.update(|state, _| state.seed_stage = SeedStage::Maintenance);
    runtime.frame(2.0 / 60.0);
    let reindex_tree = runtime.accessibility_json().to_string();
    assert!(reindex_tree
        .contains("This gateway has not advertised a durable consumer for this operation"));
    assert!(!reindex_tree.contains(seed_explanation));

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

#[test]
fn dashboard_metrics_distinguish_unavailable_zero_and_last_known_samples() {
    use crate::workstation_dashboard::HostMetrics;
    use fleetworks_workstation::native::BackendKind;

    const CPU: &str = "workstation-tile-cpu";
    const MEMORY: &str = "workstation-tile-memory";
    const DISK: &str = "workstation-tile-disk";
    const UNSYNCED: &str = "host-metrics-unsynced";
    let mut state = registered_fixture();
    state.backend_kind = BackendKind::Local;
    state.connection.origin = TextEdit::new("local://embedded");
    state.host_metrics = Load::Loading;
    state.readiness = Load::Loading;
    let mut runtime = Runtime::new(
        state,
        app::view,
        (640.0, 480.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    runtime.reduced_motion = true;
    let mut frame = 0;
    tick_status_checks(&mut runtime, &mut frame);
    assert!(runtime.find("workstation-status-header").is_some());
    assert!(runtime.find("host-metrics-loading").is_some());
    let tree = runtime.accessibility_json().to_string();
    assert!(tree.contains("API host resources"));
    assert!(tree.contains("Status checks · checking"));
    for (key, label) in [
        (CPU, "CPU: Unavailable"),
        (MEMORY, "Memory: Unavailable"),
        (DISK, "Disk: Unavailable"),
    ] {
        assert_eq!(
            status_check_element(&runtime, key).label.as_deref(),
            Some(label)
        );
    }

    runtime.update(|state, _| {
        state.apply_host_metrics(Ok(HostMetrics {
            cpu_percent: Some(0.0),
            disk_percent: Some(42.5),
            ..HostMetrics::default()
        }));
    });
    tick_status_checks(&mut runtime, &mut frame);
    assert!(runtime.find("host-metrics-loading").is_none());
    assert!(runtime.find(UNSYNCED).is_none());
    let values = [
        (CPU, "CPU: 0.0%"),
        (MEMORY, "Memory: Unavailable"),
        (DISK, "Disk: 42.5%"),
    ];
    for (key, label) in values {
        runtime.update(|_, tx| tx.scroll_into_view(key));
        tick_status_checks(&mut runtime, &mut frame);
        let tile = assert_status_check_fully_visible(&runtime, key);
        assert_eq!(tile.scrolled[0], 0.0, "{key} required horizontal scrolling");
        assert_eq!(tile.label.as_deref(), Some(label));
    }

    focus_status_check(&mut runtime, DISK, &mut frame);
    runtime.update(|state, _| state.apply_host_metrics(Err("gateway timed out".into())));
    tick_status_checks(&mut runtime, &mut frame);
    for (key, label) in values {
        assert_eq!(
            status_check_element(&runtime, key).label.as_deref(),
            Some(label)
        );
    }
    assert!(status_check_element(&runtime, DISK).focused);
    assert!(runtime.find(UNSYNCED).is_some());
    assert!(runtime
        .accessibility_json()
        .to_string()
        .contains("Last known API host readings. Open a metric for details."));

    runtime.update(|state, _| state.apply_host_metrics(Ok(HostMetrics::default())));
    tick_status_checks(&mut runtime, &mut frame);
    assert!(runtime.find(UNSYNCED).is_none());
    assert_eq!(
        status_check_element(&runtime, DISK).label.as_deref(),
        Some("Disk: Unavailable")
    );
}

pub(crate) fn brain_search_response_fixture() -> ActorSearchResponse {
    use fleetworks_contracts::search::ActorSearchHit;
    ActorSearchResponse::new(
        ["first", "second"]
            .into_iter()
            .map(|id| ActorSearchHit::new(id.into(), id.into(), 1, "test-generation".into()))
            .collect(),
        IndexStatus::unknown("2026-01-01T00:00:00Z".into()),
    )
}

fn brain_results_fixture() -> State {
    let mut state = registered_fixture();
    state.page = Page::Brain;
    state.brain.results = Load::Ready(brain_search_response_fixture());
    state.intents.clear();
    state
}

#[test]
fn brain_records_fetch_only_on_expansion_and_fence_collapse_query_and_selection() {
    let mut state = brain_results_fixture();
    assert!(state.intents.is_empty());
    state.toggle_actor(0);
    assert!(
        matches!(state.intents.last(), Some(Intent::Dossier { actor_id }) if actor_id == "first")
    );
    let first = state.brain.detail_revision;
    state.toggle_actor(0);
    assert!(!state.brain.showing_detail);
    assert!(matches!(state.brain.results, Load::Ready(_)));
    state
        .brain
        .complete_dossier(first, Err("late collapsed response".into()));
    assert!(matches!(state.brain.dossier, Load::Idle));
    assert_eq!(state.intents.len(), 1, "collapse must never fetch");
    state.toggle_actor(0);
    let reopened = state.brain.detail_revision;
    state.toggle_actor(1);
    state
        .brain
        .complete_dossier(reopened, Err("late first record".into()));
    assert!(matches!(state.brain.dossier, Load::Loading));
    assert_eq!(state.brain.selected_actor.as_deref(), Some("second"));
    let second = state.brain.detail_revision;
    state
        .brain
        .complete_dossier(second, Err("retryable failure".into()));
    state.brain.highlighted = Some(0);
    state.retry_dossier();
    assert!(
        matches!(state.intents.last(), Some(Intent::Dossier { actor_id }) if actor_id == "second")
    );
    let retry = state.brain.detail_revision;
    state.brain.edited(0.0);
    state.brain.complete_dossier(retry, Err("old query".into()));
    assert!(!state.brain.showing_detail);
    assert!(matches!(state.brain.dossier, Load::Idle));
}

#[test]
fn brain_real_result_disclosure_preserves_list_at_compact_and_wide_sizes() {
    for size in [(640.0, 480.0), (1280.0, 800.0)] {
        let mut runtime = Runtime::new(
            brain_results_fixture(),
            app::view,
            size,
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        runtime.frame(0.0);
        assert!(runtime.find("actor-dossier").is_none());
        assert!(runtime.state.intents.is_empty());
        let (x, y) = runtime
            .find("actor-hit-first")
            .expect("first result trigger");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(1.0 / 60.0);
        assert!(runtime.find("actor-dossier").is_some());
        let trigger = runtime
            .elements()
            .into_iter()
            .find(|element| element.key.as_deref() == Some("actor-hit-first"))
            .expect("disclosure element");
        assert_eq!(
            trigger.expanded,
            Some(true),
            "disclosure announces its state"
        );
        assert!(runtime.find("actor-results").is_some());
        assert!(
            runtime.find("actor-search").is_some(),
            "opening never navigates away from search"
        );
        runtime.update(|_, tx| tx.focus("actor-hit-first", true));
        runtime.frame(1.5 / 60.0);
        runtime.key(rune::KeyEvent {
            key: rune::Key::Enter,
            mods: rune::Mods::default(),
        });
        runtime.frame(2.0 / 60.0);
        assert!(runtime.find("actor-dossier").is_none());
        assert!(runtime.find("actor-hit-second").is_some());
        assert_eq!(
            runtime
                .state
                .intents
                .iter()
                .filter(|intent| matches!(intent, Intent::Dossier { .. }))
                .count(),
            1
        );
    }
}

#[test]
fn holding_enter_in_brain_search_toggles_the_highlighted_dossier_once() {
    let mut state = brain_results_fixture();
    state.brain.highlighted = Some(0);
    let mut runtime = Runtime::new(
        state,
        app::view,
        (1180.0, 800.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    runtime.frame(0.0);
    let search_input = |element: &rune::ElementInfo| {
        element.key.as_deref() == Some("input") && element.path.contains("/actor-search/")
    };
    let [x, y, width, height] = runtime
        .elements()
        .into_iter()
        .find(search_input)
        .expect("the real search editor")
        .rect;
    let (x, y) = (x + width / 2.0, y + height / 2.0);
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(1.0 / 60.0);
    assert!(runtime
        .elements()
        .iter()
        .any(|element| search_input(element) && element.focused));

    let enter = rune::KeyEvent {
        key: rune::Key::Enter,
        mods: rune::Mods::default(),
    };
    let revision = runtime.state.brain.detail_revision;
    runtime.key(enter.clone());
    runtime.frame(2.0 / 60.0);
    for step in 3..8 {
        runtime.repeat_key(enter.clone());
        runtime.frame(f64::from(step) / 60.0);
        assert!(runtime.find("actor-dossier").is_some());
    }
    assert_eq!(runtime.state.brain.detail_revision, revision + 1);
    assert_eq!(
        runtime.state.intents,
        vec![Intent::Dossier {
            actor_id: "first".into(),
        }]
    );

    runtime.key_up(enter.clone());
    runtime.key(enter.clone());
    runtime.frame(8.0 / 60.0);
    for step in 9..14 {
        runtime.repeat_key(enter.clone());
        runtime.frame(f64::from(step) / 60.0);
        assert!(runtime.find("actor-dossier").is_none());
    }
    assert_eq!(runtime.state.brain.detail_revision, revision + 2);
    assert_eq!(runtime.state.intents.len(), 1);
}
