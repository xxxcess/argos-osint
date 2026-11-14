//! Workstation operational logs, complete scoped pages/export and bounded follow.
//! Jobs SQLite owns payloads. Domain activity policy owns cohort authority.

mod cursors;

pub use cursors::LogCursors;

use std::sync::Arc;
use std::time::Duration;

use argos_osint_core::provider_diag;
use argos_osint_core::storage::jobs_client;
use argos_osint_core::storage::operational_logs;
use argos_osint_core::storage::operational_logs::CohortError;
use argos_osint_core::storage::operational_logs::CohortPlan;
use argos_osint_core::storage::operational_logs::CompleteLogCohort;
use argos_osint_core::storage::operational_logs::LogDirection;
use argos_osint_core::storage::operational_logs::LogFilters;
use argos_osint_core::storage::operational_logs::LogSelection;
use argos_osint_core::storage::operational_logs::OperationalLog;
use argos_osint_core::storage::operational_logs::ScopedLogCounts;
use argos_osint_core::storage::operational_logs::ScopedLogPage;
use axum::extract::Extension;
use axum::extract::Query;
use axum::extract::State;
use axum::http::header;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use axum::Json;
use axum::Router;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use tokio::time::Instant;

use self::cursors::Kind;
use self::cursors::Record;
use self::cursors::Snapshot;
use crate::activity;
use crate::request_context::ContextFailure;
use crate::request_context::ResolvedRequest;
use crate::ApiError;
use crate::ApiState;

const FOLLOW_LIMIT: Duration = Duration::from_secs(5);
const AUTHORITY_CHECK_LIMIT: Duration = Duration::from_secs(4);
const TICK: Duration = Duration::from_secs(1);
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LogQuery {
    #[serde(alias = "id")]
    job_id: Option<String>,
    workspace_id: Option<String>,
    descendants: Option<bool>,
    min_rank: Option<i64>,
    app: Option<String>,
    text: Option<String>,
    since: Option<String>,
    limit: Option<usize>,
    cursor: Option<String>,
    wait_ms: Option<u64>,
}
#[derive(Serialize)]
struct PublicPage {
    rows: Vec<OperationalLog>,
    counts: Option<ScopedLogCounts>,
    apps: Vec<String>,
    apps_complete: bool,
    next_cursor: Option<String>,
    follow_cursor: Option<String>,
    available: bool,
    private_logs_only: bool,
    shared_private_logs_available: bool,
    reason_code: Option<&'static str>,
    narrowing_hint: Option<&'static str>,
    has_more: bool,
}
pub(crate) fn routes() -> Router<ApiState> {
    Router::new()
        .route("/api/v1/logs", get(page))
        .route("/api/v1/logs/export", get(export))
        .route("/api/v1/logs/follow", get(follow))
        .layer(Extension(Arc::new(LogCursors::new())))
}
fn context(extension: Option<Extension<ResolvedRequest>>) -> Result<ResolvedRequest, ApiError> {
    extension
        .map(|Extension(context)| context)
        .ok_or_else(|| ContextFailure::registration_required().into())
}
fn binding(context: &ResolvedRequest) -> Result<String, ApiError> {
    let raw = serde_json::to_vec(&json!([
        context.scope_id(),
        context.credential_id(),
        context.purpose(),
        context.registered()?
    ]))
    .map_err(|_| crate::internal("log cursor identity unavailable"))?;
    Ok(argos_osint_core::classifier::artifact::sha256_hex(&raw))
}
fn normalize_since(value: &str) -> Result<String, ApiError> {
    if value.is_empty() {
        return Ok(String::new());
    }
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|time| time.with_timezone(&chrono::Utc).to_rfc3339())
        .map_err(|_| crate::bad())
}
fn filters(query: &LogQuery) -> Result<LogFilters, ApiError> {
    let mut filter = LogFilters::default();
    filter.min_rank = query.min_rank;
    filter.app = query.app.clone().unwrap_or_default();
    filter.text = query.text.clone().unwrap_or_default();
    filter.since = normalize_since(query.since.as_deref().unwrap_or_default())?;
    filter.validate().map_err(|_| crate::bad())?;
    Ok(filter)
}
fn overrides(query: &LogQuery, snapshot: &Snapshot) -> Result<(), ApiError> {
    if query
        .job_id
        .as_ref()
        .is_some_and(|id| Some(id) != snapshot.selection.job_id.as_ref())
        || query
            .workspace_id
            .as_ref()
            .is_some_and(|id| Some(id) != snapshot.workspace_filter.as_ref())
        || query
            .descendants
            .is_some_and(|value| value != snapshot.selection.descendants)
        || query.limit.is_some_and(|value| value != snapshot.limit)
        || query
            .min_rank
            .is_some_and(|value| Some(value) != snapshot.filters.min_rank)
        || query
            .app
            .as_ref()
            .is_some_and(|value| *value != snapshot.filters.app)
        || query
            .text
            .as_ref()
            .is_some_and(|value| *value != snapshot.filters.text)
        || query
            .since
            .as_ref()
            .map(|value| normalize_since(value))
            .transpose()?
            .is_some_and(|value| value != snapshot.filters.since)
    {
        return Err(changed());
    }
    Ok(())
}
fn changed() -> ApiError {
    ApiError(
        StatusCode::CONFLICT,
        "Log cursor context changed or expired; restart this scoped view",
    )
}
fn unavailable(reason: &'static str, hint: &'static str) -> PublicPage {
    PublicPage {
        rows: Vec::new(),
        counts: None,
        apps: Vec::new(),
        apps_complete: false,
        next_cursor: None,
        follow_cursor: None,
        available: false,
        private_logs_only: true,
        shared_private_logs_available: false,
        reason_code: Some(reason),
        narrowing_hint: Some(hint),
        has_more: false,
    }
}
fn plan_error(error: CohortError) -> ApiError {
    match error {
        CohortError::Invalid => crate::bad(),
        CohortError::Missing => crate::missing(),
        _ => ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Scoped log authority unavailable",
        ),
    }
}

async fn initial(
    state: &ApiState,
    context: &ResolvedRequest,
    query: &LogQuery,
) -> Result<(Arc<Snapshot>, CohortPlan), ApiError> {
    let limit = query.limit.unwrap_or(100);
    if !(1..=100).contains(&limit) {
        return Err(crate::bad());
    }
    let scope =
        activity::scope_for_request(&state.repo, context, query.workspace_id.as_deref()).await?;
    let selection = match &query.job_id {
        Some(id) => {
            LogSelection::job(id.clone(), query.descendants.unwrap_or(false)).map_err(plan_error)?
        }
        None => LogSelection::workstation(),
    };
    let filters = filters(query)?;
    let filter_digest = argos_osint_core::classifier::artifact::sha256_hex(
        &serde_json::to_vec(&json!([
            selection.job_id,
            selection.descendants,
            query.workspace_id,
            filters,
            limit
        ]))
        .map_err(|_| crate::bad())?,
    );
    let plan = operational_logs::plan(&state.repo, context.scope_id(), &scope, &selection)
        .await
        .map_err(plan_error)?;
    let cohort_digest = match &plan {
        CohortPlan::Complete(cohort) => cohort.digest().to_owned(),
        _ => String::new(),
    };
    let snapshot = Arc::new(Snapshot {
        binding: binding(context)?,
        scope,
        workspace_filter: query.workspace_id.clone(),
        selection,
        filters,
        limit,
        cohort_digest,
        filter_digest,
    });
    Ok((snapshot, plan))
}
async fn cohort(
    state: &ApiState,
    context: &ResolvedRequest,
    snapshot: &Snapshot,
) -> Result<CompleteLogCohort, ApiError> {
    if binding(context)? != snapshot.binding {
        return Err(changed());
    }
    let scope =
        activity::scope_for_request(&state.repo, context, snapshot.workspace_filter.as_deref())
            .await?;
    if scope != snapshot.scope {
        return Err(changed());
    }
    let plan = operational_logs::plan(&state.repo, context.scope_id(), &scope, &snapshot.selection)
        .await
        .map_err(plan_error)?;
    match plan {
        CohortPlan::Complete(cohort) if cohort.digest() == snapshot.cohort_digest => Ok(cohort),
        _ => Err(changed()),
    }
}
async fn read(
    state: &ApiState,
    context: &ResolvedRequest,
    snapshot: &Snapshot,
    direction: LogDirection,
    upper: Option<i64>,
    boundary: Option<i64>,
) -> Result<(ScopedLogPage, bool), ApiError> {
    let selected = cohort(state, context, snapshot).await?;
    let page = jobs_client::scoped_logs::page(
        &selected,
        snapshot.filters.clone(),
        direction,
        upper,
        boundary,
        snapshot.limit,
    )
    .await
    .map_err(|_| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Operational Jobs log owner unavailable",
        )
    })?;
    let current = cohort(state, context, snapshot).await?;
    context.revalidate(&state.repo).await?;
    Ok((page, current.excluded_shared()))
}
async fn start(
    state: &ApiState,
    context: &ResolvedRequest,
    cache: &LogCursors,
    query: &LogQuery,
    kind: Kind,
) -> Result<Result<Record, PublicPage>, ApiError> {
    if let Some(cursor) = &query.cursor {
        let record = cache
            .get(cursor, &binding(context)?, kind)
            .await
            .map_err(|_| changed())?;
        overrides(query, &record.snapshot)?;
        return Ok(Ok(record));
    }
    let (snapshot, plan) = initial(state, context, query).await?;
    context.revalidate(&state.repo).await?;
    match plan {
        CohortPlan::Complete(_)=>Ok(Ok(Record::next(snapshot,kind,None,None))),
        CohortPlan::TooLarge=>Ok(Err(unavailable("scope_exceeds_complete_log_cohort_limit","Select a workspace or an attributed job; this view requires at most 512 complete authorized jobs"))),
        CohortPlan::SharedPrivateUnavailable=>Ok(Err(unavailable("shared_private_logs_unavailable","This attributed job has mixed private origins"))),
        _=>Err(changed()),
    }
}
fn safe_rows(page: &ScopedLogPage) -> Vec<OperationalLog> {
    page.rows
        .iter()
        .map(|event| {
            let mut row = event.row.clone();
            row.message = provider_diag::sanitize(&row.message);
            row
        })
        .collect()
}
fn protected(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}

pub(crate) async fn page(
    State(state): State<ApiState>,
    extension: Option<Extension<ResolvedRequest>>,
    Extension(cache): Extension<Arc<LogCursors>>,
    Query(query): Query<LogQuery>,
) -> Result<Response, ApiError> {
    let context = context(extension)?;
    let record = match start(&state, &context, &cache, &query, Kind::Page).await? {
        Ok(record) => record,
        Err(page) => return Ok(protected(Json(page).into_response())),
    };
    let (page, shared) = read(
        &state,
        &context,
        &record.snapshot,
        LogDirection::Older,
        record.upper,
        record.boundary,
    )
    .await?;
    let next_cursor = if page.has_more {
        Some(
            cache
                .issue(Record::next(
                    record.snapshot.clone(),
                    Kind::Page,
                    Some(page.upper_seq),
                    page.next_boundary,
                ))
                .await
                .map_err(|_| crate::internal("log cursor unavailable"))?,
        )
    } else {
        None
    };
    let follow_cursor = if query.cursor.is_none() {
        Some(
            cache
                .issue(Record::next(
                    record.snapshot.clone(),
                    Kind::Follow,
                    None,
                    Some(page.upper_seq),
                ))
                .await
                .map_err(|_| crate::internal("log cursor unavailable"))?,
        )
    } else {
        None
    };
    Ok(protected(
        Json(PublicPage {
            rows: safe_rows(&page),
            counts: Some(page.counts),
            apps: page.apps,
            apps_complete: page.apps_complete,
            next_cursor,
            follow_cursor,
            available: true,
            private_logs_only: true,
            shared_private_logs_available: !shared,
            reason_code: None,
            narrowing_hint: None,
            has_more: page.has_more,
        })
        .into_response(),
    ))
}
pub(crate) async fn export(
    State(state): State<ApiState>,
    extension: Option<Extension<ResolvedRequest>>,
    Extension(cache): Extension<Arc<LogCursors>>,
    Query(query): Query<LogQuery>,
) -> Result<Response, ApiError> {
    let context = context(extension)?;
    let record = match start(&state, &context, &cache, &query, Kind::Export).await? {
        Ok(record) => record,
        Err(page) => {
            return Ok(protected(
                (StatusCode::CONFLICT, Json(page)).into_response(),
            ))
        }
    };
    let (page, _) = read(
        &state,
        &context,
        &record.snapshot,
        LogDirection::Older,
        record.upper,
        record.boundary,
    )
    .await?;
    let mut body = String::new();
    for row in safe_rows(&page) {
        body.push_str(
            &serde_json::to_string(&row)
                .map_err(|_| crate::internal("safe log export unavailable"))?,
        );
        body.push('\n');
    }
    let mut response = (
        [
            (header::CONTENT_TYPE, "application/x-ndjson"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=workstation-logs.ndjson",
            ),
        ],
        body,
    )
        .into_response();
    response.headers_mut().insert(
        "x-fleetworks-log-apps-complete",
        HeaderValue::from_static(if page.apps_complete { "true" } else { "false" }),
    );
    if page.has_more {
        let token = cache
            .issue(Record::next(
                record.snapshot,
                Kind::Export,
                Some(page.upper_seq),
                page.next_boundary,
            ))
            .await
            .map_err(|_| crate::internal("log export cursor unavailable"))?;
        response.headers_mut().insert(
            "x-fleetworks-next-log-cursor",
            HeaderValue::from_str(&token).map_err(|_| crate::internal("log cursor unavailable"))?,
        );
    }
    Ok(protected(response))
}
pub(crate) async fn follow(
    state: State<ApiState>,
    extension: Option<Extension<ResolvedRequest>>,
    cache: Extension<Arc<LogCursors>>,
    query: Query<LogQuery>,
) -> Result<Response, ApiError> {
    tokio::time::timeout(FOLLOW_LIMIT, follow_inner(state, extension, cache, query))
        .await
        .map_err(|_| {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "Bounded operational log follow is busy; retry this scoped cursor",
            )
        })?
}

async fn follow_inner(
    State(state): State<ApiState>,
    extension: Option<Extension<ResolvedRequest>>,
    Extension(cache): Extension<Arc<LogCursors>>,
    Query(query): Query<LogQuery>,
) -> Result<Response, ApiError> {
    let context = context(extension)?;
    let token = query.cursor.as_deref().ok_or_else(crate::bad)?;
    let record = cache
        .get(token, &binding(&context)?, Kind::Follow)
        .await
        .map_err(|_| changed())?;
    overrides(&query, &record.snapshot)?;
    let wait = Duration::from_millis(query.wait_ms.unwrap_or(1000));
    if wait > FOLLOW_LIMIT - TICK {
        return Err(crate::bad());
    }
    let deadline = Instant::now() + wait;
    loop {
        let (page, shared) = tokio::time::timeout(
            AUTHORITY_CHECK_LIMIT,
            read(
                &state,
                &context,
                &record.snapshot,
                LogDirection::Newer,
                None,
                record.boundary,
            ),
        )
        .await
        .map_err(|_| {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "Bounded log authority check unavailable",
            )
        })??;
        if !page.rows.is_empty() || Instant::now() >= deadline {
            let boundary = page.next_boundary.or(record.boundary);
            let next = cache
                .issue(Record::next(
                    record.snapshot.clone(),
                    Kind::Follow,
                    None,
                    boundary,
                ))
                .await
                .map_err(|_| crate::internal("log follow cursor unavailable"))?;
            return Ok(protected(
                Json(PublicPage {
                    rows: safe_rows(&page),
                    counts: Some(page.counts),
                    apps: page.apps,
                    apps_complete: page.apps_complete,
                    next_cursor: Some(next),
                    follow_cursor: None,
                    available: true,
                    private_logs_only: true,
                    shared_private_logs_available: !shared,
                    reason_code: None,
                    narrowing_hint: None,
                    has_more: page.has_more,
                })
                .into_response(),
            ));
        }
        tokio::time::sleep(TICK.min(deadline.saturating_duration_since(Instant::now()))).await;
    }
}
