//! Measured resource admission with high/low hysteresis and a bounded latency
//! probe after cooldown. Filesystem/syscall work belongs on a blocking worker.

use std::ffi::CString;
#[cfg(target_os = "linux")]
use std::fs::File;
#[cfg(target_os = "linux")]
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;

use fleetworks_contracts::actor_index::ActorIndexLimits;
use fleetworks_contracts::actor_index::ActorIndexPressure;

use crate::IndexError;

#[derive(Clone, Copy, Debug)]
pub struct Measurement {
    pub resident_bytes: Option<u64>,
    pub disk_free_bytes: Option<u64>,
}

#[derive(Default)]
struct Policy {
    // A low watermark applies only after that same resource reached its high watermark.
    memory_paused: bool,
    disk_paused: bool,
    query_paused: bool,
    commit_paused: bool,
    latency_paused_at: Option<Instant>,
    probe_inflight: bool,
    query_ms: u64,
    commit_ms: u64,
    last_query: Option<Instant>,
    last_commit: Option<Instant>,
}

impl Policy {
    fn physical_reason(
        &mut self,
        limits: &ActorIndexLimits,
        sample: Measurement,
    ) -> Option<&'static str> {
        if let Some(memory) = sample.resident_bytes {
            self.memory_paused = memory >= limits.memory_pause_bytes
                || (self.memory_paused && memory > limits.memory_resume_bytes);
        }
        if let Some(disk) = sample.disk_free_bytes {
            self.disk_paused = disk <= limits.disk_pause_free_bytes
                || (self.disk_paused && disk < limits.disk_resume_free_bytes);
        }
        if sample.resident_bytes.is_none() || sample.disk_free_bytes.is_none() {
            return Some("pressure_measurement_unavailable");
        }
        if self.memory_paused {
            return Some("memory_high_water");
        }
        if self.disk_paused {
            return Some("disk_high_water");
        }
        None
    }

    fn observe(
        &mut self,
        limits: &ActorIndexLimits,
        sample: Measurement,
        now: Instant,
    ) -> (bool, Option<&'static str>) {
        self.query_paused = self.query_ms >= u64::from(limits.query_pause_ms)
            || (self.query_paused && self.query_ms >= u64::from(limits.query_resume_ms));
        self.commit_paused = self.commit_ms >= u64::from(limits.commit_pause_ms)
            || (self.commit_paused && self.commit_ms >= u64::from(limits.commit_resume_ms));
        let latency_paused = self.query_paused || self.commit_paused;
        if !latency_paused {
            self.latency_paused_at = None;
        }
        if let Some(reason) = self.physical_reason(limits, sample) {
            return (true, Some(reason));
        }
        if latency_paused {
            let since = *self.latency_paused_at.get_or_insert(now);
            let probe_due = now.saturating_duration_since(since)
                >= Duration::from_millis(u64::from(limits.cooldown_ms));
            if probe_due && !self.probe_inflight {
                return (false, Some("latency_probe_due"));
            }
            return (
                true,
                Some(if self.query_paused {
                    "query_latency_high_water"
                } else {
                    "commit_latency_high_water"
                }),
            );
        }
        (false, None)
    }

    fn admit(&mut self, limits: &ActorIndexLimits, sample: Measurement, now: Instant) -> bool {
        let (paused, reason) = self.observe(limits, sample, now);
        if paused {
            return false;
        }
        if reason == Some("latency_probe_due") {
            self.probe_inflight = true;
        }
        true
    }

    fn query(&mut self, elapsed: Duration, now: Instant) {
        let measured = elapsed.as_millis().min(u128::from(u64::MAX)) as u64;
        self.query_ms = if self.last_query.is_none() {
            measured
        } else {
            self.query_ms.saturating_mul(7).saturating_add(measured) / 8
        };
        self.last_query = Some(now);
    }

    fn commit(&mut self, elapsed: Duration, now: Instant) {
        let measured = elapsed.as_millis().min(u128::from(u64::MAX)) as u64;
        if self.probe_inflight {
            self.commit_ms = measured;
            // Query pressure can clear only after a real newer query result,
            // never because the write probe guessed that queries were fast.
            self.probe_inflight = false;
            self.latency_paused_at = Some(now);
        } else {
            self.commit_ms = if self.last_commit.is_none() {
                measured
            } else {
                self.commit_ms.saturating_mul(7).saturating_add(measured) / 8
            };
        }
        self.last_commit = Some(now);
    }

    fn cancel_probe(&mut self, now: Instant) {
        if self.probe_inflight {
            self.probe_inflight = false;
            self.latency_paused_at = Some(now);
        }
    }
}

pub struct Meter {
    directory: PathBuf,
    limits: ActorIndexLimits,
    policy: Mutex<Policy>,
    reclaimed_at: Option<Mutex<Option<Instant>>>,
}

impl Meter {
    pub fn new(directory: PathBuf, limits: ActorIndexLimits) -> Result<Self, IndexError> {
        limits.validate().map_err(IndexError::protocol)?;
        Ok(Self {
            directory,
            limits,
            policy: Mutex::new(Policy::default()),
            reclaimed_at: None,
        })
    }

    /// Return free allocator pages ahead of co-located Embedded pressure checks.
    pub fn with_unused_memory_reclaim(mut self) -> Self {
        if cfg!(all(target_os = "linux", target_env = "gnu")) {
            self.reclaimed_at = Some(Mutex::new(None));
        }
        self
    }

    fn sample(&self) -> Result<Measurement, IndexError> {
        let mut sample = measure(&self.directory);
        let reclaim = self.reclaimed_at.as_ref().filter(|_| {
            sample
                .resident_bytes
                .is_some_and(|resident| resident >= self.limits.memory_resume_bytes)
        });
        if let Some(reclaim) = reclaim {
            // Status and writes share one trim window. The pressure policy is
            // locked only after the fresh system measurement is available.
            let mut last = reclaim
                .lock()
                .map_err(|_| IndexError::unavailable("pressure_state_poisoned"))?;
            let now = Instant::now();
            if last.is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(1)) {
                *last = Some(now);
                release_unused_heap();
            }
            sample.resident_bytes = resident_bytes();
        }
        Ok(sample)
    }

    pub fn snapshot(&self) -> Result<ActorIndexPressure, IndexError> {
        let sample = self.sample()?;
        let mut policy = self
            .policy
            .lock()
            .map_err(|_| IndexError::unavailable("pressure_state_poisoned"))?;
        let (paused, reason) = policy.observe(&self.limits, sample, Instant::now());
        Ok(ActorIndexPressure {
            resident_bytes: sample.resident_bytes,
            disk_free_bytes: sample.disk_free_bytes,
            query_ewma_ms: policy.query_ms,
            commit_ewma_ms: policy.commit_ms,
            paused,
            reason_code: reason.map(str::to_owned),
            observed_at_unix_ms: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
        })
    }

    pub fn admit_write(&self) -> Result<(), IndexError> {
        let sample = self.sample()?;
        let mut policy = self
            .policy
            .lock()
            .map_err(|_| IndexError::unavailable("pressure_state_poisoned"))?;
        if !policy.admit(&self.limits, sample, Instant::now()) {
            return Err(IndexError::unavailable("resource_pressure_paused"));
        }
        Ok(())
    }

    pub fn record_query(&self, elapsed: Duration) -> Result<(), IndexError> {
        self.policy
            .lock()
            .map_err(|_| IndexError::unavailable("pressure_state_poisoned"))?
            .query(elapsed, Instant::now());
        Ok(())
    }
    pub fn record_commit(&self, elapsed: Duration) -> Result<(), IndexError> {
        self.policy
            .lock()
            .map_err(|_| IndexError::unavailable("pressure_state_poisoned"))?
            .commit(elapsed, Instant::now());
        Ok(())
    }

    pub fn cancel_probe(&self) -> Result<(), IndexError> {
        let mut policy = self
            .policy
            .lock()
            .map_err(|_| IndexError::unavailable("pressure_state_poisoned"))?;
        policy.cancel_probe(Instant::now());
        Ok(())
    }
}

/// Return whole free pages managed by the local allocator to the kernel.
pub fn release_unused_heap() -> bool {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        // SAFETY: glibc reclaims only free pages under its own allocator locks.
        // No caller-owned pointer or live allocation is accessed.
        unsafe { libc::malloc_trim(0) != 0 }
    }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    false
}

pub fn measure(directory: &Path) -> Measurement {
    Measurement {
        resident_bytes: resident_bytes(),
        disk_free_bytes: disk_free_bytes(directory),
    }
}

#[cfg(target_os = "linux")]
fn resident_bytes() -> Option<u64> {
    let mut body = String::new();
    File::open("/proc/self/statm")
        .ok()?
        .take(512)
        .read_to_string(&mut body)
        .ok()?;
    let pages = body.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    // sysconf has no pointer arguments and reports the running kernel's page size.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    u64::try_from(page)
        .ok()
        .filter(|page| *page > 0)?
        .checked_mul(pages)
}
#[cfg(target_os = "macos")]
fn resident_bytes() -> Option<u64> {
    let mut info = std::mem::MaybeUninit::<libc::mach_task_basic_info_data_t>::uninit();
    let mut count: libc::mach_msg_type_number_t = libc::MACH_TASK_BASIC_INFO_COUNT;
    // `mach_task_self()` yields the current task port; `task_info` initializes `info`
    // and writes `count` only when it returns `KERN_SUCCESS`.
    #[allow(deprecated)]
    let status = unsafe {
        libc::task_info(
            libc::mach_task_self(),
            libc::MACH_TASK_BASIC_INFO as libc::task_flavor_t,
            info.as_mut_ptr().cast::<libc::integer_t>(),
            &mut count,
        )
    };
    if status != libc::KERN_SUCCESS || count < libc::MACH_TASK_BASIC_INFO_COUNT {
        return None;
    }
    let info = unsafe { info.assume_init() };
    let resident: u64 = info.resident_size;
    (resident > 0).then_some(resident)
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn resident_bytes() -> Option<u64> {
    None
}

#[cfg(unix)]
fn disk_free_bytes(directory: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = CString::new(directory.as_os_str().as_bytes()).ok()?;
    let mut result = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // The pathname is NUL-terminated and result is initialized only on success.
    if unsafe { libc::statvfs(path.as_ptr(), result.as_mut_ptr()) } != 0 {
        return None;
    }
    let result = unsafe { result.assume_init() };
    // Field widths vary across Unix; retain the u64 overflow boundary.
    let available = u128::from(result.f_bavail);
    let fragment_size = u128::from(result.f_frsize);
    u64::try_from(available * fragment_size).ok()
}
#[cfg(not(unix))]
fn disk_free_bytes(_directory: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn healthy(limits: &ActorIndexLimits) -> Measurement {
        Measurement {
            resident_bytes: Some(limits.memory_resume_bytes - 1),
            disk_free_bytes: Some(limits.disk_resume_free_bytes + 1),
        }
    }

    #[test]
    fn disk_memory_and_unknown_measurement_pause_until_low_water_without_burst() {
        let limits = ActorIndexLimits::default();
        let now = Instant::now();
        let mut policy = Policy::default();
        assert!(policy.admit(&limits, healthy(&limits), now));
        assert!(!policy.admit(
            &limits,
            Measurement {
                resident_bytes: Some(limits.memory_pause_bytes),
                ..healthy(&limits)
            },
            now
        ));
        assert!(!policy.admit(
            &limits,
            Measurement {
                resident_bytes: Some(limits.memory_resume_bytes + 1),
                ..healthy(&limits)
            },
            now + Duration::from_secs(300)
        ));
        assert!(policy.admit(&limits, healthy(&limits), now + Duration::from_secs(301)));
        assert!(!policy.admit(
            &limits,
            Measurement {
                disk_free_bytes: None,
                ..healthy(&limits)
            },
            now
        ));
        assert!(!policy.admit(
            &limits,
            Measurement {
                disk_free_bytes: Some(limits.disk_pause_free_bytes),
                ..healthy(&limits)
            },
            now
        ));
        assert!(policy.admit(&limits, healthy(&limits), now + Duration::from_secs(302)));
    }

    #[test]
    fn memory_and_disk_recover_independently() {
        let limits = ActorIndexLimits::default();
        let now = Instant::now();
        let mut policy = Policy::default();
        let disk_between = limits.disk_resume_free_bytes - 1;
        let memory_between = limits.memory_resume_bytes + 1;

        assert_eq!(
            policy.observe(
                &limits,
                Measurement {
                    resident_bytes: Some(limits.memory_pause_bytes),
                    disk_free_bytes: Some(disk_between),
                },
                now,
            ),
            (true, Some("memory_high_water"))
        );
        assert!(policy.admit(
            &limits,
            Measurement {
                disk_free_bytes: Some(disk_between),
                ..healthy(&limits)
            },
            now
        ));

        assert_eq!(
            policy.observe(
                &limits,
                Measurement {
                    resident_bytes: Some(memory_between),
                    disk_free_bytes: Some(limits.disk_pause_free_bytes),
                },
                now,
            ),
            (true, Some("disk_high_water"))
        );
        assert!(policy.admit(
            &limits,
            Measurement {
                resident_bytes: Some(memory_between),
                ..healthy(&limits)
            },
            now
        ));
    }

    #[test]
    fn missing_measurement_fails_closed_without_latching_healthy_resources() {
        let limits = ActorIndexLimits::default();
        let now = Instant::now();
        let mut policy = Policy::default();
        let memory_between = Some(limits.memory_resume_bytes + 1);

        assert_eq!(
            policy.observe(
                &limits,
                Measurement {
                    resident_bytes: memory_between,
                    disk_free_bytes: None,
                },
                now,
            ),
            (true, Some("pressure_measurement_unavailable"))
        );
        assert!(policy.admit(
            &limits,
            Measurement {
                resident_bytes: memory_between,
                disk_free_bytes: Some(limits.disk_resume_free_bytes - 1),
            },
            now
        ));
    }

    #[test]
    fn query_cooldown_probe_does_not_latch_memory_or_commit_latency() {
        let limits = ActorIndexLimits::default();
        let now = Instant::now();
        let mut policy = Policy::default();
        let readings = Measurement {
            resident_bytes: Some(limits.memory_resume_bytes + 1),
            ..healthy(&limits)
        };
        policy.commit(
            Duration::from_millis(u64::from(limits.commit_resume_ms)),
            now,
        );
        policy.query(Duration::from_millis(u64::from(limits.query_pause_ms)), now);
        assert_eq!(
            policy.observe(&limits, readings, now),
            (true, Some("query_latency_high_water"))
        );
        let probe = now + Duration::from_millis(u64::from(limits.cooldown_ms));
        assert!(policy.admit(&limits, readings, probe));
        assert!(!policy.admit(&limits, readings, probe));
        policy.cancel_probe(probe);
        for _ in 0..16 {
            policy.query(Duration::ZERO, probe);
        }
        assert_eq!(policy.observe(&limits, readings, probe), (false, None));
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn resident_bytes_reports_positive_process_rss() {
        let bytes = resident_bytes().expect("supported OS reports resident memory");
        assert!(bytes > 0, "expected positive RSS, got {bytes}");
    }

    #[test]
    fn slow_commit_has_one_controlled_probe_after_cooldown() {
        let limits = ActorIndexLimits::default();
        let now = Instant::now();
        let mut policy = Policy::default();
        policy.commit(Duration::from_secs(10), now);
        assert!(!policy.admit(&limits, healthy(&limits), now));
        let probe = now + Duration::from_millis(u64::from(limits.cooldown_ms));
        assert!(policy.admit(&limits, healthy(&limits), probe));
        assert!(!policy.admit(&limits, healthy(&limits), probe));
        policy.commit(Duration::from_millis(10), probe);
        assert!(policy.admit(
            &limits,
            healthy(&limits),
            probe + Duration::from_millis(500)
        ));
    }
}
