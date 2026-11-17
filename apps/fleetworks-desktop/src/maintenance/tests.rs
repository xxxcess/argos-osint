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

#[tokio::test(flavor = "current_thread")]
async fn delayed_readiness_reply_controls_reindex_confirmation_and_failure() {
    let mut runtime = runtime(state(false));
    let mut clock = 0.0;
    let Load::Ready(mut ready) = runtime.state.readiness.clone() else {
        panic!("synthetic readiness must start available")
    };
    ready
        .checks
        .iter_mut()
        .find(|check| check.id == ReadinessCheckId::ActorCatalog)
        .expect("synthetic catalog check")
        .state = ReadinessState::Ready;

    click(&mut runtime, "maintenance-refresh", &mut clock);
    let (release, response) = tokio::sync::oneshot::channel();
    let mock_gateway =
        tokio::spawn(async move { response.await.expect("mock gateway response released") });
    assert!(runtime.state.maintenance.confirmation.is_none());
    assert!(runtime
        .state
        .maintenance_disabled_reason(MaintenanceOperation::ReindexActors)
        .is_some());
    click(&mut runtime, "maintenance-reindex", &mut clock);
    assert!(
        runtime.state.maintenance.confirmation.is_none(),
        "the actual disabled control cannot confirm before catalog readiness arrives"
    );

    release
        .send(ready)
        .expect("deliver observed catalog readiness");
    let ready = mock_gateway.await.expect("mock gateway task");
    runtime.update(move |state, _| state.complete_readiness(Ok(ready)));
    clock += 1.0 / 60.0;
    runtime.frame(clock);
    assert!(runtime
        .state
        .maintenance_disabled_reason(MaintenanceOperation::ReindexActors)
        .is_none());
    click(&mut runtime, "maintenance-reindex", &mut clock);
    assert_eq!(
        runtime
            .state
            .maintenance
            .confirmation
            .as_ref()
            .expect("reindex confirmation after the response")
            .operation,
        MaintenanceOperation::ReindexActors
    );
    click(&mut runtime, "maintenance-confirm", &mut clock);
    assert_eq!(
        runtime
            .state
            .intents
            .iter()
            .filter(|intent| matches!(
                intent,
                Intent::Maintenance {
                    operation: MaintenanceOperation::ReindexActors
                }
            ))
            .count(),
        1,
        "the confirmed UI action must submit exactly one rebuild request"
    );

    click(&mut runtime, "maintenance-refresh", &mut clock);
    let (release, response) = tokio::sync::oneshot::channel();
    let mock_gateway =
        tokio::spawn(async move { response.await.expect("mock gateway failure released") });
    release
        .send(Err("synthetic readiness outage".to_owned()))
        .expect("deliver mock failure");
    let failure = mock_gateway.await.expect("mock gateway task");
    runtime.update(move |state, _| state.complete_readiness(failure));
    clock += 1.0 / 60.0;
    runtime.frame(clock);
    assert!(runtime
        .state
        .maintenance_disabled_reason(MaintenanceOperation::ReindexActors)
        .is_some());
    assert!(
        matches!(runtime.state.readiness, Load::Ready(_)),
        "a transient failure keeps the last same-scope checks as stale"
    );
    assert_eq!(
        runtime.state.readiness_error.as_deref(),
        Some("synthetic readiness outage")
    );
    click(&mut runtime, "maintenance-reindex", &mut clock);
    assert!(runtime.state.maintenance.confirmation.is_none());
    assert_eq!(
        runtime
            .state
            .intents
            .iter()
            .filter(|intent| matches!(
                intent,
                Intent::Maintenance {
                    operation: MaintenanceOperation::ReindexActors
                }
            ))
            .count(),
        1,
        "a failed readiness refresh cannot submit a second rebuild"
    );
}

#[test]
fn background_checks_are_bounded_to_the_focused_maintenance_page() {
    let mut state = state(false);
    state.scoped_logs.window_focused = true;
    state.refresh_readiness();
    // Fresh idle checks stay quiet until stale (> 5000ms).
    assert!(!state.maintenance_checks_due(1.9));
    assert!(!state.maintenance_checks_due(4.0));
    assert!(state.maintenance_checks_due(5.1));
    // Running checks poll on the 2s cadence even while fresh.
    if let Load::Ready(readiness) = &mut state.readiness {
        readiness.checks[1].state = ReadinessState::Running;
    }
    assert!(!state.maintenance_checks_due(1.9));
    assert!(state.maintenance_checks_due(2.0));
    state.page = Page::Workstation;
    assert!(state.maintenance_checks_due(2.0));
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

#[tokio::test]
async fn invalid_csv_columns_fail_locally_before_any_upload_link_or_stream_request() {
    use std::io::ErrorKind;
    let directory = tempfile::tempdir().expect("isolated selected CSV");
    for (kind, content) in [
        (DatasetKind::ActorCatalog, "sample_id,title\nfixture,Example\n"),
        (DatasetKind::ClassifierTraining, "actor_id,actor_name,actor_type,aliases,name_normalized\nfixture,Example,group,[],example\n"),
    ] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let client = Client::new(&format!("http://{}", listener.local_addr().unwrap()), None).unwrap();
        let path = directory.path().join("selected.csv");
        std::fs::write(&path, content).unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), upload(&client, LocalDataset { path, kind }))
            .await.expect("invalid columns must fail locally without waiting for HTTP");
        assert!(result.as_ref().is_err_and(|error| error.to_ascii_lowercase().contains("column")), "a file for the other purpose must report a local column error");
        assert!(matches!(listener.accept(), Err(error) if error.kind() == ErrorKind::WouldBlock),
            "header validation must finish before requesting an upload URL or sending bytes");
    }
}

#[test]
fn valid_local_actor_header_is_checked_without_changing_the_hashed_upload_bytes() {
    use std::io::Read;
    let directory = tempfile::tempdir().expect("isolated selected CSV");
    let path = directory.path().join("selected.csv");
    let content = b"\xef\xbb\xbfactor_id,actor_name,actor_type,aliases,name_normalized,description\r\nfixture,Example,group,[],example,\"comma, and quoted content\"\r\n";
    std::fs::write(&path, content).unwrap();
    let (mut file, digest, size) = open_and_hash(LocalDataset {
        path,
        kind: DatasetKind::ActorCatalog,
    })
    .unwrap();
    assert_eq!(size, content.len() as u64);
    assert_eq!(digest, format!("{:x}", Sha256::digest(content)));
    let mut actual = Vec::new();
    file.read_to_end(&mut actual).unwrap();
    assert_eq!(
        actual, content,
        "header inspection must rewind the same file for upload"
    );
}

#[test]
fn csv_hashing_checks_cancellation_between_bounded_chunks() {
    struct CancelAfterChunk<'a> {
        cancel: &'a AtomicBool,
        reads: usize,
    }
    impl std::io::Read for CancelAfterChunk<'_> {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            self.reads += 1;
            assert_eq!(
                self.reads, 1,
                "cancelled preparation cannot read another chunk"
            );
            assert_eq!(output.len(), 64 * 1024);
            output.fill(b'x');
            self.cancel.store(true, Ordering::Release);
            Ok(output.len())
        }
    }
    let cancel = AtomicBool::new(false);
    let mut reader = CancelAfterChunk {
        cancel: &cancel,
        reads: 0,
    };
    assert!(
        hash_selected_bytes(&mut reader, 256 * 1024 * 1024, 128 * 1024, &cancel)
            .unwrap_err()
            .contains("cancelled")
    );
    assert_eq!(reader.reads, 1);
}

#[tokio::test]
async fn cancelled_csv_preparation_retains_capacity_until_blocking_work_exits() {
    let slots = preparation_slots();
    let permit = slots.clone().acquire_owned().await.unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let guard = CancelPreparation(Arc::clone(&cancel));
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, finish) = std::sync::mpsc::channel();
    let work = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        entered.send(()).unwrap();
        finish.recv().unwrap();
    });
    started.await.unwrap();
    drop(guard);
    assert!(cancel.load(Ordering::Acquire));
    assert!(
        slots.clone().try_acquire_owned().is_err(),
        "cancellation must not admit a replacement reader while blocking work is alive"
    );
    release.send(()).unwrap();
    work.await.unwrap();
    let _next = tokio::time::timeout(std::time::Duration::from_secs(5), slots.acquire_owned())
        .await
        .expect("completed preparation must release capacity")
        .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn actor_reindex_persists_before_post_and_reconciles_uncertain_delivery() {
    use fleetworks_contracts::context::JobOrigin;
    use fleetworks_contracts::context::JobRequestAssociation;
    use fleetworks_contracts::context::OriginSource;
    use fleetworks_contracts::context::WORKSTATION_REQUEST_DETAIL_PATH;
    use fleetworks_workstation::native::NativeContainer;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let home = tempfile::tempdir().expect("isolated workstation home");
    let home_path = home.path().canonicalize().expect("canonical home");
    let container = NativeContainer::open_settings(&home_path).expect("protected container");

    let receipt = RegistrationReceipt::new(
        "synthetic-deployment".into(),
        "synthetic-workstation".into(),
        "synthetic-registration".into(),
        None,
        1,
    );
    let context = RegisteredContext::new(
        receipt.clone(),
        1,
        1,
        "synthetic-config".into(),
        "synthetic-access".into(),
    )
    .expect("registered context");
    let confirmation = MaintenanceConfirmation {
        operation: MaintenanceOperation::ReindexActors,
        context: context.clone(),
        origin_revision: 1,
        configuration_revision: 3,
        dataset: None,
    };

    let (request, entry) = prepare_maintenance_request(
        &confirmation,
        Some(&container),
        "https://gateway.example.test",
        None,
        None,
    )
    .expect("persist pre-POST actor reindex attempt");
    let pending = entry.expect("actor reindex writes pending attempt before HTTP");

    // Reopening from disk retains the exact request/context identity before HTTP runs.
    let reopened = std::sync::Arc::new(
        NativeContainer::open_settings(&home_path).expect("reopened container"),
    );
    let disk_pending = reopened
        .pending_actor_reindex("https://gateway.example.test")
        .expect("read pending reindex")
        .expect("pending attempt persisted");
    assert_eq!(disk_pending.request, request);
    assert_eq!(disk_pending.context, context);

    // Repeating preparation under valid authority replays the exact same request ID.
    let (replayed, _) = prepare_maintenance_request(
        &confirmation,
        Some(reopened.as_ref()),
        "https://gateway.example.test",
        None,
        None,
    )
    .expect("replay exact pending attempt");
    assert_eq!(replayed.request_id, request.request_id);

    // Changing configuration_revision refuses silent replacement while pending work is unresolved.
    let mut stale_confirmation = confirmation.clone();
    stale_confirmation.configuration_revision = 4;
    let stale_err = prepare_maintenance_request(
        &stale_confirmation,
        Some(reopened.as_ref()),
        "https://gateway.example.test",
        None,
        None,
    )
    .expect_err("stale configuration must refuse creating a second pending attempt");
    assert!(stale_err.contains("earlier registration or configuration"));

    // Simulate uncertain POST delivery followed by GET /api/v1/workstation/requests/detail lookup.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback request detail server");
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let expected_req_id = request.request_id.clone();
    let expected_context = context.clone();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept request_detail");
        let mut buf = Vec::new();
        while !buf.ends_with(b"\r\n\r\n") {
            let mut b = [0_u8; 1];
            assert_eq!(socket.read(&mut b).await.unwrap(), 1);
            buf.push(b[0]);
        }
        let head = String::from_utf8(buf).unwrap();
        let first_line = head.lines().next().unwrap_or_default().to_owned();
        let job_origin = JobOrigin::new(
            &expected_context,
            OriginSource::Native,
            expected_req_id,
            "2026-11-14T22:00:00Z".into(),
            3,
            Vec::new(),
        );
        let association = JobRequestAssociation::new(
            "job-reconciled-1".into(),
            job_origin,
            "2026-11-14T22:00:01Z".into(),
        );
        let body = serde_json::to_vec(&association).unwrap();
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        socket.write_all(reply.as_bytes()).await.unwrap();
        socket.write_all(&body).await.unwrap();
        first_line
    });

    let client = fleetworks_contracts::Client::new(&origin, Some("fixture-token"))
        .expect("loopback client")
        .with_registered_context(&context)
        .expect("registered context");
    let mut active_pending = Some(pending);
    let outcome = reconcile_uncertain_reindex(
        &client,
        Some(&reopened),
        "https://gateway.example.test",
        &mut active_pending,
        fleetworks_contracts::ClientError::Status(503),
    )
    .await;
    assert!(active_pending.is_none());
    match outcome {
        MaintenanceOutcome::Reconciled(association) => {
            assert_eq!(association.job_id, "job-reconciled-1");
        }
        _ => panic!("expected reconciled association"),
    }
    assert!(
        reopened
            .pending_actor_reindex("https://gateway.example.test")
            .unwrap()
            .is_none(),
        "reconciled request must clear durable pending journal"
    );
    let line = server.await.unwrap();
    assert!(line.starts_with(&format!("GET {WORKSTATION_REQUEST_DETAIL_PATH}?")));
    assert!(line.contains("operation=actor_reindex"));
}

#[test]
fn workstation_reindex_confirmation_guards_duplicates_supports_escape_and_refreshes_on_terminal_job(
) {
    use fleetworks_contracts::context::JobOrigin;
    use fleetworks_contracts::context::OriginSource;
    use fleetworks_contracts::readiness::MaintenanceReceipt;

    let mut s = state(true);
    s.page = crate::app::Page::Workstation;
    if let Load::Ready(readiness) = &mut s.readiness {
        for check in &mut readiness.checks {
            if check.id == ReadinessCheckId::ActorSearch {
                check
                    .actions
                    .push(fleetworks_contracts::readiness::ReadinessAction::new(
                        ReadinessActionId::RebuildActorIndex,
                        fleetworks_contracts::context::Permission::MaintainBackend,
                        true,
                    ));
            }
        }
    }

    s.preview_maintenance(MaintenanceOperation::ReindexActors);
    assert!(s.confirm_maintenance_dialog);
    assert!(s.maintenance.confirmation.is_some());

    // Simulate Loading receipt / in-flight operation blocking duplicate confirm_maintenance clicks.
    s.maintenance.receipt = Load::Loading;
    s.confirm_maintenance();
    assert!(
        s.confirm_maintenance_dialog,
        "duplicate click while receipt is already Loading must ignore confirmation without submitting"
    );

    // Idle receipt lets confirmation submit once.
    s.maintenance.receipt = Load::Idle;
    s.confirm_maintenance();
    assert!(!s.confirm_maintenance_dialog);
    assert_eq!(
        s.intents
            .iter()
            .filter(|i| matches!(
                i,
                Intent::Maintenance {
                    operation: MaintenanceOperation::ReindexActors
                }
            ))
            .count(),
        1
    );

    // Cancel clears confirmation dialog cleanly.
    s.preview_maintenance(MaintenanceOperation::ReindexActors);
    assert!(s.confirm_maintenance_dialog);
    s.cancel_maintenance_confirmation();
    assert!(!s.confirm_maintenance_dialog);
    assert!(s.maintenance.confirmation.is_none());

    // Validate receipt equality check without changing current page.
    let confirmation = MaintenanceConfirmation {
        operation: MaintenanceOperation::ReindexActors,
        context: s.registration.context.clone().unwrap(),
        origin_revision: s.origin_revision,
        configuration_revision: s.configuration_revision().unwrap(),
        dataset: None,
    };
    let req = MaintenanceRequest::for_reindex(
        "synthetic-reindex-req".into(),
        MutationContext::new(&confirmation.context).with_configuration_revision(0),
    )
    .unwrap();
    let origin = JobOrigin::new(
        &confirmation.context,
        OriginSource::Native,
        "synthetic-reindex-req".into(),
        "2026-11-14T22:00:00Z".into(),
        0,
        Vec::new(),
    );
    let receipt = MaintenanceReceipt::new(
        "job-reindex-99".into(),
        MaintenanceOperation::ReindexActors,
        origin,
        None,
        true,
    );
    assert!(valid_receipt(&receipt, &confirmation, &req));
    s.maintenance.receipt = Load::Ready(receipt);
    assert_eq!(s.page, crate::app::Page::Workstation);

    // When a running job finishes in scoped_jobs, readiness refreshes once while preserving rows/selection.
    let running: crate::backend::ActivityJob = serde_json::from_value(serde_json::json!({
        "id": "job-reindex-99",
        "app": "worker",
        "operation": "actor_reindex",
        "title": "Actor index",
        "state": "running",
        "phase": "scanning",
        "progress_done": 64,
        "progress_total": null,
        "error_summary": "",
        "private_details_available": true,
        "activity_actions_available": false,
        "status_only": false
    }))
    .unwrap();
    let completed: crate::backend::ActivityJob = serde_json::from_value(serde_json::json!({
        "id": "job-reindex-99",
        "app": "worker",
        "operation": "actor_reindex",
        "title": "Actor index",
        "state": "completed",
        "phase": "completed",
        "progress_done": 128,
        "progress_total": 128,
        "error_summary": "",
        "private_details_available": true,
        "activity_actions_available": false,
        "status_only": false
    }))
    .unwrap();
    let binding = crate::jobs_query::JobsBinding {
        context: s.registration.context.clone().unwrap(),
        connection_epoch: s.connection.epoch,
        origin_revision: s.origin_revision,
    };
    let mut initial_page = crate::jobs_query::JobsPage {
        rows: vec![running],
        activity_scope: fleetworks_contracts::context::ActivityScope::for_workstation(
            &binding.context,
        ),
        relation_counts_available: false,
        activity_actions_available: false,
        counts: None,
        dashboard_counts_scope: "unfiltered_top_level_workstation_activity".into(),
        matched_total: Some(1),
        observed_at: "2026-11-14T22:00:00Z".into(),
        apps: vec!["worker".into()],
        apps_complete: true,
        has_more: false,
        page_cursor: format!("{:064x}", 1),
        next_cursor: None,
    };
    if let Some(capabilities) = &mut s.connection.capabilities {
        capabilities
            .permissions
            .push(Permission::ReadWorkstationActivity);
    }
    s.sync_jobs_binding();
    s.request_jobs(crate::jobs_query::JobsEvent::Apply);
    let first_ticket = s
        .scoped_jobs
        .active_ticket()
        .expect("initial jobs ticket")
        .clone();
    s.complete_scoped_jobs(first_ticket, Ok(initial_page.clone()));
    s.select_job("job-reindex-99");
    let before_readiness_rev = s.readiness_revision;
    s.intents.clear();

    s.request_jobs(crate::jobs_query::JobsEvent::Poll);
    let poll_ticket = s
        .scoped_jobs
        .active_ticket()
        .expect("poll jobs ticket")
        .clone();
    s.fixture = false;
    initial_page.rows = vec![completed];
    s.complete_scoped_jobs(poll_ticket, Ok(initial_page));
    assert!(
        s.readiness_revision > before_readiness_rev,
        "terminal job completion must trigger one readiness refresh"
    );
    assert_eq!(s.backend.selected_job.as_deref(), Some("job-reindex-99"));
}

#[tokio::test]
async fn accepted_actor_reindex_recovers_job_id_across_restart_without_duplicate_post() {
    use fleetworks_contracts::context::JobOrigin;
    use fleetworks_contracts::context::OriginSource;
    use fleetworks_contracts::readiness::MaintenanceReceipt;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;

    let temporary = tempfile::tempdir().expect("temporary home");
    let home_path = temporary.path().canonicalize().expect("physical home");
    let container = Arc::new(
        fleetworks_workstation::native::NativeContainer::open_settings(&home_path)
            .expect("settings installation"),
    );

    let receipt = RegistrationReceipt::new(
        "synthetic-deployment".into(),
        "synthetic-workstation".into(),
        "synthetic-registration".into(),
        None,
        1,
    );
    let context =
        RegisteredContext::new(receipt, 1, 1, "configuration-a".into(), "access-a".into())
            .expect("context");
    let confirmation = MaintenanceConfirmation {
        operation: MaintenanceOperation::ReindexActors,
        context: context.clone(),
        origin_revision: 1,
        configuration_revision: 3,
        dataset: None,
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback reindex server");
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let post_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server_posts = Arc::clone(&post_count);
    let expected_context = context.clone();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept reindex POST");
        let mut buf = Vec::new();
        let header_end = loop {
            let mut chunk = [0_u8; 1024];
            let read = socket.read(&mut chunk).await.expect("read POST");
            assert!(read > 0);
            buf.extend_from_slice(&chunk[..read]);
            if let Some(pos) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
        let content_len = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())?
            })
            .expect("content-length");
        while buf.len() < header_end + content_len {
            let mut chunk = [0_u8; 1024];
            let read = socket.read(&mut chunk).await.expect("read POST body");
            assert!(read > 0);
            buf.extend_from_slice(&chunk[..read]);
        }
        let first_line = head.lines().next().unwrap_or_default().to_owned();
        assert!(first_line.starts_with(&format!("POST {ACTOR_REINDEX_PATH} ")));
        server_posts.fetch_add(1, Ordering::SeqCst);

        let request: MaintenanceRequest =
            serde_json::from_slice(&buf[header_end..header_end + content_len])
                .expect("decode MaintenanceRequest");
        let job_origin = JobOrigin::new(
            &expected_context,
            OriginSource::Native,
            request.request_id.clone(),
            "2026-11-14T22:00:00Z".into(),
            3,
            Vec::new(),
        );
        let receipt = MaintenanceReceipt::new(
            "job-accepted-restart-1".into(),
            MaintenanceOperation::ReindexActors,
            job_origin,
            None,
            false,
        );
        let body = serde_json::to_vec(&receipt).unwrap();
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        socket.write_all(reply.as_bytes()).await.unwrap();
        socket.write_all(&body).await.unwrap();
        request.request_id
    });

    let client = fleetworks_contracts::Client::new(&origin, Some("fixture-token"))
        .expect("loopback client")
        .with_registered_context(&context)
        .expect("registered context");
    let admitted = admit_maintenance_request(
        &client,
        Some(&container),
        &origin,
        &confirmation,
        None,
        None,
    )
    .await
    .expect("HTTP admission succeeds");
    let request_id = server.await.expect("server handled POST");
    assert_eq!(post_count.load(Ordering::SeqCst), 1);
    match &admitted.outcome {
        MaintenanceOutcome::Receipt(receipt) => {
            assert_eq!(receipt.job_id, "job-accepted-restart-1");
        }
        _ => panic!("expected admitted receipt"),
    }

    // Simulate closing/restarting before the UI handler processes and clears the receipt.
    drop(container);
    let reopened = Arc::new(
        fleetworks_workstation::native::NativeContainer::open_settings(&home_path)
            .expect("reopen container after restart"),
    );
    let persisted = reopened
        .pending_actor_reindex(&origin)
        .expect("read accepted reindex from journal")
        .expect("accepted attempt must survive restart before UI receipt clear");
    assert_eq!(persisted.request.request_id, request_id);
    assert_eq!(
        persisted.accepted_job_id.as_deref(),
        Some("job-accepted-restart-1")
    );

    // Repeating preparation before UI recovery must refuse creating a replacement POST.
    let duplicate_err =
        prepare_maintenance_request(&confirmation, Some(reopened.as_ref()), &origin, None, None)
            .expect_err("accepted job pending UI clear must block duplicate POST");
    assert!(duplicate_err.contains("job-accepted-restart-1"));

    // Reopening UI state recovers the exact accepted job_id without another POST.
    let mut reopened_state = state(true);
    reopened_state.connection.origin = rune::TextEdit::new(&origin);
    reopened_state.recover_pending_actor_reindex(persisted);
    assert!(reopened_state.maintenance.pending_reindex.is_none());
    let recovered = reopened_state
        .maintenance
        .reindex_association
        .as_ref()
        .expect("recovered accepted job association");
    assert_eq!(recovered.job_id, "job-accepted-restart-1");
    assert_eq!(recovered.origin.request_id, request_id);
    assert_eq!(post_count.load(Ordering::SeqCst), 1);

    // Explicitly verify normal UI receipt application also sets receipt Ready and clears pending.
    let mut normal_state = state(true);
    normal_state.connection.origin = rune::TextEdit::new(&origin);
    normal_state.apply_maintenance_outcome(MaintenanceOperation::ReindexActors, admitted, &origin);
    assert!(normal_state.maintenance.pending_reindex.is_none());
    assert!(matches!(
        &normal_state.maintenance.receipt,
        Load::Ready(r) if r.job_id == "job-accepted-restart-1"
    ));
}

#[tokio::test]
async fn non_fixture_receipt_persists_accepted_job_and_restores_after_restart_without_second_post()
{
    use fleetworks_contracts::context::JobOrigin;
    use fleetworks_contracts::context::OriginSource;
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;

    let temporary = tempfile::tempdir().expect("temporary home");
    let home_path = temporary.path().canonicalize().expect("physical home");
    let container = Arc::new(
        fleetworks_workstation::native::NativeContainer::open_settings(&home_path)
            .expect("settings container"),
    );
    let receipt = RegistrationReceipt::new(
        "synthetic-deployment".into(),
        "synthetic-workstation".into(),
        "synthetic-registration".into(),
        None,
        1,
    );
    let context =
        RegisteredContext::new(receipt, 1, 1, "configuration-a".into(), "access-a".into())
            .expect("context");
    let confirmation = MaintenanceConfirmation {
        operation: MaintenanceOperation::ReindexActors,
        context: context.clone(),
        origin_revision: 1,
        configuration_revision: 3,
        dataset: None,
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback reindex server");
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let post_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server_posts = Arc::clone(&post_count);
    let expected_context = context.clone();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept reindex POST");
        let mut buf = Vec::new();
        let header_end = loop {
            let mut chunk = [0_u8; 1024];
            let read = socket.read(&mut chunk).await.expect("read POST");
            assert!(read > 0);
            buf.extend_from_slice(&chunk[..read]);
            if let Some(pos) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
        let content_len = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())?
            })
            .expect("content-length");
        while buf.len() < header_end + content_len {
            let mut chunk = [0_u8; 1024];
            let read = socket.read(&mut chunk).await.expect("read POST body");
            assert!(read > 0);
            buf.extend_from_slice(&chunk[..read]);
        }
        let first_line = head.lines().next().unwrap_or_default().to_owned();
        assert!(first_line.starts_with(&format!("POST {ACTOR_REINDEX_PATH} ")));
        server_posts.fetch_add(1, Ordering::SeqCst);

        let request: MaintenanceRequest =
            serde_json::from_slice(&buf[header_end..header_end + content_len])
                .expect("decode MaintenanceRequest");
        let job_origin = JobOrigin::new(
            &expected_context,
            OriginSource::Native,
            request.request_id.clone(),
            "2026-11-15T01:00:00Z".into(),
            3,
            Vec::new(),
        );
        let receipt = MaintenanceReceipt::new(
            "job-accepted-nonfixture-1".into(),
            MaintenanceOperation::ReindexActors,
            job_origin,
            None,
            false,
        );
        let body = serde_json::to_vec(&receipt).unwrap();
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        socket.write_all(reply.as_bytes()).await.unwrap();
        socket.write_all(&body).await.unwrap();
        request.request_id
    });

    let client = fleetworks_contracts::Client::new(&origin, Some("fixture-token"))
        .expect("loopback client")
        .with_registered_context(&context)
        .expect("registered context");
    let admitted = admit_maintenance_request(
        &client,
        Some(&container),
        &origin,
        &confirmation,
        None,
        None,
    )
    .await
    .expect("HTTP admission succeeds");
    let request_id = server.await.expect("server handled POST");
    assert_eq!(post_count.load(Ordering::SeqCst), 1);

    let effects = Arc::new(GatewayEffects::new(tokio::runtime::Handle::current()));
    let mut non_fixture_state = State::with_services(effects);
    assert!(!non_fixture_state.fixture);
    non_fixture_state.connection.origin = rune::TextEdit::new(&origin);
    non_fixture_state.connection.status = ConnectionStatus::Connected;
    non_fixture_state.registration.context = Some(context.clone());
    non_fixture_state.workspaces.container = Some(Arc::clone(&container));
    non_fixture_state.apply_maintenance_outcome(
        MaintenanceOperation::ReindexActors,
        admitted,
        &origin,
    );
    assert!(non_fixture_state.maintenance.pending_reindex.is_none());
    assert!(matches!(
        &non_fixture_state.maintenance.receipt,
        Load::Ready(r) if r.job_id == "job-accepted-nonfixture-1"
    ));

    // Wait for non-fixture persistence task to promote pending guard into accepted association.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        // The catalog lock is deliberately non-blocking, so a read that overlaps the
        // persistence task reports "busy". Treat that as "not settled yet" and retry.
        let settled = match (
            container.pending_actor_reindex(&origin),
            container.accepted_actor_reindex(&origin),
        ) {
            (Ok(pending), Ok(accepted)) => pending.is_none() && accepted.is_some(),
            (Err(error), _) | (_, Err(error)) => {
                assert!(
                    format!("{error:#}").contains("catalog is busy"),
                    "unexpected non-fixture persistence read error: {error:#}"
                );
                false
            }
        };
        if settled {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for non-fixture accepted-job persistence"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    drop(non_fixture_state);
    drop(container);

    // Restart with a fresh non-fixture state and reopened workstation container.
    let reopened = Arc::new(
        fleetworks_workstation::native::NativeContainer::open_settings(&home_path)
            .expect("reopen container after non-fixture receipt"),
    );
    let pending_after_restart = reopened
        .pending_actor_reindex(&origin)
        .expect("read pending after restart");
    assert!(pending_after_restart.is_none());
    let accepted_after_restart = reopened
        .accepted_actor_reindex(&origin)
        .expect("read accepted after restart");
    assert_eq!(
        accepted_after_restart
            .as_ref()
            .map(|entry| entry.association.job_id.as_str()),
        Some("job-accepted-nonfixture-1")
    );

    let effects_reopened = Arc::new(GatewayEffects::new(tokio::runtime::Handle::current()));
    let mut reopened_state = State::with_services(effects_reopened);
    assert!(!reopened_state.fixture);
    reopened_state.connection.origin = rune::TextEdit::new(&origin);
    reopened_state.connection.status = ConnectionStatus::Connected;
    reopened_state.registration.context = Some(context);
    reopened_state.workspaces.container = Some(reopened);
    reopened_state
        .apply_restored_actor_reindex_journal(pending_after_restart, accepted_after_restart);
    let restored = reopened_state
        .maintenance
        .reindex_association
        .as_ref()
        .expect("restored accepted job association without another POST");
    assert_eq!(restored.job_id, "job-accepted-nonfixture-1");
    assert_eq!(restored.origin.request_id, request_id);
    assert!(reopened_state.maintenance.pending_reindex.is_none());
    assert_eq!(post_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn exact_retry_auth_rejection_preserves_journal_for_read_only_reconciliation() {
    use fleetworks_contracts::context::JobOrigin;
    use fleetworks_contracts::context::OriginSource;
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;

    let temporary = tempfile::tempdir().expect("temporary home");
    let home_path = temporary.path().canonicalize().expect("physical home");
    let container = Arc::new(
        fleetworks_workstation::native::NativeContainer::open_settings(&home_path)
            .expect("settings installation"),
    );

    let receipt = RegistrationReceipt::new(
        "synthetic-deployment".into(),
        "synthetic-workstation".into(),
        "synthetic-registration".into(),
        None,
        1,
    );
    let context =
        RegisteredContext::new(receipt, 1, 1, "configuration-a".into(), "access-a".into())
            .expect("context");
    let confirmation = MaintenanceConfirmation {
        operation: MaintenanceOperation::ReindexActors,
        context: context.clone(),
        origin_revision: 1,
        configuration_revision: 3,
        dataset: None,
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback auth-retry server");
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let expected_context = context.clone();

    let server = tokio::spawn(async move {
        // 1. Initial POST arrives, server accepts work, then connection drops (lost reply).
        let recorded_request_id = {
            let (mut socket, _) = listener.accept().await.expect("accept initial POST");
            let mut buf = Vec::new();
            let header_end = loop {
                let mut chunk = [0_u8; 1024];
                let read = socket.read(&mut chunk).await.expect("read POST head");
                assert!(read > 0);
                buf.extend_from_slice(&chunk[..read]);
                if let Some(pos) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
                    break pos + 4;
                }
            };
            let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
            let content_len = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())?
                })
                .expect("content-length");
            while buf.len() < header_end + content_len {
                let mut chunk = [0_u8; 1024];
                let read = socket.read(&mut chunk).await.expect("read POST body");
                assert!(read > 0);
                buf.extend_from_slice(&chunk[..read]);
            }
            let request: MaintenanceRequest =
                serde_json::from_slice(&buf[header_end..header_end + content_len])
                    .expect("decode MaintenanceRequest");
            drop(socket);
            request.request_id
        };

        // Helper closure-like loop steps for the subsequent HTTP exchanges:
        // 2. Initial read-only reconcile probe -> 503 Service Unavailable
        // 3. First exact retry POST -> 401 Unauthorized; probe -> 401 Unauthorized
        // 4. Second exact retry POST -> 403 Forbidden; probe -> 403 Forbidden
        // 5. Final read-only Check request GET -> 200 OK with original job association
        let responses = [
            ("GET ", "503 Service Unavailable", None),
            ("POST ", "401 Unauthorized", None),
            ("GET ", "401 Unauthorized", None),
            ("POST ", "403 Forbidden", None),
            ("GET ", "403 Forbidden", None),
            ("GET ", "200 OK", Some(recorded_request_id.clone())),
        ];

        for (expected_method, status_line, assoc_req_id) in responses {
            let (mut socket, _) = listener.accept().await.expect("accept HTTP request");
            let mut buf = Vec::new();
            let header_end = loop {
                let mut chunk = [0_u8; 1024];
                let read = socket.read(&mut chunk).await.expect("read request");
                assert!(read > 0);
                buf.extend_from_slice(&chunk[..read]);
                if let Some(pos) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
                    break pos + 4;
                }
            };
            let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
            let content_len = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            while buf.len() < header_end + content_len {
                let mut chunk = [0_u8; 1024];
                let read = socket.read(&mut chunk).await.expect("read body");
                assert!(read > 0);
                buf.extend_from_slice(&chunk[..read]);
            }
            let first_line = head.lines().next().unwrap_or_default();
            assert!(
                first_line.starts_with(expected_method),
                "expected {expected_method}, got {first_line}"
            );
            let body = if let Some(req_id) = assoc_req_id {
                let job_origin = JobOrigin::new(
                    &expected_context,
                    OriginSource::Native,
                    req_id,
                    "2026-11-14T22:00:00Z".into(),
                    3,
                    Vec::new(),
                );
                let association = JobRequestAssociation::new(
                    "job-auth-retry-1".into(),
                    job_origin,
                    "2026-11-14T22:00:01Z".into(),
                );
                serde_json::to_vec(&association).unwrap()
            } else {
                format!("{{\"error\":\"{status_line}\"}}").into_bytes()
            };
            let reply = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(reply.as_bytes()).await.unwrap();
            socket.write_all(&body).await.unwrap();
        }
        recorded_request_id
    });

    let client = fleetworks_contracts::Client::new(&origin, Some("fixture-token"))
        .expect("loopback client")
        .with_registered_context(&context)
        .expect("registered context");

    // Step 1-2: initial submission loses reply and probe gets 503.
    let initial = admit_maintenance_request(
        &client,
        Some(&container),
        &origin,
        &confirmation,
        None,
        None,
    )
    .await
    .expect("initial uncertain delivery returns AdmittedMaintenance");
    assert!(matches!(initial.outcome, MaintenanceOutcome::Failed(_)));
    let pending_after_loss = container
        .pending_actor_reindex(&origin)
        .unwrap()
        .expect("lost reply must retain pending journal entry");
    let original_request = pending_after_loss.request.clone();

    // Step 3: exact retry gets 401 Unauthorized -> journal must keep original request.
    let retry_401 = admit_maintenance_request(
        &client,
        Some(&container),
        &origin,
        &confirmation,
        Some(original_request.clone()),
        Some(original_request.clone()),
    )
    .await
    .expect("401 retry preserves pending attempt");
    assert!(retry_401.pending_entry.is_some());
    assert!(matches!(retry_401.outcome, MaintenanceOutcome::Failed(_)));
    assert_eq!(
        container
            .pending_actor_reindex(&origin)
            .unwrap()
            .expect("401 on exact retry must not clear pending journal")
            .request
            .request_id,
        original_request.request_id
    );

    // Step 4: exact retry gets 403 Forbidden -> journal must still keep original request.
    let retry_403 = admit_maintenance_request(
        &client,
        Some(&container),
        &origin,
        &confirmation,
        Some(original_request.clone()),
        Some(original_request.clone()),
    )
    .await
    .expect("403 retry preserves pending attempt");
    assert!(retry_403.pending_entry.is_some());
    assert!(matches!(retry_403.outcome, MaintenanceOutcome::Failed(_)));
    let active_pending = container
        .pending_actor_reindex(&origin)
        .unwrap()
        .expect("403 on exact retry must not clear pending journal");
    assert_eq!(
        active_pending.request.request_id,
        original_request.request_id
    );

    // Step 5: subsequent read-only reconciliation establishes the original job and clears journal.
    let mut active = Some(active_pending);
    let reconciled = reconcile_uncertain_reindex(
        &client,
        Some(&container),
        &origin,
        &mut active,
        fleetworks_contracts::ClientError::Status(403),
    )
    .await;
    assert!(active.is_none());
    match reconciled {
        MaintenanceOutcome::Reconciled(association) => {
            assert_eq!(association.job_id, "job-auth-retry-1");
            assert_eq!(association.origin.request_id, original_request.request_id);
        }
        other => panic!(
            "expected read-only reconciliation after 401/403 retry, got {:?}",
            matches!(other, MaintenanceOutcome::Failed(_))
        ),
    }
    assert!(container.pending_actor_reindex(&origin).unwrap().is_none());
    let accepted = container
        .accepted_actor_reindex(&origin)
        .unwrap()
        .expect("reconciliation must retain bounded accepted job association");
    assert_eq!(accepted.association.job_id, "job-auth-retry-1");
    assert_eq!(accepted.request.request_id, original_request.request_id);
    let server_request_id = server.await.expect("server completed scripted sequence");
    assert_eq!(server_request_id, original_request.request_id);
}

mod csv_path_fallback {
    use std::ffi::OsStr;
    use std::path::PathBuf;

    use fleetworks_contracts::readiness::DatasetKind;

    use crate::app::Intent;
    use crate::maintenance::parse_local_csv_path;

    fn home() -> Option<&'static OsStr> {
        Some(OsStr::new("/home/operator"))
    }

    #[test]
    fn typed_paths_are_normalized_and_bad_input_is_rejected_with_a_reason() {
        let ok = |raw: &str| parse_local_csv_path(raw, home());
        assert_eq!(
            ok("/data/actors.csv"),
            Ok(PathBuf::from("/data/actors.csv"))
        );
        assert_eq!(
            ok("  /data/actors.CSV \n"),
            Ok(PathBuf::from("/data/actors.CSV"))
        );
        assert_eq!(
            ok("\"/data/my actors.csv\""),
            Ok(PathBuf::from("/data/my actors.csv"))
        );
        assert_eq!(ok("'/data/a.csv'"), Ok(PathBuf::from("/data/a.csv")));
        assert_eq!(
            ok("~/imports/a.csv"),
            Ok(PathBuf::from("/home/operator/imports/a.csv"))
        );

        for (raw, reason) in [
            ("", "Enter the full path"),
            ("   ", "Enter the full path"),
            ("https://example.test/a.csv", "not a URL"),
            ("file:///data/a.csv", "not a URL"),
            ("data:text/csv,a", "not a URL"),
            ("relative/a.csv", "full path"),
            ("./a.csv", "full path"),
            ("/data/a.txt", ".csv"),
            ("/data/archive", ".csv"),
            ("/data/a\u{0}.csv", "cannot be used"),
            ("/data/a\n.csv", "cannot be used"),
        ] {
            let error = ok(raw).expect_err(raw);
            assert!(error.contains(reason), "{raw:?} -> {error}");
        }
        assert!(parse_local_csv_path("~/a.csv", None)
            .expect_err("no home")
            .contains("Cannot expand"));
    }

    #[test]
    fn an_unavailable_chooser_opens_the_prompt_and_a_valid_path_reuses_the_upload_flow() {
        let mut state = super::state(false);
        state.open_csv_path_prompt(DatasetKind::ActorCatalog);
        assert!(state
            .maintenance
            .message
            .as_deref()
            .is_some_and(|m| m.contains("could not open")));
        assert!(state.maintenance.csv_path_prompt.is_some());

        // A bad path keeps the prompt open with the reason and starts nothing.
        state
            .maintenance
            .csv_path_prompt
            .as_mut()
            .expect("prompt")
            .input = rune::TextEdit::new("https://example.test/a.csv".to_owned());
        state.submit_csv_path();
        let prompt = state
            .maintenance
            .csv_path_prompt
            .as_ref()
            .expect("still open");
        assert!(prompt
            .error
            .as_deref()
            .is_some_and(|e| e.contains("not a URL")));
        assert!(state.maintenance.local_dataset.is_none());

        // A valid path selects the dataset and asks for the same upload as the picker does.
        state
            .maintenance
            .csv_path_prompt
            .as_mut()
            .expect("prompt")
            .input = rune::TextEdit::new("/data/actors.csv".to_owned());
        state.submit_csv_path();
        assert!(state.maintenance.csv_path_prompt.is_none());
        let selected = state.maintenance.local_dataset.as_ref().expect("selected");
        assert_eq!(selected.path, PathBuf::from("/data/actors.csv"));
        assert_eq!(selected.kind, DatasetKind::ActorCatalog);
        assert!(state.intents.iter().any(|intent| matches!(
            intent,
            Intent::UploadDataset {
                kind: DatasetKind::ActorCatalog
            }
        )));
    }

    #[test]
    fn the_prompt_is_drawn_in_the_matching_card_and_submits_from_its_button() {
        let mut runtime = super::runtime(super::state(false));
        runtime.update(|state, _| state.open_csv_path_prompt(DatasetKind::ActorCatalog));
        runtime.frame(1.0 / 60.0);
        assert!(
            runtime.find("maintenance-csv-path").is_some(),
            "path field shown"
        );
        assert!(runtime.find("maintenance-csv-path-submit").is_some());
        assert!(runtime.find("maintenance-csv-path-cancel").is_some());

        runtime.update(|state, _| {
            state
                .maintenance
                .csv_path_prompt
                .as_mut()
                .expect("prompt")
                .input = rune::TextEdit::new("/data/actors.csv".to_owned());
        });
        let mut clock = 0.1;
        super::click(&mut runtime, "maintenance-csv-path-submit", &mut clock);
        assert!(runtime.state.maintenance.csv_path_prompt.is_none());
        assert_eq!(
            runtime
                .state
                .maintenance
                .local_dataset
                .as_ref()
                .map(|selection| selection.path.clone()),
            Some(PathBuf::from("/data/actors.csv"))
        );
    }

    #[test]
    fn cancel_and_origin_change_close_the_prompt() {
        let mut state = crate::tests::registered_fixture();
        state.open_csv_path_prompt(DatasetKind::ClassifierTraining);
        state.cancel_csv_path_prompt();
        assert!(state.maintenance.csv_path_prompt.is_none());

        state.open_csv_path_prompt(DatasetKind::ActorCatalog);
        state.maintenance.invalidate_origin();
        assert!(state.maintenance.csv_path_prompt.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_typed_symlink_gets_the_same_rejection_as_a_picked_one() {
        use crate::maintenance::LocalDataset;
        use std::io::Write;

        let dir = tempfile::tempdir().expect("temp dir");
        let real = dir
            .path()
            .canonicalize()
            .expect("canonical")
            .join("real.csv");
        std::fs::File::create(&real)
            .and_then(|mut file| file.write_all(b"id\n1\n"))
            .expect("write csv");
        let link = real.with_file_name("link.csv");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");

        let typed = parse_local_csv_path(link.to_str().expect("utf-8 path"), None)
            .expect("a symlink path is well formed; the shared preparation rejects it");
        let error = super::super::open_and_hash(LocalDataset {
            path: typed,
            kind: DatasetKind::ActorCatalog,
        })
        .expect_err("symlink rejected");
        assert!(error.contains("symbolic links are rejected"), "{error}");
    }
}
