use std::fs;
use std::path::{Path, PathBuf};

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
    fs::create_dir_all(dir.join("reports"))?;
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

pub fn mcp_path() -> PathBuf {
    home_dir().join("mcp.json")
}

pub fn hardware_cache_path() -> PathBuf {
    home_dir().join("hardware.json")
}

/// Short label for the status bar. The full path is kept when it fits.
pub fn db_label() -> String {
    let path = db_path();
    shorten_path(&path)
}

pub fn shorten_path(path: &Path) -> String {
    let raw = path.display().to_string();
    if let Some(home) = dirs::home_dir() {
        let home = home.display().to_string();
        if let Some(rest) = raw.strip_prefix(&home) {
            return format!("~{rest}");
        }
    }
    raw
}

/// Status title. Drops the database label when the line would overflow.
pub fn fit_status(width: usize, mode: &str, db: &str) -> String {
    let mode = mode.trim();
    let db = db.trim();
    if db.is_empty() || width == 0 {
        return mode.to_string();
    }
    let full = format!("{mode}  {db}");
    if full.chars().count() <= width {
        full
    } else {
        mode.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_drops_db_when_narrow() {
        assert_eq!(
            fit_status(40, "Case Desk", "~/.argos/argos.db"),
            "Case Desk  ~/.argos/argos.db"
        );
        assert_eq!(fit_status(8, "Case Desk", "~/.argos/argos.db"), "Case Desk");
        assert_eq!(fit_status(4, "Case Desk", "~/.argos/argos.db"), "Case Desk");
    }
}
