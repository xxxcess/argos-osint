//! Behavioral contracts for variable domain membership and independent calibration.
use argos_osint_core::atlas_taxonomy::DomainId;
use argos_osint_core::classifier::calibration::{
    self, CalibrationKnot, DomainCalibration, GateSet, CALIBRATION_VERSION,
};
use argos_osint_core::classifier::{ClassifyInput, ModelId, RawDomainScore, RejectionReason};

fn classify(
    relevance: [f64; 6],
    thresholds: [f64; 6],
) -> argos_osint_core::classifier::ClassificationReceipt {
    let gates = GateSet {
        version: CALIBRATION_VERSION.into(),
        domains: std::array::from_fn(|index| DomainCalibration {
            domain: DomainId::ALL[index],
            sigmoid: None,
            knots: vec![CalibrationKnot {
                raw_upper: 1.0,
                relevance: relevance[index],
            }],
            threshold: thresholds[index],
            min_informative_features: 1,
        }),
    };
    let raw = std::array::from_fn(|index| RawDomainScore {
        domain: DomainId::ALL[index],
        present_log_score: 1.0,
        absent_log_score: 0.0,
        raw_log_odds: 1.0,
        known_feature_count: 2,
        oov_count: 0,
    });
    let input = ClassifyInput::new(
        "quality-v2",
        "Fictional report",
        "Independent policy fixture",
        Some("en".into()),
    );
    let receipt = calibration::apply_gates(
        &input,
        &raw,
        &ModelId::parse("quality-model").unwrap(),
        &gates,
    )
    .unwrap();
    receipt.validate().unwrap();
    receipt
}

#[test]
fn strong_primary_keeps_single_domain_when_second_is_weak() {
    let receipt = classify([0.95, 0.7, 0.1, 0.1, 0.1, 0.1], [0.8; 6]);
    assert_eq!(receipt.selected_domains, vec![DomainId::Governance]);
    assert!(receipt.is_admitted());
    assert!(!receipt.primary_rank_ambiguous);
}
#[test]
fn eligibility_precedes_ranking() {
    let receipt = classify(
        [0.99, 0.92, 0.87, 0.1, 0.1, 0.1],
        [1.0, 0.8, 0.8, 0.8, 0.8, 0.8],
    );
    assert_eq!(
        receipt.selected_domains,
        vec![DomainId::Security, DomainId::Economic]
    );
}
#[test]
fn exactly_two_equal_eligible_domains_preserve_both_without_claiming_dominance() {
    let receipt = classify([0.9, 0.9, 0.1, 0.1, 0.1, 0.1], [0.8; 6]);
    assert_eq!(
        receipt.selected_domains,
        vec![DomainId::Governance, DomainId::Security]
    );
    assert!(receipt.primary_rank_ambiguous);
    assert_eq!(receipt.ranking_reason(), "primary_rank_ambiguous");
}
#[test]
fn competing_second_slots_leave_only_the_unambiguous_primary() {
    let receipt = classify([0.95, 0.9, 0.9, 0.1, 0.1, 0.1], [0.8; 6]);
    assert_eq!(receipt.selected_domains, vec![DomainId::Governance]);
    assert_eq!(
        receipt.ranking_reason(),
        "secondary_rank_ambiguous_primary_only"
    );
}
#[test]
fn ambiguous_primary_abstains_instead_of_using_alphabetical_order() {
    let receipt = classify([0.9, 0.9, 0.9, 0.1, 0.1, 0.1], [0.8; 6]);
    assert!(receipt.selected_domains.is_empty());
    assert_eq!(
        receipt.rejection_reason,
        Some(RejectionReason::AmbiguousRanking)
    );
}
#[test]
fn old_receipt_decode_preserves_original_policy_without_current_authority() {
    use argos_osint_core::classifier::{decode_receipt, DecodedReceipt};
    let legacy = r#"{"receipt_schema_version":"classifier-receipt-v1","policy_version":"exact-two-independent-gates-v1","selected_pair":["governance","security"]}"#;
    let DecodedReceipt::LegacyExactTwo(value) = decode_receipt(legacy).unwrap() else {
        panic!("legacy receipt was promoted")
    };
    assert_eq!(
        value["selected_pair"],
        serde_json::json!(["governance", "security"])
    );
    assert!(
        serde_json::from_str::<argos_osint_core::classifier::ClassificationReceipt>(legacy)
            .is_err()
    );
}
#[test]
fn sigmoid_scores_are_monotone_and_extreme_inputs_remain_finite() {
    let curve = DomainCalibration {
        domain: DomainId::Governance,
        sigmoid: Some([2.0, -1.0]),
        knots: Vec::new(),
        threshold: 0.8,
        min_informative_features: 1,
    };
    let values =
        [-1e300, -2.0, 0.0, 1.0, 1e300].map(|raw| calibration::score(raw, &curve).unwrap());
    assert!(values.windows(2).all(|pair| pair[0] <= pair[1]));
    assert!(values.iter().all(|value| value.is_finite()));
    let invalid = DomainCalibration {
        sigmoid: Some([-1.0, 0.0]),
        ..curve
    };
    assert!(calibration::score(0.0, &invalid).is_err());
}
