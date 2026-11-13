use std::collections::BTreeMap;

use crate::native::legacy_connect::RegistrationEnsureRequest;
use crate::native::legacy_connect::RegistrationReceipt;
use fleetworks_contracts::connect::WorkspaceName;
use serde::Deserialize;
use serde::Serialize;

use super::fs::ObjectIdentity;

pub(super) const FORMAT_VERSION: u16 = 1;
pub(super) const CATALOG_FILE: &str = ".native-state.json";
pub(super) const LOCK_FILE: &str = ".native.lock";
pub(super) const WORKSPACE_MARKER_FILE: &str = ".fleetworks-workspace.json";

/// Local state never claims backend registration or deletion success implicitly.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WorkspaceState {
    CreationPending,
    Local,
    Registered,
    DeletionPending,
    CleanupPending,
    Removed,
}

/// Non-secret record of a known backend binding requiring explicit retirement.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct BackendBinding {
    pub deployment_id: String,
    pub registration_id: String,
    pub registration_revision: u64,
    pub retired: bool,
    /// A workstation-wide rebind invalidates other children until authenticated
    /// ensure refreshes their current registration using the same markers.
    #[serde(default)]
    pub stale: bool,
    /// Stable backend identities from the authenticated response. Earlier
    /// current-format records must refresh before rebind or retirement.
    #[serde(default)]
    pub receipt: Option<RegistrationReceipt>,
}

/// Durable pre-ensure intent. An unknown HTTP outcome must be reconciled using
/// this exact idempotent request before the folder can be removed or deleted.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct RegistrationAttempt {
    pub deployment_id: String,
    pub request: RegistrationEnsureRequest,
}

/// Distinguishes forgetting an app association from deleting its directory.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RemovalKind {
    RemoveFromApp,
    Delete,
}

/// Durable local operation phase. Files remain intact until a real Trash adapter.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OperationPhase {
    RetirementPending,
    CleanupPending,
    Complete,
}

/// Receipt from an authenticated backend retirement action, not a local guess.
///
/// The gateway adapter must verify the response and caller authority before
/// constructing this value. Merely possessing this DTO grants no backend right.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct RetirementReceipt {
    pub deployment_id: String,
    pub registration_id: String,
    pub registration_revision: u64,
    pub receipt_id: String,
    pub admission_fenced: bool,
    pub sessions_revoked: bool,
    pub active_work_drained: bool,
}

impl RetirementReceipt {
    /// Record confirmed retirement only after the adapter verified all three facts.
    #[must_use]
    pub fn confirmed(
        deployment_id: String,
        registration_id: String,
        registration_revision: u64,
        receipt_id: String,
    ) -> Self {
        Self {
            deployment_id,
            registration_id,
            registration_revision,
            receipt_id,
            admission_fenced: true,
            sessions_revoked: true,
            active_work_drained: true,
        }
    }
}

/// Persisted progress across all known backend bindings and local cleanup.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct WorkspaceOperation {
    pub operation_id: String,
    pub workspace_name: WorkspaceName,
    pub workspace_marker: String,
    pub kind: RemovalKind,
    pub phase: OperationPhase,
    pub bindings: Vec<BackendBinding>,
    pub receipts: BTreeMap<String, RetirementReceipt>,
    pub trash_receipt: Option<String>,
    #[serde(default)]
    pub platform_trash: Option<super::trash::PreparedTrash>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Catalog {
    pub version: u16,
    pub installation_marker: String,
    pub root_object: ObjectIdentity,
    pub workspaces: Vec<WorkspaceRecord>,
    pub operations: Vec<WorkspaceOperation>,
    #[serde(default)]
    pub registration_attempts: Vec<RegistrationAttempt>,
    #[serde(default)]
    pub rebind_attempts: Vec<super::rebind::RebindAttempt>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkspaceRecord {
    pub name: WorkspaceName,
    pub marker: String,
    pub object: Option<ObjectIdentity>,
    pub protected_default: bool,
    pub state: WorkspaceState,
    pub bindings: Vec<BackendBinding>,
    #[serde(default)]
    pub binding_history: Vec<RegistrationReceipt>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkspaceMarker {
    pub version: u16,
    pub installation_marker: String,
    pub name: WorkspaceName,
    pub marker: String,
    pub object: ObjectIdentity,
    pub protected_default: bool,
}
