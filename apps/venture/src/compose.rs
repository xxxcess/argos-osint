//! Own one local Docker Compose project per canonical workspace.

use std::collections::BTreeMap;
use std::env;
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, ensure, Context, Result};
use clap::ValueEnum;
use fleetworks_workspace::{Environment, Workspace};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::process::Command;
use url::Url;

use crate::host_storage::{self, Config as HostStorage, Mode as HostMode};

const DISK_URL: &str = "ws://surrealdb:8000";
const OWNER_VERSION: u8 = 1;
const OWNER_LIMIT: u64 = 64 * 1024;
const STORAGE_URL: &str = "FLEETWORKS_STORAGE_URL";
const API_PORT: &str = "FLEETWORKS_API_PORT";
const WEB_PORT: &str = "FLEETWORKS_WEB_PORT";
const API_TOKEN_FILE: &str = "FLEETWORKS_API_TOKEN_FILE";
const ROOT_PASSWORD_FILE: &str = "FLEETWORKS_STORAGE_ROOT_PASSWORD_FILE";
const RUNTIME_PASSWORD_FILE: &str = "FLEETWORKS_STORAGE_PASSWORD_FILE";
const SETTINGS: &[(&str, Option<&str>)] = &[
    ("FLEETWORKS_STORAGE_NAMESPACE", Some("fleetworks")),
    ("FLEETWORKS_STORAGE_DATABASE", Some("fleetworks")),
    ("FLEETWORKS_STORAGE_USERNAME", Some("fleetworks")),
    ("FLEETWORKS_STORAGE_ROOT_USERNAME", Some("root")),
    ("FLEETWORKS_API_ALLOWED_HOSTS", None),
    ("FLEETWORKS_API_ALLOWED_ORIGINS", None),
    ("FLEETWORKS_WEB_ORIGIN", None),
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StorageMode {
    ComposeDisk,
    External,
    HostMemory,
    HostDisk,
}

impl StorageMode {
    fn host_mode(self) -> Option<HostMode> {
        match self {
            Self::HostMemory => Some(HostMode::Memory),
            Self::HostDisk => Some(HostMode::Disk),
            Self::ComposeDisk | Self::External => None,
        }
    }
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Owner {
    version: u8,
    project: String,
    workspace: PathBuf,
    file: PathBuf,
    docker_host: String,
    storage: StorageMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    host_storage: Option<HostStorage>,
    endpoint: String,
    api_port: u16,
    web_port: u16,
    uid: u32,
    gid: u32,
    token: PathBuf,
    root_password: PathBuf,
    runtime_password: PathBuf,
    settings: BTreeMap<String, String>,
}

impl Owner {
    pub(crate) async fn load(workspace: &Workspace) -> Result<Option<Self>> {
        let workspace = workspace.clone();
        tokio::task::spawn_blocking(move || Self::read(&workspace)).await?
    }

    fn read(workspace: &Workspace) -> Result<Option<Self>> {
        let path = owner_path(workspace);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => ensure!(metadata.is_file(), "invalid Venture ownership file"),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("inspect Venture ownership"),
        }
        let mut bytes = Vec::new();
        File::open(&path)?
            .take(OWNER_LIMIT + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= OWNER_LIMIT,
            "Venture ownership file is too large"
        );
        let owner: Self = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow!("invalid Venture ownership file"))?;
        ensure!(
            owner.version == OWNER_VERSION
                && owner.workspace == workspace.root()
                && owner.project == project_name(workspace.root())
                && owner.file.is_absolute()
                && owner.token.is_absolute()
                && owner.root_password.is_absolute()
                && owner.runtime_password.is_absolute()
                && owner.api_port != 0
                && owner.web_port != 0
                && owner.api_port != owner.web_port
                && owner
                    .settings
                    .keys()
                    .all(|key| SETTINGS.iter().any(|(allowed, _)| *allowed == key.as_str())),
            "Venture ownership file does not match this workspace"
        );
        local_socket(&owner.docker_host)?;
        ensure!(
            owner.host_storage.as_ref().map(|config| config.mode) == owner.storage.host_mode(),
            "Venture ownership has an inconsistent host storage mode"
        );
        match owner.storage {
            StorageMode::ComposeDisk => {
                ensure!(owner.endpoint == DISK_URL, "invalid owned storage endpoint")
            }
            StorageMode::External => {
                external_url(&owner.endpoint)?;
            }
            StorageMode::HostMemory | StorageMode::HostDisk => {
                let config = owner
                    .host_storage
                    .as_ref()
                    .context("missing host SurrealDB configuration")?;
                config.validate()?;
                ensure!(
                    config.container_url() == owner.endpoint,
                    "invalid host SurrealDB endpoint"
                );
            }
        }
        Ok(Some(owner))
    }

    pub(crate) fn api_origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.api_port)
    }

    pub(crate) fn token_file(&self) -> &Path {
        &self.token
    }

    pub(crate) async fn print_status(&self, workspace: &Workspace) -> Result<()> {
        if let Some(config) = &self.host_storage {
            host_storage::status(workspace, config).await?;
        }
        run(self.compose().args(["ps", "--all"]), "inspect services").await
    }

    pub(crate) async fn supervise_host_storage(
        &self,
        workspace: &Workspace,
        generation: String,
    ) -> Result<()> {
        let config = self
            .host_storage
            .as_ref()
            .context("this stack has no managed host storage")?;
        let root_name = self
            .settings
            .get("FLEETWORKS_STORAGE_ROOT_USERNAME")
            .map(String::as_str)
            .unwrap_or("root");
        host_storage::supervise(
            workspace,
            config,
            root_name,
            &self.root_password,
            generation,
        )
        .await
    }

    fn save(&self, workspace: &Workspace) -> Result<()> {
        let path = owner_path(workspace);
        let dir = path.parent().expect("ownership file has a parent");
        let mut staged = tempfile::NamedTempFile::new_in(dir)?;
        serde_json::to_writer_pretty(&mut staged, self)?;
        staged.write_all(b"\n")?;
        staged.as_file().sync_all()?;
        staged
            .persist_noclobber(&path)
            .map_err(|error| error.error)?;
        if let Ok(directory) = File::open(dir) {
            let _ = directory.sync_all();
        }
        Ok(())
    }

    fn docker(&self) -> Command {
        let mut command = Command::new("docker");
        command
            .kill_on_drop(true)
            .env_remove("DOCKER_CONTEXT")
            .env_remove("DOCKER_HOST")
            .env_remove("DOCKER_TLS")
            .env_remove("DOCKER_TLS_VERIFY")
            .arg("--host")
            .arg(&self.docker_host);
        command
    }

    fn compose(&self) -> Command {
        let mut command = self.docker();
        command
            .env_remove("COMPOSE_PROFILES")
            .env_remove("COMPOSE_ENV_FILES")
            .env_remove("FLEETWORKS_API_TOKEN")
            .env_remove("FLEETWORKS_STORAGE_ROOT_PASSWORD")
            .env_remove("FLEETWORKS_STORAGE_PASSWORD")
            .env("FLEETWORKS_HOST_WORKSPACE", &self.workspace)
            .env("FLEETWORKS_UID", self.uid.to_string())
            .env("FLEETWORKS_GID", self.gid.to_string())
            .env(API_PORT, self.api_port.to_string())
            .env(WEB_PORT, self.web_port.to_string())
            .env(STORAGE_URL, &self.endpoint)
            .env(API_TOKEN_FILE, &self.token)
            .env(ROOT_PASSWORD_FILE, &self.root_password)
            .env(RUNTIME_PASSWORD_FILE, &self.runtime_password);
        for (key, _) in SETTINGS {
            command.env_remove(key);
        }
        command
            .envs(&self.settings)
            .args(["compose", "--env-file", "/dev/null", "--project-name"])
            .arg(&self.project)
            .arg("--file")
            .arg(&self.file);
        if self.storage == StorageMode::ComposeDisk {
            command.args(["--profile", "disk"]);
        }
        command
    }

    async fn existing_project(&self) -> Result<bool> {
        let label = format!("label=com.docker.compose.project={}", self.project);
        for args in [["ps", "--all"], ["network", "ls"]] {
            let output = self
                .docker()
                .args(args)
                .arg("--filter")
                .arg(&label)
                .arg("--quiet")
                .output()
                .await
                .context("inspect local Docker project")?;
            ensure!(
                output.status.success(),
                "could not inspect local Docker project"
            );
            if !output.stdout.iter().all(u8::is_ascii_whitespace) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

pub(crate) async fn up(
    workspace: &Workspace,
    storage: Option<StorageMode>,
    compose_file: Option<&Path>,
) -> Result<()> {
    let docker_host = docker_host().await?;
    let owned_workspace = workspace.clone();
    let compose_file = compose_file.map(Path::to_owned);
    let (_lock, mut requested, owned) = tokio::task::spawn_blocking(move || {
        prepare(
            &owned_workspace,
            storage,
            compose_file.as_deref(),
            docker_host,
        )
    })
    .await??;
    if !owned {
        ensure!(
            !requested.existing_project().await?,
            "an unowned Docker project already uses this workspace's project name"
        );
        // A failed or interrupted start may leave resources. Record ownership
        // first so the next invocation can retry or clean up the same project.
        let workspace = workspace.clone();
        requested = tokio::task::spawn_blocking(move || -> Result<Owner> {
            requested.save(&workspace)?;
            Ok(requested)
        })
        .await??;
    }
    if let Some(config) = &requested.host_storage {
        host_storage::start(workspace, config).await?;
    }
    run(
        requested.compose().args([
            "up",
            "--detach",
            "--build",
            "--wait",
            "--wait-timeout",
            "120",
        ]),
        "start services",
    )
    .await?;
    println!("Fleetworks API {}", requested.api_origin());
    println!("Fleetworks Web http://127.0.0.1:{}", requested.web_port);
    Ok(())
}

fn prepare(
    workspace: &Workspace,
    storage: Option<StorageMode>,
    compose_file: Option<&Path>,
    docker_host: String,
) -> Result<(File, Owner, bool)> {
    let environment = workspace.environment()?;
    workspace.initialize()?;
    let lock = lifecycle_lock(workspace)?;
    let prior = Owner::read(workspace)?;
    let storage = storage
        .or_else(|| prior.as_ref().map(|owner| owner.storage))
        .unwrap_or(StorageMode::ComposeDisk);
    let file = match compose_file {
        Some(path) => fs::canonicalize(path),
        None => match environment.get("FLEETWORKS_COMPOSE_FILE") {
            Some(path) => fs::canonicalize(workspace.resolve_path(Path::new(&path))),
            None => match prior.as_ref() {
                Some(owner) => fs::canonicalize(&owner.file),
                None => fs::canonicalize("deploy/compose.yml"),
            },
        },
    }
    .context("resolve Compose file (use --compose-file from outside the checkout)")?;
    ensure!(file.is_file(), "Compose file must be a regular file");
    let host_storage = storage
        .host_mode()
        .map(|mode| {
            HostStorage::resolve(
                workspace,
                &environment,
                mode,
                prior.as_ref().and_then(|owner| owner.host_storage.as_ref()),
            )
        })
        .transpose()?;
    let endpoint = match storage {
        StorageMode::ComposeDisk => {
            if let Some(url) = value(&environment, STORAGE_URL)? {
                ensure!(
                    url.trim_end_matches('/') == DISK_URL,
                    "external storage requires --storage external"
                );
            }
            DISK_URL.to_owned()
        }
        StorageMode::External => {
            let url = value(&environment, STORAGE_URL)?
                .or_else(|| {
                    prior
                        .as_ref()
                        .filter(|owner| owner.storage == storage)
                        .map(|owner| owner.endpoint.clone())
                })
                .context("external storage requires FLEETWORKS_STORAGE_URL")?;
            external_url(&url)?
        }
        StorageMode::HostMemory | StorageMode::HostDisk => {
            let config = host_storage
                .as_ref()
                .context("missing host SurrealDB configuration")?;
            let endpoint = config.container_url();
            if let Some(url) = value(&environment, STORAGE_URL)? {
                ensure!(
                    url.trim_end_matches('/') == endpoint,
                    "FLEETWORKS_STORAGE_URL conflicts with the managed host SurrealDB bind"
                );
            }
            endpoint
        }
    };
    let api_port = configured_port(
        &environment,
        API_PORT,
        prior.as_ref().map(|owner| owner.api_port),
        8080,
    )?;
    let web_port = configured_port(
        &environment,
        WEB_PORT,
        prior.as_ref().map(|owner| owner.web_port),
        3000,
    )?;
    ensure!(
        api_port != web_port,
        "Fleetworks API and web ports must differ"
    );

    let token = secret(
        workspace,
        &environment,
        API_TOKEN_FILE,
        prior.as_ref().map(|owner| owner.token.as_path()),
        Some("api-token"),
    )?;
    workspace.read_api_token_file(&token)?;
    let root_password = secret(
        workspace,
        &environment,
        ROOT_PASSWORD_FILE,
        prior.as_ref().map(|owner| owner.root_password.as_path()),
        (storage != StorageMode::External).then_some("storage-root-password"),
    )?;
    let runtime_password = secret(
        workspace,
        &environment,
        RUNTIME_PASSWORD_FILE,
        prior.as_ref().map(|owner| owner.runtime_password.as_path()),
        Some("storage-password"),
    )?;
    let runtime_value = workspace.read_api_token_file(&runtime_password)?;
    ensure!(
        (32..=256).contains(&runtime_value.len())
            && runtime_value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-._~".contains(&byte)),
        "invalid Fleetworks runtime password file"
    );

    if storage == StorageMode::ComposeDisk {
        let data = workspace.root().join("storage/surrealdb");
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&data)
            .context("create local SurrealDB directory")?;
        ensure!(
            fs::symlink_metadata(&data)?.is_dir(),
            "SurrealDB data path must be a real directory"
        );
    }
    let (uid, gid) = workspace_ids(workspace)?;
    let mut settings = BTreeMap::new();
    for &(name, default) in SETTINGS {
        if let Some(setting) = value(&environment, name)?
            .or_else(|| {
                prior
                    .as_ref()
                    .and_then(|owner| owner.settings.get(name).cloned())
            })
            .or_else(|| default.map(str::to_owned))
        {
            settings.insert(name.to_owned(), setting);
        }
    }
    let requested = Owner {
        version: OWNER_VERSION,
        project: project_name(workspace.root()),
        workspace: workspace.root().to_owned(),
        file,
        docker_host,
        storage,
        host_storage,
        endpoint,
        api_port,
        web_port,
        uid,
        gid,
        token,
        root_password,
        runtime_password,
        settings,
    };
    if let Some(prior) = &prior {
        ensure!(
            prior == &requested,
            "stop the owned stack before changing its configuration"
        );
    }
    Ok((lock, requested, prior.is_some()))
}

pub(crate) async fn logs(workspace: &Workspace, follow: bool) -> Result<()> {
    let owner = Owner::load(workspace)
        .await?
        .context("this workspace has no Venture-owned stack")?;
    let mut command = owner.compose();
    command.arg("logs");
    if !follow {
        return run(&mut command, "read service logs").await;
    }
    let mut child = command
        .arg("--follow")
        .spawn()
        .context("follow service logs")?;
    let status = tokio::select! {
        status = child.wait() => status?,
        _ = shutdown_signal() => {
            child.kill().await.context("stop log follower")?;
            return Ok(());
        }
    };
    ensure!(status.success(), "log follower exited with {status}");
    Ok(())
}

pub(crate) async fn stop(workspace: &Workspace) -> Result<()> {
    let owned_workspace = workspace.clone();
    let prepared = tokio::task::spawn_blocking(move || prepare_stop(&owned_workspace)).await??;
    let Some((_lock, owner)) = prepared else {
        println!("This workspace has no Venture-owned stack");
        return Ok(());
    };
    let compose_result = async {
        run(
            owner.compose().args(["down", "--remove-orphans"]),
            "stop services",
        )
        .await?;
        ensure!(
            !owner.existing_project().await?,
            "owned Docker project still has resources"
        );
        Ok(())
    }
    .await;
    let host_result = match &owner.host_storage {
        Some(config) => host_storage::stop(workspace, config).await,
        None => Ok(()),
    };
    compose_result?;
    host_result?;
    tokio::fs::remove_file(owner_path(workspace))
        .await
        .context("clear stopped Venture ownership")?;
    Ok(())
}

fn prepare_stop(workspace: &Workspace) -> Result<Option<(File, Owner)>> {
    let storage = workspace.root().join("storage");
    match fs::symlink_metadata(&storage) {
        Ok(metadata) => ensure!(
            metadata.is_dir(),
            "workspace storage must be a real directory"
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => return Err(error).context("inspect workspace storage"),
    }
    let lock = lifecycle_lock(workspace)?;
    Ok(Owner::read(workspace)?.map(|owner| (lock, owner)))
}

fn owner_path(workspace: &Workspace) -> PathBuf {
    workspace.root().join("storage/venture-owner.json")
}

fn lifecycle_lock(workspace: &Workspace) -> Result<File> {
    let path = workspace.root().join("storage/venture.lock");
    match fs::symlink_metadata(&path) {
        Ok(metadata) => ensure!(metadata.is_file(), "invalid Venture lifecycle lock file"),
        Err(error) if error.kind() == io::ErrorKind::NotFound => (),
        Err(error) => return Err(error).context("inspect Venture lifecycle lock"),
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&path).context("open Venture lifecycle lock")?;
    file.try_lock()
        .map_err(|error| anyhow!("cannot acquire Venture lifecycle lock: {error}"))?;
    Ok(file)
}

fn project_name(root: &Path) -> String {
    let digest = Sha256::digest(root.as_os_str().as_encoded_bytes());
    let mut project = String::from("fleetworks-");
    for byte in &digest[..12] {
        write!(&mut project, "{byte:02x}").expect("writing into a String cannot fail");
    }
    project
}

fn value(environment: &Environment, key: &str) -> Result<Option<String>> {
    environment
        .get(key)
        .map(|value| {
            value
                .into_string()
                .map_err(|_| anyhow!("{key} must be UTF-8"))
        })
        .transpose()
}

fn configured_port(
    environment: &Environment,
    key: &str,
    prior: Option<u16>,
    default: u16,
) -> Result<u16> {
    Ok(value(environment, key)?
        .map(|raw| {
            raw.parse::<u16>()
                .ok()
                .filter(|port| *port > 0)
                .with_context(|| format!("invalid {key}"))
        })
        .transpose()?
        .or(prior)
        .unwrap_or(default))
}

fn secret(
    workspace: &Workspace,
    environment: &Environment,
    file_key: &str,
    prior: Option<&Path>,
    default: Option<&str>,
) -> Result<PathBuf> {
    let key = file_key
        .strip_suffix("_FILE")
        .expect("secret file key ends in _FILE");
    ensure!(
        environment.get(key).is_none(),
        "Venture Compose uses {file_key} for this credential"
    );
    let selected = environment
        .get(file_key)
        .map(|path| workspace.resolve_path(Path::new(&path)));
    if let Some(path) = selected.as_deref().or(prior) {
        let metadata = fs::symlink_metadata(path).with_context(|| format!("open {file_key}"))?;
        ensure!(
            metadata.is_file() && (1..=4096).contains(&metadata.len()),
            "invalid {file_key}"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            ensure!(
                metadata.permissions().mode() & 0o077 == 0,
                "{file_key} must be owner-only"
            );
        }
        return fs::canonicalize(path).with_context(|| format!("resolve {file_key}"));
    }
    workspace.initialize_secret_file(
        default.with_context(|| format!("external storage requires {file_key}"))?,
    )
}

fn workspace_ids(workspace: &Workspace) -> Result<(u32, u32)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(workspace.root().join("secrets"))?;
        Ok((metadata.uid(), metadata.gid()))
    }
    #[cfg(not(unix))]
    {
        let _ = workspace;
        anyhow::bail!("Venture Compose currently requires a Unix Docker socket")
    }
}

fn external_url(raw: &str) -> Result<String> {
    let url = Url::parse(raw).map_err(|_| anyhow!("invalid FLEETWORKS_STORAGE_URL"))?;
    let host = url.host_str().context("storage URL requires a host")?;
    ensure!(
        (url.scheme() == "wss" || (url.scheme() == "ws" && host == "host.docker.internal"))
            && url.username().is_empty()
            && url.password().is_none()
            && matches!(url.path(), "" | "/")
            && url.query().is_none()
            && url.fragment().is_none(),
        "storage URL must be a wss:// origin or ws://host.docker.internal without credentials"
    );
    ensure!(
        !host.eq_ignore_ascii_case("localhost")
            && !host.to_ascii_lowercase().ends_with(".localhost")
            && !host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback()),
        "use host.docker.internal to reach a database running on the Docker host"
    );
    Ok(url.to_string())
}

async fn docker_host() -> Result<String> {
    if env::var_os("DOCKER_CONTEXT").is_none() {
        if let Some(host) = env::var_os("DOCKER_HOST") {
            let host = host
                .into_string()
                .map_err(|_| anyhow!("invalid Docker socket"))?;
            return local_socket(&host);
        }
    }
    let output = Command::new("docker")
        .args([
            "context",
            "inspect",
            "--format",
            "{{.Endpoints.docker.Host}}",
        ])
        .kill_on_drop(true)
        .output()
        .await
        .context("inspect the local Docker context")?;
    ensure!(
        output.status.success(),
        "could not inspect the local Docker context"
    );
    let stdout = String::from_utf8(output.stdout).context("invalid Docker socket")?;
    local_socket(stdout.trim())
}

fn local_socket(raw: &str) -> Result<String> {
    ensure!(
        raw.strip_prefix("unix://")
            .is_some_and(|path| Path::new(path).is_absolute())
            && !raw.chars().any(char::is_control),
        "Venture requires a local Unix Docker socket"
    );
    Ok(raw.to_owned())
}

async fn run(command: &mut Command, operation: &str) -> Result<()> {
    let status = command
        .status()
        .await
        .with_context(|| format!("{operation} with Docker Compose"))?;
    ensure!(
        status.success(),
        "Docker Compose could not {operation} ({status})"
    );
    Ok(())
}

pub(crate) async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut terminate) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => (),
                _ = terminate.recv() => (),
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsStr;
    use std::fs;

    use fleetworks_workspace::Workspace;

    use super::{
        external_url, lifecycle_lock, local_socket, owner_path, project_name, Owner, StorageMode,
    };

    #[test]
    fn local_storage_uses_the_docker_gateway_and_remote_storage_requires_tls() {
        assert!(external_url("ws://host.docker.internal:8000").is_ok());
        assert!(external_url("wss://surreal.example.test:8000").is_ok());
        for invalid in [
            "ws://surreal.example.test:8000",
            "wss://root:fixture-secret@surreal.example.test",
            "wss://surreal.example.test/?secret=fixture-secret",
            "wss://localhost:8000",
            "wss://[::1]:8000",
        ] {
            let error = external_url(invalid).unwrap_err();
            assert!(!error.to_string().contains("fixture-secret"));
        }
        assert!(local_socket("unix:///tmp/docker.sock").is_ok());
        assert!(local_socket("ssh://fixture").is_err());
    }

    #[test]
    fn owner_record_locks_startup_and_is_bound_to_one_workspace() {
        let temporary = tempfile::tempdir().unwrap();
        let first = Workspace::resolve(Some(&temporary.path().join("one"))).unwrap();
        let second = Workspace::resolve(Some(&temporary.path().join("two"))).unwrap();
        first.initialize().unwrap();
        second.initialize().unwrap();
        assert!(Owner::read(&first).unwrap().is_none());
        assert_ne!(project_name(first.root()), project_name(second.root()));
        let lock = lifecycle_lock(&first).unwrap();
        assert!(lifecycle_lock(&first).is_err());
        drop(lock);
        assert!(lifecycle_lock(&first).is_ok());

        let owner = Owner {
            version: super::OWNER_VERSION,
            project: project_name(first.root()),
            workspace: first.root().to_owned(),
            file: first.root().join("compose.yml"),
            docker_host: "unix:///tmp/docker.sock".into(),
            storage: StorageMode::ComposeDisk,
            host_storage: None,
            endpoint: super::DISK_URL.into(),
            api_port: 18080,
            web_port: 13000,
            uid: 1000,
            gid: 1000,
            token: first.root().join("secrets/api-token"),
            root_password: first.root().join("secrets/storage-root-password"),
            runtime_password: first.root().join("secrets/storage-password"),
            settings: BTreeMap::new(),
        };
        owner.save(&first).unwrap();
        assert_eq!(Owner::read(&first).unwrap().as_ref(), Some(&owner));
        assert!(owner.save(&first).is_err());
        let command = owner.compose();
        assert!(command
            .as_std()
            .get_args()
            .any(|arg| arg == OsStr::new("disk")));
        fs::copy(owner_path(&first), owner_path(&second)).unwrap();
        assert!(Owner::read(&second).is_err());
    }
}
