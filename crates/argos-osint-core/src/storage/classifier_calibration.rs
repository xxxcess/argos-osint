//! Durable provider observations and independent post-cycle monitoring.
//! @tag:classifier @tag:atlas-collection
//!
//! Provider rules have no reviewed precision certificate in this release. Every
//! partial label therefore remains monitoring data. This worker never adjusts
//! gates, fits a model, or replaces the active model on provider agreement.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use super::classifier_admission::{self as admission, TerminalCycle, TerminalEffect, TerminalHook};
use super::jobs::{self, ClaimedTask, NewJob, NewTask};
use super::repo::Repository;
use super::tx::{RetryPolicy, Tx, TxError};
use crate::atlas_calibration::{self, FamilyCandidate, FamilyResolution, FamilySignature};
use crate::classifier::artifact::sha256_hex;
use crate::classifier::provider_capture::CaptureContext;
use crate::classifier::provider_labels::{self, ProviderObservation};
use crate::classifier::readiness::ClassifierHome;
use crate::classifier::training::{FamilyId, Split};
use crate::classifier::ModelSnapshot;

pub const OPERATION: &str = "classifier_calibrate";
pub const POOL: &str = "classifier_calibration";
const PAGE_SIZE: usize = 64;
const ACTIVATION_ROWS: usize = 512;
const LEASE_SECS: i64 = 60;

pub const SCHEMA: &str = "
DEFINE TABLE IF NOT EXISTS classifier_observation SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS payload_json ON classifier_observation TYPE string;
DEFINE FIELD IF NOT EXISTS payload_sha256 ON classifier_observation TYPE string;
DEFINE FIELD IF NOT EXISTS disposition ON classifier_observation TYPE string DEFAULT 'pending';
DEFINE FIELD IF NOT EXISTS mapping_json ON classifier_observation TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS family_id ON classifier_observation TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS first_seen ON classifier_observation TYPE datetime;
DEFINE INDEX IF NOT EXISTS classifier_observation_family ON classifier_observation FIELDS family_id,first_seen;
DEFINE TABLE IF NOT EXISTS classifier_observation_origin SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS cycle_id ON classifier_observation_origin TYPE string;
DEFINE FIELD IF NOT EXISTS sequence ON classifier_observation_origin TYPE int;
DEFINE FIELD IF NOT EXISTS observation_id ON classifier_observation_origin TYPE string;
DEFINE FIELD IF NOT EXISTS fetch_key ON classifier_observation_origin TYPE string;
DEFINE FIELD IF NOT EXISTS request_input_hash ON classifier_observation_origin TYPE string;
DEFINE FIELD IF NOT EXISTS payload_json ON classifier_observation_origin TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON classifier_observation_origin TYPE datetime;
DEFINE INDEX IF NOT EXISTS classifier_origin_cycle ON classifier_observation_origin FIELDS cycle_id,sequence UNIQUE;
DEFINE INDEX IF NOT EXISTS classifier_origin_observation ON classifier_observation_origin FIELDS observation_id;
DEFINE TABLE IF NOT EXISTS classifier_capture_counter SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS sequence ON classifier_capture_counter TYPE int DEFAULT 0;
DEFINE TABLE IF NOT EXISTS classifier_calibration_job SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS cycle_id ON classifier_calibration_job TYPE string;
DEFINE FIELD IF NOT EXISTS watermark ON classifier_calibration_job TYPE int;
DEFINE FIELD IF NOT EXISTS after_sequence ON classifier_calibration_job TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS activation ON classifier_calibration_job TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS snapshot_json ON classifier_calibration_job TYPE string;
DEFINE FIELD IF NOT EXISTS manifest_sha256 ON classifier_calibration_job TYPE string;
DEFINE FIELD IF NOT EXISTS report_json ON classifier_calibration_job TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON classifier_calibration_job TYPE datetime;
DEFINE INDEX IF NOT EXISTS classifier_calibration_cycle ON classifier_calibration_job FIELDS cycle_id,watermark UNIQUE;
DEFINE TABLE IF NOT EXISTS classifier_story_family SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS split ON classifier_story_family TYPE string;
DEFINE FIELD IF NOT EXISTS signature_json ON classifier_story_family TYPE string;
DEFINE FIELD IF NOT EXISTS url_sha256 ON classifier_story_family TYPE option<string>;
DEFINE FIELD IF NOT EXISTS content_sha256 ON classifier_story_family TYPE string;
DEFINE FIELD IF NOT EXISTS bands ON classifier_story_family TYPE array<string>;
DEFINE FIELD IF NOT EXISTS first_capture_seq ON classifier_story_family TYPE int;
DEFINE FIELD IF NOT EXISTS last_seen ON classifier_story_family TYPE datetime;
DEFINE INDEX IF NOT EXISTS classifier_family_url ON classifier_story_family FIELDS url_sha256;
DEFINE INDEX IF NOT EXISTS classifier_family_content ON classifier_story_family FIELDS content_sha256;
DEFINE INDEX IF NOT EXISTS classifier_family_bands ON classifier_story_family FIELDS bands.*;
";

/// A calibration result is separate from Atlas completion and model quality.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CalibrationReport {
    pub job_id: String,
    pub cycle_id: String,
    pub watermark: u64,
    pub processed: u64,
    pub monitored: u64,
    pub duplicates: u64,
    pub quarantined: u64,
    pub state: String,
    pub reason: String,
    pub model_id: Option<String>,
    pub promoted: bool,
}

fn invalid(code: &str) -> TxError {
    TxError::Abort(code.into())
}

fn storage_error(error: admission::AdmissionError) -> TxError {
    match error {
        admission::AdmissionError::Storage(error) => error,
        admission::AdmissionError::LeaseLost => invalid("calibration_lease_lost"),
        admission::AdmissionError::CleanupUncertain { .. } => {
            invalid("calibration_cleanup_uncertain")
        }
        _ => invalid("calibration_storage_operation"),
    }
}

fn json(value: &impl Serialize, limit: usize) -> Result<String, TxError> {
    admission::bounded_json(value, limit, "calibration_payload")
        .map_err(|_| invalid("calibration_payload_bound"))
}

fn digest(parts: &[&str]) -> String {
    sha256_hex(super::codec::encode(parts).as_bytes())
}

/// Persist exact origins separately from the cross-cycle observation identity.
/// The batch joins a single guarded transaction; an exact replay adds no rows.
pub async fn capture(
    repo: &Repository,
    context: &CaptureContext,
    observations: &[ProviderObservation],
) -> Result<(), TxError> {
    let owner = admission::AdmissionOwner::new(repo.clone());
    let outcome = owner
        .scope(capture_owned(&owner, context, observations))
        .await;
    let cleanup = owner.shutdown().await.map_err(storage_error);
    outcome?;
    cleanup
}

pub(crate) async fn capture_owned(
    owner: &admission::AdmissionOwner,
    context: &CaptureContext,
    observations: &[ProviderObservation],
) -> Result<(), TxError> {
    if observations.len() > PAGE_SIZE || !crate::intel_service::valid_id(&context.cycle_id) {
        return Err(invalid("calibration_capture_bound"));
    }
    let mut prepared = Vec::with_capacity(observations.len());
    for observation in observations {
        provider_labels::validate_observation(observation)
            .map_err(|_| invalid("calibration_observation_bound"))?;
        if observation.cycle_ids != [context.cycle_id.clone()]
            || observation.fetched_at != context.fetched_at
        {
            return Err(invalid("calibration_origin_identity"));
        }
        let origin_json = json(observation, provider_labels::MAX_OBSERVATION_BYTES)?;
        let mut stable = observation.clone();
        stable.cycle_ids.clear();
        stable.fetched_at.clear();
        let stable_json = json(&stable, provider_labels::MAX_OBSERVATION_BYTES)?;
        let key = digest(&[
            "provider-origin-v1",
            &context.cycle_id,
            &context.fetch_receipt_key,
            &observation.observation_id,
        ]);
        prepared.push((
            key,
            observation.observation_id.clone(),
            origin_json,
            sha256_hex(stable_json.as_bytes()),
            stable_json,
        ));
    }
    let context = context.clone();
    admission::owned_tx(owner, RetryPolicy::default(), move |tx| {
        let context = context.clone();
        let prepared = prepared.clone();
        async move {
            jobs::bump(&tx, "job_guard", &format!("capture:{}", context.cycle_id)).await?;
            let items: Vec<_> = prepared.into_iter().map(|(key,id,origin,hash,stable)|
                serde_json::json!({"key":key,"id":id,"origin":origin,"hash":hash,"stable":stable})
            ).collect();
            // One SDK query per capture page, with immutable per-item identities.
            // The provider payload is data bound through the SDK, never code.
            jobs::run(&tx, tx.query(
                "FOR $item IN $items { \
                   LET $observation=type::record('classifier_observation',$item.id); \
                   LET $origin=type::record('classifier_observation_origin',$item.key); \
                   LET $old=(SELECT * FROM ONLY $observation); \
                   IF $old!=NONE AND $old.payload_sha256!=$item.hash {THROW 'observation_identity_conflict';}; \
                   IF $old=NONE {CREATE $observation SET payload_json=$item.stable,payload_sha256=$item.hash,first_seen=<datetime>$at RETURN NONE;}; \
                   IF !record::exists($origin) { \
                     UPSERT $counter SET sequence=(sequence ?? 0)+1 RETURN NONE; \
                     CREATE $origin SET cycle_id=$cycle,sequence=$counter.sequence,observation_id=$item.id,fetch_key=$fetch,request_input_hash=$input,payload_json=$item.origin,created_at=<datetime>$at RETURN NONE; \
                   }; \
                 };")
                .bind(("items",serde_json::Value::Array(items)))
                .bind(("counter",jobs::rid("classifier_capture_counter",&context.cycle_id)))
                .bind(("cycle",context.cycle_id)).bind(("fetch",context.fetch_receipt_key))
                .bind(("input",context.request_input_hash)).bind(("at",context.fetched_at))
            ).await?;
            Ok(())
        }
    }).await.map_err(storage_error)?;
    Ok(())
}

pub fn terminal_hook() -> TerminalHook {
    Arc::new(|tx, cycle, now| Box::pin(enqueue_in(tx, cycle, now)))
}

async fn enqueue_in(
    tx: Tx,
    cycle: TerminalCycle,
    now: DateTime<Utc>,
) -> Result<TerminalEffect, TxError> {
    jobs::bump(&tx, "job_guard", &format!("capture:{}", cycle.run_id)).await?;
    let mut response = jobs::run(
        &tx,
        tx.query("RETURN (SELECT * FROM ONLY $counter).sequence ?? 0")
            .bind((
                "counter",
                jobs::rid("classifier_capture_counter", &cycle.run_id),
            )),
    )
    .await?;
    let watermark = jobs::take::<Option<i64>>(&mut response, 0)?.unwrap_or(0);
    let job_id = format!(
        "classifier-calibration-{}",
        digest(&[&cycle.run_id, &watermark.to_string()])
    );
    let row = jobs::rid("classifier_calibration_job", &job_id);
    let mut exists = jobs::run(
        &tx,
        tx.query("RETURN record::exists($row)")
            .bind(("row", row.clone())),
    )
    .await?;
    if jobs::take::<Option<bool>>(&mut exists, 0)? == Some(true) {
        return Ok(TerminalEffect::new(watermark as u64, Some(job_id)));
    }
    let report = CalibrationReport {
        job_id: job_id.clone(),
        cycle_id: cycle.run_id.clone(),
        watermark: watermark as u64,
        processed: 0,
        monitored: 0,
        duplicates: 0,
        quarantined: 0,
        state: if watermark == 0 {
            "completed"
        } else if cycle.snapshot.is_none() {
            "blocked"
        } else {
            "queued"
        }
        .into(),
        reason: if watermark == 0 {
            "no_new_data"
        } else if cycle.snapshot.is_none() {
            "missing_model"
        } else {
            "pending_monitoring"
        }
        .into(),
        model_id: cycle
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.model_id.clone()),
        promoted: false,
    };
    jobs::bump(&tx, "job_guard", &job_id).await?;
    jobs::create_job_in(
        &tx,
        &NewJob {
            id: &job_id,
            kind: "maintenance",
            operation: OPERATION,
            parent: None,
            created_at: now,
        },
    )
    .await?;
    if report.state == "queued" {
        let task = format!("{job_id}:activate:0");
        jobs::create_task_in(
            &tx,
            &NewTask {
                id: &task,
                job: &job_id,
                pool: POOL,
                operation: OPERATION,
                priority: 20,
                deps: Vec::new(),
                input_hash: &cycle.identity,
                created_at: now,
            },
        )
        .await?;
    }
    jobs::run(&tx,tx.query(
        "CREATE $row SET cycle_id=$cycle,watermark=$watermark,snapshot_json=$pin,manifest_sha256=$manifest,report_json=$report,updated_at=<datetime>$now RETURN NONE; \
         UPDATE $job SET state=$state,result_ref=$result,model=$model,stage_coverage_json=$report,finished_at=IF $terminal {<datetime>$now} ELSE {NONE},updated_at=<datetime>$now RETURN NONE;")
        .bind(("row",row)).bind(("job",jobs::rid("job",&job_id)))
        .bind(("cycle",cycle.run_id)).bind(("watermark",watermark))
        .bind(("pin",json(&cycle.snapshot,4096)?)).bind(("manifest",cycle.manifest_sha256.unwrap_or_default()))
        .bind(("report",json(&report,16384)?)).bind(("state",report.state.clone()))
        .bind(("terminal",report.state=="completed")).bind(("model",report.model_id.unwrap_or_default()))
        .bind(("result",format!("classifier_calibration_job:{job_id}"))).bind(("now",now.to_rfc3339()))).await?;
    jobs::log_transition_in(
        &tx,
        &job_id,
        "enqueue",
        "classifier.calibration.queued",
        "Post-cycle observation monitoring",
        &now.to_rfc3339(),
    )
    .await?;
    Ok(TerminalEffect::new(watermark as u64, Some(job_id)))
}

#[derive(SurrealValue)]
struct JobRow {
    cycle_id: String,
    watermark: i64,
    after_sequence: i64,
    activation: i64,
    snapshot_json: String,
    manifest_sha256: String,
    report_json: String,
}

#[derive(Clone, SurrealValue)]
struct OriginRow {
    sequence: i64,
    observation_id: String,
    payload_json: String,
}

#[derive(SurrealValue)]
struct FamilyRow {
    family_id: String,
    split: String,
    signature_json: String,
    first_capture_seq: i64,
}

async fn check_owner(tx: &Tx, task: &ClaimedTask, now: DateTime<Utc>) -> Result<(), TxError> {
    jobs::bump(tx, "job_guard", &task.root_id).await?;
    if !jobs::fence(tx, task, &now.to_rfc3339())
        .await?
        .is_some_and(|fence| jobs::holds(&fence, task) && !fence.cancelled)
    {
        return Err(invalid("calibration_lease_lost"));
    }
    Ok(())
}

/// Cooperative Jobs cancellation: a live owner checkpoints its own work.
/// A restarted scheduler can finish cleanup only after the old lease expires.
pub(crate) async fn cancel_in(tx: &Tx, job: &str, now: DateTime<Utc>) -> Result<bool, TxError> {
    jobs::bump(tx, "job_guard", job).await?;
    let mut live = jobs::run(tx, tx.query(
        "UPDATE $job SET cancel_requested=true,updated_at=<datetime>$now WHERE state NOT IN ['completed','cancelled','failed'] RETURN NONE; \
         RETURN count(SELECT id FROM task WHERE job=$job AND state='running' AND lease_until><datetime>$now LIMIT 1)>0;")
        .bind(("job",jobs::rid("job",job))).bind(("now",now.to_rfc3339()))).await?;
    if jobs::take::<Option<bool>>(&mut live, 1)? == Some(true) {
        return Ok(false);
    }
    finish_cancel_in(tx, job, now).await?;
    Ok(true)
}

async fn finish_cancel_in(tx: &Tx, job: &str, now: DateTime<Utc>) -> Result<(), TxError> {
    let mut response = jobs::run(
        tx,
        tx.query("SELECT VALUE report_json FROM ONLY $row")
            .bind(("row", jobs::rid("classifier_calibration_job", job))),
    )
    .await?;
    let encoded = jobs::take::<Option<String>>(&mut response, 0)?
        .ok_or_else(|| invalid("calibration_job_missing"))?;
    let mut report: CalibrationReport =
        serde_json::from_str(&encoded).map_err(|_| invalid("calibration_report_corrupt"))?;
    if matches!(report.state.as_str(), "completed" | "cancelled" | "failed") {
        return Ok(());
    }
    report.state = "cancelled".into();
    report.reason = "explicit_cancel".into();
    jobs::run(tx,tx.query(
        "LET $tasks=SELECT VALUE id FROM task WHERE job=$job AND state IN ['running','queued']; \
         UPDATE $tasks SET state='cancelled',lease_epoch+=1,lease_owner='',process_owner='',lease_until=NONE,finished_at=<datetime>$now,updated_at=<datetime>$now RETURN NONE; \
         UPDATE attempt SET outcome='cancelled',finished_at=<datetime>$now,duration_ms=duration::millis(<datetime>$now-started_at) WHERE task IN $tasks AND finished_at=NONE RETURN NONE; \
         UPDATE $row SET report_json=$report,updated_at=<datetime>$now RETURN NONE; \
         UPDATE $job SET state='cancelled',worker_owner='',finished_at=<datetime>$now,updated_at=<datetime>$now,stage_coverage_json=$report RETURN NONE;")
        .bind(("job",jobs::rid("job",job))).bind(("row",jobs::rid("classifier_calibration_job",job)))
        .bind(("report",json(&report,16384)?)).bind(("now",now.to_rfc3339()))).await?;
    jobs::log_transition_in(
        tx,
        job,
        "cancel",
        "classifier.calibration.cancelled",
        "explicit_cancel",
        &now.to_rfc3339(),
    )
    .await?;
    Ok(())
}

async fn settle_owned_cancel(
    owner: &admission::AdmissionOwner,
    task: &ClaimedTask,
) -> Result<bool, TxError> {
    let task = task.clone();
    admission::owned_tx(owner, RetryPolicy::default(), move |tx| {
        let task = task.clone();
        async move {
            jobs::bump(&tx, "job_guard", &task.root_id).await?;
            let now = Utc::now();
            let owned = jobs::fence(&tx, &task, &now.to_rfc3339())
                .await?
                .is_some_and(|fence| jobs::holds(&fence, &task) && fence.cancelled);
            if owned {
                finish_cancel_in(&tx, &task.job_id, now).await?;
            }
            Ok(owned)
        }
    })
    .await
    .map(|committed| committed.value)
    .map_err(|_| invalid("calibration_cancel_cleanup"))
}

async fn settle_pending_cancel(owner: &admission::AdmissionOwner) -> Result<bool, TxError> {
    admission::owned_tx(owner, RetryPolicy::default(), |tx| async move {
        let mut response=jobs::run(&tx,tx.query(
            "SELECT VALUE record::id(id) FROM job WHERE operation=$operation AND cancel_requested=true AND state NOT IN ['completed','cancelled','failed'] ORDER BY created_at LIMIT 1")
            .bind(("operation",OPERATION))).await?;
        let jobs:Vec<String>=jobs::take(&mut response,0)?;
        if let Some(job)=jobs.first() { cancel_in(&tx,job,Utc::now()).await } else { Ok(false) }
    }).await.map(|committed|committed.value).map_err(storage_error)
}

/// One bounded activation, yielding to foreground work between 64-row pages.
/// A dropped caller cannot abandon its transactions: AdmissionOwner joins them.
pub async fn drain_one(
    repo: &Repository,
    home: &ClassifierHome,
    worker: &str,
    cancel: Arc<AtomicBool>,
) -> Result<usize, TxError> {
    let owner = admission::AdmissionOwner::new(repo.clone());
    let outcome = owner
        .scope(async {
            if cancel.load(Ordering::Acquire) {
                return Ok(0);
            }
            if settle_pending_cancel(&owner).await? {
                return Ok(1);
            }
            if admission::foreground_requested(repo, Utc::now())
                .await
                .map_err(storage_error)?
            {
                return Ok(0);
            }
            Box::pin(drain_owned(&owner, home, worker, &cancel)).await
        })
        .await;
    let cleanup = owner
        .shutdown()
        .await
        .map_err(|_| invalid("calibration_cleanup"));
    match (outcome, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (_, Err(error)) => Err(error),
    }
}

async fn drain_owned(
    owner: &admission::AdmissionOwner,
    home: &ClassifierHome,
    worker: &str,
    cancel: &Arc<AtomicBool>,
) -> Result<usize, TxError> {
    let now = Utc::now();
    let worker = worker.to_owned();
    let claim_key = format!(
        "calibration-claim:{worker}:{}",
        now.timestamp_nanos_opt().unwrap_or_default()
    );
    let task = admission::owned_tx(owner, RetryPolicy::default(), move |tx| {
        let worker = worker.clone();
        let claim_key = claim_key.clone();
        async move {
            jobs::interrupt_expired_pool_in(&tx, POOL, now).await?;
            settle_failed_activations_in(&tx, now).await?;
            jobs::claim_in(
                &tx,
                POOL,
                None,
                &worker,
                LEASE_SECS,
                now,
                &claim_key,
                Some(jobs::claim_shard(&worker)),
            )
            .await
        }
    })
    .await
    .map_err(storage_error)?
    .value;
    let Some(task) = task else {
        return Ok(0);
    };
    let outcome = Box::pin(process_task(owner, home, task.clone(), cancel)).await;
    if outcome.is_err() && settle_owned_cancel(owner, &task).await? {
        return Ok(1);
    }
    outcome
}

async fn settle_failed_activations_in(tx: &Tx, now: chrono::DateTime<Utc>) -> Result<(), TxError> {
    let mut response = jobs::run(tx, tx.query(
        "SELECT VALUE record::id(id) FROM job WHERE operation=$operation AND state INSIDE ['queued','running'] AND count(SELECT id FROM task WHERE job=$parent.id AND state='failed' LIMIT 1)>0 LIMIT 64;"
    ).bind(("operation", OPERATION))).await?;
    for job in jobs::take::<Vec<String>>(&mut response, 0)? {
        let mut response = jobs::run(tx, tx.query(
            "SELECT VALUE IF bytes::len(type::bytes(report_json))<=16384 {report_json} ELSE {NONE} FROM ONLY $row;"
        ).bind(("row",jobs::rid("classifier_calibration_job",&job)))).await?;
        let body = jobs::take::<Option<String>>(&mut response, 0)?
            .ok_or_else(|| invalid("calibration_report_missing"))?;
        let mut report: CalibrationReport =
            serde_json::from_str(&body).map_err(|_| invalid("calibration_report_corrupt"))?;
        report.state = "failed".into();
        report.reason = "worker_recovery_exhausted".into();
        let report = json(&report, 16384)?;
        jobs::run(tx, tx.query("UPDATE $row SET report_json=$report,updated_at=<datetime>$now RETURN NONE; UPDATE $job SET state='failed',stage_coverage_json=$report,worker_owner='',finished_at=<datetime>$now,updated_at=<datetime>$now RETURN NONE;")
            .bind(("row",jobs::rid("classifier_calibration_job",&job))).bind(("job",jobs::rid("job",&job)))
            .bind(("report",report)).bind(("now",now.to_rfc3339()))).await?;
    }
    Ok(())
}

async fn process_task(
    owner: &admission::AdmissionOwner,
    home: &ClassifierHome,
    task: ClaimedTask,
    cancel: &Arc<AtomicBool>,
) -> Result<usize, TxError> {
    let mut response = admission::db_query(owner.repository().db(), "SELECT * FROM ONLY $row")
        .bind(("row", jobs::rid("classifier_calibration_job", &task.job_id)))
        .await?
        .check()?;
    let job: JobRow = response
        .take::<Option<JobRow>>(0)?
        .ok_or_else(|| invalid("calibration_job_missing"))?;
    let mut report: CalibrationReport = serde_json::from_str(&job.report_json)
        .map_err(|_| invalid("calibration_report_corrupt"))?;
    let snapshot: Option<ModelSnapshot> =
        serde_json::from_str(&job.snapshot_json).map_err(|_| invalid("calibration_pin_corrupt"))?;
    let zero_shot = snapshot.as_ref().is_some_and(|snapshot| {
        snapshot.algorithm_version == crate::classifier::ZERO_SHOT_ALGORITHM_VERSION
    });
    let integrity = match snapshot {
        // Monitoring consumes capture records only. Its durable terminal hook
        // already binds the run identity; it never loads an NLI checkpoint.
        Some(_) if zero_shot => Ok(()),
        Some(snapshot) => {
            home.validate_saved_pin_integrity(&snapshot, &job.manifest_sha256)
                .await
        }
        None => Err(crate::classifier::ClassifierError::Unavailable),
    };
    let mut after = job.after_sequence;
    let mut processed = 0;
    if integrity.is_err() {
        report.state = "blocked".into();
        report.reason = "pinned_model_missing".into();
    } else {
        while processed < ACTIVATION_ROWS
            && !cancel.load(Ordering::Acquire)
            && !admission::foreground_requested(owner.repository(), Utc::now())
                .await
                .map_err(|_| invalid("foreground_check"))?
        {
            // The job watermark freezes its cohort; later captures belong to a
            // later terminal revision and cannot extend a running activation.
            let mut response=admission::db_query(owner.repository().db(),
                "SELECT sequence,observation_id,payload_json \
                 FROM classifier_observation_origin WHERE cycle_id=$cycle AND sequence>$after AND sequence<=$upper ORDER BY sequence LIMIT 64")
                .bind(("cycle",job.cycle_id.clone())).bind(("after",after)).bind(("upper",job.watermark)).await?.check()?;
            let rows: Vec<OriginRow> = response.take(0)?;
            if rows.is_empty() {
                after = job.watermark;
                break;
            }
            let page_after = rows.last().expect("nonempty page").sequence;
            let signature_cancel = cancel.clone();
            let signatures = tokio::task::spawn_blocking(move || {
                rows.into_iter()
                    .map(|row| {
                        let observation: ProviderObservation =
                            serde_json::from_str(&row.payload_json)
                                .map_err(|_| invalid("observation_corrupt"))?;
                        let signature =
                            atlas_calibration::family_signature(&observation, &signature_cancel)
                                .ok();
                        Ok((row, observation, signature))
                    })
                    .collect::<Result<Vec<_>, TxError>>()
            })
            .await
            .map_err(|_| invalid("family_worker_failed"))??;
            let page_report = report.clone();
            let task_copy = task.clone();
            let cycle = job.cycle_id.clone();
            let prior = after;
            let committed=admission::owned_tx(owner,RetryPolicy::default(),move|tx| {
                let task=task_copy.clone();let signatures=signatures.clone();let cycle=cycle.clone();let mut page_report=page_report.clone();
                async move {
                    check_owner(&tx,&task,Utc::now()).await?;
                    jobs::bump(&tx,"job_guard","classifier-families").await?;
                    jobs::renew_owned_in(&tx,&task,LEASE_SECS,Utc::now()).await?;
                    // The family guard serializes admission for this whole page.
                    // Read retention capacity once, then account for each create
                    // locally; a retried transaction starts from durable state.
                    let mut capacity = jobs::run(&tx, tx.query("RETURN count(SELECT id FROM classifier_story_family LIMIT 10000)")).await?;
                    let retained = jobs::take::<Option<i64>>(&mut capacity, 0)?.unwrap_or(10000);
                    let mut family_slots = 10000_i64.saturating_sub(retained).max(0);
                    let mut totals=[0u64;3];
                    for(row,observation,signature)in &signatures {
                        let disposition=match signature {
                            Some(signature)=>monitor_in(&tx,row,observation,signature,&mut family_slots).await?,
                            None=>{
                                jobs::run(&tx,tx.query("UPDATE $row SET disposition='signature_unavailable' WHERE disposition='pending' RETURN NONE").bind(("row",jobs::rid("classifier_observation",&row.observation_id)))).await?;
                                2
                            }
                        };
                        totals[disposition]+=1;
                    }
                    page_report.processed += signatures.len() as u64;
                    page_report.monitored += totals[0];
                    page_report.duplicates += totals[1];
                    page_report.quarantined += totals[2];
                    jobs::run(&tx,tx.query("UPDATE $row SET after_sequence=$after,report_json=$report,updated_at=time::now() WHERE after_sequence=$prior AND cycle_id=$cycle RETURN NONE")
                        .bind(("row",jobs::rid("classifier_calibration_job",&task.job_id))).bind(("after",page_after)).bind(("report",json(&page_report,16384)?)).bind(("prior",prior)).bind(("cycle",cycle))).await?;
                    Ok((signatures.len(),page_report))
                }
            }).await.map_err(storage_error)?.value;
            after = page_after;
            processed += committed.0;
            report = committed.1;
        }
        report.state = if after >= job.watermark {
            "completed"
        } else {
            "queued"
        }
        .into();
        report.reason = if after >= job.watermark {
            "insufficient_reviewed_evidence"
        } else {
            "foreground_or_page_checkpoint"
        }
        .into();
    }
    if report.state == "completed" && after >= job.watermark && zero_shot {
        report.reason = "zero_shot_provider_monitoring_only".into();
    } else if report.state == "completed" && after >= job.watermark {
        report.reason = match super::classifier_learning::consider(owner, home).await {
            Ok("unapproved_mapping_monitor_only") => "insufficient_reviewed_evidence".into(),
            Ok(reason) => reason.into(),
            Err(TxError::Abort(code))
                if code.len() <= 128
                    && code
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') =>
            {
                code
            }
            Err(_) => "learning_capture_failed".into(),
        };
    }
    let terminal = report.state != "queued";
    let captured = (task, report, after, job.activation);
    admission::owned_tx(owner,RetryPolicy::default(),move|tx| {
        let(task,report,after,activation)=captured.clone();
        async move {
            let now=Utc::now();check_owner(&tx,&task,now).await?;
            if terminal {jobs::complete_in(&tx,&task,&sha256_hex(json(&report,16384)?.as_bytes()),&now.to_rfc3339()).await?;}
            else {
                let next=format!("{}:activate:{}",task.job_id,activation+1);
                let hash=digest(&[&task.input_hash,&after.to_string()]);
                if jobs::yield_owned_in(&tx,&task,&NewTask {id:&next,job:&task.job_id,pool:POOL,operation:OPERATION,priority:20,deps:vec![&task.id],input_hash:&hash,created_at:now},now+chrono::Duration::seconds(1),"calibration_checkpoint",now).await?!=jobs::Publish::Committed {
                    return Err(invalid("calibration_yield_lost"));
                }
            }
            let report_json=json(&report,16384)?;
            jobs::run(&tx,tx.query("UPDATE $row SET activation=$activation,report_json=$report,updated_at=<datetime>$now RETURN NONE; UPDATE $job SET state=$state,stage_coverage_json=$report,progress_done=$done,worker_owner='',finished_at=IF $completed {<datetime>$now} ELSE {NONE},updated_at=<datetime>$now RETURN NONE")
                .bind(("row",jobs::rid("classifier_calibration_job",&task.job_id))).bind(("job",jobs::rid("job",&task.job_id)))
                .bind(("activation",activation+1)).bind(("report",report_json)).bind(("state",if report.state=="queued"{"running"}else{&report.state}.to_owned()))
                .bind(("done",report.processed as i64)).bind(("completed",report.state=="completed")).bind(("now",now.to_rfc3339()))).await?;
            jobs::log_transition_in(&tx,&task.job_id,&task.id,"classifier.calibration.result",&report.reason,&now.to_rfc3339()).await?;
            Ok(())
        }
    }).await.map_err(storage_error)?;
    Ok(processed.max(1))
}

/// Monitoring returns the disposition counter slot: accepted, duplicate, quarantine.
async fn monitor_in(
    tx: &Tx,
    row: &OriginRow,
    observation: &ProviderObservation,
    signature: &FamilySignature,
    family_slots: &mut i64,
) -> Result<usize, TxError> {
    let mut current = jobs::run(
        tx,
        tx.query("SELECT VALUE disposition FROM ONLY $row").bind((
            "row",
            jobs::rid("classifier_observation", &row.observation_id),
        )),
    )
    .await?;
    let disposition = jobs::take::<Option<String>>(&mut current, 0)?
        .ok_or_else(|| invalid("observation_missing"))?;
    if disposition != "pending" {
        return Ok(1);
    }
    let mut response=jobs::run(tx,tx.query(
        "SELECT record::id(id) AS family_id,split,signature_json,first_capture_seq \
         FROM classifier_story_family WHERE content_sha256=$content OR ($url!=NONE AND url_sha256=$url) OR array::intersect(bands,$bands)!=[] \
         ORDER BY first_capture_seq,id LIMIT 101")
        .bind(("content",signature.content_sha256.clone())).bind(("url",signature.url_sha256.clone())).bind(("bands",signature.bands.to_vec()))).await?;
    let rows: Vec<FamilyRow> = jobs::take(&mut response, 0)?;
    let candidates = rows
        .into_iter()
        .map(|row| {
            let family_id =
                FamilyId::parse(&row.family_id).map_err(|_| invalid("family_identity"))?;
            let split: Split = serde_json::from_value(serde_json::Value::String(row.split))
                .map_err(|_| invalid("family_split"))?;
            let signature: FamilySignature =
                serde_json::from_str(&row.signature_json).map_err(|_| invalid("family_corrupt"))?;
            Ok(FamilyCandidate {
                family_id,
                split,
                first_capture_seq: row.first_capture_seq as u64,
                url_sha256: signature.url_sha256.clone(),
                content_sha256: signature.content_sha256.clone(),
                observed_at: signature.observed_at,
                signature: Some(signature),
            })
        })
        .collect::<Result<Vec<_>, TxError>>()?;
    let resolution = atlas_calibration::resolve_family(
        observation,
        signature,
        &candidates,
        candidates.len() > 100,
    )
    .map_err(|_| invalid("family_resolution"))?;
    let (family, split, new, reason) = match resolution {
        FamilyResolution::New { family_id, split } => {
            (Some(family_id), Some(split), true, "monitoring")
        }
        FamilyResolution::Existing {
            family_id,
            split,
            aliases,
        } if aliases.is_empty() => (Some(family_id), Some(split), false, "monitoring"),
        FamilyResolution::Existing { .. } => (None, None, false, "ambiguous_family_merge"),
        FamilyResolution::Quarantined { code, .. } => (None, None, false, code),
    };
    let mut reason = reason;
    if new && *family_slots == 0 {
        reason = "retention_capacity";
    }
    let keep = reason == "monitoring";
    let approval = super::classifier_review::active_approval_in(tx).await?;
    let mapping = provider_labels::map_labels(observation, approval.as_ref())
        .map_err(|_| invalid("provider_mapping_invalid"))?;
    if keep {
        let family = family.as_ref().expect("monitoring has a family");
        if new {
            jobs::run(tx,tx.query("CREATE $row SET split=$split,signature_json=$signature,url_sha256=$url,content_sha256=$content,bands=$bands,first_capture_seq=$seq,last_seen=time::now() RETURN NONE")
                .bind(("row",jobs::rid("classifier_story_family",family.as_str()))).bind(("split",serde_json::to_value(split).map_err(|_|invalid("family_split"))?))
                .bind(("signature",json(signature,64*1024)?)).bind(("url",signature.url_sha256.clone())).bind(("content",signature.content_sha256.clone())).bind(("bands",signature.bands.to_vec())).bind(("seq",row.sequence))).await?;
            *family_slots -= 1;
        }
    }
    jobs::run(tx,tx.query("UPDATE $row SET disposition=$reason,mapping_json=$mapping,family_id=$family RETURN NONE")
        .bind(("row",jobs::rid("classifier_observation",&row.observation_id))).bind(("reason",reason.to_owned()))
        .bind(("mapping",json(&mapping,64*1024)?)).bind(("family",if keep {family.map(|family|family.as_str().to_owned()).unwrap_or_default()}else{String::new()}))).await?;
    Ok(if keep { 0 } else { 2 })
}
