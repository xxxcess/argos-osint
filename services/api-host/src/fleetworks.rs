use std::env;
use std::fs::File;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

use anyhow::{anyhow, ensure, Context, Result};
use argos_osint_api::{
    BrainClient, ControlClient, JobsClient, NetworkPolicy, PrivateServices, ToolsClient,
};
use argos_osint_core::storage::open::{NetworkAccess, Target};
use argos_osint_core::storage::repo::Repository;

use crate::Role;

pub(crate) struct NetworkStorage {
    pub workspace: PathBuf,
    pub target: Target,
    pub namespace: String,
    pub database: String,
    pub runtime_user: Option<RuntimeUser>,
}

pub(crate) struct RuntimeUser {
    name: String,
    password: String,
}

impl NetworkStorage {
    pub fn from_environment(role: Role) -> Result<Self> {
        ensure!(
            cfg!(feature = "server"),
            "Fleetworks roles require the server build feature"
        );
        let workspace = workspace_path()?;
        let endpoint =
            env::var("FLEETWORKS_STORAGE_URL").context("FLEETWORKS_STORAGE_URL is required")?;
        let namespace = identifier("FLEETWORKS_STORAGE_NAMESPACE", "fleetworks")?;
        let database = identifier("FLEETWORKS_STORAGE_DATABASE", "fleetworks")?;
        let name = identifier("FLEETWORKS_STORAGE_USERNAME", "fleetworks")?;
        let password = read_secret("FLEETWORKS_STORAGE_PASSWORD")?;
        let (target, runtime_user) = if role == Role::Bootstrap {
            let root = identifier("FLEETWORKS_STORAGE_ROOT_USERNAME", "root")?;
            let root_password = read_secret("FLEETWORKS_STORAGE_ROOT_PASSWORD")?;
            (
                Target::network(&endpoint, root, root_password, NetworkAccess::Bootstrap)?,
                Some(RuntimeUser { name, password }),
            )
        } else {
            (
                Target::network(&endpoint, name, password, NetworkAccess::Runtime)?,
                None,
            )
        };
        Ok(Self {
            workspace,
            target,
            namespace,
            database,
            runtime_user,
        })
    }
}

pub(crate) fn workspace_path() -> Result<PathBuf> {
    let workspace = env::var_os("FLEETWORKS_WORKSPACE")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .context("FLEETWORKS_WORKSPACE must be absolute")?;
    ensure!(
        env::var_os("ARGOS_HOME").as_deref() == Some(workspace.as_os_str()),
        "ARGOS_HOME must select the same Fleetworks workspace"
    );
    Ok(workspace)
}

pub(crate) struct ApiAccess {
    pub bind: IpAddr,
    pub origins: Vec<String>,
    pub policy: Option<NetworkPolicy>,
}

impl ApiAccess {
    pub fn from_environment(role: Role, legacy_origins: &[String]) -> Result<Self> {
        if role == Role::Combined {
            let origins = legacy_origins
                .iter()
                .map(|raw| {
                    argos_osint_api::loopback_url(raw).map(|url| url.origin().ascii_serialization())
                })
                .collect::<Result<Vec<_>>>()?;
            return Ok(Self {
                bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
                origins,
                policy: None,
            });
        }
        let private = match role {
            Role::Brain => Some(internal_access("BRAIN")?),
            Role::Tools => Some(internal_access_with_token(
                "TOOLS",
                "FLEETWORKS_TOOLS_TOKEN",
            )?),
            Role::Control => Some(internal_access_with_token(
                "CONTROL",
                "FLEETWORKS_CONTROL_TOKEN",
            )?),
            Role::Jobs => Some(internal_access_with_token("JOBS", "FLEETWORKS_JOBS_TOKEN")?),
            _ => None,
        };
        if let Some((bind, policy)) = private {
            return Ok(Self {
                bind,
                origins: Vec::new(),
                policy: Some(policy),
            });
        }
        let bind = value("FLEETWORKS_API_BIND", "127.0.0.1")?
            .parse()
            .context("invalid FLEETWORKS_API_BIND")?;
        let hosts = split_csv(
            &env::var("FLEETWORKS_API_ALLOWED_HOSTS")
                .context("FLEETWORKS_API_ALLOWED_HOSTS is required")?,
        );
        let origins = match env::var("FLEETWORKS_API_ALLOWED_ORIGINS") {
            Ok(raw) => split_csv(&raw),
            Err(env::VarError::NotPresent) => legacy_origins.to_vec(),
            Err(_) => return Err(anyhow!("invalid FLEETWORKS_API_ALLOWED_ORIGINS")),
        };
        let origins = origins
            .iter()
            .map(|raw| argos_osint_api::network_origin(raw))
            .collect::<Result<Vec<_>>>()?;
        let policy = match env::var_os("FLEETWORKS_API_CREDENTIALS_FILE") {
            Some(path) => {
                NetworkPolicy::from_credentials_file(&hosts, std::path::Path::new(&path))?
            }
            None => {
                let token = read_secret("FLEETWORKS_API_TOKEN")?;
                NetworkPolicy::new(&hosts, &token)?
            }
        };
        Ok(Self {
            bind,
            origins,
            policy: Some(policy),
        })
    }
}

pub(crate) fn private_services() -> Result<PrivateServices> {
    let services = PrivateServices::new(
        private_client(
            "FLEETWORKS_BRAIN_ORIGIN",
            "FLEETWORKS_INTERNAL_TOKEN",
            BrainClient::new,
        )?,
        private_client(
            "FLEETWORKS_TOOLS_ORIGIN",
            "FLEETWORKS_TOOLS_TOKEN",
            ToolsClient::new,
        )?,
        private_client(
            "FLEETWORKS_CONTROL_ORIGIN",
            "FLEETWORKS_CONTROL_TOKEN",
            ControlClient::new,
        )?,
        private_client(
            "FLEETWORKS_JOBS_ORIGIN",
            "FLEETWORKS_JOBS_TOKEN",
            JobsClient::new,
        )?,
    );
    match env::var("FLEETWORKS_SEARCH_ORIGIN") {
        Ok(origin) => Ok(services.with_search(argos_osint_api::SearchClient::new(
            &origin,
            &read_secret("FLEETWORKS_SEARCH_TOKEN")?,
        )?)),
        Err(env::VarError::NotPresent) => Ok(services),
        Err(_) => Err(anyhow!("invalid search origin encoding")),
    }
}

pub(crate) async fn workstation_services(
    repo: &Repository,
    services: PrivateServices,
) -> Result<PrivateServices> {
    let deployment = argos_osint_core::storage::connect_registry::deployment_state(repo).await?;
    tokio::task::spawn_blocking(move || {
        use argos_osint_api::workstation::ReadinessOwners;
        use argos_osint_api::workstation::ReadinessProbe;
        use fleetworks_contracts::readiness::ReadinessCheckId;
        let mut probes = Vec::new();
        for (id, origin_key, token_key) in [
            (
                ReadinessCheckId::Brain,
                "FLEETWORKS_BRAIN_ORIGIN",
                "FLEETWORKS_INTERNAL_TOKEN",
            ),
            (
                ReadinessCheckId::Control,
                "FLEETWORKS_CONTROL_ORIGIN",
                "FLEETWORKS_CONTROL_TOKEN",
            ),
            (
                ReadinessCheckId::Jobs,
                "FLEETWORKS_JOBS_ORIGIN",
                "FLEETWORKS_JOBS_TOKEN",
            ),
            (
                ReadinessCheckId::Nlp,
                "FLEETWORKS_NLP_ORIGIN",
                "FLEETWORKS_NLP_TOKEN",
            ),
            (
                ReadinessCheckId::Classifier,
                "FLEETWORKS_CLASSIFIER_ORIGIN",
                "FLEETWORKS_CLASSIFIER_TOKEN",
            ),
        ] {
            match env::var(origin_key) {
                Ok(origin) => {
                    probes.push(ReadinessProbe::new(id, &origin, &read_secret(token_key)?)?)
                }
                Err(env::VarError::NotPresent) => {}
                Err(_) => return Err(anyhow!("invalid readiness owner encoding")),
            }
        }
        let workspace = workspace_path()?;
        let artifacts =
            argos_osint_api::classifier_activation::ActivationArtifacts::new(workspace.clone())?;
        Ok(services
            .with_workstation(
                ReadinessOwners::new(deployment.deployment_id, probes)?
                    .with_configured_inputs_file(
                        env::var_os("FLEETWORKS_MAINTENANCE_INPUTS_FILE")
                            .as_deref()
                            .map(std::path::Path::new),
                    )?,
                argos_osint_api::datasets::DatasetStore::open(workspace.join("staged-datasets"))?,
            )
            .with_activation_artifacts(artifacts))
    })
    .await?
}

pub(crate) async fn export_deployment_identity(repo: &Repository) -> Result<()> {
    let Some(path) = env::var_os("FLEETWORKS_DEPLOYMENT_ID_FILE").map(PathBuf::from) else {
        return Ok(());
    };
    let id = argos_osint_core::storage::connect_registry::deployment_state(repo)
        .await?
        .deployment_id;
    tokio::task::spawn_blocking(move || {
        ensure!(
            path.is_absolute(),
            "deployment receipt must be an absolute path"
        );
        let parent = path
            .parent()
            .context("deployment receipt requires parent")?;
        ensure!(
            parent.canonicalize()? == parent,
            "deployment receipt parent must be provisioned and canonical"
        );
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => {
                ensure!(
                    metadata.is_file() && !metadata.file_type().is_symlink(),
                    "deployment receipt is not a regular file"
                );
                let mut existing = String::new();
                File::open(&path)?.take(513).read_to_string(&mut existing)?;
                ensure!(
                    existing.len() <= 512 && existing.trim() == id,
                    "deployment receipt belongs to a different database"
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                argos_osint_core::config_commit::write_secure(&path, &format!("{id}\n"))?
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
    })
    .await?
}

fn private_client<T>(
    origin_key: &str,
    token_key: &str,
    connect: impl FnOnce(&str, &str) -> Result<T>,
) -> Result<Option<T>> {
    match env::var(origin_key) {
        Ok(origin) => Ok(Some(connect(&origin, &read_secret(token_key)?)?)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(_) => Err(anyhow!("invalid encoding in {origin_key}")),
    }
}

pub(crate) fn internal_access(service: &str) -> Result<(IpAddr, NetworkPolicy)> {
    internal_access_with_token(service, "FLEETWORKS_INTERNAL_TOKEN")
}

pub(crate) fn internal_access_with_token(
    service: &str,
    token_key: &str,
) -> Result<(IpAddr, NetworkPolicy)> {
    let bind_key = format!("FLEETWORKS_{service}_BIND");
    let host_key = format!("FLEETWORKS_{service}_ALLOWED_HOSTS");
    let bind = value(&bind_key, "127.0.0.1")?
        .parse()
        .with_context(|| format!("invalid {bind_key}"))?;
    let hosts = split_csv(&env::var(&host_key).with_context(|| format!("{host_key} is required"))?);
    let token = read_secret(token_key)?;
    let scope = env::var("FLEETWORKS_ACCESS_SCOPE_ID")
        .context("FLEETWORKS_ACCESS_SCOPE_ID is required for private service authority")?;
    let epoch = env::var("FLEETWORKS_ACCESS_EPOCH")
        .context("FLEETWORKS_ACCESS_EPOCH is required for private service authority")?;
    use fleetworks_contracts::context::CredentialPurpose;
    use fleetworks_contracts::context::Permission;
    let credential = argos_osint_api::ScopedCredential::new(
        format!("internal-{}", service.to_ascii_lowercase()),
        scope,
        epoch,
        CredentialPurpose::InternalService,
        [
            Permission::ReadBackend,
            Permission::RegisterWorkstation,
            Permission::RebindWorkstation,
            Permission::ResolveWebSession,
            Permission::ManageConfiguration,
            Permission::MaintainBackend,
            Permission::SubmitInteractive,
            Permission::ReadWorkstationActivity,
            Permission::RetryJob,
            Permission::CancelJob,
        ]
        .into_iter()
        .collect(),
        &token,
    )?;
    Ok((
        bind,
        NetworkPolicy::with_credentials(&hosts, vec![credential])?,
    ))
}

fn split_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
        .collect()
}

fn value(key: &str, default: &str) -> Result<String> {
    match env::var(key) {
        Ok(value) => Ok(value),
        Err(env::VarError::NotPresent) => Ok(default.into()),
        Err(_) => Err(anyhow!("invalid encoding in {key}")),
    }
}

fn identifier(key: &str, default: &str) -> Result<String> {
    let name = value(key, default)?;
    ensure!(
        (1..=64).contains(&name.len())
            && name.as_bytes()[0].is_ascii_alphabetic()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
        "{key} must be an ASCII identifier"
    );
    Ok(name)
}

pub(crate) fn read_secret(key: &str) -> Result<String> {
    match env::var(key) {
        Ok(secret) => return Ok(secret),
        Err(env::VarError::NotPresent) => (),
        Err(_) => return Err(anyhow!("invalid encoding in {key}")),
    }
    let file_key = format!("{key}_FILE");
    let path =
        env::var_os(&file_key).with_context(|| format!("{key} or {file_key} is required"))?;
    let mut secret = String::new();
    File::open(path)
        .with_context(|| format!("open {file_key}"))?
        .take(4097)
        .read_to_string(&mut secret)
        .with_context(|| format!("read {file_key}"))?;
    ensure!(secret.len() <= 4096, "{file_key} is too large");
    Ok(secret.trim_end_matches(['\r', '\n']).to_owned())
}

pub(crate) async fn provision_user(repo: &Repository, user: RuntimeUser) -> Result<()> {
    ensure!(
        (32..=256).contains(&user.password.len())
            && user
                .password
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-._~".contains(&byte)),
        "invalid Fleetworks runtime password"
    );
    // SurrealQL requires a literal password for DEFINE USER. Restrict both
    // fields before interpolation and omit query diagnostics from failures.
    repo.db()
        .query(format!(
            "DEFINE USER IF NOT EXISTS {} ON DATABASE PASSWORD '{}' ROLES EDITOR",
            user.name, user.password
        ))
        .await
        .and_then(|response| response.check())
        .map_err(|_| anyhow!("could not provision the Fleetworks database user"))?;
    Ok(())
}
