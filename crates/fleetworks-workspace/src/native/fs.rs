//! Descriptor-relative filesystem operations for the native container.

use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path::Component;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use rustix::fd::AsFd;
use rustix::fd::OwnedFd;
use rustix::fs::AtFlags;
use rustix::fs::FileType;
use rustix::fs::Mode;
use rustix::fs::OFlags;
use rustix::fs::RenameFlags;
use rustix::fs::Stat;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde::Serialize;

pub(super) const MAX_METADATA_BYTES: u64 = 1024 * 1024;
pub(super) const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
pub(super) const PRIVATE_DIRECTORY: Mode = Mode::RUSR.union(Mode::WUSR).union(Mode::XUSR);
const PRIVATE_FILE: Mode = Mode::RUSR.union(Mode::WUSR);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct ObjectIdentity {
    pub device: u64,
    pub inode: u64,
    pub owner: u32,
}

impl ObjectIdentity {
    pub fn from_stat(stat: &Stat) -> Result<Self> {
        Ok(Self {
            device: u64::try_from(stat.st_dev).context("invalid filesystem device")?,
            inode: stat.st_ino,
            owner: stat.st_uid,
        })
    }
}

pub(super) fn open_absolute_directory(path: &Path) -> Result<OwnedFd> {
    anyhow::ensure!(
        path.is_absolute() && path.parent().is_some(),
        "select an absolute owned home directory"
    );
    let mut current = rustix::fs::open("/", DIRECTORY_FLAGS, Mode::empty())?;
    for component in path.components() {
        match component {
            Component::RootDir => (),
            Component::Normal(name) => {
                current = rustix::fs::openat(&current, name, DIRECTORY_FLAGS, Mode::empty())
                    .context("home ancestors must be real directories without symlinks")?
            }
            _ => anyhow::bail!("home path cannot contain traversal or relative components"),
        }
    }
    Ok(current)
}

pub(super) fn directory_identity(fd: impl AsFd, private: bool) -> Result<ObjectIdentity> {
    let stat = rustix::fs::fstat(fd)?;
    anyhow::ensure!(
        FileType::from_raw_mode(stat.st_mode) == FileType::Directory,
        "owned directory required"
    );
    anyhow::ensure!(
        stat.st_uid == rustix::process::geteuid().as_raw(),
        "directory owner does not match the current user"
    );
    let mask = if private { 0o077 } else { 0o022 };
    anyhow::ensure!(
        stat.st_mode & mask == 0,
        "directory permissions require repair"
    );
    ObjectIdentity::from_stat(&stat)
}

pub(super) fn entry_identity(fd: impl AsFd, name: &str) -> Result<Option<ObjectIdentity>> {
    match rustix::fs::statat(fd, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => {
            anyhow::ensure!(
                FileType::from_raw_mode(stat.st_mode) == FileType::Directory,
                "workspace path must be a real directory, not a symlink or file"
            );
            ObjectIdentity::from_stat(&stat).map(Some)
        }
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(error) => Err(error).context("inspect owned child directory"),
    }
}

pub(super) fn open_private_file(fd: impl AsFd, name: &str, create: bool) -> Result<File> {
    let mut flags = OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
    if create {
        flags |= OFlags::CREATE | OFlags::EXCL;
    }
    let file = File::from(
        rustix::fs::openat(fd, name, flags, PRIVATE_FILE)
            .context("open protected native metadata")?,
    );
    require_private_file(&file)?;
    Ok(file)
}

fn require_private_file(file: &File) -> Result<ObjectIdentity> {
    let stat = rustix::fs::fstat(file)?;
    anyhow::ensure!(
        FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile,
        "native metadata must be a regular file"
    );
    anyhow::ensure!(
        stat.st_uid == rustix::process::geteuid().as_raw()
            && stat.st_mode & 0o077 == 0
            && stat.st_nlink == 1,
        "native metadata ownership, permissions or links require repair"
    );
    ObjectIdentity::from_stat(&stat)
}

pub(super) fn read_json<T: DeserializeOwned>(
    fd: impl AsFd,
    name: &str,
) -> Result<(T, ObjectIdentity)> {
    let file = open_private_file(fd, name, false)?;
    let identity = require_private_file(&file)?;
    let mut body = Vec::new();
    file.take(MAX_METADATA_BYTES + 1).read_to_end(&mut body)?;
    anyhow::ensure!(
        body.len() as u64 <= MAX_METADATA_BYTES,
        "native metadata exceeds its bounded size"
    );
    let value = serde_json::from_slice(&body).context("native metadata needs explicit repair")?;
    Ok((value, identity))
}

pub(super) fn write_json<T: Serialize>(
    fd: impl AsFd,
    name: &str,
    value: &T,
    replace: Option<ObjectIdentity>,
) -> Result<ObjectIdentity> {
    let fd = fd.as_fd();
    let body = serde_json::to_vec(value)?;
    anyhow::ensure!(
        body.len() as u64 <= MAX_METADATA_BYTES,
        "native metadata exceeds its bounded size"
    );
    let temporary = format!(".native-write-{}.tmp", random_marker()?);
    let mut file = open_private_file(fd, &temporary, true)?;
    let write_result = (|| -> Result<()> {
        file.write_all(&body)?;
        file.sync_all()?;
        if let Some(expected) = replace {
            let current = open_private_file(fd, name, false)?;
            anyhow::ensure!(
                require_private_file(&current)? == expected,
                "native metadata was replaced during mutation"
            );
            rustix::fs::renameat(fd, &temporary, fd, name)?;
        } else {
            rustix::fs::renameat_with(fd, &temporary, fd, name, RenameFlags::NOREPLACE)?;
        }
        rustix::fs::fsync(fd)?;
        Ok(())
    })();
    if let Err(original) = &write_result {
        // Only our exclusive temporary regular file is unlinked; never recurse.
        match rustix::fs::unlinkat(fd, &temporary, AtFlags::empty()) {
            Ok(()) | Err(rustix::io::Errno::NOENT) => (),
            Err(cleanup) => {
                anyhow::bail!("{original:#}; native temporary-file cleanup failed: {cleanup}")
            }
        }
    }
    write_result?;
    require_private_file(&file)
}

pub(super) fn random_marker() -> Result<String> {
    use std::fmt::Write as _;
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random).map_err(|_| anyhow::anyhow!("generate native identity marker"))?;
    let mut text = String::with_capacity(64);
    for byte in random {
        write!(&mut text, "{byte:02x}").expect("String writes cannot fail");
    }
    Ok(text)
}
