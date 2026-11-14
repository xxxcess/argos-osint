//! Guarded OS Trash with a persisted file-identity reference and no recursion.

use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;

use super::fs;
use super::fs::ObjectIdentity;
use super::state::OperationPhase;
use super::state::RemovalKind;
use super::state::WorkspaceMarker;
use super::state::WorkspaceState;
use super::NativeContainer;
use super::NativeWorkspace;
use super::TrashIntent;

/// Created only by the fixed platform adapter from the verified target. The
/// native application never accepts a user-entered reference or Trash path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct TrashReference {
    pub reference: String,
    pub trash_directory: PathBuf,
}

impl TrashReference {
    #[must_use]
    pub fn new(reference: String, trash_directory: PathBuf) -> Self {
        Self {
            reference,
            trash_directory,
        }
    }
}

/// Protected local journal, written before the irreversible platform effect.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreparedTrash {
    pub reference: TrashReference,
    pub(super) trash_object: ObjectIdentity,
}

/// Exact object capability for a fixed platform adapter, not a generic delete
/// path. Root/default/marker/parent/device/owner checks remain manager-owned.
pub struct TrashTarget<'a> {
    container: &'a NativeContainer,
    workspace: &'a NativeWorkspace,
}

impl TrashTarget<'_> {
    #[must_use]
    pub fn original_path(&self) -> &Path {
        self.workspace.path()
    }

    /// Revalidate the original immediate child and its held container before
    /// creating/resolving the OS identity reference and immediately before move.
    pub fn verify_original(&self) -> Result<()> {
        self.container.revalidate_root()?;
        let child = self.container.open_child(self.workspace.name())?;
        anyhow::ensure!(
            self.workspace.object == Some(fs::directory_identity(&child, true)?),
            "Trash target identity changed"
        );
        self.verify_object_at(self.workspace.path())
    }

    /// Verify an identity reference resolved to this exact marked directory.
    /// The directory may now be in OS Trash; it cannot be a different object.
    pub fn verify_object_at(&self, path: &Path) -> Result<()> {
        let child = fs::open_absolute_directory(path)?;
        let object = fs::directory_identity(&child, true)?;
        anyhow::ensure!(
            self.workspace.object == Some(object),
            "Trash reference resolved to a different object"
        );
        let (marker, _) =
            fs::read_json::<WorkspaceMarker>(&child, super::state::WORKSPACE_MARKER_FILE)?;
        anyhow::ensure!(
            marker.version == super::state::FORMAT_VERSION
                && marker.installation_marker == self.container.installation_marker
                && marker.marker == self.workspace.marker
                && marker.name == self.workspace.name
                && marker.object == object
                && !marker.protected_default,
            "Trash object marker differs from its owned child"
        );
        Ok(())
    }

    /// The reference still names the exact original entry, including its real
    /// descriptor parent. A moved/replaced child is not followed outside it.
    pub fn verify_original_reference(&self, path: &Path) -> Result<()> {
        self.verify_original()?;
        self.verify_object_at(path)?;
        anyhow::ensure!(
            path.file_name() == self.workspace.path().file_name(),
            "Trash reference basename changed"
        );
        let parent =
            fs::open_absolute_directory(path.parent().context("Trash target parent missing")?)?;
        anyhow::ensure!(
            fs::directory_identity(&parent, true)? == self.container.root_object,
            "Trash reference moved outside the protected container"
        );
        Ok(())
    }
}

/// Production uses the OS per-volume user Trash and file-reference URLs. Tests
/// inject isolated owned directories and identity-preserving fake moves only.
pub trait TrashAdapter {
    /// Obtain a local identity reference and the OS-selected user Trash for the
    /// verified source volume. This must not move the source.
    fn prepare(&self, target: &TrashTarget<'_>) -> Result<TrashReference>;
    /// Resolve that exact saved reference, without searching or using a
    /// replacement source path. Missing references leave cleanup pending.
    fn resolve(&self, reference: &TrashReference) -> Result<Option<PathBuf>>;
    /// Move only the saved identity reference after revalidating its original
    /// parent/marker/object. The manager verifies the actual destination.
    fn move_to_trash(&self, target: &TrashTarget<'_>, reference: &TrashReference) -> Result<()>;
}

impl NativeContainer {
    /// Execute/recover a previously confirmed Delete only after every recorded
    /// backend receipt. API success alone never completes local cleanup: the
    /// exact marker/object must be inside OS Trash and the old entry absent.
    pub fn trash_owned_workspace(
        &self,
        intent: &TrashIntent,
        adapter: &impl TrashAdapter,
    ) -> Result<String> {
        let mut guard = self.locked()?;
        let operation_index = guard
            .catalog
            .operations
            .iter()
            .position(|operation| {
                operation.operation_id == intent.operation_id
                    && operation.workspace_marker == intent.workspace.marker
            })
            .context("unknown owned Trash operation")?;
        let operation = guard.catalog.operations[operation_index].clone();
        if operation.phase == OperationPhase::Complete {
            return operation
                .trash_receipt
                .context("completed Trash receipt is missing");
        }
        let index = guard
            .catalog
            .workspaces
            .iter()
            .position(|record| record.marker == operation.workspace_marker)
            .context("Trash folder identity is missing")?;
        self.require_removable(&guard.catalog, index)?;
        anyhow::ensure!(
            operation.kind == RemovalKind::Delete
                && operation.phase == OperationPhase::CleanupPending
                && guard.catalog.workspaces[index].state == WorkspaceState::CleanupPending,
            "Trash is not in a confirmed cleanup phase"
        );
        for binding in &operation.bindings {
            let receipt = operation
                .receipts
                .get(&binding.deployment_id)
                .context("known backend retirement remains pending")?;
            anyhow::ensure!(
                receipt.registration_id == binding.registration_id
                    && receipt.registration_revision >= binding.registration_revision
                    && receipt.admission_fenced
                    && receipt.sessions_revoked
                    && receipt.active_work_drained,
                "backend retirement proof is incomplete"
            );
        }
        let workspace = self.snapshot(&guard.catalog.workspaces[index]);
        let target = TrashTarget {
            container: self,
            workspace: &workspace,
        };
        // Pin an existing original object for the entire platform operation. A
        // recovery after the move instead pins the resolved object below.
        let _original =
            if fs::entry_identity(&self.root_fd, workspace.name().as_str())? == workspace.object {
                Some(self.open_child(workspace.name())?)
            } else {
                None
            };
        let prepared = match &operation.platform_trash {
            Some(prepared) => prepared.clone(),
            None => {
                target.verify_original()?;
                let reference = adapter.prepare(&target)?;
                anyhow::ensure!(
                    !reference.reference.is_empty()
                        && reference.reference.len() <= 8192
                        && !reference.reference.chars().any(char::is_control),
                    "platform identity reference is invalid"
                );
                let trash = fs::open_absolute_directory(&reference.trash_directory)?;
                let trash_object = fs::directory_identity(&trash, true)?;
                anyhow::ensure!(
                    trash_object.device == self.root_object.device
                        && trash_object != self.root_object
                        && Some(trash_object) != workspace.object,
                    "OS Trash is not a distinct owned directory on this volume"
                );
                let prepared = PreparedTrash {
                    reference,
                    trash_object,
                };
                guard.catalog.operations[operation_index].platform_trash = Some(prepared.clone());
                self.persist(&mut guard)?;
                prepared
            }
        };
        let resolved = adapter
            .resolve(&prepared.reference)?
            .context("saved file reference is unavailable; cleanup remains pending")?;
        let _resolved = fs::open_absolute_directory(&resolved)?;
        target.verify_object_at(&resolved)?;
        let trash = fs::open_absolute_directory(&prepared.reference.trash_directory)?;
        anyhow::ensure!(
            fs::directory_identity(&trash, true)? == prepared.trash_object,
            "OS Trash identity changed; cleanup remains pending"
        );
        let in_trash = |path: &Path| -> Result<bool> {
            let parent = fs::open_absolute_directory(
                path.parent().context("resolved Trash parent missing")?,
            )?;
            Ok(fs::directory_identity(&parent, true)? == prepared.trash_object)
        };
        if !in_trash(&resolved)? {
            target.verify_original_reference(&resolved)?;
            // No path-only fallback: the adapter must operate on the persisted
            // identity reference and revalidate this exact source immediately.
            let effect = adapter.move_to_trash(&target, &prepared.reference);
            let location = adapter
                .resolve(&prepared.reference)?
                .context("Trash outcome is uncertain; saved reference retained")?;
            target.verify_object_at(&location)?;
            if !in_trash(&location)? {
                effect?;
                anyhow::bail!(
                    "OS did not place the exact object in Trash; cleanup remains pending"
                );
            }
        }
        let final_location = adapter
            .resolve(&prepared.reference)?
            .context("Trash location cannot be verified")?;
        target.verify_object_at(&final_location)?;
        anyhow::ensure!(
            in_trash(&final_location)?,
            "exact object is outside the verified OS Trash"
        );
        self.revalidate_root()?;
        anyhow::ensure!(
            fs::entry_identity(&self.root_fd, workspace.name().as_str())?.is_none(),
            "the original child entry exists or was replaced; it is never removed by cleanup"
        );
        let receipt = fs::random_marker()?;
        guard.catalog.workspaces[index].state = WorkspaceState::Removed;
        guard.catalog.operations[operation_index].phase = OperationPhase::Complete;
        guard.catalog.operations[operation_index].trash_receipt = Some(receipt.clone());
        self.persist(&mut guard)?;
        Ok(receipt)
    }
}
