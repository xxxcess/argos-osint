//! Home repair preserves prior lineage and requires explicit synthetic consent.
#![cfg(feature = "classifier")]

#[path = "fixtures/classifier/support.rs"]
mod support;

use std::fs;
use std::path::Path;
use std::time::Duration;

use argos_osint_core::classifier::artifact::{
    authorize_activation, ActivationAuthorizationDecision,
};
use argos_osint_core::classifier::dataset::DatasetRequest;
use argos_osint_core::classifier::readiness::{
    ClassifierHome, ManualTrainRequest, TrainingJob, TrainingJobState,
};
use argos_osint_core::classifier::training::SyntheticActivation;
use argos_osint_core::classifier::{ClassifierError, ClassifierStatus};
use tempfile::TempDir;

fn request(root: &Path) -> DatasetRequest {
    let mut request = DatasetRequest::default();
    request.cwd = root.into();
    request.executable = root.join("argos");
    request.home = root.join("home");
    request
}
fn csv(root: &Path) -> DatasetRequest {
    let path = root.join("fixture.csv");
    let mut writer = csv::Writer::from_path(&path).expect("isolated synthetic CSV");
    writer
        .write_record([
            "sample_id",
            "title",
            "description",
            "domain_labels",
            "primary_domain",
            "secondary_domain",
            "subcategories",
            "source_kind",
            "difficulty",
            "dataset_version",
            "annotation_notes",
            "split",
            "scenario_group",
            "language",
            "synthetic",
            "labels_complete",
            "expected_admission",
            "rejection_reason",
        ])
        .expect("header");
    let domains = [
        "governance",
        "security",
        "economic",
        "human",
        "environmental",
        "scientific",
    ];
    for split in ["train", "calibration", "test"] {
        for (index, domain) in domains.into_iter().enumerate() {
            writer
                .write_record([
                    format!("sample-{split}-{index}"),
                    format!("Fictional {domain} bulletin {split}"),
                    format!("Synthetic officials describe {domain} changes in a fictional region."),
                    domain.into(),
                    domain.into(),
                    String::new(),
                    String::new(),
                    "synthetic".into(),
                    "single_domain_boundary".into(),
                    "synthetic-management-fixture-v1".into(),
                    "Never model features".into(),
                    split.into(),
                    format!("family-{split}-{index}"),
                    "en".into(),
                    "true".into(),
                    "true".into(),
                    "reject".into(),
                    "insufficient_domains".into(),
                ])
                .expect("row");
        }
    }
    writer.flush().expect("persist fixture");
    let mut selected = request(root);
    selected.explicit_file = Some(path);
    selected
}

async fn finished(home: &ClassifierHome, id: &str) -> TrainingJob {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let job = home
                .job_detail(id)
                .await
                .expect("accepted job remains readable");
            if matches!(
                job.state,
                TrainingJobState::Completed
                    | TrainingJobState::Failed
                    | TrainingJobState::Cancelled
                    | TrainingJobState::Interrupted
            ) {
                return job;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("bounded synthetic fixture training")
}

async fn verified_idle(
    home: &ClassifierHome,
    request: &DatasetRequest,
) -> argos_osint_core::classifier::readiness::Readiness {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let readiness = home.refresh_integrity(request).await.unwrap();
            if readiness.can_train {
                return readiness;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("completed owner releases its writer")
}

#[tokio::test]
async fn incompatible_prior_keeps_candidate_report_until_explicit_repair_advances_its_generation() {
    // Genuine learned weights first activate normally. The shared migration
    // fixture then consistently rewrites them to the previous feature schema.
    let (prior_home, legacy) = tokio::task::spawn_blocking(|| {
        let prior = support::Candidate::from_fixture(support::trained());
        let validated = prior.write();
        let decision = authorize_activation(
            &prior.fixture.lease,
            &validated,
            &prior.report,
            &prior.request(SyntheticActivation::ExplicitOptIn),
            None,
            None,
        )
        .unwrap();
        let ActivationAuthorizationDecision::Authorized(capability) = decision else {
            panic!("fixture explicit consent");
        };
        let active = prior
            .store
            .activate(&prior.fixture.model, &capability, &prior.fixture.lease)
            .unwrap();
        let legacy = support::install_legacy_prior(&prior, &active);
        assert!(matches!(
            prior.store.read_active(),
            Err(ClassifierError::Incompatible(_))
        ));
        // The old worker and its writer lease are dropped inside this owner
        // before the new Home can accept a job; only the isolated home survives.
        drop(prior.fixture.worker);
        drop(prior.fixture.lease);
        (prior.fixture.home, legacy)
    })
    .await
    .unwrap();
    let root = TempDir::new().unwrap();
    let mut selected = csv(root.path());
    selected.home = prior_home.path().to_path_buf();
    let home = ClassifierHome::new(selected.home.clone());
    let pointer = selected.home.join("classifier/active.json");
    let prior_bytes = fs::read(&pointer).unwrap();
    let activation_record = selected.home.join("classifier/activations/1.json");
    let old_record = fs::read(&activation_record).unwrap();
    let state = home.refresh_integrity(&selected).await.unwrap();
    assert_eq!(state.state, ClassifierStatus::Incompatible);
    assert!(state.can_train);
    assert!(!state.can_infer);

    let accepted = home
        .start_training(ManualTrainRequest::new(
            selected.clone(),
            "legacy-candidate".into(),
            SyntheticActivation::RetainPrior,
        ))
        .await
        .unwrap();
    let candidate = finished(&home, &accepted.job_id).await;
    assert_eq!(
        candidate.state,
        TrainingJobState::Completed,
        "{:?}",
        candidate.diagnostic
    );
    assert_eq!(
        candidate.diagnostic.as_ref().unwrap().code,
        "prior_classifier_incompatible"
    );
    let outcome = candidate.outcome.as_ref().unwrap();
    assert_eq!(outcome.state, "trained_not_activated");
    assert_eq!(outcome.report.model_id.as_str(), outcome.candidate_model_id);
    assert_eq!(fs::read(&pointer).unwrap(), prior_bytes);
    assert_eq!(fs::read(&activation_record).unwrap(), old_record);
    assert!(selected
        .home
        .join("classifier/models")
        .join(&outcome.candidate_model_id)
        .join("manifest.json")
        .is_file());

    // D7 persists the full terminal result before releasing the writer. A
    // verified readiness read, rather than a sleep or Busy retry, gates the
    // immediate next operation.
    let idle = verified_idle(&home, &selected).await;
    assert!(idle.can_train);
    let accepted = home
        .start_training(ManualTrainRequest::new(
            selected.clone(),
            "legacy-explicit-repair".into(),
            SyntheticActivation::ExplicitOptIn,
        ))
        .await
        .unwrap();
    let repaired = finished(&home, &accepted.job_id).await;
    assert_eq!(
        repaired.state,
        TrainingJobState::Completed,
        "{:?}",
        repaired.diagnostic
    );
    assert!(repaired.diagnostic.is_none(), "{:?}", repaired.diagnostic);
    let outcome = repaired.outcome.as_ref().unwrap();
    assert_eq!(outcome.state, "manual_seed_activated");
    assert_eq!(
        outcome.active_model_id.as_deref(),
        Some(outcome.candidate_model_id.as_str())
    );
    assert_eq!(outcome.report.model_id.as_str(), outcome.candidate_model_id);
    let active = argos_osint_core::classifier::artifact::ArtifactStore::new(&selected.home)
        .unwrap()
        .read_active()
        .unwrap()
        .unwrap();
    assert_eq!(
        active.activation_generation,
        legacy.activation_generation + 1
    );
    assert_eq!(active.previous_model_id, Some(legacy.model_id));
    assert_eq!(active.model_id.as_str(), outcome.candidate_model_id);
    assert_eq!(
        fs::read(&activation_record).unwrap(),
        old_record,
        "repair preserves prior committed history"
    );
    let ready = home.refresh_integrity(&selected).await.unwrap();
    assert_eq!(ready.state, ClassifierStatus::Ready);
    assert!(ready.can_infer);
    let idle = verified_idle(&home, &selected).await;
    assert_eq!(idle.state, ClassifierStatus::Ready);
    assert!(idle.can_infer);
    assert!(idle.can_train);
    home.shutdown().await.unwrap();
}

#[tokio::test]
async fn readiness_cannot_advertise_training_while_writer_is_held_and_unlock_invalidates_cache() {
    let root = TempDir::new().unwrap();
    let selected = csv(root.path());
    let home = ClassifierHome::new(selected.home.clone());
    assert!(home.refresh_integrity(&selected).await.unwrap().can_train);
    assert!(
        !selected.home.exists(),
        "a missing writer probe never creates the home or lock"
    );
    let store = argos_osint_core::classifier::artifact::ArtifactStore::new(&selected.home).unwrap();
    let lease = store.try_writer().unwrap();
    let busy = home.refresh_integrity(&selected).await.unwrap();
    assert!(!busy.can_train);
    let diagnostic = busy.diagnostic.unwrap();
    assert_eq!(diagnostic.code, "writer_busy");
    assert!(diagnostic.retryable);
    assert!(
        !home.probe(&selected).await.can_train,
        "cached status remains truthful while held"
    );
    drop(lease);
    assert!(
        home.probe(&selected).await.can_train,
        "unlock changes the probe identity even without a file write"
    );
    assert!(home.refresh_integrity(&selected).await.unwrap().can_train);
    home.shutdown().await.unwrap();
}
