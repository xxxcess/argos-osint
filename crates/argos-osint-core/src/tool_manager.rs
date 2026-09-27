//! Explicit managed installation. No setup is triggered by opening Providers.
use crate::research::run_process;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallPlan {
    pub tool: String,
    pub version: String,
    pub source: String,
    pub destination: PathBuf,
    pub prerequisites: Vec<String>,
    pub archive: String,
    pub checksums: String,
}
pub fn plan(tool: &str, root: &Path) -> Result<InstallPlan> {
    if tool != "katana" {
        bail!("Automatic setup unavailable for {tool}: output contract and isolated execution are not verified. Configure an isolated executable/container manually and use Verify. No system Python or runtime is installed.");
    }
    let os=match std::env::consts::OS {"macos"=>"macOS","linux"=>"linux",_=>bail!("Automatic Katana setup requires macOS or Linux; process-tree cleanup is unavailable on this platform")};
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        _ => bail!("Unsupported architecture; choose an official release manually"),
    };
    let version = "1.7.0";
    let archive = format!("katana_{version}_{os}_{arch}.zip");
    let source = format!("https://github.com/projectdiscovery/katana/releases/download/v{version}");
    Ok(InstallPlan {
        tool: tool.into(),
        version: version.into(),
        source: source.clone(),
        destination: root.join("katana"),
        prerequisites: vec![
            "/usr/bin/unzip".into(),
            "Public HTTPS to official GitHub release assets".into(),
        ],
        archive,
        checksums: format!("katana-{version}-checksums.txt"),
    })
}
pub async fn install(plan: &InstallPlan) -> Result<PathBuf> {
    use sha2::{Digest, Sha256};
    if !Path::new("/usr/bin/unzip").is_file() {
        bail!("unzip is unavailable. Install an archive utility manually; no system packages were modified");
    }
    tokio::fs::create_dir_all(&plan.destination).await?;
    let directory = plan.destination.clone();
    let staging = crate::workers::spawn_blocking(move || {
        tempfile::Builder::new()
            .prefix("candidate-")
            .tempdir_in(directory)
    })
    .await??;
    let checksums = crate::search::request(
        reqwest::Method::GET,
        &format!("{}/{}", plan.source, plan.checksums),
        None,
        "text/plain",
        &[],
    )
    .await
    .map_err(anyhow::Error::msg)?;
    if checksums.status != 200 {
        bail!(
            "Official checksum download failed (HTTP {})",
            checksums.status
        );
    }
    let archive = crate::search::request_limited(
        reqwest::Method::GET,
        &format!("{}/{}", plan.source, plan.archive),
        None,
        "application/octet-stream",
        &[],
        80_000_000,
    )
    .await
    .map_err(anyhow::Error::msg)?;
    if archive.status != 200 {
        bail!("Official release download failed (HTTP {})", archive.status);
    }
    let expected = String::from_utf8(checksums.bytes)?
        .lines()
        .find_map(|line| {
            let mut p = line.split_whitespace();
            let digest = p.next()?;
            let name = p.next()?;
            (name == plan.archive || name.trim_start_matches('*') == plan.archive)
                .then(|| digest.to_string())
        })
        .ok_or_else(|| {
            anyhow::anyhow!("Official checksum file does not list the selected archive")
        })?;
    let digest_bytes = archive.bytes.clone();
    let actual =
        crate::workers::spawn_blocking(move || format!("{:x}", Sha256::digest(digest_bytes)))
            .await?;
    if expected != actual {
        bail!("Official SHA-256 checksum mismatch; previous installation remains active");
    }
    let zip = staging.path().join("release.zip");
    tokio::fs::write(&zip, &archive.bytes).await?;
    // Extract only the known binary member, never arbitrary archive paths.
    run_process(
        Path::new("/usr/bin/unzip"),
        &[
            "-j".into(),
            zip.to_string_lossy().into(),
            "katana".into(),
            "-d".into(),
            staging.path().to_string_lossy().into(),
        ],
        Duration::from_secs(30),
        16384,
        &[],
    )
    .await?;
    let binary = staging.path().join("katana");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).await?;
    }
    let version = run_process(
        &binary,
        &["-version".into()],
        Duration::from_secs(10),
        32768,
        &[],
    )
    .await?;
    let version_text = String::from_utf8_lossy(&version.stdout).to_string()
        + &String::from_utf8_lossy(&version.stderr);
    if !version_text.contains(&plan.version) {
        bail!("Downloaded executable version mismatch; previous installation preserved");
    }
    let help = run_process(&binary, &["-h".into()], Duration::from_secs(10), 64000, &[]).await?;
    let text =
        String::from_utf8_lossy(&help.stdout).to_string() + &String::from_utf8_lossy(&help.stderr);
    for capability in ["-jsonl", "-crawl-scope", "-depth"] {
        if !text.contains(capability) {
            bail!("Installed Katana lacks required {capability} capability; previous installation preserved");
        }
    }
    let manifest = serde_json::to_vec(plan)?;
    tokio::fs::write(staging.path().join(".argos-managed.json"), manifest).await?;
    let activated = plan
        .destination
        .join(crate::store::new_id(&format!("v{}", plan.version)));
    tokio::fs::rename(staging.path(), &activated).await?;
    Ok(activated.join("katana"))
}
/// Removal refuses external paths, links and installations without an Argos manifest.
pub async fn remove(root: &Path, executable: &Path) -> Result<()> {
    let managed = tokio::fs::canonicalize(root).await?;
    let directory = executable
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Executable directory missing"))?;
    if tokio::fs::symlink_metadata(directory)
        .await?
        .file_type()
        .is_symlink()
    {
        bail!("Refusing linked installation directory");
    }
    let canonical = tokio::fs::canonicalize(directory).await?;
    if canonical == managed || !canonical.starts_with(managed.join("katana")) {
        bail!("Only Argos-managed installations can be removed");
    }
    let bytes = tokio::fs::read(canonical.join(".argos-managed.json")).await?;
    let manifest: InstallPlan = serde_json::from_slice(&bytes)?;
    if manifest.tool != "katana" {
        bail!("Managed installation manifest does not match");
    }
    tokio::fs::remove_dir_all(canonical).await?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn removal_refuses_user_installation() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        assert!(remove(dir.path(), &other.path().join("katana"))
            .await
            .is_err());
        assert!(other.path().exists());
    }
    #[test]
    fn plans_are_pinned_and_unsupported_tools_are_actionable() {
        let p = plan("katana", Path::new("/tmp/tools")).unwrap();
        assert_eq!(p.version, "1.7.0");
        assert!(p.source.contains("/v1.7.0"));
        assert!(plan("mosint", Path::new("/tmp/tools"))
            .unwrap_err()
            .to_string()
            .contains("unavailable"));
    }
}
