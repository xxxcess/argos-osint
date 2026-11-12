use fleetworks_contracts::capabilities::Capabilities;
use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::context::Permission;
use fleetworks_workspace::native::BackendBinding;
use fleetworks_workspace::native::WorkspaceState;
use rune::Env;
use rune::Runtime;

use crate::app;
use crate::app::Intent;
use crate::app::Load;
use crate::app::Page;
use crate::app::State;
use crate::connection::ConnectionStatus;
use crate::platform::InterfaceSnapshot;
use crate::platform::NativeInterface;
use crate::workspace_lifecycle::RetirementResponse;
use crate::workspaces::WorkspaceRow;
use crate::workspaces::WorkspaceSnapshot;

fn receipt() -> RegistrationReceipt {
    RegistrationReceipt::new(
        "synthetic-deployment".into(),
        "synthetic-node".into(),
        "synthetic-workspace".into(),
        "synthetic-registration".into(),
        "192.0.2.42:research".parse().expect("synthetic Connect ID"),
        1,
    )
}

fn binding() -> BackendBinding {
    serde_json::from_value(serde_json::json!({"deployment_id":"synthetic-deployment", "registration_id":"synthetic-registration",
        "registration_revision":1, "retired":false, "stale":false, "receipt":receipt()})).expect("synthetic binding snapshot")
}

#[test]
fn retirement_pending_or_incomplete_flags_never_create_deletion_receipt() {
    let pending: RetirementResponse = serde_json::from_value(serde_json::json!({"status":"retirement_pending", "deletion_ready":false,
        "registration_id":"synthetic-registration", "workspace_id":"synthetic-workspace", "revision":2,
        "admission_fenced":true, "sessions_revoked":true, "active_work_drained":false, "reason_code":"associated_work_active"})).expect("actual202 shape");
    assert!(pending
        .validate(&binding())
        .expect("typed pending result")
        .is_none());
    let value = serde_json::json!({"status":"retired", "deletion_ready":true, "deployment_id":"synthetic-deployment",
        "registration_id":"synthetic-registration", "workspace_id":"synthetic-workspace", "registration_revision":3, "revision":3,
        "receipt_id":"synthetic-retirement", "admission_fenced":true, "sessions_revoked":true, "active_work_drained":true});
    let complete: RetirementResponse =
        serde_json::from_value(value.clone()).expect("actual200 shape");
    assert!(complete
        .validate(&binding())
        .expect("all real retirement facts")
        .is_some());
    for (field, wrong) in [
        ("active_work_drained", serde_json::json!(false)),
        ("workspace_id", serde_json::json!("another-workspace")),
        ("deployment_id", serde_json::json!("another-deployment")),
    ] {
        let mut invalid = value.clone();
        invalid[field] = wrong;
        let response: RetirementResponse =
            serde_json::from_value(invalid).expect("syntactically valid mismatching response");
        assert!(
            response.validate(&binding()).is_err(),
            "{field} cannot authorize deletion"
        );
    }
}

fn fixture() -> State {
    let mut state = State::fixture();
    state.page = Page::Workstation;
    state.connection.status = ConnectionStatus::Connected;
    let mut capabilities = Capabilities::default();
    capabilities.deployment_id = Some("synthetic-deployment".into());
    capabilities.permissions = vec![Permission::ReadBackend, Permission::RebindWorkstation];
    state.connection.capabilities = Some(capabilities);
    let row = WorkspaceRow {
        name: "research".into(),
        path: "/fixture/home/.fleetworks/research".into(),
        marker: "synthetic-workspace-marker".into(),
        protected_default: false,
        state: WorkspaceState::Registered,
        bindings: vec![binding()],
    };
    state.workspaces.selected = Some(row.clone());
    state.workspaces.snapshot = Load::Ready(WorkspaceSnapshot {
        root: "/fixture/home/.fleetworks".into(),
        installation_marker: "synthetic-installation".into(),
        rows: vec![row],
        operations: Vec::new(),
        registration_attempts: Vec::new(),
        rebind_attempts: Vec::new(),
    });
    let interface = NativeInterface {
        name: "synthetic-vpn".into(),
        ip: "192.0.2.43".parse().expect("synthetic new IP"),
    };
    state.registration.interfaces = Load::Ready(InterfaceSnapshot {
        interfaces: vec![interface.clone()],
        route_suggestion: Some(interface.ip),
    });
    state.registration.selected_interface = Some(interface);
    state
}

#[test]
fn real_runtime_ip_change_requires_confirmation_and_keeps_global_drafts() {
    let mut state = fixture();
    state.brain.query.text = "preserved actor draft".into();
    let mut runtime = Runtime::new(
        state,
        app::view,
        (1280.0, 2400.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    runtime.frame(0.0);
    let (x, y) = runtime
        .find("workspace-rebind-preview")
        .expect("shared production IP-change control");
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(1.0 / 60.0);
    assert!(runtime.state.rebind.confirmation.is_some());
    assert!(!runtime
        .state
        .intents
        .iter()
        .any(|intent| matches!(intent, Intent::RebindWorkspace { .. })));
    let tree = runtime.accessibility_json().to_string();
    assert!(tree.contains("192.0.2.42") && tree.contains("192.0.2.43"));
    let (x, y) = runtime
        .find("workspace-rebind-confirm")
        .expect("shared production IP confirmation");
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(2.0 / 60.0);
    assert!(runtime.state.intents.iter().any(|intent| matches!(intent, Intent::RebindWorkspace { name, ip } if name == "research" && ip == "192.0.2.43")));
    assert!(runtime.state.registration.context.is_none());
    assert_eq!(runtime.state.brain.query.text, "preserved actor draft");
}

#[test]
fn removed_permission_or_changed_interface_fences_ip_confirmation() {
    let mut state = fixture();
    state.preview_rebind();
    state.registration.selected_interface = None;
    state.confirm_rebind();
    assert!(!state
        .intents
        .iter()
        .any(|intent| matches!(intent, Intent::RebindWorkspace { .. })));
    let mut state = fixture();
    state
        .connection
        .capabilities
        .as_mut()
        .expect("fixture metadata")
        .permissions
        .clear();
    assert!(state.rebind_confirmation().is_err());
}

#[test]
fn maintenance_result_shows_safe_candidate_but_never_raw_report_or_shared_details() {
    let mut state = fixture();
    state.page = Page::Jobs;
    let detail = serde_json::json!({"job":{"id":"synthetic-train","app":"workstation","title":"Synthetic training","operation":"classifier_train",
        "state":"completed","phase":"completed","progress_done":100,"progress_total":100,"error_summary":"",
        "private_details_available":true,"activity_actions_available":false,"status_only":false},"stages":[],
        "private_details_available":true,"activity_actions_available":false,
        "maintenance_result":{"operation":"classifier_train","engine":"natural","candidate_model_id":"synthetic-candidate",
            "state":"trained_not_activated","dataset_sha256":"a".repeat(64),"raw_report":{"api_key":"synthetic-private-report"},"server_path":"/synthetic/server-path"}});
    state.backend.detail =
        Load::Ready(serde_json::from_value(detail).expect("safe optional Jobs result"));
    let mut runtime = Runtime::new(
        state,
        app::view,
        (1280.0, 2400.0),
        Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    runtime.frame(0.0);
    let tree = runtime.accessibility_json().to_string();
    assert!(tree.contains("synthetic-candidate") && tree.contains("activation is a separate"));
    assert!(!tree.contains("synthetic-private-report") && !tree.contains("/synthetic/server-path"));
    // Match the production AppHandle update path: a frame alone advances time
    // and does not commit state mutated outside a handler.
    runtime.update(|state, _| {
        if let Load::Ready(detail) = &mut state.backend.detail {
            detail.job.private_details_available = false;
        }
    });
    runtime.frame(1.0 / 60.0);
    assert!(
        !runtime
            .accessibility_json()
            .to_string()
            .contains("synthetic-candidate"),
        "both outer and job private-details flags must authorize the result"
    );
}
