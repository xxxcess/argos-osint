//! Workstation registration journal, independent of legacy child-folder state.

use anyhow::Context;
use anyhow::Result;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::connect::RegistrationStatus;
use fleetworks_contracts::context::{JobRequestAssociation, MutationContext, RegisteredContext};
use fleetworks_contracts::cycle_history::DeleteCycleRequest;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::MaintenanceRequest;
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
    #[serde(default)]
    pending_actor_reindexes: Vec<PendingActorReindex>,
    #[serde(default)]
    accepted_actor_reindexes: Vec<AcceptedActorReindex>,
}

/// An exact unresolved deletion captured before delivery; contains no credentials.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PendingCycleDeletion {
    pub context: RegisteredContext,
    pub request: DeleteCycleRequest,
}

/// Bounded non-secret unresolved or accepted actor reindex attempt captured around HTTP POST.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PendingActorReindex {
    pub origin: String,
    pub context: RegisteredContext,
    pub request: MaintenanceRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_job_id: Option<String>,
}

/// Bounded non-secret accepted request-to-job association retained separately from pending guards.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AcceptedActorReindex {
    pub origin: String,
    pub context: RegisteredContext,
    pub request: MaintenanceRequest,
    pub association: JobRequestAssociation,
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

    /// Persist or confirm an exact non-secret actor reindex attempt before HTTP POST.
    pub fn begin_actor_reindex(
        &self,
        origin: &str,
        context: &RegisteredContext,
        request: &MaintenanceRequest,
    ) -> Result<PendingActorReindex> {
        let origin = super::canonical_target_origin(origin).context("invalid gateway origin")?;
        request
            .validate(MaintenanceOperation::ReindexActors)
            .context("invalid actor reindex request")?;
        let mut expected = MutationContext::new(context);
        if let Some(revision) = request.context.expected_configuration_revision {
            expected = expected.with_configuration_revision(revision);
        }
        anyhow::ensure!(
            request.context == expected,
            "actor reindex context mismatch"
        );
        super::require_operation_id(&request.request_id)?;
        let _guard = self.locked()?;
        let (mut journal, object) = self.read_workstation()?;
        if let Some(previous) = journal
            .pending_actor_reindexes
            .iter()
            .find(|pending| pending.origin == origin)
        {
            anyhow::ensure!(
                previous.context == *context && previous.request == *request,
                "reconcile earlier actor index attempt before creating another request"
            );
            return Ok(previous.clone());
        }
        anyhow::ensure!(
            journal.pending_actor_reindexes.len() < 128,
            "actor reindex journal capacity reached"
        );
        let entry = PendingActorReindex {
            origin,
            context: context.clone(),
            request: request.clone(),
            accepted_job_id: None,
        };
        journal.pending_actor_reindexes.push(entry.clone());
        fs::write_json(&self.root_fd, FILE, &journal, object)?;
        self.revalidate_root()?;
        Ok(entry)
    }

    /// Durably record an authoritative accepted job ID alongside the original request
    /// before clearing the unresolved pending entry.
    pub fn record_accepted_actor_reindex(
        &self,
        origin: &str,
        request: &MaintenanceRequest,
        job_id: &str,
    ) -> Result<PendingActorReindex> {
        let origin = super::canonical_target_origin(origin).context("invalid gateway origin")?;
        super::require_operation_id(&request.request_id)?;
        super::require_operation_id(job_id)?;
        let _guard = self.locked()?;
        let (mut journal, object) = self.read_workstation()?;
        let entry = journal
            .pending_actor_reindexes
            .iter_mut()
            .find(|pending| pending.origin == origin && pending.request == *request)
            .ok_or_else(|| {
                anyhow::anyhow!("actor reindex pending attempt missing before receipt persistence")
            })?;
        if let Some(existing) = &entry.accepted_job_id {
            anyhow::ensure!(
                existing == job_id,
                "actor reindex pending attempt already recorded a different accepted job"
            );
            return Ok(entry.clone());
        }
        entry.accepted_job_id = Some(job_id.to_owned());
        let saved = entry.clone();
        fs::write_json(&self.root_fd, FILE, &journal, object)?;
        self.revalidate_root()?;
        Ok(saved)
    }

    /// Read-only lookup of any unresolved actor reindex attempt for one gateway origin.
    pub fn pending_actor_reindex(&self, origin: &str) -> Result<Option<PendingActorReindex>> {
        let origin = super::canonical_target_origin(origin).context("invalid gateway origin")?;
        let _guard = self.locked()?;
        Ok(self
            .read_workstation()?
            .0
            .pending_actor_reindexes
            .into_iter()
            .find(|pending| pending.origin == origin))
    }

    /// Persist a bounded accepted request-to-job association separately from the pending guard,
    /// atomically clearing any matching unresolved pending entry in the same journal write.
    pub fn record_accepted_actor_reindex_association(
        &self,
        origin: &str,
        context: &RegisteredContext,
        request: &MaintenanceRequest,
        association: &JobRequestAssociation,
    ) -> Result<AcceptedActorReindex> {
        let origin = super::canonical_target_origin(origin).context("invalid gateway origin")?;
        request
            .validate(MaintenanceOperation::ReindexActors)
            .context("invalid accepted actor reindex request")?;
        super::require_operation_id(&request.request_id)?;
        super::require_operation_id(&association.job_id)?;
        let mut expected = MutationContext::new(context);
        if let Some(revision) = request.context.expected_configuration_revision {
            expected = expected.with_configuration_revision(revision);
        }
        anyhow::ensure!(
            request.context == expected,
            "accepted actor reindex context mismatch"
        );
        anyhow::ensure!(
            association.origin.request_id == request.request_id
                && association.origin.deployment_id == context.deployment_id
                && association.origin.workstation_id == context.workstation_id
                && association.origin.workspace_id.is_none(),
            "accepted actor reindex association identity mismatch"
        );
        let _guard = self.locked()?;
        let (mut journal, object) = self.read_workstation()?;
        if let Some(existing) = journal
            .pending_actor_reindexes
            .iter()
            .find(|pending| pending.origin == origin)
        {
            anyhow::ensure!(
                existing.context == *context && existing.request == *request,
                "cannot record accepted job while a different pending reindex attempt is active"
            );
        }
        journal
            .pending_actor_reindexes
            .retain(|pending| !(pending.origin == origin && pending.request == *request));
        journal
            .accepted_actor_reindexes
            .retain(|accepted| accepted.origin != origin);
        anyhow::ensure!(
            journal.accepted_actor_reindexes.len() < 128,
            "accepted actor reindex journal capacity reached"
        );
        let saved = AcceptedActorReindex {
            origin,
            context: context.clone(),
            request: request.clone(),
            association: association.clone(),
        };
        journal.accepted_actor_reindexes.push(saved.clone());
        fs::write_json(&self.root_fd, FILE, &journal, object)?;
        self.revalidate_root()?;
        Ok(saved)
    }

    /// Read-only lookup of the retained accepted request-to-job association for one gateway origin.
    pub fn accepted_actor_reindex(&self, origin: &str) -> Result<Option<AcceptedActorReindex>> {
        let origin = super::canonical_target_origin(origin).context("invalid gateway origin")?;
        let _guard = self.locked()?;
        Ok(self
            .read_workstation()?
            .0
            .accepted_actor_reindexes
            .into_iter()
            .find(|accepted| accepted.origin == origin))
    }

    /// Explicitly clear an accepted actor reindex association by origin and exact job ID.
    pub fn clear_accepted_actor_reindex(&self, origin: &str, job_id: &str) -> Result<()> {
        let origin = super::canonical_target_origin(origin).context("invalid gateway origin")?;
        super::require_operation_id(job_id)?;
        let _guard = self.locked()?;
        let (mut journal, object) = self.read_workstation()?;
        journal.accepted_actor_reindexes.retain(|accepted| {
            !(accepted.origin == origin && accepted.association.job_id == job_id)
        });
        fs::write_json(&self.root_fd, FILE, &journal, object)?;
        self.revalidate_root()
    }

    /// Clear only the exact reindex attempt after authoritative acceptance or reconciliation.
    pub fn complete_actor_reindex(&self, origin: &str, request: &MaintenanceRequest) -> Result<()> {
        let origin = super::canonical_target_origin(origin).context("invalid gateway origin")?;
        let _guard = self.locked()?;
        let (mut journal, object) = self.read_workstation()?;
        journal
            .pending_actor_reindexes
            .retain(|pending| !(pending.origin == origin && pending.request == *request));
        fs::write_json(&self.root_fd, FILE, &journal, object)?;
        self.revalidate_root()
    }

    /// Explicitly discard a stale unresolved attempt identified by its exact request ID.
    pub fn clear_stale_actor_reindex(&self, origin: &str, request_id: &str) -> Result<()> {
        let origin = super::canonical_target_origin(origin).context("invalid gateway origin")?;
        super::require_operation_id(request_id)?;
        let _guard = self.locked()?;
        let (mut journal, object) = self.read_workstation()?;
        journal.pending_actor_reindexes.retain(|pending| {
            !(pending.origin == origin && pending.request.request_id == request_id)
        });
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
                        pending_actor_reindexes: Vec::new(),
                        accepted_actor_reindexes: Vec::new(),
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
                && journal.pending_cycle_deletions.len() <= 128
                && journal.pending_actor_reindexes.len() <= 128
                && journal.accepted_actor_reindexes.len() <= 128,
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
        let mut reindex_origins = std::collections::BTreeSet::new();
        for pending in &journal.pending_actor_reindexes {
            if let Some(job_id) = &pending.accepted_job_id {
                super::require_operation_id(job_id)?;
            }
            validate_reindex_entry(
                &pending.origin,
                &pending.context,
                &pending.request,
                &mut reindex_origins,
                "actor reindex journal identity conflict",
            )?;
        }
        let mut accepted_origins = std::collections::BTreeSet::new();
        for accepted in &journal.accepted_actor_reindexes {
            super::require_operation_id(&accepted.association.job_id)?;
            validate_reindex_entry(
                &accepted.origin,
                &accepted.context,
                &accepted.request,
                &mut accepted_origins,
                "accepted actor reindex journal identity conflict",
            )?;
            anyhow::ensure!(
                accepted.association.origin.request_id == accepted.request.request_id
                    && accepted.association.origin.deployment_id == accepted.context.deployment_id
                    && accepted.association.origin.workstation_id
                        == accepted.context.workstation_id
                    && accepted.association.origin.workspace_id.is_none(),
                "accepted actor reindex association conflict"
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

fn validate_reindex_entry<'a>(
    origin: &'a str,
    context: &RegisteredContext,
    request: &MaintenanceRequest,
    seen_origins: &mut std::collections::BTreeSet<&'a str>,
    message: &'static str,
) -> Result<()> {
    super::require_operation_id(&request.request_id)?;
    let mut expected = MutationContext::new(context);
    if let Some(revision) = request.context.expected_configuration_revision {
        expected = expected.with_configuration_revision(revision);
    }
    anyhow::ensure!(
        super::canonical_target_origin(origin).ok().as_deref() == Some(origin)
            && request
                .validate(MaintenanceOperation::ReindexActors)
                .is_ok()
            && request.context == expected
            && seen_origins.insert(origin),
        "{message}"
    );
    Ok(())
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

    #[test]
    fn actor_reindex_attempt_persists_per_origin_for_exact_reconciliation() {
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
        let request =
            MaintenanceRequest::for_reindex("reindex-req-1".into(), MutationContext::new(&context))
                .expect("request");

        let saved = container
            .begin_actor_reindex("http://127.0.0.1:8080", &context, &request)
            .expect("persist reindex");
        assert_eq!(saved.origin, "http://127.0.0.1:8080/");
        drop(container);

        let reopened = NativeContainer::open_settings(&home).expect("reopen settings");
        let pending = reopened
            .pending_actor_reindex("http://127.0.0.1:8080")
            .expect("read pending")
            .expect("pending attempt");
        assert_eq!(pending.request, request);

        // Exact replay succeeds; conflicting request ID for the same origin is rejected.
        reopened
            .begin_actor_reindex("http://127.0.0.1:8080", &context, &request)
            .expect("exact replay");
        let other =
            MaintenanceRequest::for_reindex("reindex-req-2".into(), MutationContext::new(&context))
                .expect("other");
        assert!(reopened
            .begin_actor_reindex("http://127.0.0.1:8080", &context, &other)
            .is_err());

        // Separate origin can still track its own unresolved attempt cleanly.
        reopened
            .begin_actor_reindex("http://127.0.0.1:8081", &context, &other)
            .expect("second origin");
        let accepted = reopened
            .record_accepted_actor_reindex("http://127.0.0.1:8080", &request, "job-accepted-1")
            .expect("durably record accepted job ID");
        assert_eq!(accepted.accepted_job_id.as_deref(), Some("job-accepted-1"));
        assert!(reopened
            .record_accepted_actor_reindex("http://127.0.0.1:8080", &request, "job-accepted-2")
            .is_err());
        let after_restart = NativeContainer::open_settings(&home).expect("reopen after accept");
        let recovered = after_restart
            .pending_actor_reindex("http://127.0.0.1:8080")
            .expect("read accepted")
            .expect("accepted entry survives before explicit completion");
        assert_eq!(recovered.accepted_job_id.as_deref(), Some("job-accepted-1"));
        after_restart
            .complete_actor_reindex("http://127.0.0.1:8080", &request)
            .expect("complete first");
        assert!(after_restart
            .pending_actor_reindex("http://127.0.0.1:8080")
            .expect("read")
            .is_none());
        assert!(after_restart
            .pending_actor_reindex("http://127.0.0.1:8081")
            .expect("read second")
            .is_some());

        // Separate bounded accepted-job record survives pending completion and replaces per origin.
        let origin_one = fleetworks_contracts::context::JobOrigin::new(
            &context,
            fleetworks_contracts::context::OriginSource::Native,
            request.request_id.clone(),
            "2026-11-15T00:00:00Z".into(),
            0,
            vec![],
        );
        let association_one =
            JobRequestAssociation::new("job-assoc-1".into(), origin_one, "".into());
        after_restart
            .record_accepted_actor_reindex_association(
                "http://127.0.0.1:8081",
                &context,
                &other,
                &JobRequestAssociation::new(
                    "job-assoc-2".into(),
                    fleetworks_contracts::context::JobOrigin::new(
                        &context,
                        fleetworks_contracts::context::OriginSource::Native,
                        other.request_id.clone(),
                        "2026-11-15T00:00:00Z".into(),
                        0,
                        vec![],
                    ),
                    String::new(),
                ),
            )
            .expect("record accepted association clears matching pending guard");
        assert!(after_restart
            .pending_actor_reindex("http://127.0.0.1:8081")
            .expect("pending cleared after acceptance")
            .is_none());
        let saved_assoc = after_restart
            .accepted_actor_reindex("http://127.0.0.1:8081")
            .expect("read accepted association")
            .expect("accepted association retained");
        assert_eq!(saved_assoc.association.job_id, "job-assoc-2");
        after_restart
            .record_accepted_actor_reindex_association(
                "http://127.0.0.1:8080",
                &context,
                &request,
                &association_one,
            )
            .expect("record origin one accepted association");
        after_restart
            .clear_accepted_actor_reindex("http://127.0.0.1:8080", "job-assoc-1")
            .expect("clear explicit accepted association");
        assert!(after_restart
            .accepted_actor_reindex("http://127.0.0.1:8080")
            .expect("cleared")
            .is_none());
    }

    #[test]
    fn local_and_remote_actor_reindex_journals_stay_isolated() {
        let temporary = tempfile::tempdir().expect("temporary home");
        let home_path = temporary
            .path()
            .canonicalize()
            .expect("physical temporary home");
        let container = NativeContainer::open_settings(&home_path).expect("settings installation");
        let remote_receipt = RegistrationReceipt::new(
            "remote-deployment".into(),
            "node-remote".into(),
            "reg-remote".into(),
            None,
            1,
        );
        let local_receipt = RegistrationReceipt::new(
            "local-deployment".into(),
            "node-local".into(),
            "reg-local".into(),
            None,
            1,
        );
        let remote_context =
            RegisteredContext::new(remote_receipt, 1, 1, "cfg-1".into(), "access-remote".into())
                .expect("remote context");
        let local_context =
            RegisteredContext::new(local_receipt, 1, 1, "cfg-1".into(), "access-local".into())
                .expect("local context");
        let remote_req = MaintenanceRequest::for_reindex(
            "req-remote".into(),
            MutationContext::new(&remote_context),
        )
        .expect("remote req");
        let local_req = MaintenanceRequest::for_reindex(
            "req-local".into(),
            MutationContext::new(&local_context),
        )
        .expect("local req");
        container
            .begin_actor_reindex("http://127.0.0.1:8080", &remote_context, &remote_req)
            .expect("remote reindex");
        container
            .begin_actor_reindex("local://embedded", &local_context, &local_req)
            .expect("local reindex");
        assert!(container
            .begin_actor_reindex("local://..", &local_context, &local_req)
            .is_err());

        let pending_local = container
            .pending_actor_reindex("local://embedded")
            .expect("read local")
            .expect("local entry");
        let pending_remote = container
            .pending_actor_reindex("http://127.0.0.1:8080")
            .expect("read remote")
            .expect("remote entry");
        assert_eq!(pending_local.origin, "local://embedded");
        assert_eq!(pending_remote.origin, "http://127.0.0.1:8080/");
        assert_ne!(pending_local.request, pending_remote.request);

        container
            .clear_stale_actor_reindex("local://embedded", "req-local")
            .expect("clear local");
        assert!(container
            .pending_actor_reindex("local://embedded")
            .expect("local cleared")
            .is_none());
        assert!(container
            .pending_actor_reindex("http://127.0.0.1:8080")
            .expect("remote preserved")
            .is_some());
    }
}
