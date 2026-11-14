//! Typed complete-cohort read against the existing exclusive Jobs log owner.
//! Missing owner never falls back to a domain/global/lifecycle event query.

use anyhow::Result;
use fleetworks_jobs::scoped_logs::ScopedLogRead;

use super::super::operational_logs::CompleteLogCohort;
use super::super::operational_logs::LogDirection;
use super::super::operational_logs::LogFilters;
use super::super::operational_logs::ScopedLogPage;

pub async fn page(
    cohort: &CompleteLogCohort,
    filters: LogFilters,
    direction: LogDirection,
    upper_seq: Option<i64>,
    boundary: Option<i64>,
    limit: usize,
) -> Result<ScopedLogPage> {
    if cohort.is_empty() {
        return Ok(ScopedLogPage::empty());
    }
    let read = ScopedLogRead::new(cohort.ids(), filters, direction, upper_seq, boundary, limit)?;
    let value = super::execute(super::LogCommand::ScopedPage { read }).await?;
    Ok(serde_json::from_value(value)?)
}
