//! Atlas maintenance actions started from the TUI.

use std::path::PathBuf;

/// Start the user-triggered "Repair Atlas memories" job on a background
/// thread. False when a repair is already running in this process.
pub fn start_repair(db_path: PathBuf) -> bool {
    #[cfg(test)]
    {
        let _ = db_path;
        testing::record_start()
    }
    #[cfg(not(test))]
    {
        argos_osint_core::atlas_memory::spawn_repair(db_path, true)
    }
}

/// Test hook: count repair starts instead of spawning the job.
#[cfg(test)]
pub mod testing {
    use std::cell::Cell;

    thread_local! {
        static STARTS: Cell<usize> = const { Cell::new(0) };
        static BUSY: Cell<bool> = const { Cell::new(false) };
    }

    pub(super) fn record_start() -> bool {
        if BUSY.with(Cell::get) {
            return false;
        }
        STARTS.with(|starts| starts.set(starts.get() + 1));
        true
    }

    pub fn starts() -> usize {
        STARTS.with(Cell::get)
    }

    pub fn set_busy(busy: bool) {
        BUSY.with(|cell| cell.set(busy));
    }
}
