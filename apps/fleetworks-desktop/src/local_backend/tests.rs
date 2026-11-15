use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::time::Duration;

use fleetworks_contracts::capabilities::Capabilities;
use fleetworks_contracts::capabilities::ConnectCapabilities;
use fleetworks_contracts::capabilities::ResourceScopes;
use fleetworks_contracts::capabilities::WorkspaceControlCapabilities;
use fleetworks_contracts::capabilities::CONNECT_REGISTRY_FEATURE;
use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::ipc::read_frame;
use fleetworks_contracts::ipc::write_frame;
use fleetworks_contracts::ipc::IpcEnvelope;
use fleetworks_contracts::ipc::IpcEvent;
use fleetworks_contracts::ipc::IpcHelloAck;
use fleetworks_contracts::ipc::LocalIpcClient;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::ReadinessAction;
use fleetworks_contracts::readiness::ReadinessActionId;
use fleetworks_contracts::readiness::ReadinessCheck;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use fleetworks_contracts::readiness::WorkstationReadiness;
use fleetworks_contracts::Ready;
use fleetworks_workstation::native::BackendKind;

use super::*;
use crate::app::Load;
use crate::app::State;
use crate::backend::ProfileSnapshot;
use crate::connection::ConnectionStatus;
use crate::connection::GatewayEffects;

fn local_capabilities(features: &[&str]) -> Capabilities {
    let mut capabilities = Capabilities::default();
    capabilities.deployment_id = Some("local-deployment".into());
    capabilities.configuration_epoch = Some("local-config-v1".into());
    capabilities.features = std::iter::once(CONNECT_REGISTRY_FEATURE)
        .chain(features.iter().copied())
        .map(str::to_owned)
        .collect();
    capabilities.connect = Some(ConnectCapabilities::default());
    capabilities.workspace_control = Some(WorkspaceControlCapabilities::default());
    capabilities.resource_scopes = Some(ResourceScopes::default());
    capabilities.permissions = vec![
        Permission::ReadBackend,
        Permission::MaintainBackend,
        Permission::ReadWorkstationActivity,
    ];
    capabilities
}

fn local_ready() -> Ready {
    serde_json::from_value(serde_json::json!({
        "service": "argos-api",
        "api_version": fleetworks_contracts::API_VERSION,
        "ready": true,
        "storage": true,
        "instance_id": "local-instance-1"
    }))
    .expect("valid Ready payload")
}

fn register_local_state(state: &mut State, process: Arc<LocalBackendProcess>) {
    state.backend_kind = BackendKind::Local;
    state.connection.origin = rune::TextEdit::new("local://embedded");
    state.connection.status = ConnectionStatus::Connected;
    state.client = Some(process.client().clone());
    state.connection.ready = Some(process.ready().clone());
    state.connection.capabilities = Some(process.capabilities().clone());
    state.local_backend.state_dir = Some(process.state_dir().to_path_buf());
    state.local_backend.session = Some(process);

    let receipt = RegistrationReceipt::new(
        "local-deployment".into(),
        "local-node".into(),
        "local-registration".into(),
        None,
        1,
    );
    state.registration.context = Some(
        RegisteredContext::new(
            receipt.clone(),
            1,
            1,
            "local-config-v1".into(),
            "local-access".into(),
        )
        .expect("registered local context"),
    );

    let mut checks = Vec::new();
    for (id, action) in [
        (
            ReadinessCheckId::ActorCatalog,
            ReadinessActionId::SeedActors,
        ),
        (
            ReadinessCheckId::ClassifierTraining,
            ReadinessActionId::TrainClassifier,
        ),
    ] {
        let mut check = ReadinessCheck::new(
            id,
            ReadinessState::Ready,
            "local".into(),
            "Local check".into(),
            "2026-11-14T00:00:00Z".into(),
        );
        check.fresh_for_ms = 5000;
        check.actions.push(ReadinessAction::new(
            action,
            Permission::MaintainBackend,
            true,
        ));
        checks.push(check);
    }
    let mut activate_check = ReadinessCheck::new(
        ReadinessCheckId::Classifier,
        ReadinessState::Unavailable,
        "classifier_disabled".into(),
        "Classifier optional module unavailable".into(),
        "2026-11-14T00:00:00Z".into(),
    );
    activate_check.fresh_for_ms = 5000;
    activate_check.actions.push(ReadinessAction::new(
        ReadinessActionId::ActivateClassifier,
        Permission::MaintainBackend,
        true,
    ));
    checks.push(activate_check);

    let mut readiness = WorkstationReadiness::new(
        "local-deployment".into(),
        "2026-11-14T00:00:00Z".into(),
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
}

#[test]
fn companion_executable_validation_rejects_symlinks_public_write_and_missing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let valid = root.join(COMPANION_BINARY_NAME);
    std::fs::write(&valid, b"#!/bin/sh\nexit 0\n").expect("write binary");
    std::fs::set_permissions(&valid, std::fs::Permissions::from_mode(0o700))
        .expect("chmod 700 binary");
    assert_eq!(
        validate_companion_executable(&valid).expect("valid companion binary"),
        valid
    );

    let link = root.join("symlink-api-host");
    std::os::unix::fs::symlink(&valid, &link).expect("create symlink");
    assert!(validate_companion_executable(&link).is_err());

    let non_exec = root.join("non-exec-host");
    std::fs::write(&non_exec, b"data").expect("write non-exec");
    std::fs::set_permissions(&non_exec, std::fs::Permissions::from_mode(0o600)).expect("chmod 600");
    assert!(validate_companion_executable(&non_exec).is_err());

    let group_writable = root.join("group-writable-host");
    std::fs::write(&group_writable, b"data").expect("write writable");
    std::fs::set_permissions(&group_writable, std::fs::Permissions::from_mode(0o770))
        .expect("chmod 770");
    assert!(validate_companion_executable(&group_writable).is_err());

    assert!(validate_companion_executable(Path::new("relative/argos-api-host")).is_err());
    assert!(validate_companion_executable(&root.join("missing")).is_err());

    let deps_exe = root.join("deps").join("desktop-test");
    std::fs::create_dir_all(deps_exe.parent().expect("deps dir")).expect("create deps dir");
    std::fs::write(&deps_exe, b"bin").expect("write dummy exe");
    assert_eq!(
        resolve_companion_near_exe(&deps_exe).expect("resolve Cargo profile sibling"),
        valid
    );
}

#[test]
fn build_companion_command_scrubs_remote_secrets_and_sets_local_ipc_args() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().canonicalize().expect("canonical tempdir");
    let bin = root.join(COMPANION_BINARY_NAME);
    std::fs::write(&bin, b"#!/bin/sh\nexit 0\n").expect("write binary");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).expect("chmod");
    let state_dir = root.join("state");
    std::fs::create_dir(&state_dir).expect("create state dir");
    std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o700))
        .expect("chmod dir");

    let command = build_companion_command(&bin, &state_dir).expect("build command");
    let std_cmd = command.as_std();
    let args: Vec<_> = std_cmd
        .get_args()
        .map(|arg| arg.to_string_lossy())
        .collect();
    assert_eq!(
        args,
        vec![
            "--role",
            "local-ipc",
            "--state-dir",
            state_dir.to_string_lossy().as_ref()
        ]
    );
    let envs: Vec<_> = std_cmd
        .get_envs()
        .map(|(key, _)| key.to_string_lossy().into_owned())
        .collect();
    assert!(!envs.iter().any(|key| key.starts_with("ARGOS_")
        || key.starts_with("FLEETWORKS_")
        || key.contains("TOKEN")
        || key.contains("SECRET")
        || key == "PATH"));
}

#[tokio::test]
async fn local_ipc_handshake_capabilities_missing_optional_nlp_and_classifier() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state_dir = temp.path().canonicalize().expect("canonical tempdir");
    let (client_stream, mut server_stream) = tokio::io::duplex(64 * 1024);

    let caps = local_capabilities(&[]);
    let ready = local_ready();
    let server = tokio::spawn(async move {
        let hello = read_frame(&mut server_stream)
            .await
            .expect("read hello")
            .expect("hello frame");
        assert!(matches!(hello, IpcEnvelope::Hello(_)));
        write_frame(
            &mut server_stream,
            &IpcEnvelope::HelloAck(IpcHelloAck::compatible(Some(caps), Some(ready))),
        )
        .await
        .expect("write hello ack");
        server_stream
    });

    let ipc = Arc::new(
        LocalIpcClient::connect_stream(client_stream, "fleetworks-desktop", "0.2.1")
            .await
            .expect("local ipc handshake"),
    );
    let _server_stream = server.await.expect("server handshake task");
    let process = Arc::new(
        LocalBackendProcess::from_ipc(state_dir, ipc)
            .await
            .expect("attach local backend"),
    );

    assert!(process.client().is_local_ipc());
    let assessment = assess_local_capabilities(process.capabilities());
    assert!(!assessment.nlp_available);
    assert!(!assessment.classifier_available);
    assert!(assessment.warnings.len() >= 2);

    let mut state = State::fixture();
    register_local_state(&mut state, process);
    assert!(state
        .maintenance_disabled_reason(MaintenanceOperation::SeedActors)
        .is_none());
    let train_reason = state
        .maintenance_disabled_reason(MaintenanceOperation::TrainClassifier)
        .expect("missing classifier capability disables training");
    assert!(train_reason.contains("classifier"));
    let activate_reason = state
        .activation_disabled_reason()
        .expect("missing classifier capability disables activation");
    assert!(activate_reason.contains("classifier"));
}

#[tokio::test]
async fn window_close_preserves_local_backend_while_quit_and_stop_trigger_shutdown_ack() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state_dir = temp.path().canonicalize().expect("canonical tempdir");
    let (client_stream, mut server_stream) = tokio::io::duplex(64 * 1024);

    let caps = local_capabilities(&["nlp", "classifier"]);
    let ready = local_ready();
    let server = tokio::spawn(async move {
        let _ = read_frame(&mut server_stream)
            .await
            .expect("read hello")
            .expect("hello frame");
        write_frame(
            &mut server_stream,
            &IpcEnvelope::HelloAck(IpcHelloAck::compatible(Some(caps), Some(ready))),
        )
        .await
        .expect("write hello ack");
        let shutdown = read_frame(&mut server_stream)
            .await
            .expect("read shutdown")
            .expect("shutdown frame");
        let reason = match shutdown {
            IpcEnvelope::Shutdown { reason } => reason,
            other => panic!("unexpected frame: {other:?}"),
        };
        write_frame(
            &mut server_stream,
            &IpcEnvelope::ShutdownAck {
                checkpointed: true,
                relinquished_leases: 2,
            },
        )
        .await
        .expect("write shutdown ack");
        reason
    });

    let ipc = Arc::new(
        LocalIpcClient::connect_stream(client_stream, "fleetworks-desktop", "0.2.1")
            .await
            .expect("local ipc connect"),
    );
    let process = Arc::new(
        LocalBackendProcess::from_ipc(state_dir, ipc)
            .await
            .expect("attach process"),
    );

    let mut state = State::fixture();
    state.services = Some(Arc::new(GatewayEffects::new(
        tokio::runtime::Handle::current(),
    )));
    register_local_state(&mut state, Arc::clone(&process));

    // Closing the desktop window must preserve active local companion supervision.
    state.close_window_cleanup();
    assert!(state.local_backend.session.is_some());
    assert!(!process.is_closed());
    assert!(state.connected());

    // Explicit quit triggers graceful Shutdown -> ShutdownAck.
    state.quit_cleanup();
    assert!(state.local_backend.session.is_none());
    let reason = tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("server shutdown timeout")
        .expect("server join");
    assert_eq!(reason, "application_quit");
}

#[tokio::test]
async fn simulated_ipc_events_and_pipe_close_update_state_without_http_fallback() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state_dir = temp.path().canonicalize().expect("canonical tempdir");
    let (client_stream, mut server_stream) = tokio::io::duplex(64 * 1024);

    let caps = local_capabilities(&["nlp"]);
    let ready = local_ready();
    let server = tokio::spawn(async move {
        let _ = read_frame(&mut server_stream).await.expect("read hello");
        write_frame(
            &mut server_stream,
            &IpcEnvelope::HelloAck(IpcHelloAck::compatible(Some(caps), Some(ready))),
        )
        .await
        .expect("write hello ack");
        server_stream
    });

    let ipc = Arc::new(
        LocalIpcClient::connect_stream(client_stream, "fleetworks-desktop", "0.2.1")
            .await
            .expect("connect"),
    );
    let server_stream = server.await.expect("handshake server");
    let process = Arc::new(
        LocalBackendProcess::from_ipc(state_dir, ipc)
            .await
            .expect("attach"),
    );

    let mut state = State::fixture();
    register_local_state(&mut state, Arc::clone(&process));
    let epoch = state.connection.epoch;

    state.apply_local_backend_event(
        epoch,
        &process,
        IpcEvent::JobProgress {
            job_id: "job-1".into(),
            phase: "indexing".into(),
            summary: "42 actors indexed".into(),
        },
    );
    assert_eq!(
        state
            .local_backend
            .last_job_progress
            .as_ref()
            .expect("progress recorded")
            .summary,
        "42 actors indexed"
    );

    state.apply_local_backend_event(
        epoch,
        &process,
        IpcEvent::PressurePaused {
            reason_code: "thermal_high".into(),
        },
    );
    assert_eq!(
        state.local_backend.pressure_paused.as_deref(),
        Some("thermal_high")
    );

    drop(server_stream);
    state.handle_local_backend_closed(epoch, true, "worker exited cleanly".into());
    assert!(matches!(
        state.connection.status,
        ConnectionStatus::Failed(ref msg) if msg.contains("HTTP fallback disabled")
    ));
    assert!(state.client.is_none());
}

#[test]
fn selecting_local_backend_on_fresh_root_leaves_state_dir_implicit_and_provisions_on_connect() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().canonicalize().expect("canonical tempdir");
    let container =
        fleetworks_workstation::native::NativeContainer::open_settings(&home).expect("container");
    let mut state = State::fixture();
    state.workspaces.container = Some(Arc::new(container));
    assert_eq!(state.backend_kind, BackendKind::Remote);
    assert!(state.local_backend.state_dir.is_none());

    state.set_backend_kind(BackendKind::Local);
    assert_eq!(state.backend_kind, BackendKind::Local);
    assert!(
        state.local_backend.state_dir.is_none(),
        "switching to Local mode must leave implicit state_dir unset before connect"
    );
    let displayed = state.local_state_dir_label();
    assert!(
        displayed.ends_with("local-backend/embedded"),
        "expected implicit fallback display path, got {displayed}"
    );
    let expected_default = PathBuf::from(&displayed);
    assert!(
        !expected_default.exists(),
        "default state dir must not exist yet on fresh workstation"
    );

    // Explicit validation before provisioning fails; implicit provisioning (`None`) creates 0o700 dir.
    let container_ref = state.workspaces.container.as_ref().expect("container");
    assert!(container_ref
        .validate_local_backend_state_dir(&expected_default)
        .is_err());
    let provisioned = match state.local_backend.state_dir.as_deref() {
        Some(dir) => container_ref.validate_local_backend_state_dir(dir),
        None => container_ref.ensure_local_backend_state_dir(&state.local_profile_slug()),
    }
    .expect("fresh workstation provisions implicit local state_dir");
    assert!(provisioned.ends_with("local-backend/embedded"));
    assert!(provisioned.is_dir());
}
