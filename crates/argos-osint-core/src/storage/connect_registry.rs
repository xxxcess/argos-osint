//! Durable native registration and revocable web connection context.
//!
//! IDs/markers describe registered context, not personal authentication. The HTTP
//! boundary supplies purpose-authorized scope. Database claims plus a shared
//! transaction guard prevent concurrent installation/binding collisions. Old records
//! and submission snapshots are retained; ensure never resurrects revocation.

use std::fmt;
use std::net::IpAddr;
use std::time::Duration;

use fleetworks_contracts::connect::ConnectId;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::connect::RegistrationStatus;
use fleetworks_contracts::context::RegisteredContext;
use sha2::Digest;
use sha2::Sha256;
use surrealdb::engine::any::Any;
use surrealdb::types::Duration as DbDuration;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use super::jobs::rid;
use super::jobs::run;
use super::jobs::take;
use super::repo::Repository;
use super::tx::run_tx;
use super::tx::RetryPolicy;
use super::tx::Tx;
use super::tx::TxError;

pub const SCHEMA: &str = include_str!("schema/connect_registry.surql");
const SESSION_TTL: Duration = Duration::from_secs(30 * 60);
const RESOLUTION_ATTEMPTS_PER_MINUTE: i64 = 20;

/// Authority constructed from validated server policy, never a request body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryAuthority {
    scope_id: String,
    credential_id: String,
    access_epoch: String,
}

impl RegistryAuthority {
    pub fn new(
        scope_id: &str,
        credential_id: &str,
        access_epoch: &str,
    ) -> Result<Self, RegistryError> {
        if ![scope_id, credential_id, access_epoch]
            .into_iter()
            .all(valid_identity)
        {
            return Err(RegistryError::InvalidInput);
        }
        Ok(Self {
            scope_id: scope_id.into(),
            credential_id: credential_id.into(),
            access_epoch: access_epoch.into(),
        })
    }

    pub fn scope_id(&self) -> &str {
        &self.scope_id
    }

    pub fn access_epoch(&self) -> &str {
        &self.access_epoch
    }
}

/// Persisted deployment identity and independently rotated user-config epoch.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct DeploymentState {
    pub deployment_id: String,
    pub configuration_epoch: String,
    pub admission_open: bool,
}

/// Registration creation/reuse; retries describe actual storage contention.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct EnsureResult {
    pub receipt: RegistrationReceipt,
    pub created: bool,
    pub retries: u32,
}

/// Deliberately cannot be formatted/serialized as a credential by accident.
#[derive(Clone)]
pub struct SessionToken(String);

impl SessionToken {
    /// Only the protected relay response/cookie path should expose this value.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SessionToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionToken([redacted])")
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub struct CreatedSession {
    pub token: SessionToken,
    pub context: RegisteredContext,
    pub expires_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ValidatedSession {
    pub context: RegisteredContext,
    pub expires_at: String,
}

/// Errors are non-enumerating at the browser boundary and never echo inputs.
#[derive(Debug)]
#[non_exhaustive]
pub enum RegistryError {
    InvalidInput,
    NotFound,
    Collision,
    Revoked,
    ProtectedDefault,
    RevisionMismatch,
    AdmissionPaused,
    RateLimited,
    Unconfigured,
    Storage(TxError),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidInput => "invalid registered context input",
            Self::NotFound => "registered context unavailable",
            Self::Collision => "installation, registration or request identity collision",
            Self::Revoked => "registration is no longer active",
            Self::ProtectedDefault => {
                "the protected default workspace cannot be retired or renamed"
            }
            Self::RevisionMismatch => "registered context changed; refresh before retrying",
            Self::AdmissionPaused => "registered admission is paused for maintenance",
            Self::RateLimited => "connection resolution rate limit reached",
            Self::Unconfigured => "registered context requires operator bootstrap",
            Self::Storage(_) => "registered context authority is unavailable",
        })
    }
}

impl std::error::Error for RegistryError {}

impl From<TxError> for RegistryError {
    fn from(error: TxError) -> Self {
        if let TxError::Abort(reason) = &error {
            match reason.as_str() {
                "connect:invalid" => return Self::InvalidInput,
                "connect:missing" => return Self::NotFound,
                "connect:collision" => return Self::Collision,
                "connect:revoked" => return Self::Revoked,
                "connect:protected" => return Self::ProtectedDefault,
                "connect:revision" => return Self::RevisionMismatch,
                "connect:paused" => return Self::AdmissionPaused,
                "connect:unconfigured" => return Self::Unconfigured,
                _ => {}
            }
        }
        Self::Storage(error)
    }
}

#[derive(SurrealValue)]
struct ControlRow {
    configuration_epoch: String,
    admission_open: bool,
    contract_version: i64,
}

#[derive(SurrealValue)]
struct WorkstationRow {
    workstation_id: String,
    scope_id: String,
    installation_hash: String,
    workstation_ip: String,
    state: String,
    revision: i64,
}

#[derive(SurrealValue)]
struct WorkspaceRow {
    workspace_id: String,
    workstation_id: String,
    scope_id: String,
    workspace_name: String,
    marker_hash: String,
    protected_default: bool,
    state: String,
    revision: i64,
}

#[derive(SurrealValue)]
struct RegistrationRow {
    registration_id: String,
    scope_id: String,
    workstation_id: String,
    workspace_id: String,
    workstation_ip: String,
    workspace_name: String,
    connect_id: String,
    contract_version: i64,
    state: String,
    revision: i64,
}

#[derive(SurrealValue)]
struct RequestRow {
    request_hash: String,
    registration_id: String,
}

#[derive(SurrealValue)]
struct SessionRow {
    scope_id: String,
    credential_id: String,
    access_epoch: String,
    configuration_epoch: String,
    registration_id: String,
    registration_revision: i64,
    context_revision: i64,
    scope_revision: i64,
    expires_at: String,
    live: bool,
    revoked: bool,
}

fn valid_identity(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_marker(value: &str) -> bool {
    (16..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn hash(parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn abort(reason: &'static str) -> TxError {
    TxError::Abort(format!("connect:{reason}"))
}

async fn guard_in(tx: &Tx) -> Result<i64, TxError> {
    let mut result = run(tx, tx.query("UPSERT connect_registry_guard:singleton SET revision = (revision ?? 0) + 1 RETURN NONE; SELECT VALUE revision FROM ONLY connect_registry_guard:singleton;")).await?;
    let revision: Option<i64> = take(&mut result, 1)?;
    revision
        .filter(|revision| *revision > 0)
        .ok_or_else(|| abort("unconfigured"))
}

async fn deployment_in(tx: &Tx) -> Result<DeploymentState, TxError> {
    let mut response = run(tx, tx.query("SELECT VALUE instance_id FROM ONLY storage_meta:identity; SELECT configuration_epoch, admission_open, contract_version ?? 1 AS contract_version FROM ONLY connect_control:configuration;")).await?;
    let deployment_id: Option<String> = take(&mut response, 0)?;
    let control: Option<ControlRow> = take(&mut response, 1)?;
    let deployment_id = deployment_id
        .filter(|value| !value.is_empty())
        .ok_or_else(|| abort("unconfigured"))?;
    let control = control
        .filter(|row| !row.configuration_epoch.is_empty() && row.contract_version == 2)
        .ok_or_else(|| abort("unconfigured"))?;
    Ok(DeploymentState {
        deployment_id,
        configuration_epoch: control.configuration_epoch,
        admission_open: control.admission_open,
    })
}

/// Read persisted epoch/admission metadata in a caller-owned transaction.
pub async fn deployment_state_in(tx: &Tx) -> Result<DeploymentState, TxError> {
    deployment_in(tx).await
}

/// Explicit operator bootstrap only; never called from a status/read/session path.
/// This initializes non-secret fencing metadata and does not reset credentials.
pub async fn initialize_control(db: &Surreal<Any>) -> Result<DeploymentState, RegistryError> {
    let epoch = format!("configuration-{}", crate::store::new_id());
    let result = run_tx(db, RetryPolicy::default(), |tx| {
        let epoch = epoch.clone();
        async move {
            guard_in(&tx).await?;
            run(&tx, tx.query("UPSERT connect_control:configuration SET configuration_epoch = configuration_epoch ?? $epoch, admission_open = admission_open ?? true, contract_version = contract_version ?? 2 RETURN NONE").bind(("epoch", epoch))).await?;
            deployment_in(&tx).await
        }
    }).await?;
    Ok(result.value)
}

/// Read existing fencing metadata without creating records or mutating readiness.
pub async fn deployment_state(repo: &Repository) -> Result<DeploymentState, RegistryError> {
    let result = run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        deployment_in(&tx).await
    })
    .await?;
    Ok(result.value)
}

fn receipt(row: &RegistrationRow, deployment: &str) -> Result<RegistrationReceipt, TxError> {
    if row.contract_version != 2 || row.connect_id != row.workstation_id || row.revision < 1 {
        return Err(abort("invalid"));
    }
    let ip = if row.workstation_ip.is_empty() {
        None
    } else {
        Some(row.workstation_ip.parse().map_err(|_| abort("invalid"))?)
    };
    let mut value = RegistrationReceipt::new(
        deployment.into(),
        row.workstation_id.clone(),
        row.registration_id.clone(),
        ip,
        row.revision as u64,
    );
    value.status = match row.state.as_str() {
        "active" => RegistrationStatus::Active,
        "retiring" => RegistrationStatus::Retiring,
        "retired" => RegistrationStatus::Retired,
        "revoked" => RegistrationStatus::Revoked,
        _ => return Err(abort("invalid")),
    };
    Ok(value)
}

async fn registration_in(
    tx: &Tx,
    id: &str,
    authority: &RegistryAuthority,
) -> Result<RegistrationRow, TxError> {
    let row = read_registration_in(tx, id, authority).await?;
    if row.state != "active" {
        return Err(abort("revoked"));
    }
    Ok(row)
}

async fn read_registration_in(
    tx: &Tx,
    id: &str,
    authority: &RegistryAuthority,
) -> Result<RegistrationRow, TxError> {
    let mut result = run(tx, tx.query("SELECT registration_id, scope_id, workstation_id, workspace_id ?? '' AS workspace_id, workstation_ip ?? '' AS workstation_ip, workspace_name ?? '' AS workspace_name, connect_id, contract_version ?? 1 AS contract_version, state, revision FROM ONLY $row").bind(("row", rid("connect_registration", id)))).await?;
    let row: Option<RegistrationRow> = take(&mut result, 0)?;
    let row = row
        .filter(|row| row.scope_id == authority.scope_id)
        .ok_or_else(|| abort("missing"))?;
    Ok(row)
}

async fn workstation_in(tx: &Tx, id: &str) -> Result<WorkstationRow, TxError> {
    let mut result = run(tx, tx.query("SELECT workstation_id, scope_id, installation_hash, workstation_ip ?? '' AS workstation_ip, state, revision FROM ONLY $row").bind(("row", rid("connect_workstation", id)))).await?;
    take::<Option<WorkstationRow>>(&mut result, 0)?.ok_or_else(|| abort("missing"))
}

async fn workspace_in(tx: &Tx, id: &str) -> Result<WorkspaceRow, TxError> {
    let mut result = run(tx, tx.query("SELECT workspace_id, workstation_id, scope_id, workspace_name, marker_hash, protected_default, state, revision FROM ONLY $row").bind(("row", rid("connect_workspace", id)))).await?;
    take::<Option<WorkspaceRow>>(&mut result, 0)?.ok_or_else(|| abort("missing"))
}

async fn claim_value(
    tx: &Tx,
    table: &'static str,
    key: &str,
    field: &'static str,
) -> Result<Option<String>, TxError> {
    // Only module-owned table/field literals form this statement; all caller data is bound.
    let mut result = run(
        tx,
        tx.query(format!("SELECT VALUE {field} FROM ONLY $claim"))
            .bind(("claim", rid(table, key))),
    )
    .await?;
    take(&mut result, 0)
}

/// Idempotently ensure the exact owned installation/folder binding in one transaction.
pub async fn ensure(
    repo: &Repository,
    authority: &RegistryAuthority,
    input: &RegistrationEnsureRequest,
) -> Result<EnsureResult, RegistryError> {
    ensure_observed(repo, authority, input, None).await
}

/// Server-observed socket metadata is audit-only and never replaces the native IP.
/// Do not populate this from untrusted Forwarded/X-Forwarded-For headers.
pub async fn ensure_observed(
    repo: &Repository,
    authority: &RegistryAuthority,
    input: &RegistrationEnsureRequest,
    observed_peer: Option<IpAddr>,
) -> Result<EnsureResult, RegistryError> {
    if !valid_identity(&input.request_id) || !valid_marker(&input.installation_marker) {
        return Err(RegistryError::InvalidInput);
    }
    let installation = hash(&[&input.installation_marker]);
    let ip = input.workstation_ip.map(|ip| ip.to_string());
    let fingerprint = hash(&[&installation, ip.as_deref().unwrap_or("")]);
    let request_key = hash(&[&authority.scope_id, "ensure-v2", &input.request_id]);
    let new_workstation = crate::store::new_id();
    let new_registration = crate::store::new_id();
    let peer = observed_peer.map(|ip| ip.to_string());
    let committed = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let authority = authority.clone();
        let (installation, ip, fingerprint, request_key, new_workstation, new_registration, peer) = (installation.clone(), ip.clone(), fingerprint.clone(), request_key.clone(), new_workstation.clone(), new_registration.clone(), peer.clone());
        async move {
            guard_in(&tx).await?;
            let deployment = deployment_in(&tx).await?;
            if !deployment.admission_open { return Err(abort("paused")); }
            super::configuration_reset::check_user_epoch_in(&tx, &authority.scope_id, &authority.access_epoch, None).await?;
            let mut replay = run(&tx, tx.query("SELECT request_hash, registration_id FROM ONLY $request").bind(("request", rid("connect_request", &request_key)))).await?;
            if let Some(replay) = take::<Option<RequestRow>>(&mut replay, 0)? {
                if replay.request_hash != fingerprint { return Err(abort("collision")); }
                let registration = registration_in(&tx, &replay.registration_id, &authority).await?;
                return Ok((receipt(&registration, &deployment.deployment_id)?, false));
            }
            let mut found = run(&tx, tx.query("SELECT workstation_id, scope_id, installation_hash, workstation_ip ?? '' AS workstation_ip, state, revision FROM connect_workstation WHERE scope_id=$scope AND installation_hash=$installation LIMIT 2").bind(("scope", authority.scope_id.clone())).bind(("installation", installation.clone()))).await?;
            let mut found: Vec<WorkstationRow> = take(&mut found, 0)?;
            if found.len() > 1 { return Err(abort("collision")); }
            let workstation = if let Some(workstation) = found.pop() {
                if workstation.state != "active" { return Err(abort("revoked")); }
                workstation
            } else {
                run(&tx, tx.query("CREATE $row SET workstation_id=$id, scope_id=$scope, installation_hash=$installation, workstation_ip=$ip, state='active', revision=1, last_seen_at=time::now() RETURN NONE")
                    .bind(("row", rid("connect_workstation", &new_workstation))).bind(("id", new_workstation.clone())).bind(("scope", authority.scope_id.clone())).bind(("installation", installation)).bind(("ip", ip.clone()))).await?;
                workstation_in(&tx, &new_workstation).await?
            };
            let binding_key = hash(&[&workstation.workstation_id]);
            let (registration, created) = if let Some(id) = claim_value(&tx, "connect_binding", &binding_key, "registration_id").await? {
                let row = registration_in(&tx, &id, &authority).await?;
                if row.workstation_id != workstation.workstation_id || row.contract_version != 2 { return Err(abort("collision")); }
                (row, false)
            } else {
                let mut history = run(&tx, tx.query("SELECT VALUE registration_id FROM connect_registration WHERE workstation_id=$node AND contract_version=2 LIMIT 1").bind(("node", workstation.workstation_id.clone()))).await?;
                if !take::<Vec<String>>(&mut history, 0)?.is_empty() { return Err(abort("revoked")); }
                run(&tx, tx.query("CREATE $row SET registration_id=$id, scope_id=$scope, workstation_id=$node, workstation_ip=$ip, connect_id=$node, contract_version=2, state='active', revision=1, created_at=time::now() RETURN NONE; CREATE $binding SET connect_id=$node, registration_id=$id RETURN NONE;")
                    .bind(("row", rid("connect_registration", &new_registration))).bind(("binding", rid("connect_binding", &binding_key))).bind(("id", new_registration.clone())).bind(("scope", authority.scope_id.clone())).bind(("node", workstation.workstation_id.clone())).bind(("ip", ip.clone()))).await?;
                (registration_in(&tx, &new_registration, &authority).await?, true)
            };
            // IP observations never alter identity or revoke a browser session.
            run(&tx, tx.query("UPDATE $registration SET workstation_ip=$ip RETURN NONE; CREATE $request SET request_hash=$fingerprint, registration_id=$registration_id RETURN NONE; UPDATE $node SET workstation_ip=$ip, last_seen_at=time::now(), observed_peer_ip=$peer ?? observed_peer_ip, observed_peer_at=IF $peer != NONE { time::now() } ELSE { observed_peer_at } RETURN NONE;")
                .bind(("registration", rid("connect_registration", &registration.registration_id))).bind(("registration_id", registration.registration_id.clone())).bind(("request", rid("connect_request", &request_key))).bind(("fingerprint", fingerprint)).bind(("node", rid("connect_workstation", &workstation.workstation_id))).bind(("ip", ip)).bind(("peer", peer))).await?;
            let registration = registration_in(&tx, &registration.registration_id, &authority).await?;
            Ok((receipt(&registration, &deployment.deployment_id)?, created))
        }
    }).await?;
    Ok(EnsureResult {
        receipt: committed.value.0,
        created: committed.value.1,
        retries: committed.retries,
    })
}

async fn active_context_in(
    tx: &Tx,
    authority: &RegistryAuthority,
    registration: &RegistrationRow,
    context_revision: i64,
    scope_revision: Option<i64>,
) -> Result<RegisteredContext, TxError> {
    super::configuration_reset::check_user_epoch_in(
        tx,
        &authority.scope_id,
        &authority.access_epoch,
        None,
    )
    .await?;
    let deployment = deployment_in(tx).await?;
    let workstation = workstation_in(tx, &registration.workstation_id).await?;
    if registration.state != "active"
        || registration.contract_version != 2
        || workstation.state != "active"
        || workstation.scope_id != authority.scope_id
        || context_revision < 1
        || scope_revision.is_some_and(|revision| revision != workstation.revision)
    {
        return Err(abort("revoked"));
    }
    let binding = claim_value(
        tx,
        "connect_binding",
        &hash(&[&registration.connect_id]),
        "registration_id",
    )
    .await?;
    if binding.as_deref() != Some(registration.registration_id.as_str()) {
        return Err(abort("revoked"));
    }
    RegisteredContext::new(
        receipt(registration, &deployment.deployment_id)?,
        context_revision as u64,
        workstation.revision as u64,
        deployment.configuration_epoch,
        authority.access_epoch.clone(),
    )
    .map_err(|_| abort("invalid"))
}

/// Validate a direct native registration against current authoritative state.
pub async fn registration_context(
    repo: &Repository,
    authority: &RegistryAuthority,
    registration_id: &str,
    expected_revision: u64,
) -> Result<RegisteredContext, RegistryError> {
    if !valid_identity(registration_id) || expected_revision == 0 {
        return Err(RegistryError::InvalidInput);
    }
    let result = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let authority = authority.clone();
        let id = registration_id.to_owned();
        async move {
            let row = registration_in(&tx, &id, &authority).await?;
            if row.revision as u64 != expected_revision {
                return Err(abort("revision"));
            }
            active_context_in(&tx, &authority, &row, row.revision, None).await
        }
    })
    .await?;
    Ok(result.value)
}

/// Join authoritative native validation to the actual durable admission transaction.
/// Writing the same guard as retirement/rebind prevents acceptance after their fence.
pub async fn validate_registration_in(
    tx: &Tx,
    authority: &RegistryAuthority,
    registration_id: &str,
    expected_revision: u64,
) -> Result<RegisteredContext, TxError> {
    if !valid_identity(registration_id) || expected_revision == 0 {
        return Err(abort("invalid"));
    }
    guard_in(tx).await?;
    if !deployment_in(tx).await?.admission_open {
        return Err(abort("paused"));
    }
    let row = registration_in(tx, registration_id, authority).await?;
    if row.revision as u64 != expected_revision {
        return Err(abort("revision"));
    }
    active_context_in(tx, authority, &row, row.revision, None).await
}

/// Confirmed rebind request. Scope/owner is supplied separately by trusted policy.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct RebindInput {
    pub request_id: String,
    pub registration_id: String,
    pub expected_revision: u64,
    pub workstation_ip: Option<IpAddr>,
    pub installation_marker: String,
}

impl RebindInput {
    pub fn new(
        request: RegistrationEnsureRequest,
        registration_id: String,
        expected_revision: u64,
    ) -> Self {
        Self {
            request_id: request.request_id,
            registration_id,
            expected_revision,
            workstation_ip: request.workstation_ip,
            installation_marker: request.installation_marker,
        }
    }
}

/// IP is mutable metadata. Rebind validates the current authority before ensuring it.
pub async fn rebind(
    repo: &Repository,
    authority: &RegistryAuthority,
    input: &RebindInput,
) -> Result<EnsureResult, RegistryError> {
    registration_context(
        repo,
        authority,
        &input.registration_id,
        input.expected_revision,
    )
    .await?;
    ensure(
        repo,
        authority,
        &RegistrationEnsureRequest::new(
            input.request_id.clone(),
            input.workstation_ip,
            input.installation_marker.clone(),
        ),
    )
    .await
}

/// A retirement fence is not proof that accepted/associated jobs have drained.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct RetirementPending {
    pub registration_id: String,
    pub workspace_id: String,
    pub revision: u64,
}

/// Fence new admission and revoke sessions. Final deletion readiness intentionally
/// stays pending until the owning Jobs transaction proves all active associations drained.
pub async fn begin_retirement(
    repo: &Repository,
    authority: &RegistryAuthority,
    registration_id: &str,
    expected_revision: u64,
) -> Result<RetirementPending, RegistryError> {
    if !valid_identity(registration_id) || expected_revision == 0 {
        return Err(RegistryError::InvalidInput);
    }
    let committed = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let authority = authority.clone();
        let id = registration_id.to_owned();
        async move {
            guard_in(&tx).await?;
            let row = read_registration_in(&tx, &id, &authority).await?;
            let folder = workspace_in(&tx, &row.workspace_id).await?;
            if folder.protected_default || folder.workspace_name == "default" { return Err(abort("protected")); }
            if row.state == "retired" && expected_revision >= (row.revision as u64).saturating_sub(2) && expected_revision <= row.revision as u64 {
                return Ok(RetirementPending { registration_id:id, workspace_id:row.workspace_id, revision:row.revision as u64 });
            }
            if row.state == "retiring" && (row.revision as u64 == expected_revision.saturating_add(1) || row.revision as u64 == expected_revision) {
                return Ok(RetirementPending { registration_id:id, workspace_id:row.workspace_id, revision:row.revision as u64 });
            }
            if row.state != "active" || folder.state != "active" { return Err(abort("revoked")); }
            if row.revision as u64 != expected_revision { return Err(abort("revision")); }
            run(&tx, tx.query("UPDATE $registration SET state='retiring', revision+=1 RETURN NONE; UPDATE $workspace SET state='retiring', revision+=1 RETURN NONE; UPDATE connect_session SET revoked=true WHERE registration_id=$id RETURN NONE;")
                .bind(("registration", rid("connect_registration", &id))).bind(("workspace", rid("connect_workspace", &row.workspace_id))).bind(("id", id.clone()))).await?;
            Ok(RetirementPending { registration_id:id, workspace_id:row.workspace_id, revision:row.revision as u64 + 1 })
        }
    }).await?;
    Ok(committed.value)
}

/// Stable backend proof of fencing, revoked sessions and drained work.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[non_exhaustive]
pub struct RetirementComplete {
    pub deployment_id: String,
    pub registration_id: String,
    pub workspace_id: String,
    pub registration_revision: u64,
    pub receipt_id: String,
    pub admission_fenced: bool,
    pub sessions_revoked: bool,
    pub active_work_drained: bool,
}

/// Pending retains bindings/files; only Complete permits local retirement.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RetirementResult {
    Pending(RetirementPending),
    Complete(RetirementComplete),
}

fn retirement_receipt(row: &RegistrationRow, deployment: &DeploymentState) -> RetirementComplete {
    RetirementComplete {
        deployment_id: deployment.deployment_id.clone(),
        registration_id: row.registration_id.clone(),
        workspace_id: row.workspace_id.clone(),
        registration_revision: row.revision as u64,
        receipt_id: format!(
            "retirement-{}",
            hash(&[
                &deployment.deployment_id,
                &row.scope_id,
                &row.registration_id,
                &row.revision.to_string()
            ])
        ),
        admission_fenced: true,
        sessions_revoked: true,
        active_work_drained: true,
    }
}

/// Finish under the same registry guard that admission/session checks write.
/// Tombstones/history remain; a later same-name creation needs a new marker
/// and identity rather than reviving the retired workspace or registration.
pub async fn finish_retirement(
    repo: &Repository,
    authority: &RegistryAuthority,
    registration_id: &str,
    expected_revision: u64,
) -> Result<RetirementResult, RegistryError> {
    if !valid_identity(registration_id) || expected_revision == 0 {
        return Err(RegistryError::InvalidInput);
    }
    let result = run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        guard_in(&tx).await?;
        let deployment = deployment_in(&tx).await?;
        let row = read_registration_in(&tx, registration_id, authority).await?;
        let folder = workspace_in(&tx, &row.workspace_id).await?;
        if folder.protected_default || folder.workspace_name == "default" {
            return Err(abort("protected"));
        }
        if row.state == "retired"
            && folder.state == "retired"
            && row.revision as u64 == expected_revision
        {
            return Ok(RetirementResult::Complete(retirement_receipt(
                &row,
                &deployment,
            )));
        }
        if row.state != "retiring" || folder.state != "retiring" {
            return Err(abort("revoked"));
        }
        if row.revision as u64 != expected_revision {
            return Err(abort("revision"));
        }
        run(
            &tx,
            tx.query(
                "UPDATE connect_session SET revoked=true WHERE registration_id=$id RETURN NONE",
            )
            .bind(("id", registration_id.to_owned())),
        )
        .await?;
        if super::job_context::has_active_workspace_work_in(
            &tx,
            authority.scope_id(),
            &deployment.deployment_id,
            &row.workspace_id,
        )
        .await?
        {
            return Ok(RetirementResult::Pending(RetirementPending {
                registration_id: row.registration_id,
                workspace_id: row.workspace_id,
                revision: row.revision as u64,
            }));
        }
        run(
            &tx,
            tx.query(
                "UPDATE $registration SET state='retired',revision+=1 RETURN NONE;
            UPDATE $workspace SET state='retired',revision+=1 RETURN NONE;
            DELETE $binding WHERE registration_id=$id RETURN NONE;
            DELETE $name WHERE workspace_id=$workspace_id RETURN NONE;",
            )
            .bind(("registration", rid("connect_registration", registration_id)))
            .bind(("workspace", rid("connect_workspace", &row.workspace_id)))
            .bind(("binding", rid("connect_binding", &hash(&[&row.connect_id]))))
            .bind(("id", registration_id.to_owned()))
            .bind((
                "name",
                rid(
                    "connect_name_claim",
                    &hash(&[&row.workstation_id, &row.workspace_name]),
                ),
            ))
            .bind(("workspace_id", row.workspace_id.clone())),
        )
        .await?;
        let retired = read_registration_in(&tx, registration_id, authority).await?;
        Ok(RetirementResult::Complete(retirement_receipt(
            &retired,
            &deployment,
        )))
    })
    .await?;
    Ok(result.value)
}

/// Fence creation of a genuine attributed causal child against final retirement.
/// Already accepted work may extend while retiring, but a completed retirement
/// can never acquire new causal work after its drained receipt commits.
pub(crate) async fn fence_causal_child_in(
    tx: &Tx,
    scope_id: &str,
    workstation_id: &str,
) -> Result<(), TxError> {
    if !valid_identity(scope_id) || !valid_identity(workstation_id) {
        return Err(abort("invalid"));
    }
    super::configuration_reset::fence_new_work_in(tx).await?;
    guard_in(tx).await?;
    let workstation = workstation_in(tx, workstation_id).await?;
    if workstation.scope_id != scope_id
        || !matches!(workstation.state.as_str(), "active" | "retiring")
    {
        return Err(abort("revoked"));
    }
    Ok(())
}

/// Atomic forward migration, checkpointed with the schema version by operator bootstrap.
/// Existing origins and registration-to-workstation mappings remain immutable.
pub(crate) async fn migrate_workstation_contract_in(tx: &Tx) -> Result<(), TxError> {
    guard_in(tx).await?;
    let mut version = run(
        tx,
        tx.query("SELECT VALUE contract_version FROM ONLY connect_control:configuration"),
    )
    .await?;
    if take::<Option<i64>>(&mut version, 0)? == Some(2) {
        return Ok(());
    }
    let epoch = format!("configuration-{}", crate::store::new_id());
    run(tx, tx.query("UPSERT connect_control:configuration SET configuration_epoch=configuration_epoch ?? $epoch, admission_open=admission_open ?? true RETURN NONE").bind(("epoch",epoch))).await?;
    let mut rows = run(tx, tx.query("SELECT workstation_id, scope_id, installation_hash, workstation_ip ?? '' AS workstation_ip, state, revision FROM connect_workstation WHERE state='active' LIMIT 10001")).await?;
    let rows: Vec<WorkstationRow> = take(&mut rows, 0)?;
    if rows.len() > 10000 {
        return Err(abort("invalid"));
    }
    // Registration revocation is independent of workstation lifecycle. Retain an
    // inactive v2 row when no old binding can authorize new admission, while
    // leaving accepted causal jobs under the unchanged workstation fence.
    let mut migrations = Vec::with_capacity(rows.len());
    for row in rows {
        let mut states = run(tx, tx.query("SELECT VALUE state FROM connect_registration WHERE scope_id=$scope AND workstation_id=$node AND (contract_version ?? 1)<2 GROUP BY state LIMIT 5")
            .bind(("scope",row.scope_id.clone())).bind(("node",row.workstation_id.clone()))).await?;
        let states: Vec<String> = take(&mut states, 0)?;
        let state = if states.iter().any(|state| state == "active") {
            "active"
        } else if states.iter().any(|state| state == "retiring") {
            "retiring"
        } else if !states.is_empty() && states.iter().all(|state| state == "retired") {
            "retired"
        } else {
            "revoked"
        };
        migrations.push((row, state));
    }
    run(tx, tx.query("UPDATE connect_registration SET state='legacy_read_only', revision+=1 WHERE state='active' AND (contract_version ?? 1) < 2 RETURN NONE; UPDATE connect_session SET revoked=true RETURN NONE; DELETE connect_binding RETURN NONE;")).await?;
    for (row, state) in migrations {
        let registration_id = format!("workstation-{}", row.workstation_id);
        let binding_key = hash(&[&row.workstation_id]);
        let ip = if row.workstation_ip.is_empty() {
            None
        } else {
            Some(row.workstation_ip)
        };
        run(tx, tx.query("CREATE $registration SET registration_id=$id, scope_id=$scope, workstation_id=$node, workstation_ip=$ip, connect_id=$node, contract_version=2, state=$state, revision=1, created_at=time::now() RETURN NONE")
            .bind(("registration",rid("connect_registration",&registration_id))).bind(("id",registration_id.clone())).bind(("scope",row.scope_id)).bind(("node",row.workstation_id.clone())).bind(("ip",ip)).bind(("state",state))).await?;
        if state == "active" {
            run(
                tx,
                tx.query("CREATE $binding SET connect_id=$node,registration_id=$id RETURN NONE")
                    .bind(("binding", rid("connect_binding", &binding_key)))
                    .bind(("node", row.workstation_id))
                    .bind(("id", registration_id)),
            )
            .await?;
        }
    }
    run(
        tx,
        tx.query("UPDATE connect_control:configuration SET contract_version=2 RETURN NONE"),
    )
    .await?;
    Ok(())
}

#[derive(SurrealValue)]
struct RateRow {
    attempts: i64,
    live: bool,
}

async fn allow_resolution(
    repo: &Repository,
    authority: &RegistryAuthority,
) -> Result<bool, RegistryError> {
    let key = hash(&[&authority.scope_id, &authority.credential_id]);
    let result = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let key = key.clone();
        async move {
            guard_in(&tx).await?;
            let mut response = run(
                &tx,
                tx.query("SELECT attempts, (started_at > time::now() - 1m) AS live FROM ONLY $row")
                    .bind(("row", rid("connect_rate_window", &key))),
            )
            .await?;
            let existing: Option<RateRow> = take(&mut response, 0)?;
            if existing
                .as_ref()
                .is_some_and(|row| row.live && row.attempts >= RESOLUTION_ATTEMPTS_PER_MINUTE)
            {
                return Ok(false);
            }
            let attempts = existing
                .as_ref()
                .filter(|row| row.live)
                .map_or(1, |row| row.attempts + 1);
            let fresh = existing.as_ref().is_some_and(|row| row.live);
            // Preserve an existing window without referring to its field from
            // an UPSERT assignment: SurrealDB 3 resolves that field as NONE.
            // The registry guard serializes the read and either mutation.
            let statement = if fresh {
                "UPDATE $row SET attempts=$attempts RETURN NONE"
            } else {
                "UPSERT $row SET attempts=$attempts, started_at=time::now() RETURN NONE"
            };
            run(
                &tx,
                tx.query(statement)
                    .bind(("row", rid("connect_rate_window", &key)))
                    .bind(("attempts", attempts)),
            )
            .await?;
            Ok(true)
        }
    })
    .await?;
    Ok(result.value)
}

async fn session_in(tx: &Tx, key: &str) -> Result<SessionRow, TxError> {
    let mut result = run(tx, tx.query("SELECT scope_id, credential_id, access_epoch, configuration_epoch, registration_id, registration_revision, context_revision, scope_revision, <string>expires_at AS expires_at, (expires_at > time::now()) AS live, revoked FROM ONLY $row").bind(("row", rid("connect_session", key)))).await?;
    take::<Option<SessionRow>>(&mut result, 0)?.ok_or_else(|| abort("missing"))
}

/// Mint an expiring browser context for an existing active registration only.
pub async fn create_session(
    repo: &Repository,
    authority: &RegistryAuthority,
    connect_id: &ConnectId,
) -> Result<CreatedSession, RegistryError> {
    if !allow_resolution(repo, authority).await? {
        return Err(RegistryError::RateLimited);
    }
    let token = tokio::task::spawn_blocking(super::home::random_secret)
        .await
        .map_err(|_| RegistryError::Unconfigured)?
        .map_err(|_| RegistryError::Unconfigured)?;
    let key = hash(&[&token]);
    let binding_key = hash(&[&connect_id.to_string()]);
    let result = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let authority = authority.clone();
        let (key, binding_key) = (key.clone(), binding_key.clone());
        async move {
            let revision = guard_in(&tx).await?;
            let deployment = deployment_in(&tx).await?;
            if !deployment.admission_open { return Err(abort("paused")); }
            super::configuration_reset::check_user_epoch_in(&tx, &authority.scope_id, &authority.access_epoch, None).await?;
            let id = claim_value(&tx, "connect_binding", &binding_key, "registration_id").await?.ok_or_else(|| abort("missing"))?;
            let registration = registration_in(&tx, &id, &authority).await?;
            let context = active_context_in(&tx, &authority, &registration, revision, None).await?;
            run(&tx, tx.query("CREATE $row SET scope_id=$scope, credential_id=$credential, access_epoch=$principal_epoch, configuration_epoch=$configuration, registration_id=$registration, registration_revision=$registration_revision, context_revision=$context_revision, scope_revision=$scope_revision, created_at=time::now(), expires_at=time::now()+$ttl, revoked=false RETURN NONE")
                .bind(("row", rid("connect_session", &key))).bind(("scope", authority.scope_id.clone())).bind(("credential", authority.credential_id.clone())).bind(("principal_epoch", authority.access_epoch.clone())).bind(("configuration", context.configuration_epoch.clone())).bind(("registration", context.registration_id.clone())).bind(("registration_revision", context.registration_revision as i64)).bind(("context_revision", context.context_revision as i64)).bind(("scope_revision", context.scope_revision as i64)).bind(("ttl", DbDuration::from(SESSION_TTL)))).await?;
            let session = session_in(&tx, &key).await?;
            Ok((context, session.expires_at))
        }
    }).await?;
    Ok(CreatedSession {
        token: SessionToken(token),
        context: result.value.0,
        expires_at: result.value.1,
    })
}

fn session_key(token: &str) -> Result<String, RegistryError> {
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(RegistryError::NotFound);
    }
    Ok(hash(&[token]))
}

/// Recheck expiry/revocation/current registry/epochs on every protected boundary.
pub async fn validate_session(
    repo: &Repository,
    authority: &RegistryAuthority,
    token: &str,
    expected_context_revision: Option<u64>,
) -> Result<ValidatedSession, RegistryError> {
    let key = session_key(token)?;
    let result =
        run_tx(repo.db(), RetryPolicy::default(), |tx| {
            let authority = authority.clone();
            let key = key.clone();
            async move {
                validate_session_key_in(&tx, &authority, &key, expected_context_revision).await
            }
        })
        .await?;
    Ok(result.value)
}

async fn validate_session_key_in(
    tx: &Tx,
    authority: &RegistryAuthority,
    key: &str,
    expected_context_revision: Option<u64>,
) -> Result<ValidatedSession, TxError> {
    let session = session_in(tx, key).await?;
    if !session.live
        || session.revoked
        || session.scope_id != authority.scope_id
        || session.credential_id != authority.credential_id
        || session.access_epoch != authority.access_epoch
    {
        return Err(abort("revoked"));
    }
    if expected_context_revision.is_some_and(|revision| revision != session.context_revision as u64)
    {
        return Err(abort("revision"));
    }
    let registration = registration_in(tx, &session.registration_id, authority).await?;
    if registration.revision != session.registration_revision {
        return Err(abort("revoked"));
    }
    let context = active_context_in(
        tx,
        authority,
        &registration,
        session.context_revision,
        Some(session.scope_revision),
    )
    .await?;
    if context.configuration_epoch != session.configuration_epoch {
        return Err(abort("revoked"));
    }
    Ok(ValidatedSession {
        context,
        expires_at: session.expires_at,
    })
}

/// Validate a web proof inside durable acceptance while sharing the retirement
/// and logout guard. The token is not forwarded in URLs or persisted in jobs.
pub async fn validate_session_for_admission_in(
    tx: &Tx,
    authority: &RegistryAuthority,
    token: &str,
    expected_context_revision: u64,
) -> Result<ValidatedSession, TxError> {
    let key = session_key(token).map_err(|_| abort("missing"))?;
    guard_in(tx).await?;
    if !deployment_in(tx).await?.admission_open {
        return Err(abort("paused"));
    }
    validate_session_key_in(tx, authority, &key, Some(expected_context_revision)).await
}

/// Idempotently revoke only the caller relay's session; no unknown-ID disclosure.
pub async fn revoke_session(
    repo: &Repository,
    authority: &RegistryAuthority,
    token: &str,
) -> Result<(), RegistryError> {
    let key = match session_key(token) {
        Ok(key) => key,
        Err(_) => return Ok(()),
    };
    run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let authority = authority.clone();
        let key = key.clone();
        async move {
            guard_in(&tx).await?;
            run(&tx, tx.query("UPDATE $row SET revoked=true WHERE scope_id=$scope AND credential_id=$credential AND access_epoch=$principal_epoch RETURN NONE")
                .bind(("row", rid("connect_session", &key))).bind(("scope", authority.scope_id)).bind(("credential", authority.credential_id)).bind(("principal_epoch", authority.access_epoch))).await?;
            Ok(())
        }
    }).await?;
    Ok(())
}
