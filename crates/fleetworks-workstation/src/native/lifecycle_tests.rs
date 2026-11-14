//! Offline lifecycle tests use isolated owned directories, never the OS Trash.

use std::cell::Cell;
use std::cell::RefCell;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;

use crate::native::legacy_connect::RegistrationEnsureRequest;
use crate::native::legacy_connect::RegistrationReceipt;
use anyhow::Result;

use super::NativeContainer;
use super::OperationPhase;
use super::RemovalKind;
use super::RemovalOutcome;
use super::TrashAdapter;
use super::TrashReference;
use super::TrashTarget;
use super::WorkspaceState;

struct IsolatedTrash {
    root: PathBuf,
    location: RefCell<Option<PathBuf>>,
    fail_after_move: Cell<bool>,
    fail_next_resolve: Cell<bool>,
    replace_source: Cell<bool>,
    moves: Cell<usize>,
}

impl IsolatedTrash {
    fn new(home: &Path) -> Self {
        let root = home.join("synthetic-trash");
        std::fs::create_dir(&root).expect("isolated synthetic Trash");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .expect("private synthetic Trash");
        Self {
            root,
            location: RefCell::new(None),
            fail_after_move: Cell::new(false),
            fail_next_resolve: Cell::new(false),
            replace_source: Cell::new(false),
            moves: Cell::new(0),
        }
    }
}

impl TrashAdapter for IsolatedTrash {
    fn prepare(&self, target: &TrashTarget<'_>) -> Result<TrashReference> {
        target.verify_original()?;
        *self.location.borrow_mut() = Some(target.original_path().to_path_buf());
        Ok(TrashReference::new(
            "synthetic-owned-reference".into(),
            self.root.clone(),
        ))
    }
    fn resolve(&self, _: &TrashReference) -> Result<Option<PathBuf>> {
        anyhow::ensure!(
            !self.fail_next_resolve.replace(false),
            "synthetic uncertain platform response"
        );
        Ok(self.location.borrow().clone())
    }
    fn move_to_trash(&self, target: &TrashTarget<'_>, _: &TrashReference) -> Result<()> {
        target.verify_original_reference(target.original_path())?;
        let destination = self.root.join("owned-folder");
        std::fs::rename(target.original_path(), &destination)?;
        *self.location.borrow_mut() = Some(destination);
        self.moves.set(self.moves.get() + 1);
        if self.replace_source.get() {
            std::fs::create_dir(target.original_path())?;
        }
        if self.fail_after_move.replace(false) {
            self.fail_next_resolve.set(true);
        }
        Ok(())
    }
}

fn receipt(name: &str, workspace_id: &str, registration: &str, ip: &str) -> RegistrationReceipt {
    RegistrationReceipt::new(
        "deployment-a".into(),
        "node-a".into(),
        workspace_id.into(),
        registration.into(),
        format!("{ip}:{name}")
            .parse()
            .expect("synthetic canonical Connect ID"),
        1,
    )
}

#[test]
fn saved_trash_reference_recovers_after_move_without_repeating_effect() {
    let temporary = tempfile::tempdir().expect("isolated fixture home");
    let home = temporary.path().canonicalize().expect("canonical fixture");
    let container = NativeContainer::open(&home).expect("owned container");
    let workspace = container.create("research").expect("owned child");
    std::fs::write(workspace.path().join("preserved.txt"), b"owned bytes")
        .expect("synthetic owned content");
    container
        .begin_removal(&workspace, "delete-research", RemovalKind::Delete)
        .expect("confirmed deletion journal");
    let RemovalOutcome::TrashPending(intent) = container
        .finish_removal("delete-research")
        .expect("no backend bindings")
    else {
        panic!("Delete requires platform cleanup")
    };
    let adapter = IsolatedTrash::new(&home);
    adapter.fail_after_move.set(true);
    assert!(container.trash_owned_workspace(&intent, &adapter).is_err());
    assert_eq!(adapter.moves.get(), 1);
    assert!(adapter.root.join("owned-folder/preserved.txt").exists());
    drop(container);
    let container = NativeContainer::open(&home).expect("reopen does not forget moved object");
    let pending = container.pending_operations().expect("durable recovery");
    assert_eq!(pending[0].phase, OperationPhase::CleanupPending);
    assert!(
        pending[0].platform_trash.is_some(),
        "reference was persisted before the uncertain move"
    );
    assert_eq!(
        container
            .list()
            .expect("pending moved child is visible for recovery")[1]
            .state(),
        WorkspaceState::CleanupPending
    );
    let RemovalOutcome::TrashPending(recovered) = container
        .finish_removal("delete-research")
        .expect("recover intent without source path")
    else {
        panic!("cleanup remains pending")
    };
    assert!(!container
        .trash_owned_workspace(&recovered, &adapter)
        .expect("verify saved object in isolated Trash")
        .is_empty());
    assert_eq!(
        adapter.moves.get(),
        1,
        "restart verifies the saved reference instead of moving again"
    );
    assert!(
        adapter.root.join("owned-folder/preserved.txt").exists(),
        "cleanup never recursively deletes bytes"
    );
}

#[test]
fn replaced_original_entry_remains_untouched_and_cleanup_pending() {
    let temporary = tempfile::tempdir().expect("isolated fixture home");
    let home = temporary.path().canonicalize().expect("canonical fixture");
    let container = NativeContainer::open(&home).expect("owned container");
    let workspace = container.create("research").expect("owned child");
    container
        .begin_removal(&workspace, "delete-research", RemovalKind::Delete)
        .expect("confirmed deletion journal");
    let RemovalOutcome::TrashPending(intent) = container
        .finish_removal("delete-research")
        .expect("known bindings clear")
    else {
        panic!("Delete produces intent")
    };
    let adapter = IsolatedTrash::new(&home);
    adapter.replace_source.set(true);
    assert!(container.trash_owned_workspace(&intent, &adapter).is_err());
    assert!(
        workspace.path().is_dir(),
        "a new entry at the old name is never removed"
    );
    assert!(
        adapter.root.join("owned-folder").is_dir(),
        "the original object is still recoverable"
    );
    assert_eq!(
        container.pending_operations().expect("pending journal")[0].phase,
        OperationPhase::CleanupPending
    );
}

#[test]
fn pending_rebind_survives_restart_and_fences_other_backend_operations() {
    let temporary = tempfile::tempdir().expect("isolated fixture home");
    let home = temporary.path().canonicalize().expect("canonical fixture");
    let container = NativeContainer::open(&home).expect("owned container");
    let workspace = container.create("research").expect("owned child");
    let old = receipt("research", "workspace-a", "registration-a", "192.0.2.42");
    let workspace = container
        .record_registration(&workspace, &old)
        .expect("authenticated original receipt");
    let request = RegistrationEnsureRequest::new(
        "rebind-a".into(),
        "192.0.2.43".parse().expect("synthetic IP"),
        workspace.name().clone(),
        container.installation_marker().to_owned(),
        workspace.marker().to_owned(),
    );
    let attempt = container
        .begin_rebind_attempt(&workspace, &old, request.clone())
        .expect("journal precedes HTTP");
    assert!(
        container.record_registration(&workspace, &old).is_err(),
        "even a valid receipt cannot overwrite an unresolved rebind journal"
    );
    assert!(container
        .begin_removal(&workspace, "remove-a", RemovalKind::Delete)
        .is_err());
    assert!(container
        .begin_registration_attempt(&workspace, "deployment-a", request.clone())
        .is_err());
    drop(container);
    let container = NativeContainer::open(&home).expect("reopen owned container");
    assert_eq!(
        container
            .pending_rebind_attempts()
            .expect("exact retry persisted"),
        vec![attempt]
    );
    let workspace = container
        .select(Some("research".as_ref()))
        .expect("same owned child");
    assert!(
        container
            .begin_rebind_attempt(&workspace, &old, request)
            .is_ok(),
        "exact original nonce/input remains retryable"
    );
    let wrong = receipt(
        "research",
        "different-workspace",
        "registration-b",
        "192.0.2.43",
    );
    assert!(container
        .complete_rebind_attempt("rebind-a", &wrong)
        .is_err());
    assert_eq!(
        container
            .pending_rebind_attempts()
            .expect("conflict never clears intent")
            .len(),
        1
    );
}

#[test]
fn rebind_preserves_receipt_history_and_other_children_need_refresh() {
    let temporary = tempfile::tempdir().expect("isolated fixture home");
    let home = temporary.path().canonicalize().expect("canonical fixture");
    let container = NativeContainer::open(&home).expect("owned container");
    let research = container.create("research").expect("owned research");
    let other = container.create("other").expect("owned sibling");
    let old = receipt("research", "workspace-a", "registration-a", "192.0.2.42");
    let research = container
        .record_registration(&research, &old)
        .expect("known receipt");
    let mut silently_changed = old.clone();
    silently_changed.workstation_ip = "192.0.2.43".parse().expect("different synthetic IP");
    silently_changed.connect_id = "192.0.2.43:research"
        .parse()
        .expect("consistent changed Connect ID");
    silently_changed.revision = 2;
    assert!(
        container
            .record_registration(&research, &silently_changed)
            .is_err(),
        "same registration ID cannot silently change its Connect ID"
    );
    let other = container
        .record_registration(
            &other,
            &receipt("other", "workspace-b", "registration-b", "192.0.2.42"),
        )
        .expect("known sibling receipt");
    let request = RegistrationEnsureRequest::new(
        "rebind-a".into(),
        "192.0.2.43".parse().expect("synthetic IP"),
        research.name().clone(),
        container.installation_marker().to_owned(),
        research.marker().to_owned(),
    );
    container
        .begin_rebind_attempt(&research, &old, request)
        .expect("persisted intent");
    let new = receipt("research", "workspace-a", "registration-new", "192.0.2.43");
    let updated = container
        .complete_rebind_attempt("rebind-a", &new)
        .expect("same stable workspace/workstation identity");
    assert_eq!(updated.binding_history(), &[old]);
    assert_eq!(updated.bindings()[0].registration_id, "registration-new");
    assert!(
        !updated.bindings()[0].retired,
        "a revoked historical binding is not a fabricated drained receipt"
    );
    let sibling = container
        .select(Some(other.name().as_str().as_ref()))
        .expect("same sibling");
    assert!(sibling.bindings()[0].stale);
    assert!(container
        .begin_removal(&sibling, "remove-sibling", RemovalKind::Delete)
        .is_err());
    let refreshed = container
        .record_registration(
            &sibling,
            &receipt(
                "other",
                "workspace-b",
                "registration-other-new",
                "192.0.2.43",
            ),
        )
        .expect("authenticated ensure refreshes same stable IDs");
    assert!(!refreshed.bindings()[0].stale);
    assert_eq!(
        refreshed.binding_history()[0].registration_id,
        "registration-b"
    );
}
