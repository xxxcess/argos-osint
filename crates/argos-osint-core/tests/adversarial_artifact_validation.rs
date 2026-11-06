//! Adversarial empirical stress harness for Argos Classifier Artifact Validation & Error Handling (R1-R5).
//!
//! Verifies:
//! 1. R1: `ClassificationReceipt` serde faithfully serializes and deserializes `receipt_schema_version`.
//! 2. R2: Per-domain label sets and support: rejects missing class, 3 classes, 0 or <= 1 support, non-disjoint example ID sets, duplicate IDs.
//! 3. R3: Active manifest validation: rejects empty checksum, altered immutable fields with valid checksum, generation <= 0, empty timestamp.
//! 4. R4: Canonical domains validation: rejects 7th domain (e.g. "unk"), missing domain, non-canonical key, non-object domain.
//! 5. R5: Bounded file reading: `read_bounded` fails with `ModelTooLarge` when files exceed limit without reading whole file into memory.
//! 6. Restore smoke check: corrupted payload fails restore-before-activate, and prior active artifact remains intact (atomic abort).
//! 7. Writer lease: concurrent `try_writer` returns `ClassifierError::Busy`.

#![cfg(feature = "classifier")]
#[path = "fixtures/classifier/support.rs"]
mod support;

use std::fs::{self, File};
use std::io::Write;
use tempfile::TempDir;

use argos_osint_core::atlas_taxonomy::{DomainId, TAXONOMY_VERSION};
use argos_osint_core::classifier::artifact::{
    sha256_hex, validate_artifact, ActivationAuthorization, ActivationReason, ArtifactStore,
    ModelManifest, WriterLease, COMPILED_BUNDLE_SHA256, FEATURE_SCHEMA_VERSION,
    MANIFEST_SCHEMA_VERSION, MAX_MANIFEST_BYTES, MODEL_MANIFEST_FILE, MODEL_PAYLOAD_FILE,
    MODEL_REPORT_FILE, PAYLOAD_SCHEMA_VERSION,
};

use argos_osint_core::classifier::{
    ClassificationReceipt, ClassifierError, ModelId, Provenance, ADMISSION_POLICY_VERSION,
    ALGORITHM_VERSION, BRIDGE_VERSION, PREPROCESSING_VERSION, RECEIPT_SCHEMA_VERSION,
};

fn create_valid_payload_json() -> serde_json::Value {
    let mut domains = serde_json::Map::new();
    for domain in DomainId::ALL {
        let domain_model = serde_json::json!({
            "feature_order": ["polici", "govern", "trade", "secur"],
            "feature_index": [{"token":"polici","index":0},{"token":"govern","index":1},{"token":"trade","index":2},{"token":"secur","index":3}],
            "class_totals": {
                "absent": 20,
                "present": 20
            },
            "class_features": {
                "present": { "0": 5, "1": 6, "2": 2, "3": 3 },
                "absent": { "0": 2, "1": 2, "2": 4, "3": 3 }
            },
            "total_examples": 39,
            "smoothing": 1.0,
            "example_ids": {
                "present": ["ex_p1", "ex_p2"],
                "absent": ["ex_a1", "ex_a2"]
            }
        });
        domains.insert(domain.as_str().to_string(), domain_model);
    }

    serde_json::json!({
        "schema_version": PAYLOAD_SCHEMA_VERSION,
        "feature_schema_version": FEATURE_SCHEMA_VERSION,
        "bridge_version": BRIDGE_VERSION,
        "taxonomy_version": TAXONOMY_VERSION,
        "algorithm_version": ALGORITHM_VERSION,
        "preprocessing_version": PREPROCESSING_VERSION,
        "domains": domains,
    })
}

fn create_valid_manifest(model_id_str: &str, payload_str: &str, report_str: &str) -> ModelManifest {
    let thresholds: Vec<serde_json::Value> = DomainId::ALL
        .iter()
        .map(|d| {
            serde_json::json!({
                "domain": d.as_str(),
                "threshold": 0.5,
            })
        })
        .collect();
    let class_counts: Vec<serde_json::Value> = DomainId::ALL
        .iter()
        .map(|d| {
            serde_json::json!({
                "domain": d.as_str(),
                "present": 19,
                "absent": 19,
            })
        })
        .collect();
    let family_counts: Vec<serde_json::Value> = DomainId::ALL
        .iter()
        .map(|d| {
            serde_json::json!({
                "domain": d.as_str(),
                "accepted_families": 2,
            })
        })
        .collect();

    let identity = argos_osint_core::classifier::compiled_identity().unwrap();
    let manifest_json = serde_json::json!({
        "schema_version": MANIFEST_SCHEMA_VERSION,
        "activation_generation": 0,
        "model_id": model_id_str,
        "previous_model_id": null,
        "activated_at": "",
        "model_sha256": sha256_hex(payload_str.as_bytes()),
        "manifest_sha256": "",
        "report_sha256": sha256_hex(report_str.as_bytes()),
        "dataset_sha256": "a".repeat(64),
        "train_split_sha256": "b".repeat(64),
        "calibration_split_sha256": "c".repeat(64),
        "test_split_sha256": "d".repeat(64),
        "family_assignment_sha256": "e".repeat(64),
        "taxonomy_version": TAXONOMY_VERSION,
        "domains": DomainId::ALL.to_vec(),
        "algorithm_version": ALGORITHM_VERSION,
        "preprocessing_version": PREPROCESSING_VERSION,
        "feature_schema_version": FEATURE_SCHEMA_VERSION,
        "admission_policy_version": ADMISSION_POLICY_VERSION,
        "natural_version": "8.2.6",
        "apparatus_version": "0.0.10",
        "bundle_sha256": COMPILED_BUNDLE_SHA256,
        "adapter_sha256": identity.adapter_sha256,
        "bridge_sha256": identity.bridge_sha256,
        "bridge_version": identity.bridge_version,
        "source_manifest_sha256": identity.source_manifest_sha256,
        "smoothing": 1.0,
        "thresholds": thresholds,
        "class_counts": class_counts,
        "family_counts": family_counts,
        "metrics": { "accuracy": 0.95 },
        "provenance": serde_json::to_value(Provenance::synthetic_baseline("seed", "a".repeat(64))).unwrap(),
        "activation_reason": serde_json::to_value(ActivationReason::ManualSeed).unwrap(),
    });

    let mut manifest: ModelManifest =
        serde_json::from_value(manifest_json).expect("valid manifest deserialization");
    let report = report_for(&manifest);
    manifest.metrics = serde_json::to_value(&report).unwrap();
    manifest.report_sha256 =
        sha256_hex(format!("{}\n", serde_json::to_string_pretty(&report).unwrap()).as_bytes());
    manifest
}

fn write_model_files(
    store: &ArtifactStore,
    manifest: &ModelManifest,
    payload_str: &str,
    _report_str: &str,
) {
    let dir = store.model_dir(&manifest.model_id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(MODEL_MANIFEST_FILE),
        format!("{}\n", serde_json::to_string_pretty(manifest).unwrap()),
    )
    .unwrap();
    fs::write(dir.join(MODEL_PAYLOAD_FILE), payload_str).unwrap();
    fs::write(
        dir.join(MODEL_REPORT_FILE),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&report_for(manifest)).unwrap()
        ),
    )
    .unwrap();
}

#[test]
fn test_adversarial_r1_receipt_schema_version_serde() {
    let _serial = support::serial();
    let receipt = ClassificationReceipt::new("occ-1", "hash-1", "model-1");
    assert_eq!(
        receipt.receipt_schema_version, RECEIPT_SCHEMA_VERSION,
        "ClassificationReceipt::new must populate receipt_schema_version with RECEIPT_SCHEMA_VERSION"
    );

    let serialized = serde_json::to_string(&receipt).expect("serialization succeeds");
    assert!(
        serialized.contains(&format!(
            r#""receipt_schema_version":"{RECEIPT_SCHEMA_VERSION}""#
        )),
        "Serialized receipt must include receipt_schema_version"
    );

    let deserialized: ClassificationReceipt =
        serde_json::from_str(&serialized).expect("deserialization succeeds");
    assert_eq!(
        deserialized.receipt_schema_version, RECEIPT_SCHEMA_VERSION,
        "Deserialized receipt must preserve receipt_schema_version faithfully"
    );

    let altered_json = serialized.replace(RECEIPT_SCHEMA_VERSION, "classifier-receipt-custom-v99");
    assert!(serde_json::from_str::<ClassificationReceipt>(&altered_json)
        .unwrap_err()
        .to_string()
        .contains("requires migration"));
}

#[test]
fn test_adversarial_r2_class_sets_and_training_support() {
    let _serial = support::serial();
    let valid_payload = create_valid_payload_json();
    let valid_payload_str = valid_payload.to_string();
    let report_str = "{}";
    let manifest = create_valid_manifest("model-r2", &valid_payload_str, report_str);

    // Baseline valid artifact passes validate_artifact
    assert!(
        validate_artifact(&manifest, &valid_payload_str, COMPILED_BUNDLE_SHA256).is_ok(),
        "Baseline valid artifact should pass validation"
    );

    // 1. Missing "absent" class (only "present")
    {
        let mut bad_payload = valid_payload.clone();
        bad_payload["domains"]["governance"]["class_totals"] = serde_json::json!({
            "present": 20
        });
        let bad_str = bad_payload.to_string();
        let mut m = manifest.clone();
        m.model_sha256 = sha256_hex(bad_str.as_bytes());
        match validate_artifact(&m, &bad_str, COMPILED_BUNDLE_SHA256) {
            Err(ClassifierError::InvalidArtifact(msg)) => {
                assert!(msg.contains("invalid class set") && msg.contains("governance"));
            }
            other => panic!(
                "Expected InvalidArtifact for missing 'absent', got: {:?}",
                other
            ),
        }
    }

    // 2. Missing "present" class (only "absent")
    {
        let mut bad_payload = valid_payload.clone();
        bad_payload["domains"]["security"]["class_totals"] = serde_json::json!({
            "absent": 20
        });
        let bad_str = bad_payload.to_string();
        let mut m = manifest.clone();
        m.model_sha256 = sha256_hex(bad_str.as_bytes());
        match validate_artifact(&m, &bad_str, COMPILED_BUNDLE_SHA256) {
            Err(ClassifierError::InvalidArtifact(msg)) => {
                assert!(msg.contains("invalid class set") && msg.contains("security"));
            }
            other => panic!(
                "Expected InvalidArtifact for missing 'present', got: {:?}",
                other
            ),
        }
    }

    // 3. 3 classes (e.g. {"absent": 20, "present": 20, "unknown": 5})
    {
        let mut bad_payload = valid_payload.clone();
        bad_payload["domains"]["economic"]["class_totals"] = serde_json::json!({
            "absent": 20,
            "present": 20,
            "unknown": 5
        });
        let bad_str = bad_payload.to_string();
        let mut m = manifest.clone();
        m.model_sha256 = sha256_hex(bad_str.as_bytes());
        match validate_artifact(&m, &bad_str, COMPILED_BUNDLE_SHA256) {
            Err(ClassifierError::InvalidArtifact(msg)) => {
                assert!(msg.contains("invalid class set") && msg.contains("economic"));
            }
            other => panic!("Expected InvalidArtifact for 3 classes, got: {:?}", other),
        }
    }

    // 4. Low/zero support for present (count <= 1)
    {
        let mut bad_payload = valid_payload.clone();
        bad_payload["domains"]["human"]["class_totals"] = serde_json::json!({
            "absent": 20,
            "present": 1
        });
        let bad_str = bad_payload.to_string();
        let mut m = manifest.clone();
        m.model_sha256 = sha256_hex(bad_str.as_bytes());
        match validate_artifact(&m, &bad_str, COMPILED_BUNDLE_SHA256) {
            Err(ClassifierError::InvalidArtifact(msg)) => {
                assert!(
                    msg.contains("must have nonzero training support") && msg.contains("human")
                );
            }
            other => panic!(
                "Expected InvalidArtifact for support <= 1, got: {:?}",
                other
            ),
        }
    }

    // 5. Zero support for absent (count = 0)
    {
        let mut bad_payload = valid_payload.clone();
        bad_payload["domains"]["environmental"]["class_totals"] = serde_json::json!({
            "absent": 0,
            "present": 20
        });
        let bad_str = bad_payload.to_string();
        let mut m = manifest.clone();
        m.model_sha256 = sha256_hex(bad_str.as_bytes());
        match validate_artifact(&m, &bad_str, COMPILED_BUNDLE_SHA256) {
            Err(ClassifierError::InvalidArtifact(msg)) => {
                assert!(
                    msg.contains("must have nonzero training support")
                        && msg.contains("environmental")
                );
            }
            other => panic!(
                "Expected InvalidArtifact for zero absent support, got: {:?}",
                other
            ),
        }
    }

    // 6. Non-disjoint example IDs (id_shared present in both present and absent sets)
    {
        let mut bad_payload = valid_payload.clone();
        bad_payload["domains"]["scientific"]["example_ids"] = serde_json::json!({
            "present": ["id_shared", "id_p2"],
            "absent": ["id_shared", "id_a2"]
        });
        let bad_str = bad_payload.to_string();
        let mut m = manifest.clone();
        m.model_sha256 = sha256_hex(bad_str.as_bytes());
        match validate_artifact(&m, &bad_str, COMPILED_BUNDLE_SHA256) {
            Err(ClassifierError::InvalidArtifact(msg)) => {
                assert!(
                    msg.contains("belongs to both present and absent sets")
                        && msg.contains("id_shared")
                );
            }
            other => panic!(
                "Expected InvalidArtifact for non-disjoint example IDs, got: {:?}",
                other
            ),
        }
    }

    // 7. Empty example IDs list in present
    {
        let mut bad_payload = valid_payload.clone();
        bad_payload["domains"]["governance"]["example_ids"] = serde_json::json!({
            "present": [],
            "absent": ["id_a1"]
        });
        let bad_str = bad_payload.to_string();
        let mut m = manifest.clone();
        m.model_sha256 = sha256_hex(bad_str.as_bytes());
        match validate_artifact(&m, &bad_str, COMPILED_BUNDLE_SHA256) {
            Err(ClassifierError::InvalidArtifact(msg)) => {
                assert!(msg.contains("retained present example IDs must not be empty"));
            }
            other => panic!(
                "Expected InvalidArtifact for empty present IDs, got: {:?}",
                other
            ),
        }
    }

    // 8. Duplicate example IDs in present set
    {
        let mut bad_payload = valid_payload.clone();
        bad_payload["domains"]["governance"]["example_ids"] = serde_json::json!({
            "present": ["dup_id", "dup_id"],
            "absent": ["id_a1"]
        });
        let bad_str = bad_payload.to_string();
        let mut m = manifest.clone();
        m.model_sha256 = sha256_hex(bad_str.as_bytes());
        match validate_artifact(&m, &bad_str, COMPILED_BUNDLE_SHA256) {
            Err(ClassifierError::InvalidArtifact(msg)) => {
                assert!(msg.contains("duplicate present example ID"));
            }
            other => panic!(
                "Expected InvalidArtifact for duplicate example ID, got: {:?}",
                other
            ),
        }
    }
}

#[test]
fn test_adversarial_r3_active_manifest_validation() {
    let _serial = support::serial();
    let temp = TempDir::new().unwrap();
    let store = ArtifactStore::new(temp.path()).unwrap();

    let valid_payload = create_valid_payload_json();
    let valid_payload_str = valid_payload.to_string();
    let report_str = "{}";
    let manifest = create_valid_manifest("model-r3", &valid_payload_str, report_str);

    write_model_files(&store, &manifest, &valid_payload_str, report_str);

    // Initial normal activation
    let lease = store.try_writer().expect("acquire lease");
    let auth = authorize_for(&lease, &manifest, &valid_payload_str, None);
    let active = store
        .activate(&manifest.model_id, &auth, &lease)
        .expect("initial activation succeeds");
    assert_eq!(active.activation_generation, 1);
    assert!(!active.manifest_sha256.is_empty());

    let active_path = store.active_manifest_path();
    let stored_active_bytes = fs::read(&active_path).unwrap();

    // 1. Empty manifest_sha256
    {
        let mut tampered: ModelManifest = serde_json::from_slice(&stored_active_bytes).unwrap();
        tampered.manifest_sha256 = String::new();
        fs::write(&active_path, serde_json::to_string(&tampered).unwrap()).unwrap();
        match store.read_active() {
            Err(ClassifierError::InvalidArtifact(msg)) => {
                assert!(msg.contains("must carry non-empty manifest_sha256"));
            }
            other => panic!(
                "Expected InvalidArtifact for empty manifest_sha256, got: {:?}",
                other
            ),
        }
    }

    // 2. Generation 0
    {
        let mut tampered: ModelManifest = serde_json::from_slice(&stored_active_bytes).unwrap();
        tampered.activation_generation = 0;
        tampered.manifest_sha256 = String::new();
        fs::write(&active_path, serde_json::to_string(&tampered).unwrap()).unwrap();
        match store.read_active() {
            Err(ClassifierError::InvalidArtifact(msg)) => {
                assert!(msg.contains("activation_generation > 0"));
            }
            other => panic!(
                "Expected InvalidArtifact for generation 0, got: {:?}",
                other
            ),
        }
    }

    // 3. Empty activated_at
    {
        let mut tampered: ModelManifest = serde_json::from_slice(&stored_active_bytes).unwrap();
        tampered.activated_at = String::new();
        fs::write(&active_path, serde_json::to_string(&tampered).unwrap()).unwrap();
        match store.read_active() {
            Err(ClassifierError::InvalidArtifact(msg)) => {
                assert!(
                    msg.contains("activated_at timestamp") || msg.contains("activation timestamp")
                );
            }
            other => panic!(
                "Expected InvalidArtifact for empty activated_at, got: {:?}",
                other
            ),
        }
    }

    // 4. Checksum mismatch against stored manifest
    {
        let mut tampered: ModelManifest = serde_json::from_slice(&stored_active_bytes).unwrap();
        tampered.manifest_sha256 = "f".repeat(64);
        fs::write(&active_path, serde_json::to_string(&tampered).unwrap()).unwrap();
        match store.read_active() {
            Err(ClassifierError::InvalidArtifact(msg)) => {
                assert!(msg.contains("checksum does not match the stored manifest.json"));
            }
            other => panic!(
                "Expected InvalidArtifact for checksum mismatch, got: {:?}",
                other
            ),
        }
    }

    // 5. Altered immutable threshold with valid manifest_sha256
    {
        let mut tampered: ModelManifest = serde_json::from_slice(&stored_active_bytes).unwrap();
        tampered.thresholds[0].threshold = 0.999;
        fs::write(&active_path, serde_json::to_string(&tampered).unwrap()).unwrap();
        match store.read_active() {
            Err(ClassifierError::InvalidArtifact(msg)) => {
                assert!(msg.contains("immutable fields do not match the stored manifest"));
            }
            other => panic!(
                "Expected InvalidArtifact for altered threshold, got: {:?}",
                other
            ),
        }
    }

    // 6. Altered smoothing with valid manifest_sha256
    {
        let mut tampered: ModelManifest = serde_json::from_slice(&stored_active_bytes).unwrap();
        tampered.smoothing = 2.5;
        fs::write(&active_path, serde_json::to_string(&tampered).unwrap()).unwrap();
        match store.read_active() {
            Err(ClassifierError::InvalidArtifact(msg)) => {
                assert!(msg.contains("smoothing must equal1.0 for the compiled Bayes algorithm"));
            }
            other => panic!(
                "Expected InvalidArtifact for altered smoothing, got: {:?}",
                other
            ),
        }
    }
}

#[test]
fn test_adversarial_r4_canonical_six_domains_validation() {
    let _serial = support::serial();
    let valid_payload = create_valid_payload_json();
    let valid_payload_str = valid_payload.to_string();
    let report_str = "{}";
    let manifest = create_valid_manifest("model-r4", &valid_payload_str, report_str);

    // 1. Extra 7th domain key "unk"
    {
        let mut bad_payload = valid_payload.clone();
        bad_payload["domains"]["unk"] = serde_json::json!({
            "feature_order": ["token"],
            "class_totals": { "present": 10, "absent": 10 },
            "class_features": {},
            "total_examples": 20,
            "smoothing": 1.0
        });
        let bad_str = bad_payload.to_string();
        let mut m = manifest.clone();
        m.model_sha256 = sha256_hex(bad_str.as_bytes());
        match validate_artifact(&m, &bad_str, COMPILED_BUNDLE_SHA256) {
            Err(ClassifierError::InvalidArtifact(msg)) => {
                assert!(msg.contains("exactly the 6 canonical domains, found 7"));
            }
            other => panic!("Expected InvalidArtifact for 7th domain, got: {:?}", other),
        }
    }

    // 2. Missing domain key (only 5 domains)
    {
        let mut bad_payload = valid_payload.clone();
        bad_payload["domains"]
            .as_object_mut()
            .unwrap()
            .remove("governance");
        let bad_str = bad_payload.to_string();
        let mut m = manifest.clone();
        m.model_sha256 = sha256_hex(bad_str.as_bytes());
        match validate_artifact(&m, &bad_str, COMPILED_BUNDLE_SHA256) {
            Err(ClassifierError::InvalidArtifact(msg)) => {
                assert!(msg.contains("exactly the 6 canonical domains, found 5"));
            }
            other => panic!(
                "Expected InvalidArtifact for missing domain, got: {:?}",
                other
            ),
        }
    }

    // 3. Non-canonical domain replaced
    {
        let mut bad_payload = valid_payload.clone();
        bad_payload["domains"]
            .as_object_mut()
            .unwrap()
            .remove("governance");
        bad_payload["domains"]["cyber"] = serde_json::json!({
            "feature_order": ["token"],
            "class_totals": { "present": 10, "absent": 10 },
            "class_features": {},
            "total_examples": 20,
            "smoothing": 1.0
        });
        let bad_str = bad_payload.to_string();
        let mut m = manifest.clone();
        m.model_sha256 = sha256_hex(bad_str.as_bytes());
        match validate_artifact(&m, &bad_str, COMPILED_BUNDLE_SHA256) {
            Err(ClassifierError::InvalidArtifact(msg)) => {
                assert!(msg.contains("domain `governance` is missing"));
            }
            other => panic!(
                "Expected InvalidArtifact for non-canonical domain, got: {:?}",
                other
            ),
        }
    }

    // 4. Domain value is not an object
    {
        let mut bad_payload = valid_payload.clone();
        bad_payload["domains"]["governance"] = serde_json::json!(42);
        let bad_str = bad_payload.to_string();
        let mut m = manifest.clone();
        m.model_sha256 = sha256_hex(bad_str.as_bytes());
        match validate_artifact(&m, &bad_str, COMPILED_BUNDLE_SHA256) {
            Err(ClassifierError::InvalidArtifact(msg)) => {
                assert!(msg.contains("not a JSON object"));
            }
            other => panic!(
                "Expected InvalidArtifact for non-object domain, got: {:?}",
                other
            ),
        }
    }
}

#[test]
fn test_adversarial_r5_bounded_file_reading() {
    let _serial = support::serial();
    let temp = TempDir::new().unwrap();
    let store = ArtifactStore::new(temp.path()).unwrap();

    let model_id = ModelId::parse("model-r5").unwrap();
    let dir = store.model_dir(&model_id);
    fs::create_dir_all(&dir).unwrap();

    // 1. Oversized manifest (2 MiB > 1 MiB limit)
    let big_manifest_path = dir.join(MODEL_MANIFEST_FILE);
    {
        let mut f = File::create(&big_manifest_path).unwrap();
        let chunk = vec![b' '; 64 * 1024];
        for _ in 0..32 {
            f.write_all(&chunk).unwrap();
        }
    }
    match store.read_manifest(&model_id) {
        Err(ClassifierError::ModelTooLarge) => {}
        other => panic!(
            "Expected ModelTooLarge for oversized manifest, got: {:?}",
            other
        ),
    }

    // 2. Exact boundary MAX_MANIFEST_BYTES + 1 byte
    {
        let mut f = File::create(&big_manifest_path).unwrap();
        let chunk = vec![b'a'; (MAX_MANIFEST_BYTES + 1) as usize];
        f.write_all(&chunk).unwrap();
    }
    match store.read_manifest(&model_id) {
        Err(ClassifierError::ModelTooLarge) => {}
        other => panic!(
            "Expected ModelTooLarge for MAX_MANIFEST_BYTES + 1, got: {:?}",
            other
        ),
    }

    // 3. Oversized payload (65 MiB > 64 MiB limit)
    let big_payload_path = dir.join(MODEL_PAYLOAD_FILE);
    {
        let mut f = File::create(&big_payload_path).unwrap();
        let chunk = vec![b'0'; 1024 * 1024];
        for _ in 0..65 {
            f.write_all(&chunk).unwrap();
        }
    }
    match store.read_payload(&model_id) {
        Err(ClassifierError::ModelTooLarge) => {}
        other => panic!(
            "Expected ModelTooLarge for oversized payload, got: {:?}",
            other
        ),
    }

    // 4. Oversized active.json
    let active_path = store.active_manifest_path();
    {
        let mut f = File::create(&active_path).unwrap();
        let chunk = vec![b'x'; (MAX_MANIFEST_BYTES + 10) as usize];
        f.write_all(&chunk).unwrap();
    }
    match store.read_active() {
        Err(ClassifierError::ModelTooLarge) => {}
        other => panic!(
            "Expected ModelTooLarge for oversized active pointer, got: {:?}",
            other
        ),
    }
}

#[test]
fn test_adversarial_restore_smoke_check_and_atomic_abort() {
    let _serial = support::serial();
    let temp = TempDir::new().unwrap();
    let store = ArtifactStore::new(temp.path()).unwrap();

    let valid_payload = create_valid_payload_json();
    let valid_payload_str = valid_payload.to_string();
    let report_str = "{}";

    // 1. Activate baseline model 1
    let m1 = create_valid_manifest("model-v1", &valid_payload_str, report_str);
    write_model_files(&store, &m1, &valid_payload_str, report_str);
    let lease = store.try_writer().expect("acquire lease");
    let auth1 = authorize_for(&lease, &m1, &valid_payload_str, None);
    let active1 = store
        .activate(&m1.model_id, &auth1, &lease)
        .expect("initial activation succeeds");
    assert_eq!(active1.model_id.as_str(), "model-v1");
    assert_eq!(active1.activation_generation, 1);

    // 2. Candidate model 2 with prototype pollution injected
    let mut corrupt_payload = valid_payload.clone();
    corrupt_payload["domains"]["governance"]["class_features"]["__proto__"] =
        serde_json::json!({ "present": 1 });
    let corrupt_payload_str = corrupt_payload.to_string();

    let m2 = create_valid_manifest("model-v2", &corrupt_payload_str, report_str);
    write_model_files(&store, &m2, &corrupt_payload_str, report_str);

    let auth2 = authorize_for(&lease, &m2, &corrupt_payload_str, Some(&active1));

    match store.activate(&m2.model_id, &auth2, &lease) {
        Err(ClassifierError::InvalidArtifact(msg)) => {
            assert!(
                msg.contains("forbidden key `__proto__`") || msg.contains("prototype pollution")
            );
        }
        other => panic!(
            "Expected InvalidArtifact for corrupt candidate, got: {:?}",
            other
        ),
    }

    // 3. Prior active model MUST remain intact at generation 1
    let active_after = store
        .read_active()
        .expect("read_active succeeds")
        .expect("active exists");
    assert_eq!(active_after.model_id.as_str(), "model-v1");
    assert_eq!(active_after.activation_generation, 1);
}

#[test]
fn test_adversarial_writer_lease_contention() {
    let _serial = support::serial();
    let temp = TempDir::new().unwrap();
    let store = ArtifactStore::new(temp.path()).unwrap();

    let lease1 = store
        .try_writer()
        .expect("first lease acquisition must succeed");
    assert!(lease1.path().ends_with("writer.lock"));

    // Concurrent try_writer returns ClassifierError::Busy
    match store.try_writer() {
        Err(ClassifierError::Busy) => {}
        other => panic!("Expected ClassifierError::Busy, got: {:?}", other),
    }

    // Dropping lease allows subsequent acquisition
    drop(lease1);
    let lease2 = store
        .try_writer()
        .expect("lease acquisition after release must succeed");
    drop(lease2);
}

fn report_for(manifest: &ModelManifest) -> argos_osint_core::classifier::training::TrainingReport {
    let decisions: Vec<_> = DomainId::ALL.iter().map(|domain| serde_json::json!({
        "domain": domain, "accepted": false, "failures": ["missing_reviewed_real_evidence"],
        "true_positive_families": 0, "false_positive_families": 0, "false_negative_families": 0,
        "true_negative_families": 0, "accepted_families": 0, "precision": null, "precision_ci95": null
    })).collect();
    serde_json::from_value(serde_json::json!({
        "schema_version": argos_osint_core::classifier::training::TRAINING_REPORT_SCHEMA_VERSION, "request_id": "adversarial", "model_id": manifest.model_id,
        "provenance": manifest.provenance, "class_counts": manifest.class_counts, "family_counts": manifest.family_counts,
        "calibration": { "version": argos_osint_core::classifier::calibration::CALIBRATION_VERSION, "domains": DomainId::ALL.iter().map(|domain| serde_json::json!({
            "domain": domain, "knots": [{"raw_upper": 0.0, "relevance": 0.5}], "threshold": 0.5, "min_informative_features": 1
        })).collect::<Vec<_>>() },
        "evaluation": { "domains": decisions, "macro_f1": 0.0, "complete_families": 0, "exact_pair_families": 0,
            "false_oos_admissions": 0, "false_single_domain_admissions": 0, "out_of_scope_families": 0,
            "single_domain_families": 0, "invalid_selected_pairs": 0, "coverage": 0.0 },
        "domain_decisions": decisions
    })).unwrap()
}

fn authorize_for(
    lease: &WriterLease,
    manifest: &ModelManifest,
    payload: &str,
    prior: Option<&ModelManifest>,
) -> ActivationAuthorization {
    use argos_osint_core::classifier::calibration::{
        authorize_activation, ActivationAuthorizationDecision,
    };
    use argos_osint_core::classifier::training::{SyntheticActivation, TrainRequest};
    let baseline_payload = create_valid_payload_json().to_string();
    let baseline = create_valid_manifest(manifest.model_id.as_str(), &baseline_payload, "{}");
    let mut artifact =
        validate_artifact(&baseline, &baseline_payload, COMPILED_BUNDLE_SHA256).unwrap();
    // Deliberate post-validation corruption exercises activation's independent validation.
    artifact.manifest = manifest.clone();
    artifact.payload = payload.into();
    let request = TrainRequest {
        request_id: "adversarial".into(),
        model_id: manifest.model_id.clone(),
        provenance: manifest.provenance.clone(),
        synthetic_activation: SyntheticActivation::ExplicitOptIn,
    };
    match authorize_activation(
        lease,
        &artifact,
        &report_for(manifest),
        &request,
        None,
        prior,
    )
    .unwrap()
    {
        ActivationAuthorizationDecision::Authorized(authorization) => authorization,
        other => panic!("expected explicit synthetic authorization, got {other:?}"),
    }
}

#[test]
fn writer_authorization_is_bound_to_home_and_acquisition() {
    let _serial = support::serial();
    let first_home = TempDir::new().unwrap();
    let second_home = TempDir::new().unwrap();
    let first = ArtifactStore::new(first_home.path()).unwrap();
    let second = ArtifactStore::new(second_home.path()).unwrap();
    let payload = create_valid_payload_json().to_string();
    let manifest = create_valid_manifest("lease-bound", &payload, "{}");
    write_model_files(&first, &manifest, &payload, "{}");
    let lease = first.try_writer().unwrap();
    let authorization = authorize_for(&lease, &manifest, &payload, None);
    assert!(matches!(
        second.activate(&manifest.model_id, &authorization, &lease),
        Err(ClassifierError::ActivationDenied { .. })
    ));
    drop(lease);
    let replacement = first.try_writer().unwrap();
    assert!(matches!(
        first.activate(&manifest.model_id, &authorization, &replacement),
        Err(ClassifierError::ActivationDenied { .. })
    ));
    assert!(first.read_active().unwrap().is_none());
}

#[test]
fn raw_duplicate_keys_and_manifest_support_disagreement_are_rejected() {
    let _serial = support::serial();
    let payload = create_valid_payload_json().to_string();
    let mut manifest = create_valid_manifest("duplicate", &payload, "{}");
    let duplicate = payload.replacen("\"class_totals\":{", "\"class_totals\":{\"present\":20,", 1);
    manifest.model_sha256 = sha256_hex(duplicate.as_bytes());
    assert!(
        matches!(validate_artifact(&manifest, &duplicate, COMPILED_BUNDLE_SHA256), Err(ClassifierError::InvalidArtifact(message)) if message.contains("duplicate JSON key"))
    );
    manifest.model_sha256 = sha256_hex(payload.as_bytes());
    manifest.class_counts[0].present += 1;
    assert!(
        matches!(validate_artifact(&manifest, &payload, COMPILED_BUNDLE_SHA256), Err(ClassifierError::InvalidArtifact(message)) if message.contains("manifest class counts"))
    );
}

#[test]
fn malformed_numeric_parameters_and_bridge_version_fail_before_restore() {
    let _serial = support::serial();
    let baseline = create_valid_payload_json();
    for (field, value) in [
        ("negative_weight", serde_json::json!(-1)),
        ("string_weight", serde_json::json!("2")),
        ("total_examples", serde_json::json!(40)),
        ("smoothing", serde_json::json!(-1)),
    ] {
        let mut payload = baseline.clone();
        if field.ends_with("weight") {
            payload["domains"]["governance"]["class_features"]["present"]["0"] = value;
        } else {
            payload["domains"]["governance"][field] = value;
        }
        let text = payload.to_string();
        let manifest = create_valid_manifest("numeric", &text, "{}");
        assert!(
            matches!(
                validate_artifact(&manifest, &text, COMPILED_BUNDLE_SHA256),
                Err(ClassifierError::InvalidArtifact(_))
            ),
            "{field} must fail despite its repaired payload checksum"
        );
    }
    let mut payload = baseline;
    payload["bridge_version"] = serde_json::json!("foreign-bridge");
    let text = payload.to_string();
    let manifest = create_valid_manifest("bridge-version", &text, "{}");
    assert!(matches!(
        validate_artifact(&manifest, &text, COMPILED_BUNDLE_SHA256),
        Err(ClassifierError::Incompatible(_))
    ));
}

#[test]
fn activation_history_failure_is_reported_as_uncertain() {
    let _serial = support::serial();
    let home = TempDir::new().unwrap();
    let store = ArtifactStore::new(home.path()).unwrap();
    let payload = create_valid_payload_json().to_string();
    let manifest = create_valid_manifest("history-fault", &payload, "{}");
    write_model_files(&store, &manifest, &payload, "{}");
    let lease = store.try_writer().unwrap();
    let first_authorization = authorize_for(&lease, &manifest, &payload, None);
    let prior = store
        .activate(&manifest.model_id, &first_authorization, &lease)
        .unwrap();
    let second = create_valid_manifest("history-fault-second", &payload, "{}");
    write_model_files(&store, &second, &payload, "{}");
    fs::create_dir(
        store
            .root()
            .join("activated-models")
            .join(format!("{}.json", second.model_id)),
    )
    .unwrap();
    let authorization = authorize_for(&lease, &second, &payload, Some(&prior));
    assert!(matches!(
        store.activate(&second.model_id, &authorization, &lease),
        Err(ClassifierError::ActivationCommitUncertain { .. })
    ));
    assert_eq!(
        store.read_active().unwrap().unwrap().model_id,
        second.model_id
    );
}

#[test]
fn receipt_validation_rejects_inconsistent_persisted_decisions() {
    let _serial = support::serial();
    use argos_osint_core::classifier::{ClassifyInput, RejectionReason};
    let input = ClassifyInput::new(
        "receipt",
        "Policy announcement",
        "Synthetic test",
        Some("en".into()),
    );
    let mut receipt = ClassificationReceipt::new("receipt", input.input_hash, "receipt-model");
    receipt.evaluated_at = "2026-01-01T00:00:00Z".into();
    receipt.rejection_reason = Some(RejectionReason::InsufficientDomains);
    for score in &mut receipt.scores {
        score.calibrated_score = 0.5;
        score.threshold = 0.9;
        score.informative_features = 1;
    }
    receipt.validate().unwrap();
    let mut malformed = receipt.clone();
    malformed.scores[0].calibrated_score = 1.1;
    assert!(malformed.validate().is_err());
    let mut malformed = receipt.clone();
    malformed.scores[0].passes_gate = true;
    assert!(malformed.validate().is_err());
    let mut malformed = receipt.clone();
    malformed.rejection_reason = None;
    assert!(malformed.validate().is_err());
    let mut malformed = receipt.clone();
    malformed.input_hash = "bad".into();
    assert!(malformed.validate().is_err());
    let mut malformed = receipt;
    malformed.evaluated_at = "bad".into();
    assert!(malformed.validate().is_err());
}
