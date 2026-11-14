//! Genuine bounded CSV/Natural training exclusively in the caller's temp home.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use argos_osint_core::atlas_taxonomy::DomainId;
use argos_osint_core::classifier::artifact::ArtifactStore;
use argos_osint_core::classifier::candidate_activation;
use argos_osint_core::classifier::dataset;
use argos_osint_core::classifier::dataset::DatasetRequest;
use argos_osint_core::classifier::dataset::ParsedDataset;
use argos_osint_core::classifier::training;
use argos_osint_core::classifier::training::SyntheticActivation;
use argos_osint_core::classifier::training::TrainRequest;
use argos_osint_core::classifier::ModelId;
use argos_osint_core::classifier::Provenance;
use argos_osint_core::classifier::WorkControl;
use fleetworks_contracts::classifier_activation::CandidatePreview;
use fleetworks_contracts::readiness::DatasetKind;
use fleetworks_contracts::readiness::DatasetRef;

pub fn prepare(root: &Path) -> (ParsedDataset, DatasetRef) {
    let mut csv="sample_id,title,description,domain_labels,primary_domain,secondary_domain,subcategories,source_kind,difficulty,dataset_version,annotation_notes,split,scenario_group,language,synthetic,labels_complete,expected_admission,rejection_reason\n".to_owned();
    for split in ["train", "calibration"] {
        for group in 0..3 {
            for repetition in 0..2 {
                let a = DomainId::ALL[group * 2].as_str();
                let b = DomainId::ALL[group * 2 + 1].as_str();
                let id = format!("{split}-{group}-{repetition}");
                csv.push_str(&format!("{id},Synthetic {a} {b} bulletin {repetition},Fictional {a} and {b} report,{a}|{b},{a},{b},,synthetic,cross_domain,synthetic-development-v1,,{split},{id},en,true,true,admit,\n"));
            }
        }
    }
    let file = root.join("training.csv");
    fs::write(&file, &csv).expect("synthetic dataset");
    let mut selection = DatasetRequest::default();
    selection.explicit_file = Some(file);
    selection.cwd = root.into();
    selection.home = root.into();
    selection.executable = root.join("fixture-program");
    let parsed =
        dataset::parse_csv(&dataset::resolve_dataset(&selection).expect("selected actual file"))
            .expect("validated actual CSV");
    let reference = DatasetRef::new(
        "disposable-training-data".into(),
        DatasetKind::ClassifierTraining,
        parsed.summary.dataset_sha256.clone(),
        csv.len() as u64,
    )
    .expect("real byte identity");
    (parsed, reference)
}
pub fn train(root: &Path, parsed: &ParsedDataset, source_job: &str) -> CandidatePreview {
    let store = ArtifactStore::new(root).expect("temporary artifact store");
    let lease = Arc::new(store.try_writer().expect("training writer"));
    let model = ModelId::parse("disposable-natural-candidate").expect("model ID");
    let control = WorkControl::new();
    let mut worker =
        argos_osint_core::classifier::start_candidate_worker(&model, lease.clone(), &control)
            .expect("real Natural worker");
    let request = TrainRequest {
        request_id: source_job.into(),
        model_id: model.clone(),
        provenance: Provenance::synthetic_baseline(
            &parsed.dataset_version,
            &parsed.corpus.dataset_sha256,
        ),
        synthetic_activation: SyntheticActivation::RetainPrior,
    };
    let candidate = training::train_candidate(
        worker.as_mut(),
        &request,
        &parsed.corpus,
        &control,
        &mut |_| {},
    )
    .expect("genuine candidate fit");
    let manifest = training::candidate_manifest(&parsed.corpus, &request, &candidate)
        .expect("genuine manifest");
    store
        .write_candidate(&lease, &manifest, &candidate.payload, &candidate.report)
        .expect("immutable candidate");
    drop(worker);
    drop(lease);
    assert!(store
        .read_active()
        .expect("no automatic activation")
        .is_none());
    candidate_activation::preview(&store, source_job, model.as_str())
        .expect("actual immutable candidate preview")
}
