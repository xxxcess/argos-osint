//! Real loaded policy and authenticated process-local cache acknowledgements.
//! No mounted file reread is represented as a loaded policy/epoch change.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use argos_osint_core::storage::configuration_reset;
use argos_osint_core::storage::configuration_reset::ClearCachesFuture;
use argos_osint_core::storage::configuration_reset::ObservationFuture;
use argos_osint_core::storage::configuration_reset::OwnerObservation;
use argos_osint_core::storage::configuration_reset::PolicyObserver;
use argos_osint_core::storage::configuration_reset::ResetAuthority;
use argos_osint_core::storage::configuration_reset::ResetError;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::tx;
use axum::extract::Extension;
use axum::http::header;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::Json;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::Permission;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::RwLock;

use super::Failure;
use crate::network;
use crate::AuthenticatedPrincipal;
use crate::NetworkPolicy;

#[cfg(all(test, feature = "embedded", unix))]
#[path = "owner_tests.rs"]
mod tests;

pub(super) const OBSERVE_PATH: &str = "/api/v1/internal/configuration-reset/observe";
pub(super) const CLEAR_PATH: &str = "/api/v1/internal/configuration-reset/clear";
const LIMIT: usize = 64 * 1024;

fn valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Report {
    version: u16,
    scope_id: String,
    deployment_id: String,
    service: String,
    instance_id: String,
    access_epoch: String,
    configuration_epoch: String,
    cache_epoch: String,
    reader_contract: u16,
}
impl Report {
    fn observation(self) -> OwnerObservation {
        OwnerObservation {
            scope_id: self.scope_id,
            deployment_id: self.deployment_id,
            service: self.service,
            instance_id: self.instance_id,
            access_epoch: self.access_epoch,
            configuration_epoch: self.configuration_epoch,
            cache_epoch: self.cache_epoch,
            reader_contract: self.reader_contract,
        }
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Clear {
    configuration_epoch: String,
}

/// Fixed server-to-server connection to one actual private configuration owner.
#[derive(Clone)]
pub struct ResetOwnerClient {
    origin: url::Url,
    authorization: HeaderValue,
    service: String,
    scope_id: String,
    http: reqwest::Client,
}
impl ResetOwnerClient {
    pub fn new(
        origin: &str,
        token: &str,
        service: &str,
        scope_id: &str,
    ) -> Result<Self, ResetError> {
        if !network::valid_token(token.as_bytes()) || !valid(service) || !valid(scope_id) {
            return Err(ResetError::Invalid);
        }
        let origin = network::validate_origin(origin).map_err(|_| ResetError::Invalid)?;
        let mut authorization =
            HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| ResetError::Invalid)?;
        authorization.set_sensitive(true);
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| ResetError::Unavailable)?;
        Ok(Self {
            origin,
            authorization,
            service: service.into(),
            scope_id: scope_id.into(),
            http,
        })
    }
    async fn request(&self, clear: Option<&str>) -> Result<OwnerObservation, ResetError> {
        let mut url = self.origin.clone();
        url.set_path(if clear.is_some() {
            CLEAR_PATH
        } else {
            OBSERVE_PATH
        });
        let request = if let Some(epoch) = clear {
            if !valid(epoch) {
                return Err(ResetError::Invalid);
            }
            self.http.post(url).json(&Clear {
                configuration_epoch: epoch.into(),
            })
        } else {
            self.http.get(url)
        };
        let mut reply = request
            .header(header::AUTHORIZATION, self.authorization.clone())
            .send()
            .await
            .map_err(|_| ResetError::Unavailable)?;
        if !reply.status().is_success()
            || reply
                .content_length()
                .is_some_and(|length| length > LIMIT as u64)
        {
            return Err(ResetError::Unavailable);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = reply.chunk().await.map_err(|_| ResetError::Unavailable)? {
            if chunk.len() > LIMIT - bytes.len() {
                return Err(ResetError::Unavailable);
            }
            bytes.extend_from_slice(&chunk);
        }
        let report: Report = serde_json::from_slice(&bytes).map_err(|_| ResetError::Unavailable)?;
        if report.version != 1
            || report.service != self.service
            || report.scope_id != self.scope_id
            || !valid(&report.instance_id)
            || !valid(&report.deployment_id)
        {
            return Err(ResetError::Unavailable);
        }
        if clear
            .is_some_and(|epoch| report.configuration_epoch != epoch || report.cache_epoch != epoch)
        {
            return Err(ResetError::ReprovisioningRequired);
        }
        Ok(report.observation())
    }
}

/// Observes the exact immutable policy used by ingress and actual private RPCs.
/// Restart/reprovision installs a new observer and policy together.
pub struct ResetPolicyObserver {
    policy: NetworkPolicy,
    deployment_id: String,
    owners: Vec<ResetOwnerClient>,
}
impl ResetPolicyObserver {
    pub fn new(
        policy: NetworkPolicy,
        deployment_id: String,
        owners: Vec<ResetOwnerClient>,
    ) -> Result<Self, ResetError> {
        let services: std::collections::BTreeSet<_> =
            owners.iter().map(|owner| owner.service.as_str()).collect();
        if !valid(&deployment_id) || owners.len() > 32 || services.len() != owners.len() {
            return Err(ResetError::Invalid);
        }
        Ok(Self {
            policy,
            deployment_id,
            owners,
        })
    }
}
impl PolicyObserver for ResetPolicyObserver {
    fn observe<'a>(&'a self, authority: &'a ResetAuthority) -> ObservationFuture<'a> {
        Box::pin(async move {
            // Check independent loaded authority before issuing any private RPC.
            self.policy
                .reset_observation(authority, &self.deployment_id, Vec::new())?;
            let mut owners = Vec::new();
            for owner in &self.owners {
                if owner.scope_id != authority.scope_id() {
                    return Err(ResetError::Forbidden);
                }
                owners.push(owner.request(None).await?);
            }
            self.policy
                .reset_observation(authority, &self.deployment_id, owners)
        })
    }
    fn clear_owned_caches<'a>(
        &'a self,
        authority: &'a ResetAuthority,
        epoch: &'a str,
    ) -> ClearCachesFuture<'a> {
        Box::pin(async move {
            self.policy
                .reset_observation(authority, &self.deployment_id, Vec::new())?;
            for owner in &self.owners {
                if owner.scope_id != authority.scope_id() {
                    return Err(ResetError::Forbidden);
                }
                owner.request(Some(epoch)).await?;
            }
            Ok(())
        })
    }
}

/// Provision before publishing the private router/starting credential consumers.
/// The only process-global secret cache currently cleared here is AuthFile's
/// parsed cache. The compiled reader contract stays 0 while untracked legacy
/// consumers/caches remain, so this cannot attest broader retirement.
pub struct ResetOwner {
    repo: Repository,
    policy: NetworkPolicy,
    root: PathBuf,
    scope_id: String,
    deployment_id: String,
    service: String,
    instance_id: String,
    cache_epoch: RwLock<Option<String>>,
}
impl ResetOwner {
    pub async fn new(
        repo: Repository,
        policy: NetworkPolicy,
        service: String,
        instance_id: String,
        root: PathBuf,
    ) -> Result<Self, ResetError> {
        if !valid(&service)
            || !valid(&instance_id)
            || !root.is_absolute()
            || root.canonicalize().map_err(|_| ResetError::UnsafeFiles)? != root
        {
            return Err(ResetError::Invalid);
        }
        let scope_id = policy.owner_scope()?;
        let deployment = tx::run_tx(
            repo.db(),
            tx::RetryPolicy::default(),
            |transaction| async move {
                argos_osint_core::storage::connect_registry::deployment_state_in(&transaction).await
            },
        )
        .await?
        .value;
        let owner = Self {
            repo,
            policy,
            root,
            scope_id,
            deployment_id: deployment.deployment_id,
            service,
            instance_id,
            cache_epoch: RwLock::new(None),
        };
        // A restarted reprovisioned process has no acknowledged epoch until it
        // verifies actual empty files and clears its real cache under the fence.
        // Interrupted file prefixes/unfinished consumers leave it unacknowledged
        // but observable, so the coordinator can resume completion before clear.
        if !deployment.admission_open {
            match owner.clear_cache(&deployment.configuration_epoch).await {
                Ok(()) | Err(ResetError::UnreviewedSources | ResetError::Draining) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(owner)
    }
    fn authorize(
        &self,
        principal: Option<Extension<AuthenticatedPrincipal>>,
    ) -> Result<(), Failure> {
        let Some(Extension(principal)) = principal else {
            return Err(Failure(
                StatusCode::UNAUTHORIZED,
                "Internal owner authority required",
            ));
        };
        if principal.purpose() != CredentialPurpose::InternalService
            || principal.scope_id() != self.scope_id
            || principal.access_epoch()
                != self
                    .policy
                    .owner_epoch(&self.scope_id)
                    .map_err(Failure::from)?
            || !principal.permits(Permission::MaintainBackend)
            || !principal.permits(Permission::ManageConfiguration)
        {
            return Err(Failure(
                StatusCode::FORBIDDEN,
                "Internal owner authority required",
            ));
        }
        Ok(())
    }
    async fn report(&self) -> Result<Report, ResetError> {
        let epoch = self
            .cache_epoch
            .read()
            .await
            .clone()
            .unwrap_or_else(|| "unacknowledged".into());
        Ok(Report {
            version: 1,
            scope_id: self.scope_id.clone(),
            deployment_id: self.deployment_id.clone(),
            service: self.service.clone(),
            instance_id: self.instance_id.clone(),
            access_epoch: self.policy.owner_epoch(&self.scope_id)?,
            configuration_epoch: epoch.clone(),
            cache_epoch: epoch,
            reader_contract: configuration_reset::owner_reader_contract(),
        })
    }
    async fn clear_cache(&self, epoch: &str) -> Result<(), ResetError> {
        if !valid(epoch) {
            return Err(ResetError::Invalid);
        }
        configuration_reset::clear_process_cache(
            &self.repo,
            self.root.clone(),
            &self.scope_id,
            epoch,
        )
        .await?;
        // The ACK is process-local and follows the actual checked clear. A new
        // process cannot inherit it merely by reading an epoch from the DB.
        *self.cache_epoch.write().await = Some(epoch.into());
        Ok(())
    }
}

pub(super) async fn observe(
    Extension(owner): Extension<Arc<ResetOwner>>,
    principal: Option<Extension<AuthenticatedPrincipal>>,
) -> Result<Json<Report>, Failure> {
    owner.authorize(principal)?;
    Ok(Json(owner.report().await?))
}
pub(super) async fn clear(
    Extension(owner): Extension<Arc<ResetOwner>>,
    principal: Option<Extension<AuthenticatedPrincipal>>,
    Json(request): Json<Clear>,
) -> Result<Json<Report>, Failure> {
    owner.authorize(principal)?;
    owner.clear_cache(&request.configuration_epoch).await?;
    Ok(Json(owner.report().await?))
}
