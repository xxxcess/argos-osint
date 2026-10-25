//! Hermetic frame-rebuild budget for a large Recon transcript.
//!
//! Builds an in-memory `App`, paints a full frame through the same `draw`
//! entry the TUI uses, then repeats the draw with unchanged inputs so the
//! frame cache must early-return. No storage runtime, SurrealDB server,
//! network access or `ARGOS_HOME` state is required.

use std::time::Instant;

use argos_osint_bin::tui::{draw, testkit};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

const WIDTH: u16 = 100;
const HEIGHT: u16 = 34;
const MESSAGES: usize = 1000;
/// The index of the newest message `testkit::push_messages` appends.
const NEWEST: usize = MESSAGES - 1;

/// Same extraction the TUI's unit tests use: every cell symbol on screen.
fn screen_text(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn a_full_transcript_frame_builds_in_budget_and_the_unchanged_repeat_is_cheap() {
    let mut app = testkit::app(WIDTH, HEIGHT);
    testkit::show_recon_chat(&mut app);
    testkit::push_messages(&mut app, MESSAGES);
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test terminal");

    // Cold path: build every chat block and row, then paint the frame.
    let first = Instant::now();
    terminal
        .draw(|frame| draw(frame, &app))
        .expect("first draw");
    let first_ms = first.elapsed().as_millis();
    println!("full frame build+draw ({MESSAGES} messages): {first_ms} ms");
    assert!(first_ms < 5_000, "first draw took {first_ms} ms");

    // Content check so the budget cannot be met by rendering nothing.
    let painted = screen_text(&terminal);
    assert!(
        painted.chars().any(|character| !character.is_whitespace()),
        "the frame must paint content"
    );
    assert!(
        painted.contains(&format!("perf transcript line {NEWEST:04}")),
        "the newest message must be visible in the transcript"
    );

    // Repeat path: identical inputs, `ensure_frame` must early-return on the
    // matching stamp and only the visible cells may be repainted.
    let second = Instant::now();
    terminal
        .draw(|frame| draw(frame, &app))
        .expect("repeat draw");
    let second_ms = second.elapsed().as_millis();
    println!("unchanged repeat draw: {second_ms} ms");
    assert!(second_ms < 500, "repeat draw took {second_ms} ms");
}
