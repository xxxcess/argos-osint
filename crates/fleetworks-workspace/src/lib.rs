//! Host workspace paths, configuration and credentials for Fleetworks clients.

mod import;

use std::collections::BTreeMap;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, ensure, Context, Result};
use serde::Deserialize;

pub use import::ImportSummary;

/// The canonical application state root selected for a Venture invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    root: PathBuf,
}

const DIRECTORIES: &[&str] = &[
    "datasets", "exports", "config", "secrets", "models", "logs", "storage",
];
const CONFIG_LIMIT: usize = 1024 * 1024;
const TOKEN_LIMIT: usize = 4096;
const TOKEN: &str = "FLEETWORKS_API_TOKEN";
const TOKEN_FILE: &str = "FLEETWORKS_API_TOKEN_FILE";
const DEFAULT_TOKEN_FILE: &str = "secrets/api-token";

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    datasets: BTreeMap<String, PathBuf>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SecretSettings {
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

/// Workspace environment defaults. Process variables take precedence at lookup.
pub struct Environment {
    configured: BTreeMap<String, String>,
}

impl Environment {
    /// Resolve an environment value without changing this process's environment.
    pub fn get(&self, name: &str) -> Option<OsString> {
        env::var_os(name).or_else(|| self.configured.get(name).map(OsString::from))
    }

    /// Supply effective values for configured names to a child command.
    pub fn overrides(&self) -> impl Iterator<Item = (&str, OsString)> {
        self.configured.iter().map(|(name, default)| {
            (
                name.as_str(),
                env::var_os(name).unwrap_or_else(|| OsString::from(default)),
            )
        })
    }
}

impl Workspace {
    /// Resolve an explicit path, then `FLEETWORKS_WORKSPACE`, then `cwd/.fleetworks`.
    /// The workspace may be missing and this method does not create it.
    pub fn resolve(explicit: Option<&Path>) -> Result<Self> {
        let cwd = env::current_dir().context("resolve current directory")?;
        let configured = env::var_os("FLEETWORKS_WORKSPACE");
        Self::resolve_from(explicit, configured.as_deref(), &cwd)
    }

    fn resolve_from(
        explicit: Option<&Path>,
        configured: Option<&OsStr>,
        cwd: &Path,
    ) -> Result<Self> {
        let selected = explicit
            .or_else(|| configured.filter(|value| !value.is_empty()).map(Path::new))
            .unwrap_or_else(|| Path::new(".fleetworks"));
        ensure!(!selected.as_os_str().is_empty(), "workspace path is empty");
        let root = canonical_with_missing(&cwd.join(selected))?;
        if let Some(metadata) = metadata_if_exists(&root)? {
            ensure!(metadata.is_dir(), "workspace path is not a directory");
        }
        Ok(Self { root })
    }

    /// The resolved absolute path on the host.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve a workspace-relative input. Absolute overrides are kept as supplied.
    pub fn resolve_path(&self, path: &Path) -> PathBuf {
        self.root.join(path)
    }

    /// Create a private workspace and its managed directories where missing.
    /// Existing workspace data and ignore rules are preserved.
    pub fn initialize(&self) -> Result<()> {
        create_private_directory(&self.root, true)?;
        for name in DIRECTORIES {
            let path = self.root.join(name);
            create_private_directory(&path, false)?;
            if *name == "secrets" {
                require_private(&fs::symlink_metadata(&path)?, &path)?;
            }
        }
        let ignore = self.root.join(".gitignore");
        if metadata_if_exists(&ignore)?.is_none() {
            match persist_new(&ignore, b"*\n") {
                Ok(()) => (),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
                Err(error) => return Err(error).context("write workspace ignore rules"),
            }
        }
        Ok(())
    }

    fn settings(&self) -> Result<Settings> {
        let path = self.root.join("config/workspace.toml");
        read_toml(&path, false)
    }

    /// Load workspace defaults and override them with private environment defaults.
    /// Values from the live process environment are looked up by the returned object.
    pub fn environment(&self) -> Result<Environment> {
        let mut configured = self.settings()?.environment;
        let path = self.root.join("secrets/environment.toml");
        let secret: SecretSettings = read_toml(&path, true)?;
        configured.extend(secret.environment);
        for (name, value) in &configured {
            let mut bytes = name.bytes();
            ensure!(
                bytes
                    .next()
                    .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
                    && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
                    && !value.contains('\0'),
                "invalid workspace environment entry"
            );
        }
        Ok(Environment { configured })
    }

    /// Resolve a configured dataset binding against the workspace root.
    pub fn dataset(&self, name: &str) -> Result<Option<PathBuf>> {
        let settings = self.settings()?;
        let Some(path) = settings.datasets.get(name) else {
            return Ok(None);
        };
        ensure!(
            !path.as_os_str().is_empty(),
            "configured dataset path is empty"
        );
        Ok(Some(self.resolve_path(path)))
    }

    /// Provision an owner-only random API token when the default token file is absent.
    pub fn initialize_api_token(&self) -> Result<()> {
        self.initialize()?;
        let path = self.root.join(DEFAULT_TOKEN_FILE);
        if metadata_if_exists(&path)?.is_some() {
            return self
                .read_api_token_file(Path::new(DEFAULT_TOKEN_FILE))
                .map(|_| ());
        }
        let mut random = [0_u8; 32];
        getrandom::fill(&mut random).map_err(|_| anyhow!("generate workspace API token"))?;
        let mut body = String::with_capacity(65);
        for byte in random {
            write!(&mut body, "{byte:02x}").expect("writing into a String cannot fail");
        }
        body.push('\n');
        match persist_new(&path, body.as_bytes()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => self
                .read_api_token_file(Path::new(DEFAULT_TOKEN_FILE))
                .map(|_| ()),
            Err(error) => Err(error).context("write workspace API token"),
        }
    }

    /// Resolve an API token from the process, workspace defaults or the default file.
    /// A token takes precedence over a token file at the same configuration level.
    pub fn api_token(&self, environment: &Environment) -> Result<Option<String>> {
        if let Some(value) = env::var_os(TOKEN) {
            let value = value
                .into_string()
                .map_err(|_| anyhow!("invalid API token"))?;
            return checked_token(value).map(Some);
        }
        if let Some(path) = env::var_os(TOKEN_FILE) {
            return self.read_api_token_file(Path::new(&path)).map(Some);
        }
        if let Some(value) = environment.configured.get(TOKEN) {
            return checked_token(value.clone()).map(Some);
        }
        if let Some(path) = environment.configured.get(TOKEN_FILE) {
            return self.read_api_token_file(Path::new(path)).map(Some);
        }
        optional_text(&self.root.join(DEFAULT_TOKEN_FILE), TOKEN_LIMIT, true)?
            .map(checked_token)
            .transpose()
    }

    /// Read an explicitly selected private token file. Relative paths are anchored here.
    pub fn read_api_token_file(&self, path: &Path) -> Result<String> {
        ensure!(!path.as_os_str().is_empty(), "API token file path is empty");
        let path = self.resolve_path(path);
        let value = optional_text(&path, TOKEN_LIMIT, true)?
            .with_context(|| format!("API token file {} does not exist", path.display()))?;
        checked_token(value)
    }
}

fn checked_token(value: String) -> Result<String> {
    let value = value.trim_end_matches(['\r', '\n']);
    ensure!(
        !value.is_empty()
            && value.len() <= TOKEN_LIMIT
            && value.bytes().all(|byte| (b'!'..=b'~').contains(&byte)),
        "invalid API token"
    );
    Ok(value.to_owned())
}

fn canonical_with_missing(path: &Path) -> Result<PathBuf> {
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => resolved.push(component.as_os_str()),
            Component::CurDir => (),
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(name) => {
                let candidate = resolved.join(name);
                resolved = if metadata_if_exists(&candidate)?.is_some() {
                    fs::canonicalize(&candidate).with_context(|| {
                        format!("resolve workspace path {}", candidate.display())
                    })?
                } else {
                    candidate
                };
            }
        }
    }
    Ok(resolved)
}

fn metadata_if_exists(path: &Path) -> io::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn create_private_directory(path: &Path, recursive: bool) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(recursive);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => (),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error).with_context(|| format!("create {}", path.display())),
    }
    ensure!(
        fs::symlink_metadata(path)?.is_dir(),
        "workspace path {} is not a real directory",
        path.display()
    );
    Ok(())
}

fn require_private(metadata: &fs::Metadata, path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "{} must be accessible to its owner only",
            path.display()
        );
    }
    #[cfg(not(unix))]
    let _ = (metadata, path);
    Ok(())
}

fn read_toml<T: Default + serde::de::DeserializeOwned>(path: &Path, private: bool) -> Result<T> {
    let Some(text) = optional_text(path, CONFIG_LIMIT, private)? else {
        return Ok(T::default());
    };
    toml::from_str(&text).map_err(|_| anyhow!("invalid TOML in {}", path.display()))
}

fn optional_text(path: &Path, limit: usize, private: bool) -> Result<Option<String>> {
    let Some(metadata) = metadata_if_exists(path)? else {
        return Ok(None);
    };
    ensure!(
        metadata.is_file(),
        "{} is not a regular file",
        path.display()
    );
    if private {
        require_private(&metadata, path)?;
    }
    let mut body = String::new();
    fs::File::open(path)
        .with_context(|| format!("open {}", path.display()))?
        .take(limit as u64 + 1)
        .read_to_string(&mut body)
        .with_context(|| format!("read {}", path.display()))?;
    ensure!(
        body.len() <= limit,
        "{} exceeds its size limit",
        path.display()
    );
    Ok(Some(body))
}

fn persist_new(path: &Path, body: &[u8]) -> io::Result<()> {
    let parent = path.parent().expect("workspace files have a parent");
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    staged.write_all(body)?;
    staged.as_file().sync_all()?;
    staged
        .persist_noclobber(path)
        .map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests;
