//! Versioned local Intel HTTP adapter. @tag:intel-brief
use argos_osint_core::{
    intel_service as intel,
    storage::{self, repo::Repository},
};
use axum::{
    extract::{DefaultBodyLimit, Query, Request, State},
    http::{HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};

#[derive(Clone)]
pub struct ApiState {
    pub repo: Repository,
    pub instance_id: String,
    pub port: u16,
    pub origins: Arc<Vec<String>>,
}

pub fn loopback_url(raw: &str) -> anyhow::Result<url::Url> {
    let u = url::Url::parse(raw)?;
    anyhow::ensure!(
        u.scheme() == "http"
            && matches!(u.host_str(), Some("127.0.0.1" | "localhost"))
            && u.username().is_empty()
            && u.password().is_none()
            && u.path() == "/"
            && u.query().is_none()
            && u.fragment().is_none(),
        "expected a loopback HTTP origin"
    );
    Ok(u)
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/api/v1/health/ready", get(ready))
        .route("/api/v1/capabilities", get(capabilities))
        .route("/api/v1/days", get(days))
        .route("/api/v1/articles", get(articles))
        .route("/api/v1/articles/detail", get(detail))
        .route("/api/v1/articles/related", get(related))
        .route("/api/v1/brief/ensure", post(ensure))
        .route("/api/v1/brief/reload", post(reload))
        .route("/api/v1/brief/stages", get(stages))
        .route("/api/v1/jobs", get(jobs))
        .route("/api/v1/jobs/detail", get(job))
        .route("/api/v1/jobs/retry", post(retry))
        .route("/api/v1/topology", post(topology))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .layer(middleware::from_fn(request_timeout))
        .layer(middleware::from_fn_with_state(state.clone(), protect))
        .with_state(state)
}

async fn request_timeout(request: Request, next: Next) -> Response {
    match tokio::time::timeout(Duration::from_secs(30), next.run(request)).await {
        Ok(response) => response,
        Err(_) => (
            StatusCode::REQUEST_TIMEOUT,
            Json(json!({"error": "Request timed out"})),
        )
            .into_response(),
    }
}

type ApiResult = Result<Json<Value>, ApiError>;
struct ApiError(StatusCode, &'static str);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error":self.1}))).into_response()
    }
}
fn internal(e: impl std::fmt::Display) -> ApiError {
    eprintln!(
        "API operation failed: {}",
        argos_osint_core::provider_diag::sanitize(&e.to_string())
    );
    ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "Service operation unavailable; inspect API host diagnostics",
    )
}
fn bad() -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, "Invalid request")
}
fn missing() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "Resource not found")
}

async fn protect(State(s): State<ApiState>, request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if host != format!("127.0.0.1:{}", s.port)
        && host != format!("localhost:{}", s.port)
        && !(s.port == 80 && matches!(host, "127.0.0.1" | "localhost"))
    {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "Host refused"})),
        )
            .into_response();
    }
    let origin = request.headers().get("origin").cloned();
    if let Some(o) = &origin {
        if !s
            .origins
            .iter()
            .any(|allowed| o.as_bytes() == allowed.as_bytes())
        {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"error": "Origin refused"})),
            )
                .into_response();
        }
    }
    if request
        .headers()
        .get("sec-fetch-site")
        .is_some_and(|v| v == "cross-site")
        && origin.is_none()
    {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "Origin required"})),
        )
            .into_response();
    }

    let mut response = if request.method() == Method::OPTIONS {
        StatusCode::NO_CONTENT.into_response()
    } else {
        next.run(request).await
    };
    let h = response.headers_mut();
    h.insert("cache-control", HeaderValue::from_static("no-store"));
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("vary", HeaderValue::from_static("Origin"));
    if let Some(o) = origin {
        h.insert("access-control-allow-origin", o);
        h.insert(
            "access-control-allow-methods",
            HeaderValue::from_static("GET, POST, OPTIONS"),
        );
        h.insert(
            "access-control-allow-headers",
            HeaderValue::from_static("content-type"),
        );
    }
    response
}

async fn ready(State(s): State<ApiState>) -> ApiResult {
    // Each probe reaches storage, including after startup. TCP alone is insufficient.
    let mut response = s
        .repo
        .db()
        .query("SELECT VALUE version FROM ONLY storage_meta:schema")
        .await
        .map_err(internal)?
        .check()
        .map_err(internal)?;
    let version: Option<i64> = response.take(0).map_err(internal)?;
    if version != Some(storage::bootstrap::SCHEMA_VERSION) {
        return Err(internal("schema readiness failed"));
    }
    Ok(Json(
        json!({"service":"argos-intel","api_version":1,"ready":true,"storage":true,"instance_id":s.instance_id}),
    ))
}
async fn capabilities() -> Json<Value> {
    Json(
        json!({"api_version":1,"categories":intel::CATEGORIES,"auth":false,"features":["bulletin","brief","related","jobs","topology"]}),
    )
}
async fn days(State(s): State<ApiState>) -> ApiResult {
    Ok(Json(
        json!({"days":storage::atlas::run_days(s.repo.db()).await.map_err(internal)?,"categories":intel::CATEGORIES}),
    ))
}
async fn articles(State(s): State<ApiState>, Query(q): Query<intel::ArticleQuery>) -> ApiResult {
    q.validate().map_err(|_| bad())?;
    Ok(Json(
        serde_json::to_value(intel::articles(&s.repo, &q).await.map_err(internal)?)
            .map_err(internal)?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArticleKey {
    run: String,
    id: String,
}
async fn load(
    s: &ApiState,
    k: &ArticleKey,
) -> Result<argos_osint_core::store::AtlasArticleRow, ApiError> {
    if !intel::valid_id(&k.run) || !intel::valid_id(&k.id) {
        return Err(bad());
    }
    intel::article(&s.repo, &k.run, &k.id)
        .await
        .map_err(internal)?
        .ok_or_else(missing)
}
async fn detail(State(s): State<ApiState>, Query(k): Query<ArticleKey>) -> ApiResult {
    let a = load(&s, &k).await?;
    Ok(Json(intel::brief(&s.repo, &a).await.map_err(internal)?))
}
#[derive(Deserialize)]
struct RelatedKey {
    run: String,
    id: String,
    cursor: Option<String>,
    limit: Option<usize>,
}
async fn related(State(s): State<ApiState>, Query(k): Query<RelatedKey>) -> ApiResult {
    let a = load(
        &s,
        &ArticleKey {
            run: k.run,
            id: k.id,
        },
    )
    .await?;
    let limit = k.limit.unwrap_or(20);
    if !(1..=100).contains(&limit)
        || k.cursor
            .as_ref()
            .is_some_and(|c| c.len() > 20 || c.parse::<usize>().is_err())
    {
        return Err(bad());
    }
    Ok(Json(
        intel::related(&s.repo, &a, k.cursor.as_deref(), limit)
            .await
            .map_err(internal)?,
    ))
}
async fn ensure(State(s): State<ApiState>, Json(k): Json<ArticleKey>) -> ApiResult {
    let a = load(&s, &k).await?;
    Ok(Json(
        json!({"job_id":intel::ensure_brief(&s.repo, &a).await.map_err(internal)?}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReloadKey {
    run: String,
    id: String,
    request_id: String,
}
async fn reload(State(s): State<ApiState>, Json(k): Json<ReloadKey>) -> ApiResult {
    if !(8..=128).contains(&k.request_id.len())
        || !k
            .request_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(bad());
    }
    let a = load(
        &s,
        &ArticleKey {
            run: k.run,
            id: k.id,
        },
    )
    .await?;
    Ok(Json(
        json!({"job_id":intel::reload_brief(&s.repo, &a, &k.request_id).await.map_err(internal)?}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JobKey {
    id: String,
}
fn validate_job(k: &JobKey) -> Result<(), ApiError> {
    if intel::valid_id(&k.id) {
        Ok(())
    } else {
        Err(bad())
    }
}
async fn stages(State(s): State<ApiState>, Query(k): Query<JobKey>) -> ApiResult {
    validate_job(&k)?;
    Ok(Json(
        json!({"stages":safe_stages(&s.repo, &k.id).await?,"outputs":storage::brief_stages::stage_outputs(&s.repo, &k.id).await.map_err(internal)?}),
    ))
}
async fn jobs(State(s): State<ApiState>) -> ApiResult {
    let mut rows = storage::jobs_view::list_jobs(s.repo.db(), &Default::default(), 100)
        .await
        .map_err(internal)?;
    for row in &mut rows {
        row.error_summary = argos_osint_core::provider_diag::sanitize(&row.error_summary);
    }
    Ok(Json(json!({"rows":rows})))
}
async fn job(State(s): State<ApiState>, Query(k): Query<JobKey>) -> ApiResult {
    validate_job(&k)?;
    let mut row = storage::jobs_view::get_job(s.repo.db(), &k.id)
        .await
        .map_err(internal)?
        .ok_or_else(missing)?;
    row.error_summary = argos_osint_core::provider_diag::sanitize(&row.error_summary);
    Ok(Json(
        json!({"job":row,"stages":safe_stages(&s.repo, &k.id).await?}),
    ))
}
async fn retry(State(s): State<ApiState>, Json(k): Json<JobKey>) -> ApiResult {
    validate_job(&k)?;
    Ok(Json(
        json!({"requeued":storage::jobs_view::retry_failed_tasks(s.repo.db(), &k.id).await.map_err(internal)?}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TopologyRequest {
    seed: argos_osint_core::brain_topology::TopologySeed,
}
async fn topology(State(s): State<ApiState>, Json(k): Json<TopologyRequest>) -> ApiResult {
    if serde_json::to_string(&k.seed).map_err(|_| bad())?.len() > 8192 {
        return Err(bad());
    }
    Ok(Json(
        serde_json::to_value(
            storage::topology::topology_snapshot(&s.repo, &k.seed, &Default::default())
                .await
                .map_err(internal)?,
        )
        .map_err(internal)?,
    ))
}

async fn safe_stages(
    repo: &Repository,
    id: &str,
) -> Result<Vec<argos_osint_core::intel_recon::brief_pipeline::StageView>, ApiError> {
    let mut rows = storage::brief_stages::stage_views(repo, id)
        .await
        .map_err(internal)?;
    for row in &mut rows {
        row.row = argos_osint_core::provider_diag::sanitize(&row.row);
        row.detail = argos_osint_core::provider_diag::sanitize(&row.detail);
    }
    Ok(rows)
}
