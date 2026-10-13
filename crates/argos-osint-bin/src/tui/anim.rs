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
#[cfg(test)]
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

/// Fit `text` to `width` columns: truncated with `…` when too wide.
pub fn fit_width(text: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthStr;
    if text.width() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

/// Left padding that centers `text` (fitted to `width`) horizontally.
pub fn center_text(text: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthStr;
    let fitted = fit_width(text, width);
    let pad = (width - fitted.width()) / 2;
    format!("{}{fitted}", " ".repeat(pad))
}

/// Animated loading line for `label`, centered in a `width`×`height` panel
/// body: `(height - 1) / 2` blank rows above, then the horizontally centered
/// line (truncated with `…` when wider than the panel). Empty for a 0 size.
pub fn centered_lines(
    label: &str,
    style: ratatui::style::Style,
    width: u16,
    height: u16,
) -> Vec<ratatui::text::Line<'static>> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let text = loading(label);
    let mut lines = vec![ratatui::text::Line::raw(""); usize::from((height - 1) / 2)];
    lines.push(ratatui::text::Line::styled(
        center_text(&text, usize::from(width)),
        style,
    ));
    lines
}

/// Draw the animated loading line centered in `area` (a panel's inner rect).
pub fn draw_centered(
    buf: &mut ratatui::buffer::Buffer,
    area: ratatui::layout::Rect,
    label: &str,
    style: ratatui::style::Style,
) {
    use ratatui::widgets::Widget;
    ratatui::widgets::Paragraph::new(centered_lines(label, style, area.width, area.height))
        .render(area, buf);
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

    fn row_of(lines: &[ratatui::text::Line<'_>]) -> (usize, String) {
        let last = lines.last().unwrap();
        (lines.len() - 1, last.to_string())
    }

    #[test]
    fn centered_lines_center_odd_and_even_sizes() {
        let style = ratatui::style::Style::default();
        // "⠋ Loading   " is 12 columns wide.
        for (w, h, row, pad) in [
            (20u16, 9u16, 4, 4),
            (21, 10, 4, 4),
            (13, 1, 0, 0),
            (12, 2, 0, 0),
        ] {
            let lines = with_frame(0, || centered_lines("Loading", style, w, h));
            let (r, text) = row_of(&lines);
            assert_eq!(r, row, "{w}x{h} row");
            assert_eq!(text.len() - text.trim_start().len(), pad, "{w}x{h} pad");
            assert!(text.trim_start().starts_with("⠋ Loading"));
        }
    }

    #[test]
    fn centered_lines_handle_tiny_and_wide() {
        let style = ratatui::style::Style::default();
        assert!(centered_lines("Loading", style, 0, 5).is_empty());
        assert!(centered_lines("Loading", style, 5, 0).is_empty());
        let one = with_frame(0, || centered_lines("Loading", style, 1, 1));
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].to_string(), "…");
        let wide = with_frame(0, || {
            centered_lines("Loading a very long label", style, 10, 3)
        });
        let (row, text) = row_of(&wide);
        assert_eq!(row, 1);
        assert_eq!(text, "⠋ Loading…");
        assert_eq!(unicode_width::UnicodeWidthStr::width(text.as_str()), 10);
    }

    #[test]
    fn draw_centered_paints_glyph_at_center() {
        let area = ratatui::layout::Rect::new(2, 1, 30, 7);
        let mut buf = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 40, 10));
        with_frame(3, || {
            draw_centered(&mut buf, area, "Loading", Default::default())
        });
        // 12-col line in 30 cols → pad 9; 7 rows → row 3.
        assert_eq!(buf[(2 + 9, 1 + 3)].symbol(), SPINNER[3]);
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
