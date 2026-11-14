//! Durable explicit Natural candidate activation and operation-owned effects.
//! The API must enforce MaintainBackend and private registered job eligibility.
//! Admission, requester origin and dispatch outbox commit together. The shared
//! natural_classifier resource excludes train/activation across Connect IDs.

mod source;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use chrono::Utc;
use fleetworks_contracts::classifier_activation::ActivationReceipt;
use fleetworks_contracts::classifier_activation::ActivationRequest;
use fleetworks_contracts::classifier_activation::ActivationResult;
use fleetworks_contracts::classifier_activation::ActivationState;
use fleetworks_contracts::context::RegisteredContext;
use surrealdb::types::SurrealValue;

use super::job_context::InteractiveAdmission;
use super::job_dispatch;
use super::jobs;
use super::jobs::ClaimedTask;
use super::jobs::NewJob;
use super::jobs::NewTask;
use super::repo::Repository;
use super::tx::run_tx;
use super::tx::RetryPolicy;
use super::tx::Tx;
use super::tx::TxError;
use crate::classifier::artifact::ArtifactStore;
use crate::classifier::candidate_activation;

pub const SCHEMA: &str = include_str!("schema/classifier_activation.surql");
pub const POOL: &str = "classifier_activate";
pub const OPERATION: &str = "classifier_activate";
pub const RECONCILE_OPERATION: &str = "classifier_activation_reconcile";

fn invalid(reason: &str) -> TxError {
    TxError::Abort(format!("classifier_activation:{reason}"))
}
fn key(values: &[&str]) -> String {
    crate::classifier::artifact::sha256_hex(
        &values
            .iter()
            .map(|value| format!("{}:{value}", value.len()))
            .collect::<String>()
            .into_bytes(),
    )
}
#[derive(SurrealValue)]
struct Operation {
    scope_id: String,
    request_json: String,
    request_hash: String,
    receipt_json: String,
    state: String,
    result_json: String,
}
#[derive(SurrealValue)]
struct Resource {
    job_id: String,
    active: bool,
}

/// Enable only after the actual consumer and its queue transport are serving.
pub async fn configure_consumer(repo: &Repository, enabled: bool) -> Result<(), TxError> {
    job_dispatch::configure_pool(repo.db(), POOL, enabled).await?;
    repo.db()
        .query("UPSERT $record SET enabled=$enabled RETURN NONE")
        .bind(("record", jobs::rid("setup_dispatch", POOL)))
        .bind(("enabled", enabled))
        .await?
        .check()?;
    Ok(())
}
async fn operation_in(tx: &Tx, id: &str) -> Result<Option<Operation>, TxError> {
    let mut reply=jobs::run(tx,tx.query("SELECT scope_id,request_json,request_hash,receipt_json,state,result_json FROM ONLY $record").bind(("record",jobs::rid("classifier_activation_operation",id)))).await?;
    jobs::take(&mut reply, 0)
}

/// Call under setup_resource:natural_classifier's revision guard in train
/// admission too. A failed Job does not resolve an uncertain filesystem effect.
pub(crate) async fn require_unambiguous_resource_in(tx: &Tx) -> Result<(), TxError> {
    let mut reply = jobs::run(
        tx,
        tx.query("SELECT VALUE job_id FROM ONLY setup_resource:natural_classifier"),
    )
    .await?;
    if let Some(id) = jobs::take::<Option<String>>(&mut reply, 0)?.filter(|id| !id.is_empty()) {
        if operation_in(tx, &id)
            .await?
            .is_some_and(|row| matches!(row.state.as_str(), "running" | "effect_uncertain"))
        {
            return Err(invalid("resource_busy"));
        }
    }
    Ok(())
}

/// Eligible training identity without placeholder artifact hashes.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct TrainingCandidateSource {
    pub model_id: String,
    pub training_job_id: String,
    pub dataset_sha256: String,
}

/// Readiness metadata, not a global activity feed. Authorize the optional job
/// ID before exposing a link; an uncertain effect keeps train/activation busy.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct NaturalResourceObservation {
    pub consumer_enabled: bool,
    pub blocked: bool,
    pub effect_uncertain: bool,
    pub blocking_job_id: Option<String>,
}

/// Read only: never acquires a writer, loads a model or changes resource state.
pub async fn observation(repo: &Repository) -> Result<NaturalResourceObservation, TxError> {
    Ok(run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        jobs::run(&tx, tx.query("INFO FOR TABLE classifier_activation_operation; INFO FOR TABLE setup_dispatch; INFO FOR TABLE setup_resource;")).await?;
        let mut enabled = jobs::run(&tx, tx.query("SELECT VALUE enabled FROM ONLY $record").bind(("record", jobs::rid("setup_dispatch", POOL)))).await?;
        let consumer_enabled = jobs::take::<Option<bool>>(&mut enabled, 0)? == Some(true);
        let mut resource = jobs::run(&tx, tx.query("SELECT job_id, IF job_id!='' { ((type::record('job',job_id).state ?? '') NOT IN ['completed','succeeded','failed','partial','cancelled','superseded','stopped',''] OR array::len((SELECT VALUE id FROM task WHERE job=type::record('job',$parent.job_id) AND state NOT IN ['completed','failed','cancelled','superseded','stopped','skipped'] LIMIT 1))>0) } ELSE { false } AS active FROM ONLY setup_resource:natural_classifier")).await?;
        let resource: Option<Resource> = jobs::take(&mut resource, 0)?;
        let mut effect_uncertain = false;
        if let Some(resource) = &resource {
            if !resource.job_id.is_empty() {
                effect_uncertain = operation_in(&tx, &resource.job_id).await?.is_some_and(|row| row.state == "effect_uncertain");
            }
        }
        let blocked = resource.as_ref().is_some_and(|resource| resource.active) || effect_uncertain;
        Ok(NaturalResourceObservation { consumer_enabled, blocked, effect_uncertain,
            blocking_job_id: resource.filter(|resource| blocked && !resource.job_id.is_empty()).map(|resource| resource.job_id) })
    }).await?.value)
}

/// Selected durable training metadata is returned only after exact requester
/// membership and private eligibility, before touching any candidate/report.
pub async fn training_source(
    repo: &Repository,
    scope: &str,
    context: &RegisteredContext,
    job_id: &str,
) -> Result<TrainingCandidateSource, TxError> {
    let source = run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        source::training_in(&tx, scope, context, job_id).await
    })
    .await?
    .value;
    Ok(TrainingCandidateSource {
        model_id: source.model_id,
        training_job_id: job_id.into(),
        dataset_sha256: source.dataset_sha256,
    })
}

pub async fn admit(
    repo: &Repository,
    admission: &InteractiveAdmission,
    request: &ActivationRequest,
) -> Result<ActivationReceipt, TxError> {
    request.validate().map_err(|_| invalid("request_invalid"))?;
    if request.synthetic_baseline != fleetworks_contracts::classifier_activation::SyntheticBaselineConsent::ActivateSyntheticBaseline {
        return Err(invalid("consent_required"));
    }
    if request.request_id != admission.origin().request_id {
        return Err(invalid("origin_changed"));
    }
    let id = format!(
        "activate-{}",
        key(&[
            admission.authority().scope_id(),
            &admission.context().deployment_id,
            &admission.context().registration_id,
            &request.request_id
        ])
    );
    let encoded = serde_json::to_string(request).map_err(|_| invalid("request_invalid"))?;
    let hash = candidate_activation::input_hash(request).map_err(|_| invalid("request_invalid"))?;
    Ok(run_tx(repo.db(),RetryPolicy::default(),|tx|{
        let id=&id;let encoded=&encoded;let hash=&hash;
        async move {
            if let Some(saved)=operation_in(&tx,id).await? {
                if saved.scope_id!=admission.authority().scope_id()||saved.request_hash!=*hash {return Err(invalid("request_conflict"));}
                admission.record_in(&tx,id,OPERATION,false).await?;
                let mut receipt:ActivationReceipt=serde_json::from_str(&saved.receipt_json).map_err(|_|invalid("receipt_unavailable"))?;receipt.reused=true;return Ok(receipt);
            }
            let context=admission.context();
            if request.context.registration_id!=context.registration_id||request.context.expected_registration_revision!=context.registration_revision
                ||request.context.expected_context_revision!=context.context_revision||request.context.expected_configuration_revision!=Some(admission.origin().configuration_revision) {return Err(invalid("context_changed"));}
            let source=source::training_in(&tx,admission.authority().scope_id(),context,&request.candidate.training_job_id).await?;
            if source.model_id!=request.candidate.model_id||source.dataset_sha256!=request.candidate.dataset_sha256 {return Err(invalid("source_changed"));}
            let mut enabled=jobs::run(&tx,tx.query("SELECT VALUE enabled FROM ONLY $record").bind(("record",jobs::rid("setup_dispatch",POOL)))).await?;
            if jobs::take::<Option<bool>>(&mut enabled,0)?!=Some(true) {return Err(invalid("consumer_unavailable"));}
            jobs::run(&tx,tx.query("UPSERT setup_resource:natural_classifier SET revision=(revision ?? 0)+1 RETURN NONE")).await?;
            require_unambiguous_resource_in(&tx).await?;
            let mut resource=jobs::run(&tx,tx.query("SELECT job_id, IF job_id!='' { ((type::record('job',job_id).state ?? '') NOT IN ['completed','succeeded','failed','partial','cancelled','superseded','stopped',''] OR array::len((SELECT VALUE id FROM task WHERE job=type::record('job',$parent.job_id) AND state NOT IN ['completed','failed','cancelled','superseded','stopped','skipped'] LIMIT 1))>0) } ELSE { false } AS active FROM ONLY setup_resource:natural_classifier")).await?;
            if jobs::take::<Option<Resource>>(&mut resource,0)?.is_some_and(|resource|!resource.job_id.is_empty()&&resource.active) {return Err(invalid("resource_busy"));}
            let task_id=format!("{id}-execute");let now=Utc::now();
            let created=job_dispatch::submit_in(&tx,&NewJob{id,kind:"maintenance",operation:OPERATION,parent:None,created_at:now},&NewTask{id:&task_id,job:id,pool:POOL,operation:OPERATION,priority:0,deps:Vec::new(),input_hash:hash,created_at:now}).await?;
            if !created {return Err(invalid("job_collision"));}
            admission.record_in(&tx,id,OPERATION,true).await?;
            let receipt=ActivationReceipt::new(id.clone(),admission.origin().clone(),request,false);
            jobs::run(&tx,tx.query("CREATE $record SET scope_id=$scope,request_json=$request,request_hash=$hash,receipt_json=$receipt,state='queued',updated_at=time::now() RETURN NONE;
                UPDATE setup_resource:natural_classifier SET job_id=$job,payload_hash=$hash,scope_id=$scope RETURN NONE;")
                .bind(("record",jobs::rid("classifier_activation_operation",id))).bind(("scope",admission.authority().scope_id().to_owned())).bind(("request",encoded.clone())).bind(("hash",hash.clone()))
                .bind(("receipt",serde_json::to_string(&receipt).map_err(|_|invalid("receipt_invalid"))?)).bind(("job",id.clone()))).await?;
            Ok(receipt)
        }
    }).await?.value)
}

async fn live(tx: &Tx, task: &ClaimedTask) -> Result<bool, TxError> {
    jobs::bump(tx, "job_guard", &task.root_id).await?;
    Ok(jobs::fence(tx, task, &Utc::now().to_rfc3339())
        .await?
        .is_some_and(|fence| jobs::holds(&fence, task) && !fence.cancelled))
}
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ActivationWork {
    Complete { result_ref: String },
    Failed { reason: &'static str },
    Stale,
}

/// Host renews the dispatch lease and finishes the actual task/outbox. This
/// performs one effect or reconciles its own proof, never an in-memory accept.
pub async fn execute_claimed(
    repo: &Repository,
    home: PathBuf,
    task: &ClaimedTask,
    cancel: Arc<AtomicBool>,
) -> Result<ActivationWork, TxError> {
    if task.pool != POOL || task.operation != OPERATION {
        return Err(invalid("wrong_consumer"));
    }
    let start=run_tx(repo.db(),RetryPolicy::default(),|tx|async move {
        if !live(&tx,task).await? {return Ok(None);}
        let row=operation_in(&tx,&task.job_id).await?.ok_or_else(||invalid("payload_missing"))?;
        if row.request_hash!=task.input_hash {return Err(invalid("payload_changed"));}
        let request:ActivationRequest=serde_json::from_str(&row.request_json).map_err(|_|invalid("payload_invalid"))?;
        if candidate_activation::input_hash(&request).map_err(|_|invalid("payload_invalid"))?!=row.request_hash {return Err(invalid("payload_changed"));}
        let was_started=row.state!="queued";
        if row.result_json.is_empty() {
            jobs::run(&tx,tx.query("UPDATE $record SET state='running',started_at=started_at ?? time::now(),updated_at=time::now() RETURN NONE")
                .bind(("record",jobs::rid("classifier_activation_operation",&task.job_id)))).await?;
        }
        Ok(Some((request,was_started,row.result_json)))
    }).await?.value;
    let Some((request, was_started, saved)) = start else {
        return Ok(ActivationWork::Stale);
    };
    if !saved.is_empty() {
        let result: ActivationResult =
            serde_json::from_str(&saved).map_err(|_| invalid("result_invalid"))?;
        return Ok(outcome(&result));
    }
    let job_id = task.job_id.clone();
    let expected = request.clone();
    let result = tokio::task::spawn_blocking(move || {
        let store =
            ArtifactStore::new(&home).map_err(candidate_activation::ActivationFailure::from)?;
        let lease = store
            .try_writer()
            .map_err(candidate_activation::ActivationFailure::from)?;
        match candidate_activation::reconcile(&store, &job_id, &expected) {
            Ok(Some(result)) => return Ok(result),
            Ok(None) => {}
            Err(_) => {
                return Ok(candidate_activation::uncertain(
                    &job_id,
                    &expected,
                    "activation_effect_proof_unavailable",
                ))
            }
        }
        if was_started {
            return Ok(candidate_activation::uncertain(
                &job_id,
                &expected,
                "activation_previous_effect_uncertain",
            ));
        }
        candidate_activation::execute(&store, &lease, &job_id, &expected, &cancel)
    })
    .await;
    let result = match result {
        Ok(Ok(result)) => result,
        Err(_) => candidate_activation::uncertain(
            &task.job_id,
            &request,
            "activation_worker_effect_uncertain",
        ),
        Ok(Err(_)) if was_started => candidate_activation::uncertain(
            &task.job_id,
            &request,
            "activation_previous_effect_unavailable",
        ),
        Ok(Err(error)) => ActivationResult::new(
            task.job_id.clone(),
            request.candidate.clone(),
            if error == candidate_activation::ActivationFailure::Interrupted {
                ActivationState::Interrupted
            } else {
                ActivationState::Rejected
            },
            None,
            error.reason_code().into(),
        ),
    };
    persist(repo, task, &result).await
}
fn outcome(result: &ActivationResult) -> ActivationWork {
    if result.state == ActivationState::Activated {
        ActivationWork::Complete {
            result_ref: format!("classifier-activation:{}", result.job_id),
        }
    } else {
        ActivationWork::Failed {
            reason: if result.state == ActivationState::EffectUncertain {
                "activation_effect_uncertain"
            } else {
                "activation_rejected_or_interrupted"
            },
        }
    }
}
async fn persist(
    repo: &Repository,
    task: &ClaimedTask,
    result: &ActivationResult,
) -> Result<ActivationWork, TxError> {
    let encoded = serde_json::to_string(result).map_err(|_| invalid("result_invalid"))?;
    let state = match result.state {
        ActivationState::Activated => "activated",
        ActivationState::EffectUncertain => "effect_uncertain",
        ActivationState::Interrupted => "interrupted",
        _ => "rejected",
    };
    let saved=run_tx(repo.db(),RetryPolicy::default(),|tx|{let encoded=&encoded;async move {
        if !live(&tx,task).await? {return Ok(false);}
        jobs::run(&tx,tx.query("UPDATE $record SET state=$state,result_json=$result,updated_at=time::now() RETURN NONE; UPDATE $job SET phase=$phase,progress_done=1,progress_total=1 RETURN NONE;")
            .bind(("record",jobs::rid("classifier_activation_operation",&task.job_id))).bind(("state",state)).bind(("result",encoded.clone())).bind(("job",jobs::rid("job",&task.job_id))).bind(("phase",if state=="activated" {"Natural candidate activated"}else if state=="effect_uncertain" {"Activation effect requires reconciliation"}else{"Prior Natural model retained"}))).await?;Ok(true)
    }}).await?.value;
    Ok(if saved {
        outcome(result)
    } else {
        ActivationWork::Stale
    })
}

/// Call only after current activity membership/privacy and separate permission.
pub async fn result_for_job(
    repo: &Repository,
    job_id: &str,
) -> Result<Option<ActivationResult>, TxError> {
    let row = run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        operation_in(&tx, job_id).await
    })
    .await?
    .value;
    row.filter(|row| !row.result_json.is_empty())
        .map(|row| serde_json::from_str(&row.result_json).map_err(|_| invalid("result_invalid")))
        .transpose()
}
