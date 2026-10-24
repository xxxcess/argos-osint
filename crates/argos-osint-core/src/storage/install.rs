//! `argos storage install` (P4; design.md "Install, never silently").
//!
//! Downloads `surreal-v3.3.4.<asset>.tgz` from the official GitHub release
//! (HTTPS, github.com only), or takes a local archive with `--from`, checks
//! the archive's SHA-256 against the pin compiled in here, unpacks the
//! `surreal` binary, checks that it reports 3.3.4, and installs it as
//! `bin/surreal-3.3.4` atomically (temp file, fsync, rename). Ordinary
//! startup never calls this.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};

use super::home::{Layout, SERVER_VERSION};

/// (os, arch, asset, archive SHA-256), confirmed three ways on 2026-10-22.
pub const PINS: &[(&str, &str, &str, &str)] = &[
    (
        "linux",
        "x86_64",
        "linux-amd64",
        "6593071833ad9c85cf533cf70b1643eb08966c868f8624001e30839ea29a2689",
    ),
    (
        "linux",
        "aarch64",
        "linux-arm64",
        "978262fd9d1f3adfc5791031663855baf0a3cf8a89883428109734ba5a065811",
    ),
    (
        "macos",
        "aarch64",
        "darwin-arm64",
        "d0b0afc47948a46ceac9db178d2a10a7eb21e1c4f219a74b3573c850122ed9fb",
    ),
    (
        "macos",
        "x86_64",
        "darwin-amd64",
        "0723b555340da2cee1e0cff6f95552438536eab8d984b91f227b3ce2d52a5006",
    ),
];

/// `(asset, sha256)` for this host.
pub fn host_pin() -> Result<(&'static str, &'static str)> {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    PINS.iter()
        .find(|p| p.0 == os && p.1 == arch)
        .map(|p| (p.2, p.3))
        .ok_or_else(|| anyhow!("no pinned SurrealDB {SERVER_VERSION} build for {os}/{arch}"))
}

pub fn download_url(asset: &str) -> String {
    format!(
        "https://github.com/surrealdb/surrealdb/releases/download/v{SERVER_VERSION}/surreal-v{SERVER_VERSION}.{asset}.tgz"
    )
}

/// Hex SHA-256 of a file, streamed.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Verify `archive` against `expected`, unpack it and install the binary.
pub fn install_archive(layout: &Layout, archive: &Path, expected: &str) -> Result<PathBuf> {
    let actual = sha256_file(archive)?;
    if actual != expected {
        bail!(
            "{} has SHA-256 {actual}, expected {expected}; not installing",
            archive.display()
        );
    }
    let target = layout.server_bin();
    let dir = target
        .parent()
        .ok_or_else(|| anyhow!("bad bin path"))?
        .to_path_buf();
    std::fs::create_dir_all(&dir)?;
    let unpack = tempfile::tempdir_in(&dir)?;
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(unpack.path())
        .status()
        .context("run tar")?;
    if !status.success() {
        bail!("could not unpack {} ({status})", archive.display());
    }
    let unpacked = unpack.path().join("surreal");
    if !unpacked.is_file() {
        bail!("{} has no `surreal` binary", archive.display());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&unpacked, std::fs::Permissions::from_mode(0o755))?;
    }
    let out = Command::new(&unpacked)
        .arg("version")
        .output()
        .context("run surreal version")?;
    let reported = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || !reported.starts_with(SERVER_VERSION) {
        bail!("unpacked server reports {reported:?}, expected {SERVER_VERSION}");
    }
    std::fs::File::open(&unpacked)?.sync_all()?;
    std::fs::rename(&unpacked, &target)?;
    #[cfg(unix)]
    std::fs::File::open(&dir)?.sync_all()?;
    Ok(target)
}

/// `argos storage install [--from <archive>]`.
pub async fn install(layout: &Layout, from: Option<&Path>) -> Result<PathBuf> {
    let (asset, sha) = host_pin()?;
    if let Some(archive) = from {
        return install_archive(layout, archive, sha);
    }
    let url = download_url(asset);
    let bin_dir = layout.home.join("bin");
    std::fs::create_dir_all(&bin_dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(&bin_dir)?;
    let mut resp = reqwest::Client::builder()
        .https_only(true)
        .build()?
        .get(&url)
        .send()
        .await?
        .error_for_status()
        .with_context(|| format!("download {url}"))?;
    while let Some(chunk) = resp.chunk().await? {
        tmp.write_all(&chunk)?;
    }
    tmp.as_file().sync_all()?;
    install_archive(layout, tmp.path(), sha)
}
