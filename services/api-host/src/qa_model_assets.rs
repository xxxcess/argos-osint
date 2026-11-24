//! Provision pinned model data independently of the verified worker and library pack.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{ensure, Context, Result};
use fleetworks_contracts::qa_worker::{QaModelTarget, QA_MODEL_PINS};
use reqwest::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::sync::Mutex;

const BLOCK_BYTES: usize = 1024 * 1024;
static PROVISIONING: Mutex<()> = Mutex::const_new(());

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelPin {
    repository: String,
    revision: String,
    files: BTreeMap<String, AssetPin>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetPin {
    bytes: u64,
    sha256: String,
}

struct StagedAsset(PathBuf);

impl Drop for StagedAsset {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn model_pin(target: QaModelTarget) -> Result<ModelPin> {
    let mut pins: BTreeMap<String, ModelPin> = serde_json::from_str(QA_MODEL_PINS)?;
    let pin = pins
        .remove(target.slug())
        .context("unknown QA model target")?;
    ensure!(
        pin.revision == target.revision(),
        "QA model revision mismatch"
    );
    Ok(pin)
}

/// Trusted generator filenames and SHA-256 digests for verified full packs.
pub(crate) fn expected_files(target: QaModelTarget) -> Result<BTreeMap<String, String>> {
    Ok(model_pin(target)?
        .files
        .into_iter()
        .map(|(name, pin)| (name, pin.sha256))
        .collect())
}

/// Return a private directory with the verified files used by this model.
/// Complete cached assets survive retries. Dropping the future removes partial downloads.
pub(crate) async fn provision(state_dir: &Path, target: QaModelTarget) -> Result<PathBuf> {
    let pin = model_pin(target)?;

    let _permit = PROVISIONING.lock().await;
    check_private_dir(state_dir).await?;
    let cache = state_dir.join("qa-model-assets");
    private_dir(&cache).await?;
    let model = cache.join(target.slug());
    private_dir(&model).await?;

    let mut client = None;
    for (name, asset) in &pin.files {
        let destination = model.join(name);
        if cached(&destination, asset).await? {
            continue;
        }
        if client.is_none() {
            client = Some(download_client()?);
        }
        let url = format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            pin.repository, pin.revision, name
        );
        download(
            client.as_ref().context("missing QA asset client")?,
            &url,
            &cache,
            &destination,
            asset,
        )
        .await?;
    }
    Ok(model)
}

async fn private_dir(path: &Path) -> Result<()> {
    match tokio::fs::DirBuilder::new().mode(0o700).create(path).await {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    check_private_dir(path).await
}

async fn check_private_dir(path: &Path) -> Result<()> {
    let metadata = tokio::fs::symlink_metadata(path).await?;
    ensure!(
        metadata.is_dir() && metadata.uid() == uid() && (metadata.mode() & 0o7777) == 0o700,
        "QA model directory must be owner-private"
    );
    Ok(())
}

fn uid() -> u32 {
    // SAFETY: geteuid has no arguments or memory effects.
    unsafe { libc::geteuid() }
}

async fn cached(path: &Path, asset: &AssetPin) -> Result<bool> {
    let file = match tokio::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .await
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata().await?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == uid()
            && metadata.nlink() == 1
            && (metadata.mode() & 0o7777) == 0o600,
        "QA model asset must be an owner-private regular file"
    );
    if metadata.len() != asset.bytes {
        return Ok(false);
    }
    let mut file = file.take(asset.bytes + 1);
    let mut digest = Sha256::new();
    let mut received = 0;
    let mut buffer = vec![0; BLOCK_BYTES];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        received += read as u64;
        digest.update(&buffer[..read]);
    }
    Ok(received == asset.bytes && format!("{:x}", digest.finalize()) == asset.sha256)
}

fn download_client() -> Result<Client> {
    let redirect = reqwest::redirect::Policy::custom(|attempt| {
        let url = attempt.url();
        if attempt.previous().len() >= 5
            || url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
        {
            attempt.error("QA asset redirect is outside the download policy")
        } else {
            attempt.follow()
        }
    });
    Ok(Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(redirect)
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(90))
        .timeout(Duration::from_secs(20 * 60))
        .build()?)
}

async fn download(
    client: &Client,
    url: &str,
    cache: &Path,
    destination: &Path,
    asset: &AssetPin,
) -> Result<()> {
    let mut response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(reqwest::Error::without_url)?;
    ensure!(
        response
            .content_length()
            .is_none_or(|bytes| bytes == asset.bytes),
        "QA asset size differs from its pin"
    );

    let path = cache.join(format!(".qa-asset-{}", argos_osint_core::store::new_id()));
    let (stage, file) = tokio::task::spawn_blocking(move || {
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        Ok::<_, io::Error>((StagedAsset(path), file))
    })
    .await??;
    let mut output = BufWriter::with_capacity(BLOCK_BYTES, tokio::fs::File::from_std(file));
    let mut digest = Sha256::new();
    let mut received = 0;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(reqwest::Error::without_url)?
    {
        received += chunk.len() as u64;
        ensure!(received <= asset.bytes, "QA asset exceeded its pinned size");
        digest.update(&chunk);
        output.write_all(&chunk).await?;
    }
    ensure!(
        received == asset.bytes && format!("{:x}", digest.finalize()) == asset.sha256,
        "QA asset differs from its pinned contents"
    );
    output.flush().await?;
    output.get_ref().sync_all().await?;
    drop(output);
    tokio::fs::rename(&stage.0, destination).await?;
    Ok(())
}
