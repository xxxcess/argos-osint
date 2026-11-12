//! Confirmed folder retirement followed by guarded local cleanup. A pending
//! backend or uncertain platform result leaves its protected journal intact.

use std::sync::Arc;

use anyhow::Context;
use anyhow::Result;
use fleetworks_contracts::Client;
use fleetworks_workspace::native::BackendBinding;
use fleetworks_workspace::native::NativeContainer;
use fleetworks_workspace::native::RemovalOutcome;
use fleetworks_workspace::native::RetirementReceipt;
use fleetworks_workspace::native::WorkspaceOperation;
use serde::Deserialize;
use serde::Serialize;

use crate::platform;
use crate::platform_trash::MacTrash;
use crate::workspaces;
use crate::workspaces::WorkspaceCommand;
use crate::workspaces::WorkspaceUpdate;

pub(crate) struct RetirementGateway {
    pub client: Client,
    pub deployment: String,
}

#[derive(Serialize)]
struct RetireRequest<'a> {
    registration_id: &'a str,
    expected_revision: u64,
}

#[derive(Clone, Deserialize)]
#[serde(tag = "status")]
pub(crate) enum RetirementResponse {
    #[serde(rename = "retirement_pending")]
    Pending {
        deletion_ready: bool,
        registration_id: String,
        workspace_id: String,
        revision: u64,
        admission_fenced: bool,
        sessions_revoked: bool,
        active_work_drained: bool,
        reason_code: String,
    },
    #[serde(rename = "retired")]
    Retired {
        deletion_ready: bool,
        deployment_id: String,
        registration_id: String,
        workspace_id: String,
        registration_revision: u64,
        revision: u64,
        receipt_id: String,
        admission_fenced: bool,
        sessions_revoked: bool,
        active_work_drained: bool,
    },
}

impl RetirementResponse {
    /// Only an authenticated, identity-matching deletion-ready result can be
    /// transformed into the manager's retirement receipt. A202 never can.
    pub(crate) fn validate(self, binding: &BackendBinding) -> Result<Option<RetirementReceipt>> {
        let known = binding
            .receipt
            .as_ref()
            .context("refresh this backend registration before removing the folder")?;
        match self {
            Self::Pending {
                deletion_ready,
                registration_id,
                workspace_id,
                revision,
                admission_fenced,
                sessions_revoked,
                active_work_drained,
                reason_code,
            } => {
                anyhow::ensure!(
                    !deletion_ready
                        && registration_id == binding.registration_id
                        && workspace_id == known.workspace_id
                        && revision >= binding.registration_revision
                        && admission_fenced
                        && sessions_revoked
                        && !active_work_drained
                        && reason_code == "associated_work_active",
                    "backend returned an inconsistent pending retirement"
                );
                Ok(None)
            }
            Self::Retired {
                deletion_ready,
                deployment_id,
                registration_id,
                workspace_id,
                registration_revision,
                revision,
                receipt_id,
                admission_fenced,
                sessions_revoked,
                active_work_drained,
            } => {
                anyhow::ensure!(
                    deletion_ready
                        && deployment_id == binding.deployment_id
                        && registration_id == binding.registration_id
                        && workspace_id == known.workspace_id
                        && registration_revision >= binding.registration_revision
                        && revision == registration_revision
                        && !receipt_id.is_empty()
                        && admission_fenced
                        && sessions_revoked
                        && active_work_drained,
                    "backend has not confirmed safe retirement of this exact folder binding"
                );
                Ok(Some(RetirementReceipt::confirmed(
                    deployment_id,
                    registration_id,
                    registration_revision,
                    receipt_id,
                )))
            }
        }
    }
}

pub(crate) async fn run_removal(
    container: Arc<NativeContainer>,
    command: WorkspaceCommand,
    gateway: Option<RetirementGateway>,
) -> Result<WorkspaceUpdate, String> {
    match run_removal_inner(container.clone(), command, gateway).await {
        Ok(update) => Ok(update),
        Err(error) => refreshed_error(container, error).await,
    }
}

pub(crate) async fn refreshed_error(
    container: Arc<NativeContainer>,
    error: String,
) -> Result<WorkspaceUpdate, String> {
    tokio::task::spawn_blocking(move || -> Result<WorkspaceUpdate> {
        Ok(WorkspaceUpdate {
            snapshot: workspaces::snapshot(&container)?,
            container,
            selected: None,
            message: Some(format!(
                "{error}. Any incomplete folder operation remains recorded for retry."
            )),
        })
    })
    .await
    .map_err(|_| "Folder recovery refresh worker stopped".to_owned())?
    .map_err(|error| error.to_string())
}

async fn run_removal_inner(
    container: Arc<NativeContainer>,
    command: WorkspaceCommand,
    gateway: Option<RetirementGateway>,
) -> Result<WorkspaceUpdate, String> {
    let operation = tokio::task::spawn_blocking({
        let container = container.clone();
        move || -> Result<WorkspaceOperation> {
            match command {
                WorkspaceCommand::Remove { name, kind } => {
                    let workspace = container.select(Some(name.as_ref()))?;
                    container.begin_removal(&workspace, &platform::request_id()?, kind)
                }
                WorkspaceCommand::Resume(id) => container
                    .pending_operations()?
                    .into_iter()
                    .find(|operation| operation.operation_id == id)
                    .context("folder operation is no longer pending"),
                _ => anyhow::bail!("expected a confirmed folder operation"),
            }
        }
    })
    .await
    .map_err(|_| "Folder operation worker stopped".to_owned())?
    .map_err(|error| error.to_string())?;
    let mut waiting_for_work = false;
    if let Some(gateway) = gateway {
        if let Some(binding) = operation.bindings.iter().find(|binding| {
            binding.deployment_id == gateway.deployment
                && !operation.receipts.contains_key(&binding.deployment_id)
        }) {
            let response: RetirementResponse = gateway
                .client
                .post(
                    "/api/v1/connect/registrations/retire",
                    &RetireRequest {
                        registration_id: &binding.registration_id,
                        expected_revision: binding.registration_revision,
                    },
                )
                .await
                .map_err(|error| error.to_string())?;
            if let Some(receipt) = response
                .validate(binding)
                .map_err(|error| error.to_string())?
            {
                tokio::task::spawn_blocking({
                    let container = container.clone();
                    let id = operation.operation_id.clone();
                    move || container.record_retirement(&id, receipt)
                })
                .await
                .map_err(|_| "Retirement journal worker stopped".to_owned())?
                .map_err(|error| error.to_string())?;
            } else {
                waiting_for_work = true;
            }
        }
    }
    tokio::task::spawn_blocking(move || -> Result<WorkspaceUpdate> {
        let current = container.pending_operations()?.into_iter().find(|candidate| candidate.operation_id == operation.operation_id)
            .context("folder operation journal changed")?;
        let message = if current.bindings.iter().any(|binding| !current.receipts.contains_key(&binding.deployment_id)) {
            if waiting_for_work { "Waiting for existing work to finish. Files are preserved; check again to continue.".into() }
            else { "Connect to each listed backend with folder management permission to finish retirement. Files are preserved.".into() }
        } else {
            match container.finish_removal(&current.operation_id)? {
                RemovalOutcome::RemovedFromApp => "Removed from this app. The folder remains on disk.".into(),
                RemovalOutcome::TrashPending(intent) => {
                    container.trash_owned_workspace(&intent, &MacTrash)?;
                    "Moved the verified folder to macOS Trash. Finder can restore it.".into()
                }
                _ => "Folder cleanup remains pending; files are preserved.".into(),
            }
        };
        Ok(WorkspaceUpdate { snapshot: workspaces::snapshot(&container)?, container, selected: None, message: Some(message) })
    }).await.map_err(|_| "Folder cleanup worker stopped".to_owned())?
        .map_err(|error| error.to_string())
}
