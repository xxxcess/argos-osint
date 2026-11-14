//! Read one locked current-format configuration without migration or environment fallback.
//!
//! This shares the existing backend files and commit revision with API admission
//! and workers. Snapshots contain secrets and deliberately cannot be serialized
//! or printed. Only non-secret references belong in durable Jobs metadata.

use crate::config_commit;
use crate::config_commit::ConfigLock;
use crate::config_transfer::QuotaSettingsFile;
use crate::provider;
use crate::provider::ModelAssignment;
use crate::provider::SettingsFile;
use crate::secrets::AuthFile;
use serde_json::Value;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;

const LIMIT: u64 = 2 * 1024 * 1024;
const FORMAT: u16 = 1;

/// A safe error classification containing no configuration bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ReadError {
    LegacyFormat,
    Unavailable,
}
impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::LegacyFormat => "Current configuration format required",
            Self::Unavailable => "Current configuration unavailable",
        })
    }
}
impl std::error::Error for ReadError {}
fn internal(_: &str) -> ReadError {
    ReadError::Unavailable
}

/// A private in-memory copy captured under the shared configuration lock.
#[non_exhaustive]
pub struct Snapshot {
    pub revision: u64,
    pub settings: SettingsFile,
    pub auth: AuthFile,
    pub quotas: QuotaSettingsFile,
}

impl Snapshot {
    /// Opaque saved-account references only. Runtime route capture owns the
    /// role-specific subset; no bearer or API key can enter a job origin.
    pub fn credential_references(&self) -> Vec<String> {
        let mut references = std::collections::BTreeSet::new();
        for role in provider::CANONICAL_ROLE_KEYS {
            if let Some(assignment) = self.settings.defaults.role(role) {
                for route in assignment.chain() {
                    let id = if route.account.is_empty() {
                        &route.provider
                    } else {
                        &route.account
                    };
                    if self.auth.accounts.contains_key(id) {
                        references.insert(format!("account:{id}"));
                    }
                }
            }
        }
        for (id, _) in provider::KEY_ENV {
            if !self.settings.saved_key(id).is_empty() {
                references.insert(format!("tool:{id}:primary"));
            }
            if !self.settings.saved_fallback_key(id).is_empty() {
                references.insert(format!("tool:{id}:fallback"));
            }
        }
        references.into_iter().collect()
    }
}

fn bytes(root: &Path, name: &str) -> Result<Option<Vec<u8>>, ReadError> {
    let path = root.join(name);
    let before = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(internal("configuration file unavailable")),
    };
    if !before.is_file() || before.file_type().is_symlink() || before.len() > LIMIT {
        return Err(internal("configuration file is not a bounded regular file"));
    }
    let file = fs::File::open(path).map_err(|_| internal("configuration file unavailable"))?;
    let after = file
        .metadata()
        .map_err(|_| internal("configuration file unavailable"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if (before.dev(), before.ino()) != (after.dev(), after.ino())
            || after.nlink() != 1
            || after.mode() & 0o077 != 0
        {
            return Err(internal("configuration file is not privately owned"));
        }
    }
    let mut raw = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut raw)
        .map_err(|_| internal("configuration read failed"))?;
    if raw.len() as u64 > LIMIT {
        return Err(internal("configuration byte bound exceeded"));
    }
    Ok(Some(raw))
}

fn empty_settings() -> SettingsFile {
    let mut settings = SettingsFile::default();
    settings.defaults.recon_agent = Some(ModelAssignment::default());
    settings.defaults.recon_decisions = Some(ModelAssignment::default());
    settings.defaults.recon_analyst = Some(ModelAssignment::default());
    settings.defaults.atlas_evaluator = Some(ModelAssignment::default());
    settings.defaults.atlas_analyst = Some(ModelAssignment::default());
    settings.defaults.brain_summary = Some(ModelAssignment::default());
    settings
}

fn reset_required() -> ReadError {
    ReadError::LegacyFormat
}

/// Select the explicit network workspace without falling back to a user home.
/// Retained embedded/operator processes have no network workspace selection.
pub(crate) fn network_root() -> Result<Option<PathBuf>, ReadError> {
    let Some(root) = std::env::var_os("FLEETWORKS_WORKSPACE").map(PathBuf::from) else {
        return Ok(None);
    };
    if !root.is_absolute() || std::env::var_os("ARGOS_HOME").as_deref() != Some(root.as_os_str()) {
        return Err(ReadError::Unavailable);
    }
    Ok(Some(root))
}

/// Read the current private auth file without rewriting legacy slots. This
/// atomic-file reader also serves event redaction, which can run while a
/// configuration writer holds the cross-process lock.
pub(crate) fn read_auth(root: &Path) -> Result<AuthFile, ReadError> {
    let Some(raw) = bytes(root, "auth.json")? else {
        return Ok(AuthFile::default());
    };
    let value: Value = serde_json::from_slice(&raw).map_err(|_| reset_required())?;
    if value.get("configuration_format").and_then(Value::as_u64) != Some(u64::from(FORMAT)) {
        return Err(reset_required());
    }
    let auth: AuthFile = serde_json::from_value(value).map_err(|_| reset_required())?;
    if auth.text.is_some() || auth.voice.is_some() {
        return Err(reset_required());
    }
    Ok(auth)
}

pub fn read_locked(root: &Path, lock: &ConfigLock) -> Result<Snapshot, ReadError> {
    if lock.root() != root {
        return Err(ReadError::Unavailable);
    }
    if config_commit::read_journal_at(root).is_some_and(|journal| journal.state == "pending") {
        return Err(internal("configuration startup recovery is required"));
    }
    let settings = match bytes(root, "config.toml")? {
        None => empty_settings(),
        Some(raw) => {
            let text = std::str::from_utf8(&raw).map_err(|_| reset_required())?;
            let value: toml::Value = toml::from_str(text).map_err(|_| reset_required())?;
            if value
                .get("configuration_format")
                .and_then(toml::Value::as_integer)
                != Some(i64::from(FORMAT))
            {
                return Err(reset_required());
            }
            let settings: SettingsFile = toml::from_str(text).map_err(|_| reset_required())?;
            if provider::CANONICAL_ROLE_KEYS
                .iter()
                .any(|role| settings.defaults.role(role).is_none())
            {
                return Err(reset_required());
            }
            settings
        }
    };
    let auth = read_auth(root)?;
    let quotas = match bytes(root, "quota.json")? {
        None => QuotaSettingsFile::default(),
        Some(raw) => serde_json::from_slice(&raw).map_err(|_| reset_required())?,
    };
    Ok(Snapshot {
        revision: lock.generation(),
        settings,
        auth,
        quotas,
    })
}

/// Acquire the existing commit lock before observing current files. Use on a blocking worker.
pub fn read(root: &Path) -> Result<Snapshot, ReadError> {
    let lock = ConfigLock::acquire_at(root).map_err(|_| ReadError::Unavailable)?;
    read_locked(root, &lock)
}
