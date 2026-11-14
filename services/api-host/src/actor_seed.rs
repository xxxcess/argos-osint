//! One-shot Brain actor catalog import from a pinned workspace snapshot.

use std::fs::{self, File};
use std::future::Future;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{ensure, Context, Result};
use argos_osint_core::actor_catalog::{self, Actor};
use argos_osint_core::storage::{actor_catalog::seed_actors_pinned, repo::Repository};
use sha2::{Digest, Sha256};

use crate::{fleetworks::NetworkStorage, Role};

// Keep the same ceiling as Venture's actor snapshot worker.
const MAX_ACTOR_CSV_BYTES: u64 = 256 * 1024 * 1024;

pub(crate) async fn run(
    file: PathBuf,
    expected_sha256: String,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    let expected = expected_sha256.clone();
    let prepare = tokio::task::spawn_blocking(move || -> Result<_> {
        let network = NetworkStorage::from_environment(Role::SeedActors)?;
        let actors = read_snapshot(&network.workspace, &file, &expected)?;
        Ok((network, actors))
    });
    tokio::pin!(shutdown);
    let (network, actors) = tokio::select! {
        biased;
        _ = &mut shutdown => anyhow::bail!("actor catalog seeding cancelled before writes"),
        result = prepare => result??,
    };
    let repo = tokio::select! {
        biased;
        _ = &mut shutdown => anyhow::bail!("actor catalog seeding cancelled before writes"),
        result = Repository::connect(&network.target, &network.namespace, &network.database) => result?,
    };
    let cancel = AtomicBool::new(false);
    let seed = seed_actors_pinned(&repo, actors, &expected_sha256, &cancel, |progress| {
        if progress.processed.is_multiple_of(2048) || progress.complete {
            eprintln!(
                "Actor catalog {}/{} actors, {} aliases",
                progress.processed, progress.total, progress.aliases
            );
        }
    });
    tokio::pin!(seed);
    let progress = tokio::select! {
        result = &mut seed => result?,
        _ = &mut shutdown => {
            cancel.store(true, Ordering::Release);
            seed.await?
        }
    };
    println!(
        "{}",
        serde_json::json!({
            "dataset_sha256": expected_sha256,
            "actors": progress.processed,
            "aliases": progress.aliases,
            "complete": progress.complete,
        })
    );
    Ok(())
}

fn read_snapshot(workspace: &Path, file: &Path, expected_sha256: &str) -> Result<Vec<Actor>> {
    ensure!(
        expected_sha256.len() == 64
            && expected_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "expected CSV SHA-256 must contain 64 lowercase hex characters"
    );
    ensure!(
        file.is_absolute(),
        "actor CSV snapshot path must be absolute"
    );
    let workspace = workspace
        .canonicalize()
        .context("resolve actor seed workspace")?;
    let datasets = workspace.join("datasets");
    ensure!(
        fs::symlink_metadata(&datasets)?.is_dir(),
        "workspace datasets must be a real directory"
    );
    let file = file.canonicalize().context("resolve actor CSV snapshot")?;
    ensure!(
        file.parent() == Some(datasets.as_path()),
        "actor CSV snapshot must be in workspace datasets"
    );
    ensure!(
        fs::metadata(&file)?.is_file(),
        "actor CSV snapshot must be a regular file"
    );
    let source = File::open(&file).context("open actor CSV snapshot")?;
    let metadata = source.metadata()?;
    ensure!(
        metadata.is_file() && (1..=MAX_ACTOR_CSV_BYTES).contains(&metadata.len()),
        "actor CSV snapshot must be a nonempty regular file within the 256 MiB limit"
    );
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    source
        .take(MAX_ACTOR_CSV_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        (1..=MAX_ACTOR_CSV_BYTES).contains(&(bytes.len() as u64)),
        "actor CSV snapshot is empty or exceeds the 256 MiB limit"
    );
    ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == expected_sha256,
        "actor CSV snapshot SHA-256 does not match the selected file"
    );
    actor_catalog::read(bytes.as_slice()).context("validate pinned actor CSV")
}
