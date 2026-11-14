//! Durable, single-issuance native pairing and per-installation credential fences.
//!
//! Polling secrets and access tokens are never stored. The exact token is derived
//! with HMAC from the client-held polling secret and a random issuance nonce.
use std::collections::BTreeSet;

use fleetworks_contracts::context::Permission;
use fleetworks_contracts::pairing::PairingChallenge;
use fleetworks_contracts::pairing::PairingDecisionRequest;
use fleetworks_contracts::pairing::PairingPollRequest;
use fleetworks_contracts::pairing::PairingStartRequest;
use fleetworks_contracts::pairing::PairingStatus;
use hmac::Hmac;
use hmac::Mac;
use sha2::Digest;
use sha2::Sha256;
use surrealdb::types::SurrealValue;

use super::jobs;
use super::repo::Repository;
use super::tx::run_tx;
use super::tx::RetryPolicy;
use super::tx::Tx;
use super::tx::TxError;

pub const SCHEMA: &str = include_str!("schema/native_pairing.surql");
pub const CREDENTIAL_PREFIX: &str = "paired-";
const PENDING_MS: i64 = 600_000;
const POLL_AFTER_MS: u64 = 2000;

/// The validated current Operator policy; callers must never deserialize this.
#[derive(Clone)]
pub struct OperatorAuthority {
    pub credential_id: String,
    pub scope_id: String,
    pub access_epoch: String,
    pub permissions: BTreeSet<Permission>,
}

/// Non-secret persisted Native authority. There is no bearer token in this type.
#[derive(Clone)]
pub struct NativeAuthority {
    pub credential_id: String,
    pub scope_id: String,
    pub access_epoch: String,
    pub issuer_id: String,
    pub permissions: BTreeSet<Permission>,
}

#[derive(Clone, SurrealValue)]
struct PairingRow {
    pairing_id: String,
    installation_hash: String,
    proof_hash: String,
    approval_code: String,
    expires_at_ms: i64,
    state: String,
    credential_id: String,
    issuance_nonce: String,
    issuer_id: String,
    scope_id: String,
    access_epoch: String,
    permissions_json: String,
    token_hash: String,
}
#[derive(SurrealValue)]
struct RateRow {
    started_at_ms: i64,
    attempts: i64,
}

fn abort(reason: &'static str) -> TxError {
    TxError::Abort(format!("pairing:{reason}"))
}
fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
/// Match the existing length-delimited installation digest in the registry.
fn installation_hash(marker: &str) -> String {
    let mut hash = Sha256::new();
    hash.update((marker.len() as u64).to_be_bytes());
    hash.update(marker.as_bytes());
    format!("{:x}", hash.finalize())
}
fn digest(secret: &str) -> String {
    format!("{:x}", Sha256::digest(secret.as_bytes()))
}
fn derive_token(secret: &str, credential: &str, nonce: &str) -> Result<String, TxError> {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|_| abort("invalid"))?;
    mac.update(b"fleetworks/native-pairing/access/v1\0");
    for part in [credential, nonce] {
        mac.update(&(part.len() as u64).to_be_bytes());
        mac.update(part.as_bytes());
    }
    Ok(format!("{:x}", mac.finalize().into_bytes()))
}
async fn random_secret() -> Result<String, TxError> {
    tokio::task::spawn_blocking(super::home::random_secret)
        .await
        .map_err(|_| abort("unavailable"))?
        .map_err(|_| abort("unavailable"))
}
/// This is also the registry/admission/reset guard: revocation serializes with
/// accepted admission, rather than merely racing an earlier middleware lookup.
async fn guard(tx: &Tx) -> Result<(), TxError> {
    jobs::run(
        tx,
        tx.query(
            "UPSERT connect_registry_guard:singleton SET revision=(revision ?? 0)+1 RETURN NONE",
        ),
    )
    .await?;
    Ok(())
}
async fn read_in(tx: &Tx, id: &str) -> Result<Option<PairingRow>, TxError> {
    let mut response = jobs::run(
        tx,
        tx.query("SELECT * OMIT id FROM ONLY $row")
            .bind(("row", jobs::rid("native_pairing", id))),
    )
    .await?;
    jobs::take(&mut response, 0)
}
async fn save_in(tx: &Tx, row: &PairingRow) -> Result<(), TxError> {
    jobs::run(
        tx,
        tx.query("UPSERT $row CONTENT $data RETURN NONE")
            .bind(("row", jobs::rid("native_pairing", &row.pairing_id)))
            .bind(("data", row.clone())),
    )
    .await?;
    Ok(())
}
/// Fixed operation keys prevent attacker-controlled rate-row growth. Limits are
/// deployment-wide intentionally, including unknown IDs and invalid secrets.
async fn rate(repo: &Repository, operation: &'static str, limit: i64) -> Result<(), TxError> {
    let now = now_ms();
    let result = run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        guard(&tx).await?;
        let mut response = jobs::run(
            &tx,
            tx.query("SELECT * OMIT id FROM ONLY $row")
                .bind(("row", jobs::rid("native_pairing_rate", operation))),
        )
        .await?;
        let previous: Option<RateRow> = jobs::take(&mut response, 0)?;
        let live = previous
            .as_ref()
            .is_some_and(|r| r.started_at_ms > now - 60_000);
        if live && previous.as_ref().is_some_and(|r| r.attempts >= limit) {
            return Ok(false);
        }
        let attempts = previous
            .as_ref()
            .filter(|_| live)
            .map_or(1, |r| r.attempts + 1);
        let started = previous
            .as_ref()
            .filter(|_| live)
            .map_or(now, |r| r.started_at_ms);
        jobs::run(
            &tx,
            tx.query("UPSERT $row SET started_at_ms=$started,attempts=$attempts RETURN NONE")
                .bind(("row", jobs::rid("native_pairing_rate", operation)))
                .bind(("started", started))
                .bind(("attempts", attempts)),
        )
        .await?;
        Ok(true)
    })
    .await?;
    if result.value {
        Ok(())
    } else {
        Err(abort("rate"))
    }
}

/// Start/recover the exact client-persisted request. Reusing an ID with another
/// secret/installation is a conflict and cannot disclose the challenge.
pub async fn start(
    repo: &Repository,
    input: &PairingStartRequest,
) -> Result<PairingChallenge, TxError> {
    input.validate().map_err(|_| abort("invalid"))?;
    rate(repo, "start", 20).await?;
    let now = now_ms();
    let approval = random_secret().await?;
    let approval = approval[..32].to_owned();
    let result = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let approval = approval.clone();
        async move {
            guard(&tx).await?;
            let proof_hash = digest(&input.polling_secret);
            let installation_hash = installation_hash(&input.installation_marker);
            let row = if let Some(existing) = read_in(&tx, &input.request_id).await? {
                if existing.proof_hash != proof_hash || existing.installation_hash != installation_hash {
                    return Err(abort("conflict"));
                }
                existing
            } else {
                let query = "SELECT VALUE count() FROM native_pairing WHERE state IN ['pending','approved'] AND expires_at_ms>$now GROUP ALL";
                let mut count = jobs::run(&tx, tx.query(query).bind(("now", now))).await?;
                let count: Vec<i64> = jobs::take(&mut count, 0)?;
                if count.first().copied().unwrap_or(0) >= 100 {
                    return Err(abort("rate"));
                }
                let row = PairingRow {
                    pairing_id: input.request_id.clone(),
                    installation_hash,
                    proof_hash,
                    approval_code: approval,
                    expires_at_ms: now + PENDING_MS,
                    state: "pending".into(),
                    credential_id: String::new(),
                    issuance_nonce: String::new(),
                    issuer_id: String::new(),
                    scope_id: String::new(),
                    access_epoch: String::new(),
                    permissions_json: "[]".into(),
                    token_hash: String::new(),
                };
                save_in(&tx, &row).await?;
                row
            };
            Ok(PairingChallenge {
                pairing_id: row.pairing_id,
                approval_code: row.approval_code,
                expires_at_ms: row.expires_at_ms as u64,
                poll_after_ms: POLL_AFTER_MS,
            })
        }
    }).await?;
    Ok(result.value)
}

fn grants(authority: &OperatorAuthority) -> Result<BTreeSet<Permission>, TxError> {
    if !authority
        .permissions
        .contains(&Permission::ManageConfiguration)
        || !authority
            .permissions
            .contains(&Permission::RegisterWorkstation)
        || !authority.permissions.contains(&Permission::ReadBackend)
    {
        return Err(abort("forbidden"));
    }
    Ok(authority
        .permissions
        .iter()
        .copied()
        .filter(|p| *p != Permission::ResolveWebSession)
        .collect())
}
fn native(row: &PairingRow) -> Result<NativeAuthority, TxError> {
    Ok(NativeAuthority {
        credential_id: row.credential_id.clone(),
        scope_id: row.scope_id.clone(),
        access_epoch: row.access_epoch.clone(),
        issuer_id: row.issuer_id.clone(),
        permissions: serde_json::from_str(&row.permissions_json)
            .map_err(|_| abort("unavailable"))?,
    })
}
/// Operator approval captures authority from the authenticated policy only.
/// Exact repeated approval is allowed; terminal denial/expiry is never revived.
pub async fn decide(
    repo: &Repository,
    authority: &OperatorAuthority,
    input: &PairingDecisionRequest,
    approve: bool,
) -> Result<(), TxError> {
    input.validate().map_err(|_| abort("invalid"))?;
    let permissions = grants(authority)?;
    rate(repo, "decision", 100).await?;
    let nonce = random_secret().await?;
    let credential = format!("{CREDENTIAL_PREFIX}{}", random_secret().await?);
    let now = now_ms();
    run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (nonce, credential, permissions) =
            (nonce.clone(), credential.clone(), permissions.clone());
        async move {
            guard(&tx).await?;
            super::configuration_reset::check_user_epoch_in(
                &tx,
                &authority.scope_id,
                &authority.access_epoch,
                None,
            )
            .await?;
            let mut row = read_in(&tx, &input.pairing_id)
                .await?
                .ok_or_else(|| abort("missing"))?;
            if row.approval_code != input.approval_code {
                return Err(abort("forbidden"));
            }
            if row.expires_at_ms <= now {
                return Err(abort("expired"));
            }
            if row.state != "pending" {
                let matching = row.issuer_id == authority.credential_id
                    && row.scope_id == authority.scope_id
                    && row.access_epoch == authority.access_epoch
                    && row.permissions_json
                        == serde_json::to_string(&permissions).map_err(|_| abort("unavailable"))?;
                if matching
                    && ((approve && matches!(row.state.as_str(), "approved" | "issued"))
                        || (!approve && row.state == "denied"))
                {
                    return Ok(());
                }
                return Err(abort("conflict"));
            }
            row.state = if approve { "approved" } else { "denied" }.into();
            row.issuer_id = authority.credential_id.clone();
            row.scope_id = authority.scope_id.clone();
            row.access_epoch = authority.access_epoch.clone();
            row.permissions_json =
                serde_json::to_string(&permissions).map_err(|_| abort("unavailable"))?;
            if approve {
                row.credential_id = credential;
                row.issuance_nonce = nonce;
            }
            save_in(&tx, &row).await
        }
    })
    .await?;
    Ok(())
}

/// Deliver the same credential after an uncertain response/restart. Approval is
/// a single issuance, not a source of fresh tokens. Once issued, exact recovery
/// remains possible until revocation/reset or issuer-policy invalidation.
pub async fn poll(repo: &Repository, input: &PairingPollRequest) -> Result<PairingStatus, TxError> {
    input.validate().map_err(|_| abort("invalid"))?;
    rate(repo, "poll", 600).await?;
    // Per-pairing windows are unnecessary: a fixed global cap also bounds
    // unknown requests without materializing arbitrary attacker-owned rows.
    let now = now_ms();
    let result = run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        guard(&tx).await?;
        let mut row = read_in(&tx, &input.pairing_id)
            .await?
            .ok_or_else(|| abort("missing"))?;
        if row.proof_hash != digest(&input.polling_secret) {
            return Err(abort("forbidden"));
        }
        if row.state == "denied" || row.state == "revoked" {
            return Ok(PairingStatus::Denied);
        }
        if row.state != "issued" && row.expires_at_ms <= now {
            return Ok(PairingStatus::Expired);
        }
        if row.state == "pending" {
            return Ok(PairingStatus::Pending {
                poll_after_ms: POLL_AFTER_MS,
            });
        }
        if !matches!(row.state.as_str(), "approved" | "issued") {
            return Err(abort("forbidden"));
        }
        super::configuration_reset::check_user_epoch_in(
            &tx,
            &row.scope_id,
            &row.access_epoch,
            None,
        )
        .await?;
        let token = derive_token(
            &input.polling_secret,
            &row.credential_id,
            &row.issuance_nonce,
        )?;
        let token_hash = digest(&token);
        if row.state == "issued" && row.token_hash != token_hash {
            return Err(abort("forbidden"));
        }
        row.state = "issued".into();
        row.token_hash = token_hash;
        save_in(&tx, &row).await?;
        Ok(PairingStatus::Approved {
            credential_id: row.credential_id,
            access_token: token,
            scope_id: row.scope_id,
            access_epoch: row.access_epoch,
        })
    })
    .await?;
    Ok(result.value)
}

/// Lookup only a token digest. The gateway additionally checks current issuer
/// purpose/scope/epoch/grants from its loaded operator policy before accepting.
pub async fn authenticate(
    repo: &Repository,
    token: &str,
) -> Result<Option<NativeAuthority>, TxError> {
    if !fleetworks_contracts::pairing::valid_secret(token) {
        return Ok(None);
    }
    let token_hash = digest(token);
    let result = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let token_hash = token_hash.clone();
        async move {
            let query = "SELECT * OMIT id FROM native_pairing WHERE token_hash=$hash AND state='issued' LIMIT 2";
            let mut response = jobs::run(&tx, tx.query(query).bind(("hash", token_hash))).await?;
            let mut rows: Vec<PairingRow> = jobs::take(&mut response, 0)?;
            if rows.len() != 1 { return Ok(None); }
            let row = rows.pop().ok_or_else(|| abort("missing"))?;
            if let Err(error) = super::configuration_reset::check_user_epoch_in(&tx, &row.scope_id, &row.access_epoch, None).await {
                if matches!(&error, TxError::Abort(reason) if reason == "configuration_reset:forbidden") { return Ok(None); }
                return Err(error);
            }
            Ok(Some(native(&row)?))
        }
    }).await?;
    Ok(result.value)
}

/// Validate per-installation authority inside the caller's admission/read TX.
/// Reserved IDs fail closed even if a credential row was removed or revoked.
pub(crate) async fn check_installation_in(
    tx: &Tx,
    scope: &str,
    credential: &str,
    epoch: &str,
    installation: Option<&str>,
) -> Result<(), TxError> {
    if !credential.starts_with(CREDENTIAL_PREFIX) {
        return Ok(());
    }
    let mut response = jobs::run(
        tx,
        tx.query("SELECT * OMIT id FROM native_pairing WHERE credential_id=$credential LIMIT 2")
            .bind(("credential", credential.to_owned())),
    )
    .await?;
    let mut rows: Vec<PairingRow> = jobs::take(&mut response, 0)?;
    if rows.len() != 1 {
        return Err(TxError::Abort("connect:revoked".into()));
    }
    let row = rows
        .pop()
        .ok_or_else(|| TxError::Abort("connect:revoked".into()))?;
    if row.state != "issued"
        || row.scope_id != scope
        || row.access_epoch != epoch
        || installation.is_some_and(|i| i != row.installation_hash)
    {
        return Err(TxError::Abort("connect:revoked".into()));
    }
    Ok(())
}

/// Revoke only Native credentials in the authenticated Operator's current scope.
/// The caller holds the same configuration-owner lock through this transaction:
/// ConfigLock precedes the registry guard everywhere, including configuration
/// writers, accepted admission, reset and revocation. This fences file writes
/// that resolved a paired credential before revocation but are still waiting.
pub async fn revoke(
    repo: &Repository,
    authority: &OperatorAuthority,
    credential: &str,
    _configuration_lock: &crate::config_commit::ConfigLock,
) -> Result<(), TxError> {
    grants(authority)?;
    if !credential.starts_with(CREDENTIAL_PREFIX) {
        return Err(abort("invalid"));
    }
    run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        guard(&tx).await?;
        let mut response = jobs::run(
            &tx,
            tx.query(
                "SELECT * OMIT id FROM native_pairing WHERE credential_id=$credential LIMIT 2",
            )
            .bind(("credential", credential.to_owned())),
        )
        .await?;
        let mut rows: Vec<PairingRow> = jobs::take(&mut response, 0)?;
        if rows.len() != 1 {
            return Err(abort("missing"));
        }
        let mut row = rows.pop().ok_or_else(|| abort("missing"))?;
        if row.scope_id != authority.scope_id || row.access_epoch != authority.access_epoch {
            return Err(abort("forbidden"));
        }
        row.state = "revoked".into();
        save_in(&tx, &row).await
    })
    .await?;
    Ok(())
}

/// Revalidate dynamic global/delegated authority; static manifest IDs are unchanged.
pub async fn validate_authority(
    repo: &Repository,
    scope: &str,
    credential: &str,
    epoch: &str,
    permissions: &BTreeSet<Permission>,
) -> Result<(), TxError> {
    if !credential.starts_with(CREDENTIAL_PREFIX) {
        return Ok(());
    }
    run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        check_installation_in(&tx, scope, credential, epoch, None).await?;
        let mut response = jobs::run(
            &tx,
            tx.query(
                "SELECT * OMIT id FROM native_pairing WHERE credential_id=$credential LIMIT 1",
            )
            .bind(("credential", credential.to_owned())),
        )
        .await?;
        let rows: Vec<PairingRow> = jobs::take(&mut response, 0)?;
        let row = rows.first().ok_or_else(|| abort("forbidden"))?;
        if !permissions.is_subset(&native(row)?.permissions) {
            return Err(abort("forbidden"));
        }
        super::configuration_reset::check_user_epoch_in(&tx, scope, epoch, None).await
    })
    .await?;
    Ok(())
}

/// A previously issued token cannot be relabelled through a static manifest to
/// escape its installation, revoked state or original epoch. Keep its digest
/// even after revocation; unknown static/headless credentials remain supported.
pub async fn is_issued_token(repo: &Repository, token: &str) -> Result<bool, TxError> {
    if !fleetworks_contracts::pairing::valid_secret(token) {
        return Ok(false);
    }
    let hash = digest(token);
    let mut response = repo
        .db()
        .query("SELECT VALUE token_hash FROM native_pairing WHERE token_hash=$hash LIMIT 1")
        .bind(("hash", hash))
        .await?
        .check()?;
    let hashes: Vec<String> = response.take(0)?;
    Ok(!hashes.is_empty())
}
