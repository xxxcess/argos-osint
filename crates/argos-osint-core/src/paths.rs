use std::fs;
use std::path::PathBuf;

/// Config root. `ARGOS_HOME` overrides `~/.argos`.
pub fn home_dir() -> PathBuf {
    if let Ok(over) = std::env::var("ARGOS_HOME") {
        if !over.trim().is_empty() {
            return PathBuf::from(over);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".argos")
}

pub fn ensure_home() -> std::io::Result<PathBuf> {
    let dir = home_dir();
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn db_path() -> PathBuf {
    home_dir().join("argos.db")
}

/// Brain vector index (LanceDB) for the default database, beside `argos.db`.
pub fn lancedb_dir() -> PathBuf {
    home_dir().join("memory_lancedb")
}

/// Brain vector index for the database at `db`: `memory_lancedb/` in the same folder.
/// For [`db_path`] this is [`lancedb_dir`].
pub fn lancedb_dir_for(db: &std::path::Path) -> PathBuf {
    db.parent()
        .map(|dir| dir.join("memory_lancedb"))
        .unwrap_or_else(lancedb_dir)
}

pub fn auth_path() -> PathBuf {
    home_dir().join("auth.json")
}

pub fn config_path() -> PathBuf {
    home_dir().join("config.toml")
}

/// App-wide configuration write lock, beside `config.toml`.
///
/// [`crate::config_commit::ConfigLock`] locks the directory that holds the
/// configuration files; this is the fixed path an operator inspects by hand.
pub fn config_lock_path() -> PathBuf {
    home_dir().join(".argos-config.lock")
}

/// Recoverable commit journal for the app-wide configuration lock. It records
/// paths and state only, never a file body and never key material.
pub fn config_journal_path() -> PathBuf {
    home_dir().join(".argos-config-journal.json")
}

/// Provider quota settings, the third configuration file that moves with
/// `config.toml` and `auth.json` under one lock.
pub fn quota_path() -> PathBuf {
    home_dir().join("quota.json")
}

pub fn hardware_cache_path() -> PathBuf {
    home_dir().join("hardware.json")
}

/// Root directory for cached dataset snapshots (`~/.argos/datasets/`).
pub fn datasets_dir() -> PathBuf {
    home_dir().join("datasets")
}

/// Directory for a specific dataset (`~/.argos/datasets/<name>/`).
pub fn dataset_dir(name: &str) -> PathBuf {
    datasets_dir().join(name)
}
