//! Threshold selection and holdout metrics for independent, reviewed news-cycle families.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde::Serialize;

use crate::classifier::calibration;
use crate::classifier::select_domains;
use crate::classifier::ClassifierError;
use crate::classifier::DomainId;
use crate::classifier::DomainScore;

use super::raw_scores;

const MAX_REVIEWED_ROWS: usize = 512;
const MIN_FAMILIES_PER_CLASS: usize = calibration::MIN_ACCEPTED_FAMILIES_PER_DOMAIN;
const MIN_PROMOTION_MACRO_F1_PERCENT: u128 = 65;
const CALIBRATION_GOAL_MACRO_F1_PERCENT: u128 = 85;

/// The reference used to judge domain selections in reviewed news cycles.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReviewedReferenceKind {
    #[default]
    CompleteArticleLabels,
    ReviewedProviderMappings,
}

impl ReviewedReferenceKind {
    fn is_complete(&self) -> bool {
        *self == Self::CompleteArticleLabels
    }
}

/// One reviewed story family with reference domain membership in canonical order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ReviewedPrediction {
    /// Raw Atlas worker probabilities, before the runtime's log-odds round trip.
    pub(crate) scores: [f64; 6],
    pub(crate) labels: [bool; 6],
}

/// Domain agreement counts. For provider mappings, an absent reference
/// domain is a nonmatch to the mapped set, not a negative article label.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ReviewedDomainCounts {
    pub(crate) tp: usize,
    pub(crate) fp: usize,
    #[serde(rename = "fn")]
    pub(crate) false_negatives: usize,
}

impl ReviewedDomainCounts {
    fn f1(self) -> f64 {
        let numerator = 2 * self.tp;
        let denominator = numerator + self.fp + self.false_negatives;
        if denominator == 0 {
            return 0.0;
        }
        numerator as f64 / denominator as f64
    }
}

/// Calibration and independent Test results at the selected threshold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ReviewedCycleMetrics {
    #[serde(default, skip_serializing_if = "ReviewedReferenceKind::is_complete")]
    pub(crate) reference: ReviewedReferenceKind,
    pub(crate) threshold: f64,
    pub(crate) calibration_rows: usize,
    pub(crate) calibration_macro_f1: f64,
    pub(crate) calibration_per_domain: BTreeMap<DomainId, ReviewedDomainCounts>,
    pub(crate) calibration_out_of_scope_rows: usize,
    pub(crate) calibration_false_out_of_scope_admissions: usize,
    pub(crate) test_rows: usize,
    pub(crate) test_macro_f1: f64,
    pub(crate) test_per_domain: BTreeMap<DomainId, ReviewedDomainCounts>,
    pub(crate) test_out_of_scope_rows: usize,
    pub(crate) test_false_out_of_scope_admissions: usize,
}

impl ReviewedCycleMetrics {
    /// Whether the result compares against reviewed provider domain mappings.
    pub(crate) fn is_mapping_agreement(&self) -> bool {
        self.reference == ReviewedReferenceKind::ReviewedProviderMappings
    }

    /// Checks the persisted score against the published counts and cohort bounds.
    pub(crate) fn valid(&self) -> bool {
        (50..100).any(|ticks| (f64::from(ticks) / 100.0 - self.threshold).abs() <= 1e-12)
            && valid_partition(
                self.reference,
                self.calibration_rows,
                &self.calibration_per_domain,
                self.calibration_out_of_scope_rows,
                self.calibration_false_out_of_scope_admissions,
                self.calibration_macro_f1,
            )
            && valid_partition(
                self.reference,
                self.test_rows,
                &self.test_per_domain,
                self.test_out_of_scope_rows,
                self.test_false_out_of_scope_admissions,
                self.test_macro_f1,
            )
    }

    /// Both partitions must clear the promotion floor.
    pub(crate) fn promotion_eligible(&self) -> bool {
        self.valid()
            && exceeds_macro_f1(&self.calibration_per_domain, MIN_PROMOTION_MACRO_F1_PERCENT)
            && exceeds_macro_f1(&self.test_per_domain, MIN_PROMOTION_MACRO_F1_PERCENT)
    }

    /// Stop calibrating only after the independent Test result clears the goal.
    pub(crate) fn freeze_calibration(&self) -> bool {
        self.promotion_eligible()
            && exceeds_macro_f1(&self.test_per_domain, CALIBRATION_GOAL_MACRO_F1_PERCENT)
    }
}

// After partition validation, denominators are positive and bounded by twice
// the row count. Sum the six fractions exactly so rounding or serialized
// metric differences cannot move a report across either strict boundary.
fn exceeds_macro_f1(per_domain: &BTreeMap<DomainId, ReviewedDomainCounts>, percent: u128) -> bool {
    let (numerator, denominator) = per_domain.values().fold((0, 1), |(sum, common), counts| {
        let numerator = (2 * counts.tp) as u128;
        let denominator = numerator + counts.fp as u128 + counts.false_negatives as u128;
        (sum * denominator + numerator * common, common * denominator)
    });
    numerator * 100 > percent * DomainId::ALL.len() as u128 * denominator
}

fn valid_partition(
    reference: ReviewedReferenceKind,
    rows: usize,
    per_domain: &BTreeMap<DomainId, ReviewedDomainCounts>,
    out_of_scope_rows: usize,
    false_out_of_scope_admissions: usize,
    macro_f1: f64,
) -> bool {
    let out_of_scope_supported = if reference.is_complete() {
        (MIN_FAMILIES_PER_CLASS..=rows).contains(&out_of_scope_rows)
    } else {
        out_of_scope_rows == 0
    };
    if rows > MAX_REVIEWED_ROWS
        || !out_of_scope_supported
        || false_out_of_scope_admissions > out_of_scope_rows
        || !(0.0..=1.0).contains(&macro_f1)
        || per_domain.len() != DomainId::ALL.len()
    {
        return false;
    }
    let mut sum_f1 = 0.0;
    let mut false_positives = 0;
    let mut selected_domains = 0;
    let mut reference_domains = 0;
    for domain in DomainId::ALL {
        let Some(counts) = per_domain.get(&domain) else {
            return false;
        };
        if [counts.tp, counts.fp, counts.false_negatives]
            .into_iter()
            .any(|count| count > rows)
        {
            return false;
        }
        let positives = counts.tp + counts.false_negatives;
        if !(MIN_FAMILIES_PER_CLASS..=rows - out_of_scope_rows).contains(&positives)
            || rows - positives < MIN_FAMILIES_PER_CLASS
            || counts.fp > rows - positives
        {
            return false;
        }
        false_positives += counts.fp;
        selected_domains += counts.tp + counts.fp;
        reference_domains += positives;
        sum_f1 += counts.f1();
    }
    (reference.is_complete() || reference_domains >= rows)
        && false_out_of_scope_admissions <= false_positives
        && selected_domains <= 2 * rows
        && (sum_f1 / DomainId::ALL.len() as f64 - macro_f1).abs() <= 1e-12
}

struct ScoredReview {
    scores: [DomainScore; 6],
    labels: [bool; 6],
}

#[derive(Default)]
struct Metrics {
    per_domain: [ReviewedDomainCounts; 6],
    out_of_scope_rows: usize,
    false_out_of_scope_admissions: usize,
}

impl Metrics {
    fn macro_f1(&self) -> f64 {
        self.per_domain
            .iter()
            .copied()
            .map(ReviewedDomainCounts::f1)
            .sum::<f64>()
            / DomainId::ALL.len() as f64
    }
}

/// Selects an admission threshold from Calibration and measures it on Test.
///
/// The caller supplies distinct complete story families partitioned before scoring.
/// When Calibration scores tie, prefer fewer out-of-scope admissions and then
/// the higher threshold. Test only measures the selected threshold.
///
/// # Errors
/// Rejects non-probability scores, oversized cohorts and insufficient class
/// support in either split.
#[cfg(test)]
pub(crate) fn calibrate_reviewed_cycle(
    calibration: &[ReviewedPrediction],
    test: &[ReviewedPrediction],
) -> Result<ReviewedCycleMetrics, ClassifierError> {
    calibrate(
        ReviewedReferenceKind::CompleteArticleLabels,
        calibration,
        test,
    )
}

/// Selects a threshold from reviewed provider mapping agreement on Calibration
/// and evaluates it against the independently mapped Test domain sets.
///
/// Rows without mapped domains are outside the cohort. A nonmatch to a mapped
/// domain set is evidence of disagreement only and makes no claim about article
/// truth or out-of-scope admissions.
///
/// # Errors
/// Rejects non-probability scores, oversized cohorts and insufficient mapped or
/// nonmapped reference support for any domain in either split.
pub(crate) fn calibrate_mapping_cycle(
    calibration: &[ReviewedPrediction],
    test: &[ReviewedPrediction],
) -> Result<ReviewedCycleMetrics, ClassifierError> {
    calibrate(
        ReviewedReferenceKind::ReviewedProviderMappings,
        calibration,
        test,
    )
}

fn calibrate(
    reference: ReviewedReferenceKind,
    calibration: &[ReviewedPrediction],
    test: &[ReviewedPrediction],
) -> Result<ReviewedCycleMetrics, ClassifierError> {
    let mut calibration = prepare(reference, calibration)?;
    let mut test = prepare(reference, test)?;
    let mut threshold = 0.5;
    let mut selected = measure(&mut calibration, threshold);
    for ticks in 51..100 {
        let trial_threshold = f64::from(ticks) / 100.0;
        let trial = measure(&mut calibration, trial_threshold);
        let rank = trial
            .macro_f1()
            .total_cmp(&selected.macro_f1())
            .then_with(|| {
                selected
                    .false_out_of_scope_admissions
                    .cmp(&trial.false_out_of_scope_admissions)
            });
        // The grid is ascending, so an exact tie favors the higher threshold.
        if rank.is_ge() {
            threshold = trial_threshold;
            selected = trial;
        }
    }
    let measured = measure(&mut test, threshold);
    Ok(ReviewedCycleMetrics {
        reference,
        threshold,
        calibration_rows: calibration.len(),
        calibration_macro_f1: selected.macro_f1(),
        calibration_per_domain: DomainId::ALL.into_iter().zip(selected.per_domain).collect(),
        calibration_out_of_scope_rows: selected.out_of_scope_rows,
        calibration_false_out_of_scope_admissions: selected.false_out_of_scope_admissions,
        test_rows: test.len(),
        test_macro_f1: measured.macro_f1(),
        test_per_domain: DomainId::ALL.into_iter().zip(measured.per_domain).collect(),
        test_out_of_scope_rows: measured.out_of_scope_rows,
        test_false_out_of_scope_admissions: measured.false_out_of_scope_admissions,
    })
}

fn prepare(
    reference: ReviewedReferenceKind,
    rows: &[ReviewedPrediction],
) -> Result<Vec<ScoredReview>, ClassifierError> {
    if rows.len() > MAX_REVIEWED_ROWS {
        return Err(ClassifierError::CorpusTooLarge);
    }
    let mut positives = [0; 6];
    let mut out_of_scope_rows = 0;
    let mut scored = Vec::with_capacity(rows.len());
    for row in rows {
        let has_reference = row.labels.contains(&true);
        if !reference.is_complete() && !has_reference {
            continue;
        }
        if !row.scores.iter().all(|score| (0.0..=1.0).contains(score)) {
            return Err(ClassifierError::InvalidCalibrationData {
                family_id: None,
                reason: "invalid_reviewed_bert_probability".into(),
            });
        }
        for (count, label) in positives.iter_mut().zip(row.labels) {
            *count += usize::from(label);
        }
        out_of_scope_rows += usize::from(!has_reference);
        scored.push(scored_review(row));
    }
    if (reference.is_complete() && out_of_scope_rows < MIN_FAMILIES_PER_CLASS)
        || positives.iter().any(|&count| {
            count < MIN_FAMILIES_PER_CLASS || scored.len() - count < MIN_FAMILIES_PER_CLASS
        })
    {
        return Err(ClassifierError::InvalidCalibrationData {
            family_id: None,
            reason: "insufficient_reviewed_bert_class_support".into(),
        });
    }
    Ok(scored)
}

fn scored_review(review: &ReviewedPrediction) -> ScoredReview {
    ScoredReview {
        scores: raw_scores(review.scores).map(|raw| DomainScore {
            domain: raw.domain,
            raw_log_odds: raw.raw_log_odds,
            calibrated_score: calibration::sigmoid(raw.raw_log_odds),
            threshold: 0.0,
            informative_features: 0,
            oov_features: 0,
            passes_gate: false,
        }),
        labels: review.labels,
    }
}

fn measure(rows: &mut [ScoredReview], threshold: f64) -> Metrics {
    let mut metrics = Metrics::default();
    for row in rows {
        for score in &mut row.scores {
            score.threshold = threshold;
            score.passes_gate = score.calibrated_score >= threshold;
        }
        let (selected, _, _) = select_domains(&row.scores);
        for (index, counts) in metrics.per_domain.iter_mut().enumerate() {
            match (row.labels[index], selected.contains(&DomainId::ALL[index])) {
                (true, true) => counts.tp += 1,
                (false, true) => counts.fp += 1,
                (true, false) => counts.false_negatives += 1,
                (false, false) => (),
            }
        }
        if !row.labels.contains(&true) {
            metrics.out_of_scope_rows += 1;
            metrics.false_out_of_scope_admissions += usize::from(!selected.is_empty());
        }
    }
    metrics
}

#[cfg(test)]
mod tests {
    use super::*;

    fn balanced(positive_score: f64, true_positives: usize) -> Vec<ReviewedPrediction> {
        let mut rows = Vec::new();
        for domain in 0..DomainId::ALL.len() {
            for family in 0..MIN_FAMILIES_PER_CLASS {
                let mut scores = [0.1; 6];
                if family < true_positives {
                    scores[domain] = positive_score;
                }
                let mut labels = [false; 6];
                labels[domain] = true;
                rows.push(ReviewedPrediction { scores, labels });
            }
        }
        rows.extend(std::iter::repeat_n(
            ReviewedPrediction {
                scores: [0.1; 6],
                labels: [false; 6],
            },
            MIN_FAMILIES_PER_CLASS,
        ));
        rows
    }

    #[test]
    fn only_calibration_selects_threshold_and_test_keeps_out_of_scope_admissions() {
        let calibration = balanced(0.705, MIN_FAMILIES_PER_CLASS);
        let mut test = balanced(0.905, MIN_FAMILIES_PER_CLASS);
        let clean = calibrate_reviewed_cycle(&calibration, &test).expect("supported cohorts");
        assert_eq!(clean.threshold, 0.7);
        assert_eq!(clean.calibration_macro_f1, 1.0);
        assert_eq!(clean.test_macro_f1, 1.0);
        assert!(clean.valid());
        assert!(clean.freeze_calibration());

        for row in test.iter_mut().filter(|row| !row.labels.contains(&true)) {
            row.scores[0] = 0.8;
        }
        let noisy = calibrate_reviewed_cycle(&calibration, &test).expect("supported cohorts");
        assert_eq!(noisy.threshold, clean.threshold);
        assert_eq!(noisy.calibration_per_domain, clean.calibration_per_domain);
        assert_eq!(
            noisy.test_false_out_of_scope_admissions,
            MIN_FAMILIES_PER_CLASS
        );
        assert!(noisy.test_macro_f1 < clean.test_macro_f1);
        assert!(noisy.valid());
    }

    #[test]
    fn mapping_agreement_excludes_unmapped_rows_from_reference_support() {
        let mut calibration = balanced(0.705, MIN_FAMILIES_PER_CLASS);
        let mut test = balanced(0.905, MIN_FAMILIES_PER_CLASS);
        let mapped_rows = MIN_FAMILIES_PER_CLASS * DomainId::ALL.len();
        calibration.truncate(mapped_rows);
        test.truncate(mapped_rows);
        let clean = calibrate_mapping_cycle(&calibration, &test).expect("supported mapped cohorts");
        assert!(clean.is_mapping_agreement());
        assert_eq!(clean.threshold, 0.7);
        assert_eq!(clean.calibration_rows, mapped_rows);
        assert_eq!(clean.test_rows, mapped_rows);
        assert!(clean.freeze_calibration());

        test.push(ReviewedPrediction {
            labels: [false; 6],
            scores: [0.999, 0.0, 0.0, 0.0, 0.0, 0.0],
        });
        let still_clean =
            calibrate_mapping_cycle(&calibration, &test).expect("unmapped row ignored");
        assert_eq!(still_clean, clean);

        test[0].scores[1] = 0.8;
        let report = calibrate_mapping_cycle(&calibration, &test).expect("mapped nonmatch counted");
        assert_eq!(report.test_per_domain[&DomainId::Security].fp, 1);
        assert_eq!(report.test_out_of_scope_rows, 0);
        assert_eq!(report.test_false_out_of_scope_admissions, 0);
        assert!(report.valid());
        assert!(calibrate_mapping_cycle(&calibration[MIN_FAMILIES_PER_CLASS..], &test).is_err());
    }

    #[test]
    fn serialized_mapping_evidence_cannot_claim_out_of_scope_admissions() {
        let complete = balanced(0.905, MIN_FAMILIES_PER_CLASS);
        let legacy = calibrate_reviewed_cycle(&complete, &complete).expect("complete cohorts");
        let legacy_json = serde_json::to_value(&legacy).expect("serialize complete report");
        assert!(legacy_json.get("reference").is_none());
        let decoded: ReviewedCycleMetrics =
            serde_json::from_value(legacy_json).expect("read legacy report");
        assert_eq!(decoded, legacy);
        assert!(!decoded.is_mapping_agreement());

        let mut mapping = calibrate_mapping_cycle(&complete, &complete).expect("mapped cohorts");
        let mapping_json = serde_json::to_value(&mapping).expect("serialize mapping report");
        assert_eq!(mapping_json["reference"], "reviewed_provider_mappings");
        let decoded: ReviewedCycleMetrics =
            serde_json::from_value(mapping_json).expect("read mapping report");
        assert_eq!(decoded, mapping);
        mapping.test_out_of_scope_rows = 1;
        mapping.test_false_out_of_scope_admissions = 1;
        assert!(!mapping.valid());
        assert!(!mapping.freeze_calibration());
    }

    #[test]
    fn scores_at_cutoff_and_rank_ties_follow_runtime_selection() {
        let mut rows = [
            ReviewedPrediction {
                scores: [0.5, 0.4999, 0.0, 0.0, 0.0, 0.0],
                labels: [true, true, false, false, false, false],
            },
            ReviewedPrediction {
                scores: [0.9, 0.9, 0.0, 0.0, 0.0, 0.0],
                labels: [true, true, false, false, false, false],
            },
            ReviewedPrediction {
                scores: [0.9, 0.8, 0.8, 0.0, 0.0, 0.0],
                labels: [true, true, true, false, false, false],
            },
            ReviewedPrediction {
                scores: [0.9, 0.9, 0.8, 0.0, 0.0, 0.0],
                labels: [true, true, true, false, false, false],
            },
        ]
        .each_ref()
        .map(scored_review);
        let metrics = measure(&mut rows, 0.5);
        assert_eq!(metrics.per_domain[0].tp, 3);
        assert_eq!(metrics.per_domain[0].false_negatives, 1);
        assert_eq!(metrics.per_domain[1].tp, 1);
        assert_eq!(metrics.per_domain[1].false_negatives, 3);
        assert_eq!(metrics.per_domain[2].tp, 0);
        assert_eq!(metrics.per_domain[2].false_negatives, 2);
    }

    #[test]
    fn gates_are_strict_and_require_consistent_whole_cohort_counts() {
        let rows = balanced(0.905, MIN_FAMILIES_PER_CLASS);
        let mut report = calibrate_reviewed_cycle(&rows, &rows).expect("supported cohorts");
        for (correct, eligible, freeze) in [
            (13, false, false),
            (14, true, false),
            (17, true, false),
            (18, true, true),
        ] {
            // Give each domain 20 positives and 20 predictions so domain F1
            // equals correct / 20 and the two boundaries are tested exactly.
            let counts = ReviewedDomainCounts {
                tp: correct,
                fp: 20 - correct,
                false_negatives: 20 - correct,
            };
            report
                .test_per_domain
                .values_mut()
                .for_each(|value| *value = counts);
            report.test_macro_f1 = counts.f1();
            assert!(report.valid());
            assert_eq!(report.promotion_eligible(), eligible);
            assert_eq!(report.freeze_calibration(), freeze);
            // A reported value can differ by rounding within the comparison
            // tolerance. The gates still use the exact measured counts.
            report.test_macro_f1 += 1e-14;
            assert!(report.valid());
            assert_eq!(report.promotion_eligible(), eligible);
            assert_eq!(report.freeze_calibration(), freeze);
        }
        report.test_per_domain.remove(&DomainId::Scientific);
        assert!(!report.valid());
        assert!(!report.freeze_calibration());
    }

    #[test]
    fn both_splits_require_complete_class_support_and_finite_scores() {
        let all = balanced(0.905, MIN_FAMILIES_PER_CLASS);
        let missing_domain = &all[MIN_FAMILIES_PER_CLASS..];
        assert!(calibrate_reviewed_cycle(&all, missing_domain).is_err());
        assert!(calibrate_reviewed_cycle(missing_domain, &all).is_err());
        let mut invalid = all.clone();
        invalid[0].scores[0] = f64::NAN;
        assert!(calibrate_reviewed_cycle(&all, &invalid).is_err());
    }
}
