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
        let gates=GateSet {version:CALIBRATION_VERSION.into(),domains:DomainId::ALL.map(|domain|DomainCalibration {domain,knots:(0..=100).map(|score|CalibrationKnot {raw_upper:f64::from(score),relevance:f64::from(score)/100.0}).collect(),threshold:0.5,min_informative_features:1})};
        let input=ClassifyInput::new("property/occurrence","Independent offline fixture","Ballot patrol freight river",Some("en".into()));
        let receipt=calibration::apply_gates(&input,&raw,&ModelId::parse("property-model").unwrap(),&gates).unwrap();
        let mut rank:Vec<_>=DomainId::ALL.into_iter().zip(values).collect();
        rank.sort_by(|(left,ls),(right,rs)|rs.cmp(ls).then_with(||left.as_str().cmp(right.as_str())));
        if rank[1].1>=50 {prop_assert_eq!(receipt.selected_pair,Some([rank[0].0,rank[1].0]));} else {prop_assert!(receipt.selected_pair.is_none());}
        for (score,value) in receipt.scores.iter().zip(values) {prop_assert_eq!(score.calibrated_score,f64::from(value)/100.0);}
        prop_assert_eq!(receipt.occurrence_id,input.occurrence_id);
    }
}
