//! Public classifier-home contracts with isolated homes and synthetic data.
use std::fs;
use std::path::Path;
use std::time::Duration;

use argos_osint_core::classifier::dataset::DatasetRequest;
use argos_osint_core::classifier::readiness::{
    ClassifierHome, EvaluateRequest, HomeError, ManualTrainRequest, TrainingJob, TrainingJobState,
};
use argos_osint_core::classifier::training::SyntheticActivation;
use argos_osint_core::classifier::{ClassifierError, ClassifierStatus};
use tempfile::TempDir;
// Test homes share the actual process runtime reservation budget. Individual
// tests still exercise their own concurrency; unrelated fits are serialized.
static RUNTIME_CASES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
                    "synthetic-development-v1".into(),
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

#[tokio::test]
async fn construction_and_missing_status_do_not_create_home_or_start_storage() {
    let _runtime_case = RUNTIME_CASES.lock().await;
    let root = TempDir::new().expect("isolated root");
    let selected = request(root.path());
    let home = ClassifierHome::new(selected.home.clone());
    assert!(!selected.home.exists());
    let readiness = home
        .refresh_integrity(&selected)
        .await
        .expect("missing state is data");
    assert_eq!(
        readiness.state,
        if cfg!(feature = "classifier") {
            ClassifierStatus::Missing
        } else {
            ClassifierStatus::Unavailable
        }
    );
    assert!(!readiness.can_infer);
    assert!(!readiness.can_train);
    assert!(readiness.active_model.is_none());
    assert!(
        !selected.home.exists(),
        "read-only status must not initialize home"
    );
    let json = serde_json::to_value(&readiness).expect("wire DTO");
    assert!(json
        .get("active_model")
        .is_some_and(serde_json::Value::is_null));
    assert!(json.get("training").is_some_and(serde_json::Value::is_null));
    assert_eq!(json["schema_version"], "classifier-readiness-v1");
    home.shutdown().await.expect("owned refresh cleanup");
}

#[cfg(feature = "classifier")]
#[tokio::test]
async fn status_hashes_without_parsing_csv_and_first_fit_is_available() {
    let _runtime_case = RUNTIME_CASES.lock().await;
    let root = TempDir::new().expect("isolated root");
    let mut selected = request(root.path());
    let path = root.path().join("not-csv.csv");
    fs::write(&path, b"This is deliberately not a CSV corpus").expect("non-CSV fixture");
    selected.explicit_file = Some(path);
    let home = ClassifierHome::new(selected.home.clone());
    let readiness = home
        .refresh_integrity(&selected)
        .await
        .expect("status reads hashes, not CSV schema");
    assert_eq!(readiness.state, ClassifierStatus::Missing);
    assert!(readiness.can_train);
    assert!(readiness.retraining_available);
    assert_eq!(readiness.dataset.availability, "present");
    assert_eq!(readiness.dataset.hash_check, "verified");
    assert_eq!(readiness.dataset.change, "unknown");
    assert_eq!(readiness.dataset.sha256.as_ref().map(String::len), Some(64));
    assert!(!selected.home.exists());
    home.shutdown().await.expect("shutdown");
}

#[cfg(feature = "classifier")]
#[tokio::test]
async fn corrupt_active_pointer_is_invalid_not_missing_or_ready() {
    let _runtime_case = RUNTIME_CASES.lock().await;
    let root = TempDir::new().expect("isolated root");
    let selected = request(root.path());
    fs::create_dir_all(selected.home.join("classifier")).expect("classifier fixture directory");
    fs::write(selected.home.join("classifier/active.json"), b"{corrupt").expect("bad pointer");
    let before = fs::read(selected.home.join("classifier/active.json")).expect("pointer bytes");
    let home = ClassifierHome::new(selected.home.clone());
    let readiness = home
        .refresh_integrity(&selected)
        .await
        .expect("integrity error is status data");
    assert_eq!(readiness.state, ClassifierStatus::Invalid);
    assert_eq!(readiness.integrity, "failed");
    assert!(!readiness.can_infer);
    assert_eq!(
        before,
        fs::read(selected.home.join("classifier/active.json")).expect("unchanged pointer")
    );
    home.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn invalid_ids_and_unknown_jobs_never_traverse_paths_or_initialize_home() {
    let _runtime_case = RUNTIME_CASES.lock().await;
    let root = TempDir::new().expect("isolated root");
    let selected = request(root.path());
    let home = ClassifierHome::new(selected.home.clone());
    for id in [
        "",
        "../outside",
        "/tmp/outside",
        "classifier-../../outside",
        "classifier-ABC",
    ] {
        assert!(matches!(
            home.job_detail(id).await,
            Err(HomeError::NotFound { .. })
        ));
        assert!(matches!(
            home.cancel_training(id).await,
            Err(HomeError::NotFound { .. })
        ));
    }
    let unknown = format!("classifier-{}", "0".repeat(64));
    assert!(matches!(
        home.job_detail(&unknown).await,
        Err(HomeError::NotFound { .. })
    ));
    assert!(!selected.home.exists());
    home.shutdown().await.expect("shutdown");
}

#[cfg(feature = "classifier")]
#[tokio::test]
async fn invalid_manual_request_ids_are_rejected_before_file_or_writer_work() {
    let _runtime_case = RUNTIME_CASES.lock().await;
    let root = TempDir::new().expect("isolated root");
    let selected = request(root.path());
    let home = ClassifierHome::new(selected.home.clone());
    for id in ["", ".", "..", "a/b", "white space", "line\nbreak"] {
        let operation = ManualTrainRequest::new(
            selected.clone(),
            id.into(),
            SyntheticActivation::RetainPrior,
        );
        assert!(matches!(
            home.start_training(operation).await,
            Err(HomeError::InvalidRequest { .. })
        ));
    }
    assert!(!selected.home.exists());
    home.shutdown().await.expect("shutdown");
}

#[cfg(feature = "classifier")]
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

#[cfg(feature = "classifier")]
#[tokio::test]
async fn configured_save_directory_survives_restart_and_csv_removal() {
    let _runtime_case = RUNTIME_CASES.lock().await;
    let root = TempDir::new().expect("isolated configuration and artifacts");
    let selected = csv(root.path());
    fs::create_dir_all(&selected.home).unwrap();
    let saved = argos_osint_core::data_paths::DataPaths {
        actor_csv: "inputs/actors.csv".into(),
        article_csv: selected
            .explicit_file
            .as_ref()
            .unwrap()
            .display()
            .to_string(),
        classifier_dir: root
            .path()
            .join("saved article classifier")
            .display()
            .to_string(),
    };
    let settings = argos_osint_core::provider::SettingsFile {
        data_paths: saved.clone(),
        ..Default::default()
    };
    settings
        .save_to(&selected.home.join("config.toml"))
        .unwrap();
    let home = ClassifierHome::new(selected.home.clone());
    let accepted = home
        .start_training(ManualTrainRequest::new(
            selected.clone(),
            "custom-save".into(),
            SyntheticActivation::ExplicitOptIn,
        ))
        .await
        .unwrap();
    let job = finished(&home, &accepted.job_id).await;
    assert_eq!(
        job.state,
        TrainingJobState::Completed,
        "{:?}",
        job.diagnostic
    );
    let (first, second) = tokio::join!(home.pin(None), home.pin(None));
    let pinned = first.unwrap();
    let second = second.unwrap();
    assert!(
        std::sync::Arc::ptr_eq(&pinned.service, &second.service),
        "simultaneous cold pins share one restored runtime"
    );
    let snapshot = pinned.snapshot.clone();
    drop(pinned);
    drop(second);
    home.shutdown().await.unwrap();
    fs::remove_file(selected.explicit_file.as_ref().unwrap()).unwrap();
    assert!(saved
        .classifier_root(&selected.home)
        .join("active.json")
        .is_file());
    assert!(
        !selected.home.join("classifier").exists(),
        "configured saves never write the fallback store"
    );

    let reopened = ClassifierHome::new(selected.home.clone());
    let readiness = reopened.refresh_integrity(&selected).await.unwrap();
    assert!(
        readiness.can_infer,
        "saved model is independent of the training CSV"
    );
    assert!(!readiness.can_train);
    assert_eq!(reopened.pin(None).await.unwrap().snapshot, snapshot);
    assert_eq!(
        reopened.job_detail(&accepted.job_id).await.unwrap().state,
        TrainingJobState::Completed
    );
    reopened.shutdown().await.unwrap();

    let mut missing = saved;
    missing.classifier_dir = "new-empty-store".into();
    let settings = argos_osint_core::provider::SettingsFile {
        data_paths: missing,
        ..Default::default()
    };
    settings
        .save_to(&selected.home.join("config.toml"))
        .unwrap();
    let empty = ClassifierHome::new(selected.home.clone());
    assert!(
        !empty.refresh_integrity(&selected).await.unwrap().can_infer,
        "a changed save path never falls back to the previous model"
    );
    assert!(
        !selected.home.join("new-empty-store").exists(),
        "readiness is read-only"
    );
    empty.shutdown().await.unwrap();
}

#[cfg(feature = "classifier")]
#[tokio::test]
async fn manual_training_preserves_candidate_only_consent_and_idempotent_replay() {
    let _runtime_case = RUNTIME_CASES.lock().await;
    let root = TempDir::new().expect("isolated synthetic home");
    let selected = csv(root.path());
    let home = ClassifierHome::new(selected.home.clone());
    let operation = ManualTrainRequest::new(
        selected.clone(),
        "candidate-only".into(),
        SyntheticActivation::RetainPrior,
    );
    let accepted = home
        .start_training(operation.clone())
        .await
        .expect("accept owned training");
    let replay = home
        .start_training(operation.clone())
        .await
        .expect("running replay is idempotent");
    assert_eq!(replay.job_id, accepted.job_id);
    assert!(replay.reused);
    let job = finished(&home, &accepted.job_id).await;
    assert_eq!(
        job.state,
        TrainingJobState::Completed,
        "training diagnostic: {:?}",
        job.diagnostic
    );
    let outcome = job.outcome.expect("real candidate report");
    assert_eq!(outcome.state, "trained_not_activated");
    assert_eq!(outcome.provenance, "Synthetic baseline");
    assert!(outcome.active_model_id.is_none());
    assert!(!selected.home.join("classifier/active.json").exists());
    assert!(selected
        .home
        .join("classifier/models")
        .join(&outcome.candidate_model_id)
        .join("metrics.json")
        .is_file());
    fs::remove_file(selected.explicit_file.as_ref().expect("explicit fixture"))
        .expect("remove CSV after completed immutable request");
    let after = home
        .start_training(operation)
        .await
        .expect("terminal replay survives removed CSV");
    assert!(after.reused);
    let conflicting = ManualTrainRequest::new(
        selected,
        "candidate-only".into(),
        SyntheticActivation::ExplicitOptIn,
    );
    assert!(matches!(
        home.start_training(conflicting).await,
        Err(HomeError::RequestConflict)
    ));
    home.shutdown()
        .await
        .expect("owner joined before lease release");
}

#[cfg(feature = "classifier")]
#[tokio::test]
async fn explicit_activation_enables_pinning_and_evaluation_does_not_write_model_data() {
    let _runtime_case = RUNTIME_CASES.lock().await;
    let root = TempDir::new().expect("isolated synthetic home");
    let selected = csv(root.path());
    let home = ClassifierHome::new(selected.home.clone());
    let accepted = home
        .start_training(ManualTrainRequest::new(
            selected.clone(),
            "manual-seed".into(),
            SyntheticActivation::ExplicitOptIn,
        ))
        .await
        .expect("explicit seed request");
    let job = finished(&home, &accepted.job_id).await;
    assert_eq!(
        job.state,
        TrainingJobState::Completed,
        "training diagnostic: {:?}",
        job.diagnostic
    );
    assert_eq!(job.outcome.expect("outcome").state, "manual_seed_activated");
    let pinned = home.pin(None).await.expect("pin actual activated model");
    let pointer = selected.home.join("classifier/active.json");
    let before = fs::read(&pointer).expect("active pointer");
    let report = home
        .evaluate_file(EvaluateRequest::new(selected.clone()))
        .await
        .expect("real held-out family evaluation");
    assert_eq!(report.model.model_id, pinned.snapshot.model_id);
    assert_eq!(report.metrics.complete_families, 6);
    assert_eq!(report.dataset.split_families, [6, 6, 0, 6]);
    assert_eq!(
        fs::read(&pointer).expect("pointer after evaluation"),
        before
    );
    let mut missing = pinned.snapshot.clone();
    missing.model_id = "missing-retained-model".into();
    assert!(matches!(
        home.pin(Some(&missing)).await,
        Err(ClassifierError::InvalidArtifact(_)) | Err(ClassifierError::Io(_))
    ));
    home.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn cycle_preflight_requires_saved_revision_only_for_resumed_admission() {
    let _runtime_case = RUNTIME_CASES.lock().await;
    use argos_osint_core::classifier::readiness::{
        preflight_cycle, CyclePreflightError, CycleRequirement,
    };
    let root = TempDir::new().expect("isolated preflight home");
    let home_path = root.path().join("home");
    let home = ClassifierHome::new(home_path.clone());
    let permit = preflight_cycle(&home, CycleRequirement::PostAdmissionOnly, None)
        .await
        .expect("post-admission needs no model");
    assert_eq!(permit.requirement(), CycleRequirement::PostAdmissionOnly);
    assert!(permit.pinned().is_none());
    assert!(permit.into_pinned().is_none());
    for requirement in [
        CycleRequirement::AcquisitionAndAdmission,
        CycleRequirement::AdmissionOnly,
    ] {
        assert!(matches!(
            preflight_cycle(&home, requirement, None).await,
            Err(CyclePreflightError::MissingPinnedRevision)
        ));
    }
    assert!(matches!(
        preflight_cycle(&home, CycleRequirement::FreshAcquisition, None).await,
        Err(CyclePreflightError::Classifier { .. })
    ));
    assert!(
        !home_path.exists(),
        "blocked or post-only checks must not initialize storage/home"
    );
    home.shutdown().await.expect("shutdown");
}

#[test]
fn activation_view_uses_the_single_tagged_wire_contract() {
    use argos_osint_core::classifier::calibration::{ActivationOutcome, EvidenceFailure};
    use argos_osint_core::classifier::readiness::ActivationView;
    let view = ActivationView::from(ActivationOutcome::CandidateOnly {
        reason_code: "insufficient_quality_evidence".into(),
        reason: "Synthetic baseline requires explicit consent".into(),
        failures: vec![EvidenceFailure::SyntheticOptInRequired],
    });
    let json = serde_json::to_value(view).expect("wire serialization");
    assert_eq!(json["kind"], "candidate_only");
    assert_eq!(json["reason_code"], "insufficient_quality_evidence");
    assert!(json.get("candidate_only").is_none());
}

#[cfg(feature = "classifier")]
#[tokio::test]
async fn malformed_prior_does_not_prevent_candidate_results_or_reset_activation_lineage() {
    let _runtime_case = RUNTIME_CASES.lock().await;
    let root = TempDir::new().expect("isolated synthetic home");
    let selected = csv(root.path());
    let home = ClassifierHome::new(selected.home.clone());
    let pointer = selected.home.join("classifier/active.json");
    fs::create_dir_all(pointer.parent().unwrap()).unwrap();
    let invalid_pointer = b"{ deliberately malformed prior identity";
    fs::write(&pointer, invalid_pointer).unwrap();
    for (request_id, consent) in [
        ("candidate-invalid-prior", SyntheticActivation::RetainPrior),
        ("explicit-invalid-prior", SyntheticActivation::ExplicitOptIn),
    ] {
        let operation = ManualTrainRequest::new(selected.clone(), request_id.into(), consent);
        let accepted = home.start_training(operation.clone()).await.unwrap();
        let job = finished(&home, &accepted.job_id).await;
        // A terminal result is durable before writer teardown. Replay joins
        // the exact owner, proving it cannot refit or reset invalid lineage.
        assert!(home.start_training(operation).await.unwrap().reused);
        assert_eq!(
            job.state,
            TrainingJobState::Completed,
            "{:?}",
            job.diagnostic
        );
        let diagnostic = job
            .diagnostic
            .as_ref()
            .expect("prior state remains explicit");
        assert_eq!(diagnostic.code, "prior_classifier_invalid");
        let outcome = job.outcome.as_ref().expect("full report remains available");
        assert_eq!(outcome.state, "trained_not_activated");
        assert!(matches!(
            &outcome.activation,
            argos_osint_core::classifier::readiness::ActivationView::CandidateOnly { .. }
        ));
        assert!(selected
            .home
            .join("classifier/models")
            .join(&outcome.candidate_model_id)
            .join("manifest.json")
            .is_file());
        assert_eq!(
            fs::read(&pointer).unwrap(),
            invalid_pointer,
            "malformed lineage is neither overwritten nor treated as generation zero"
        );
        let stored: TrainingJob = serde_json::from_slice(
            &fs::read(
                selected
                    .home
                    .join("classifier/manual-jobs")
                    .join(&accepted.job_id)
                    .join("result.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            stored.outcome.unwrap().report.model_id.as_str(),
            outcome.candidate_model_id
        );
    }
    let state = home.refresh_integrity(&selected).await.unwrap();
    assert!(state.can_train);
    assert!(!state.can_infer);
    home.shutdown().await.unwrap();
}

#[cfg(feature = "classifier")]
#[tokio::test]
async fn dataset_content_changes_offer_retraining_without_invalidating_the_saved_model() {
    let _runtime_case = RUNTIME_CASES.lock().await;
    let root = TempDir::new().unwrap();
    let selected = csv(root.path());
    let home = ClassifierHome::new(selected.home.clone());
    let accepted = home
        .start_training(ManualTrainRequest::new(
            selected.clone(),
            "dataset-change-model".into(),
            SyntheticActivation::ExplicitOptIn,
        ))
        .await
        .unwrap();
    let job = finished(&home, &accepted.job_id).await;
    assert_eq!(
        job.state,
        TrainingJobState::Completed,
        "{:?}",
        job.diagnostic
    );
    let pointer = selected.home.join("classifier/active.json");
    let before = fs::read(&pointer).unwrap();
    let path = selected.explicit_file.as_ref().unwrap();
    let bytes = fs::read(path).unwrap();
    let initial = home.refresh_integrity(&selected).await.unwrap();
    assert_eq!(initial.dataset.change, "unchanged");
    assert!(!initial.retraining_available);
    assert!(initial.can_infer);
    fs::write(path, &bytes).unwrap();
    let identical = home.refresh_integrity(&selected).await.unwrap();
    assert_eq!(
        identical.dataset.change, "unchanged",
        "metadata changes alone must not trigger retraining"
    );
    let mut changed = bytes.clone();
    changed.extend_from_slice(b"\n");
    fs::write(path, changed).unwrap();
    let status = home.refresh_integrity(&selected).await.unwrap();
    assert_eq!(status.dataset.change, "changed");
    assert!(status.retraining_available);
    assert!(status.can_infer);
    assert_eq!(
        fs::read(&pointer).unwrap(),
        before,
        "a readiness prompt must not replace the active model"
    );
    fs::remove_file(path).unwrap();
    let missing = home.refresh_integrity(&selected).await.unwrap();
    assert!(
        missing.can_infer,
        "saved inference does not depend on retaining its training CSV"
    );
    assert!(
        !missing.retraining_available,
        "missing CSV is not a positive change detection"
    );
    assert_eq!(fs::read(pointer).unwrap(), before);
    home.shutdown().await.unwrap();
}
