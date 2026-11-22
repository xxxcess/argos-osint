//! Operator entry points for the classifier workspace.

use std::future::Future;
use std::path::PathBuf;

use anyhow::Result;
use argos_osint_core::storage::repo::Repository;

pub(crate) enum Command {
    Revision,
    Activate {
        frozen_sha256: String,
        evaluation_sha256: String,
        expected_revision: String,
    },
}

pub(crate) enum ReviewCommand {
    Export {
        cycle: String,
    },
    Import {
        file: PathBuf,
        reference: String,
    },
    Recalibrate {
        cycles: Vec<String>,
        expected_revision: String,
    },
}

pub(crate) async fn run(
    workspace: PathBuf,
    command: Command,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    #[cfg(not(feature = "classifier"))]
    {
        let _ = (workspace, shutdown);
        if let Command::Activate {
            frozen_sha256,
            evaluation_sha256,
            expected_revision,
        } = command
        {
            let _ = (frozen_sha256, evaluation_sha256, expected_revision);
        }
        anyhow::bail!("this build does not include Atlas BERT support")
    }
    #[cfg(feature = "classifier")]
    {
        use std::sync::atomic::Ordering;

        use argos_osint_core::classifier::{bert_activation, WorkControl};

        match command {
            Command::Revision => {
                println!("{}", bert_activation::revision(&workspace).await?);
            }
            Command::Activate {
                frozen_sha256,
                evaluation_sha256,
                expected_revision,
            } => {
                let control = WorkControl::new();
                let work = bert_activation::activate(
                    &workspace,
                    &frozen_sha256,
                    &evaluation_sha256,
                    &expected_revision,
                    &control,
                );
                tokio::pin!(work);
                let active = tokio::select! {
                    biased;
                    () = shutdown => {
                        control.cancel.store(true, Ordering::Release);
                        work.await
                    }
                    result = &mut work => result,
                }?;
                println!("{}", serde_json::to_string(&active)?);
            }
        }
        Ok(())
    }
}

pub(crate) async fn run_review(
    workspace: PathBuf,
    repo: Repository,
    command: ReviewCommand,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    #[cfg(not(feature = "classifier"))]
    {
        let _ = (workspace, repo, shutdown);
        match command {
            ReviewCommand::Export { cycle } => {
                let _ = cycle;
            }
            ReviewCommand::Import { file, reference } => {
                let _ = (file, reference);
            }
            ReviewCommand::Recalibrate {
                cycles,
                expected_revision,
            } => {
                let _ = (cycles, expected_revision);
            }
        }
        anyhow::bail!("this build does not include Atlas BERT support")
    }
    #[cfg(feature = "classifier")]
    {
        use std::sync::atomic::Ordering;

        use argos_osint_core::classifier::readiness::ClassifierHome;
        use argos_osint_core::classifier::{bert_activation, WorkControl};
        use argos_osint_core::storage::classifier_review;

        match command {
            ReviewCommand::Export { cycle } => {
                let review = classifier_review::export_completed_cycle(&repo, &cycle).await?;
                println!("{}", serde_json::to_string(&review)?);
            }
            ReviewCommand::Import { file, reference } => {
                let review = tokio::task::spawn_blocking(move || read_review_file(&file)).await??;
                let home = ClassifierHome::new(workspace);
                let imported = classifier_review::import(&repo, &home, &review, &reference).await?;
                println!("{}", serde_json::json!({ "imported": imported }));
            }
            ReviewCommand::Recalibrate {
                cycles,
                expected_revision,
            } => {
                let review = classifier_review::recalibration_cohort(&repo, cycles).await?;
                let control = WorkControl::new();
                let work =
                    bert_activation::recalibrate(&workspace, &expected_revision, review, &control);
                tokio::pin!(work);
                let outcome = tokio::select! {
                    biased;
                    () = shutdown => {
                        control.cancel.store(true, Ordering::Release);
                        work.await
                    }
                    result = &mut work => result,
                }?;
                println!("{}", serde_json::to_string(&outcome)?);
            }
        }
        Ok(())
    }
}

#[cfg(feature = "classifier")]
fn read_review_file(
    path: &std::path::Path,
) -> Result<argos_osint_core::classifier::review::ReviewExchange> {
    use std::fs::OpenOptions;
    use std::io::Read;

    use anyhow::ensure;
    use argos_osint_core::classifier::review::MAX_REVIEW_BYTES;

    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(not(unix))]
    ensure!(
        std::fs::symlink_metadata(path)?.is_file(),
        "review file must be a regular file"
    );

    let file = options.open(path)?;
    let metadata = file.metadata()?;
    let max_bytes = MAX_REVIEW_BYTES as u64;
    ensure!(
        metadata.is_file() && (1..=max_bytes).contains(&metadata.len()),
        "review file must be a nonempty regular file of at most {MAX_REVIEW_BYTES} bytes"
    );
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(max_bytes + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 == metadata.len(),
        "review file length changed while reading"
    );
    Ok(serde_json::from_slice(&bytes)?)
}
