//! Job registration for operations the TUI launches (spec §5.1, phase 6).
//!
//! Each asynchronous entry point registers a registry job before it spawns
//! work and finishes it from the outcome. Registry failures never block the
//! operation. Core-owned operations (Atlas cycles, Recon investigations,
//! Intel Recon assessments, memory repair, index/summary workers) register
//! themselves and are not wrapped here.

use std::fmt::Display;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use argos_osint_core::job_registry::{self, Finish, JobHandle, JobSpec};

/// Database the registry writes to (tests opt in with [`testing::use_db`]).
fn db() -> Option<std::path::PathBuf> {
    #[cfg(test)]
    {
        testing::db()
    }
    #[cfg(not(test))]
    {
        Some(argos_osint_core::paths::db_path())
    }
}

/// Database for operations that register their own jobs (graph explanations).
pub fn db_path() -> Option<std::path::PathBuf> {
    db()
}

/// Register and start a job; `None` when the registry is unavailable.
pub fn begin(spec: JobSpec) -> Option<JobHandle> {
    job_registry::begin_optional(&db()?, spec)
}

/// Register a cancellable job whose stop flag is the operation's own flag.
pub fn begin_cancellable(spec: JobSpec, cancel: Arc<AtomicBool>) -> Option<JobHandle> {
    job_registry::begin_optional_with(&db()?, spec.cancellable(), cancel)
}

/// Finish from a result: success → completed; failure → cancelled when the
/// stop flag was set, otherwise failed with the (redacted) error.
pub fn finish<T, E: Display>(
    job: Option<JobHandle>,
    category: &str,
    outcome: &Result<T, E>,
    cancel: Option<&AtomicBool>,
) {
    let Some(job) = job else {
        return;
    };
    job.finish(match outcome {
        Ok(_) => Finish::completed(),
        Err(_) if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) => Finish::Cancelled {
            summary: "cancelled by user".into(),
        },
        Err(err) => Finish::failed(category, err.to_string()),
    });
}

#[cfg(test)]
pub mod testing {
    use std::cell::RefCell;
    use std::path::PathBuf;

    thread_local! {
        static DB: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }

    /// Route registrations on this thread to `path` (None disables them).
    pub fn use_db(path: Option<PathBuf>) {
        DB.with(|db| *db.borrow_mut() = path);
    }

    pub fn db() -> Option<PathBuf> {
        DB.with(|db| db.borrow().clone())
    }
}
