//! Foreground local stack supervisor. @tag:intel-brief
//! State files locate a live capability channel; they never authorize PID kills.
mod node;
mod presentation;
mod traffic;

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    process::{Child, Command},
};

#[derive(Debug, Subcommand)]
pub enum StackCommand {
    Up(Up),
    Status,
    Down(Platform),
    Doctor {
        #[arg(long, value_enum)]
        target: Target,
    },
    Build(Platform),
    /// Internal child-process entry point; shares this binary's API version/features.
    #[command(hide = true)]
    ApiHost {
        #[arg(long)]
        port: u16,
        #[arg(long, value_delimiter = ',')]
        origins: Vec<String>,
    },
}
#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    Tui,
    Server,
    Web,
    Macos,
    Android,
    Ios,
    Windows,
}
#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
pub struct Platform {
    #[arg(long)]
    pub tui: bool,
    #[arg(long)]
    pub server: bool,
    #[arg(long)]
    pub web: bool,
    #[arg(long)]
    pub macos: bool,
    #[arg(long)]
    pub android: bool,
    #[arg(long)]
    pub ios: bool,
    #[arg(long)]
    pub windows: bool,
}
impl Platform {
    pub fn target(&self) -> Target {
        if self.tui {
            Target::Tui
        } else if self.server {
            Target::Server
        } else if self.web {
            Target::Web
        } else if self.macos {
            Target::Macos
        } else if self.android {
            Target::Android
        } else if self.ios {
            Target::Ios
        } else {
            Target::Windows
        }
    }
}
#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq, Serialize, Deserialize)]
pub enum Profile {
    Dev,
    Installed,
}
#[derive(Clone, Copy, Debug, Default, ValueEnum, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalDisplay {
    Tui,
    #[default]
    Logs,
}
#[derive(Debug, Args)]
pub struct Up {
    #[command(flatten)]
    pub platform: Platform,
    #[arg(long)]
    pub port: Option<u16>,
    #[arg(long, conflicts_with = "api_url")]
    pub api_port: Option<u16>,
    #[arg(long)]
    pub api_url: Option<String>,
    #[arg(long, value_enum, default_value = "dev")]
    pub profile: Profile,
    /// What occupies the terminal while services run.
    #[arg(long, value_enum, default_value = "logs")]
    pub terminal: TerminalDisplay,
    /// Do not open the web URL in the default browser.
    #[arg(long)]
    pub no_open: bool,
    #[arg(long, default_value = "auto")]
    pub arch: String,
    #[arg(long, default_value = "auto")]
    pub device: String,
}
impl Up {
    pub fn validate(&self) -> Result<Target> {
        let target = self.platform.target();
        supported(target)?;
        anyhow::ensure!(
            self.port.is_none() || target == Target::Web,
            "--port applies only to web"
        );
        anyhow::ensure!(
            self.arch == "auto" && self.device == "auto",
            "--arch/--device require a later platform phase"
        );
        anyhow::ensure!(
            target != Target::Tui || (self.api_url.is_none() && self.api_port.is_none()),
            "TUI does not use an API"
        );
        anyhow::ensure!(
            self.port != Some(0) && self.api_port != Some(0),
            "ports must be 1..65535"
        );
        anyhow::ensure!(
            target != Target::Web || self.api_port != Some(self.port.unwrap_or(3000)),
            "web/API port collision"
        );
        if let Some(url) = &self.api_url {
            loopback_url(url)?;
        }
        Ok(target)
    }
}
fn supported(target: Target) -> Result<()> {
    anyhow::ensure!(
        matches!(target, Target::Tui | Target::Server | Target::Web),
        "target not yet implemented"
    );
    Ok(())
}
pub fn loopback_url(raw: &str) -> Result<url::Url> {
    let u = url::Url::parse(raw)?;
    anyhow::ensure!(
        u.scheme() == "http"
            && matches!(u.host_str(), Some("127.0.0.1" | "localhost"))
            && u.username().is_empty()
            && u.password().is_none()
            && u.path() == "/"
            && u.query().is_none()
            && u.fragment().is_none(),
        "API URL must be a loopback HTTP origin"
    );
    Ok(u)
}
#[derive(Debug, Serialize, Deserialize)]
struct Managed {
    target: Target,
    profile: Profile,
    #[serde(default)]
    terminal: TerminalDisplay,
    web_port: Option<u16>,
    instance: String,
    token: String,
    control_port: u16,
    api_url: String,
    api_instance: String,
    adopted: bool,
    supervisor_pid: u32,
    api_pid: Option<u32>,
    frontend_pid: Option<u32>,
}
fn state_path() -> Result<PathBuf> {
    Ok(argos_osint_core::paths::ensure_home()?.join("stack.json"))
}
fn workspace() -> Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    for p in cwd.ancestors() {
        if p.join("apps/web-desktop/package.json").is_file() && p.join("Cargo.toml").is_file() {
            return Ok(p.into());
        }
    }
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    anyhow::ensure!(
        source.join("apps/web-desktop/package.json").is_file(),
        "source checkout unavailable; run from the Argos workspace"
    );
    Ok(source.canonicalize()?)
}
fn api_executable() -> Result<PathBuf> {
    // Cargo run builds only argos. The API router is already linked into this
    // executable, so use an internal child mode instead of requiring a sibling.
    std::env::current_exe().context("locate argos for the API child process")
}

/// True when this home (or `ARGOS_STORAGE_MODE`) selected a mode. The default
/// embedded build still reports an implicit config from `load_config`; the
/// launcher requires an explicit choice so a new home cannot start storage.
fn storage_selected(layout: &argos_osint_core::storage::home::Layout) -> Result<bool> {
    Ok(
        std::env::var_os(argos_osint_core::storage::home::MODE_ENV).is_some()
            || argos_osint_core::storage::home::load_config_file(layout)?.is_some(),
    )
}

async fn version(tool: &str) -> Result<String> {
    let out = Command::new(tool)
        .arg("--version")
        .output()
        .await
        .with_context(|| format!("{tool} unavailable; install the documented toolchain"))?;
    anyhow::ensure!(out.status.success(), "{tool} --version failed");
    Ok(String::from_utf8_lossy(&out.stdout).trim().into())
}
async fn web_preflight(root: &Path, environment: &node::NodeEnvironment) -> Result<()> {
    let node = environment.version("node").await?;
    anyhow::ensure!(
        node.starts_with("v24."),
        "Node 24 LTS required; found {node}"
    );
    let pnpm = environment.version("pnpm").await?;
    anyhow::ensure!(pnpm == "12.15.0", "pnpm 12.15.0 required; found {pnpm}");
    anyhow::ensure!(
        root.join("apps/web-desktop/node_modules/next/dist/bin/next")
            .is_file(),
        "web dependencies missing; run pnpm install --frozen-lockfile"
    );
    Ok(())
}
struct OwnedChild {
    child: Child,
    group: u32,
}
impl OwnedChild {
    fn id(&self) -> Option<u32> {
        Some(self.group)
    }
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }
}
fn owned(command: &mut Command) -> Result<OwnedChild> {
    command.stdin(Stdio::null()).kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let child = command.spawn()?;
    let group = child.id().context("owned child PID")?;
    Ok(OwnedChild { child, group })
}
async fn stop(owned: &mut OwnedChild) {
    // Keep the owned process group even if try_wait reaped its leader, so
    // descendants cannot survive a failed frontend/API parent.
    #[cfg(unix)]
    unsafe {
        libc::kill(-(owned.group as i32), libc::SIGTERM);
    }
    #[cfg(not(unix))]
    {
        let _ = owned.child.start_kill();
    }
    if tokio::time::timeout(Duration::from_secs(8), owned.child.wait())
        .await
        .is_err()
    {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(owned.group as i32), libc::SIGKILL);
        }
        let _ = owned.child.kill().await;
    }
}

pub(crate) fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let res = unsafe { libc::kill(pid as i32, 0) };
        if res == 0 {
            return true;
        }
        let err = std::io::Error::last_os_error();
        err.raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

pub(crate) async fn reap_orphaned_pid(pid: u32) {
    #[cfg(unix)]
    {
        if pid_alive(pid) {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGTERM);
                libc::kill(pid as i32, libc::SIGTERM);
            }
            for _ in 0..20 {
                if !pid_alive(pid) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if pid_alive(pid) {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                    libc::kill(pid as i32, libc::SIGKILL);
                }
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
    }
}

async fn recover_abandoned_state(path: &Path, state: &Managed) -> Result<()> {
    if let Some(pid) = state.api_pid {
        reap_orphaned_pid(pid).await;
    }
    if let Some(pid) = state.frontend_pid {
        reap_orphaned_pid(pid).await;
    }
    let _ = std::fs::remove_file(path);
    Ok(())
}
async fn health(base: &str) -> Result<String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(600))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let h: serde_json::Value = client
        .get(format!(
            "{}/api/v1/health/ready",
            base.trim_end_matches('/')
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    anyhow::ensure!(
        h["service"] == "argos-intel"
            && h["api_version"] == 1
            && h["ready"] == true
            && h["storage"] == true,
        "API/storage is not ready or incompatible"
    );
    Ok(h["instance_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .context("API instance missing")?
        .into())
}
async fn control(state: &Managed, verb: &str) -> Result<String> {
    let mut socket = TcpStream::connect(("127.0.0.1", state.control_port)).await?;
    socket
        .write_all(format!("{} {verb}\n", state.token).as_bytes())
        .await?;
    let mut bytes = Vec::new();
    socket.take(4096).read_to_end(&mut bytes).await?;
    let reply = String::from_utf8(bytes)?;
    anyhow::ensure!(
        reply.starts_with(&state.instance),
        "supervisor identity mismatch"
    );
    Ok(reply)
}
struct StateGuard(PathBuf);
impl Drop for StateGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub async fn dispatch(command: StackCommand) -> Result<()> {
    match command {
        StackCommand::Up(up) => run(up).await,
        StackCommand::ApiHost { port, origins } => {
            anyhow::ensure!(port > 0, "API port must be 1..65535");
            let layout = argos_osint_core::storage::home::Layout::new(
                argos_osint_core::paths::ensure_home()?,
            );
            anyhow::ensure!(
                storage_selected(&layout)?,
                "storage is not initialized; run argos storage init --mode embedded (or server)"
            );
            let listener = TcpListener::bind(("127.0.0.1", port)).await?;
            let mut host = presentation::InlineApi::start_with_origins(listener, origins).await?;
            println!("Argos API http://127.0.0.1:{port}");
            let signal = shutdown_signal();
            tokio::pin!(signal);
            loop {
                tokio::select! {
                    _ = &mut signal => break,
                    _ = tokio::time::sleep(Duration::from_millis(250)) => {
                        anyhow::ensure!(!host.finished(), "API listener exited");
                    }
                }
            }
            host.stop().await;
            Ok(())
        }
        StackCommand::Doctor { target } => {
            supported(target)?;
            println!("Rust: {}", version("rustc").await?);
            if target != Target::Tui {
                println!(
                    "API executable: {} (built-in host mode)",
                    api_executable()?.display()
                );
            }
            if target == Target::Web {
                let root = workspace()?;
                let environment = node::NodeEnvironment::select(&root).await?;
                web_preflight(&root, &environment).await?;
                println!("Node 24 / pnpm 12.15.0 / Next dependencies available");
            }
            println!("Prerequisites available. No tools installed.");
            Ok(())
        }
        StackCommand::Build(p) => {
            let target = p.target();
            supported(target)?;
            anyhow::ensure!(
                target == Target::Web,
                "build currently supports --web; build native Rust executables with Cargo"
            );
            let root = workspace()?;
            let environment = node::NodeEnvironment::select(&root).await?;
            web_preflight(&root, &environment).await?;
            let status = environment
                .command("pnpm")
                .args(["--dir", "apps/web-desktop", "build"])
                .current_dir(root)
                .status()
                .await?;
            anyhow::ensure!(status.success(), "web build failed: {status}");
            Ok(())
        }
        StackCommand::Status => {
            let path = state_path()?;
            if !path.exists() {
                println!("No managed stack");
                return Ok(());
            }
            let bytes = std::fs::read(&path)?;
            let state: Managed = match serde_json::from_slice(&bytes) {
                Ok(s) => s,
                Err(_) => {
                    println!(
                        "Corrupted supervisor state at {}. Run 'argos stack down' to clean up.",
                        path.display()
                    );
                    return Ok(());
                }
            };
            match tokio::time::timeout(Duration::from_secs(2), control(&state, "STATUS")).await {
                Ok(Ok(status)) => {
                    println!("{status}");
                    println!(
                        "API health: {} / adopted: {}",
                        health(&state.api_url).await?,
                        state.adopted
                    );
                    Ok(())
                }
                _ => {
                    if !pid_alive(state.supervisor_pid) {
                        println!(
                            "Abandoned supervisor state at {} (supervisor PID {} exited). Run 'argos stack down' to clean up.",
                            path.display(),
                            state.supervisor_pid
                        );
                        Ok(())
                    } else {
                        bail!("supervisor unresponsive");
                    }
                }
            }
        }
        StackCommand::Down(p) => {
            let target = p.target();
            supported(target)?;
            let path = state_path()?;
            if !path.exists() {
                println!("No managed stack");
                return Ok(());
            }
            let bytes = std::fs::read(&path)?;
            let state: Managed = match serde_json::from_slice(&bytes) {
                Ok(s) => s,
                Err(_) => {
                    let _ = std::fs::remove_file(&path);
                    println!(
                        "Removed corrupted supervisor state file at {}",
                        path.display()
                    );
                    return Ok(());
                }
            };
            if !pid_alive(state.supervisor_pid) {
                recover_abandoned_state(&path, &state).await?;
                println!("Cleaned up abandoned supervisor state and stopped surviving services.");
                return Ok(());
            }
            anyhow::ensure!(
                state.target == target,
                "requested target does not own this stack (active stack: {:?})",
                state.target
            );
            match tokio::time::timeout(Duration::from_secs(2), control(&state, "STOP")).await {
                Ok(Ok(reply)) => {
                    println!("{reply}");
                    for _ in 0..100 {
                        if !path.exists() {
                            return Ok(());
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    bail!("supervisor shutdown still in progress")
                }
                _ => {
                    bail!("supervisor unresponsive");
                }
            }
        }
    }
}
async fn run(up: Up) -> Result<()> {
    let target = up.validate()?;
    let path = state_path()?;
    if path.exists() {
        let bytes = std::fs::read(&path)?;
        let state: Option<Managed> = serde_json::from_slice(&bytes).ok();
        if let Some(state) = state {
            if tokio::time::timeout(Duration::from_secs(2), control(&state, "STATUS"))
                .await
                .is_ok_and(|r| r.is_ok())
            {
                anyhow::ensure!(
                    state.target == target,
                    "another managed stack is running; use stack status/down"
                );
                anyhow::ensure!(
                    state.profile == up.profile
                        && state.terminal == up.terminal
                        && (target != Target::Web
                            || state.web_port == Some(up.port.unwrap_or(3000))),
                    "managed stack configuration differs; stop it before changing configuration"
                );
                if let Some(url) = &up.api_url {
                    anyhow::ensure!(
                        loopback_url(url)?.origin() == loopback_url(&state.api_url)?.origin(),
                        "managed API differs from requested API"
                    );
                }
                if let Some(port) = up.api_port {
                    anyhow::ensure!(
                        loopback_url(&state.api_url)?.port_or_known_default() == Some(port),
                        "managed API port differs"
                    );
                }
                health(&state.api_url).await?;
                println!("Stack already running: {}", state.api_url);
                let port = state.web_port.unwrap_or(3000);
                if target == Target::Web && !up.no_open {
                    presentation::open_browser(
                        &format!("http://localhost:{port}/?api={}", state.api_url),
                        None,
                    )
                    .await;
                }
                return Ok(());
            }
            if !pid_alive(state.supervisor_pid) {
                eprintln!(
                    "argos: recovering from abandoned supervisor state (PID {} exited without cleanup)",
                    state.supervisor_pid
                );
                recover_abandoned_state(&path, &state).await?;
            } else {
                bail!(
                    "stale supervisor state at {}; verify its processes have exited before removing this file",
                    path.display()
                );
            }
        } else {
            let _ = std::fs::remove_file(&path);
            eprintln!(
                "argos: removed corrupted supervisor state file at {}",
                path.display()
            );
        }
    }
    if target == Target::Tui {
        let app = tokio::task::spawn_blocking(crate::tui::App::boot).await??;
        return crate::tui::run(app).await;
    }
    if up.terminal == TerminalDisplay::Tui {
        presentation::require_terminal()?;
        if up.api_url.is_some() {
            let layout = argos_osint_core::storage::home::Layout::new(
                argos_osint_core::paths::ensure_home()?,
            );
            anyhow::ensure!(
                argos_osint_core::storage::home::load_config(&layout)?.is_some_and(|c| c.mode == argos_osint_core::storage::home::Mode::Server),
                "TUI with an adopted API requires local server-mode storage; use --terminal logs for embedded storage, or start an owned API to share its repository"
            );
        }
    }
    let root = if target == Target::Web {
        Some(workspace()?)
    } else {
        None
    };
    let environment = if let Some(root) = &root {
        let environment = node::NodeEnvironment::select(root).await?;
        if up.profile == Profile::Dev {
            web_preflight(root, &environment).await?;
        }
        Some(environment)
    } else {
        None
    };
    if let Some(root) = &root {
        if up.profile == Profile::Installed {
            anyhow::ensure!(
                root.join("apps/web-desktop/out/index.html").is_file(),
                "static bundle missing; run argos stack build --web"
            );
        }
    }
    if up.api_url.is_none() {
        let layout =
            argos_osint_core::storage::home::Layout::new(argos_osint_core::paths::ensure_home()?);
        anyhow::ensure!(
            storage_selected(&layout)?,
            "storage is not initialized; run argos storage init --mode embedded (or your supported server configuration)"
        );
    }
    let executable = if up.api_url.is_none() && up.terminal == TerminalDisplay::Logs {
        Some(api_executable()?)
    } else {
        None
    };
    // Exclusive state reservation prevents concurrent embedded owners during startup.
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&path)?;
    let _guard = StateGuard(path.clone());
    let control_listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let web_port = up.port.unwrap_or(3000);
    let web_reservation = if target == Target::Web {
        Some(
            TcpListener::bind(("127.0.0.1", web_port))
                .await
                .context("web port already in use")?,
        )
    } else {
        None
    };
    let api_port = if let Some(url) = &up.api_url {
        loopback_url(url)?
            .port_or_known_default()
            .context("API port")?
    } else if let Some(port) = up.api_port {
        port
    } else {
        let socket = TcpListener::bind(("127.0.0.1", 0)).await?;
        socket.local_addr()?.port()
    };
    anyhow::ensure!(
        target != Target::Web || api_port != web_port,
        "web/API port collision"
    );
    let api_url = up
        .api_url
        .clone()
        .unwrap_or_else(|| format!("http://127.0.0.1:{api_port}"));
    let api_reservation = if up.api_url.is_none() {
        Some(
            TcpListener::bind(("127.0.0.1", api_port))
                .await
                .context("API port already in use; adopt it explicitly with --api-url")?,
        )
    } else {
        None
    };
    let mut inline_listener = if up.api_url.is_none() && up.terminal == TerminalDisplay::Tui {
        api_reservation
    } else {
        drop(api_reservation);
        None
    };
    let mut inline_api = None;
    let mut api = if let Some(executable) = executable {
        let mut cmd = Command::new(executable);
        cmd.args(["stack", "api-host", "--port"])
            .arg(api_port.to_string())
            .arg("--origins")
            .arg(format!(
                "http://localhost:{web_port},http://127.0.0.1:{web_port}"
            ));
        Some(owned(&mut cmd)?)
    } else {
        None
    };
    let mut frontend = None;
    let mut frontend_traffic: Option<traffic::ServiceTraffic> = None;
    let mut terminal_ui: Option<presentation::TerminalUi> = None;
    let mut service_log = None;
    let operation = async {
        // Include repository/worker startup in the signal-cancellable scope.
        if let Some(listener) = inline_listener.take() {
            inline_api = Some(presentation::InlineApi::start(listener, web_port).await?);
        }
        let mut instance = None;
        for _ in 0..100 {
            if let Some(child) = api.as_mut() {
                anyhow::ensure!(
                    child.try_wait()?.is_none(),
                    "API host exited before readiness"
                );
            }
            if let Ok(id) = health(&api_url).await {
                if let Some(pid) = api.as_ref().and_then(OwnedChild::id) {
                    anyhow::ensure!(
                        id.starts_with(&format!("{pid}-")),
                        "readiness belongs to another API process"
                    );
                }
                instance = Some(id);
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let instance =
            instance.context("API did not reach storage-backed readiness; frontend not started")?;
        if target == Target::Web {
            let origin = loopback_url(&format!("http://localhost:{web_port}"))?
                .origin()
                .ascii_serialization();
            let response = reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .redirect(reqwest::redirect::Policy::none())
                .build()?
                .get(format!(
                    "{}/api/v1/health/ready",
                    api_url.trim_end_matches('/')
                ))
                .header("Origin", &origin)
                .send()
                .await?;
            anyhow::ensure!(
                response.status().is_success()
                    && response
                        .headers()
                        .get("access-control-allow-origin")
                        .is_some_and(|v| v.as_bytes() == origin.as_bytes()),
                "API does not allow web Origin {origin}; configure its --origins before launch"
            );
        }
        drop(web_reservation);
        if let Some(root) = &root {
            let mut cmd = environment
                .as_ref()
                .context("selected Node environment")?
                .command("node");
            cmd.current_dir(root);
            if up.profile == Profile::Dev {
                // Next's development Babel loader resolves next/babel from the
                // process cwd, even when the app directory is an explicit arg.
                cmd.current_dir(root.join("apps/web-desktop"));
                cmd.arg(root.join("apps/web-desktop/node_modules/next/dist/bin/next"))
                    .args(["dev", "--webpack", "-H", "127.0.0.1", "-p"])
                    .arg(web_port.to_string())
                    .arg(root.join("apps/web-desktop"));
            } else {
                cmd.arg(root.join("scripts/serve_web.mjs"))
                    .arg("--port")
                    .arg(web_port.to_string());
            }
            if up.terminal == TerminalDisplay::Tui {
                let (log, file) = presentation::service_log()?;
                println!("Service logs: {}", log.display());
                cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
                service_log = Some(log);
                frontend = Some(owned(&mut cmd)?);
                frontend_traffic = Some(
                    traffic::ServiceTraffic::capture(
                        frontend.as_mut().expect("frontend started"),
                        file,
                        up.profile == Profile::Dev,
                    )
                    .await?,
                );
            } else {
                frontend = Some(owned(&mut cmd)?);
            }
            let url = format!("http://localhost:{web_port}/?api={api_url}");
            println!("Web: {url}");
            presentation::wait_for_web(web_port, frontend.as_mut().unwrap()).await?;
            if !up.no_open {
                presentation::open_browser(&url, service_log.as_deref()).await;
            }
        }
        let token = argos_osint_core::storage::home::random_secret()?;
        let state = Managed {
            target,
            profile: up.profile,
            terminal: up.terminal,
            web_port: (target == Target::Web).then_some(web_port),
            instance: token[..16].into(),
            token,
            control_port: control_listener.local_addr()?.port(),
            api_url: api_url.clone(),
            api_instance: instance,
            adopted: up.api_url.is_some(),
            supervisor_pid: std::process::id(),
            api_pid: api
                .as_ref()
                .and_then(OwnedChild::id)
                .or_else(|| inline_api.as_ref().map(|_| std::process::id())),
            frontend_pid: frontend.as_ref().and_then(OwnedChild::id),
        };
        serde_json::to_writer(file, &state)?;
        println!("API ready: {api_url}; foreground supervisor active (Ctrl-C to stop)");
        if up.terminal == TerminalDisplay::Tui {
            terminal_ui = Some(presentation::TerminalUi::start()?);
        }
        loop {
            tokio::select! {
             _=shutdown_signal()=>break,
             outcome=async { (&mut terminal_ui.as_mut().expect("TUI active").result).await }, if terminal_ui.is_some()=>{
                 terminal_ui.as_mut().expect("TUI active").completed = true;
                 outcome.context("TUI thread stopped without a result")??;
                 break;
             },
             connection=control_listener.accept()=>{
              let (mut socket,_)=connection?;
              let mut bytes=[0u8;256];
              let n=match tokio::time::timeout(Duration::from_secs(1),socket.read(&mut bytes)).await { Ok(Ok(n))=>n, _=>continue };
              let request=String::from_utf8_lossy(&bytes[..n]);
              if request.trim()==format!("{} STATUS",state.token){socket.write_all(format!("{} running {}\n",state.instance,state.api_url).as_bytes()).await?;}
              if request.trim()==format!("{} STOP",state.token){socket.write_all(format!("{} stopping\n",state.instance).as_bytes()).await?;break;}
             },
             _=tokio::time::sleep(Duration::from_millis(250))=>{
              if let Some(host)=inline_api.as_ref(){anyhow::ensure!(!host.finished(),"in-process API host exited");}
              if let Some(child)=api.as_mut(){anyhow::ensure!(child.try_wait()?.is_none(),"API host exited");}
              if let Some(child)=frontend.as_mut(){anyhow::ensure!(child.try_wait()?.is_none(),"frontend exited");}
             }
            }
        }
        Ok(())
    };
    let result = tokio::select! { value = operation => value, _ = shutdown_signal() => Ok(()) };
    if let Some(ui) = terminal_ui.as_mut() {
        ui.stop().await;
    }
    if let Some(host) = inline_api.as_mut() {
        host.stop().await;
    }
    if let Some(child) = frontend.as_mut() {
        stop(child).await;
    }
    if let Some(traffic) = frontend_traffic.as_mut() {
        traffic.finish(result.is_err()).await;
    }
    if let Some(child) = api.as_mut() {
        stop(child).await;
    }
    result
}
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("signal handler");
        tokio::select! {_=tokio::signal::ctrl_c()=>{},_=term.recv()=>{}}
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn readiness_rejects_tcp_only_or_storage_unready() {
        let socket = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = socket.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            for ready in [false, true] {
                let (mut client, _) = socket.accept().await.unwrap();
                let mut bytes = [0; 2048];
                let _ = client.read(&mut bytes).await.unwrap();
                let body = serde_json::json!({"service":"argos-intel","api_version":1,"ready":ready,"storage":ready,"instance_id":"fixture"}).to_string();
                client.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        let base = format!("http://127.0.0.1:{port}");
        assert!(health(&base).await.is_err());
        assert_eq!(health(&base).await.unwrap(), "fixture");
        task.await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn owned_child_is_reaped_on_stop() {
        let mut command = Command::new("sleep");
        command.arg("30");
        let mut child = owned(&mut command).unwrap();
        let pid = child.id().unwrap();
        stop(&mut child).await;
        assert!(child.try_wait().unwrap().is_some());
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
    }

    #[test]
    fn unconfigured_home_is_not_a_storage_selection() {
        std::env::remove_var(argos_osint_core::storage::home::MODE_ENV);
        let dir = tempfile::tempdir().unwrap();
        let layout = argos_osint_core::storage::home::Layout::new(dir.path().to_path_buf());
        assert!(!storage_selected(&layout).unwrap());
        assert!(argos_osint_core::storage::home::load_config(&layout)
            .unwrap()
            .is_some());
    }
}
