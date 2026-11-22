//! Operator-owned activation of a qualified Atlas BERT candidate.

use std::path::Path;

use super::artifact::{self, ArtifactStore};
use super::bert::{
    self, active_atlas_bert_revision, verify_atlas_bert_candidate, ATLAS_BERT_ACTIVE_FILE,
};
use super::remote::PinIdentity;
use super::{ClassifierError, WorkControl};

/// Reads the exact prior revision for an operator's activation confirmation.
///
/// # Errors
/// Returns an error if the selected baseline or active candidate is invalid.
pub async fn revision(home: &Path) -> Result<String, ClassifierError> {
    let home = home.to_owned();
    tokio::task::spawn_blocking(move || active_atlas_bert_revision(&home))
        .await
        .map_err(|_| ClassifierError::WorkerStopped)?
}

/// Verifies and explicitly activates the staged candidate under the workspace writer lease.
///
/// The lease spans verification, compare-and-swap and publication. Cancellation is
/// observed up to the commit boundary. Once that boundary is crossed, publication
/// finishes even if the caller stops waiting.
///
/// # Errors
/// Returns an error for invalid evidence, cancellation, a concurrent writer,
/// a stale prior revision, or a filesystem failure.
pub async fn activate(
    home: &Path,
    frozen_sha256: &str,
    evaluation_sha256: &str,
    expected_revision: &str,
    control: &WorkControl,
) -> Result<PinIdentity, ClassifierError> {
    control.check_active()?;
    let lease_home = home.to_owned();
    let prior = expected_revision.to_owned();
    let (lease, root) = tokio::task::spawn_blocking(move || {
        let store = ArtifactStore::new(&lease_home)?;
        let lease = store.try_writer()?;
        if active_atlas_bert_revision(&lease_home)? != prior {
            return Err(ClassifierError::StaleActivation);
        }
        Ok::<_, ClassifierError>((lease, store.root().to_owned()))
    })
    .await
    .map_err(|_| ClassifierError::WorkerStopped)??;

    let verified =
        verify_atlas_bert_candidate(home, frozen_sha256, evaluation_sha256, control).await?;
    let bytes = artifact::bounded_json_bytes(&verified)?;
    let final_home = home.to_owned();
    let prior = expected_revision.to_owned();
    let control = control.clone();
    tokio::task::spawn_blocking(move || {
        let _lease = lease;
        // Reconcile the exact prior and the staged contents under the same
        // writer lease before publishing the bytes checked by the real worker.
        let configured_root = artifact::root_for(&final_home)?;
        if configured_root.canonicalize().ok().as_ref() != Some(&root)
            || active_atlas_bert_revision(&final_home)? != prior
        {
            return Err(ClassifierError::StaleActivation);
        }
        bert::validate_atlas_bert_identity(&final_home, &verified)?;
        control.check_active()?;
        artifact::write_atomic(&root.join(ATLAS_BERT_ACTIVE_FILE), &bytes)?;
        Ok(verified)
    })
    .await
    .map_err(|_| ClassifierError::WorkerStopped)?
}
