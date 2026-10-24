//! `argos storage status|init|start|stop|reset --yes` (P4, spec step 8).
//!
//! Reset is the only wipe, and only on explicit request: it never runs on a
//! connection failure or ordinary startup, and it never falls back to
//! creating SQLite. It needs exclusive ownership (no other live client
//! lease in server mode; `embedded.lock` in embedded mode). It removes the
//! application data (the SurrealDB database, old `argos.db` with its
//! WAL/SHM/journal sidecars, and `memory_lancedb/`) and keeps config,
//! provider credentials, quotas, models and datasets. Then it bootstraps
//! the schema and publishes a new reset epoch. Each phase is recorded in
//! `storage/reset.json` before it runs, so an interrupted reset resumes
//! where it stopped.

use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use surrealdb::engine::any::Any;
use surrealdb::Surreal;

use super::home::{self, Layout, Mode, StorageConfig};
use super::server::{self, ServerInfo, DB, NS};

/// `argos storage status` output.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub home: PathBuf,
    /// None: `argos storage init` has not run.
    pub mode: Option<Mode>,
    pub address: Option<String>,
    pub server: Option<ServerInfo>,
    /// Schema version read from the store, when it could be read.
    pub schema_version: Option<i64>,
    pub reset_epoch: Option<String>,
    pub ready: bool,
    /// Phase of an interrupted reset, if one is pending.
    pub pending_reset: Option<ResetPhase>,
    /// Why the store is not ready (never triggers a reset).
    pub detail: Option<String>,
}

/// Reset phases, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResetPhase {
    WipeLegacy,
    WipeStore,
    Bootstrap,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ResetState {
    phase: ResetPhase,
    started_at: String,
}

fn reset_state_path(layout: &Layout) -> PathBuf {
    layout.storage().join("reset.json")
}

pub fn pending_reset(layout: &Layout) -> Result<Option<ResetPhase>> {
    Ok(home::read_json::<ResetState>(&reset_state_path(layout))?.map(|s| s.phase))
}

fn require_config(layout: &Layout) -> Result<StorageConfig> {
    home::load_config(layout)?.ok_or_else(|| {
        anyhow!("storage is not initialized; run `argos storage init --mode server|embedded`")
    })
}

async fn read_meta(db: &Surreal<Any>) -> Result<(Option<i64>, Option<String>)> {
    let mut r = db
        .query(
            "SELECT VALUE version FROM storage_meta WHERE id = storage_meta:schema; \
             SELECT VALUE reset_epoch FROM storage_meta WHERE id = storage_meta:epoch",
        )
        .await?
        .check()?;
    let v: Vec<Option<i64>> = r.take(0usize)?;
    let e: Vec<Option<String>> = r.take(1usize)?;
    Ok((
        v.into_iter().next().flatten(),
        e.into_iter().next().flatten(),
    ))
}

/// Report mode, address, schema and readiness. Never starts a server and
/// never changes data (the embedded store is opened only if no other
/// process owns it, which applies the idempotent schema).
pub async fn status(layout: &Layout) -> Result<Status> {
    let mut st = Status {
        home: layout.home.clone(),
        mode: None,
        address: None,
        server: None,
        schema_version: None,
        reset_epoch: None,
        ready: false,
        pending_reset: pending_reset(layout)?,
        detail: None,
    };
    let Some(cfg) = home::load_config(layout)? else {
        st.detail = Some("not initialized; run `argos storage init --mode server|embedded`".into());
        return Ok(st);
    };
    st.mode = Some(cfg.mode);
    match cfg.mode {
        Mode::Server => {
            let info = server::discover(layout).await?;
            match info {
                Some(info) => {
                    st.address = Some(format!("ws://127.0.0.1:{}", info.port));
                    let creds = home::load_or_create_credentials(layout)?;
                    let db = surrealdb::engine::any::connect(st.address.clone().unwrap()).await?;
                    db.signin(surrealdb::opt::auth::Root {
                        username: creds.username,
                        password: creds.password,
                    })
                    .await?;
                    db.use_ns(NS).use_db(DB).await?;
                    let (v, e) = read_meta(&db).await?;
                    st.schema_version = v;
                    st.reset_epoch = e;
                    st.server = Some(info);
                }
                None => st.detail = Some("server not running (starts on demand)".into()),
            }
        }
        Mode::Embedded => {
            st.address = Some(format!(
                "rocksdb://{}",
                std::path::absolute(layout.data())?.display()
            ));
            #[cfg(feature = "embedded")]
            match home::open_embedded(layout, NS, DB).await {
                Ok((opened, _guard)) => {
                    let (v, e) = read_meta(&opened.db).await?;
                    st.schema_version = v;
                    st.reset_epoch = e;
                }
                Err(e) => st.detail = Some(e.to_string()),
            }
            #[cfg(not(feature = "embedded"))]
            {
                st.detail = Some("this build has no embedded engine (`embedded` feature)".into());
            }
        }
    }
    st.ready =
        st.pending_reset.is_none() && st.schema_version == Some(super::bootstrap::SCHEMA_VERSION);
    if st.pending_reset.is_some() && st.detail.is_none() {
        st.detail = Some("an interrupted reset is pending; run `argos storage reset --yes`".into());
    }
    Ok(st)
}

/// Record the mode and bring the store up with its schema.
pub async fn init(layout: &Layout, mode: Mode, port: Option<u16>) -> Result<Status> {
    if let Some(existing) = home::load_config_file(layout)? {
        if existing.mode != mode {
            bail!(
                "storage is already initialized in {:?} mode; run `argos storage reset --yes` before switching",
                existing.mode
            );
        }
    }
    home::save_config(layout, &StorageConfig { mode, port })?;
    match mode {
        Mode::Server => {
            let (_opened, lease, _) = server::connect_server(layout).await?;
            drop(lease);
        }
        Mode::Embedded => {
            #[cfg(feature = "embedded")]
            {
                let (_opened, _guard) = home::open_embedded(layout, NS, DB).await?;
            }
            #[cfg(not(feature = "embedded"))]
            bail!("this build has no embedded engine (`embedded` feature)");
        }
    }
    status(layout).await
}

fn remove_path(path: &std::path::Path) -> Result<()> {
    let r = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    match r {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("remove {}", path.display())),
    }
}

/// Old SQLite/LanceDB application data under the home.
pub fn legacy_paths(layout: &Layout) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = [
        "argos.db",
        "argos.db-wal",
        "argos.db-shm",
        "argos.db-journal",
    ]
    .iter()
    .map(|n| layout.home.join(n))
    .collect();
    out.push(layout.home.join("memory_lancedb"));
    out
}

fn record_phase(layout: &Layout, phase: ResetPhase, started_at: &str) -> Result<()> {
    home::write_json(
        &reset_state_path(layout),
        &ResetState {
            phase,
            started_at: started_at.to_string(),
        },
    )?;
    Ok(())
}

async fn publish_epoch(db: &Surreal<Any>) -> Result<String> {
    let epoch = home::random_secret()?;
    db.query("UPSERT storage_meta:epoch SET reset_epoch = $e, reset_at = $now RETURN NONE")
        .bind(("e", epoch.clone()))
        .bind(("now", chrono::Utc::now().to_rfc3339()))
        .await?
        .check()?;
    Ok(epoch)
}

/// Wipe this home's application data and bootstrap a fresh store.
/// `yes` must be true (the CLI's `--yes`).
pub async fn reset(layout: &Layout, yes: bool) -> Result<Status> {
    if !yes {
        bail!("`argos storage reset` deletes all Argos application data; pass --yes to confirm");
    }
    let cfg = require_config(layout)?;
    let prior = home::read_json::<ResetState>(&reset_state_path(layout))?;
    let started_at = prior
        .as_ref()
        .map(|s| s.started_at.clone())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    let from = prior.map(|s| s.phase).unwrap_or(ResetPhase::WipeLegacy);

    // Exclusive ownership for the whole reset.
    let me = std::process::id();
    let others: Vec<u32> = server::live_clients(layout)?
        .into_iter()
        .filter(|&p| p != me)
        .collect();
    if !others.is_empty() {
        bail!("storage is in use by PID(s) {others:?}; stop other Argos sessions before reset");
    }
    #[allow(unused_mut)]
    let mut _embedded_guard = match cfg.mode {
        Mode::Embedded => Some(home::lock_embedded(layout)?),
        Mode::Server => None,
    };

    if from <= ResetPhase::WipeLegacy {
        record_phase(layout, ResetPhase::WipeLegacy, &started_at)?;
        for p in legacy_paths(layout) {
            remove_path(&p)?;
        }
    }
    if from <= ResetPhase::WipeStore {
        record_phase(layout, ResetPhase::WipeStore, &started_at)?;
        match cfg.mode {
            Mode::Server => {
                // Database-scoped: the server, identity of other homes and
                // the namespace stay; only the argos database goes.
                let (opened, lease, _) = server::connect_server(layout).await?;
                opened
                    .db
                    .query(format!("REMOVE DATABASE IF EXISTS {DB}"))
                    .await?
                    .check()?;
                drop(lease);
            }
            Mode::Embedded => {
                // Database-scoped here too, not `rm -rf storage/surreal`:
                // the SDK cannot close a datastore, and a just-dropped
                // handle in this process can outlive the directory removal
                // and be reused on reopen (design.md 7b).
                #[cfg(feature = "embedded")]
                {
                    drop(_embedded_guard.take());
                    let (opened, guard) = home::open_embedded(layout, NS, DB).await?;
                    opened
                        .db
                        .query(format!("REMOVE DATABASE IF EXISTS {DB}"))
                        .await?
                        .check()?;
                    drop(opened);
                    _embedded_guard = Some(guard);
                }
            }
        }
    }
    record_phase(layout, ResetPhase::Bootstrap, &started_at)?;
    match cfg.mode {
        Mode::Server => {
            // A fresh database needs the server identity back before
            // discovery can adopt the server again.
            // If that server died, start fresh (a new identity is fine:
            // the database is empty).
            let info = home::read_json::<ServerInfo>(&layout.server_json())?
                .filter(|i| server::pid_alive(i.pid));
            let creds = home::load_or_create_credentials(layout)?;
            if let Some(info) = info {
                let opened = super::open::open(
                    &super::open::Target::Server {
                        port: info.port,
                        username: creds.username,
                        password: creds.password,
                    },
                    NS,
                    DB,
                )
                .await
                .map_err(|e| anyhow!("{e}"))?;
                opened
                    .db
                    .query("UPSERT storage_meta:identity SET instance_id = $id RETURN NONE")
                    .bind(("id", info.instance_id.clone()))
                    .await?
                    .check()?;
                publish_epoch(&opened.db).await?;
            } else {
                let (opened, lease, _) = server::connect_server(layout).await?;
                publish_epoch(&opened.db).await?;
                drop(lease);
            }
        }
        Mode::Embedded => {
            #[cfg(feature = "embedded")]
            {
                drop(_embedded_guard.take());
                let (opened, _guard) = home::open_embedded(layout, NS, DB).await?;
                publish_epoch(&opened.db).await?;
            }
            #[cfg(not(feature = "embedded"))]
            bail!("this build has no embedded engine (`embedded` feature)");
        }
    }
    remove_path(&reset_state_path(layout))?;
    status(layout).await
}
