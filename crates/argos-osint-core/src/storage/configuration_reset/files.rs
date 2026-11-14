//! Exact server-owned configuration payloads; no caller path or secret backup.

#[cfg(unix)]
use std::ffi::CString;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;

use super::ResetError;
use crate::config_commit;
use crate::config_commit::CommitJournal;
use crate::config_commit::ConfigLock;

const LIMIT: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct Object {
    dev: u64,
    ino: u64,
    uid: u32,
    size: u64,
    hash: String,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct Payload {
    name: String,
    before: Option<Object>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct Inventory {
    root: Object,
    payloads: Vec<Payload>,
    pub(super) revision: u64,
}

/// A provisioned configuration owner, created by server startup, never JSON.
#[derive(Clone)]
pub struct BackendFiles {
    root: PathBuf,
    uid: u32,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(unix)]
fn object(file: &File, hash: String) -> Result<Object, ResetError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata().map_err(|_| ResetError::Unavailable)?;
    Ok(Object {
        dev: metadata.dev(),
        ino: metadata.ino(),
        uid: metadata.uid(),
        size: metadata.len(),
        hash,
    })
}

fn canonical_body(name: &str) -> Vec<u8> {
    match name {
        "config.toml" => {
            let mut body = "configuration_format = 1\n".to_owned();
            for role in crate::provider::CANONICAL_ROLE_KEYS {
                body.push_str(&format!("\n[defaults.{role}]\nprovider = \"\"\nmodel = \"\"\naccount = \"\"\nfallbacks = []\n"));
            }
            body.into_bytes()
        }
        "auth.json" => b"{\"configuration_format\":1,\"accounts\":{}}\n".to_vec(),
        "quota.json" => b"{\"settings\":[]}\n".to_vec(),
        _ => Vec::new(),
    }
}

fn allowed(name: &str) -> bool {
    if matches!(name, "config.toml" | "auth.json" | "quota.json") {
        return true;
    }
    ["config.toml.", "auth.json.", "quota.json."]
        .iter()
        .any(|prefix| {
            name.strip_prefix(prefix)
                .and_then(|tail| tail.rsplit_once('.'))
                .is_some_and(|(generation, suffix)| {
                    !generation.is_empty()
                        && generation.bytes().all(|byte| byte.is_ascii_digit())
                        && matches!(suffix, "bak" | "staged")
                })
        })
}

impl BackendFiles {
    /// Bind the actual server-configured directory and its provisioning UID.
    /// Unsupported platforms stay closed rather than weakening object checks.
    pub fn new(root: PathBuf, expected_uid: u32) -> Result<Self, ResetError> {
        let value = Self {
            root,
            uid: expected_uid,
        };
        value.check_root()?;
        Ok(value)
    }

    #[cfg(unix)]
    fn check_root(&self) -> Result<Object, ResetError> {
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::fs::OpenOptionsExt;
        let metadata = fs::symlink_metadata(&self.root).map_err(|_| ResetError::Unavailable)?;
        if !self.root.is_absolute()
            || self.root.parent().is_none()
            || self
                .root
                .canonicalize()
                .map_err(|_| ResetError::Unavailable)?
                != self.root
            || !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.uid() != self.uid
            || metadata.mode() & 0o022 != 0
        {
            return Err(ResetError::UnsafeFiles);
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(&self.root)
            .map_err(|_| ResetError::UnsafeFiles)?;
        let identity = object(&file, String::new())?;
        if identity.dev != metadata.dev() || identity.ino != metadata.ino() {
            return Err(ResetError::UnsafeFiles);
        }
        Ok(identity)
    }

    #[cfg(not(unix))]
    fn check_root(&self) -> Result<Object, ResetError> {
        Err(ResetError::UnsupportedBuild)
    }

    pub(super) fn lock(&self) -> Result<ConfigLock, ResetError> {
        self.check_root()?;
        for name in [
            ".argos-config.guard",
            ".argos-config.lock",
            ".argos-config-journal.json",
        ] {
            if let Ok(metadata) = fs::symlink_metadata(self.root.join(name)) {
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err(ResetError::UnsafeFiles);
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if metadata.uid() != self.uid
                        || metadata.nlink() != 1
                        || metadata.mode() & 0o077 != 0
                    {
                        return Err(ResetError::UnsafeFiles);
                    }
                }
            }
        }
        let lock = ConfigLock::acquire_at(&self.root).map_err(|_| ResetError::Unavailable)?;
        if config_commit::read_journal_at(&self.root)
            .is_some_and(|journal| journal.state == "pending")
        {
            return Err(ResetError::UnsafeFiles);
        }
        self.check_root()?;
        Ok(lock)
    }

    #[cfg(unix)]
    fn open_payload(&self, name: &str, write: bool, create: bool) -> std::io::Result<File> {
        use std::os::fd::AsRawFd;
        use std::os::fd::FromRawFd;
        use std::os::unix::fs::OpenOptionsExt;
        if !allowed(name) {
            return Err(std::io::Error::other("reset payload not allowlisted"));
        }
        let expected = self
            .check_root()
            .map_err(|_| std::io::Error::other("configuration owner changed"))?;
        let root = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(&self.root)?;
        let opened = object(&root, String::new())
            .map_err(|_| std::io::Error::other("configuration owner unavailable"))?;
        if opened.dev != expected.dev || opened.ino != expected.ino {
            return Err(std::io::Error::other("configuration owner substituted"));
        }
        let name =
            CString::new(name).map_err(|_| std::io::Error::other("invalid configuration name"))?;
        let flags = (if write { libc::O_RDWR } else { libc::O_RDONLY })
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | (if create {
                libc::O_CREAT | libc::O_EXCL
            } else {
                0
            });
        // std has no safe openat equivalent. The root File retains this valid
        // directory descriptor; CString owns the allowlisted NUL-terminated
        // name for the call. A successful return is one newly owned descriptor.
        let descriptor = unsafe { libc::openat(root.as_raw_fd(), name.as_ptr(), flags, 0o600) };
        if descriptor < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // Ownership transfers exactly once from the successful openat call.
        Ok(unsafe { File::from_raw_fd(descriptor) })
    }

    #[cfg(unix)]
    fn read(&self, name: &str) -> Result<(Option<Object>, Vec<u8>), ResetError> {
        use std::os::unix::fs::MetadataExt;
        if !allowed(name) {
            return Err(ResetError::UnsafeFiles);
        }
        let mut file = match self.open_payload(name, false, false) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok((None, Vec::new()))
            }
            Err(_) => return Err(ResetError::UnsafeFiles),
        };
        let before = file.metadata().map_err(|_| ResetError::Unavailable)?;
        if !before.is_file()
            || before.uid() != self.uid
            || before.nlink() != 1
            || before.len() > LIMIT
        {
            return Err(ResetError::UnsafeFiles);
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ResetError::Unavailable)?;
        let after = file.metadata().map_err(|_| ResetError::Unavailable)?;
        if bytes.len() as u64 > LIMIT
            || !after.is_file()
            || after.uid() != self.uid
            || after.nlink() != 1
            || (
                before.mtime(),
                before.mtime_nsec(),
                before.ctime(),
                before.ctime_nsec(),
                before.len(),
            ) != (
                after.mtime(),
                after.mtime_nsec(),
                after.ctime(),
                after.ctime_nsec(),
                after.len(),
            )
        {
            return Err(ResetError::UnsafeFiles);
        }
        Ok((Some(object(&file, digest(&bytes))?), bytes))
    }

    #[cfg(not(unix))]
    fn read(&self, _: &str) -> Result<(Option<Object>, Vec<u8>), ResetError> {
        Err(ResetError::UnsupportedBuild)
    }

    pub(super) fn inventory(&self, lock: &ConfigLock) -> Result<Inventory, ResetError> {
        if lock.root() != self.root.as_path() {
            return Err(ResetError::UnsafeFiles);
        }
        let mut names = vec![
            "config.toml".to_owned(),
            "auth.json".to_owned(),
            "quota.json".to_owned(),
        ];
        for (count, entry) in fs::read_dir(&self.root)
            .map_err(|_| ResetError::Unavailable)?
            .enumerate()
        {
            if count >= 4096 {
                return Err(ResetError::UnsafeFiles);
            }
            let entry = entry.map_err(|_| ResetError::Unavailable)?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                return Err(ResetError::UnsafeFiles);
            };
            if name.starts_with(".argos-tmp-") {
                return Err(ResetError::UnreviewedSources);
            }
            if allowed(&name) && !names.contains(&name) {
                names.push(name);
            }
        }
        let mut payloads = Vec::new();
        for name in names {
            payloads.push(Payload {
                name: name.clone(),
                before: self.read(&name)?.0,
            });
        }
        Ok(Inventory {
            root: self.check_root()?,
            payloads,
            revision: lock
                .generation()
                .checked_add(1)
                .ok_or(ResetError::Unavailable)?,
        })
    }

    #[cfg(unix)]
    pub(super) fn clear(
        &self,
        lock: &ConfigLock,
        inventory: &Inventory,
        resuming: bool,
    ) -> Result<(), ResetError> {
        use std::os::unix::fs::PermissionsExt;
        let actual = self.check_root()?;
        if lock.root() != self.root.as_path()
            || actual.dev != inventory.root.dev
            || actual.ino != inventory.root.ino
            || actual.uid != inventory.root.uid
            || lock.generation() > inventory.revision
        {
            return Err(ResetError::UnsafeFiles);
        }
        for payload in &inventory.payloads {
            let (expected, bytes) = self.read(&payload.name)?;
            let fresh = canonical_body(&payload.name);
            let same = expected == payload.before;
            let same_inode = match (&expected, &payload.before) {
                (Some(a), Some(b)) => a.dev == b.dev && a.ino == b.ino && a.uid == b.uid,
                // A target absent from the frozen inventory has no prior
                // inode. Resume only canonical empty-output prefixes, never
                // unrelated bytes or newly entered credentials.
                (Some(_), None) => fresh.starts_with(&bytes),
                _ => false,
            };
            if !same && !(resuming && same_inode && fresh.starts_with(&bytes)) {
                return Err(ResetError::UnsafeFiles);
            }
            if bytes == fresh && expected.is_some() {
                continue;
            }
            let mut file = self
                .open_payload(&payload.name, true, expected.is_none())
                .map_err(|_| ResetError::UnsafeFiles)?;
            let opened = object(&file, String::new())?;
            {
                use std::os::unix::fs::MetadataExt;
                let metadata = file.metadata().map_err(|_| ResetError::Unavailable)?;
                if !metadata.is_file() || metadata.uid() != self.uid || metadata.nlink() != 1 {
                    return Err(ResetError::UnsafeFiles);
                }
            }
            if expected.as_ref().is_some_and(|expected| {
                opened.dev != expected.dev
                    || opened.ino != expected.ino
                    || opened.uid != expected.uid
            }) {
                return Err(ResetError::UnsafeFiles);
            }
            let mut current = Vec::new();
            Read::by_ref(&mut file)
                .take(LIMIT + 1)
                .read_to_end(&mut current)
                .map_err(|_| ResetError::Unavailable)?;
            if current != bytes {
                return Err(ResetError::UnsafeFiles);
            }
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|_| ResetError::Unavailable)?;
            file.set_len(0).map_err(|_| ResetError::Unavailable)?;
            file.seek(SeekFrom::Start(0))
                .map_err(|_| ResetError::Unavailable)?;
            file.write_all(&fresh)
                .and_then(|_| file.sync_all())
                .map_err(|_| ResetError::Unavailable)?;
            let (after, contents) = self.read(&payload.name)?;
            if after.is_none_or(|after| after.dev != opened.dev || after.ino != opened.ino)
                || contents != fresh
            {
                return Err(ResetError::UnsafeFiles);
            }
        }
        let now = chrono::Utc::now().to_rfc3339();
        config_commit::write_journal_at(
            &self.root,
            &CommitJournal {
                generation: inventory.revision,
                state: "committed".into(),
                started_at: now.clone(),
                finished_at: Some(now),
                error: None,
                files: Vec::new(),
            },
        )
        .map_err(|_| ResetError::Unavailable)?;
        crate::secrets::invalidate_auth_cache();
        config_commit::notify_reload();
        File::open(&self.root)
            .and_then(|file| file.sync_all())
            .map_err(|_| ResetError::Unavailable)?;
        Ok(())
    }

    #[cfg(not(unix))]
    pub(super) fn clear(&self, _: &ConfigLock, _: &Inventory, _: bool) -> Result<(), ResetError> {
        Err(ResetError::UnsupportedBuild)
    }
}
