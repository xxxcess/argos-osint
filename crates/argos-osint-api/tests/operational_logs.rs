//! Real gateway authority and Jobs SQLite log pagination, export and follow.
#![cfg(feature = "embedded")]

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
use argos_osint_core::storage::jobs_client;
use argos_osint_core::storage::open::Target;
use argos_osint_core::storage::operational_logs::OperationalLogStore;
use argos_osint_core::storage::repo::Repository;
use axum::body::to_bytes;
use axum::body::Body;
use axum::http::HeaderMap;
use axum::http::Request;
use axum::http::StatusCode;
use axum::Router;
use chrono::Utc;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::connect::WorkspaceName;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::JobOrigin;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::operational_logs::LogPage;
use serde_json::json;
use serde_json::Value;
use tower::ServiceExt;

const NATIVE: &str = "fixture-native-logs-token-0123456789abcdef";
const READER: &str = "fixture-reader-logs-token-0123456789abcdef";
const INTERNAL: &str = "fixture-internal-logs-token-0123456789abcdef";

struct OwnerTask(tokio::task::JoinHandle<()>);
impl Drop for OwnerTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct Environment(Vec<(&'static str, Option<OsString>)>);
impl Drop for Environment {
    fn drop(&mut self) {
        for (key, value) in &self.0 {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
    }
}
fn authority() -> RegistryAuthority {
    RegistryAuthority::new("logs-scope", "native", "logs-access")
        .expect("explicit fixture authority")
}
fn ensure_request(node: &str, child: &str, request: &str) -> RegistrationEnsureRequest {
    RegistrationEnsureRequest::new(
        request.into(),
        if node == "a" {
            "192.0.2.42"
        } else {
            "198.51.100.42"
        }
        .parse()
        .expect("synthetic IP"),
        WorkspaceName::new(child).expect("fixture child"),
        format!("fixture-installation-{node}"),
        format!("fixture-folder-{node}-{child}"),
    )
}
async fn context(repo: &Repository, node: &str, child: &str) -> RegisteredContext {
    let receipt = connect_registry::ensure(
        repo,
        &authority(),
        &ensure_request(node, child, &format!("ensure-{node}-{child}")),
    )
    .await
    .expect("actual registration")
    .receipt;
    connect_registry::registration_context(
        repo,
        &authority(),
        &receipt.registration_id,
        receipt.revision,
    )
    .await
    .expect("actual registered context")
}
async fn enqueue(repo: &Repository, context: &RegisteredContext, job: &str, request: &str) {
    let capture = InteractiveAdmission::native(
        authority(),
        context.clone(),
        JobOrigin::new(
            context,
            OriginSource::Native,
            request.into(),
            Utc::now().to_rfc3339(),
            0,
            Vec::new(),
        ),
    )
    .expect("server-derived admission");
    let task = format!("{job}-execute");
    let now = Utc::now();
    job_dispatch::submit_interactive(
        repo.db(),
        &NewJob {
            id: job,
            kind: "task",
            operation: "logs_fixture",
            parent: None,
            created_at: now,
        },
        &NewTask {
            id: &task,
            job,
            pool: "logs_fixture",
            operation: "logs_fixture",
            priority: 100,
            deps: Vec::new(),
            input_hash: "logs-fixture-input",
            created_at: now,
        },
        &capture,
    )
    .await
    .expect("actual durable attributed acceptance");
}
async fn call(
    app: &Router,
    path: &str,
    token: &str,
    context: &RegisteredContext,
) -> (StatusCode, HeaderMap, String) {
    let request = Request::builder()
        .uri(path)
        .header("host", "127.0.0.1:8080")
        .header("authorization", format!("Bearer {token}"))
        // A public caller cannot replace the gateway's authenticated proof.
        .header("x-fleetworks-delegation", "forged-public-proof")
        .header("x-fleetworks-origin", "forged-public-origin")
        .header("x-fleetworks-registration", &context.registration_id)
        .header(
            "x-fleetworks-registration-revision",
            context.registration_revision.to_string(),
        )
        .body(Body::empty())
        .expect("bounded request");
    let response = app.clone().oneshot(request).await.expect("actual gateway");
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 256 * 1024)
        .await
        .expect("bounded response");
    (
        status,
        headers,
        String::from_utf8(body.to_vec()).expect("text response"),
    )
}
fn json_body(body: &str) -> Value {
    serde_json::from_str(body).expect("JSON page")
}
fn validated_log_page(body: &str) -> Value {
    let page: LogPage = serde_json::from_str(body).expect("actual client-safe log DTO");
    page.validate()
        .expect("actual owner/gateway page matches the shared wire contract");
    serde_json::to_value(page).expect("validated public page")
}
fn token(value: &Value, key: &str) -> String {
    let value = value[key].as_str().expect("opaque continuation");
    assert_eq!(value.len(), 64);
    assert!(value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')));
    value.into()
}
fn row_ids(value: &Value) -> BTreeSet<String> {
    value["rows"]
        .as_array()
        .expect("safe rows")
        .iter()
        .map(|row| {
            assert!(row.get("seq").is_none());
            assert!(row.get("details").is_none());
            row["id"].as_str().expect("safe row identity").into()
        })
        .collect()
}

#[tokio::test]
async fn operational_logs_authorize_before_counts_export_follow_and_cursor_revalidation() {
    let temporary = tempfile::tempdir().expect("disposable backend");
    let root = temporary
        .path()
        .canonicalize()
        .expect("canonical temp home");
    let _environment = Environment(
        ["FLEETWORKS_WORKSPACE", "ARGOS_HOME"]
            .into_iter()
            .map(|key| {
                let previous = std::env::var_os(key);
                std::env::set_var(key, &root);
                (key, previous)
            })
            .collect(),
    );
    let repo = Repository::connect(
        &Target::Embedded {
            path: root.join("store"),
        },
        "fixture",
        "operational_logs_api",
    )
    .await
    .expect("actual isolated domain store");
    bootstrap::ensure_network_identity(repo.db())
        .await
        .expect("actual identity");
    connect_registry::initialize_control(repo.db())
        .await
        .expect("registry control");
    job_dispatch::enable_pool(repo.db(), "logs_fixture")
        .await
        .expect("explicit fixture pool");
    let a = context(&repo, "a", "research").await;
    let sibling = context(&repo, "a", "analysis").await;
    let b = context(&repo, "b", "research").await;
    for (ctx, id, request) in [
        (&a, "a-root", "a-root-request"),
        (&sibling, "a-sibling", "a-sibling-request"),
        (&b, "b-root", "b-root-request"),
        (&a, "shared-root", "shared-a"),
        (&b, "shared-root", "shared-b"),
    ] {
        enqueue(&repo, ctx, id, request).await;
    }
    jobs::create_job(
        repo.db(),
        &NewJob {
            id: "a-child",
            kind: "task",
            operation: "logs_child",
            parent: Some("a-root"),
            created_at: Utc::now(),
        },
        &[],
    )
    .await
    .expect("actual causal attribution");
    jobs::create_job(
        repo.db(),
        &NewJob {
            id: "legacy-root",
            kind: "task",
            operation: "legacy",
            parent: None,
            created_at: Utc::now(),
        },
        &[],
    )
    .await
    .expect("unassigned historical job");
    jobs::create_job(
        repo.db(),
        &NewJob {
            id: "system-feed",
            kind: "service",
            operation: "api_server",
            parent: None,
            created_at: Utc::now(),
        },
        &[],
    )
    .await
    .expect("infrastructure job");
    let logs = OperationalLogStore::open(root.join("operational.sqlite3"))
        .await
        .expect("actual Jobs SQLite");
    jobs_client::install_local(logs.clone()).expect("isolated actual log owner");
    for (id, job, message) in [
        ("root-log", "a-root", "allowed root"),
        ("foreign-log", "b-root", "foreign private"),
        ("child-log", "a-child", "allowed causal"),
        ("shared-log", "shared-root", "mixed private"),
        ("system-log", "system-feed", "system private"),
        ("sibling-log", "a-sibling", "allowed sibling"),
        ("legacy-log", "legacy-root", "legacy private"),
    ] {
        logs.log_append(json!({"id":id,"ts":Utc::now().to_rfc3339(),"severity":"info","app":"tools","job_id":job,"message":message,"details":"hidden-details-sentinel"})).await.expect("actual operational row");
    }
    let hosts = vec!["127.0.0.1:8080".into()];
    let credential = |id: &str, token: &str, permissions: BTreeSet<Permission>| {
        ScopedCredential::new(
            id.into(),
            "logs-scope".into(),
            "logs-access".into(),
            CredentialPurpose::Native,
            permissions,
            token,
        )
        .expect("purpose credential")
    };
    let policy = NetworkPolicy::with_credentials(
        &hosts,
        vec![
            credential(
                "native",
                NATIVE,
                BTreeSet::from([
                    Permission::ReadBackend,
                    Permission::RegisterWorkstation,
                    Permission::ReadWorkstationActivity,
                ]),
            ),
            credential(
                "reader",
                READER,
                BTreeSet::from([Permission::ReadBackend, Permission::RegisterWorkstation]),
            ),
        ],
    )
    .expect("actual purpose policy");
    let state = ApiState {
        repo: repo.clone(),
        instance_id: "logs-api".into(),
        port: 8080,
        origins: Arc::new(Vec::new()),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("fixture-owned private listener");
    let address = listener.local_addr().expect("private address");
    let internal_policy = NetworkPolicy::with_credentials(
        &[address.to_string()],
        vec![ScopedCredential::new(
            "gateway-jobs".into(),
            "logs-scope".into(),
            "logs-access".into(),
            CredentialPurpose::InternalService,
            BTreeSet::from([Permission::ReadWorkstationActivity]),
            INTERNAL,
        )
        .expect("independent infrastructure credential")],
    )
    .expect("actual private ingress policy");
    let mut private_state = state.clone();
    private_state.port = address.port();
    let owner = jobs_router(private_state, internal_policy);
    let mut owner_task = OwnerTask(tokio::spawn(async move {
        axum::serve(listener, owner)
            .await
            .expect("private fixture service");
    }));
    let bare_private = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .expect("finite private fixture client")
        .get(format!("http://{address}/api/v1/logs"))
        .bearer_auth(INTERNAL)
        .send()
        .await
        .expect("actual private boundary response");
    assert_eq!(
        bare_private.status(),
        StatusCode::UNAUTHORIZED,
        "infrastructure authentication alone cannot supply workstation scope"
    );
    let app = network_router_with_private_services(
        state,
        policy,
        PrivateServices::new(
            None,
            None,
            None,
            Some(
                JobsClient::new(&format!("http://{address}"), INTERNAL)
                    .expect("fixed authenticated Jobs owner"),
            ),
        ),
    );
    assert_eq!(
        call(&app, "/api/v1/logs", READER, &a).await.0,
        StatusCode::FORBIDDEN
    );
    let (status, headers, body) = call(&app, "/api/v1/logs?limit=1", NATIVE, &a).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(headers["cache-control"], "no-store, private");
    let first = validated_log_page(&body);
    assert_eq!(first["counts"]["total"], 3, "scope before LIMIT");
    assert_eq!(first["apps_complete"], true);
    assert_eq!(first["shared_private_logs_available"], false);
    assert_eq!(row_ids(&first), BTreeSet::from(["sibling-log".into()]));
    assert!(!body.contains("hidden-details-sentinel"));
    let next = token(&first, "next_cursor");
    let follow = token(&first, "follow_cursor");
    assert_eq!(
        call(&app, &format!("/api/v1/logs?cursor={next}"), NATIVE, &b)
            .await
            .0,
        StatusCode::CONFLICT,
        "another workstation cannot consume this proof-bound cursor"
    );
    assert_eq!(
        call(
            &app,
            &format!("/api/v1/logs?cursor={next}&text=changed"),
            NATIVE,
            &a
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let mut ids = row_ids(&first);
    let mut cursor = Some(next);
    for _ in 0..3 {
        let Some(current) = cursor else {
            break;
        };
        let (status, _, body) =
            call(&app, &format!("/api/v1/logs?cursor={current}"), NATIVE, &a).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let page = validated_log_page(&body);
        ids.extend(row_ids(&page));
        cursor = page["next_cursor"].as_str().map(str::to_owned);
    }
    assert!(cursor.is_none(), "finite complete pagination");
    assert_eq!(
        ids,
        BTreeSet::from(["root-log".into(), "child-log".into(), "sibling-log".into()])
    );
    let (_, _, body) = call(
        &app,
        &format!(
            "/api/v1/logs?workspace_id={}&job_id=a-root&descendants=true",
            a.workspace_id
        ),
        NATIVE,
        &a,
    )
    .await;
    let clean = validated_log_page(&body);
    assert_eq!(
        clean["shared_private_logs_available"], true,
        "the clean selected cohort has no mixed-origin private exclusions"
    );
    assert_eq!(
        row_ids(&clean),
        BTreeSet::from(["root-log".into(), "child-log".into()])
    );
    assert_eq!(
        call(&app, "/api/v1/logs?job_id=b-root", NATIVE, &a).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            &format!("/api/v1/logs?workspace_id={}", b.workspace_id),
            NATIVE,
            &a
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (_, _, body) = call(&app, "/api/v1/logs?job_id=shared-root", NATIVE, &a).await;
    assert_eq!(json_body(&body)["available"], false);
    for text in ["foreign", "hidden-details-sentinel"] {
        let (_, _, body) = call(&app, &format!("/api/v1/logs?text={text}"), NATIVE, &a).await;
        let page = json_body(&body);
        assert_eq!(
            page["counts"]["total"], 0,
            "hidden material cannot influence search count"
        );
        assert_eq!(page["apps"], json!([]));
    }
    let (status, headers, body) = call(&app, "/api/v1/logs/export?limit=2", NATIVE, &a).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(headers["content-type"], "application/x-ndjson");
    assert_eq!(headers["x-fleetworks-log-apps-complete"], "true");
    assert!(headers["content-disposition"]
        .to_str()
        .expect("export disposition")
        .contains("workstation-logs.ndjson"));
    let export_next = headers["x-fleetworks-next-log-cursor"]
        .to_str()
        .expect("bounded export continuation");
    assert_eq!(body.lines().count(), 2);
    let (_, _, last) = call(
        &app,
        &format!("/api/v1/logs/export?cursor={export_next}"),
        NATIVE,
        &a,
    )
    .await;
    let exported: Vec<Value> = body.lines().chain(last.lines()).map(json_body).collect();
    assert_eq!(exported.len(), 3);
    for row in &exported {
        assert!(row.get("details").is_none() && row.get("seq").is_none());
        assert!(["a-root", "a-child", "a-sibling"]
            .contains(&row["job_id"].as_str().expect("authorized export job")));
    }
    for (id, job) in [("new-allowed", "a-root"), ("new-foreign", "b-root")] {
        logs.log_append(json!({"id":id,"ts":Utc::now().to_rfc3339(),"severity":"info","app":"tools","job_id":job,"message":"new operational event"})).await.expect("actual follow append");
    }
    let (status, _, body) = call(
        &app,
        &format!("/api/v1/logs/follow?cursor={follow}&wait_ms=0"),
        NATIVE,
        &a,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let page = validated_log_page(&body);
    assert_eq!(row_ids(&page), BTreeSet::from(["new-allowed".into()]));
    let follow_next = token(&page, "next_cursor");
    let (status, _, body) = call(
        &app,
        &format!("/api/v1/logs/follow?cursor={follow_next}&wait_ms=0"),
        NATIVE,
        &a,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        row_ids(&validated_log_page(&body)).is_empty(),
        "foreign seq does not appear or advance public cursor"
    );
    enqueue(
        &repo,
        &a,
        "newly-associated-job",
        "newly-associated-request",
    )
    .await;
    assert_eq!(
        call(
            &app,
            &format!("/api/v1/logs/follow?cursor={follow_next}&wait_ms=0"),
            NATIVE,
            &a
        )
        .await
        .0,
        StatusCode::CONFLICT,
        "a changed complete cohort requires restart instead of skipping a new job"
    );
    let mut changed_binding = ensure_request("a", "research", "confirmed-rebind");
    changed_binding.workstation_ip = "192.0.2.43".parse().expect("synthetic rebind IP");
    connect_registry::rebind(
        &repo,
        &authority(),
        &RebindInput::new(
            changed_binding,
            a.registration_id.clone(),
            a.registration_revision,
        ),
    )
    .await
    .expect("actual authoritative rebind");
    let denied = call(
        &app,
        &format!("/api/v1/logs/follow?cursor={follow_next}&wait_ms=0"),
        NATIVE,
        &a,
    )
    .await
    .0;
    assert_eq!(
        denied,
        StatusCode::UNAUTHORIZED,
        "old registered proof cannot follow after revocation"
    );
    owner_task.0.abort();
    assert!(
        (&mut owner_task.0)
            .await
            .expect_err("aborted fixture service")
            .is_cancelled(),
        "the fixture harvests its owned service task"
    );
}
