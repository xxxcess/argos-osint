use super::*;
use crate::app;
use crate::app::Page;
use crate::backend::ProfileSnapshot;
use crate::connection::ConnectionStatus;
use fleetworks_contracts::capabilities::Capabilities;
use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::readiness::ReadinessAction;
use fleetworks_contracts::readiness::ReadinessCheck;
use fleetworks_contracts::readiness::WorkstationReadiness;
use rune::Env;
use rune::Runtime;

fn state(catalog_ready: bool) -> State {
    let mut state = State::fixture();
    state.page = Page::Seed;
    state.setup_gateway = false;
    state.connection.status = ConnectionStatus::Connected;
    let mut capabilities = Capabilities::default();
    capabilities.deployment_id = Some("synthetic-deployment".into());
    capabilities.permissions = vec![Permission::ReadBackend, Permission::MaintainBackend];
    state.connection.capabilities = Some(capabilities);
    let receipt = RegistrationReceipt::new(
        "synthetic-deployment".into(),
        "synthetic-workstation".into(),
        "synthetic-registration".into(),
        None,
        1,
    );
    state.registration.context = Some(
        RegisteredContext::new(
            receipt.clone(),
            1,
            1,
            "configuration-a".into(),
            "access-a".into(),
        )
        .unwrap(),
    );
    let mut checks = Vec::new();
    for (id, action) in [
        (
            ReadinessCheckId::ActorCatalog,
            ReadinessActionId::SeedActors,
        ),
        (
            ReadinessCheckId::ActorSearch,
            ReadinessActionId::RebuildActorIndex,
        ),
        (
            ReadinessCheckId::ClassifierTraining,
            ReadinessActionId::TrainClassifier,
        ),
    ] {
        let ready = id != ReadinessCheckId::ActorCatalog || catalog_ready;
        let mut check = ReadinessCheck::new(
            id,
            if ready {
                ReadinessState::Ready
            } else {
                ReadinessState::NotReady
            },
            "synthetic".into(),
            "Synthetic maintenance availability".into(),
            "2026-11-13T00:00:00Z".into(),
        );
        check.fresh_for_ms = 5000;
        check.actions.push(ReadinessAction::new(
            action,
            Permission::MaintainBackend,
            true,
        ));
        checks.push(check);
    }
    let mut readiness = WorkstationReadiness::new(
        "synthetic-deployment".into(),
        "2026-11-13T00:00:00Z".into(),
        checks,
    );
    readiness.registration = Some(receipt);
    state.readiness = Load::Ready(readiness);
    state.readiness_received_at = Some(0.0);
    state.backend.profile = Load::Ready(ProfileSnapshot {
        configuration_revision: 0,
        configuration_format: 1,
        accounts: vec![],
        tool_credentials: vec![],
        setup_required: false,
        portable_schema_version: 4,
    });
    state
}

fn runtime(state: State) -> Runtime<State> {
    let mut runtime = Runtime::new(
        state,
        app::view,
        (1180.0, 1200.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    runtime.frame(0.0);
    runtime
}

fn click(runtime: &mut Runtime<State>, key: &str, clock: &mut f64) {
    runtime.update(|_, tx| tx.scroll_into_view(key));
    *clock += 1.0 / 60.0;
    runtime.frame(*clock);
    let (x, y) = runtime.find(key).expect("actual maintenance control");
    assert!(
        (0.0..1180.0).contains(&x) && (140.0..1200.0).contains(&y),
        "control must be inside the scroll viewport"
    );
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    *clock += 1.0 / 60.0;
    runtime.frame(*clock);
}

fn uploaded(kind: DatasetKind) -> DatasetUpload {
    serde_json::from_value(serde_json::json!({"dataset":{"id":"synthetic-upload","kind":kind,"sha256":"a".repeat(64),"size_bytes":128},"expires_in_seconds":3600})).unwrap()
}

#[test]
fn actor_and_training_buttons_request_their_own_csv_picker_without_submitting_jobs() {
    for (key, kind) in [
        ("maintenance-seed", DatasetKind::ActorCatalog),
        ("maintenance-train", DatasetKind::ClassifierTraining),
    ] {
        let mut runtime = runtime(state(false));
        let mut clock = 0.0;
        click(&mut runtime, key, &mut clock);
        assert_eq!(runtime.state.intents, vec![Intent::ChooseDataset { kind }]);
        assert!(runtime.state.maintenance.confirmation.is_none());
    }
}

#[test]
fn uploaded_csv_requires_confirmation_and_keeps_initial_configuration_revision() {
    for (kind, operation) in [
        (DatasetKind::ActorCatalog, MaintenanceOperation::SeedActors),
        (
            DatasetKind::ClassifierTraining,
            MaintenanceOperation::TrainClassifier,
        ),
    ] {
        let mut runtime = runtime(state(false));
        let mut clock = 0.0;
        let epoch = runtime.state.connection.epoch;
        let origin = runtime.state.origin_revision;
        runtime.update(|state, _| {
            state.complete_dataset_upload(epoch, 0, origin, kind, Ok(uploaded(kind)))
        });
        clock += 1.0 / 60.0;
        runtime.frame(clock);
        let confirmation = runtime.state.maintenance.confirmation.as_ref().unwrap();
        assert_eq!(confirmation.operation, operation);
        assert_eq!(confirmation.configuration_revision, 0);
        assert_eq!(confirmation.dataset.as_ref().unwrap().kind, kind);
        assert!(!runtime
            .state
            .intents
            .iter()
            .any(|intent| matches!(intent, Intent::Maintenance { .. })));
        click(&mut runtime, "maintenance-confirm", &mut clock);
        assert!(runtime
            .state
            .intents
            .contains(&Intent::Maintenance { operation }));
    }
}

#[test]
fn index_control_stays_disabled_until_completed_nonempty_catalog_is_observed() {
    let mut runtime = runtime(state(false));
    let mut clock = 0.0;
    click(&mut runtime, "maintenance-reindex", &mut clock);
    assert!(runtime.state.maintenance.confirmation.is_none());
    assert!(runtime
        .state
        .maintenance_disabled_reason(MaintenanceOperation::ReindexActors)
        .is_some());
    runtime.update(|state, _| {
        let Load::Ready(readiness) = &mut state.readiness else {
            panic!("fixture readiness")
        };
        readiness
            .checks
            .iter_mut()
            .find(|check| check.id == ReadinessCheckId::ActorCatalog)
            .unwrap()
            .state = ReadinessState::Ready;
    });
    clock += 1.0 / 60.0;
    runtime.frame(clock);
    click(&mut runtime, "maintenance-reindex", &mut clock);
    assert_eq!(
        runtime
            .state
            .maintenance
            .confirmation
            .as_ref()
            .unwrap()
            .operation,
        MaintenanceOperation::ReindexActors
    );
    assert!(runtime
        .state
        .maintenance
        .confirmation
        .as_ref()
        .unwrap()
        .dataset
        .is_none());
}

#[test]
fn failed_wrong_purpose_and_stale_uploads_never_confirm_or_submit() {
    for case in 0..3 {
        let mut state = state(false);
        let epoch = state.connection.epoch;
        let origin = state.origin_revision;
        let result = if case == 0 {
            Err("CSV validation failed".into())
        } else {
            Ok(uploaded(DatasetKind::ClassifierTraining))
        };
        state.complete_dataset_upload(
            epoch,
            if case == 2 { 1 } else { 0 },
            origin,
            DatasetKind::ActorCatalog,
            result,
        );
        assert!(state.maintenance.confirmation.is_none());
        assert!(state.maintenance.selected_dataset.is_none());
        assert!(!state
            .intents
            .iter()
            .any(|intent| matches!(intent, Intent::Maintenance { .. })));
    }
}

#[test]
fn refresh_checks_reloads_read_only_availability_without_starting_work() {
    let mut runtime = runtime(state(false));
    let mut clock = 0.0;
    runtime.state.clock = 10.0;
    assert!(runtime
        .state
        .maintenance_disabled_reason(MaintenanceOperation::SeedActors)
        .is_some());
    click(&mut runtime, "maintenance-refresh", &mut clock);
    assert!(runtime.state.intents.contains(&Intent::Readiness));
    assert!(runtime
        .state
        .intents
        .iter()
        .all(|intent| matches!(intent, Intent::Readiness | Intent::BackendRead { .. })));
}

#[test]
fn background_checks_are_bounded_to_the_focused_maintenance_page() {
    let mut state = state(false);
    state.scoped_logs.window_focused = true;
    state.refresh_readiness();
    assert!(!state.maintenance_checks_due(3.9));
    assert!(state.maintenance_checks_due(4.0));
    state.scoped_logs.window_focused = false;
    assert!(!state.maintenance_checks_due(10.0));
    state.scoped_logs.window_focused = true;
    state.page = Page::Brain;
    assert!(!state.maintenance_checks_due(10.0));
    state.page = Page::Seed;
    state.setup_gateway = true;
    assert!(!state.maintenance_checks_due(10.0));
    state.setup_gateway = false;
    state.connection.status = ConnectionStatus::Disconnected;
    assert!(!state.maintenance_checks_due(10.0));
}

#[test]
fn accepted_reseed_cannot_use_a_cached_ready_catalog_to_enable_index() {
    let mut state = state(true);
    state.maintenance.seed_accepted_at = Some("2026-11-13T00:00:01Z".into());
    assert!(state
        .maintenance_disabled_reason(MaintenanceOperation::ReindexActors)
        .is_some());
    let Load::Ready(readiness) = &mut state.readiness else {
        panic!("fixture readiness")
    };
    let catalog = readiness
        .checks
        .iter_mut()
        .find(|check| check.id == ReadinessCheckId::ActorCatalog)
        .unwrap();
    catalog.observed_at = "2026-11-13T00:00:02Z".into();
    assert!(state
        .maintenance_disabled_reason(MaintenanceOperation::ReindexActors)
        .is_none());
}
