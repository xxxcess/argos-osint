//! Scripted startup scenes with synthetic, authority-matched readiness observations.

use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::ReadinessCheck;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use fleetworks_desktop::accessibility::FixtureAccessibility;
use fleetworks_desktop::app::Intent;
use fleetworks_desktop::app::Load;
use fleetworks_desktop::backend::ReadKind;
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

fn embedded_setup(state: &mut State, readiness_scene: fn(&mut State)) {
    state.fixture_show_startup_setup(BackendKind::Local);
    readiness_scene(state);
    state.page = Page::Workstation;
    state.setup_gateway = true;
    state.seed_startup_bundled_dataset_fixture();
    state.connection.origin = TextEdit::new("local://embedded");
    state.local_backend.binary_status =
        BinaryDiscoveryStatus::Ready("/fixture/bundle/argos-api-host".into());
    state.fixture_effect = Some(reply);
}

fn reply(state: &mut State, intent: &Intent) {
    if !matches!(
        intent,
        Intent::BackendRead {
            kind: ReadKind::Profile
        }
    ) {
        super::reply(state, intent);
    }
}

pub(super) fn seed(state: &mut State) {
    embedded_setup(state, super::redesign_scenes::seed);
}

pub(super) fn embedded_health(state: &mut State) {
    seed(state);
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
    state.fixture_show_embedded_health_checks();
}

pub(super) fn index(state: &mut State) {
    embedded_setup(state, super::redesign_scenes::seed_ready);
}

pub(super) fn index_queued(state: &mut State) {
    index(state);
    state.maintenance.set_operation_job(
        MaintenanceOperation::ReindexActors,
        "synthetic-startup-index-job".into(),
        false,
    );
}

fn ready_actor_data(state: &mut State) {
    index(state);
    set_check(
        state,
        ReadinessCheckId::ActorSearch,
        ReadinessState::Ready,
        "index_reader_visible",
        "The synthetic actor index is reader-visible.",
    );
}

pub(super) fn checks_unavailable(state: &mut State) {
    seed(state);
    set_unavailable(state);
    state.fixture_effect = Some(|state, intent| {
        if matches!(intent, Intent::Readiness) {
            set_unavailable(state);
        } else {
            reply(state, intent);
        }
    });
}

fn set_unavailable(state: &mut State) {
    state.readiness = Load::Failed("Synthetic gateway could not read startup checks.".into());
    state.readiness_received_at = None;
}

pub(super) fn ready(state: &mut State, reduced_motion: bool) {
    ready_actor_data(state);
    state.accessibility.fixture_override = Some(FixtureAccessibility {
        reduced_motion,
        ..Default::default()
    });
}

pub(super) fn mixed_checks(state: &mut State) {
    ready(state, false);
    set_check(
        state,
        ReadinessCheckId::Jobs,
        ReadinessState::NotReady,
        "process_not_ready",
        "The synthetic Jobs service reports that its dispatcher is not ready.",
    );
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
