//! Creation-keyed workstation activity. The window anchors creation time, not
//! a frozen result cohort: current lifecycle, membership and privacy still apply.

use std::collections::HashMap;

use anyhow::ensure;
use anyhow::Context;
use anyhow::Result;
use chrono::DateTime;
use chrono::Duration;
use chrono::Utc;
use fleetworks_contracts::context::ActivityScope;
use serde::Deserialize;
use serde::Serialize;
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use super::jobs;
use super::jobs_view;
use super::jobs_view::ActivityJobRow;
use super::tx;
use crate::jobs_view::JobStatusFilter;

pub const MAX_PAGE: usize = 200;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct Filters {
    pub status: JobStatusFilter,
    pub app: String,
    pub operation: String,
    pub kind: String,
    pub correlation_id: String,
    pub text: String,
    pub since: String,
}
impl Filters {
    pub fn validate(&self) -> Result<()> {
        for (value, bound) in [
            (&self.app, 256),
            (&self.operation, 256),
            (&self.kind, 64),
            (&self.correlation_id, 256),
            (&self.text, 256),
            (&self.since, 64),
        ] {
            ensure!(
                value.len() <= bound && !value.chars().any(char::is_control),
                "invalid bounded activity filter"
            );
        }
        if !self.since.is_empty() {
            DateTime::parse_from_rfc3339(&self.since)
                .context("invalid activity creation lower bound")?;
        }
        Ok(())
    }
}

/// Inclusive immutable creation bounds. The same exact 72-hour window is
/// reused on every cursor request, even while status and membership evolve.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct CreationWindow {
    pub cutoff: String,
    pub upper: String,
}
impl CreationWindow {
    pub fn at(now: DateTime<Utc>) -> Self {
        Self {
            cutoff: (now - Duration::hours(72)).to_rfc3339(),
            upper: now.to_rfc3339(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        let cutoff = DateTime::parse_from_rfc3339(&self.cutoff)?;
        let upper = DateTime::parse_from_rfc3339(&self.upper)?;
        ensure!(
            upper - cutoff == Duration::hours(72),
            "fixed72hour activity window required"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct Anchor {
    pub created_at: String,
    pub id: String,
}
impl Anchor {
    pub fn new(created_at: String, id: String) -> Result<Self> {
        DateTime::parse_from_rfc3339(&created_at)?;
        ensure!(
            !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control),
            "invalid activity page anchor"
        );
        Ok(Self { created_at, id })
    }
}

#[non_exhaustive]
pub struct Page {
    pub rows: Vec<ActivityJobRow>,
    pub has_more: bool,
    pub next_anchor: Option<Anchor>,
    pub matched_total: Option<u64>,
    pub apps: Vec<String>,
    pub apps_complete: bool,
    pub observed_at: String,
}

#[derive(SurrealValue)]
struct Candidate {
    key: String,
    created_at_text: String,
}
#[derive(SurrealValue)]
struct Privacy {
    key: String,
    available: bool,
}
#[derive(SurrealValue)]
struct Count {
    n: i64,
}

fn predicate(filters: &Filters) -> String {
    let mut sql = format!(
        "kind!='service' AND {} AND created_at <= <datetime>$paging_upper AND ({})",
        jobs_view::ELIGIBLE,
        jobs_view::activity_membership()
    );
    if !filters.status.states().is_empty() {
        sql.push_str(" AND state IN $paging_states");
    }
    for (value, column, binding) in [
        (&filters.app, "app", "paging_app"),
        (&filters.operation, "operation", "paging_operation"),
        (&filters.kind, "kind", "paging_kind"),
    ] {
        if !value.is_empty() {
            sql.push_str(&format!(" AND {column}=${binding}"));
        }
    }
    if !filters.correlation_id.is_empty() {
        sql.push_str(&format!(
            " AND ({}) AND correlation_id=$paging_correlation",
            jobs_view::activity_private()
        ));
    }
    if !filters.text.trim().is_empty() {
        sql.push_str(&format!(" AND (string::contains(string::lowercase(operation),$paging_text) OR string::contains(string::lowercase(<string>record::id(id)),$paging_text) OR (({}) AND (string::contains(string::lowercase(title),$paging_text) OR string::contains(string::lowercase(error_summary),$paging_text))))",jobs_view::activity_private()));
    }
    if !filters.since.is_empty() {
        sql.push_str(" AND created_at >= <datetime>$paging_since");
    }
    sql
}

/// Apply current membership and every filter before keyset selection, counts
/// and app facets. Causal rows require durable origin; parent links grant none.
/// No current full-cohort immutability or historical status snapshot is claimed.
pub async fn page(
    db: &Surreal<Any>,
    authority_scope: &str,
    scope: &ActivityScope,
    filters: &Filters,
    window: &CreationWindow,
    after: Option<&Anchor>,
    limit: usize,
) -> Result<Page> {
    jobs_view::validate_activity_scope(authority_scope, scope)?;
    jobs_view::require_activity_schema(db).await?;
    filters.validate()?;
    window.validate()?;
    ensure!(
        (1..=MAX_PAGE).contains(&limit),
        "invalid activity page limit"
    );
    if let Some(after) = after {
        Anchor::new(after.created_at.clone(), after.id.clone())?;
        let time = DateTime::parse_from_rfc3339(&after.created_at)?;
        ensure!(
            time >= DateTime::parse_from_rfc3339(&window.cutoff)?
                && time <= DateTime::parse_from_rfc3339(&window.upper)?,
            "anchor outside fixed activity window"
        );
    }
    let matched = predicate(filters);
    let boundary = if after.is_some() {
        " AND (created_at < <datetime>$paging_after_time OR (created_at = <datetime>$paging_after_time AND record::id(id)>$paging_after_id))"
    } else {
        ""
    };
    let sql=format!("LET $paging_visible=(SELECT id, record::id(id) AS key, created_at, <string>created_at AS created_at_text FROM job WHERE {matched}{boundary} ORDER BY created_at DESC,key ASC LIMIT $paging_limit);
        SELECT key,created_at,created_at_text FROM $paging_visible ORDER BY created_at DESC,key ASC;
        SELECT {fields},{relations} FROM job WHERE id IN $paging_visible.id AND ({matched});
        SELECT record::id(id) AS key, (({private}) ?? false) AS available FROM job WHERE id IN $paging_visible.id AND ({matched});
        SELECT count() AS n FROM job WHERE {matched} GROUP ALL;
        SELECT VALUE app FROM job WHERE {matched} AND app!='' GROUP BY app ORDER BY app LIMIT 33;",
        fields=jobs_view::ROW_FIELDS,relations=jobs_view::ACTIVITY_RELATIONS_UNAVAILABLE,private=jobs_view::activity_private());
    let committed = tx::run_tx(db, tx::RetryPolicy::default(), |transaction| {
        let sql = sql.clone();
        async move {
            let states: Vec<String> = filters
                .status
                .states()
                .iter()
                .map(|state| (*state).into())
                .collect();
            let mut query = transaction
                .query(sql)
                .bind(("activity_scope", authority_scope.to_owned()))
                .bind(("activity_deployment", scope.deployment_id.clone()))
                .bind(("activity_node", scope.workstation_id.clone()))
                .bind(("cutoff", window.cutoff.clone()))
                .bind(("paging_upper", window.upper.clone()))
                .bind(("paging_states", states))
                .bind(("paging_app", filters.app.clone()))
                .bind(("paging_operation", filters.operation.clone()))
                .bind(("paging_kind", filters.kind.clone()))
                .bind(("paging_correlation", filters.correlation_id.clone()))
                .bind(("paging_text", filters.text.trim().to_lowercase()))
                .bind(("paging_since", filters.since.clone()))
                .bind(("paging_limit", (limit + 1) as i64));
            if let Some(after) = after {
                query = query
                    .bind(("paging_after_time", after.created_at.clone()))
                    .bind(("paging_after_id", after.id.clone()));
            }
            let mut response = jobs::run(&transaction, query).await?;
            let candidates: Vec<Candidate> = jobs::take(&mut response, 1)?;
            let raw: Vec<jobs_view::Raw> = jobs::take(&mut response, 2)?;
            let privacy: Vec<Privacy> = jobs::take(&mut response, 3)?;
            let counts: Vec<Count> = jobs::take(&mut response, 4)?;
            let apps: Vec<String> = jobs::take(&mut response, 5)?;
            Ok((candidates, raw, privacy, counts, apps))
        }
    })
    .await
    .context("read current scoped activity page")?
    .value;
    let (mut candidates, raw, privacy, counts, mut apps) = committed;
    ensure!(
        candidates.len() <= limit + 1 && apps.len() <= 33 && counts.len() <= 1,
        "invalid bounded activity projection"
    );
    let has_more = candidates.len() > limit;
    candidates.truncate(limit);
    let privacy: HashMap<_, _> = privacy
        .into_iter()
        .map(|row| (row.key, row.available))
        .collect();
    let mut raw: HashMap<_, _> = raw.into_iter().map(|row| (row.key.clone(), row)).collect();
    let mut rows = Vec::with_capacity(candidates.len());
    for candidate in &candidates {
        let row = raw
            .remove(&candidate.key)
            .context("activity changed while reading page")?;
        rows.push(jobs_view::activity_row(
            row,
            privacy.get(&candidate.key).copied().unwrap_or(false),
        ));
    }
    let next_anchor = if has_more {
        candidates
            .last()
            .map(|row| Anchor::new(row.created_at_text.clone(), row.key.clone()))
            .transpose()?
    } else {
        None
    };
    let total = counts.first().map_or(0, |count| count.n);
    let matched_total = Some(u64::try_from(total).context("negative observed activity count")?);
    for app in &apps {
        ensure!(
            app.len() <= 256 && !app.chars().any(char::is_control),
            "invalid bounded activity app facet"
        );
    }
    let apps_complete = apps.len() <= 32;
    apps.truncate(32);
    Ok(Page {
        rows,
        has_more,
        next_anchor,
        matched_total,
        apps,
        apps_complete,
        observed_at: Utc::now().to_rfc3339(),
    })
}
