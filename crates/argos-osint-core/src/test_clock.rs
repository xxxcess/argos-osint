//! CPU-time stopwatch for tests that bound how much *work* code does.
//!
//! Wall-clock budgets in tests fail whenever the host is busy, because a
//! preempted thread keeps accruing wall time without running. The calling
//! thread's CPU clock only advances while the thread runs, so a budget on it
//! still catches real regressions but ignores scheduling delay.

use std::time::Duration;

/// CPU time consumed so far by the calling thread.
#[cfg(unix)]
pub(crate) fn thread_cpu_time() -> Duration {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid, writable timespec for the duration of the call.
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    assert_eq!(rc, 0, "clock_gettime(CLOCK_THREAD_CPUTIME_ID) failed");
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

/// Non-unix fallback: wall time since first use (no thread CPU clock in std).
#[cfg(not(unix))]
pub(crate) fn thread_cpu_time() -> Duration {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed()
}

/// Runs `f` and returns its result with the CPU time it used on this thread.
pub(crate) fn cpu_timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let start = thread_cpu_time();
    let out = f();
    (out, thread_cpu_time().saturating_sub(start))
}
