//! Durable, bounded historical admission; never part of an Atlas cycle.
//! @tag:classifier @tag:atlas-collection
use std::fmt;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::atlas_classify::{AdmissionContext, OccurrenceKey};
use crate::classifier::readiness::ClassifierHome;
use crate::classifier::ModelSnapshot;
use crate::storage::classifier_admission::{AdmissionError, TaxonomyCoverage};
use crate::storage::repo::Repository;
use crate::storage::tx::TxError;

pub use crate::storage::classifier_admission::RetainedPublicationScope as MaintenanceScope;

pub const OPERATION: &str = "atlas_admission_reclassify";
pub const POOL: &str = "atlas_admission_maintenance";
pub const RECLASSIFY_ROWS_PER_ACTIVATION: usize = 64;
pub const PRIORITY: i64 = 20;
pub const LEASE_SECS: i64 = 30;
pub const HEARTBEAT_SECS: u64 = 5;
pub const CHECKPOINT_VERSION: &str = "atlas-reclassification-checkpoint-v1";
pub const REPORT_VERSION: &str = "atlas-reclassification-report-v1";
pub const MAX_SOURCE_CONFLICTS: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MaintenanceState {
    Queued,
    Running,
    Paused,
    Blocked,
    Reconciling,
    Completed,
    Cancelled,
    Failed,
}
impl MaintenanceState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Blocked => "blocked",
            Self::Reconciling => "reconciling",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }
    pub fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ReclassificationRequest {
    pub request_id: String,
    pub scope: MaintenanceScope,
    pub requested_model: Option<ModelSnapshot>,
}
impl ReclassificationRequest {
    #[must_use]
    pub fn new(request_id: impl Into<String>, scope: MaintenanceScope) -> Self {
        Self {
            request_id: request_id.into(),
            scope,
            requested_model: None,
        }
    }
    #[must_use]
    pub fn with_model(mut self, model: ModelSnapshot) -> Self {
        self.requested_model = Some(model);
        self
    }
    fn validate(&self) -> Result<(), MaintenanceError> {
        if self.request_id.is_empty()
            || self.request_id.len() > 256
            || self.request_id.chars().any(char::is_control)
            || matches!(&self.scope,MaintenanceScope::Run {run_id} if !crate::intel_service::valid_id(run_id))
        {
            return Err(MaintenanceError::InvalidRequest {
                code: "invalid_request_identity".into(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MaintenanceCheckpoint {
    pub schema_version: String,
    pub job_id: String,
    pub scope: MaintenanceScope,
    pub pinned_snapshot: ModelSnapshot,
    pub manifest_sha256: String,
    pub cutoff_at: String,
    pub scan_after: Option<OccurrenceKey>,
    pub scan_upper: Option<OccurrenceKey>,
    pub activation_seq: u64,
    pub derivative_after: Option<String>,
    pub rows_scanned: u64,
    pub rows_reclassified: u64,
    pub rows_rejected: u64,
    pub rows_skipped_unmigrated: u64,
    pub rows_conflicted: u64,
    pub conflict_attempts: u64,
    pub conflict_generation: u64,
    pub rows_already_current: u64,
    pub derivatives_pending: u64,
    pub scan_exhausted: bool,
    pub revision: u64,
}
impl MaintenanceCheckpoint {
    pub(crate) fn new(
        job_id: String,
        scope: MaintenanceScope,
        pinned_snapshot: ModelSnapshot,
        manifest_sha256: String,
        cutoff_at: String,
        scan_upper: Option<OccurrenceKey>,
    ) -> Self {
        Self {
            schema_version: CHECKPOINT_VERSION.into(),
            job_id,
            scope,
            pinned_snapshot,
            manifest_sha256,
            cutoff_at,
            scan_after: None,
            scan_upper,
            activation_seq: 0,
            derivative_after: None,
            rows_scanned: 0,
            rows_reclassified: 0,
            rows_rejected: 0,
            rows_skipped_unmigrated: 0,
            rows_conflicted: 0,
            conflict_attempts: 0,
            conflict_generation: 0,
            rows_already_current: 0,
            derivatives_pending: 0,
            scan_exhausted: false,
            revision: 0,
        }
    }
    pub(crate) fn validate(&self) -> Result<(), MaintenanceError> {
        if self.schema_version != CHECKPOINT_VERSION
            || self.job_id.is_empty()
            || self.manifest_sha256.len() != 64
            || !self.manifest_sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || chrono::DateTime::parse_from_rfc3339(&self.cutoff_at).is_err()
            || self.rows_rejected > self.rows_reclassified
            || self
                .scan_after
                .as_ref()
                .zip(self.scan_upper.as_ref())
                .is_some_and(|(after, upper)| after > upper)
            || [
                self.activation_seq,
                self.rows_scanned,
                self.rows_reclassified,
                self.rows_rejected,
                self.rows_skipped_unmigrated,
                self.rows_conflicted,
                self.conflict_attempts,
                self.conflict_generation,
                self.rows_already_current,
                self.derivatives_pending,
                self.revision,
            ]
            .iter()
            .any(|value| *value > i64::MAX as u64)
        {
            return Err(MaintenanceError::InvalidRequest {
                code: "invalid_checkpoint".into(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct MaintenanceReport {
    pub schema_version: String,
    pub job_id: String,
    pub state: MaintenanceState,
    pub model_id: String,
    pub taxonomy_version: String,
    pub rows_scanned: u64,
    /// All evaluated durable outcomes; rejected is a subset.
    pub rows_reclassified: u64,
    pub rows_rejected: u64,
    pub rows_skipped_unmigrated: u64,
    pub rows_conflicted: u64,
    pub conflict_attempts: u64,
    pub rows_already_current: u64,
    pub derivatives_pending: u64,
    pub coverage: TaxonomyCoverage,
    pub checkpoint_revision: u64,
    pub reason_codes: Vec<String>,
}
impl MaintenanceReport {
    pub(crate) fn from_checkpoint(
        checkpoint: &MaintenanceCheckpoint,
        state: MaintenanceState,
        coverage: TaxonomyCoverage,
        reason_codes: Vec<String>,
    ) -> Self {
        Self {
            schema_version: REPORT_VERSION.into(),
            job_id: checkpoint.job_id.clone(),
            state,
            model_id: checkpoint.pinned_snapshot.model_id.clone(),
            taxonomy_version: crate::atlas_taxonomy::TAXONOMY_VERSION.into(),
            rows_scanned: checkpoint.rows_scanned,
            rows_reclassified: checkpoint.rows_reclassified,
            rows_rejected: checkpoint.rows_rejected,
            rows_skipped_unmigrated: checkpoint.rows_skipped_unmigrated,
            rows_conflicted: checkpoint.rows_conflicted,
            conflict_attempts: checkpoint.conflict_attempts,
            rows_already_current: checkpoint.rows_already_current,
            derivatives_pending: checkpoint.derivatives_pending,
            coverage,
            checkpoint_revision: checkpoint.revision,
            reason_codes,
        }
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum MaintenanceError {
    Admission(AdmissionError),
    InvalidRequest { code: String },
    NotFound,
    RequestConflict,
    MissingPinnedRevision,
    Storage(TxError),
}
impl fmt::Display for MaintenanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Admission(_) => f.write_str("historical admission operation failed"),
            Self::InvalidRequest { code } => {
                write!(f, "invalid historical admission request: {code}")
            }
            Self::NotFound => f.write_str("historical admission job not found"),
            Self::RequestConflict => {
                f.write_str("request ID already belongs to another scope or model revision")
            }
            Self::MissingPinnedRevision => f.write_str(
                "saved classifier revision is unavailable; request an explicit new revision",
            ),
            Self::Storage(_) => f.write_str("historical admission storage operation failed"),
        }
    }
}
impl std::error::Error for MaintenanceError {}
impl From<AdmissionError> for MaintenanceError {
    fn from(value: AdmissionError) -> Self {
        Self::Admission(value)
    }
}
impl From<TxError> for MaintenanceError {
    fn from(value: TxError) -> Self {
        Self::Storage(value)
    }
}

pub(crate) fn stable_job_id(
    request_id: &str,
    scope: &MaintenanceScope,
    pin: &ModelSnapshot,
    manifest: &str,
) -> String {
    format!(
        "atlas-reclassification-{}",
        crate::classifier::artifact::sha256_hex(
            crate::storage::codec::encode(&[
                "atlas-reclassification-job-v1",
                request_id,
                &scope.key(),
                &pin.model_id,
                manifest
            ])
            .as_bytes()
        )
    )
}

/// Accept a durable request only after exact compatible pin validation. A
/// duplicate ID returns its original accepted request even after active changes.
pub async fn start(
    repo: &Repository,
    home: &ClassifierHome,
    request: ReclassificationRequest,
) -> Result<String, MaintenanceError> {
    request.validate()?;
    if let Some(existing) =
        crate::storage::atlas_admission_maintenance::by_request(repo, &request.request_id).await?
    {
        if existing.checkpoint.scope != request.scope
            || request
                .requested_model
                .as_ref()
                .is_some_and(|model| model != &existing.checkpoint.pinned_snapshot)
        {
            return Err(MaintenanceError::RequestConflict);
        }
        return Ok(existing.checkpoint.job_id);
    }
    let pinned = home
        .pin(request.requested_model.as_ref())
        .await
        .map_err(AdmissionError::Classifier)?;
    let path = home.home().to_path_buf();
    let context = tokio::task::spawn_blocking(move || AdmissionContext::load(&path, pinned))
        .await
        .map_err(|_| MaintenanceError::InvalidRequest {
            code: "pin_worker_failed".into(),
        })??;
    crate::storage::atlas_admission_maintenance::create(
        repo,
        &request,
        &context,
        chrono::Utc::now(),
    )
    .await
}

pub async fn pause(repo: &Repository, job_id: &str) -> Result<(), MaintenanceError> {
    crate::storage::atlas_admission_maintenance::control(
        repo,
        job_id,
        crate::storage::atlas_admission_maintenance::Control::Pause,
    )
    .await
}
pub async fn cancel(repo: &Repository, job_id: &str) -> Result<(), MaintenanceError> {
    crate::storage::atlas_admission_maintenance::control(
        repo,
        job_id,
        crate::storage::atlas_admission_maintenance::Control::Cancel,
    )
    .await
}
pub async fn result(
    repo: &Repository,
    job_id: &str,
) -> Result<MaintenanceReport, MaintenanceError> {
    Ok(
        crate::storage::atlas_admission_maintenance::load(repo, job_id)
            .await?
            .report,
    )
}
pub async fn resume(repo: &Repository, job_id: &str) -> Result<(), MaintenanceError> {
    crate::storage::atlas_admission_maintenance::control(
        repo,
        job_id,
        crate::storage::atlas_admission_maintenance::Control::Resume,
    )
    .await
}

struct DispatchGuard {
    cancel: Arc<AtomicBool>,
}
impl Drop for DispatchGuard {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Release);
    }
}

/// Awaits the SAME in-flight operation after signalling stop. No select branch
/// abandons an inference/derivative future or starts a replacement operation.
async fn guarded<T>(
    owner: &crate::storage::classifier_admission::AdmissionOwner,
    activation: &crate::storage::atlas_admission_maintenance::Activation,
    caller_cancel: &AtomicBool,
    work_cancel: Arc<AtomicBool>,
    future: impl std::future::Future<Output = T>,
) -> Result<(T, Option<&'static str>), MaintenanceError> {
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    let _guard = DispatchGuard {
        cancel: work_cancel.clone(),
    };
    tokio::pin!(future);
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut renewed = std::time::Instant::now();
    let mut stop = None;
    loop {
        tokio::select! {
            biased;
            result = &mut future => return Ok((result, stop)),
            _ = interval.tick() => {
                if stop.is_some() {
                    if renewed.elapsed() >= Duration::from_secs(HEARTBEAT_SECS)
                        && crate::storage::atlas_admission_maintenance::heartbeat(
                            owner, activation, true, chrono::Utc::now(),
                        ).await.is_ok()
                    {
                        renewed = std::time::Instant::now();
                    }
                    continue;
                }
                let now = chrono::Utc::now();
                let renew = renewed.elapsed() >= Duration::from_secs(HEARTBEAT_SECS);
                let status = crate::storage::atlas_admission_maintenance::heartbeat(
                    owner, activation, renew, now,
                ).await;
                match status {
                    Ok(false) => stop = Some("control_requested"),
                    Err(_) => stop = Some("lease_check_failed"),
                    Ok(true) => {
                        if renew {
                            renewed = std::time::Instant::now();
                        }
                        if caller_cancel.load(Ordering::Acquire) {
                            stop = Some("host_shutdown");
                        } else {
                            match crate::storage::classifier_admission::foreground_requested(
                                owner.repository(), now,
                            ).await {
                                Ok(true) => stop = Some("foreground_requested"),
                                Ok(false) => {},
                                Err(_) => stop = Some("foreground_check_failed"),
                            }
                        }
                    }
                }
                if stop.is_some() {
                    work_cancel.store(true, Ordering::Release);
                }
            },
        }
    }
}

async fn exact_context(
    home: &ClassifierHome,
    checkpoint: &MaintenanceCheckpoint,
) -> Result<AdmissionContext, MaintenanceError> {
    let pin = home
        .pin(Some(&checkpoint.pinned_snapshot))
        .await
        .map_err(AdmissionError::Classifier)?;
    let root = home.home().to_path_buf();
    let context = tokio::task::spawn_blocking(move || AdmissionContext::load(&root, pin))
        .await
        .map_err(|_| MaintenanceError::InvalidRequest {
            code: "pin_worker_failed".into(),
        })??;
    if context.pin.snapshot != checkpoint.pinned_snapshot
        || context.manifest_sha256 != checkpoint.manifest_sha256
    {
        return Err(MaintenanceError::MissingPinnedRevision);
    }
    Ok(context)
}
fn pin_is_busy(error: &MaintenanceError) -> bool {
    matches!(
        error,
        MaintenanceError::Admission(AdmissionError::Classifier(
            crate::classifier::ClassifierError::Busy
                | crate::classifier::ClassifierError::RuntimeBudgetExceeded { .. }
        ))
    )
}

/// Drains at most one owned activation. A shared host home preserves exact pin
/// reuse; neither startup nor restart resolves a replacement active model.
///
/// The pool retains this drain's handle until it returns. Cancellation only
/// signals the local operation; every owned transaction is joined before exit.
pub async fn drain_one(
    repo: &Repository,
    home: &ClassifierHome,
    worker: &str,
    cancel: Arc<AtomicBool>,
) -> Result<Option<MaintenanceReport>, MaintenanceError> {
    if worker.is_empty() || worker.len() > 256 {
        return Err(MaintenanceError::InvalidRequest {
            code: "worker_identity".into(),
        });
    }
    if cancel.load(std::sync::atomic::Ordering::Acquire)
        || crate::storage::classifier_admission::foreground_requested(repo, chrono::Utc::now())
            .await?
    {
        return Ok(None);
    }
    if let Some(report) =
        crate::storage::atlas_admission_maintenance::settle_pending_cancel(repo, chrono::Utc::now())
            .await?
    {
        return Ok(Some(report));
    }
    // Blocked requests never become runnable just because model files arrive.
    // This path consumes only a separately persisted explicit resume intent.
    if let Some(stored) =
        crate::storage::atlas_admission_maintenance::requested_resume(repo).await?
    {
        match exact_context(home, &stored.checkpoint).await {
            Ok(context) => {
                crate::storage::atlas_admission_maintenance::settle_resume(
                    repo,
                    &stored,
                    Some(&context),
                    "pin_verified",
                    chrono::Utc::now(),
                )
                .await?
            }
            Err(error) if pin_is_busy(&error) => return Ok(None),
            Err(_) => {
                crate::storage::atlas_admission_maintenance::settle_resume(
                    repo,
                    &stored,
                    None,
                    "pinned_model_missing",
                    chrono::Utc::now(),
                )
                .await?
            }
        }
        return Ok(Some(result(repo, &stored.checkpoint.job_id).await?));
    }
    let owner = crate::storage::classifier_admission::AdmissionOwner::new(repo.clone());
    let result = owner
        .scope(Box::pin(drain_owned(&owner, home, worker, &cancel)))
        .await;
    let closed = owner.shutdown().await;
    match (result, closed) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (_, Err(error)) => Err(error.into()),
    }
}

async fn drain_owned(
    owner: &crate::storage::classifier_admission::AdmissionOwner,
    home: &ClassifierHome,
    worker: &str,
    cancel: &AtomicBool,
) -> Result<Option<MaintenanceReport>, MaintenanceError> {
    let started = std::time::Instant::now();
    let Some(activation) =
        crate::storage::atlas_admission_maintenance::claim(owner, worker, chrono::Utc::now())
            .await?
    else {
        return Ok(None);
    };
    let task = activation.task.clone();
    let before = activation.checkpoint.clone();
    let mut observed = ActivationObservation::new();
    let recovery = activation.clone();
    let mut result = Box::pin(process_activation(
        owner,
        home,
        activation,
        cancel,
        &mut observed,
    ))
    .await;
    // Controls may win during an owned preparation transaction, before the
    // dispatch loop exists. Settle them under retained cleanup authority too.
    if result.is_err() {
        if let Ok(stored) =
            crate::storage::atlas_admission_maintenance::load(owner.repository(), &task.job_id)
                .await
        {
            if stored.request_hash == recovery.request_hash
                && (stored.cancel_requested || stored.pause_requested)
            {
                if let Ok(report) = crate::storage::atlas_admission_maintenance::settle_activation(
                    owner,
                    &recovery,
                    &stored.checkpoint,
                    &[],
                    crate::storage::atlas_admission_maintenance::ActivationEnd::Continue {
                        reason: "control_checkpoint".into(),
                        delay_secs: 0,
                    },
                    chrono::Utc::now(),
                )
                .await
                {
                    result = Ok(Some(report));
                }
            }
        }
    }
    record_activation_metrics(owner, &task, &before, &result, &observed, started.elapsed()).await;
    result
}

async fn process_activation(
    owner: &crate::storage::classifier_admission::AdmissionOwner,
    home: &ClassifierHome,
    mut activation: crate::storage::atlas_admission_maintenance::Activation,
    cancel: &AtomicBool,
    observed: &mut ActivationObservation,
) -> Result<Option<MaintenanceReport>, MaintenanceError> {
    use crate::storage::atlas_admission_maintenance as repository;
    use crate::storage::classifier_admission as admission;
    use repository::ActivationEnd;
    match home
        .validate_saved_pin_integrity(
            &activation.checkpoint.pinned_snapshot,
            &activation.checkpoint.manifest_sha256,
        )
        .await
    {
        Ok(()) => {}
        Err(error) => {
            let end = if matches!(
                error,
                crate::classifier::ClassifierError::Busy
                    | crate::classifier::ClassifierError::RuntimeBudgetExceeded { .. }
            ) {
                ActivationEnd::Continue {
                    reason: "classifier_busy".into(),
                    delay_secs: 1,
                }
            } else {
                ActivationEnd::Blocked {
                    reason: "pinned_model_missing".into(),
                }
            };
            let report = Box::pin(repository::settle_activation(
                owner,
                &activation,
                &activation.checkpoint,
                &[],
                end,
                chrono::Utc::now(),
            ))
            .await?;
            return Ok(Some(report));
        }
    };
    // An uncertain prior publication may have committed before its reply was
    // received. Reconcile its bounded accounting queue without JS or rescanning.
    let dirty = repository::dirty_items(owner, &activation, 64, chrono::Utc::now()).await?;
    if !dirty.is_empty() {
        observed.stage = "accounting";
        observed.rows_read = dirty.len();
        let report = Box::pin(repository::settle_activation(
            owner,
            &activation,
            &activation.checkpoint,
            &dirty,
            ActivationEnd::Continue {
                reason: "accounting_checkpoint".into(),
                delay_secs: 0,
            },
            chrono::Utc::now(),
        ))
        .await?;
        return Ok(Some(report));
    }
    let pending =
        repository::pending_derivatives(owner, &activation, 64, chrono::Utc::now()).await?;
    if !pending.is_empty() {
        observed.stage = "derivatives";
        observed.rows_read = pending.len();
        // Keep each source-audit transaction short enough that this same
        // worker's lease heartbeat cannot repeatedly conflict with its writes.
        // The activation still owns at most 64 occurrences, processed in groups
        // of eight; item checkpoints persist before the next group dispatches.
        let mut delay = 0;
        let mut stopped = None;
        for group in pending.chunks(8) {
            // Each guarded group has its own polling timer. Renew at the group
            // boundary too: many short groups can exceed one activation lease.
            if !repository::heartbeat(owner, &activation, true, chrono::Utc::now()).await? {
                stopped = Some("control_requested");
                break;
            }
            let work: Vec<_> = group
                .iter()
                .map(|item| {
                    admission::DerivativeWork::new(
                        item.occurrence.clone(),
                        item.receipt_key.clone(),
                    )
                })
                .collect();
            let work_cancel = Arc::new(AtomicBool::new(false));
            let (report, stop) = guarded(
                owner,
                &activation,
                cancel,
                work_cancel.clone(),
                Box::pin(admission::process_maintenance_derivatives(
                    owner,
                    &activation.fence,
                    &work,
                    &work_cancel,
                )),
            )
            .await?;
            let report = report?;
            if stop.is_some() || report.states.iter().any(|state| !state.complete) {
                delay = 1;
            }
            if let Some(reason) = stop {
                // Stage results are already durable. A paused/cancelled fence
                // cannot audit them; resumption reconciles this same group.
                stopped = Some(reason);
                break;
            }
            Box::pin(repository::record_derivative_states(
                owner,
                &activation,
                group,
                chrono::Utc::now(),
            ))
            .await?;
        }
        let items: Vec<_> = pending.into_iter().map(|item| item.item_id).collect();
        let report = Box::pin(repository::settle_activation(
            owner,
            &activation,
            &activation.checkpoint,
            &items,
            ActivationEnd::Continue {
                reason: stopped.unwrap_or("derivative_checkpoint").into(),
                delay_secs: delay,
            },
            chrono::Utc::now(),
        ))
        .await?;
        return Ok(Some(report));
    }
    let prepared = Box::pin(repository::prepare_page(
        owner,
        &activation,
        64,
        chrono::Utc::now(),
    ))
    .await?;
    observed.rows_read = prepared.examined;
    observed.stage = "already_current_or_skipped";
    activation.checkpoint = prepared.checkpoint;
    let mut account = prepared.accounting_items;
    // Already-current and deliberately skipped rows need no retry backoff.
    // The activation boundary itself yields; only actual retryable work waits.
    let mut retry_delay = 0;
    if !prepared.inputs.is_empty() {
        observed.stage = "classification";
        let context = match exact_context(home, &activation.checkpoint).await {
            Ok(value) => value,
            Err(error) => {
                let end = if pin_is_busy(&error) {
                    ActivationEnd::Continue {
                        reason: "classifier_busy".into(),
                        delay_secs: 1,
                    }
                } else {
                    ActivationEnd::Blocked {
                        reason: "pinned_model_missing".into(),
                    }
                };
                let report = Box::pin(repository::settle_activation(
                    owner,
                    &activation,
                    &activation.checkpoint,
                    &account,
                    end,
                    chrono::Utc::now(),
                ))
                .await?;
                return Ok(Some(report));
            }
        };
        let worker_before = crate::atlas::classifier_worker_observation(&context)
            .await
            .ok();
        let control = crate::classifier::WorkControl::new();
        let mut cache = crate::atlas_classify::ComputationCache::default();
        let (scored, stop) = guarded(
            owner,
            &activation,
            cancel,
            control.cancel.clone(),
            crate::atlas_classify::classify_page(&context, prepared.inputs, &mut cache, &control),
        )
        .await?;
        observed.page = scored.as_ref().ok().map(|page| page.metrics.clone());
        let worker_after = crate::atlas::classifier_worker_observation(&context).await;
        observed.worker = crate::atlas::classifier_worker_metrics(
            worker_before.as_ref(),
            worker_after.as_ref().map_err(|reason| *reason),
        );
        if let Some(reason) = stop {
            let report = Box::pin(repository::settle_activation(
                owner,
                &activation,
                &activation.checkpoint,
                &account,
                ActivationEnd::Continue {
                    reason: reason.into(),
                    delay_secs: 1,
                },
                chrono::Utc::now(),
            ))
            .await?;
            return Ok(Some(report));
        }
        let scored = match scored {
            Ok(value) => value,
            Err(_) => {
                let report = Box::pin(repository::settle_activation(
                    owner,
                    &activation,
                    &activation.checkpoint,
                    &account,
                    ActivationEnd::Continue {
                        reason: "classifier_retryable".into(),
                        delay_secs: 1,
                    },
                    chrono::Utc::now(),
                ))
                .await?;
                return Ok(Some(report));
            }
        };
        let outcomes: Vec<_> = scored
            .outcomes
            .into_iter()
            .map(|classification| {
                let key = match &classification {
                    crate::atlas_classify::ItemClassification::Evaluated { claim, .. }
                    | crate::atlas_classify::ItemClassification::Retryable { claim, .. } => {
                        &claim.occurrence
                    }
                };
                prepared
                    .sources
                    .get(key)
                    .cloned()
                    .map(|source| admission::MaintenanceOutcome::new(source, classification))
                    .ok_or_else(|| MaintenanceError::InvalidRequest {
                        code: "classification_source_missing".into(),
                    })
            })
            .collect::<Result<_, _>>()?;
        if cancel.load(std::sync::atomic::Ordering::Acquire)
            || admission::foreground_requested(owner.repository(), chrono::Utc::now()).await?
        {
            let report = Box::pin(repository::settle_activation(
                owner,
                &activation,
                &activation.checkpoint,
                &account,
                ActivationEnd::Continue {
                    reason: "foreground_or_shutdown".into(),
                    delay_secs: 1,
                },
                chrono::Utc::now(),
            ))
            .await?;
            return Ok(Some(report));
        }
        let (committed, checkpoint) = Box::pin(repository::publish_page(
            owner,
            &activation,
            &outcomes,
            &prepared.sources,
            chrono::Utc::now(),
        ))
        .await?;
        activation.checkpoint = checkpoint;
        for item in &committed.items {
            let kind = match item {
                admission::ItemCommit::Committed { .. } => "committed",
                admission::ItemCommit::AlreadyCommitted { .. } => "already_committed",
                admission::ItemCommit::StaleInput { .. } => "stale",
                admission::ItemCommit::LeaseLost { .. } => "lease_lost",
                admission::ItemCommit::Cancelled { .. } => "cancelled",
                admission::ItemCommit::Retryable { .. } => "retryable",
                admission::ItemCommit::InvalidReceipt { .. } => "invalid_receipt",
            };
            *observed.dispositions.entry(kind.to_owned()).or_default() += 1;
        }
        if committed.items.iter().any(|item| {
            matches!(
                item,
                admission::ItemCommit::Retryable { .. } | admission::ItemCommit::LeaseLost { .. }
            )
        }) {
            retry_delay = 1;
        }
        let accounted = repository::committed_item_ids(&activation, &prepared.sources, &committed)?;
        account.extend(accounted);
        account.sort();
        account.dedup();
        let invalid = committed
            .items
            .iter()
            .any(|item| matches!(item, admission::ItemCommit::InvalidReceipt { .. }));
        if invalid {
            let report = Box::pin(repository::settle_activation(
                owner,
                &activation,
                &activation.checkpoint,
                &account,
                ActivationEnd::Blocked {
                    reason: "invalid_classification_receipt".into(),
                },
                chrono::Utc::now(),
            ))
            .await?;
            return Ok(Some(report));
        }
    }
    let report = Box::pin(repository::settle_activation(
        owner,
        &activation,
        &activation.checkpoint,
        &account,
        ActivationEnd::Continue {
            reason: if prepared.examined == 0 {
                "scan_checkpoint"
            } else {
                "classification_checkpoint"
            }
            .into(),
            delay_secs: retry_delay,
        },
        chrono::Utc::now(),
    ))
    .await?;
    Ok(Some(report))
}

/// Schedules the default migration once for retained legacy rows. An absent
/// model returns a prerequisite error; no CSV, training, or active replacement
/// is attempted. The worker's existing diagnostic policy reports the block.
pub(crate) async fn ensure_initial(
    repo: &Repository,
    home: &ClassifierHome,
) -> Result<Option<String>, MaintenanceError> {
    if let Some(stored) =
        crate::storage::atlas_admission_maintenance::by_request(repo, "legacy-v3-initial").await?
    {
        return Ok(Some(stored.checkpoint.job_id));
    }
    if !crate::storage::atlas_admission_maintenance::legacy_remaining(repo).await? {
        return Ok(None);
    }
    start(
        repo,
        home,
        ReclassificationRequest::new("legacy-v3-initial", MaintenanceScope::AllRetained),
    )
    .await
    .map(Some)
}

struct ActivationObservation {
    stage: &'static str,
    rows_read: usize,
    worker: serde_json::Value,
    page: Option<crate::atlas_classify::PageMetrics>,
    dispositions: std::collections::BTreeMap<String, u64>,
}
impl ActivationObservation {
    fn new() -> Self {
        Self {
            stage: "identity",
            rows_read: 0,
            worker: crate::atlas::classifier_worker_metrics(None, Err("no_classifier_dispatch")),
            page: None,
            dispositions: std::collections::BTreeMap::new(),
        }
    }
}

/// Numeric observations follow the durable outcome. A telemetry failure is a
/// diagnostic only, never a rewrite of an accepted classifier publication.
async fn record_activation_metrics(
    owner: &crate::storage::classifier_admission::AdmissionOwner,
    task: &crate::storage::jobs::ClaimedTask,
    before: &MaintenanceCheckpoint,
    result: &Result<Option<MaintenanceReport>, MaintenanceError>,
    observed: &ActivationObservation,
    elapsed: std::time::Duration,
) {
    use crate::telemetry::{EventKind, TelemetryEvent, Trigger};
    let report = result.as_ref().ok().and_then(Option::as_ref);
    let db = owner.metrics();
    let payload = serde_json::json!({"schema_version":"atlas-maintenance-metrics-v1","stage":observed.stage,
        "atlas.stage.wall_ms":u64::try_from(elapsed.as_millis()).ok(),"atlas.stage.queue_ms":null,"atlas.stage.queue_reason":"activation_enqueue_timestamp_not_captured",
        "atlas.db":{"round_trips":db.round_trips(),"conflict_retries":db.conflict_retries(),"begin":db.begins(),"commit":db.commits(),"rollback":db.rollbacks()},
        "atlas.db.scope":"this_attempt_before_metrics_persistence_owner_queries_and_transaction_boundaries_excludes_native_memory_publisher_boundaries",
        "atlas.rows.read":observed.rows_read,"atlas.rows.unit":"occurrence_headers","atlas.rows.dispositions":observed.dispositions,
        "atlas.rows.scanned":report.and_then(|report|report.rows_scanned.checked_sub(before.rows_scanned)),
        "atlas.rows.reclassified":report.and_then(|report|report.rows_reclassified.checked_sub(before.rows_reclassified)),
        "atlas.rows.rejected":report.and_then(|report|report.rows_rejected.checked_sub(before.rows_rejected)),
        "atlas.rows.already_current":report.and_then(|report|report.rows_already_current.checked_sub(before.rows_already_current)),
        "atlas.rows.unavailable_reason":if report.is_none(){Some("durable_report_not_returned")}else{None},
        "atlas.classifier.model_id":before.pinned_snapshot.model_id,"atlas.classifier.manifest_sha256":before.manifest_sha256,
        "atlas.classifier.worker":observed.worker,"atlas.classifier.worker_scope":"pinned_runtime_window_including_other_clients",
        "atlas.classifier.page":observed.page,"atlas.classifier.page_unavailable_reason":if observed.page.is_none(){Some("no_completed_classifier_page")}else{None},
        "atlas.host.rss_bytes":null,"atlas.host.rss_reason":"process_rss_not_collected_by_runtime"});
    let payload = match serde_json::to_string(&payload) {
        Ok(serialized) if serialized.chars().count() <= crate::telemetry::MAX_PAYLOAD_CHARS => {
            payload
        }
        _ => {
            serde_json::json!({"schema_version":"atlas-maintenance-metrics-v1","metrics":null,"unavailable_reason":"telemetry_payload_bound"})
        }
    };
    let event = TelemetryEvent::new(EventKind::AtlasStage)
        .trigger(Trigger::Repair)
        .app("atlas")
        .job(&task.job_id)
        .tool("local_classifier")
        .category("maintenance")
        .model(&before.pinned_snapshot.model_id)
        .outcome(
            report
                .map(|report| report.state.as_str())
                .unwrap_or("failed"),
        )
        .duration_ms(i64::try_from(elapsed.as_millis()).ok())
        .count(1)
        .with_id(format!("maintenance-activation:{}", task.id))
        .payload(payload);
    if crate::storage::telemetry::record(owner.repository().db(), &event)
        .await
        .is_err()
    {
        tracing::warn!(target:"argos::classifier",job_id=%task.job_id,code="maintenance_telemetry_unavailable");
    }
}
