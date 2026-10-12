//! Shared loading animations for every TUI screen.
//!
//! One clock drives every spinner: [`frame`] maps wall time to a frame index
//! (one step per [`STEP_MS`]). Every glyph helper is a pure function of that
//! index, so tests pin the index with [`with_frame`] and assert exact frames.
//!
//! Drawing any animation marks the current draw as animated ([`take_animated`]);
//! the event loop reads that mark after each draw and repaints on the next
//! step. A screen therefore only has to call a helper here to stay live: no
//! per-screen visibility checks in the event loop.

use std::cell::Cell;

/// Milliseconds per animation step (the Intel brief cadence).
pub const STEP_MS: u64 = 80;

/// Braille spinner used by the Intel brief and Atlas extraction.
pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Pulsing trailing dots; each phase holds for [`DOTS_HOLD`] steps.
pub const DOTS: [&str; 4] = ["   ", ".  ", ".. ", "..."];
pub const DOTS_HOLD: u64 = 4;

/// Small pulsing marker for compact indicators (tabs, status cells).
pub const PULSE: [&str; 4] = ["◌", "○", "◎", "●"];
pub const PULSE_HOLD: u64 = 3;

thread_local! {
    static PINNED: Cell<Option<u64>> = const { Cell::new(None) };
    static ANIMATED: Cell<bool> = const { Cell::new(false) };
}

/// Current frame index: the pinned one in tests, otherwise wall time / STEP_MS.
pub fn frame() -> u64 {
    if let Some(pinned) = PINNED.with(Cell::get) {
        return pinned;
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64 / STEP_MS)
        .unwrap_or(0)
}

/// Run `f` with the frame index pinned to `index` (restored afterwards).
pub fn with_frame<R>(index: u64, f: impl FnOnce() -> R) -> R {
    let prev = PINNED.with(|p| p.replace(Some(index)));
    let out = f();
    PINNED.with(|p| p.set(prev));
    out
}

/// True when anything animated was drawn since the last call; clears the mark.
pub fn take_animated() -> bool {
    ANIMATED.with(|a| a.replace(false))
}

fn mark() {
    ANIMATED.with(|a| a.set(true));
}

/// Spinner glyph for a frame index.
pub fn spinner_at(index: u64) -> &'static str {
    SPINNER[(index % SPINNER.len() as u64) as usize]
}

/// Dots for a frame index.
pub fn dots_at(index: u64) -> &'static str {
    DOTS[((index / DOTS_HOLD) % DOTS.len() as u64) as usize]
}

/// Pulse glyph for a frame index.
pub fn pulse_at(index: u64) -> &'static str {
    PULSE[((index / PULSE_HOLD) % PULSE.len() as u64) as usize]
}

/// Spinner glyph at the current frame.
pub fn spinner() -> &'static str {
    mark();
    spinner_at(frame())
}

/// Pulse glyph at the current frame.
pub fn pulse() -> &'static str {
    mark();
    pulse_at(frame())
}

/// `"⠹ Loading jobs..."`: spinner, label (trailing ellipsis dropped), dots.
pub fn loading(label: &str) -> String {
    mark();
    let index = frame();
    let label = label
        .trim_end_matches('…')
        .trim_end_matches("...")
        .trim_end();
    format!("{} {label}{}", spinner_at(index), dots_at(index))
}

/// Job status cell: spinner while running, pulse while queued/waiting.
pub fn job_state_label(state: &str, label: &str) -> String {
    match state {
        "running" => format!("{} {label}", spinner()),
        "queued" | "retry_scheduled" | "waiting" => format!("{} {label}", pulse()),
        _ => label.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_cycles_and_wraps() {
        for (i, glyph) in SPINNER.iter().enumerate() {
            assert_eq!(spinner_at(i as u64), *glyph);
        }
        assert_eq!(spinner_at(10), SPINNER[0]);
        assert_eq!(spinner_at(u64::MAX), SPINNER[(u64::MAX % 10) as usize]);
        assert_ne!(spinner_at(3), spinner_at(4));
    }

    #[test]
    fn dots_and_pulse_hold_then_wrap() {
        assert_eq!(dots_at(0), DOTS[0]);
        assert_eq!(dots_at(DOTS_HOLD - 1), DOTS[0]);
        assert_eq!(dots_at(DOTS_HOLD), DOTS[1]);
        assert_eq!(dots_at(DOTS_HOLD * 4), DOTS[0]);
        assert_eq!(pulse_at(PULSE_HOLD * 3), PULSE[3]);
        assert_eq!(pulse_at(PULSE_HOLD * 4), PULSE[0]);
        assert!(DOTS.iter().all(|d| d.chars().count() == 3), "fixed width");
    }

    #[test]
    fn job_states_animate_only_while_live() {
        assert_eq!(
            with_frame(1, || job_state_label("running", "running")),
            "⠙ running"
        );
        assert_eq!(
            with_frame(0, || job_state_label("queued", "queued")),
            "◌ queued"
        );
        assert_eq!(
            with_frame(9, || job_state_label("queued", "queued")),
            "● queued"
        );
        assert_eq!(job_state_label("completed", "completed"), "completed");
    }

    #[test]
    fn pinned_frame_is_deterministic_and_restored() {
        with_frame(7, || {
            assert_eq!(frame(), 7);
            with_frame(2, || assert_eq!(spinner(), SPINNER[2]));
            assert_eq!(frame(), 7);
        });
        assert!(PINNED.with(Cell::get).is_none());
    }

    #[test]
    fn loading_label_normalizes_ellipsis_and_marks_animated() {
        take_animated();
        assert_eq!(
            with_frame(0, || loading("Loading jobs…")),
            "⠋ Loading jobs   "
        );
        assert!(take_animated());
        assert!(!take_animated());
        assert_eq!(with_frame(12, || loading("Loading")), "⠹ Loading...");
    }
}
