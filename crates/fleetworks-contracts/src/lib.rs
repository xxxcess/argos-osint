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
/// Cancel a durable job.
pub const JOB_CANCEL_PATH: &str = "/api/v1/jobs/cancel";
/// Submit an Atlas collection cycle.
pub const ATLAS_CYCLES_PATH: &str = "/api/v1/atlas/cycles";
/// Durable operation name for Atlas collection.
pub const ATLAS_CYCLE_OPERATION: &str = "atlas_cycle";
const ATLAS_JOB_PREFIX: &str = "fleet-atlas-job-";

/// Stable request identity for a single Atlas collection cycle.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AtlasCycleRequest {
    pub request_id: String,
}

impl AtlasCycleRequest {
    /// Build a request from 1–64 ASCII letters, digits, dashes or underscores.
    pub fn new(request_id: String) -> Option<Self> {
        valid_atlas_request_id(&request_id).then_some(Self { request_id })
    }

    /// Compute the durable IDs, including for requests decoded from JSON.
    pub fn receipt(&self) -> Option<AtlasCycleReceipt> {
        AtlasCycleReceipt::from_request_id(&self.request_id)
    }
}

/// Durable identities for an accepted Atlas cycle.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct AtlasCycleReceipt {
    pub job_id: String,
    pub run_id: String,
}

impl AtlasCycleReceipt {
    /// Derive the same identities for every valid replay of this request.
    pub fn from_request_id(request_id: &str) -> Option<Self> {
        valid_atlas_request_id(request_id).then(|| Self {
            job_id: format!("{ATLAS_JOB_PREFIX}{request_id}"),
            run_id: format!("fleet-atlas-run-{request_id}"),
        })
    }

    /// Resolve a job ID owned by this dispatch contract.
    pub fn from_job_id(job_id: &str) -> Option<Self> {
        Self::from_request_id(job_id.strip_prefix(ATLAS_JOB_PREFIX)?)
    }

    /// The only collection task for this request.
    pub fn task_id(&self) -> String {
        format!("{}-collect", self.job_id)
    }
}

fn valid_atlas_request_id(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

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

#[cfg(test)]
mod tests {
    use super::{AtlasCycleReceipt, AtlasCycleRequest};

    #[test]
    fn atlas_cycle_ids_are_stable_and_exclude_non_identifier_input() {
        let request = AtlasCycleRequest::new("cycle_2026-11-09".into()).unwrap();
        let receipt = request.receipt().unwrap();
        assert_eq!(receipt.job_id, "fleet-atlas-job-cycle_2026-11-09");
        assert_eq!(receipt.run_id, "fleet-atlas-run-cycle_2026-11-09");
        assert_eq!(
            receipt.task_id(),
            "fleet-atlas-job-cycle_2026-11-09-collect"
        );
        assert_eq!(
            AtlasCycleReceipt::from_job_id(&receipt.job_id),
            Some(receipt)
        );
        assert!(AtlasCycleReceipt::from_request_id(&"a".repeat(64)).is_some());
        for value in ["", "../cycle", "cycle with spaces", "🧪", &"a".repeat(65)] {
            assert!(AtlasCycleReceipt::from_request_id(value).is_none());
        }
    }
}
