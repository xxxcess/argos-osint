//! Additive newest-first workstation activity, preserving the retained
//! no-sort active-first route. Opaque metadata cursors never cache job payloads.

mod cursors;

pub use cursors::JobsCursors;

use std::sync::Arc;

use argos_osint_core::jobs_view::JobStatusFilter;
use argos_osint_core::provider_diag;
use argos_osint_core::storage::jobs_paging;
use argos_osint_core::storage::jobs_paging::CreationWindow;
use argos_osint_core::storage::jobs_paging::Filters;
use argos_osint_core::storage::jobs_view;
use axum::extract::Extension;
use axum::extract::Query;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::Json;
use axum::Router;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;

use self::cursors::Record;
use self::cursors::Snapshot;
use crate::activity;
use crate::request_context::ContextFailure;
use crate::request_context::ResolvedRequest;
use crate::ApiError;
use crate::ApiResult;
use crate::ApiState;

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JobsQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    operation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    correlation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    since: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor: Option<String>,
}

pub(crate) fn routes() -> Router<ApiState> {
    Router::new()
        .route(fleetworks_contracts::JOBS_PATH, get(jobs))
        .layer(Extension(Arc::new(JobsCursors::new())))
}
fn changed() -> ApiError {
    ApiError(
        StatusCode::CONFLICT,
        "Jobs cursor expired or context/filters changed; refresh this scoped view",
    )
}
fn binding(request: &ResolvedRequest) -> Result<String, ApiError> {
    let bytes = serde_json::to_vec(&json!([
        request.scope_id(),
        request.credential_id(),
        request.purpose(),
        request.registered()?
    ]))
    .map_err(|_| crate::internal("activity cursor identity unavailable"))?;
    Ok(argos_osint_core::classifier::artifact::sha256_hex(&bytes))
}
fn status(value: &str) -> Result<JobStatusFilter, ApiError> {
    Ok(match value {
        "all" => JobStatusFilter::All,
        "active" => JobStatusFilter::Active,
        "retrying" => JobStatusFilter::Retrying,
        "failed" => JobStatusFilter::Failed,
        "completed" => JobStatusFilter::Completed,
        "stopped" => JobStatusFilter::Stopped,
        _ => return Err(crate::bad()),
    })
}
fn since(value: &str) -> Result<String, ApiError> {
    if value.is_empty() {
        return Ok(String::new());
    }
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|time| time.with_timezone(&Utc).to_rfc3339())
        .map_err(|_| crate::bad())
}
fn filters(query: &JobsQuery) -> Result<Filters, ApiError> {
    let mut filters = Filters::default();
    filters.status = status(query.status.as_deref().unwrap_or("all"))?;
    filters.app = query.app.clone().unwrap_or_default();
    filters.operation = query.operation.clone().unwrap_or_default();
    filters.kind = query.kind.clone().unwrap_or_default();
    filters.correlation_id = query.correlation_id.clone().unwrap_or_default();
    filters.text = query.text.clone().unwrap_or_default();
    filters.since = since(query.since.as_deref().unwrap_or_default())?;
    filters.validate().map_err(|_| crate::bad())?;
    Ok(filters)
}
fn overrides(query: &JobsQuery, snapshot: &Snapshot) -> Result<(), ApiError> {
    if query
        .workspace_id
        .as_ref()
        .is_some_and(|workspace| Some(workspace) != snapshot.workspace_filter.as_ref())
        || query.limit.is_some_and(|limit| limit != snapshot.limit)
        || query
            .status
            .as_ref()
            .map(|value| status(value))
            .transpose()?
            .is_some_and(|status| status != snapshot.filters.status)
        || query
            .app
            .as_ref()
            .is_some_and(|value| *value != snapshot.filters.app)
        || query
            .operation
            .as_ref()
            .is_some_and(|value| *value != snapshot.filters.operation)
        || query
            .kind
            .as_ref()
            .is_some_and(|value| *value != snapshot.filters.kind)
        || query
            .correlation_id
            .as_ref()
            .is_some_and(|value| *value != snapshot.filters.correlation_id)
        || query
            .text
            .as_ref()
            .is_some_and(|value| *value != snapshot.filters.text)
        || query
            .since
            .as_ref()
            .map(|value| since(value))
            .transpose()?
            .is_some_and(|value| value != snapshot.filters.since)
    {
        return Err(changed());
    }
    Ok(())
}

pub(crate) async fn jobs(
    State(state): State<ApiState>,
    extension: Option<Extension<ResolvedRequest>>,
    Extension(cache): Extension<Arc<JobsCursors>>,
    Query(query): Query<JobsQuery>,
) -> ApiResult {
    if query.sort.is_none()
        && query.cursor.is_none()
        && query.kind.is_none()
        && query.correlation_id.is_none()
    {
        let legacy: activity::ActivityQuery =
            serde_json::from_value(serde_json::to_value(query).map_err(crate::internal)?)
                .map_err(|_| crate::bad())?;
        return activity::jobs(State(state), extension, Query(legacy)).await;
    }
    if query.sort.as_deref() != Some("newest") || query.id.is_some() || query.job_id.is_some() {
        return Err(crate::bad());
    }
    let request = extension
        .as_ref()
        .map(|Extension(request)| request)
        .ok_or_else(ContextFailure::registration_required)?;
    let proof = binding(request)?;
    let (record, current_cursor) = if let Some(cursor) = &query.cursor {
        let record = cache.get(cursor, &proof).await.map_err(|_| changed())?;
        overrides(&query, &record.snapshot)?;
        (record, Some(cursor.clone()))
    } else {
        let limit = query.limit.unwrap_or(50);
        if !(1..=jobs_paging::MAX_PAGE).contains(&limit) {
            return Err(crate::bad());
        }
        let scope =
            activity::scope_for_request(&state.repo, request, query.workspace_id.as_deref())
                .await?;
        let snapshot = Snapshot::new(
            proof,
            scope,
            query.workspace_id.clone(),
            filters(&query)?,
            CreationWindow::at(Utc::now()),
            limit,
        );
        (Record::new(Arc::new(snapshot), None), None)
    };
    let current_scope = activity::scope_for_request(
        &state.repo,
        request,
        record.snapshot.workspace_filter.as_deref(),
    )
    .await?;
    if current_scope != record.snapshot.scope {
        return Err(changed());
    }
    let mut page = jobs_paging::page(
        state.repo.db(),
        request.scope_id(),
        &current_scope,
        &record.snapshot.filters,
        &record.snapshot.window,
        record.after.as_ref(),
        record.snapshot.limit,
    )
    .await
    .map_err(|_| ContextFailure::unavailable())?;
    for row in &mut page.rows {
        row.job.error_summary = provider_diag::sanitize(&row.job.error_summary);
    }
    activity::action_availability(&state.repo, request, &mut page.rows).await?;
    // Retained dashboard buckets intentionally keep their existing top-level,
    // unfiltered scope. They are NOT the filtered all-row match count.
    let counts =
        jobs_view::activity_job_counts(state.repo.db(), request.scope_id(), &current_scope)
            .await
            .map_err(|_| ContextFailure::unavailable())?;
    request.revalidate(&state.repo).await?;
    let verified_scope = activity::scope_for_request(
        &state.repo,
        request,
        record.snapshot.workspace_filter.as_deref(),
    )
    .await?;
    if verified_scope != current_scope {
        return Err(changed());
    }
    let page_cursor = match current_cursor {
        Some(cursor) => cursor,
        None => cache
            .issue(record.clone())
            .await
            .map_err(|_| crate::internal("activity cursor unavailable"))?,
    };
    let next_cursor = match page.next_anchor {
        Some(anchor) => Some(
            cache
                .issue(Record::new(record.snapshot.clone(), Some(anchor)))
                .await
                .map_err(|_| crate::internal("activity continuation unavailable"))?,
        ),
        None => None,
    };
    Ok(Json(
        json!({"rows":page.rows,"activity_scope":current_scope,"counts":counts,
        "dashboard_counts_scope":"unfiltered_top_level_workstation_activity",
        "relation_counts_available":false,"activity_actions_available":page.rows.iter().any(|row|row.activity_actions_available),
        "sort":"newest","has_more":page.has_more,"page_cursor":page_cursor,"next_cursor":next_cursor,
        "matched_total":page.matched_total,"observed_at":page.observed_at,"apps":page.apps,"apps_complete":page.apps_complete,
        "visibility":"current_membership_and_status_with_fixed_creation_window"}),
    ))
}
