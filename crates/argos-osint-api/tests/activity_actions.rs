//! Real scoped gateway permissions and durable action acknowledgement.
#![cfg(feature = "embedded")]

use std::ffi::OsString;
use std::sync::Arc;

use argos_osint_api::network_router;
use argos_osint_api::ApiState;
use argos_osint_api::NetworkPolicy;
use argos_osint_api::ScopedCredential;
use argos_osint_core::storage::actor_search_outbox;
use argos_osint_core::storage::actor_search_outbox::SourceCause;
use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::job_actions;
use argos_osint_core::storage::job_context::InteractiveAdmission;
use argos_osint_core::storage::job_dispatch;
use argos_osint_core::storage::job_dispatch::DeliveryClaim;
use argos_osint_core::storage::jobs::NewJob;
use argos_osint_core::storage::jobs::NewTask;
use argos_osint_core::storage::jobs::MAX_WORKER_RECOVERIES;
use argos_osint_core::storage::open::Target;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::setup_jobs;
use argos_osint_core::storage::tx;
use argos_osint_core::storage::tx::RetryPolicy;
use axum::body::to_bytes;
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::Router;
use chrono::Duration;
use chrono::Utc;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::JobOrigin;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::JOB_CANCEL_PATH;
use fleetworks_contracts::JOB_DETAIL_PATH;
use fleetworks_contracts::JOB_RETRY_PATH;
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
            Some("192.0.2.42".parse().expect("synthetic IP")),
            "action-installation-marker".into(),
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
    let admission = InteractiveAdmission::native(authority(), context.clone(), origin)
        .expect("server admission");
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
        ApiState::new(
            repo.clone(),
            "action-fixture".into(),
            8080,
            Arc::new(Vec::new()),
        ),
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

    // Failed actor_seed job also supports retry behind RetryJob permission & proof.
    repo.db()
        .query(setup_jobs::SCHEMA)
        .await
        .expect("setup schema")
        .check()
        .expect("setup schema statements");
    job_dispatch::enable_pool(repo.db(), "actor_seed")
        .await
        .expect("enable actor_seed pool");
    setup_jobs::enable_operation(repo.db(), MaintenanceOperation::SeedActors, true)
        .await
        .expect("enable actor_seed setup");
    let retry_origin = JobOrigin::new(
        &context,
        OriginSource::Native,
        "submit-retry-fixture".into(),
        Utc::now().to_rfc3339(),
        0,
        Vec::new(),
    );
    let retry_admission = InteractiveAdmission::native(authority(), context.clone(), retry_origin)
        .expect("retry admission");
    job_dispatch::submit_interactive(
        repo.db(),
        &NewJob {
            id: "action-retry-seed-job",
            kind: "maintenance",
            operation: "actor_seed",
            parent: None,
            created_at: Utc::now(),
        },
        &NewTask {
            id: "action-retry-seed-job:execute",
            job: "action-retry-seed-job",
            pool: "actor_seed",
            operation: "actor_seed",
            priority: 0,
            deps: Vec::new(),
            input_hash: "fixture-seed-hash",
            created_at: Utc::now(),
        },
        &retry_admission,
    )
    .await
    .expect("submit retry fixture");
    repo.db()
        .query(
            "CREATE setup_operation:`action-retry-seed-job` SET operation='actor_seed', payload_json='{}', payload_hash='fixture-seed-hash', scope_id='action-scope', updated_at=time::now() RETURN NONE;",
        )
        .await
        .expect("seed operation")
        .check()
        .expect("seed operation statement");

    let mut now = Utc::now();
    for attempt in 0..MAX_WORKER_RECOVERIES {
        let relay = job_dispatch::claim_ready(repo.db(), "actor_seed", "seed-relay", now)
            .await
            .expect("claim seed outbox")
            .expect("seed delivery");
        let DeliveryClaim::Claimed(_) = job_dispatch::claim_delivery(
            repo.db(),
            &relay.envelope,
            "seed-worker",
            5,
            now,
            &format!("seed-claim-{attempt}"),
        )
        .await
        .expect("claim seed delivery") else {
            panic!("seed delivery must be runnable after each recovery");
        };
        if attempt == 0 {
            // A committed seed batch creates a causal index child via the same
            // outbox transaction used by the worker.
            tx::run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
                actor_search_outbox::enqueue_actors_in(
                    &tx,
                    &["synthetic-seeded-actor".into()],
                    Some(&SourceCause {
                        parent_job: "action-retry-seed-job".into(),
                        operation_id: "seed-fixture-attempt".into(),
                    }),
                    now,
                )
                .await
            })
            .await
            .expect("publish seed index intent");
        }
        now += Duration::seconds(6);
        assert_eq!(
            job_dispatch::recover_expired(repo.db(), "actor_seed", now)
                .await
                .expect("recover expired seed lease"),
            1
        );
    }

    let retry_input = json!({"id":"action-retry-seed-job","request_id":"retry-action-once"});
    assert_eq!(
        call(
            &app,
            "POST",
            JOB_RETRY_PATH,
            READER,
            Some(&registration),
            retry_input.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (_, jobs_before) = call(
        &app,
        "GET",
        "/api/v1/jobs",
        ACTOR,
        Some(&registration),
        Value::Null,
    )
    .await;
    let seed_row = jobs_before["rows"]
        .as_array()
        .expect("job rows")
        .iter()
        .find(|row| row["id"] == "action-retry-seed-job")
        .expect("seed row");
    assert_eq!(seed_row["can_retry"], true);

    let detail_path = format!("{JOB_DETAIL_PATH}?id=action-retry-seed-job");
    let (detail_status, detail) = call(
        &app,
        "GET",
        &detail_path,
        ACTOR,
        Some(&registration),
        Value::Null,
    )
    .await;
    assert_eq!(detail_status, StatusCode::OK, "{detail}");
    assert_eq!(detail["activity_actions_available"], true);
    assert_eq!(detail["job"]["can_retry"], true);
    assert_eq!(detail["job"]["state"], "failed");
    assert_eq!(
        detail["job"]["error_summary"],
        "Task lease expired after maximum worker recoveries"
    );

    let (retry_status, retry_receipt) = call(
        &app,
        "POST",
        JOB_RETRY_PATH,
        ACTOR,
        Some(&registration),
        retry_input.clone(),
    )
    .await;
    assert_eq!(retry_status, StatusCode::OK, "{retry_receipt}");
    assert_eq!(retry_receipt["receipt"]["action"], "retry");
    assert_eq!(retry_receipt["receipt"]["state"], "queued");
    assert_eq!(
        call(
            &app,
            "POST",
            JOB_RETRY_PATH,
            ACTOR,
            Some(&registration),
            retry_input
        )
        .await,
        (StatusCode::OK, retry_receipt)
    );
    let relay = job_dispatch::claim_ready(repo.db(), "actor_seed", "seed-relay", now)
        .await
        .expect("claim retry outbox")
        .expect("retry delivery for the original job");
    assert_eq!(relay.envelope.job_id, "action-retry-seed-job");
    let DeliveryClaim::Claimed(retried) = job_dispatch::claim_delivery(
        repo.db(),
        &relay.envelope,
        "seed-worker-successor",
        5,
        now,
        "seed-claim-manual-retry",
    )
    .await
    .expect("claim retried seed") else {
        panic!("the existing seed task must be runnable after manual retry");
    };
    assert_eq!(retried.task.id, "action-retry-seed-job:execute");
    assert_eq!(retried.task.attempts, MAX_WORKER_RECOVERIES + 1);

    let mut index_jobs = repo
        .db()
        .query(
            "SELECT VALUE state FROM job WHERE parent=job:`action-retry-seed-job`
             AND operation='index_actor_catalog';",
        )
        .await
        .expect("read seed index jobs")
        .check()
        .expect("check seed index jobs");
    assert_eq!(
        index_jobs.take::<Vec<String>>(0).expect("index children"),
        ["queued"]
    );
}
