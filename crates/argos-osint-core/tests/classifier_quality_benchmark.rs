//! Opt-in end-to-end training/development measurements on the versioned corpus.
//! Test is deliberately not read by this training/development benchmark.
#![cfg(feature = "classifier")]
#[path = "fixtures/classifier/support.rs"]
mod support;
use argos_osint_core::classifier::dataset::{self, DatasetRequest};
use argos_osint_core::classifier::training::{self, Split, SyntheticActivation, TrainRequest};
use argos_osint_core::classifier::{
    BridgeOperation, ClassifierError, ClassifierWorker, Provenance, WorkControl,
};
use std::time::Instant;

struct StepTrace<'a>(&'a mut dyn ClassifierWorker);
impl ClassifierWorker for StepTrace<'_> {
    fn call(
        &mut self,
        operation: BridgeOperation,
        request: &str,
        control: &WorkControl,
    ) -> Result<String, ClassifierError> {
        let started = Instant::now();
        let result = self.0.call(operation, request, control);
        if let Err(error) = &result {
            let value: serde_json::Value = serde_json::from_str(request).unwrap_or_default();
            eprintln!(
                "ARGOS_BENCH_FAILURE {}",
                serde_json::json!({"operation":format!("{operation:?}"),"phase":value["phase"],"elapsed_ms":started.elapsed().as_millis(),"error":error.to_string()})
            );
        }
        result
    }
    fn restore_completed_candidate(
        &mut self,
        model: &argos_osint_core::classifier::artifact::ModelId,
        payload: &str,
        control: &WorkControl,
    ) -> Result<(), ClassifierError> {
        let started = Instant::now();
        let result = self.0.restore_completed_candidate(model, payload, control);
        if let Err(error) = &result {
            eprintln!(
                "ARGOS_BENCH_FAILURE {}",
                serde_json::json!({"operation":"terminalRestore","elapsed_ms":started.elapsed().as_millis(),"payload_bytes":payload.len(),"error":error.to_string()})
            );
        }
        result
    }
}

#[test]
#[ignore = "explicit offline quality/resource benchmark; no real-world accuracy claim"]
fn distinct_corpus_training_and_independent_development() {
    let _serial = support::serial();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut request = DatasetRequest::default();
    request.cwd = root.clone();
    request.executable = root.join("argos");
    request.home = root.join(".agent-scratch/quality-unselected-home");
    request.explicit_file = Some(root.join("datasets/argos_atlas_strategic_domains_v2.csv"));
    let parsed = dataset::parse_csv(&dataset::resolve_dataset(&request).unwrap()).unwrap();
    let mut fixture = support::Fixture::new("quality-v2-benchmark");
    let request = TrainRequest {
        request_id: "quality-v2-benchmark".into(),
        model_id: fixture.model.clone(),
        provenance: Provenance::synthetic_baseline(
            &parsed.dataset_version,
            &parsed.summary.dataset_sha256,
        ),
        synthetic_activation: SyntheticActivation::RetainPrior,
    };
    let started = Instant::now();
    let candidate = training::train_candidate(
        &mut StepTrace(&mut *fixture.worker),
        &request,
        &parsed.corpus,
        &fixture.control,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(candidate.report.evaluation_split, Split::Development);
    assert_eq!(candidate.report.evaluation.complete_families, 165);
    assert!(candidate
        .report
        .fit_diagnostics
        .as_ref()
        .is_some_and(|fit| fit.complete_families == 230));
    let fit_ms = started.elapsed().as_millis();
    let inputs:Vec<_>=parsed.corpus.rows.iter().filter(|row|row.split==Split::Development).take(64).map(|row|serde_json::json!({"id":row.sample_id,"title":row.title,"description":row.description})).collect();
    let restore_started = Instant::now();
    let mut inference = support::Fixture::new("quality-v2-inference");
    inference.call(BridgeOperation::RestoreModel,serde_json::json!({"artifact":candidate.payload,"restored_model_id":"quality-v2-inference"}));
    let cold_restore_us = restore_started.elapsed().as_micros();
    let mut samples = Vec::new();
    for _ in 0..21 {
        let start = Instant::now();
        let reply = inference.call(
            BridgeOperation::ClassifyBatch,
            serde_json::json!({"items":inputs}),
        );
        assert_eq!(reply["items"].as_array().unwrap().len(), 64);
        samples.push(start.elapsed().as_micros());
    }
    let cold = samples.remove(0);
    samples.sort_unstable();
    let mut single_samples = Vec::new();
    for input in &inputs {
        let start = Instant::now();
        let reply = inference.call(
            BridgeOperation::ClassifyBatch,
            serde_json::json!({"items":[input]}),
        );
        assert_eq!(reply["items"].as_array().unwrap().len(), 1);
        single_samples.push(start.elapsed().as_micros());
    }
    let first_single = single_samples.remove(0);
    single_samples.sort_unstable();
    println!(
        "ARGOS_QUALITY_RESULT {}",
        serde_json::json!({"schema_version":"classifier-development-benchmark-v2","dataset_sha256":parsed.summary.dataset_sha256,"development_split_sha256":parsed.corpus.development_split_sha256,"payload_sha256":argos_osint_core::classifier::artifact::sha256_hex(candidate.payload.as_bytes()),"gate_set_sha256":argos_osint_core::classifier::artifact::sha256_hex(&serde_json::to_vec(&candidate.report.gate_set).unwrap()),"source_manifest_sha256":argos_osint_core::classifier::compiled_identity().unwrap().source_manifest_sha256,"bridge_sha256":argos_osint_core::classifier::compiled_identity().unwrap().bridge_sha256,"fit_ms":fit_ms,"cold_restore_us":cold_restore_us,"first_batch_us":cold,"warm_batch64_p50_us":samples[9],"warm_batch64_p95_us":samples[18],"first_single_us":first_single,"warm_single_samples":single_samples.len(),"warm_single_p50_us":single_samples[single_samples.len()/2],"warm_single_p95_us":single_samples[(single_samples.len()*95).div_ceil(100)-1],"report":candidate.report,"test_opened":false,"rss":null,"rss_reason":"external peak RSS sampler required; runtime reservations are not RSS"})
    );
}

#[test]
#[ignore = "explicit offline high-vocabulary fitting diagnostic"]
fn large_mixed_training_stage_budget() {
    use argos_osint_core::atlas_taxonomy::DomainId;
    use argos_osint_core::classifier::{learning, provider_labels, review};
    let _serial = support::serial();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut selected = DatasetRequest::default();
    selected.cwd = root.clone();
    selected.home = root.join(".agent-scratch/unselected-mixed-benchmark");
    selected.executable = root.join("argos");
    selected.explicit_file = Some(root.join("datasets/argos_atlas_strategic_domains_v2.csv"));
    let seed = dataset::parse_csv(&dataset::resolve_dataset(&selected).unwrap())
        .unwrap()
        .corpus;
    let reviews:Vec<_>=(0u64..400).map(|index| {
        let title=(0..40).map(|word|format!("w{:016x}",index*64+word)).collect::<Vec<_>>().join(" ");
        let observation=provider_labels::ProviderObservation {observation_id:format!("offline-review-{index}"),provider:provider_labels::ProviderFormat::NewsDataV1,endpoint:"fixture-v1".into(),publisher:Some("Fixture".into()),url:format!("https://fixture.invalid/story-{index}"),article_id:Some(format!("story-{index}")),published_at:None,fetched_at:String::new(),cycle_ids:Vec::new(),input_hash:"0".repeat(64),source_revision:"source-fixture".into(),title,description:"Fictional parliament discusses a military operation, central bank measures, hospital care, water pollution and laboratory research.".into(),language:Some("en".into()),raw_categories:serde_json::json!(["business","crime","politics","health","environment","science"]),raw_ai_tags:None,raw_metadata:serde_json::json!({})};
        let hash=argos_osint_core::classifier::artifact::sha256_hex(&serde_json::to_vec(&observation).unwrap());
        learning::ReviewedCapture {item:review::ReviewItem {observation,payload_sha256:hash,family_id:training::FamilyId::parse(&format!("offline-family-{index}")).unwrap(),split:if index<280 {Split::Train}else if index<340 {Split::Calibration}else if index<380 {Split::Development}else{Split::Test},revision:1,decision:review::ReviewDecision::Complete,domains:DomainId::ALL.to_vec(),review_note:"Isolated test annotation, not production evidence".into()},review_reference:"isolated-reviewer-fixture".into(),corpus_sha256:"a".repeat(64)}
    }).collect();
    let approval = provider_labels::MappingApproval {
        mapping_version: provider_labels::MAPPING_VERSION.into(),
        predicates_sha256: provider_labels::mapping_predicates_sha256(),
        review_reference: "isolated-mapping-fixture".into(),
        reviewed_corpus_sha256: "b".repeat(64),
        domains: DomainId::ALL.map(|domain| provider_labels::RuleValidation {
            domain,
            true_positive_families: 20,
            false_positive_families: 0,
        }),
    };
    let input = learning::capture(&seed, &reviews, &[], &approval).unwrap();
    let mut fixture = support::Fixture::new("large-mixed-benchmark");
    let request = TrainRequest {
        request_id: "large-mixed-benchmark".into(),
        model_id: fixture.model.clone(),
        provenance: input.provenance,
        synthetic_activation: SyntheticActivation::RetainPrior,
    };
    let started = Instant::now();
    let candidate = training::train_candidate(
        &mut StepTrace(&mut *fixture.worker),
        &request,
        &input.corpus,
        &fixture.control,
        &mut |_| {},
    )
    .unwrap();
    println!(
        "ARGOS_LARGE_FIT_RESULT {}",
        serde_json::json!({"fit_ms":started.elapsed().as_millis(),"payload_bytes":candidate.payload.len(),"reviewed_families":input.eligible_family_ids.len(),"test_opened":false})
    );
}
