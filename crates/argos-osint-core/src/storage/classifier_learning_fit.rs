//! Owned fitting, comparative measurements and atomic activation. The artifact
//! writer also serializes review/approval changes through the final readback.
use super::classifier_admission::AdmissionOwner;
use super::classifier_learning::Outcome;
use super::jobs::ClaimedTask;
use crate::classifier::artifact::{ArtifactStore, ModelId};
use crate::classifier::calibration::{self, ActivationAuthorizationDecision, ActivationOutcome};
use crate::classifier::learning::LearningInput;
use crate::classifier::readiness::ClassifierHome;
use crate::classifier::training::{self, SyntheticActivation, TrainRequest};
use crate::classifier::{ClassifierError, WorkControl};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

pub(super) fn fit(
    owner: AdmissionOwner,
    home: ClassifierHome,
    task: ClaimedTask,
    input: LearningInput,
    model: ModelId,
    control: WorkControl,
    stage: Arc<AtomicUsize>,
    runtime: tokio::runtime::Handle,
) -> Result<Outcome, ClassifierError> {
    crate::classifier::learning::validate_pipeline(&input)?;
    let store = ArtifactStore::new(home.home())?;
    let lease = Arc::new(store.try_writer()?);
    control.check_active()?;
    runtime
        .block_on(super::classifier_review::verify_capture(
            owner.repository(),
            &input,
        ))
        .map_err(|_| ClassifierError::InvalidArtifact("learning_review_capture_changed".into()))?;
    #[cfg(feature = "classifier")]
    let prepared = match runtime.block_on(home.pin(None)) {
        Ok(baseline) => crate::classifier::promotion::prepare(&input, baseline, &store, &control)?,
        Err(ClassifierError::Unavailable) => None,
        Err(error) => return Err(error),
    };
    let request = TrainRequest {
        request_id: task.job_id.clone(),
        model_id: model.clone(),
        provenance: input.provenance.clone(),
        synthetic_activation: SyntheticActivation::RetainPrior,
    };
    let mut worker = crate::classifier::start_candidate_worker(&model, lease.clone(), &control)?;
    #[cfg(feature = "classifier")]
    let running = Arc::new(AtomicBool::new(true));
    #[cfg(feature = "classifier")]
    let peak = Arc::new(AtomicU64::new(0));
    #[cfg(feature = "classifier")]
    let (signal, ready) = std::sync::mpsc::channel();
    #[cfg(feature = "classifier")]
    let concurrent = prepared.as_ref().map(|prepared| {
        crate::classifier::promotion::concurrent(
            prepared,
            ready,
            running.clone(),
            control.clone(),
            peak.clone(),
        )
    });
    #[cfg(feature = "classifier")]
    let _ = signal.send(());
    let fitted = training::train_candidate(
        &mut *worker,
        &request,
        &input.corpus,
        &control,
        &mut |progress| {
            stage.store(progress.stage as usize, Ordering::Release);
        },
    );
    // Always join the comparative foreground work, including fit/cancel errors.
    #[cfg(feature = "classifier")]
    running.store(false, Ordering::Release);
    #[cfg(feature = "classifier")]
    let concurrent = concurrent
        .map(|handle| {
            runtime
                .block_on(handle)
                .map_err(|_| ClassifierError::WorkerStopped)
        })
        .transpose()?;
    #[cfg(feature = "classifier")]
    let training_stats = worker.performance();
    drop(worker);
    let candidate = fitted?;
    control.check_active()?;
    let manifest = training::candidate_manifest(&input.corpus, &request, &candidate)?;
    let validated =
        match store.write_candidate(&lease, &manifest, &candidate.payload, &candidate.report) {
            Ok(artifact) => artifact,
            Err(ClassifierError::CandidateAlreadyExists) => {
                let existing = store.validate_model(&model)?;
                if existing.manifest != manifest
                    || existing.payload != candidate.payload
                    || store.read_report_raw(&model)?
                        != crate::classifier::artifact::bounded_json_bytes(&candidate.report)?
                {
                    return Err(ClassifierError::InvalidArtifact(
                        "learning_candidate_replay_conflict".into(),
                    ));
                }
                existing
            }
            Err(error) => return Err(error),
        };
    let prior = store.read_active_integrity()?;
    #[cfg(feature = "classifier")]
    let evidence = if let (Some(prepared), Some(Ok(concurrent)), Ok(training_stats)) =
        (prepared.as_ref(), concurrent, training_stats)
    {
        // A failed measurement retains the immutable candidate; it does not
        // rewrite an unavailable counter or failed resource check into a pass.
        crate::classifier::runtime_proof::verify(&control)
            .and_then(|proof| {
                crate::classifier::promotion::evidence(
                    &input,
                    prepared,
                    &candidate,
                    &manifest,
                    validated.clone(),
                    concurrent,
                    training_stats,
                    &proof,
                    peak.load(Ordering::Acquire),
                    &control,
                )
            })
            .ok()
    } else {
        None
    };
    #[cfg(not(feature = "classifier"))]
    let evidence = None;
    control.check_active()?;
    // Recheck durable authority immediately before entering atomic activation.
    if !runtime
        .block_on(super::classifier_learning::renew(&owner, &task))
        .unwrap_or(false)
    {
        return Err(ClassifierError::Cancelled);
    }
    let decision = calibration::authorize_activation(
        &lease,
        &validated,
        &candidate.report,
        &request,
        evidence.as_ref(),
        prior.as_ref(),
    )?;
    let (activation, promoted) = match decision {
        ActivationAuthorizationDecision::RetainPrior(outcome) => (outcome, Some(false)),
        ActivationAuthorizationDecision::Authorized(capability) => {
            control.check_active()?;
            match store.activate(&model, &capability, &lease) {
                Ok(active) => (
                    ActivationOutcome::Activated {
                        reason: active.activation_reason,
                        model_id: active.model_id,
                        generation: active.activation_generation,
                    },
                    Some(true),
                ),
                Err(ClassifierError::ActivationCommitUncertain { .. }) => {
                    // Read back durable identity under the writer. Unknown
                    // integrity stays unknown and must never report prior retained.
                    let promoted = store.read_active_integrity().ok().map(|active| {
                        active.is_some_and(|active| {
                            active.model_id == model && active.model_sha256 == manifest.model_sha256
                        })
                    });
                    return Ok(Outcome {
                        schema_version: "classifier-learning-outcome-v2".into(),
                        model_id: model.to_string(),
                        state: if promoted == Some(true) {
                            "activated_after_readback"
                        } else {
                            "activation_uncertain"
                        }
                        .into(),
                        reason: "activation_commit_uncertain_readback".into(),
                        report_sha256: manifest.report_sha256,
                        promoted,
                        activation: None,
                        promotion_evidence: evidence,
                    });
                }
                Err(error) => return Err(error),
            }
        }
    };
    let reason = match &activation {
        ActivationOutcome::CandidateOnly { reason_code, .. } => reason_code.clone(),
        ActivationOutcome::Activated { .. } => "reviewed_automatic_promotion".into(),
    };
    Ok(Outcome {
        schema_version: "classifier-learning-outcome-v2".into(),
        model_id: model.to_string(),
        state: if promoted == Some(true) {
            "activated"
        } else {
            "candidate_retained"
        }
        .into(),
        reason,
        report_sha256: manifest.report_sha256,
        promoted,
        activation: Some(activation),
        promotion_evidence: evidence,
    })
}
