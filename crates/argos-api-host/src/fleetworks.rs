use std::env;
use std::fs::File;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

use anyhow::{anyhow, ensure, Context, Result};
use argos_osint_api::NetworkPolicy;
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
        let token = read_secret("FLEETWORKS_API_TOKEN")?;
        let policy = NetworkPolicy::new(&hosts, &token)?;
        Ok(Self {
            bind,
            origins,
            policy: Some(policy),
        })
    }
}

pub(crate) fn internal_access(service: &str) -> Result<(IpAddr, NetworkPolicy)> {
    let bind_key = format!("FLEETWORKS_{service}_BIND");
    let host_key = format!("FLEETWORKS_{service}_ALLOWED_HOSTS");
    let bind = value(&bind_key, "127.0.0.1")?
        .parse()
        .with_context(|| format!("invalid {bind_key}"))?;
    let hosts = split_csv(&env::var(&host_key).with_context(|| format!("{host_key} is required"))?);
    let token = read_secret("FLEETWORKS_INTERNAL_TOKEN")?;
    Ok((bind, NetworkPolicy::new(&hosts, &token)?))
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
