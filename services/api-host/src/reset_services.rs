//! Explicit configuration-reset startup owners and a headless listener lifetime.
//! No reset runs at startup. The coordinator's compiled prepare gate stays shut.
//! Each owner observes the actual loaded policy and process cache state.

use std::collections::BTreeSet;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::ensure;
use anyhow::Context;
use anyhow::Result;
use argos_osint_api::configuration_reset::ResetOwner;
use argos_osint_api::configuration_reset::ResetOwnerClient;
use argos_osint_api::configuration_reset::ResetPolicyObserver;
use argos_osint_api::ApiState;
use argos_osint_api::NetworkPolicy;
use argos_osint_core::storage::configuration_reset::BackendFiles;
use argos_osint_core::storage::configuration_reset::ClearCachesFuture;
use argos_osint_core::storage::configuration_reset::Coordinator;
use argos_osint_core::storage::configuration_reset::CredentialFingerprint;
use argos_osint_core::storage::configuration_reset::ObservationFuture;
use argos_osint_core::storage::configuration_reset::OwnerObservation;
use argos_osint_core::storage::configuration_reset::PolicyObserver;
use argos_osint_core::storage::configuration_reset::ResetAuthority;
use argos_osint_core::storage::configuration_reset::ResetError;
use argos_osint_core::storage::configuration_reset::RuntimeObservation;
use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::repo::Repository;
use tokio::sync::watch;
use tokio::sync::RwLock;

use crate::fleetworks;
use crate::Role;

fn enabled(key: &str) -> Result<bool> {
    match std::env::var(key).as_deref() {
        Ok("1") => Ok(true),
        Ok("0") | Err(std::env::VarError::NotPresent) => Ok(false),
        _ => anyhow::bail!("{key} must be 0 or 1"),
    }
}

pub(crate) fn service(role: Role) -> Option<&'static str> {
    match role {
        Role::Brain => Some("fleetworks-brain"),
        Role::Tools => Some("fleetworks-tools"),
        Role::Control => Some("fleetworks-control"),
        Role::Jobs => Some("fleetworks-jobs"),
        Role::Execution => Some("fleetworks-execution"),
        Role::Worker => Some("fleetworks-worker"),
        Role::BriefWorker => Some("fleetworks-brief-worker"),
        Role::AtlasWorker => Some("fleetworks-atlas-worker"),
        Role::NativeWorker => Some("fleetworks-native-worker"),
        _ => None,
    }
}

/// Build before serving the existing private HTTP router. Pass the same policy
/// clone used by its actual ingress and the same real process instance ID.
pub(crate) async fn http_owner(
    state: &ApiState,
    policy: NetworkPolicy,
    role: Role,
) -> Result<Option<Arc<ResetOwner>>> {
    if !enabled("FLEETWORKS_CONFIGURATION_RESET_ENABLED")? {
        return Ok(None);
    }
    let service = service(role).context("role is not a user-configuration owner")?;
    let owner = ResetOwner::new(
        state.repo.clone(),
        policy,
        service.into(),
        state.instance_id.clone(),
        fleetworks::workspace_path()?,
    )
    .await?;
    Ok(Some(Arc::new(owner)))
}

/// Merge an independently guarded private owner endpoint after the ordinary
/// data router. Its InternalService authority never needs a user registration.
pub(crate) async fn attach_http_owner(
    router: axum::Router,
    state: &ApiState,
    policy: NetworkPolicy,
    role: Role,
) -> Result<axum::Router> {
    let Some(owner) = http_owner(state, policy.clone(), role).await? else {
        return Ok(router);
    };
    let owner_routes = argos_osint_api::configuration_reset::owner_routes()
        .layer(axum::Extension(owner))
        .layer(axum::middleware::from_fn_with_state(
            state.repo.clone(),
            owner_fingerprint,
        ))
        .layer(axum::extract::DefaultBodyLimit::max(1024))
        .with_state(state.clone());
    Ok(router.merge(argos_osint_api::guard_network(
        owner_routes,
        policy,
        Arc::new(Vec::new()),
        state.port,
    )))
}

const OWNERS: &[(&str, &str, &str)] = &[
    (
        "fleetworks-brain",
        "FLEETWORKS_BRAIN_ORIGIN",
        "FLEETWORKS_INTERNAL_TOKEN",
    ),
    (
        "fleetworks-tools",
        "FLEETWORKS_TOOLS_ORIGIN",
        "FLEETWORKS_TOOLS_TOKEN",
    ),
    (
        "fleetworks-control",
        "FLEETWORKS_CONTROL_ORIGIN",
        "FLEETWORKS_CONTROL_TOKEN",
    ),
    (
        "fleetworks-jobs",
        "FLEETWORKS_JOBS_ORIGIN",
        "FLEETWORKS_JOBS_TOKEN",
    ),
    // Execution's retained attempts read account material, unlike NLP,
    // classifier and search engines, which are deliberately absent here.
    (
        "fleetworks-execution",
        "FLEETWORKS_EXECUTION_ORIGIN",
        "FLEETWORKS_EXECUTION_TOKEN",
    ),
    (
        "fleetworks-worker",
        "FLEETWORKS_RESET_WORKER_ORIGIN",
        "FLEETWORKS_RESET_WORKER_TOKEN",
    ),
    (
        "fleetworks-brief-worker",
        "FLEETWORKS_RESET_BRIEF_WORKER_ORIGIN",
        "FLEETWORKS_RESET_BRIEF_WORKER_TOKEN",
    ),
    (
        "fleetworks-atlas-worker",
        "FLEETWORKS_RESET_ATLAS_WORKER_ORIGIN",
        "FLEETWORKS_RESET_ATLAS_WORKER_TOKEN",
    ),
    (
        "fleetworks-native-worker",
        "FLEETWORKS_RESET_NATIVE_WORKER_ORIGIN",
        "FLEETWORKS_RESET_NATIVE_WORKER_TOKEN",
    ),
];

async fn enabled_consumers(repo: &Repository) -> Result<BTreeSet<String>> {
    let mut required = BTreeSet::new();
    let mut enabled_pools = repo
        .db()
        .query("SELECT VALUE record::id(id) FROM job_dispatch_pool WHERE enabled=true LIMIT 65")
        .await?
        .check()?;
    let enabled_pools: Vec<String> = enabled_pools.take(0)?;
    ensure!(
        enabled_pools.len() <= 64,
        "reset consumer pool inventory exceeds its bound"
    );
    for pool in enabled_pools {
        let name = match pool.as_str() {
            argos_osint_core::storage::job_dispatch::ATLAS_POOL => "fleetworks-atlas-worker",
            argos_osint_core::osint::manual::TOOLS_POOL
            | argos_osint_core::storage::setup_jobs::SEED_POOL
            | argos_osint_core::storage::setup_jobs::TRAIN_POOL
            | argos_osint_core::storage::setup_jobs::REINDEX_POOL
            | argos_osint_core::storage::classifier_activation::POOL => "fleetworks-native-worker",
            _ => anyhow::bail!("enabled dispatch consumer has no reviewed reset owner"),
        };
        required.insert(name.to_owned());
    }
    Ok(required)
}

/// Build only in the API gateway using its exact loaded policy. Missing owner
/// provisioning fails startup when explicitly enabled, never invents an ACK.
pub(crate) async fn gateway(
    state: &ApiState,
    policy: NetworkPolicy,
) -> Result<Option<Arc<Coordinator>>> {
    if !enabled("FLEETWORKS_CONFIGURATION_RESET_ENABLED")? {
        return Ok(None);
    }
    let scope = std::env::var("FLEETWORKS_ACCESS_SCOPE_ID")
        .context("FLEETWORKS_ACCESS_SCOPE_ID required for reset owners")?;
    // An explicit deployment topology describes the headless consumers. Pool
    // state independently adds any enabled consumer, so it cannot be omitted.
    let topology = std::env::var("FLEETWORKS_RESET_CONSUMERS")
        .context("FLEETWORKS_RESET_CONSUMERS must explicitly name deployed headless consumers")?;
    let mut required = BTreeSet::from(["fleetworks-api".to_owned()]);
    for role in topology
        .split(',')
        .map(str::trim)
        .filter(|role| !role.is_empty())
    {
        let name = match role {
            "worker" => "fleetworks-worker",
            "brief-worker" => "fleetworks-brief-worker",
            "atlas-worker" => "fleetworks-atlas-worker",
            "native-worker" => "fleetworks-native-worker",
            _ => anyhow::bail!("unknown explicit reset consumer"),
        };
        required.insert(name.to_owned());
    }
    required.extend(enabled_consumers(&state.repo).await?);
    let mut clients = Vec::new();
    for (service, origin_key, token_key) in OWNERS {
        let configured = std::env::var(origin_key).ok();
        let headless = service.starts_with("fleetworks-worker")
            || matches!(
                *service,
                "fleetworks-brief-worker" | "fleetworks-atlas-worker" | "fleetworks-native-worker"
            );
        if !headless && configured.is_some() {
            required.insert((*service).to_owned());
        }
        if !required.contains(*service) {
            continue;
        }
        let origin =
            configured.with_context(|| format!("{origin_key} required for enabled reset owner"))?;
        ensure!(
            std::env::var_os(format!("{token_key}_FILE")).is_some(),
            "{token_key}_FILE required for reset owner"
        );
        let token = fleetworks::read_secret(token_key)?;
        clients.push(ResetOwnerClient::new(&origin, &token, service, &scope)?);
    }
    let root = fleetworks::workspace_path()?;
    let files = BackendFiles::new(root.clone(), effective_uid()?)?;
    let deployment = connect_registry::deployment_state(&state.repo).await?;
    let remote = ResetPolicyObserver::new(policy, deployment.deployment_id, clients)?;
    let observer = GatewayObserver {
        remote,
        repo: state.repo.clone(),
        root,
        instance_id: state.instance_id.clone(),
        cache_epoch: RwLock::new(None),
        provisioned_consumers: required.clone(),
    };
    if !deployment.admission_open {
        match argos_osint_core::storage::configuration_reset::clear_process_cache(
            &observer.repo,
            observer.root.clone(),
            &scope,
            &deployment.configuration_epoch,
        )
        .await
        {
            Ok(()) => *observer.cache_epoch.write().await = Some(deployment.configuration_epoch),
            Err(ResetError::UnreviewedSources | ResetError::Draining) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(Some(Arc::new(Coordinator::new(
        files,
        Arc::new(observer),
        required,
    )?)))
}

struct GatewayObserver {
    remote: ResetPolicyObserver,
    repo: Repository,
    root: std::path::PathBuf,
    instance_id: String,
    cache_epoch: RwLock<Option<String>>,
    provisioned_consumers: BTreeSet<String>,
}
impl GatewayObserver {
    async fn verify_consumers(&self) -> Result<(), ResetError> {
        let enabled = enabled_consumers(&self.repo)
            .await
            .map_err(|_| ResetError::Unavailable)?;
        if !enabled.is_subset(&self.provisioned_consumers) {
            return Err(ResetError::UnsupportedBuild);
        }
        Ok(())
    }
}
impl PolicyObserver for GatewayObserver {
    fn observe<'a>(&'a self, authority: &'a ResetAuthority) -> ObservationFuture<'a> {
        Box::pin(async move {
            self.verify_consumers().await?;
            let mut observed: RuntimeObservation = self.remote.observe(authority).await?;
            let epochs: BTreeSet<_> = observed
                .users
                .iter()
                .map(|user| user.access_epoch.clone())
                .collect();
            if epochs.len() != 1 {
                return Err(ResetError::Conflict);
            }
            let access_epoch = epochs.into_iter().next().ok_or(ResetError::Unavailable)?;
            let epoch = self
                .cache_epoch
                .read()
                .await
                .clone()
                .unwrap_or_else(|| "unacknowledged".into());
            observed.owners.push(OwnerObservation {
                scope_id: authority.scope_id().to_owned(),
                deployment_id: observed.deployment_id.clone(),
                service: "fleetworks-api".into(),
                instance_id: self.instance_id.clone(),
                access_epoch,
                configuration_epoch: epoch.clone(),
                cache_epoch: epoch,
                reader_contract:
                    argos_osint_core::storage::configuration_reset::owner_reader_contract(),
            });
            Ok(observed)
        })
    }
    fn clear_owned_caches<'a>(
        &'a self,
        authority: &'a ResetAuthority,
        epoch: &'a str,
    ) -> ClearCachesFuture<'a> {
        Box::pin(async move {
            self.verify_consumers().await?;
            self.remote.clear_owned_caches(authority, epoch).await?;
            argos_osint_core::storage::configuration_reset::clear_process_cache(
                &self.repo,
                self.root.clone(),
                authority.scope_id(),
                epoch,
            )
            .await?;
            *self.cache_epoch.write().await = Some(epoch.into());
            Ok(())
        })
    }
}

#[cfg(unix)]
fn effective_uid() -> Result<u32> {
    // std exposes no process UID API. geteuid has no arguments or pointers and
    // only observes the kernel's effective identity; it cannot mutate state.
    Ok(unsafe { libc::geteuid() })
}
#[cfg(not(unix))]
fn effective_uid() -> Result<u32> {
    anyhow::bail!("protected reset file ownership requires Unix")
}

/// Existing worker runners accept this future as their actual stop signal.
/// A closed sender also stops the process; no detached listener outlives work.
pub(crate) async fn stopped(mut receive: watch::Receiver<bool>) {
    loop {
        if *receive.borrow() {
            return;
        }
        if receive.changed().await.is_err() {
            return;
        }
    }
}

async fn headless_listener(
    repo: &Repository,
    role: Role,
) -> Result<Option<(tokio::net::TcpListener, axum::Router)>> {
    if !enabled("FLEETWORKS_RESET_OWNER_ENABLED")? {
        return Ok(None);
    }
    ensure!(
        matches!(
            role,
            Role::Worker | Role::BriefWorker | Role::AtlasWorker | Role::NativeWorker
        ),
        "headless reset owner requires a credential-consuming worker role"
    );
    for key in [
        "FLEETWORKS_RESET_OWNER_BIND",
        "FLEETWORKS_RESET_OWNER_ALLOWED_HOSTS",
        "FLEETWORKS_RESET_OWNER_TOKEN_FILE",
    ] {
        ensure!(
            std::env::var_os(key).is_some(),
            "{key} required for private reset owner"
        );
    }
    let port: u16 = std::env::var("FLEETWORKS_RESET_OWNER_PORT")
        .context("FLEETWORKS_RESET_OWNER_PORT required")?
        .parse()
        .context("invalid private reset owner port")?;
    ensure!(
        port != 0,
        "private reset owner port must be fixed and nonzero"
    );
    let (bind, policy) =
        fleetworks::internal_access_with_token("RESET_OWNER", "FLEETWORKS_RESET_OWNER_TOKEN")?;
    let listener = tokio::net::TcpListener::bind(SocketAddr::new(bind, port)).await?;
    let state = ApiState {
        repo: repo.clone(),
        instance_id: format!(
            "reset-owner-{}-{}",
            std::process::id(),
            argos_osint_core::store::new_id()
        ),
        port,
        origins: Arc::new(Vec::new()),
    };
    let owner = Arc::new(
        ResetOwner::new(
            repo.clone(),
            policy.clone(),
            service(role).context("owner service")?.into(),
            state.instance_id.clone(),
            fleetworks::workspace_path()?,
        )
        .await?,
    );
    let router = argos_osint_api::configuration_reset::owner_routes()
        .layer(axum::Extension(owner))
        .layer(axum::middleware::from_fn_with_state(
            repo.clone(),
            owner_fingerprint,
        ))
        .layer(axum::extract::DefaultBodyLimit::max(1024))
        .with_state(state);
    let router = argos_osint_api::guard_network(router, policy, Arc::new(Vec::new()), port);
    Ok(Some((listener, router)))
}

async fn owner_fingerprint(
    axum::extract::State(repo): axum::extract::State<Repository>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(fingerprint) = request.extensions().get::<CredentialFingerprint>() else {
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    };
    match argos_osint_core::storage::configuration_reset::authorize_independent_access(
        &repo,
        fingerprint,
    )
    .await
    {
        Ok(()) => next.run(request).await,
        Err(argos_osint_core::storage::configuration_reset::ResetError::Forbidden) => {
            axum::http::StatusCode::UNAUTHORIZED.into_response()
        }
        Err(_) => axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

/// Own a real worker and optional private listener with one shutdown signal.
/// On shutdown, signal the actual consumer and wait for its real completion
/// before dropping the listener; accepted work keeps its durable worker rules.
pub(crate) async fn run_headless<F, W, S>(
    repo: Repository,
    role: Role,
    worker: F,
    signal: S,
) -> Result<()>
where
    F: FnOnce(watch::Receiver<bool>) -> W,
    W: Future<Output = Result<()>>,
    S: Future<Output = ()>,
{
    let listener = headless_listener(&repo, role).await?;
    let serving = listener.is_some();
    let (stop, receive) = watch::channel(false);
    let mut worker = Box::pin(worker(receive.clone()));
    let mut server: Pin<Box<dyn Future<Output = Result<()>> + Send>> =
        if let Some((listener, router)) = listener {
            Box::pin(async move {
                axum::serve(listener, router)
                    .with_graceful_shutdown(stopped(receive))
                    .await
                    .map_err(anyhow::Error::from)
            })
        } else {
            Box::pin(std::future::pending())
        };
    tokio::pin!(signal);
    enum End {
        Worker(Result<()>),
        Server(Result<()>),
        Signal,
    }
    let ended = tokio::select! {
        result = &mut worker => End::Worker(result),
        result = &mut server => End::Server(result),
        _ = &mut signal => End::Signal,
    };
    let _ = stop.send(true);
    let (result, server_done) = match ended {
        End::Worker(result) => (result, false),
        End::Server(result) => {
            let consumer = worker.await;
            (
                result.and(consumer).and_then(|_| {
                    Err(anyhow::anyhow!(
                        "private reset owner ended before consumer shutdown"
                    ))
                }),
                true,
            )
        }
        End::Signal => (worker.await, false),
    };
    if serving && !server_done {
        tokio::time::timeout(Duration::from_secs(15), &mut server)
            .await
            .context("private reset owner shutdown deadline")??;
    }
    result
}
