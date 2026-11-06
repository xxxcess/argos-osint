//! Additional classifier persistence, isolation and numerical boundary proofs.
#![cfg(feature = "classifier")]
#[path = "fixtures/classifier/support.rs"]
mod support;

use argos_osint_core::atlas_taxonomy::DomainId;
use argos_osint_core::classifier::artifact::{
    self, ArtifactStore, ModelId, ModelManifest, WriterLease,
};
use argos_osint_core::classifier::calibration::{
    authorize_activation, ActivationAuthorizationDecision,
};
use argos_osint_core::classifier::training::SyntheticActivation;
use argos_osint_core::classifier::{BridgeOperation, ClassifierError, WorkControl};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

fn activate(
    store: &ArtifactStore,
    lease: &WriterLease,
    candidate: &support::Candidate,
    prior: Option<&ModelManifest>,
) -> ModelManifest {
    let validated = store
        .write_candidate(
            lease,
            &candidate.manifest,
            &candidate.payload,
            &candidate.report,
        )
        .unwrap();
    let decision = authorize_activation(
        lease,
        &validated,
        &candidate.report,
        &candidate.request(SyntheticActivation::ExplicitOptIn),
        None,
        prior,
    )
    .unwrap();
    let ActivationAuthorizationDecision::Authorized(authorization) = decision else {
        panic!("explicit fixture consent denied")
    };
    store
        .activate(&candidate.manifest.model_id, &authorization, lease)
        .unwrap()
}

#[test]
fn activation_and_rollback_keep_immutable_prior_models() {
    let _serial = support::serial();
    let first = support::Candidate::from_fixture(support::trained());
    let active_first = activate(&first.store, &first.fixture.lease, &first, None);
    assert_eq!(active_first.activation_generation, 1);
    let first_manifest_path = first
        .store
        .model_dir(&first.manifest.model_id)
        .join(artifact::MODEL_MANIFEST_FILE);
    let first_manifest_bytes = fs::read(&first_manifest_path).unwrap();
    let mut second_fixture = support::Fixture::new("fixture-m2");
    second_fixture.fit(support::recipe()["rows"].clone());
    second_fixture.fit(serde_json::json!([{"row_id":"extra","title":"additional ballot","description":"","membership":["Present","Unknown","Unknown","Unknown","Unknown","Unknown"]}]));
    let second = support::Candidate::from_fixture(second_fixture);
    assert_ne!(second.payload, first.payload);
    let active_second = activate(
        &first.store,
        &first.fixture.lease,
        &second,
        Some(&active_first),
    );
    assert_eq!(active_second.activation_generation, 2);
    assert_eq!(
        active_second.previous_model_id.as_ref(),
        Some(&first.manifest.model_id)
    );
    let second_id = second.manifest.model_id.clone();
    let second_manifest_path = first
        .store
        .model_dir(&second_id)
        .join(artifact::MODEL_MANIFEST_FILE);
    let second_manifest_bytes = fs::read(&second_manifest_path).unwrap();
    drop(second);
    let rolled = first
        .store
        .rollback_to(&first.manifest.model_id, &first.fixture.lease)
        .unwrap();
    assert_eq!(rolled.activation_generation, 3);
    assert_eq!(rolled.previous_model_id.as_ref(), Some(&second_id));
    assert_eq!(
        fs::read(&first_manifest_path).unwrap(),
        first_manifest_bytes
    );
    assert_eq!(
        fs::read(&second_manifest_path).unwrap(),
        second_manifest_bytes
    );
    let before = fs::read(first.store.active_manifest_path()).unwrap();
    fs::write(
        first
            .store
            .model_dir(&second_id)
            .join(artifact::MODEL_PAYLOAD_FILE),
        "{}",
    )
    .unwrap();
    assert!(matches!(
        first.store.rollback_to(&second_id, &first.fixture.lease),
        Err(ClassifierError::InvalidArtifact(_))
    ));
    assert_eq!(
        fs::read(first.store.active_manifest_path()).unwrap(),
        before
    );
}

#[test]
fn persisted_order_binding_rejects_order_only_permutations_in_each_layer() {
    let _serial = support::serial();
    let candidate = support::Candidate::from_fixture(support::trained());
    let mut restored = support::Fixture::new("restore-permutation");
    for domain in DomainId::ALL {
        let mut value: serde_json::Value = serde_json::from_str(&candidate.payload).unwrap();
        value["domains"][domain.as_str()]["feature_order"]
            .as_array_mut()
            .unwrap()
            .swap(0, 1);
        let payload = value.to_string();
        let mut manifest = candidate.manifest.clone();
        manifest.model_sha256 = artifact::sha256_hex(payload.as_bytes());
        assert!(
            matches!(artifact::validate_artifact(&manifest,&payload,artifact::compiled_bundle_sha256()),Err(ClassifierError::InvalidArtifact(message)) if message.contains(domain.as_str()) && message.contains("feature_index"))
        );
        let request = serde_json::json!({"version":"1","request_id":format!("permutation-{}",domain.as_str()),"model_id":restored.model,"artifact":value});
        assert!(
            matches!(restored.worker.call(BridgeOperation::RestoreModel,&request.to_string(),&restored.control),Err(ClassifierError::FeatureOrderMismatch { domain:found }) if found==domain)
        );
    }
    let mut old: serde_json::Value = serde_json::from_str(&candidate.payload).unwrap();
    old.as_object_mut()
        .unwrap()
        .remove("feature_schema_version");
    let payload = old.to_string();
    let mut manifest = candidate.manifest.clone();
    manifest.model_sha256 = artifact::sha256_hex(payload.as_bytes());
    assert!(matches!(
        artifact::validate_artifact(&manifest, &payload, artifact::compiled_bundle_sha256()),
        Err(ClassifierError::InvalidArtifact(_))
    ));
}

#[test]
fn unique_known_features_remain_finite_after_upstream_probability_underflow() {
    let _serial = support::serial();
    let mut fixture = support::trained();
    let text = (0..1000)
        .map(|index| format!("sentinelfeature{index:04}"))
        .collect::<Vec<_>>()
        .join(" ");
    fixture.fit(serde_json::json!([{"row_id":"metadataidentitysentinel","title":text,"description":"","membership":["Present","Unknown","Unknown","Unknown","Unknown","Unknown"]}]));
    let payload = fixture.payload();
    let order = payload["domains"]["governance"]["feature_order"]
        .as_array()
        .unwrap();
    assert!(order.len() >= 1000);
    assert!(!order.contains(&serde_json::json!("metadataidentitysentinel")));
    let scores = fixture.classify(&text);
    let governance = &scores[0];
    assert_eq!(governance["known_feature_count"], 1000);
    let absent = governance["absent_log_score"].as_f64().unwrap();
    assert!(
        absent.is_finite() && absent < -745.0,
        "fixture must cross ordinary exponentiation's underflow boundary"
    );
    assert_eq!(absent.exp(), 0.0);
    let extended = fixture.classify(&format!("{text} whollyunseenword"));
    for (original, extended) in scores
        .as_array()
        .unwrap()
        .iter()
        .zip(extended.as_array().unwrap())
    {
        for field in ["present_log_score", "absent_log_score", "raw_log_odds"] {
            assert!(original[field].as_f64().unwrap().is_finite());
            assert_eq!(
                original[field], extended[field],
                "OOV text cannot add an absent-token factor"
            );
        }
    }
}

#[test]
fn duplicated_training_tokens_do_not_change_fitted_binary_counts() {
    let _serial = support::serial();
    let mut first = support::Fixture::new("single-token-recipe");
    let mut second = support::Fixture::new("repeated-token-recipe");
    let rows = support::recipe()["rows"].clone();
    let repeat_rows = |separator: &str| {
        let mut repeated = rows.clone();
        for row in repeated.as_array_mut().unwrap() {
            let title = row["title"].as_str().unwrap();
            row["title"] = format!("{title}{separator}{title}").into();
            let description = row["description"].as_str().unwrap();
            row["description"] = format!("{description}{separator}{description}").into();
        }
        repeated
    };
    first.fit(rows.clone());
    // Repeating the same phrases across a hard boundary changes occurrence
    // frequency, but must preserve every binary count, binding and prior.
    second.fit(repeat_rows("\n"));
    let original = first.payload();
    assert_eq!(original, second.payload());
    drop(first);
    drop(second);

    // Adjacent copies introduce a real bigram at their join. They are distinct
    // feature sets, so the unigram-era equality assertion does not apply.
    let mut adjacent = support::Fixture::new("adjacent-repeat-recipe");
    adjacent.fit(repeat_rows(" "));
    assert_ne!(original, adjacent.payload());
}

fn pad(path: &Path, limit: u64) {
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    let length = file.metadata().unwrap().len();
    assert!(length <= limit);
    let chunk = [b' '; 8192];
    let mut remaining = limit - length;
    while remaining > 0 {
        let count = remaining.min(chunk.len() as u64) as usize;
        file.write_all(&chunk[..count]).unwrap();
        remaining -= count as u64;
    }
    file.sync_all().unwrap();
}
fn hash_file(path: &Path) -> String {
    use sha2::Digest;
    let mut hash = sha2::Sha256::new();
    let mut file = fs::File::open(path).unwrap();
    let mut chunk = [0; 65536];
    loop {
        let count = file.read(&mut chunk).unwrap();
        if count == 0 {
            break;
        }
        hash.update(&chunk[..count]);
    }
    format!("{:x}", hash.finalize())
}

#[test]
fn artifact_files_accept_limit_and_reject_limit_plus_one() {
    let _serial = support::serial();
    let candidate = support::Candidate::from_fixture(support::trained());
    let active = activate(&candidate.store, &candidate.fixture.lease, &candidate, None);
    let directory = candidate.store.model_dir(&candidate.manifest.model_id);
    let pointer = candidate.store.active_manifest_path();
    let manifest_path = directory.join(artifact::MODEL_MANIFEST_FILE);
    let report_path = directory.join(artifact::MODEL_REPORT_FILE);
    let payload_path = directory.join(artifact::MODEL_PAYLOAD_FILE);
    let pointer_bytes = fs::read(&pointer).unwrap();
    let manifest_bytes = fs::read(&manifest_path).unwrap();
    let report_bytes = fs::read(&report_path).unwrap();
    let payload_bytes = fs::read(&payload_path).unwrap();
    pad(&pointer, artifact::MAX_MANIFEST_BYTES);
    assert!(candidate.store.read_active().unwrap().is_some());
    pad(&pointer, artifact::MAX_MANIFEST_BYTES + 1);
    assert!(matches!(
        candidate.store.read_active(),
        Err(ClassifierError::ModelTooLarge)
    ));
    fs::write(&pointer, &pointer_bytes).unwrap();

    pad(&manifest_path, artifact::MAX_MANIFEST_BYTES);
    let mut overlay = active.clone();
    overlay.manifest_sha256 = hash_file(&manifest_path);
    fs::write(&pointer, serde_json::to_vec_pretty(&overlay).unwrap()).unwrap();
    assert_eq!(
        candidate
            .store
            .read_manifest(&candidate.manifest.model_id)
            .unwrap(),
        candidate.manifest
    );
    assert!(candidate.store.read_active().unwrap().is_some());
    pad(&manifest_path, artifact::MAX_MANIFEST_BYTES + 1);
    assert!(matches!(
        candidate.store.read_manifest(&candidate.manifest.model_id),
        Err(ClassifierError::ModelTooLarge)
    ));
    assert!(matches!(
        candidate.store.read_active(),
        Err(ClassifierError::ModelTooLarge)
    ));
    fs::write(&manifest_path, &manifest_bytes).unwrap();
    fs::write(&pointer, &pointer_bytes).unwrap();

    pad(&report_path, artifact::MAX_MANIFEST_BYTES);
    let mut manifest = candidate.manifest.clone();
    manifest.report_sha256 = hash_file(&report_path);
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    candidate
        .store
        .validate_model(&candidate.manifest.model_id)
        .unwrap();
    pad(&report_path, artifact::MAX_MANIFEST_BYTES + 1);
    assert!(matches!(
        candidate.store.validate_model(&candidate.manifest.model_id),
        Err(ClassifierError::ModelTooLarge)
    ));
    fs::write(&report_path, &report_bytes).unwrap();
    fs::write(&manifest_path, &manifest_bytes).unwrap();

    pad(&payload_path, artifact::MAX_ARTIFACT_BYTES);
    let payload = candidate
        .store
        .read_payload(&candidate.manifest.model_id)
        .unwrap();
    assert_eq!(payload.len() as u64, artifact::MAX_ARTIFACT_BYTES);
    let mut manifest = candidate.manifest.clone();
    manifest.model_sha256 = hash_file(&payload_path);
    let valid =
        artifact::validate_artifact(&manifest, &payload, artifact::compiled_bundle_sha256())
            .unwrap();
    drop(valid);
    drop(payload);
    pad(&payload_path, artifact::MAX_ARTIFACT_BYTES + 1);
    assert!(matches!(
        candidate.store.read_payload(&candidate.manifest.model_id),
        Err(ClassifierError::ModelTooLarge)
    ));
    fs::write(&payload_path, &payload_bytes).unwrap();

    fs::remove_file(&report_path).unwrap();
    assert!(matches!(
        candidate.store.validate_model(&candidate.manifest.model_id),
        Err(ClassifierError::InvalidArtifact(_))
    ));
    fs::write(&report_path, &report_bytes).unwrap();
    fs::remove_file(&payload_path).unwrap();
    fs::create_dir(&payload_path).unwrap();
    assert!(matches!(
        candidate.store.read_payload(&candidate.manifest.model_id),
        Err(ClassifierError::Io(_))
    ));
}

#[test]
fn writer_lease_child_probe() {
    if let Some(home) = std::env::var_os("ARGOS_CLASSIFIER_LOCK_CHILD_HOME") {
        let store = ArtifactStore::new(Path::new(&home)).unwrap();
        let busy = std::env::var_os("ARGOS_CLASSIFIER_LOCK_EXPECT_BUSY").is_some();
        if busy {
            assert!(matches!(store.try_writer(), Err(ClassifierError::Busy)));
        } else {
            store.try_writer().unwrap();
        }
    } else {
        let home = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(home.path()).unwrap();
        let lease = store.try_writer().unwrap();
        assert_eq!(lease.path(), store.root().join("writer.lock"));
        drop(lease);
        drop(store);
        home.close().unwrap();
    }
}
fn child_probe(home: &Path, busy: bool) {
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};
    struct OwnedChild(Option<Child>);
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            if let Some(mut child) = self.0.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
    let mut command = Command::new(std::env::current_exe().unwrap());
    // Preserve only the executable loader paths; provider, CSV and process
    // settings never enter the helper. The parent process environment is untouched.
    command.env_clear();
    for name in [
        "PATH",
        "DYLD_FALLBACK_LIBRARY_PATH",
        "DYLD_LIBRARY_PATH",
        "LD_LIBRARY_PATH",
        "SYSTEMROOT",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
        .args(["--exact", "writer_lease_child_probe", "--nocapture"])
        .current_dir(home)
        .env("ARGOS_HOME", home)
        .env("ARGOS_CLASSIFIER_LOCK_CHILD_HOME", home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if busy {
        command.env("ARGOS_CLASSIFIER_LOCK_EXPECT_BUSY", "1");
    } else {
        command.env_remove("ARGOS_CLASSIFIER_LOCK_EXPECT_BUSY");
    }
    let mut child = OwnedChild(Some(command.spawn().unwrap()));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if child.0.as_mut().unwrap().try_wait().unwrap().is_some() {
            let output = child.0.take().unwrap().wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "lease child failed: {} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        assert!(
            Instant::now() < deadline,
            "lease child exceeded bounded lifetime"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn writer_leases_are_exclusive_across_processes_and_independent_homes() {
    let home = tempfile::tempdir().unwrap();
    let store = ArtifactStore::new(home.path()).unwrap();
    let lease = store.try_writer().unwrap();
    child_probe(home.path(), true);
    let independent = tempfile::tempdir().unwrap();
    child_probe(independent.path(), false);
    drop(lease);
    child_probe(home.path(), false);
    drop(store);
    home.close().unwrap();
    independent.close().unwrap();
}

#[test]
fn cached_live_pin_and_integrity_checks_do_not_require_a_fourth_runtime() {
    use argos_osint_core::classifier::readiness::ClassifierHome;
    use argos_osint_core::classifier::{self, ClassifyInput};
    use std::sync::Arc;
    let _serial = support::serial();
    let candidate = support::Candidate::from_fixture(support::trained());
    activate(&candidate.store, &candidate.fixture.lease, &candidate, None);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let home = ClassifierHome::new(candidate.fixture.home.path().to_path_buf());
    let pin = runtime.block_on(home.pin(None)).unwrap();
    let third = classifier::start_candidate_worker(
        &ModelId::parse("third-reservation").unwrap(),
        candidate.fixture.lease.clone(),
        &WorkControl::new(),
    )
    .unwrap();
    candidate
        .store
        .validate_model_integrity(&candidate.fixture.model)
        .unwrap();
    assert!(matches!(
        candidate.store.validate_model(&candidate.fixture.model),
        Err(ClassifierError::RuntimeBudgetExceeded { .. })
    ));
    let reused = runtime.block_on(home.pin(Some(&pin.snapshot))).unwrap();
    assert!(Arc::ptr_eq(&pin.service, &reused.service));
    let initial = pin.service.performance().unwrap();
    assert!(initial.js_heap_current_bytes > 0);
    assert!(initial.js_heap_peak_bytes >= initial.js_heap_current_bytes);
    assert_eq!(initial.runtime_reserved_bytes, 129 * 1024 * 1024);
    assert_eq!(initial.stack_limit_bytes, 1024 * 1024);
    assert!(initial.batch_execution_micros.is_empty());
    let control = WorkControl::new();
    let inputs: Vec<_> = (0..64)
        .map(|index| {
            ClassifyInput::new(
                format!("performance-{index}"),
                "ballot",
                "patrol",
                Some("en".into()),
            )
        })
        .collect();
    assert_eq!(
        pin.service
            .classify_batch_with_control(inputs, &control)
            .unwrap()
            .len(),
        64
    );
    let observed = pin.service.performance().unwrap();
    assert_eq!(observed.batch_sizes, vec![64]);
    assert_eq!(observed.batch_execution_micros.len(), 1);
    assert!(observed.js_heap_peak_bytes >= observed.js_heap_current_bytes);
    assert!(observed.js_heap_current_bytes > 0);
    let cancelled = WorkControl::new();
    cancelled
        .cancel
        .store(true, std::sync::atomic::Ordering::Release);
    let input = ClassifyInput::new("cancelled", "ballot", "patrol", Some("en".into()));
    assert!(matches!(
        pin.service
            .classify_batch_with_control(vec![input.clone()], &cancelled),
        Err(ClassifierError::Cancelled)
    ));
    let mut expired = WorkControl::new();
    expired.overall_deadline = std::time::Instant::now();
    assert!(matches!(
        pin.service
            .classify_batch_with_control(vec![input.clone()], &expired),
        Err(ClassifierError::Timeout)
    ));
    let mut changed = input;
    changed.input_hash = "0".repeat(64);
    assert!(matches!(
        pin.service.classify_batch(vec![changed]),
        Err(ClassifierError::InvalidArtifact(_))
    ));
    assert_eq!(
        pin.service.performance().unwrap(),
        observed,
        "rejected-before-dispatch work must not invent samples"
    );
    assert_eq!(pin.service.metrics().unwrap().articles, 64);
    drop(third);
}

#[test]
fn incremental_numeric_fit_matches_an_equivalent_one_shot_fit() {
    let _serial = support::serial();
    let mut incremental = support::Fixture::new("incremental");
    let mut one_shot = support::Fixture::new("one-shot");
    let base = support::recipe()["rows"].clone();
    let extra = serde_json::json!({"row_id":"extra","title":"0 numericappend","description":"","membership":["Present","Unknown","Unknown","Unknown","Unknown","Unknown"]});
    incremental.fit(base.clone());
    incremental.fit(serde_json::json!([extra.clone()]));
    let mut combined = base;
    combined.as_array_mut().unwrap().push(extra);
    one_shot.fit(combined);
    assert_eq!(incremental.payload(), one_shot.payload());
    assert_eq!(
        incremental.classify("0 numericappend ballot"),
        one_shot.classify("0 numericappend ballot")
    );
}

#[test]
fn artifact_adversarial_invariants_are_checked_in_every_domain_position() {
    let _serial = support::serial();
    let candidate = support::Candidate::from_fixture(support::trained());
    for domain in DomainId::ALL {
        for invariant in [
            "missing_class",
            "empty_class",
            "third_class",
            "wrong_case",
            "mismatched_maps",
            "mismatched_counts",
            "feature_index",
            "numeric_weight",
        ] {
            let mut payload: serde_json::Value = serde_json::from_str(&candidate.payload).unwrap();
            let model = &mut payload["domains"][domain.as_str()];
            let mut manifest = candidate.manifest.clone();
            let expected = match invariant {
                "missing_class" => {
                    model["class_totals"]
                        .as_object_mut()
                        .unwrap()
                        .remove("absent");
                    "class set"
                }
                "empty_class" => {
                    model["class_totals"]["absent"] = 1.into();
                    "support"
                }
                "third_class" => {
                    model["class_totals"]["maybe"] = 2.into();
                    "class set"
                }
                "wrong_case" => {
                    let value = model["class_totals"]
                        .as_object_mut()
                        .unwrap()
                        .remove("present")
                        .unwrap();
                    model["class_totals"]["Present"] = value;
                    "class set"
                }
                "mismatched_maps" => {
                    model["class_features"]
                        .as_object_mut()
                        .unwrap()
                        .remove("absent");
                    "class_features"
                }
                "mismatched_counts" => {
                    manifest
                        .class_counts
                        .iter_mut()
                        .find(|count| count.domain == domain)
                        .unwrap()
                        .present += 1;
                    "class counts"
                }
                "feature_index" => {
                    model["feature_index"][0]["index"] = 1.into();
                    "feature_index"
                }
                "numeric_weight" => {
                    model["class_features"]["present"]["0"] = serde_json::json!("not a number");
                    "class_features"
                }
                _ => unreachable!(),
            };
            let text = payload.to_string();
            manifest.model_sha256 = artifact::sha256_hex(text.as_bytes());
            assert!(
                matches!(artifact::validate_artifact(&manifest,&text,artifact::compiled_bundle_sha256()),Err(ClassifierError::InvalidArtifact(message)) if message.contains(domain.as_str())&&message.contains(expected)),
                "{domain:?}:{invariant}"
            );
        }
    }
    for field in [
        "bridge_sha256",
        "bridge_version",
        "adapter_sha256",
        "source_manifest_sha256",
        "feature_schema_version",
    ] {
        let mut manifest = serde_json::to_value(&candidate.manifest).unwrap();
        manifest.as_object_mut().unwrap().remove(field);
        let path = candidate.store.model_dir(&candidate.manifest.model_id);
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join(artifact::MODEL_MANIFEST_FILE),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(
            matches!(
                candidate.store.read_manifest(&candidate.manifest.model_id),
                Err(ClassifierError::InvalidArtifact(_))
            ),
            "missing {field}"
        );
    }
}

#[test]
fn active_pointer_metadata_and_immutable_bytes_are_bound_before_restore() {
    let _serial = support::serial();
    let candidate = support::Candidate::from_fixture(support::trained());
    let active = activate(&candidate.store, &candidate.fixture.lease, &candidate, None);
    let pointer = candidate.store.active_manifest_path();
    let original = fs::read(&pointer).unwrap();
    let base: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let mut variants = Vec::new();
    let mut value = base.clone();
    value["activation_generation"] = (-1).into();
    variants.push(value);
    let mut value = base.clone();
    value.as_object_mut().unwrap().remove("activated_at");
    variants.push(value);
    let mut value = base.clone();
    value["activated_at"] = "tomorrow".into();
    variants.push(value);
    let mut value = base.clone();
    value["provenance"] = serde_json::json!({"reviewed_real":{"dataset_version":"other","dataset_sha256":"0".repeat(64),"review_reference":"fictional-review"}});
    variants.push(value);
    for value in variants {
        fs::write(&pointer, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(matches!(
            candidate.store.read_active_integrity(),
            Err(ClassifierError::InvalidArtifact(_))
        ));
        fs::write(&pointer, &original).unwrap();
    }
    let immutable = candidate
        .store
        .model_dir(&candidate.manifest.model_id)
        .join(artifact::MODEL_MANIFEST_FILE);
    let immutable_bytes = fs::read(&immutable).unwrap();
    let mut changed = immutable_bytes.clone();
    changed.push(b' ');
    fs::write(&immutable, changed).unwrap();
    assert!(
        matches!(candidate.store.read_active_integrity(),Err(ClassifierError::InvalidArtifact(message)) if message.contains("checksum"))
    );
    let mut substitute: serde_json::Value = serde_json::from_slice(&immutable_bytes).unwrap();
    substitute["model_id"] = "different-immutable".into();
    let changed = serde_json::to_vec(&substitute).unwrap();
    fs::write(&immutable, &changed).unwrap();
    let mut overlay = base;
    overlay["manifest_sha256"] = artifact::sha256_hex(&changed).into();
    fs::write(&pointer, serde_json::to_vec(&overlay).unwrap()).unwrap();
    assert!(matches!(
        candidate.store.read_active_integrity(),
        Err(ClassifierError::InvalidArtifact(_))
    ));
    fs::write(&immutable, &immutable_bytes).unwrap();
    fs::write(&pointer, &original).unwrap();
    assert_eq!(
        candidate.store.read_active_integrity().unwrap(),
        Some(active)
    );
}

#[test]
fn numeric_parameters_require_real_binary_occurrence_counts() {
    let _serial = support::serial();
    let candidate = support::Candidate::from_fixture(support::trained());
    artifact::validate_artifact(
        &candidate.manifest,
        &candidate.payload,
        artifact::compiled_bundle_sha256(),
    )
    .unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&candidate.payload).unwrap();
    let weights = value["domains"]["governance"]["class_features"]["absent"]
        .as_object_mut()
        .unwrap();
    let key = weights.keys().next().unwrap().clone();
    weights[&key] = 2.5.into(); // strictly within the legitimate[2,7] support bounds
    let payload = value.to_string();
    let mut manifest = candidate.manifest.clone();
    manifest.model_sha256 = artifact::sha256_hex(payload.as_bytes());
    assert!(
        matches!(artifact::validate_artifact(&manifest,&payload,artifact::compiled_bundle_sha256()),Err(ClassifierError::InvalidArtifact(message)) if message.contains("governance")&&message.contains("fractional_feature_count"))
    );
    let mut manifest = candidate.manifest.clone();
    manifest.smoothing = 0.5;
    assert!(
        matches!(artifact::validate_artifact(&manifest,&candidate.payload,artifact::compiled_bundle_sha256()),Err(ClassifierError::InvalidArtifact(message)) if message.contains("smoothing"))
    );
}

fn named_candidate(name: &str) -> support::Candidate {
    let mut fixture = support::Fixture::new(name);
    fixture.fit(support::recipe()["rows"].clone());
    support::Candidate::from_fixture(fixture)
}

#[test]
fn explicit_manual_repair_preserves_legacy_identity_and_advances_generation() {
    let _serial = support::serial();
    let prior = support::Candidate::from_fixture(support::trained());
    let active = activate(&prior.store, &prior.fixture.lease, &prior, None);
    let legacy = support::install_legacy_prior(&prior, &active);
    assert!(matches!(
        prior.store.read_active(),
        Err(ClassifierError::Incompatible(_))
    ));
    let identity = prior.store.read_active_repair_identity().unwrap().unwrap();
    assert_eq!(identity.model_id(), &legacy.model_id);
    assert_eq!(identity.generation(), 1);
    assert_eq!(identity.manifest_sha256(), legacy.manifest_sha256);
    let next = named_candidate("repaired-v2");
    let validated = prior
        .store
        .write_candidate(
            &prior.fixture.lease,
            &next.manifest,
            &next.payload,
            &next.report,
        )
        .unwrap();
    let request = next.request(SyntheticActivation::ExplicitOptIn);
    let ordinary = authorize_activation(
        &prior.fixture.lease,
        &validated,
        &next.report,
        &request,
        None,
        None,
    )
    .unwrap();
    let ActivationAuthorizationDecision::Authorized(ordinary) = ordinary else {
        panic!("explicit fixture consent")
    };
    let pointer = fs::read(prior.store.active_manifest_path()).unwrap();
    assert!(matches!(
        prior
            .store
            .activate(&next.manifest.model_id, &ordinary, &prior.fixture.lease),
        Err(ClassifierError::Incompatible(_))
    ));
    assert_eq!(
        fs::read(prior.store.active_manifest_path()).unwrap(),
        pointer
    );
    assert!(matches!(
        prior.store.authorize_manual_repair(
            &prior.fixture.lease,
            &validated,
            &next.report,
            &next.request(SyntheticActivation::RetainPrior)
        ),
        Err(ClassifierError::ActivationDenied { .. })
    ));
    let old_directory = prior.store.model_dir(&prior.manifest.model_id);
    let old_files = [
        artifact::MODEL_PAYLOAD_FILE,
        artifact::MODEL_MANIFEST_FILE,
        artifact::MODEL_REPORT_FILE,
    ]
    .map(|name| (name, fs::read(old_directory.join(name)).unwrap()));
    let record_path = prior.store.root().join("activations/1.json");
    let index_path = prior
        .store
        .root()
        .join("activated-models")
        .join(format!("{}.json", legacy.model_id));
    let record = fs::read(&record_path).unwrap();
    let index = fs::read(&index_path).unwrap();
    fs::write(&index_path, b"{\"schema_version\":\"malformed\"}").unwrap();
    assert!(matches!(
        prior.store.authorize_manual_repair(
            &prior.fixture.lease,
            &validated,
            &next.report,
            &request
        ),
        Err(ClassifierError::InvalidArtifact(_))
    ));
    fs::write(&index_path, &index).unwrap();
    let decision = prior
        .store
        .authorize_manual_repair(&prior.fixture.lease, &validated, &next.report, &request)
        .unwrap();
    let ActivationAuthorizationDecision::Authorized(capability) = decision else {
        panic!("explicit repair consent")
    };
    let repaired = prior
        .store
        .activate(&next.manifest.model_id, &capability, &prior.fixture.lease)
        .unwrap();
    assert_eq!(repaired.activation_generation, 2);
    assert_eq!(repaired.previous_model_id, Some(legacy.model_id));
    assert_eq!(
        repaired.feature_schema_version,
        artifact::FEATURE_SCHEMA_VERSION
    );
    assert!(matches!(
        prior
            .store
            .activate(&next.manifest.model_id, &capability, &prior.fixture.lease),
        Err(ClassifierError::StaleActivation)
    ));
    for (name, bytes) in old_files {
        assert_eq!(fs::read(old_directory.join(name)).unwrap(), bytes);
    }
    assert_eq!(fs::read(record_path).unwrap(), record);
    assert_eq!(fs::read(index_path).unwrap(), index);
}

#[test]
fn missing_pointer_and_occupied_history_cannot_reset_or_replace_generations() {
    let _serial = support::serial();
    let prior = support::Candidate::from_fixture(support::trained());
    let active = activate(&prior.store, &prior.fixture.lease, &prior, None);
    let next = named_candidate("history-preservation");
    let validated = prior
        .store
        .write_candidate(
            &prior.fixture.lease,
            &next.manifest,
            &next.payload,
            &next.report,
        )
        .unwrap();
    let pointer_path = prior.store.active_manifest_path();
    let pointer = fs::read(&pointer_path).unwrap();
    let record_path = prior.store.root().join("activations/1.json");
    let record = fs::read(&record_path).unwrap();
    let index_path = prior
        .store
        .root()
        .join("activated-models")
        .join(format!("{}.json", active.model_id));
    let index = fs::read(&index_path).unwrap();
    fs::remove_file(&pointer_path).unwrap();
    assert!(
        matches!(prior.store.read_active(),Err(ClassifierError::InvalidArtifact(message)) if message.contains("recovery_required"))
    );
    assert!(matches!(
        prior.store.read_active_repair_identity(),
        Err(ClassifierError::InvalidArtifact(_))
    ));
    assert!(matches!(
        prior.store.authorize_manual_repair(
            &prior.fixture.lease,
            &validated,
            &next.report,
            &next.request(SyntheticActivation::ExplicitOptIn)
        ),
        Err(ClassifierError::InvalidArtifact(_))
    ));
    let decision = authorize_activation(
        &prior.fixture.lease,
        &validated,
        &next.report,
        &next.request(SyntheticActivation::ExplicitOptIn),
        None,
        None,
    )
    .unwrap();
    let ActivationAuthorizationDecision::Authorized(capability) = decision else {
        panic!("explicit fixture consent")
    };
    assert!(matches!(
        prior
            .store
            .activate(&next.manifest.model_id, &capability, &prior.fixture.lease),
        Err(ClassifierError::InvalidArtifact(_))
    ));
    assert!(!pointer_path.exists());
    assert_eq!(fs::read(&record_path).unwrap(), record);
    assert_eq!(fs::read(&index_path).unwrap(), index);
    fs::write(&pointer_path, &pointer).unwrap();
    let future_path = prior.store.root().join("activations/2.json");
    let future = b"pre-existing future history bytes";
    fs::write(&future_path, future).unwrap();
    let decision = authorize_activation(
        &prior.fixture.lease,
        &validated,
        &next.report,
        &next.request(SyntheticActivation::ExplicitOptIn),
        None,
        Some(&active),
    )
    .unwrap();
    let ActivationAuthorizationDecision::Authorized(capability) = decision else {
        panic!("explicit fixture consent")
    };
    assert!(
        matches!(prior.store.activate(&next.manifest.model_id,&capability,&prior.fixture.lease),Err(ClassifierError::InvalidArtifact(message)) if message.contains("recovery_required"))
    );
    assert!(
        matches!(prior.store.rollback_to(&active.model_id,&prior.fixture.lease),Err(ClassifierError::InvalidArtifact(message)) if message.contains("recovery_required"))
    );
    let decision = prior
        .store
        .authorize_manual_repair(
            &prior.fixture.lease,
            &validated,
            &next.report,
            &next.request(SyntheticActivation::ExplicitOptIn),
        )
        .unwrap();
    let ActivationAuthorizationDecision::Authorized(capability) = decision else {
        panic!("explicit repair consent")
    };
    assert!(
        matches!(prior.store.activate(&next.manifest.model_id,&capability,&prior.fixture.lease),Err(ClassifierError::InvalidArtifact(message)) if message.contains("recovery_required"))
    );
    assert_eq!(fs::read(future_path).unwrap(), future);
    assert_eq!(fs::read(pointer_path).unwrap(), pointer);
    assert_eq!(fs::read(record_path).unwrap(), record);
    assert_eq!(fs::read(index_path).unwrap(), index);
}
