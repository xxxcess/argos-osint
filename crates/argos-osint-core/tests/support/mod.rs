//! Shared harness for the SurrealDB integration suites: a pinned 3.3.4 disk
//! server on loopback (or an embedded RocksDB datastore) in a temp directory.
//! See `storage_contracts.rs` for the mode and environment contract.
#![allow(dead_code)]

use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use surrealdb::engine::any::{connect, Any};
use surrealdb::opt::auth::Root;
use surrealdb::Surreal;

pub const PINNED_VERSION: &str = "3.3.4";
pub const DIM: usize = 384;

/// Disk profile from the storage audit appendix (section 4).
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

pub struct Server {
    pub embedded: bool,
    /// Embedded mode: the one open datastore, cloned for every caller.
    pub client: Mutex<Option<Surreal<Any>>>,
    pub bin: PathBuf,
    pub dir: tempfile::TempDir,
    pub port: u16,
    /// Relaunches after losing a port race (see `launch`).
    port_retries: u32,
    pub password: String,
    pub child: Option<Child>,
}

impl Server {
    /// Start a server, or return `None` (skip) when no binary is configured.
    pub fn start() -> Option<Server> {
        match std::env::var("ARGOS_SURREAL_MODE").as_deref() {
            Ok("embedded") => {
                assert!(
                    embedded_compiled(),
                    "ARGOS_SURREAL_MODE=embedded needs `--features embedded`"
                );
                return Some(Server {
                    embedded: true,
                    client: Mutex::new(None),
                    bin: PathBuf::new(),
                    dir: tempfile::tempdir().expect("temp dir"),
                    port: 0,
                    port_retries: 0,
                    password: String::new(),
                    child: None,
                });
            }
            Ok("server") | Err(_) => {}
            Ok(other) => panic!("ARGOS_SURREAL_MODE must be server or embedded, not {other:?}"),
        }
        let required = std::env::var("ARGOS_SURREAL_REQUIRED").as_deref() == Ok("1");
        let bin = match std::env::var_os("ARGOS_SURREAL_BIN").map(PathBuf::from) {
            Some(bin) if bin.is_file() => bin,
            other => {
                let why = match other {
                    Some(bin) => format!("ARGOS_SURREAL_BIN={} is not a file", bin.display()),
                    None => "ARGOS_SURREAL_BIN is not set".to_string(),
                };
                assert!(
                    !required,
                    "storage_contracts: {why}, and ARGOS_SURREAL_REQUIRED=1"
                );
                eprintln!(
                    "storage_contracts: SKIPPED, {why}. Run `python3 scripts/test_surreal.py` \
                     to test against a real SurrealDB {PINNED_VERSION} disk server."
                );
                return None;
            }
        };
        let version = Command::new(&bin)
            .arg("version")
            .output()
            .expect("run surreal version");
        let version = String::from_utf8_lossy(&version.stdout).to_string();
        assert!(
            version.starts_with(PINNED_VERSION),
            "expected SurrealDB {PINNED_VERSION}, got {version:?}"
        );
        let password = format!("argos-test-{}-{}", std::process::id(), nanos());
        let mut server = Server {
            embedded: false,
            client: Mutex::new(None),
            bin,
            dir: tempfile::tempdir().expect("temp dir"),
            port: free_port(),
            port_retries: 0,
            password,
            child: None,
        };
        server.launch();
        Some(server)
    }

    pub fn data_path(&self) -> PathBuf {
        self.dir.path().join("surreal")
    }

    pub fn launch(&mut self) {
        if self.embedded {
            return;
        }
        let url = format!(
            "rocksdb://{}?versioned=false&sync=every",
            self.data_path().display()
        );
        let log = std::fs::File::create(self.dir.path().join("server.log")).expect("log");
        let mut command = Command::new(&self.bin);
        command
            .args(["start", "--no-banner", "--log", "info", "--bind"])
            .arg(format!("127.0.0.1:{}", self.port))
            .arg(url)
            // Credentials go through the environment, never argv.
            .env("SURREAL_USER", "root")
            .env("SURREAL_PASS", &self.password)
            .stdin(Stdio::null())
            .stdout(log.try_clone().expect("log"))
            .stderr(log);
        for (key, value) in PROFILE {
            command.env(key, value);
        }
        let mut child = command.spawn().expect("spawn surreal");
        // Ready means *our* child logged that it is listening. A bare TCP
        // probe is not enough: `free_port` releases its port before the
        // server binds it, so a parallel test can take the same port, our
        // server then exits with "Address already in use", and the probe
        // connects to the other test's server, which goes away when that
        // test ends ("Connection refused" mid-test).
        let ready = format!("Started web server on 127.0.0.1:{}", self.port);
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = child.try_wait().expect("wait") {
                let log = self.log();
                assert!(
                    log.contains("Address already in use") && self.port_retries < 5,
                    "server exited ({status}):\n{log}"
                );
                // Lost the port race: take a new port and start again.
                self.port_retries += 1;
                self.port = free_port();
                return self.launch();
            }
            if self.log().contains(&ready) && health_ok(self.port) {
                break;
            }
            if Instant::now() >= deadline {
                // Kill it before failing: the child is not yet in
                // self.child, so Drop would leave it running.
                let _ = child.kill();
                let _ = child.wait();
                panic!("server did not listen:\n{}", self.log());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        self.child = Some(child);
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("server.log")).unwrap_or_default()
    }

    /// Graceful stop (SIGTERM on Unix), waiting for the process to exit.
    pub fn stop(&mut self) {
        // Embedded: dropping the last client closes the datastore.
        self.client.lock().expect("client lock").take();
        if let Some(mut child) = self.child.take() {
            terminate(&child);
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                if child.try_wait().expect("wait").is_some() {
                    break;
                }
                if Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }

    pub fn restart(&mut self) {
        self.stop();
        self.launch();
    }

    /// A client on `ns`/`db` obtained through the async repository
    /// (`storage::repo::Repository`), so every contract test runs against
    /// the bootstrapped repository path in both modes.
    pub async fn root(&self, ns: &str, db: &str) -> Surreal<Any> {
        self.repo(ns, db).await.db().clone()
    }

    /// The async repository on `ns`/`db` (bootstrapped).
    #[allow(dead_code)]
    pub async fn repo(&self, ns: &str, db: &str) -> argos_osint_core::storage::repo::Repository {
        use argos_osint_core::storage::open::Target;
        use argos_osint_core::storage::repo::Repository;
        let repo = if self.embedded {
            Repository::attach(self.raw_root(ns, db).await, ns, db).await
        } else {
            Repository::connect(
                &Target::Server {
                    port: self.port,
                    username: "root".into(),
                    password: self.password.clone(),
                },
                ns,
                db,
            )
            .await
        };
        match repo {
            Ok(repo) => repo,
            // Show the server's side (its log never holds the password).
            Err(e) => panic!("open repository: {e}\nserver log:\n{}", self.log()),
        }
    }

    /// A bare client without the schema bootstrap (bootstrap tests only).
    pub async fn raw_root(&self, ns: &str, db: &str) -> Surreal<Any> {
        if self.embedded {
            let cached = self.client.lock().expect("client lock").clone();
            if let Some(client) = cached {
                client.use_ns(ns).use_db(db).await.expect("use ns/db");
                return client;
            }
            let url = format!(
                "rocksdb://{}?versioned=false&sync=every",
                self.data_path().display()
            );
            // A just-dropped datastore may release its RocksDB lock a moment later.
            let deadline = Instant::now() + Duration::from_secs(20);
            let client = loop {
                match connect(url.as_str()).await {
                    Ok(client) => break client,
                    Err(error) if Instant::now() < deadline => {
                        eprintln!("embedded reopen retry: {error}");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    Err(error) => panic!("open embedded datastore: {error}"),
                }
            };
            client.use_ns(ns).use_db(db).await.expect("use ns/db");
            *self.client.lock().expect("client lock") = Some(client.clone());
            return client;
        }
        let client = connect(format!("ws://127.0.0.1:{}", self.port))
            .await
            .expect("connect");
        client
            .signin(Root {
                username: "root".into(),
                password: self.password.clone(),
            })
            .await
            .unwrap_or_else(|e| panic!("root signin: {e}\nserver log:\n{}", self.log()));
        client.use_ns(ns).use_db(db).await.expect("use ns/db");
        client
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(unix)]
pub fn terminate(child: &Child) {
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .status();
}

#[cfg(not(unix))]
pub fn terminate(_child: &Child) {}

/// Readiness: our server answers `GET /health` with 200, which SurrealDB
/// only does once the datastore (and with it the root user created from
/// SURREAL_USER/SURREAL_PASS) is ready. A bare TCP accept is not enough.
fn health_ok(port: u16) -> bool {
    use std::io::{Read, Write};
    let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    if s.write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut buf = [0u8; 16];
    matches!(s.read(&mut buf), Ok(n) if n >= 12 && &buf[9..12] == b"200")
}

pub fn embedded_compiled() -> bool {
    cfg!(feature = "embedded")
}

pub fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

pub fn nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default()
}

pub async fn count(db: &Surreal<Any>, table: &str) -> i64 {
    let mut response = db
        .query(format!("RETURN count(SELECT * FROM {table})"))
        .await
        .expect("count query")
        .check()
        .expect("count statement");
    response
        .take::<Option<i64>>(0)
        .expect("take count")
        .unwrap_or_default()
}

pub fn assert_on_disk(path: &Path) {
    let entries = std::fs::read_dir(path).map(|d| d.count()).unwrap_or(0);
    assert!(entries > 0, "no RocksDB files under {}", path.display());
}
