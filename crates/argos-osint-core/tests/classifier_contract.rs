//! Public classifier contracts exercised with real QuickJS and a generated upstream oracle.
#[cfg(feature = "classifier")]
#[path = "fixtures/classifier/support.rs"]
mod support;
use argos_osint_core::atlas_taxonomy::{DomainId, TAXONOMY_VERSION};
use argos_osint_core::classifier::ClassifyInput;
#[cfg(not(feature = "classifier"))]
use argos_osint_core::classifier::{ClassifierError, ClassifierService, ClassifierStatus};

#[test]
fn input_hash_is_length_framed_and_content_only() {
    let a = ClassifyInput::new("a", "ab", "c", Some("en".into()));
    let b = ClassifyInput::new("b", "a", "bc", Some("fr".into()));
    assert_ne!(a.input_hash, b.input_hash);
    let same = ClassifyInput::new("different", "ab", "c", Some("fr".into()));
    assert_eq!(a.input_hash, same.input_hash);
}
#[test]
fn taxonomy_and_binary_models_are_exact_and_ordered() {
    assert_eq!(TAXONOMY_VERSION, "strategic-domains-v1");
    assert_eq!(
        DomainId::ALL.map(DomainId::as_str),
        [
            "governance",
            "security",
            "economic",
            "human",
            "environmental",
            "scientific"
        ]
    );
    let names = [
        "Politics & Governance",
        "Security & Conflict",
        "Economy & Supply Chains",
        "Health & Public Welfare",
        "Environment & Resources",
        "Science & Technology",
    ];
    let manifest = argos_osint_core::atlas_taxonomy::taxonomy_manifest();
    assert_eq!(manifest["taxonomy_version"], TAXONOMY_VERSION);
    for (index, domain) in DomainId::ALL.iter().enumerate() {
        assert_eq!(domain.display_name(), names[index]);
        assert_eq!(manifest["domains"][index]["name"], names[index]);
        assert_eq!(manifest["domains"][index]["id"], domain.as_str());
    }
    for legacy in ["unk", "Governance", "politics", "conflict"] {
        assert!(DomainId::parse(legacy).is_none());
    }
    #[cfg(feature = "classifier")]
    {
        let _serial = support::serial();
        let mut fixture = support::trained();
        let payload = fixture.payload();
        assert_eq!(payload["domains"].as_object().unwrap().len(), 6);
        for domain in DomainId::ALL {
            let model = &payload["domains"][domain.as_str()];
            assert_eq!(model["class_totals"]["present"], 2); // one example plus upstream prior
            assert_eq!(model["class_totals"]["absent"], 7);
            assert_eq!(model["class_features"].as_object().unwrap().len(), 2);
        }
    }
}
#[cfg(feature = "classifier")]
#[test]
fn upstream_golden_matches_quickjs_scores_and_features() {
    let _serial = support::serial();
    let mut fixture = support::trained();
    let golden: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/classifier/bayes-parity.json")).unwrap();
    let identity = argos_osint_core::classifier::compiled_identity().unwrap();
    assert_eq!(
        golden["source_manifest_sha256"],
        identity.source_manifest_sha256
    );
    assert_eq!(golden["bundle_sha256"], identity.bundle_sha256);
    let tolerance = golden["tolerance"].as_f64().unwrap();
    assert!(tolerance <= 1e-9);
    let payload = fixture.payload();
    for (idx, domain) in DomainId::ALL.iter().enumerate() {
        let oracle = &golden["models"][idx];
        assert_eq!(oracle["domain"], domain.as_str());
        assert_eq!(
            payload["domains"][domain.as_str()]["feature_order"],
            oracle["feature_order"]
        );
        for query in oracle["queries"].as_array().unwrap() {
            let actual = fixture.classify(query["text"].as_str().unwrap());
            for field in ["present_log_score", "absent_log_score", "raw_log_odds"] {
                let got = actual[idx][field].as_f64().unwrap();
                let expected = query[field].as_f64().unwrap();
                assert!(
                    got.is_finite() && (got - expected).abs() <= tolerance,
                    "{domain:?}: {field}: got {got}, expected {expected}"
                );
            }
        }
    }
}
#[cfg(feature = "classifier")]
#[test]
fn unknown_memberships_do_not_update_the_masked_model() {
    use argos_osint_core::classifier::BridgeOperation;
    let _serial = support::serial();
    let mut fixture = support::trained();
    let before = fixture.payload();
    fixture.fit(serde_json::json!([{"row_id":"masked","title":"quarantine-sentinel","description":"","membership":["Present","Unknown","Unknown","Unknown","Unknown","Unknown"]}]));
    let after = fixture.payload();
    assert_eq!(after["domains"]["governance"]["class_totals"]["present"], 3);
    for domain in &DomainId::ALL[1..] {
        assert_eq!(
            before["domains"][domain.as_str()],
            after["domains"][domain.as_str()]
        );
    }
    fixture.call(BridgeOperation::TrainBatch,serde_json::json!({"rows":[{"row_id":"ignored-row","title":"ignored-token","description":"","membership":["Unknown","Unknown","Unknown","Unknown","Unknown","Unknown"]}]}));
    assert_eq!(fixture.payload(), after);
}
#[cfg(feature = "classifier")]
#[test]
fn repetition_is_binary_presence_not_token_weight() {
    let _serial = support::serial();
    let mut fixture = support::trained();
    let once = fixture.classify("ballot patrol common");
    let repeated = fixture.classify("ballot ballot patrol patrol common");
    for (once, repeated) in once
        .as_array()
        .unwrap()
        .iter()
        .zip(repeated.as_array().unwrap())
    {
        for field in ["present_log_score", "absent_log_score", "raw_log_odds"] {
            assert_eq!(
                once[field], repeated[field],
                "binary feature repetition must not weight scores"
            );
        }
    }
}
#[cfg(feature = "classifier")]
#[test]
fn save_restore_preserves_order_and_raw_scores() {
    use argos_osint_core::classifier::BridgeOperation;
    let _serial = support::serial();
    let mut original = support::trained();
    let payload = original.payload();
    let expected = original.classify("ballot patrol common");
    let mut restored = support::Fixture::new("restored");
    restored.call(
        BridgeOperation::RestoreModel,
        serde_json::json!({"artifact":payload}),
    );
    assert_eq!(restored.payload(), payload);
    assert_eq!(restored.classify("ballot patrol common"), expected);
}
#[cfg(not(feature = "classifier"))]
#[test]
fn feature_off_facade_is_callable_and_unavailable() {
    let service = ClassifierService::default();
    assert_eq!(service.status(), ClassifierStatus::Missing);
    assert!(matches!(
        service.classify_batch(vec![]),
        Err(ClassifierError::Unavailable)
    ));
    assert!(matches!(
        service.classify_batch(vec![
            ClassifyInput::new("off", "title", "description", None);
            65
        ]),
        Err(ClassifierError::Unavailable)
    ));
    assert_eq!(
        argos_osint_core::classifier::status_for_model(None, true, false),
        ClassifierStatus::Unavailable
    );
    let home = tempfile::tempdir().unwrap();
    let store = argos_osint_core::classifier::artifact::ArtifactStore::new(home.path()).unwrap();
    let lease = std::sync::Arc::new(store.try_writer().unwrap());
    assert!(matches!(
        argos_osint_core::classifier::start_candidate_worker(
            &argos_osint_core::classifier::ModelId::parse("off").unwrap(),
            lease,
            &argos_osint_core::classifier::WorkControl::new()
        ),
        Err(ClassifierError::Unavailable)
    ));
    assert_eq!(service.performance(), Err(ClassifierError::Unavailable));
}

#[cfg(feature = "classifier")]
#[test]
fn late_numeric_features_append_per_model_without_reindexing() {
    let _serial = support::serial();
    let mut fixture = support::trained();
    let before = fixture.payload();
    let old_order = before["domains"]["governance"]["feature_order"]
        .as_array()
        .unwrap();
    fixture.fit(serde_json::json!([{"row_id":"late","title":"0 lateword","description":"","membership":["Present","Unknown","Unknown","Unknown","Unknown","Unknown"]}]));
    let after = fixture.payload();
    let new_order = after["domains"]["governance"]["feature_order"]
        .as_array()
        .unwrap();
    assert_eq!(&new_order[..old_order.len()], old_order.as_slice());
    assert!(new_order[old_order.len()..].contains(&serde_json::json!("u:\"0\"")));
    for domain in &DomainId::ALL[1..] {
        assert_eq!(
            before["domains"][domain.as_str()],
            after["domains"][domain.as_str()]
        );
    }
    use argos_osint_core::classifier::BridgeOperation;
    fixture.call(
        BridgeOperation::FinishTraining,
        serde_json::json!({"max_documents_per_step":16}),
    );
    assert_eq!(fixture.payload(), after, "completed finish must be a no-op");
}

#[cfg(feature = "classifier")]
#[test]
fn inference_accepts_64_and_rejects_65_before_dispatch() {
    use argos_osint_core::classifier::{ClassifierError, ClassifierService, ModelSnapshot};
    let _serial = support::serial();
    let candidate = support::Candidate::from_fixture(support::trained());
    let artifact = candidate.write();
    let snapshot = ModelSnapshot::new(
        candidate.fixture.model.as_str(),
        candidate.manifest.model_sha256.clone(),
        candidate.manifest.provenance.clone(),
    );
    let service = ClassifierService::from_snapshot(
        snapshot,
        artifact,
        Box::new(candidate.report.gate_set.clone()),
    )
    .unwrap();
    let inputs: Vec<_> = (0..64)
        .map(|idx| ClassifyInput::new(format!("occ-{idx}"), "ballot", "patrol", Some("en".into())))
        .collect();
    let receipts = service.classify_batch(inputs).unwrap();
    assert_eq!(receipts.len(), 64);
    for (idx, receipt) in receipts.iter().enumerate() {
        assert_eq!(receipt.occurrence_id, format!("occ-{idx}"));
        receipt.validate().unwrap();
    }
    let oversized = (0..65)
        .map(|idx| ClassifyInput::new(format!("occ-{idx}"), "ballot", "patrol", Some("en".into())))
        .collect();
    assert!(matches!(
        service.classify_batch(oversized),
        Err(ClassifierError::BatchTooLarge)
    ));
}

#[cfg(feature = "classifier")]
#[test]
fn long_oov_and_kept_tokens_remain_finite_without_feature_leakage() {
    let _serial = support::serial();
    let mut fixture = support::trained();
    let payload = fixture.payload();
    let order = payload["domains"]["governance"]["feature_order"]
        .as_array()
        .unwrap();
    for token in ["cannot", "never", "nato", "2026", "1", "2"] {
        assert!(
            order.contains(&serde_json::json!(format!(
                "u:{}",
                serde_json::to_string(token).unwrap()
            ))),
            "missing {token}"
        );
    }
    for token in ["p-governance", "present", "absent", "train", "fixture-m1"] {
        assert!(
            !order.contains(&serde_json::json!(format!(
                "u:{}",
                serde_json::to_string(token).unwrap()
            ))),
            "leaked {token}"
        );
    }
    let scores = fixture.classify(&"ballot patrol common zzqxjv ".repeat(128));
    for score in scores.as_array().unwrap() {
        for field in ["present_log_score", "absent_log_score", "raw_log_odds"] {
            assert!(score[field].as_f64().unwrap().is_finite());
        }
    }
    let oov = fixture.classify("zzqxjv");
    for score in oov.as_array().unwrap() {
        assert_eq!(score["known_feature_count"], 0);
    }
}

#[test]
fn top_two_independent_gates_cover_admission_edges() {
    use argos_osint_core::classifier::calibration::{
        self, CalibrationKnot, DomainCalibration, GateSet, CALIBRATION_VERSION,
    };
    use argos_osint_core::classifier::{ModelId, RawDomainScore, RejectionReason};
    let model = ModelId::parse("policy-fixture").unwrap();
    for (name, relevances, thresholds, expected) in [
        ("two", [0.95, 0.90, 0.1, 0.1, 0.1, 0.1], [0.8; 6], true),
        (
            "weak-second",
            [0.95, 0.79, 0.1, 0.1, 0.1, 0.1],
            [0.8; 6],
            true,
        ),
        ("three", [0.95, 0.90, 0.85, 0.1, 0.1, 0.1], [0.8; 6], true),
        ("inclusive", [0.8, 0.8, 0.1, 0.1, 0.1, 0.1], [0.8; 6], true),
        (
            "no-skip",
            [0.95, 0.90, 0.85, 0.1, 0.1, 0.1],
            [0.99, 0.8, 0.8, 0.8, 0.8, 0.8],
            true,
        ),
    ] {
        let gates = GateSet {
            version: CALIBRATION_VERSION.into(),
            domains: core::array::from_fn(|idx| DomainCalibration {
                sigmoid: None,
                domain: DomainId::ALL[idx],
                knots: vec![CalibrationKnot {
                    raw_upper: 1.0,
                    relevance: relevances[idx],
                }],
                threshold: thresholds[idx],
                min_informative_features: 1,
            }),
        };
        let raw = core::array::from_fn(|idx| RawDomainScore {
            domain: DomainId::ALL[idx],
            present_log_score: 1.0,
            absent_log_score: 0.0,
            raw_log_odds: 1.0,
            known_feature_count: 1,
            oov_count: 0,
        });
        let input = ClassifyInput::new(name, "Headline", "Description", Some("en".into()));
        let receipt = calibration::apply_gates(&input, &raw, &model, &gates).unwrap();
        assert_eq!(receipt.is_admitted(), expected, "{name}");
        receipt.validate().unwrap();
        if !expected {
            assert_eq!(
                receipt.rejection_reason,
                Some(RejectionReason::InsufficientDomains)
            );
        }
    }
}

#[test]
fn receipt_versions_diagnostics_and_occurrence_identity_survive_roundtrip() {
    use argos_osint_core::classifier::calibration::{
        self, CalibrationKnot, DomainCalibration, GateSet, CALIBRATION_VERSION,
    };
    use argos_osint_core::classifier::{ModelId, RawDomainScore};
    let gates = GateSet {
        version: CALIBRATION_VERSION.into(),
        domains: DomainId::ALL.map(|domain| DomainCalibration {
            sigmoid: None,
            domain,
            knots: vec![CalibrationKnot {
                raw_upper: 1.0,
                relevance: if matches!(domain, DomainId::Economic | DomainId::Environmental) {
                    0.9
                } else {
                    0.1
                },
            }],
            threshold: 0.8,
            min_informative_features: 1,
        }),
    };
    let raw = core::array::from_fn(|idx| RawDomainScore {
        domain: DomainId::ALL[idx],
        present_log_score: 1.0,
        absent_log_score: 0.0,
        raw_log_odds: 1.0,
        known_feature_count: 1,
        oov_count: 0,
    });
    let model = ModelId::parse("receipt-model").unwrap();
    let mut previous_hash = None;
    for id in ["first-occurrence", "second-occurrence"] {
        let input = ClassifyInput::new(id, "Same headline", "Same description", Some("en".into()));
        let receipt = calibration::apply_gates(&input, &raw, &model, &gates).unwrap();
        assert_eq!(
            receipt.selected_domains,
            vec![DomainId::Economic, DomainId::Environmental]
        );
        assert_eq!(receipt.occurrence_id, id);
        assert_eq!(receipt.receipt_schema_version, "classifier-receipt-v2");
        assert!(
            receipt
                .scores
                .iter()
                .map(|s| s.calibrated_score)
                .sum::<f64>()
                > 1.0
        );
        receipt.validate().unwrap();
        let json = serde_json::to_value(&receipt).unwrap();
        let decoded: argos_osint_core::classifier::ClassificationReceipt =
            serde_json::from_value(json.clone()).unwrap();
        assert_eq!(receipt, decoded);
        if let Some(hash) = previous_hash {
            assert_eq!(receipt.input_hash, hash);
        }
        previous_hash = Some(receipt.input_hash.clone());
        let mut legacy = json;
        legacy
            .as_object_mut()
            .unwrap()
            .remove("receipt_schema_version");
        assert!(
            serde_json::from_value::<argos_osint_core::classifier::ClassificationReceipt>(legacy)
                .is_err()
        );
    }
}

#[cfg(feature = "classifier")]
#[test]
fn mutation_request_replays_do_not_refit_or_accept_changed_payloads() {
    use argos_osint_core::classifier::{BridgeOperation, ClassifierError};
    let _serial = support::serial();
    let mut fixture = support::trained();
    let request = serde_json::json!({"version":"1","request_id":"replay-mutation","model_id":fixture.model,
        "rows":[{"row_id":"replay-row","title":"novelword","description":"","membership":["Present","Unknown","Unknown","Unknown","Unknown","Unknown"]}]});
    let first = fixture
        .worker
        .call(
            BridgeOperation::TrainBatch,
            &request.to_string(),
            &fixture.control,
        )
        .unwrap();
    let pending = fixture.worker.call(
        BridgeOperation::SerializeModel,
        &serde_json::json!({"version":"1","request_id":"pending-serialization","model_id":fixture.model}).to_string(),
        &fixture.control,
    ).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&pending).unwrap()["error"],
        "training_round_pending"
    );
    let finished = fixture.call(
        BridgeOperation::FinishTraining,
        serde_json::json!({"max_documents_per_step":16}),
    );
    assert_eq!(finished["done"], true);
    let after_first = fixture.payload();
    let replay = fixture
        .worker
        .call(
            BridgeOperation::TrainBatch,
            &request.to_string(),
            &fixture.control,
        )
        .unwrap();
    assert_eq!(first, replay);
    assert_eq!(fixture.payload(), after_first);
    let mut changed = request;
    changed["rows"][0]["title"] = "differentword".into();
    assert!(matches!(
        fixture.worker.call(
            BridgeOperation::TrainBatch,
            &changed.to_string(),
            &fixture.control
        ),
        Err(ClassifierError::BridgeProtocol { .. })
    ));
}

#[cfg(feature = "classifier")]
#[test]
fn finish_budget_is_sixteen_total_and_reports_real_monotonic_cursors() {
    use argos_osint_core::classifier::BridgeOperation;
    let _serial = support::serial();
    let mut fixture = support::Fixture::new("bounded-fit");
    let rows = (0..32).map(|index| serde_json::json!({
        "row_id":format!("row-{index}"), "title":format!("ballot policy token{index}"), "description":"",
        "membership":["Present","Absent","Present","Absent","Present","Absent"]
    })).collect::<Vec<_>>();
    let batch = fixture.call(
        BridgeOperation::TrainBatch,
        serde_json::json!({"rows":rows}),
    );
    assert_eq!(batch["round"], 1);
    assert_eq!(
        batch["fitted_memberships"],
        serde_json::json!([0, 0, 0, 0, 0, 0])
    );
    let mut prior = [0u64; 6];
    let mut calls = 0;
    loop {
        calls += 1;
        let finish = fixture.call(
            BridgeOperation::FinishTraining,
            serde_json::json!({"max_documents_per_step":16}),
        );
        let next: [u64; 6] = serde_json::from_value(finish["fitted_memberships"].clone()).unwrap();
        assert_eq!(
            next.iter()
                .zip(prior)
                .map(|(value, before)| value - before)
                .sum::<u64>(),
            16
        );
        assert_eq!(finish["round"], 1);
        assert_eq!(finish["fit_step"], calls);
        let sizes: [u64; 6] = serde_json::from_value(finish["vocabulary_sizes"].clone()).unwrap();
        assert_eq!(finish["frozen_order_length"], sizes.iter().sum::<u64>());
        assert_eq!(
            finish["feature_order_revisions"],
            batch["feature_order_revisions"]
        );
        prior = next;
        if finish["done"] == true {
            break;
        }
        assert!(calls < 13);
    }
    assert_eq!(calls, 12);
    assert_eq!(prior, [32; 6]);
    let noop = fixture.call(
        BridgeOperation::FinishTraining,
        serde_json::json!({"max_documents_per_step":16}),
    );
    assert_eq!(noop["fit_step"], calls);
    assert_eq!(
        noop["fitted_memberships"],
        serde_json::json!([32, 32, 32, 32, 32, 32])
    );
}

#[cfg(feature = "classifier")]
#[test]
fn raw_mutation_control_ids_fail_before_mutation_and_printable_max_ids_work() {
    use argos_osint_core::classifier::{BridgeOperation, ClassifierError};
    let _serial = support::serial();
    let mut fixture = support::Fixture::new("valid-identity");
    for id in ["bad\0id", "bad\nid", "bad\u{0085}id"] {
        let request = serde_json::json!({"version":"1","model_id":fixture.model.as_str(),"request_id":id,"rows":[]});
        assert!(matches!(
            fixture.worker.call(
                BridgeOperation::TrainBatch,
                &request.to_string(),
                &fixture.control
            ),
            Err(ClassifierError::BridgeProtocol { .. })
        ));
    }
    let id = "\"\\".repeat(128);
    let request=serde_json::json!({"version":"1","model_id":fixture.model.as_str(),"request_id":id,"rows":[]}).to_string();
    let first = fixture
        .worker
        .call(BridgeOperation::TrainBatch, &request, &fixture.control)
        .unwrap();
    assert!(first.len() <= 1024);
    let same = fixture
        .worker
        .call(BridgeOperation::TrainBatch, &request, &fixture.control)
        .unwrap();
    assert_eq!(first, same);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&first).unwrap()["round"],
        1
    );
    let next = fixture.call(BridgeOperation::TrainBatch, serde_json::json!({"rows":[]}));
    assert_eq!(next["round"], 2);
}

#[test]
fn calibrated_order_is_independent_of_raw_ranking_and_receipts_reject_corruption() {
    use argos_osint_core::classifier::calibration::{
        self, CalibrationKnot, DomainCalibration, GateSet, CALIBRATION_VERSION,
    };
    use argos_osint_core::classifier::{ModelId, RawDomainScore, RejectionReason};
    let raw = core::array::from_fn(|index| RawDomainScore {
        domain: DomainId::ALL[index],
        present_log_score: (6 - index) as f64,
        absent_log_score: 0.0,
        raw_log_odds: (6 - index) as f64,
        known_feature_count: 1,
        oov_count: 2,
    });
    let gates = GateSet {
        version: CALIBRATION_VERSION.into(),
        domains: DomainId::ALL.map(|domain| DomainCalibration {
            sigmoid: None,
            domain,
            knots: vec![CalibrationKnot {
                raw_upper: 10.0,
                relevance: if matches!(domain, DomainId::Environmental | DomainId::Scientific) {
                    0.95
                } else {
                    0.1
                },
            }],
            threshold: 0.8,
            min_informative_features: 1,
        }),
    };
    let input = ClassifyInput::new("occurrence", "Fictional evidence", "", Some("en".into()));
    let receipt = calibration::apply_gates(
        &input,
        &raw,
        &ModelId::parse("receipt-proof").unwrap(),
        &gates,
    )
    .unwrap();
    assert!(raw[0].raw_log_odds > raw[4].raw_log_odds);
    assert_eq!(
        receipt.selected_domains,
        vec![DomainId::Environmental, DomainId::Scientific]
    );
    receipt.validate().unwrap();
    let mut corruptions = Vec::new();
    let mut bad = receipt.clone();
    bad.scores.swap(0, 1);
    corruptions.push(bad);
    let mut bad = receipt.clone();
    bad.scores[0].raw_log_odds = f64::NAN;
    corruptions.push(bad);
    let mut bad = receipt.clone();
    bad.scores[0].threshold = f64::INFINITY;
    corruptions.push(bad);
    let mut bad = receipt.clone();
    bad.scores[0].calibrated_score = -0.1;
    corruptions.push(bad);
    let mut bad = receipt.clone();
    bad.scores[0].passes_gate = true;
    corruptions.push(bad);
    let mut bad = receipt.clone();
    bad.selected_domains = vec![DomainId::Governance, DomainId::Security];
    corruptions.push(bad);
    let mut bad = receipt.clone();
    bad.rejection_reason = Some(RejectionReason::InsufficientDomains);
    corruptions.push(bad);
    let mut bad = receipt.clone();
    bad.selected_domains.clear();
    bad.rejection_reason = None;
    corruptions.push(bad);
    let mut bad = receipt.clone();
    bad.model_id = "../escape".into();
    corruptions.push(bad);
    let mut bad = receipt.clone();
    bad.input_hash = "A".repeat(64);
    corruptions.push(bad);
    let mut bad = receipt.clone();
    bad.evaluated_at = "yesterday".into();
    corruptions.push(bad);
    let mut bad = receipt.clone();
    bad.raw_diagnostics.as_mut().unwrap().domains[0].known_feature_count += 1;
    corruptions.push(bad);
    let mut bad = receipt.clone();
    bad.raw_diagnostics.as_mut().unwrap().domains[0].oov_count += 1;
    corruptions.push(bad);
    let mut bad = receipt.clone();
    bad.raw_diagnostics.as_mut().unwrap().domains[0].absent_log_score = 2.0;
    corruptions.push(bad);
    let mut bad = receipt.clone();
    bad.raw_diagnostics.as_mut().unwrap().calibration_version = "legacy".into();
    corruptions.push(bad);
    for (index, bad) in corruptions.into_iter().enumerate() {
        assert!(bad.validate().is_err(), "corruption {index}");
    }
}

#[test]
fn typed_rejection_priority_and_informative_gates_match_generated_diagnostics() {
    use argos_osint_core::classifier::calibration::{
        self, CalibrationKnot, DomainCalibration, GateSet, CALIBRATION_VERSION,
    };
    use argos_osint_core::classifier::{ModelId, RawDomainScore, RejectionReason};
    let model = ModelId::parse("rejection-proof").unwrap();
    let gates = GateSet {
        version: CALIBRATION_VERSION.into(),
        domains: DomainId::ALL.map(|domain| DomainCalibration {
            sigmoid: None,
            domain,
            knots: vec![CalibrationKnot {
                raw_upper: 10.0,
                relevance: if matches!(domain, DomainId::Governance | DomainId::Security) {
                    0.95
                } else {
                    0.1
                },
            }],
            threshold: 0.8,
            min_informative_features: 1,
        }),
    };
    for (title, language, known, expected) in [
        (" \t", "fr", [1usize; 6], RejectionReason::Blank),
        (
            "Fictional headline",
            "fr",
            [1; 6],
            RejectionReason::UnsupportedLanguage,
        ),
        (
            "Wholly unseen text",
            "en",
            [0; 6],
            RejectionReason::WhollyOov,
        ),
        (
            "Qualifying gates lack discriminative support",
            "en",
            [0, 0, 1, 1, 1, 1],
            RejectionReason::OutOfScope,
        ),
    ] {
        let input = ClassifyInput::new("rejected", title, "", Some(language.into()));
        let raw = core::array::from_fn(|index| RawDomainScore {
            domain: DomainId::ALL[index],
            present_log_score: 1.0,
            absent_log_score: 0.0,
            raw_log_odds: 1.0,
            known_feature_count: known[index],
            oov_count: 3,
        });
        let receipt = calibration::apply_gates(&input, &raw, &model, &gates).unwrap();
        assert_eq!(receipt.rejection_reason, Some(expected));
        assert!(receipt.selected_domains.is_empty());
        receipt.validate().unwrap();
        let diagnostics = receipt.raw_diagnostics.as_ref().unwrap();
        for (index, score) in receipt.scores.iter().enumerate() {
            assert_eq!(score.domain, DomainId::ALL[index]);
            assert_eq!(
                score.passes_gate,
                score.calibrated_score >= score.threshold && known[index] >= 1
            );
            assert_eq!(
                score.informative_features,
                diagnostics.domains[index].known_feature_count
            );
            assert_eq!(score.oov_features, diagnostics.domains[index].oov_count);
        }
    }
    let input = ClassifyInput::new(
        "hard-negative",
        "Knitting instructions with known words",
        "",
        Some("en".into()),
    );
    let raw = core::array::from_fn(|index| RawDomainScore {
        domain: DomainId::ALL[index],
        present_log_score: 0.0,
        absent_log_score: 1.0,
        raw_log_odds: -1.0,
        known_feature_count: 1,
        oov_count: 0,
    });
    let mut failed = gates;
    for domain in &mut failed.domains {
        domain.threshold = 2.0;
    }
    let receipt = calibration::apply_gates(&input, &raw, &model, &failed).unwrap();
    assert_eq!(receipt.rejection_reason, Some(RejectionReason::OutOfScope));
    receipt.validate().unwrap();
    let serialized = serde_json::to_string(&RejectionReason::OutOfScope).unwrap();
    assert_eq!(
        serde_json::from_str::<RejectionReason>(&serialized).unwrap(),
        RejectionReason::OutOfScope
    );
}

#[cfg(feature = "classifier")]
#[test]
fn stemmed_function_word_phrases_cannot_create_training_support() {
    use argos_osint_core::classifier::BridgeOperation;
    let _serial = support::serial();
    let mut fixture = support::Fixture::new("neutral-phrase-regression");
    let rows:Vec<_>=["This was the","are the","was in","the and of","those are the","it was in the"].iter().enumerate().map(|(index,title)|serde_json::json!({"row_id":format!("neutral-{index}"),"title":title,"description":"","membership":vec![if index%2==0 {"Present"}else{"Absent"};6]})).collect();
    let result = fixture.call(
        BridgeOperation::TrainBatch,
        serde_json::json!({"rows":rows}),
    );
    assert_eq!(result["accepted"], 0);
    assert_eq!(result["skipped"], 6);
    assert_eq!(
        result["vocabulary_sizes"],
        serde_json::json!([0, 0, 0, 0, 0, 0])
    );
    assert_eq!(
        result["fitted_memberships"],
        serde_json::json!([0, 0, 0, 0, 0, 0])
    );
}

#[cfg(feature = "classifier")]
#[test]
fn phrase_support_stops_at_hard_line_boundaries() {
    use argos_osint_core::classifier::BridgeOperation;
    let _serial = support::serial();
    let mut fixture = support::Fixture::new("phrase-line-boundary");
    fixture.fit(serde_json::json!([
        {"row_id":"positive","title":"armed strike","description":"","membership":["Present","Present","Present","Present","Present","Present"]},
        {"row_id":"negative","title":"culinary pastry","description":"","membership":["Absent","Absent","Absent","Absent","Absent","Absent"]}
    ]));
    let result = fixture.call(
        BridgeOperation::ClassifyBatch,
        serde_json::json!({"items":[
            {"id":"space","title":"armed strike","description":"","explain":true},
            {"id":"line","title":"armed\nstrike","description":"","explain":true}
        ]}),
    );
    assert_eq!(
        result["items"][0]["results"][0]["support"]["active_phrases"],
        1
    );
    assert_eq!(
        result["items"][1]["results"][0]["support"]["active_phrases"],
        0
    );
}
