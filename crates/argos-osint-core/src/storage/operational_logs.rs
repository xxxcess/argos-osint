//! Complete authorized operational-log cohorts, never bounded ID approximations.
//! Reuses exactly the activity reader's membership, private and lifecycle policy.

use std::sync::Arc;

use chrono::Utc;
use fleetworks_contracts::context::ActivityScope;
use surrealdb::types::SurrealValue;

pub use fleetworks_jobs::scoped_logs::LogDirection;
pub use fleetworks_jobs::scoped_logs::LogFilters;
pub use fleetworks_jobs::scoped_logs::OperationalLog;
pub use fleetworks_jobs::scoped_logs::ScopedLogCounts;
pub use fleetworks_jobs::scoped_logs::ScopedLogPage;
/// Provision only in the actual Jobs process (or a disposable fixture).
pub use fleetworks_jobs::Store as OperationalLogStore;

use super::jobs;
use super::jobs_view;
use super::repo::Repository;

pub const COHORT_LIMIT: usize = 512;
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct LogSelection {
    pub job_id: Option<String>,
    pub descendants: bool,
}
impl LogSelection {
    pub fn workstation() -> Self {
        Self {
            job_id: None,
            descendants: true,
        }
    }
    pub fn job(id: String, descendants: bool) -> Result<Self, CohortError> {
        if id.is_empty() || id.len() > 512 || id.chars().any(char::is_control) {
            return Err(CohortError::Invalid);
        }
        Ok(Self {
            job_id: Some(id),
            descendants,
        })
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CohortError {
    Invalid,
    Missing,
    Unavailable,
}
impl std::fmt::Display for CohortError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Scoped operational log authority unavailable")
    }
}
impl std::error::Error for CohortError {}

/// Not deserializable: only the authoritative planner constructs these IDs.
#[derive(Clone)]
pub struct CompleteLogCohort {
    ids: Arc<Vec<String>>,
    digest: String,
    excluded_shared: bool,
}
impl CompleteLogCohort {
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
    pub fn len(&self) -> usize {
        self.ids.len()
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn excluded_shared(&self) -> bool {
        self.excluded_shared
    }
    pub fn memory_bytes(&self) -> usize {
        self.ids
            .iter()
            .map(|id| id.len() + std::mem::size_of::<String>())
            .sum::<usize>()
            + self.digest.len()
    }
    pub(crate) fn ids(&self) -> Vec<String> {
        self.ids.as_ref().clone()
    }
}
#[non_exhaustive]
pub enum CohortPlan {
    Complete(CompleteLogCohort),
    TooLarge,
    SharedPrivateUnavailable,
}
#[derive(SurrealValue)]
struct CohortRow {
    key: String,
}

/// Select every eligible private job in this scope or refuse completeness.
/// A selected parent link alone never grants descendant membership.
pub async fn plan(
    repo: &Repository,
    authority_scope: &str,
    scope: &ActivityScope,
    selection: &LogSelection,
) -> Result<CohortPlan, CohortError> {
    jobs_view::validate_activity_scope(authority_scope, scope).map_err(|_| CohortError::Invalid)?;
    jobs_view::require_activity_schema(repo.db())
        .await
        .map_err(|_| CohortError::Unavailable)?;
    if let Some(selected) = &selection.job_id {
        let root = jobs_view::get_activity_job(repo.db(), authority_scope, scope, selected)
            .await
            .map_err(|_| CohortError::Unavailable)?
            .ok_or(CohortError::Missing)?;
        if !root.private_details_available {
            return Ok(CohortPlan::SharedPrivateUnavailable);
        }
    }
    if selection.descendants && selection.job_id.is_some() {
        repo.db()
            .query("INFO FOR TABLE job_ancestor;")
            .await
            .map_err(|_| CohortError::Unavailable)?
            .check()
            .map_err(|_| CohortError::Unavailable)?;
    }
    let mut where_sql = format!(
        "kind!='service' AND {} AND ({})",
        jobs_view::ELIGIBLE,
        jobs_view::activity_membership()
    );
    if selection.job_id.is_some() {
        where_sql.push_str(if selection.descendants {" AND (id=$selected OR id IN (SELECT VALUE job FROM job_ancestor WHERE ancestor=$selected))"}else{" AND id=$selected"});
    }
    let sql=format!("SELECT record::id(id) AS key FROM job WHERE {where_sql} AND ({private}) ORDER BY key LIMIT $n;
        SELECT VALUE record::id(id) FROM job WHERE {where_sql} AND !(({private}) ?? false) LIMIT 1;",private=jobs_view::activity_private());
    let mut query = repo
        .db()
        .query(sql)
        .bind(("activity_scope", authority_scope.to_owned()))
        .bind(("activity_deployment", scope.deployment_id.clone()))
        .bind(("activity_node", scope.workstation_id.clone()))
        .bind(("activity_workspace", scope.workspace_id.clone()))
        .bind(("cutoff", jobs_view::cutoff(Utc::now())))
        .bind(("n", (COHORT_LIMIT + 1) as i64));
    if let Some(selected) = &selection.job_id {
        query = query.bind(("selected", jobs::rid("job", selected)));
    }
    let mut result = query
        .await
        .map_err(|_| CohortError::Unavailable)?
        .check()
        .map_err(|_| CohortError::Unavailable)?;
    let rows: Vec<CohortRow> = result.take(0).map_err(|_| CohortError::Unavailable)?;
    if rows.len() > COHORT_LIMIT {
        return Ok(CohortPlan::TooLarge);
    }
    let excluded: Vec<String> = result.take(1).map_err(|_| CohortError::Unavailable)?;
    let ids: Vec<_> = rows.into_iter().map(|row| row.key).collect();
    let mut framed = String::new();
    for id in &ids {
        framed.push_str(&format!("{}:{id}", id.len()));
    }
    framed.push_str(if excluded.is_empty() {
        "private-complete"
    } else {
        "shared-excluded"
    });
    let digest = crate::classifier::artifact::sha256_hex(framed.as_bytes());
    Ok(CohortPlan::Complete(CompleteLogCohort {
        ids: Arc::new(ids),
        digest,
        excluded_shared: !excluded.is_empty(),
    }))
}
