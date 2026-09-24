mod app;
mod theme;
mod ui;

pub use app::{report_dir, run, App};

use argos_osint_core::session::Card;

/// Slash menu for the prompt. Kept `pub(crate)` at the TUI boundary.
pub(crate) fn slash_menu(query: &str) -> Card {
    argos_osint_core::session::slash_menu(query)
}
