//! Worker-local memory/disk gate for bulk setup only. Included as a child of
//! setup_jobs after its compiling checkpoint; Tools never calls this gate.

#[cfg(unix)]
use std::ffi::CString;
use std::fs::File;
use std::io::Read;
use std::io::Seek;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use anyhow::ensure;
use anyhow::Result;
use argos_osint_core::actor_catalog::Actor;
use argos_osint_core::actor_catalog::ActorReadLimits;
use argos_osint_core::storage::actor_search_outbox;
use argos_osint_core::storage::jobs::ClaimedTask;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::setup_jobs;
use fleetworks_contracts::actor_index::ActorIndexLimits;
use fleetworks_contracts::readiness::DatasetRef;

#[derive(Clone, Copy, Debug)]
pub(super) struct Measurement {
    resident_bytes: Option<u64>,
    workspace_free_bytes: Option<u64>,
}

#[derive(Default)]
struct Admission {
    paused: bool,
    cooldown_until: Option<Instant>,
}

#[derive(Clone, Debug)]
pub(super) struct ParserBudget {
    limits: ActorIndexLimits,
    retained_bytes: u64,
}

enum Decision {
    Ready(ParserBudget),
    Wait {
        reason: &'static str,
        retry_after: Duration,
    },
}

fn physical_reason(
    limits: &ActorIndexLimits,
    sample: Measurement,
    resuming: bool,
) -> Option<&'static str> {
    let (Some(memory), Some(disk)) = (sample.resident_bytes, sample.workspace_free_bytes) else {
        return Some("worker_pressure_unknown");
    };
    if memory >= limits.memory_pause_bytes || (resuming && memory > limits.memory_resume_bytes) {
        return Some("worker_memory_pressure");
    }
    if disk <= limits.disk_pause_free_bytes || (resuming && disk < limits.disk_resume_free_bytes) {
        return Some("worker_workspace_disk_pressure");
    }
    None
}

impl Admission {
    fn observe(
        &mut self,
        limits: &ActorIndexLimits,
        sample: Measurement,
        now: Instant,
    ) -> Result<Decision> {
        limits.validate()?;
        let reason = physical_reason(limits, sample, self.paused);
        if reason.is_some() && !self.paused {
            self.paused = true;
            self.cooldown_until = Some(now + Duration::from_millis(u64::from(limits.cooldown_ms)));
        }
        if let Some(reason) = reason {
            return Ok(Decision::Wait {
                reason,
                retry_after: Duration::from_secs(5),
            });
        }
        if let Some(until) = self.cooldown_until.filter(|until| *until > now) {
            return Ok(Decision::Wait {
                reason: "worker_pressure_cooldown",
                retry_after: until
                    .saturating_duration_since(now)
                    .min(Duration::from_secs(5)),
            });
        }
        let memory = sample
            .resident_bytes
            .ok_or_else(|| anyhow::anyhow!("worker_pressure_unknown"))?;
        // Keep a fourfold margin for temporary decode/allocator overhead and
        // other bounded worker work. Retained input has its own hard 32 MiB cap.
        let retained_bytes = ((limits.memory_pause_bytes - memory) / 4).min(32 * 1024 * 1024);
        if retained_bytes < 8 * 1024 * 1024 {
            self.paused = true;
            self.cooldown_until = Some(now + Duration::from_millis(u64::from(limits.cooldown_ms)));
            return Ok(Decision::Wait {
                reason: "worker_parser_headroom",
                retry_after: Duration::from_secs(5),
            });
        }
        self.paused = false;
        self.cooldown_until = None;
        Ok(Decision::Ready(ParserBudget {
            limits: limits.clone(),
            retained_bytes,
        }))
    }
}

impl ParserBudget {
    fn actor_limits(&self) -> ActorReadLimits {
        ActorReadLimits {
            max_retained_bytes: self.retained_bytes,
            ..Default::default()
        }
    }
    fn check_sample(&self, sample: Measurement) -> Result<()> {
        ensure!(
            physical_reason(&self.limits, sample, false).is_none(),
            "worker_parse_resource_budget"
        );
        Ok(())
    }

    /// The existing Natural parser already bounds rows, raw records and corpus
    /// text. Apply an additional conservative local input cap before starting
    /// that domain worker; larger validated uploads remain explicit failures.
    pub(super) fn check_training_input(&self, input_bytes: u64) -> Result<()> {
        ensure!(
            input_bytes <= self.retained_bytes,
            "worker_training_input_budget"
        );
        Ok(())
    }

    /// Use on a blocking worker at bounded training progress intervals. On
    /// failure the caller cancels and awaits the actual training operation.
    pub(super) fn check_running(&self, workspace: &Path) -> Result<()> {
        self.check_sample(measure(workspace))
    }
}

/// Wait before reading/parsing any bulk input, retaining no Actor Vec. The
/// existing delivery future owns renewals/cancellation and the separate Tools
/// lane continues while this lane waits. OS reads run on a blocking worker.
pub(super) async fn await_admission(
    repo: &Repository,
    task: &ClaimedTask,
    workspace: &Path,
    cancel: &AtomicBool,
) -> Result<ParserBudget> {
    let mut policy = Admission::default();
    loop {
        ensure!(!cancel.load(Ordering::Acquire), "worker_bulk_cancelled");
        let limits = actor_search_outbox::resource_limits(repo).await?;
        let sample = tokio::task::spawn_blocking({
            let workspace = workspace.to_owned();
            move || measure(&workspace)
        })
        .await?;
        ensure!(!cancel.load(Ordering::Acquire), "worker_bulk_cancelled");
        match policy.observe(&limits, sample, Instant::now())? {
            Decision::Ready(budget) => return Ok(budget),
            Decision::Wait {
                reason,
                retry_after,
            } => {
                setup_jobs::record_progress_unknown_total(repo, task, reason, 0).await?;
                wait_pause(cancel, retry_after).await?;
            }
        }
    }
}

async fn wait_pause(cancel: &AtomicBool, retry_after: Duration) -> Result<()> {
    let deadline = tokio::time::Instant::now() + retry_after;
    loop {
        ensure!(!cancel.load(Ordering::Acquire), "worker_bulk_cancelled");
        tokio::select! {
            _=tokio::time::sleep_until(deadline)=>return Ok(()),
            _=tokio::time::sleep(Duration::from_millis(50))=>(),
        }
    }
}

const MAX_ACTOR_FRAME_BYTES: usize = 1024 * 1024;
const MAX_ACTOR_SPOOL_BYTES: u64 = 512 * 1024 * 1024;

/// Private validated records: no original CSV is read after source writes begin.
pub(super) struct ValidatedActors {
    pub(super) total: usize,
    spool: File,
}

struct ActorFrame(Vec<u8>);
impl Write for ActorFrame {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_ACTOR_FRAME_BYTES {
            return Err(std::io::Error::other("actor_spool_frame_budget"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn private_actor_spool(path: &Path) -> Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("actor_spool_parent"))?;
        ensure!(parent.canonicalize()? == parent, "actor_spool_parent");
        let name = parent.join(format!(
            ".actor-spool-{}",
            argos_osint_core::storage::home::random_secret()?
        ));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&name)?;
        // Linux/macOS keep only the owned descriptor, including every error exit.
        std::fs::remove_file(&name)?;
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        anyhow::bail!("actor_spool_platform_unavailable")
    }
}

/// Verify every original CSV byte/record before acquiring a source lease.
/// Only bounded IDs/one actor live in RAM; the private spool retains all metadata.
pub(super) fn scan_actors_checked(
    path: &Path,
    dataset: &DatasetRef,
    workspace: &Path,
    budget: &ParserBudget,
    cancel: &AtomicBool,
) -> Result<ValidatedActors> {
    scan_actors_with_probe(path, dataset, budget, cancel, || {
        budget.check_sample(measure(workspace))
    })
}

fn scan_actors_with_probe(
    path: &Path,
    dataset: &DatasetRef,
    budget: &ParserBudget,
    cancel: &AtomicBool,
    mut probe: impl FnMut() -> Result<()>,
) -> Result<ValidatedActors> {
    probe()?;
    let mut reader = super::verified_reader(path, dataset)?;
    let mut spool = private_actor_spool(path)?;
    let mut spool_bytes = 0_u64;
    let summary = argos_osint_core::actor_catalog::scan_bounded(
        &mut reader,
        &budget.actor_limits(),
        cancel,
        |_| probe(),
        |actor| {
            ensure!(!cancel.load(Ordering::Acquire), "worker_bulk_cancelled");
            let mut frame = ActorFrame(Vec::new());
            serde_json::to_writer(&mut frame, &actor)?;
            spool_bytes = spool_bytes
                .checked_add(frame.0.len() as u64 + 4)
                .ok_or_else(|| anyhow::anyhow!("actor_spool_byte_budget"))?;
            ensure!(
                spool_bytes <= MAX_ACTOR_SPOOL_BYTES,
                "actor_spool_byte_budget"
            );
            // Verify the later writer can represent this whole actor before any writes.
            ensure!(
                argos_osint_core::actor_catalog::actor_seed_payload_bytes(&actor)?
                    <= argos_osint_core::storage::actor_catalog::MAX_SEED_BATCH_BYTES,
                "actor_seed_batch_budget"
            );
            ensure!(
                argos_osint_core::storage::actor_catalog::seed_actor_transaction_bytes(&actor)?
                    <= argos_osint_core::storage::actor_catalog::MAX_SEED_TRANSACTION_BYTES,
                "actor_seed_batch_budget"
            );
            spool.write_all(&(frame.0.len() as u32).to_le_bytes())?;
            spool.write_all(&frame.0)?;
            Ok(())
        },
    )?;
    super::verify_complete(reader, dataset)?;
    ensure!(!cancel.load(Ordering::Acquire), "worker_bulk_cancelled");
    probe()?;
    spool.flush()?;
    spool.sync_data()?;
    spool.rewind()?;
    Ok(ValidatedActors {
        total: summary.rows,
        spool,
    })
}

/// One blocking producer, one buffered batch. Caller closes the receiver and
/// joins this worker on every completion/error/cancellation path.
pub(super) fn produce_actor_batches(
    validated: ValidatedActors,
    workspace: &Path,
    budget: &ParserBudget,
    cancel: &AtomicBool,
    sender: tokio::sync::mpsc::Sender<Result<Vec<Actor>>>,
) -> Result<()> {
    produce_actor_batches_with_probe(validated, cancel, sender, || {
        budget.check_sample(measure(workspace))
    })
}

fn produce_actor_batches_with_probe(
    mut validated: ValidatedActors,
    cancel: &AtomicBool,
    sender: tokio::sync::mpsc::Sender<Result<Vec<Actor>>>,
    mut probe: impl FnMut() -> Result<()>,
) -> Result<()> {
    let result: Result<()> = (|| {
        let mut count = 0;
        let mut batch = Vec::new();
        let mut bytes = 0_u64;
        let mut expanded_bytes = 0_u64;
        loop {
            ensure!(!cancel.load(Ordering::Acquire), "worker_bulk_cancelled");
            let mut length = [0_u8; 4];
            if validated.spool.read(&mut length[..1])? == 0 {
                break;
            }
            validated.spool.read_exact(&mut length[1..])?;
            let length = u32::from_le_bytes(length) as usize;
            ensure!(
                (1..=MAX_ACTOR_FRAME_BYTES).contains(&length),
                "actor_spool_frame_budget"
            );
            let mut frame = vec![0; length];
            validated.spool.read_exact(&mut frame)?;
            let actor: Actor = serde_json::from_slice(&frame)?;
            let charge = argos_osint_core::actor_catalog::actor_seed_payload_bytes(&actor)?;
            ensure!(
                charge <= argos_osint_core::storage::actor_catalog::MAX_SEED_BATCH_BYTES,
                "actor_seed_batch_budget"
            );
            let expanded =
                argos_osint_core::storage::actor_catalog::seed_actor_transaction_bytes(&actor)?;
            ensure!(
                expanded <= argos_osint_core::storage::actor_catalog::MAX_SEED_TRANSACTION_BYTES,
                "actor_seed_batch_budget"
            );
            if !batch.is_empty()
                && (batch.len() == 128
                    || bytes + charge
                        > argos_osint_core::storage::actor_catalog::MAX_SEED_BATCH_BYTES
                    || expanded_bytes + expanded
                        > argos_osint_core::storage::actor_catalog::MAX_SEED_TRANSACTION_BYTES)
            {
                probe()?;
                sender
                    .blocking_send(Ok(std::mem::take(&mut batch)))
                    .map_err(|_| anyhow::anyhow!("worker_bulk_cancelled"))?;
                bytes = 0;
                expanded_bytes = 0;
            }
            count += 1;
            ensure!(count <= validated.total, "actor_seed_count_changed");
            bytes += charge;
            expanded_bytes += expanded;
            batch.push(actor);
        }
        ensure!(count == validated.total, "actor_seed_count_changed");
        ensure!(!cancel.load(Ordering::Acquire), "worker_bulk_cancelled");
        probe()?;
        if !batch.is_empty() {
            sender
                .blocking_send(Ok(batch))
                .map_err(|_| anyhow::anyhow!("worker_bulk_cancelled"))?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        let message = error.root_cause().to_string();
        let _ = sender.blocking_send(Err(error));
        return Err(anyhow::anyhow!(message));
    }
    Ok(())
}

fn measure(workspace: &Path) -> Measurement {
    Measurement {
        resident_bytes: resident_bytes(),
        workspace_free_bytes: free_bytes(workspace),
    }
}

fn resident_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let mut bytes = Vec::new();
        File::open("/proc/self/statm")
            .ok()?
            .take(513)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() > 512 {
            return None;
        }
        let pages = std::str::from_utf8(&bytes)
            .ok()?
            .split_whitespace()
            .nth(1)?
            .parse::<u64>()
            .ok()?;
        // SAFETY: sysconf has no pointer arguments. Invalid/unknown results fail closed.
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if page_size <= 0 {
            return None;
        }
        pages
            .checked_mul(page_size as u64)
            .filter(|value| *value > 0)
    }
    #[cfg(not(target_os = "linux"))]
    None
}

fn free_bytes(workspace: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = CString::new(workspace.as_os_str().as_bytes()).ok()?;
        let mut output = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        // SAFETY: path is NUL terminated and output is valid syscall storage.
        if unsafe { libc::statvfs(path.as_ptr(), output.as_mut_ptr()) } != 0 {
            return None;
        }
        // SAFETY: successful statvfs initialized the complete output.
        let output = unsafe { output.assume_init() };
        // Field widths vary across Unix; retain the u64 overflow boundary.
        let available = u128::from(output.f_bavail);
        let fragment_size = u128::from(output.f_frsize);
        u64::try_from(available * fragment_size).ok()
    }
    #[cfg(not(unix))]
    {
        let _ = workspace;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn healthy() -> Measurement {
        Measurement {
            resident_bytes: Some(64 * 1024 * 1024),
            workspace_free_bytes: Some(2 * 1024 * 1024 * 1024),
        }
    }

    fn fixture_budget() -> ParserBudget {
        ParserBudget {
            limits: ActorIndexLimits::default(),
            retained_bytes: 32 * 1024 * 1024,
        }
    }

    fn spool_fixture(rows: usize) -> (tempfile::TempDir, std::path::PathBuf, DatasetRef) {
        use fleetworks_contracts::readiness::DatasetKind;
        use sha2::Digest;
        let root = tempfile::tempdir().expect("private fixture");
        let path = root
            .path()
            .canonicalize()
            .expect("canonical parent")
            .join("actors.csv");
        let mut bytes = String::from("actor_id,actor_name,actor_type,aliases,name_normalized\n");
        for index in 0..rows {
            bytes.push_str(&format!(
                "synthetic-{index},Harbor Union {index},organization,[],harbor union {index}\n"
            ));
        }
        std::fs::write(&path, bytes.as_bytes()).expect("fixture CSV");
        let dataset = DatasetRef::new(
            "dataset-spool-fixture".into(),
            DatasetKind::ActorCatalog,
            format!("{:x}", sha2::Sha256::digest(bytes.as_bytes())),
            bytes.len() as u64,
        )
        .expect("reference");
        (root, path, dataset)
    }

    #[tokio::test]
    async fn validated_private_spool_preserves_exact_records_after_original_changes() {
        let (root, path, dataset) = spool_fixture(1);
        let cancel = AtomicBool::new(false);
        let validated =
            scan_actors_with_probe(&path, &dataset, &fixture_budget(), &cancel, || Ok(()))
                .expect("complete original validated");
        assert_eq!(validated.total, 1);
        assert_eq!(
            std::fs::read_dir(root.path())
                .expect("private directory")
                .count(),
            1,
            "spool has no reachable pathname"
        );
        std::fs::write(&path, b"changed after full validation").expect("replace original");
        let (send, mut receive) = tokio::sync::mpsc::channel(1);
        let producer = tokio::task::spawn_blocking(move || {
            produce_actor_batches_with_probe(validated, &AtomicBool::new(false), send, || Ok(()))
        });
        let batch = receive
            .recv()
            .await
            .expect("bounded batch")
            .expect("validated records");
        assert_eq!(batch[0].name, "Harbor Union 0");
        assert!(receive.recv().await.is_none());
        producer
            .await
            .expect("owned producer joined")
            .expect("complete spool");
        assert!(
            scan_actors_with_probe(&path, &dataset, &fixture_budget(), &cancel, || Ok(())).is_err(),
            "changed original cannot produce a newly validated spool"
        );
    }

    #[tokio::test]
    async fn closed_consumer_joins_blocked_producer_and_corrupt_frame_never_allocates_past_bound() {
        let (_root, path, dataset) = spool_fixture(260);
        let validated = scan_actors_with_probe(
            &path,
            &dataset,
            &fixture_budget(),
            &AtomicBool::new(false),
            || Ok(()),
        )
        .expect("validated spool");
        let (send, mut receive) = tokio::sync::mpsc::channel(1);
        let producer = tokio::task::spawn_blocking(move || {
            produce_actor_batches_with_probe(validated, &AtomicBool::new(false), send, || Ok(()))
        });
        let first = receive
            .recv()
            .await
            .expect("first batch")
            .expect("bounded records");
        assert!(first.len() <= 128);
        drop(receive);
        assert!(tokio::time::timeout(Duration::from_secs(2), producer)
            .await
            .expect("closed receiver wakes producer")
            .expect("joined producer")
            .is_err());
        let mut spool = private_actor_spool(&path).expect("unlinked spool");
        spool
            .write_all(&u32::MAX.to_le_bytes())
            .expect("malformed frame");
        spool.rewind().expect("rewind");
        let (send, mut receive) = tokio::sync::mpsc::channel(1);
        let producer = tokio::task::spawn_blocking(move || {
            produce_actor_batches_with_probe(
                ValidatedActors { total: 1, spool },
                &AtomicBool::new(false),
                send,
                || Ok(()),
            )
        });
        assert!(receive.recv().await.expect("controlled error").is_err());
        assert!(producer.await.expect("joined malformed producer").is_err());
    }

    #[test]
    fn unknown_and_high_pressure_fail_closed_then_require_cooldown_and_low_water() {
        let limits = ActorIndexLimits::default();
        let now = Instant::now();
        assert!(matches!(
            Admission::default()
                .observe(
                    &limits,
                    Measurement {
                        resident_bytes: Some(limits.memory_pause_bytes),
                        ..healthy()
                    },
                    now
                )
                .expect("high-water admission"),
            Decision::Wait {
                reason: "worker_memory_pressure",
                ..
            }
        ));
        let mut admission = Admission::default();
        assert!(matches!(
            admission
                .observe(
                    &limits,
                    Measurement {
                        resident_bytes: None,
                        ..healthy()
                    },
                    now
                )
                .expect("policy"),
            Decision::Wait {
                reason: "worker_pressure_unknown",
                ..
            }
        ));
        assert!(matches!(
            admission
                .observe(&limits, healthy(), now + Duration::from_secs(1))
                .expect("cooldown"),
            Decision::Wait {
                reason: "worker_pressure_cooldown",
                ..
            }
        ));
        assert!(matches!(
            admission
                .observe(
                    &limits,
                    Measurement {
                        resident_bytes: Some(limits.memory_resume_bytes + 1),
                        ..healthy()
                    },
                    now + Duration::from_secs(61)
                )
                .expect("hysteresis"),
            Decision::Wait { .. }
        ));
        let Decision::Ready(budget) = admission
            .observe(&limits, healthy(), now + Duration::from_secs(62))
            .expect("recovery")
        else {
            panic!("expected available budget")
        };
        assert!(budget.retained_bytes <= 32 * 1024 * 1024);
        assert!(budget
            .check_sample(Measurement {
                resident_bytes: Some(limits.memory_pause_bytes),
                ..healthy()
            })
            .is_err());
        assert!(budget
            .check_sample(Measurement {
                workspace_free_bytes: None,
                ..healthy()
            })
            .is_err());
    }

    #[tokio::test]
    async fn bulk_pressure_wait_is_async_and_does_not_hold_interactive_execution() {
        let limits = ActorIndexLimits::default();
        let mut admission = Admission::default();
        assert!(matches!(
            admission
                .observe(
                    &limits,
                    Measurement {
                        workspace_free_bytes: Some(1),
                        ..healthy()
                    },
                    Instant::now()
                )
                .expect("closed gate"),
            Decision::Wait { .. }
        ));
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let bulk = tokio::spawn({
            let cancel = std::sync::Arc::clone(&cancel);
            async move { wait_pause(&cancel, Duration::from_secs(3600)).await }
        });
        let tools = tokio::spawn(async { "interactive completed" });
        assert_eq!(
            tools.await.expect("independent task"),
            "interactive completed"
        );
        assert!(!bulk.is_finished());
        cancel.store(true, Ordering::Release);
        assert!(tokio::time::timeout(Duration::from_secs(1), bulk)
            .await
            .expect("bounded pressure cancellation")
            .expect("joined bulk task")
            .is_err());
    }
}
