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
    paused: bool,
    paused_at: Option<Instant>,
    probe_inflight: bool,
    query_ms: u64,
    commit_ms: u64,
    last_query: Option<Instant>,
    last_commit: Option<Instant>,
}

impl Policy {
    fn physical_reason(
        &self,
        limits: &ActorIndexLimits,
        sample: Measurement,
    ) -> Option<&'static str> {
        let (Some(memory), Some(disk)) = (sample.resident_bytes, sample.disk_free_bytes) else {
            return Some("pressure_measurement_unavailable");
        };
        if memory >= limits.memory_pause_bytes
            || (self.paused && memory > limits.memory_resume_bytes)
        {
            return Some("memory_high_water");
        }
        if disk <= limits.disk_pause_free_bytes
            || (self.paused && disk < limits.disk_resume_free_bytes)
        {
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
        if let Some(reason) = self.physical_reason(limits, sample) {
            if !self.paused {
                self.paused_at = Some(now);
            }
            self.paused = true;
            return (true, Some(reason));
        }
        let query_slow = self.query_ms
            >= u64::from(if self.paused {
                limits.query_resume_ms
            } else {
                limits.query_pause_ms
            });
        let commit_slow = self.commit_ms
            >= u64::from(if self.paused {
                limits.commit_resume_ms
            } else {
                limits.commit_pause_ms
            });
        if query_slow || commit_slow {
            if !self.paused {
                self.paused_at = Some(now);
            }
            self.paused = true;
            let probe_due = self.paused_at.is_some_and(|at| {
                now.saturating_duration_since(at)
                    >= Duration::from_millis(u64::from(limits.cooldown_ms))
            });
            if probe_due && !self.probe_inflight {
                return (false, Some("latency_probe_due"));
            }
            return (
                true,
                Some(if query_slow {
                    "query_latency_high_water"
                } else {
                    "commit_latency_high_water"
                }),
            );
        }
        self.paused = false;
        self.paused_at = None;
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
            self.paused_at = Some(now);
        } else {
            self.commit_ms = if self.last_commit.is_none() {
                measured
            } else {
                self.commit_ms.saturating_mul(7).saturating_add(measured) / 8
            };
        }
        self.last_commit = Some(now);
    }
}

pub struct Meter {
    directory: PathBuf,
    limits: ActorIndexLimits,
    policy: Mutex<Policy>,
}

impl Meter {
    pub fn new(directory: PathBuf, limits: ActorIndexLimits) -> Result<Self, IndexError> {
        limits.validate().map_err(IndexError::protocol)?;
        Ok(Self {
            directory,
            limits,
            policy: Mutex::new(Policy::default()),
        })
    }

    pub fn snapshot(&self) -> Result<ActorIndexPressure, IndexError> {
        let sample = measure(&self.directory);
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
        let sample = measure(&self.directory);
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
        if policy.probe_inflight {
            policy.probe_inflight = false;
            policy.paused_at = Some(Instant::now());
        }
        Ok(())
    }
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
#[cfg(not(target_os = "linux"))]
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
