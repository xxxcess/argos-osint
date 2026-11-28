//! Local saved gateway destinations. Credential bytes never enter this catalog.

use std::collections::BTreeSet;
use std::fmt;
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use fleetworks_contracts::Client;
use rustix::fs::AtFlags;
use rustix::fs::FileType;
use rustix::fs::Mode;
use rustix::fs::OFlags;
use serde::Deserialize;
use serde::Serialize;
use unicase::UniCase;

use super::fs;
use super::fs::ObjectIdentity;
use super::NativeContainer;

pub const CONNECTIONS_FILE: &str = ".native-connections.json";
/// Canonical origin identifier for the default embedded local companion backend.
pub const DEFAULT_LOCAL_ORIGIN: &str = "local://embedded";
/// Private subdirectory under the workstation root for per-profile local backend state.
pub const LOCAL_BACKEND_DIR: &str = "local-backend";
const FORMAT: u16 = 1;
const MAX_PROFILES: usize = 32;
const MAX_CATALOG_BYTES: u64 = 256 * 1024;
const MAX_PROFILE_NAME_BYTES: usize = 128;
const TOKEN_LIMIT: u64 = 4096;

/// Transport kind for a saved Fleetworks destination profile.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BackendKind {
    /// Remote HTTP(S) gateway paired via token or system keyring.
    #[default]
    Remote,
    /// Supervised local embedded companion backend running over stdio IPC.
    Local,
}

/// Canonicalize a `local://<slug>` identifier for local embedded backends.
pub fn canonical_local_origin(origin: &str) -> Result<String> {
    anyhow::ensure!(
        origin.len() <= 2048 && origin.trim() == origin && !origin.chars().any(char::is_control),
        "local backend origin contains invalid characters or edge whitespace"
    );
    let prefix = origin
        .get(..8)
        .filter(|prefix| prefix.eq_ignore_ascii_case("local://"))
        .context("local backend origin must start with local://")?;
    let _ = prefix;
    let slug = origin[8..].strip_suffix('/').unwrap_or(&origin[8..]);
    validate_local_slug(slug)?;
    Ok(format!("local://{}", slug.to_ascii_lowercase()))
}

/// Derive the canonical `local://` origin string for a saved local profile ID.
pub fn local_origin_for_profile(profile_id: &str) -> Result<String> {
    validate_local_slug(profile_id)?;
    Ok(format!("local://{}", profile_id.to_ascii_lowercase()))
}

/// Canonicalize either a `local://<slug>` target or a remote HTTP(S) gateway origin.
pub fn canonical_target_origin(origin: &str) -> Result<String> {
    if origin
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("local://"))
    {
        canonical_local_origin(origin)
    } else {
        Client::canonical_origin(origin).map_err(anyhow::Error::from)
    }
}

fn validate_local_slug(slug: &str) -> Result<()> {
    anyhow::ensure!(
        !slug.is_empty()
            && slug.len() <= 64
            && slug != "."
            && slug != ".."
            && slug
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'),
        "local backend slug must be 1–64 ASCII alphanumeric, '-' or '_' characters"
    );
    Ok(())
}

fn validate_local_state_dir_path(path: &Path) -> Result<()> {
    let text = path
        .to_str()
        .context("local backend state directory must be valid UTF-8")?;
    anyhow::ensure!(
        path.is_absolute()
            && path.parent().is_some()
            && text.len() <= 4096
            && !text.chars().any(char::is_control)
            && path
                .components()
                .all(|component| matches!(component, Component::RootDir | Component::Normal(_))),
        "local backend state directory must be a clean absolute path without traversal"
    );
    Ok(())
}

fn ensure_private_child_directory(
    parent: &rustix::fd::OwnedFd,
    name: &str,
) -> Result<(rustix::fd::OwnedFd, ObjectIdentity)> {
    match fs::entry_identity(parent, name)? {
        Some(_) => {}
        None => match rustix::fs::mkdirat(parent, name, fs::PRIVATE_DIRECTORY) {
            Ok(()) => {}
            Err(rustix::io::Errno::EXIST) => {}
            Err(error) => return Err(error).context("create private local backend directory"),
        },
    }
    let dir_fd = rustix::fs::openat(parent, name, fs::DIRECTORY_FLAGS, Mode::empty())
        .context("local backend state directory must be a real directory without symlinks")?;
    let object = fs::directory_identity(&dir_fd, true)?;
    Ok((dir_fd, object))
}

/// Origin-bound reference to one explicitly selected protected token file.
/// Serialization contains identity/path only; Debug omits its private path.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialReference {
    path: PathBuf,
    origin: String,
    object: ObjectIdentity,
}

impl fmt::Debug for CredentialReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialReference")
            .field("origin", &self.origin)
            .field("file", &"<protected file reference>")
            .finish()
    }
}

impl CredentialReference {
    /// Capture metadata only. The caller explicitly picked this file; no home,
    /// Keychain, environment, sibling folder or credential scan is performed.
    pub fn capture(path: &Path, origin: &str) -> Result<Self> {
        let origin = Client::canonical_origin(origin)?;
        let (file, object) = open_token(path)?;
        drop(file);
        Ok(Self {
            path: path.to_path_buf(),
            origin,
            object,
        })
    }
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
    #[must_use]
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Reopen the exact protected object for a single Test/Connect. A replacement
    /// token file requires explicit re-selection instead of silent adoption.
    pub fn open_verified(&self, origin: &str) -> Result<File> {
        anyhow::ensure!(
            Client::canonical_origin(origin)? == self.origin,
            "credential reference is bound to another gateway"
        );
        let (file, object) = open_token(&self.path)?;
        anyhow::ensure!(
            object == self.object,
            "protected credential file changed; select it again"
        );
        Ok(file)
    }
    fn validate(&self, owner: u32) -> Result<()> {
        validate_path(&self.path)?;
        anyhow::ensure!(
            self.object.owner == owner
                && self.origin.len() <= 2048
                && Client::canonical_origin(&self.origin)? == self.origin,
            "credential reference ownership or origin is invalid"
        );
        Ok(())
    }
}

fn validate_path(path: &Path) -> Result<()> {
    let text = path
        .to_str()
        .context("credential reference path must be valid UTF-8")?;
    anyhow::ensure!(
        path.is_absolute()
            && text.len() <= 4096
            && !text.chars().any(char::is_control)
            && path
                .components()
                .all(|component| matches!(component, Component::RootDir | Component::Normal(_))),
        "credential reference must be one exact absolute file path"
    );
    Ok(())
}

fn open_token(path: &Path) -> Result<(File, ObjectIdentity)> {
    validate_path(path)?;
    let parent =
        fs::open_absolute_directory(path.parent().context("credential file parent missing")?)?;
    let name = path.file_name().context("credential file name missing")?;
    let file = File::from(
        rustix::fs::openat(
            &parent,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .context("cannot open selected protected credential file")?,
    );
    let stat = rustix::fs::fstat(&file)?;
    anyhow::ensure!(
        FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile
            && stat.st_uid == rustix::process::geteuid().as_raw()
            && stat.st_mode & 0o077 == 0
            && stat.st_nlink == 1
            && (0..=TOKEN_LIMIT as i64).contains(&stat.st_size),
        "credential file must be an owned private bounded regular file with one link"
    );
    Ok((file, ObjectIdentity::from_stat(&stat)?))
}

/// Saved destination metadata, unrelated to backend-global Profile settings.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GatewayProfile {
    pub id: String,
    pub revision: u64,
    #[serde(default)]
    pub kind: BackendKind,
    pub name: String,
    pub origin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<CredentialReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_state_dir: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ConnectionProfilesSnapshot {
    pub revision: u64,
    pub selected_profile_id: Option<String>,
    pub profiles: Vec<GatewayProfile>,
}
impl ConnectionProfilesSnapshot {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            revision: 0,
            selected_profile_id: None,
            profiles: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct GatewayProfileInput {
    kind: BackendKind,
    name: String,
    origin: Option<String>,
    credential: Option<CredentialReference>,
    local_state_dir: Option<PathBuf>,
}
impl GatewayProfileInput {
    /// Construct validated metadata for a remote HTTP(S) gateway profile.
    pub fn new(name: String, origin: String, credential: CredentialReference) -> Result<Self> {
        validate_name(&name)?;
        let origin = Client::canonical_origin(&origin)?;
        anyhow::ensure!(
            origin.len() <= 2048 && credential.origin == origin,
            "select a credential reference bound to this exact gateway origin"
        );
        Ok(Self {
            kind: BackendKind::Remote,
            name,
            origin: Some(origin),
            credential: Some(credential),
            local_state_dir: None,
        })
    }

    /// Construct validated metadata for a local embedded companion backend profile.
    pub fn new_local(
        name: String,
        origin: Option<String>,
        local_state_dir: Option<PathBuf>,
    ) -> Result<Self> {
        validate_name(&name)?;
        let origin = match origin {
            Some(origin) => Some(canonical_local_origin(&origin)?),
            None => None,
        };
        if let Some(dir) = &local_state_dir {
            validate_local_state_dir_path(dir)?;
        }
        Ok(Self {
            kind: BackendKind::Local,
            name,
            origin,
            credential: None,
            local_state_dir,
        })
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ConnectionCatalog {
    format_version: u16,
    installation_marker: String,
    root_object: ObjectIdentity,
    snapshot: ConnectionProfilesSnapshot,
}

impl NativeContainer {
    /// Read-only, including an absent catalog. Existing unmarked/unknown or
    /// unsafe documents require repair and are never overwritten or imported.
    pub fn connection_profiles(&self) -> Result<ConnectionProfilesSnapshot> {
        let _guard = self.locked_settings()?;
        Ok(self.read_connections()?.0.snapshot)
    }

    /// Provision one unambiguous exact-origin credential reference without token bytes.
    /// Explicit provisioning is also the only way to accept a replaced token file.
    pub fn provision_credential_binding(&self, origin: &str, path: &Path) -> Result<()> {
        let origin = Client::canonical_origin(origin)?;
        let credential = CredentialReference::capture(path, &origin)?;
        drop(credential.open_verified(&origin)?);
        let _guard = self.locked_settings()?;
        let (mut catalog, object) = self.read_connections()?;
        let matches: Vec<_> = catalog
            .snapshot
            .profiles
            .iter()
            .enumerate()
            .filter(|(_, profile)| profile.kind == BackendKind::Remote && profile.origin == origin)
            .map(|(index, _)| index)
            .collect();
        anyhow::ensure!(
            matches.len() <= 1,
            "ambiguous credential bindings require explicit catalog repair"
        );
        let revision = catalog
            .snapshot
            .revision
            .checked_add(1)
            .context("credential binding revision exhausted")?;
        let id = if let Some(index) = matches.first() {
            let profile = &mut catalog.snapshot.profiles[*index];
            profile.credential = Some(credential);
            profile.revision = revision;
            profile.id.clone()
        } else {
            anyhow::ensure!(
                catalog.snapshot.profiles.len() < MAX_PROFILES,
                "credential binding catalog is full"
            );
            let id = fs::random_marker()?;
            catalog.snapshot.profiles.push(GatewayProfile {
                id: id.clone(),
                revision,
                kind: BackendKind::Remote,
                name: format!("Gateway {}", &id[..12]),
                origin,
                credential: Some(credential),
                local_state_dir: None,
            });
            id
        };
        catalog.snapshot.revision = revision;
        catalog.snapshot.selected_profile_id = Some(id);
        self.write_connections(&catalog, object)
    }

    /// Resolve a single remote binding; no origin fallback, token scanning or file adoption.
    pub fn credential_binding(&self, origin: &str) -> Result<CredentialReference> {
        let origin = Client::canonical_origin(origin)?;
        let catalog = self.connection_profiles()?;
        let matches: Vec<_> = catalog
            .profiles
            .iter()
            .filter(|profile| profile.kind == BackendKind::Remote && profile.origin == origin)
            .collect();
        anyhow::ensure!(matches.len() == 1,
            "gateway credential missing or ambiguous; run --provision-gateway-credential --api-origin <URL> --api-token-file <private-file>");
        let credential = matches[0]
            .credential
            .as_ref()
            .context("remote gateway profile is missing its credential reference")?;
        drop(credential.open_verified(&origin)?);
        Ok(credential.clone())
    }

    /// Provision and verify a private `0o700` state directory for a local embedded profile.
    pub fn ensure_local_backend_state_dir(&self, profile_slug: &str) -> Result<PathBuf> {
        validate_local_slug(profile_slug)?;
        self.revalidate_root()?;
        let (base_fd, _base_identity) =
            ensure_private_child_directory(&self.root_fd, LOCAL_BACKEND_DIR)?;
        let (_profile_fd, _profile_identity) =
            ensure_private_child_directory(&base_fd, profile_slug)?;
        self.revalidate_root()?;
        Ok(self.root.join(LOCAL_BACKEND_DIR).join(profile_slug))
    }

    /// Validate or provision an explicit local backend state directory without symlinks or public access.
    pub fn validate_local_backend_state_dir(&self, path: &Path) -> Result<PathBuf> {
        validate_local_state_dir_path(path)?;
        self.revalidate_root()?;
        let parent_path = path
            .parent()
            .context("local backend state directory must have a parent")?;
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .context("local backend state directory name must be valid UTF-8")?;
        let parent_fd = fs::open_absolute_directory(parent_path)?;
        let (_dir_fd, _identity) = ensure_private_child_directory(&parent_fd, name)?;
        self.revalidate_root()?;
        Ok(path.to_path_buf())
    }

    /// Add/edit using an explicit catalog revision; no last-writer-wins overwrite.
    /// Run on a blocking worker. This does not connect, copy token bytes or alter
    /// workspace identities, bindings or backend-global settings.
    pub fn save_connection_profile(
        &self,
        expected_revision: u64,
        id: Option<&str>,
        input: GatewayProfileInput,
    ) -> Result<ConnectionProfilesSnapshot> {
        self.save_profile(expected_revision, id, true, |_| Ok(input))
    }

    /// Save a private Embedded folder as a startup target without changing the editor selection.
    /// Uses the same catalog revision and lock as connection profile edits.
    pub fn add_local_backend_folder(
        &self,
        expected_revision: u64,
        path: PathBuf,
    ) -> Result<ConnectionProfilesSnapshot> {
        self.save_profile(expected_revision, None, false, |snapshot| {
            validate_local_state_dir_path(&path)?;
            let default_path = self.root.join(LOCAL_BACKEND_DIR).join("embedded");
            let object = std::fs::symlink_metadata(&path)
                .ok()
                .map(|metadata| (metadata.dev(), metadata.ino()));
            let matches_path = |candidate: &Path| {
                path == candidate
                    || object.is_some_and(|object| {
                        std::fs::symlink_metadata(candidate)
                            .is_ok_and(|metadata| (metadata.dev(), metadata.ino()) == object)
                    })
            };
            anyhow::ensure!(
                !matches_path(&default_path),
                "This folder is already available as the default folder."
            );
            anyhow::ensure!(
                !snapshot.profiles.iter().any(|profile| {
                    if let Some(folder) = &profile.local_state_dir {
                        return matches_path(folder);
                    }
                    profile.origin.strip_prefix("local://").is_some_and(|slug| {
                        matches_path(&self.root.join(LOCAL_BACKEND_DIR).join(slug))
                    })
                }),
                "This folder is already saved."
            );
            let name = local_folder_name(&path, &snapshot.profiles)?;
            GatewayProfileInput::new_local(name, None, Some(path))
        })
    }

    fn save_profile(
        &self,
        expected_revision: u64,
        id: Option<&str>,
        select: bool,
        prepare_input: impl FnOnce(&ConnectionProfilesSnapshot) -> Result<GatewayProfileInput>,
    ) -> Result<ConnectionProfilesSnapshot> {
        let _guard = self.locked_settings()?;
        let (mut catalog, object) = self.read_connections()?;
        anyhow::ensure!(
            catalog.snapshot.revision == expected_revision,
            "saved connections changed; refresh before saving"
        );
        let input = prepare_input(&catalog.snapshot)?;
        anyhow::ensure!(
            !catalog
                .snapshot
                .profiles
                .iter()
                .any(|profile| Some(profile.id.as_str()) != id
                    && UniCase::new(profile.name.as_str()) == UniCase::new(input.name.as_str())),
            "a saved connection already has this name"
        );
        let revision = expected_revision
            .checked_add(1)
            .context("saved connection revision exhausted")?;
        let profile_id = match id {
            Some(id) => {
                anyhow::ensure!(
                    catalog
                        .snapshot
                        .profiles
                        .iter()
                        .any(|profile| profile.id == id),
                    "saved connection no longer exists"
                );
                id.to_owned()
            }
            None => {
                anyhow::ensure!(
                    catalog.snapshot.profiles.len() < MAX_PROFILES,
                    "saved connection catalog is full"
                );
                fs::random_marker()?
            }
        };
        let next_profile = match input.kind {
            BackendKind::Remote => {
                let origin = input
                    .origin
                    .context("remote gateway profile requires an origin")?;
                let credential = input
                    .credential
                    .context("remote gateway profile requires a credential reference")?;
                drop(credential.open_verified(&origin)?);
                GatewayProfile {
                    id: profile_id.clone(),
                    revision,
                    kind: BackendKind::Remote,
                    name: input.name,
                    origin,
                    credential: Some(credential),
                    local_state_dir: None,
                }
            }
            BackendKind::Local => {
                let origin = match input.origin {
                    Some(origin) => canonical_local_origin(&origin)?,
                    None => local_origin_for_profile(&profile_id)?,
                };
                let state_dir = match input.local_state_dir {
                    Some(dir) => self.validate_local_backend_state_dir(&dir).context(
                        "Choose a private folder you own or enter the path to a new subfolder",
                    )?,
                    None => self.ensure_local_backend_state_dir(&profile_id)?,
                };
                GatewayProfile {
                    id: profile_id.clone(),
                    revision,
                    kind: BackendKind::Local,
                    name: input.name,
                    origin,
                    credential: None,
                    local_state_dir: Some(state_dir),
                }
            }
        };
        match id {
            Some(existing_id) => {
                let profile = catalog
                    .snapshot
                    .profiles
                    .iter_mut()
                    .find(|profile| profile.id == existing_id)
                    .context("saved connection no longer exists")?;
                anyhow::ensure!(
                    profile.kind == next_profile.kind,
                    "saved destination cannot switch between remote gateway and local embedded"
                );
                *profile = next_profile;
            }
            None => catalog.snapshot.profiles.push(next_profile),
        }
        catalog.snapshot.revision = revision;
        if select {
            catalog.snapshot.selected_profile_id = Some(profile_id);
        }
        self.write_connections(&catalog, object)?;
        Ok(catalog.snapshot)
    }

    /// Remember editor selection only. Loading a selection never connects or
    /// selects a workspace implicitly.
    pub fn select_connection_profile(
        &self,
        expected_revision: u64,
        id: &str,
    ) -> Result<ConnectionProfilesSnapshot> {
        let _guard = self.locked_settings()?;
        let (mut catalog, object) = self.read_connections()?;
        anyhow::ensure!(
            catalog.snapshot.revision == expected_revision,
            "saved connections changed; refresh before selecting"
        );
        anyhow::ensure!(
            catalog
                .snapshot
                .profiles
                .iter()
                .any(|profile| profile.id == id),
            "saved connection no longer exists"
        );
        if catalog.snapshot.selected_profile_id.as_deref() == Some(id) {
            return Ok(catalog.snapshot);
        }
        catalog.snapshot.revision = expected_revision
            .checked_add(1)
            .context("saved connection revision exhausted")?;
        catalog.snapshot.selected_profile_id = Some(id.to_owned());
        self.write_connections(&catalog, object)?;
        Ok(catalog.snapshot)
    }

    fn read_connections(&self) -> Result<(ConnectionCatalog, Option<ObjectIdentity>)> {
        self.revalidate_root()?;
        match rustix::fs::statat(&self.root_fd, CONNECTIONS_FILE, AtFlags::SYMLINK_NOFOLLOW) {
            Err(rustix::io::Errno::NOENT) => Ok((
                ConnectionCatalog {
                    format_version: FORMAT,
                    installation_marker: self.installation_marker.clone(),
                    root_object: self.catalog_root_object,
                    snapshot: ConnectionProfilesSnapshot::empty(),
                },
                None,
            )),
            Ok(_) => {
                let (catalog, object) = fs::read_json_limited::<ConnectionCatalog>(
                    &self.root_fd,
                    CONNECTIONS_FILE,
                    MAX_CATALOG_BYTES,
                )?;
                self.validate_connections(&catalog)?;
                Ok((catalog, Some(object)))
            }
            Err(error) => Err(error).context("inspect protected connection catalog"),
        }
    }
    fn write_connections(
        &self,
        catalog: &ConnectionCatalog,
        object: Option<ObjectIdentity>,
    ) -> Result<()> {
        self.validate_connections(catalog)?;
        anyhow::ensure!(
            serde_json::to_vec(catalog)?.len() as u64 <= MAX_CATALOG_BYTES,
            "saved connection catalog exceeds its size limit"
        );
        fs::write_json(&self.root_fd, CONNECTIONS_FILE, catalog, object)?;
        self.revalidate_root()
    }
    fn validate_connections(&self, catalog: &ConnectionCatalog) -> Result<()> {
        anyhow::ensure!(
            catalog.format_version == FORMAT
                && catalog.installation_marker == self.installation_marker
                && catalog.root_object == self.catalog_root_object
                && catalog.snapshot.profiles.len() <= MAX_PROFILES,
            "saved connection ownership or format needs repair"
        );
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        for profile in &catalog.snapshot.profiles {
            validate_name(&profile.name)?;
            anyhow::ensure!(
                super::valid_marker(&profile.id)
                    && ids.insert(profile.id.clone())
                    && profile.revision > 0
                    && profile.revision <= catalog.snapshot.revision
                    && names.insert(UniCase::new(profile.name.as_str()).to_folded_case())
                    && profile.origin.len() <= 2048,
                "saved connection identity, name or destination conflicts"
            );
            match profile.kind {
                BackendKind::Remote => {
                    anyhow::ensure!(
                        profile.local_state_dir.is_none()
                            && Client::canonical_origin(&profile.origin)? == profile.origin,
                        "remote connection destination conflicts with local state metadata"
                    );
                    let credential = profile
                        .credential
                        .as_ref()
                        .context("remote saved connection is missing a credential reference")?;
                    anyhow::ensure!(
                        credential.origin == profile.origin,
                        "saved connection identity, name or destination conflicts"
                    );
                    credential.validate(self.root_object.owner)?;
                }
                BackendKind::Local => {
                    anyhow::ensure!(
                        profile.credential.is_none()
                            && canonical_local_origin(&profile.origin)? == profile.origin,
                        "local embedded connection must not store remote deployment credentials"
                    );
                    if let Some(dir) = &profile.local_state_dir {
                        validate_local_state_dir_path(dir)?;
                    }
                }
            }
        }
        anyhow::ensure!(
            catalog
                .snapshot
                .selected_profile_id
                .as_ref()
                .is_none_or(|id| ids.contains(id)),
            "selected saved connection is missing"
        );
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<()> {
    anyhow::ensure!(
        !name.is_empty()
            && name.len() <= MAX_PROFILE_NAME_BYTES
            && name.trim() == name
            && !name.chars().any(char::is_control),
        "connection name must be 1–128 UTF-8 bytes without control or edge whitespace"
    );
    Ok(())
}

fn local_folder_name(path: &Path, profiles: &[GatewayProfile]) -> Result<String> {
    let basename = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("Embedded folder must have a UTF-8 name")?
        .trim();
    let basename = if basename.is_empty() {
        "Embedded folder"
    } else {
        basename
    };
    for number in 1..=MAX_PROFILES + 1 {
        let suffix = if number == 1 {
            String::new()
        } else {
            format!(" ({number})")
        };
        let limit = basename.floor_char_boundary(MAX_PROFILE_NAME_BYTES - suffix.len());
        let name = format!("{}{suffix}", basename[..limit].trim_end());
        if !profiles
            .iter()
            .any(|profile| UniCase::new(profile.name.as_str()) == UniCase::new(name.as_str()))
        {
            return Ok(name);
        }
    }
    anyhow::bail!("No unused name is available for this Embedded folder")
}

#[cfg(test)]
mod tests;
