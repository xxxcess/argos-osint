//! Bounded workstation cycle history; deletion hides an association, never shared evidence.
use serde::Deserialize;
use serde::Serialize;

use crate::context::MutationContext;

pub const CYCLE_HISTORY_PATH: &str = "/api/v1/atlas/cycles";
pub const CYCLE_HISTORY_DELETE_PATH: &str = "/api/v1/atlas/cycles/delete";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct CycleSummary {
    pub job_id: String,
    pub run_id: String,
    pub state: String,
    pub created_at: String,
    pub finished_at: Option<String>,
    /// Missing means unavailable; clients must not display an invented zero.
    pub article_count: Option<u64>,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct CycleHistory {
    pub rows: Vec<CycleSummary>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DeleteCycleRequest {
    pub request_id: String,
    pub context: MutationContext,
    pub job_id: String,
    pub expected_revision: u64,
}

impl DeleteCycleRequest {
    #[must_use]
    pub fn new(
        request_id: String,
        context: MutationContext,
        job_id: String,
        expected_revision: u64,
    ) -> Self {
        Self {
            request_id,
            context,
            job_id,
            expected_revision,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct DeleteCycleReceipt {
    pub job_id: String,
    pub revision: u64,
    pub hidden: bool,
}

impl CycleSummary {
    #[must_use]
    pub fn new(
        job_id: String,
        run_id: String,
        state: String,
        created_at: String,
        finished_at: Option<String>,
    ) -> Self {
        Self {
            job_id,
            run_id,
            state,
            created_at,
            finished_at,
            article_count: None,
            revision: 1,
        }
    }
}
impl CycleHistory {
    #[must_use]
    pub fn new(rows: Vec<CycleSummary>) -> Self {
        Self { rows }
    }
}
impl DeleteCycleReceipt {
    #[must_use]
    pub fn hidden(job_id: String) -> Self {
        Self {
            job_id,
            revision: 2,
            hidden: true,
        }
    }
}
