//! Versioned local Intel HTTP adapter. @tag:intel-brief
mod actor_dossier;
mod brain;
mod control;
mod host_metrics;
mod jobs_api;
mod live;
mod network;
mod operations;
mod private_relay;
mod tools;
mod topology_summary;
mod topology_web;

use argos_osint_core::{
    intel_service as intel,
    storage::{self, repo::Repository},
};
use axum::{
    extract::{DefaultBodyLimit, Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use fleetworks_contracts::{AtlasCycleReceipt, ATLAS_CYCLES_PATH};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};

pub use brain::BrainClient;
pub use control::ControlClient;
pub use jobs_api::JobsClient;
pub use network::{network_origin, NetworkPolicy};
pub use tools::ToolsClient;

const MAX_REQUEST_BYTES: usize = 16 * 1024;

use network::{AccessMode, AccessPolicy};

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

/// Serves the existing unauthenticated loopback API.
pub fn router(state: ApiState) -> Router {
    router_with_access(state, None, PrivateServices::default())
}

/// Serves public routes and authenticated job controls with explicit ingress restrictions.
pub fn network_router(state: ApiState, policy: NetworkPolicy) -> Router {
    router_with_access(state, Some(policy), PrivateServices::default())
}

/// Serves the public API and sends Brain and Intel HTTP operations to a private service.
pub fn network_router_with_brain(
    state: ApiState,
    policy: NetworkPolicy,
    brain: BrainClient,
) -> Router {
    network_router_with_services(state, policy, Some(brain), None)
}

/// Serve public routes and forward configured service operations privately.
pub fn network_router_with_services(
    state: ApiState,
    policy: NetworkPolicy,
    brain: Option<BrainClient>,
    tools: Option<ToolsClient>,
) -> Router {
    network_router_with_private_services(
        state,
        policy,
        PrivateServices::new(brain, tools, None, None),
    )
}

/// Optional private service clients for the authenticated gateway.
#[derive(Default)]
pub struct PrivateServices {
    brain: Option<BrainClient>,
    tools: Option<ToolsClient>,
    control: Option<ControlClient>,
    jobs: Option<JobsClient>,
}

impl PrivateServices {
    /// Construct a gateway service map. Absent clients use local route handlers.
    pub fn new(
        brain: Option<BrainClient>,
        tools: Option<ToolsClient>,
        control: Option<ControlClient>,
        jobs: Option<JobsClient>,
    ) -> Self {
        Self {
            brain,
            tools,
            control,
            jobs,
        }
    }
}

/// Serve public REST routes and forward configured private service operations.
pub fn network_router_with_private_services(
    state: ApiState,
    policy: NetworkPolicy,
    services: PrivateServices,
) -> Router {
    router_with_access(state, Some(policy), services)
}

/// Serves the private Brain and Intel HTTP operations with storage-backed readiness.
pub fn brain_router(state: ApiState, policy: NetworkPolicy) -> Router {
    private_router(
        state,
        policy,
        brain::routes().route("/api/v1/health/ready", get(brain::ready)),
    )
}

/// Serve private Tools operations with storage-backed readiness.
pub fn tools_router(state: ApiState, policy: NetworkPolicy) -> Router {
    private_router(
        state,
        policy,
        tools::routes().route("/api/v1/health/ready", get(tools::ready)),
    )
}

/// Serve the private workspace Control operations.
pub fn control_router(state: ApiState, policy: NetworkPolicy) -> Router {
    private_router(
        state,
        policy,
        control::routes().route("/api/v1/health/ready", get(control::ready)),
    )
}

/// Serve the private Jobs REST operations.
pub fn jobs_router(state: ApiState, policy: NetworkPolicy) -> Router {
    private_router(
        state,
        policy,
        jobs_api::routes().route("/api/v1/health/ready", get(jobs_api::ready)),
    )
}

fn private_router(state: ApiState, policy: NetworkPolicy, routes: Router<ApiState>) -> Router {
    let port = state.port;
    let router = routes
        .layer(axum::Extension(AccessMode::Bearer))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .layer(middleware::from_fn(request_timeout))
        .with_state(state);
    guard_network(router, policy, Arc::new(Vec::new()), port)
}

/// Apply the same host, origin and bearer policy to a private service router.
pub fn guard_network(
    router: Router,
    policy: NetworkPolicy,
    origins: Arc<Vec<String>>,
    port: u16,
) -> Router {
    router.layer(middleware::from_fn_with_state(
        AccessPolicy {
            port,
            origins,
            network: Some(policy),
        },
        network::protect,
    ))
}

fn router_with_access(
    state: ApiState,
    network: Option<NetworkPolicy>,
    services: PrivateServices,
) -> Router {
    let mode = if network.is_some() {
        AccessMode::Bearer
    } else {
        AccessMode::Loopback
    };
    let access = AccessPolicy {
        port: state.port,
        origins: state.origins.clone(),
        network,
    };
    let sampler = Arc::new(std::sync::Mutex::new(host_metrics::HostSampler::default()));
    let feed = Arc::new(host_metrics::HostFeed::new(sampler.clone()));
    let brain_routes = brain::routes();
    let brain_routes = if let Some(brain) = services.brain {
        brain_routes.route_layer(middleware::from_fn_with_state(brain, brain::forward))
    } else {
        brain_routes
    };
    let router = Router::new()
        .route("/api/v1/health/ready", get(ready))
        .route("/api/v1/capabilities", get(capabilities))
        .route("/api/v1/host/metrics", get(host_metrics::read))
        .route("/api/v1/live", get(live::upgrade))
        .route("/api/v1/days", get(days))
        .route("/api/v1/articles", get(articles))
        .route("/api/v1/articles/search", get(article_search))
        .route("/api/v1/articles/counts", get(article_counts))
        .merge(brain_routes);
    let router = if mode == AccessMode::Bearer {
        let tools_routes = tools::routes();
        let tools_routes = if let Some(tools) = services.tools {
            tools_routes.route_layer(middleware::from_fn_with_state(tools, tools::forward))
        } else {
            tools_routes
        };
        let control_routes = control::routes();
        let control_routes = if let Some(control) = services.control {
            control_routes.route_layer(middleware::from_fn_with_state(control, control::forward))
        } else {
            control_routes
        };
        let jobs_routes = jobs_api::routes();
        let jobs_routes = if let Some(jobs) = services.jobs {
            jobs_routes.route_layer(middleware::from_fn_with_state(jobs, jobs_api::forward))
        } else {
            jobs_routes
        };
        router
            .route(ATLAS_CYCLES_PATH, post(operations::submit_atlas_cycle))
            .merge(tools_routes)
            .merge(control_routes)
            .merge(jobs_routes)
    } else {
        router
            .route("/api/v1/tools", get(operations::tools))
            .merge(control::read_routes())
            .merge(jobs_api::read_and_retry_routes())
    };
    router
        .layer(axum::Extension(sampler))
        .layer(axum::Extension(feed))
        .layer(axum::Extension(mode))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .layer(middleware::from_fn(request_timeout))
        .layer(middleware::from_fn_with_state(access, network::protect))
        .with_state(state)
}

async fn request_timeout(request: Request, next: Next) -> Response {
    // A read queued behind a running brief still has to return. 45s covers
    // that wait; the web client waits longer so it receives this JSON timeout
    // instead of aborting with "signal timed out".
    match tokio::time::timeout(Duration::from_secs(45), next.run(request)).await {
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

async fn ready(State(s): State<ApiState>) -> ApiResult {
    storage_ready(&s.repo).await?;
    Ok(Json(
        json!({"service":"argos-intel","api_version":1,"ready":true,"storage":true,"instance_id":s.instance_id}),
    ))
}

async fn private_ready(state: ApiState, service: &'static str) -> ApiResult {
    storage_ready(&state.repo).await?;
    Ok(Json(json!({
        "service": service, "api_version": 1, "ready": true,
        "storage": true, "instance_id": state.instance_id,
    })))
}

async fn storage_ready(repo: &Repository) -> Result<(), ApiError> {
    // Each probe reaches storage, including after startup. TCP alone is insufficient.
    let mut response = repo
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
    Ok(())
}
async fn capabilities(axum::Extension(mode): axum::Extension<AccessMode>) -> Json<Value> {
    let mut features = vec![
        "bulletin",
        "brief",
        "related",
        "jobs",
        "topology",
        "live",
        "article-search",
        "logs",
        "tools",
        "model-defaults",
        "profile",
    ];
    if mode == AccessMode::Bearer {
        features.extend(["atlas-cycles", "job-cancel", "tool-execution"]);
    }
    Json(
        json!({"api_version":1,"categories":intel::CATEGORIES,"auth":mode == AccessMode::Bearer,"features":features,"live":{"path":"/api/v1/live","channels":["metrics","brief"],"version":1}}),
    )
}
async fn days(State(s): State<ApiState>) -> ApiResult {
    Ok(Json(
        json!({"days":storage::atlas::run_days(s.repo.db()).await.map_err(internal)?,"categories":intel::CATEGORIES}),
    ))
}
#[derive(Deserialize)]
struct DayQuery {
    day: String,
}
async fn article_counts(State(s): State<ApiState>, Query(q): Query<DayQuery>) -> ApiResult {
    Ok(Json(json!({
        "counts": intel::category_counts(&s.repo, &q.day).await.map_err(internal)?
    })))
}
async fn articles(State(s): State<ApiState>, Query(q): Query<intel::ArticleQuery>) -> ApiResult {
    q.validate().map_err(|_| bad())?;
    Ok(Json(
        serde_json::to_value(intel::articles(&s.repo, &q).await.map_err(internal)?)
            .map_err(internal)?,
    ))
}
/// Discovery-compatible article search (`search_articles`): admitted
/// titles/descriptions as separate full-text fields with matched-field
/// attribution, weighted ranks and bounded pages.
async fn article_search(
    State(s): State<ApiState>,
    Query(q): Query<argos_osint_core::storage::article_discovery::ArticleSearchQuery>,
) -> ApiResult {
    q.validate().map_err(|_| bad())?;
    Ok(Json(
        serde_json::to_value(intel::article_search(&s.repo, &q).await.map_err(internal)?)
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
async fn cancel(State(s): State<ApiState>, Json(k): Json<JobKey>) -> ApiResult {
    use argos_osint_core::job_registry::CancelRequest;

    validate_job(&k)?;
    match storage::jobs_view::cancel_job(s.repo.db(), &k.id)
        .await
        .map_err(internal)?
    {
        CancelRequest::Requested => Ok(Json(json!({"done":true}))),
        CancelRequest::Missing => Err(missing()),
        CancelRequest::NotCancellable | CancelRequest::NotRunning => Err(ApiError(
            StatusCode::CONFLICT,
            "Job is not active and cancellable",
        )),
    }
}

async fn retry(
    State(s): State<ApiState>,
    axum::Extension(mode): axum::Extension<AccessMode>,
    Json(k): Json<JobKey>,
) -> ApiResult {
    validate_job(&k)?;
    let requeued = if let Some(cycle) = AtlasCycleReceipt::from_job_id(&k.id) {
        if mode != AccessMode::Bearer {
            return Err(missing());
        }
        usize::from(
            storage::job_dispatch::retry_failed_atlas(
                s.repo.db(),
                &cycle.job_id,
                &cycle.task_id(),
                chrono::Utc::now(),
            )
            .await
            .map_err(internal)?,
        )
    } else {
        storage::jobs_view::retry_failed_tasks(s.repo.db(), &k.id)
            .await
            .map_err(internal)?
    };
    Ok(Json(json!({"requeued":requeued})))
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
