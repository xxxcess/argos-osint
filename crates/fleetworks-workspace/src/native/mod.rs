//! Protected native-client home container and managed child identities.
//!
//! This API is separate from the operator [`crate::Workspace`]. It never loads
//! old configuration, copies keys/roles, starts services or contacts a backend.
//! Run filesystem work on a blocking worker, never a native view/UI callback.
//!
//! Removal journals and verifies retirement before producing a [`TrashIntent`].
//! The guarded platform path persists an exact object reference before its OS
//! Trash effect, verifies the actual destination, and retains uncertain recovery
//! state. It performs no recursive deletion or unverified path fallback.

mod connections;
mod fs;
mod lifecycle_catalog;
#[cfg(test)]
mod lifecycle_tests;
mod rebind;
mod state;
#[cfg(test)]
mod tests;
mod trash;

pub use connections::ConnectionProfilesSnapshot;
pub use connections::CredentialReference;
pub use connections::GatewayProfile;
pub use connections::GatewayProfileInput;
pub use connections::CONNECTIONS_FILE;
pub use rebind::RebindAttempt;
pub use state::BackendBinding;
pub use state::OperationPhase;
pub use state::RegistrationAttempt;
pub use state::RemovalKind;
pub use state::RetirementReceipt;
pub use state::WorkspaceOperation;
pub use state::WorkspaceState;
pub use trash::PreparedTrash;
pub use trash::TrashAdapter;
pub use trash::TrashReference;
pub use trash::TrashTarget;

use std::collections::BTreeSet;
use std::fs::File;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::connect::RegistrationStatus;
use fleetworks_contracts::connect::WorkspaceName;
use fleetworks_contracts::connect::DEFAULT_WORKSPACE_NAME;
use rustix::fd::OwnedFd;
use rustix::fs::AtFlags;
use rustix::fs::FlockOperation;
use rustix::fs::Mode;
use unicase::UniCase;

use fs::ObjectIdentity;
use state::Catalog;
use state::WorkspaceMarker;
use state::WorkspaceRecord;

/// Protected native root basename, resolved from the selected user home.
pub const NATIVE_CONTAINER_NAME: &str = ".fleetworks";

/// A verified catalog snapshot of one managed immediate child.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeWorkspace {
    name: WorkspaceName,
    path: PathBuf,
    marker: String,
    protected_default: bool,
    state: WorkspaceState,
    bindings: Vec<BackendBinding>,
    binding_history: Vec<RegistrationReceipt>,
    object: Option<ObjectIdentity>,
}

impl NativeWorkspace {
    /// Normalized directory basename, suitable for registration.
    #[must_use]
    pub fn name(&self) -> &WorkspaceName {
        &self.name
    }
    /// Absolute local path, never sent to a remote backend for file access.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Non-secret local folder identity; not a credential.
    #[must_use]
    pub fn marker(&self) -> &str {
        &self.marker
    }
    /// Persisted protection combined with the reserved default identity.
    #[must_use]
    pub fn is_default(&self) -> bool {
        self.protected_default || self.name.is_default()
    }
    /// Local operation state; registration is not inferred from folder existence.
    #[must_use]
    pub fn state(&self) -> WorkspaceState {
        self.state
    }
    /// Known backend bindings, including retired historical bindings.
    #[must_use]
    pub fn bindings(&self) -> &[BackendBinding] {
        &self.bindings
    }
    /// Immutable receipt snapshots replaced by authenticated IP rebinding.
    #[must_use]
    pub fn binding_history(&self) -> &[RegistrationReceipt] {
        &self.binding_history
    }
}

/// A protected root directory capability plus its stable installation identity.
pub struct NativeContainer {
    home: PathBuf,
    root: PathBuf,
    home_fd: OwnedFd,
    root_fd: OwnedFd,
    home_object: ObjectIdentity,
    root_object: ObjectIdentity,
    installation_marker: String,
}

struct LockedCatalog {
    _lock: File,
    catalog: Catalog,
    object: ObjectIdentity,
}

/// Safe local intent produced after all known backend bindings are retired.
///
/// No constructor is public. The original child remains intact until a real
/// platform Trash adapter acts; this object is not a generic deletion capability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrashIntent {
    operation_id: String,
    workspace: NativeWorkspace,
}

impl TrashIntent {
    /// Idempotent operation identity retained in the protected root journal.
    #[must_use]
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }
    /// Exact verified child snapshot for the platform adapter's confirmation.
    #[must_use]
    pub fn workspace(&self) -> &NativeWorkspace {
        &self.workspace
    }
}

/// Completion of local association removal versus pending physical cleanup.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RemovalOutcome {
    RemovedFromApp,
    TrashPending(TrashIntent),
}

impl NativeContainer {
    /// Ensure a private native container/default under an explicit user home.
    ///
    /// The production caller obtains home from the OS, independently of CWD.
    /// Fixtures pass a temporary owned home. Existing legacy roots or unsafe
    /// markers are repair states, never automatically adopted or overwritten.
    pub fn open(home: &Path) -> Result<Self> {
        let home_fd = fs::open_absolute_directory(home)?;
        let home_object = fs::directory_identity(&home_fd, false)?;
        let created =
            match rustix::fs::mkdirat(&home_fd, NATIVE_CONTAINER_NAME, fs::PRIVATE_DIRECTORY) {
                Ok(()) => true,
                Err(rustix::io::Errno::EXIST) => false,
                Err(error) => return Err(error).context("create protected native container"),
            };
        let root_fd = rustix::fs::openat(
            &home_fd,
            NATIVE_CONTAINER_NAME,
            fs::DIRECTORY_FLAGS,
            Mode::empty(),
        )
        .context("native container cannot be a symlink or file")?;
        let root_object = fs::directory_identity(&root_fd, true)?;
        anyhow::ensure!(
            root_object.device == home_object.device,
            "native container mount requires explicit repair"
        );
        let lock = fs::open_private_file(&root_fd, state::LOCK_FILE, created).context(
            "native root is not a current owned container; use the reviewed layout transition",
        )?;
        rustix::fs::flock(&lock, FlockOperation::NonBlockingLockExclusive)
            .context("native workspace catalog is busy")?;
        let catalog = if created {
            let catalog = Catalog {
                version: state::FORMAT_VERSION,
                installation_marker: fs::random_marker()?,
                root_object,
                workspaces: vec![WorkspaceRecord {
                    name: WorkspaceName::new(DEFAULT_WORKSPACE_NAME)?,
                    marker: fs::random_marker()?,
                    object: None,
                    protected_default: true,
                    state: WorkspaceState::CreationPending,
                    bindings: Vec::new(),
                    binding_history: Vec::new(),
                }],
                operations: Vec::new(),
                registration_attempts: Vec::new(),
                rebind_attempts: Vec::new(),
            };
            fs::write_json(&root_fd, state::CATALOG_FILE, &catalog, None)?;
            catalog
        } else {
            fs::read_json::<Catalog>(&root_fd, state::CATALOG_FILE)
                .context("native root needs explicit ownership/layout repair")?
                .0
        };
        anyhow::ensure!(
            catalog.version == state::FORMAT_VERSION && catalog.root_object == root_object,
            "native root identity or format requires repair"
        );
        let container = Self {
            home: home.to_path_buf(),
            root: home.join(NATIVE_CONTAINER_NAME),
            home_fd,
            root_fd,
            home_object,
            root_object,
            installation_marker: catalog.installation_marker,
        };
        drop(lock);
        container.ensure_default()?;
        Ok(container)
    }

    /// Protected container path. It is never a workspace or Connect registration.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Stable non-secret installation marker reused by native registration.
    #[must_use]
    pub fn installation_marker(&self) -> &str {
        &self.installation_marker
    }

    /// Resolve the protected default child without importing operator state.
    pub fn default_workspace(&self) -> Result<NativeWorkspace> {
        self.select(None)
    }

    /// Select an existing owned child by basename or exact immediate-child path.
    ///
    /// The container, external/nested paths and symlinks are rejected. Selecting
    /// never creates a workspace or registration and never reads backend keys.
    pub fn select(&self, selection: Option<&Path>) -> Result<NativeWorkspace> {
        let name = match selection {
            None => WorkspaceName::new(DEFAULT_WORKSPACE_NAME)?,
            Some(path) if path.is_absolute() => {
                anyhow::ensure!(
                    path.parent() == Some(self.root.as_path()),
                    "select an owned immediate child, not the container or an external/nested path"
                );
                WorkspaceName::new(
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .context("workspace basename must be valid UTF-8")?,
                )?
            }
            Some(path) => {
                anyhow::ensure!(
                    path.components().count() == 1,
                    "workspace selection must be one child basename"
                );
                WorkspaceName::new(
                    path.to_str()
                        .context("workspace basename must be valid UTF-8")?,
                )?
            }
        };
        let guard = self.locked()?;
        let requested = fs::entry_identity(&self.root_fd, name.as_str())?
            .context("selected workspace is missing; repair its existing identity")?;
        let record = guard
            .catalog
            .workspaces
            .iter()
            .find(|record| {
                record.object == Some(requested) && record.state != WorkspaceState::Removed
            })
            .context("selected directory is not in the verified native catalog")?;
        anyhow::ensure!(
            matches!(
                record.state,
                WorkspaceState::Local | WorkspaceState::Registered
            ),
            "workspace operation is pending; resolve it before new submissions"
        );
        self.verify_record(record)?;
        Ok(self.snapshot(record))
    }

    /// List only verified catalog entries; unknown subdirectories are not adopted.
    pub fn list(&self) -> Result<Vec<NativeWorkspace>> {
        let guard = self.locked()?;
        let mut workspaces = Vec::new();
        for record in guard
            .catalog
            .workspaces
            .iter()
            .filter(|record| record.state != WorkspaceState::Removed)
        {
            if record.state != WorkspaceState::CreationPending
                && !(record.state == WorkspaceState::CleanupPending
                    && guard.catalog.operations.iter().any(|operation| {
                        operation.workspace_marker == record.marker
                            && operation.phase == OperationPhase::CleanupPending
                            && operation.platform_trash.is_some()
                    }))
            {
                self.verify_record(record)?;
            }
            workspaces.push(self.snapshot(record));
        }
        workspaces.sort_by(|first, second| first.name.cmp(&second.name));
        Ok(workspaces)
    }

    /// Create a privately owned named child with no copied configuration or data.
    ///
    /// Local creation is durable before any separately performed remote ensure.
    /// Repeating a pending creation resumes its recorded identity where safe.
    pub fn create(&self, name: &str) -> Result<NativeWorkspace> {
        let name = WorkspaceName::new_child(name)?;
        let mut guard = self.locked()?;
        if let Some(index) = guard.catalog.workspaces.iter().position(|record| {
            record.name == name && record.state == WorkspaceState::CreationPending
        }) {
            return self.finish_creation(&mut guard, index);
        }
        self.require_new_name(&guard.catalog, &name)?;
        guard.catalog.workspaces.push(WorkspaceRecord {
            name,
            marker: fs::random_marker()?,
            object: None,
            protected_default: false,
            state: WorkspaceState::CreationPending,
            bindings: Vec::new(),
            binding_history: Vec::new(),
        });
        let index = guard.catalog.workspaces.len() - 1;
        self.persist(&mut guard)?;
        self.finish_creation(&mut guard, index)
    }

    /// Attach one already marked child after explicit marker confirmation.
    ///
    /// This cannot adopt an arbitrary folder, legacy configuration/deployment
    /// directory, external path, other installation or conflicting marker.
    pub fn attach_owned_child(
        &self,
        name: &str,
        confirmed_marker: &str,
    ) -> Result<NativeWorkspace> {
        let name = WorkspaceName::new_child(name)?;
        let mut guard = self.locked()?;
        let child = self.open_child(&name)?;
        let object = fs::directory_identity(&child, true)?;
        let (marker, _) = fs::read_json::<WorkspaceMarker>(&child, state::WORKSPACE_MARKER_FILE)
            .context(
            "unknown folders need a separately reviewed data-only import, not automatic adoption",
        )?;
        anyhow::ensure!(
            marker.version == state::FORMAT_VERSION
                && marker.installation_marker == self.installation_marker
                && marker.name == name
                && marker.marker == confirmed_marker
                && marker.object == object
                && !marker.protected_default,
            "workspace ownership or confirmed marker does not match"
        );
        if let Some(index) = guard
            .catalog
            .workspaces
            .iter()
            .position(|record| record.marker == marker.marker)
        {
            let record = &mut guard.catalog.workspaces[index];
            anyhow::ensure!(
                record.name == name && record.object == Some(object),
                "catalog attachment conflict needs repair"
            );
            if record.state == WorkspaceState::Removed {
                anyhow::ensure!(
                    record.bindings.iter().all(|binding| binding.retired),
                    "existing backend bindings have not retired"
                );
                record.state = WorkspaceState::Local;
                self.persist(&mut guard)?;
            }
            let record = &guard.catalog.workspaces[index];
            anyhow::ensure!(
                matches!(
                    record.state,
                    WorkspaceState::Local | WorkspaceState::Registered
                ),
                "workspace operation remains pending"
            );
            self.verify_record(record)?;
            return Ok(self.snapshot(record));
        }
        self.require_catalog_name_available(&guard.catalog, &name)?;
        let record = WorkspaceRecord {
            name,
            marker: marker.marker,
            object: Some(object),
            protected_default: false,
            state: WorkspaceState::Local,
            bindings: Vec::new(),
            binding_history: Vec::new(),
        };
        self.verify_record(&record)?;
        let snapshot = self.snapshot(&record);
        guard.catalog.workspaces.push(record);
        self.persist(&mut guard)?;
        Ok(snapshot)
    }

    /// Persist an exact registration intent before sending it to the gateway.
    ///
    /// Unknown HTTP outcomes retain this journal across restarts and prevent
    /// removal. A different IP/request cannot overwrite an unresolved attempt.
    pub fn begin_registration_attempt(
        &self,
        workspace: &NativeWorkspace,
        deployment_id: &str,
        request: RegistrationEnsureRequest,
    ) -> Result<RegistrationAttempt> {
        require_operation_id(deployment_id)?;
        require_operation_id(&request.request_id)?;
        fleetworks_contracts::connect::ConnectId::new(
            request.workstation_ip,
            request.workspace_name.clone(),
        )?;
        let mut guard = self.locked()?;
        let index = self.record_index(&guard.catalog, workspace)?;
        let record = &guard.catalog.workspaces[index];
        self.verify_record(record)?;
        anyhow::ensure!(
            !guard
                .catalog
                .rebind_attempts
                .iter()
                .any(|attempt| attempt.completed.is_none()
                    && attempt.prior.deployment_id == deployment_id),
            "reconcile the pending workstation IP change before registration"
        );
        anyhow::ensure!(
            matches!(
                record.state,
                WorkspaceState::Local | WorkspaceState::Registered
            ) && request.installation_marker == self.installation_marker
                && request.workspace_marker == record.marker
                && request.workspace_name == record.name,
            "registration intent does not match an active owned child"
        );
        if let Some(existing) = guard.catalog.registration_attempts.iter().find(|attempt| {
            attempt.deployment_id == deployment_id
                && attempt.request.workspace_marker == record.marker
        }) {
            anyhow::ensure!(
                existing.request == request,
                "reconcile the existing registration attempt before changing its input"
            );
            return Ok(existing.clone());
        }
        anyhow::ensure!(
            guard.catalog.registration_attempts.len() < 1024,
            "registration attempt journal capacity requires reconciliation"
        );
        let attempt = RegistrationAttempt {
            deployment_id: deployment_id.to_owned(),
            request,
        };
        guard.catalog.registration_attempts.push(attempt.clone());
        self.persist(&mut guard)?;
        Ok(attempt)
    }

    /// Read unresolved non-secret intents for exact retry; this never contacts
    /// the backend or discards an outcome because a local request was cancelled.
    pub fn pending_registration_attempts(&self) -> Result<Vec<RegistrationAttempt>> {
        Ok(self.locked()?.catalog.registration_attempts)
    }

    /// Remember a confirmed active registration without copying backend settings.
    pub fn record_registration(
        &self,
        workspace: &NativeWorkspace,
        receipt: &RegistrationReceipt,
    ) -> Result<NativeWorkspace> {
        let mut guard = self.locked()?;
        let index = self.record_index(&guard.catalog, workspace)?;
        self.verify_record(&guard.catalog.workspaces[index])?;
        anyhow::ensure!(
            !guard
                .catalog
                .rebind_attempts
                .iter()
                .any(|attempt| attempt.completed.is_none()
                    && attempt.prior.deployment_id == receipt.deployment_id),
            "reconcile the pending workstation IP change before recording a registration"
        );
        anyhow::ensure!(
            receipt.is_consistent()
                && receipt.status == RegistrationStatus::Active
                && receipt.workspace_name == workspace.name,
            "registration receipt does not match this child"
        );
        if let Some(attempt) = guard.catalog.registration_attempts.iter().find(|attempt| {
            attempt.deployment_id == receipt.deployment_id
                && attempt.request.workspace_marker == workspace.marker
        }) {
            anyhow::ensure!(
                attempt.request.workstation_ip == receipt.workstation_ip,
                "registration receipt IP differs from its durable intent"
            );
        }
        let record = &mut guard.catalog.workspaces[index];
        anyhow::ensure!(
            matches!(
                record.state,
                WorkspaceState::Local | WorkspaceState::Registered
            ),
            "workspace operation is fenced pending retirement or repair"
        );
        if let Some(binding_index) = record
            .bindings
            .iter()
            .position(|binding| binding.deployment_id == receipt.deployment_id)
        {
            let binding = &record.bindings[binding_index];
            anyhow::ensure!(
                !binding.retired,
                "registration rebind or revoked binding needs explicit recovery"
            );
            if let Some(known) = &binding.receipt {
                anyhow::ensure!(
                    known.workstation_id == receipt.workstation_id
                        && known.workspace_id == receipt.workspace_id,
                    "registration refresh changed stable workstation/workspace identity"
                );
                anyhow::ensure!(
                    binding.stale
                        || binding.registration_id != receipt.registration_id
                        || known.connect_id == receipt.connect_id,
                    "registration Connect ID changed without an authenticated rebind"
                );
            }
            if binding.registration_id != receipt.registration_id {
                anyhow::ensure!(
                    binding.stale,
                    "registration changed without an authenticated local rebind"
                );
                let previous = binding.receipt.clone().context(
                    "refresh complete historical identity before replacing this binding",
                )?;
                anyhow::ensure!(
                    record.binding_history.len() < 512,
                    "local binding history needs reviewed archival"
                );
                record.binding_history.push(previous);
            } else {
                anyhow::ensure!(
                    receipt.revision >= binding.registration_revision,
                    "registration revision moved backwards"
                );
            }
            record.bindings[binding_index] = BackendBinding {
                deployment_id: receipt.deployment_id.clone(),
                registration_id: receipt.registration_id.clone(),
                registration_revision: receipt.revision,
                retired: false,
                stale: false,
                receipt: Some(receipt.clone()),
            };
        } else {
            record.bindings.push(BackendBinding {
                deployment_id: receipt.deployment_id.clone(),
                registration_id: receipt.registration_id.clone(),
                registration_revision: receipt.revision,
                retired: false,
                stale: false,
                receipt: Some(receipt.clone()),
            });
        }
        record.state = WorkspaceState::Registered;
        let snapshot = self.snapshot(record);
        guard.catalog.registration_attempts.retain(|attempt| {
            !(attempt.deployment_id == receipt.deployment_id
                && attempt.request.workspace_marker == workspace.marker)
        });
        self.persist(&mut guard)?;
        Ok(snapshot)
    }

    /// Begin an explicitly confirmed non-default removal, preserving all bytes.
    ///
    /// The UI confirms exact name/path, contents impact, known bindings and work
    /// before calling. Backend retirement/drain remains a separate API action.
    pub fn begin_removal(
        &self,
        workspace: &NativeWorkspace,
        operation_id: &str,
        kind: RemovalKind,
    ) -> Result<WorkspaceOperation> {
        require_operation_id(operation_id)?;
        let mut guard = self.locked()?;
        let index = self.record_index(&guard.catalog, workspace)?;
        self.require_removable(&guard.catalog, index)?;
        self.verify_record(&guard.catalog.workspaces[index])?;
        if let Some(operation) = guard
            .catalog
            .operations
            .iter()
            .find(|operation| operation.operation_id == operation_id)
        {
            anyhow::ensure!(
                operation.workspace_marker == workspace.marker && operation.kind == kind,
                "operation ID already belongs to different work"
            );
            return Ok(operation.clone());
        }
        anyhow::ensure!(
            matches!(
                guard.catalog.workspaces[index].state,
                WorkspaceState::Local | WorkspaceState::Registered
            ),
            "workspace already has a pending operation"
        );
        let record = &mut guard.catalog.workspaces[index];
        let operation = WorkspaceOperation {
            operation_id: operation_id.to_owned(),
            workspace_name: record.name.clone(),
            workspace_marker: record.marker.clone(),
            kind,
            phase: OperationPhase::RetirementPending,
            bindings: record
                .bindings
                .iter()
                .filter(|binding| !binding.retired)
                .cloned()
                .collect(),
            receipts: Default::default(),
            trash_receipt: None,
            platform_trash: None,
        };
        record.state = WorkspaceState::DeletionPending;
        guard.catalog.operations.push(operation.clone());
        self.persist(&mut guard)?;
        Ok(operation)
    }

    /// Journal verified retirement from one known backend; a failure keeps files.
    pub fn record_retirement(
        &self,
        operation_id: &str,
        receipt: RetirementReceipt,
    ) -> Result<WorkspaceOperation> {
        let mut guard = self.locked()?;
        let operation = guard
            .catalog
            .operations
            .iter_mut()
            .find(|operation| operation.operation_id == operation_id)
            .context("unknown removal operation")?;
        if operation.phase != OperationPhase::RetirementPending {
            anyhow::ensure!(
                operation.receipts.get(&receipt.deployment_id) == Some(&receipt),
                "retirement phase has closed and receipt does not match its journal"
            );
            return Ok(operation.clone());
        }
        let binding = operation
            .bindings
            .iter()
            .find(|binding| {
                binding.deployment_id == receipt.deployment_id
                    && binding.registration_id == receipt.registration_id
            })
            .context("receipt is not for a recorded backend binding")?;
        require_operation_id(&receipt.receipt_id)?;
        anyhow::ensure!(
            receipt.registration_revision >= binding.registration_revision
                && receipt.admission_fenced
                && receipt.sessions_revoked
                && receipt.active_work_drained,
            "backend retirement is incomplete; preserve files and retry"
        );
        if let Some(existing) = operation.receipts.get(&receipt.deployment_id) {
            anyhow::ensure!(
                existing == &receipt,
                "conflicting retirement receipt needs reconciliation"
            );
        } else {
            operation
                .receipts
                .insert(receipt.deployment_id.clone(), receipt);
        }
        let snapshot = operation.clone();
        self.persist(&mut guard)?;
        Ok(snapshot)
    }

    /// Finish association removal or prepare a real OS Trash action after retirement.
    pub fn finish_removal(&self, operation_id: &str) -> Result<RemovalOutcome> {
        let mut guard = self.locked()?;
        let operation_index = guard
            .catalog
            .operations
            .iter()
            .position(|operation| operation.operation_id == operation_id)
            .context("unknown removal operation")?;
        let operation = guard.catalog.operations[operation_index].clone();
        let index = guard
            .catalog
            .workspaces
            .iter()
            .position(|record| record.marker == operation.workspace_marker)
            .context("removal identity is missing from the protected catalog")?;
        self.require_removable(&guard.catalog, index)?;
        if operation.phase == OperationPhase::Complete
            && operation.kind == RemovalKind::RemoveFromApp
        {
            return Ok(RemovalOutcome::RemovedFromApp);
        }
        if !(operation.phase == OperationPhase::CleanupPending
            && operation.platform_trash.is_some())
        {
            self.verify_record(&guard.catalog.workspaces[index])?;
        }
        for binding in &operation.bindings {
            let receipt = operation
                .receipts
                .get(&binding.deployment_id)
                .context("retirement remains pending for a known backend; files preserved")?;
            anyhow::ensure!(
                receipt.registration_id == binding.registration_id
                    && receipt.admission_fenced
                    && receipt.registration_revision >= binding.registration_revision
                    && receipt.sessions_revoked
                    && receipt.active_work_drained,
                "backend retirement proof is incomplete"
            );
        }
        for binding in &mut guard.catalog.workspaces[index].bindings {
            binding.retired = true;
        }
        let outcome = match operation.kind {
            RemovalKind::RemoveFromApp => {
                guard.catalog.workspaces[index].state = WorkspaceState::Removed;
                guard.catalog.operations[operation_index].phase = OperationPhase::Complete;
                RemovalOutcome::RemovedFromApp
            }
            RemovalKind::Delete => {
                anyhow::ensure!(
                    operation.phase != OperationPhase::Complete,
                    "Trash completion is already recorded"
                );
                guard.catalog.workspaces[index].state = WorkspaceState::CleanupPending;
                guard.catalog.operations[operation_index].phase = OperationPhase::CleanupPending;
                RemovalOutcome::TrashPending(TrashIntent {
                    operation_id: operation_id.to_owned(),
                    workspace: self.snapshot(&guard.catalog.workspaces[index]),
                })
            }
        };
        self.persist(&mut guard)?;
        Ok(outcome)
    }

    /// Revalidate the exact directory immediately before a platform Trash action.
    pub fn verify_trash_intent(&self, intent: &TrashIntent) -> Result<()> {
        let guard = self.locked()?;
        let index = self.record_index(&guard.catalog, &intent.workspace)?;
        self.require_removable(&guard.catalog, index)?;
        self.verify_record(&guard.catalog.workspaces[index])?;
        let operation = guard
            .catalog
            .operations
            .iter()
            .find(|operation| {
                operation.operation_id == intent.operation_id
                    && operation.workspace_marker == intent.workspace.marker
            })
            .context("Trash intent no longer matches its journal")?;
        anyhow::ensure!(
            operation.kind == RemovalKind::Delete
                && operation.phase == OperationPhase::CleanupPending
                && guard.catalog.workspaces[index].state == WorkspaceState::CleanupPending,
            "Trash is not authorized by the current local lifecycle phase"
        );
        Ok(())
    }

    /// Record a real platform receipt only after the original child is absent.
    ///
    /// This does not perform Trash or secure erasure. A replacement object at the
    /// old name blocks completion; it is never deleted or silently adopted.
    pub fn record_trash_completed(&self, operation_id: &str, receipt_id: &str) -> Result<()> {
        require_operation_id(receipt_id)?;
        let mut guard = self.locked()?;
        let operation_index = guard
            .catalog
            .operations
            .iter()
            .position(|operation| operation.operation_id == operation_id)
            .context("unknown Trash operation")?;
        let operation = &guard.catalog.operations[operation_index];
        if operation.kind == RemovalKind::Delete && operation.phase == OperationPhase::Complete {
            anyhow::ensure!(
                operation.trash_receipt.as_deref() == Some(receipt_id),
                "completed Trash operation has a different receipt"
            );
            return Ok(());
        }
        anyhow::ensure!(
            operation.kind == RemovalKind::Delete
                && operation.phase == OperationPhase::CleanupPending,
            "no pending platform Trash operation"
        );
        let index = guard
            .catalog
            .workspaces
            .iter()
            .position(|record| record.marker == operation.workspace_marker)
            .context("Trash identity is missing")?;
        self.require_removable(&guard.catalog, index)?;
        anyhow::ensure!(
            fs::entry_identity(&self.root_fd, guard.catalog.workspaces[index].name.as_str())?
                .is_none(),
            "original child still exists or was replaced; Trash remains pending"
        );
        guard.catalog.workspaces[index].state = WorkspaceState::Removed;
        guard.catalog.operations[operation_index].phase = OperationPhase::Complete;
        guard.catalog.operations[operation_index].trash_receipt = Some(receipt_id.to_owned());
        self.persist(&mut guard)
    }

    /// Retain incomplete operations for retry/repair without resurrecting sessions.
    pub fn pending_operations(&self) -> Result<Vec<WorkspaceOperation>> {
        let guard = self.locked()?;
        Ok(guard
            .catalog
            .operations
            .into_iter()
            .filter(|operation| operation.phase != OperationPhase::Complete)
            .collect())
    }

    fn ensure_default(&self) -> Result<()> {
        let mut guard = self.locked()?;
        let index = guard
            .catalog
            .workspaces
            .iter()
            .position(|record| record.name.is_default())
            .context("protected default identity is missing; repair instead of recreating it")?;
        if guard.catalog.workspaces[index].state == WorkspaceState::CreationPending {
            self.finish_creation(&mut guard, index)?;
        } else {
            self.verify_record(&guard.catalog.workspaces[index])?;
        }
        Ok(())
    }

    fn finish_creation(&self, guard: &mut LockedCatalog, index: usize) -> Result<NativeWorkspace> {
        let name = guard.catalog.workspaces[index].name.clone();
        if guard.catalog.workspaces[index].object.is_none() {
            anyhow::ensure!(
                fs::entry_identity(&self.root_fd, name.as_str())?.is_none(),
                "unreconciled folder at a pending creation path needs explicit repair"
            );
            self.revalidate_root()?;
            rustix::fs::mkdirat(&self.root_fd, name.as_str(), fs::PRIVATE_DIRECTORY)
                .context("create new private child without overwriting")?;
            let child = self.open_child(&name)?;
            guard.catalog.workspaces[index].object = Some(fs::directory_identity(&child, true)?);
            self.persist(guard)?;
        }
        let child = self.open_child(&name)?;
        let object = fs::directory_identity(&child, true)?;
        anyhow::ensure!(
            guard.catalog.workspaces[index].object == Some(object),
            "pending child identity changed; preserve data and repair"
        );
        let record = &guard.catalog.workspaces[index];
        let expected = WorkspaceMarker {
            version: state::FORMAT_VERSION,
            installation_marker: self.installation_marker.clone(),
            name: name.clone(),
            marker: record.marker.clone(),
            object,
            protected_default: record.protected_default,
        };
        match rustix::fs::statat(
            &child,
            state::WORKSPACE_MARKER_FILE,
            AtFlags::SYMLINK_NOFOLLOW,
        ) {
            Err(rustix::io::Errno::NOENT) => {
                fs::write_json(&child, state::WORKSPACE_MARKER_FILE, &expected, None)?;
            }
            Ok(_) => {
                let (marker, _) =
                    fs::read_json::<WorkspaceMarker>(&child, state::WORKSPACE_MARKER_FILE)?;
                anyhow::ensure!(
                    marker == expected,
                    "pending child marker conflicts; preserve data and repair"
                );
            }
            Err(error) => return Err(error).context("inspect pending child marker"),
        }
        guard.catalog.workspaces[index].state = WorkspaceState::Local;
        self.verify_record(&guard.catalog.workspaces[index])?;
        let workspace = self.snapshot(&guard.catalog.workspaces[index]);
        self.persist(guard)?;
        Ok(workspace)
    }

    fn locked(&self) -> Result<LockedCatalog> {
        self.revalidate_root()?;
        let lock = fs::open_private_file(&self.root_fd, state::LOCK_FILE, false)?;
        rustix::fs::flock(&lock, FlockOperation::NonBlockingLockExclusive)
            .context("native workspace catalog is busy")?;
        let (catalog, object) = fs::read_json::<Catalog>(&self.root_fd, state::CATALOG_FILE)?;
        self.validate_catalog(&catalog)?;
        if let Some(default) = catalog
            .workspaces
            .iter()
            .find(|record| record.name.is_default())
        {
            if default.state != WorkspaceState::CreationPending {
                self.verify_record(default)?;
            }
        }
        Ok(LockedCatalog {
            _lock: lock,
            catalog,
            object,
        })
    }

    fn persist(&self, guard: &mut LockedCatalog) -> Result<()> {
        self.revalidate_root()?;
        self.validate_catalog(&guard.catalog)?;
        guard.object = fs::write_json(
            &self.root_fd,
            state::CATALOG_FILE,
            &guard.catalog,
            Some(guard.object),
        )?;
        self.revalidate_root()
    }

    fn revalidate_root(&self) -> Result<()> {
        let home = fs::open_absolute_directory(&self.home)?;
        anyhow::ensure!(
            fs::directory_identity(&home, false)? == self.home_object
                && fs::directory_identity(&self.home_fd, false)? == self.home_object,
            "home identity changed; workspace actions are blocked"
        );
        anyhow::ensure!(
            fs::entry_identity(&self.home_fd, NATIVE_CONTAINER_NAME)? == Some(self.root_object)
                && fs::directory_identity(&self.root_fd, true)? == self.root_object,
            "protected container was moved, substituted or changed"
        );
        Ok(())
    }

    fn validate_catalog(&self, catalog: &Catalog) -> Result<()> {
        anyhow::ensure!(
            catalog.version == state::FORMAT_VERSION
                && catalog.installation_marker == self.installation_marker
                && catalog.root_object == self.root_object,
            "native catalog ownership/version conflicts"
        );
        anyhow::ensure!(
            valid_marker(&catalog.installation_marker)
                && catalog.workspaces.len() <= 1024
                && catalog.operations.len() <= 4096,
            "native catalog is invalid or exceeds bounded capacity"
        );
        let defaults: Vec<_> = catalog
            .workspaces
            .iter()
            .filter(|record| record.name.is_default())
            .collect();
        anyhow::ensure!(
            defaults.len() == 1
                && defaults[0].protected_default
                && !matches!(
                    defaults[0].state,
                    WorkspaceState::DeletionPending
                        | WorkspaceState::CleanupPending
                        | WorkspaceState::Removed
                ),
            "protected default identity/state is invalid"
        );
        let mut markers = BTreeSet::new();
        let mut active_names = BTreeSet::new();
        for record in &catalog.workspaces {
            anyhow::ensure!(
                valid_marker(&record.marker) && markers.insert(&record.marker),
                "duplicate or invalid workspace marker"
            );
            if record.state != WorkspaceState::Removed {
                anyhow::ensure!(
                    active_names.insert(UniCase::new(record.name.as_str()).to_folded_case()),
                    "case/Unicode-equivalent child collision needs repair"
                );
            }
            if let Some(object) = record.object {
                anyhow::ensure!(
                    object != self.root_object
                        && object.device == self.root_object.device
                        && object.owner == self.root_object.owner,
                    "workspace object overlaps protected root or another mount/owner"
                );
            } else {
                anyhow::ensure!(
                    record.state == WorkspaceState::CreationPending,
                    "workspace object identity is missing"
                );
            }
        }
        anyhow::ensure!(
            catalog.registration_attempts.len() <= 1024,
            "registration journal exceeds bounded capacity"
        );
        let mut attempt_keys = BTreeSet::new();
        let mut request_ids = BTreeSet::new();
        for attempt in &catalog.registration_attempts {
            require_operation_id(&attempt.deployment_id)?;
            require_operation_id(&attempt.request.request_id)?;
            anyhow::ensure!(
                attempt_keys.insert((&attempt.deployment_id, &attempt.request.workspace_marker))
                    && request_ids.insert(&attempt.request.request_id)
                    && attempt.request.installation_marker == catalog.installation_marker
                    && catalog.workspaces.iter().any(|record| record.marker
                        == attempt.request.workspace_marker
                        && record.name == attempt.request.workspace_name
                        && matches!(
                            record.state,
                            WorkspaceState::Local | WorkspaceState::Registered
                        )),
                "registration journal identity conflicts with its protected catalog"
            );
        }
        self.validate_lifecycle_catalog(catalog)
    }

    fn open_child(&self, name: &WorkspaceName) -> Result<OwnedFd> {
        self.revalidate_root()?;
        let child = rustix::fs::openat(
            &self.root_fd,
            name.as_str(),
            fs::DIRECTORY_FLAGS,
            Mode::empty(),
        )
        .context("managed child is missing, moved, a symlink or not a directory")?;
        let object = fs::directory_identity(&child, true)?;
        anyhow::ensure!(
            object.device == self.root_object.device && object != self.root_object,
            "child cannot be a mount or protected container alias"
        );
        let parent = rustix::fs::openat(&child, "..", fs::DIRECTORY_FLAGS, Mode::empty())?;
        anyhow::ensure!(
            fs::directory_identity(&parent, true)? == self.root_object,
            "child parent was substituted"
        );
        Ok(child)
    }

    fn verify_record(&self, record: &WorkspaceRecord) -> Result<()> {
        let child = self.open_child(&record.name)?;
        let object = fs::directory_identity(&child, true)?;
        anyhow::ensure!(
            record.object == Some(object),
            "managed child identity changed; repair without adoption"
        );
        let (marker, _) = fs::read_json::<WorkspaceMarker>(&child, state::WORKSPACE_MARKER_FILE)?;
        anyhow::ensure!(
            marker.version == state::FORMAT_VERSION
                && marker.installation_marker == self.installation_marker
                && marker.name == record.name
                && marker.marker == record.marker
                && marker.object == object
                && marker.protected_default == record.protected_default,
            "managed child marker conflicts with protected catalog"
        );
        Ok(())
    }

    fn require_catalog_name_available(
        &self,
        catalog: &Catalog,
        name: &WorkspaceName,
    ) -> Result<()> {
        let folded = UniCase::new(name.as_str());
        anyhow::ensure!(
            !catalog
                .workspaces
                .iter()
                .any(|record| record.state != WorkspaceState::Removed
                    && UniCase::new(record.name.as_str()) == folded),
            "case/Unicode-equivalent workspace already exists"
        );
        Ok(())
    }

    fn require_new_name(&self, catalog: &Catalog, name: &WorkspaceName) -> Result<()> {
        self.require_catalog_name_available(catalog, name)?;
        for entry in rustix::fs::Dir::read_from(&self.root_fd)? {
            let entry = entry?;
            if let Ok(existing) = entry.file_name().to_str() {
                anyhow::ensure!(
                    UniCase::new(existing) != UniCase::new(name.as_str()),
                    "existing folder/file or case alias requires explicit repair or attachment"
                );
            }
        }
        Ok(())
    }

    fn record_index(&self, catalog: &Catalog, workspace: &NativeWorkspace) -> Result<usize> {
        anyhow::ensure!(
            workspace.path == self.root.join(workspace.name.as_str()),
            "workspace snapshot is not a child of this container"
        );
        catalog
            .workspaces
            .iter()
            .position(|record| {
                record.marker == workspace.marker
                    && record.name == workspace.name
                    && record.object == workspace.object
            })
            .context("workspace snapshot is stale or belongs to a different container")
    }

    fn require_removable(&self, catalog: &Catalog, index: usize) -> Result<()> {
        let record = &catalog.workspaces[index];
        let default = catalog
            .workspaces
            .iter()
            .find(|record| record.name.is_default())
            .context("protected default identity missing")?;
        anyhow::ensure!(
            !record.name.is_default()
                && !record.protected_default
                && record.object != default.object
                && record.object != Some(self.root_object),
            "container, default and protected object aliases cannot be removed or deleted"
        );
        anyhow::ensure!(
            !catalog
                .registration_attempts
                .iter()
                .any(|attempt| attempt.request.workspace_marker == record.marker),
            "registration outcome remains unknown; reconcile its durable request before removal"
        );
        anyhow::ensure!(
            record
                .bindings
                .iter()
                .filter(|binding| !binding.retired)
                .all(|binding| !binding.stale && binding.receipt.is_some()),
            "refresh complete current backend registrations before removal"
        );
        anyhow::ensure!(
            !catalog
                .rebind_attempts
                .iter()
                .any(|attempt| attempt.completed.is_none()
                    && record
                        .bindings
                        .iter()
                        .any(|binding| binding.deployment_id == attempt.prior.deployment_id)),
            "reconcile the pending workstation IP change before removal"
        );
        self.verify_record(default)
    }

    fn snapshot(&self, record: &WorkspaceRecord) -> NativeWorkspace {
        NativeWorkspace {
            name: record.name.clone(),
            path: self.root.join(record.name.as_str()),
            marker: record.marker.clone(),
            protected_default: record.protected_default,
            state: record.state,
            bindings: record.bindings.clone(),
            binding_history: record.binding_history.clone(),
            object: record.object,
        }
    }
}

fn require_operation_id(value: &str) -> Result<()> {
    anyhow::ensure!(
        (1..=128).contains(&value.len())
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "operation/receipt ID must be a bounded identifier"
    );
    Ok(())
}

fn valid_marker(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
