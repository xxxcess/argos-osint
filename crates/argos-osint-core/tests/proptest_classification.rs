//! Independent raw scores always produce exactly the ranked, passing pair.
use argos_osint_core::atlas_taxonomy::DomainId;
use argos_osint_core::classifier::calibration::{
    self, CalibrationKnot, DomainCalibration, GateSet, CALIBRATION_VERSION,
};
use argos_osint_core::classifier::{ClassifyInput, ModelId, RawDomainScore};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    #[test]
    fn exact_pair_tracks_independent_scores_without_probability_normalization(values in prop::array::uniform6(0u8..=100)) {
        let raw=core::array::from_fn(|index|RawDomainScore {domain:DomainId::ALL[index],present_log_score:f64::from(values[index]),absent_log_score:0.0,raw_log_odds:f64::from(values[index]),known_feature_count:4,oov_count:0});
        let gates=GateSet {version:CALIBRATION_VERSION.into(),domains:DomainId::ALL.map(|domain|DomainCalibration {sigmoid: None,domain,knots:(0..=100).map(|score|CalibrationKnot {raw_upper:f64::from(score),relevance:f64::from(score)/100.0}).collect(),threshold:0.5,min_informative_features:1})};
        let input=ClassifyInput::new("property/occurrence","Independent offline fixture","Ballot patrol freight river",Some("en".into()));
        let receipt=calibration::apply_gates(&input,&raw,&ModelId::parse("property-model").unwrap(),&gates).unwrap();
        receipt.validate().unwrap();
        prop_assert!(receipt.selected_domains.len()<=2);
        for domain in &receipt.selected_domains {let index=DomainId::ALL.iter().position(|id|id==domain).unwrap();prop_assert!(values[index]>=50);}
        if values.iter().all(|value|*value<50) {prop_assert!(receipt.selected_domains.is_empty());}
        if receipt.primary_rank_ambiguous {prop_assert_eq!(receipt.selected_domains.len(),2);}
        for (score,value) in receipt.scores.iter().zip(values) {prop_assert_eq!(score.calibrated_score,f64::from(value)/100.0);}
        prop_assert_eq!(receipt.occurrence_id,input.occurrence_id);
    }
}
