//! Full-screen ratatui shell shared by the six `tui` clients.
//!
//! Each client supplies a [`View`] that owns its data and network calls. The
//! shell owns focus, selection, scrolling, input and confirmation modals, and
//! all drawing. Drawing reads immutable snapshots from the view and never
//! performs a request.

mod draw;
mod shell;

use std::io;
use std::io::IsTerminal;

use anyhow::Result;
use crossterm::event;
use crossterm::event::DisableMouseCapture;
use crossterm::event::EnableMouseCapture;
use crossterm::execute;
use crossterm::terminal::disable_raw_mode;
use crossterm::terminal::enable_raw_mode;
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

pub(crate) use shell::Action;
pub(crate) use shell::Prompt;
pub(crate) use shell::Shell;
pub(crate) use shell::Step;

/// Data and actions behind one client screen.
///
/// Every method that talks to the service returns a [`Step`]; failures become
/// an error status line and never leave the shell.
pub(crate) trait View {
    /// Header text after the product name.
    fn title(&self) -> String;
    /// Application key hints appended to the shared hints.
    fn hints(&self) -> &'static str;
    fn list_title(&self) -> &'static str;
    fn detail_title(&self) -> String;
    /// One line per record; the shell truncates by terminal cells.
    fn rows(&self) -> Vec<String>;
    /// Plain detail text; the shell wraps and scrolls it.
    fn detail(&self) -> Vec<String>;
    /// Zero-gap record list (otherwise records are separated by a blank row).
    fn dense(&self) -> bool {
        false
    }
    fn refresh(&mut self) -> Result<Step>;
    /// The selection moved to `index`.
    fn select(&mut self, index: usize) -> Result<Step>;
    /// Enter was pressed on `index`.
    fn activate(&mut self, index: usize) -> Result<Step>;
    /// An application hotkey with the current selection.
    fn key(&mut self, key: char, index: usize) -> Result<Step>;
    /// A prompt raised earlier produced `value` (`"yes"` for confirmations).
    fn submit(&mut self, tag: &'static str, value: &str) -> Result<Step>;
}

/// Whether both ends are interactive terminals.
pub(crate) fn available() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

struct Restore;

impl Restore {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        if let Err(error) = execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture) {
            let _ = disable_raw_mode();
            return Err(error.into());
        }
        Ok(Self)
    }
}

impl Drop for Restore {
    fn drop(&mut self) {
        restore_terminal();
    }
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        DisableMouseCapture,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    );
}

/// Run `view` full screen until the user quits.
pub(crate) fn run(view: &mut dyn View) -> Result<()> {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        previous(info);
    }));
    let _restore = Restore::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut shell = Shell::new();
    shell.status("Loading…");
    terminal.draw(|frame| draw::draw(frame, &mut shell, view))?;
    let first = view.refresh();
    shell.apply(first, view.rows().len());
    loop {
        terminal.draw(|frame| draw::draw(frame, &mut shell, view))?;
        if shell.quit() {
            return Ok(());
        }
        let Some(action) = shell.event(event::read()?, view.rows().len()) else {
            continue;
        };
        shell.status("Working…");
        terminal.draw(|frame| draw::draw(frame, &mut shell, view))?;
        let result = match action {
            Action::Refresh => view.refresh(),
            Action::Select(index) => view.select(index),
            Action::Activate(index) => view.activate(index),
            Action::Key(key, index) => view.key(key, index),
            Action::Submit(tag, value) => view.submit(tag, &value),
        };
        shell.apply(result, view.rows().len());
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use anyhow::Result;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::draw;
    use super::Shell;
    use super::Step;
    use super::View;

    /// Fixed-data view for shell tests.
    pub(crate) struct StaticView {
        pub rows: Vec<String>,
        pub detail: Vec<String>,
        pub dense: bool,
    }

    impl StaticView {
        pub fn new(rows: usize) -> Self {
            Self {
                rows: (1..=rows).map(|row| format!("row {row}")).collect(),
                detail: (1..=40).map(|line| format!("detail line {line}")).collect(),
                dense: false,
            }
        }
    }

    impl View for StaticView {
        fn title(&self) -> String {
            "Static".into()
        }
        fn hints(&self) -> &'static str {
            "x action"
        }
        fn list_title(&self) -> &'static str {
            "Items"
        }
        fn detail_title(&self) -> String {
            "Detail".into()
        }
        fn rows(&self) -> Vec<String> {
            self.rows.clone()
        }
        fn detail(&self) -> Vec<String> {
            self.detail.clone()
        }
        fn dense(&self) -> bool {
            self.dense
        }
        fn refresh(&mut self) -> Result<Step> {
            Ok(Step::status("refreshed"))
        }
        fn select(&mut self, _: usize) -> Result<Step> {
            Ok(Step::none())
        }
        fn activate(&mut self, _: usize) -> Result<Step> {
            Ok(Step::none())
        }
        fn key(&mut self, _: char, _: usize) -> Result<Step> {
            Ok(Step::none())
        }
        fn submit(&mut self, _: &'static str, _: &str) -> Result<Step> {
            Ok(Step::none())
        }
    }

    /// Render one frame and return its rows as text.
    pub(crate) fn render(
        shell: &mut Shell,
        view: &dyn View,
        width: u16,
        height: u16,
    ) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| draw::draw(frame, shell, view))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_owned())
                    .collect::<String>()
            })
            .collect()
    }

    /// Client and runtime that never connect; for views seeded with fixture data.
    pub(crate) fn offline() -> (fleetworks_contracts::Client, tokio::runtime::Runtime) {
        (
            fleetworks_contracts::Client::new("http://127.0.0.1:9", None).unwrap(),
            tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap(),
        )
    }

    /// Render a view at the standard sizes, assert the shared invariants and,
    /// when `FLEETWORKS_TUI_DUMP` names a directory, write cell dumps for
    /// `scripts/render_tui_cells.py`.
    pub(crate) fn capture(name: &str, shell: &mut Shell, view: &dyn View) {
        for (width, height) in [(160, 50), (120, 40), (100, 32), (80, 24), (60, 18)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| draw::draw(frame, shell, view))
                .unwrap();
            let buffer = terminal.backend().buffer().clone();
            let header: String = (0..width).map(|x| buffer[(x, 0)].symbol()).collect();
            assert!(header.contains("Fleetworks"), "{name} {width}x{height}");
            if let Ok(dir) = std::env::var("FLEETWORKS_TUI_DUMP") {
                let cells: Vec<Vec<serde_json::Value>> = (0..height)
                    .map(|y| {
                        (0..width)
                            .map(|x| {
                                let cell = &buffer[(x, y)];
                                serde_json::json!({
                                    "s": cell.symbol(),
                                    "fg": format!("{:?}", cell.fg),
                                    "bg": format!("{:?}", cell.bg),
                                    "b": cell.modifier.contains(ratatui::style::Modifier::BOLD),
                                    "u": cell.modifier.contains(ratatui::style::Modifier::UNDERLINED),
                                })
                            })
                            .collect()
                    })
                    .collect();
                std::fs::create_dir_all(&dir).unwrap();
                let dump = serde_json::json!({"width": width, "height": height, "cells": cells});
                std::fs::write(
                    format!("{dir}/{name}-{width}x{height}.json"),
                    serde_json::to_vec(&dump).unwrap(),
                )
                .unwrap();
            }
        }
    }
}
