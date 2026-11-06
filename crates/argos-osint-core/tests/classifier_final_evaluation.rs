//! Explicit frozen Test regression. The initial Test result predates a logical
//! neutral-phrase fix; repeats are not fresh blind quality evidence.
#![cfg(feature = "classifier")]
#[path = "fixtures/classifier/support.rs"]
mod support;
use argos_osint_core::classifier::{
    self, artifact, calibration, dataset, training, BridgeOperation, Provenance,
};
use serde_json::{json, Value};
#[test]
#[ignore = "explicit synthetic Test regression; requires matching pipeline-freeze.json"]
fn frozen_pipeline_evaluates_test_without_refitting_any_gate() {
    let _serial = support::serial();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let freeze: Value = serde_json::from_slice(
        &std::fs::read(
            root.join(".planning/2026-11-06-classifier-quality-v2/pipeline-freeze.json"),
        )
        .expect("freeze the selected Development pipeline before opening Test"),
    )
    .unwrap();
    let identity = classifier::compiled_identity().unwrap();
    assert_eq!(
        freeze["source_manifest_sha256"],
        identity.source_manifest_sha256
    );
    assert_eq!(freeze["bridge_sha256"], identity.bridge_sha256);
    assert_eq!(
        freeze["policy_version"],
        classifier::ADMISSION_POLICY_VERSION
    );
    let mut selected = dataset::DatasetRequest::default();
    selected.cwd = root.clone();
    selected.home = root.join(".agent-scratch/unselected-final-home");
    selected.executable = root.join("argos");
    selected.explicit_file = Some(root.join("datasets/argos_atlas_strategic_domains_v2.csv"));
    let parsed = dataset::parse_csv(&dataset::resolve_dataset(&selected).unwrap()).unwrap();
    assert_eq!(freeze["dataset_sha256"], parsed.summary.dataset_sha256);
    let mut fixture = support::Fixture::new("frozen-final-test");
    let candidate = training::train_candidate(
        &mut *fixture.worker,
        &training::TrainRequest {
            request_id: "frozen-final-test".into(),
            model_id: fixture.model.clone(),
            provenance: Provenance::synthetic_baseline(
                &parsed.dataset_version,
                &parsed.summary.dataset_sha256,
            ),
            synthetic_activation: training::SyntheticActivation::RetainPrior,
        },
        &parsed.corpus,
        &fixture.control,
        &mut |_| {},
    )
    .unwrap();
    let gate_hash = artifact::sha256_hex(&serde_json::to_vec(&candidate.report.gate_set).unwrap());
    assert_eq!(
        freeze["gate_set_sha256"], gate_hash,
        "Development-selected gates must be frozen before Test"
    );
    let rows: Vec<_> = parsed
        .corpus
        .rows
        .iter()
        .filter(|row| row.split == training::Split::Test)
        .collect();
    assert_eq!(rows.len(), 185);
    let mut inference = support::Fixture::new("frozen-final-inference");
    inference.call(
        BridgeOperation::RestoreModel,
        json!({"artifact":candidate.payload,"restored_model_id":"frozen-final-inference"}),
    );
    let mut scored = Vec::new();
    for batch in rows.chunks(classifier::INFERENCE_BATCH_SIZE) {
        let inputs: Vec<_> = batch
            .iter()
            .map(|row| json!({"id":row.sample_id,"title":row.title,"description":row.description}))
            .collect();
        let reply = inference.call(BridgeOperation::ClassifyBatch, json!({"items":inputs}));
        let predictions = reply["items"].as_array().unwrap();
        assert_eq!(predictions.len(), batch.len());
        for (row, prediction) in batch.iter().zip(predictions) {
            scored.push(calibration::ScoredFamily {
                family_id: row.family_id.clone(),
                representative_id: row.sample_id.clone(),
                split: row.split,
                provenance: row.provenance.clone(),
                membership: row.membership,
                raw: serde_json::from_value(prediction["results"].clone()).unwrap(),
            });
        }
    }
    let result = json!({"schema_version":"classifier-frozen-test-regression-v2","evaluation_note":"Test was previously evaluated before the neutral-phrase bug fix; this is regression evidence, not a fresh blind result","provenance":"Synthetic baseline; no reviewed-real production approval","dataset_sha256":parsed.summary.dataset_sha256,"test_split_sha256":parsed.corpus.test_split_sha256,"payload_sha256":artifact::sha256_hex(candidate.payload.as_bytes()),"gate_set_sha256":gate_hash,"gate_fit_split":"calibration","pipeline_selection_split":"development","evaluation_split":"test","metrics":calibration::evaluate(&scored,&candidate.report.gate_set).unwrap(),"brier":calibration::brier_score(&scored,&candidate.report.gate_set).unwrap(),"activation":"none"});
    std::fs::write(
        root.join(".agent-scratch/classifier-v2-test-regression.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
    println!("ARGOS_FINAL_TEST_RESULT {}", result);
}
