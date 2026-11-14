//! Workstation registration journal, independent of legacy child-folder state.

use anyhow::Context;
use anyhow::Result;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::connect::RegistrationStatus;
use fleetworks_contracts::context::{MutationContext, RegisteredContext};
use fleetworks_contracts::cycle_history::DeleteCycleRequest;
use rustix::fs::AtFlags;
use serde::Deserialize;
use serde::Serialize;

use super::fs;
use super::NativeContainer;

const FILE: &str = ".native-workstation.json";

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Binding {
    deployment_id: String,
    pending: Option<RegistrationEnsureRequest>,
    receipt: Option<RegistrationReceipt>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u16,
    installation_marker: String,
    bindings: Vec<Binding>,
    #[serde(default)]
    pending_cycle_deletions: Vec<PendingCycleDeletion>,
}

/// An exact unresolved deletion captured before delivery; contains no credentials.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PendingCycleDeletion {
    pub context: RegisteredContext,
    pub request: DeleteCycleRequest,
}

impl NativeContainer {
    /// Persist/reuse the exact non-secret request before any network delivery.
    /// Cancellation and restart never manufacture a replacement request ID.
    pub fn begin_workstation_registration(
        &self,
        deployment_id: &str,
        request: RegistrationEnsureRequest,
    ) -> Result<RegistrationEnsureRequest> {
        super::require_operation_id(deployment_id)?;
        super::require_operation_id(&request.request_id)?;
        anyhow::ensure!(
            request.installation_marker == self.installation_marker,
            "registration does not match this protected installation"
        );
        let _guard = self.locked()?;
        let (mut journal, object) = self.read_workstation()?;
        let index = match journal
            .bindings
            .iter()
            .position(|binding| binding.deployment_id == deployment_id)
        {
            Some(index) => index,
            None => {
                anyhow::ensure!(
                    journal.bindings.len() < 128,
                    "workstation journal capacity reached"
                );
                journal.bindings.push(Binding {
                    deployment_id: deployment_id.to_owned(),
                    pending: None,
                    receipt: None,
                });
                journal.bindings.len() - 1
            }
        };
        if let Some(pending) = &journal.bindings[index].pending {
            return Ok(pending.clone());
        }
        journal.bindings[index].pending = Some(request.clone());
        fs::write_json(&self.root_fd, FILE, &journal, object)?;
        self.revalidate_root()?;
        Ok(request)
    }

    /// Retain an authenticated receipt only when it matches the durable request.
    pub fn record_workstation_registration(
        &self,
        request: &RegistrationEnsureRequest,
        receipt: &RegistrationReceipt,
    ) -> Result<()> {
        anyhow::ensure!(
            receipt.is_consistent() && receipt.status == RegistrationStatus::Active,
            "inconsistent or inactive workstation receipt"
        );
        let _guard = self.locked()?;
        let (mut journal, object) = self.read_workstation()?;
        let binding = journal
            .bindings
            .iter_mut()
            .find(|binding| binding.deployment_id == receipt.deployment_id)
            .context("registration has no journaled deployment")?;
        anyhow::ensure!(
            binding.pending.as_ref() == Some(request),
            "registration request identity changed"
        );
        if let Some(previous) = &binding.receipt {
            anyhow::ensure!(
                previous.workstation_id == receipt.workstation_id,
                "backend changed the protected installation's workstation identity"
            );
        }
        binding.pending = None;
        binding.receipt = Some(receipt.clone());
        fs::write_json(&self.root_fd, FILE, &journal, object)?;
        self.revalidate_root()
    }

    /// Keep an exact deletion input across process exit and uncertain delivery.
    pub fn begin_cycle_deletion(
        &self,
        context: &RegisteredContext,
        request: &DeleteCycleRequest,
    ) -> Result<()> {
        let configuration_revision = request
            .context
            .expected_configuration_revision
            .context("cycle deletion requires a captured configuration revision")?;
        anyhow::ensure!(
            request.context
                == MutationContext::new(context)
                    .with_configuration_revision(configuration_revision),
            "cycle deletion context mismatch"
        );
        super::require_operation_id(&request.request_id)?;
        let _guard = self.locked()?;
        let (mut journal, object) = self.read_workstation()?;
        if let Some(previous) = journal.pending_cycle_deletions.iter().find(|pending| {
            pending.context.deployment_id == context.deployment_id
                && pending.context.workstation_id == context.workstation_id
        }) {
            anyhow::ensure!(
                previous.context == *context && previous.request == *request,
                "resolve earlier deletion before creating another request"
            );
            return Ok(());
        }
        anyhow::ensure!(
            journal.pending_cycle_deletions.len() < 128,
            "deletion journal capacity reached"
        );
        journal.pending_cycle_deletions.push(PendingCycleDeletion {
            context: context.clone(),
            request: request.clone(),
        });
        fs::write_json(&self.root_fd, FILE, &journal, object)?;
        self.revalidate_root()
    }

    /// Read-only recovery inputs, never an implicit replay under new registration.
    pub fn pending_cycle_deletions(&self) -> Result<Vec<PendingCycleDeletion>> {
        let _guard = self.locked()?;
        Ok(self.read_workstation()?.0.pending_cycle_deletions)
    }

    /// Clear only the exact command after an authoritative receipt/read-only lookup.
    pub fn complete_cycle_deletion(&self, request: &DeleteCycleRequest) -> Result<()> {
        let _guard = self.locked()?;
        let (mut journal, object) = self.read_workstation()?;
        journal
            .pending_cycle_deletions
            .retain(|pending| pending.request != *request);
        fs::write_json(&self.root_fd, FILE, &journal, object)?;
        self.revalidate_root()
    }

    fn read_workstation(&self) -> Result<(Journal, Option<fs::ObjectIdentity>)> {
        let (journal, object) =
            match rustix::fs::statat(&self.root_fd, FILE, AtFlags::SYMLINK_NOFOLLOW) {
                Err(rustix::io::Errno::NOENT) => (
                    Journal {
                        version: 2,
                        installation_marker: self.installation_marker.clone(),
                        bindings: Vec::new(),
                        pending_cycle_deletions: Vec::new(),
                    },
                    None,
                ),
                Ok(_) => {
                    let (journal, object) =
                        fs::read_json_limited::<Journal>(&self.root_fd, FILE, 256 * 1024)?;
                    (journal, Some(object))
                }
                Err(error) => return Err(error).context("inspect workstation journal"),
            };
        anyhow::ensure!(
            journal.version == 2
                && journal.installation_marker == self.installation_marker
                && journal.bindings.len() <= 128
                && journal.pending_cycle_deletions.len() <= 128,
            "workstation journal format/ownership requires repair"
        );
        let mut deletions = std::collections::BTreeSet::new();
        for pending in &journal.pending_cycle_deletions {
            super::require_operation_id(&pending.request.request_id)?;
            // Retain older unresolved inputs without a configuration revision for
            // read-only recovery; new delivery always requires an explicit revision.
            let mut expected = MutationContext::new(&pending.context);
            if let Some(revision) = pending.request.context.expected_configuration_revision {
                expected = expected.with_configuration_revision(revision);
            }
            anyhow::ensure!(
                pending.request.context == expected
                    && deletions.insert((
                        &pending.context.deployment_id,
                        &pending.context.workstation_id
                    )),
                "cycle deletion journal identity conflict"
            );
        }
        let mut deployments = std::collections::BTreeSet::new();
        for binding in &journal.bindings {
            super::require_operation_id(&binding.deployment_id)?;
            anyhow::ensure!(
                deployments.insert(&binding.deployment_id),
                "duplicate workstation deployment binding"
            );
            if let Some(request) = &binding.pending {
                super::require_operation_id(&request.request_id)?;
                anyhow::ensure!(
                    request.installation_marker == self.installation_marker,
                    "workstation pending request ownership changed"
                );
            }
            if let Some(receipt) = &binding.receipt {
                anyhow::ensure!(
                    receipt.is_consistent() && receipt.deployment_id == binding.deployment_id,
                    "workstation receipt identity changed"
                );
            }
        }
        Ok((journal, object))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_startup_and_pending_registration_preserve_installation_without_child() {
        let temporary = tempfile::tempdir().expect("temporary home");
        let home_path = temporary
            .path()
            .canonicalize()
            .expect("physical temporary home");
        let container = NativeContainer::open_settings(&home_path).expect("settings installation");
        assert!(!container.root().join("default").exists());
        let installation = container.installation_marker().to_owned();
        let request =
            RegistrationEnsureRequest::new("request-one".into(), None, installation.clone());
        let first = container
            .begin_workstation_registration("deployment-one", request)
            .expect("journal intent");
        drop(container);
        let reopened = NativeContainer::open_settings(&home_path).expect("reopen settings");
        assert_eq!(reopened.installation_marker(), installation);
        let replacement = RegistrationEnsureRequest::new(
            "request-two".into(),
            Some("192.0.2.42".parse().expect("IP")),
            installation,
        );
        assert_eq!(
            reopened
                .begin_workstation_registration("deployment-one", replacement)
                .expect("reuse pending"),
            first
        );
        assert!(!reopened.root().join("default").exists());
    }
    #[test]
    fn cycle_deletion_is_durable_exact_and_independent_of_child_folders() {
        let temporary = tempfile::tempdir().expect("temporary home");
        let home = temporary.path().canonicalize().expect("physical home");
        let container = NativeContainer::open_settings(&home).expect("settings installation");
        let receipt = RegistrationReceipt::new(
            "deployment-one".into(),
            "node-one".into(),
            "registration-one".into(),
            None,
            1,
        );
        let context = RegisteredContext::new(
            receipt,
            1,
            1,
            "configuration-one".into(),
            "access-one".into(),
        )
        .expect("context");
        let request = DeleteCycleRequest::new(
            "delete-one".into(),
            MutationContext::new(&context).with_configuration_revision(0),
            "atlas-cycle-synthetic".into(),
            1,
        );
        let mut unconfirmed = request.clone();
        unconfirmed.context.expected_configuration_revision = None;
        assert!(container
            .begin_cycle_deletion(&context, &unconfirmed)
            .is_err());
        container
            .begin_cycle_deletion(&context, &request)
            .expect("journal exact deletion with explicit revision zero");
        drop(container);
        let reopened = NativeContainer::open_settings(&home).expect("reopen settings");
        let pending = reopened
            .pending_cycle_deletions()
            .expect("read pending deletion");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].request, request);
        assert_eq!(
            pending[0].request.context.expected_configuration_revision,
            Some(0)
        );
        reopened
            .begin_cycle_deletion(&context, &request)
            .expect("exact captured replay");
        let mut changed_revision = request.clone();
        changed_revision.context.expected_configuration_revision = Some(1);
        assert!(reopened
            .begin_cycle_deletion(&context, &changed_revision)
            .is_err());
        let mut changed_origin = context.clone();
        changed_origin.registration_revision += 1;
        assert!(reopened
            .begin_cycle_deletion(&changed_origin, &request)
            .is_err());
        let different = DeleteCycleRequest::new(
            "delete-two".into(),
            MutationContext::new(&context).with_configuration_revision(0),
            "atlas-cycle-other".into(),
            1,
        );
        assert!(reopened.begin_cycle_deletion(&context, &different).is_err());
        reopened
            .complete_cycle_deletion(&request)
            .expect("clear exact accepted command");
        assert!(reopened
            .pending_cycle_deletions()
            .expect("pending deleted")
            .is_empty());
        assert!(!reopened.root().join("default").exists());
    }
}
