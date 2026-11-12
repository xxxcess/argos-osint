//! Authenticated, one-shot global user-configuration cutover coordinator.
//!
//! This module is deliberately unreferenced until its hooks are integrated.
//! Production prepare is unsupported while legacy readers/importers remain.
//! No HTTP request can supply a build gate, loaded policy, cache ACK or path.

mod files;
mod new_work_barrier;
mod process_cache;
#[cfg(all(test, unix))]
#[path = "configuration_reset/tests.rs"]
mod tests;

pub use files::BackendFiles;
pub(crate) use new_work_barrier::fence_new_work_in;
pub use process_cache::clear_process_cache;

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use fleetworks_contracts::context::CredentialPurpose;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use surrealdb::types::SurrealValue;

use self::files::Inventory;
use super::connect_registry;
use super::jobs;
use super::repo::Repository;
use super::tx::run_tx;
use super::tx::RetryPolicy;
use super::tx::Tx;
use super::tx::TxError;
use crate::config_commit::ConfigLock;

pub const SCHEMA: &str = include_str!("schema/configuration_reset.surql");
const OWNER_CONTRACT: u16 = 1;

/// Failures contain classifications, never credentials or filesystem payloads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ResetError {
    Invalid,
    Forbidden,
    Conflict,
    Missing,
    UnsupportedBuild,
    UnreviewedSources,
    UnsafeFiles,
    Draining,
    ReprovisioningRequired,
    Unavailable,
}
impl std::fmt::Display for ResetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Invalid => "Invalid bounded reset identity",
            Self::Forbidden => "Independent operator reset authority required",
            Self::Conflict => "Reset identity or frozen state changed",
            Self::Missing => "Reset record unavailable",
            Self::UnsupportedBuild => {
                "Full current-only configuration cutover support is not installed"
            }
            Self::UnreviewedSources => "Unreviewed credential-bearing source remains",
            Self::UnsafeFiles => "Frozen owned configuration object changed",
            Self::Draining => "Credential-consuming accepted work has not drained",
            Self::ReprovisioningRequired => {
                "Fresh user policy or private owner epoch reload is still required"
            }
            Self::Unavailable => "Reset authority unavailable",
        })
    }
}
impl std::error::Error for ResetError {}
impl From<TxError> for ResetError {
    fn from(error: TxError) -> Self {
        match error {
            TxError::Abort(code) if code == "configuration_reset:conflict" => Self::Conflict,
            TxError::Abort(code) if code == "configuration_reset:missing" => Self::Missing,
            TxError::Abort(code) if code == "configuration_reset:forbidden" => Self::Forbidden,
            TxError::Abort(code) if code == "configuration_reset:draining" => Self::Draining,
            _ => Self::Unavailable,
        }
    }
}
fn abort(code: &str) -> TxError {
    TxError::Abort(format!("configuration_reset:{code}"))
}
fn valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
fn digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}
fn key(scope: &str, id: &str) -> String {
    digest(format!("{}:{scope}{}:{id}", scope.len(), id.len()).as_bytes())
}

/// Built from authenticated Operator policy by the API, never deserialized.
#[derive(Clone)]
pub struct ResetAuthority {
    scope_id: String,
    credential_id: String,
}
impl ResetAuthority {
    pub fn new(scope_id: &str, credential_id: &str) -> Result<Self, ResetError> {
        if !valid(scope_id) || !valid(credential_id) {
            return Err(ResetError::Invalid);
        }
        Ok(Self {
            scope_id: scope_id.into(),
            credential_id: credential_id.into(),
        })
    }
    pub fn scope_id(&self) -> &str {
        &self.scope_id
    }
    pub fn credential_id(&self) -> &str {
        &self.credential_id
    }
}

/// Opaque digest from the actually matched/loaded policy. Never diagnostic JSON.
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct CredentialFingerprint([u8; 32]);
impl CredentialFingerprint {
    pub fn from_validated_policy_digest(value: [u8; 32]) -> Self {
        Self(value)
    }
    fn encoded(&self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

/// One loaded user credential. No Deserialize or bearer bytes are accepted.
#[derive(Clone)]
pub struct LoadedUserCredential {
    pub scope_id: String,
    pub purpose: CredentialPurpose,
    pub access_epoch: String,
    pub fingerprint: CredentialFingerprint,
}

/// Must come from each authenticated service's actual in-memory owner state.
/// Reading a mounted secret file or echoing DB epochs is not an observation.
#[derive(Clone)]
pub struct OwnerObservation {
    pub scope_id: String,
    pub deployment_id: String,
    pub service: String,
    pub instance_id: String,
    pub access_epoch: String,
    pub configuration_epoch: String,
    pub cache_epoch: String,
    pub reader_contract: u16,
}

/// Server-owned observation; deliberately not Debug/Serialize/Deserialize.
#[derive(Clone)]
pub struct RuntimeObservation {
    pub scope_id: String,
    pub deployment_id: String,
    pub independent_operator_id: String,
    pub independent_operator_fingerprint: CredentialFingerprint,
    pub users: Vec<LoadedUserCredential>,
    pub owners: Vec<OwnerObservation>,
}

pub type ObservationFuture<'a> =
    Pin<Box<dyn Future<Output = Result<RuntimeObservation, ResetError>> + Send + 'a>>;
pub type ClearCachesFuture<'a> = Pin<Box<dyn Future<Output = Result<(), ResetError>> + Send + 'a>>;

/// Implement using loaded gateway policy and authenticated owner RPCs. No
/// endpoint parameter, caller JSON proof or configuration path belongs here.
/// Enumerate every loaded Native/WebRelay credential; do not filter by scope.
/// A mixed-scope user policy is unsupported for this global configuration owner.
pub trait PolicyObserver: Send + Sync {
    fn observe<'a>(&'a self, authority: &'a ResetAuthority) -> ObservationFuture<'a>;
    fn clear_owned_caches<'a>(
        &'a self,
        authority: &'a ResetAuthority,
        configuration_epoch: &'a str,
    ) -> ClearCachesFuture<'a>;
}

/// Common exact protocol shared with the external inventory utility.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ResetRequest {
    pub version: u16,
    pub reset_id: String,
    pub plan_sha256: String,
    pub deployment_id: String,
    pub expected_configuration_epoch: String,
    pub expected_access_epoch: String,
}
impl ResetRequest {
    pub fn new(
        reset_id: String,
        plan_sha256: String,
        deployment_id: String,
        configuration_epoch: String,
        access_epoch: String,
    ) -> Self {
        Self {
            version: 1,
            reset_id,
            plan_sha256,
            deployment_id,
            expected_configuration_epoch: configuration_epoch,
            expected_access_epoch: access_epoch,
        }
    }
    fn validate(&self) -> Result<(), ResetError> {
        if self.version != 1
            || ![
                &self.reset_id,
                &self.deployment_id,
                &self.expected_configuration_epoch,
                &self.expected_access_epoch,
            ]
            .into_iter()
            .all(|value| valid(value))
            || self.plan_sha256.len() != 64
            || !self
                .plan_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
        {
            return Err(ResetError::Invalid);
        }
        Ok(())
    }
}

/// A local phase report is not evidence of backend authority or backend work.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct LocalReceipt {
    pub target_name: String,
    pub inventory_sha256: String,
    pub state: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CompletionRequest {
    pub version: u16,
    pub reset_id: String,
    pub plan_sha256: String,
    pub deployment_id: String,
    pub expected_configuration_epoch: String,
    pub expected_access_epoch: String,
    pub local_receipts: Vec<LocalReceipt>,
}
impl CompletionRequest {
    pub fn common(&self) -> ResetRequest {
        ResetRequest {
            version: self.version,
            reset_id: self.reset_id.clone(),
            plan_sha256: self.plan_sha256.clone(),
            deployment_id: self.deployment_id.clone(),
            expected_configuration_epoch: self.expected_configuration_epoch.clone(),
            expected_access_epoch: self.expected_access_epoch.clone(),
        }
    }
    fn validate(&self) -> Result<(), ResetError> {
        self.common().validate()?;
        let mut names = BTreeSet::new();
        if self.local_receipts.is_empty()
            || self.local_receipts.len() > 64
            || self.local_receipts.iter().any(|receipt| {
                !valid(&receipt.target_name)
                    || !names.insert(&receipt.target_name)
                    || receipt.inventory_sha256 != self.plan_sha256
                    || receipt.state != "files_reset"
            })
        {
            return Err(ResetError::Invalid);
        }
        Ok(())
    }
}

/// No secrets/paths/fingerprints are returned to the operator or native clients.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ResetReceipt {
    pub version: u16,
    pub reset_id: String,
    pub plan_sha256: String,
    pub deployment_id: String,
    pub scope_id: String,
    pub configuration_epoch: String,
    pub access_epoch: String,
    pub state: String,
    pub admission_fenced: bool,
    pub first_use_blocked: bool,
    pub work_drained: bool,
    pub user_access_revoked: bool,
    pub sessions_revoked: bool,
    pub backend_configuration_cleared: bool,
    pub legacy_reload_disabled: bool,
}

#[derive(Clone, SurrealValue)]
struct Row {
    scope_id: String,
    reset_id: String,
    plan_sha256: String,
    deployment_id: String,
    old_configuration_epoch: String,
    old_access_epoch: String,
    configuration_epoch: String,
    access_epoch: String,
    phase: String,
    inventory_json: String,
    revoked_json: String,
    local_receipts_json: String,
    configuration_revision: i64,
}
#[derive(SurrealValue)]
struct ScopeRow {
    active_reset: String,
    access_epoch: String,
    first_use_blocked: bool,
    revoked_json: String,
}

struct CurrentOnlyBuild;
fn running_build_support() -> Option<CurrentOnlyBuild> {
    // Deliberately false while any installation legacy importer, fallback,
    // no-context adapter or untracked credential consumer remains supported.
    // Strict new-network readers alone are insufficient to flip this gate.
    None
}

/// The actual compiled owner reader contract, never an environment assertion.
pub fn owner_reader_contract() -> u16 {
    if running_build_support().is_some() {
        OWNER_CONTRACT
    } else {
        0
    }
}

/// Coordinator injected by server startup with its actual config/policy owners.
pub struct Coordinator {
    files: BackendFiles,
    observer: Arc<dyn PolicyObserver>,
    required_owners: BTreeSet<String>,
    support: Option<CurrentOnlyBuild>,
}
impl Coordinator {
    pub fn new(
        files: BackendFiles,
        observer: Arc<dyn PolicyObserver>,
        required_owners: BTreeSet<String>,
    ) -> Result<Self, ResetError> {
        if required_owners.is_empty()
            || required_owners.len() > 32
            || required_owners.iter().any(|name| !valid(name))
        {
            return Err(ResetError::Invalid);
        }
        Ok(Self {
            files,
            observer,
            required_owners,
            support: running_build_support(),
        })
    }
    fn gate(&self) -> Result<(), ResetError> {
        self.support
            .as_ref()
            .map(|_| ())
            .ok_or(ResetError::UnsupportedBuild)
    }
    fn check_observation(
        &self,
        authority: &ResetAuthority,
        deployment_id: &str,
        observed: &RuntimeObservation,
    ) -> Result<(), ResetError> {
        if observed.scope_id != authority.scope_id
            || observed.deployment_id != deployment_id
            || observed.independent_operator_id != authority.credential_id
            || observed.users.len() > 64
            || observed.owners.len() > 32
            || observed.users.iter().any(|user| {
                user.scope_id != authority.scope_id
                    || !matches!(
                        user.purpose,
                        CredentialPurpose::Native | CredentialPurpose::WebRelay
                    )
                    || !valid(&user.access_epoch)
                    || user.fingerprint == observed.independent_operator_fingerprint
            })
        {
            return Err(ResetError::Forbidden);
        }
        let mut owners = BTreeSet::new();
        if observed.owners.iter().any(|owner| {
            owner.scope_id != authority.scope_id
                || owner.deployment_id != deployment_id
                || !owners.insert(owner.service.clone())
                || !valid(&owner.instance_id)
                || owner.reader_contract != OWNER_CONTRACT
        }) || owners != self.required_owners
        {
            return Err(ResetError::UnsupportedBuild);
        }
        Ok(())
    }
    async fn lock(&self) -> Result<ConfigLock, ResetError> {
        let files = self.files.clone();
        tokio::task::spawn_blocking(move || files.lock())
            .await
            .map_err(|_| ResetError::Unavailable)?
    }

    /// Refuses unsupported builds before lock creation, fencing or epoch change.
    pub async fn prepare(
        &self,
        repo: &Repository,
        authority: &ResetAuthority,
        request: &ResetRequest,
    ) -> Result<ResetReceipt, ResetError> {
        self.gate()?;
        request.validate()?;
        let observed = self.observer.observe(authority).await?;
        self.check_observation(authority, &request.deployment_id, &observed)?;
        let lock = self.lock().await?;
        let files = self.files.clone();
        let held = lock.clone();
        let inventory = tokio::task::spawn_blocking(move || files.inventory(&held))
            .await
            .map_err(|_| ResetError::Unavailable)??;
        let inventory_json =
            serde_json::to_string(&inventory).map_err(|_| ResetError::Unavailable)?;
        let configuration_epoch = format!("reset-configuration-{}", crate::store::new_id());
        let access_epoch = format!("reset-access-{}", crate::store::new_id());
        let fingerprints: BTreeSet<_> = observed
            .users
            .iter()
            .map(|user| user.fingerprint.encoded())
            .collect();
        let revoked_json =
            serde_json::to_string(&fingerprints).map_err(|_| ResetError::Unavailable)?;
        let row=run_tx(repo.db(),RetryPolicy::default(),|tx|{
            let inventory_json=&inventory_json;let configuration_epoch=&configuration_epoch;let access_epoch=&access_epoch;let revoked_json=&revoked_json;
            let observed=&observed;let fingerprints=&fingerprints;let revision=inventory.revision;
            async move {
                registry_guard(&tx).await?;
                let id=key(&authority.scope_id,&request.reset_id);
                if let Some(existing)=row_in(&tx,&id).await? {matches_request(&existing,authority,request)?;return refresh_drain(&tx,existing).await;}
                let deployment=connect_registry::deployment_state_in(&tx).await?;
                if deployment.deployment_id!=request.deployment_id||deployment.configuration_epoch!=request.expected_configuration_epoch
                    ||!deployment.admission_open||observed.users.is_empty()||observed.users.iter().any(|user|user.access_epoch!=request.expected_access_epoch){return Err(abort("conflict"));}
                let scope=scope_in(&tx,&authority.scope_id).await?;
                if scope.as_ref().is_some_and(|scope|scope.first_use_blocked||!scope.active_reset.is_empty()||scope.access_epoch!=request.expected_access_epoch){return Err(abort("conflict"));}
                let mut all:BTreeSet<String>=scope.as_ref().map(|scope|serde_json::from_str(&scope.revoked_json)).transpose().map_err(|_|abort("conflict"))?.unwrap_or_default();
                all.extend(fingerprints.iter().cloned());
                jobs::run(&tx,tx.query("CREATE $record SET scope_id=$scope,reset_id=$reset,plan_sha256=$plan,deployment_id=$deployment,old_configuration_epoch=$old_configuration,old_access_epoch=$old_access,configuration_epoch=$configuration,access_epoch=$principal_epoch,phase='draining',inventory_json=$inventory,revoked_json=$revoked,local_receipts_json='',configuration_revision=$revision,operator_credential_id=$operator RETURN NONE;
                    UPSERT $scope_record SET scope_id=$scope,active_reset=$reset,access_epoch=$principal_epoch,first_use_blocked=true,revoked_json=$all RETURN NONE;
                    UPDATE connect_control:configuration SET configuration_epoch=$configuration,admission_open=false RETURN NONE;
                    UPDATE connect_session SET revoked=true WHERE scope_id=$scope RETURN NONE;")
                    .bind(("record",jobs::rid("configuration_reset",&id))).bind(("scope_record",jobs::rid("configuration_reset_scope",&key(&authority.scope_id,"scope"))))
                    .bind(("scope",authority.scope_id.clone())).bind(("reset",request.reset_id.clone())).bind(("plan",request.plan_sha256.clone())).bind(("deployment",request.deployment_id.clone()))
                    .bind(("old_configuration",request.expected_configuration_epoch.clone())).bind(("old_access",request.expected_access_epoch.clone()))
                    .bind(("configuration",configuration_epoch.clone())).bind(("principal_epoch",access_epoch.clone())).bind(("inventory",inventory_json.clone())).bind(("revoked",revoked_json.clone()))
                    .bind(("all",serde_json::to_string(&all).map_err(|_|abort("conflict"))?)).bind(("revision",revision as i64)).bind(("operator",authority.credential_id.clone()))).await?;
                for fingerprint in fingerprints {
                    jobs::run(&tx, tx.query("UPSERT $record SET fingerprint=$fingerprint,scope_id=$scope,reset_id=$reset RETURN NONE")
                        .bind(("record", jobs::rid("configuration_reset_revoked", fingerprint)))
                        .bind(("fingerprint", fingerprint.clone())).bind(("scope", authority.scope_id.clone())).bind(("reset", request.reset_id.clone()))).await?;
                }
                refresh_drain(&tx,row_in(&tx,&id).await?.ok_or_else(||abort("missing"))?).await
            }
        }).await?;
        drop(lock);
        self.receipt(repo, row.value).await
    }

    pub async fn status(
        &self,
        repo: &Repository,
        authority: &ResetAuthority,
        request: &ResetRequest,
    ) -> Result<ResetReceipt, ResetError> {
        request.validate()?;
        let row = self.load(repo, authority, request).await?;
        self.receipt(repo, row).await
    }

    async fn load(
        &self,
        repo: &Repository,
        authority: &ResetAuthority,
        request: &ResetRequest,
    ) -> Result<Row, ResetError> {
        let result = run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
            let row = row_in(&tx, &key(&authority.scope_id, &request.reset_id))
                .await?
                .ok_or_else(|| abort("missing"))?;
            matches_request(&row, authority, request)?;
            Ok(row)
        })
        .await?;
        Ok(result.value)
    }

    /// Clear exact frozen backend payloads only after the actual global drain.
    pub async fn complete(
        &self,
        repo: &Repository,
        authority: &ResetAuthority,
        input: &CompletionRequest,
    ) -> Result<ResetReceipt, ResetError> {
        self.gate()?;
        input.validate()?;
        let request = input.common();
        let row = self.load(repo, authority, &request).await?;
        if row.phase == "complete" || row.phase == "backend_complete" {
            if serde_json::to_string(&input.local_receipts).map_err(|_| ResetError::Invalid)?
                != row.local_receipts_json
            {
                return Err(ResetError::Conflict);
            }
            return self.receipt(repo, row).await;
        }
        let observed = self.observer.observe(authority).await?;
        self.check_observation(authority, &request.deployment_id, &observed)?;
        let lock = self.lock().await?;
        let row=run_tx(repo.db(),RetryPolicy::default(),|tx|{let request=&request;async move{
            registry_guard(&tx).await?;let mut row=row_in(&tx,&key(&authority.scope_id,&request.reset_id)).await?.ok_or_else(||abort("missing"))?;
            matches_request(&row,authority,&request)?;if !drained(&tx).await?{return Err(abort("draining"));}
            if !matches!(row.phase.as_str(),"prepared"|"clearing"){return Err(abort("conflict"));}
            jobs::run(&tx,tx.query("UPDATE $record SET phase='clearing',local_receipts_json=$receipts RETURN NONE")
                .bind(("record",jobs::rid("configuration_reset",&key(&authority.scope_id,&request.reset_id))))
                .bind(("receipts",serde_json::to_string(&input.local_receipts).map_err(|_|abort("conflict"))?))).await?;
            row.phase="clearing".into();Ok(row)
        }}).await?.value;
        let inventory: Inventory =
            serde_json::from_str(&row.inventory_json).map_err(|_| ResetError::Unavailable)?;
        let files = self.files.clone();
        let held = lock.clone();
        tokio::task::spawn_blocking(move || files.clear(&held, &inventory, true))
            .await
            .map_err(|_| ResetError::Unavailable)??;
        drop(lock);
        self.observer
            .clear_owned_caches(authority, &row.configuration_epoch)
            .await?;
        let current = self.observer.observe(authority).await?;
        self.check_observation(authority, &request.deployment_id, &current)?;
        if current.owners.iter().any(|owner| {
            owner.configuration_epoch != row.configuration_epoch
                || owner.cache_epoch != row.configuration_epoch
        }) {
            return Err(ResetError::ReprovisioningRequired);
        }
        let row=run_tx(repo.db(),RetryPolicy::default(),|tx|{let request=&request;async move{
            registry_guard(&tx).await?;if !drained(&tx).await?{return Err(abort("draining"));}
            jobs::run(&tx,tx.query("UPDATE $record SET phase='backend_complete' WHERE phase='clearing' RETURN NONE")
                .bind(("record",jobs::rid("configuration_reset",&key(&authority.scope_id,&request.reset_id))))).await?;
            row_in(&tx,&key(&authority.scope_id,&request.reset_id)).await?.ok_or_else(||abort("missing"))
        }}).await?.value;
        self.receipt(repo, row).await
    }

    /// Wait for real new policy/owner observations; never rotate mounted secrets.
    pub async fn finalize(
        &self,
        repo: &Repository,
        authority: &ResetAuthority,
        input: &CompletionRequest,
    ) -> Result<ResetReceipt, ResetError> {
        self.gate()?;
        input.validate()?;
        let request = input.common();
        let row = self.load(repo, authority, &request).await?;
        if row.phase == "complete" {
            return self.receipt(repo, row).await;
        }
        if row.phase != "backend_complete" {
            return Err(ResetError::Conflict);
        }
        if serde_json::to_string(&input.local_receipts).map_err(|_| ResetError::Invalid)?
            != row.local_receipts_json
        {
            return Err(ResetError::Conflict);
        }
        let observed = self.observer.observe(authority).await?;
        self.check_observation(authority, &request.deployment_id, &observed)?;
        let revoked = run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
            let scope = scope_in(&tx, &authority.scope_id)
                .await?
                .ok_or_else(|| abort("missing"))?;
            serde_json::from_str::<BTreeSet<String>>(&scope.revoked_json)
                .map_err(|_| abort("conflict"))
        })
        .await?
        .value;
        let captured: BTreeSet<String> =
            serde_json::from_str(&row.revoked_json).map_err(|_| ResetError::Unavailable)?;
        if !captured.is_subset(&revoked) {
            return Err(ResetError::Unavailable);
        }
        if observed.users.is_empty()
            || observed.users.iter().any(|user| {
                user.access_epoch != row.access_epoch
                    || revoked.contains(&user.fingerprint.encoded())
            })
            || observed.owners.iter().any(|owner| {
                owner.access_epoch != row.access_epoch
                    || owner.configuration_epoch != row.configuration_epoch
                    || owner.cache_epoch != row.configuration_epoch
            })
        {
            return Err(ResetError::ReprovisioningRequired);
        }
        let current_users = &observed.users;
        let was_revoked = run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
            for user in current_users {
                if fingerprint_revoked_in(&tx, &user.fingerprint).await? {
                    return Ok(true);
                }
            }
            Ok(false)
        })
        .await?
        .value;
        if was_revoked {
            return Err(ResetError::ReprovisioningRequired);
        }
        let row=run_tx(repo.db(),RetryPolicy::default(),|tx|{let request=&request;let row=&row;async move{
            registry_guard(&tx).await?;let current=row_in(&tx,&key(&authority.scope_id,&request.reset_id)).await?.ok_or_else(||abort("missing"))?;matches_request(&current,authority,&request)?;
            if current.phase=="complete"{return Ok(current);}
            if current.phase!="backend_complete"||!drained(&tx).await?{return Err(abort("conflict"));}
            jobs::run(&tx,tx.query("UPDATE $record SET phase='complete' RETURN NONE;
                UPDATE $scope SET active_reset='',first_use_blocked=false RETURN NONE;
                UPDATE connect_control:configuration SET admission_open=true WHERE configuration_epoch=$configuration RETURN NONE;")
                .bind(("record",jobs::rid("configuration_reset",&key(&authority.scope_id,&request.reset_id))))
                .bind(("scope",jobs::rid("configuration_reset_scope",&key(&authority.scope_id,"scope")))).bind(("configuration",row.configuration_epoch.clone()))).await?;
            row_in(&tx,&key(&authority.scope_id,&request.reset_id)).await?.ok_or_else(||abort("missing"))
        }}).await?.value;
        self.receipt(repo, row).await
    }

    async fn receipt(&self, repo: &Repository, row: Row) -> Result<ResetReceipt, ResetError> {
        let complete = row.phase == "complete";
        // Completion durably proves the old epoch drained. New accepted work
        // after first-use opens cannot invalidate an uncertain-response replay.
        let work_drained = if complete {
            true
        } else {
            run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
                drained(&tx).await
            })
            .await?
            .value
        };
        Ok(ResetReceipt {
            version: 1,
            reset_id: row.reset_id,
            plan_sha256: row.plan_sha256,
            deployment_id: row.deployment_id,
            scope_id: row.scope_id,
            configuration_epoch: row.configuration_epoch,
            access_epoch: row.access_epoch,
            state: row.phase.clone(),
            admission_fenced: !complete,
            first_use_blocked: !complete,
            work_drained,
            user_access_revoked: true,
            sessions_revoked: true,
            backend_configuration_cleared: matches!(
                row.phase.as_str(),
                "backend_complete" | "complete"
            ),
            legacy_reload_disabled: self.support.is_some(),
        })
    }
}

async fn registry_guard(tx: &Tx) -> Result<(), TxError> {
    jobs::run(
        tx,
        tx.query(
            "UPSERT connect_registry_guard:singleton SET revision=(revision ?? 0)+1 RETURN NONE",
        ),
    )
    .await?;
    Ok(())
}
async fn row_in(tx: &Tx, id: &str) -> Result<Option<Row>, TxError> {
    let mut result=jobs::run(tx,tx.query("SELECT scope_id,reset_id,plan_sha256,deployment_id,old_configuration_epoch,old_access_epoch,configuration_epoch,access_epoch,phase,inventory_json,revoked_json,local_receipts_json,configuration_revision FROM ONLY $row").bind(("row",jobs::rid("configuration_reset",id)))).await?;
    jobs::take(&mut result, 0)
}
async fn scope_in(tx: &Tx, scope: &str) -> Result<Option<ScopeRow>, TxError> {
    let mut result = jobs::run(
        tx,
        tx.query("SELECT active_reset,access_epoch,first_use_blocked,revoked_json FROM ONLY $row")
            .bind((
                "row",
                jobs::rid("configuration_reset_scope", &key(scope, "scope")),
            )),
    )
    .await?;
    jobs::take(&mut result, 0)
}
fn matches_request(
    row: &Row,
    authority: &ResetAuthority,
    request: &ResetRequest,
) -> Result<(), TxError> {
    if row.scope_id != authority.scope_id
        || row.reset_id != request.reset_id
        || row.plan_sha256 != request.plan_sha256
        || row.deployment_id != request.deployment_id
        || row.old_configuration_epoch != request.expected_configuration_epoch
        || row.old_access_epoch != request.expected_access_epoch
        || row.configuration_revision < 1
    {
        return Err(abort("conflict"));
    }
    Ok(())
}

async fn drained(tx: &Tx) -> Result<bool, TxError> {
    jobs::run(tx, tx.query("INFO FOR TABLE job; INFO FOR TABLE task; INFO FOR TABLE classifier_activation_operation;")).await?;
    let mut result=jobs::run(tx,tx.query("RETURN array::len((SELECT VALUE id FROM job WHERE kind!='service' AND state NOT IN ['completed','succeeded','failed','partial','cancelled','paused','superseded'] LIMIT 1))=0;
        RETURN array::len((SELECT VALUE id FROM task WHERE state NOT IN ['completed','failed','cancelled','superseded'] LIMIT 1))=0;
        RETURN array::len((SELECT VALUE id FROM classifier_activation_operation WHERE state IN ['running','effect_uncertain'] LIMIT 1))=0;")).await?;
    Ok(jobs::take::<Option<bool>>(&mut result, 0)? == Some(true)
        && jobs::take::<Option<bool>>(&mut result, 1)? == Some(true)
        && jobs::take::<Option<bool>>(&mut result, 2)? == Some(true))
}
async fn refresh_drain(tx: &Tx, mut row: Row) -> Result<Row, TxError> {
    if row.phase == "draining" && drained(tx).await? {
        jobs::run(
            tx,
            tx.query("UPDATE $row SET phase='prepared' RETURN NONE")
                .bind((
                    "row",
                    jobs::rid("configuration_reset", &key(&row.scope_id, &row.reset_id)),
                )),
        )
        .await?;
        row.phase = "prepared".into();
    }
    Ok(row)
}

/// Gateway hook for every Native/Web request, including global reads. A
/// relabeled epoch cannot resurrect a fingerprint revoked by a completed reset.
pub async fn authorize_user_access(
    repo: &Repository,
    scope: &str,
    epoch: &str,
    fingerprint: &CredentialFingerprint,
) -> Result<(), ResetError> {
    run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        check_user_epoch_in(&tx, scope, epoch, Some(fingerprint)).await
    })
    .await?;
    Ok(())
}

/// A previously revoked user token cannot be relabeled Operator or service to
/// regain authority. Independent admin/infrastructure fingerprints are retained.
pub async fn authorize_independent_access(
    repo: &Repository,
    fingerprint: &CredentialFingerprint,
) -> Result<(), ResetError> {
    run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        if fingerprint_revoked_in(&tx, fingerprint).await? {
            return Err(abort("forbidden"));
        }
        Ok(())
    })
    .await?;
    Ok(())
}

/// Recheck a gateway-derived epoch after a validated InternalService hop.
/// This does not replace public matched-token fingerprint authorization.
pub async fn authorize_delegated_access(
    repo: &Repository,
    scope: &str,
    epoch: &str,
) -> Result<(), ResetError> {
    run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        check_user_epoch_in(&tx, scope, epoch, None).await
    })
    .await?;
    Ok(())
}

/// A private owner may clear its real caches only while this exact reset owns
/// the durable global fence and every accepted credential consumer has drained.
pub async fn verify_owner_clear(
    repo: &Repository,
    scope: &str,
    configuration_epoch: &str,
) -> Result<(), ResetError> {
    run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        registry_guard(&tx).await?;
        let current = scope_in(&tx, scope)
            .await?
            .ok_or_else(|| abort("missing"))?;
        let deployment = connect_registry::deployment_state_in(&tx).await?;
        if !current.first_use_blocked
            || current.active_reset.is_empty()
            || deployment.admission_open
            || deployment.configuration_epoch != configuration_epoch
        {
            return Err(abort("forbidden"));
        }
        if !drained(&tx).await? {
            return Err(abort("draining"));
        }
        Ok(())
    })
    .await?;
    Ok(())
}

/// Actual registry/admission hook inside the caller's existing guard TX.
pub(crate) async fn check_user_epoch_in(
    tx: &Tx,
    scope: &str,
    epoch: &str,
    fingerprint: Option<&CredentialFingerprint>,
) -> Result<(), TxError> {
    jobs::run(tx, tx.query("INFO FOR TABLE configuration_reset_scope;")).await?;
    let mut fences = jobs::run(tx, tx.query("RETURN array::len((SELECT VALUE id FROM configuration_reset_scope WHERE first_use_blocked=true LIMIT 1))=0;")).await?;
    if jobs::take::<Option<bool>>(&mut fences, 0)? != Some(true) {
        return Err(abort("forbidden"));
    }
    if let Some(fingerprint) = fingerprint {
        if fingerprint_revoked_in(tx, fingerprint).await? {
            return Err(abort("forbidden"));
        }
    }
    if let Some(current) = scope_in(tx, scope).await? {
        let revoked: BTreeSet<String> =
            serde_json::from_str(&current.revoked_json).map_err(|_| abort("forbidden"))?;
        if current.first_use_blocked
            || current.access_epoch != epoch
            || fingerprint.is_some_and(|value| revoked.contains(&value.encoded()))
        {
            return Err(abort("forbidden"));
        }
    }
    Ok(())
}

async fn fingerprint_revoked_in(
    tx: &Tx,
    fingerprint: &CredentialFingerprint,
) -> Result<bool, TxError> {
    jobs::run(tx, tx.query("INFO FOR TABLE configuration_reset_revoked;")).await?;
    let mut result = jobs::run(
        tx,
        tx.query("SELECT VALUE fingerprint FROM ONLY $record")
            .bind((
                "record",
                jobs::rid("configuration_reset_revoked", &fingerprint.encoded()),
            )),
    )
    .await?;
    Ok(jobs::take::<Option<String>>(&mut result, 0)?.is_some())
}

/// Configuration writers must hold ConfigLock across this DB check and their
/// actual filesystem commit. Prepare acquires the same lock before fencing.
pub async fn configuration_write_allowed(
    repo: &Repository,
    _scope: &str,
    _lock: &ConfigLock,
) -> Result<(), ResetError> {
    let allowed = run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        jobs::run(&tx, tx.query("INFO FOR TABLE configuration_reset_scope;")).await?;
        // Configuration bytes are global. A writer authenticated under another
        // scope cannot race a reset of the same configured owner directory.
        let mut result = jobs::run(&tx, tx.query("RETURN array::len((SELECT VALUE id FROM configuration_reset_scope WHERE first_use_blocked=true LIMIT 1))=0;")).await?;
        Ok(jobs::take::<Option<bool>>(&mut result, 0)? == Some(true))
    })
    .await?
    .value;
    if !allowed {
        return Err(ResetError::Conflict);
    }
    Ok(())
}
