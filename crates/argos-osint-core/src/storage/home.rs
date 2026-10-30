//! Per-home storage layout, mode config, credentials and the embedded
//! single-owner lock (P4; design.md "Server lifecycle").
//!
//! ```text
//! storage/config.json       {mode, port?}                            0600
//! storage/surreal/          RocksDB data directory
//! storage/server.json       {pid, port, version, instance_id, started_at} 0600
//! storage/server.lock       advisory lock while starting/stopping
//! storage/embedded.lock     advisory lock held by the embedded owner
//! storage/credentials.json  root secret                              0600
//! storage/clients/<pid>     one lease file per live client
//! bin/surreal-3.3.4         installed server binary
//! ```
//!
//! Locks are OS advisory locks (`File::try_lock`, `flock` on Unix), released
//! by the kernel when the holder dies. The pid written inside is for the
//! error text only and never decides ownership. Every JSON file is written
//! atomically (temp file, fsync, rename).

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// Pinned server version.
pub const SERVER_VERSION: &str = "3.3.4";

/// Paths under one Argos home.
#[derive(Debug, Clone)]
pub struct Layout {
    pub home: PathBuf,
}

impl Layout {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into() }
    }
    pub fn storage(&self) -> PathBuf {
        self.home.join("storage")
    }
    pub fn data(&self) -> PathBuf {
        self.storage().join("surreal")
    }
    pub fn config(&self) -> PathBuf {
        self.storage().join("config.json")
    }
    pub fn server_json(&self) -> PathBuf {
        self.storage().join("server.json")
    }
    pub fn server_lock(&self) -> PathBuf {
        self.storage().join("server.lock")
    }
    pub fn embedded_lock(&self) -> PathBuf {
        self.storage().join("embedded.lock")
    }
    pub fn credentials(&self) -> PathBuf {
        self.storage().join("credentials.json")
    }
    pub fn clients(&self) -> PathBuf {
        self.storage().join("clients")
    }
    pub fn server_log(&self) -> PathBuf {
        self.storage().join("server.log")
    }
    pub fn server_bin(&self) -> PathBuf {
        self.home
            .join("bin")
            .join(format!("surreal-{SERVER_VERSION}"))
    }
}

/// Storage mode recorded by `argos storage init`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Server,
    Embedded,
}

impl std::str::FromStr for Mode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "server" => Ok(Mode::Server),
            "embedded" => Ok(Mode::Embedded),
            other => Err(format!(
                "storage mode must be server or embedded, not {other:?}"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageConfig {
    pub mode: Mode,
    /// Fixed server port (`storage.port`); None picks one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

/// Root credentials for the per-home server. Debug never prints the secret.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Write `bytes` to `path` atomically: temp file in the same directory,
/// fsync, rename, then fsync the directory. Mode 0600 on Unix.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("path has no parent"))?;
    fs::create_dir_all(dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    #[cfg(unix)]
    File::open(dir)?.sync_all()?;
    Ok(())
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

/// Read a JSON file; a missing file is `None`.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| io::Error::other(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Environment override for the storage mode (`server` or `embedded`).
pub const MODE_ENV: &str = "ARGOS_STORAGE_MODE";

/// The storage config this process runs with: `ARGOS_STORAGE_MODE` first,
/// then `storage/config.json` (written by `argos storage init`), and
/// otherwise the embedded RocksDB store under `<home>/storage/surreal` when
/// this build has the engine (the default build does), with no network.
/// Server mode is only ever chosen explicitly. `None` only in a build
/// without the embedded engine and with nothing configured.
pub fn load_config(layout: &Layout) -> io::Result<Option<StorageConfig>> {
    if let Some(mode) = std::env::var_os(MODE_ENV) {
        let mode = mode
            .to_str()
            .unwrap_or_default()
            .parse::<Mode>()
            .map_err(|e| io::Error::other(format!("{MODE_ENV}: {e}")))?;
        let port = load_config_file(layout)?.and_then(|c| c.port);
        return Ok(Some(StorageConfig { mode, port }));
    }
    if let Some(config) = load_config_file(layout)? {
        return Ok(Some(config));
    }
    Ok(cfg!(feature = "embedded").then_some(StorageConfig {
        mode: Mode::Embedded,
        port: None,
    }))
}

/// Only what `argos storage init` recorded in `storage/config.json`.
pub fn load_config_file(layout: &Layout) -> io::Result<Option<StorageConfig>> {
    read_json(&layout.config())
}

pub fn save_config(layout: &Layout, config: &StorageConfig) -> io::Result<()> {
    write_json(&layout.config(), config)
}

/// Load the server credentials, creating a random root secret on first use.
pub fn load_or_create_credentials(layout: &Layout) -> io::Result<Credentials> {
    if let Some(c) = read_json(&layout.credentials())? {
        return Ok(c);
    }
    let creds = Credentials {
        username: "root".into(),
        password: random_secret()?,
    };
    write_json(&layout.credentials(), &creds)?;
    Ok(creds)
}

/// 32 random bytes as hex.
pub fn random_secret() -> io::Result<String> {
    let mut buf = [0u8; 32];
    File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// An exclusively held advisory lock; released on drop or process death.
#[derive(Debug)]
pub struct LockGuard {
    _file: File,
    pub path: PathBuf,
}

/// Why a lock could not be taken.
#[derive(Debug)]
pub enum LockError {
    /// Another live process holds it; `pid` is what it wrote (informational).
    Held {
        pid: Option<u32>,
    },
    Io(io::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Held { pid: Some(pid) } => write!(f, "held by PID {pid}"),
            LockError::Held { pid: None } => write!(f, "held by another process"),
            LockError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for LockError {}

/// Take `path` exclusively without waiting, then record our pid in it.
pub fn try_lock(path: &Path) -> Result<LockGuard, LockError> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(LockError::Io)?;
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(LockError::Io)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(fs::TryLockError::WouldBlock) => {
            let mut text = String::new();
            let _ = file.read_to_string(&mut text);
            return Err(LockError::Held {
                pid: text.trim().parse().ok(),
            });
        }
        Err(fs::TryLockError::Error(e)) => return Err(LockError::Io(e)),
    }
    file.set_len(0).map_err(LockError::Io)?;
    writeln!(file, "{}", std::process::id()).map_err(LockError::Io)?;
    file.sync_all().map_err(LockError::Io)?;
    Ok(LockGuard {
        _file: file,
        path: path.to_path_buf(),
    })
}

/// Take the embedded owner lock, failing fast with the documented message.
pub fn lock_embedded(layout: &Layout) -> io::Result<LockGuard> {
    match try_lock(&layout.embedded_lock()) {
        Ok(g) => Ok(g),
        Err(LockError::Held { pid }) => Err(io::Error::other(format!(
            "Argos home in use by PID {}; use server mode for concurrent CLI and TUI",
            pid.map_or_else(|| "?".to_string(), |p| p.to_string())
        ))),
        Err(LockError::Io(e)) => Err(e),
    }
}

/// Open this home's embedded store: owner lock first, then the schema
/// bootstrapping [`super::open::open`]. The guard must outlive the client.
#[cfg(feature = "embedded")]
pub async fn open_embedded(
    layout: &Layout,
    ns: &str,
    database: &str,
) -> anyhow::Result<(super::open::Opened, LockGuard)> {
    let guard = lock_embedded(layout)?;
    let opened = super::open::open(
        &super::open::Target::Embedded {
            path: layout.data(),
        },
        ns,
        database,
    )
    .await
    .map_err(|e| anyhow::anyhow!("open embedded store: {e}"))?;
    Ok((opened, guard))
}

/// What keeps a home's store open: a server lease or the embedded lock.
#[derive(Debug)]
pub enum Hold {
    Server(super::server::Lease),
    Embedded(LockGuard),
}

/// An open store for one Argos home. Drop it to release the hold.
pub struct HomeStore {
    pub opened: super::open::Opened,
    pub mode: Mode,
    pub hold: Hold,
}

/// The entry point for consumers: open this home's store in its configured
/// mode (starting or adopting the server, or taking the embedded lock),
/// always through the schema-bootstrapping [`super::open::open`]. An
/// uninitialized home is an error; it never falls back to a legacy store.
pub async fn open_home(layout: &Layout) -> anyhow::Result<HomeStore> {
    let config = load_config(layout)?.ok_or_else(|| {
        anyhow::anyhow!(
            "storage is not initialized; run `argos storage init --mode server|embedded`"
        )
    })?;
    match config.mode {
        Mode::Server => {
            let (opened, lease, _) = super::server::connect_server(layout).await?;
            Ok(HomeStore {
                opened,
                mode: Mode::Server,
                hold: Hold::Server(lease),
            })
        }
        Mode::Embedded => {
            #[cfg(feature = "embedded")]
            {
                let (opened, guard) =
                    open_embedded(layout, super::server::NS, super::server::DB).await?;
                Ok(HomeStore {
                    opened,
                    mode: Mode::Embedded,
                    hold: Hold::Embedded(guard),
                })
            }
            #[cfg(not(feature = "embedded"))]
            anyhow::bail!("this build has no embedded engine (`embedded` feature)")
        }
    }
}
