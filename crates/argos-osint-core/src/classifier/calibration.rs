//! Calibration, gates, monotone isotonic regression, evaluation metrics,
//! and activation authorization for Argos classifier models.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::artifact::{ActivationReason, ModelId, ModelManifest, ValidatedArtifact, WriterLease};
use super::provider_labels::{MappingApproval, ProviderObservation};
use super::training::{
    FamilyId, LabelMembership, Split, SyntheticActivation, TrainRequest, TrainingReport,
};
use super::{
    ClassificationReceipt, ClassifierError, ClassifyInput, DomainScore, Provenance, RawDiagnostics,
    RawDomainScore, ReceiptPolicy, RejectionReason,
};
use crate::atlas_taxonomy::DomainId;

/// Observations processed per calibration activation page (512).
pub const OBSERVATIONS_PER_ACTIVATION: usize = 512;

/// Maximum number of active families retained in calibration state (10,000).
pub const MAX_RETAINED_FAMILIES: usize = 10_000;

/// Number of new eligible training families required to trigger retraining (100).
pub const RETRAIN_NEW_FAMILIES: usize = 100;

/// Minimum accepted independent families required per domain for quality acceptance (20).
pub const MIN_ACCEPTED_FAMILIES_PER_DOMAIN: usize = 20;

/// Point-estimate precision target numerator (9).
pub const PRECISION_TARGET_NUMERATOR: u64 = 9;

/// Point-estimate precision target denominator (10) -> 90% target precision.
pub const PRECISION_TARGET_DENOMINATOR: u64 = 10;

/// Canonical schema version for isotonic domain calibration.
pub const CALIBRATION_VERSION: &str = "domain-isotonic-family-v1";

/// Maximum weak-training families per provider per activation (256).
pub const MAX_WEAK_FAMILIES_PER_PROVIDER: usize = 256;

/// Maximum weak-training families per publisher per activation (32).
pub const MAX_WEAK_FAMILIES_PER_PUBLISHER: usize = 32;

/// Threshold value assigned to disabled or unsupportable domains (2.0).
pub const DISABLED_THRESHOLD: f64 = 2.0;

/// Schema version for calibration checkpoints.
pub const CALIBRATION_CHECKPOINT_SCHEMA_VERSION: &str = "classifier-calibration-checkpoint-v1";

/// Immutable fixed v1 calibration policy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CalibrationPolicy;

/// A knot in the piecewise-constant monotone isotonic calibration curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationKnot {
    pub raw_upper: f64,
    pub relevance: f64,
}

/// Fitted calibration parameters and gate threshold for a single domain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DomainCalibration {
    pub domain: DomainId,
    pub knots: Vec<CalibrationKnot>,
    pub threshold: f64,
    pub min_informative_features: usize,
}

/// Complete six-domain gate set defining the admission policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateSet {
    pub version: String,
    pub domains: [DomainCalibration; 6],
}

impl ReceiptPolicy for GateSet {
    fn receipt(
        &self,
        input: &ClassifyInput,
        raw: &[RawDomainScore; 6],
        model_id: &ModelId,
    ) -> Result<ClassificationReceipt, ClassifierError> {
        apply_gates(input, raw, model_id, self)
    }
}

/// A family representative scored by the classifier for calibration or evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoredFamily {
    pub family_id: FamilyId,
    pub representative_id: String,
    pub split: Split,
    pub provenance: Provenance,
    pub membership: [LabelMembership; 6],
    pub raw: [RawDomainScore; 6],
}

/// Specific reasons why quality promotion or domain acceptance failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceFailure {
    InsufficientAcceptedFamilies,
    PrecisionBelowTarget,
    MissingCompleteLabels,
    MissingReviewedRealEvidence,
    MissingIndependentPairEvaluation,
    MissingBaselineComparison,
    PrecisionRegression,
    MacroF1Regression,
    IncreasedFalseAdmission,
    InvalidSelectedPair,
    RuntimeBudgetRegression,
    SyntheticOptInRequired,
    ConflictingFamily,
    MissingRuntimeMeasurements,
}

/// Acceptance decision and confusion counts for a single domain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DomainAcceptance {
    pub domain: DomainId,
    pub accepted: bool,
    pub failures: Vec<EvidenceFailure>,
    pub true_positive_families: u64,
    pub false_positive_families: u64,
    pub false_negative_families: u64,
    pub true_negative_families: u64,
    pub accepted_families: u64,
    pub precision: Option<f64>,
    pub precision_ci95: Option<[f64; 2]>,
}

/// Macro evaluation metrics across all six domains.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationMetrics {
    pub domains: [DomainAcceptance; 6],
    pub macro_f1: f64,
    pub complete_families: u64,
    pub exact_pair_families: u64,
    pub false_oos_admissions: u64,
    pub false_single_domain_admissions: u64,
    pub out_of_scope_families: u64,
    pub single_domain_families: u64,
    pub invalid_selected_pairs: u64,
    pub coverage: f64,
}

/// The result of an isotonic calibration fit round.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationFit {
    pub gates: GateSet,
    pub metrics: EvaluationMetrics,
}

/// Lifecycle states of a calibration attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalibrationState {
    Pending,
    Collecting,
    Ready,
    RetrainTriggered,
    Promoted,
    Rejected,
}

/// Durable checkpoint tracking calibration job progress and retained families.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationCheckpoint {
    pub schema_version: String,
    pub job_id: String,
    pub state: CalibrationState,
    pub observation_watermark: u64,
    pub observations_processed: u64,
    pub retained_families: usize,
    pub new_eligible_training_families: usize,
    pub last_completed_fit_watermark: u64,
    pub candidate_model_id: Option<ModelId>,
    pub report_sha256: Option<String>,
    pub reasons: Vec<EvidenceFailure>,
    pub active_generation: Option<u64>,
}

/// Events driving calibration checkpoint state transitions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CalibrationEvent {
    BeginCollection,
    ActivationProcessed {
        checkpoint: ObservationActivation,
    },
    TriggerRetrain,
    /// A threshold-only candidate has a persisted report without learned-statistic retraining.
    ThresholdEvaluated {
        model_id: ModelId,
        decision: PromotionDecision,
        report_sha256: String,
    },
    CandidateEvaluated {
        model_id: ModelId,
        decision: PromotionDecision,
        report_sha256: String,
    },
    Activated {
        model_id: ModelId,
        active_generation: u64,
    },
    Reject {
        reasons: Vec<EvidenceFailure>,
        report_sha256: String,
    },
}

/// Decision on candidate model promotion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionDecision {
    Eligible,
    RetainPrior { reasons: Vec<EvidenceFailure> },
}

/// Runtime benchmark and memory metrics for promotion comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeComparison {
    pub hardware_id: String,
    pub corpus_sha256: String,
    pub measurement_reference: String,
    pub candidate_warm_p95_ms: f64,
    pub baseline_warm_p95_ms: f64,
    pub foreground_without_calibration_p95_ms: f64,
    pub foreground_with_calibration_p95_ms: f64,
    pub peak_heap_bytes: u64,
    pub configured_stack_limit_bytes: u64,
    pub stack_limit_fixture_passed: bool,
    pub peak_reservation_bytes: u64,
}

/// Comprehensive evidence required for automatic candidate promotion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromotionEvidence {
    pub candidate_model_id: ModelId,
    pub candidate_payload_sha256: String,
    pub candidate_report_sha256: String,
    pub provenance: Provenance,
    pub independent_review_reference: String,
    pub reviewed_corpus_sha256: String,
    pub family_assignment_sha256: String,
    pub baseline_model_id: Option<ModelId>,
    pub baseline_active_generation: Option<u64>,
    pub candidate: EvaluationMetrics,
    pub baseline: Option<EvaluationMetrics>,
    pub evaluation_family_ids: Vec<FamilyId>,
    /// Independent holdout source attribution, copied from the evaluated family records.
    /// Missing legacy evidence is insufficient for promotion.
    #[serde(default)]
    pub evaluation_family_provenance: Vec<(FamilyId, Provenance)>,
    pub training_family_ids: Vec<FamilyId>,
    pub threshold_fit_family_ids: Vec<FamilyId>,
    pub split: Split,
    pub runtime: Option<RuntimeComparison>,
}

/// An incoming observation envelope tagged with watermark and story family.
#[derive(Debug, Clone, PartialEq)]
pub struct ObservationEnvelope {
    pub watermark: u64,
    pub observation: ProviderObservation,
    pub family_id: FamilyId,
    pub split: Split,
}

/// Permanent record of a story family tracked in the active corpus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FamilyRecord {
    pub family_id: FamilyId,
    pub split: Split,
    pub first_seen_watermark: u64,
    pub last_seen_watermark: u64,
    pub eligible_training: bool,
    pub eligibility_watermark: Option<u64>,
    pub protected: bool,
    /// Approved membership and source lineage of the first selected representative.
    /// Missing on legacy records; such records cannot silently gain new training evidence.
    #[serde(default)]
    pub approved_membership: Option<[LabelMembership; 6]>,
    #[serde(default)]
    pub selected_provider: Option<String>,
    #[serde(default)]
    pub selected_publisher: Option<String>,
}

/// Disposition reason for an individual provider observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationReason {
    AcceptedForMonitoring,
    EligiblePartialTraining,
    ExcludedLabels,
    UnknownVocabulary,
    UnsupportedPayload,
    ConflictingEvidence,
    Duplicate,
    ProviderCap,
    PublisherCap,
    WeakContributionCap,
    UnsupportedLanguage,
    EvictedFamily,
}

/// Disposition recorded for a processed observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationDisposition {
    pub observation_id: String,
    pub family_id: FamilyId,
    pub reason: ObservationReason,
}

/// Running budget for weak-label contributions from provider data.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WeakTrainingBudget {
    pub seed_positive_families: [u64; 6],
    pub selected_weak_positive_families: [u64; 6],
    pub by_provider: BTreeMap<String, u64>,
    pub by_publisher: BTreeMap<String, u64>,
    pub counted_family_ids: BTreeSet<FamilyId>,
}

/// Output of processing an observation page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservationActivation {
    pub next_watermark: u64,
    pub processed: u64,
    pub families: Vec<FamilyRecord>,
    pub budget: WeakTrainingBudget,
    pub dispositions: Vec<ObservationDisposition>,
    pub new_eligible_training_families: usize,
}

/// High-level outcome of an activation decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationOutcome {
    CandidateOnly {
        reason_code: String,
        reason: String,
        failures: Vec<EvidenceFailure>,
    },
    Activated {
        reason: ActivationReason,
        model_id: ModelId,
        generation: u64,
    },
}

/// An unforgeable authorization token permitting model activation.
#[derive(Debug)]
pub struct ActivationAuthorization {
    pub(super) candidate_model_id: ModelId,
    pub(super) payload_sha256: String,
    pub(super) report_sha256: String,
    pub(super) manifest_sha256: String,
    pub(super) expected_prior_id: Option<String>,
    pub(super) expected_prior_generation: Option<u64>,
    pub(super) reason: ActivationReason,
    pub(super) writer_lease_id: u64,
    // Present only for explicit manual repair; ordinary promotion still requires
    // an executable-compatible prior and cannot use this identity-only path.
    pub(super) manual_repair_prior_sha256: Option<String>,
}

impl ActivationAuthorization {
    /// Issues a capability bound to the writer that evaluated the evidence.
    #[must_use]
    pub(super) fn new(
        lease: &WriterLease,
        candidate_model_id: ModelId,
        payload_sha256: String,
        report_sha256: String,
        manifest_sha256: String,
        expected_prior_id: Option<String>,
        expected_prior_generation: Option<u64>,
        reason: ActivationReason,
    ) -> Self {
        Self {
            candidate_model_id,
            payload_sha256,
            report_sha256,
            manifest_sha256,
            expected_prior_id,
            expected_prior_generation,
            reason,
            writer_lease_id: lease.identity(),
            manual_repair_prior_sha256: None,
        }
    }
}

/// Authorization decision returned by `authorize_activation`.
#[derive(Debug)]
pub enum ActivationAuthorizationDecision {
    Authorized(ActivationAuthorization),
    RetainPrior(ActivationOutcome),
}

// -----------------------------------------------------------------------------
// Monotone Isotonic Transform & Gate Application
// -----------------------------------------------------------------------------

fn invalid_calibration(reason: impl Into<String>) -> ClassifierError {
    ClassifierError::InvalidCalibrationData {
        family_id: None,
        reason: reason.into(),
    }
}

fn validate_domain_calibration(calibration: &DomainCalibration) -> Result<(), ClassifierError> {
    if !calibration.threshold.is_finite()
        || !(0.0..=DISABLED_THRESHOLD).contains(&calibration.threshold)
        || calibration.min_informative_features == 0
    {
        return Err(invalid_calibration("invalid_domain_gate"));
    }
    let mut previous: Option<&CalibrationKnot> = None;
    for knot in &calibration.knots {
        if !knot.raw_upper.is_finite()
            || !knot.relevance.is_finite()
            || !(0.0..=1.0).contains(&knot.relevance)
            || previous
                .is_some_and(|p| knot.raw_upper <= p.raw_upper || knot.relevance < p.relevance)
        {
            return Err(invalid_calibration("invalid_isotonic_knots"));
        }
        previous = Some(knot);
    }
    Ok(())
}

fn validate_gates(gates: &GateSet) -> Result<(), ClassifierError> {
    if gates.version != CALIBRATION_VERSION {
        return Err(invalid_calibration("unsupported_calibration_version"));
    }
    for (domain, calibration) in DomainId::ALL.iter().zip(&gates.domains) {
        if *domain != calibration.domain {
            return Err(invalid_calibration("noncanonical_gate_domains"));
        }
        validate_domain_calibration(calibration)?;
    }
    Ok(())
}

fn validate_raw(
    raw: &[RawDomainScore; 6],
    family: Option<&FamilyId>,
) -> Result<(), ClassifierError> {
    for (domain, score) in DomainId::ALL.iter().zip(raw) {
        if *domain != score.domain
            || !score.present_log_score.is_finite()
            || !score.absent_log_score.is_finite()
            || !score.raw_log_odds.is_finite()
            || (score.raw_log_odds - (score.present_log_score - score.absent_log_score)).abs()
                > 1e-12
            || !(score.present_log_score - score.absent_log_score).is_finite()
        {
            return Err(ClassifierError::InvalidCalibrationData {
                family_id: family.map(ToString::to_string),
                reason: "invalid_raw_domain_scores".into(),
            });
        }
    }
    Ok(())
}

fn validate_representatives(rows: &[ScoredFamily], fitting: bool) -> Result<(), ClassifierError> {
    if rows.len() > MAX_RETAINED_FAMILIES {
        return Err(invalid_calibration("too_many_calibration_families"));
    }
    let mut seen = BTreeSet::new();
    for row in rows {
        if !seen.insert(&row.family_id) {
            return Err(invalid_calibration("duplicate_calibration_family"));
        }
        if row.membership.contains(&LabelMembership::Unknown)
            || (fitting && row.split != Split::Calibration)
            || (!fitting && row.split == Split::Train)
        {
            return Err(invalid_calibration(
                "evaluation_requires_complete_holdout_families",
            ));
        }
        validate_raw(&row.raw, Some(&row.family_id))?;
    }
    Ok(())
}

/// Scores raw log odds through a domain's monotonic isotonic calibration curve.
///
/// # Errors
/// Returns [`ClassifierError::InvalidCalibrationData`] if the raw score or calibration curve is invalid.
pub fn score(raw_log_odds: f64, calibration: &DomainCalibration) -> Result<f64, ClassifierError> {
    if !raw_log_odds.is_finite() {
        return Err(ClassifierError::InvalidCalibrationData {
            family_id: None,
            reason: "raw_log_odds must be finite".into(),
        });
    }

    validate_domain_calibration(calibration)?;
    let Some(last) = calibration.knots.last() else {
        return Ok(0.0);
    };

    // Piecewise constant lookup: first knot with raw_log_odds <= knot.raw_upper
    for knot in &calibration.knots {
        if raw_log_odds <= knot.raw_upper {
            return Ok(knot.relevance.clamp(0.0, 1.0));
        }
    }

    // If beyond the highest upper bound, clamp to the last knot's relevance
    Ok(last.relevance)
}

#[derive(Debug, Clone)]
struct PavBlock {
    max_raw: f64,
    weight: f64,
    sum_positive: f64,
    mean: f64,
}

/// Fits a monotone isotonic relevance transform for each domain using the
/// Pool Adjacent Violators (PAV) algorithm.
///
/// # Errors
/// Returns [`ClassifierError::InvalidCalibrationData`] if non-finite scores are encountered.
pub fn fit(
    rows: &[ScoredFamily],
    _policy: &CalibrationPolicy,
) -> Result<CalibrationFit, ClassifierError> {
    validate_representatives(rows, true)?;
    let mut domain_calibrations: Vec<DomainCalibration> = Vec::with_capacity(6);

    for (domain_idx, domain) in DomainId::ALL.iter().enumerate() {
        // Collect (raw_log_odds, is_positive) pairs for complete-label rows
        let mut pairs: Vec<(f64, f64)> = Vec::new();

        for row in rows {
            let m = row.membership[domain_idx];
            match m {
                LabelMembership::Present => {
                    let raw = row.raw[domain_idx].raw_log_odds;
                    if !raw.is_finite() {
                        return Err(ClassifierError::InvalidCalibrationData {
                            family_id: Some(row.family_id.to_string()),
                            reason: "non-finite raw log odds encountered in calibration fit".into(),
                        });
                    }
                    pairs.push((raw, 1.0));
                }
                LabelMembership::Absent => {
                    let raw = row.raw[domain_idx].raw_log_odds;
                    if !raw.is_finite() {
                        return Err(ClassifierError::InvalidCalibrationData {
                            family_id: Some(row.family_id.to_string()),
                            reason: "non-finite raw log odds encountered in calibration fit".into(),
                        });
                    }
                    pairs.push((raw, 0.0));
                }
                LabelMembership::Unknown => {}
            }
        }

        if pairs.is_empty() {
            domain_calibrations.push(DomainCalibration {
                domain: *domain,
                knots: Vec::new(),
                threshold: DISABLED_THRESHOLD,
                min_informative_features: 1,
            });
            continue;
        }

        // Sort ascending by raw log odds
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0));

        // Group identical raw log odds into initial blocks
        let mut blocks: Vec<PavBlock> = Vec::new();
        for (raw, is_pos) in pairs {
            if let Some(last_block) = blocks.last_mut() {
                if last_block.max_raw == raw {
                    last_block.weight += 1.0;
                    last_block.sum_positive += is_pos;
                    last_block.mean = last_block.sum_positive / last_block.weight;
                    continue;
                }
            }
            blocks.push(PavBlock {
                max_raw: raw,
                weight: 1.0,
                sum_positive: is_pos,
                mean: is_pos,
            });
        }

        // Pool Adjacent Violators: maintain nondecreasing block means
        let mut i = 0;
        while i + 1 < blocks.len() {
            if blocks[i].mean > blocks[i + 1].mean {
                // Merge blocks i and i+1
                let next = blocks.remove(i + 1);
                blocks[i].max_raw = next.max_raw;
                blocks[i].weight += next.weight;
                blocks[i].sum_positive += next.sum_positive;
                blocks[i].mean = blocks[i].sum_positive / blocks[i].weight;

                // Step back if previous block violates monotonicity
                i = i.saturating_sub(1);
            } else {
                i += 1;
            }
        }

        // Generate knots from merged blocks
        let knots: Vec<CalibrationKnot> = blocks
            .into_iter()
            .map(|b| CalibrationKnot {
                raw_upper: b.max_raw,
                relevance: b.mean.clamp(0.0, 1.0),
            })
            .collect();

        // Threshold search: choose distinct relevance score that maximizes accepted families
        // subject to precision >= 90% (10 * TP >= 9 * (TP + FP))
        let mut candidate_thresholds: Vec<f64> = knots.iter().map(|k| k.relevance).collect();
        candidate_thresholds.sort_by(f64::total_cmp);
        candidate_thresholds.dedup();

        let mut best_threshold = DISABLED_THRESHOLD;
        let mut max_accepted_families = 0u64;

        for t in candidate_thresholds.into_iter().rev() {
            let temp_calib = DomainCalibration {
                domain: *domain,
                knots: knots.clone(),
                threshold: t,
                min_informative_features: 1,
            };

            let mut tp = 0u64;
            let mut fp = 0u64;

            for row in rows {
                let m = row.membership[domain_idx];
                let raw_score = row.raw[domain_idx];
                if raw_score.known_feature_count >= 1 {
                    let rel = score(raw_score.raw_log_odds, &temp_calib)?;
                    if rel >= t {
                        if m == LabelMembership::Present {
                            tp += 1;
                        } else if m == LabelMembership::Absent {
                            fp += 1;
                        }
                    }
                }
            }

            let accepted = tp + fp;
            if accepted > 0
                && 10 * tp >= 9 * accepted
                && (accepted > max_accepted_families
                    || (accepted == max_accepted_families && t > best_threshold))
            {
                max_accepted_families = accepted;
                best_threshold = t;
            }
        }

        domain_calibrations.push(DomainCalibration {
            domain: *domain,
            knots,
            threshold: best_threshold,
            min_informative_features: 1,
        });
    }

    let gates = GateSet {
        version: CALIBRATION_VERSION.into(),
        domains: [
            domain_calibrations[0].clone(),
            domain_calibrations[1].clone(),
            domain_calibrations[2].clone(),
            domain_calibrations[3].clone(),
            domain_calibrations[4].clone(),
            domain_calibrations[5].clone(),
        ],
    };

    let metrics = evaluate(rows, &gates)?;

    Ok(CalibrationFit { gates, metrics })
}

/// Evaluates family representatives against a fitted gate set.
///
/// # Errors
/// Returns [`ClassifierError`] on invalid inputs.
pub fn evaluate(
    rows: &[ScoredFamily],
    gates: &GateSet,
) -> Result<EvaluationMetrics, ClassifierError> {
    validate_representatives(rows, false)?;
    validate_gates(gates)?;
    let mut acceptances: Vec<DomainAcceptance> = Vec::with_capacity(6);
    let mut f1_sum = 0.0;

    for (domain_idx, domain_calib) in gates.domains.iter().enumerate() {
        let domain = domain_calib.domain;
        let mut tp = 0u64;
        let mut fp = 0u64;
        let mut fn_count = 0u64;
        let mut tn = 0u64;

        for row in rows {
            let m = row.membership[domain_idx];
            let raw_score = row.raw[domain_idx];
            let fires = raw_score.known_feature_count >= domain_calib.min_informative_features
                && score(raw_score.raw_log_odds, domain_calib)? >= domain_calib.threshold;

            match m {
                LabelMembership::Present => {
                    if fires {
                        tp += 1;
                    } else {
                        fn_count += 1;
                    }
                }
                LabelMembership::Absent => {
                    if fires {
                        fp += 1;
                    } else {
                        tn += 1;
                    }
                }
                LabelMembership::Unknown => {}
            }
        }

        let accepted_families = tp + fp;
        let mut failures = Vec::new();

        if accepted_families < (MIN_ACCEPTED_FAMILIES_PER_DOMAIN as u64) {
            failures.push(EvidenceFailure::InsufficientAcceptedFamilies);
        }

        let precision = if accepted_families > 0 {
            let p = tp as f64 / accepted_families as f64;
            if 10 * tp < 9 * accepted_families {
                failures.push(EvidenceFailure::PrecisionBelowTarget);
            }
            Some(p)
        } else {
            failures.push(EvidenceFailure::InsufficientAcceptedFamilies);
            failures.push(EvidenceFailure::PrecisionBelowTarget);
            None
        };

        // Compute Wilson 95% CI (z = 1.959963984540054)
        let precision_ci95 = precision.map(|p| {
            let n = accepted_families as f64;
            let z = 1.959_963_984_540_054;
            let z2 = z * z;
            let denom = 1.0 + z2 / n;
            let center = p + z2 / (2.0 * n);
            let margin = z * ((p * (1.0 - p) / n) + (z2 / (4.0 * n * n))).sqrt();
            let lower = ((center - margin) / denom).clamp(0.0, 1.0);
            let upper = ((center + margin) / denom).clamp(0.0, 1.0);
            [lower, upper]
        });

        // Compute domain F1
        let f1_denom = 2 * tp + fp + fn_count;
        if f1_denom > 0 {
            f1_sum += (2 * tp) as f64 / f1_denom as f64;
        }

        acceptances.push(DomainAcceptance {
            domain,
            accepted: failures.is_empty(),
            failures,
            true_positive_families: tp,
            false_positive_families: fp,
            false_negative_families: fn_count,
            true_negative_families: tn,
            accepted_families,
            precision,
            precision_ci95,
        });
    }

    let macro_f1 = f1_sum / 6.0;

    let complete_families = rows.len() as u64;
    let mut exact_pair_families = 0;
    let mut false_oos_admissions = 0;
    let mut false_single_domain_admissions = 0;
    let mut out_of_scope_families = 0;
    let mut single_domain_families = 0;
    let mut invalid_selected_pairs = 0;
    let mut admitted = 0u64;
    let model_id = ModelId::parse("evaluation")
        .ok_or_else(|| invalid_calibration("invalid_evaluation_model_id"))?;
    for row in rows {
        let truth_count = row
            .membership
            .iter()
            .filter(|m| **m == LabelMembership::Present)
            .count();
        if truth_count == 0 {
            out_of_scope_families += 1;
        }
        if truth_count == 1 {
            single_domain_families += 1;
        }
        let input = ClassifyInput::new(
            &row.representative_id,
            "Evaluation",
            "Family representative",
            Some("en".into()),
        );
        let receipt = apply_gates(&input, &row.raw, &model_id, gates)?;
        if let Some(pair) = receipt.selected_pair {
            admitted += 1;
            if truth_count == 0 {
                false_oos_admissions += 1;
            }
            if truth_count == 1 {
                false_single_domain_admissions += 1;
            }
            let both_true = pair.iter().all(|domain| {
                DomainId::ALL
                    .iter()
                    .position(|d| d == domain)
                    .is_some_and(|idx| row.membership[idx] == LabelMembership::Present)
            });
            if pair[0] == pair[1] || !both_true {
                invalid_selected_pairs += 1;
            } else if truth_count == 2 {
                exact_pair_families += 1;
            }
        }
    }

    Ok(EvaluationMetrics {
        domains: [
            acceptances[0].clone(),
            acceptances[1].clone(),
            acceptances[2].clone(),
            acceptances[3].clone(),
            acceptances[4].clone(),
            acceptances[5].clone(),
        ],
        macro_f1,
        complete_families,
        exact_pair_families,
        false_oos_admissions,
        false_single_domain_admissions,
        out_of_scope_families,
        single_domain_families,
        invalid_selected_pairs,
        coverage: if complete_families == 0 {
            0.0
        } else {
            admitted as f64 / complete_families as f64
        },
    })
}

/// Applies calibrated gate thresholds and selects the top-two domains.
///
/// If calibrated scores tie, ties are broken using `DomainId.as_str()` lexical
/// ascending order (e.g. `[Economic, Environmental]`), NOT canonical enum order.
///
fn make_receipt(
    input: &ClassifyInput,
    model_id: &ModelId,
    gates: &GateSet,
    raw: &[RawDomainScore; 6],
    scores: Vec<DomainScore>,
    selected_pair: Option<[DomainId; 2]>,
    rejection_reason: Option<RejectionReason>,
) -> ClassificationReceipt {
    let mut receipt = ClassificationReceipt::new(
        input.occurrence_id.clone(),
        input.input_hash.clone(),
        model_id.as_str(),
    );
    receipt.scores = scores;
    receipt.selected_pair = selected_pair;
    receipt.rejection_reason = rejection_reason;
    receipt.raw_diagnostics = Some(RawDiagnostics {
        schema_version: "classifier-raw-diagnostics-v1".into(),
        calibration_version: gates.version.clone(),
        domains: *raw,
        provider: None,
    });
    receipt
}

/// Applies calibrated gates to live classifier raw scores, enforcing:
/// 1. Minimum informative feature gates.
/// 2. Calibrated relevance threshold gates.
/// 3. Top-2 selection with lexical tie-breaking.
/// 4. Rejection code priority.
pub fn apply_gates(
    input: &ClassifyInput,
    raw: &[RawDomainScore; 6],
    model_id: &ModelId,
    gates: &GateSet,
) -> Result<ClassificationReceipt, ClassifierError> {
    validate_gates(gates)?;
    validate_raw(raw, None)?;
    let expected = ClassifyInput::new(
        input.occurrence_id.clone(),
        input.title.clone(),
        input.description.clone(),
        input.language.clone(),
    );
    if input.input_hash != expected.input_hash {
        return Err(ClassifierError::InvalidCalibrationData {
            family_id: None,
            reason: "input_hash_mismatch".into(),
        });
    }
    // 1. Check blank input
    if input.title.trim().is_empty() && input.description.trim().is_empty() {
        let scores = DomainId::ALL
            .iter()
            .enumerate()
            .map(|(idx, &domain)| DomainScore {
                domain,
                raw_log_odds: raw[idx].raw_log_odds,
                calibrated_score: 0.0,
                threshold: gates.domains[idx].threshold,
                informative_features: raw[idx].known_feature_count,
                oov_features: raw[idx].oov_count,
                passes_gate: false,
            })
            .collect();
        return Ok(make_receipt(
            input,
            model_id,
            gates,
            raw,
            scores,
            None,
            Some(RejectionReason::Blank),
        ));
    }

    // 2. Check unsupported language if language tag is provided
    if let Some(lang) = &input.language {
        let lang_lower = lang.trim().to_ascii_lowercase();
        if !lang_lower.is_empty() && lang_lower != "en" && !lang_lower.starts_with("en-") {
            let scores = DomainId::ALL
                .iter()
                .enumerate()
                .map(|(idx, &domain)| DomainScore {
                    domain,
                    raw_log_odds: raw[idx].raw_log_odds,
                    calibrated_score: 0.0,
                    threshold: gates.domains[idx].threshold,
                    informative_features: raw[idx].known_feature_count,
                    oov_features: raw[idx].oov_count,
                    passes_gate: false,
                })
                .collect();
            return Ok(make_receipt(
                input,
                model_id,
                gates,
                raw,
                scores,
                None,
                Some(RejectionReason::UnsupportedLanguage),
            ));
        }
    }

    // 3. Check wholly OOV (zero known features across all 6 domains)
    if raw.iter().all(|s| s.known_feature_count == 0) {
        let scores = DomainId::ALL
            .iter()
            .enumerate()
            .map(|(idx, &domain)| DomainScore {
                domain,
                raw_log_odds: raw[idx].raw_log_odds,
                calibrated_score: 0.0,
                threshold: gates.domains[idx].threshold,
                informative_features: raw[idx].known_feature_count,
                oov_features: raw[idx].oov_count,
                passes_gate: false,
            })
            .collect();
        return Ok(make_receipt(
            input,
            model_id,
            gates,
            raw,
            scores,
            None,
            Some(RejectionReason::WhollyOov),
        ));
    }

    // 4. Compute calibrated relevance for all 6 domains
    let mut domain_scores = Vec::with_capacity(6);
    for (idx, domain_calib) in gates.domains.iter().enumerate() {
        let rel = score(raw[idx].raw_log_odds, domain_calib)?;
        let passes = raw[idx].known_feature_count >= domain_calib.min_informative_features
            && rel >= domain_calib.threshold;
        domain_scores.push((domain_calib.domain, rel, passes));
    }

    // Sort ranked candidates:
    // 1. Calibrated relevance descending
    // 2. Lexical ascending order of domain name: DomainId.as_str()
    domain_scores.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then_with(|| a.0.as_str().cmp(b.0.as_str()))
    });

    // Check top 2 domains
    let top1 = domain_scores[0];
    let top2 = domain_scores[1];

    let (selected_pair, rejection_reason) = if top1.2 && top2.2 {
        (Some([top1.0, top2.0]), None)
    } else {
        (None, Some(RejectionReason::InsufficientDomains))
    };

    // Build receipt domain scores in canonical DomainId::ALL order
    let canonical_scores: Vec<DomainScore> = DomainId::ALL
        .iter()
        .enumerate()
        .map(|(idx, &domain)| {
            let rel = score(raw[idx].raw_log_odds, &gates.domains[idx])?;
            let passes = raw[idx].known_feature_count
                >= gates.domains[idx].min_informative_features
                && rel >= gates.domains[idx].threshold;
            Ok(DomainScore {
                domain,
                raw_log_odds: raw[idx].raw_log_odds,
                calibrated_score: rel,
                threshold: gates.domains[idx].threshold,
                informative_features: raw[idx].known_feature_count,
                oov_features: raw[idx].oov_count,
                passes_gate: passes,
            })
        })
        .collect::<Result<_, ClassifierError>>()?;

    Ok(make_receipt(
        input,
        model_id,
        gates,
        raw,
        canonical_scores,
        selected_pair,
        rejection_reason,
    ))
}

// -----------------------------------------------------------------------------
// State Machine & Observation Paging
// -----------------------------------------------------------------------------

/// Transitions a calibration checkpoint to a new state based on an event.
///
/// # Errors
/// Returns [`ClassifierError::InvalidTransition`] if the state transition is illegal.
pub fn transition(
    checkpoint: &CalibrationCheckpoint,
    event: CalibrationEvent,
) -> Result<CalibrationCheckpoint, ClassifierError> {
    validate_checkpoint(checkpoint)?;
    let mut next = checkpoint.clone();

    match (checkpoint.state, event) {
        (CalibrationState::Pending, CalibrationEvent::BeginCollection) => {
            next.state = CalibrationState::Collecting;
        }
        (CalibrationState::Collecting, CalibrationEvent::BeginCollection) => {
            // Idempotent start
        }
        (
            CalibrationState::Collecting,
            CalibrationEvent::ActivationProcessed { checkpoint: act },
        ) => {
            if act.next_watermark < next.observation_watermark
                || act.processed > OBSERVATIONS_PER_ACTIVATION as u64
                || act.families.len() > MAX_RETAINED_FAMILIES
                || act.new_eligible_training_families > act.processed as usize
            {
                return Err(invalid_calibration("invalid_observation_activation"));
            }
            next.observation_watermark = act.next_watermark;
            next.observations_processed = next
                .observations_processed
                .checked_add(act.processed)
                .ok_or_else(|| invalid_calibration("observation_counter_overflow"))?;
            next.retained_families = act.families.len();
            next.new_eligible_training_families = next
                .new_eligible_training_families
                .checked_add(act.new_eligible_training_families)
                .ok_or_else(|| invalid_calibration("family_counter_overflow"))?;

            if next.new_eligible_training_families >= RETRAIN_NEW_FAMILIES {
                next.state = CalibrationState::RetrainTriggered;
            }
        }
        (CalibrationState::Collecting, CalibrationEvent::TriggerRetrain)
            if checkpoint.new_eligible_training_families >= RETRAIN_NEW_FAMILIES =>
        {
            next.state = CalibrationState::RetrainTriggered;
        }
        (
            CalibrationState::RetrainTriggered,
            CalibrationEvent::CandidateEvaluated {
                model_id,
                decision,
                report_sha256,
            },
        ) => {
            if !valid_report_hash(&report_sha256) {
                return Err(invalid_calibration("invalid_report_sha256"));
            }
            next.candidate_model_id = Some(model_id);
            next.report_sha256 = Some(report_sha256);
            next.last_completed_fit_watermark = next.observation_watermark;
            next.new_eligible_training_families = 0;
            next.state = CalibrationState::Ready;
            next.reasons = match decision {
                PromotionDecision::Eligible => Vec::new(),
                PromotionDecision::RetainPrior { reasons } if !reasons.is_empty() => reasons,
                _ => return Err(invalid_calibration("missing_rejection_reasons")),
            };
        }
        (
            CalibrationState::Collecting,
            CalibrationEvent::ThresholdEvaluated {
                model_id,
                decision,
                report_sha256,
            },
        ) => {
            if !valid_report_hash(&report_sha256) {
                return Err(invalid_calibration("invalid_report_sha256"));
            }
            next.candidate_model_id = Some(model_id);
            next.report_sha256 = Some(report_sha256);
            next.state = CalibrationState::Ready;
            next.reasons = match decision {
                PromotionDecision::Eligible => Vec::new(),
                PromotionDecision::RetainPrior { reasons } if !reasons.is_empty() => reasons,
                _ => return Err(invalid_calibration("missing_rejection_reasons")),
            };
        }
        (
            CalibrationState::Ready,
            CalibrationEvent::Activated {
                model_id,
                active_generation,
            },
        ) => {
            if next.candidate_model_id.as_ref() != Some(&model_id)
                || !next.reasons.is_empty()
                || active_generation == 0
                || next
                    .active_generation
                    .is_some_and(|prior| active_generation <= prior)
            {
                return Err(invalid_calibration(
                    "activation_checkpoint_identity_mismatch",
                ));
            }
            next.candidate_model_id = Some(model_id);
            next.active_generation = Some(active_generation);
            next.state = CalibrationState::Promoted;
        }
        (
            CalibrationState::Ready,
            CalibrationEvent::Reject {
                reasons,
                report_sha256,
            },
        ) => {
            if reasons.is_empty() || next.report_sha256.as_deref() != Some(report_sha256.as_str()) {
                return Err(invalid_calibration("invalid_rejection_report"));
            }
            next.reasons = reasons;
            next.report_sha256 = Some(report_sha256);
            next.state = CalibrationState::Rejected;
        }
        (from, evt) => {
            return Err(ClassifierError::InvalidTransition {
                from: format!("{from:?}"),
                event: format!("{evt:?}"),
            });
        }
    }

    validate_checkpoint(&next)?;
    Ok(next)
}

fn valid_report_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}
fn validate_checkpoint(checkpoint: &CalibrationCheckpoint) -> Result<(), ClassifierError> {
    if checkpoint.schema_version != CALIBRATION_CHECKPOINT_SCHEMA_VERSION
        || checkpoint.job_id.is_empty()
        || checkpoint.job_id.len() > 256
        || checkpoint.job_id.chars().any(char::is_control)
        || checkpoint.retained_families > MAX_RETAINED_FAMILIES
        || checkpoint.new_eligible_training_families > MAX_RETAINED_FAMILIES
        || checkpoint.last_completed_fit_watermark > checkpoint.observation_watermark
        || checkpoint
            .report_sha256
            .as_deref()
            .is_some_and(|hash| !valid_report_hash(hash))
        || checkpoint.active_generation == Some(0)
        || (matches!(
            checkpoint.state,
            CalibrationState::Ready | CalibrationState::Promoted | CalibrationState::Rejected
        ) && (checkpoint.candidate_model_id.is_none() || checkpoint.report_sha256.is_none()))
        || (checkpoint.state == CalibrationState::Promoted
            && (checkpoint.active_generation.is_none() || !checkpoint.reasons.is_empty()))
    {
        return Err(invalid_calibration("invalid_calibration_checkpoint"));
    }
    Ok(())
}

fn validate_metrics(metrics: &EvaluationMetrics) -> Result<(), ClassifierError> {
    let total = metrics.complete_families;
    let unit = |value: f64| value.is_finite() && (0.0..=1.0).contains(&value);
    let invalid = || invalid_calibration("inconsistent_evaluation_metrics");
    if total > MAX_RETAINED_FAMILIES as u64
        || !unit(metrics.macro_f1)
        || !unit(metrics.coverage)
        || metrics
            .out_of_scope_families
            .checked_add(metrics.single_domain_families)
            .is_none_or(|count| count > total)
        || metrics.exact_pair_families
            > total
                .saturating_sub(metrics.out_of_scope_families)
                .saturating_sub(metrics.single_domain_families)
        || metrics.false_oos_admissions > metrics.out_of_scope_families
        || metrics.false_single_domain_admissions > metrics.single_domain_families
        || metrics.invalid_selected_pairs > total
        || metrics
            .false_oos_admissions
            .checked_add(metrics.false_single_domain_admissions)
            .is_none_or(|count| count > metrics.invalid_selected_pairs)
    {
        return Err(invalid());
    }
    let admitted_float = metrics.coverage * total as f64;
    let admitted = admitted_float.round();
    if (admitted - admitted_float).abs() > 1e-8
        || admitted < (metrics.exact_pair_families + metrics.invalid_selected_pairs) as f64
    {
        return Err(invalid());
    }
    let mut f1_sum = 0.0;
    for (idx, domain) in metrics.domains.iter().enumerate() {
        let tp = domain.true_positive_families;
        let fp = domain.false_positive_families;
        let fn_count = domain.false_negative_families;
        let tn = domain.true_negative_families;
        let accepted = tp.checked_add(fp).ok_or_else(invalid)?;
        let summed = accepted
            .checked_add(fn_count)
            .and_then(|n| n.checked_add(tn));
        let precision = (accepted != 0).then(|| tp as f64 / accepted as f64);
        let should_accept = accepted >= MIN_ACCEPTED_FAMILIES_PER_DOMAIN as u64
            && 10 * u128::from(tp) >= 9 * u128::from(accepted);
        if domain.domain != DomainId::ALL[idx]
            || summed != Some(total)
            || domain.accepted_families != accepted
            || domain.accepted != should_accept
            || domain.accepted != domain.failures.is_empty()
            || match (domain.precision, precision) {
                (Some(actual), Some(expected)) => {
                    !unit(actual) || (actual - expected).abs() > 1e-12
                }
                (None, None) => false,
                _ => true,
            }
            || match domain.precision_ci95 {
                Some([lo, hi]) => accepted == 0 || !unit(lo) || !unit(hi) || lo > hi,
                None => accepted != 0,
            }
        {
            return Err(invalid());
        }
        let denominator = 2 * u128::from(tp) + u128::from(fp) + u128::from(fn_count);
        if denominator > 0 {
            f1_sum += (2 * u128::from(tp)) as f64 / denominator as f64;
        }
    }
    if (metrics.macro_f1 - f1_sum / 6.0).abs() > 1e-12 {
        return Err(invalid());
    }
    Ok(())
}

/// Evaluates quality criteria to determine whether a candidate model is eligible
/// for automatic promotion.
///
/// # Errors
/// Returns [`ClassifierError`] on invalid comparison data.
pub fn assess_promotion(
    evidence: &PromotionEvidence,
) -> Result<PromotionDecision, ClassifierError> {
    validate_metrics(&evidence.candidate)?;
    if let Some(baseline) = &evidence.baseline {
        validate_metrics(baseline)?;
    }
    super::training::validate_provenance(&evidence.provenance)?;
    let mut reasons = Vec::new();
    let mut fail = |reason| {
        if !reasons.contains(&reason) {
            reasons.push(reason);
        }
    };
    let valid_hash = |s: &str| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit());
    if !matches!(evidence.provenance, Provenance::ReviewedReal { .. })
        || evidence.split != Split::Development
        || evidence.independent_review_reference.trim().is_empty()
        || !valid_hash(&evidence.reviewed_corpus_sha256)
        || !valid_hash(&evidence.family_assignment_sha256)
        || !valid_hash(&evidence.candidate_payload_sha256)
        || !valid_hash(&evidence.candidate_report_sha256)
    {
        fail(EvidenceFailure::MissingReviewedRealEvidence);
    }
    let eval_set: BTreeSet<_> = evidence.evaluation_family_ids.iter().collect();
    let train_set: BTreeSet<_> = evidence.training_family_ids.iter().collect();
    let fit_set: BTreeSet<_> = evidence.threshold_fit_family_ids.iter().collect();
    if eval_set.is_empty()
        || eval_set.len() != evidence.evaluation_family_ids.len()
        || !eval_set.is_disjoint(&train_set)
        || !eval_set.is_disjoint(&fit_set)
        || !fit_set.is_disjoint(&train_set)
    {
        fail(EvidenceFailure::ConflictingFamily);
    }
    let mut evaluated_sources = BTreeSet::new();
    for (family, provenance) in &evidence.evaluation_family_provenance {
        if !evaluated_sources.insert(family)
            || !eval_set.contains(family)
            || super::training::validate_provenance(provenance).is_err()
            || !matches!(provenance, Provenance::ReviewedReal { dataset_sha256, review_reference, .. }
                if dataset_sha256 == &evidence.reviewed_corpus_sha256 && review_reference == &evidence.independent_review_reference)
        {
            fail(EvidenceFailure::MissingReviewedRealEvidence);
        }
    }
    if evaluated_sources != eval_set {
        fail(EvidenceFailure::MissingReviewedRealEvidence);
    }
    if evidence.candidate.complete_families != eval_set.len() as u64
        || evidence.candidate.complete_families == 0
    {
        fail(EvidenceFailure::MissingCompleteLabels);
    }
    if evidence.candidate.exact_pair_families == 0 {
        fail(EvidenceFailure::MissingIndependentPairEvaluation);
    }
    for (idx, domain) in evidence.candidate.domains.iter().enumerate() {
        let accepted =
            u128::from(domain.true_positive_families) + u128::from(domain.false_positive_families);
        if domain.domain != DomainId::ALL[idx]
            || domain.accepted_families as u128 != accepted
            || accepted < MIN_ACCEPTED_FAMILIES_PER_DOMAIN as u128
        {
            fail(EvidenceFailure::InsufficientAcceptedFamilies);
        }
        if accepted == 0 || 10 * u128::from(domain.true_positive_families) < 9 * accepted {
            fail(EvidenceFailure::PrecisionBelowTarget);
        }
        if !domain.accepted {
            for reason in &domain.failures {
                fail(*reason);
            }
        }
    }
    if let Some(base) = &evidence.baseline {
        if evidence.baseline_model_id.is_none()
            || evidence.baseline_active_generation.is_none()
            || base.complete_families != evidence.candidate.complete_families
            || base.out_of_scope_families != evidence.candidate.out_of_scope_families
            || base.single_domain_families != evidence.candidate.single_domain_families
        {
            fail(EvidenceFailure::MissingBaselineComparison);
        }
        for (idx, candidate) in evidence.candidate.domains.iter().enumerate() {
            let baseline = &base.domains[idx];
            let cp = u128::from(candidate.true_positive_families);
            let bp = u128::from(baseline.true_positive_families);
            let cn = cp + u128::from(candidate.false_positive_families);
            let bn = bp + u128::from(baseline.false_positive_families);
            if bn == 0 || cn == 0 || baseline.domain != DomainId::ALL[idx] {
                fail(EvidenceFailure::MissingBaselineComparison);
            } else if cp * bn < bp * cn {
                fail(EvidenceFailure::PrecisionRegression);
            }
        }
        if !base.macro_f1.is_finite()
            || !evidence.candidate.macro_f1.is_finite()
            || evidence.candidate.macro_f1 < base.macro_f1
        {
            fail(EvidenceFailure::MacroF1Regression);
        }
        // Denominators are required identical above, so integer counts compare the rates exactly.
        if evidence.candidate.false_oos_admissions > base.false_oos_admissions
            || evidence.candidate.false_single_domain_admissions
                > base.false_single_domain_admissions
        {
            fail(EvidenceFailure::IncreasedFalseAdmission);
        }
    } else {
        fail(EvidenceFailure::MissingBaselineComparison);
    }
    if evidence.candidate.invalid_selected_pairs > 0 {
        fail(EvidenceFailure::InvalidSelectedPair);
    }
    if let Some(rt) = &evidence.runtime {
        let timings = [
            rt.candidate_warm_p95_ms,
            rt.baseline_warm_p95_ms,
            rt.foreground_without_calibration_p95_ms,
            rt.foreground_with_calibration_p95_ms,
        ];
        if timings.iter().any(|n| !n.is_finite() || *n < 0.0)
            || rt.hardware_id.trim().is_empty()
            || rt.measurement_reference.trim().is_empty()
            || rt.corpus_sha256 != evidence.reviewed_corpus_sha256
            || rt.candidate_warm_p95_ms > 10.0
            || rt.foreground_with_calibration_p95_ms > rt.foreground_without_calibration_p95_ms
            || rt.peak_heap_bytes > 128 * 1024 * 1024
            || rt.configured_stack_limit_bytes == 0
            || rt.configured_stack_limit_bytes > 1024 * 1024
            || rt.peak_reservation_bytes > 512 * 1024 * 1024
            || !rt.stack_limit_fixture_passed
        {
            fail(EvidenceFailure::RuntimeBudgetRegression);
        }
    } else {
        fail(EvidenceFailure::MissingRuntimeMeasurements);
    }
    if reasons.is_empty() {
        Ok(PromotionDecision::Eligible)
    } else {
        Ok(PromotionDecision::RetainPrior { reasons })
    }
}

/// Processes an incoming batch of provider observation envelopes.
///
/// # Errors
/// Returns [`ClassifierError::ObservationBatchTooLarge`] if batch length exceeds 512.
pub fn process_observations(
    checkpoint: &CalibrationCheckpoint,
    families: &[FamilyRecord],
    budget: &WeakTrainingBudget,
    batch: &[ObservationEnvelope],
    approvals: &[MappingApproval],
) -> Result<ObservationActivation, ClassifierError> {
    validate_checkpoint(checkpoint)?;
    if batch.len() > OBSERVATIONS_PER_ACTIVATION {
        return Err(ClassifierError::ObservationBatchTooLarge);
    }

    if checkpoint.schema_version != CALIBRATION_CHECKPOINT_SCHEMA_VERSION
        || families.len() > MAX_RETAINED_FAMILIES
        || batch.windows(2).any(|w| w[0].watermark >= w[1].watermark)
    {
        return Err(invalid_calibration(
            "invalid_observation_page_or_checkpoint",
        ));
    }
    for approval in approvals {
        super::provider_labels::validate_approval(approval)?;
    }
    let approval = approvals.first();
    let mut family_map: BTreeMap<FamilyId, FamilyRecord> = BTreeMap::new();
    for family in families {
        if family_map
            .insert(family.family_id.clone(), family.clone())
            .is_some()
        {
            return Err(invalid_calibration("duplicate_retained_family"));
        }
    }
    let mut new_budget = budget.clone();
    let mut dispositions = Vec::with_capacity(batch.len());
    let mut next_watermark = checkpoint.observation_watermark;
    let mut new_eligible_count = 0usize;
    let mut observation_ids = BTreeSet::new();
    for envelope in batch {
        let obs = &envelope.observation;
        let fam_id = &envelope.family_id;
        super::provider_labels::validate_observation(obs)?;
        next_watermark = next_watermark.max(envelope.watermark);
        let mut reason = ObservationReason::AcceptedForMonitoring;
        if envelope.watermark <= checkpoint.observation_watermark
            || !observation_ids.insert(&obs.observation_id)
        {
            dispositions.push(ObservationDisposition {
                observation_id: obs.observation_id.clone(),
                family_id: fam_id.clone(),
                reason: ObservationReason::Duplicate,
            });
            continue;
        }
        if family_map
            .get(fam_id)
            .is_some_and(|family| family.split != envelope.split)
        {
            dispositions.push(ObservationDisposition {
                observation_id: obs.observation_id.clone(),
                family_id: fam_id.clone(),
                reason: ObservationReason::ConflictingEvidence,
            });
            continue;
        }
        let mapping = super::provider_labels::map_labels(obs, approval)?;
        let mut eligible = envelope.split == Split::Train && mapping.eligible_for_training;
        use super::provider_labels::AbstainReason;
        if let Some(abstain) = mapping.abstain {
            reason = match abstain {
                AbstainReason::UnsupportedPayloadVersion | AbstainReason::MalformedCategories => {
                    ObservationReason::UnsupportedPayload
                }
                AbstainReason::UnknownVocabulary => ObservationReason::UnknownVocabulary,
                AbstainReason::NoEligibleLabels => ObservationReason::ExcludedLabels,
                AbstainReason::UnsupportedLanguage => ObservationReason::UnsupportedLanguage,
                AbstainReason::NoTextualEvidence | AbstainReason::UnreviewedMapping => {
                    ObservationReason::AcceptedForMonitoring
                }
            };
        }
        let previously_counted = new_budget.counted_family_ids.contains(fam_id);
        let provider_name = format!("{:?}", obs.provider);
        if eligible
            && previously_counted
            && family_map.get(fam_id).is_none_or(|family| {
                family.approved_membership != Some(mapping.training_membership)
                    || family.selected_provider.as_deref() != Some(provider_name.as_str())
                    || family.selected_publisher != obs.publisher
            })
        {
            dispositions.push(ObservationDisposition {
                observation_id: obs.observation_id.clone(),
                family_id: fam_id.clone(),
                reason: ObservationReason::ConflictingEvidence,
            });
            continue;
        }
        if eligible && !previously_counted {
            if obs
                .publisher
                .as_deref()
                .is_none_or(|publisher| publisher.trim().is_empty())
            {
                eligible = false;
                reason = ObservationReason::PublisherCap;
            } else if new_budget
                .by_provider
                .get(&provider_name)
                .copied()
                .unwrap_or(0)
                >= MAX_WEAK_FAMILIES_PER_PROVIDER as u64
            {
                eligible = false;
                reason = ObservationReason::ProviderCap;
            } else if obs.publisher.as_ref().is_some_and(|publisher| {
                new_budget.by_publisher.get(publisher).copied().unwrap_or(0)
                    >= MAX_WEAK_FAMILIES_PER_PUBLISHER as u64
            }) {
                eligible = false;
                reason = ObservationReason::PublisherCap;
            } else if mapping
                .training_membership
                .iter()
                .enumerate()
                .any(|(idx, membership)| {
                    *membership == LabelMembership::Present
                        && new_budget.selected_weak_positive_families[idx]
                            >= new_budget.seed_positive_families[idx]
                })
            {
                eligible = false;
                reason = ObservationReason::WeakContributionCap;
            }
        }
        if !family_map.contains_key(fam_id) && family_map.len() >= MAX_RETAINED_FAMILIES {
            let evicted = family_map
                .values()
                .filter(|f| !f.protected)
                .min_by_key(|f| (f.last_seen_watermark, &f.family_id))
                .map(|f| f.family_id.clone())
                .ok_or(ClassifierError::FamilyCapacityExceeded)?;
            family_map.remove(&evicted);
            dispositions.push(ObservationDisposition {
                observation_id: String::new(),
                family_id: evicted,
                reason: ObservationReason::EvictedFamily,
            });
        }
        let record = family_map
            .entry(fam_id.clone())
            .or_insert_with(|| FamilyRecord {
                family_id: fam_id.clone(),
                split: envelope.split,
                first_seen_watermark: envelope.watermark,
                last_seen_watermark: envelope.watermark,
                eligible_training: false,
                eligibility_watermark: None,
                protected: false,
                approved_membership: None,
                selected_provider: None,
                selected_publisher: None,
            });
        record.last_seen_watermark = envelope.watermark;
        if eligible {
            reason = ObservationReason::EligiblePartialTraining;
            record.eligible_training = true;
            record.approved_membership = Some(mapping.training_membership);
            record.selected_provider = Some(provider_name.clone());
            record.selected_publisher = obs.publisher.clone();
            if record.eligibility_watermark.is_none() {
                record.eligibility_watermark = Some(envelope.watermark);
            }
            if !previously_counted {
                new_eligible_count += 1;
                new_budget.counted_family_ids.insert(fam_id.clone());
                *new_budget.by_provider.entry(provider_name).or_insert(0) += 1;
                if let Some(publisher) = &obs.publisher {
                    *new_budget
                        .by_publisher
                        .entry(publisher.clone())
                        .or_insert(0) += 1;
                }
                for (idx, membership) in mapping.training_membership.iter().enumerate() {
                    if *membership == LabelMembership::Present {
                        new_budget.selected_weak_positive_families[idx] += 1;
                    }
                }
            }
        }
        dispositions.push(ObservationDisposition {
            observation_id: obs.observation_id.clone(),
            family_id: fam_id.clone(),
            reason,
        });
    }

    Ok(ObservationActivation {
        next_watermark,
        processed: batch.len() as u64,
        families: family_map.into_values().collect(),
        budget: new_budget,
        dispositions,
        new_eligible_training_families: new_eligible_count,
    })
}

/// Issues an `ActivationAuthorization` token if quality evidence and consent
/// requirements are satisfied.
///
/// # Errors
/// Returns [`ClassifierError::ActivationDenied`] if activation authorization fails.
pub fn authorize_activation(
    lease: &WriterLease,
    candidate: &ValidatedArtifact,
    report: &TrainingReport,
    request: &TrainRequest,
    promotion: Option<&PromotionEvidence>,
    prior: Option<&ModelManifest>,
) -> Result<ActivationAuthorizationDecision, ClassifierError> {
    if candidate.model_id != request.model_id
        || report.model_id != request.model_id
        || report.request_id != request.request_id
        || report.provenance != request.provenance
        || candidate.manifest.provenance != request.provenance
        || report.schema_version != super::training::TRAINING_REPORT_SCHEMA_VERSION
    {
        return Err(ClassifierError::ActivationDenied {
            reason: "candidate_request_report_mismatch".into(),
        });
    }
    validate_gates(&report.gate_set)?;
    let payload_sha256 = format!("{:x}", Sha256::digest(candidate.payload.as_bytes()));
    let report_bytes = super::artifact::bounded_json_bytes(report)?;
    let report_sha256 = format!("{:x}", Sha256::digest(&report_bytes));
    let manifest_bytes = super::artifact::bounded_json_bytes(&candidate.manifest)?;
    let manifest_sha256 = format!("{:x}", Sha256::digest(&manifest_bytes));
    if report_sha256 != candidate.manifest.report_sha256
        || payload_sha256 != candidate.manifest.model_sha256
        || serde_json::to_value(report).map_err(|e| invalid_calibration(e.to_string()))?
            != candidate.manifest.metrics
    {
        return Err(ClassifierError::ActivationDenied {
            reason: "candidate_report_hash_mismatch".into(),
        });
    }

    let expected_prior_id = prior.map(|p| p.model_id.as_str().to_string());
    let expected_prior_generation = prior.map(|p| p.activation_generation);

    // Synthetic candidates require explicit opt-in
    if matches!(request.provenance, Provenance::SyntheticBaseline { .. }) {
        if request.synthetic_activation == SyntheticActivation::RetainPrior {
            return Ok(ActivationAuthorizationDecision::RetainPrior(
                ActivationOutcome::CandidateOnly {
                    reason_code: "insufficient_quality_evidence".into(),
                    reason: "Synthetic baseline lacks independent reviewed-real calibration/development evidence; explicit --activate-synthetic-baseline opt-in is required.".into(),
                    failures: vec![EvidenceFailure::SyntheticOptInRequired],
                },
            ));
        }

        // Explicit opt-in granted
        let auth = ActivationAuthorization::new(
            lease,
            candidate.model_id.clone(),
            payload_sha256,
            report_sha256,
            manifest_sha256,
            expected_prior_id,
            expected_prior_generation,
            ActivationReason::ManualSeed,
        );
        return Ok(ActivationAuthorizationDecision::Authorized(auth));
    }

    // Reviewed-real candidates require promotion evidence
    if let Some(promo) = promotion {
        if promo.candidate_model_id != candidate.model_id
            || promo.candidate_payload_sha256 != payload_sha256
            || promo.candidate_report_sha256 != report_sha256
            || promo.provenance != candidate.manifest.provenance
            || promo.baseline_model_id.as_ref().map(ModelId::as_str) != expected_prior_id.as_deref()
            || promo.baseline_active_generation != expected_prior_generation
        {
            return Err(ClassifierError::ActivationDenied {
                reason: "promotion_evidence_identity_mismatch".into(),
            });
        }
        match assess_promotion(promo)? {
            PromotionDecision::Eligible => {
                let auth = ActivationAuthorization::new(
                    lease,
                    candidate.model_id.clone(),
                    payload_sha256,
                    report_sha256,
                    manifest_sha256,
                    expected_prior_id,
                    expected_prior_generation,
                    ActivationReason::AutomaticPromotion,
                );
                Ok(ActivationAuthorizationDecision::Authorized(auth))
            }
            PromotionDecision::RetainPrior { reasons } => Ok(
                ActivationAuthorizationDecision::RetainPrior(ActivationOutcome::CandidateOnly {
                    reason_code: "insufficient_quality_evidence".into(),
                    reason: "Candidate failed reviewed-real promotion gates.".into(),
                    failures: reasons,
                }),
            ),
        }
    } else {
        Ok(ActivationAuthorizationDecision::RetainPrior(
            ActivationOutcome::CandidateOnly {
                reason_code: "insufficient_quality_evidence".into(),
                reason: "Reviewed-real candidate lacks promotion evidence.".into(),
                failures: vec![EvidenceFailure::MissingReviewedRealEvidence],
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_raw_scores(raw_odds: [f64; 6]) -> [RawDomainScore; 6] {
        DomainId::ALL.map(|d| {
            let idx = DomainId::ALL.iter().position(|x| *x == d).unwrap();
            RawDomainScore {
                domain: d,
                present_log_score: raw_odds[idx],
                absent_log_score: 0.0,
                raw_log_odds: raw_odds[idx],
                known_feature_count: 5,
                oov_count: 0,
            }
        })
    }

    #[test]
    fn test_isotonic_knot_evaluation() {
        let calib = DomainCalibration {
            domain: DomainId::Economic,
            knots: vec![
                CalibrationKnot {
                    raw_upper: -1.0,
                    relevance: 0.1,
                },
                CalibrationKnot {
                    raw_upper: 0.0,
                    relevance: 0.5,
                },
                CalibrationKnot {
                    raw_upper: 1.0,
                    relevance: 0.9,
                },
            ],
            threshold: 0.8,
            min_informative_features: 1,
        };

        assert!((score(-2.0, &calib).unwrap() - 0.1).abs() < 1e-9);
        assert!((score(-0.5, &calib).unwrap() - 0.5).abs() < 1e-9);
        assert!((score(0.5, &calib).unwrap() - 0.9).abs() < 1e-9);
        assert!((score(2.0, &calib).unwrap() - 0.9).abs() < 1e-9);
    }

    #[test]
    fn test_lexical_tie_breaking_order() {
        // When all 6 domains have identical calibrated score of 0.9,
        // selection must sort by DomainId.as_str() ascending:
        // economic ("economic") then environmental ("environmental")
        let knots = vec![CalibrationKnot {
            raw_upper: 10.0,
            relevance: 0.9,
        }];
        let gates = GateSet {
            version: CALIBRATION_VERSION.into(),
            domains: DomainId::ALL.map(|d| DomainCalibration {
                domain: d,
                knots: knots.clone(),
                threshold: 0.8,
                min_informative_features: 1,
            }),
        };

        let raw = dummy_raw_scores([1.0; 6]);
        let input = ClassifyInput::new(
            "occ-1",
            "A news headline",
            "Story description",
            Some("en".to_string()),
        );
        let model_id = ModelId::parse("model-tie").unwrap();

        let receipt = apply_gates(&input, &raw, &model_id, &gates).unwrap();
        assert!(receipt.rejection_reason.is_none());
        let pair = receipt.selected_pair.expect("pair must be selected");
        assert_eq!(pair[0], DomainId::Economic);
        assert_eq!(pair[1], DomainId::Environmental);
    }

    #[test]
    fn test_subthreshold_rejection_insufficient_domains() {
        let knots = vec![CalibrationKnot {
            raw_upper: 10.0,
            relevance: 0.5,
        }];
        let gates = GateSet {
            version: CALIBRATION_VERSION.into(),
            domains: DomainId::ALL.map(|d| DomainCalibration {
                domain: d,
                knots: knots.clone(),
                threshold: 0.8,
                min_informative_features: 1,
            }),
        };

        let raw = dummy_raw_scores([1.0; 6]);
        let input = ClassifyInput::new("occ-2", "Headline", "Desc", Some("en".to_string()));
        let model_id = ModelId::parse("model-sub").unwrap();

        let receipt = apply_gates(&input, &raw, &model_id, &gates).unwrap();
        assert_eq!(receipt.selected_pair, None);
        assert_eq!(
            receipt.rejection_reason,
            Some(RejectionReason::InsufficientDomains)
        );
    }
}
