//! Private, rebuildable actor discovery projection for the Linux backend.
//!
//! No database, inference engine, GUI or deployment supervisor is linked here.

pub mod config;
pub mod engine;
pub mod http;
pub mod pressure;
mod query;
mod schema;

use std::fmt;

use fleetworks_contracts::actor_index::ProtocolError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    Invalid,
    Conflict,
    Unavailable,
}

/// Stable safe reason codes; underlying storage diagnostics never enter HTTP.
#[derive(Debug)]
pub struct IndexError {
    pub kind: ErrorKind,
    pub code: &'static str,
    source: Option<anyhow::Error>,
}

impl IndexError {
    pub fn invalid(code: &'static str) -> Self {
        Self {
            kind: ErrorKind::Invalid,
            code,
            source: None,
        }
    }
    pub fn conflict(code: &'static str) -> Self {
        Self {
            kind: ErrorKind::Conflict,
            code,
            source: None,
        }
    }
    pub fn unavailable(code: &'static str) -> Self {
        Self {
            kind: ErrorKind::Unavailable,
            code,
            source: None,
        }
    }
    pub fn protocol(error: ProtocolError) -> Self {
        Self::invalid(error.0)
    }
    pub fn storage(error: impl Into<anyhow::Error>) -> Self {
        Self {
            kind: ErrorKind::Unavailable,
            code: "index_storage_unavailable",
            source: Some(error.into()),
        }
    }
}

impl fmt::Display for IndexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}
impl std::error::Error for IndexError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|error| error.as_ref())
    }
}
