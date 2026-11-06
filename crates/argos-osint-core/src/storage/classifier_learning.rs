//! Durable candidate fitting with frozen captured inputs and an independent
//! task lease. Missing reviewed/runtime evidence retains the prior artifact.
use super::classifier_admission::{self as admission, AdmissionOwner};
use super::jobs::{self, ClaimedTask, NewJob, NewTask};
use super::repo::Repository;
use super::tx::{RetryPolicy, TxError};
use crate::classifier::artifact::{self, ModelId};
use crate::classifier::dataset::{self, DatasetRequest};
use crate::classifier::learning::{self, LearningInput, ReviewedCapture, WeakCapture};
use crate::classifier::readiness::ClassifierHome;
use crate::classifier::training;
use crate::classifier::WorkControl;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use surrealdb::types::SurrealValue;

pub const OPERATION: &str = "classifier_learn";
pub const POOL: &str = "classifier_learning";
const LEASE_SECS: i64 = 60;
pub const SCHEMA: &str = "
DEFINE TABLE IF NOT EXISTS classifier_learning_job SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS input_json ON classifier_learning_job TYPE string;
DEFINE FIELD IF NOT EXISTS input_sha256 ON classifier_learning_job TYPE string;
DEFINE FIELD IF NOT EXISTS result_json ON classifier_learning_job TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS promotion_json ON classifier_learning_job TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS candidate_model_id ON classifier_learning_job TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS updated_at ON classifier_learning_job TYPE datetime;
DEFINE TABLE IF NOT EXISTS classifier_learning_used SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS job_id ON classifier_learning_used TYPE string;
DEFINE FIELD IF NOT EXISTS learned_at ON classifier_learning_used TYPE datetime;
";
fn invalid(code: &str) -> TxError {
    TxError::Abort(code.into())
}
fn encode(value: &impl Serialize, limit: usize) -> Result<String, TxError> {
    admission::bounded_json(value, limit, "learning_payload")
        .map_err(|_| invalid("learning_payload_bound"))
}

/// Called after terminal-cycle monitoring. No model work runs in the cycle or a
/// transaction. An exact immutable input creates at most one root job.
pub(crate) async fn consider(
    owner: &AdmissionOwner,
    home: &ClassifierHome,
) -> Result<&'static str, TxError> {
    let evidence=admission::owned_tx(owner,RetryPolicy::default(),|tx|async move {
        let approval=super::classifier_review::active_approval_in(&tx).await?;
        let Some(approval)=approval else {return Ok(None);};
        let mut reply=tx.query("SELECT item_json,review_reference,corpus_sha256 FROM classifier_review WHERE split!='test' ORDER BY family_id,observation_id LIMIT 1025; SELECT payload_json,family_id,(SELECT VALUE split FROM ONLY type::record('classifier_story_family',$parent.family_id)) AS split FROM classifier_observation WHERE disposition='monitoring' AND family_id!='' ORDER BY first_seen DESC LIMIT 2048; SELECT VALUE record::id(id) FROM classifier_learning_used LIMIT 10001;").await?.check()?;
        #[derive(SurrealValue)] struct Review {item_json:String,review_reference:String,corpus_sha256:String}
        #[derive(SurrealValue)] struct Weak {payload_json:String,family_id:String,split:String}
        let reviews:Vec<Review>=reply.take(0)?;let weak:Vec<Weak>=reply.take(1)?;let used:Vec<String>=reply.take(2)?;
        if reviews.len()>1024 || used.len()>10000 {return Err(invalid("learning_retention_bound"));}
        let reviews=reviews.into_iter().map(|row|Ok(ReviewedCapture {item:serde_json::from_str(&row.item_json).map_err(|_|invalid("review_capture_corrupt"))?,review_reference:row.review_reference,corpus_sha256:row.corpus_sha256})).collect::<Result<Vec<_>,TxError>>()?;
        let weak=weak.into_iter().map(|row|Ok(WeakCapture {observation:serde_json::from_str(&row.payload_json).map_err(|_|invalid("weak_capture_corrupt"))?,family_id:training::FamilyId::parse(&row.family_id).map_err(|_|invalid("weak_family_invalid"))?,split:serde_json::from_value(row.split.into()).map_err(|_|invalid("weak_partition_invalid"))?})).collect::<Result<Vec<_>,TxError>>()?;
        Ok(Some((approval,reviews,weak,used)))
    }).await.map_err(|_|invalid("learning_capture_failed"))?.value;
    let Some((approval, reviews, weak, used)) = evidence else {
        return Ok("unapproved_mapping_monitor_only");
    };
    let home_path = home.home().to_path_buf();
    let input = tokio::task::spawn_blocking(move || -> anyhow::Result<LearningInput> {
        let mut request = DatasetRequest {
            cwd: std::env::current_dir()?,
            executable: std::env::current_exe()?,
            environment_file: std::env::var_os("ARGOS_CLASSIFIER_CSV")
                .map(std::path::PathBuf::from),
            home: home_path,
            ..DatasetRequest::default()
        };
        let paths = crate::data_paths::DataPaths::load_from_home(&request.home)?;
        request.configured_file =
            crate::data_paths::DataPaths::resolve(&request.home, &paths.article_csv);
        let seed = dataset::parse_csv(&dataset::resolve_dataset(&request)?)?;
        Ok(learning::capture(&seed.corpus, &reviews, &weak, &approval)?)
    })
    .await
    .map_err(|_| invalid("learning_capture_worker_failed"))?
    .map_err(|_| invalid("learning_corpus_unavailable"))?;
    let used: BTreeSet<_> = used.into_iter().collect();
    if input
        .eligible_family_ids
        .iter()
        .filter(|id| !used.contains(id.as_str()))
        .count()
        < 100
    {
        return Ok("fewer_than_100_new_eligible_families");
    }
    let encoded = encode(&input, 32 * 1024 * 1024)?;
    let hash = artifact::sha256_hex(encoded.as_bytes());
    let id = format!("classifier-learning-{hash}");
    admission::owned_tx(owner,RetryPolicy::default(),move|tx| {let(encoded,hash,id)=(encoded.clone(),hash.clone(),id.clone());async move {
        jobs::bump(&tx,"job_guard",&id).await?;
        let mut response=tx.query("RETURN record::exists($row);").bind(("row",jobs::rid("classifier_learning_job",&id))).await?.check()?;
        if response.take::<Option<bool>>(0)?==Some(true) {return Ok(());}
        let now=Utc::now();
        jobs::create_job_in(&tx,&NewJob {id:&id,kind:"maintenance",operation:OPERATION,parent:None,created_at:now}).await?;
        jobs::create_task_in(&tx,&NewTask {id:&format!("{id}:fit"),job:&id,pool:POOL,operation:OPERATION,priority:40,deps:Vec::new(),input_hash:&hash,created_at:now}).await?;
        tx.query("CREATE $row SET input_json=$input,input_sha256=$hash,result_json='',candidate_model_id='',updated_at=time::now() RETURN NONE; UPDATE $job SET phase='Candidate queued',progress_total=1,progress_done=0 RETURN NONE;")
            .bind(("row",jobs::rid("classifier_learning_job",&id))).bind(("job",jobs::rid("job",&id))).bind(("input",encoded)).bind(("hash",hash)).await?.check()?;
        Ok(())
    }}).await.map_err(|_|invalid("learning_enqueue_failed"))?;
    Ok("learning_candidate_queued")
}

#[derive(Serialize, Deserialize)]
pub(super) struct Outcome {
    pub(super) schema_version: String,
    pub(super) model_id: String,
    pub(super) state: String,
    pub(super) reason: String,
    pub(super) report_sha256: String,
    pub(super) promoted: Option<bool>,
    pub(super) activation: Option<crate::classifier::calibration::ActivationOutcome>,
    #[serde(skip)]
    pub(super) promotion_evidence: Option<crate::classifier::calibration::PromotionEvidence>,
}
#[derive(SurrealValue)]
struct InputRow {
    input_json: String,
    input_sha256: String,
}

/// Owns the fit handle through cancellation/shutdown. Heartbeats remain on the
/// async storage runtime while Natural executes on its worker thread.
pub async fn drain_one(
    repo: &Repository,
    home: &ClassifierHome,
    worker: &str,
    cancel: Arc<AtomicBool>,
) -> Result<usize, TxError> {
    if cancel.load(Ordering::Acquire)
        || admission::foreground_requested(repo, Utc::now())
            .await
            .map_err(|_| invalid("learning_foreground_check"))?
    {
        return Ok(0);
    }
    let owner = AdmissionOwner::new(repo.clone());
    let result = owner
        .scope(async {
            let worker = worker.to_owned();
            let task = admission::owned_tx(&owner, RetryPolicy::default(), move |tx| {
                let worker = worker.clone();
                async move {
                    let now = Utc::now();
                    jobs::interrupt_expired_pool_in(&tx, POOL, now).await?;
                    jobs::claim_in(
                        &tx,
                        POOL,
                        None,
                        &worker,
                        LEASE_SECS,
                        now,
                        &format!(
                            "learning:{worker}:{}",
                            now.timestamp_nanos_opt().unwrap_or_default()
                        ),
                        None,
                    )
                    .await
                }
            })
            .await
            .map_err(|_| invalid("learning_claim_failed"))?
            .value;
            let Some(task) = task else {
                return Ok(0);
            };
            Box::pin(fit_owned(&owner, home, task, &cancel)).await
        })
        .await;
    let cleanup = owner
        .shutdown()
        .await
        .map_err(|_| invalid("learning_cleanup_failed"));
    let value = result?;
    cleanup?;
    Ok(value)
}
pub(super) async fn renew(owner: &AdmissionOwner, task: &ClaimedTask) -> Result<bool, TxError> {
    let task = task.clone();
    admission::owned_tx(owner, RetryPolicy::default(), move |tx| {
        let task = task.clone();
        async move {
            jobs::bump(&tx, "job_guard", &task.root_id).await?;
            let now = Utc::now();
            let Some(fence) = jobs::fence(&tx, &task, &now.to_rfc3339()).await? else {
                return Ok(false);
            };
            if !jobs::holds(&fence, &task) || fence.cancelled {
                return Ok(false);
            }
            jobs::renew_owned_in(&tx, &task, LEASE_SECS, now).await
        }
    })
    .await
    .map(|value| value.value)
    .map_err(|_| invalid("learning_heartbeat_failed"))
}
fn failure_code(error: &crate::classifier::ClassifierError) -> String {
    use crate::classifier::ClassifierError;
    let bounded = |text: &str| {
        text.len() <= 128
            && !text.is_empty()
            && text
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    };
    match error {
        ClassifierError::BridgeProtocol { operation, code }
            if bounded(operation) && bounded(code) =>
        {
            format!("{operation}:{code}")
        }
        ClassifierError::InvalidTrainingData { reason, .. } if bounded(reason) => reason.clone(),
        ClassifierError::InvalidArtifact(reason) if bounded(reason) => reason.clone(),
        ClassifierError::InvalidCalibrationData { reason, .. } if bounded(reason) => reason.clone(),
        ClassifierError::Timeout => "execution_timeout".into(),
        ClassifierError::RuntimeLimitExceeded { resource } if bounded(resource) => {
            format!("runtime_{resource}_limit")
        }
        ClassifierError::RuntimeBudgetExceeded { .. } => "runtime_memory_budget".into(),
        ClassifierError::InvalidArtifact(_) => "invalid_artifact".into(),
        ClassifierError::InvalidTrainingData { .. } => "invalid_training_data".into(),
        ClassifierError::BridgeProtocol { .. } => "bridge_protocol_error".into(),
        _ => "candidate_fit_failed".into(),
    }
}

async fn fit_owned(
    owner: &AdmissionOwner,
    home: &ClassifierHome,
    task: ClaimedTask,
    cancel: &Arc<AtomicBool>,
) -> Result<usize, TxError> {
    let mut reply = owner
        .query("SELECT input_json,input_sha256 FROM ONLY $row;")
        .bind(("row", jobs::rid("classifier_learning_job", &task.job_id)))
        .await?
        .check()?;
    let row: InputRow = reply
        .take::<Option<InputRow>>(0)?
        .ok_or_else(|| invalid("learning_input_missing"))?;
    if row.input_json.len() > 32 * 1024 * 1024
        || artifact::sha256_hex(row.input_json.as_bytes()) != row.input_sha256
        || row.input_sha256 != task.input_hash
    {
        return Err(invalid("learning_input_changed"));
    }
    let input: LearningInput =
        serde_json::from_str(&row.input_json).map_err(|_| invalid("learning_input_corrupt"))?;
    let eligible = input.eligible_family_ids.clone();
    let model = ModelId::parse(&format!("learn-{}", &row.input_sha256[..48]))
        .ok_or_else(|| invalid("learning_model_id"))?;
    let control = WorkControl::new();
    let stop = control.cancel.clone();
    let fit_control = control.clone();
    let stage = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (fit_owner, fit_home, fit_task, fit_model, fit_stage) = (
        owner.clone(),
        home.clone(),
        task.clone(),
        model.clone(),
        stage.clone(),
    );
    let runtime = tokio::runtime::Handle::current();
    let mut fit = tokio::task::spawn_blocking(move || {
        super::classifier_learning_fit::fit(
            fit_owner,
            fit_home,
            fit_task,
            input,
            fit_model,
            fit_control,
            fit_stage,
            runtime,
        )
    });
    let mut timer = tokio::time::interval(std::time::Duration::from_secs(5));
    let mut owned = true;
    let outcome = loop {
        tokio::select! {
            value=&mut fit=>break value.map_err(|_|invalid("learning_fit_worker_failed"))?,
            _=timer.tick()=>{
                owned=renew(owner,&task).await.unwrap_or(false);
                let foreground=admission::foreground_requested(owner.repository(),Utc::now()).await.unwrap_or(true);
                if cancel.load(Ordering::Acquire) || !owned || foreground {stop.store(true,Ordering::Release);}
            }
        }
    };
    if !owned {
        return Err(invalid("learning_lease_lost"));
    }
    let failure_stage = [
        "validating",
        "collecting",
        "fitting",
        "calibrating",
        "validating_restore",
        "candidate_written",
        "finished",
    ]
    .get(stage.load(Ordering::Acquire))
    .copied()
    .unwrap_or("unknown");
    let promotion_json = match &outcome {
        Ok(outcome) => encode(&outcome.promotion_evidence, 2 * 1024 * 1024)?,
        Err(_) => String::new(),
    };
    let (terminal, encoded) = match outcome {
        Ok(outcome) => (true, encode(&outcome, 16384)?),
        Err(
            crate::classifier::ClassifierError::Busy
            | crate::classifier::ClassifierError::Cancelled,
        ) => (
            false,
            encode(
                &serde_json::json!({"state":"queued","reason":"foreground_or_writer_checkpoint"}),
                16384,
            )?,
        ),
        Err(error) => (
            true,
            encode(
                &serde_json::json!({"state":"failed","reason":failure_code(&error),"stage":failure_stage,"promoted":false}),
                16384,
            )?,
        ),
    };
    let completed_outcome = serde_json::from_str::<Outcome>(&encoded).ok();
    let success = completed_outcome.is_some();
    let activated = completed_outcome
        .as_ref()
        .is_some_and(|outcome| outcome.promoted == Some(true));
    let uncertain = completed_outcome
        .as_ref()
        .is_some_and(|outcome| outcome.promoted.is_none());
    admission::owned_tx(owner,RetryPolicy::default(),move|tx| {let(task,encoded,eligible,model,promotion_json)=(task.clone(),encoded.clone(),eligible.clone(),model.clone(),promotion_json.clone());async move {
        let now=Utc::now();jobs::bump(&tx,"job_guard",&task.root_id).await?;
        let Some(fence)=jobs::fence(&tx,&task,&now.to_rfc3339()).await? else {return Err(invalid("learning_finish_lease_lost"));};
        if !jobs::holds(&fence,&task) || fence.cancelled {return Err(invalid("learning_finish_lease_lost"));}
        if terminal {jobs::complete_in(&tx,&task,&artifact::sha256_hex(encoded.as_bytes()),&now.to_rfc3339()).await?;}else {
            let next=format!("{}:retry:{}",task.job_id,artifact::sha256_hex(format!("{}:{}",task.id,task.lease_epoch).as_bytes()));
            if jobs::yield_owned_in(&tx,&task,&NewTask {id:&next,job:&task.job_id,pool:POOL,operation:OPERATION,priority:40,deps:vec![&task.id],input_hash:&task.input_hash,created_at:now},now+chrono::Duration::seconds(30),"learning_foreground_checkpoint",now).await? != jobs::Publish::Committed {return Err(invalid("learning_yield_lost"));}
        }
        if success {for family in eligible {tx.query("UPSERT $row SET job_id=$job,learned_at=time::now() RETURN NONE;").bind(("row",jobs::rid("classifier_learning_used",family.as_str()))).bind(("job",task.job_id.clone())).await?.check()?;}}
        tx.query("UPDATE $row SET result_json=$result,promotion_json=$promotion,candidate_model_id=$model,updated_at=time::now() RETURN NONE; UPDATE $job SET state=$state,phase=$phase,stage_coverage_json=$result,progress_done=$done,worker_owner='',finished_at=IF $terminal {time::now()} ELSE {NONE},updated_at=time::now() RETURN NONE;")
            .bind(("row",jobs::rid("classifier_learning_job",&task.job_id))).bind(("job",jobs::rid("job",&task.job_id))).bind(("result",encoded)).bind(("promotion",promotion_json)).bind(("model",if success {model.to_string()}else{String::new()})).bind(("state",if !terminal {"running"}else if success {"completed"}else{"failed"})).bind(("phase",if activated {"Candidate activated"}else if uncertain {"Activation unresolved; inspect classifier status"}else if success {"Candidate retained; prior active"}else if terminal {"Candidate fitting failed"}else{"Learning checkpoint"})).bind(("done",i64::from(success))).bind(("terminal",terminal)).await?.check()?;
        Ok(())
    }}).await.map_err(|_|invalid("learning_finish_failed"))?;
    Ok(1)
}
