//! Argos OSINT core: cases, hardware, brain, providers, search, and Gmail.
//!
//! The terminal UI lives in `argos-osint-bin`. This crate is the part that
//! can run headless and under test.

pub mod agent;
pub mod brain;
pub mod gmail;
pub mod grok_oauth;
pub mod hardware;
pub mod mcp;
pub mod paths;
pub mod prompt;
pub mod provider;
pub mod report;
pub mod search;
pub mod secrets;
pub mod session;
pub mod store;
pub mod tna;

pub use session::{resolve_case, Card, CardOption, Case};
