//! Offline real gateway/worker acceptance, attribution and result boundaries.
#![cfg(feature = "embedded")]

use std::ffi::OsString;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use argos_osint_api::network_router;
use argos_osint_api::ApiState;
use argos_osint_api::NetworkPolicy;
use argos_osint_api::ScopedCredential;
use argos_osint_core::config_commit::CommitFile;
use argos_osint_core::config_commit::ConfigLock;
use argos_osint_core::osint;
use argos_osint_core::osint::manual;
use argos_osint_core::osint::manual::DurableToolOutcome;
use argos_osint_core::osint::manual::Tools;
use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::job_dispatch;
use argos_osint_core::storage::job_dispatch::DeliveryClaim;
use argos_osint_core::storage::job_dispatch::DeliveryFinish;
use argos_osint_core::storage::job_dispatch::DeliveryOutcome;
use argos_osint_core::storage::open::Target;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::tool_state;
use axum::body::to_bytes;
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::Router;
use chrono::Utc;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::MutationContext;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::TOOL_RESULT_PATH;
use fleetworks_contracts::TOOL_RUN_PATH;
use serde_json::json;
use serde_json::Value;
use tower::ServiceExt;

const NATIVE: &str = "tools-native-fixture-0123456789abcdefghijklmnopqrstuvwxyz";
const WEB: &str = "tools-web-fixture-0123456789abcdefghijklmnopqrstuvwxyz";
const READ_ONLY: &str = "tools-readonly-fixture-0123456789abcdefghijklmnopqrstuvwxyz";

struct Environment(Vec<(&'static str, Option<OsString>)>);

impl Environment {
    fn install(home: &std::path::Path) -> Self {
        let keys = ["ARGOS_HOME", "FLEETWORKS_WORKSPACE"];
        let previous = keys
            .into_iter()
            .map(|key| (key, std::env::var_os(key)))
            .collect();
        for key in keys {
            std::env::set_var(key, home);
        }
        Self(previous)
    }
}

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

fn authority(id: &str) -> RegistryAuthority {
    RegistryAuthority::new("fixture-scope", id, "fixture-access").expect("fixture authority")
}

fn app(repo: &Repository) -> Router {
    let native = ScopedCredential::new(
        "fixture-native".into(),
        "fixture-scope".into(),
        "fixture-access".into(),
        CredentialPurpose::Native,
        [
            Permission::ReadBackend,
            Permission::RegisterWorkstation,
            Permission::SubmitInteractive,
            Permission::ReadWorkstationActivity,
        ]
        .into_iter()
        .collect(),
        NATIVE,
    )
    .expect("fixture Native policy");
    let web = ScopedCredential::new(
        "fixture-web".into(),
        "fixture-scope".into(),
        "fixture-access".into(),
        CredentialPurpose::WebRelay,
        [
            Permission::ReadBackend,
            Permission::ResolveWebSession,
            Permission::SubmitInteractive,
            Permission::ReadWorkstationActivity,
        ]
        .into_iter()
        .collect(),
        WEB,
    )
    .expect("fixture WebRelay policy");
    let read_only = ScopedCredential::new(
        "fixture-reader".into(),
        "fixture-scope".into(),
        "fixture-access".into(),
        CredentialPurpose::Native,
        [Permission::ReadBackend].into_iter().collect(),
        READ_ONLY,
    )
    .expect("fixture read-only policy");
    let policy =
        NetworkPolicy::with_credentials(&["127.0.0.1:8080".into()], vec![native, web, read_only])
            .expect("fixture gateway policy");
    network_router(
        ApiState {
            repo: repo.clone(),
            instance_id: "fixture-tools".into(),
            port: 8080,
            origins: Arc::new(Vec::new()),
        },
        policy,
    )
}

enum Proof<'a> {
    None,
    Native(&'a RegistrationReceipt),
    Web(&'a str, u64),
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    proof: Proof<'_>,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:8080")
        .header("authorization", format!("Bearer {token}"))
        .header("x-fleetworks-origin-workstation-id", "forged-other-node")
        .header("x-fleetworks-workspace-id", "forged-other-child");
    request = match proof {
        Proof::None => request,
        Proof::Native(receipt) => request
            .header("x-fleetworks-registration", &receipt.registration_id)
            .header(
                "x-fleetworks-registration-revision",
                receipt.revision.to_string(),
            ),
        Proof::Web(token, revision) => request
            .header("x-fleetworks-session", token)
            .header("x-fleetworks-context-revision", revision.to_string()),
    };
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
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("bounded API response");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn register(repo: &Repository, key: &str, ip: &str, install: &str) -> RegistrationReceipt {
    connect_registry::ensure(
        repo,
        &authority("fixture-native"),
        &RegistrationEnsureRequest::new(
            key.into(),
            Some(ip.parse().expect("synthetic unicast IP")),
            install.into(),
        ),
    )
    .await
    .expect("fixture registry")
    .receipt
}

#[tokio::test]
async fn acceptance_survives_router_drop_and_native_web_origins_and_results_stay_scoped() {
    Box::pin(acceptance_scenario()).await;
}

async fn acceptance_scenario() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let home = directory.path().join("home");
    std::fs::create_dir(&home).expect("fixture home");
    let _environment = Environment::install(&home.canonicalize().expect("canonical fixture home"));
    let repo = Repository::connect(
        &Target::Embedded {
            path: directory.path().join("db"),
        },
        "argos",
        "durable_tools",
    )
    .await
    .expect("fixture repository");
    repo.db()
        .query(
            "UPSERT storage_meta:identity SET instance_id='fixture-tools-deployment' RETURN NONE",
        )
        .await
        .expect("fixture deployment")
        .check()
        .expect("deployment statement");
    connect_registry::initialize_control(repo.db())
        .await
        .expect("explicit fixture bootstrap");
    job_dispatch::enable_pool(repo.db(), manual::TOOLS_POOL)
        .await
        .expect("explicit fixture Tools consumer enablement");
    let native = register(&repo, "native-a", "192.0.2.42", "fixture-install-a").await;
    let other_node = register(&repo, "native-b", "198.51.100.42", "fixture-install-b").await;
    assert_ne!(native.workstation_id, other_node.workstation_id);
    let native_context = connect_registry::registration_context(
        &repo,
        &authority("fixture-native"),
        &native.registration_id,
        native.revision,
    )
    .await
    .expect("server-resolved native mutation context");
    let configuration_revision = ConfigLock::acquire_at(&home)
        .expect("fixture current configuration lock")
        .generation();
    let mutation =
        MutationContext::new(&native_context).with_configuration_revision(configuration_revision);
    let inputs = json!({"domain":"example.test"});
    tool_state::set_tool_enabled(&repo, "crtsh_certificates", true)
        .await
        .expect("fixture tool enabled");
    // The real ToolRunner consumes this saved result; no provider is contacted.
    tool_state::cache_put(
        &repo,
        &osint::cache_identity("crtsh_certificates", &inputs),
        &osint::ToolResult {
            tool_id: "crtsh_certificates".into(),
            inputs: inputs.clone(),
            status: "completed".into(),
            source_url: "https://example.test/fixture".into(),
            retrieved_at: Utc::now().to_rfc3339(),
            observations: json!({"domains":["cached.example.test"]}),
            raw: String::new(),
            error: None,
            cached: false,
            truncated: false,
            credits_charged: 0,
            credits_reported: None,
        },
        3600,
    )
    .await
    .expect("offline tool cache fixture");
    let request = json!({"request_id":"native-request-1","tool_id":"crtsh_certificates","inputs":inputs,"context":mutation});
    let gateway = app(&repo);
    assert_eq!(call(&gateway,"POST",TOOL_RUN_PATH,NATIVE,Proof::Native(&native),json!({"request_id":"missing-context","tool_id":"crtsh_certificates","inputs":{"domain":"example.test"}})).await.0,StatusCode::PRECONDITION_REQUIRED);
    assert_eq!(
        call(
            &gateway,
            "POST",
            TOOL_RUN_PATH,
            NATIVE,
            Proof::None,
            request.clone()
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &gateway,
            "POST",
            TOOL_RUN_PATH,
            READ_ONLY,
            Proof::Native(&native),
            request.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, receipt) = call(
        &gateway,
        "POST",
        TOOL_RUN_PATH,
        NATIVE,
        Proof::Native(&native),
        request.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{receipt}");
    assert_eq!(receipt["call_id"], receipt["job_id"]);
    let job_id = receipt["job_id"]
        .as_str()
        .expect("durable receipt job ID")
        .to_owned();
    assert_eq!(
        call(
            &gateway,
            "POST",
            TOOL_RUN_PATH,
            NATIVE,
            Proof::Native(&native),
            request.clone()
        )
        .await,
        (StatusCode::ACCEPTED, receipt.clone())
    );
    let changed = json!({"request_id":"native-request-1","tool_id":"crtsh_certificates","inputs":{"domain":"different.test"},"context":mutation});
    assert_eq!(
        call(
            &gateway,
            "POST",
            TOOL_RUN_PATH,
            NATIVE,
            Proof::Native(&native),
            changed
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    drop(gateway);
    let queued = manual::durable_status(&repo, &job_id)
        .await
        .expect("persisted status")
        .expect("persisted call");
    assert_eq!(queued.status, "queued");
    let mut response = repo.db().query("RETURN (SELECT VALUE origin_json FROM ONLY type::record('job_origin',$job)); SELECT VALUE count() FROM job_request WHERE job_id=$job GROUP ALL; SELECT VALUE count() FROM job_dispatch_outbox WHERE job=type::record('job',$job) GROUP ALL;")
        .bind(("job",job_id.clone())).await.expect("durable acceptance audit").check().expect("acceptance audit statements");
    let origin: Value = serde_json::from_str(
        &response
            .take::<Option<String>>(0)
            .expect("origin field")
            .expect("saved origin"),
    )
    .expect("immutable origin JSON");
    assert_eq!(origin["source"], "native");
    assert_eq!(origin["workstation_id"], native.workstation_id);
    assert_eq!(origin["workspace_id"], serde_json::Value::Null);
    assert_ne!(origin["workstation_id"], "forged-other-node");
    assert_eq!(
        response.take::<Vec<i64>>(1).expect("request count"),
        vec![1]
    );
    assert_eq!(response.take::<Vec<i64>>(2).expect("outbox count"), vec![1]);
    let delivery =
        job_dispatch::claim_ready(repo.db(), manual::TOOLS_POOL, "fixture-relay", Utc::now())
            .await
            .expect("outbox claim")
            .expect("durable pending work");
    let lease = match job_dispatch::claim_delivery(
        repo.db(),
        &delivery.envelope,
        "fixture-worker",
        60,
        Utc::now(),
        "fixture-claim-1",
    )
    .await
    .expect("actual execution claim")
    {
        DeliveryClaim::Claimed(lease) => lease,
        other => panic!("expected executable claim, got {other:?}"),
    };
    let outcome = Tools::default()
        .execute_claimed(&repo, &lease.task, Arc::new(AtomicBool::new(false)))
        .await
        .expect("real cached ToolRunner execution");
    let DurableToolOutcome::Complete { result_ref } = outcome else {
        panic!("cached fixture must complete: {outcome:?}");
    };
    assert_eq!(
        job_dispatch::finish_delivery(
            repo.db(),
            &lease,
            DeliveryOutcome::Complete {
                result_ref: &result_ref
            },
            Utc::now()
        )
        .await
        .expect("fenced worker completion"),
        DeliveryFinish::Completed
    );
    let gateway = app(&repo);
    let path = format!("{TOOL_RESULT_PATH}?id={job_id}");
    let (status, result) = call(
        &gateway,
        "GET",
        &path,
        NATIVE,
        Proof::Native(&native),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["status"], "completed");
    assert_eq!(result["result"]["cached"], true);
    assert_eq!(
        call(
            &gateway,
            "GET",
            &path,
            NATIVE,
            Proof::Native(&other_node),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &gateway,
            "GET",
            &path,
            READ_ONLY,
            Proof::Native(&native),
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&gateway, "GET", &path, NATIVE, Proof::None, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert!(matches!(
        Tools::default()
            .execute_claimed(&repo, &lease.task, Arc::new(AtomicBool::new(false)))
            .await
            .expect("stale duplicate claim"),
        DurableToolOutcome::Stale
    ));
    // A genuine second requester may see lifecycle membership, but an opaque
    // result URL cannot expose private shared payload to either participant.
    repo.db().query("CREATE job_request:shared_tool_result SET request_key='shared-tool-result',operation='manual_tool',job_id=$job,scope_id='fixture-scope',deployment_id=$deployment,registration_id=$registration,workstation_id=$node,workspace_id=$workspace,requested_at=$now,origin_json='{}' RETURN NONE")
        .bind(("job",job_id.clone())).bind(("deployment",other_node.deployment_id.clone())).bind(("registration",other_node.registration_id.clone()))
        .bind(("node",other_node.workstation_id.clone())).bind(("workspace",None::<String>)).bind(("now",Utc::now().to_rfc3339()))
        .await.expect("genuine shared association fixture").check().expect("shared association statement");
    assert_eq!(
        call(
            &gateway,
            "GET",
            &path,
            NATIVE,
            Proof::Native(&native),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &gateway,
            "GET",
            &path,
            NATIVE,
            Proof::Native(&other_node),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let uncertain_request = json!({"request_id":"uncertain-request-1","tool_id":"crtsh_certificates","inputs":{"domain":"example.test"},"context":mutation});
    let (status, uncertain_receipt) = call(
        &gateway,
        "POST",
        TOOL_RUN_PATH,
        NATIVE,
        Proof::Native(&native),
        uncertain_request,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{uncertain_receipt}");
    let uncertain_job = uncertain_receipt["job_id"]
        .as_str()
        .expect("uncertain job ID");
    let uncertain_delivery =
        job_dispatch::claim_ready(repo.db(), manual::TOOLS_POOL, "fixture-relay-2", Utc::now())
            .await
            .expect("uncertain outbox claim")
            .expect("pending work");
    let uncertain_lease = match job_dispatch::claim_delivery(
        repo.db(),
        &uncertain_delivery.envelope,
        "fixture-worker-2",
        60,
        Utc::now(),
        "fixture-claim-2",
    )
    .await
    .expect("uncertain execution claim")
    {
        DeliveryClaim::Claimed(lease) => lease,
        other => panic!("expected uncertain claim, got {other:?}"),
    };
    assert_eq!(uncertain_lease.task.job_id, uncertain_job);
    repo.db()
        .query(
            "UPDATE type::record('osint_call',$call) SET status='running',attempts=1 RETURN NONE",
        )
        .bind(("call", uncertain_job.to_owned()))
        .await
        .expect("simulate persisted pre-provider attempt marker")
        .check()
        .expect("attempt marker statement");
    let outcome = Tools::default()
        .execute_claimed(
            &repo,
            &uncertain_lease.task,
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .expect("uncertain attempt recovery");
    assert!(
        matches!(
            outcome,
            DurableToolOutcome::Failed {
                reason: "manual_tool_previous_attempt_uncertain"
            }
        ),
        "provider invocation must not be silently repeated: {outcome:?}"
    );
    assert_eq!(
        manual::durable_status(&repo, uncertain_job)
            .await
            .expect("interrupted status")
            .expect("saved call")
            .status,
        "interrupted"
    );
    job_dispatch::finish_delivery(
        repo.db(),
        &uncertain_lease,
        DeliveryOutcome::Failed {
            reason: "manual_tool_previous_attempt_uncertain",
        },
        Utc::now(),
    )
    .await
    .expect("durable interrupted worker finish");
    let session =
        connect_registry::create_session(&repo, &authority("fixture-web"), &native.connect_id)
            .await
            .expect("fixture WebRelay session");
    let web_mutation =
        MutationContext::new(&session.context).with_configuration_revision(configuration_revision);
    let web_request = json!({"request_id":"web-request-1","tool_id":"crtsh_certificates","inputs":{"domain":"example.test"},"context":web_mutation});
    let (status, web_receipt) = call(
        &gateway,
        "POST",
        TOOL_RUN_PATH,
        WEB,
        Proof::Web(
            session.token.expose_secret(),
            session.context.context_revision,
        ),
        web_request.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{web_receipt}");
    let mut response = repo
        .db()
        .query("RETURN type::record('job_origin',$job).origin_json;")
        .bind((
            "job",
            web_receipt["job_id"]
                .as_str()
                .expect("web job ID")
                .to_owned(),
        ))
        .await
        .expect("web origin audit")
        .check()
        .expect("web origin statement");
    let web_origin: Value = serde_json::from_str(
        &response
            .take::<Option<String>>(0)
            .expect("web origin field")
            .expect("web origin"),
    )
    .expect("web origin JSON");
    assert_eq!(web_origin["source"], "intel_web");
    assert!(!web_origin
        .to_string()
        .contains(session.token.expose_secret()));
    connect_registry::revoke_session(
        &repo,
        &authority("fixture-web"),
        session.token.expose_secret(),
    )
    .await
    .expect("explicit session logout");
    assert_eq!(
        call(
            &gateway,
            "POST",
            TOOL_RUN_PATH,
            WEB,
            Proof::Web(
                session.token.expose_secret(),
                session.context.context_revision
            ),
            web_request
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    // Logout cannot cancel the already accepted queued work.
    assert_eq!(
        manual::durable_status(&repo, web_receipt["call_id"].as_str().expect("web call ID"))
            .await
            .expect("accepted work after logout")
            .expect("durable call")
            .status,
        "queued"
    );
    job_dispatch::configure_pool(repo.db(), manual::TOOLS_POOL, false)
        .await
        .expect("pause durable admission");
    let request = json!({"request_id":"disabled-request","tool_id":"crtsh_certificates","inputs":{"domain":"disabled.example.test"},"context":mutation});
    assert_eq!(
        call(
            &gateway,
            "POST",
            TOOL_RUN_PATH,
            NATIVE,
            Proof::Native(&native),
            request
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let mut response = repo.db().query("SELECT VALUE count() FROM osint_call WHERE string::contains(inputs_json,'disabled.example.test') GROUP ALL;")
        .await.expect("failed transaction rollback audit").check().expect("rollback statement");
    assert_eq!(
        response
            .take::<Vec<i64>>(0)
            .expect("rolled back input rows")
            .first()
            .copied()
            .unwrap_or(0),
        0
    );
    job_dispatch::configure_pool(repo.db(), manual::TOOLS_POOL, true)
        .await
        .expect("re-enable fixture admission");
    let current_config = home.join("config.toml");
    let mut new_config = std::fs::read_to_string(&current_config).unwrap_or_else(|_| {
        let mut settings = argos_osint_core::provider::SettingsFile::default();
        for role in argos_osint_core::provider::CANONICAL_ROLE_KEYS {
            *settings.defaults.role_mut(role).expect("canonical role") = Default::default();
        }
        format!(
            "configuration_format=1\n{}",
            toml::to_string(&settings).expect("fixture current format")
        )
    });
    new_config.push_str("\n# synthetic configuration revision change\n");
    let revision = ConfigLock::acquire_at(&home)
        .expect("fixture config mutation lock")
        .commit_files(&[CommitFile::new(
            "current_settings",
            current_config,
            new_config,
        )])
        .expect("actual fixture configuration revision commit");
    assert!(revision > configuration_revision);
    let stale = json!({"request_id":"stale-new-request","tool_id":"crtsh_certificates","inputs":{"domain":"example.test"},"context":mutation});
    assert_eq!(
        call(
            &gateway,
            "POST",
            TOOL_RUN_PATH,
            NATIVE,
            Proof::Native(&native),
            stale
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let exact = json!({"request_id":"native-request-1","tool_id":"crtsh_certificates","inputs":{"domain":"example.test"},"context":mutation});
    assert_eq!(
        call(
            &gateway,
            "POST",
            TOOL_RUN_PATH,
            NATIVE,
            Proof::Native(&native),
            exact
        )
        .await,
        (StatusCode::ACCEPTED, receipt)
    );
}
