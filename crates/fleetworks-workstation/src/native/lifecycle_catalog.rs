//! Validation of the additive native lifecycle journal, without disk effects.

use std::collections::BTreeSet;

use crate::native::legacy_connect::RegistrationReceipt;
use anyhow::Result;

use super::state::Catalog;
use super::state::OperationPhase;
use super::state::RemovalKind;
use super::state::WorkspaceState;
use super::NativeContainer;

impl NativeContainer {
    pub(super) fn validate_lifecycle_catalog(&self, catalog: &Catalog) -> Result<()> {
        for record in &catalog.workspaces {
            anyhow::ensure!(
                record.bindings.len() <= 256 && record.binding_history.len() <= 512,
                "backend binding history exceeds bounded capacity"
            );
            let mut deployments = BTreeSet::new();
            for binding in &record.bindings {
                super::require_operation_id(&binding.deployment_id)?;
                super::require_operation_id(&binding.registration_id)?;
                anyhow::ensure!(
                    binding.registration_revision > 0 && deployments.insert(&binding.deployment_id),
                    "backend binding revision or deployment identity conflicts"
                );
                if let Some(receipt) = &binding.receipt {
                    validate_receipt(receipt)?;
                    anyhow::ensure!(
                        receipt.deployment_id == binding.deployment_id
                            && receipt.registration_id == binding.registration_id
                            && receipt.revision == binding.registration_revision
                            && receipt.workspace_name == record.name,
                        "backend binding differs from its authenticated receipt snapshot"
                    );
                }
            }
            for receipt in &record.binding_history {
                validate_receipt(receipt)?;
                anyhow::ensure!(
                    receipt.workspace_name == record.name
                        && record.bindings.iter().any(|binding| binding.deployment_id
                            == receipt.deployment_id
                            && binding
                                .receipt
                                .as_ref()
                                .is_some_and(|current| current.workstation_id
                                    == receipt.workstation_id
                                    && current.workspace_id == receipt.workspace_id)),
                    "historical receipt changed the stable backend workspace identity"
                );
            }
        }
        let mut operations = BTreeSet::new();
        let mut pending_markers = BTreeSet::new();
        for operation in &catalog.operations {
            super::require_operation_id(&operation.operation_id)?;
            anyhow::ensure!(
                operations.insert(&operation.operation_id),
                "duplicate folder operation identity"
            );
            let record = catalog
                .workspaces
                .iter()
                .find(|record| {
                    record.marker == operation.workspace_marker
                        && record.name == operation.workspace_name
                })
                .ok_or_else(|| anyhow::anyhow!("folder operation has no owned child identity"))?;
            anyhow::ensure!(
                !record.protected_default && !record.name.is_default(),
                "protected default has an invalid removal operation"
            );
            if operation.phase != OperationPhase::Complete {
                anyhow::ensure!(
                    pending_markers.insert(&operation.workspace_marker)
                        && matches!(
                            (operation.phase, record.state),
                            (
                                OperationPhase::RetirementPending,
                                WorkspaceState::DeletionPending
                            ) | (
                                OperationPhase::CleanupPending,
                                WorkspaceState::CleanupPending
                            )
                        ),
                    "pending folder operation conflicts with local lifecycle state"
                );
            }
            let mut deployments = BTreeSet::new();
            for binding in &operation.bindings {
                anyhow::ensure!(
                    deployments.insert(&binding.deployment_id)
                        && record
                            .bindings
                            .iter()
                            .any(|known| known.deployment_id == binding.deployment_id
                                && known.registration_id == binding.registration_id),
                    "removal binding changed or duplicated its known backend identity"
                );
            }
            for (deployment, receipt) in &operation.receipts {
                super::require_operation_id(&receipt.receipt_id)?;
                anyhow::ensure!(
                    &receipt.deployment_id == deployment
                        && receipt.admission_fenced
                        && receipt.sessions_revoked
                        && receipt.active_work_drained
                        && operation
                            .bindings
                            .iter()
                            .any(|binding| &binding.deployment_id == deployment
                                && binding.registration_id == receipt.registration_id
                                && receipt.registration_revision >= binding.registration_revision),
                    "retirement receipt does not confirm its known backend binding"
                );
            }
            if let Some(prepared) = &operation.platform_trash {
                let reference = &prepared.reference;
                anyhow::ensure!(
                    operation.kind == RemovalKind::Delete
                        && matches!(
                            operation.phase,
                            OperationPhase::CleanupPending | OperationPhase::Complete
                        )
                        && !reference.reference.is_empty()
                        && reference.reference.len() <= 8192
                        && !reference.reference.chars().any(char::is_control)
                        && reference.trash_directory.is_absolute(),
                    "saved platform Trash reference is invalid"
                );
                anyhow::ensure!(
                    prepared.trash_object.device == self.root_object.device
                        && prepared.trash_object.owner == self.root_object.owner
                        && prepared.trash_object != self.root_object
                        && catalog
                            .workspaces
                            .iter()
                            .all(|child| child.object != Some(prepared.trash_object)),
                    "saved OS Trash identity overlaps a managed child or another owner/volume"
                );
            }
        }
        anyhow::ensure!(
            catalog.rebind_attempts.len() <= 512,
            "rebind journal exceeds bounded capacity"
        );
        let mut request_ids: BTreeSet<_> = catalog
            .registration_attempts
            .iter()
            .map(|attempt| &attempt.request.request_id)
            .collect();
        let mut pending_deployments = BTreeSet::new();
        for attempt in &catalog.rebind_attempts {
            validate_receipt(&attempt.prior)?;
            super::require_operation_id(&attempt.request.request_id)?;
            anyhow::ensure!(
                request_ids.insert(&attempt.request.request_id)
                    && attempt.request.installation_marker == self.installation_marker
                    && catalog
                        .workspaces
                        .iter()
                        .any(|record| record.marker == attempt.request.workspace_marker
                            && record.name == attempt.request.workspace_name
                            && record.name == attempt.prior.workspace_name),
                "rebind journal conflicts with its owned workspace identity"
            );
            if let Some(completed) = &attempt.completed {
                validate_receipt(completed)?;
                anyhow::ensure!(
                    completed.deployment_id == attempt.prior.deployment_id
                        && completed.workstation_id == attempt.prior.workstation_id
                        && completed.workspace_id == attempt.prior.workspace_id
                        && completed.workspace_name == attempt.request.workspace_name
                        && completed.workstation_ip == attempt.request.workstation_ip,
                    "rebind completion changed stable backend identity or confirmed IP"
                );
            } else {
                anyhow::ensure!(
                    pending_deployments.insert(&attempt.prior.deployment_id)
                        && !catalog
                            .registration_attempts
                            .iter()
                            .any(|registration| registration.deployment_id
                                == attempt.prior.deployment_id)
                        && !catalog.operations.iter().any(|operation| operation.phase
                            != OperationPhase::Complete
                            && operation.bindings.iter().any(
                                |binding| binding.deployment_id == attempt.prior.deployment_id
                            )),
                    "unresolved rebind overlaps another backend operation"
                );
            }
        }
        Ok(())
    }
}

fn validate_receipt(receipt: &RegistrationReceipt) -> Result<()> {
    anyhow::ensure!(
        receipt.is_consistent(),
        "registration snapshot is inconsistent"
    );
    for value in [
        &receipt.deployment_id,
        &receipt.workstation_id,
        &receipt.workspace_id,
        &receipt.registration_id,
    ] {
        super::require_operation_id(value)?;
    }
    Ok(())
}
