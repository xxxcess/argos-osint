//! Scripted startup scenes with synthetic, authority-matched readiness observations.

use fleetworks_contracts::readiness::ReadinessCheck;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use fleetworks_desktop::accessibility::FixtureAccessibility;
use fleetworks_desktop::app::Load;
use fleetworks_desktop::local_backend::BinaryDiscoveryStatus;
use fleetworks_desktop::workspaces::WorkspaceSnapshot;
use fleetworks_desktop::Page;
use fleetworks_desktop::State;
use fleetworks_workstation::native::BackendKind;
use rune::TextEdit;

pub(super) fn folders(state: &mut State) {
    state.fixture_show_backend_choice();
    state.workspaces.snapshot = Load::Ready(WorkspaceSnapshot {
        root: "/fixture/home/.fleetworks".into(),
        installation_marker: "synthetic-installation".into(),
        rows: Vec::new(),
        operations: Vec::new(),
        registration_attempts: Vec::new(),
        rebind_attempts: Vec::new(),
    });
    state.connection_profiles.catalog = Load::Ready(
        serde_json::from_value(serde_json::json!({
            "revision": 1,
            "selected_profile_id": null,
            "profiles": [{
                "id": "a".repeat(64),
                "revision": 1,
                "kind": "local",
                "name": "Field workstation",
                "origin": "local://field-workstation",
                "local_state_dir": "/fixture/Workstations/Field workstation"
            }],
        }))
        .expect("synthetic Embedded folder catalog"),
    );
}

fn embedded_setup(state: &mut State) {
    state.fixture_show_startup_setup(BackendKind::Local);
    super::redesign_scenes::seed(state);
    state.page = Page::Workstation;
    state.setup_gateway = true;
    state.connection.origin = TextEdit::new("local://embedded");
    state.local_backend.binary_status =
        BinaryDiscoveryStatus::Ready("/fixture/bundle/argos-api-host".into());
    set_check(
        state,
        ReadinessCheckId::ActorCatalog,
        ReadinessState::NotReady,
        "database_seed_incomplete",
        "The synthetic actor catalog needs its first seed.",
    );
    set_check(
        state,
        ReadinessCheckId::ActorSearch,
        ReadinessState::NotReady,
        "index_not_built",
        "The synthetic actor index has not been built.",
    );
    set_check(
        state,
        ReadinessCheckId::Jobs,
        ReadinessState::NotApplicable,
        "embedded_backend",
        "This standalone service check does not apply to Embedded.",
    );
}

pub(super) fn embedded_health(state: &mut State) {
    embedded_setup(state);
    state.fixture_show_embedded_health_checks();
}

pub(super) fn checks_unavailable(state: &mut State) {
    embedded_health(state);
    state.readiness = Load::Failed("Synthetic gateway could not read startup checks.".into());
    state.readiness_received_at = None;
}

pub(super) fn registered(state: &mut State, reduced_motion: bool) {
    embedded_setup(state);
    state.fixture_show_embedded_registration();
    state.accessibility.fixture_override = Some(FixtureAccessibility {
        reduced_motion,
        ..Default::default()
    });
}

pub(super) fn mixed_checks(state: &mut State) {
    embedded_health(state);
    set_check(
        state,
        ReadinessCheckId::Datastore,
        ReadinessState::Ready,
        "datastore_reachable",
        "The last datastore observation is no longer current.",
    )
    .fresh_for_ms = 0;
}

fn set_check<'a>(
    state: &'a mut State,
    id: ReadinessCheckId,
    status: ReadinessState,
    reason: &str,
    explanation: &str,
) -> &'a mut ReadinessCheck {
    let Load::Ready(readiness) = &mut state.readiness else {
        panic!("Synthetic setup must contain scoped readiness");
    };
    let check = readiness
        .checks
        .iter_mut()
        .find(|check| check.id == id)
        .expect("synthetic setup check");
    check.state = status;
    check.reason_code = reason.into();
    check.explanation = explanation.into();
    check
}
