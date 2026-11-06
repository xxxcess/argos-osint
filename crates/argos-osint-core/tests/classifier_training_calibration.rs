//! Behavioral regressions for independent-family calibration and training policy.
use argos_osint_core::atlas_taxonomy::DomainId;
use argos_osint_core::classifier::artifact::ModelId;
use argos_osint_core::classifier::calibration::{
    self, CalibrationKnot, CalibrationPolicy, DomainCalibration, GateSet, ScoredFamily,
    CALIBRATION_VERSION,
};
use argos_osint_core::classifier::training::{
    self, DatasetRow, FamilyId, LabelMembership, Split, SyntheticActivation, TrainRequest,
    TrainingCorpus,
};
use argos_osint_core::classifier::{
    BridgeOperation, ClassifierError, ClassifierWorker, ClassifyInput, Provenance, RawDomainScore,
    WorkControl,
};

fn raw(odds: [f64; 6]) -> [RawDomainScore; 6] {
    core::array::from_fn(|idx| RawDomainScore {
        domain: DomainId::ALL[idx],
        present_log_score: odds[idx],
        absent_log_score: 0.0,
        raw_log_odds: odds[idx],
        known_feature_count: 1,
        oov_count: 0,
    })
}
fn gates() -> GateSet {
    GateSet {
        version: CALIBRATION_VERSION.into(),
        domains: DomainId::ALL.map(|domain| DomainCalibration {
            domain,
            knots: vec![
                CalibrationKnot {
                    raw_upper: 0.0,
                    relevance: 0.0,
                },
                CalibrationKnot {
                    raw_upper: 10.0,
                    relevance: 1.0,
                },
            ],
            threshold: 0.5,
            min_informative_features: 1,
        }),
    }
}
fn family(id: &str, truth: &[usize], odds: [f64; 6]) -> ScoredFamily {
    ScoredFamily {
        family_id: FamilyId::parse(id).unwrap(),
        representative_id: id.into(),
        split: Split::Calibration,
        provenance: Provenance::synthetic_baseline("fixture-v1", "0".repeat(64)),
        membership: core::array::from_fn(|idx| {
            if truth.contains(&idx) {
                LabelMembership::Present
            } else {
                LabelMembership::Absent
            }
        }),
        raw: raw(odds),
    }
}

#[test]
fn evaluation_reports_actual_pairs_false_admissions_and_coverage() {
    let rows = [
        family("pair", &[0, 1], [1.0, 1.0, -1.0, -1.0, -1.0, -1.0]),
        family("oos", &[], [1.0, 1.0, -1.0, -1.0, -1.0, -1.0]),
        family("single", &[0], [1.0, 1.0, -1.0, -1.0, -1.0, -1.0]),
        family("rejected", &[], [-1.0; 6]),
    ];
    let metrics = calibration::evaluate(&rows, &gates()).unwrap();
    assert_eq!(metrics.complete_families, 4);
    assert_eq!(metrics.exact_pair_families, 1);
    assert_eq!(metrics.out_of_scope_families, 2);
    assert_eq!(metrics.single_domain_families, 1);
    assert_eq!(metrics.false_oos_admissions, 1);
    assert_eq!(metrics.false_single_domain_admissions, 1);
    assert_eq!(metrics.invalid_selected_pairs, 2);
    assert_eq!(metrics.coverage, 0.75);
    assert_eq!(metrics.domains[0].true_positive_families, 2);
    assert_eq!(metrics.domains[0].false_positive_families, 1);
}

#[test]
fn repeated_variants_and_partial_labels_cannot_inflate_calibration_evidence() {
    let row = family("same-family", &[0, 1], [1.0; 6]);
    assert!(calibration::fit(&[row.clone(), row.clone()], &CalibrationPolicy).is_err());
    assert!(calibration::evaluate(&[row.clone(), row.clone()], &gates()).is_err());
    let mut partial = row;
    partial.membership[5] = LabelMembership::Unknown;
    assert!(calibration::fit(&[partial], &CalibrationPolicy).is_err());
}

#[test]
fn corrupt_calibration_and_raw_scores_are_errors_instead_of_rejection_receipts() {
    let input = ClassifyInput::new("occ", "Headline", "Description", Some("en".into()));
    let model = ModelId::parse("fixture").unwrap();
    let mut scores = raw([1.0; 6]);
    scores[2].raw_log_odds = f64::NAN;
    assert!(calibration::apply_gates(&input, &scores, &model, &gates()).is_err());
    let mut corrupt = gates();
    corrupt.domains[0].knots[1].relevance = f64::NAN;
    assert!(calibration::apply_gates(&input, &raw([1.0; 6]), &model, &corrupt).is_err());
    let mut reordered = raw([1.0; 6]);
    reordered.swap(0, 1);
    assert!(calibration::apply_gates(&input, &reordered, &model, &gates()).is_err());
}

fn corpus() -> TrainingCorpus {
    let provenance = Provenance::synthetic_baseline("fixture-v1", "0".repeat(64));
    let mut rows = Vec::new();
    for split in [Split::Train, Split::Calibration] {
        for idx in 0..4 {
            let id = format!("{}-{idx}", split.as_str());
            rows.push(DatasetRow {
                sample_id: id.clone(),
                family_id: FamilyId::parse(&id).unwrap(),
                split,
                title: if idx % 2 == 0 {
                    "Parliament security election"
                } else {
                    "Scientific health environmental"
                }
                .into(),
                description: "Independent fixture article".into(),
                language: "en".into(),
                membership: core::array::from_fn(|domain| {
                    if (domain < 3) == (idx % 2 == 0) {
                        LabelMembership::Present
                    } else {
                        LabelMembership::Absent
                    }
                }),
                labels_complete: true,
                provenance: provenance.clone(),
            });
        }
    }
    let mut variant = rows[0].clone();
    variant.sample_id.push_str("-variant");
    rows.push(variant);
    let hashes = training::corpus_hashes(&rows).unwrap();
    TrainingCorpus {
        rows,
        dataset_sha256: "0".repeat(64),
        train_split_sha256: hashes.train_split_sha256,
        calibration_split_sha256: hashes.calibration_split_sha256,
        test_split_sha256: hashes.test_split_sha256,
        family_assignment_sha256: hashes.family_assignment_sha256,
    }
}
fn request() -> TrainRequest {
    TrainRequest {
        request_id: "train-regression".into(),
        model_id: ModelId::parse("fixture").unwrap(),
        provenance: Provenance::synthetic_baseline("fixture-v1", "0".repeat(64)),
        synthetic_activation: SyntheticActivation::RetainPrior,
    }
}

#[derive(Default)]
struct MalformedScores {
    step: u64,
}
impl ClassifierWorker for MalformedScores {
    fn call(
        &mut self,
        operation: BridgeOperation,
        request: &str,
        _: &WorkControl,
    ) -> Result<String, ClassifierError> {
        let req: serde_json::Value = serde_json::from_str(request).unwrap();
        let base = serde_json::json!({"version":"1", "request_id":req["request_id"], "errors":[]});
        let mut response = base;
        match operation {
            BridgeOperation::TrainBatch => {
                response["accepted"] = serde_json::json!(req["rows"].as_array().unwrap().len());
                response["skipped"] = 0.into();
                response["token_bytes"] = 10.into();
                response["vocabulary_new"] = 2.into();
                response["progress"] = 1.0.into();
                response["round"] = 1.into();
                response["feature_order_revisions"] = serde_json::json!([1, 1, 1, 1, 1, 1]);
                response["fitted_memberships"] = serde_json::json!([0, 0, 0, 0, 0, 0]);
                response["vocabulary_sizes"] = serde_json::json!([2, 2, 2, 2, 2, 2]);
            }
            BridgeOperation::FinishTraining => {
                self.step += 1;
                response["done"] = (self.step == 2).into();
                response["round"] = 1.into();
                response["fit_step"] = self.step.into();
                response["frozen_order_length"] = 12.into();
                response["vocabulary_sizes"] = serde_json::json!([2, 2, 2, 2, 2, 2]);
                response["feature_order_revisions"] = serde_json::json!([1, 1, 1, 1, 1, 1]);
                response["fitted_memberships"] = if self.step == 1 {
                    serde_json::json!([4, 4, 4, 4, 0, 0])
                } else {
                    serde_json::json!([4, 4, 4, 4, 4, 4])
                };
            }
            BridgeOperation::SerializeModel => return Ok("{}".into()),
            BridgeOperation::ClassifyBatch => {
                response["items"] = serde_json::json!([{"id":req["items"][0]["id"], "results": DomainId::ALL.map(|domain| serde_json::json!({"domain":domain.as_str()}))}]);
            }
            _ => panic!("malformed scores must abort before restore"),
        }
        Ok(response.to_string())
    }
}

#[test]
fn malformed_bridge_scores_cannot_be_silently_calibrated_as_zero() {
    let err = training::train_candidate(
        &mut MalformedScores::default(),
        &request(),
        &corpus(),
        &WorkControl::new(),
        &mut |_| {},
    )
    .unwrap_err();
    assert!(matches!(err, ClassifierError::BridgeProtocol { .. }));
}

#[cfg(feature = "classifier")]
#[test]
fn real_quickjs_training_restores_and_counts_independent_training_families() {
    use argos_osint_core::classifier::artifact::ArtifactStore;
    use std::sync::Arc;
    let home = tempfile::tempdir().unwrap();
    let store = ArtifactStore::new(home.path()).unwrap();
    let lease = Arc::new(store.try_writer().unwrap());
    let request = request();
    let control = WorkControl::new();
    let mut worker =
        argos_osint_core::classifier::start_candidate_worker(&request.model_id, lease, &control)
            .unwrap();
    let candidate =
        training::train_candidate(worker.as_mut(), &request, &corpus(), &control, &mut |_| {})
            .unwrap();
    for support in candidate.report.class_counts {
        assert_eq!(support.present, 2);
        assert_eq!(support.absent, 2);
    }
    assert_eq!(candidate.report.evaluation.complete_families, 4);
    assert!(candidate.report.evaluation.coverage.is_finite());
    assert!(!home.path().join("classifier/active.json").exists());
}

#[test]
fn calibration_support_is_independent_and_ninety_percent_is_the_point_target() {
    for (families, positives, accepted) in [(19, 18, false), (20, 18, true), (20, 17, false)] {
        let rows: Vec<_> = (0..families)
            .map(|idx| {
                family(
                    &format!("support-{idx}"),
                    if idx < positives { &[0] } else { &[] },
                    [1.0; 6],
                )
            })
            .collect();
        let metrics = calibration::evaluate(&rows, &gates()).unwrap();
        assert_eq!(
            metrics.domains[0].accepted, accepted,
            "{positives}/{families}"
        );
        assert_eq!(metrics.domains[0].accepted_families, families as u64);
    }
}

fn observation() -> argos_osint_core::classifier::provider_labels::ProviderObservation {
    use argos_osint_core::classifier::provider_labels::{ProviderFormat, ProviderObservation};
    ProviderObservation {
        observation_id: "observation-1".into(),
        provider: ProviderFormat::NewsDataV1,
        endpoint: "fixture-v1".into(),
        publisher: Some("Fixture publisher".into()),
        url: "https://example.invalid/fictional".into(),
        article_id: Some("article".into()),
        published_at: None,
        fetched_at: "2026-01-01T00:00:00Z".into(),
        cycle_ids: vec!["cycle".into()],
        input_hash: "0".repeat(64),
        source_revision: "revision".into(),
        title: "Fictional parliament election commission".into(),
        description: "A fictional constitutional court".into(),
        language: Some("en".into()),
        raw_categories: serde_json::json!(["politics"]),
        raw_ai_tags: None,
        raw_metadata: serde_json::json!({}),
    }
}
fn checkpoint() -> calibration::CalibrationCheckpoint {
    calibration::CalibrationCheckpoint {
        schema_version: calibration::CALIBRATION_CHECKPOINT_SCHEMA_VERSION.into(),
        job_id: "calibration-fixture".into(),
        state: calibration::CalibrationState::Collecting,
        observation_watermark: 0,
        observations_processed: 0,
        retained_families: 0,
        new_eligible_training_families: 0,
        last_completed_fit_watermark: 0,
        candidate_model_id: None,
        report_sha256: None,
        reasons: vec![],
        active_generation: None,
    }
}
#[test]
fn uncertified_provider_topics_are_monitored_without_triggering_retraining() {
    let envelope = calibration::ObservationEnvelope {
        watermark: 1,
        observation: observation(),
        family_id: FamilyId::parse("story").unwrap(),
        split: Split::Train,
    };
    let result = calibration::process_observations(
        &checkpoint(),
        &[],
        &calibration::WeakTrainingBudget::default(),
        &[envelope],
        &[],
    )
    .unwrap();
    assert_eq!(result.processed, 1);
    assert_eq!(result.next_watermark, 1);
    assert_eq!(result.new_eligible_training_families, 0);
    assert!(!result.families[0].eligible_training);
    assert!(result.budget.counted_family_ids.is_empty());
}
#[test]
fn approved_provider_training_requires_matching_predicates_language_and_family_deduplication() {
    use argos_osint_core::classifier::provider_labels::{self, MappingApproval, RuleValidation};
    let approval = MappingApproval {
        mapping_version: provider_labels::MAPPING_VERSION.into(),
        predicates_sha256: provider_labels::mapping_predicates_sha256(),
        review_reference: "fixture-reviewed-mapping".into(),
        reviewed_corpus_sha256: "a".repeat(64),
        domains: DomainId::ALL.map(|domain| RuleValidation {
            domain,
            true_positive_families: 20,
            false_positive_families: 0,
        }),
    };
    let budget = calibration::WeakTrainingBudget {
        seed_positive_families: [10; 6],
        ..Default::default()
    };
    let envelope = calibration::ObservationEnvelope {
        watermark: 1,
        observation: observation(),
        family_id: FamilyId::parse("story").unwrap(),
        split: Split::Train,
    };
    let first = calibration::process_observations(
        &checkpoint(),
        &[],
        &budget,
        std::slice::from_ref(&envelope),
        std::slice::from_ref(&approval),
    )
    .unwrap();
    assert_eq!(first.new_eligible_training_families, 1);
    assert!(first.families[0].eligible_training);
    let mut next = checkpoint();
    next.observation_watermark = 1;
    let mut second = envelope;
    second.watermark = 2;
    second.observation.observation_id = "observation-2".into();
    let replay = calibration::process_observations(
        &next,
        &first.families,
        &first.budget,
        &[second.clone()],
        std::slice::from_ref(&approval),
    )
    .unwrap();
    assert_eq!(replay.new_eligible_training_families, 0);
    assert_eq!(
        replay.budget.selected_weak_positive_families,
        first.budget.selected_weak_positive_families
    );
    let mut bad = approval.clone();
    bad.predicates_sha256 = "b".repeat(64);
    assert!(provider_labels::map_labels(&observation(), Some(&bad)).is_err());
    let mut nonenglish = observation();
    nonenglish.language = Some("fr".into());
    assert!(
        !provider_labels::map_labels(&nonenglish, Some(&approval))
            .unwrap()
            .eligible_for_training
    );
}
#[test]
fn observation_bounds_include_unstructured_provider_metadata() {
    let mut huge = observation();
    huge.raw_metadata = serde_json::json!({"untrusted":"x".repeat(128*1024)});
    assert!(matches!(
        argos_osint_core::classifier::provider_labels::validate_observation(&huge),
        Err(ClassifierError::ObservationTooLarge)
    ));
}

#[test]
fn automatic_promotion_requires_reviewed_independent_baseline_and_runtime_evidence() {
    use calibration::{PromotionDecision, PromotionEvidence, RuntimeComparison};
    let rows: Vec<_> = (0..60)
        .map(|idx| {
            let pair = (idx / 20) * 2;
            family(
                &format!("eval-{idx}"),
                &[pair, pair + 1],
                core::array::from_fn(|domain| {
                    if domain == pair || domain == pair + 1 {
                        1.0
                    } else {
                        -1.0
                    }
                }),
            )
        })
        .collect();
    let metrics = calibration::evaluate(&rows, &gates()).unwrap();
    assert!(metrics.domains.iter().all(|domain| domain.accepted));
    assert_eq!(metrics.exact_pair_families, 60);
    let valid = PromotionEvidence {
        candidate_model_id: ModelId::parse("candidate").unwrap(),
        candidate_payload_sha256: "a".repeat(64),
        candidate_report_sha256: "b".repeat(64),
        provenance: Provenance::reviewed_real("reviewed-v1", "c".repeat(64), "independent review"),
        independent_review_reference: "independent review".into(),
        reviewed_corpus_sha256: "d".repeat(64),
        family_assignment_sha256: "e".repeat(64),
        baseline_model_id: Some(ModelId::parse("baseline").unwrap()),
        baseline_active_generation: Some(1),
        candidate: metrics.clone(),
        baseline: Some(metrics),
        evaluation_family_ids: rows.iter().map(|row| row.family_id.clone()).collect(),
        evaluation_family_provenance: rows
            .iter()
            .map(|row| {
                (
                    row.family_id.clone(),
                    Provenance::reviewed_real(
                        "reviewed-holdout",
                        "d".repeat(64),
                        "independent review",
                    ),
                )
            })
            .collect(),
        training_family_ids: vec![FamilyId::parse("train-only").unwrap()],
        threshold_fit_family_ids: vec![FamilyId::parse("fit-only").unwrap()],
        split: Split::Development,
        runtime: Some(RuntimeComparison {
            hardware_id: "fixture hardware".into(),
            corpus_sha256: "d".repeat(64),
            measurement_reference: "fixture measurements".into(),
            candidate_warm_p95_ms: 1.0,
            baseline_warm_p95_ms: 2.0,
            foreground_without_calibration_p95_ms: 10.0,
            foreground_with_calibration_p95_ms: 10.0,
            peak_heap_bytes: 1024,
            configured_stack_limit_bytes: 1024 * 1024,
            stack_limit_fixture_passed: true,
            peak_reservation_bytes: 129 * 1024 * 1024,
        }),
    };
    assert_eq!(
        calibration::assess_promotion(&valid).unwrap(),
        PromotionDecision::Eligible
    );
    let mut missing_baseline = valid.clone();
    missing_baseline.baseline = None;
    assert!(matches!(
        calibration::assess_promotion(&missing_baseline).unwrap(),
        PromotionDecision::RetainPrior { .. }
    ));
    let mut synthetic = valid.clone();
    synthetic.provenance = Provenance::synthetic_baseline("seed", "a".repeat(64));
    assert!(matches!(
        calibration::assess_promotion(&synthetic).unwrap(),
        PromotionDecision::RetainPrior { .. }
    ));
    let mut overlap = valid.clone();
    overlap
        .training_family_ids
        .push(overlap.evaluation_family_ids[0].clone());
    assert!(matches!(
        calibration::assess_promotion(&overlap).unwrap(),
        PromotionDecision::RetainPrior { .. }
    ));
    let mut inflated = valid.clone();
    inflated.candidate.complete_families = 1;
    inflated.evaluation_family_ids.truncate(1);
    assert!(
        calibration::assess_promotion(&inflated).is_err(),
        "one family cannot certify twenty independent successes"
    );
    let mut forged_decision = valid.clone();
    forged_decision.candidate.domains[0].accepted = false;
    assert!(calibration::assess_promotion(&forged_decision).is_err());
    let mut forged_f1 = valid.clone();
    forged_f1.candidate.macro_f1 = 0.5;
    assert!(calibration::assess_promotion(&forged_f1).is_err());
    let mut forged_rates = valid.clone();
    forged_rates.candidate.false_oos_admissions = 1;
    assert!(calibration::assess_promotion(&forged_rates).is_err());
    let mut forged_precision = valid.clone();
    forged_precision.candidate.domains[0].precision = Some(1.5);
    assert!(calibration::assess_promotion(&forged_precision).is_err());
    let mut synthetic_holdout = valid.clone();
    synthetic_holdout.evaluation_family_provenance[0].1 =
        Provenance::synthetic_baseline("seed", "d".repeat(64));
    assert!(matches!(
        calibration::assess_promotion(&synthetic_holdout).unwrap(),
        PromotionDecision::RetainPrior { .. }
    ));
    let mut missing_lineage = valid.clone();
    missing_lineage.evaluation_family_provenance.clear();
    assert!(matches!(
        calibration::assess_promotion(&missing_lineage).unwrap(),
        PromotionDecision::RetainPrior { .. }
    ));
    let mut invalid_runtime = valid;
    invalid_runtime
        .runtime
        .as_mut()
        .unwrap()
        .candidate_warm_p95_ms = f64::NAN;
    assert!(matches!(
        calibration::assess_promotion(&invalid_runtime).unwrap(),
        PromotionDecision::RetainPrior { .. }
    ));
}

struct NoWorkerCalls;
impl ClassifierWorker for NoWorkerCalls {
    fn call(
        &mut self,
        _: BridgeOperation,
        _: &str,
        _: &WorkControl,
    ) -> Result<String, ClassifierError> {
        panic!("invalid source provenance must be rejected before any worker operation");
    }
}

#[test]
fn synthetic_training_sources_cannot_be_relabelled_as_reviewed_real() {
    let corpus = corpus();
    let mut request = request();
    request.provenance = Provenance::reviewed_real(
        "claimed-real",
        corpus.dataset_sha256.clone(),
        "independent review",
    );
    let error = training::train_candidate(
        &mut NoWorkerCalls,
        &request,
        &corpus,
        &WorkControl::new(),
        &mut |_| {},
    )
    .unwrap_err();
    assert!(
        matches!(error, ClassifierError::InvalidTrainingData { reason, .. } if reason == "training_source_provenance_mismatch")
    );
    let mut invalid = corpus;
    invalid.rows[0].provenance = Provenance::reviewed_real("reviewed", "0".repeat(64), "");
    assert!(training::validate_corpus(&invalid).is_err());
}

#[test]
fn changing_an_already_selected_family_cannot_bypass_domain_contribution_caps() {
    use argos_osint_core::classifier::provider_labels::{self, MappingApproval, RuleValidation};
    let approval = MappingApproval {
        mapping_version: provider_labels::MAPPING_VERSION.into(),
        predicates_sha256: provider_labels::mapping_predicates_sha256(),
        review_reference: "independent fixture review".into(),
        reviewed_corpus_sha256: "a".repeat(64),
        domains: DomainId::ALL.map(|domain| RuleValidation {
            domain,
            true_positive_families: 20,
            false_positive_families: 0,
        }),
    };
    let budget = calibration::WeakTrainingBudget {
        seed_positive_families: [0, 0, 0, 0, 0, 1],
        ..Default::default()
    };
    let mut obs = observation();
    obs.title = "Scientific study of laboratory research".into();
    obs.description.clear();
    obs.raw_categories = serde_json::json!(["science"]);
    let first_envelope = calibration::ObservationEnvelope {
        watermark: 1,
        observation: obs,
        family_id: FamilyId::parse("same-family").unwrap(),
        split: Split::Train,
    };
    let first = calibration::process_observations(
        &checkpoint(),
        &[],
        &budget,
        std::slice::from_ref(&first_envelope),
        std::slice::from_ref(&approval),
    )
    .unwrap();
    assert_eq!(first.new_eligible_training_families, 1);
    let mut second_envelope = first_envelope;
    second_envelope.watermark = 2;
    second_envelope.observation.observation_id = "changed-observation".into();
    second_envelope.observation.title = "Central bank reports supply chain disruption".into();
    second_envelope.observation.raw_categories = serde_json::json!(["business"]);
    let mut next = checkpoint();
    next.observation_watermark = 1;
    let second = calibration::process_observations(
        &next,
        &first.families,
        &first.budget,
        &[second_envelope],
        &[approval],
    )
    .unwrap();
    assert_eq!(
        second.dispositions[0].reason,
        calibration::ObservationReason::ConflictingEvidence
    );
    assert_eq!(second.budget, first.budget);
    assert_eq!(
        second.families[0].approved_membership,
        first.families[0].approved_membership
    );
}

#[test]
fn calibration_checkpoint_requires_new_training_families_and_fenced_activation() {
    use calibration::{CalibrationEvent, CalibrationState, EvidenceFailure, PromotionDecision};
    let mut collecting = checkpoint();
    collecting.state = CalibrationState::Collecting;
    assert!(calibration::transition(&collecting, CalibrationEvent::TriggerRetrain).is_err());
    let ready = calibration::transition(
        &collecting,
        CalibrationEvent::ThresholdEvaluated {
            model_id: ModelId::parse("threshold-candidate").unwrap(),
            decision: PromotionDecision::Eligible,
            report_sha256: "a".repeat(64),
        },
    )
    .unwrap();
    assert_eq!(ready.state, CalibrationState::Ready);
    assert_eq!(ready.new_eligible_training_families, 0);
    assert!(calibration::transition(
        &ready,
        CalibrationEvent::Activated {
            model_id: ModelId::parse("different-candidate").unwrap(),
            active_generation: 1
        }
    )
    .is_err());
    let rejected = calibration::transition(
        &collecting,
        CalibrationEvent::ThresholdEvaluated {
            model_id: ModelId::parse("threshold-candidate").unwrap(),
            decision: PromotionDecision::RetainPrior {
                reasons: vec![EvidenceFailure::MissingReviewedRealEvidence],
            },
            report_sha256: "b".repeat(64),
        },
    )
    .unwrap();
    assert!(calibration::transition(
        &rejected,
        CalibrationEvent::Activated {
            model_id: ModelId::parse("threshold-candidate").unwrap(),
            active_generation: 1
        }
    )
    .is_err());
    let mut invalid = collecting.clone();
    invalid.last_completed_fit_watermark = 10;
    assert!(calibration::transition(&invalid, CalibrationEvent::BeginCollection).is_err());
    collecting.new_eligible_training_families = calibration::RETRAIN_NEW_FAMILIES;
    assert_eq!(
        calibration::transition(&collecting, CalibrationEvent::TriggerRetrain)
            .unwrap()
            .state,
        CalibrationState::RetrainTriggered
    );
}

#[test]
fn provider_anchor_phrases_preserve_unicode_whitespace_spans_and_sentence_boundaries() {
    use argos_osint_core::classifier::provider_labels;
    let mut obs = observation();
    obs.raw_categories = serde_json::json!(["science"]);
    obs.description.clear();
    for phrase in [
        "scientific study",
        "scientific  study",
        "scientific\tstudy",
        "scientific\u{2003}study",
    ] {
        obs.title = format!("Reports on {phrase} today");
        let mapped = provider_labels::map_labels(&obs, None).unwrap();
        assert_eq!(mapped.membership[5], LabelMembership::Present, "{phrase:?}");
        let span = mapped
            .evidence
            .iter()
            .find(|e| e.domain == DomainId::Scientific)
            .unwrap();
        assert_eq!(&obs.title[span.start_byte..span.end_byte], phrase);
        assert_eq!(span.matched_text, phrase);
    }
    for phrase in [
        "scientific\nstudy",
        "scientific.study",
        "unscientific study",
        "scientific studies",
    ] {
        obs.title = phrase.into();
        assert_eq!(
            provider_labels::map_labels(&obs, None).unwrap().membership[5],
            LabelMembership::Unknown,
            "{phrase:?}"
        );
    }
}

struct RecordOperationIds(Vec<String>, MalformedScores);
impl ClassifierWorker for RecordOperationIds {
    fn call(
        &mut self,
        operation: BridgeOperation,
        request: &str,
        control: &WorkControl,
    ) -> Result<String, ClassifierError> {
        let value: serde_json::Value = serde_json::from_str(request).unwrap();
        self.0.push(value["request_id"].as_str().unwrap().into());
        self.1.call(operation, request, control)
    }
    fn restore_completed_candidate(
        &mut self,
        model: &ModelId,
        payload: &str,
        control: &WorkControl,
    ) -> Result<(), ClassifierError> {
        self.1.restore_completed_candidate(model, payload, control)
    }
}
#[test]
fn maximal_logical_ids_produce_bounded_deterministic_operation_ids_without_truncation() {
    let mut data = corpus();
    for row in &mut data.rows {
        row.sample_id = format!("{}{}", "x".repeat(256 - row.sample_id.len()), row.sample_id);
    }
    let hashes = training::corpus_hashes(&data.rows).unwrap();
    data.train_split_sha256 = hashes.train_split_sha256;
    data.calibration_split_sha256 = hashes.calibration_split_sha256;
    data.test_split_sha256 = hashes.test_split_sha256;
    data.family_assignment_sha256 = hashes.family_assignment_sha256;
    let mut req = request();
    req.request_id = "q".repeat(256);
    let mut first = RecordOperationIds(vec![], MalformedScores::default());
    let mut second = RecordOperationIds(vec![], MalformedScores::default());
    for worker in [&mut first, &mut second] {
        assert!(matches!(
            training::train_candidate(worker, &req, &data, &WorkControl::new(), &mut |_| {}),
            Err(ClassifierError::BridgeProtocol { .. })
        ));
    }
    assert!(!first.0.is_empty());
    assert_eq!(first.0, second.0);
    assert!(first.0.iter().all(|id| id.len() <= 256));
    let mut changed = req;
    changed.request_id.replace_range(255.., "z");
    let mut third = RecordOperationIds(vec![], MalformedScores::default());
    let _ = training::train_candidate(
        &mut third,
        &changed,
        &data,
        &WorkControl::new(),
        &mut |_| {},
    );
    assert_ne!(
        first.0, third.0,
        "the final logical-ID byte must remain part of framed identity"
    );
}
#[test]
fn finite_but_inconsistent_raw_log_odds_cannot_create_a_valid_receipt() {
    let input = ClassifyInput::new("occ", "Headline", "Description", Some("en".into()));
    let mut scores = raw([1.0; 6]);
    scores[0].raw_log_odds = 2.0;
    assert!(calibration::apply_gates(
        &input,
        &scores,
        &ModelId::parse("fixture").unwrap(),
        &gates()
    )
    .is_err());
}

#[test]
fn family_identifiers_reject_unicode_controls_and_keep_byte_boundary() {
    for value in [
        "story\u{0085}next",
        "story\u{009f}",
        "story\nnext",
        "story\u{007f}",
    ] {
        assert!(FamilyId::parse(value).is_err());
    }
    let boundary = "é".repeat(128);
    assert_eq!(FamilyId::parse(&boundary).unwrap().as_str(), boundary);
    assert!(FamilyId::parse(&(boundary + "a")).is_err());
}

#[cfg(feature = "classifier")]
struct TracePackedWorker {
    inner: Box<dyn ClassifierWorker>,
    batches: Vec<(usize, Vec<String>)>,
    terminal_calls: usize,
}
#[cfg(feature = "classifier")]
impl ClassifierWorker for TracePackedWorker {
    fn call(
        &mut self,
        operation: BridgeOperation,
        request: &str,
        control: &WorkControl,
    ) -> Result<String, ClassifierError> {
        if operation == BridgeOperation::TrainBatch {
            let value: serde_json::Value = serde_json::from_str(request).unwrap();
            self.batches.push((
                request.len(),
                value["rows"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|row| row["row_id"].as_str().unwrap().into())
                    .collect(),
            ));
        }
        self.inner.call(operation, request, control)
    }
    fn restore_completed_candidate(
        &mut self,
        model: &ModelId,
        payload: &str,
        control: &WorkControl,
    ) -> Result<(), ClassifierError> {
        self.terminal_calls += 1;
        self.inner
            .restore_completed_candidate(model, payload, control)
    }
}

#[cfg(feature = "classifier")]
#[test]
fn escaped_training_rows_split_by_exact_wire_bytes_without_loss() {
    use argos_osint_core::classifier::artifact::ArtifactStore;
    use std::{collections::BTreeSet, sync::Arc};
    for control_escaping in [false, true] {
        let mut data = corpus();
        let template = data
            .rows
            .iter()
            .find(|row| row.split == Split::Train)
            .unwrap()
            .clone();
        data.rows.retain(|row| row.split != Split::Train);
        let mut title = "Fictional evidence ".to_string();
        while title.len() < training::MAX_ROW_TEXT_BYTES {
            title.push(if control_escaping {
                '\0'
            } else if title.len().is_multiple_of(2) {
                '"'
            } else {
                '\\'
            });
        }
        let mut expected = BTreeSet::new();
        for index in 0..32 {
            let mut row = template.clone();
            row.sample_id = format!("escaped-{index:02}");
            row.family_id = FamilyId::parse(&row.sample_id).unwrap();
            row.title = title.clone();
            row.description.clear();
            row.membership = [if index % 2 == 0 {
                LabelMembership::Present
            } else {
                LabelMembership::Absent
            }; 6];
            expected.insert(row.sample_id.clone());
            data.rows.push(row);
        }
        let hashes = training::corpus_hashes(&data.rows).unwrap();
        data.train_split_sha256 = hashes.train_split_sha256;
        data.calibration_split_sha256 = hashes.calibration_split_sha256;
        data.test_split_sha256 = hashes.test_split_sha256;
        data.family_assignment_sha256 = hashes.family_assignment_sha256;
        let root = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(root.path()).unwrap();
        let lease = Arc::new(store.try_writer().unwrap());
        let control = WorkControl::new();
        let request = request();
        let inner = argos_osint_core::classifier::start_candidate_worker(
            &request.model_id,
            lease,
            &control,
        )
        .unwrap();
        let mut worker = TracePackedWorker {
            inner,
            batches: Vec::new(),
            terminal_calls: 0,
        };
        let mut progress = Vec::new();
        let candidate =
            training::train_candidate(&mut worker, &request, &data, &control, &mut |value| {
                progress.push(value.clone())
            })
            .unwrap();
        assert!(
            worker.batches.len() > 1,
            "valid escaped rows must split below the2MiB envelope bound"
        );
        assert!(worker.batches.iter().all(|(bytes, rows)| *bytes
            <= training::MAX_TRAINING_ENVELOPE_BYTES
            && !rows.is_empty()
            && rows.len() <= 32));
        let emitted = worker
            .batches
            .iter()
            .flat_map(|(_, rows)| rows.iter().cloned())
            .collect::<Vec<_>>();
        assert_eq!(emitted.len(), 32);
        assert_eq!(emitted.into_iter().collect::<BTreeSet<_>>(), expected);
        assert_eq!(
            worker.terminal_calls, 1,
            "traced workers must forward the owned terminal port"
        );
        for count in candidate.report.class_counts {
            assert_eq!((count.present, count.absent), (16, 16));
        }
        let final_progress = progress.last().unwrap();
        assert_eq!(final_progress.rows_seen, 32);
        assert_eq!(final_progress.rows_fitted, 32);
        assert_eq!(final_progress.round, worker.batches.len() as u64);
        assert_eq!(final_progress.fitted_memberships, [32; 6]);
        assert!(progress
            .windows(2)
            .all(|pair| pair[0].rows_seen <= pair[1].rows_seen
                && pair[0].rows_fitted <= pair[1].rows_fitted));
        assert!(store.read_active().unwrap().is_none());
    }
}

#[test]
fn an_individually_oversized_row_is_identified_before_worker_dispatch() {
    let mut data = corpus();
    let row_id = data.rows[0].sample_id.clone();
    data.rows[0].title = "x".repeat(training::MAX_ROW_TEXT_BYTES + 1);
    data.rows[0].description.clear();
    assert!(
        matches!(training::train_candidate(&mut NoWorkerCalls,&request(),&data,&WorkControl::new(),&mut |_|{}),Err(ClassifierError::InvalidTrainingData{row_id:Some(id),reason}) if id==row_id && reason=="row_text_exceeds_32k")
    );
}
