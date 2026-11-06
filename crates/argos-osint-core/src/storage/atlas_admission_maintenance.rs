//! Historical admission checkpoints and immutable retained-source captures.
//! Uses the common maintenance publication fence; never invents a RunClaim.
use std::collections::BTreeMap;
use std::future::Future;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use super::classifier_admission::{
    self as admission, AdmissionClaim, AdmissionOwner, ClaimedInput, MaintenancePublicationFence,
    RetainedSourceFence,
};
use super::jobs::{self, ClaimedTask, NewJob, NewTask};
use super::repo::Repository;
use super::tx::{RetryPolicy, Tx, TxError};
use crate::atlas_admission_maintenance::{
    MaintenanceCheckpoint, MaintenanceError, MaintenanceReport, MaintenanceScope, MaintenanceState,
    ReclassificationRequest, LEASE_SECS, OPERATION, POOL, PRIORITY,
};
use crate::atlas_classify::{AdmissionContext, OccurrenceKey};
use crate::classifier::ModelSnapshot;
use crate::store::AtlasArticleRow;

pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS atlas_reclassification_job SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS request_id ON atlas_reclassification_job TYPE string;
DEFINE FIELD IF NOT EXISTS request_hash ON atlas_reclassification_job TYPE string;
DEFINE FIELD IF NOT EXISTS scope_key ON atlas_reclassification_job TYPE string;
DEFINE FIELD IF NOT EXISTS pin_json ON atlas_reclassification_job TYPE string;
DEFINE FIELD IF NOT EXISTS manifest_sha256 ON atlas_reclassification_job TYPE string;
DEFINE FIELD IF NOT EXISTS state ON atlas_reclassification_job TYPE string;
DEFINE FIELD IF NOT EXISTS activation_task_id ON atlas_reclassification_job TYPE string;
DEFINE FIELD IF NOT EXISTS activation_input_hash ON atlas_reclassification_job TYPE string;
DEFINE FIELD IF NOT EXISTS cancel_requested ON atlas_reclassification_job TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS pause_requested ON atlas_reclassification_job TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS resume_requested ON atlas_reclassification_job TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS checkpoint_json ON atlas_reclassification_job TYPE string;
DEFINE FIELD IF NOT EXISTS report_json ON atlas_reclassification_job TYPE string;
DEFINE FIELD IF NOT EXISTS schema_version ON atlas_reclassification_job TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON atlas_reclassification_job TYPE datetime;
DEFINE INDEX IF NOT EXISTS atlas_reclassification_request ON atlas_reclassification_job FIELDS request_id UNIQUE;
DEFINE INDEX IF NOT EXISTS atlas_reclassification_state ON atlas_reclassification_job FIELDS state,resume_requested,updated_at;
DEFINE TABLE IF NOT EXISTS atlas_reclassification_origin SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS run_id ON atlas_reclassification_origin TYPE string;
DEFINE FIELD IF NOT EXISTS article_id ON atlas_reclassification_origin TYPE string;
DEFINE FIELD IF NOT EXISTS source_revision ON atlas_reclassification_origin TYPE string;
DEFINE FIELD IF NOT EXISTS capture_json ON atlas_reclassification_origin TYPE string;
DEFINE FIELD IF NOT EXISTS capture_sha256 ON atlas_reclassification_origin TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON atlas_reclassification_origin TYPE datetime;
DEFINE INDEX IF NOT EXISTS atlas_reclassification_origin_occurrence ON atlas_reclassification_origin FIELDS run_id,article_id,source_revision UNIQUE;
DEFINE TABLE IF NOT EXISTS atlas_reclassification_item SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS job_id ON atlas_reclassification_item TYPE string;
DEFINE FIELD IF NOT EXISTS run_id ON atlas_reclassification_item TYPE string;
DEFINE FIELD IF NOT EXISTS article_id ON atlas_reclassification_item TYPE string;
DEFINE FIELD IF NOT EXISTS source_revision ON atlas_reclassification_item TYPE string;
DEFINE FIELD IF NOT EXISTS origin_key ON atlas_reclassification_item TYPE string;
DEFINE FIELD IF NOT EXISTS original_row_json ON atlas_reclassification_item TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS original_metadata_json ON atlas_reclassification_item TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS original_input_revision ON atlas_reclassification_item TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS original_provenance_json ON atlas_reclassification_item TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS source_fence_json ON atlas_reclassification_item TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS receipt_key ON atlas_reclassification_item TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS disposition ON atlas_reclassification_item TYPE string;
DEFINE FIELD IF NOT EXISTS owner ON atlas_reclassification_item TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS epoch ON atlas_reclassification_item TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS lease_until ON atlas_reclassification_item TYPE option<datetime>;
DEFINE FIELD IF NOT EXISTS input_revision ON atlas_reclassification_item TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS conflict_attempts ON atlas_reclassification_item TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS evaluation_generation ON atlas_reclassification_item TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS last_error ON atlas_reclassification_item TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS derivative_state ON atlas_reclassification_item TYPE string DEFAULT 'pending';
DEFINE FIELD IF NOT EXISTS source_retry_pending ON atlas_reclassification_item TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS stats_dirty ON atlas_reclassification_item TYPE bool DEFAULT true;
DEFINE FIELD IF NOT EXISTS evaluated_counted ON atlas_reclassification_item TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS skipped_counted ON atlas_reclassification_item TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS already_counted ON atlas_reclassification_item TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS conflicted_counted ON atlas_reclassification_item TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS derivative_open_counted ON atlas_reclassification_item TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS accounted_conflict_attempts ON atlas_reclassification_item TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS last_conflict_task ON atlas_reclassification_item TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS last_conflict_epoch ON atlas_reclassification_item TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS updated_at ON atlas_reclassification_item TYPE datetime;
DEFINE INDEX IF NOT EXISTS atlas_reclassification_item_stats ON atlas_reclassification_item FIELDS job_id,stats_dirty,run_id,article_id;
DEFINE INDEX IF NOT EXISTS atlas_reclassification_item_occurrence ON atlas_reclassification_item FIELDS job_id,run_id,article_id;
DEFINE INDEX IF NOT EXISTS atlas_reclassification_item_work ON atlas_reclassification_item FIELDS job_id,disposition,run_id,article_id;
DEFINE INDEX IF NOT EXISTS atlas_reclassification_item_derivative ON atlas_reclassification_item FIELDS job_id,derivative_state,run_id,article_id;
DEFINE TABLE IF NOT EXISTS atlas_reclassification_conflict SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS job_id ON atlas_reclassification_conflict TYPE string;
DEFINE FIELD IF NOT EXISTS run_id ON atlas_reclassification_conflict TYPE string;
DEFINE FIELD IF NOT EXISTS article_id ON atlas_reclassification_conflict TYPE string;
DEFINE FIELD IF NOT EXISTS kind ON atlas_reclassification_conflict TYPE string;
DEFINE FIELD IF NOT EXISTS capture_revision ON atlas_reclassification_conflict TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS generation ON atlas_reclassification_conflict TYPE int;
DEFINE FIELD IF NOT EXISTS cumulative_attempts ON atlas_reclassification_conflict TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS generation_attempts ON atlas_reclassification_conflict TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS created_at ON atlas_reclassification_conflict TYPE datetime;
DEFINE INDEX IF NOT EXISTS atlas_reclassification_conflict_limit ON atlas_reclassification_conflict FIELDS job_id,kind,generation,generation_attempts;";

#[derive(SurrealValue)]
struct JobRow {
    request_id: String,
    request_hash: String,
    scope_key: String,
    pin_json: Option<String>,
    manifest_sha256: String,
    state: String,
    activation_task_id: String,
    activation_input_hash: String,
    cancel_requested: bool,
    pause_requested: bool,
    resume_requested: bool,
    checkpoint_json: Option<String>,
    report_json: Option<String>,
}
pub(crate) struct StoredJob {
    pub request_id: String,
    pub request_hash: String,
    pub activation_task_id: String,
    pub activation_input_hash: String,
    pub checkpoint: MaintenanceCheckpoint,
    pub report: MaintenanceReport,
    pub cancel_requested: bool,
    pub pause_requested: bool,
    pub resume_requested: bool,
}
const JOB_PROJECTION: &str="request_id,request_hash,scope_key,IF bytes::len(type::bytes(pin_json))<=4096 {pin_json} ELSE {NONE} AS pin_json,manifest_sha256,state,activation_task_id,activation_input_hash,cancel_requested,pause_requested,resume_requested,IF bytes::len(type::bytes(checkpoint_json))<=16384 {checkpoint_json} ELSE {NONE} AS checkpoint_json,IF bytes::len(type::bytes(report_json))<=16384 {report_json} ELSE {NONE} AS report_json";
fn decode(row: JobRow) -> Result<StoredJob, MaintenanceError> {
    let invalid = || MaintenanceError::InvalidRequest {
        code: "corrupt_maintenance_request".into(),
    };
    let checkpoint: MaintenanceCheckpoint =
        serde_json::from_str(&row.checkpoint_json.ok_or_else(invalid)?).map_err(|_| invalid())?;
    checkpoint.validate()?;
    let report: MaintenanceReport =
        serde_json::from_str(&row.report_json.ok_or_else(invalid)?).map_err(|_| invalid())?;
    let pin: ModelSnapshot =
        serde_json::from_str(&row.pin_json.ok_or_else(invalid)?).map_err(|_| invalid())?;
    if report.job_id != checkpoint.job_id
        || report.model_id != checkpoint.pinned_snapshot.model_id
        || report.state.as_str() != row.state
        || checkpoint.scope.key() != row.scope_key
        || checkpoint.pinned_snapshot != pin
        || checkpoint.manifest_sha256 != row.manifest_sha256
        || row.request_hash.len() != 64
    {
        return Err(invalid());
    }
    let expected_job = crate::atlas_admission_maintenance::stable_job_id(
        &row.request_id,
        &checkpoint.scope,
        &pin,
        &row.manifest_sha256,
    );
    let expected_hash = crate::classifier::artifact::sha256_hex(
        serde_json::to_string(&serde_json::json!([
            row.request_id,
            checkpoint.scope,
            pin,
            row.manifest_sha256
        ]))
        .map_err(|_| invalid())?
        .as_bytes(),
    );
    if checkpoint.job_id != expected_job
        || row.request_hash != expected_hash
        || row.activation_input_hash.len() != 64
        || row.activation_task_id
            != format!(
                "{}:activate:{}",
                checkpoint.job_id, checkpoint.activation_seq
            )
    {
        return Err(invalid());
    }
    Ok(StoredJob {
        request_id: row.request_id,
        request_hash: row.request_hash,
        activation_task_id: row.activation_task_id,
        activation_input_hash: row.activation_input_hash,
        checkpoint,
        report,
        cancel_requested: row.cancel_requested,
        pause_requested: row.pause_requested,
        resume_requested: row.resume_requested,
    })
}
pub(crate) async fn load(repo: &Repository, job: &str) -> Result<StoredJob, MaintenanceError> {
    if !crate::intel_service::valid_id(job) {
        return Err(MaintenanceError::InvalidRequest {
            code: "invalid_job_id".into(),
        });
    }
    let mut response =
        admission::db_query(repo.db(), format!("SELECT {JOB_PROJECTION} FROM ONLY $job"))
            .bind(("job", jobs::rid("atlas_reclassification_job", job)))
            .await
            .map_err(TxError::from)?
            .check()
            .map_err(TxError::from)?;
    decode(
        response
            .take::<Option<JobRow>>(0)
            .map_err(TxError::from)?
            .ok_or(MaintenanceError::NotFound)?,
    )
}
pub(crate) async fn by_request(
    repo: &Repository,
    id: &str,
) -> Result<Option<StoredJob>, MaintenanceError> {
    let mut response=admission::db_query(repo.db(),format!("SELECT {JOB_PROJECTION} FROM atlas_reclassification_job WHERE request_id=$request LIMIT 1"))
        .bind(("request",id.to_owned())).await.map_err(TxError::from)?.check().map_err(TxError::from)?;
    let rows: Vec<JobRow> = response.take(0).map_err(TxError::from)?;
    rows.into_iter().next().map(decode).transpose()
}
async fn owned<T, F, Fut>(repo: &Repository, body: F) -> Result<T, MaintenanceError>
where
    T: Send + 'static,
    F: FnMut(Tx) -> Fut + Send + 'static,
    Fut: Future<Output = Result<T, TxError>> + Send + 'static,
{
    let owner = AdmissionOwner::new(repo.clone());
    let result = admission::owned_tx(&owner, RetryPolicy::default(), body).await;
    owner.shutdown().await?;
    Ok(result?.value)
}
fn scope_run(scope: &MaintenanceScope) -> Option<&str> {
    match scope {
        MaintenanceScope::AllRetained => None,
        MaintenanceScope::Run { run_id } => Some(run_id),
    }
}
fn scope_where(scope: &MaintenanceScope) -> &'static str {
    if matches!(scope, MaintenanceScope::AllRetained) {
        "true"
    } else {
        "run_id=$scope_run"
    }
}
#[derive(SurrealValue)]
struct Header {
    run_id: String,
    article_id: String,
}
impl From<Header> for OccurrenceKey {
    fn from(value: Header) -> Self {
        Self::new(value.run_id, value.article_id)
    }
}

pub(crate) async fn create(
    repo: &Repository,
    request: &ReclassificationRequest,
    context: &AdmissionContext,
    now: DateTime<Utc>,
) -> Result<String, MaintenanceError> {
    let request = request.clone();
    let pin = context.pin.snapshot.clone();
    let manifest = context.manifest_sha256.clone();
    let job = crate::atlas_admission_maintenance::stable_job_id(
        &request.request_id,
        &request.scope,
        &pin,
        &manifest,
    );
    let request_hash = crate::classifier::artifact::sha256_hex(
        serde_json::to_string(&serde_json::json!([
            request.request_id,
            request.scope,
            pin,
            manifest
        ]))
        .map_err(|_| MaintenanceError::InvalidRequest {
            code: "request_json".into(),
        })?
        .as_bytes(),
    );
    let captured = (request, pin, manifest, job, request_hash);
    owned(repo,move|tx| {let (request,pin,manifest,job,request_hash)=captured.clone();async move {
        jobs::bump(&tx,"job_guard",&format!("maintenance-request:{}",crate::classifier::artifact::sha256_hex(request.request_id.as_bytes()))).await?;
        let mut existing=jobs::run(&tx,tx.query("SELECT record::id(id) AS job_id,request_hash FROM atlas_reclassification_job WHERE request_id=$request LIMIT 1").bind(("request",request.request_id.clone()))).await?;
        #[derive(SurrealValue)] struct Existing {job_id:String,request_hash:String}
        if let Some(existing)=jobs::take::<Vec<Existing>>(&mut existing,0)?.into_iter().next() {
            if existing.request_hash!=request_hash {return Err(TxError::Abort("maintenance_request_conflict".into()));}
            return Ok(existing.job_id);
        }
        let mut upper=jobs::run(&tx,tx.query(format!("SELECT run_id,article_id FROM atlas_article WHERE ({}) AND seen_at<=$cutoff ORDER BY run_id DESC,article_id DESC LIMIT 1",scope_where(&request.scope))).bind(("scope_run",scope_run(&request.scope).unwrap_or("").to_owned())).bind(("cutoff",now.to_rfc3339()))).await?;
        let upper=jobs::take::<Vec<Header>>(&mut upper,0)?.into_iter().next().map(Into::into);
        let coverage=admission::coverage_retained_scope_in(&tx,&request.scope,&now.to_rfc3339(),upper.as_ref()).await?;
        let checkpoint=MaintenanceCheckpoint::new(job.clone(),request.scope.clone(),pin.clone(),manifest.clone(),now.to_rfc3339(),upper);
        let report=MaintenanceReport::from_checkpoint(&checkpoint,MaintenanceState::Queued,coverage,Vec::new());
        let checkpoint_json=admission::bounded_json(&checkpoint,16384,"maintenance_checkpoint").map_err(|_|TxError::Abort("maintenance_checkpoint_bound".into()))?;
        let report_json=admission::bounded_json(&report,16384,"maintenance_report").map_err(|_|TxError::Abort("maintenance_report_bound".into()))?;
        let pin_json=admission::bounded_json(&pin,4096,"maintenance_pin").map_err(|_|TxError::Abort("maintenance_pin_bound".into()))?;
        jobs::bump(&tx,"job_guard",&job).await?;
        jobs::create_job_in(&tx,&NewJob {id:&job,kind:"maintenance",operation:OPERATION,parent:None,created_at:now}).await?;
        let task=format!("{job}:activate:0");
        let input_hash=crate::classifier::artifact::sha256_hex(checkpoint_json.as_bytes());
        jobs::create_task_in(&tx,&NewTask {id:&task,job:&job,pool:POOL,operation:OPERATION,priority:PRIORITY,deps:Vec::new(),input_hash:&input_hash,created_at:now}).await?;
        jobs::run(&tx,tx.query("CREATE $row SET request_id=$request,request_hash=$hash,scope_key=$scope,pin_json=$pin,manifest_sha256=$manifest,state='queued',activation_task_id=$activation_task,activation_input_hash=$activation_input,checkpoint_json=$checkpoint,report_json=$report,schema_version='atlas-reclassification-job-v1',updated_at=<datetime>$now RETURN NONE; UPDATE $job SET model=$model,result_ref=$result,cancellable=true,stage_coverage_json=$report RETURN NONE")
            .bind(("row",jobs::rid("atlas_reclassification_job",&job))).bind(("request",request.request_id)).bind(("hash",request_hash)).bind(("scope",request.scope.key())).bind(("pin",pin_json)).bind(("manifest",manifest)).bind(("activation_task",task)).bind(("activation_input",input_hash)).bind(("checkpoint",checkpoint_json)).bind(("report",report_json)).bind(("now",now.to_rfc3339())).bind(("job",jobs::rid("job",&job))).bind(("model",pin.model_id)).bind(("result",format!("atlas_reclassification_job:{job}")))).await?;
        Ok(job)
    }}).await.map_err(|error|match error {MaintenanceError::Admission(admission::AdmissionError::Storage(TxError::Abort(ref reason))) if reason=="maintenance_request_conflict"=>MaintenanceError::RequestConflict,other=>other})
}

#[derive(Clone, Copy)]
pub(crate) enum Control {
    Pause,
    Resume,
    Cancel,
}
pub(crate) async fn control(
    repo: &Repository,
    job: &str,
    control: Control,
) -> Result<(), MaintenanceError> {
    let _ = load(repo, job).await?;
    let job = job.to_owned();
    let now = Utc::now();
    owned(repo, move |tx| {
        let job = job.clone();
        async move { control_in(&tx, &job, control, now).await }
    })
    .await
    .map(|_| ())
}

/// Shared cooperative control transaction, used by typed maintenance calls and
/// the generic Jobs Stop adapter. A live task retains ownership until its
/// worker checkpoints; an expired owner can never be resurrected by cleanup.
async fn control_in(
    tx: &Tx,
    job: &str,
    control: Control,
    now: DateTime<Utc>,
) -> Result<MaintenanceState, TxError> {
    jobs::bump(tx, "job_guard", job).await?;
    let mut response=jobs::run(tx,tx.query(format!("SELECT VALUE id FROM task WHERE job=$job AND state='running' AND lease_until> <datetime>$now LIMIT 1; SELECT {JOB_PROJECTION} FROM ONLY $row"))
            .bind(("job",jobs::rid("job",job))).bind(("row",jobs::rid("atlas_reclassification_job",job))).bind(("now",now.to_rfc3339()))).await?;
    let running = !jobs::take::<Vec<surrealdb::types::RecordId>>(&mut response, 0)?.is_empty();
    let row = jobs::take::<Option<JobRow>>(&mut response, 1)?
        .ok_or_else(|| TxError::Abort("maintenance_missing".into()))?;
    let mut stored =
        decode(row).map_err(|_| TxError::Abort("maintenance_request_corrupt".into()))?;
    if stored.report.state.terminal() {
        return Ok(stored.report.state);
    }
    let (pause, cancel, resume, next, reason) = match control {
        Control::Pause if stored.report.state == MaintenanceState::Blocked => (
            true,
            false,
            false,
            MaintenanceState::Blocked,
            "pause_requested",
        ),
        Control::Pause => (
            true,
            false,
            false,
            if running {
                stored.report.state
            } else {
                MaintenanceState::Paused
            },
            "pause_requested",
        ),
        Control::Cancel => (
            false,
            true,
            false,
            if running {
                stored.report.state
            } else {
                MaintenanceState::Cancelled
            },
            "cancel_requested",
        ),
        Control::Resume if stored.cancel_requested => {
            return Err(TxError::Abort("cancelled_request_cannot_resume".into()))
        }
        Control::Resume if stored.report.state == MaintenanceState::Blocked => (
            false,
            false,
            true,
            MaintenanceState::Blocked,
            "resume_validation_pending",
        ),
        Control::Resume if stored.report.state == MaintenanceState::Paused => (
            false,
            false,
            false,
            MaintenanceState::Queued,
            "explicit_resume",
        ),
        Control::Resume => return Ok(stored.report.state),
    };
    if matches!(control, Control::Resume) && stored.report.state == MaintenanceState::Paused {
        stored.checkpoint.conflict_generation = stored
            .checkpoint
            .conflict_generation
            .checked_add(1)
            .filter(|value| *value <= i64::MAX as u64)
            .ok_or_else(|| TxError::Abort("maintenance_conflict_generation_overflow".into()))?;
        stored.checkpoint.revision = stored
            .checkpoint
            .revision
            .checked_add(1)
            .filter(|value| *value <= i64::MAX as u64)
            .ok_or_else(|| TxError::Abort("maintenance_revision_overflow".into()))?;
        ensure_queued_activation_in(tx, &mut stored, now).await?;
    }
    stored.report = MaintenanceReport::from_checkpoint(
        &stored.checkpoint,
        next,
        stored.report.coverage,
        vec![reason.into()],
    );
    stored.report.reason_codes = vec![reason.into()];
    let report = admission::bounded_json(&stored.report, 16384, "maintenance_report")
        .map_err(|_| TxError::Abort("maintenance_report_bound".into()))?;
    let root_state = if cancel && !running {
        "cancelled"
    } else if pause {
        "paused"
    } else if next == MaintenanceState::Blocked {
        "blocked"
    } else {
        "running"
    };
    jobs::run(tx,tx.query("UPDATE $row SET pause_requested=$pause,cancel_requested=$cancel,resume_requested=$resume,state=$next,activation_task_id=$activation_task,activation_input_hash=$activation_input,checkpoint_json=$checkpoint,report_json=$report,updated_at=<datetime>$now RETURN NONE; UPDATE $job SET state=$root_state,cancel_requested=$cancel,stage_coverage_json=$report,updated_at=<datetime>$now,finished_at=IF $terminal {<datetime>$now} ELSE {NONE},stopped=IF $cancel AND !$running {true} ELSE {(stopped ?? false)} RETURN NONE; IF $cancel AND !$running {LET $tasks=SELECT VALUE id FROM task WHERE job=$job AND (state='queued' OR (state='running' AND lease_until<= <datetime>$now)); LET $attempts=SELECT VALUE id FROM attempt WHERE task INSIDE $tasks AND finished_at=NONE; UPDATE $attempts SET finished_at=<datetime>$now,duration_ms=duration::millis(<datetime>$now-started_at),outcome='cancelled' RETURN NONE; UPDATE $tasks SET state='cancelled',lease_epoch+=1,lease_owner='',process_owner='',lease_until=NONE,finished_at=<datetime>$now,updated_at=<datetime>$now RETURN NONE; UPDATE atlas_reclassification_item SET disposition='retryable',last_error='job_cancelled',owner='',lease_until=NONE,updated_at=<datetime>$now WHERE job_id=$job_key AND disposition='evaluating' RETURN NONE;}; IF !$pause AND !$cancel AND !$resume {UPDATE task SET next_eligible_at=<datetime>$now WHERE job=$job AND state='queued' RETURN NONE;};")
            .bind(("row",jobs::rid("atlas_reclassification_job",job))).bind(("job",jobs::rid("job",job))).bind(("job_key",job.to_owned())).bind(("pause",pause)).bind(("cancel",cancel)).bind(("resume",resume)).bind(("next",next.as_str())).bind(("root_state",root_state)).bind(("terminal",next.terminal())).bind(("running",running)).bind(("activation_task",stored.activation_task_id)).bind(("activation_input",stored.activation_input_hash)).bind(("checkpoint",json(&stored.checkpoint,16384,"maintenance_checkpoint")?)).bind(("report",report)).bind(("now",now.to_rfc3339()))).await?;
    jobs::log_transition_in(
        tx,
        job,
        &format!("{job}:control:{reason}:{}", stored.checkpoint.revision),
        if cancel {
            "atlas.reclassification.cancelled"
        } else if pause {
            "atlas.reclassification.paused"
        } else {
            "atlas.reclassification.activation"
        },
        reason,
        &now.to_rfc3339(),
    )
    .await?;
    Ok(next)
}

pub(crate) async fn cancel_in(
    tx: &Tx,
    job: &str,
    now: DateTime<Utc>,
) -> Result<MaintenanceState, TxError> {
    control_in(tx, job, Control::Cancel, now).await
}

/// One immutable source capture, retained independently of a job's lifetime.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OriginalCapture {
    article: AtlasArticleRow,
    metadata: Option<crate::atlas_metadata::ArticleMetadata>,
    original_metadata_source: String,
    original_text_complete: bool,
    original_taxonomy_version: String,
    original_receipt_key: String,
    source: RetainedSourceFence,
}
#[derive(Clone)]
pub(crate) struct Activation {
    pub task: ClaimedTask,
    pub request_hash: String,
    pub checkpoint: MaintenanceCheckpoint,
    pub fence: MaintenancePublicationFence,
}

pub(crate) async fn claim(
    owner: &AdmissionOwner,
    worker: &str,
    now: DateTime<Utc>,
) -> Result<Option<Activation>, MaintenanceError> {
    let worker = worker.to_owned();
    let op = format!(
        "maintenance-claim:{}:{}",
        worker,
        now.timestamp_nanos_opt().unwrap_or_default()
    );
    let result=admission::owned_tx(owner,RetryPolicy::default(),move|tx| {
        let (worker,op)=(worker.clone(),op.clone());async move {
            jobs::interrupt_expired_pool_in(&tx,POOL,now).await?;
            settle_failed_activations_in(&tx,now).await?;
            let Some(task)=jobs::claim_in(&tx,POOL,None,&worker,LEASE_SECS,now,&op,Some(jobs::claim_shard(&worker))).await? else{return Ok(None)};
            jobs::bump(&tx,"job_guard",&task.root_id).await?;
            let mut response=jobs::run(&tx,tx.query(format!("SELECT {JOB_PROJECTION} FROM ONLY $row"))
                .bind(("row",jobs::rid("atlas_reclassification_job",&task.job_id)))).await?;
            let row=jobs::take::<Option<JobRow>>(&mut response,0)?.ok_or_else(||TxError::Abort("maintenance_request_missing".into()))?;
            let mut stored=decode(row).map_err(|_|TxError::Abort("maintenance_request_corrupt".into()))?;
            if stored.cancel_requested || stored.pause_requested || !matches!(stored.report.state,MaintenanceState::Queued|MaintenanceState::Reconciling|MaintenanceState::Running) {
                return Err(TxError::Abort("maintenance_request_ineligible".into()));
            }
            if task.id!=stored.activation_task_id || task.input_hash!=stored.activation_input_hash {
                return Err(TxError::Abort("maintenance_checkpoint_task_mismatch".into()));
            }
            stored.report.state=MaintenanceState::Running;
            stored.report.reason_codes.clear();
            let report=admission::bounded_json(&stored.report,16384,"maintenance_report").map_err(|_|TxError::Abort("maintenance_report_bound".into()))?;
            jobs::run(&tx,tx.query("UPDATE $row SET state='running',report_json=$report,updated_at=<datetime>$now RETURN NONE; UPDATE $job SET state='running',worker_owner=$owner,heartbeat_at=<datetime>$now,stage_coverage_json=$report RETURN NONE")
                .bind(("row",jobs::rid("atlas_reclassification_job",&task.job_id))).bind(("job",jobs::rid("job",&task.job_id))).bind(("owner",worker)).bind(("now",now.to_rfc3339())).bind(("report",report))).await?;
            let fence=MaintenancePublicationFence::new(task.clone(),stored.request_hash.clone(),stored.checkpoint.scope.clone(),stored.checkpoint.pinned_snapshot.clone(),stored.checkpoint.manifest_sha256.clone()).map_err(|_|TxError::Abort("maintenance_fence_invalid".into()))?;
            Ok(Some(Activation {task,request_hash:stored.request_hash,checkpoint:stored.checkpoint,fence}))
        }
    }).await?;
    Ok(result.value)
}

async fn settle_failed_activations_in(tx: &Tx, now: DateTime<Utc>) -> Result<(), TxError> {
    let mut response = jobs::run(tx, tx.query(format!(
        "SELECT {JOB_PROJECTION} FROM atlas_reclassification_job WHERE state INSIDE ['queued','running','reconciling'] AND (SELECT VALUE state FROM ONLY type::record('task',$parent.activation_task_id))='failed' LIMIT 64;"
    ))).await?;
    for row in jobs::take::<Vec<JobRow>>(&mut response, 0)? {
        let mut stored =
            decode(row).map_err(|_| TxError::Abort("maintenance_request_corrupt".into()))?;
        stored.report.state = MaintenanceState::Failed;
        stored.report.reason_codes = vec!["worker_recovery_exhausted".into()];
        let report = json(&stored.report, 16384, "maintenance_report")?;
        jobs::run(tx,tx.query("UPDATE $row SET state='failed',report_json=$report,updated_at=<datetime>$now RETURN NONE; UPDATE $job SET state='failed',stage_coverage_json=$report,worker_owner='',finished_at=<datetime>$now,updated_at=<datetime>$now RETURN NONE;")
            .bind(("row",jobs::rid("atlas_reclassification_job",&stored.checkpoint.job_id)))
            .bind(("job",jobs::rid("job",&stored.checkpoint.job_id)))
            .bind(("report",report)).bind(("now",now.to_rfc3339()))).await?;
    }
    Ok(())
}

/// Poll controls and foreground intent without changing evidence. Heartbeat
/// leases cover both the task and its still-owned evaluating items.
pub(crate) async fn heartbeat(
    owner: &AdmissionOwner,
    activation: &Activation,
    renew: bool,
    now: DateTime<Utc>,
) -> Result<bool, MaintenanceError> {
    if !renew {
        // Control polling must not write the publication guard. A read every
        // second otherwise conflicts with this same owner's source audits.
        let mut response = owner.query("RETURN count(SELECT id FROM task WHERE id=$task AND state='running' AND lease_owner=$owner AND lease_epoch=$epoch AND lease_until> <datetime>$now LIMIT 1)=1 AND count(SELECT id FROM atlas_reclassification_job WHERE id=$job AND request_hash=$request AND !cancel_requested AND !pause_requested AND state INSIDE ['running','reconciling'] LIMIT 1)=1;")
            .bind(("task", jobs::rid("task", &activation.task.id)))
            .bind(("owner", activation.task.lease_owner.clone()))
            .bind(("epoch", activation.task.lease_epoch))
            .bind(("now", now.to_rfc3339()))
            .bind(("job", jobs::rid("atlas_reclassification_job", &activation.task.job_id)))
            .bind(("request", activation.request_hash.clone()))
            .await.map_err(TxError::from)?.check().map_err(TxError::from)?;
        return Ok(response.take::<Option<bool>>(0).map_err(TxError::from)? == Some(true));
    }
    let captured = activation.clone();
    Ok(admission::owned_tx(owner,RetryPolicy::default(),move|tx| {let activation=captured.clone();async move {
        if !activation.fence.owns_in(&tx,now).await? {return Ok(false);}
        let mut flags=jobs::run(&tx,tx.query("RETURN count(SELECT id FROM atlas_reclassification_job WHERE id=$row AND !cancel_requested AND !pause_requested AND state IN ['running','reconciling'] LIMIT 1)=1;")
            .bind(("row",jobs::rid("atlas_reclassification_job",&activation.task.job_id)))).await?;
        let allowed=jobs::take::<Option<bool>>(&mut flags,0)?==Some(true);
        if renew {
            jobs::renew_owned_in(&tx,&activation.task,LEASE_SECS,now).await?;
            jobs::run(&tx,tx.query("UPDATE atlas_reclassification_item SET lease_until=<datetime>$until,updated_at=<datetime>$now WHERE job_id=$job AND disposition='evaluating' AND owner=$owner AND epoch=$epoch RETURN NONE")
                .bind(("until",(now+chrono::Duration::seconds(LEASE_SECS)).to_rfc3339())).bind(("now",now.to_rfc3339())).bind(("job",activation.task.job_id)).bind(("owner",activation.task.lease_owner)).bind(("epoch",activation.task.lease_epoch))).await?;
        }
        Ok(allowed)
    }}).await?.value)
}

pub(crate) struct PreparedPage {
    pub inputs: Vec<ClaimedInput>,
    pub sources: BTreeMap<OccurrenceKey, RetainedSourceFence>,
    pub checkpoint: MaintenanceCheckpoint,
    pub examined: usize,
    pub accounting_items: Vec<String>,
}
#[derive(SurrealValue)]
struct RetryHeader {
    run_id: String,
    article_id: String,
}

pub(crate) async fn prepare_page(
    owner: &AdmissionOwner,
    activation: &Activation,
    limit: usize,
    now: DateTime<Utc>,
) -> Result<PreparedPage, MaintenanceError> {
    if limit == 0 || limit > 64 {
        return Err(MaintenanceError::InvalidRequest {
            code: "maintenance_page_budget".into(),
        });
    }
    let captured = activation.clone();
    Ok(admission::owned_tx(owner,RetryPolicy::default(),move|tx| {let activation=captured.clone();async move {
        if !activation.fence.check_in(&tx,now).await? {return Err(TxError::Abort("maintenance_lease_lost".into()));}
        let mut checkpoint=activation.checkpoint.clone();
        // Retry rows consume the SAME64 occurrence budget. Reserve room for
        // later source pages so a failing first row cannot starve the corpus.
        let retry_limit=if checkpoint.scan_exhausted {limit}else{16.min(limit)};
        let mut response=jobs::run(&tx,tx.query("SELECT run_id,article_id FROM atlas_reclassification_item WHERE job_id=$job AND (source_retry_pending=true OR disposition IN ['retryable','conflicted','pending'] OR (disposition='evaluating' AND lease_until<= <datetime>$now)) ORDER BY run_id,article_id LIMIT $limit")
            .bind(("job",activation.task.job_id.clone())).bind(("now",now.to_rfc3339())).bind(("limit",retry_limit as i64))).await?;
        let retries=jobs::take::<Vec<RetryHeader>>(&mut response,0)?;
        let mut keys:Vec<_>=retries.into_iter().map(|header|OccurrenceKey::new(header.run_id,header.article_id)).collect();
        keys.sort();keys.dedup();
        let mut scanned: Vec<OccurrenceKey> = Vec::new();
        if !checkpoint.scan_exhausted && keys.len()<limit {
            if let Some(upper)=&checkpoint.scan_upper {
                let after=checkpoint.scan_after.clone().unwrap_or_else(||OccurrenceKey::new("",""));
                let mut response=jobs::run(&tx,tx.query(format!("SELECT run_id,article_id FROM atlas_article WHERE ({}) AND seen_at<=$cutoff AND (run_id<$upper_run OR (run_id=$upper_run AND article_id<=$upper_article)) AND ($first OR run_id>$after_run OR (run_id=$after_run AND article_id>$after_article)) ORDER BY run_id,article_id LIMIT $limit",scope_where(&checkpoint.scope)))
                    .bind(("scope_run",scope_run(&checkpoint.scope).unwrap_or("").to_owned())).bind(("cutoff",checkpoint.cutoff_at.clone())).bind(("upper_run",upper.run_id.clone())).bind(("upper_article",upper.article_id.clone())).bind(("first",checkpoint.scan_after.is_none())).bind(("after_run",after.run_id)).bind(("after_article",after.article_id)).bind(("limit",(limit-keys.len()) as i64))).await?;
                scanned=jobs::take::<Vec<Header>>(&mut response,0)?.into_iter().map(Into::into).collect();
                if let Some(last)=scanned.last() {checkpoint.scan_after=Some(last.clone());}
                checkpoint.scan_exhausted=scanned.len()<limit-keys.len() || checkpoint.scan_after.as_ref()==checkpoint.scan_upper.as_ref();
            } else {checkpoint.scan_exhausted=true;}
        }
        let examined=keys.len()+scanned.len();
        keys.extend(scanned);keys.sort();keys.dedup();
        let originals=super::atlas::article_originals_many_in(&tx,&keys).await?;
        let metadata=super::atlas_metadata::metadata_originals_many_in(&tx,&keys).await?;
        let articles:Vec<_>=originals.values().filter_map(|original|match original {super::atlas::OriginalArticle::Present(row)=>Some((**row).clone()),super::atlas::OriginalArticle::Oversized=>None}).collect();
        let sources=admission::retained_source_fences_many_in(&tx,&articles).await?;
        let views=admission::classification_views_many_in(&tx,&keys).await.map_err(|_|TxError::Abort("maintenance_current_receipt_invalid".into()))?;
        let already:std::collections::BTreeSet<_>=sources.iter().filter_map(|(key,source)| {
            views.get(key).and_then(|view|view.receipt.as_ref()).filter(|receipt|
                receipt.key.model_id==activation.checkpoint.pinned_snapshot.model_id &&
                receipt.key.artifact_sha256==activation.checkpoint.pinned_snapshot.artifact_sha256 &&
                receipt.key.manifest_sha256==activation.checkpoint.manifest_sha256 &&
                receipt.key.input_revision==source.native_input_revision &&
                receipt.key.classifier_input_hash==source.classifier_input_hash && receipt.key.source_revision==source.source_hash && receipt.provenance==source.provenance
            ).map(|_|key.clone())
        }).collect();
        let reservations=admission::reserve_retained_revisions_in(&tx,&activation.fence,&sources.iter().filter(|(key,_)|!already.contains(*key)).map(|(_,source)|source.clone()).collect::<Vec<_>>(),now).await?;
        let mut inputs=Vec::new();
        let mut used_sources=BTreeMap::new();
        let mut actions=Vec::new();
        for key in keys {
            let Some(super::atlas::OriginalArticle::Present(article))=originals.get(&key) else {
                actions.push(ItemAction::Skip {key:key.clone(),reason:"original_row_missing_or_oversized"});
                continue;
            };
            let original_metadata=match metadata.get(&key) {
                Some(super::atlas_metadata::OriginalMetadata::Present(value))=>Some((**value).clone()),
                Some(super::atlas_metadata::OriginalMetadata::Oversized|super::atlas_metadata::OriginalMetadata::Invalid)=>{
                    actions.push(ItemAction::Skip {key:key.clone(),reason:"original_metadata_invalid_or_oversized"});continue;
                },
                _=>None,
            };
            let Some(source)=sources.get(&key) else {actions.push(ItemAction::Conflict {key:key.clone(),source_hash:crate::atlas_metadata::input_hash(article),reason:"source_pending"});continue;};
            if already.contains(&key) {
                let receipt=views[&key].receipt.as_ref().ok_or_else(||TxError::Abort("already_current_receipt_missing".into()))?;
                actions.push(ItemAction::Already {source:Box::new(source.clone()),receipt:Box::new(receipt.clone())});
                continue;
            }
            let Some(revision)=reservations.get(&key) else {actions.push(ItemAction::Conflict {key:key.clone(),source_hash:source.source_hash.clone(),reason:"source_changed"});continue;};
            if article.title.trim().is_empty() && article.description.trim().is_empty() && views.get(&key).is_none_or(|view|view.legacy) {
                actions.push(ItemAction::Skip {key:key.clone(),reason:"original_text_unavailable"});continue;
            }
            let claim=AdmissionClaim {occurrence:key.clone(),owner:activation.task.lease_owner.clone(),epoch:u64::try_from(activation.task.lease_epoch).map_err(|_|TxError::Abort("negative_maintenance_epoch".into()))?,input_revision:*revision,source_revision:source.source_hash.clone(),classifier_input_hash:source.classifier_input_hash.clone(),work_seq:0};
            actions.push(ItemAction::Evaluate {index:inputs.len()});
            used_sources.insert(key,source.clone());
            inputs.push(ClaimedInput {claim,article:(**article).clone(),metadata:original_metadata,body_revision:None,provenance:source.provenance.clone()});
        }
        let written=write_items_in(&tx,&activation,&actions,&inputs,&used_sources,&views,now).await?;
        increase(&mut checkpoint.rows_scanned,written.created)?;
        account_in(&tx,&activation,&mut checkpoint,&written.items).await?;
        // Persist scan progress and item snapshots BEFORE any JS dispatch.
        // Counts are derived from the durable item ledger at activation close.
        checkpoint.revision=checkpoint.revision.checked_add(1).ok_or_else(||TxError::Abort("maintenance_revision_overflow".into()))?;
        save_checkpoint_in(&tx,&activation,&checkpoint,MaintenanceState::Running,&[],now).await?;
        Ok(PreparedPage {inputs,sources:used_sources,checkpoint,examined,accounting_items:written.items})
    }}).await?.value)
}

fn json<T: Serialize>(value: &T, limit: usize, kind: &str) -> Result<String, TxError> {
    admission::bounded_json(value, limit, kind).map_err(|_| TxError::Abort(format!("{kind}_bound")))
}

#[derive(Clone)]
enum ItemAction {
    Skip {
        key: OccurrenceKey,
        reason: &'static str,
    },
    Conflict {
        key: OccurrenceKey,
        source_hash: String,
        reason: &'static str,
    },
    Already {
        source: Box<RetainedSourceFence>,
        receipt: Box<crate::atlas_classify::StoredReceipt>,
    },
    Evaluate {
        index: usize,
    },
}

struct WrittenItems {
    created: u64,
    items: Vec<String>,
}
fn action_key(
    action: &ItemAction,
    job: &str,
    inputs: &[ClaimedInput],
    sources: &BTreeMap<OccurrenceKey, RetainedSourceFence>,
) -> String {
    match action {
        ItemAction::Skip { key, .. } => {
            super::codec::encode(&[job, &key.run_id, &key.article_id, "unavailable-original"])
        }
        ItemAction::Conflict {
            key, source_hash, ..
        } => super::codec::encode(&[
            job,
            &key.run_id,
            &key.article_id,
            &format!("unreserved:{source_hash}"),
        ]),
        ItemAction::Already { source, .. } => admission::maintenance_item_key(job, source),
        ItemAction::Evaluate { index } => {
            admission::maintenance_item_key(job, &sources[&inputs[*index].claim.occurrence])
        }
    }
}

/// Bounded driver groups avoid per-occurrence round trips and keep the largest
/// original row+metadata+capture bind set below16MiB. No source text is logged.
async fn write_items_in(
    tx: &Tx,
    activation: &Activation,
    actions: &[ItemAction],
    inputs: &[ClaimedInput],
    sources: &BTreeMap<OccurrenceKey, RetainedSourceFence>,
    views: &BTreeMap<OccurrenceKey, admission::ClassificationView>,
    now: DateTime<Utc>,
) -> Result<WrittenItems, TxError> {
    if actions.len() > 64 {
        return Err(TxError::Abort("maintenance_item_batch_limit".into()));
    }
    #[derive(SurrealValue)]
    struct Origin {
        key: String,
        capture_json: Option<String>,
        capture_sha256: String,
    }
    let mut written = WrittenItems {
        created: 0,
        items: Vec::new(),
    };
    for group in actions.chunks(16) {
        let item_keys: Vec<_> = group
            .iter()
            .map(|action| action_key(action, &activation.task.job_id, inputs, sources))
            .collect();
        let origin_keys: Vec<_> = group
            .iter()
            .filter_map(|action| match action {
                ItemAction::Evaluate { index } => {
                    let input = &inputs[*index];
                    let source = &sources[&input.claim.occurrence];
                    Some(super::codec::encode(&[
                        &source.occurrence.run_id,
                        &source.occurrence.article_id,
                        &source.capture_revision(),
                    ]))
                }
                _ => None,
            })
            .collect();
        let mut originals = BTreeMap::new();
        let mut response=jobs::run(tx,tx.query("SELECT record::id(id) AS key,IF bytes::len(type::bytes(capture_json))<=262144 {capture_json} ELSE {NONE} AS capture_json,capture_sha256 FROM $origins; SELECT VALUE record::id(id) FROM $items;")
            .bind(("origins",origin_keys.iter().map(|key|jobs::rid("atlas_reclassification_origin",key)).collect::<Vec<_>>())).bind(("items",item_keys.iter().map(|key|jobs::rid("atlas_reclassification_item",key)).collect::<Vec<_>>()))).await?;
        let existing: std::collections::BTreeSet<String> =
            jobs::take::<Vec<String>>(&mut response, 1)?
                .into_iter()
                .collect();
        increase(
            &mut written.created,
            item_keys
                .iter()
                .filter(|key| !existing.contains(*key))
                .count() as u64,
        )?;
        written.items.extend(item_keys);
        for origin in jobs::take::<Vec<Origin>>(&mut response, 0)? {
            let raw = origin
                .capture_json
                .ok_or_else(|| TxError::Abort("original_capture_oversized".into()))?;
            if crate::classifier::artifact::sha256_hex(raw.as_bytes()) != origin.capture_sha256 {
                return Err(TxError::Abort("original_capture_checksum".into()));
            }
            let capture: OriginalCapture = serde_json::from_str(&raw)
                .map_err(|_| TxError::Abort("original_capture_invalid".into()))?;
            if originals.insert(origin.key, capture).is_some() {
                return Err(TxError::Abort("duplicate_original_capture".into()));
            }
        }
        let mut values = Vec::new();
        for action in group {
            let value = match action {
                ItemAction::Skip { key, reason } => {
                    serde_json::json!({"kind":"skip","item":super::codec::encode(&[&activation.task.job_id,&key.run_id,&key.article_id,"unavailable-original"]),"run":key.run_id,"article":key.article_id,"reason":reason})
                }
                ItemAction::Conflict {
                    key,
                    source_hash,
                    reason,
                } => {
                    serde_json::json!({"kind":"conflict","item":super::codec::encode(&[&activation.task.job_id,&key.run_id,&key.article_id,&format!("unreserved:{source_hash}")]),"run":key.run_id,"article":key.article_id,"source":source_hash,"reason":reason})
                }
                ItemAction::Already { source, receipt } => {
                    serde_json::json!({"kind":"already","item":admission::maintenance_item_key(&activation.task.job_id,source),"run":source.occurrence.run_id,"article":source.occurrence.article_id,"source":source.source_hash,"receipt":receipt.key.record_id(),"input":receipt.key.input_revision,"native":source.native_input_revision,"provenance":json(&source.provenance,16384,"original_provenance")?,"fence":json(source,32768,"retained_source_fence")?})
                }
                ItemAction::Evaluate { index } => {
                    let input = &inputs[*index];
                    let source = &sources[&input.claim.occurrence];
                    let origin = super::codec::encode(&[
                        &source.occurrence.run_id,
                        &source.occurrence.article_id,
                        &source.capture_revision(),
                    ]);
                    let captured = originals
                        .remove(&origin)
                        .unwrap_or_else(|| OriginalCapture {
                            article: input.article.clone(),
                            metadata: input.metadata.clone(),
                            original_metadata_source: "retained_occurrence".into(),
                            original_text_complete: false,
                            original_taxonomy_version: views
                                .get(&source.occurrence)
                                .map(|view| view.taxonomy_version.clone())
                                .unwrap_or_else(|| {
                                    crate::atlas_taxonomy::LEGACY_TAXONOMY_VERSION.into()
                                }),
                            original_receipt_key: views
                                .get(&source.occurrence)
                                .and_then(|view| view.receipt.as_ref())
                                .map(|receipt| receipt.key.record_id())
                                .unwrap_or_default(),
                            source: source.clone(),
                        });
                    if captured.source != *source
                        || captured.article.title != input.article.title
                        || captured.article.description != input.article.description
                    {
                        return Err(TxError::Abort("original_capture_identity_conflict".into()));
                    }
                    let capture = json(&captured, 262144, "original_capture")?;
                    serde_json::json!({"kind":"evaluate","item":admission::maintenance_item_key(&activation.task.job_id,source),"run":source.occurrence.run_id,"article":source.occurrence.article_id,"source":source.source_hash,"origin":origin,"capture_revision":source.capture_revision(),"capture_hash":crate::classifier::artifact::sha256_hex(capture.as_bytes()),"capture":capture,"row":json(&captured.article,65536,"original_row")?,"metadata":captured.metadata.as_ref().map(|metadata|json(metadata,131072,"original_metadata")).transpose()?.unwrap_or_default(),"native":source.native_input_revision,"provenance":json(&source.provenance,16384,"original_provenance")?,"fence":json(source,32768,"retained_source_fence")?,"input":input.claim.input_revision})
                }
            };
            values.push(value);
        }
        jobs::run(tx,tx.query("FOR $value IN $values {
            LET $item=type::record('atlas_reclassification_item',$value.item);
            IF $value.kind='skip' {
                IF !record::exists($item) {CREATE $item SET job_id=$job,run_id=$value.run,article_id=$value.article,source_revision='unavailable-original',origin_key='',disposition='skipped_unmigrated',derivative_state='policy_ineligible',last_error=$value.reason,updated_at=<datetime>$now RETURN NONE;};
            } ELSE IF $value.kind='conflict' {
                UPSERT $item SET job_id=$job,run_id=$value.run,article_id=$value.article,source_revision=$value.source,origin_key='',disposition='retryable',derivative_state='pending',last_error=$value.reason,stats_dirty=true,owner='',lease_until=NONE,updated_at=<datetime>$now RETURN NONE;
            } ELSE IF $value.kind='already' {
                IF !record::exists($item) {CREATE $item SET job_id=$job,run_id=$value.run,article_id=$value.article,source_revision=$value.source,origin_key='',disposition='already_current',receipt_key=$value.receipt,input_revision=$value.input,original_input_revision=$value.native,original_provenance_json=$value.provenance,source_fence_json=$value.fence,derivative_state='pending',updated_at=<datetime>$now RETURN NONE;};
            } ELSE {
                LET $origin=type::record('atlas_reclassification_origin',$value.origin);
                IF !record::exists($origin) {CREATE $origin SET run_id=$value.run,article_id=$value.article,source_revision=$value.capture_revision,capture_json=$value.capture,capture_sha256=$value.capture_hash,created_at=<datetime>$now RETURN NONE;};
                UPSERT $item SET job_id=$job,run_id=$value.run,article_id=$value.article,source_revision=$value.source,origin_key=$value.origin,original_row_json=$value.row,original_metadata_json=$value.metadata,original_input_revision=$value.native,original_provenance_json=$value.provenance,source_fence_json=$value.fence,disposition='evaluating',evaluation_generation=$generation,source_retry_pending=false,stats_dirty=true,owner=$owner,epoch=$epoch,lease_until=<datetime>$until,input_revision=$value.input,updated_at=<datetime>$now RETURN NONE;
                UPDATE atlas_reclassification_item SET disposition=IF disposition INSIDE ['admitted','rejected','already_current'] {disposition} ELSE {'superseded_source'},derivative_state='superseded_input',source_retry_pending=false,stats_dirty=true,owner='',lease_until=NONE,updated_at=<datetime>$now WHERE job_id=$job AND run_id=$value.run AND article_id=$value.article AND id!=$item AND (disposition IN ['conflicted','retryable','pending'] OR source_retry_pending=true) RETURN NONE;
            };
        };")
            .bind(("values",values)).bind(("generation",i64::try_from(activation.checkpoint.conflict_generation).map_err(|_|TxError::Abort("conflict_generation_overflow".into()))?)).bind(("job",activation.task.job_id.clone())).bind(("owner",activation.task.lease_owner.clone())).bind(("epoch",activation.task.lease_epoch)).bind(("until",(now+chrono::Duration::seconds(LEASE_SECS)).to_rfc3339())).bind(("now",now.to_rfc3339()))).await?;
    }
    Ok(written)
}

async fn save_checkpoint_in(
    tx: &Tx,
    activation: &Activation,
    checkpoint: &MaintenanceCheckpoint,
    state: MaintenanceState,
    reasons: &[String],
    now: DateTime<Utc>,
) -> Result<(), TxError> {
    if !activation.fence.owns_in(tx, now).await? {
        return Err(TxError::Abort("maintenance_lease_lost".into()));
    }
    checkpoint
        .validate()
        .map_err(|_| TxError::Abort("maintenance_checkpoint_invalid".into()))?;
    let mut reply = jobs::run(
        tx,
        tx.query(format!("SELECT {JOB_PROJECTION} FROM ONLY $row"))
            .bind((
                "row",
                jobs::rid("atlas_reclassification_job", &activation.task.job_id),
            )),
    )
    .await?;
    let stored = decode(
        jobs::take::<Option<JobRow>>(&mut reply, 0)?
            .ok_or_else(|| TxError::Abort("maintenance_request_missing".into()))?,
    )
    .map_err(|_| TxError::Abort("maintenance_request_corrupt".into()))?;
    if stored.request_hash != activation.request_hash
        || stored.checkpoint.revision > checkpoint.revision
    {
        return Err(TxError::Abort("maintenance_checkpoint_stale".into()));
    }
    let report = MaintenanceReport::from_checkpoint(
        checkpoint,
        state,
        stored.report.coverage,
        reasons.to_vec(),
    );
    let checkpoint_json = json(checkpoint, 16384, "maintenance_checkpoint")?;
    let report_json = json(&report, 16384, "maintenance_report")?;
    jobs::run(tx,tx.query("UPDATE $row SET state=$state,checkpoint_json=$checkpoint,report_json=$report,updated_at=<datetime>$now RETURN NONE; UPDATE $job SET stage_coverage_json=$report,progress_done=$done,updated_at=<datetime>$now RETURN NONE")
        .bind(("row",jobs::rid("atlas_reclassification_job",&activation.task.job_id))).bind(("job",jobs::rid("job",&activation.task.job_id))).bind(("state",state.as_str())).bind(("checkpoint",checkpoint_json)).bind(("report",report_json)).bind(("now",now.to_rfc3339())).bind(("done",i64::try_from(checkpoint.rows_scanned).map_err(|_|TxError::Abort("maintenance_progress_overflow".into()))?))).await?;
    Ok(())
}

#[derive(SurrealValue)]
struct AccountRow {
    key: String,
    disposition: String,
    conflict_attempts: i64,
    accounted_conflict_attempts: i64,
    evaluated_counted: bool,
    skipped_counted: bool,
    already_counted: bool,
    conflicted_counted: bool,
    derivative_open_counted: bool,
    derivative_state: String,
}
fn increase(value: &mut u64, delta: u64) -> Result<(), TxError> {
    *value = value
        .checked_add(delta)
        .filter(|value| *value <= i64::MAX as u64)
        .ok_or_else(|| TxError::Abort("maintenance_counter_overflow".into()))?;
    Ok(())
}
async fn account_in(
    tx: &Tx,
    activation: &Activation,
    checkpoint: &mut MaintenanceCheckpoint,
    items: &[String],
) -> Result<(), TxError> {
    if items.len() > 64 {
        return Err(TxError::Abort("maintenance_account_batch_limit".into()));
    }
    if items.is_empty() {
        return Ok(());
    }
    account_source_conflicts_in(tx, activation, checkpoint, items).await?;
    let mut result=jobs::run(tx,tx.query("SELECT record::id(id) AS key,disposition,conflict_attempts,accounted_conflict_attempts,evaluated_counted,skipped_counted,already_counted,conflicted_counted,derivative_open_counted,derivative_state FROM $items WHERE job_id=$job")
        .bind(("items",items.iter().map(|id|jobs::rid("atlas_reclassification_item",id)).collect::<Vec<_>>())).bind(("job",activation.task.job_id.clone()))).await?;
    let rows = jobs::take::<Vec<AccountRow>>(&mut result, 0)?;
    let mut updates = Vec::new();
    for row in rows {
        let evaluated = matches!(row.disposition.as_str(), "admitted" | "rejected");
        let skipped = row.disposition == "skipped_unmigrated";
        let already = row.disposition == "already_current";
        let conflicted = row.conflict_attempts > 0 || row.disposition == "conflicted";
        let conflict = u64::try_from(row.conflict_attempts)
            .map_err(|_| TxError::Abort("negative_conflict_count".into()))?;
        let accounted = u64::try_from(row.accounted_conflict_attempts)
            .map_err(|_| TxError::Abort("negative_accounted_conflict_count".into()))?;
        if conflict < accounted {
            return Err(TxError::Abort("maintenance_counter_regressed".into()));
        }
        if evaluated && !row.evaluated_counted {
            increase(&mut checkpoint.rows_reclassified, 1)?;
            if row.disposition == "rejected" {
                increase(&mut checkpoint.rows_rejected, 1)?;
            }
        }
        if skipped && !row.skipped_counted {
            increase(&mut checkpoint.rows_skipped_unmigrated, 1)?;
        }
        if already && !row.already_counted {
            increase(&mut checkpoint.rows_already_current, 1)?;
        }
        if conflicted && !row.conflicted_counted {
            increase(&mut checkpoint.rows_conflicted, 1)?;
        }
        increase(&mut checkpoint.conflict_attempts, conflict - accounted)?;
        let open = (evaluated || already)
            && !matches!(
                row.derivative_state.as_str(),
                "current" | "policy_ineligible" | "disabled" | "superseded_input"
            );
        if open && !row.derivative_open_counted {
            increase(&mut checkpoint.derivatives_pending, 1)?;
        }
        if !open && row.derivative_open_counted {
            checkpoint.derivatives_pending = checkpoint
                .derivatives_pending
                .checked_sub(1)
                .ok_or_else(|| TxError::Abort("maintenance_derivative_count_underflow".into()))?;
        }
        updates.push(serde_json::json!({"key":row.key,"evaluated":row.evaluated_counted || evaluated,"skipped":row.skipped_counted || skipped,"already":row.already_counted || already,"conflicted":row.conflicted_counted || conflicted,"conflicts":conflict,"open":open,"dirty":matches!(row.disposition.as_str(),"evaluating"|"pending")}));
    }
    jobs::run(tx,tx.query("FOR $item IN $items {UPDATE type::record('atlas_reclassification_item',$item.key) SET evaluated_counted=$item.evaluated,skipped_counted=$item.skipped,already_counted=$item.already,conflicted_counted=$item.conflicted,accounted_conflict_attempts=$item.conflicts,derivative_open_counted=$item.open,stats_dirty=$item.dirty RETURN NONE;};")
        .bind(("items",updates))).await?;
    Ok(())
}

/// A bounded crash/reply-loss reconciliation queue. It does not infer completion
/// from an empty dispatcher result or replay counts from a whole-history scan.
pub(crate) async fn dirty_items(
    owner: &AdmissionOwner,
    activation: &Activation,
    limit: usize,
    now: DateTime<Utc>,
) -> Result<Vec<String>, MaintenanceError> {
    if limit > 64 {
        return Err(MaintenanceError::InvalidRequest {
            code: "dirty_item_budget".into(),
        });
    }
    let captured = activation.clone();
    Ok(admission::owned_tx(owner,RetryPolicy::default(),move|tx| {let activation=captured.clone();async move {
        if !activation.fence.owns_in(&tx,now).await? {return Err(TxError::Abort("maintenance_lease_lost".into()));}
        let mut result=jobs::run(&tx,tx.query("SELECT VALUE record::id(id) FROM atlas_reclassification_item WHERE job_id=$job AND stats_dirty=true AND disposition NOT IN ['evaluating','pending'] ORDER BY run_id,article_id LIMIT $limit")
            .bind(("job",activation.task.job_id)).bind(("limit",limit as i64))).await?;
        jobs::take(&mut result,0)
    }}).await?.value)
}

/// The shared publisher already persisted each outcome. Accounting uses the
/// immutable item IDs, including after reply loss; it never counts an epoch as
/// another logical source change.
pub(crate) fn committed_item_ids(
    activation: &Activation,
    sources: &BTreeMap<OccurrenceKey, RetainedSourceFence>,
    results: &admission::BatchCommit,
) -> Result<Vec<String>, MaintenanceError> {
    if results.items.len() != sources.len() {
        return Err(MaintenanceError::InvalidRequest {
            code: "maintenance_outcome_count".into(),
        });
    }
    Ok(sources
        .values()
        .map(|source| admission::maintenance_item_key(&activation.task.job_id, source))
        .collect())
}

pub(crate) async fn requested_resume(
    repo: &Repository,
) -> Result<Option<StoredJob>, MaintenanceError> {
    let mut result=admission::db_query(repo.db(),format!("SELECT {JOB_PROJECTION} FROM atlas_reclassification_job WHERE state='blocked' AND resume_requested=true ORDER BY updated_at LIMIT 1"))
        .await.map_err(TxError::from)?.check().map_err(TxError::from)?;
    let rows: Vec<JobRow> = result.take(0).map_err(TxError::from)?;
    rows.into_iter().next().map(decode).transpose()
}

/// Consumes one explicit resume intent only after the injected host checked the
/// exact saved pin+manifest. Missing/corrupt pins remain blocked, so restoring
/// files later never silently restarts work without another user request.
pub(crate) async fn settle_resume(
    repo: &Repository,
    stored: &StoredJob,
    context: Option<&AdmissionContext>,
    reason: &str,
    now: DateTime<Utc>,
) -> Result<(), MaintenanceError> {
    if !matches!(
        reason,
        "pin_verified" | "pinned_model_missing" | "pinned_model_incompatible"
    ) {
        return Err(MaintenanceError::InvalidRequest {
            code: "invalid_resume_disposition".into(),
        });
    }
    let expected_pin = stored.checkpoint.pinned_snapshot.clone();
    let expected_manifest = stored.checkpoint.manifest_sha256.clone();
    if context.is_some_and(|context| {
        context.pin.snapshot != expected_pin || context.manifest_sha256 != expected_manifest
    }) {
        return Err(MaintenanceError::RequestConflict);
    }
    let captured = (
        stored.checkpoint.job_id.clone(),
        stored.request_hash.clone(),
        context.is_some(),
        reason.to_owned(),
    );
    owned(repo,move|tx| {let (job,request_hash,valid,reason)=captured.clone();async move {
        jobs::bump(&tx,"job_guard",&job).await?;
        let mut result=jobs::run(&tx,tx.query(format!("SELECT {JOB_PROJECTION} FROM ONLY $row"))
            .bind(("row",jobs::rid("atlas_reclassification_job",&job)))).await?;
        let mut latest=decode(jobs::take::<Option<JobRow>>(&mut result,0)?.ok_or_else(||TxError::Abort("maintenance_request_missing".into()))?).map_err(|_|TxError::Abort("maintenance_request_corrupt".into()))?;
        if latest.request_hash!=request_hash {return Err(TxError::Abort("maintenance_resume_identity_changed".into()));}
        if latest.report.state!=MaintenanceState::Blocked || !latest.resume_requested || latest.cancel_requested {return Ok(());}
        let next_state=if valid {MaintenanceState::Queued}else{MaintenanceState::Blocked};
        if valid {
            latest.checkpoint.conflict_generation=latest.checkpoint.conflict_generation.checked_add(1).filter(|value|*value<=i64::MAX as u64).ok_or_else(||TxError::Abort("maintenance_conflict_generation_overflow".into()))?;
            latest.checkpoint.activation_seq=latest.checkpoint.activation_seq.checked_add(1).filter(|value|*value<=i64::MAX as u64).ok_or_else(||TxError::Abort("maintenance_activation_overflow".into()))?;
            latest.checkpoint.revision=latest.checkpoint.revision.checked_add(1).filter(|value|*value<=i64::MAX as u64).ok_or_else(||TxError::Abort("maintenance_revision_overflow".into()))?;
            let id=format!("{job}:activate:{}",latest.checkpoint.activation_seq);
            let input=crate::classifier::artifact::sha256_hex(json(&latest.checkpoint,16384,"maintenance_checkpoint")?.as_bytes());
            jobs::create_task_in(&tx,&NewTask {id:&id,job:&job,pool:POOL,operation:OPERATION,priority:PRIORITY,deps:vec![latest.activation_task_id.as_str()],input_hash:&input,created_at:now}).await?;
            latest.activation_task_id=id;
            latest.activation_input_hash=input;
        }
        latest.report=MaintenanceReport::from_checkpoint(&latest.checkpoint,next_state,latest.report.coverage,vec![reason.clone()]);
        jobs::run(&tx,tx.query("UPDATE $row SET resume_requested=false,pause_requested=false,state=$state,activation_task_id=$task,activation_input_hash=$input,checkpoint_json=$checkpoint,report_json=$report,updated_at=<datetime>$now RETURN NONE; UPDATE $job SET state=$root_state,cancel_requested=false,finished_at=NONE,stage_coverage_json=$report,updated_at=<datetime>$now RETURN NONE;")
            .bind(("row",jobs::rid("atlas_reclassification_job",&job))).bind(("job",jobs::rid("job",&job))).bind(("state",next_state.as_str())).bind(("root_state",if valid {"running"}else{"blocked"})).bind(("task",latest.activation_task_id)).bind(("input",latest.activation_input_hash)).bind(("checkpoint",json(&latest.checkpoint,16384,"maintenance_checkpoint")?)).bind(("report",json(&latest.report,16384,"maintenance_report")?)).bind(("now",now.to_rfc3339()))).await?;
        jobs::log_transition_in(&tx,&job,&format!("{job}:resume:{}:{reason}",latest.checkpoint.revision),if valid {"atlas.reclassification.activation"}else{"atlas.reclassification.blocked"},&reason,&now.to_rfc3339()).await?;
        Ok(())
    }}).await
}

#[derive(Clone)]
pub(crate) struct PendingDerivative {
    pub item_id: String,
    pub occurrence: OccurrenceKey,
    pub receipt_key: String,
}
#[derive(SurrealValue)]
struct DerivativeRow {
    key: String,
    run_id: String,
    article_id: String,
    receipt_key: String,
}
pub(crate) async fn pending_derivatives(
    owner: &AdmissionOwner,
    activation: &Activation,
    limit: usize,
    now: DateTime<Utc>,
) -> Result<Vec<PendingDerivative>, MaintenanceError> {
    if limit > 64 {
        return Err(MaintenanceError::InvalidRequest {
            code: "derivative_budget".into(),
        });
    }
    let captured = activation.clone();
    Ok(admission::owned_tx(owner,RetryPolicy::default(),move|tx| {let activation=captured.clone();async move {
        if !activation.fence.check_in(&tx,now).await? {return Err(TxError::Abort("maintenance_lease_lost".into()));}
        let mut response=jobs::run(&tx,tx.query("SELECT record::id(id) AS key,run_id,article_id,receipt_key FROM atlas_reclassification_item WHERE job_id=$job AND derivative_state='pending' AND disposition IN ['admitted','rejected','already_current'] ORDER BY run_id,article_id LIMIT $limit")
            .bind(("job",activation.task.job_id)).bind(("limit",limit as i64))).await?;
        let rows=jobs::take::<Vec<DerivativeRow>>(&mut response,0)?;
        Ok(rows.into_iter().map(|row|PendingDerivative {item_id:row.key,occurrence:OccurrenceKey::new(row.run_id,row.article_id),receipt_key:row.receipt_key}).collect())
    }}).await?.value)
}

pub(crate) async fn legacy_remaining(repo: &Repository) -> Result<bool, MaintenanceError> {
    let mut result=admission::db_query(repo.db(),"RETURN count(SELECT id FROM atlas_article WHERE taxonomy_version!=$taxonomy OR admission_state='legacy' LIMIT 1)>0")
        .bind(("taxonomy",crate::atlas_taxonomy::TAXONOMY_VERSION)).await.map_err(TxError::from)?.check().map_err(TxError::from)?;
    result
        .take::<Option<bool>>(0)
        .map_err(TxError::from)?
        .ok_or_else(|| MaintenanceError::InvalidRequest {
            code: "missing_legacy_coverage_reply".into(),
        })
}

#[derive(Clone)]
pub(crate) enum ActivationEnd {
    Continue { reason: String, delay_secs: i64 },
    Blocked { reason: String },
    Failed { reason: String },
}

/// Completes the currently owned activation and, when required, creates its
/// exact continuation atomically with the durable scan/counter checkpoint.
/// Controls win over new work; only this owner's evaluating items are released.
pub(crate) async fn settle_activation(
    owner: &AdmissionOwner,
    activation: &Activation,
    checkpoint: &MaintenanceCheckpoint,
    accounting_items: &[String],
    end: ActivationEnd,
    now: DateTime<Utc>,
) -> Result<MaintenanceReport, MaintenanceError> {
    if accounting_items.len() > 64 {
        return Err(MaintenanceError::InvalidRequest {
            code: "maintenance_close_budget".into(),
        });
    }
    let captured = (
        activation.clone(),
        checkpoint.clone(),
        accounting_items.to_vec(),
        end,
    );
    Ok(admission::owned_tx(owner,RetryPolicy::default(),move|tx| {
        let (activation,mut checkpoint,items,end)=captured.clone();async move {
            if !activation.fence.owns_in(&tx,now).await? {return Err(TxError::Abort("maintenance_lease_lost".into()));}
            let mut response=jobs::run(&tx,tx.query(format!("SELECT {JOB_PROJECTION} FROM ONLY $row"))
                .bind(("row",jobs::rid("atlas_reclassification_job",&activation.task.job_id)))).await?;
            let current=decode(jobs::take::<Option<JobRow>>(&mut response,0)?.ok_or_else(||TxError::Abort("maintenance_missing".into()))?).map_err(|_|TxError::Abort("maintenance_request_corrupt".into()))?;
            if current.checkpoint.revision!=checkpoint.revision {return Err(TxError::Abort("maintenance_checkpoint_stale".into()));}
            jobs::run(&tx,tx.query("UPDATE atlas_reclassification_item SET disposition='retryable',last_error='activation_interrupted',owner='',lease_until=NONE,stats_dirty=true,updated_at=<datetime>$now WHERE job_id=$job AND disposition='evaluating' AND owner=$owner AND epoch=$epoch RETURN NONE;")
                .bind(("job",activation.task.job_id.clone())).bind(("owner",activation.task.lease_owner.clone())).bind(("epoch",activation.task.lease_epoch)).bind(("now",now.to_rfc3339()))).await?;
            account_in(&tx,&activation,&mut checkpoint,&items).await?;
            let mut pending=jobs::run(&tx,tx.query("RETURN count(SELECT id FROM atlas_reclassification_item WHERE job_id=$job AND (source_retry_pending=true OR disposition INSIDE ['evaluating','retryable','conflicted','pending'] OR stats_dirty=true OR (disposition INSIDE ['admitted','rejected','already_current'] AND derivative_state='pending')) LIMIT 1)>0;")
                .bind(("job",activation.task.job_id.clone()))).await?;
            let open=jobs::take::<Option<bool>>(&mut pending,0)?.ok_or_else(||TxError::Abort("maintenance_pending_reply".into()))?;
            let mut churn=jobs::run(&tx,tx.query("RETURN count(SELECT id FROM atlas_reclassification_conflict WHERE job_id=$job AND kind='counter' AND generation=$generation AND generation_attempts>=3 LIMIT 1)>0;")
                .bind(("job",activation.task.job_id.clone())).bind(("generation",i64::try_from(checkpoint.conflict_generation).map_err(|_|TxError::Abort("conflict_generation_overflow".into()))?))).await?;
            let source_churn=jobs::take::<Option<bool>>(&mut churn,0)?.ok_or_else(||TxError::Abort("maintenance_churn_reply".into()))?;
            let (state,reason,delay)=if current.cancel_requested {
                (MaintenanceState::Cancelled,"explicit_cancel".to_owned(),0)
            }else if current.pause_requested {
                (MaintenanceState::Paused,"explicit_pause".to_owned(),0)
            }else if source_churn {(MaintenanceState::Blocked,"source_churn".to_owned(),0)
            }else {match end {
                ActivationEnd::Blocked {reason}=>(MaintenanceState::Blocked,reason,0),
                ActivationEnd::Failed {reason}=>(MaintenanceState::Failed,reason,0),
                ActivationEnd::Continue {reason,delay_secs}=> {
                    if !(0..=60).contains(&delay_secs) {return Err(TxError::Abort("maintenance_delay_bound".into()));}
                    if checkpoint.scan_exhausted && !open && checkpoint.derivatives_pending==0 {
                        (MaintenanceState::Completed,"scope_complete".to_owned(),0)
                    }else {(if checkpoint.scan_exhausted {MaintenanceState::Reconciling}else{MaintenanceState::Queued},reason,delay_secs)}
                }
            }};
            if reason.is_empty() || reason.len()>64 || !reason.bytes().all(|b|b.is_ascii_lowercase()||b.is_ascii_digit()||b==b'_') {return Err(TxError::Abort("maintenance_reason_bound".into()));}
            checkpoint.revision=checkpoint.revision.checked_add(1).filter(|value|*value<=i64::MAX as u64).ok_or_else(||TxError::Abort("maintenance_revision_overflow".into()))?;
            let continuation=matches!(state,MaintenanceState::Queued|MaintenanceState::Reconciling|MaintenanceState::Paused);
            let mut next_task=current.activation_task_id;
            let mut next_input=current.activation_input_hash;
            if continuation {
                checkpoint.activation_seq=checkpoint.activation_seq.checked_add(1).filter(|value|*value<=i64::MAX as u64).ok_or_else(||TxError::Abort("maintenance_activation_overflow".into()))?;
                next_task=format!("{}:activate:{}",activation.task.job_id,checkpoint.activation_seq);
                next_input=crate::classifier::artifact::sha256_hex(json(&checkpoint,16384,"maintenance_checkpoint")?.as_bytes());
                let published=jobs::yield_owned_in(&tx,&activation.task,&NewTask {id:&next_task,job:&activation.task.job_id,pool:POOL,operation:OPERATION,priority:PRIORITY,deps:vec![activation.task.id.as_str()],input_hash:&next_input,created_at:now},now+chrono::Duration::seconds(delay),&reason,now).await?;
                if published!=jobs::Publish::Committed {return Err(TxError::Abort("maintenance_yield_lost".into()));}
            }else {
                jobs::complete_in(&tx,&activation.task,&format!("maintenance:{}:{}",state.as_str(),checkpoint.revision),&now.to_rfc3339()).await?;
            }
            let coverage=admission::coverage_retained_scope_in(&tx,&checkpoint.scope,&checkpoint.cutoff_at,checkpoint.scan_upper.as_ref()).await?;
            let report=MaintenanceReport::from_checkpoint(&checkpoint,state,coverage,vec![reason.clone()]);
            let report_json=json(&report,16384,"maintenance_report")?;
            jobs::run(&tx,tx.query("UPDATE $row SET state=$state,activation_task_id=$task,activation_input_hash=$input,checkpoint_json=$checkpoint,report_json=$report,resume_requested=false,updated_at=<datetime>$now RETURN NONE; UPDATE $job SET state=$root,worker_owner='',stage_coverage_json=$report,progress_done=$done,finished_at=IF $terminal {<datetime>$now} ELSE {NONE},updated_at=<datetime>$now RETURN NONE; IF $cancelled {UPDATE $old SET state='cancelled' RETURN NONE; UPDATE attempt SET outcome='cancelled' WHERE task=$old AND epoch=$epoch RETURN NONE;};")
                .bind(("row",jobs::rid("atlas_reclassification_job",&activation.task.job_id))).bind(("job",jobs::rid("job",&activation.task.job_id))).bind(("old",jobs::rid("task",&activation.task.id))).bind(("epoch",activation.task.lease_epoch))
                .bind(("state",state.as_str())).bind(("root",match state {MaintenanceState::Paused=>"paused",MaintenanceState::Blocked=>"blocked",MaintenanceState::Completed=>"completed",MaintenanceState::Cancelled=>"cancelled",MaintenanceState::Failed=>"failed",_=>"running"}))
                .bind(("task",next_task)).bind(("input",next_input)).bind(("checkpoint",json(&checkpoint,16384,"maintenance_checkpoint")?)).bind(("report",report_json)).bind(("done",i64::try_from(checkpoint.rows_scanned).map_err(|_|TxError::Abort("maintenance_progress_overflow".into()))?)).bind(("terminal",state.terminal())).bind(("cancelled",state==MaintenanceState::Cancelled)).bind(("now",now.to_rfc3339()))).await?;
            jobs::log_transition_in(&tx,&activation.task.job_id,&format!("{}:settle:{}",activation.task.id,activation.task.lease_epoch),match state {MaintenanceState::Completed=>"atlas.reclassification.completed",MaintenanceState::Blocked=>"atlas.reclassification.blocked",MaintenanceState::Cancelled=>"atlas.reclassification.cancelled",MaintenanceState::Paused=>"atlas.reclassification.paused",_=>"atlas.reclassification.activation"},&reason,&now.to_rfc3339()).await?;
            Ok(report)
        }
    }).await?.value)
}

/// Audits actual native stage receipts and persists the local completion
/// checkpoint in the SAME source/body-fenced transaction. Historical outcomes
/// remain intact when their input is superseded; a separate retry flag reopens
/// only that occurrence under a fresh immutable capture.
pub(crate) async fn record_derivative_states(
    owner: &AdmissionOwner,
    activation: &Activation,
    pending: &[PendingDerivative],
    now: DateTime<Utc>,
) -> Result<(), MaintenanceError> {
    if pending.len() > 64 {
        return Err(MaintenanceError::InvalidRequest {
            code: "derivative_audit_budget".into(),
        });
    }
    let captured = (activation.clone(), pending.to_vec());
    admission::owned_tx(owner,RetryPolicy::default(),move|tx| {
        let (activation,pending)=captured.clone();async move {
            if !activation.fence.owns_in(&tx,now).await? {return Err(TxError::Abort("maintenance_lease_lost".into()));}
            let work:Vec<_>=pending.iter().map(|item|admission::DerivativeWork::new(item.occurrence.clone(),item.receipt_key.clone())).collect();
            let states=admission::maintenance_derivative_states_in(&tx,&activation.fence,&work,now).await?;
            let keys:Vec<_>=pending.iter().map(|item|item.occurrence.clone()).collect();
            let views=admission::classification_views_many_in(&tx,&keys).await.map_err(|_|TxError::Abort("maintenance_derivative_receipt_invalid".into()))?;
            if states.len()!=pending.len() {return Err(TxError::Abort("maintenance_derivative_audit_missing".into()));}
            let mut changes=Vec::new();
            for item in pending {
                let state=states.iter().find(|state|state.work.occurrence==item.occurrence && state.work.receipt_key==item.receipt_key).ok_or_else(||TxError::Abort("maintenance_derivative_audit_identity".into()))?;
                let same=views.get(&item.occurrence).and_then(|view|view.receipt.as_ref()).is_some_and(|receipt|receipt.key.record_id()==item.receipt_key);
                let disposition=if !same {"superseded_input"}else if state.complete {
                    if state.stages.iter().any(|stage|matches!(stage.disposition.as_ref(),Some(admission::DerivativeDisposition::PolicyIneligible {..}))) {"policy_ineligible"}
                    else if state.stages.iter().any(|stage|matches!(stage.disposition.as_ref(),Some(admission::DerivativeDisposition::Disabled {..}))) {"disabled"}
                    else {"current"}
                }else {"pending"};
                changes.push(serde_json::json!({"item":item.item_id,"receipt":item.receipt_key,"state":disposition,"retry":!same}));
            }
            jobs::run(&tx,tx.query("FOR $change IN $changes {UPDATE type::record('atlas_reclassification_item',$change.item) SET derivative_state=$change.state,source_retry_pending=$change.retry,stats_dirty=true,updated_at=<datetime>$now WHERE job_id=$job AND receipt_key=$change.receipt AND disposition INSIDE ['admitted','rejected','already_current'] RETURN NONE;};")
                .bind(("changes",changes)).bind(("job",activation.task.job_id)).bind(("now",now.to_rfc3339()))).await?;
            Ok(())
        }
    }).await?;
    Ok(())
}

/// An immutable capture/generation marker prevents uncertain reply or later
/// owner recovery from counting the same logical source conflict twice.
async fn account_source_conflicts_in(
    tx: &Tx,
    activation: &Activation,
    checkpoint: &MaintenanceCheckpoint,
    items: &[String],
) -> Result<(), TxError> {
    #[derive(SurrealValue)]
    struct ConflictItem {
        key: String,
        evaluation_generation: i64,
        source_fence_json: Option<String>,
    }
    let mut response=jobs::run(tx,tx.query("SELECT record::id(id) AS key,evaluation_generation,IF bytes::len(type::bytes(source_fence_json))<=32768 {source_fence_json} ELSE {NONE} AS source_fence_json FROM $items WHERE job_id=$job AND disposition='conflicted' AND last_error='source_changed'")
        .bind(("items",items.iter().map(|key|jobs::rid("atlas_reclassification_item",key)).collect::<Vec<_>>())).bind(("job",activation.task.job_id.clone()))).await?;
    let mut captured = Vec::new();
    for item in jobs::take::<Vec<ConflictItem>>(&mut response, 0)? {
        let json = item
            .source_fence_json
            .ok_or_else(|| TxError::Abort("conflict_source_oversized".into()))?;
        let source: RetainedSourceFence = serde_json::from_str(&json)
            .map_err(|_| TxError::Abort("conflict_source_invalid".into()))?;
        if admission::maintenance_item_key(&activation.task.job_id, &source) != item.key {
            return Err(TxError::Abort("conflict_item_identity".into()));
        }
        let generation = u64::try_from(item.evaluation_generation)
            .map_err(|_| TxError::Abort("negative_conflict_generation".into()))?;
        if generation > checkpoint.conflict_generation {
            return Err(TxError::Abort("future_conflict_generation".into()));
        }
        captured.push((item.key, source, generation));
    }
    let keys: Vec<_> = captured
        .iter()
        .map(|(_, source, _)| source.occurrence.clone())
        .collect();
    let articles = super::atlas::articles_many_in(tx, &keys).await?;
    let current = admission::retained_source_fences_for_audit_in(
        tx,
        &articles.into_values().collect::<Vec<_>>(),
    )
    .await?;
    let mut values = Vec::new();
    let mut deferred = Vec::new();
    for (item_key, source, generation) in captured {
        // An unchanged canonical capture/guard reservation is operational
        // contention. A definite changed pending head is still a real conflict.
        if current
            .get(&source.occurrence)
            .is_none_or(|current| current == &source)
        {
            deferred.push(item_key);
            continue;
        }
        let capture = source.capture_revision();
        values.push(serde_json::json!({"item":item_key,"counter":super::codec::encode(&[&activation.task.job_id,&source.occurrence.run_id,&source.occurrence.article_id,"counter"]),"marker":super::codec::encode(&[&activation.task.job_id,&source.occurrence.run_id,&source.occurrence.article_id,"attempt",&capture,&generation.to_string()]),"generation":generation,"run":source.occurrence.run_id,"article":source.occurrence.article_id,"capture":capture}));
    }
    if !deferred.is_empty() {
        jobs::run(tx,tx.query("UPDATE $items SET disposition='retryable',last_error='source_temporarily_unavailable',stats_dirty=true WHERE job_id=$job AND disposition='conflicted' RETURN NONE;")
            .bind(("items",deferred.iter().map(|item|jobs::rid("atlas_reclassification_item",item)).collect::<Vec<_>>())).bind(("job",activation.task.job_id.clone()))).await?;
    }
    if values.is_empty() {
        return Ok(());
    }
    jobs::run(tx,tx.query("FOR $value IN $values {LET $marker=type::record('atlas_reclassification_conflict',$value.marker); IF !record::exists($marker) {CREATE $marker SET job_id=$job,run_id=$value.run,article_id=$value.article,kind='attempt',capture_revision=$value.capture,generation=$value.generation,created_at=<datetime>$now RETURN NONE; UPSERT type::record('atlas_reclassification_conflict',$value.counter) SET job_id=$job,run_id=$value.run,article_id=$value.article,kind='counter',generation_attempts=IF (generation ?? 0)=$value.generation {(generation_attempts ?? 0)+1} ELSE IF (generation ?? 0)<$value.generation {1} ELSE {generation_attempts},generation=math::max([(generation ?? 0),$value.generation]),cumulative_attempts=(cumulative_attempts ?? 0)+1,created_at=(created_at ?? <datetime>$now) RETURN NONE; UPDATE type::record('atlas_reclassification_item',$value.item) SET disposition='conflicted',conflict_attempts+=1,stats_dirty=true WHERE job_id=$job AND disposition='conflicted' AND last_error='source_changed' RETURN NONE;};};")
        .bind(("values",values)).bind(("job",activation.task.job_id.clone())).bind(("now",chrono::Utc::now().to_rfc3339()))).await?;
    Ok(())
}

/// The common publication SQL, immutable conflict markers, counter deltas and
/// checkpoint advance share one owned transaction. A lost reply cannot expose
/// a partial classification/conflict accounting result.
pub(crate) async fn publish_page(
    owner: &AdmissionOwner,
    activation: &Activation,
    outcomes: &[admission::MaintenanceOutcome],
    sources: &BTreeMap<OccurrenceKey, RetainedSourceFence>,
    now: DateTime<Utc>,
) -> Result<(admission::BatchCommit, MaintenanceCheckpoint), MaintenanceError> {
    if outcomes.len() > 64 || outcomes.len() != sources.len() {
        return Err(MaintenanceError::InvalidRequest {
            code: "maintenance_publication_budget".into(),
        });
    }
    let prepared = admission::prepare_maintenance_outcomes(&activation.fence, outcomes)?;
    let captured = (activation.clone(), prepared, sources.clone());
    let committed = admission::owned_tx(owner, RetryPolicy::default(), move |tx| {
        let (activation, prepared, sources) = captured.clone();
        async move {
            let outcomes =
                admission::commit_prepared_maintenance_in(&tx, &activation.fence, &prepared, now)
                    .await?;
            if outcomes.len() != sources.len() {
                return Err(TxError::Abort("maintenance_outcome_count".into()));
            }
            let items: Vec<_> = sources
                .values()
                .map(|source| admission::maintenance_item_key(&activation.task.job_id, source))
                .collect();
            let mut checkpoint = activation.checkpoint.clone();
            account_in(&tx, &activation, &mut checkpoint, &items).await?;
            checkpoint.revision = checkpoint
                .revision
                .checked_add(1)
                .filter(|value| *value <= i64::MAX as u64)
                .ok_or_else(|| TxError::Abort("maintenance_revision_overflow".into()))?;
            save_checkpoint_in(
                &tx,
                &activation,
                &checkpoint,
                MaintenanceState::Running,
                &[],
                now,
            )
            .await?;
            Ok((outcomes, checkpoint))
        }
    })
    .await?;
    let (items, checkpoint) = committed.value;
    Ok((
        admission::BatchCommit {
            items,
            retries: committed.retries,
        },
        checkpoint,
    ))
}

/// Pausing a blocked request does not resurrect its completed task. Explicit
/// resume creates a dependent activation only when no queued activation exists;
/// the actual drain still verifies the exact saved model before dispatch.
async fn ensure_queued_activation_in(
    tx: &Tx,
    stored: &mut StoredJob,
    now: DateTime<Utc>,
) -> Result<(), TxError> {
    let mut response = jobs::run(
        tx,
        tx.query("SELECT VALUE state FROM ONLY $task;")
            .bind(("task", jobs::rid("task", &stored.activation_task_id))),
    )
    .await?;
    match jobs::take::<Option<String>>(&mut response, 0)?.as_deref() {
        Some("queued") => return Ok(()),
        Some("completed") => {}
        _ => return Err(TxError::Abort("maintenance_resume_task_unavailable".into())),
    }
    let previous = stored.activation_task_id.clone();
    stored.checkpoint.activation_seq = stored
        .checkpoint
        .activation_seq
        .checked_add(1)
        .filter(|value| *value <= i64::MAX as u64)
        .ok_or_else(|| TxError::Abort("maintenance_activation_overflow".into()))?;
    stored.activation_task_id = format!(
        "{}:activate:{}",
        stored.checkpoint.job_id, stored.checkpoint.activation_seq
    );
    stored.activation_input_hash = crate::classifier::artifact::sha256_hex(
        json(&stored.checkpoint, 16384, "maintenance_checkpoint")?.as_bytes(),
    );
    jobs::create_task_in(
        tx,
        &NewTask {
            id: &stored.activation_task_id,
            job: &stored.checkpoint.job_id,
            pool: POOL,
            operation: OPERATION,
            priority: PRIORITY,
            deps: vec![previous.as_str()],
            input_hash: &stored.activation_input_hash,
            created_at: now,
        },
    )
    .await
}

#[cfg(all(test, feature = "embedded"))]
mod conflict_tests {
    use super::*;
    /// This fixture exercises storage ownership/counters only. No inference
    /// service is installed or invoked; successful classifier receipts are
    /// covered by the separate public real-worker historical integration test.
    fn administrative_context() -> AdmissionContext {
        let snapshot = ModelSnapshot::new(
            "counter-fixture",
            "a".repeat(64),
            crate::classifier::Provenance::synthetic_baseline(
                "counter-storage-only",
                "b".repeat(64),
            ),
        );
        AdmissionContext {
            pin: crate::classifier::readiness::PinnedClassifier {
                snapshot,
                manifest_sha256: "c".repeat(64),
                service: std::sync::Arc::new(crate::classifier::ClassifierService::default()),
            },
            manifest_sha256: "c".repeat(64),
            gates: crate::classifier::calibration::GateSet {
                version: crate::classifier::calibration::CALIBRATION_VERSION.into(),
                domains: crate::atlas_taxonomy::DomainId::ALL.map(|domain| {
                    crate::classifier::calibration::DomainCalibration {
                        domain,
                        knots: Vec::new(),
                        threshold: 2.0,
                        min_informative_features: 1,
                    }
                }),
            },
        }
    }
    async fn ledger(repo: &Repository, job: &str, article: &str) -> (i64, i64, i64) {
        let key = super::super::codec::encode(&[job, "retained", article, "counter"]);
        let mut response=repo.db().query("RETURN (SELECT VALUE cumulative_attempts FROM ONLY $key) ?? 0; RETURN (SELECT VALUE generation_attempts FROM ONLY $key) ?? 0; RETURN (SELECT VALUE generation FROM ONLY $key) ?? 0;")
            .bind(("key",jobs::rid("atlas_reclassification_conflict",&key))).await.unwrap().check().unwrap();
        (
            response.take::<Option<i64>>(0).unwrap().unwrap(),
            response.take::<Option<i64>>(1).unwrap().unwrap(),
            response.take::<Option<i64>>(2).unwrap().unwrap(),
        )
    }
    fn article(id: &str, version: usize) -> AtlasArticleRow {
        AtlasArticleRow {
            run_id: "retained".into(),
            id: id.into(),
            title: format!("Original retained metadata revision {version}"),
            description: "Complete metadata for storage ownership proof".into(),
            provider: format!("provider-{version}"),
            url: format!("https://fixture.invalid/{id}"),
            seen_at: "2026-01-01T00:00:00Z".into(),
            category: "economic".into(),
            categories: vec!["economic".into()],
            ..Default::default()
        }
    }
    #[tokio::test]
    async fn three_distinct_source_captures_block_once_and_explicit_resume_starts_a_new_conflict_generation(
    ) {
        let directory = tempfile::tempdir().unwrap();
        let repo = Repository::connect(
            &super::super::open::Target::Embedded {
                path: directory.path().join("db"),
            },
            "argos",
            "maintenance_conflict_ledger",
        )
        .await
        .unwrap();
        for id in ["changing", "unrelated"] {
            super::super::atlas::upsert_article(repo.db(), &article(id, 0))
                .await
                .unwrap();
        }
        let context = administrative_context();
        let job = create(
            &repo,
            &ReclassificationRequest::new("three-source-versions", MaintenanceScope::AllRetained),
            &context,
            Utc::now(),
        )
        .await
        .unwrap();
        let owner = AdmissionOwner::new(repo.clone());
        let mut old_activation = None;
        let mut old_items = Vec::new();
        for version in 1..=3 {
            let mut activation = claim(&owner, "storage-counter-owner", Utc::now())
                .await
                .unwrap()
                .unwrap();
            let prepared = prepare_page(&owner, &activation, 1, Utc::now())
                .await
                .unwrap();
            activation.checkpoint = prepared.checkpoint;
            assert_eq!(prepared.inputs.len(), 1);
            assert_eq!(prepared.inputs[0].claim.occurrence.article_id, "changing");
            // Legitimate public metadata correction, after capture but before
            // the exact common publication transaction, creates actual staleness.
            super::super::atlas::upsert_article(repo.db(), &article("changing", version))
                .await
                .unwrap();
            let outcomes: Vec<_> = prepared
                .inputs
                .into_iter()
                .map(|input| {
                    admission::MaintenanceOutcome::new(
                        prepared.sources[&input.claim.occurrence].clone(),
                        crate::atlas_classify::ItemClassification::Retryable {
                            claim: input.claim,
                            error: crate::classifier::ClassifierError::Busy,
                        },
                    )
                })
                .collect();
            let (committed, checkpoint) = publish_page(
                &owner,
                &activation,
                &outcomes,
                &prepared.sources,
                Utc::now(),
            )
            .await
            .unwrap();
            activation.checkpoint = checkpoint;
            assert!(matches!(committed.items.as_slice(),[admission::ItemCommit::StaleInput {..}]),"actual stale source dominates an operational result without creating a classifier receipt");
            assert_eq!(
                ledger(&repo, &job, "changing").await,
                (version as i64, version as i64, 0)
            );
            let items = committed_item_ids(&activation, &prepared.sources, &committed).unwrap();
            // Bounded reconciliation replay in the SAME live owner cannot
            // increment the immutable logical-attempt marker again.
            let captured = (activation.clone(), items.clone());
            admission::owned_tx(&owner, RetryPolicy::default(), move |tx| {
                let (activation, items) = captured.clone();
                async move {
                    if !activation.fence.owns_in(&tx, Utc::now()).await? {
                        return Err(TxError::Abort("test_owner_lost".into()));
                    }
                    account_source_conflicts_in(&tx, &activation, &activation.checkpoint, &items)
                        .await
                }
            })
            .await
            .unwrap();
            assert_eq!(
                ledger(&repo, &job, "changing").await,
                (version as i64, version as i64, 0)
            );
            assert_eq!(ledger(&repo, &job, "unrelated").await, (0, 0, 0));
            let report = settle_activation(
                &owner,
                &activation,
                &activation.checkpoint,
                &items,
                ActivationEnd::Continue {
                    reason: "source_retry".into(),
                    delay_secs: 0,
                },
                Utc::now(),
            )
            .await
            .unwrap();
            assert_eq!(report.conflict_attempts, version as u64);
            assert_eq!(
                report.state,
                if version == 3 {
                    MaintenanceState::Blocked
                } else {
                    MaintenanceState::Queued
                }
            );
            if version == 1 {
                old_activation = Some(activation);
                old_items = items;
            }
        }
        assert!(claim(&owner, "blocked-owner", Utc::now())
            .await
            .unwrap()
            .is_none());
        let old = old_activation.unwrap();
        assert!(
            settle_activation(
                &owner,
                &old,
                &old.checkpoint,
                &old_items,
                ActivationEnd::Continue {
                    reason: "late_replay".into(),
                    delay_secs: 0
                },
                Utc::now()
            )
            .await
            .is_err(),
            "old ownership cannot replay a later logical attempt"
        );
        assert_eq!(ledger(&repo, &job, "changing").await, (3, 3, 0));
        control(&repo, &job, Control::Resume).await.unwrap();
        let stored = requested_resume(&repo).await.unwrap().unwrap();
        assert_eq!(stored.report.state, MaintenanceState::Blocked);
        // This is the storage CAS port, given its already-verified context.
        // Missing/restored real pin validation is covered by the public test.
        settle_resume(&repo, &stored, Some(&context), "pin_verified", Utc::now())
            .await
            .unwrap();
        let mut activation = claim(&owner, "resumed-owner", Utc::now())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(activation.checkpoint.conflict_generation, 1);
        let prepared = prepare_page(&owner, &activation, 1, Utc::now())
            .await
            .unwrap();
        activation.checkpoint = prepared.checkpoint;
        super::super::atlas::upsert_article(repo.db(), &article("changing", 4))
            .await
            .unwrap();
        let outcomes: Vec<_> = prepared
            .inputs
            .into_iter()
            .map(|input| {
                admission::MaintenanceOutcome::new(
                    prepared.sources[&input.claim.occurrence].clone(),
                    crate::atlas_classify::ItemClassification::Retryable {
                        claim: input.claim,
                        error: crate::classifier::ClassifierError::Busy,
                    },
                )
            })
            .collect();
        let (committed, checkpoint) = publish_page(
            &owner,
            &activation,
            &outcomes,
            &prepared.sources,
            Utc::now(),
        )
        .await
        .unwrap();
        activation.checkpoint = checkpoint;
        let items = committed_item_ids(&activation, &prepared.sources, &committed).unwrap();
        let report = settle_activation(
            &owner,
            &activation,
            &activation.checkpoint,
            &items,
            ActivationEnd::Continue {
                reason: "source_retry".into(),
                delay_secs: 0,
            },
            Utc::now(),
        )
        .await
        .unwrap();
        assert_eq!(report.state, MaintenanceState::Queued);
        assert_eq!(ledger(&repo, &job, "changing").await, (4, 1, 1));
        assert_eq!(ledger(&repo, &job, "unrelated").await, (0, 0, 0));
        let mut receipts=repo.db().query("RETURN count(SELECT id FROM atlas_classification_receipt); RETURN count(SELECT id FROM atlas_reclassification_origin);").await.unwrap().check().unwrap();
        assert_eq!(
            receipts.take::<Option<i64>>(0).unwrap(),
            Some(0),
            "this ownership proof never fabricates an admitted/rejected classifier receipt"
        );
        assert_eq!(
            receipts.take::<Option<i64>>(1).unwrap(),
            Some(4),
            "all pre-change original captures are retained"
        );
        owner.shutdown().await.unwrap();
    }
}

/// Cancellation is administrative cleanup, not a scoring prerequisite. A
/// crashed cancelled worker must be settleable even if its saved model later
/// disappears. No live task owner is displaced and no new evidence is written.
pub(crate) async fn settle_pending_cancel(
    repo: &Repository,
    now: DateTime<Utc>,
) -> Result<Option<MaintenanceReport>, MaintenanceError> {
    owned(repo,move|tx|async move {
        #[derive(SurrealValue)]struct Pending {key:String}
        let mut response=jobs::run(&tx,tx.query("SELECT record::id(id) AS key FROM atlas_reclassification_job WHERE cancel_requested=true AND state NOTINSIDE ['completed','cancelled','failed'] AND count(SELECT id FROM task WHERE job=type::record('job',record::id($parent.id)) AND state='running' AND lease_until> <datetime>$now LIMIT 1)=0 ORDER BY updated_at LIMIT 1;").bind(("now",now.to_rfc3339()))).await?;
        let Some(pending)=jobs::take::<Vec<Pending>>(&mut response,0)?.into_iter().next() else{return Ok(None)};
        cancel_in(&tx,&pending.key,now).await?;
        let mut response=jobs::run(&tx,tx.query(format!("SELECT {JOB_PROJECTION} FROM ONLY $row")).bind(("row",jobs::rid("atlas_reclassification_job",&pending.key)))).await?;
        let stored=decode(jobs::take::<Option<JobRow>>(&mut response,0)?.ok_or_else(||TxError::Abort("maintenance_missing".into()))?).map_err(|_|TxError::Abort("maintenance_request_corrupt".into()))?;
        Ok(Some(stored.report))
    }).await
}
