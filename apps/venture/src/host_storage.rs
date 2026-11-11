//! Detached host dependency supervisor. Only the owner signals the children.
//! @tag:process-lifecycle

use std::env;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use fleetworks_workspace::{Environment, Workspace};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::time::{sleep, timeout, Instant};

use crate::host_broker;

const VERSION: &str = "3.3.4";
const DEFAULT_BIND: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 8010);
const STATE_LIMIT: u64 = 4096;
const START_TIMEOUT: Duration = Duration::from_secs(35);
const STOP_TIMEOUT: Duration = Duration::from_secs(22);
const POLL: Duration = Duration::from_millis(150);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Mode {
    Memory,
    Disk,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    pub(crate) mode: Mode,
    pub(crate) bind: SocketAddrV4,
    binary: PathBuf,
}

impl Config {
    pub(crate) fn resolve(
        workspace: &Workspace,
        environment: &Environment,
        mode: Mode,
        prior: Option<&Self>,
    ) -> Result<Self> {
        let bind = environment
            .get("FLEETWORKS_HOST_DB_BIND")
            .map(|value| {
                value
                    .to_str()
                    .and_then(|value| value.parse::<SocketAddrV4>().ok())
                    .context("FLEETWORKS_HOST_DB_BIND must be an IPv4 address and port")
            })
            .transpose()?
            .or_else(|| prior.map(|prior| prior.bind))
            .unwrap_or(DEFAULT_BIND);
        let binary = match environment.get("FLEETWORKS_SURREAL_BIN") {
            Some(path) => workspace.resolve_path(Path::new(&path)),
            None => prior
                .map(|prior| prior.binary.clone())
                .or_else(|| {
                    env::var_os("PATH").and_then(|paths| {
                        env::split_paths(&paths)
                            .map(|path| path.join("surreal"))
                            .find(|path| path.is_file())
                    })
                })
                .context("SurrealDB 3.3.4 was not found; set FLEETWORKS_SURREAL_BIN")?,
        };
        let binary = binary
            .canonicalize()
            .context("resolve host SurrealDB executable")?;
        ensure!(
            binary.is_file(),
            "host SurrealDB executable must be a regular file"
        );
        let config = Self { mode, bind, binary };
        config.validate()?;
        Ok(config)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.binary.is_absolute()
                && self.bind.port() != 0
                && !self.bind.ip().is_multicast()
                && !self.bind.ip().is_broadcast(),
            "invalid owned host SurrealDB configuration"
        );
        Ok(())
    }

    pub(crate) fn container_url(&self) -> String {
        format!("ws://host.docker.internal:{}", self.bind.port())
    }

    fn probe_addr(&self) -> SocketAddrV4 {
        if self.bind.ip().is_unspecified() {
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.bind.port())
        } else {
            self.bind
        }
    }

    fn identity(&self, broker: Option<&Broker>) -> Result<String> {
        let body = match broker {
            Some(broker) => serde_json::to_vec(&(self, broker))?,
            None => serde_json::to_vec(self)?,
        };
        Ok(format!("{:x}", Sha256::digest(body)))
    }
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Broker {
    pub(crate) config: host_broker::Config,
    pub(crate) password: PathBuf,
}

impl Broker {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.password.is_absolute(),
            "invalid host RabbitMQ password path"
        );
        self.config.validate()
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct State {
    generation: String,
    identity: String,
    ready: bool,
}

#[derive(Clone)]
struct Control {
    dir: PathBuf,
}

enum Observation {
    Unowned(File),
    Owned(Option<State>),
}

impl Control {
    fn open(workspace: &Workspace) -> Result<Self> {
        let dir = workspace.root().join("storage/venture-host-db");
        private_dir(&dir)?;
        Ok(Self { dir })
    }

    async fn load(workspace: &Workspace) -> Result<Self> {
        let workspace = workspace.clone();
        tokio::task::spawn_blocking(move || Self::open(&workspace)).await?
    }

    async fn observe(&self) -> Result<Observation> {
        let control = self.clone();
        tokio::task::spawn_blocking(move || match control.lock()? {
            Some(file) => Ok(Observation::Unowned(file)),
            None => Ok(Observation::Owned(control.state()?)),
        })
        .await?
    }

    fn lock(&self) -> Result<Option<File>> {
        let path = self.dir.join("supervisor.lock");
        match fs::symlink_metadata(&path) {
            Ok(metadata) => ensure!(metadata.is_file(), "invalid host SurrealDB supervisor lock"),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error).context("inspect host SurrealDB supervisor lock"),
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(error).context("lock host SurrealDB supervisor"),
        }
    }

    fn state(&self) -> Result<Option<State>> {
        let state = read_small(&self.dir.join("state.json"))?
            .map(|bytes| serde_json::from_slice::<State>(&bytes))
            .transpose()
            .context("read host SurrealDB state")?;
        if let Some(state) = &state {
            ensure!(
                valid_generation(&state.generation),
                "invalid host SurrealDB generation"
            );
        }
        Ok(state)
    }

    fn save(&self, state: &State) -> Result<()> {
        self.write("state.json", &serde_json::to_vec(state)?)
    }

    fn request_stop(&self, generation: &str) -> Result<()> {
        self.write("stop", generation.as_bytes())
    }

    fn stop_requested(&self, generation: &str) -> Result<bool> {
        Ok(read_small(&self.dir.join("stop"))?.is_some_and(|bytes| bytes == generation.as_bytes()))
    }

    fn write(&self, name: &str, bytes: &[u8]) -> Result<()> {
        use std::io::Write;
        let mut pending = tempfile::NamedTempFile::new_in(&self.dir)?;
        pending.write_all(bytes)?;
        pending.as_file().sync_all()?;
        pending
            .persist(self.dir.join(name))
            .map_err(|error| error.error)?;
        Ok(())
    }
}

fn valid_generation(generation: &str) -> bool {
    (8..=80).contains(&generation.len())
        && generation
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn read_small(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => ensure!(metadata.is_file(), "invalid host SurrealDB control file"),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("inspect host SurrealDB control file"),
    }
    let mut body = Vec::new();
    File::open(path)?
        .take(STATE_LIMIT + 1)
        .read_to_end(&mut body)?;
    ensure!(
        body.len() as u64 <= STATE_LIMIT,
        "host SurrealDB control file is too large"
    );
    Ok(Some(body))
}

pub(crate) fn private_dir(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => (),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error).context("create host SurrealDB directory"),
    }
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir(),
        "host SurrealDB path must be a real directory"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "host SurrealDB directory must be accessible to its owner only"
        );
    }
    Ok(())
}

pub(crate) fn private_log(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(0o600);
        match fs::symlink_metadata(path) {
            Ok(metadata) => ensure!(
                metadata.is_file() && metadata.permissions().mode() & 0o077 == 0,
                "host service log must be a private regular file"
            ),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error).context("inspect host service log"),
        }
    }
    options.open(path).context("open host service log")
}

pub(crate) fn check_binds(config: &Config, broker: Option<&Broker>) -> Result<()> {
    // A collision probe does not confer ownership of a listener.
    TcpListener::bind(config.bind)
        .context("host SurrealDB bind is already in use or unavailable")?;
    if let Some(broker) = broker {
        TcpListener::bind(broker.config.bind)
            .context("host RabbitMQ bind is already in use or unavailable")?;
    }
    Ok(())
}

fn startup_timeout(broker: Option<&Broker>) -> Duration {
    if broker.is_some() {
        Duration::from_secs(60)
    } else {
        START_TIMEOUT
    }
}

pub(crate) async fn start(
    workspace: &Workspace,
    config: &Config,
    broker: Option<&Broker>,
) -> Result<()> {
    ensure!(
        broker.is_none(),
        "stop the legacy RabbitMQ stack before starting the embedded queue stack"
    );
    let control = Control::load(workspace).await?;
    let identity = config.identity(broker)?;
    let mut supervisor: Option<(String, std::process::Child)> = None;
    let deadline = Instant::now() + startup_timeout(broker);
    loop {
        match control.observe().await? {
            Observation::Unowned(lock) => {
                if let Some((_, child)) = &mut supervisor {
                    if child.try_wait()?.is_some() {
                        bail!("host dependency supervisor exited; see logs/host-surreal-supervisor.log");
                    }
                    drop(lock);
                } else {
                    check_binds(config, broker)?;
                    // The coordinator still holds the stack lifecycle lock.
                    // Release the probe lock before the child tries to own it.
                    drop(lock);
                    let workspace = workspace.clone();
                    let dir = control.dir.clone();
                    supervisor =
                        Some(tokio::task::spawn_blocking(move || launch(&workspace, &dir)).await??);
                }
            }
            Observation::Owned(Some(state)) => {
                if supervisor
                    .as_ref()
                    .is_none_or(|(generation, _)| *generation == state.generation)
                {
                    ensure!(
                        state.identity == identity,
                        "another Venture host dependency configuration owns this workspace"
                    );
                    if state.ready && dependencies_ready(config, broker).await {
                        if let Some((_, mut child)) = supervisor {
                            std::thread::spawn(move || {
                                let _ = child.wait();
                            });
                        }
                        return Ok(());
                    }
                }
            }
            Observation::Owned(None) => (),
        }
        ensure!(
            Instant::now() < deadline,
            "host dependencies did not become ready; see logs/host-surreal-supervisor.log"
        );
        sleep(POLL).await;
    }
}

fn launch(workspace: &Workspace, control_dir: &Path) -> Result<(String, std::process::Child)> {
    let request = tempfile::Builder::new()
        .prefix("generation-")
        .tempfile_in(control_dir)?;
    let generation = request
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .context("generate host SurrealDB request")?
        .to_owned();
    let mut command = std::process::Command::new(env::current_exe()?);
    let log = private_log(&workspace.root().join("logs/host-surreal-supervisor.log"))?;
    command
        .arg("--workspace")
        .arg(workspace.root())
        .args(["__host-storage-supervisor", &generation])
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command
        .spawn()
        .context("launch host SurrealDB supervisor")?;
    Ok((generation, child))
}

pub(crate) async fn status(
    workspace: &Workspace,
    config: &Config,
    broker: Option<&Broker>,
) -> Result<()> {
    let control = Control::load(workspace).await?;
    let identity = config.identity(broker)?;
    let status = match control.observe().await? {
        Observation::Unowned(_) => "stopped",
        Observation::Owned(Some(state)) if state.ready && state.identity == identity => {
            if dependencies_ready(config, broker).await {
                "ready"
            } else {
                "unavailable"
            }
        }
        Observation::Owned(_) => "starting or unavailable",
    };
    println!(
        "Host SurrealDB {:?} {} ({status})",
        config.mode, config.bind
    );
    if let Some(broker) = broker {
        println!("Host RabbitMQ {} ({status})", broker.config.bind);
    }
    Ok(())
}

pub(crate) async fn stop(
    workspace: &Workspace,
    config: &Config,
    broker: Option<&Broker>,
) -> Result<()> {
    let control = Control::load(workspace).await?;
    let identity = config.identity(broker)?;
    let deadline = Instant::now() + STOP_TIMEOUT;
    let mut signalled = None;
    loop {
        match control.observe().await? {
            Observation::Unowned(_lock) => {
                ensure!(
                    timeout(
                        Duration::from_secs(1),
                        TcpStream::connect(config.probe_addr())
                    )
                    .await
                    .is_ok_and(|result| result.is_err()),
                    "host SurrealDB address still has a listener without a Venture supervisor"
                );
                if let Some(broker) = broker {
                    ensure!(
                        timeout(
                            Duration::from_secs(1),
                            TcpStream::connect(broker.config.probe_addr())
                        )
                        .await
                        .is_ok_and(|result| result.is_err()),
                        "host RabbitMQ address still has a listener without a Venture supervisor"
                    );
                }
                return Ok(());
            }
            Observation::Owned(Some(state)) => {
                ensure!(
                    state.identity == identity,
                    "Venture does not own this host dependency configuration"
                );
                if signalled.as_deref() != Some(state.generation.as_str()) {
                    let control = control.clone();
                    let generation = state.generation.clone();
                    tokio::task::spawn_blocking(move || control.request_stop(&generation))
                        .await??;
                    signalled = Some(state.generation);
                }
            }
            Observation::Owned(None) => (),
        }
        ensure!(
            Instant::now() < deadline,
            "host dependency supervisor did not finish stopping"
        );
        sleep(POLL).await;
    }
}

pub(crate) async fn supervise(
    workspace: &Workspace,
    config: &Config,
    root_name: &str,
    root_password: &Path,
    broker: Option<&Broker>,
    generation: String,
) -> Result<()> {
    ensure!(
        broker.is_none(),
        "legacy RabbitMQ supervision is shutdown-only"
    );
    ensure!(
        valid_generation(&generation),
        "invalid host SurrealDB generation"
    );
    let control = Control::load(workspace).await?;
    let initial = State {
        generation,
        identity: config.identity(broker)?,
        ready: false,
    };
    let control_for_start = control.clone();
    let (_lock, mut state) = tokio::task::spawn_blocking(move || -> Result<(File, State)> {
        let lock = control_for_start
            .lock()?
            .context("host SurrealDB already has a Venture supervisor")?;
        control_for_start.save(&initial)?;
        Ok((lock, initial))
    })
    .await??;
    let result = run_server(
        workspace,
        config,
        root_name,
        root_password,
        broker,
        &control,
        &mut state,
    )
    .await;
    // Only the process with the lock may clear the marker.
    if let Err(error) = tokio::fs::remove_file(control.dir.join("state.json")).await {
        if error.kind() != io::ErrorKind::NotFound {
            return result.and(Err(error).context("clear host SurrealDB state"));
        }
    }
    result
}

async fn run_server(
    workspace: &Workspace,
    config: &Config,
    root_name: &str,
    root_password: &Path,
    _broker: Option<&Broker>,
    control: &Control,
    state: &mut State,
) -> Result<()> {
    let output = timeout(
        Duration::from_secs(5),
        Command::new(&config.binary)
            .arg("version")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("host SurrealDB version probe timed out")??;
    let version = std::str::from_utf8(&output.stdout).unwrap_or_default();
    ensure!(
        output.status.success() && version.split_ascii_whitespace().next() == Some(VERSION),
        "host SurrealDB executable must report version {VERSION}"
    );
    let data_workspace = workspace.clone();
    let mode = config.mode;
    let (backend, log) = tokio::task::spawn_blocking(move || -> Result<(String, File)> {
        let backend = match mode {
            Mode::Memory => "memory".to_owned(),
            Mode::Disk => {
                let path = data_workspace.root().join("storage/host-surrealdb");
                private_dir(&path)?;
                let path = path
                    .to_str()
                    .context("host SurrealDB data path must be UTF-8")?;
                ensure!(
                    !path.contains(['?', '#']),
                    "invalid host SurrealDB data path"
                );
                format!("rocksdb://{path}?versioned=false&sync=every")
            }
        };
        let log = private_log(&data_workspace.root().join("logs/host-surreal.log"))?;
        Ok((backend, log))
    })
    .await??;
    let mut password = String::new();
    tokio::fs::File::open(root_password)
        .await
        .context("open host SurrealDB root credential")?
        .take(4097)
        .read_to_string(&mut password)
        .await
        .context("read host SurrealDB root credential")?;
    let password = password.trim_end_matches(['\r', '\n']);
    ensure!(
        !password.is_empty() && password.len() <= 4096 && !password.contains('\0'),
        "invalid host SurrealDB root credential"
    );
    let mut command = Command::new(&config.binary);
    for (name, _) in env::vars_os() {
        if name.to_string_lossy().starts_with("SURREAL_") {
            command.env_remove(name);
        }
    }
    TcpListener::bind(config.bind)
        .context("host SurrealDB bind is already in use or unavailable")?;
    let mut child = command
        .args(["start", "--no-banner", "--log", "info", "--bind"])
        .arg(config.bind.to_string())
        .arg(backend)
        .env("SURREAL_USER", root_name)
        .env("SURREAL_PASS", password)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .kill_on_drop(true)
        .spawn()
        .context("start host SurrealDB")?;
    let result = watch_child(config, control, state, &mut child).await;
    let stopped = finish_child(&mut child, "SurrealDB").await;
    result.and(stopped)
}

async fn watch_child(
    config: &Config,
    control: &Control,
    state: &mut State,
    child: &mut Child,
) -> Result<()> {
    let deadline = Instant::now() + START_TIMEOUT - Duration::from_secs(5);
    let signal = crate::compose::shutdown_signal();
    tokio::pin!(signal);
    loop {
        tokio::select! {
            result = child.wait() => {
                bail!("host SurrealDB exited ({}); see logs/host-surreal.log", result?);
            }
            _ = &mut signal => return Ok(()),
            _ = sleep(POLL) => {
                let stop_control = control.clone();
                let generation = state.generation.clone();
                if tokio::task::spawn_blocking(move || stop_control.stop_requested(&generation)).await?? {
                    return Ok(());
                }
                if !state.ready {
                    if dependencies_ready(config, None).await && child.try_wait()?.is_none() {
                        state.ready = true;
                        let control = control.clone();
                        let body = serde_json::to_vec(state)?;
                        tokio::task::spawn_blocking(move || control.write("state.json", &body)).await??;
                    } else if Instant::now() >= deadline {
                        bail!("host SurrealDB did not become ready; see logs/host-surreal.log");
                    }
                }
            }
        }
    }
}

async fn finish_child(child: &mut Child, service: &str) -> Result<()> {
    if child.try_wait()?.is_some() {
        return Ok(());
    }
    if let Some(pid) = child.id() {
        // The supervisor still owns this unreaped child, so the PID cannot
        // refer to a different process. Controller files never supply a PID.
        let sent = Command::new("/bin/kill")
            .args(["-TERM", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .status()
            .await;
        if sent.is_ok_and(|status| status.success()) {
            if let Ok(result) = timeout(Duration::from_secs(12), child.wait()).await {
                result?;
                return Ok(());
            }
        }
    }
    child
        .kill()
        .await
        .with_context(|| format!("stop owned {service} child"))
}

async fn dependencies_ready(config: &Config, broker: Option<&Broker>) -> bool {
    if !healthy(config).await {
        return false;
    }
    match broker {
        Some(broker) => timeout(
            Duration::from_secs(1),
            TcpStream::connect(broker.config.probe_addr()),
        )
        .await
        .is_ok_and(|result| result.is_ok()),
        None => true,
    }
}

async fn healthy(config: &Config) -> bool {
    async fn request(bind: SocketAddrV4) -> io::Result<bool> {
        let mut stream = TcpStream::connect(bind).await?;
        stream
            .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await?;
        let mut line = Vec::new();
        BufReader::new(stream.take(512))
            .read_until(b'\n', &mut line)
            .await?;
        Ok(line.starts_with(b"HTTP/1.1 200 ") || line.starts_with(b"HTTP/1.0 200 "))
    }
    timeout(Duration::from_secs(1), request(config.probe_addr()))
        .await
        .is_ok_and(|result| result.unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use fleetworks_workspace::Workspace;

    use super::{Control, State};

    #[test]
    fn lock_and_generation_fence_supervisor_control() {
        let temporary = tempfile::tempdir().unwrap();
        let workspace = Workspace::resolve(Some(temporary.path())).unwrap();
        workspace.initialize().unwrap();
        let control = Control::open(&workspace).unwrap();
        let lock = control.lock().unwrap().unwrap();
        assert!(control.lock().unwrap().is_none());
        let state = State {
            generation: "generation-active".into(),
            identity: "fixture".into(),
            ready: true,
        };
        control.save(&state).unwrap();
        assert!(control.state().unwrap().unwrap().ready);
        control.request_stop("generation-stale").unwrap();
        assert!(!control.stop_requested(&state.generation).unwrap());
        control.request_stop(&state.generation).unwrap();
        assert!(control.stop_requested(&state.generation).unwrap());
        drop(lock);
        assert!(control.lock().unwrap().is_some());
    }
}
