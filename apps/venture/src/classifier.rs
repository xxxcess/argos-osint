//! Pin workspace CSVs before starting one-shot training or actor seed workers.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{ensure, Context, Result};
use fleetworks_workspace::Workspace;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

use crate::compose::{self, Owner};

const DATASET: &str = "argos_atlas_strategic_domains_v2.csv";
const MAX_CSV_BYTES: u64 = 128 * 1024 * 1024;
// The tracked actor catalog exceeds the Natural training limit.
const MAX_ACTOR_CSV_BYTES: u64 = 256 * 1024 * 1024;

struct PinnedCsv {
    snapshot: NamedTempFile,
    source: PathBuf,
    hash: String,
}

pub(crate) async fn train(
    workspace: &Workspace,
    file: Option<PathBuf>,
    activate: bool,
) -> Result<()> {
    run_csv_worker(workspace, file, "train-classifier", MAX_CSV_BYTES, activate).await
}

pub(crate) async fn seed_actors(workspace: &Workspace, file: PathBuf) -> Result<()> {
    run_csv_worker(
        workspace,
        Some(file),
        "seed-actors",
        MAX_ACTOR_CSV_BYTES,
        false,
    )
    .await
}

async fn run_csv_worker(
    workspace: &Workspace,
    file: Option<PathBuf>,
    role: &str,
    max_csv_bytes: u64,
    activate: bool,
) -> Result<()> {
    let owner = Owner::load(workspace)
        .await?
        .context("start the workspace with venture up before running a CSV worker")?;
    let workspace_for_input = workspace.clone();
    let input =
        tokio::task::spawn_blocking(move || snapshot(&workspace_for_input, file, max_csv_bytes))
            .await??;
    let relative = input.snapshot.path().strip_prefix(workspace.root())?;
    let container_csv = Path::new("/workspace").join(relative);
    eprintln!(
        "Selected {} (SHA-256 {})",
        input.source.display(),
        input.hash
    );
    let mut command = owner.compose();
    command
        .args([
            "run",
            "--rm",
            "--no-deps",
            "-T",
            "worker",
            "--role",
            role,
            "--file",
        ])
        .arg(container_csv)
        .args(["--expected-sha256", &input.hash])
        .stdin(Stdio::null());
    if activate {
        command.arg("--activate");
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("launch the {role} worker"))?;
    let status = tokio::select! {
        status = child.wait() => status?,
        _ = compose::shutdown_signal() => {
            #[cfg(unix)]
            if let Some(pid) = child.id() {
                let _ = tokio::process::Command::new("/bin/kill")
                    .args(["-TERM", &pid.to_string()])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .kill_on_drop(true)
                    .status()
                    .await;
            }
            match tokio::time::timeout(Duration::from_secs(30), child.wait()).await {
                Ok(status) => status?,
                Err(_) => {
                    child.kill().await?;
                    anyhow::bail!("{role} launcher did not acknowledge shutdown");
                }
            }
        }
    };
    // Keep the exact mounted bytes available until the one-shot worker exits.
    drop(input);
    ensure!(status.success(), "{role} worker exited with {status}");
    Ok(())
}

fn snapshot(workspace: &Workspace, file: Option<PathBuf>, max_csv_bytes: u64) -> Result<PinnedCsv> {
    let selected = match file {
        Some(path) => workspace.resolve_path(&path),
        None => {
            let environment = workspace.environment()?;
            if let Some(path) = environment.get("ARGOS_CLASSIFIER_CSV") {
                workspace.resolve_path(Path::new(&path))
            } else if let Some(path) = workspace.dataset("articles")? {
                path
            } else {
                let current = workspace.root().join("datasets").join(DATASET);
                let legacy = workspace.root().join("dataset").join(DATASET);
                if !current.try_exists()? && legacy.is_file() {
                    legacy
                } else {
                    current
                }
            }
        }
    };
    let source = selected.canonicalize().context("resolve workspace CSV")?;
    let relative = source
        .strip_prefix(workspace.root())
        .context("CSV must be in the workspace")?;
    ensure!(
        !relative.starts_with("secrets") && !relative.starts_with("storage"),
        "CSV must be outside workspace secrets and storage"
    );
    let datasets = workspace.root().join("datasets");
    ensure!(
        fs::symlink_metadata(&datasets)?.is_dir(),
        "workspace datasets must be a real directory"
    );
    ensure!(
        fs::metadata(&source)?.is_file(),
        "CSV must be a regular file"
    );
    let mut source_file = File::open(&source).context("open workspace CSV")?;
    let metadata = source_file.metadata()?;
    ensure!(
        metadata.is_file() && (1..=max_csv_bytes).contains(&metadata.len()),
        "CSV must be a regular file within the size limit"
    );
    let mut staged = tempfile::Builder::new()
        .prefix(".venture-csv-")
        .suffix(".csv")
        .tempfile_in(&datasets)?;
    let mut hash = Sha256::new();
    let mut length = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = source_file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        length += read as u64;
        ensure!(length <= max_csv_bytes, "CSV exceeds the size limit");
        staged.write_all(&buffer[..read])?;
        hash.update(&buffer[..read]);
    }
    let after = fs::metadata(&source)?;
    ensure!(
        length > 0
            && metadata.len() == length
            && after.len() == length
            && metadata.modified()? == after.modified()?,
        "CSV changed while creating its workspace snapshot"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        staged
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o400))?;
    }
    staged.as_file().sync_all()?;
    Ok(PinnedCsv {
        snapshot: staged,
        source,
        hash: format!("{:x}", hash.finalize()),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use fleetworks_workspace::Workspace;
    use sha2::{Digest, Sha256};

    use super::{snapshot, DATASET, MAX_CSV_BYTES};

    #[test]
    fn each_training_snapshot_pins_the_current_workspace_csv_bytes() {
        let temporary = tempfile::tempdir().unwrap();
        let workspace = Workspace::resolve(Some(temporary.path())).unwrap();
        workspace.initialize().unwrap();
        let file = workspace.root().join("datasets").join(DATASET);
        fs::write(&file, b"title\nfirst\n").unwrap();
        let first = snapshot(&workspace, Some(file.clone()), MAX_CSV_BYTES).unwrap();
        assert_eq!(
            first.hash,
            format!("{:x}", Sha256::digest(b"title\nfirst\n"))
        );

        fs::write(&file, b"title\nnext\n").unwrap();
        let second = snapshot(&workspace, Some(file), MAX_CSV_BYTES).unwrap();
        assert_ne!(first.hash, second.hash);
        assert_eq!(fs::read(first.snapshot.path()).unwrap(), b"title\nfirst\n");
        assert_eq!(fs::read(second.snapshot.path()).unwrap(), b"title\nnext\n");
    }
}
