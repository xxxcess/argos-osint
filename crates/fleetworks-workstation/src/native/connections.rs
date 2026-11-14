//! Local saved gateway destinations. Credential bytes never enter this catalog.

use std::collections::BTreeSet;
use std::fmt;
use std::fs::File;
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
const FORMAT: u16 = 1;
const MAX_PROFILES: usize = 32;
const MAX_CATALOG_BYTES: u64 = 256 * 1024;
const TOKEN_LIMIT: u64 = 4096;

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
    pub name: String,
    pub origin: String,
    pub credential: CredentialReference,
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
    name: String,
    origin: String,
    credential: CredentialReference,
}
impl GatewayProfileInput {
    pub fn new(name: String, origin: String, credential: CredentialReference) -> Result<Self> {
        validate_name(&name)?;
        let origin = Client::canonical_origin(&origin)?;
        anyhow::ensure!(
            origin.len() <= 2048 && credential.origin == origin,
            "select a credential reference bound to this exact gateway origin"
        );
        Ok(Self {
            name,
            origin,
            credential,
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
        let _guard = self.locked()?;
        Ok(self.read_connections()?.0.snapshot)
    }

    /// Provision one unambiguous exact-origin credential reference without token bytes.
    /// Explicit provisioning is also the only way to accept a replaced token file.
    pub fn provision_credential_binding(&self, origin: &str, path: &Path) -> Result<()> {
        let origin = Client::canonical_origin(origin)?;
        let credential = CredentialReference::capture(path, &origin)?;
        drop(credential.open_verified(&origin)?);
        let _guard = self.locked()?;
        let (mut catalog, object) = self.read_connections()?;
        let matches: Vec<_> = catalog
            .snapshot
            .profiles
            .iter()
            .enumerate()
            .filter(|(_, profile)| profile.origin == origin)
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
            profile.credential = credential;
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
                name: format!("Gateway {}", &id[..12]),
                origin,
                credential,
            });
            id
        };
        catalog.snapshot.revision = revision;
        catalog.snapshot.selected_profile_id = Some(id);
        self.write_connections(&catalog, object)
    }

    /// Resolve a single binding; no origin fallback, token scanning or file adoption.
    pub fn credential_binding(&self, origin: &str) -> Result<CredentialReference> {
        let origin = Client::canonical_origin(origin)?;
        let catalog = self.connection_profiles()?;
        let matches: Vec<_> = catalog
            .profiles
            .iter()
            .filter(|profile| profile.origin == origin)
            .collect();
        anyhow::ensure!(matches.len() == 1,
            "gateway credential missing or ambiguous; run --provision-gateway-credential --api-origin <URL> --api-token-file <private-file>");
        drop(matches[0].credential.open_verified(&origin)?);
        Ok(matches[0].credential.clone())
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
        let _guard = self.locked()?;
        let (mut catalog, object) = self.read_connections()?;
        anyhow::ensure!(
            catalog.snapshot.revision == expected_revision,
            "saved connections changed; refresh before saving"
        );
        drop(input.credential.open_verified(&input.origin)?);
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
        let id = match id {
            Some(id) => {
                let profile = catalog
                    .snapshot
                    .profiles
                    .iter_mut()
                    .find(|profile| profile.id == id)
                    .context("saved connection no longer exists")?;
                *profile = GatewayProfile {
                    id: id.to_owned(),
                    revision,
                    name: input.name,
                    origin: input.origin,
                    credential: input.credential,
                };
                id.to_owned()
            }
            None => {
                anyhow::ensure!(
                    catalog.snapshot.profiles.len() < MAX_PROFILES,
                    "saved connection catalog is full"
                );
                let id = fs::random_marker()?;
                catalog.snapshot.profiles.push(GatewayProfile {
                    id: id.clone(),
                    revision,
                    name: input.name,
                    origin: input.origin,
                    credential: input.credential,
                });
                id
            }
        };
        catalog.snapshot.revision = revision;
        catalog.snapshot.selected_profile_id = Some(id);
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
        let _guard = self.locked()?;
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
                    root_object: self.root_object,
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
                && catalog.root_object == self.root_object
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
                    && profile.origin.len() <= 2048
                    && Client::canonical_origin(&profile.origin)? == profile.origin
                    && profile.credential.origin == profile.origin,
                "saved connection identity, name or destination conflicts"
            );
            profile.credential.validate(self.root_object.owner)?;
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
            && name.len() <= 128
            && name.trim() == name
            && !name.chars().any(char::is_control),
        "connection name must be 1–128 UTF-8 bytes without control or edge whitespace"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
