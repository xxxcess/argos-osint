//! Versioned public API routes, wire types, and a bounded client.

mod client;

pub use client::Client;
pub use client::ClientError;

use serde::{Deserialize, Serialize};

/// Workspace environment variable for a public Fleetworks API origin.
pub const API_ORIGIN_ENV: &str = "FLEETWORKS_API_ORIGIN";
/// Workspace environment variable for a bearer token.
pub const API_TOKEN_ENV: &str = "FLEETWORKS_API_TOKEN";
/// Workspace environment variable for the path to a bearer token file.
pub const API_TOKEN_FILE_ENV: &str = "FLEETWORKS_API_TOKEN_FILE";
/// Default API origin for a local development installation.
pub const DEFAULT_API_ORIGIN: &str = "http://127.0.0.1:8080";

/// The supported public API version.
pub const API_VERSION: u16 = 1;
/// Readiness route.
pub const READY_PATH: &str = "/api/v1/health/ready";
/// Capabilities route.
pub const CAPABILITIES_PATH: &str = "/api/v1/capabilities";
/// Recent jobs route.
pub const JOBS_PATH: &str = "/api/v1/jobs";
/// Job detail route.
pub const JOB_DETAIL_PATH: &str = "/api/v1/jobs/detail";
/// Retry failed job tasks.
pub const JOB_RETRY_PATH: &str = "/api/v1/jobs/retry";

/// Readiness of an API instance and its backing store.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct Ready {
    pub service: String,
    pub api_version: u16,
    pub ready: bool,
    pub storage: bool,
    pub instance_id: String,
}

/// Stable fields in a Jobs row. Rich consumers may read the full JSON row.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct JobSummary {
    pub id: String,
    pub app: String,
    pub title: String,
    pub operation: String,
    pub state: String,
    pub phase: String,
    pub progress_done: Option<i64>,
    pub progress_total: Option<i64>,
    pub error_summary: String,
}

/// Recent Jobs response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct Jobs {
    pub rows: Vec<JobSummary>,
}
