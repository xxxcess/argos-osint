//! CLI dispatch and terminal shell for the `argos` binary, exposed as a
//! library so integration tests can link the same code.

pub mod cli;
#[cfg(test)]
mod test_storage;
pub mod tui;
