//! Comparative promotion measurements on one immutable reviewed cohort. These
//! are classifier request timings, not whole-UI latency or physical RSS.
use super::artifact::{ArtifactStore, ModelManifest};
use super::calibration::{
    self, EvaluationMetrics, GateSet, PromotionEvidence, RuntimeComparison, ScoredFamily,
};
use super::learning::LearningInput;
use super::readiness::PinnedClassifier;
use super::training::{DatasetRow, Split, TrainingReport};
use super::{
    ClassifierError, ClassifierPerformance, ClassifierService, ClassifyInput, Provenance,
    WorkControl,
};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
const SAMPLES: usize = 21;
fn invalid(code: &str) -> ClassifierError {
    ClassifierError::InvalidArtifact(code.into())
}

pub(crate) struct Prepared {
    pub baseline: PinnedClassifier,
    pub prior: ModelManifest,
    pub rows: Vec<DatasetRow>,
    pub baseline_metrics: EvaluationMetrics,
    pub baseline_p95_ms: f64,
    pub review_hash: String,
    pub reference: String,
}
fn inputs(rows: &[DatasetRow]) -> Vec<ClassifyInput> {
    rows.iter()
        .map(|row| {
            ClassifyInput::new(
                &row.sample_id,
                &row.title,
                &row.description,
                Some(row.language.clone()),
            )
        })
        .collect()
}
fn evaluate(
    service: &ClassifierService,
    rows: &[DatasetRow],
    gates: &GateSet,
    control: &WorkControl,
) -> Result<EvaluationMetrics, ClassifierError> {
    let mut scored = Vec::new();
    for batch in rows.chunks(64) {
        let receipts = service.classify_batch_with_control(inputs(batch), control)?;
        if receipts.len() != batch.len() {
            return Err(invalid("promotion_receipt_count"));
        }
        for (row, receipt) in batch.iter().zip(receipts) {
            let raw = receipt
                .raw_diagnostics
                .ok_or_else(|| invalid("promotion_raw_diagnostics_missing"))?;
            scored.push(ScoredFamily {
                family_id: row.family_id.clone(),
                representative_id: row.sample_id.clone(),
                split: row.split,
                provenance: row.provenance.clone(),
                membership: row.membership,
                raw: raw.domains,
            });
        }
    }
    calibration::evaluate(&scored, gates)
}
fn p95(mut samples: Vec<f64>) -> Result<f64, ClassifierError> {
    if samples.len() != SAMPLES
        || samples
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err(invalid("promotion_measurement_count"));
    }
    samples.sort_by(f64::total_cmp);
    Ok(samples[(samples.len() * 95).div_ceil(100) - 1])
}
fn warm(
    service: &ClassifierService,
    rows: &[DatasetRow],
    control: &WorkControl,
) -> Result<f64, ClassifierError> {
    if rows.len() < SAMPLES {
        return Err(invalid("promotion_measurement_cohort_too_small"));
    }
    let mut samples = Vec::new();
    for row in rows.iter().take(SAMPLES) {
        control.check_active()?;
        let started = Instant::now();
        service.classify_batch_with_control(inputs(std::slice::from_ref(row)), control)?;
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    p95(samples)
}
/// Prewarms and measures the actual baseline before fitting; all selected rows
/// are complete reviewed Development, disjoint from threshold fitting.
pub(crate) fn prepare(
    input: &LearningInput,
    baseline: PinnedClassifier,
    store: &ArtifactStore,
    control: &WorkControl,
) -> Result<Option<Prepared>, ClassifierError> {
    let Provenance::MixedTraining {
        reviewed_corpus_sha256,
        review_reference,
        ..
    } = &input.provenance
    else {
        return Ok(None);
    };
    if !input.reviewed_thresholds
        || input
            .corpus
            .rows
            .iter()
            .filter(|row| row.split == Split::Calibration)
            .any(|row| !matches!(row.provenance, Provenance::ReviewedReal { .. }))
    {
        return Ok(None);
    }
    let rows: Vec<_> = input
        .corpus
        .rows
        .iter()
        .filter(|row| row.split == Split::Development)
        .cloned()
        .collect();
    if rows.len()<SAMPLES || rows.iter().any(|row|!matches!(&row.provenance,Provenance::ReviewedReal {dataset_sha256,review_reference:reference,..} if dataset_sha256==reviewed_corpus_sha256 && reference==review_reference)) || !rows.iter().any(|row|row.membership.iter().filter(|label|**label==super::training::LabelMembership::Present).count()==2) {return Ok(None);}
    let prior = store
        .read_active_integrity()?
        .ok_or(ClassifierError::Unavailable)?;
    if baseline.snapshot.model_id != prior.model_id.as_str()
        || baseline.snapshot.artifact_sha256 != prior.model_sha256
    {
        return Err(invalid("promotion_baseline_changed"));
    }
    let report: TrainingReport = serde_json::from_str(&store.read_report(&prior.model_id)?)
        .map_err(|_| invalid("promotion_baseline_report"))?;
    let baseline_metrics = evaluate(&baseline.service, &rows, &report.gate_set, control)?;
    let baseline_p95_ms = warm(&baseline.service, &rows, control)?;
    Ok(Some(Prepared {
        baseline,
        prior,
        rows,
        baseline_metrics,
        baseline_p95_ms,
        review_hash: reviewed_corpus_sha256.clone(),
        reference: review_reference.clone(),
    }))
}
#[derive(Debug)]
pub(crate) struct Concurrent {
    pub samples: Vec<f64>,
    pub observed_reservation: u64,
}
/// An owned blocking task waits for actual fitting to start. Only requests whose
/// whole interval overlaps active fitting count; insufficient overlap abstains.
pub(crate) fn concurrent(
    prepared: &Prepared,
    ready: std::sync::mpsc::Receiver<()>,
    running: Arc<AtomicBool>,
    control: WorkControl,
    peak: Arc<AtomicU64>,
) -> tokio::task::JoinHandle<Result<Concurrent, ClassifierError>> {
    let service = prepared.baseline.service.clone();
    let rows = prepared.rows[..SAMPLES].to_vec();
    tokio::task::spawn_blocking(move || {
        if ready
            .recv_timeout(std::time::Duration::from_secs(10))
            .is_err()
        {
            return Ok(Concurrent {
                samples: Vec::new(),
                observed_reservation: 0,
            });
        }
        let mut samples = Vec::new();
        let mut observed = 0;
        for row in rows {
            control.check_active()?;
            if !running.load(Ordering::Acquire) {
                break;
            }
            observed = observed
                .max(crate::js_runtime_budget::classifier_process_budget().reserved_bytes() as u64);
            peak.fetch_max(observed, Ordering::AcqRel);
            let started = Instant::now();
            service.classify_batch_with_control(inputs(std::slice::from_ref(&row)), &control)?;
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            if running.load(Ordering::Acquire) {
                samples.push(elapsed);
            } else {
                break;
            }
        }
        Ok(Concurrent {
            samples,
            observed_reservation: observed,
        })
    })
}
/// Produces evidence only from real requests/observations. Missing overlap,
/// unavailable counters or failed runtime-limit proof cannot become a pass.
pub(crate) fn evidence(
    input: &LearningInput,
    prepared: &Prepared,
    candidate: &super::training::TrainedCandidate,
    manifest: &ModelManifest,
    artifact: super::artifact::ValidatedArtifact,
    concurrent: Concurrent,
    training: ClassifierPerformance,
    proof: &super::runtime_proof::RuntimeLimitProof,
    observed_reservation: u64,
    control: &WorkControl,
) -> Result<PromotionEvidence, ClassifierError> {
    let snapshot = super::ModelSnapshot::new(
        candidate.model_id.as_str(),
        &manifest.model_sha256,
        input.provenance.clone(),
    );
    let service = ClassifierService::from_snapshot(
        snapshot,
        artifact,
        Box::new(candidate.report.gate_set.clone()),
    )?;
    let candidate_metrics = evaluate(
        &service,
        &prepared.rows,
        &candidate.report.gate_set,
        control,
    )?;
    if candidate_metrics != candidate.report.evaluation {
        return Err(invalid("promotion_candidate_report_differs"));
    }
    let candidate_p95_ms = warm(&service, &prepared.rows, control)?;
    let baseline_stats = prepared.baseline.service.performance()?;
    let stats = service.performance()?;
    let sample_hash = super::artifact::sha256_hex(&super::artifact::bounded_json_bytes(&inputs(
        &prepared.rows[..SAMPLES],
    ))?);
    let runtime = if concurrent.samples.len() == SAMPLES
        && stats.stack_limit_bytes == proof.stack_limit_bytes()
        && baseline_stats.stack_limit_bytes == proof.stack_limit_bytes()
    {
        Some(RuntimeComparison {hardware_id:format!("{}-{}-{}cpus",std::env::consts::OS,std::env::consts::ARCH,std::thread::available_parallelism().map_or(1,usize::from)),corpus_sha256:prepared.review_hash.clone(),measurement_reference:format!("classifier-single-request;warm-and-overlap-n={SAMPLES};sample={sample_hash};heap=observed-QuickJS-snapshot;reservation=observed-process-ledger;actual-heap-stack-limit-proof"),candidate_warm_p95_ms:candidate_p95_ms,baseline_warm_p95_ms:prepared.baseline_p95_ms,foreground_without_calibration_p95_ms:prepared.baseline_p95_ms,foreground_with_calibration_p95_ms:p95(concurrent.samples)?,peak_heap_bytes:training.js_heap_peak_bytes.max(stats.js_heap_peak_bytes).max(baseline_stats.js_heap_peak_bytes),configured_stack_limit_bytes:proof.stack_limit_bytes(),stack_limit_fixture_passed:true,peak_reservation_bytes:observed_reservation.max(concurrent.observed_reservation).max(proof.observed_reservation_bytes())})
    } else {
        None
    };
    Ok(PromotionEvidence {
        candidate_model_id: candidate.model_id.clone(),
        candidate_payload_sha256: manifest.model_sha256.clone(),
        candidate_report_sha256: manifest.report_sha256.clone(),
        provenance: input.provenance.clone(),
        independent_review_reference: prepared.reference.clone(),
        reviewed_corpus_sha256: prepared.review_hash.clone(),
        family_assignment_sha256: manifest.family_assignment_sha256.clone(),
        development_split_sha256: manifest.development_split_sha256.clone(),
        baseline_model_id: Some(prepared.prior.model_id.clone()),
        baseline_active_generation: Some(prepared.prior.activation_generation),
        candidate: candidate_metrics,
        baseline: Some(prepared.baseline_metrics.clone()),
        evaluation_family_ids: prepared
            .rows
            .iter()
            .map(|row| row.family_id.clone())
            .collect(),
        evaluation_family_provenance: prepared
            .rows
            .iter()
            .map(|row| (row.family_id.clone(), row.provenance.clone()))
            .collect(),
        training_family_ids: input
            .corpus
            .rows
            .iter()
            .filter(|row| row.split == Split::Train)
            .map(|row| row.family_id.clone())
            .collect(),
        threshold_fit_family_ids: input
            .corpus
            .rows
            .iter()
            .filter(|row| row.split == Split::Calibration)
            .map(|row| row.family_id.clone())
            .collect(),
        threshold_fit_family_provenance: input
            .corpus
            .rows
            .iter()
            .filter(|row| row.split == Split::Calibration)
            .map(|row| (row.family_id.clone(), row.provenance.clone()))
            .collect(),
        calibration_split_sha256: manifest.calibration_split_sha256.clone(),
        split: Split::Development,
        runtime,
    })
}
