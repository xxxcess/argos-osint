//! Public Native gateway to a real private Jobs owner and scoped creation pages.
#![cfg(any(feature = "embedded", feature = "server"))]

#[cfg(not(feature = "embedded"))]
#[path = "../../argos-osint-core/tests/support/mod.rs"]
mod support;

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::sync::Arc;

use argos_osint_api::jobs_router;
use argos_osint_api::network_router_with_private_services;
use argos_osint_api::ApiState;
use argos_osint_api::JobsClient;
use argos_osint_api::NetworkPolicy;
use argos_osint_api::PrivateServices;
use argos_osint_api::ScopedCredential;
use argos_osint_core::storage::bootstrap;
use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RebindInput;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::job_context::InteractiveAdmission;
use argos_osint_core::storage::job_dispatch;
use argos_osint_core::storage::jobs;
use argos_osint_core::storage::jobs::NewJob;
use argos_osint_core::storage::jobs::NewTask;
use argos_osint_core::storage::jobs_view;
use argos_osint_core::storage::jobs_view::JobMeta;
#[cfg(feature = "embedded")]
use argos_osint_core::storage::open::Target;
use argos_osint_core::storage::repo::Repository;
use axum::body::to_bytes;
use axum::body::Body;
use axum::http::HeaderMap;
use axum::http::Request;
use axum::http::StatusCode;
use axum::Router;
use chrono::DateTime;
use chrono::Duration;
use chrono::Utc;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::JobOrigin;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::context::RegisteredContext;
use serde_json::Value;
#[cfg(not(feature = "embedded"))]
use support::Server;
use tower::ServiceExt;

const SCOPE: &str = "paging-api-scope";
const POOL: &str = "paging_api_fixture";
const NATIVE: &str = "synthetic-paging-native-token-0123456789abcdef";
const PEER: &str = "synthetic-paging-peer-token-0123456789abcdef";
const READER: &str = "synthetic-paging-reader-token-0123456789abcdef";
const INTERNAL: &str = "synthetic-paging-internal-token-0123456789abcdef";
static ENVIRONMENT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Environment(Vec<(&'static str, Option<OsString>)>);
impl Environment {
    fn install(root: &std::path::Path) -> Self {
        Self(
            ["ARGOS_HOME", "FLEETWORKS_WORKSPACE"]
                .into_iter()
                .map(|key| {
                    let previous = std::env::var_os(key);
                    std::env::set_var(key, root);
                    (key, previous)
                })
                .collect(),
        )
    }
}
impl Drop for Environment {
    fn drop(&mut self) {
        for (key, previous) in &self.0 {
            match previous {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

struct OwnerTask(Option<tokio::task::JoinHandle<()>>);
impl OwnerTask {
    async fn shutdown(&mut self) {
        let handle = self.0.take().expect("one owned private server");
        handle.abort();
        assert!(handle
            .await
            .expect_err("aborted fixture server")
            .is_cancelled());
    }
}
impl Drop for OwnerTask {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
        }
    }
}

fn authority() -> RegistryAuthority {
    RegistryAuthority::new(SCOPE, "native", "paging-api-access")
        .expect("explicit fixture registry authority")
}

fn ensure_request(node: &str, key: &str) -> RegistrationEnsureRequest {
    RegistrationEnsureRequest::new(
        key.into(),
        Some(
            if node == "a" {
                "192.0.2.72"
            } else {
                "198.51.100.72"
            }
            .parse()
            .expect("synthetic IP"),
        ),
        format!("paging-api-install-{node}"),
    )
}

async fn register(repo: &Repository, node: &str, registration_attempt: &str) -> RegisteredContext {
    let receipt = connect_registry::ensure(
        repo,
        &authority(),
        &ensure_request(node, &format!("ensure-{node}-{registration_attempt}")),
    )
    .await
    .expect("actual registered native identity")
    .receipt;
    connect_registry::registration_context(
        repo,
        &authority(),
        &receipt.registration_id,
        receipt.revision,
    )
    .await
    .expect("actual current native proof")
}

async fn enqueue(
    repo: &Repository,
    context: &RegisteredContext,
    id: &str,
    key: &str,
    created_at: DateTime<Utc>,
) {
    let admission = InteractiveAdmission::native(
        authority(),
        context.clone(),
        JobOrigin::new(
            context,
            OriginSource::Native,
            key.into(),
            Utc::now().to_rfc3339(),
            0,
            Vec::new(),
        ),
    )
    .expect("server-derived actual admission");
    let task = format!("{id}:execute");
    job_dispatch::submit_interactive(
        repo.db(),
        &NewJob {
            id,
            parent: None,
            kind: "task",
            operation: POOL,
            created_at,
        },
        &NewTask {
            id: &task,
            job: id,
            pool: POOL,
            operation: POOL,
            input_hash: "paging-api-input",
            priority: 0,
            deps: Vec::new(),
            created_at,
        },
        &admission,
    )
    .await
    .expect("actual durable attributed Job/task/outbox transaction");
}

async fn metadata(repo: &Repository, id: &str, app: &str, correlation: &str) {
    jobs_view::set_job_meta(
        repo.db(),
        id,
        &JobMeta {
            app: Some(app.into()),
            correlation_id: Some(correlation.into()),
            title: Some(format!("private-title-{id}")),
            error_summary: Some(format!("private-error-{id}")),
            provider: Some("private-provider".into()),
            result_ref: Some("private-result".into()),
            ..Default::default()
        },
    )
    .await
    .expect("actual safe dashboard writer");
}

struct Fixture {
    app: Router,
    repo: Repository,
    first: RegisteredContext,
    foreign: RegisteredContext,
    owner: OwnerTask,
    private_origin: String,
    _environment: Environment,
    _home: tempfile::TempDir,
    #[cfg(not(feature = "embedded"))]
    _server: Server,
}

impl Fixture {
    async fn new(database: &str) -> Self {
        let home = tempfile::tempdir().expect("disposable backend config root");
        let root = home
            .path()
            .canonicalize()
            .expect("canonical disposable root");
        let environment = Environment::install(&root);
        #[cfg(not(feature = "embedded"))]
        let server = Server::start().expect(
            "paging API fixture requires actual configured pinned SurrealDB; no silent skip",
        );
        #[cfg(not(feature = "embedded"))]
        let repo = server.repo("paging_api", database).await;
        #[cfg(feature = "embedded")]
        let repo = Repository::connect(
            &Target::Embedded {
                path: root.join("store"),
            },
            "paging_api",
            database,
        )
        .await
        .expect("actual isolated embedded store");
        bootstrap::ensure_network_identity(repo.db())
            .await
            .expect("actual deployment identity");
        connect_registry::initialize_control(repo.db())
            .await
            .expect("actual registry control");
        job_dispatch::ensure_queue_transport(repo.db(), POOL)
            .await
            .expect("actual queue policy");
        job_dispatch::enable_pool(repo.db(), POOL)
            .await
            .expect("explicit fixture consumer");
        let first = register(&repo, "a", "first").await;
        let sibling = register(&repo, "a", "repeat").await;
        let foreign = register(&repo, "b", "other").await;
        assert_eq!(first.workstation_id, sibling.workstation_id);
        assert_eq!(first.registration_id, sibling.registration_id);
        assert_ne!(first.workstation_id, foreign.workstation_id);
        let created = Utc::now() - Duration::minutes(1);
        for (id, context, offset, app, correlation) in [
            ("owned-a", &first, 3, "tools", "case-23"),
            ("owned-b", &first, 2, "tools", "case-23"),
            ("sibling", &sibling, 1, "atlas", "sibling"),
            ("foreign", &foreign, 5, "foreign-only", "case-23"),
            ("shared", &first, 4, "tools", "case-23"),
        ] {
            enqueue(&repo, context, id, id, created + Duration::seconds(offset)).await;
            metadata(&repo, id, app, correlation).await;
        }
        enqueue(
            &repo,
            &foreign,
            "shared",
            "shared-foreign",
            created + Duration::seconds(4),
        )
        .await;
        jobs::create_job(
            repo.db(),
            &NewJob {
                id: "causal-child",
                parent: Some("owned-a"),
                kind: "task",
                operation: POOL,
                created_at: created,
            },
            &[],
        )
        .await
        .expect("real causal child origin");
        metadata(&repo, "causal-child", "tools", "child").await;
        let credential = |id: &str, token: &str, activity: bool| {
            let mut permissions =
                BTreeSet::from([Permission::ReadBackend, Permission::RegisterWorkstation]);
            if activity {
                permissions.insert(Permission::ReadWorkstationActivity);
            }
            ScopedCredential::new(
                id.into(),
                SCOPE.into(),
                "paging-api-access".into(),
                CredentialPurpose::Native,
                permissions,
                token,
            )
            .expect("actual independently provisioned purpose credential")
        };
        let policy = NetworkPolicy::with_credentials(
            &["127.0.0.1:8080".into()],
            vec![
                credential("native", NATIVE, true),
                credential("peer", PEER, true),
                credential("reader", READER, false),
            ],
        )
        .expect("actual gateway ingress policy");
        let state = ApiState {
            repo: repo.clone(),
            instance_id: "paging-api".into(),
            port: 8080,
            origins: Arc::new(Vec::new()),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("owned private Jobs endpoint");
        let address = listener.local_addr().expect("actual private address");
        let internal_policy = NetworkPolicy::with_credentials(
            &[address.to_string()],
            vec![ScopedCredential::new(
                "gateway-jobs".into(),
                SCOPE.into(),
                "paging-api-access".into(),
                CredentialPurpose::InternalService,
                BTreeSet::from([Permission::ReadWorkstationActivity]),
                INTERNAL,
            )
            .expect("distinct internal service purpose")],
        )
        .expect("actual private boundary policy");
        let mut private_state = state.clone();
        private_state.port = address.port();
        let owner_router = jobs_router(private_state, internal_policy);
        let owner = OwnerTask(Some(tokio::spawn(async move {
            axum::serve(listener, owner_router)
                .await
                .expect("private fixture server");
        })));
        let private_origin = format!("http://{address}");
        let app = network_router_with_private_services(
            state,
            policy,
            PrivateServices::new(
                None,
                None,
                None,
                Some(
                    JobsClient::new(&private_origin, INTERNAL).expect("actual private Jobs client"),
                ),
            ),
        );
        Self {
            app,
            repo,
            first,
            foreign,
            owner,
            private_origin,
            _environment: environment,
            _home: home,
            #[cfg(not(feature = "embedded"))]
            _server: server,
        }
    }
}

async fn call(
    app: &Router,
    path: &str,
    token: Option<&str>,
    context: Option<&RegisteredContext>,
) -> (StatusCode, HeaderMap, Value) {
    let mut request = Request::builder()
        .uri(path)
        .header("host", "127.0.0.1:8080")
        .header("x-fleetworks-delegation", "forged-public-proof")
        .header("x-fleetworks-origin", "forged-public-origin");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    if let Some(context) = context {
        request = request
            .header("x-fleetworks-registration", &context.registration_id)
            .header(
                "x-fleetworks-registration-revision",
                context.registration_revision.to_string(),
            );
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).expect("finite public request"))
        .await
        .expect("actual gateway and private owner response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 256 * 1024)
        .await
        .expect("bounded public page");
    let json = serde_json::from_slice(&body)
        .unwrap_or_else(|_| serde_json::json!({"non_json":String::from_utf8_lossy(&body)}));
    (status, headers, json)
}

fn ids(page: &Value) -> Vec<String> {
    page["rows"]
        .as_array()
        .expect("activity rows")
        .iter()
        .map(|row| row["id"].as_str().expect("flat safe job row ID").to_owned())
        .collect()
}

fn cursor(page: &Value, field: &str) -> String {
    let token = page[field].as_str().expect("opaque creation-page cursor");
    assert_eq!(token.len(), 64);
    assert!(token
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    token.into()
}

#[tokio::test]
async fn jobs_filters_page_only_current_registered_private_cohort_before_totals_and_facets() {
    let _serial = ENVIRONMENT.lock().await;
    let mut fixture = Fixture::new("paging_filters").await;
    let path = "/api/v1/jobs?sort=newest&limit=1&status=active&app=tools&operation=paging_api_fixture&kind=task&correlation_id=case-23";
    let (status, headers, first) =
        call(&fixture.app, path, Some(NATIVE), Some(&fixture.first)).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(headers["cache-control"], "no-store, private");
    assert_eq!(ids(&first), ["owned-a"]);
    assert_eq!(first["matched_total"], 2);
    assert_eq!(first["apps"], serde_json::json!(["tools"]));
    assert_eq!(first["apps_complete"], true);
    assert_eq!(first["has_more"], true);
    assert_eq!(
        first["dashboard_counts_scope"],
        "unfiltered_top_level_workstation_activity"
    );
    assert_eq!(
        first["counts"]["queued"], 4,
        "dashboard counts retain their separately labelled root cohort"
    );
    let next = cursor(&first, "next_cursor");
    let current = cursor(&first, "page_cursor");
    DateTime::parse_from_rfc3339(
        first["observed_at"]
            .as_str()
            .expect("live page observation"),
    )
    .expect("actual observation timestamp");
    let (status, _, second) = call(
        &fixture.app,
        &format!("/api/v1/jobs?sort=newest&cursor={next}"),
        Some(NATIVE),
        Some(&fixture.first),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(ids(&second), ["owned-b"]);
    assert_eq!(second["matched_total"], 2);
    DateTime::parse_from_rfc3339(
        second["observed_at"]
            .as_str()
            .expect("live continuation observation"),
    )
    .expect("actual continuation timestamp");
    assert_eq!(
        second["visibility"],
        "current_membership_and_status_with_fixed_creation_window"
    );
    assert_eq!(second["has_more"], false);
    assert!(second["next_cursor"].is_null());
    let (status, _, previous) = call(
        &fixture.app,
        &format!("/api/v1/jobs?sort=newest&cursor={current}"),
        Some(NATIVE),
        Some(&fixture.first),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{previous}");
    assert_eq!(
        ids(&previous),
        ["owned-a"],
        "page_cursor supports a previous page without changing its window"
    );
    enqueue(
        &fixture.repo,
        &fixture.foreign,
        "owned-b",
        "owned-b-shared-after-snapshot",
        Utc::now() - Duration::minutes(1),
    )
    .await;
    let (status, _, changed_private) = call(
        &fixture.app,
        &format!("/api/v1/jobs?sort=newest&cursor={next}"),
        Some(NATIVE),
        Some(&fixture.first),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{changed_private}");
    assert!(
        ids(&changed_private).is_empty(),
        "current mixed-origin privacy is rechecked before a cursor can reveal correlation matches"
    );
    assert_eq!(
        changed_private["matched_total"], 1,
        "the fixed creation window still observes live membership/private predicates"
    );
    let (status, _, all) = call(
        &fixture.app,
        "/api/v1/jobs?sort=newest&limit=20",
        Some(NATIVE),
        Some(&fixture.first),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{all}");
    assert!(ids(&all).contains(&"causal-child".to_owned()));
    let shared = all["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|row| row["id"] == "shared")
        .expect("genuine shared status member");
    assert_eq!(shared["status_only"], true);
    assert_eq!(shared["private_details_available"], false);
    let safe = shared.to_string();
    for private in [
        "private-title",
        "private-error",
        "private-provider",
        "private-result",
        "case-23",
    ] {
        assert!(!safe.contains(private), "shared status leaked {private}");
    }
    assert!(!all.to_string().contains("private-title-foreign"));
    fixture.owner.shutdown().await;
}

#[tokio::test]
async fn jobs_paging_rejects_bad_filters_proof_cursor_changes_and_real_rebind_revocation() {
    let _serial = ENVIRONMENT.lock().await;
    let mut fixture = Fixture::new("paging_proof_revalidation").await;
    let first_context = &fixture.first;
    assert_eq!(
        call(
            &fixture.app,
            "/api/v1/jobs?sort=newest",
            None,
            Some(first_context)
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&fixture.app, "/api/v1/jobs?sort=newest", Some(NATIVE), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &fixture.app,
            "/api/v1/jobs?sort=newest",
            Some(READER),
            Some(first_context)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let direct = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .expect("bounded private probe")
        .get(format!(
            "{}/api/v1/jobs?sort=newest",
            fixture.private_origin
        ))
        .bearer_auth(INTERNAL)
        .send()
        .await
        .expect("actual private boundary");
    assert_eq!(
        direct.status(),
        StatusCode::UNAUTHORIZED,
        "infrastructure bearer alone cannot supply current workstation proof"
    );
    for query in [
        "sort=updated",
        "sort=newest&limit=0",
        "sort=newest&limit=201",
        "sort=newest&status=unknown",
        "sort=newest&since=yesterday",
        "sort=newest&kind=%00",
        "sort=newest&workstation_id=guessed-node",
    ] {
        let (status, _, body) = call(
            &fixture.app,
            &format!("/api/v1/jobs?{query}"),
            Some(NATIVE),
            Some(first_context),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}: {body}");
    }
    let (status, _, page) = call(
        &fixture.app,
        "/api/v1/jobs?sort=newest&limit=1",
        Some(NATIVE),
        Some(first_context),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let next = cursor(&page, "next_cursor");
    for suffix in [
        "&app=changed",
        "&kind=task",
        "&correlation_id=changed",
        "&limit=2",
        "&workspace_id=changed",
    ] {
        let (status, _, body) = call(
            &fixture.app,
            &format!("/api/v1/jobs?sort=newest&cursor={next}{suffix}"),
            Some(NATIVE),
            Some(first_context),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "changed frozen query {suffix}: {body}"
        );
    }
    let unknown = "0".repeat(64);
    assert_eq!(
        call(
            &fixture.app,
            &format!("/api/v1/jobs?sort=newest&cursor={unknown}"),
            Some(NATIVE),
            Some(first_context)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            &fixture.app,
            "/api/v1/jobs?sort=newest&cursor=not-a-token",
            Some(NATIVE),
            Some(first_context)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            &fixture.app,
            &format!("/api/v1/jobs?sort=newest&cursor={next}"),
            Some(NATIVE),
            Some(&fixture.foreign)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            &fixture.app,
            &format!("/api/v1/jobs?sort=newest&cursor={next}"),
            Some(PEER),
            Some(first_context)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (status, _, body) = call(
        &fixture.app,
        &format!("/api/v1/jobs?sort=newest&workspace_id={}", "foreign-child"),
        Some(NATIVE),
        Some(first_context),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "legacy workspace query fields fail explicitly"
    );
    assert!(
        body.get("rows").is_none(),
        "guessed child must never produce a foreign page"
    );
    let mut request = ensure_request("a", "confirmed-rebind");
    request.workstation_ip = Some("192.0.2.73".parse().expect("synthetic new interface IP"));
    let changed = connect_registry::rebind(
        &fixture.repo,
        &authority(),
        &RebindInput::new(
            request,
            first_context.registration_id.clone(),
            first_context.registration_revision,
        ),
    )
    .await
    .expect("actual authoritative workstation rebind")
    .receipt;
    let stale = call(
        &fixture.app,
        &format!("/api/v1/jobs?sort=newest&cursor={next}"),
        Some(NATIVE),
        Some(first_context),
    )
    .await;
    assert_eq!(
        stale.0,
        StatusCode::CONFLICT,
        "IP metadata changes invalidate the cursor snapshot without revoking identity"
    );
    let current = connect_registry::registration_context(
        &fixture.repo,
        &authority(),
        &changed.registration_id,
        changed.revision,
    )
    .await
    .expect("fresh actual post-rebind context");
    assert_eq!(
        call(
            &fixture.app,
            &format!("/api/v1/jobs?sort=newest&cursor={next}"),
            Some(NATIVE),
            Some(&current)
        )
        .await
        .0,
        StatusCode::CONFLICT,
        "new proof cannot revive an earlier cursor binding"
    );
    assert_eq!(
        call(
            &fixture.app,
            "/api/v1/jobs?sort=newest&limit=20",
            Some(NATIVE),
            Some(&current)
        )
        .await
        .0,
        StatusCode::OK
    );
    fixture.owner.shutdown().await;
}
