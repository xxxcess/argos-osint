//! Local cache/policy behavior. No provider wire or saved model is involved.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::atlas_classify::{
    CacheClock, CachedComputation, ComputationCache, ComputationKey, CACHE_BYTES, CACHE_ENTRIES,
    CACHE_SCHEMA_VERSION,
};
use crate::atlas_taxonomy::DomainId;
use crate::classifier::calibration::{
    self, CalibrationKnot, DomainCalibration, GateSet, CALIBRATION_VERSION,
};
use crate::classifier::{ClassifyInput, ModelId, RawDomainScore, RejectionReason};

#[derive(Default)]
struct Clock(AtomicU64);
impl CacheClock for Clock {
    fn now(&self) -> Duration {
        Duration::from_secs(self.0.load(Ordering::Acquire))
    }
}
fn key(index: usize) -> ComputationKey {
    ComputationKey {
        cache_schema_version: CACHE_SCHEMA_VERSION.into(),
        input_hash: format!("{index:064x}"),
        model_id: "local-test".into(),
        artifact_sha256: "1".repeat(64),
        manifest_sha256: "2".repeat(64),
        preprocessing_version: crate::classifier::PREPROCESSING_VERSION.into(),
        policy_version: crate::classifier::ADMISSION_POLICY_VERSION.into(),
        calibration_version: CALIBRATION_VERSION.into(),
    }
}
fn raw() -> [RawDomainScore; 6] {
    core::array::from_fn(|index| RawDomainScore {
        domain: DomainId::ALL[index],
        present_log_score: if index < 2 { 10.0 } else { 0.0 },
        absent_log_score: 0.0,
        raw_log_odds: if index < 2 { 10.0 } else { 0.0 },
        known_feature_count: 4,
        oov_count: 0,
    })
}
fn gates() -> GateSet {
    GateSet {
        version: CALIBRATION_VERSION.into(),
        domains: DomainId::ALL.map(|domain| DomainCalibration {
            sigmoid: None,
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

#[test]
fn cache_absolute_ttl_does_not_extend_when_a_hot_entry_is_read() {
    let clock = Arc::new(Clock::default());
    let mut cache = ComputationCache::new(clock.clone());
    cache
        .insert(CachedComputation {
            key: key(0),
            raw: raw(),
        })
        .unwrap();
    clock.0.store(3599, Ordering::Release);
    assert!(cache.get(&key(0)).is_some());
    clock.0.store(3600, Ordering::Release);
    assert!(cache.get(&key(0)).is_none());
    assert_eq!(cache.metrics().bytes, 0);
}
#[test]
fn cache_evicts_the_least_recent_entry_and_preserves_the_byte_bound() {
    let mut cache = ComputationCache::new(Arc::new(Clock::default()));
    for index in 0..CACHE_ENTRIES {
        cache
            .insert(CachedComputation {
                key: key(index),
                raw: raw(),
            })
            .unwrap();
    }
    assert!(cache.get(&key(0)).is_some());
    cache
        .insert(CachedComputation {
            key: key(CACHE_ENTRIES),
            raw: raw(),
        })
        .unwrap();
    assert!(cache.get(&key(1)).is_none());
    assert!(cache.get(&key(0)).is_some());
    assert!(cache.metrics().evictions >= 1);
    assert!(cache.metrics().bytes <= CACHE_BYTES);
}
#[test]
fn a_cached_computation_reapplies_language_and_rebuilds_occurrence_identity() {
    let mut cache = ComputationCache::new(Arc::new(Clock::default()));
    let english = ClassifyInput::new(
        "run-one/article",
        "Ballot patrol common",
        "Independent fixture",
        Some("en".into()),
    );
    let french = ClassifyInput::new(
        "run-two/article",
        "Ballot patrol common",
        "Independent fixture",
        Some("fr".into()),
    );
    assert_eq!(english.input_hash, french.input_hash);
    let mut computation = key(0);
    computation.input_hash = english.input_hash.clone();
    cache
        .insert(CachedComputation {
            key: computation.clone(),
            raw: raw(),
        })
        .unwrap();
    let scores = cache.get(&computation).unwrap();
    let model = ModelId::parse("local-test").unwrap();
    let admitted = calibration::apply_gates(&english, &scores, &model, &gates()).unwrap();
    let rejected = calibration::apply_gates(&french, &scores, &model, &gates()).unwrap();
    assert_eq!(
        admitted.selected_domains,
        vec![DomainId::Governance, DomainId::Security]
    );
    assert_eq!(
        rejected.rejection_reason,
        Some(RejectionReason::UnsupportedLanguage)
    );
    assert_eq!(admitted.occurrence_id, english.occurrence_id);
    assert_eq!(rejected.occurrence_id, french.occurrence_id);
    computation.model_id = "other-model".into();
    assert!(cache.get(&computation).is_none());
}
#[test]
fn inconsistent_raw_scores_are_not_cached() {
    let mut cache = ComputationCache::default();
    let mut scores = raw();
    scores[0].raw_log_odds += 1.0;
    assert!(cache
        .insert(CachedComputation {
            key: key(0),
            raw: scores
        })
        .is_err());
    assert!(cache.get(&key(0)).is_none());
}
