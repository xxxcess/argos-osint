//! Persist-before-network workstation IP rebinding and immutable receipt history.

use crate::native::legacy_connect::ConnectId;
use crate::native::legacy_connect::RegistrationEnsureRequest;
use crate::native::legacy_connect::RegistrationReceipt;
use crate::native::legacy_connect::RegistrationStatus;
use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;

use super::state::BackendBinding;
use super::state::OperationPhase;
use super::state::WorkspaceState;
use super::NativeContainer;
use super::NativeWorkspace;

/// Exact authenticated rebind intent. It contains non-secret identities only.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct RebindAttempt {
    pub prior: RegistrationReceipt,
    pub request: RegistrationEnsureRequest,
    pub completed: Option<RegistrationReceipt>,
}

impl NativeContainer {
    /// Journal a new IP change before sending it. A pending outcome blocks
    /// registration and removal for this backend until its exact retry resolves.
    pub fn begin_rebind_attempt(
        &self,
        workspace: &NativeWorkspace,
        prior: &RegistrationReceipt,
        request: RegistrationEnsureRequest,
    ) -> Result<RebindAttempt> {
        super::require_operation_id(&request.request_id)?;
        super::require_operation_id(&prior.deployment_id)?;
        ConnectId::new(request.workstation_ip, request.workspace_name.clone())?;
        let mut guard = self.locked()?;
        let index = self.record_index(&guard.catalog, workspace)?;
        self.verify_record(&guard.catalog.workspaces[index])?;
        if let Some(existing) = guard
            .catalog
            .rebind_attempts
            .iter()
            .find(|attempt| attempt.request.request_id == request.request_id)
        {
            anyhow::ensure!(
                existing.prior == *prior && existing.request == request,
                "rebind request identity already has different input"
            );
            return Ok(existing.clone());
        }
        let record = &guard.catalog.workspaces[index];
        anyhow::ensure!(
            prior.is_consistent()
                && prior.status == RegistrationStatus::Active
                && request.workspace_name == record.name
                && request.workspace_marker == record.marker
                && request.installation_marker == self.installation_marker
                && matches!(
                    record.state,
                    WorkspaceState::Local | WorkspaceState::Registered
                ),
            "rebind intent must match the verified owned child"
        );
        let binding = record
            .bindings
            .iter()
            .find(|binding| binding.deployment_id == prior.deployment_id && !binding.retired)
            .context("no active binding for this backend")?;
        anyhow::ensure!(
            !binding.stale
                && binding.receipt.as_ref() == Some(prior)
                && binding.registration_id == prior.registration_id
                && binding.registration_revision == prior.revision,
            "refresh this exact registration receipt before rebinding"
        );
        anyhow::ensure!(
            !guard
                .catalog
                .registration_attempts
                .iter()
                .any(|attempt| attempt.deployment_id == prior.deployment_id)
                && !guard
                    .catalog
                    .rebind_attempts
                    .iter()
                    .any(|attempt| attempt.completed.is_none()
                        && attempt.prior.deployment_id == prior.deployment_id),
            "reconcile existing registration/rebind outcomes before changing this workstation IP"
        );
        anyhow::ensure!(
            !guard
                .catalog
                .operations
                .iter()
                .any(|operation| operation.phase != OperationPhase::Complete
                    && operation
                        .bindings
                        .iter()
                        .any(|binding| binding.deployment_id == prior.deployment_id)),
            "finish pending folder retirement before rebinding this backend"
        );
        for child in &guard.catalog.workspaces {
            for known in &child.bindings {
                if known.deployment_id == prior.deployment_id && !known.retired {
                    anyhow::ensure!(
                        known
                            .receipt
                            .as_ref()
                            .is_some_and(|receipt| receipt.workstation_id == prior.workstation_id),
                        "refresh all known child registrations before rebinding this workstation"
                    );
                }
            }
        }
        anyhow::ensure!(
            guard.catalog.rebind_attempts.len() < 512,
            "local rebind history needs reviewed archival"
        );
        let attempt = RebindAttempt {
            prior: prior.clone(),
            request,
            completed: None,
        };
        guard.catalog.rebind_attempts.push(attempt.clone());
        self.persist(&mut guard)?;
        Ok(attempt)
    }

    /// Exact unresolved retries, never a new IP or inferred backend result.
    pub fn pending_rebind_attempts(&self) -> Result<Vec<RebindAttempt>> {
        Ok(self
            .locked()?
            .catalog
            .rebind_attempts
            .into_iter()
            .filter(|attempt| attempt.completed.is_none())
            .collect())
    }

    /// Record only a validated authenticated rebind response. Stable backend
    /// identities cannot change; replaced receipts remain immutable history.
    pub fn complete_rebind_attempt(
        &self,
        request_id: &str,
        receipt: &RegistrationReceipt,
    ) -> Result<NativeWorkspace> {
        let mut guard = self.locked()?;
        let attempt_index = guard
            .catalog
            .rebind_attempts
            .iter()
            .position(|attempt| attempt.request.request_id == request_id)
            .context("unknown rebind operation; preserve the existing binding")?;
        let attempt = guard.catalog.rebind_attempts[attempt_index].clone();
        let index = guard
            .catalog
            .workspaces
            .iter()
            .position(|record| record.marker == attempt.request.workspace_marker)
            .context("rebind folder identity is missing")?;
        self.verify_record(&guard.catalog.workspaces[index])?;
        anyhow::ensure!(
            receipt.is_consistent()
                && receipt.status == RegistrationStatus::Active
                && receipt.deployment_id == attempt.prior.deployment_id
                && receipt.workstation_id == attempt.prior.workstation_id
                && receipt.workspace_id == attempt.prior.workspace_id
                && receipt.workspace_name == attempt.request.workspace_name
                && receipt.workstation_ip == attempt.request.workstation_ip,
            "rebind response changed stable identity or confirmed IP/name"
        );
        if let Some(completed) = &attempt.completed {
            anyhow::ensure!(
                completed == receipt,
                "rebind completion receipt conflicts with its journal"
            );
            return Ok(self.snapshot(&guard.catalog.workspaces[index]));
        }
        let binding_index = guard.catalog.workspaces[index]
            .bindings
            .iter()
            .position(|binding| binding.deployment_id == receipt.deployment_id && !binding.retired)
            .context("rebind active binding is missing")?;
        anyhow::ensure!(
            guard.catalog.workspaces[index].bindings[binding_index]
                .receipt
                .as_ref()
                == Some(&attempt.prior),
            "original binding changed while rebind was unresolved"
        );
        for (child_index, child) in guard.catalog.workspaces.iter_mut().enumerate() {
            for binding in &mut child.bindings {
                if binding.deployment_id == receipt.deployment_id && !binding.retired {
                    binding.stale = child_index != index;
                }
            }
        }
        let record = &mut guard.catalog.workspaces[index];
        record.binding_history.push(attempt.prior.clone());
        record.bindings[binding_index] = BackendBinding {
            deployment_id: receipt.deployment_id.clone(),
            registration_id: receipt.registration_id.clone(),
            registration_revision: receipt.revision,
            retired: false,
            stale: false,
            receipt: Some(receipt.clone()),
        };
        record.state = WorkspaceState::Registered;
        guard.catalog.rebind_attempts[attempt_index].completed = Some(receipt.clone());
        let workspace = self.snapshot(&guard.catalog.workspaces[index]);
        self.persist(&mut guard)?;
        Ok(workspace)
    }
}
