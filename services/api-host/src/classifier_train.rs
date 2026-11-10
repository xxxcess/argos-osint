//! One-shot Natural classifier training. Atlas admission remains on Rust-BERT.

use std::fs::{self, File};
use std::future::Future;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, ensure, Context, Result};
use argos_osint_core::classifier::artifact;
use argos_osint_core::classifier::dataset::{DatasetRequest, MAX_CSV_BYTES};
use argos_osint_core::classifier::readiness::{
    ClassifierHome, ManualTrainRequest, TrainingJobState, TrainingOutcomeView,
};
use argos_osint_core::classifier::training::SyntheticActivation;
use argos_osint_core::classifier::Provenance;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::time::{interval, Instant, MissedTickBehavior};

pub(crate) async fn run(
    workspace: PathBuf,
    file: PathBuf,
    expected_sha256: String,
    activate: bool,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    ensure!(
        cfg!(feature = "classifier"),
        "Natural training requires a build with classifier support"
    );
    let expected = expected_sha256.clone();
    let dataset =
        tokio::task::spawn_blocking(move || prepare(workspace, file, &expected)).await??;
    let home = ClassifierHome::new(dataset.home.clone());
    let consent = if activate {
        SyntheticActivation::ExplicitOptIn
    } else {
        SyntheticActivation::RetainPrior
    };
    // Fresh jobs pick up current workspace bytes even when a path is reused.
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let request = ManualTrainRequest::new(
        dataset,
        format!("venture-{}-{now}", std::process::id()),
        consent,
    );
    let result = wait_for_training(&home, request, shutdown).await;
    let stopped = home.shutdown().await;
    let outcome = result?;
    stopped?;
    let Provenance::SyntheticBaseline { dataset_sha256, .. } = &outcome.report.provenance else {
        bail!("Natural training result is missing its CSV provenance");
    };
    ensure!(
        dataset_sha256 == &expected_sha256,
        "Natural classifier training report does not match the selected CSV"
    );
    let summary = json!({
        "classifier": "natural",
        "atlas_backend": "rust_bert_zero_shot",
        "dataset_sha256": dataset_sha256,
        "job_id": outcome.job_id,
        "state": outcome.state,
        "candidate_model_id": outcome.candidate_model_id,
        "active_model_id": outcome.active_model_id,
        "admission_thresholds_enforced": outcome.admission_thresholds_enforced,
        "synthetic_activation_requested": activate,
    });
    let mut output = io::stdout().lock();
    serde_json::to_writer(&mut output, &summary)?;
    writeln!(output)?;
    Ok(())
}

fn prepare(workspace: PathBuf, file: PathBuf, expected_sha256: &str) -> Result<DatasetRequest> {
    ensure!(
        expected_sha256.len() == 64
            && expected_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "expected CSV SHA-256 must contain 64 lowercase hex characters"
    );
    ensure!(
        workspace.is_absolute() && file.is_absolute(),
        "classifier inputs must be absolute"
    );
    let workspace = workspace
        .canonicalize()
        .context("resolve classifier workspace")?;
    let file = file.canonicalize().context("resolve classifier CSV")?;
    ensure!(
        workspace_relative(&workspace, &file)?.starts_with("datasets"),
        "classifier training snapshot must be in workspace datasets"
    );
    ensure!(
        fs::metadata(&file)?.is_file(),
        "classifier CSV must be a regular file"
    );
    let mut source = File::open(&file).context("open classifier CSV")?;
    let metadata = source.metadata()?;
    ensure!(metadata.is_file(), "classifier CSV must be a regular file");
    ensure!(
        hash_csv(&mut source)? == expected_sha256,
        "classifier CSV changed before training started"
    );

    let root = artifact::root_for(&workspace)?;
    let relative = workspace_relative(&workspace, &root)?;
    let mut ancestor = workspace.clone();
    for component in relative.components() {
        ancestor.push(component.as_os_str());
        match fs::symlink_metadata(&ancestor) {
            Ok(metadata) => ensure!(
                metadata.is_dir(),
                "classifier artifact path must contain only real directories"
            ),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error).context("inspect classifier artifact path"),
        }
    }

    let mut request = DatasetRequest::default();
    request.explicit_file = Some(file);
    request.cwd = workspace.clone();
    request.executable = std::env::current_exe()?;
    request.home = workspace;
    Ok(request)
}

fn workspace_relative<'a>(workspace: &Path, path: &'a Path) -> Result<&'a Path> {
    let relative = path
        .strip_prefix(workspace)
        .context("classifier paths must be inside the workspace")?;
    ensure!(
        !relative.as_os_str().is_empty()
            && relative
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
            && !relative.starts_with("secrets")
            && !relative.starts_with("storage"),
        "invalid classifier workspace path"
    );
    Ok(relative)
}

fn hash_csv(input: impl Read) -> Result<String> {
    let mut sha = Sha256::new();
    let length = io::copy(&mut input.take(MAX_CSV_BYTES + 1), &mut sha)?;
    ensure!(
        (1..=MAX_CSV_BYTES).contains(&length),
        "classifier CSV is empty or exceeds the training size limit"
    );
    Ok(format!("{:x}", sha.finalize()))
}

async fn wait_for_training(
    home: &ClassifierHome,
    request: ManualTrainRequest,
    shutdown: impl Future<Output = ()>,
) -> Result<TrainingOutcomeView> {
    tokio::pin!(shutdown);
    let start = home.start_training(request);
    tokio::pin!(start);
    let (accepted, cancel) = tokio::select! {
        result = &mut start => (result?, false),
        _ = &mut shutdown => (start.await?, true),
    };
    eprintln!("Natural training job {} accepted", accepted.job_id);
    let mut cancellation = None;
    if cancel {
        home.cancel_training(&accepted.job_id).await?;
        cancellation = Some(Instant::now());
    }
    let mut tick = interval(Duration::from_millis(250));
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut last_stage = None;
    let mut last_progress = Instant::now();
    loop {
        let job = tokio::select! {
            _ = &mut shutdown, if cancellation.is_none() => {
                cancellation = Some(Instant::now());
                home.cancel_training(&accepted.job_id).await?
            }
            _ = tick.tick() => home.job_detail(&accepted.job_id).await?,
        };
        match job.state {
            TrainingJobState::Completed => {
                return job
                    .outcome
                    .ok_or_else(|| anyhow!("Natural training completed without a report"));
            }
            TrainingJobState::Failed
            | TrainingJobState::Cancelled
            | TrainingJobState::Interrupted => {
                let code = job
                    .diagnostic
                    .as_ref()
                    .map(|diagnostic| diagnostic.code.as_str())
                    .unwrap_or("no_diagnostic");
                bail!("Natural classifier training {:?} ({code})", job.state);
            }
            _ => (),
        }
        ensure!(
            cancellation.is_none_or(|started| started.elapsed() < Duration::from_secs(20)),
            "Natural training cancellation is still pending for job {}",
            accepted.job_id
        );
        if let Some(progress) = job.progress {
            if last_stage != Some(progress.stage)
                || last_progress.elapsed() >= Duration::from_secs(5)
            {
                eprintln!(
                    "Natural classifier {} {}/{}",
                    progress.stage.as_str(),
                    progress.rows_seen,
                    progress.rows_total
                );
                last_stage = Some(progress.stage);
                last_progress = Instant::now();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{hash_csv, workspace_relative};

    #[test]
    fn classifier_hash_is_the_exact_nonempty_csv_bytes() {
        let expected = argos_osint_core::classifier::artifact::sha256_hex(b"title\r\nfixture\r\n");
        assert_eq!(hash_csv(&b"title\r\nfixture\r\n"[..]).unwrap(), expected);
        assert_ne!(hash_csv(&b"title\nfixture\n"[..]).unwrap(), expected);
        assert!(hash_csv(&b""[..]).is_err());
    }

    #[test]
    fn classifier_paths_stay_in_unmasked_workspace_directories() {
        let root = Path::new("/workspace");
        assert!(workspace_relative(root, Path::new("/workspace/models/classifier")).is_ok());
        for path in [
            "/elsewhere",
            "/workspace/secrets/model",
            "/workspace/storage/model",
            "/workspace/../elsewhere",
        ] {
            assert!(workspace_relative(root, Path::new(path)).is_err());
        }
    }
}
