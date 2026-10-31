//! Terminal ownership, shared-process API and default-browser presentation.
use super::traffic::{Recorder, Sink};
use super::{OwnedChild, Result};
use anyhow::Context;
use argos_osint_core::events::Severity;
use axum::{
    extract::{MatchedPath, Request},
    middleware,
    middleware::Next,
    response::Response,
    Router,
};
use std::{
    io::IsTerminal,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{net::TcpListener, process::Command, sync::oneshot, task::JoinHandle};

pub(super) fn require_terminal() -> Result<()> {
    anyhow::ensure!(std::io::stdin().is_terminal() && std::io::stdout().is_terminal(), "--terminal tui requires an interactive terminal; use --terminal logs for redirected output");
    Ok(())
}

pub(super) struct InlineApi {
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<Result<()>>,
    _workers: Vec<argos_osint_core::scheduler::WorkerPool>,
    _beat: argos_osint_core::job_registry::ProcessBeat,
    recorder: Recorder,
}
impl InlineApi {
    pub async fn start(listener: TcpListener, web_port: u16) -> Result<Self> {
        Self::start_with_origins(
            listener,
            vec![
                format!("http://localhost:{web_port}"),
                format!("http://127.0.0.1:{web_port}"),
            ],
        )
        .await
    }

    pub async fn start_with_origins(listener: TcpListener, origins: Vec<String>) -> Result<Self> {
        let port = listener.local_addr()?.port();
        let repo = argos_osint_core::storage::runtime::repository().await?;
        argos_osint_core::provider::SettingsFile::load()?;
        let recorder = Recorder::start("api_server", "Rust API server", None).await?;
        let state = argos_osint_api::ApiState {
            repo,
            port,
            instance_id: format!(
                "{}-{}",
                std::process::id(),
                argos_osint_core::storage::home::random_secret()?
            ),
            origins: Arc::new(
                origins
                    .into_iter()
                    .map(|s| {
                        argos_osint_api::loopback_url(&s).map(|u| u.origin().ascii_serialization())
                    })
                    .collect::<Result<_>>()?,
            ),
        };
        let (services_tx, services_rx) = oneshot::channel();
        std::thread::Builder::new()
            .name("argos-stack-api-init".into())
            .spawn(move || {
                let outcome = argos_osint_core::job_registry::start_process_beat(
                    &argos_osint_core::paths::db_path(),
                )
                .map(|beat| {
                    (
                        argos_osint_core::scheduler::WorkerPool::spawn_default(
                            argos_osint_core::paths::db_path(),
                        ),
                        beat,
                    )
                });
                let _ = services_tx.send(outcome);
            })?;
        let (workers, beat) = services_rx.await??;
        let (shutdown, stopped) = oneshot::channel();
        let router = traffic_layer(argos_osint_api::router(state), recorder.sink());
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .map_err(Into::into)
        });
        Ok(Self {
            shutdown: Some(shutdown),
            task,
            _workers: workers,
            _beat: beat,
            recorder,
        })
    }
    pub fn finished(&self) -> bool {
        self.task.is_finished()
    }
    pub async fn stop(&mut self) {
        let failed = self.task.is_finished();
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if tokio::time::timeout(Duration::from_secs(5), &mut self.task)
            .await
            .is_err()
        {
            self.task.abort();
            let _ = (&mut self.task).await;
        }
        self.recorder.finish(failed).await;
    }
}

fn traffic_layer(router: Router, sink: Sink) -> Router {
    router.layer(middleware::from_fn(move |request: Request, next: Next| {
        let sink = sink.clone();
        async move {
            let method = request.method().clone();
            let path = request
                .extensions()
                .get::<MatchedPath>()
                .map(|route| route.as_str())
                .unwrap_or("[unmatched route]")
                .to_string();
            let started = std::time::Instant::now();
            let response: Response = next.run(request).await;
            let status = response.status();
            let severity = if status.is_server_error() {
                Severity::Error
            } else if status.is_client_error() {
                Severity::Warn
            } else {
                Severity::Info
            };
            sink.event(
                "api.request",
                format!(
                    "{method} {path} · {} · {} ms",
                    status.as_u16(),
                    started.elapsed().as_millis()
                ),
                severity,
            );
            response
        }
    }))
}

impl Drop for InlineApi {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(super) struct TerminalUi {
    shutdown: Arc<AtomicBool>,
    pub result: oneshot::Receiver<Result<()>>,
    pub completed: bool,
}
impl TerminalUi {
    pub fn start() -> Result<Self> {
        let shutdown = Arc::new(AtomicBool::new(false));
        let stop = shutdown.clone();
        let (sender, result) = oneshot::channel();
        // Initialize the heartbeat outside Tokio, then enter the existing
        // runtime so normal TUI actions (images, Atlas, Recon) can spawn tasks.
        let runtime = tokio::runtime::Handle::current();
        std::thread::Builder::new()
            .name("argos-stack-tui".into())
            .spawn(move || {
                let outcome = (|| {
                    let _beat = argos_osint_core::job_registry::start_process_beat(
                        &argos_osint_core::paths::db_path(),
                    )?;
                    let app = crate::tui::App::boot()?;
                    let _entered = runtime.enter();
                    crate::tui::run_with_shutdown(app, stop, false)
                })();
                let _ = sender.send(outcome);
            })?;
        Ok(Self {
            shutdown,
            result,
            completed: false,
        })
    }
    pub async fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if !self.completed {
            if tokio::time::timeout(Duration::from_secs(20), &mut self.result)
                .await
                .is_err()
            {
                let _ = crossterm::terminal::disable_raw_mode();
                let _ = crossterm::execute!(
                    std::io::stdout(),
                    crossterm::event::DisableMouseCapture,
                    crossterm::terminal::LeaveAlternateScreen,
                    crossterm::cursor::Show
                );
            }
            self.completed = true;
        }
    }
}
impl Drop for TerminalUi {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

pub(super) fn service_log() -> Result<(PathBuf, std::fs::File)> {
    let path = argos_osint_core::paths::ensure_home()?.join("stack-services.log");
    let mut options = std::fs::OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&path)?;
    Ok((path, file))
}

pub(super) async fn wait_for_web(port: u16, child: &mut OwnedChild) -> Result<()> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(2))
        .build()?;
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            anyhow::ensure!(
                child.try_wait()?.is_none(),
                "frontend exited before HTTP readiness"
            );
            if client
                .get(format!("http://localhost:{port}/"))
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .context("web did not reach HTTP readiness")?
}

fn browser_command(url: &str) -> Command {
    #[cfg(target_os = "macos")]
    {
        let mut cmd = Command::new("open");
        cmd.arg(url);
        cmd
    }
    #[cfg(target_os = "windows")]
    {
        let mut cmd = Command::new("rundll32.exe");
        cmd.args(["url.dll,FileProtocolHandler", url]);
        cmd
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let mut cmd = Command::new("xdg-open");
        cmd.arg(url);
        cmd
    }
}

pub(super) async fn open_browser(url: &str, log: Option<&Path>) {
    // Use the OS URL handler (existing default-browser session/tab preferences),
    // never a shell, browser executable override or a forced new window.
    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        browser_command(url).kill_on_drop(true).output(),
    )
    .await;
    if !matches!(outcome, Ok(Ok(ref output)) if output.status.success()) {
        let message = format!("Could not open the default browser. Open {url} manually.");
        if let Some(path) = log {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(path) {
                let _ = writeln!(file, "{message}");
            }
        } else {
            eprintln!("{message}");
        }
    }
}
