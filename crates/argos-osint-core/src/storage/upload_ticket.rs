//! Shared-backend, single-use destinations for bounded native CSV streams.
//!
//! These rows carry no bearer tokens or filesystem paths. Authority comes from
//! the authenticated gateway and exact registered proof, never request JSON.
//! Per-issuer guards bound outstanding destinations without a global hot row.
use std::time::Duration;

use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::dataset_upload::DatasetUploadRequest;
use fleetworks_contracts::dataset_upload::DatasetUploadTicket;
use fleetworks_contracts::dataset_upload::DATASET_UPLOADS_PATH;
use fleetworks_contracts::readiness::DatasetKind;
use sha2::Digest;
use sha2::Sha256;
use surrealdb::types::Duration as DbDuration;
use surrealdb::types::RecordId;
use surrealdb::types::SurrealValue;

use super::jobs;
use super::repo::Repository;
use super::tx::run_tx;
use super::tx::RetryPolicy;
use super::tx::TxError;

pub const SCHEMA: &str = include_str!("schema/upload_ticket.surql");
pub const TICKET_TTL: Duration = Duration::from_secs(300);
pub const PER_ISSUER_LIMIT: usize = 64;
const CLEANUP_BATCH: usize = 32;

/// Trusted current authority. Deliberately not deserializable from HTTP input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadTicketBinding {
    issuer_key: String,
    scope_id: String,
    credential_id: String,
    access_epoch: String,
    purpose: String,
    context_json: String,
}
impl UploadTicketBinding {
    pub fn new(
        scope: &str,
        credential: &str,
        epoch: &str,
        purpose: CredentialPurpose,
        context: &RegisteredContext,
    ) -> Result<Self, TxError> {
        let purpose = match purpose {
            CredentialPurpose::Native => "native",
            CredentialPurpose::Operator => "operator",
            _ => return Err(abort("invalid")),
        };
        if ![
            scope,
            credential,
            epoch,
            context.deployment_id.as_str(),
            context.registration_id.as_str(),
            context.workstation_id.as_str(),
            context.configuration_epoch.as_str(),
        ]
        .into_iter()
        .all(valid_identity)
            || context.access_epoch != epoch
            || context.connect_id.as_str() != context.workstation_id
            || context.registration_revision == 0
            || context.context_revision == 0
            || context.scope_revision == 0
        {
            return Err(abort("invalid"));
        }
        let context_json = serde_json::to_string(context).map_err(|_| abort("invalid"))?;
        if context_json.len() > 8192 {
            return Err(abort("invalid"));
        }
        let mut hash = Sha256::new();
        for part in [scope, credential, epoch, purpose] {
            hash.update((part.len() as u64).to_be_bytes());
            hash.update(part.as_bytes());
        }
        Ok(Self {
            issuer_key: format!("{:x}", hash.finalize()),
            scope_id: scope.into(),
            credential_id: credential.into(),
            access_epoch: epoch.into(),
            purpose: purpose.into(),
            context_json,
        })
    }
}
fn valid_identity(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}
fn valid_ticket(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn abort(code: &'static str) -> TxError {
    TxError::Abort(format!("upload_ticket:{code}"))
}

#[derive(SurrealValue)]
struct TicketRow {
    issuer_key: String,
    scope_id: String,
    credential_id: String,
    access_epoch: String,
    purpose: String,
    context_json: String,
    kind: String,
    sha256: String,
    size_bytes: i64,
    state: String,
    live: bool,
}
impl TicketRow {
    fn matches(&self, binding: &UploadTicketBinding) -> bool {
        self.issuer_key == binding.issuer_key
            && self.scope_id == binding.scope_id
            && self.credential_id == binding.credential_id
            && self.access_epoch == binding.access_epoch
            && self.purpose == binding.purpose
            && self.context_json == binding.context_json
    }
    fn request(&self) -> Result<DatasetUploadRequest, TxError> {
        let kind = match self.kind.as_str() {
            "actor_catalog" => DatasetKind::ActorCatalog,
            "classifier_training" => DatasetKind::ClassifierTraining,
            _ => return Err(abort("unavailable")),
        };
        DatasetUploadRequest::new(
            kind,
            self.sha256.clone(),
            u64::try_from(self.size_bytes).map_err(|_| abort("unavailable"))?,
        )
        .map_err(|_| abort("unavailable"))
    }
}

/// Issue a five-minute destination. Quota reads at most 64 indexed unexpired
/// rows (pending or claimed), so rapid claim/fail loops cannot bypass the bound.
/// Issuance prunes at most 64 expired rows under this principal's own guard.
pub async fn issue(
    repo: &Repository,
    binding: &UploadTicketBinding,
    request: &DatasetUploadRequest,
) -> Result<DatasetUploadTicket, TxError> {
    request.validate().map_err(|_| abort("invalid"))?;
    let id = tokio::task::spawn_blocking(super::home::random_secret)
        .await
        .map_err(|_| abort("unavailable"))?
        .map_err(|_| abort("unavailable"))?;
    let kind = match request.kind {
        DatasetKind::ActorCatalog => "actor_catalog",
        DatasetKind::ClassifierTraining => "classifier_training",
        _ => return Err(abort("invalid")),
    };
    run_tx(repo.db(),RetryPolicy::default(),|tx| {
        let id=id.clone();
        async move {
            jobs::run(&tx,tx.query("UPSERT $issuer SET revision=(revision ?? 0)+1 RETURN NONE").bind(("issuer",jobs::rid("upload_ticket_issuer",&binding.issuer_key)))).await?;
            let mut expired=jobs::run(&tx,tx.query("SELECT VALUE id FROM upload_ticket WHERE issuer_key=$issuer AND expires_at<=time::now() LIMIT $limit").bind(("issuer",binding.issuer_key.clone())).bind(("limit",PER_ISSUER_LIMIT as i64))).await?;
            let expired:Vec<RecordId>=jobs::take(&mut expired,0)?;
            if !expired.is_empty() {
                jobs::run(&tx,tx.query("DELETE $rows WHERE issuer_key=$issuer AND expires_at<=time::now() RETURN NONE").bind(("rows",expired)).bind(("issuer",binding.issuer_key.clone()))).await?;
            }
            let mut response=jobs::run(&tx,tx.query("SELECT VALUE id FROM upload_ticket WHERE issuer_key=$issuer AND expires_at>time::now() LIMIT $limit").bind(("issuer",binding.issuer_key.clone())).bind(("limit",PER_ISSUER_LIMIT as i64))).await?;
            let unexpired:Vec<RecordId>=jobs::take(&mut response,0)?;
            if unexpired.len()>=PER_ISSUER_LIMIT {return Err(abort("capacity"));}
            jobs::run(&tx,tx.query("CREATE $row SET issuer_key=$issuer,scope_id=$scope,credential_id=$credential,access_epoch=$epoch,purpose=$purpose,context_json=$context,kind=$kind,sha256=$hash,size_bytes=$size,state='pending',created_at=time::now(),expires_at=time::now()+$ttl RETURN NONE")
                .bind(("row",jobs::rid("upload_ticket",&id))).bind(("issuer",binding.issuer_key.clone())).bind(("scope",binding.scope_id.clone())).bind(("credential",binding.credential_id.clone())).bind(("epoch",binding.access_epoch.clone())).bind(("purpose",binding.purpose.clone())).bind(("context",binding.context_json.clone())).bind(("kind",kind)).bind(("hash",request.sha256.to_ascii_lowercase())).bind(("size",request.size_bytes as i64)).bind(("ttl",DbDuration::from(TICKET_TTL)))).await?;
            Ok(())
        }
    }).await?;
    DatasetUploadTicket::new(format!("{DATASET_UPLOADS_PATH}/{id}"), TICKET_TTL.as_secs())
        .map_err(|_| abort("unavailable"))
}

/// Atomically consume this exact destination before reading bytes. Concurrent
/// claims conflict on this row; a retry sees `claimed` and cannot stream twice.
/// An unauthorized binding never consumes the legitimate owner's destination.
pub async fn claim(
    repo: &Repository,
    binding: &UploadTicketBinding,
    id: &str,
) -> Result<DatasetUploadRequest, TxError> {
    if !valid_ticket(id) {
        return Err(abort("unavailable"));
    }
    let committed=run_tx(repo.db(),RetryPolicy::default(),|tx|async move {
        let row=jobs::rid("upload_ticket",id);
        let mut response=jobs::run(&tx,tx.query("SELECT issuer_key,scope_id,credential_id,access_epoch,purpose,context_json,kind,sha256,size_bytes,state,(expires_at>time::now()) AS live FROM ONLY $row").bind(("row",row.clone()))).await?;
        let ticket:TicketRow=jobs::take::<Option<TicketRow>>(&mut response,0)?.ok_or_else(||abort("unavailable"))?;
        if !ticket.matches(binding){return Err(abort("forbidden"));}
        if ticket.state!="pending"||!ticket.live{return Err(abort("unavailable"));}
        let request=ticket.request()?;
        let mut changed=jobs::run(&tx,tx.query("RETURN count(UPDATE $row SET state='claimed',claimed_at=time::now() WHERE state='pending' AND expires_at>time::now() RETURN VALUE id)=1").bind(("row",row))).await?;
        if jobs::take::<Option<bool>>(&mut changed,0)?!=Some(true){return Err(abort("unavailable"));}
        Ok(request)
    }).await?;
    Ok(committed.value)
}

/// Bounded metadata-only cleanup for the existing native-worker housekeeping.
/// Claimed streams keep their captured request; setup_upload separately owns
/// abandoned .part files and accepted DatasetRefs/pins retain their own TTLs.
pub async fn expire(repo: &Repository) -> Result<usize, TxError> {
    let committed = run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        let mut response = jobs::run(
            &tx,
            tx.query(
                "SELECT VALUE id FROM upload_ticket WHERE expires_at<=time::now() LIMIT $limit",
            )
            .bind(("limit", CLEANUP_BATCH as i64)),
        )
        .await?;
        let expired: Vec<RecordId> = jobs::take(&mut response, 0)?;
        if expired.is_empty() {
            return Ok(0);
        }
        let mut response = jobs::run(
            &tx,
            tx.query("RETURN count(DELETE $rows WHERE expires_at<=time::now() RETURN BEFORE)")
                .bind(("rows", expired)),
        )
        .await?;
        let removed: Option<i64> = jobs::take(&mut response, 0)?;
        usize::try_from(removed.unwrap_or(0)).map_err(|_| abort("unavailable"))
    })
    .await?;
    Ok(committed.value)
}
