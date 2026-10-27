//! Per-home SurrealDB server lifecycle (P4; design.md "Server lifecycle").
//!
//! Discovery takes `server.lock`, reads `server.json`, and adopts the server
//! only if the pid is alive, the port accepts on 127.0.0.1, root sign-in
//! works, the reported version is 3.3.4 and `storage_meta:identity` in the
//! `argos` database holds the recorded `instance_id`. Anything else is stale:
//! `server.json` is removed and a fresh server is started. A port answering
//! with the wrong identity is never adopted. Starting happens under
//! `server.lock`, so two sessions cannot both start one. Each client holds a
//! `clients/<pid>` lease; the last client to leave stops the server.
//! Credentials go to the child through `SURREAL_USER`/`SURREAL_PASS`, never
//! argv, and are never logged.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tokio::net::TcpStream;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use surrealdb::engine::any::{connect, Any};
use surrealdb::opt::auth::Root;
use surrealdb::Surreal;

use super::home::{self, Credentials, Layout, LockError, LockGuard, SERVER_VERSION};
use super::open::{self, Opened, Target};

/// Namespace and database of the Argos store.
pub const NS: &str = "argos";
pub const DB: &str = "argos";

const START_TIMEOUT: Duration = Duration::from_secs(30);
const LOCK_TIMEOUT: Duration = Duration::from_secs(35);
const PORT_RETRIES: u32 = 5;

/// Server tuning (the audited memory profile).
pub const PROFILE: &[(&str, &str)] = &[
    ("SURREAL_ROCKSDB_BLOCK_CACHE_SIZE", "67108864"),
    ("SURREAL_DISKANN_CACHE_SIZE", "33554432"),
    ("SURREAL_GRAPH_RESOLVE_CACHE_SIZE", "8388608"),
    ("SURREAL_ROCKSDB_WRITE_BUFFER_SIZE", "16777216"),
    ("SURREAL_ROCKSDB_MAX_WRITE_BUFFER_NUMBER", "2"),
    ("SURREAL_ROCKSDB_MIN_WRITE_BUFFER_NUMBER_TO_MERGE", "2"),
    ("SURREAL_ROCKSDB_JOBS_COUNT", "2"),
    ("SURREAL_ROCKSDB_THREAD_COUNT", "2"),
    ("SURREAL_ROCKSDB_MAX_CONCURRENT_SUBCOMPACTIONS", "1"),
    ("SURREAL_KVS_THREADPOOL_SIZE", "4"),
    ("SURREAL_SCAN_BATCH_BYTES", "524288"),
    ("SURREAL_MEMORY_THRESHOLD", "402653184"),
    ("SURREAL_DATASTORE_VERSIONED", "false"),
    ("SURREAL_DATASTORE_SYNC_DATA", "every"),
];

/// `storage/server.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    pub pid: u32,
    pub port: u16,
    pub version: String,
    pub instance_id: String,
    pub started_at: String,
}

/// Server binary: `ARGOS_SURREAL_BIN` if set, else `bin/surreal-3.3.4`.
pub fn resolve_binary(layout: &Layout) -> Result<PathBuf> {
    let bin = std::env::var_os("ARGOS_SURREAL_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| layout.server_bin());
    if !bin.is_file() {
        bail!(
            "SurrealDB {SERVER_VERSION} server binary not found at {}; run `argos storage install`",
            bin.display()
        );
    }
    Ok(bin)
}

/// Is `pid` a live process? (Informational pids only; never ownership.)
pub fn pid_alive(pid: u32) -> bool {
    // `ps -p` also sees processes of other users (`kill -0` reports EPERM).
    Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "pid="])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Take `server.lock`, waiting up to a bound for the current holder.
pub async fn lock_server(layout: &Layout) -> Result<LockGuard> {
    let deadline = Instant::now() + LOCK_TIMEOUT;
    loop {
        match home::try_lock(&layout.server_lock()) {
            Ok(g) => return Ok(g),
            Err(LockError::Held { pid }) if Instant::now() < deadline => {
                let _ = pid;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(e) => return Err(anyhow!("storage server lock: {e}")),
        }
    }
}

async fn signed_in(port: u16, creds: &Credentials) -> Result<Surreal<Any>> {
    let db = connect(format!("ws://127.0.0.1:{port}")).await?;
    db.signin(Root {
        username: creds.username.clone(),
        password: creds.password.clone(),
    })
    .await?;
    Ok(db)
}

/// Check one recorded server; `Ok(true)` only if every probe passes.
pub async fn probe(info: &ServerInfo, creds: &Credentials) -> bool {
    if !pid_alive(info.pid) || TcpStream::connect(("127.0.0.1", info.port)).await.is_err() {
        return false;
    }
    let check = async {
        let db = signed_in(info.port, creds).await?;
        let version = db.version().await?.to_string();
        if !version.contains(SERVER_VERSION) {
            bail!("server version {version}");
        }
        db.use_ns(NS).use_db(DB).await?;
        let mut r = db
            .query("SELECT VALUE instance_id FROM storage_meta WHERE id = storage_meta:identity")
            .await?
            .check()?;
        let ids: Vec<Option<String>> = r.take(0usize)?;
        Ok::<bool, anyhow::Error>(
            ids.first().cloned().flatten().as_deref() == Some(&info.instance_id),
        )
    };
    matches!(
        tokio::time::timeout(Duration::from_secs(5), check).await,
        Ok(Ok(true))
    )
}

/// Live server for this home, or `None` (a stale `server.json` is removed).
/// The caller must hold `server.lock`.
pub async fn discover_locked(layout: &Layout, creds: &Credentials) -> Result<Option<ServerInfo>> {
    let Some(info) = home::read_json::<ServerInfo>(&layout.server_json())? else {
        return Ok(None);
    };
    if probe(&info, creds).await {
        return Ok(Some(info));
    }
    match std::fs::remove_file(layout.server_json()) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(None)
}

/// [`discover_locked`] under `server.lock`.
pub async fn discover(layout: &Layout) -> Result<Option<ServerInfo>> {
    let creds = home::load_or_create_credentials(layout)?;
    let _lock = lock_server(layout).await?;
    discover_locked(layout, &creds).await
}

fn free_port() -> Result<u16> {
    Ok(TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port())
}

async fn log_text(layout: &Layout) -> String {
    tokio::fs::read_to_string(layout.server_log())
        .await
        .unwrap_or_default()
}

/// Spawn the server on `port` and wait until it listens. `Ok(None)`: the
/// port was taken (retry with another).
async fn spawn_on(
    layout: &Layout,
    bin: &Path,
    port: u16,
    creds: &Credentials,
) -> Result<Option<u32>> {
    let data = std::path::absolute(layout.data())?;
    std::fs::create_dir_all(&data)?;
    let log = std::fs::File::create(layout.server_log())?;
    let mut command = Command::new(bin);
    command
        .args(["start", "--no-banner", "--log", "info", "--bind"])
        .arg(format!("127.0.0.1:{port}"))
        .arg(format!(
            "rocksdb://{}?versioned=false&sync=every",
            data.display()
        ))
        .env("SURREAL_USER", &creds.username)
        .env("SURREAL_PASS", &creds.password)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    for (k, v) in PROFILE {
        command.env(k, v);
    }
    // Detach from the terminal's process group so it outlives the TUI.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().context("spawn surreal server")?;
    let pid = child.id();
    let ready = format!("Started web server on 127.0.0.1:{port}");
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        let log = log_text(layout).await;
        if let Some(status) = child.try_wait()? {
            if log.contains("Address already in use") {
                return Ok(None);
            }
            bail!(
                "storage server exited ({status}); see {}",
                layout.server_log().display()
            );
        }
        if log.contains(&ready) && TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            // Reap it in the background so it never lingers as a zombie of
            // a long-lived parent; it keeps running after we exit.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return Ok(Some(pid));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "storage server did not become ready in {}s; see {}",
                START_TIMEOUT.as_secs(),
                layout.server_log().display()
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Live server for this home, starting one if none is (under `server.lock`).
pub async fn ensure_started(layout: &Layout) -> Result<ServerInfo> {
    let creds = home::load_or_create_credentials(layout)?;
    let _lock = lock_server(layout).await?;
    if let Some(info) = discover_locked(layout, &creds).await? {
        return Ok(info);
    }
    let bin = resolve_binary(layout)?;
    let configured = home::load_config(layout)?.and_then(|c| c.port);
    let mut port = match configured {
        Some(p) => p,
        None => free_port()?,
    };
    let mut tries = 0;
    let pid = loop {
        if let Some(pid) = spawn_on(layout, &bin, port, &creds).await? {
            break pid;
        }
        tries += 1;
        if configured.is_some() || tries >= PORT_RETRIES {
            bail!("storage server could not bind 127.0.0.1:{port}");
        }
        port = free_port()?;
    };
    let ready = async {
        let db = signed_in(port, &creds).await?;
        db.use_ns(NS).use_db(DB).await?;
        super::bootstrap::ensure_schema(&db)
            .await
            .map_err(|e| anyhow!("{e}"))?;
        let mut r = db
            .query(
                "UPSERT storage_meta:identity SET instance_id = instance_id ?? $new \
                 RETURN VALUE instance_id",
            )
            .bind(("new", home::random_secret()?))
            .await?
            .check()?;
        let ids: Vec<String> = r.take(0usize)?;
        let version = db.version().await?.to_string();
        Ok::<_, anyhow::Error>((ids.into_iter().next().unwrap_or_default(), version))
    };
    let (instance_id, version) = match ready.await {
        Ok(v) => v,
        Err(e) => {
            terminate(pid);
            return Err(e.context("storage server health check"));
        }
    };
    let info = ServerInfo {
        pid,
        port,
        version,
        instance_id,
        started_at: chrono::Utc::now().to_rfc3339(),
    };
    home::write_json(&layout.server_json(), &info)?;
    Ok(info)
}

fn terminate(pid: u32) {
    let _ = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// One live client's lease (`clients/<pid>`), removed on drop.
#[derive(Debug)]
pub struct Lease {
    pub path: PathBuf,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Record this process as a client.
pub fn take_lease(layout: &Layout) -> Result<Lease> {
    let pid = std::process::id();
    let path = layout.clients().join(pid.to_string());
    home::write_json(
        &path,
        &serde_json::json!({ "pid": pid, "since": chrono::Utc::now().to_rfc3339() }),
    )?;
    Ok(Lease { path })
}

/// Pids with a lease file whose process is alive (dead leases are removed).
pub fn live_clients(layout: &Layout) -> Result<Vec<u32>> {
    let mut out = Vec::new();
    let dir = match std::fs::read_dir(layout.clients()) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e.into()),
    };
    for entry in dir {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        if pid_alive(pid) {
            out.push(pid);
        } else {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    out.sort_unstable();
    Ok(out)
}

/// Start (or adopt) the server, take a lease, and open the store through the
/// schema-bootstrapping path.
pub async fn connect_server(layout: &Layout) -> Result<(Opened, Lease, ServerInfo)> {
    let info = ensure_started(layout).await?;
    let lease = take_lease(layout)?;
    let creds = home::load_or_create_credentials(layout)?;
    let opened = open::open(
        &Target::Server {
            port: info.port,
            username: creds.username,
            password: creds.password,
        },
        NS,
        DB,
    )
    .await
    .map_err(|e| anyhow!("open storage server: {e}"))?;
    Ok((opened, lease, info))
}

/// Stop the server. Unless `force`, only when no live client other than
/// this process holds a lease. Returns whether a server was stopped.
pub async fn stop(layout: &Layout, force: bool) -> Result<bool> {
    let creds = home::load_or_create_credentials(layout)?;
    let _lock = lock_server(layout).await?;
    let me = std::process::id();
    if !force && live_clients(layout)?.iter().any(|&p| p != me) {
        return Ok(false);
    }
    let Some(info) = discover_locked(layout, &creds).await? else {
        return Ok(false);
    };
    terminate(info.pid);
    let deadline = Instant::now() + Duration::from_secs(20);
    while pid_alive(info.pid) {
        if Instant::now() >= deadline {
            bail!("storage server PID {} did not stop", info.pid);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    std::fs::remove_file(layout.server_json()).ok();
    Ok(true)
}

/// Drop `lease` and stop the server if it was the last client.
pub async fn release(layout: &Layout, lease: Lease) -> Result<bool> {
    drop(lease);
    stop(layout, false).await
}
