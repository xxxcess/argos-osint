//! Own one local Docker Compose project per canonical workspace.

use std::collections::BTreeMap;
use std::env;
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, ensure, Context, Result};
use clap::ValueEnum;
use fleetworks_workspace::{Environment, Workspace};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::process::Command;
use url::Url;

use crate::host_storage::{self, Broker as HostBroker, Config as HostStorage, Mode as HostMode};

const DISK_URL: &str = "ws://surrealdb:8000";
const OWNER_VERSION: u8 = 1;
const OWNER_LIMIT: u64 = 64 * 1024;
const STORAGE_URL: &str = "FLEETWORKS_STORAGE_URL";
const API_PORT: &str = "FLEETWORKS_API_PORT";
const WEB_PORT: &str = "FLEETWORKS_WEB_PORT";
const API_TOKEN_FILE: &str = "FLEETWORKS_API_TOKEN_FILE";
const ROOT_PASSWORD_FILE: &str = "FLEETWORKS_STORAGE_ROOT_PASSWORD_FILE";
const RUNTIME_PASSWORD_FILE: &str = "FLEETWORKS_STORAGE_PASSWORD_FILE";
const BROKER_PASSWORD_FILE: &str = "FLEETWORKS_BROKER_PASSWORD_FILE";
const BROKER_URL_FILE: &str = "FLEETWORKS_BROKER_URL_FILE";
const COMPOSE_BROKER_CREDENTIAL_FILE: &str = "FLEETWORKS_COMPOSE_BROKER_CREDENTIAL_FILE";
const INTERNAL_TOKEN_FILE: &str = "FLEETWORKS_INTERNAL_TOKEN_FILE";
const PRIVATE_TOKEN_FILES: &[(&str, &str)] = &[
    ("FLEETWORKS_BRAIN_TOKEN_FILE", "brain-token"),
    ("FLEETWORKS_TOOLS_TOKEN_FILE", "tools-token"),
    ("FLEETWORKS_CONTROL_TOKEN_FILE", "control-token"),
    ("FLEETWORKS_JOBS_TOKEN_FILE", "jobs-token"),
    ("FLEETWORKS_EXECUTION_TOKEN_FILE", "execution-token"),
];
const SETTINGS: &[(&str, Option<&str>)] = &[
    ("FLEETWORKS_STORAGE_NAMESPACE", Some("fleetworks")),
    ("FLEETWORKS_STORAGE_DATABASE", Some("fleetworks")),
    ("FLEETWORKS_STORAGE_USERNAME", Some("fleetworks")),
    ("FLEETWORKS_STORAGE_ROOT_USERNAME", Some("root")),
    ("FLEETWORKS_API_ALLOWED_HOSTS", None),
    ("FLEETWORKS_API_ALLOWED_ORIGINS", None),
    ("FLEETWORKS_WEB_ORIGIN", None),
    ("FLEETWORKS_QUEUE_NAME", None),
    // Legacy owner decoding only; new launch preparation skips this setting.
    ("FLEETWORKS_BROKER_QUEUE", None),
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StorageMode {
    ComposeDisk,
    External,
    HostMemory,
    HostDisk,
    LocalNet,
    LocalProcess,
}

impl StorageMode {
    fn host_mode(self) -> Option<HostMode> {
        match self {
            Self::HostMemory => Some(HostMode::Memory),
            Self::HostDisk | Self::LocalProcess => Some(HostMode::Disk),
            Self::ComposeDisk | Self::External | Self::LocalNet => None,
        }
    }

    fn network_broker(self) -> bool {
        matches!(self, Self::LocalNet | Self::LocalProcess)
    }
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct NetworkBroker {
    overlay: PathBuf,
    url_file: PathBuf,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct AtlasStack {
    file: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    broker_password: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    network_broker: Option<NetworkBroker>,
    internal_token: PathBuf,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    host_broker: Option<HostBroker>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    atlas: Option<AtlasStack>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dev_file: Option<PathBuf>,
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
                && owner.dev_file.as_deref().is_none_or(Path::is_absolute)
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
        if let Some(atlas) = &owner.atlas {
            ensure!(
                atlas.file.is_absolute()
                    && atlas.internal_token.is_absolute()
                    && match (&atlas.broker_password, &atlas.network_broker) {
                        (Some(password), None) => {
                            !owner.storage.network_broker() && password.is_absolute()
                        }
                        (None, Some(broker)) => {
                            owner.storage.network_broker()
                                && broker.overlay.is_absolute()
                                && broker.url_file.is_absolute()
                                && (owner.storage != StorageMode::LocalProcess
                                    || broker.url_file == host_broker_url_path(workspace))
                        }
                        (None, None) => true,
                        (Some(_), Some(_)) => false,
                    },
                "invalid Atlas Compose ownership"
            );
        }
        ensure!(
            owner.host_storage.as_ref().map(|config| config.mode) == owner.storage.host_mode(),
            "Venture ownership has an inconsistent host storage mode"
        );
        ensure!(
            owner.host_broker.is_none() || owner.storage == StorageMode::LocalProcess,
            "Venture ownership has an inconsistent host broker configuration"
        );
        if let Some(broker) = &owner.host_broker {
            broker.validate()?;
        }
        match owner.storage {
            StorageMode::ComposeDisk => {
                ensure!(owner.endpoint == DISK_URL, "invalid owned storage endpoint")
            }
            StorageMode::External => {
                external_url(&owner.endpoint)?;
            }
            StorageMode::LocalNet => {
                local_net_url(&owner.endpoint)?;
            }
            StorageMode::HostMemory | StorageMode::HostDisk | StorageMode::LocalProcess => {
                let config = owner
                    .host_storage
                    .as_ref()
                    .context("missing host SurrealDB configuration")?;
                config.validate()?;
                if let Some(broker) = &owner.host_broker {
                    validate_distinct_host_ports(config, broker)?;
                }
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
            host_storage::status(workspace, config, self.host_broker.as_ref()).await?;
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
            self.host_broker.as_ref(),
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
        let mut command = docker_command();
        command
            .env_remove("DOCKER_CONTEXT")
            .env_remove("DOCKER_HOST")
            .env_remove("DOCKER_TLS")
            .env_remove("DOCKER_TLS_VERIFY")
            .arg("--host")
            .arg(&self.docker_host);
        command
    }

    pub(crate) fn compose(&self) -> Command {
        let mut command = self.docker();
        command
            .env_remove("COMPOSE_PROFILES")
            .env_remove("COMPOSE_ENV_FILES")
            .env_remove("FLEETWORKS_API_TOKEN")
            .env_remove("FLEETWORKS_TOOLS_TOKEN")
            .env_remove("FLEETWORKS_CONTROL_TOKEN")
            .env_remove("FLEETWORKS_JOBS_TOKEN")
            .env_remove("FLEETWORKS_STORAGE_ROOT_PASSWORD")
            .env_remove("FLEETWORKS_STORAGE_PASSWORD")
            .env_remove(COMPOSE_BROKER_CREDENTIAL_FILE)
            .env("FLEETWORKS_HOST_WORKSPACE", &self.workspace)
            .env("FLEETWORKS_UID", self.uid.to_string())
            .env("FLEETWORKS_GID", self.gid.to_string())
            .env(API_PORT, self.api_port.to_string())
            .env(WEB_PORT, self.web_port.to_string())
            .env(STORAGE_URL, &self.endpoint)
            .env(API_TOKEN_FILE, &self.token)
            .env(ROOT_PASSWORD_FILE, &self.root_password)
            .env(RUNTIME_PASSWORD_FILE, &self.runtime_password);
        for (variable, name) in PRIVATE_TOKEN_FILES {
            command.env(variable, self.workspace.join("secrets").join(name));
        }
        for (key, _) in SETTINGS {
            command.env_remove(key);
        }
        command
            .envs(&self.settings)
            .args(["compose", "--env-file", "/dev/null", "--project-name"])
            .arg(&self.project)
            .arg("--file")
            .arg(&self.file);
        if let Some(atlas) = &self.atlas {
            command
                .env_remove("FLEETWORKS_BROKER_PASSWORD")
                .env_remove(BROKER_PASSWORD_FILE)
                .env_remove("FLEETWORKS_BROKER_URL")
                .env_remove(BROKER_URL_FILE)
                .env_remove("FLEETWORKS_INTERNAL_TOKEN")
                .env(INTERNAL_TOKEN_FILE, &atlas.internal_token)
                .arg("--file")
                .arg(&atlas.file);
            if let Some(password) = &atlas.broker_password {
                command.env(BROKER_PASSWORD_FILE, password);
            }
            if let Some(broker) = &atlas.network_broker {
                command
                    .env(COMPOSE_BROKER_CREDENTIAL_FILE, &broker.url_file)
                    .arg("--file")
                    .arg(&broker.overlay);
            }
        }
        if let Some(file) = &self.dev_file {
            command.arg("--file").arg(file);
        }
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
    atlas: bool,
    dev: bool,
    no_build: bool,
) -> Result<()> {
    let prior = Owner::load(workspace).await?;
    let interactive = io::stdin().is_terminal();
    let prompt_for_url = interactive && storage.is_none() && prior.is_none();
    let storage = select_storage_target(
        storage,
        prior.as_ref().map(|owner| owner.storage),
        interactive,
    )?;
    let storage_url = if prompt_for_url
        && matches!(storage, StorageMode::External | StorageMode::LocalNet)
        && value(&workspace.environment()?, STORAGE_URL)?.is_none()
    {
        Some(prompt_storage_url(storage)?)
    } else {
        None
    };
    let docker_host = docker_host().await?;
    let owned_workspace = workspace.clone();
    let compose_file = compose_file.map(Path::to_owned);
    let (_lock, mut requested, owned) = tokio::task::spawn_blocking(move || {
        prepare(
            &owned_workspace,
            storage,
            storage_url.as_deref(),
            compose_file.as_deref(),
            docker_host,
            atlas,
            dev,
        )
    })
    .await??;
    if requested.storage == StorageMode::External {
        probe_remote_storage(&requested.endpoint).await?;
    }
    if !owned {
        ensure!(
            !requested.existing_project().await?,
            "an unowned Docker project already uses this workspace's project name"
        );
        if let Some(config) = &requested.host_storage {
            host_storage::check_binds(config, requested.host_broker.as_ref())?;
        }
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
        host_storage::start(workspace, config, requested.host_broker.as_ref()).await?;
    }
    let image_policy: &[&str] = if no_build {
        &["--no-build", "--pull", "never"]
    } else {
        &["--build"]
    };
    run(
        requested
            .compose()
            .args(["up", "--detach"])
            .args(image_policy)
            .args(["--wait", "--wait-timeout", "120"]),
        "start services",
    )
    .await?;
    println!("Fleetworks API {}", requested.api_origin());
    println!("Fleetworks Web http://127.0.0.1:{}", requested.web_port);
    Ok(())
}

fn select_storage_target(
    requested: Option<StorageMode>,
    prior: Option<StorageMode>,
    interactive: bool,
) -> Result<StorageMode> {
    if let Some(mode) = requested.or(prior) {
        return Ok(mode);
    }
    if !interactive {
        return Ok(StorageMode::ComposeDisk);
    }
    prompt_storage_target()
}

fn prompt_storage_target() -> Result<StorageMode> {
    println!("Choose the storage target for this stack.");
    println!("  local-net      Use SurrealDB on your local network");
    println!("  local-process  Start disk-backed SurrealDB on this host");
    println!("  remote         Connect to a remote SurrealDB server");
    loop {
        let choice = prompt_line("Storage target [local-net]: ")?;
        match interpret_storage_choice(&choice) {
            Some(mode) => return Ok(mode),
            None => println!("Enter local-net, local-process, or remote."),
        }
    }
}

fn prompt_storage_url(storage: StorageMode) -> Result<String> {
    let (label, validate): (_, fn(&str) -> Result<String>) = match storage {
        StorageMode::LocalNet => (
            "SurrealDB URL (ws://host.docker.internal:8000): ",
            local_net_url,
        ),
        _ => ("SurrealDB URL (wss://host:port): ", external_url),
    };
    loop {
        let raw = prompt_line(label)?;
        match validate(&raw) {
            Ok(url) => return Ok(url),
            Err(error) => println!("{error:#}"),
        }
    }
}

fn prompt_line(label: &str) -> Result<String> {
    print!("{label}");
    io::stdout().flush().context("write storage prompt")?;
    let mut line = String::new();
    let bytes = io::stdin()
        .read_line(&mut line)
        .context("read storage choice")?;
    ensure!(bytes != 0, "storage prompt closed");
    Ok(line.trim().to_owned())
}

fn interpret_storage_choice(line: &str) -> Option<StorageMode> {
    match line.trim().to_ascii_lowercase().as_str() {
        "" | "local-net" => Some(StorageMode::LocalNet),
        "local-process" => Some(StorageMode::LocalProcess),
        "local" => Some(StorageMode::HostDisk),
        "remote" => Some(StorageMode::External),
        _ => None,
    }
}

fn remote_health_url(endpoint: &str) -> Result<Url> {
    let mut url = Url::parse(endpoint).context("invalid storage URL")?;
    let scheme = match url.scheme() {
        "wss" => "https",
        "ws" => "http",
        _ => anyhow::bail!("storage URL must use ws or wss"),
    };
    url.set_scheme(scheme)
        .map_err(|()| anyhow!("invalid storage health URL"))?;
    url.set_path("health");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

async fn probe_remote_storage(endpoint: &str) -> Result<()> {
    let health = remote_health_url(endpoint)?;
    let host = health
        .host_str()
        .unwrap_or("the remote database")
        .to_owned();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("prepare the SurrealDB connectivity check")?;
    let response = client
        .get(health)
        .send()
        .await
        .with_context(|| format!("SurrealDB at {host} is not reachable"))?;
    ensure!(
        response.status().is_success(),
        "SurrealDB at {host} is not operational ({})",
        response.status()
    );
    println!("SurrealDB at {host} is reachable");
    Ok(())
}

fn prepare(
    workspace: &Workspace,
    storage: StorageMode,
    storage_url: Option<&str>,
    compose_file: Option<&Path>,
    docker_host: String,
    atlas: bool,
    dev: bool,
) -> Result<(File, Owner, bool)> {
    let environment = workspace.environment()?;
    workspace.initialize()?;
    let lock = lifecycle_lock(workspace)?;
    let prior = Owner::read(workspace)?;
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
    let dev_file = if dev || prior.as_ref().is_some_and(|owner| owner.dev_file.is_some()) {
        Some(compose_overlay(&file, "compose.dev.yml")?)
    } else {
        None
    };
    let host_broker = None;
    let prior_atlas = prior.as_ref().and_then(|owner| owner.atlas.as_ref());
    let atlas = if atlas || prior_atlas.is_some() {
        Some(prepare_atlas(workspace, &environment, &file, prior_atlas)?)
    } else {
        None
    };
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
                    "a configured storage URL requires --storage local-net or --storage external"
                );
            }
            DISK_URL.to_owned()
        }
        StorageMode::External | StorageMode::LocalNet => {
            let url = match storage_url {
                Some(url) => url.to_owned(),
                None => value(&environment, STORAGE_URL)?
                    .or_else(|| {
                        prior
                            .as_ref()
                            .filter(|owner| owner.storage == storage)
                            .map(|owner| owner.endpoint.clone())
                    })
                    .context("network storage requires FLEETWORKS_STORAGE_URL")?,
            };
            if storage == StorageMode::LocalNet {
                local_net_url(&url)?
            } else {
                external_url(&url)?
            }
        }
        StorageMode::HostMemory | StorageMode::HostDisk | StorageMode::LocalProcess => {
            let config = host_storage
                .as_ref()
                .context("missing host SurrealDB configuration")?;
            if let Some(broker) = &host_broker {
                validate_distinct_host_ports(config, broker)?;
            }
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
    for (_, name) in PRIVATE_TOKEN_FILES {
        workspace.initialize_secret_file(name)?;
    }
    let root_password = secret(
        workspace,
        &environment,
        ROOT_PASSWORD_FILE,
        prior.as_ref().map(|owner| owner.root_password.as_path()),
        (!matches!(storage, StorageMode::External | StorageMode::LocalNet))
            .then_some("storage-root-password"),
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

    create_storage_dir(workspace, "jobs")?;
    if storage == StorageMode::ComposeDisk {
        create_storage_dir(workspace, "surrealdb")?;
    }
    let (uid, gid) = workspace_ids(workspace)?;
    let mut settings = BTreeMap::new();
    for &(name, default) in SETTINGS {
        if name == "FLEETWORKS_BROKER_QUEUE" {
            continue;
        }
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
        host_broker,
        atlas,
        dev_file,
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

fn prepare_atlas(
    workspace: &Workspace,
    environment: &Environment,
    compose_file: &Path,
    prior: Option<&AtlasStack>,
) -> Result<AtlasStack> {
    let file = compose_overlay(compose_file, "compose.atlas.yml")?;
    let internal_token = secret(
        workspace,
        environment,
        INTERNAL_TOKEN_FILE,
        prior.map(|atlas| atlas.internal_token.as_path()),
        Some("internal-token"),
    )?;
    workspace.read_api_token_file(&internal_token)?;
    Ok(AtlasStack {
        file,
        broker_password: None,
        network_broker: None,
        internal_token,
    })
}

fn compose_overlay(base: &Path, name: &str) -> Result<PathBuf> {
    let path = fs::canonicalize(base.with_file_name(name))
        .with_context(|| format!("resolve {name} beside the base Compose file"))?;
    ensure!(path.is_file(), "{name} must be a regular file");
    Ok(path)
}

// Kept solely to validate and stop owners created by the previous launcher.
fn host_broker_url_path(workspace: &Workspace) -> PathBuf {
    workspace.root().join("secrets/host-broker-url")
}

fn validate_distinct_host_ports(storage: &HostStorage, broker: &HostBroker) -> Result<()> {
    ensure!(
        storage.bind.port() != broker.config.bind.port(),
        "host SurrealDB and RabbitMQ ports must differ"
    );
    Ok(())
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
        Some(config) => host_storage::stop(workspace, config, owner.host_broker.as_ref()).await,
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
        default.with_context(|| format!("network services require {file_key}"))?,
    )
}

fn create_storage_dir(workspace: &Workspace, name: &str) -> Result<()> {
    let data = workspace.root().join("storage").join(name);
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(&data)
        .with_context(|| format!("create {name} data directory"))?;
    ensure!(
        fs::symlink_metadata(&data)?.is_dir(),
        "{name} data path must be a real directory"
    );
    Ok(())
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
    storage_origin(raw, false)
}

fn local_net_url(raw: &str) -> Result<String> {
    storage_origin(raw, true)
}

fn storage_origin(raw: &str, local_net: bool) -> Result<String> {
    let url = Url::parse(raw).map_err(|_| anyhow!("invalid FLEETWORKS_STORAGE_URL"))?;
    let host = url.host_str().context("storage URL requires a host")?;
    ensure!(
        (url.scheme() == "wss"
            || (url.scheme() == "ws" && (local_net || host == "host.docker.internal")))
            && url.username().is_empty()
            && url.password().is_none()
            && matches!(url.path(), "" | "/")
            && url.query().is_none()
            && url.fragment().is_none()
            && url.port() != Some(0),
        "storage URL must be a credential-free origin (ws:// or wss:// for local-net, wss:// or ws://host.docker.internal for external)"
    );
    ensure!(
        !loopback_host(host),
        "use host.docker.internal to reach a database running on the Docker host"
    );
    Ok(url.to_string())
}

fn loopback_host(host: &str) -> bool {
    let host = host.trim_end_matches('.');
    host.eq_ignore_ascii_case("localhost")
        || host.to_ascii_lowercase().ends_with(".localhost")
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
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
    let output = docker_command()
        .args([
            "context",
            "inspect",
            "--format",
            "{{.Endpoints.docker.Host}}",
        ])
        .output()
        .await
        .context("inspect the local Docker context (install Docker and make its CLI available)")?;
    ensure!(
        output.status.success(),
        "could not inspect the local Docker context"
    );
    let stdout = String::from_utf8(output.stdout).context("invalid Docker socket")?;
    local_socket(stdout.trim())
}

fn docker_command() -> Command {
    let mut command = Command::new("docker");
    command.kill_on_drop(true);
    #[cfg(target_os = "macos")]
    {
        let search_path = env::var_os("PATH").unwrap_or_default();
        let mut directories: Vec<_> = env::split_paths(&search_path).collect();
        directories.push(PathBuf::from(
            "/Applications/Docker.app/Contents/Resources/bin",
        ));
        if let Some(home) = env::var_os("HOME") {
            directories
                .push(PathBuf::from(home).join("Applications/Docker.app/Contents/Resources/bin"));
        }
        if let Ok(search_path) = env::join_paths(directories) {
            command.env("PATH", search_path);
        }
    }
    command
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
        external_url, interpret_storage_choice, lifecycle_lock, local_net_url, local_socket,
        owner_path, project_name, remote_health_url, Owner, StorageMode,
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
    fn storage_prompt_defaults_to_local_network_and_accepts_owned_processes() {
        assert_eq!(interpret_storage_choice(""), Some(StorageMode::LocalNet));
        assert_eq!(
            interpret_storage_choice("local-net"),
            Some(StorageMode::LocalNet)
        );
        assert_eq!(
            interpret_storage_choice("local-process"),
            Some(StorageMode::LocalProcess)
        );
        assert_eq!(
            interpret_storage_choice("local"),
            Some(StorageMode::HostDisk)
        );
        assert_eq!(
            interpret_storage_choice(" remote "),
            Some(StorageMode::External)
        );
        assert_eq!(interpret_storage_choice("compose-disk"), None);
    }

    #[test]
    fn local_net_accepts_docker_reachable_endpoints_without_exposing_credentials() {
        assert!(local_net_url("ws://surreal.internal:8010").is_ok());
        assert!(local_net_url("wss://surreal.internal:8010").is_ok());
        assert!(local_net_url("ws://192.168.64.2:8000").is_ok());
        assert!(local_net_url("ws://127.0.0.1:8000").is_err());
        assert!(local_net_url("ws://surreal.internal/rpc").is_err());
    }

    #[test]
    fn new_atlas_configuration_needs_only_the_internal_service_token() {
        let temporary = tempfile::tempdir().unwrap();
        let workspace = Workspace::resolve(Some(temporary.path())).unwrap();
        workspace.initialize().unwrap();
        let base = workspace.root().join("compose.yml");
        fs::write(&base, "services: {}\n").unwrap();
        fs::write(base.with_file_name("compose.atlas.yml"), "services: {}\n").unwrap();
        let atlas =
            super::prepare_atlas(&workspace, &workspace.environment().unwrap(), &base, None)
                .unwrap();
        assert!(atlas.broker_password.is_none());
        assert!(atlas.network_broker.is_none());
        assert!(atlas.internal_token.is_file());
        assert!(!workspace.root().join("secrets/broker-password").exists());
        assert!(!workspace.root().join("secrets/host-broker-url").exists());
        assert!(!workspace.root().join("storage/rabbitmq").exists());
    }

    #[test]
    fn remote_health_check_uses_the_storage_origin() {
        let health = remote_health_url("wss://surreal.example.test:8000").unwrap();
        assert_eq!(health.as_str(), "https://surreal.example.test:8000/health");
        let host = remote_health_url("ws://host.docker.internal:8010/").unwrap();
        assert_eq!(host.as_str(), "http://host.docker.internal:8010/health");
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

        let mut owner = Owner {
            version: super::OWNER_VERSION,
            project: project_name(first.root()),
            workspace: first.root().to_owned(),
            file: first.root().join("compose.yml"),
            docker_host: "unix:///tmp/docker.sock".into(),
            storage: StorageMode::ComposeDisk,
            host_storage: None,
            host_broker: None,
            atlas: None,
            dev_file: None,
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

        fs::remove_file(owner_path(&first)).unwrap();
        let dev_file = owner.file.with_file_name("compose.dev.yml");
        owner.dev_file = Some(dev_file.clone());
        owner.save(&first).unwrap();
        let saved = Owner::read(&first).unwrap().unwrap();
        let command = saved.compose();
        let args: Vec<_> = command.as_std().get_args().collect();
        assert!(args.windows(4).any(|pair| pair
            == [
                OsStr::new("--file"),
                owner.file.as_os_str(),
                OsStr::new("--file"),
                dev_file.as_os_str()
            ]));

        fs::remove_file(owner_path(&first)).unwrap();
        owner.atlas = Some(super::AtlasStack {
            file: first.root().join("compose.atlas.yml"),
            broker_password: Some(first.root().join("secrets/broker-password")),
            network_broker: None,
            internal_token: first.root().join("secrets/internal-token"),
        });
        owner.save(&first).unwrap();
        let legacy = Owner::read(&first).unwrap().unwrap();
        assert_eq!(legacy, owner);
        assert!(legacy
            .compose()
            .as_std()
            .get_envs()
            .any(|(key, value)| key == OsStr::new(super::BROKER_PASSWORD_FILE) && value.is_some()));

        fs::copy(owner_path(&first), owner_path(&second)).unwrap();
        assert!(Owner::read(&second).is_err());
    }
}
