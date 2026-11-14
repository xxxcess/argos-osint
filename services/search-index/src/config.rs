//! Operator-owned configuration; no request chooses a directory or upstream.

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::net::SocketAddr;
use std::path::Path;

use anyhow::ensure;
use anyhow::Context;
use anyhow::Result;
use axum::http::uri::Authority;
use fleetworks_contracts::actor_index::ActorIndexLimits;
use sha2::Digest;

use crate::engine::EngineConfig;

#[derive(Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub allowed_hosts: BTreeSet<String>,
    pub read_digest: [u8; 32],
    pub write_digest: [u8; 32],
    pub query_capacity: usize,
    pub engine: EngineConfig,
    pub limits: ActorIndexLimits,
}

impl Config {
    pub fn from_environment() -> Result<Self> {
        let bind = env::var("FLEETWORKS_SEARCH_BIND")
            .unwrap_or_else(|_| "127.0.0.1:8087".into())
            .parse()
            .context("invalid private search bind address")?;
        let hosts = env::var("FLEETWORKS_SEARCH_ALLOWED_HOSTS")
            .context("FLEETWORKS_SEARCH_ALLOWED_HOSTS is required")?;
        let allowed_hosts = parse_hosts(&hosts)?;
        let read_digest = token_digest(Path::new(
            &env::var("FLEETWORKS_SEARCH_TOKEN_FILE")
                .context("FLEETWORKS_SEARCH_TOKEN_FILE is required")?,
        ))?;
        let write_digest = token_digest(Path::new(
            &env::var("FLEETWORKS_SEARCH_WRITE_TOKEN_FILE")
                .context("FLEETWORKS_SEARCH_WRITE_TOKEN_FILE is required")?,
        ))?;
        ensure!(
            read_digest != write_digest,
            "search read and write credentials must be different"
        );
        let mut limits = match env::var("FLEETWORKS_SEARCH_LIMITS_FILE") {
            Ok(path) => {
                let path = Path::new(&path);
                let metadata =
                    fs::symlink_metadata(path).context("cannot inspect search limits")?;
                ensure!(
                    metadata.is_file()
                        && !metadata.file_type().is_symlink()
                        && metadata.len() <= 64 * 1024,
                    "invalid search limits file"
                );
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    ensure!(
                        metadata.permissions().mode() & 0o077 == 0,
                        "search limits must be owner-only"
                    );
                }
                serde_json::from_slice::<ActorIndexLimits>(
                    &fs::read(path).context("cannot read search limits")?,
                )
                .context("invalid search limits JSON")?
            }
            Err(env::VarError::NotPresent) => ActorIndexLimits::default(),
            Err(_) => anyhow::bail!("invalid search limits path encoding"),
        };
        limits.writer_memory_bytes = number(
            "FLEETWORKS_SEARCH_WRITER_MEMORY_BYTES",
            limits.writer_memory_bytes as usize,
        )? as u64;
        limits.query_capacity = u8::try_from(number(
            "FLEETWORKS_SEARCH_QUERY_CAPACITY",
            usize::from(limits.query_capacity),
        )?)
        .context("invalid query capacity")?;
        limits.validate()?;
        let engine = EngineConfig {
            directory: env::var("FLEETWORKS_SEARCH_INDEX_DIR")
                .context("FLEETWORKS_SEARCH_INDEX_DIR is required")?
                .into(),
            deployment_id: env::var("FLEETWORKS_SEARCH_DEPLOYMENT_ID")
                .context("FLEETWORKS_SEARCH_DEPLOYMENT_ID is required")?,
            source_epoch: env::var("FLEETWORKS_SEARCH_SOURCE_EPOCH")
                .context("FLEETWORKS_SEARCH_SOURCE_EPOCH is required")?,
            writer_memory_bytes: limits.writer_memory_bytes as usize,
            writer_threads: usize::from(limits.writer_threads),
            merge_threads: usize::from(limits.merge_threads),
        };
        engine.validate()?;
        let query_capacity = usize::from(limits.query_capacity);
        ensure!(
            (1..=8).contains(&query_capacity),
            "search query capacity must be 1 through 8"
        );
        Ok(Self {
            bind,
            allowed_hosts,
            read_digest,
            write_digest,
            query_capacity,
            engine,
            limits,
        })
    }
}

pub fn parse_hosts(value: &str) -> Result<BTreeSet<String>> {
    let mut hosts = BTreeSet::new();
    for value in value.split(',').map(str::trim) {
        ensure!(
            !value.is_empty()
                && value.len() <= 256
                && !value
                    .chars()
                    .any(|character| matches!(character, '@' | '*' | '/' | '\\')),
            "invalid private search allowed host"
        );
        let authority: Authority = value
            .parse()
            .context("invalid private search allowed host")?;
        ensure!(
            !authority.host().is_empty(),
            "empty private search allowed host"
        );
        ensure!(
            authority.port().is_none() || authority.port_u16().is_some(),
            "invalid private search allowed-host port"
        );
        hosts.insert(value.to_ascii_lowercase());
    }
    ensure!(
        !hosts.is_empty() && hosts.len() <= 32,
        "private search requires 1 through 32 allowed hosts"
    );
    Ok(hosts)
}

pub fn valid_token(value: &[u8]) -> bool {
    (32..=256).contains(&value.len())
        && value
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._~+/=".contains(byte))
}

fn token_digest(path: &Path) -> Result<[u8; 32]> {
    let metadata =
        fs::symlink_metadata(path).context("cannot read private search credential metadata")?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() <= 512,
        "private search credential must be a bounded regular file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "private search credential must not be accessible to group/others"
        );
    }
    let value = fs::read_to_string(path).context("cannot read private search credential")?;
    ensure!(
        valid_token(value.trim().as_bytes()),
        "invalid private search credential syntax"
    );
    Ok(sha2::Sha256::digest(value.trim().as_bytes()).into())
}

fn number(name: &str, default: usize) -> Result<usize> {
    match env::var(name) {
        Ok(value) => value.parse().with_context(|| format!("invalid {name}")),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(_) => anyhow::bail!("invalid non-Unicode {name}"),
    }
}
