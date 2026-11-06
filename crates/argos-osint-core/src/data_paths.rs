//! Saved dataset and classifier locations shared by Profile, Venture and workers.
//! @tag:classifier @tag:brain-memory

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Empty fields select the ordinary automatic locations. Relative saved paths
/// are anchored to `ARGOS_HOME`, independent of a service's working directory.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DataPaths {
    pub actor_csv: String,
    pub article_csv: String,
    /// Directory containing active.json, immutable models and training journals.
    pub classifier_dir: String,
}

impl DataPaths {
    /// Reads only the path settings, without migrations, writes or directory creation.
    pub fn load_from_home(home: &Path) -> io::Result<Self> {
        #[derive(Default, Deserialize)]
        struct Document {
            #[serde(default)]
            data_paths: DataPaths,
        }
        let file = match File::open(home.join("config.toml")) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;
        let mut body = String::new();
        file.take(MAX_CONFIG_BYTES + 1).read_to_string(&mut body)?;
        if body.len() as u64 > MAX_CONFIG_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "config.toml exceeds size limit",
            ));
        }
        let document: Document = toml::from_str(&body).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid data path configuration",
            )
        })?;
        document.data_paths.validate()?;
        Ok(document.data_paths)
    }

    pub fn validate(&self) -> io::Result<()> {
        for value in [&self.actor_csv, &self.article_csv, &self.classifier_dir] {
            if value.len() > 4096 || value.chars().any(char::is_control) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "paths must contain at most 4096 bytes and no control characters",
                ));
            }
        }
        Ok(())
    }

    /// Resolves a saved path without requiring the destination to exist yet.
    pub fn resolve(home: &Path, value: &str) -> Option<PathBuf> {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        let path = if value == "~" || value.starts_with("~/") {
            dirs::home_dir()
                .map(|root| {
                    value
                        .strip_prefix("~/")
                        .map_or(root.clone(), |tail| root.join(tail))
                })
                .unwrap_or_else(|| PathBuf::from(value))
        } else {
            PathBuf::from(value)
        };
        Some(if path.is_absolute() {
            path
        } else {
            home.join(path)
        })
    }

    pub fn classifier_root(&self, home: &Path) -> PathBuf {
        Self::resolve(home, &self.classifier_dir).unwrap_or_else(|| home.join("classifier"))
    }
}
