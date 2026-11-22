//! Operator entry points for the classifier workspace.

use std::future::Future;
use std::path::PathBuf;

use anyhow::Result;

pub(crate) enum Command {
    Revision,
    Activate {
        frozen_sha256: String,
        evaluation_sha256: String,
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
