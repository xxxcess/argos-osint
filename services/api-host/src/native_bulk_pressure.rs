//! Worker-local memory/disk gate for bulk setup only. Included as a child of
//! setup_jobs after its compiling checkpoint; Tools never calls this gate.

#[cfg(unix)]
use std::ffi::CString;
#[cfg(target_os = "linux")]
use std::fs::File;
#[cfg(target_os = "linux")]
use std::io::Read;
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

/// The complete validated Vec is returned only after bounded incremental parse,
/// exact admitted-byte verification and a final resource check. Any failure
/// drops retained input here, before a source lease/transaction can be acquired.
pub(super) fn read_actors_checked(
    path: &Path,
    dataset: &DatasetRef,
    workspace: &Path,
    budget: &ParserBudget,
    cancel: &AtomicBool,
) -> Result<Vec<Actor>> {
    budget.check_sample(measure(workspace))?;
    let mut reader = super::verified_reader(path, dataset)?;
    let actors = argos_osint_core::actor_catalog::read_bounded(
        &mut reader,
        &budget.actor_limits(),
        cancel,
        |_| budget.check_sample(measure(workspace)),
    )?;
    super::verify_complete(reader, dataset)?;
    ensure!(!cancel.load(Ordering::Acquire), "worker_bulk_cancelled");
    budget.check_sample(measure(workspace))?;
    Ok(actors)
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
        return pages
            .checked_mul(page_size as u64)
            .filter(|value| *value > 0);
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
