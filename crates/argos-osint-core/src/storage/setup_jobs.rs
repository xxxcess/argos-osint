//! Immutable staged dataset references and durable backend maintenance admission.
//!
//! Bytes are validated/staged by the server before registration here. Admission
//! pins those bytes, submits the existing Jobs/outbox and records origin in one
//! SurrealDB transaction. No app lifetime, shell process or in-memory queue is
//! an acceptance guarantee. Operation enablement is explicit and defaults off.

use std::time::Duration;

use chrono::Utc;
use fleetworks_contracts::context::JobOrigin;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::readiness::DatasetKind;
use fleetworks_contracts::readiness::DatasetRef;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::MaintenanceReceipt;
use fleetworks_contracts::readiness::MaintenanceRequest;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use surrealdb::engine::any::Any;
use surrealdb::types::Duration as DbDuration;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use super::connect_registry::RegistryAuthority;
use super::job_context;
use super::job_dispatch;
use super::jobs;
use super::jobs::NewJob;
use super::jobs::NewTask;
use super::repo::Repository;
use super::tx::run_tx;
use super::tx::RetryPolicy;
use super::tx::Tx;
use super::tx::TxError;

pub const SCHEMA: &str = include_str!("schema/setup_jobs.surql");
pub const ACTOR_DATASET_LIMIT: u64 = 256 * 1024 * 1024;
pub const TRAINING_DATASET_LIMIT: u64 = crate::classifier::dataset::MAX_CSV_BYTES;
pub const DATASET_TTL: Duration = Duration::from_secs(24 * 60 * 60);
pub const SEED_POOL: &str = "actor_seed";
pub const TRAIN_POOL: &str = "classifier_train";
pub const REINDEX_POOL: &str = "actor_reindex";

/// Small durable receipt observation; no catalog scan or mutation.
#[derive(Clone, Debug, SurrealValue)]
pub struct SeedObservation {
    pub status: String,
    pub processed: i64,
    pub total: i64,
}

pub async fn actor_seed_observation(repo: &Repository) -> Result<Option<SeedObservation>, TxError> {
    let mut response = repo
        .db()
        .query("SELECT status,processed,total FROM ONLY brain_actor_seed:catalog")
        .await?
        .check()?;
    Ok(response.take(0)?)
}

pub fn dataset_limit(kind: DatasetKind) -> u64 {
    match kind {
        DatasetKind::ActorCatalog => ACTOR_DATASET_LIMIT,
        DatasetKind::ClassifierTraining => TRAINING_DATASET_LIMIT,
        _ => 0,
    }
}

pub fn operation_name(operation: MaintenanceOperation) -> &'static str {
    match operation {
        MaintenanceOperation::SeedActors => "actor_seed",
        MaintenanceOperation::TrainClassifier => "classifier_train",
        MaintenanceOperation::ReindexActors => "actor_reindex",
        _ => "unsupported",
    }
}

fn resource_name(operation: MaintenanceOperation) -> &'static str {
    match operation {
        MaintenanceOperation::SeedActors => "actor_catalog",
        MaintenanceOperation::TrainClassifier => "natural_classifier",
        MaintenanceOperation::ReindexActors => "actor_index",
        _ => "unsupported",
    }
}

fn dataset_kind(kind: DatasetKind) -> &'static str {
    match kind {
        DatasetKind::ActorCatalog => "actor_catalog",
        DatasetKind::ClassifierTraining => "classifier_training",
        _ => "unsupported",
    }
}

fn invalid(code: &str) -> TxError {
    TxError::Abort(format!("setup:{code}"))
}

/// Journal ownership/expiry before receiving bytes so cancellation is recoverable.
pub async fn begin_upload(
    repo: &Repository,
    authority: &RegistryAuthority,
    id: &str,
    kind: DatasetKind,
) -> Result<(), TxError> {
    if DatasetRef::new(id.into(), kind, "0".repeat(64), 1).is_err() || dataset_limit(kind) == 0 {
        return Err(invalid("dataset_invalid"));
    }
    repo.db().query("CREATE $row SET dataset_id=$id,scope_id=$scope,kind=$kind,state='partial',expires_at=time::now()+$ttl RETURN NONE")
        .bind(("row",jobs::rid("setup_upload",id))).bind(("id",id.to_owned())).bind(("scope",authority.scope_id().to_owned()))
        .bind(("kind",dataset_kind(kind))).bind(("ttl",DbDuration::from(DATASET_TTL))).await?.check()?;
    Ok(())
}

fn digest(parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

#[derive(SurrealValue)]
struct DatasetRow {
    dataset_id: String,
    scope_id: String,
    kind: String,
    sha256: String,
    size_bytes: i64,
    validated: bool,
    live: bool,
}

fn reference(row: &DatasetRow) -> Result<DatasetRef, TxError> {
    let kind = match row.kind.as_str() {
        "actor_catalog" => DatasetKind::ActorCatalog,
        "classifier_training" => DatasetKind::ClassifierTraining,
        _ => return Err(invalid("dataset_kind")),
    };
    if !row.validated || row.size_bytes <= 0 || row.size_bytes as u64 > dataset_limit(kind) {
        return Err(invalid("dataset_invalid"));
    }
    DatasetRef::new(
        row.dataset_id.clone(),
        kind,
        row.sha256.clone(),
        row.size_bytes as u64,
    )
    .map_err(|_| invalid("dataset_invalid"))
}

async fn dataset_in(tx: &Tx, id: &str) -> Result<DatasetRow, TxError> {
    let mut result = jobs::run(tx, tx.query("SELECT dataset_id, scope_id, kind, sha256, size_bytes, validated, (expires_at > time::now()) AS live FROM ONLY $row")
        .bind(("row", jobs::rid("setup_dataset", id)))).await?;
    jobs::take::<Option<DatasetRow>>(&mut result, 0)?.ok_or_else(|| invalid("dataset_unavailable"))
}

/// Persist only server-validated immutable bytes; request payloads cannot call this.
pub async fn register_dataset(
    repo: &Repository,
    authority: &RegistryAuthority,
    dataset: &DatasetRef,
    ttl: Duration,
) -> Result<(), TxError> {
    dataset.validate().map_err(|_| invalid("dataset_invalid"))?;
    if dataset.size_bytes > dataset_limit(dataset.kind) || ttl.is_zero() || ttl > DATASET_TTL {
        return Err(invalid("dataset_limits"));
    }
    run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        let record = jobs::rid("setup_dataset", &dataset.id);
        let mut existing = jobs::run(&tx, tx.query("SELECT dataset_id, scope_id, kind, sha256, size_bytes, validated, (expires_at > time::now()) AS live FROM ONLY $row").bind(("row", record.clone()))).await?;
        if let Some(row) = jobs::take::<Option<DatasetRow>>(&mut existing, 0)? {
            if row.scope_id == authority.scope_id() && reference(&row)? == *dataset { return Ok(()); }
            return Err(invalid("dataset_identity_conflict"));
        }
        jobs::run(&tx, tx.query("CREATE $row SET dataset_id=$id, scope_id=$scope, kind=$kind, sha256=$hash, size_bytes=$size, validated=true, created_at=time::now(), expires_at=time::now()+$ttl RETURN NONE")
            .bind(("row", record)).bind(("id", dataset.id.clone())).bind(("scope", authority.scope_id().to_owned())).bind(("kind", dataset_kind(dataset.kind)))
            .bind(("hash", dataset.sha256.to_ascii_lowercase())).bind(("size", dataset.size_bytes as i64)).bind(("ttl", DbDuration::from(ttl)))).await?;
        jobs::run(&tx,tx.query("UPDATE $upload SET state='validated',expires_at=time::now()+$ttl WHERE scope_id=$scope RETURN NONE")
            .bind(("upload",jobs::rid("setup_upload",&dataset.id))).bind(("scope",authority.scope_id().to_owned())).bind(("ttl",DbDuration::from(ttl)))).await?;
        Ok(())
    }).await?;
    Ok(())
}

/// Resolve a permitted unexpired opaque reference before interactive admission.
pub async fn dataset(
    repo: &Repository,
    authority: &RegistryAuthority,
    id: &str,
) -> Result<DatasetRef, TxError> {
    let result = run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        let row = dataset_in(&tx, id).await?;
        if row.scope_id != authority.scope_id() || !row.live {
            return Err(invalid("dataset_unavailable"));
        }
        reference(&row)
    })
    .await?;
    Ok(result.value)
}

/// List only permitted unexpired metadata; never scan user/server directories.
pub async fn list_datasets(
    repo: &Repository,
    authority: &RegistryAuthority,
) -> Result<(Vec<DatasetRef>, bool), TxError> {
    let mut result=repo.db().query("SELECT dataset_id,scope_id,kind,sha256,size_bytes,validated,true AS live FROM setup_dataset WHERE scope_id=$scope AND validated=true AND expires_at>time::now() ORDER BY dataset_id LIMIT 101")
        .bind(("scope",authority.scope_id().to_owned())).await?.check()?;
    let mut rows: Vec<DatasetRow> = result.take(0)?;
    let truncated = rows.len() > 100;
    rows.truncate(100);
    Ok((
        rows.iter().map(reference).collect::<Result<Vec<_>, _>>()?,
        truncated,
    ))
}

/// Exact server-owned expired staging object, not an arbitrary cleanup path.
#[derive(Clone, Debug, SurrealValue)]
pub struct ExpiredUpload {
    pub dataset_id: String,
}

/// Claim a bounded expired/unpinned set. Dataset-row writes fence simultaneous
/// admission pinning; accepted inputs are retained independently of upload TTL.
pub async fn claim_expired_uploads(repo: &Repository) -> Result<Vec<ExpiredUpload>, TxError> {
    let result=run_tx(repo.db(),RetryPolicy::default(),|tx|async move {
        let mut result=jobs::run(&tx,tx.query("SELECT dataset_id FROM setup_upload WHERE expires_at<=time::now() AND state IN ['partial','validated','cleanup_pending'] ORDER BY dataset_id LIMIT 32")).await?;
        let candidates:Vec<ExpiredUpload>=jobs::take(&mut result,0)?;
        let mut selected=Vec::new();
        for candidate in candidates {
            let dataset=jobs::rid("setup_dataset",&candidate.dataset_id);
            jobs::run(&tx,tx.query("UPDATE $row SET pin_revision+=1 RETURN NONE").bind(("row",dataset.clone()))).await?;
            let mut pins=jobs::run(&tx,tx.query("SELECT VALUE job_id FROM setup_dataset_pin WHERE dataset_id=$id LIMIT 1").bind(("id",candidate.dataset_id.clone()))).await?;
            if !jobs::take::<Vec<String>>(&mut pins,0)?.is_empty(){continue;}
            jobs::run(&tx,tx.query("UPDATE $dataset SET validated=false RETURN NONE; UPDATE $upload SET state='cleanup_pending' RETURN NONE")
                .bind(("dataset",dataset)).bind(("upload",jobs::rid("setup_upload",&candidate.dataset_id)))).await?;
            selected.push(candidate);
        }
        Ok(selected)
    }).await?;
    Ok(result.value)
}

pub async fn finish_upload_cleanup(repo: &Repository, id: &str) -> Result<(), TxError> {
    repo.db()
        .query("UPDATE $row SET state='cleaned' WHERE state='cleanup_pending' RETURN NONE")
        .bind(("row", jobs::rid("setup_upload", id)))
        .await?
        .check()?;
    Ok(())
}

/// A queued job's durable pin retains immutable bytes beyond upload expiry.
pub async fn dataset_for_job(
    repo: &Repository,
    job_id: &str,
    expected: &DatasetRef,
) -> Result<DatasetRef, TxError> {
    expected
        .validate()
        .map_err(|_| invalid("dataset_invalid"))?;
    let result = run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        let mut pin = jobs::run(
            &tx,
            tx.query("SELECT VALUE job_id FROM ONLY $pin").bind((
                "pin",
                jobs::rid("setup_dataset_pin", &digest(&[&expected.id, job_id])),
            )),
        )
        .await?;
        if jobs::take::<Option<String>>(&mut pin, 0)?.as_deref() != Some(job_id) {
            return Err(invalid("dataset_not_pinned"));
        }
        let row = dataset_in(&tx, &expected.id).await?;
        let actual = reference(&row)?;
        if actual != *expected {
            return Err(invalid("dataset_identity_conflict"));
        }
        Ok(actual)
    })
    .await?;
    Ok(result.value)
}

/// Explicit operator hook, enabled only after its real durable consumer exists.
pub async fn enable_operation(
    db: &Surreal<Any>,
    operation: MaintenanceOperation,
    enabled: bool,
) -> Result<(), TxError> {
    if operation_name(operation) == "unsupported" {
        return Err(invalid("unsupported_operation"));
    }
    db.query("UPSERT $row SET enabled=$enabled RETURN NONE")
        .bind((
            "row",
            jobs::rid("setup_dispatch", operation_name(operation)),
        ))
        .bind(("enabled", enabled))
        .await?
        .check()?;
    Ok(())
}

/// Whether the operator enabled a real consumer; missing schema/config is not success.
pub async fn operation_enabled(
    repo: &Repository,
    operation: MaintenanceOperation,
) -> Result<bool, TxError> {
    let mut result = repo
        .db()
        .query("SELECT VALUE enabled FROM ONLY $row")
        .bind((
            "row",
            jobs::rid("setup_dispatch", operation_name(operation)),
        ))
        .await?
        .check()?;
    Ok(result.take::<Option<bool>>(0)? == Some(true))
}

/// Worker payload contains immutable references, never raw keys or user paths.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetupPayload {
    pub operation: MaintenanceOperation,
    pub dataset: Option<DatasetRef>,
}

#[derive(SurrealValue)]
struct ResourceRow {
    job_id: String,
    payload_hash: String,
    scope_id: String,
    active: bool,
}

#[derive(SurrealValue)]
struct RequestRow {
    payload_hash: String,
    receipt_json: String,
}

async fn save_receipt_in(
    tx: &Tx,
    request_key: &str,
    payload_hash: &str,
    receipt: &MaintenanceReceipt,
) -> Result<(), TxError> {
    let encoded = serde_json::to_string(receipt).map_err(|_| invalid("receipt_invalid"))?;
    jobs::run(
        tx,
        tx.query("CREATE $row SET payload_hash=$hash, receipt_json=$receipt RETURN NONE")
            .bind(("row", jobs::rid("setup_request", request_key)))
            .bind(("hash", payload_hash.to_owned()))
            .bind(("receipt", encoded)),
    )
    .await?;
    Ok(())
}

/// Join setup admission to the caller's existing durable acceptance transaction.
/// The caller constructs origin from validated context/effective configuration.
pub async fn admit_in(
    tx: &Tx,
    authority: &RegistryAuthority,
    context: &RegisteredContext,
    origin: &JobOrigin,
    operation: MaintenanceOperation,
    request: &MaintenanceRequest,
) -> Result<MaintenanceReceipt, TxError> {
    request
        .validate(operation)
        .map_err(|_| invalid("invalid_request"))?;
    if origin.request_id != request.request_id {
        return Err(invalid("effective_configuration_required"));
    }
    if request.context.registration_id != context.registration_id
        || request.context.expected_registration_revision != context.registration_revision
        || request.context.expected_context_revision != context.context_revision
    {
        return Err(invalid("context_revision_changed"));
    }
    let pool = operation_name(operation);
    let id = format!(
        "setup-{}",
        digest(&[
            &context.deployment_id,
            &context.registration_id,
            pool,
            &request.request_id
        ])
    );
    let payload = SetupPayload {
        operation,
        dataset: request.dataset.clone(),
    };
    let payload_json = serde_json::to_string(&payload).map_err(|_| invalid("invalid_payload"))?;
    let payload_hash = digest(&[&payload_json]);
    let mut replay = jobs::run(
        tx,
        tx.query("SELECT payload_hash,receipt_json FROM ONLY $row")
            .bind(("row", jobs::rid("setup_request", &id))),
    )
    .await?;
    if let Some(replay) = jobs::take::<Option<RequestRow>>(&mut replay, 0)? {
        if replay.payload_hash != payload_hash {
            return Err(invalid("request_conflict"));
        }
        let receipt: MaintenanceReceipt =
            serde_json::from_str(&replay.receipt_json).map_err(|_| invalid("receipt_invalid"))?;
        job_context::record_admission_in(
            tx,
            authority,
            context,
            origin,
            &receipt.job_id,
            pool,
            false,
        )
        .await?;
        return Ok(receipt);
    }
    // Exact accepted replay retains its first receipt even after a later
    // configuration save. New work must still match the effective snapshot.
    if request
        .context
        .expected_configuration_revision
        .is_some_and(|revision| revision != origin.configuration_revision)
    {
        return Err(invalid("configuration_revision_changed"));
    }
    let mut enabled = jobs::run(
        tx,
        tx.query("SELECT VALUE enabled FROM ONLY $row")
            .bind(("row", jobs::rid("setup_dispatch", pool))),
    )
    .await?;
    if jobs::take::<Option<bool>>(&mut enabled, 0)? != Some(true) {
        return Err(invalid("dispatch_unavailable"));
    }
    if let Some(expected) = &request.dataset {
        jobs::run(
            tx,
            tx.query("UPDATE $row SET pin_revision+=1 RETURN NONE")
                .bind(("row", jobs::rid("setup_dataset", &expected.id))),
        )
        .await?;
        let row = dataset_in(tx, &expected.id).await?;
        if !row.live || row.scope_id != authority.scope_id() || reference(&row)? != *expected {
            return Err(invalid("dataset_unavailable"));
        }
    }
    let resource = jobs::rid("setup_resource", resource_name(operation));
    jobs::run(
        tx,
        tx.query("UPSERT $resource SET revision=(revision ?? 0)+1 RETURN NONE")
            .bind(("resource", resource.clone())),
    )
    .await?;
    if operation == MaintenanceOperation::TrainClassifier {
        super::classifier_activation::require_unambiguous_resource_in(tx).await?;
    }
    // The revision guard creates an empty resource on first admission. Never
    // construct a record ID from that initial empty job_id.
    let mut resource_row = jobs::run(tx, tx.query("SELECT job_id, payload_hash, scope_id, IF job_id != '' { ((type::record('job', job_id).state ?? '') IN ['queued','running','waiting','reconciling'] OR array::len((SELECT VALUE id FROM task WHERE job=type::record('job',$parent.job_id) AND state NOT IN ['completed','failed','cancelled','superseded','stopped','skipped'] LIMIT 1))>0) } ELSE { false } AS active FROM ONLY $resource").bind(("resource", resource.clone()))).await?;
    let existing: Option<ResourceRow> = jobs::take(&mut resource_row, 0)?;
    if let Some(existing) = existing.filter(|row| row.active) {
        if existing.payload_hash != payload_hash || existing.scope_id != authority.scope_id() {
            return Err(invalid("resource_busy"));
        }
        job_context::record_admission_in(
            tx,
            authority,
            context,
            origin,
            &existing.job_id,
            pool,
            false,
        )
        .await?;
        let receipt = MaintenanceReceipt::new(
            existing.job_id,
            operation,
            origin.clone(),
            request.dataset.clone(),
            true,
        );
        save_receipt_in(tx, &id, &payload_hash, &receipt).await?;
        return Ok(receipt);
    }
    let task_id = format!("{id}-execute");
    let now = Utc::now();
    let created = job_dispatch::submit_in(
        tx,
        &NewJob {
            id: &id,
            operation: pool,
            kind: "maintenance",
            parent: None,
            created_at: now,
        },
        &NewTask {
            id: &task_id,
            job: &id,
            pool,
            operation: pool,
            input_hash: &payload_hash,
            priority: 0,
            deps: Vec::new(),
            created_at: now,
        },
    )
    .await?;
    if created {
        jobs::run(tx, tx.query("CREATE $row SET operation=$operation, payload_json=$payload, payload_hash=$hash, scope_id=$scope, updated_at=time::now() RETURN NONE")
            .bind(("row", jobs::rid("setup_operation", &id))).bind(("operation", pool)).bind(("payload", payload_json)).bind(("hash", payload_hash.clone())).bind(("scope", authority.scope_id().to_owned()))).await?;
        if let Some(dataset) = &request.dataset {
            jobs::run(
                tx,
                tx.query("CREATE $pin SET dataset_id=$dataset, job_id=$job RETURN NONE")
                    .bind((
                        "pin",
                        jobs::rid("setup_dataset_pin", &digest(&[&dataset.id, &id])),
                    ))
                    .bind(("dataset", dataset.id.clone()))
                    .bind(("job", id.clone())),
            )
            .await?;
            jobs::run(
                tx,
                tx.query("UPDATE $upload SET state='pinned' RETURN NONE")
                    .bind(("upload", jobs::rid("setup_upload", &dataset.id))),
            )
            .await?;
        }
    }
    job_context::record_admission_in(tx, authority, context, origin, &id, pool, created).await?;
    jobs::run(
        tx,
        tx.query(
            "UPDATE $resource SET job_id=$job, payload_hash=$hash, scope_id=$scope RETURN NONE",
        )
        .bind(("resource", resource))
        .bind(("job", id.clone()))
        .bind(("hash", payload_hash.clone()))
        .bind(("scope", authority.scope_id().to_owned())),
    )
    .await?;
    let receipt = MaintenanceReceipt::new(
        id.clone(),
        operation,
        origin.clone(),
        request.dataset.clone(),
        !created,
    );
    save_receipt_in(tx, &id, &payload_hash, &receipt).await?;
    Ok(receipt)
}

/// Read one durable immutable payload for an actually claimed task.
pub async fn payload(
    repo: &Repository,
    job_id: &str,
    expected_hash: &str,
) -> Result<SetupPayload, TxError> {
    #[derive(SurrealValue)]
    struct PayloadRow {
        payload_json: String,
        payload_hash: String,
    }
    let mut result = repo
        .db()
        .query("SELECT payload_json, payload_hash FROM ONLY $row")
        .bind(("row", jobs::rid("setup_operation", job_id)))
        .await?
        .check()?;
    let row: Option<PayloadRow> = result.take(0)?;
    let row = row
        .filter(|row| row.payload_hash == expected_hash)
        .ok_or_else(|| invalid("payload_unavailable"))?;
    serde_json::from_str(&row.payload_json).map_err(|_| invalid("payload_invalid"))
}

/// Persist safe bounded worker result metadata; the dispatch lease owns terminal status.
pub async fn record_result(
    repo: &Repository,
    claimed: &jobs::ClaimedTask,
    result: &serde_json::Value,
) -> Result<(), TxError> {
    let encoded = serde_json::to_string(result).map_err(|_| invalid("result_invalid"))?;
    if encoded.len() > 64 * 1024 {
        return Err(invalid("result_too_large"));
    }
    run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let encoded = encoded.clone();
        async move {
            jobs::bump(&tx, "job_guard", &claimed.root_id).await?;
            let now = Utc::now().to_rfc3339();
            let fence = jobs::fence(&tx, claimed, &now).await?;
            if !fence.is_some_and(|fence| !fence.cancelled && jobs::holds(&fence, claimed)) {
                return Err(invalid("execution_lease_lost"));
            }
            jobs::run(
                &tx,
                tx.query("UPDATE $row SET result_json=$result, updated_at=time::now() RETURN NONE")
                    .bind(("row", jobs::rid("setup_operation", &claimed.job_id)))
                    .bind(("result", encoded)),
            )
            .await?;
            Ok(())
        }
    })
    .await?;
    Ok(())
}

/// Read bounded result metadata for an already authorized job. This trusted
/// storage adapter does not itself grant HTTP activity or private-data access.
pub async fn result_for_job(
    repo: &Repository,
    job_id: &str,
) -> Result<Option<serde_json::Value>, TxError> {
    if job_id.is_empty() || job_id.len() > 512 || job_id.chars().any(char::is_control) {
        return Err(invalid("job_invalid"));
    }
    let mut response = repo
        .db()
        .query("SELECT VALUE result_json FROM ONLY $row")
        .bind(("row", jobs::rid("setup_operation", job_id)))
        .await?
        .check()?;
    let result: Option<String> = response.take(0)?;
    match result {
        Some(result) if result.is_empty() => Ok(None),
        Some(result) if result.len() <= 64 * 1024 => serde_json::from_str(&result)
            .map(Some)
            .map_err(|_| invalid("result_invalid")),
        Some(_) => Err(invalid("result_too_large")),
        None => Ok(None),
    }
}

/// Progress describes committed domain work under the currently held task lease.
pub async fn record_progress(
    repo: &Repository,
    claimed: &jobs::ClaimedTask,
    phase: &str,
    done: u64,
    total: u64,
) -> Result<(), TxError> {
    if phase.len() > 128
        || phase.chars().any(char::is_control)
        || done > total
        || total > i64::MAX as u64
    {
        return Err(invalid("progress_invalid"));
    }
    run_tx(repo.db(),RetryPolicy::default(),|tx|async move {
        jobs::bump(&tx,"job_guard",&claimed.root_id).await?;
        let now=Utc::now().to_rfc3339();
        let fence=jobs::fence(&tx,claimed,&now).await?;
        if !fence.is_some_and(|fence|!fence.cancelled&&jobs::holds(&fence,claimed)){return Err(invalid("execution_lease_lost"));}
        jobs::run(&tx,tx.query("UPDATE $job SET phase=$phase,progress_done=$done,progress_total=$total,updated_at=time::now() RETURN NONE")
            .bind(("job",jobs::rid("job",&claimed.job_id))).bind(("phase",phase.to_owned())).bind(("done",done as i64)).bind(("total",total as i64))).await?;
        Ok(())
    }).await?;
    Ok(())
}

/// Publish actual scan progress when a cheap authoritative total is unavailable.
pub async fn record_progress_unknown_total(
    repo: &Repository,
    claimed: &jobs::ClaimedTask,
    phase: &str,
    done: u64,
) -> Result<(), TxError> {
    if phase.len() > 128 || phase.chars().any(char::is_control) || done > i64::MAX as u64 {
        return Err(invalid("progress_invalid"));
    }
    run_tx(repo.db(),RetryPolicy::default(),|tx|async move {
        jobs::bump(&tx,"job_guard",&claimed.root_id).await?;
        let now=Utc::now().to_rfc3339();
        let fence=jobs::fence(&tx,claimed,&now).await?;
        if !fence.is_some_and(|fence|!fence.cancelled&&jobs::holds(&fence,claimed)){return Err(invalid("execution_lease_lost"));}
        jobs::run(&tx,tx.query("UPDATE $job SET phase=$phase,progress_done=$done,progress_total=NONE,updated_at=time::now() RETURN NONE")
            .bind(("job",jobs::rid("job",&claimed.job_id))).bind(("phase",phase.to_owned())).bind(("done",done as i64))).await?;
        Ok(())
    }).await?;
    Ok(())
}
