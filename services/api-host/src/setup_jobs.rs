//! Domain execution adapter for actually claimed durable setup tasks.
//!
//! The existing queue/dispatcher owns delivery/renewal/cancellation/terminal
//! status. This module neither spawns a process nor runs Compose. The caller
//! must signal cancellation and retain this future until the domain work stops.

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use argos_osint_api::datasets::DatasetStore;
#[cfg(test)]
use argos_osint_core::actor_catalog::Actor;
use argos_osint_core::classifier::dataset::DatasetRequest;
use argos_osint_core::classifier::readiness::ClassifierHome;
use argos_osint_core::classifier::readiness::ManualTrainRequest;
use argos_osint_core::classifier::readiness::TrainingJobState;
use argos_osint_core::classifier::training::SyntheticActivation;
use argos_osint_core::classifier::Provenance;
use argos_osint_core::storage::actor_catalog;
use argos_osint_core::storage::jobs::ClaimedTask;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::setup_jobs;
use fleetworks_contracts::readiness::DatasetRef;
use fleetworks_contracts::readiness::MaintenanceOperation;
use serde_json::json;
use sha2::Digest;
use sha2::Sha256;
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;

#[path = "native_bulk_pressure.rs"]
mod bulk_pressure;

/// Complete means the domain operation completed, not model activation or index visibility.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SetupOutcome {
    Complete,
    Interrupted,
}

/// Bounded public Jobs diagnostics; never return raw parser text, paths or IDs.
pub(crate) fn safe_failure_reason(error: &anyhow::Error) -> &'static str {
    for cause in error.chain() {
        match cause.to_string().as_str() {
            "worker_bulk_cancelled" => return "setup_interrupted",
            "worker_parse_resource_budget"
            | "worker_seed_resource_budget"
            | "worker_training_resource_budget" => return "setup_worker_resource_pressure",
            "worker_training_input_budget"
            | "actor_parse_memory_budget"
            | "actor_parse_raw_record_budget"
            | "actor_parse_row_budget"
            | "actor_parse_identity_expansion_budget"
            | "actor_spool_frame_budget"
            | "actor_spool_byte_budget"
            | "actor_seed_batch_budget"
            | "actor_seed_count_changed" => return "setup_dataset_resource_budget",
            _ => (),
        }
    }
    "native_domain_operation_failed"
}

pub(crate) async fn execute(
    repo: &Repository,
    workspace: PathBuf,
    store: &DatasetStore,
    task: &ClaimedTask,
    cancel: Arc<AtomicBool>,
) -> anyhow::Result<SetupOutcome> {
    let payload = setup_jobs::payload(repo, &task.job_id, &task.input_hash).await?;
    anyhow::ensure!(
        setup_jobs::operation_name(payload.operation) == task.operation
            && task.pool == task.operation,
        "setup_dispatch_operation_mismatch"
    );
    let dataset = payload
        .dataset
        .ok_or_else(|| anyhow::anyhow!("setup_dataset_required"))?;
    let path = store
        .path_for_job(repo, &task.job_id, &dataset)
        .await
        .context("setup_dataset_unavailable")?;
    let budget = bulk_pressure::await_admission(repo, task, &workspace, &cancel).await?;
    match payload.operation {
        MaintenanceOperation::SeedActors => {
            seed(repo, task, workspace, path, dataset, budget, cancel).await
        }
        MaintenanceOperation::TrainClassifier => {
            train(repo, task, workspace, path, dataset, budget, cancel).await
        }
        // Reindex belongs to the actual index coordinator, not CSV ingestion.
        _ => anyhow::bail!("setup_operation_requires_its_own_consumer"),
    }
}

struct VerifiedReader {
    file: File,
    hash: Sha256,
    size: u64,
    limit: u64,
}

impl Read for VerifiedReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let length = self.file.read(buffer)?;
        self.size = self
            .size
            .checked_add(length as u64)
            .ok_or_else(|| std::io::Error::other("dataset_size_exceeded"))?;
        if self.size > self.limit {
            return Err(std::io::Error::other("dataset_size_exceeded"));
        }
        self.hash.update(&buffer[..length]);
        Ok(length)
    }
}

fn verified_reader(path: &Path, dataset: &DatasetRef) -> anyhow::Result<VerifiedReader> {
    let metadata = std::fs::symlink_metadata(path)?;
    anyhow::ensure!(
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.len() == dataset.size_bytes,
        "setup_dataset_changed"
    );
    let file = File::open(path)?;
    let opened = file.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        anyhow::ensure!(
            (metadata.dev(), metadata.ino()) == (opened.dev(), opened.ino()),
            "setup_dataset_changed"
        );
    }
    Ok(VerifiedReader {
        file,
        hash: Sha256::new(),
        size: 0,
        limit: setup_jobs::dataset_limit(dataset.kind),
    })
}

fn verify_complete(reader: VerifiedReader, dataset: &DatasetRef) -> anyhow::Result<()> {
    anyhow::ensure!(
        reader.size == dataset.size_bytes
            && format!("{:x}", reader.hash.finalize()) == dataset.sha256.to_ascii_lowercase(),
        "setup_dataset_hash_mismatch"
    );
    Ok(())
}

#[cfg(test)]
fn read_actors(path: &Path, dataset: &DatasetRef) -> anyhow::Result<Vec<Actor>> {
    let mut reader = verified_reader(path, dataset)?;
    let actors = argos_osint_core::actor_catalog::read_bounded(
        &mut reader,
        &Default::default(),
        &AtomicBool::new(false),
        |_| Ok(()),
    )?;
    verify_complete(reader, dataset)?;
    Ok(actors)
}

async fn seed(
    repo: &Repository,
    task: &ClaimedTask,
    workspace: PathBuf,
    path: PathBuf,
    dataset: DatasetRef,
    budget: bulk_pressure::ParserBudget,
    cancel: Arc<AtomicBool>,
) -> anyhow::Result<SetupOutcome> {
    let execution = actor_catalog::SeedExecution::from_claimed(repo, task, &cancel).await?;
    let selected = dataset.clone();
    let validated = tokio::task::spawn_blocking({
        let cancel = Arc::clone(&cancel);
        let workspace = workspace.clone();
        let budget = budget.clone();
        move || bulk_pressure::scan_actors_checked(&path, &selected, &workspace, &budget, &cancel)
    })
    .await??;
    if cancel.load(Ordering::Acquire) {
        return Ok(SetupOutcome::Interrupted);
    }
    let total = validated.total;
    let (send_batch, batches) = mpsc::channel(1);
    let producer = tokio::task::spawn_blocking({
        let cancel = Arc::clone(&cancel);
        let workspace = workspace.clone();
        let budget = budget.clone();
        move || {
            bulk_pressure::produce_actor_batches(
                validated, &workspace, &budget, &cancel, send_batch,
            )
        }
    });
    let batches = futures_util::stream::unfold(batches, |mut receiver| async move {
        receiver.recv().await.map(|batch| (batch, receiver))
    });
    let (report, mut receive) = mpsc::channel(1);
    let mut operation = Box::pin(actor_catalog::seed_actors_stream_claimed(
        repo,
        total,
        batches,
        &execution,
        &cancel,
        move |progress| {
            let _ = report.try_send(progress);
        },
    ));
    let complete = loop {
        tokio::select! {
            result=&mut operation=>break result,
            Some(progress)=receive.recv()=>{
                let observation=tokio::task::spawn_blocking({let workspace=workspace.clone();let budget=budget.clone();move||budget.check_running(&workspace)}).await;
                if !matches!(observation,Ok(Ok(()))) {
                    argos_osint_core::cancellation::cancel_with_notify(&cancel);
                    let _=operation.as_mut().await;
                    break Err(anyhow::anyhow!("worker_seed_resource_budget"));
                }
                if let Err(error)=setup_jobs::record_progress(repo,task,"Seeding actors",progress.processed as u64,progress.total as u64).await {
                    argos_osint_core::cancellation::cancel_with_notify(&cancel);
                    // Retain the source future so owner-conditional partial
                    // cleanup closes only its already committed causal range.
                    let _=operation.as_mut().await;
                    break Err(error.into());
                }
            }
        }
    };
    if complete.is_err() {
        argos_osint_core::cancellation::cancel_with_notify(&cancel);
    }
    // Dropping the completed source future closes its bounded receiver before
    // joining, so a blocked producer always wakes on failure/cancellation.
    drop(operation);
    let produced = producer.await;
    let complete = complete?;
    produced.context("actor batch producer stopped")??;
    let phase = if complete.complete {
        "Database seeded"
    } else {
        "Seed incomplete"
    };
    setup_jobs::record_progress(
        repo,
        task,
        phase,
        complete.processed as u64,
        complete.total as u64,
    )
    .await?;
    setup_jobs::record_result(repo,task,&json!({"operation":"actor_seed","dataset_sha256":dataset.sha256,"processed":complete.processed,"total":complete.total,"aliases":complete.aliases,"database_seeded":complete.complete,"actor_search_state":"not_observed"})).await?;
    Ok(if complete.complete {
        SetupOutcome::Complete
    } else {
        SetupOutcome::Interrupted
    })
}

async fn train(
    repo: &Repository,
    task: &ClaimedTask,
    workspace: PathBuf,
    path: PathBuf,
    dataset: DatasetRef,
    budget: bulk_pressure::ParserBudget,
    cancel: Arc<AtomicBool>,
) -> anyhow::Result<SetupOutcome> {
    budget.check_training_input(dataset.size_bytes)?;
    let check_path = path.clone();
    let check_dataset = dataset.clone();
    tokio::task::spawn_blocking(move || {
        let mut reader = verified_reader(&check_path, &check_dataset)?;
        std::io::copy(&mut reader, &mut std::io::sink())?;
        verify_complete(reader, &check_dataset)
    })
    .await??;
    let mut request = DatasetRequest::default();
    request.explicit_file = Some(path);
    request.cwd = workspace.clone();
    request.home = workspace.clone();
    tokio::task::spawn_blocking({
        let workspace = workspace.clone();
        let budget = budget.clone();
        move || budget.check_running(&workspace)
    })
    .await??;
    if cancel.load(Ordering::Acquire) {
        return Ok(SetupOutcome::Interrupted);
    }
    let home = ClassifierHome::new(workspace.clone());
    // The exact durable job ID is the stable domain request key. Activation is
    // absent from the public maintenance request and explicitly retained here.
    let accepted = home
        .start_training(ManualTrainRequest::new(
            request,
            task.job_id.clone(),
            SyntheticActivation::RetainPrior,
        ))
        .await?;
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut cancelled = false;
    let mut pressure_failed = false;
    let mut last_pressure = std::time::Instant::now();
    let result = async {loop {
        tick.tick().await;
        if last_pressure.elapsed()>=Duration::from_secs(1) {
            let observation=tokio::task::spawn_blocking({let workspace=workspace.clone();let budget=budget.clone();move||budget.check_running(&workspace)}).await?;
            last_pressure=std::time::Instant::now();
            if observation.is_err()&&!cancelled {
                home.cancel_training(&accepted.job_id).await?;
                cancelled=true;pressure_failed=true;
            }
        }
        if cancel.load(Ordering::Acquire) && !cancelled {
            home.cancel_training(&accepted.job_id).await?;
            cancelled = true;
        }
        let detail = home.job_detail(&accepted.job_id).await?;
        if let Some(progress) = detail.progress {
            setup_jobs::record_progress(
                repo,
                task,
                progress.stage.as_str(),
                progress.rows_seen,
                progress.rows_total,
            )
            .await?;
        }
        match detail.state {
            TrainingJobState::Completed => {
                anyhow::ensure!(!pressure_failed,"worker_training_resource_budget");
                let outcome = detail
                    .outcome
                    .ok_or_else(|| anyhow::anyhow!("training_completed_without_candidate"))?;
                let Provenance::SyntheticBaseline { dataset_sha256, .. } =
                    &outcome.report.provenance
                else {
                    anyhow::bail!("training_dataset_provenance_unavailable");
                };
                anyhow::ensure!(
                    dataset_sha256 == &dataset.sha256,
                    "training_dataset_hash_mismatch"
                );
                setup_jobs::record_result(repo,task,&json!({"operation":"classifier_train","engine":"natural","atlas_engine":"rust_bert_zero_shot","dataset_sha256":dataset.sha256,"candidate_model_id":outcome.candidate_model_id,"active_model_id":outcome.active_model_id,"activation_requested":false,"state":outcome.state})).await?;
                break Ok(SetupOutcome::Complete);
            }
            TrainingJobState::Cancelled | TrainingJobState::Interrupted => {
                anyhow::ensure!(!pressure_failed,"worker_training_resource_budget");
                break Ok(SetupOutcome::Interrupted)
            }
            TrainingJobState::Failed => break Err(anyhow::anyhow!("natural_training_failed")),
            _ => {}
        }
    }}.await;
    let stopped = home.shutdown().await;
    stopped?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use fleetworks_contracts::readiness::DatasetKind;

    #[test]
    fn worker_failure_diagnostics_preserve_known_budgets_without_exposing_input() {
        assert_eq!(
            safe_failure_reason(&anyhow::anyhow!("worker_seed_resource_budget")),
            "setup_worker_resource_pressure"
        );
        assert_eq!(
            safe_failure_reason(
                &anyhow::anyhow!("actor_parse_memory_budget").context("bounded parser stopped")
            ),
            "setup_dataset_resource_budget"
        );
        assert_eq!(
            safe_failure_reason(&anyhow::anyhow!(
                "synthetic private path /not-for-job-output"
            )),
            "native_domain_operation_failed"
        );
    }

    #[test]
    fn actor_parse_hashes_the_same_bytes_it_parses_and_refuses_changed_content() {
        let root = tempfile::tempdir().expect("temporary fixture");
        let path = root.path().join("catalog.csv");
        let bytes=b"actor_id,actor_name,actor_type,aliases,name_normalized\nsynthetic-1,Harbor Union,organization,[],harbor union\n";
        std::fs::write(&path, bytes).expect("synthetic catalog");
        let reference = DatasetRef::new(
            "dataset-fixture".into(),
            DatasetKind::ActorCatalog,
            format!("{:x}", Sha256::digest(bytes)),
            bytes.len() as u64,
        )
        .expect("dataset reference");
        assert_eq!(
            read_actors(&path, &reference).expect("exact catalog").len(),
            1
        );
        let mut changed = bytes.to_vec();
        changed[75] ^= 1;
        std::fs::write(&path, changed).expect("changed exact-size fixture");
        assert!(read_actors(&path, &reference).is_err());
    }
}
