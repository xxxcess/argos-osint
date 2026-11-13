//! Versioned local Intel HTTP adapter. @tag:intel-brief
mod activity;
mod activity_actions;
mod activity_paging;
mod actor_dossier;
mod actor_search;
mod admission;
mod brain;
pub mod classifier_activation;
mod configuration;
pub mod configuration_reset;
mod connect;
mod context_boundary;
mod control;
mod cycle_history;
pub mod datasets;
mod host_metrics;
mod jobs_api;
mod live;
mod maintenance;
mod network;
mod operational_logs;
mod operations;
mod private_relay;
mod request_context;
mod request_reconciliation;
#[cfg(all(test, not(feature = "embedded")))]
#[path = "../../argos-osint-core/tests/support/mod.rs"]
mod storage_test_support;
mod tools;
mod topology_summary;
mod topology_web;
pub mod workstation;

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

pub use actor_search::SearchClient;
pub use brain::BrainClient;
pub use control::ControlClient;
pub use jobs_api::JobsClient;
pub use network::AuthenticatedPrincipal;
pub use network::ScopedCredential;
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
    search: Option<SearchClient>,
    readiness: Option<workstation::ReadinessOwners>,
    datasets: Option<datasets::DatasetStore>,
    activation: Option<Arc<classifier_activation::ActivationArtifacts>>,
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
            search: None,
            readiness: None,
            datasets: None,
            activation: None,
        }
    }

    pub fn with_search(mut self, search: SearchClient) -> Self {
        self.search = Some(search);
        self
    }

    /// Bind the actual server-owned Natural artifact store at startup.
    pub fn with_activation_artifacts(
        mut self,
        artifacts: classifier_activation::ActivationArtifacts,
    ) -> Self {
        self.activation = Some(Arc::new(artifacts));
        self
    }

    pub fn with_workstation(
        mut self,
        readiness: workstation::ReadinessOwners,
        datasets: datasets::DatasetStore,
    ) -> Self {
        self.readiness = Some(readiness);
        self.datasets = Some(datasets);
        self
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
        .layer(middleware::from_fn_with_state(
            state.clone(),
            context_boundary::private,
        ))
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
    let brain_routes = if mode == AccessMode::Bearer {
        brain::routes()
    } else {
        brain::operator_routes()
    };
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
            .merge(configuration_reset::routes())
            .merge(classifier_activation::routes())
            .route(
                ATLAS_CYCLES_PATH,
                get(cycle_history::list).post(operations::submit_atlas_cycle),
            )
            .route(
                fleetworks_contracts::cycle_history::CYCLE_HISTORY_DELETE_PATH,
                post(cycle_history::delete),
            )
            .route("/api/v1/workstation/readiness", get(workstation::read))
            .route(
                "/api/v1/workstation/requests/detail",
                get(request_reconciliation::read),
            )
            .route("/api/v1/workstation/datasets", get(datasets::list))
            .route(
                "/api/v1/workstation/datasets/upload",
                post(datasets::upload).layer(DefaultBodyLimit::disable()),
            )
            .route(
                "/api/v1/workstation/maintenance/seed-actors",
                post(maintenance::seed),
            )
            .route(
                "/api/v1/workstation/maintenance/train-classifier",
                post(maintenance::train),
            )
            .route(
                fleetworks_contracts::search::ACTOR_REINDEX_PATH,
                post(maintenance::reindex),
            )
            .route(
                "/api/v1/connect/registrations/ensure",
                post(connect::ensure),
            )
            .route(
                "/api/v1/connect/registrations/rebind",
                post(connect::rebind),
            )
            .route(
                "/api/v1/connect/registrations/retire",
                post(connect::retire),
            )
            .route(
                "/api/v1/connect/registrations/context",
                post(connect::registration_context),
            )
            .route("/api/v1/connect/sessions", post(connect::create_session))
            .route(
                "/api/v1/connect/sessions/resolve",
                post(connect::resolve_session),
            )
            .route(
                "/api/v1/connect/sessions/logout",
                post(connect::logout_session),
            )
            .merge(tools_routes)
            .merge(control_routes)
            .merge(jobs_routes)
    } else {
        router
            .route("/api/v1/tools", get(operations::tools))
            .merge(control::read_routes())
            .merge(jobs_api::read_and_retry_routes())
    };
    let router = if let Some(search) = services.search {
        router.layer(axum::Extension(search))
    } else {
        router
    };
    let router = if let Some(readiness) = services.readiness {
        router.layer(axum::Extension(readiness))
    } else {
        router
    };
    let router = if let Some(activation) = services.activation {
        router.layer(axum::Extension(activation))
    } else {
        router
    };
    let router = if let Some(datasets) = services.datasets {
        router.layer(axum::Extension(datasets))
    } else {
        router
    };
    router
        .layer(axum::Extension(sampler))
        .layer(axum::Extension(feed))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .layer(middleware::from_fn(request_timeout))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            context_boundary::gateway,
        ))
        .layer(axum::Extension(mode))
        .layer(middleware::from_fn_with_state(access, network::protect))
        .with_state(state)
}

async fn request_timeout(request: Request, next: Next) -> Response {
    // A read queued behind a running brief still has to return. 45s covers
    // that wait; the web client waits longer so it receives this JSON timeout
    // instead of aborting with "signal timed out".
    let timeout = if request.uri().path() == "/api/v1/workstation/datasets/upload" {
        Duration::from_secs(600)
    } else {
        Duration::from_secs(45)
    };
    match tokio::time::timeout(timeout, next.run(request)).await {
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
async fn capabilities(
    State(state): State<ApiState>,
    axum::Extension(mode): axum::Extension<AccessMode>,
    principal: Option<axum::Extension<AuthenticatedPrincipal>>,
) -> ApiResult {
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
        use fleetworks_contracts::capabilities;
        let Some(axum::Extension(principal)) = principal else {
            return Err(ApiError(
                StatusCode::UNAUTHORIZED,
                "Purpose-scoped credential required",
            ));
        };
        let deployment = storage::connect_registry::deployment_state(&state.repo)
            .await
            .map_err(internal)?;
        features.extend([
            capabilities::CONNECT_REGISTRY_FEATURE,
            "workstation-readiness",
            "dataset-upload",
            "profile-management",
            "actor-discovery",
        ]);
        for (pool, feature) in [
            (storage::job_dispatch::ATLAS_POOL, "atlas-cycles"),
            (
                argos_osint_core::osint::manual::TOOLS_POOL,
                "tool-execution",
            ),
        ] {
            let mut result = state
                .repo
                .db()
                .query("SELECT VALUE enabled FROM ONLY type::record('job_dispatch_pool',$pool)")
                .bind(("pool", pool.to_owned()))
                .await
                .map_err(internal)?
                .check()
                .map_err(internal)?;
            if result.take::<Option<bool>>(0).map_err(internal)? == Some(true) {
                features.push(feature);
            }
        }
        let mut response = capabilities::Capabilities::default();
        response.categories = intel::CATEGORIES
            .iter()
            .map(|category| (*category).to_owned())
            .collect();
        response.auth = true;
        response.features = features.into_iter().map(str::to_owned).collect();
        response.instance_id = Some(state.instance_id);
        response.deployment_id = Some(deployment.deployment_id);
        response.configuration_epoch = Some(deployment.configuration_epoch);
        response.connect = Some(capabilities::ConnectCapabilities::default());
        response
            .features
            .push("workstation-cycle-history-v1".into());
        response.workspace_control = Some(capabilities::WorkspaceControlCapabilities::default());
        response.resource_scopes = Some(capabilities::ResourceScopes::default());
        response.permissions = principal.permissions().collect();
        response.live = Some(capabilities::LiveCapabilities::new(
            "/api/v1/live".into(),
            vec!["metrics".into(), "brief".into()],
            1,
        ));
        let mut search = capabilities::SearchCapabilities::default();
        search.actors = Some(capabilities::ActorSearchCapabilities::default());
        response.search = Some(search);
        if let Ok(raw) = std::env::var("FLEETWORKS_INTEL_WEB_URL") {
            let mut web =
                url::Url::parse(&raw).map_err(|_| internal("invalid public Intel Web URL"))?;
            let loopback = web.host_str().is_some_and(|host| {
                host == "localhost"
                    || host
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            });
            if web.host().is_none()
                || !(web.scheme() == "https" || web.scheme() == "http" && loopback)
                || !web.username().is_empty()
                || web.password().is_some()
                || web.query().is_some()
                || web.fragment().is_some()
            {
                return Err(internal("unsafe public Intel Web URL"));
            }
            let base = web.to_string();
            let path = format!("{}/connect", web.path().trim_end_matches('/'));
            web.set_path(&path);
            response.links.intel_web =
                Some(capabilities::IntelWebLinks::new(base, web.to_string()));
        }
        return Ok(Json(serde_json::to_value(response).map_err(internal)?));
    }
    Ok(Json(
        json!({"api_version":1,"categories":intel::CATEGORIES,"auth":mode == AccessMode::Bearer,"features":features,"live":{"path":"/api/v1/live","channels":["metrics","brief"],"version":1}}),
    ))
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
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnsureKey {
    run: String,
    id: String,
    request_id: Option<String>,
}
async fn ensure(
    State(s): State<ApiState>,
    context: Option<axum::Extension<request_context::ResolvedRequest>>,
    Json(k): Json<EnsureKey>,
) -> ApiResult {
    let a = load(
        &s,
        &ArticleKey {
            run: k.run,
            id: k.id,
        },
    )
    .await?;
    let job_id = if let Some(axum::Extension(context)) = context {
        let request_id = k.request_id.unwrap_or_else(argos_osint_core::store::new_id);
        let (admission, settings, auth) =
            admission::capture_with_configuration(&context, &request_id).await?;
        if let Some(job) = admission
            .replay_job(&s.repo, "intel_brief")
            .await
            .map_err(internal)?
        {
            job
        } else {
            let inputs =
                storage::brief_stages::inputs_for_article_with_auth(&s.repo, &a, &settings, &auth)
                    .await
                    .map_err(internal)?;
            storage::brief_stages::ensure_interactive(&s.repo, &inputs, &admission)
                .await
                .map_err(internal)?
                .job_id
        }
    } else {
        intel::ensure_brief(&s.repo, &a).await.map_err(internal)?
    };
    Ok(Json(json!({"job_id":job_id})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReloadKey {
    run: String,
    id: String,
    request_id: String,
}
async fn reload(
    State(s): State<ApiState>,
    context: Option<axum::Extension<request_context::ResolvedRequest>>,
    Json(k): Json<ReloadKey>,
) -> ApiResult {
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
    let job_id = if let Some(axum::Extension(context)) = context {
        let (admission, settings, auth) =
            admission::capture_with_configuration(&context, &k.request_id).await?;
        if let Some(job) = admission
            .replay_job(&s.repo, "intel_brief")
            .await
            .map_err(internal)?
        {
            job
        } else {
            let mut inputs =
                storage::brief_stages::inputs_for_article_with_auth(&s.repo, &a, &settings, &auth)
                    .await
                    .map_err(internal)?;
            inputs.input_revision = format!(
                "{}:reload:{}:{}",
                inputs.input_revision,
                admission.context().registration_id,
                k.request_id
            );
            inputs.stage_revisions.clear();
            storage::brief_stages::ensure_interactive(&s.repo, &inputs, &admission)
                .await
                .map_err(internal)?
                .job_id
        }
    } else {
        intel::reload_brief(&s.repo, &a, &k.request_id)
            .await
            .map_err(internal)?
    };
    Ok(Json(json!({"job_id":job_id})))
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
async fn retry(
    State(s): State<ApiState>,
    axum::Extension(mode): axum::Extension<AccessMode>,
    context: Option<axum::Extension<request_context::ResolvedRequest>>,
    Json(k): Json<JobKey>,
) -> ApiResult {
    validate_job(&k)?;
    if context.is_some() {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Durable scoped retry is not enabled",
        ));
    }
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
