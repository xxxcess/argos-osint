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

pub fn auth_path() -> PathBuf {
    home_dir().join("auth.json")
}

pub fn config_path() -> PathBuf {
    home_dir().join("config.toml")
}

pub fn hardware_cache_path() -> PathBuf {
    home_dir().join("hardware.json")
}
