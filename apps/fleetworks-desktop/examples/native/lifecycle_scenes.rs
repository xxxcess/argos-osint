use fleetworks_desktop::app::Intent;
use fleetworks_desktop::app::Load;
use fleetworks_desktop::registration::NativeInterface;
use fleetworks_desktop::workspaces::WorkspaceCommand;
use fleetworks_desktop::Page;
use fleetworks_desktop::State;
use fleetworks_workstation::native::legacy_connect::RegistrationEnsureRequest;
use fleetworks_workstation::native::legacy_connect::RegistrationReceipt;
use fleetworks_workstation::native::BackendBinding;
use fleetworks_workstation::native::RemovalKind;
use fleetworks_workstation::native::WorkspaceOperation;
use fleetworks_workstation::native::WorkspaceState;

fn receipt() -> RegistrationReceipt {
    RegistrationReceipt::new(
        "synthetic-deployment".into(),
        "synthetic-node".into(),
        "synthetic-research".into(),
        "synthetic-registration-research".into(),
        "192.0.2.42:research".parse().expect("synthetic Connect ID"),
        1,
    )
}
fn binding() -> BackendBinding {
    serde_json::from_value(serde_json::json!({"deployment_id":"synthetic-deployment", "registration_id":"synthetic-registration-research",
        "registration_revision":1, "retired":false, "stale":false, "receipt":receipt()})).expect("synthetic authenticated snapshot")
}
fn setup(state: &mut State) {
    super::registered(state);
    let Load::Ready(snapshot) = &mut state.workspaces.snapshot else {
        return;
    };
    let row = snapshot
        .rows
        .iter_mut()
        .find(|row| row.name == "research")
        .expect("synthetic child");
    row.bindings = vec![binding()];
    row.state = WorkspaceState::Registered;
    let row = row.clone();
    state.change_workspace(row);
    let interface = NativeInterface {
        name: "synthetic-vpn-new".into(),
        ip: "192.0.2.43".parse().expect("synthetic new IP"),
    };
    if let Load::Ready(snapshot) = &mut state.registration.interfaces {
        snapshot.interfaces.push(interface.clone());
    }
    state.registration.selected_interface = Some(interface);
}

pub fn rebind_preview(state: &mut State) {
    setup(state);
    state.preview_rebind();
}
pub fn rebind_uncertain(state: &mut State) {
    rebind_preview(state);
    state.confirm_rebind();
    state.page = Page::Workspaces;
}
pub fn retirement_pending(state: &mut State) {
    setup(state);
    state.page = Page::Workspaces;
    state.preview_removal("research", RemovalKind::Delete);
    state.confirm_removal();
}
pub fn trash_recovery(state: &mut State) {
    retirement_pending(state);
    if let Load::Ready(snapshot) = &mut state.workspaces.snapshot {
        for row in &mut snapshot.rows {
            if row.name == "research" {
                row.state = WorkspaceState::CleanupPending;
            }
        }
        let mut operation =
            serde_json::to_value(&snapshot.operations[0]).expect("synthetic journal");
        operation["phase"] = serde_json::json!("cleanup_pending");
        operation["receipts"] = serde_json::json!({"synthetic-deployment":{"deployment_id":"synthetic-deployment", "registration_id":"synthetic-registration-research", "registration_revision":3,
            "receipt_id":"synthetic-retirement", "admission_fenced":true,"sessions_revoked":true,"active_work_drained":true}});
        operation["platform_trash"] = serde_json::json!({"reference":{"reference":"synthetic-file-reference","trash_directory":"/fixture/home/synthetic-trash"},"trash_object":{"device":1,"inode":42,"owner":501}});
        snapshot.operations[0] =
            serde_json::from_value(operation).expect("synthetic uncertain platform journal");
    }
    state.workspaces.message = Some("Synthetic Trash outcome is uncertain. Recheck the saved folder reference; no user files or OS Trash were changed.".into());
}

pub fn training_result(state: &mut State) {
    result(state, true);
}
pub fn seed_result(state: &mut State) {
    result(state, false);
}
fn result(state: &mut State, training: bool) {
    super::jobs_ready(state);
    if let Load::Ready(detail) = &mut state.backend.detail {
        detail.job.operation = if training {
            "classifier_train"
        } else {
            "actor_seed"
        }
        .into();
        detail.job.title = if training {
            "Synthetic classifier training"
        } else {
            "Synthetic actor seed"
        }
        .into();
        detail.job.state = "completed".into();
        detail.job.phase = "completed".into();
        detail.job.progress_done = Some(1000);
        detail.job.progress_total = Some(1000);
        detail.maintenance_result = Some(serde_json::from_value(if training {
            serde_json::json!({"operation":"classifier_train","engine":"natural","candidate_model_id":"synthetic-natural-candidate", "candidate_sha256":"b".repeat(64),"report_sha256":"c".repeat(64),"dataset_sha256":"a".repeat(64),"state":"trained_not_activated","activation_requested":false,"processed":1000,"total":1000})
        } else { serde_json::json!({"operation":"actor_seed","database_seeded":true,"state":"seeded","actor_search_state":"indexing","processed":1000,"total":1000,"aliases":2400,"dataset_sha256":"a".repeat(64)}) }).expect("safe bounded result projection"));
        if let Load::Ready(jobs) = &mut state.backend.jobs {
            jobs.rows = vec![detail.job.clone()];
        }
    }
}

pub fn reply(state: &mut State, intent: &Intent) -> bool {
    match intent {
        Intent::RebindWorkspace { .. } => {
            let Some(row) = &state.workspaces.selected else {
                return true;
            };
            let request = RegistrationEnsureRequest::new(
                "synthetic-rebind-request".into(),
                "192.0.2.43".parse().expect("synthetic new IP"),
                row.name.parse().expect("synthetic child name"),
                "synthetic-installation".into(),
                row.marker.clone(),
            );
            let attempt = serde_json::from_value(
                serde_json::json!({"prior":receipt(),"request":request,"completed":null}),
            )
            .expect("synthetic exact rebind journal");
            if let Load::Ready(snapshot) = &mut state.workspaces.snapshot {
                snapshot.rebind_attempts = vec![attempt];
            }
            state.workspaces.message = Some("Synthetic IP-change response lost; exact request is retained for retry. No gateway was contacted.".into());
        }
        Intent::Workspace(WorkspaceCommand::Remove { name, kind }) => {
            if let Load::Ready(snapshot) = &mut state.workspaces.snapshot {
                let row = snapshot
                    .rows
                    .iter_mut()
                    .find(|row| &row.name == name)
                    .expect("synthetic owned child");
                row.state = WorkspaceState::DeletionPending;
                let operation: WorkspaceOperation = serde_json::from_value(serde_json::json!({"operation_id":"synthetic-delete-request", "workspace_name":name,
                    "workspace_marker":row.marker,"kind":kind,"phase":"retirement_pending", "bindings":row.bindings,"receipts":{},"trash_receipt":null,"platform_trash":null})).expect("synthetic pending journal");
                snapshot.operations = vec![operation];
            }
            state.workspaces.message =
                Some("Waiting for existing backend work to finish. Files are preserved.".into());
        }
        Intent::Workspace(WorkspaceCommand::Resume(_)) | Intent::ReconcileRebind { .. } => {
            state.workspaces.message = Some("Synthetic recovery effect observed. No gateway, filesystem or OS Trash was changed.".into());
        }
        _ => return false,
    }
    true
}
