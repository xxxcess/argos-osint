//! Narrow requester actions with current proof, ownership and mutation in one TX.
//!
//! API adapters independently require CancelJob/RetryJob and activity access.
//! Membership alone never grants control. Audit receipts do not replace or
//! rewrite immutable creator/request provenance.

use chrono::Duration;
use chrono::Utc;
use fleetworks_contracts::context::RegisteredContext;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use surrealdb::types::SurrealValue;

use super::connect_registry;
use super::job_context::InteractiveAdmission;
use super::job_dispatch;
use super::jobs;
use super::repo::Repository;
use super::setup_jobs;
use super::tx::run_tx;
use super::tx::RetryPolicy;
use super::tx::Tx;
use super::tx::TxError;

/// Additive receipt storage; no original job origin is changed by an action.
pub const SCHEMA: &str = "
DEFINE TABLE IF NOT EXISTS job_action SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS action_key ON job_action TYPE string;
DEFINE FIELD IF NOT EXISTS job_id ON job_action TYPE string;
DEFINE FIELD IF NOT EXISTS action ON job_action TYPE string;
DEFINE FIELD IF NOT EXISTS scope_id ON job_action TYPE string;
DEFINE FIELD IF NOT EXISTS deployment_id ON job_action TYPE string;
DEFINE FIELD IF NOT EXISTS registration_id ON job_action TYPE string;
DEFINE FIELD IF NOT EXISTS workstation_id ON job_action TYPE string;
DEFINE FIELD OVERWRITE workspace_id ON job_action TYPE option<string>;
DEFINE FIELD IF NOT EXISTS request_id ON job_action TYPE string;
DEFINE FIELD IF NOT EXISTS source ON job_action TYPE string;
DEFINE FIELD IF NOT EXISTS context_revision ON job_action TYPE int;
DEFINE FIELD IF NOT EXISTS configuration_epoch ON job_action TYPE string;
DEFINE FIELD IF NOT EXISTS access_epoch ON job_action TYPE string;
DEFINE FIELD IF NOT EXISTS performed_at ON job_action TYPE string;
DEFINE FIELD IF NOT EXISTS receipt_json ON job_action TYPE string;
DEFINE INDEX IF NOT EXISTS job_action_identity ON job_action FIELDS action_key UNIQUE;
";

/// Each action has a separate externally granted permission.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Action {
    Cancel,
    Retry,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cancel => "cancel",
            Self::Retry => "retry",
        }
    }
}

/// Server-side domain availability; API permission checks further narrow it.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ActionAvailability {
    pub job_id: String,
    pub can_cancel: bool,
    pub can_retry: bool,
    pub reason_code: String,
}

/// Durable action acknowledgement, separate from an operation's final outcome.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct ActionReceipt {
    pub action_id: String,
    pub job_id: String,
    pub action: Action,
    pub state: String,
    pub affected_tasks: u64,
}

#[derive(SurrealValue)]
struct Head {
    job_id: String,
    kind: String,
    operation: String,
    state: String,
    is_root: bool,
    cancel_requested: bool,
    stopped: bool,
    owned: bool,
    children: bool,
    task_count: i64,
    task_id: String,
    task_state: String,
    task_pool: String,
    task_operation: String,
    attempts: i64,
    retry_enabled: bool,
}

#[derive(SurrealValue)]
struct SavedAction {
    job_id: String,
    action: String,
    receipt_json: String,
}

fn abort(code: &str) -> TxError {
    TxError::Abort(format!("job_action:{code}"))
}

fn valid(value: &str, limit: usize) -> bool {
    !value.is_empty()
        && value.len() <= limit
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_:.".contains(&byte))
}

fn hash(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

fn head_query() -> &'static str {
    "SELECT record::id(id) AS job_id,kind,operation,state,(parent=NONE) AS is_root,cancel_requested,stopped,
     (type::record('job_origin',record::id(id)).scope_id=$scope
       AND type::record('job_origin',record::id(id)).deployment_id=$deployment
       AND type::record('job_origin',record::id(id)).workstation_id=$node
       AND array::len((SELECT VALUE id FROM job_request WHERE job_id=record::id($parent.id)
         AND scope_id=$scope AND deployment_id=$deployment AND workstation_id=$node LIMIT 1))>0
       AND array::len((SELECT VALUE id FROM job_request WHERE job_id=record::id($parent.id)
         AND (scope_id!=$scope OR deployment_id!=$deployment OR workstation_id!=$node) LIMIT 1))=0) ?? false AS owned,
     (array::len((SELECT VALUE id FROM job WHERE parent=$parent.id LIMIT 1))>0
       OR array::len((SELECT VALUE id FROM job_ancestor WHERE ancestor=$parent.id LIMIT 1))>0) AS children,
     array::len((SELECT VALUE id FROM task WHERE job=$parent.id LIMIT 2)) AS task_count,
     (SELECT VALUE record::id(id) FROM task WHERE job=$parent.id LIMIT 1)[0] ?? '' AS task_id,
     (SELECT VALUE state FROM task WHERE job=$parent.id LIMIT 1)[0] ?? '' AS task_state,
     (SELECT VALUE pool FROM task WHERE job=$parent.id LIMIT 1)[0] ?? '' AS task_pool,
     (SELECT VALUE operation FROM task WHERE job=$parent.id LIMIT 1)[0] ?? '' AS task_operation,
     (SELECT VALUE attempts FROM task WHERE job=$parent.id LIMIT 1)[0] ?? -1 AS attempts,
     (SELECT VALUE enabled FROM job_dispatch_pool WHERE id=type::record('job_dispatch_pool', IF $parent.operation='manual_tool' {'manual_tools'} ELSE {$parent.operation}) LIMIT 1)[0] ?? false AS retry_enabled
     FROM job WHERE id IN $jobs AND kind!='service' AND created_at>=<datetime>$cutoff
       AND !record::exists(type::record('job_retirement',record::id(id))) LIMIT 200"
}

fn pool_for_operation(operation: &str) -> Option<&'static str> {
    match operation {
        job_dispatch::ATLAS_POOL => Some(job_dispatch::ATLAS_POOL),
        "actor_seed" => Some("actor_seed"),
        "classifier_train" => Some("classifier_train"),
        "actor_reindex" => Some("actor_reindex"),
        "manual_tool" => Some("manual_tools"),
        _ => None,
    }
}

fn available(head: &Head) -> ActionAvailability {
    let safe = head.owned
        && head.is_root
        && head.kind != "service"
        && !head.children
        && head.task_count == 1
        && !head.cancel_requested
        && !head.stopped;
    let expected_pool = pool_for_operation(&head.operation);
    let can_cancel = safe
        && expected_pool.is_some_and(|pool| {
            head.task_pool == pool
                && head.task_operation == head.operation
                && head.state == "queued"
                && head.task_state == "queued"
                && head.attempts == 0
        });
    let can_retry =
        safe && expected_pool.is_some_and(|pool| {
            head.task_pool == pool
                && head.task_operation == head.operation
                && head.state == "failed"
                && head.task_state == "failed"
        }) && head.retry_enabled;
    ActionAvailability {
        job_id: head.job_id.clone(),
        can_cancel,
        can_retry,
        reason_code: if !head.owned {
            "shared_or_unowned_work"
        } else if can_cancel || can_retry {
            "supported"
        } else {
            "operation_or_state_not_supported"
        }
        .into(),
    }
}

/// Bounded batch availability after the caller has authorized list membership.
/// No per-row API/storage round trip or unbounded requester ID materialization.
pub async fn availability(
    repo: &Repository,
    authority_scope: &str,
    context: &RegisteredContext,
    ids: &[String],
) -> Result<Vec<ActionAvailability>, TxError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    if ids.len() > 200 || ids.iter().any(|id| !valid(id, 512)) {
        return Err(abort("invalid"));
    }
    let mut response = repo
        .db()
        .query(head_query())
        .bind((
            "jobs",
            ids.iter()
                .map(|id| jobs::rid("job", id))
                .collect::<Vec<_>>(),
        ))
        .bind(("scope", authority_scope.to_owned()))
        .bind(("deployment", context.deployment_id.clone()))
        .bind(("node", context.workstation_id.clone()))
        .bind((
            "cutoff",
            jobs::ts(Utc::now() - Duration::hours(jobs::EXECUTION_WINDOW_HOURS)),
        ))
        .await?
        .check()?;
    let rows: Vec<Head> = response.take(0)?;
    let mut items = Vec::with_capacity(rows.len());
    for head in &rows {
        let mut item = available(head);
        if item.can_retry
            && setup_jobs::is_setup_operation(&head.operation)
            && !setup_jobs::retry_ready_db(repo.db(), &head.job_id, &head.operation).await?
        {
            item.can_retry = false;
            item.reason_code = "operation_or_state_not_supported".into();
        }
        items.push(item);
    }
    Ok(items)
}

async fn proof_in(tx: &Tx, admission: &InteractiveAdmission) -> Result<(), TxError> {
    let expected = admission.context();
    let current = match admission.web_proof() {
        Some((token, revision)) => {
            connect_registry::validate_session_for_admission_in(
                tx,
                admission.authority(),
                token,
                revision,
            )
            .await?
            .context
        }
        None => {
            connect_registry::validate_registration_in(
                tx,
                admission.authority(),
                &expected.registration_id,
                expected.registration_revision,
            )
            .await?
        }
    };
    if !current.same_request_context(expected) {
        return Err(abort("context_changed"));
    }
    Ok(())
}

/// Perform one supported action under the current native/web proof and job guard.
/// The trusted adapter must require the action's explicit permission separately
/// from ReadWorkstationActivity before calling this function.
pub async fn perform(
    repo: &Repository,
    admission: &InteractiveAdmission,
    action: Action,
    job_id: &str,
) -> Result<ActionReceipt, TxError> {
    let request_id = &admission.origin().request_id;
    if !valid(job_id, 512) || !valid(request_id, 128) {
        return Err(abort("invalid"));
    }
    let context = admission.context();
    let key = hash(&[
        admission.authority().scope_id(),
        &context.deployment_id,
        &context.registration_id,
        action.as_str(),
        request_id,
    ]);
    let receipt=run_tx(repo.db(),RetryPolicy::default(),|tx|{
        let key=&key;
        async move {
            proof_in(&tx,admission).await?;
            jobs::bump(&tx,"job_guard",job_id).await?;
            let mut response=jobs::run(&tx,tx.query(head_query()).bind(("jobs",vec![jobs::rid("job",job_id)]))
                .bind(("scope",admission.authority().scope_id().to_owned())).bind(("deployment",context.deployment_id.clone()))
                .bind(("node",context.workstation_id.clone()))
                .bind(("cutoff",jobs::ts(Utc::now()-Duration::hours(jobs::EXECUTION_WINDOW_HOURS))))).await?;
            let heads:Vec<Head>=jobs::take(&mut response,0)?;
            let head=heads.into_iter().next().ok_or_else(||abort("unavailable"))?;
            if !head.owned {return Err(abort("forbidden"));}
            let mut response=jobs::run(&tx,tx.query("SELECT job_id,action,receipt_json FROM ONLY $record")
                .bind(("record",jobs::rid("job_action",key)))).await?;
            if let Some(saved)=jobs::take::<Option<SavedAction>>(&mut response,0)? {
                if saved.job_id!=job_id||saved.action!=action.as_str(){return Err(abort("request_conflict"));}
                return serde_json::from_str(&saved.receipt_json).map_err(|_|abort("receipt_unavailable"));
            }
            let availability=available(&head);
            if !(match action {Action::Cancel=>availability.can_cancel,Action::Retry=>availability.can_retry}) {return Err(abort("unsupported"));}
            let now=Utc::now();
            let at=jobs::ts(now);
            let state=match action {
                Action::Cancel=>{
                    // Exactly one never-claimed queued task; there is no live
                    // provider/worker to mislabel as already stopped.
                    jobs::run(&tx,tx.query("UPDATE $task SET state='cancelled',lease_epoch+=1,finished_at=<datetime>$now,updated_at=<datetime>$now RETURN NONE;
                        UPDATE $job SET state='cancelled',cancel_requested=true,stopped=true,phase='Cancelled before execution',finished_at=<datetime>$now,updated_at=<datetime>$now RETURN NONE;")
                        .bind(("task",jobs::rid("task",&head.task_id))).bind(("job",jobs::rid("job",job_id))).bind(("now",at.clone()))).await?;
                    "cancelled"
                }
                Action::Retry=>{
                    let pool=pool_for_operation(&head.operation).ok_or_else(||abort("unsupported"))?;
                    if setup_jobs::is_setup_operation(&head.operation)
                        && !setup_jobs::claim_retry_resource_in(&tx,job_id,&head.operation).await?
                    {
                        return Err(abort("unsupported"));
                    }
                    if !job_dispatch::retry_failed_task_in(&tx,job_id,&head.task_id,pool,&head.operation,now).await? {return Err(abort("unsupported"));}
                    "queued"
                }
            };
            let receipt=ActionReceipt{action_id:format!("job-action-{key}"),job_id:job_id.to_owned(),action,state:state.into(),affected_tasks:1};
            let source=match admission.origin().source {fleetworks_contracts::context::OriginSource::Native=>"native",fleetworks_contracts::context::OriginSource::IntelWeb=>"intel_web",_=>return Err(abort("invalid"))};
            jobs::run(&tx,tx.query("CREATE $record SET action_key=$key,job_id=$job,action=$action,scope_id=$scope,deployment_id=$deployment,registration_id=$registration,workstation_id=$node,request_id=$request,source=$source,context_revision=$revision,configuration_epoch=$configuration,access_epoch=$principal_epoch,performed_at=$at,receipt_json=$receipt RETURN NONE")
                .bind(("record",jobs::rid("job_action",key))).bind(("key",key.clone())).bind(("job",job_id.to_owned())).bind(("action",action.as_str()))
                .bind(("scope",admission.authority().scope_id().to_owned())).bind(("deployment",context.deployment_id.clone())).bind(("registration",context.registration_id.clone()))
                .bind(("node",context.workstation_id.clone())).bind(("request",request_id.clone())).bind(("source",source))
                .bind(("revision",context.context_revision as i64)).bind(("configuration",context.configuration_epoch.clone())).bind(("principal_epoch",context.access_epoch.clone()))
                .bind(("at",at.clone())).bind(("receipt",serde_json::to_string(&receipt).map_err(|_|abort("invalid"))?))).await?;
            jobs::log_transition_in(&tx,job_id,&receipt.action_id,&format!("job.{}",action.as_str()),
                if action==Action::Cancel {"Cancelled before first execution"}else{"Queued for retry"},&at).await?;
            Ok(receipt)
        }
    }).await?;
    Ok(receipt.value)
}
