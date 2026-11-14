//! Explicit durable Natural activation worker seam. Training never calls it.
//! An absent post-effect proof stays uncertain; matching active model IDs alone
//! cannot attribute another process's activation to this operation.

mod effect;
#[cfg(all(test, feature = "classifier"))]
mod tests;

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use fleetworks_contracts::classifier_activation::ActivationRequest;
use fleetworks_contracts::classifier_activation::ActivationResult;
use fleetworks_contracts::classifier_activation::ActivationState;
use fleetworks_contracts::classifier_activation::ActiveModelRevision;
use fleetworks_contracts::classifier_activation::CandidateIdentity;
use fleetworks_contracts::classifier_activation::CandidatePreview;
use fleetworks_contracts::classifier_activation::SyntheticBaselineConsent;

use super::artifact;
use super::artifact::ActivationAuthorizationDecision;
use super::artifact::ArtifactStore;
use super::artifact::ModelId;
use super::artifact::ModelManifest;
use super::artifact::WriterLease;
use super::training::SyntheticActivation;
use super::training::TrainRequest;
use super::training::TrainingReport;
use super::ClassifierError;
use super::Provenance;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ActivationFailure {
    Invalid,
    Unavailable,
    Unsupported,
    Busy,
    Stale,
    ConsentRequired,
    CandidateUnqualified,
    Interrupted,
}
impl std::fmt::Display for ActivationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason_code())
    }
}
impl std::error::Error for ActivationFailure {}
impl ActivationFailure {
    pub fn reason_code(self) -> &'static str {
        match self {
            Self::Invalid => "activation_invalid",
            Self::Unavailable => "activation_unavailable",
            Self::Unsupported => "natural_activation_unsupported",
            Self::Busy => "natural_classifier_busy",
            Self::Stale => "activation_prior_changed",
            Self::ConsentRequired => "synthetic_baseline_consent_required",
            Self::CandidateUnqualified => "candidate_not_qualified",
            Self::Interrupted => "activation_interrupted_before_effect",
        }
    }
}
impl From<ClassifierError> for ActivationFailure {
    fn from(error: ClassifierError) -> Self {
        match error {
            ClassifierError::Busy => Self::Busy,
            ClassifierError::StaleActivation => Self::Stale,
            ClassifierError::ActivationDenied { .. }
            | ClassifierError::InvalidArtifact(_)
            | ClassifierError::Incompatible(_) => Self::CandidateUnqualified,
            ClassifierError::Cancelled => Self::Interrupted,
            _ => Self::Unavailable,
        }
    }
}

pub fn input_hash(request: &ActivationRequest) -> Result<String, ActivationFailure> {
    serde_json::to_vec(request)
        .map(|bytes| artifact::sha256_hex(&bytes))
        .map_err(|_| ActivationFailure::Invalid)
}

pub fn active_revision(store: &ArtifactStore) -> Result<ActiveModelRevision, ActivationFailure> {
    store
        .read_active_integrity()
        .map_err(ActivationFailure::from)?
        .map_or(Ok(ActiveModelRevision::NeverActivated), |active| {
            revision(&active)
        })
}
fn revision(active: &ModelManifest) -> Result<ActiveModelRevision, ActivationFailure> {
    ActiveModelRevision::active(
        active.model_id.to_string(),
        active.activation_generation,
        active.manifest_sha256.clone(),
    )
    .map_err(|_| ActivationFailure::Invalid)
}

fn candidate(
    store: &ArtifactStore,
    training_job_id: &str,
    model_id: &str,
    restore: bool,
) -> Result<(CandidateIdentity, TrainingReport, bool), ActivationFailure> {
    let id = ModelId::parse(model_id).ok_or(ActivationFailure::Invalid)?;
    let candidate = if restore {
        store.validate_model(&id)
    } else {
        store.validate_model_integrity(&id)
    }
    .map_err(ActivationFailure::from)?;
    let report: TrainingReport = serde_json::from_slice(
        &store
            .read_report_raw(&id)
            .map_err(ActivationFailure::from)?,
    )
    .map_err(|_| ActivationFailure::CandidateUnqualified)?;
    if report.request_id != training_job_id || report.model_id != id {
        return Err(ActivationFailure::CandidateUnqualified);
    }
    let identity = CandidateIdentity::new(
        id.to_string(),
        training_job_id.into(),
        candidate.manifest.dataset_sha256.clone(),
        effect::manifest_hash(store, &id)?,
        candidate.manifest.model_sha256.clone(),
        candidate.manifest.report_sha256.clone(),
    )
    .map_err(|_| ActivationFailure::Invalid)?;
    let synthetic = matches!(report.provenance, Provenance::SyntheticBaseline { .. });
    Ok((identity, report, synthetic))
}

/// Read only after actual registered training-job privacy/membership checks.
/// This checks bytes/compatibility without loading a model or starting training.
pub fn preview(
    store: &ArtifactStore,
    training_job_id: &str,
    model_id: &str,
) -> Result<CandidatePreview, ActivationFailure> {
    let (identity, _, synthetic) = candidate(store, training_job_id, model_id, false)?;
    let id = ModelId::parse(model_id).ok_or(ActivationFailure::Invalid)?;
    let previously_activated = effect::previously_activated(store, &id)?;
    let supported = cfg!(feature = "classifier") && synthetic && !previously_activated;
    Ok(CandidatePreview::new(
        identity,
        active_revision(store)?,
        synthetic,
        supported,
        if previously_activated {
            "candidate_previously_activated_use_explicit_rollback"
        } else if supported {
            "explicit_synthetic_baseline_consent_required"
        } else {
            "activation_evidence_unavailable"
        }
        .into(),
    ))
}

/// Reconcile only this operation's immutable terminal proof. Missing proof is
/// never replaced with inference from the current active candidate/generation.
pub fn reconcile(
    store: &ArtifactStore,
    job_id: &str,
    request: &ActivationRequest,
) -> Result<Option<ActivationResult>, ActivationFailure> {
    effect::read_result(store, job_id, request)
}

/// Perform one already admitted effect under the existing exclusive OS lease.
/// A durable started intent prevents retry from executing an ambiguous effect
/// twice. No promotion evidence can arrive from an HTTP caller.
pub fn execute(
    store: &ArtifactStore,
    lease: &WriterLease,
    job_id: &str,
    request: &ActivationRequest,
    cancel: &AtomicBool,
) -> Result<ActivationResult, ActivationFailure> {
    request.validate().map_err(|_| ActivationFailure::Invalid)?;
    if !cfg!(feature = "classifier") {
        return Err(ActivationFailure::Unsupported);
    }
    match reconcile(store, job_id, request) {
        Ok(Some(result)) => return Ok(result),
        Ok(None) => {}
        Err(_) => {
            return Ok(uncertain(
                job_id,
                request,
                "activation_effect_proof_unavailable",
            ))
        }
    }
    match effect::started(store, job_id, request) {
        Ok(false) => {}
        Ok(true) => {
            return Ok(uncertain(
                job_id,
                request,
                "activation_previous_effect_uncertain",
            ))
        }
        Err(_) => return Ok(uncertain(job_id, request, "activation_intent_unavailable")),
    }
    if cancel.load(Ordering::Acquire) {
        return Err(ActivationFailure::Interrupted);
    }
    let (identity, report, synthetic) = candidate(
        store,
        &request.candidate.training_job_id,
        &request.candidate.model_id,
        true,
    )?;
    if identity != request.candidate {
        return Err(ActivationFailure::CandidateUnqualified);
    }
    let prior = store.read_active().map_err(ActivationFailure::from)?;
    let prior_revision = prior
        .as_ref()
        .map_or(Ok(ActiveModelRevision::NeverActivated), revision)?;
    if prior_revision != request.expected_prior {
        return Err(ActivationFailure::Stale);
    }
    // Reviewed-real promotion has its own independent-evidence owner. This
    // explicit baseline route cannot invent or accept promotion evidence.
    if !synthetic {
        return Err(ActivationFailure::CandidateUnqualified);
    }
    if request.synthetic_baseline != SyntheticBaselineConsent::ActivateSyntheticBaseline {
        return Err(ActivationFailure::ConsentRequired);
    }
    let id = ModelId::parse(&identity.model_id).ok_or(ActivationFailure::Invalid)?;
    if effect::previously_activated(store, &id)? {
        return Err(ActivationFailure::CandidateUnqualified);
    }
    let validated = store.validate_model(&id).map_err(ActivationFailure::from)?;
    let authorization = artifact::authorize_activation(
        lease,
        &validated,
        &report,
        &TrainRequest {
            request_id: report.request_id.clone(),
            model_id: id.clone(),
            provenance: report.provenance.clone(),
            synthetic_activation: SyntheticActivation::ExplicitOptIn,
        },
        None,
        prior.as_ref(),
    )
    .map_err(ActivationFailure::from)?;
    let ActivationAuthorizationDecision::Authorized(authorization) = authorization else {
        return Err(ActivationFailure::CandidateUnqualified);
    };
    if cancel.load(Ordering::Acquire) {
        return Err(ActivationFailure::Interrupted);
    }
    effect::begin(store, job_id, request)?;
    let result = match store.activate(&id, &authorization, lease) {
        Ok(active) => ActivationResult::new(
            job_id.into(),
            identity,
            ActivationState::Activated,
            Some(revision(&active)?),
            "natural_candidate_activated".into(),
        ),
        Err(ClassifierError::ActivationCommitUncertain { .. }) => {
            return Ok(uncertain(job_id, request, "artifact_commit_uncertain"))
        }
        Err(error) => ActivationResult::new(
            job_id.into(),
            identity,
            ActivationState::Rejected,
            None,
            ActivationFailure::from(error).reason_code().into(),
        ),
    };
    // Only this synchronous successful call may write an owned effect proof.
    // If proof persistence fails, report uncertainty rather than rediscovering
    // a matching active pointer and claiming another writer's work.
    if effect::finish(store, job_id, request, &result).is_err() {
        return Ok(uncertain(
            job_id,
            request,
            "activation_effect_proof_uncertain",
        ));
    }
    Ok(result)
}

pub fn uncertain(job_id: &str, request: &ActivationRequest, reason: &str) -> ActivationResult {
    ActivationResult::new(
        job_id.into(),
        request.candidate.clone(),
        ActivationState::EffectUncertain,
        None,
        reason.into(),
    )
}
