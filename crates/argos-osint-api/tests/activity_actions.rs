//! Real scoped gateway permissions and durable action acknowledgement.
#![cfg(feature = "embedded")]

use std::ffi::OsString;
use std::sync::Arc;

use argos_osint_api::network_router;
use argos_osint_api::ApiState;
use argos_osint_api::NetworkPolicy;
use argos_osint_api::ScopedCredential;
use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::job_actions;
use argos_osint_core::storage::job_context::InteractiveAdmission;
use argos_osint_core::storage::job_dispatch;
use argos_osint_core::storage::jobs::NewJob;
use argos_osint_core::storage::jobs::NewTask;
use argos_osint_core::storage::open::Target;
use argos_osint_core::storage::repo::Repository;
use axum::body::to_bytes;
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::Router;
use chrono::Utc;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::connect::WorkspaceName;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::JobOrigin;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::JOB_CANCEL_PATH;
use serde_json::json;
use serde_json::Value;
use tower::ServiceExt;

const ACTOR: &str = "fixture-scoped-job-action-token-0123456789abcdef";
const READER: &str = "fixture-scoped-job-reader-token-0123456789abcdef";

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
    RegistryAuthority::new("action-scope", "action-native", "action-access").expect("fixture scope")
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    registration: Option<&RegistrationReceipt>,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:8080")
        .header("authorization", format!("Bearer {token}"));
    if let Some(registration) = registration {
        request = request
            .header("x-fleetworks-registration", &registration.registration_id)
            .header(
                "x-fleetworks-registration-revision",
                registration.revision.to_string(),
            );
    }
    if method == "POST" {
        request = request.header("content-type", "application/json");
    }
    let response = app
        .clone()
        .oneshot(
            request
                .body(if method == "POST" {
                    Body::from(body.to_string())
                } else {
                    Body::empty()
                })
                .expect("fixture request"),
        )
        .await
        .expect("gateway response");
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("bounded response");
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

#[tokio::test]
async fn activity_permission_does_not_grant_cancel_and_a_supported_action_has_one_durable_receipt()
{
    Box::pin(scenario()).await;
}

async fn scenario() {
    let directory = tempfile::tempdir().expect("fixture home");
    let home = directory.path().join("home");
    std::fs::create_dir(&home).expect("fixture config directory");
    let canonical = home.canonicalize().expect("canonical fixture home");
    let _environment = Environment(
        ["ARGOS_HOME", "FLEETWORKS_WORKSPACE"]
            .into_iter()
            .map(|key| (key, std::env::var_os(key)))
            .collect(),
    );
    for key in ["ARGOS_HOME", "FLEETWORKS_WORKSPACE"] {
        std::env::set_var(key, &canonical);
    }
    let repo = Repository::connect(
        &Target::Embedded {
            path: directory.path().join("db"),
        },
        "argos",
        "scoped_action_api",
    )
    .await
    .expect("fixture repository");
    repo.db()
        .query("UPSERT storage_meta:identity SET instance_id='action-api-deployment' RETURN NONE")
        .await
        .expect("fixture identity")
        .check()
        .expect("identity statement");
    connect_registry::initialize_control(repo.db())
        .await
        .expect("explicit fixture registry");
    repo.db()
        .query(job_actions::SCHEMA)
        .await
        .expect("fixture action schema")
        .check()
        .expect("action schema statements");
    job_dispatch::enable_pool(repo.db(), job_dispatch::ATLAS_POOL)
        .await
        .expect("fixture consumer");
    let registration = connect_registry::ensure(
        &repo,
        &authority(),
        &RegistrationEnsureRequest::new(
            "action-registration".into(),
            "192.0.2.42".parse().expect("synthetic IP"),
            WorkspaceName::new("research").expect("fixture name"),
            "action-installation-marker".into(),
            "action-workspace-marker".into(),
        ),
    )
    .await
    .expect("real registration")
    .receipt;
    let context = connect_registry::registration_context(
        &repo,
        &authority(),
        &registration.registration_id,
        registration.revision,
    )
    .await
    .expect("current proof context");
    let origin = JobOrigin::new(
        &context,
        OriginSource::Native,
        "submit-action-fixture".into(),
        Utc::now().to_rfc3339(),
        0,
        Vec::new(),
    );
    let admission =
        InteractiveAdmission::native(authority(), context, origin).expect("server admission");
    job_dispatch::submit_interactive(
        repo.db(),
        &NewJob {
            id: "action-api-job",
            kind: "task",
            operation: job_dispatch::ATLAS_POOL,
            parent: None,
            created_at: Utc::now(),
        },
        &NewTask {
            id: "action-api-job:execute",
            job: "action-api-job",
            pool: job_dispatch::ATLAS_POOL,
            operation: job_dispatch::ATLAS_POOL,
            priority: 100,
            deps: Vec::new(),
            input_hash: "fixture-input",
            created_at: Utc::now(),
        },
        &admission,
    )
    .await
    .expect("durable attributed fixture work");
    let actor = ScopedCredential::new(
        "action-native".into(),
        "action-scope".into(),
        "action-access".into(),
        CredentialPurpose::Native,
        [
            Permission::ReadWorkstationActivity,
            Permission::CancelJob,
            Permission::RetryJob,
        ]
        .into_iter()
        .collect(),
        ACTOR,
    )
    .expect("action credential");
    let reader = ScopedCredential::new(
        "action-reader".into(),
        "action-scope".into(),
        "action-access".into(),
        CredentialPurpose::Native,
        [Permission::ReadWorkstationActivity].into_iter().collect(),
        READER,
    )
    .expect("read credential");
    let app = network_router(
        ApiState {
            repo: repo.clone(),
            instance_id: "action-fixture".into(),
            port: 8080,
            origins: Arc::new(Vec::new()),
        },
        NetworkPolicy::with_credentials(&["127.0.0.1:8080".into()], vec![actor, reader])
            .expect("scoped policy"),
    );
    let input = json!({"id":"action-api-job","request_id":"cancel-action-once"});
    assert_eq!(
        call(
            &app,
            "POST",
            JOB_CANCEL_PATH,
            READER,
            Some(&registration),
            input.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&app, "POST", JOB_CANCEL_PATH, ACTOR, None, input.clone())
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (_, reader_view) = call(
        &app,
        "GET",
        "/api/v1/jobs",
        READER,
        Some(&registration),
        Value::Null,
    )
    .await;
    assert_eq!(reader_view["rows"][0]["can_cancel"], false);
    let (_, actor_view) = call(
        &app,
        "GET",
        "/api/v1/jobs",
        ACTOR,
        Some(&registration),
        Value::Null,
    )
    .await;
    assert_eq!(actor_view["rows"][0]["can_cancel"], true);
    let (status, receipt) = call(
        &app,
        "POST",
        JOB_CANCEL_PATH,
        ACTOR,
        Some(&registration),
        input.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    assert_eq!(receipt["receipt"]["state"], "cancelled");
    assert_eq!(
        call(
            &app,
            "POST",
            JOB_CANCEL_PATH,
            ACTOR,
            Some(&registration),
            input
        )
        .await,
        (StatusCode::OK, receipt)
    );
    let mut response=repo.db().query("SELECT VALUE count() FROM job_action WHERE job_id='action-api-job' GROUP ALL; RETURN type::record('task','action-api-job:execute').state;")
        .await.expect("committed action observation").check().expect("observation statements");
    assert_eq!(
        response.take::<Vec<i64>>(0).expect("one action receipt"),
        vec![1]
    );
    assert_eq!(
        response
            .take::<Option<String>>(1)
            .expect("task state")
            .as_deref(),
        Some("cancelled")
    );
}
