//! Shared runtime recipe adapter. Saved models always come from the real worker.
#![allow(dead_code)]
use argos_osint_core::classifier::artifact::ArtifactStore;
use argos_osint_core::classifier::{BridgeOperation, ClassifierWorker, ModelId, WorkControl};
use serde_json::Value;
use std::sync::{Arc, Mutex, MutexGuard};
static WORKERS: Mutex<()> = Mutex::new(());
pub fn serial() -> MutexGuard<'static, ()> {
    // This test-only lock serializes runtime-budget users; a failed assertion must
    // not prevent unrelated cases from reporting their independent results.
    WORKERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
pub struct Fixture {
    pub worker: Box<dyn ClassifierWorker>,
    pub home: tempfile::TempDir,
    pub control: WorkControl,
    pub model: ModelId,
    pub lease: Arc<argos_osint_core::classifier::artifact::WriterLease>,
    sequence: u64,
}
impl Fixture {
    pub fn new(model: &str) -> Self {
        let home = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(home.path()).unwrap();
        let lease = Arc::new(store.try_writer().unwrap());
        let model = ModelId::parse(model).unwrap();
        let control = WorkControl::new();
        let worker =
            argos_osint_core::classifier::start_candidate_worker(&model, lease.clone(), &control)
                .unwrap();
        Self {
            worker,
            home,
            control,
            model,
            lease,
            sequence: 0,
        }
    }
    pub fn call(&mut self, operation: BridgeOperation, mut request: Value) -> Value {
        request["version"] = "1".into();
        self.sequence += 1;
        request["request_id"] = format!("fixture-call-{}", self.sequence).into();
        request["model_id"] = self.model.as_str().into();
        let response = self
            .worker
            .call(operation, &request.to_string(), &self.control)
            .expect("bridge call");
        let value: Value = serde_json::from_str(&response).expect("bridge JSON");
        assert!(
            value.get("error").is_none(),
            "bridge returned error: {value}"
        );
        value
    }
    pub fn fit(&mut self, rows: Value) {
        self.call(
            BridgeOperation::TrainBatch,
            serde_json::json!({"rows":rows}),
        );
        loop {
            let response = self.call(
                BridgeOperation::FinishTraining,
                serde_json::json!({"max_documents_per_step":16}),
            );
            if response["done"].as_bool().unwrap() {
                break;
            }
        }
    }
    pub fn payload(&mut self) -> Value {
        self.call(BridgeOperation::SerializeModel, serde_json::json!({}))
    }
    pub fn classify(&mut self, text: &str) -> Value {
        self.call(
            BridgeOperation::ClassifyBatch,
            serde_json::json!({"items":[{"id":"query","title":text,"description":""}]}),
        )["items"][0]["results"]
            .clone()
    }
}
pub fn recipe() -> Value {
    serde_json::from_str(include_str!("six-model.json")).unwrap()
}
pub fn trained() -> Fixture {
    let mut fixture = Fixture::new("fixture-m1");
    fixture.fit(recipe()["rows"].clone());
    fixture
}

pub struct Candidate {
    pub fixture: Fixture,
    pub store: ArtifactStore,
    pub manifest: argos_osint_core::classifier::artifact::ModelManifest,
    pub report: argos_osint_core::classifier::training::TrainingReport,
    pub payload: String,
}
impl Candidate {
    pub fn from_fixture(mut fixture: Fixture) -> Self {
        use argos_osint_core::atlas_taxonomy::{DomainId, TAXONOMY_VERSION};
        use argos_osint_core::classifier::artifact::{self, ActivationReason};
        use argos_osint_core::classifier::calibration::{
            self, DomainCalibration, GateSet, CALIBRATION_VERSION,
        };
        use argos_osint_core::classifier::{self, Provenance};
        let payload_value = fixture.payload();
        let payload = payload_value.to_string();
        let provenance = Provenance::synthetic_baseline("fixture-v1", "0".repeat(64));
        let gates = GateSet {
            version: CALIBRATION_VERSION.into(),
            domains: DomainId::ALL.map(|domain| DomainCalibration {
                sigmoid: None,
                domain,
                knots: vec![],
                threshold: 2.0,
                min_informative_features: 1,
            }),
        };
        let evaluation = calibration::evaluate(&[], &gates).unwrap();
        let class_counts:Vec<Value>=DomainId::ALL.iter().map(|domain|serde_json::json!({"domain":domain.as_str(),
            "present":payload_value["domains"][domain.as_str()]["class_totals"]["present"].as_u64().unwrap()-1,
            "absent":payload_value["domains"][domain.as_str()]["class_totals"]["absent"].as_u64().unwrap()-1})).collect();
        let family_counts: Vec<Value> = DomainId::ALL
            .iter()
            .map(|domain| serde_json::json!({"domain":domain.as_str(),"accepted_families":0}))
            .collect();
        let report=serde_json::from_value(serde_json::json!({
            "schema_version":argos_osint_core::classifier::training::TRAINING_REPORT_SCHEMA_VERSION,"request_id":"fixture-activation","model_id":fixture.model,
            "provenance":provenance,"class_counts":class_counts,"family_counts":family_counts,
            "calibration":gates,"evaluation":evaluation,"domain_decisions":evaluation.domains,
        })).unwrap();
        let metrics = serde_json::to_value(&report).unwrap();
        let report_bytes = format!("{}\n", serde_json::to_string_pretty(&report).unwrap());
        let identity = classifier::compiled_identity().unwrap();
        let manifest=serde_json::from_value(serde_json::json!({
            "schema_version":artifact::MANIFEST_SCHEMA_VERSION,"activation_generation":0,"model_id":fixture.model,
            "previous_model_id":null,"activated_at":"","manifest_sha256":"",
            "model_sha256":artifact::sha256_hex(payload.as_bytes()),"report_sha256":artifact::sha256_hex(report_bytes.as_bytes()),
            "dataset_sha256":"0".repeat(64),"train_split_sha256":"1".repeat(64),"calibration_split_sha256":"2".repeat(64),
            "development_split_sha256":"7".repeat(64),"test_split_sha256":"3".repeat(64),"family_assignment_sha256":"4".repeat(64),
            "taxonomy_version":TAXONOMY_VERSION,"domains":DomainId::ALL,"algorithm_version":classifier::ALGORITHM_VERSION,
            "preprocessing_version":classifier::PREPROCESSING_VERSION,"feature_schema_version":artifact::FEATURE_SCHEMA_VERSION,
            "admission_policy_version":classifier::ADMISSION_POLICY_VERSION,"natural_version":identity.natural_version,
            "apparatus_version":identity.apparatus_version,"bundle_sha256":identity.bundle_sha256,"bridge_version":identity.bridge_version,
            "bridge_sha256":identity.bridge_sha256,"adapter_sha256":identity.adapter_sha256,"source_manifest_sha256":identity.source_manifest_sha256,
            "smoothing":1.0,"thresholds":DomainId::ALL.map(|domain|serde_json::json!({"domain":domain,"threshold":2.0})),
            "class_counts":class_counts,"family_counts":family_counts,"metrics":metrics,"provenance":provenance,
            "activation_reason":ActivationReason::ManualSeed,
        })).unwrap();
        let store = ArtifactStore::new(fixture.home.path()).unwrap();
        Self {
            fixture,
            store,
            manifest,
            report,
            payload,
        }
    }
    pub fn request(
        &self,
        activation: argos_osint_core::classifier::training::SyntheticActivation,
    ) -> argos_osint_core::classifier::training::TrainRequest {
        argos_osint_core::classifier::training::TrainRequest {
            request_id: self.report.request_id.clone(),
            model_id: self.fixture.model.clone(),
            provenance: self.report.provenance.clone(),
            synthetic_activation: activation,
        }
    }
    pub fn write(&self) -> argos_osint_core::classifier::artifact::ValidatedArtifact {
        self.store
            .write_candidate(
                &self.fixture.lease,
                &self.manifest,
                &self.payload,
                &self.report,
            )
            .unwrap()
    }
}

/// Simulates a fully committed previous feature-schema artifact, with genuine
/// learned Natural weights and consistent payload/manifest/history/index hashes.
/// This is a migration fixture, not an assertion of a previous build's authenticity.
pub fn install_legacy_prior(
    candidate: &Candidate,
    active: &argos_osint_core::classifier::artifact::ModelManifest,
) -> argos_osint_core::classifier::artifact::ModelManifest {
    use argos_osint_core::classifier::artifact::{self, ActivatedModelIndex, ActivationRecord};
    use std::fs;
    fn wire(value: &impl serde::Serialize) -> Vec<u8> {
        let mut bytes = serde_json::to_vec_pretty(value).unwrap();
        bytes.push(b'\n');
        bytes
    }
    let mut payload: Value = serde_json::from_str(&candidate.payload).unwrap();
    payload["feature_schema_version"] = "classifier-features-v1".into();
    for model in payload["domains"].as_object_mut().unwrap().values_mut() {
        model.as_object_mut().unwrap().remove("feature_index");
    }
    let payload = payload.to_string();
    let mut immutable = candidate.manifest.clone();
    immutable.feature_schema_version = "classifier-features-v1".into();
    immutable.model_sha256 = artifact::sha256_hex(payload.as_bytes());
    let immutable_bytes = wire(&immutable);
    let directory = candidate.store.model_dir(&immutable.model_id);
    fs::write(directory.join(artifact::MODEL_PAYLOAD_FILE), payload).unwrap();
    fs::write(
        directory.join(artifact::MODEL_MANIFEST_FILE),
        &immutable_bytes,
    )
    .unwrap();
    let mut legacy = active.clone();
    legacy.feature_schema_version = immutable.feature_schema_version;
    legacy.model_sha256 = immutable.model_sha256;
    legacy.manifest_sha256 = artifact::sha256_hex(&immutable_bytes);
    fs::write(candidate.store.active_manifest_path(), wire(&legacy)).unwrap();
    let record_path = candidate
        .store
        .root()
        .join("activations")
        .join(format!("{}.json", legacy.activation_generation));
    let mut record: ActivationRecord =
        serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    record.model_sha256 = legacy.model_sha256.clone();
    record.immutable_manifest_sha256 = legacy.manifest_sha256.clone();
    let record_bytes = wire(&record);
    fs::write(record_path, &record_bytes).unwrap();
    let index_path = candidate
        .store
        .root()
        .join("activated-models")
        .join(format!("{}.json", legacy.model_id));
    let mut index: ActivatedModelIndex =
        serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
    index.activation_record_sha256 = artifact::sha256_hex(&record_bytes);
    fs::write(index_path, wire(&index)).unwrap();
    legacy
}
