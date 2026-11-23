#![cfg(feature = "embedded")]

#[allow(dead_code)]
#[path = "../src/actor_index_worker.rs"]
mod actor_index_worker;
#[cfg(all(feature = "classifier", unix))]
#[path = "../src/classifier_service.rs"]
mod classifier_service;
#[allow(dead_code)]
#[path = "../src/native_worker.rs"]
mod native_worker;
#[allow(dead_code)]
#[path = "../src/queue_client.rs"]
mod queue_client;
#[allow(dead_code)]
#[path = "../src/setup_jobs.rs"]
mod setup_jobs;

#[allow(dead_code)]
mod fleetworks {
    pub(crate) fn read_secret(_name: &str) -> anyhow::Result<String> {
        anyhow::bail!("in-process parity HTTP harness injects local services directly")
    }
}

#[cfg(test)]
mod tests {
    pub(crate) static EMBEDDED_LIFECYCLE_TEST_LOCK: tokio::sync::Mutex<()> =
        tokio::sync::Mutex::const_new(());
}

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use argos_osint_api::actor_search::SearchClient;
use argos_osint_api::classifier_activation::ActivationArtifacts;
use argos_osint_api::datasets::DatasetStore;
use argos_osint_api::network_router_with_private_services;
use argos_osint_api::workstation::ReadinessOwners;
use argos_osint_api::workstation::ReadinessProbe;
use argos_osint_api::ApiState;
use argos_osint_api::NetworkPolicy;
use argos_osint_api::PrivateServices;
use argos_osint_api::ScopedCredential;
use argos_osint_core::storage::bootstrap;
use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::jobs_client;
use argos_osint_core::storage::open::Target;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::server;
use argos_osint_core::storage::setup_jobs as core_setup_jobs;
use fleetworks_contracts::actor_index::ActorIndexLimits;
use fleetworks_contracts::capabilities::Capabilities;
use fleetworks_contracts::chat::ChatTurnAck;
use fleetworks_contracts::chat::QaCancelRequest;
use fleetworks_contracts::chat::QaEventQuery;
use fleetworks_contracts::chat::QaPhase;
use fleetworks_contracts::chat::QaSettingsUpdate;
use fleetworks_contracts::chat::QaTurnRequest;
use fleetworks_contracts::chat::CHAT_TURNS_PATH;
use fleetworks_contracts::classifier_activation::ActivationRequest;
use fleetworks_contracts::classifier_activation::ActiveModelRevision;
use fleetworks_contracts::classifier_activation::CandidateIdentity;
use fleetworks_contracts::classifier_activation::SyntheticBaselineConsent;
use fleetworks_contracts::classifier_activation::ACTIVATION_PATH;
use fleetworks_contracts::classifier_activation::ACTIVATION_RESULT_PATH;
use fleetworks_contracts::classifier_activation::CANDIDATE_DETAIL_PATH;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::connect::REGISTRATION_ENSURE_PATH;
use fleetworks_contracts::connect::WEB_SESSION_CREATE_PATH;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::MutationContext;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::context::WorkstationRequestDetailQuery;
use fleetworks_contracts::cycle_history::DeleteCycleRequest;
use fleetworks_contracts::ipc::read_frame;
use fleetworks_contracts::ipc::write_frame;
use fleetworks_contracts::ipc::IpcEnvelope;
use fleetworks_contracts::ipc::IpcHello;
use fleetworks_contracts::ipc::IpcMethod;
use fleetworks_contracts::ipc::IpcOperation;
use fleetworks_contracts::ipc::LocalIpcClient;
use fleetworks_contracts::operational_logs::LogQuery;
use fleetworks_contracts::readiness::DatasetKind;
use fleetworks_contracts::readiness::DatasetRef;
use fleetworks_contracts::readiness::DeclaredAtlasBertCandidate;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::MaintenanceReceipt;
use fleetworks_contracts::readiness::MaintenanceRequest;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use fleetworks_contracts::readiness::WorkstationReadiness;
use fleetworks_contracts::readiness::ACTOR_SEED_PATH;
use fleetworks_contracts::readiness::CLASSIFIER_TRAIN_PATH;
use fleetworks_contracts::search::ActorSearchQuery;
use fleetworks_contracts::search::ActorSearchResponse;
use fleetworks_contracts::search::IndexState;
use fleetworks_contracts::search::ACTOR_DETAIL_PATH;
use fleetworks_contracts::search::ACTOR_SEARCH_PATH;
use fleetworks_contracts::AtlasCycleRequest;
use fleetworks_contracts::Client;
use fleetworks_contracts::ClientError;
use fleetworks_contracts::Jobs;
use fleetworks_contracts::ToolRunRequest;
use fleetworks_contracts::ATLAS_CYCLES_PATH;
use fleetworks_contracts::CAPABILITIES_PATH;
use fleetworks_contracts::JOBS_PATH;
use fleetworks_contracts::JOB_CANCEL_PATH;
use fleetworks_contracts::JOB_DETAIL_PATH;
use fleetworks_contracts::JOB_RETRY_PATH;
use fleetworks_contracts::TOOL_RESULT_PATH;
use fleetworks_contracts::TOOL_RUN_PATH;
use fleetworks_search_index::config::LocalEngineConfig;
use fleetworks_search_index::engine::Engine;
use fleetworks_search_index::service::EngineSlot;
use fleetworks_search_index::service::SearchService;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use sha2::Digest;
use sha2::Sha256;
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::sync::oneshot;
use tokio::sync::watch;

use crate::actor_index_worker::ActorIndexWorker;
use crate::actor_index_worker::LocalActorSearchBackend;
use crate::native_worker::LocalWorkerConfig;

const HOST_METRICS_PATH: &str = "/api/v1/host/metrics";
const WORKSTATION_DATASETS_PATH: &str = "/api/v1/workstation/datasets";
const TOOLS_PATH: &str = "/api/v1/tools";
const MODELS_DEFAULTS_PATH: &str = "/api/v1/models/defaults";
const MODELS_DEFAULTS_SET_PATH: &str = "/api/v1/models/defaults/set";
const PROFILE_PATH: &str = "/api/v1/profile";
const PROFILE_ACCOUNTS_UPSERT_PATH: &str = "/api/v1/profile/accounts/upsert";
const PROFILE_ACCOUNTS_DELETE_PATH: &str = "/api/v1/profile/accounts/delete";
const PROFILE_TOOLS_SET_PATH: &str = "/api/v1/profile/tools/set";
const ARTICLES_PATH: &str = "/api/v1/articles";
const ARTICLES_SEARCH_PATH: &str = "/api/v1/articles/search";
const LOGS_PATH: &str = "/api/v1/logs";
const LOGS_FOLLOW_PATH: &str = "/api/v1/logs/follow";
const REGISTRATION_CONTEXT_PATH: &str = "/api/v1/connect/registrations/context";
const REGISTRATION_REBIND_PATH: &str = "/api/v1/connect/registrations/rebind";
const REGISTRATION_RETIRE_PATH: &str = "/api/v1/connect/registrations/retire";

const TOKEN: &str = "parity-test-native-credential-0123456789abcdef";

#[derive(Deserialize)]
struct RegistrationContextBody {
    context: RegisteredContext,
}

#[derive(Deserialize)]
struct ConfigurationStatusView {
    configuration_revision: u64,
}

#[derive(Clone, Debug, Serialize)]
struct ParityOperationRecord {
    category: String,
    http_verified: bool,
    ipc_verified: bool,
    summary: String,
}

#[derive(Clone, Debug, Serialize)]
struct IntentionalExceptionRecord {
    id: String,
    operations: Vec<String>,
    reason: String,
    http_behavior: String,
    ipc_behavior: String,
    verified: bool,
}

#[derive(Debug, Default, Serialize)]
struct ParityReport {
    report_version: u16,
    generated_by: String,
    total_ipc_operations: usize,
    covered_ipc_operations: usize,
    operations: BTreeMap<String, ParityOperationRecord>,
    intentional_exceptions: Vec<IntentionalExceptionRecord>,
}

impl ParityReport {
    fn new() -> Self {
        Self {
            report_version: 1,
            generated_by: format!(
                "services/api-host/tests/http_ipc_parity.rs (companion: {})",
                companion_source_description()
            ),
            total_ipc_operations: IpcOperation::ALL_OPERATION_NAMES.len(),
            covered_ipc_operations: 0,
            operations: BTreeMap::new(),
            intentional_exceptions: Vec::new(),
        }
    }

    fn record_http(&mut self, op: &str, category: &str, summary: &str) {
        let entry = self
            .operations
            .entry(op.to_owned())
            .or_insert_with(|| ParityOperationRecord {
                category: category.to_owned(),
                http_verified: false,
                ipc_verified: false,
                summary: summary.to_owned(),
            });
        entry.http_verified = true;
        entry.category = category.to_owned();
        entry.summary = summary.to_owned();
    }

    fn record_ipc(&mut self, op: &str, category: &str, summary: &str) {
        let entry = self
            .operations
            .entry(op.to_owned())
            .or_insert_with(|| ParityOperationRecord {
                category: category.to_owned(),
                http_verified: false,
                ipc_verified: false,
                summary: summary.to_owned(),
            });
        entry.ipc_verified = true;
        entry.category = category.to_owned();
        entry.summary = summary.to_owned();
    }

    fn record_exception(&mut self, entry: IntentionalExceptionRecord) {
        assert!(
            entry.verified,
            "intentional exception must be executable and verified: {}",
            entry.id
        );
        self.intentional_exceptions.push(entry);
    }

    fn finalize_and_assert_complete(&mut self) {
        let expected: BTreeSet<&'static str> =
            IpcOperation::ALL_OPERATION_NAMES.iter().copied().collect();
        let actual: BTreeSet<&str> = self.operations.keys().map(String::as_str).collect();
        let missing: Vec<&str> = expected.difference(&actual).copied().collect();
        let extra: Vec<&str> = actual.difference(&expected).copied().collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "IPC operation parity report mismatch: missing={missing:?}, extra={extra:?}"
        );
        for (name, rec) in &self.operations {
            assert!(
                rec.http_verified && rec.ipc_verified,
                "operation `{name}` missing verification (http={}, ipc={})",
                rec.http_verified,
                rec.ipc_verified
            );
        }
        self.covered_ipc_operations = self.operations.len();
    }
}

fn create_private_dir(prefix: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp root");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .expect("chmod 0700 temp root");
    }
    dir.canonicalize().expect("canonical temp root")
}

/// Spawn a local IPC companion process for parity testing.
///
/// In CI, the `FLEETWORKS_PARITY_COMPANION_PATH` environment variable should be set to the
/// extracted companion binary path from the native archive. When unset (local dev), falls back
/// to the Cargo-built binary via `CARGO_BIN_EXE_argos-api-host`.
///
/// The test records which binary was exercised in the parity report's `generated_by` field.
fn spawn_local_ipc_command(state_dir: &Path) -> Command {
    let companion_path = std::env::var("FLEETWORKS_PARITY_COMPANION_PATH");
    let mut command = match companion_path {
        Ok(ref path) => {
            let path = std::path::PathBuf::from(path);
            assert!(
                path.is_file(),
                "FLEETWORKS_PARITY_COMPANION_PATH does not point to a file: {}",
                path.display()
            );
            Command::new(path)
        }
        Err(_) => {
            assert!(
                std::env::var_os("FLEETWORKS_PARITY_REQUIRE_COMPANION").is_none(),
                "FLEETWORKS_PARITY_REQUIRE_COMPANION is set but FLEETWORKS_PARITY_COMPANION_PATH is not"
            );
            Command::new(env!("CARGO_BIN_EXE_argos-api-host"))
        }
    };
    command
        .arg("--role")
        .arg("local-ipc")
        .arg("--state-dir")
        .arg(state_dir);
    command
}

/// Returns a description of which companion binary source is being used.
fn companion_source_description() -> String {
    match std::env::var("FLEETWORKS_PARITY_COMPANION_PATH") {
        Ok(path) => format!("extracted companion from native archive: {}", path),
        Err(_) => "Cargo-built argos-api-host (dev fallback)".to_owned(),
    }
}

struct DeclaredClassifierHarness {
    home: tempfile::TempDir,
    expected: DeclaredAtlasBertCandidate,
    probe: ReadinessProbe,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for DeclaredClassifierHarness {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(all(feature = "classifier", unix))]
mod declared_classifier_fixture {
    use argos_osint_core::atlas_taxonomy::{DomainId, DOMAIN_IDS};
    use argos_osint_core::classifier::artifact;

    mod candidate {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/argos-osint-core/tests/fixtures/classifier/atlas_bert_candidate.rs"
        ));
    }

    use super::*;
    use argos_osint_core::classifier::bert_activation;
    use argos_osint_core::classifier::readiness::ClassifierHome;
    use argos_osint_core::classifier::WorkControl;
    use std::os::unix::fs::PermissionsExt;

    const CLASSIFIER_TOKEN: &str = "parity-test-private-classifier-0123456789abcdef";

    pub(super) async fn start() -> DeclaredClassifierHarness {
        let fixture = candidate::StagedCandidate::new(0.8);
        let home = fixture.home.path();
        std::fs::set_permissions(home, std::fs::Permissions::from_mode(0o700))
            .expect("make classifier and companion fixture workspace private");
        let prior = bert_activation::revision(home)
            .await
            .expect("baseline revision");
        let activated = bert_activation::activate(
            home,
            &fixture.frozen_sha256,
            &fixture.evaluation_sha256,
            &prior,
            &WorkControl::new(),
        )
        .await
        .expect("activate synthetic fixture candidate");
        let revision = bert_activation::revision(home)
            .await
            .expect("operator readback revision");
        let expected = DeclaredAtlasBertCandidate::new(activated.snapshot.model_id, revision);
        // The readback is intentionally independent of model bytes and runtime warming.
        std::fs::write(
            fixture.model_dir.join("rust_model.ot"),
            "different fixture bytes",
        )
        .expect("change the candidate weights without changing the declaration");
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("private classifier listener");
        let address = listener.local_addr().expect("private classifier address");
        let policy = NetworkPolicy::new(&[address.to_string()], CLASSIFIER_TOKEN)
            .expect("private classifier credential");
        let app = argos_osint_api::guard_network(
            classifier_service::router(ClassifierHome::new(home.to_owned())),
            policy,
            Arc::new(Vec::new()),
            address.port(),
        );
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve private classifier status");
        });
        let probe = ReadinessProbe::new(
            ReadinessCheckId::Classifier,
            &format!("http://{address}"),
            CLASSIFIER_TOKEN,
        )
        .expect("private classifier probe");
        DeclaredClassifierHarness {
            home: fixture.home,
            expected,
            probe,
            task,
        }
    }
}

struct HttpHarness {
    root: PathBuf,
    origin: String,
    repo: Repository,
    stop_tx: watch::Sender<bool>,
    server_shutdown: Option<oneshot::Sender<()>>,
    server_task: tokio::task::JoinHandle<()>,
    worker_task: tokio::task::JoinHandle<()>,
}

impl HttpHarness {
    async fn start(probes: Vec<ReadinessProbe>) -> Self {
        let root = create_private_dir("fleetworks-http-parity");
        let surreal_dir = root.join("surreal");
        let jobs_dir = root.join("jobs");
        let search_dir = root.join("search");
        let datasets_dir = root.join("datasets");
        let staged_dir = datasets_dir.join("staged");
        std::fs::create_dir_all(&jobs_dir).expect("jobs dir");
        std::fs::create_dir_all(&datasets_dir).expect("datasets dir");

        let repo = Repository::connect(
            &Target::Embedded { path: surreal_dir },
            server::NS,
            server::DB,
        )
        .await
        .expect("connect embedded surreal");
        bootstrap::ensure_network_identity(repo.db())
            .await
            .expect("ensure network identity");
        connect_registry::initialize_control(repo.db())
            .await
            .expect("initialize connect control");
        let deployment = connect_registry::deployment_state(&repo)
            .await
            .expect("read deployment state");

        let jobs_store = fleetworks_jobs::Store::open(jobs_dir.join("jobs.db"))
            .await
            .expect("open jobs sqlite");
        let jobs_queue = Arc::new(
            fleetworks_jobs::queue::Queue::open(jobs_dir.join("queue"))
                .await
                .expect("open jobs queue"),
        );
        jobs_client::install_local(jobs_store).expect("install local jobs store");
        jobs_client::install_queue((*jobs_queue).clone()).expect("install local jobs queue");

        let engine_config = LocalEngineConfig::for_identity(
            search_dir,
            deployment.deployment_id.clone(),
            "local-source-1".to_owned(),
            ActorIndexLimits::default(),
        )
        .expect("search config");
        let engine = Engine::open(engine_config.engine.clone()).expect("open tantivy engine");
        let engine_slot = Arc::new(EngineSlot::new(Some(Arc::new(engine))));
        let search_service = Arc::new(
            SearchService::new(
                engine_config,
                Arc::clone(&engine_slot),
                "actor_index_unavailable",
            )
            .expect("start local search service"),
        );

        let dataset_store = DatasetStore::open(staged_dir).expect("open dataset store");
        let readiness = ReadinessOwners::new(deployment.deployment_id.clone(), probes)
            .expect("readiness owners");
        let services = PrivateServices::default()
            .with_search(SearchClient::from_local(Arc::new(LocalActorSearchBackend(
                Arc::clone(&search_service),
            ))))
            .with_workstation(readiness, dataset_store.clone())
            .with_activation_artifacts(
                ActivationArtifacts::new(root.clone()).expect("historical artifact owner"),
            );

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback HTTP parity port");
        let addr: SocketAddr = listener.local_addr().expect("listener addr");
        let port = addr.port();
        let origin = format!("http://127.0.0.1:{port}");

        let permissions = BTreeSet::from([
            Permission::ReadBackend,
            Permission::RegisterWorkstation,
            Permission::RebindWorkstation,
            Permission::ManageConfiguration,
            Permission::ManageInboxQa,
            Permission::MaintainBackend,
            Permission::SubmitInteractive,
            Permission::ReadWorkstationActivity,
            Permission::RetryJob,
            Permission::CancelJob,
        ]);
        let credential = ScopedCredential::new(
            "parity-http-native-cred".to_owned(),
            deployment.deployment_id,
            format!("epoch-{}", deployment.configuration_epoch),
            CredentialPurpose::Native,
            permissions,
            TOKEN,
        )
        .expect("build HTTP native credential");
        let policy = NetworkPolicy::with_credentials(
            &[format!("127.0.0.1:{port}"), format!("localhost:{port}")],
            vec![credential],
        )
        .expect("build network policy");

        let state = ApiState::new(
            repo.clone(),
            "http-parity-host-1".to_owned(),
            port,
            Arc::new(Vec::new()),
        )
        .with_workspace_root(root.clone());
        let router = network_router_with_private_services(state, policy, services);

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let server_task = tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("axum serve parity");
        });

        let (stop_tx, stop_rx) = watch::channel(false);
        let index_worker =
            ActorIndexWorker::local(Arc::clone(&search_service), "parity-http-index".to_owned())
                .expect("actor index worker");
        let worker_config = LocalWorkerConfig {
            broker: queue_client::Client::local(
                Arc::clone(&jobs_queue),
                "fleetworks.native".into(),
            ),
            store: dataset_store,
            index: Some(index_worker),
            relinquish_on_shutdown: false,
        };
        let worker_repo = repo.clone();
        let worker_root = root.clone();
        let worker_task = tokio::spawn(async move {
            let mut rx = stop_rx;
            let _ = native_worker::run_with_config(
                worker_repo,
                worker_root,
                worker_config,
                async move {
                    if !*rx.borrow() {
                        let _ = rx.changed().await;
                    }
                },
            )
            .await;
        });

        Self {
            root,
            origin,
            repo,
            stop_tx,
            server_shutdown: Some(shutdown_tx),
            server_task,
            worker_task,
        }
    }

    async fn shutdown(mut self) {
        let _ = self.stop_tx.send(true);
        if let Some(tx) = self.server_shutdown.take() {
            let _ = tx.send(());
        }
        let _ = self.worker_task.await;
        let _ = self.server_task.await;
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

struct SharedFlowResult {
    deployment_id: String,
    registered_context: RegisteredContext,
    seed_job_id: String,
    seed_request_id: String,
    canonical_actor_id: String,
}

async fn run_shared_flow(
    base_client: &Client,
    unauthenticated_http_client: Option<&Client>,
    csv_path: &Path,
    transport: &str,
    expected_candidate: Option<&DeclaredAtlasBertCandidate>,
    report: &mut ParityReport,
) -> SharedFlowResult {
    let record = |report: &mut ParityReport, op: &str, category: &str, summary: &str| {
        if transport == "http" {
            report.record_http(op, category, summary);
        } else {
            report.record_ipc(op, category, summary);
        }
    };

    let ready = base_client.ready().await.expect("ready parity");
    assert!(ready.ready && ready.storage);
    assert_eq!(ready.api_version, 1);
    record(
        &mut *report,
        "ready",
        "live_parity",
        "health/ready contract parity",
    );

    let caps: Capabilities = base_client
        .get(CAPABILITIES_PATH)
        .await
        .expect("capabilities parity");
    caps.require_native_contract()
        .expect("capabilities satisfy native contract");
    assert!(caps.features.iter().any(|f| f == "actor-discovery"));
    assert!(caps.features.iter().any(|f| f == "workstation-readiness"));
    assert!(caps.features.iter().any(|f| f == "inbox-qa-v2"));
    assert!(caps.features.iter().any(|f| f == "jobs"));
    assert!(caps.features.iter().any(|f| f == "logs"));
    let local = transport == "ipc";
    let local_nlp = local && cfg!(feature = "nlp");
    assert_eq!(caps.features.iter().any(|f| f == "nlp"), local_nlp);
    assert!(!caps.features.iter().any(|f| f == "classifier"));
    record(
        &mut *report,
        "capabilities",
        "intentional_exception",
        "shared native contract and transport-specific optional engines",
    );

    if let Some(unauth) = unauthenticated_http_client {
        assert_eq!(
            unauth.get::<Jobs>(JOBS_PATH).await.unwrap_err(),
            ClientError::Status(401),
            "HTTP ingress rejects missing bearer token with 401"
        );
    }
    assert_eq!(
        base_client.get::<Jobs>(JOBS_PATH).await.unwrap_err(),
        ClientError::Status(401),
        "{transport}: protected route rejects unregistered context with 401"
    );

    let metrics: serde_json::Value = base_client
        .get(HOST_METRICS_PATH)
        .await
        .expect("host metrics");
    assert!(
        metrics.is_object(),
        "{transport}: host_metrics returns object"
    );
    record(
        &mut *report,
        "host_metrics",
        "live_parity",
        "host metrics read",
    );

    let tools: serde_json::Value = base_client.get(TOOLS_PATH).await.expect("tools list");
    assert!(tools.is_object(), "{transport}: tools list returns object");
    record(&mut *report, "tools_list", "live_parity", "tools list read");

    // Pre-registration ensure + idempotent replay
    let ensure_req = RegistrationEnsureRequest::new(
        format!("parity-ensure-{transport}-1"),
        None,
        format!("install-{transport}-0001"),
    );
    let receipt: RegistrationReceipt = base_client
        .post(REGISTRATION_ENSURE_PATH, &ensure_req)
        .await
        .expect("registration ensure");
    assert!(receipt.is_consistent());
    let replayed_receipt: RegistrationReceipt = base_client
        .post(REGISTRATION_ENSURE_PATH, &ensure_req)
        .await
        .expect("registration ensure replay");
    assert_eq!(replayed_receipt.registration_id, receipt.registration_id);
    assert_eq!(replayed_receipt.revision, receipt.revision);
    record(
        &mut *report,
        "registration_ensure",
        "live_parity",
        "workstation registration creation and idempotent replay",
    );

    // Stale registration context revision -> 409 conflict on both transports.
    assert_eq!(
        base_client
            .post::<serde_json::Value>(
                REGISTRATION_CONTEXT_PATH,
                &json!({
                    "registration_id": receipt.registration_id,
                    "expected_revision": receipt.revision + 99,
                }),
            )
            .await
            .unwrap_err(),
        ClientError::Status(409),
        "{transport}: stale expected_revision on registration context must return 409"
    );
    let context_body: RegistrationContextBody = base_client
        .post(
            REGISTRATION_CONTEXT_PATH,
            &json!({
                "registration_id": receipt.registration_id,
                "expected_revision": receipt.revision,
            }),
        )
        .await
        .expect("registration context");
    let registered_context = context_body.context;
    record(
        &mut *report,
        "registration_context",
        "live_parity",
        "registration context resolution and stale-revision 409 parity",
    );

    let client = base_client
        .clone()
        .with_registered_context(&registered_context)
        .expect("attach registered context");

    // Stale-revision 409 conflicts on protected registration / profile / models routes.
    assert_eq!(
        client
            .post::<serde_json::Value>(
                REGISTRATION_REBIND_PATH,
                &json!({
                    "request_id": format!("rebind-stale-{transport}"),
                    "registration_id": registered_context.registration_id,
                    "expected_revision": receipt.revision + 99,
                    "installation_marker": format!("install-{transport}-0001"),
                }),
            )
            .await
            .unwrap_err(),
        ClientError::Status(409),
        "{transport}: stale registration_rebind returns 409"
    );
    record(
        &mut *report,
        "registration_rebind",
        "live_parity",
        "stale-revision 409 conflict parity on registration rebind",
    );

    assert_eq!(
        client
            .post::<serde_json::Value>(
                REGISTRATION_RETIRE_PATH,
                &json!({
                    "registration_id": registered_context.registration_id,
                    "expected_revision": receipt.revision + 99,
                }),
            )
            .await
            .unwrap_err(),
        ClientError::Status(409),
        "{transport}: stale registration_retire returns 409"
    );
    record(
        &mut *report,
        "registration_retire",
        "live_parity",
        "stale-revision 409 conflict parity on registration retire",
    );

    assert_eq!(
        client
            .post::<serde_json::Value>(
                WEB_SESSION_CREATE_PATH,
                &json!({ "connect_id": registered_context.connect_id.as_str() }),
            )
            .await
            .unwrap_err(),
        ClientError::Status(403),
        "{transport}: native credential without ResolveWebSession cannot mint web relay session"
    );
    record(
        &mut *report,
        "web_session_create",
        "live_parity",
        "purpose-scoped 403 boundary parity on web session creation",
    );

    let defaults_view: ConfigurationStatusView = client
        .get(MODELS_DEFAULTS_PATH)
        .await
        .expect("models defaults");
    record(
        &mut *report,
        "models_defaults",
        "live_parity",
        "models/defaults revision read",
    );
    assert_eq!(
        client
            .post::<serde_json::Value>(
                MODELS_DEFAULTS_SET_PATH,
                &json!({
                    "expected_revision": defaults_view.configuration_revision + 99,
                    "role": "assistant",
                    "primary": serde_json::Value::Null,
                    "fallbacks": [],
                    "variant_updates": {},
                }),
            )
            .await
            .unwrap_err(),
        ClientError::Status(409),
        "{transport}: stale models/defaults/set returns 409"
    );
    record(
        &mut *report,
        "model_role_set",
        "live_parity",
        "stale-revision 409 conflict parity on models/defaults/set",
    );

    let profile_view: ConfigurationStatusView =
        client.get(PROFILE_PATH).await.expect("profile read");
    record(&mut *report, "profile_read", "live_parity", "profile read");

    assert_eq!(
        client
            .post::<serde_json::Value>(
                PROFILE_ACCOUNTS_UPSERT_PATH,
                &json!({
                    "expected_revision": profile_view.configuration_revision + 99,
                    "account_id": "acct-stale",
                    "provider": "openai",
                    "base_url": "https://example.invalid/v1",
                }),
            )
            .await
            .unwrap_err(),
        ClientError::Status(409),
        "{transport}: stale profile account upsert returns 409"
    );
    record(
        &mut *report,
        "profile_account_upsert",
        "live_parity",
        "stale-revision 409 conflict parity on profile account upsert",
    );

    assert_eq!(
        client
            .post::<serde_json::Value>(
                PROFILE_ACCOUNTS_DELETE_PATH,
                &json!({
                    "expected_revision": profile_view.configuration_revision + 99,
                    "account_id": "acct-stale",
                }),
            )
            .await
            .unwrap_err(),
        ClientError::Status(409),
        "{transport}: stale profile account delete returns 409"
    );
    record(
        &mut *report,
        "profile_account_delete",
        "live_parity",
        "stale-revision 409 conflict parity on profile account delete",
    );

    assert_eq!(
        client
            .post::<serde_json::Value>(
                PROFILE_TOOLS_SET_PATH,
                &json!({
                    "expected_revision": profile_view.configuration_revision + 99,
                    "provider": "firecrawl",
                }),
            )
            .await
            .unwrap_err(),
        ClientError::Status(409),
        "{transport}: stale profile tools set returns 409"
    );
    record(
        &mut *report,
        "profile_tools_set",
        "live_parity",
        "stale-revision 409 conflict parity on profile tools set",
    );

    let exported = client
        .export_configuration(profile_view.configuration_revision)
        .await
        .expect("export configuration");
    assert!(!exported.is_empty());
    record(
        &mut *report,
        "configuration_export",
        "live_parity",
        "configuration export roundtrip",
    );
    let import_receipt = client
        .import_configuration(profile_view.configuration_revision, exported.clone())
        .await
        .expect("import configuration");
    assert_eq!(
        import_receipt.configuration_revision,
        profile_view.configuration_revision + 1
    );
    assert_eq!(
        client
            .import_configuration(profile_view.configuration_revision, exported)
            .await
            .unwrap_err(),
        ClientError::Status(409),
        "{transport}: replaying old expected_revision after import returns 409"
    );
    record(
        &mut *report,
        "configuration_import",
        "live_parity",
        "configuration import revision bump and stale-revision 409 parity",
    );

    // Pre-seed reindex fence -> 409 Conflict parity before seeding actors.
    let stale_mutation = MutationContext::new(&registered_context)
        .with_configuration_revision(import_receipt.configuration_revision);
    assert_eq!(
        client
            .reindex_actors(
                &MaintenanceRequest::for_reindex(
                    format!("parity-preseed-reindex-{transport}"),
                    stale_mutation.clone(),
                )
                .expect("build reindex"),
            )
            .await
            .unwrap_err(),
        ClientError::Status(409),
        "{transport}: reindex before actor seed must return 409"
    );

    // Upload CSV dataset & inspect datasets list.
    let csv_bytes = std::fs::read(csv_path).expect("read fixture csv");
    let csv_sha256 = format!("{:x}", Sha256::digest(&csv_bytes));
    let csv_file = tokio::fs::File::open(csv_path)
        .await
        .expect("open fixture csv");
    let uploaded = client
        .upload_dataset(
            DatasetKind::ActorCatalog,
            &csv_sha256,
            csv_file,
            csv_bytes.len() as u64,
        )
        .await
        .expect("upload actor catalog dataset");
    record(
        &mut *report,
        "dataset_upload_start",
        "live_parity",
        "dataset upload start ticket parity",
    );
    record(
        &mut *report,
        "dataset_upload_chunk",
        "live_parity",
        "dataset upload chunk/stream transfer parity",
    );
    record(
        &mut *report,
        "dataset_upload_finish",
        "live_parity",
        "dataset upload commit and SHA-256 verification parity",
    );

    let datasets: serde_json::Value = client
        .get(WORKSTATION_DATASETS_PATH)
        .await
        .expect("datasets list");
    assert!(datasets.is_object() || datasets.is_array());
    record(
        &mut *report,
        "datasets_list",
        "live_parity",
        "datasets listing after upload",
    );

    // Seed actors & replay exact same request_id to assert lost-response job_id preservation.
    let seed_request_id = format!("parity-seed-{transport}-1");
    let seed_request = MaintenanceRequest::for_seed(
        seed_request_id.clone(),
        stale_mutation.clone(),
        uploaded.dataset.clone(),
    )
    .expect("build seed request");
    let seed_receipt: MaintenanceReceipt = client
        .post(ACTOR_SEED_PATH, &seed_request)
        .await
        .expect("submit seed actors");
    assert_eq!(seed_receipt.operation, MaintenanceOperation::SeedActors);
    assert!(!seed_receipt.reused);

    let seed_replay: MaintenanceReceipt = client
        .post(ACTOR_SEED_PATH, &seed_request)
        .await
        .expect("replay seed actors");
    assert_eq!(seed_replay.job_id, seed_receipt.job_id);
    assert_eq!(seed_replay, seed_receipt);
    record(
        &mut *report,
        "seed_actors",
        "live_parity",
        "seed submission, execution, and lost-response replay preserving exact job_id and receipt",
    );

    // Request reconciliation lookup matches exact job_id.
    let req_assoc = client
        .request_detail(&WorkstationRequestDetailQuery::new(
            seed_request_id.clone(),
            "actor_seed".to_owned(),
            None,
        ))
        .await
        .expect("workstation request detail");
    assert_eq!(req_assoc.job_id, seed_receipt.job_id);
    record(
        &mut *report,
        "workstation_request_detail",
        "live_parity",
        "request_id -> job_id reconciliation parity",
    );

    // Wait for seed completion in Jobs + JobDetail + Logs.
    let seed_deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let jobs: Jobs = client.get(JOBS_PATH).await.expect("list jobs");
        let matches: Vec<_> = jobs
            .rows
            .iter()
            .filter(|row| row.id == seed_receipt.job_id)
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "{transport}: replayed seed must produce exactly one job row"
        );
        if matches[0].state == "completed" {
            break;
        }
        assert!(
            tokio::time::Instant::now() < seed_deadline,
            "{transport}: timed out waiting for seed job {} completion",
            seed_receipt.job_id
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    record(
        &mut *report,
        "jobs_list",
        "live_parity",
        "jobs list visibility and zero-duplicate replay guarantee",
    );

    let job_detail: serde_json::Value = client
        .get_with_query(JOB_DETAIL_PATH, &[("id", seed_receipt.job_id.as_str())])
        .await
        .expect("job detail");
    assert_eq!(job_detail["job"]["id"], seed_receipt.job_id);
    record(
        &mut *report,
        "job_detail",
        "live_parity",
        "job detail visibility parity",
    );

    let logs_page: serde_json::Value = client
        .get_with_query(LOGS_PATH, &LogQuery::default())
        .await
        .expect("logs page");
    assert!(logs_page["rows"].is_array());
    record(
        &mut *report,
        "logs_page",
        "live_parity",
        "logs page read parity",
    );

    let job_logs: serde_json::Value = client
        .get_with_query(
            LOGS_PATH,
            &json!({"id": seed_receipt.job_id.as_str(), "limit": 20}),
        )
        .await
        .expect("job logs");
    assert!(job_logs["rows"].is_array());
    record(
        &mut *report,
        "job_logs",
        "live_parity",
        "job-filtered logs visibility parity",
    );

    let exported_logs = client
        .export_logs(&LogQuery::default())
        .await
        .expect("export logs");
    assert!(exported_logs.apps_complete);
    record(
        &mut *report,
        "logs_export",
        "live_parity",
        "logs export parity",
    );

    // Reindex actors & await search hit.
    let reindex_req = MaintenanceRequest::for_reindex(
        format!("parity-reindex-{transport}-1"),
        stale_mutation.clone(),
    )
    .expect("build reindex request");
    let admission_deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let reindex_receipt = loop {
        match client.reindex_actors(&reindex_req).await {
            Ok(receipt) => break receipt,
            Err(ClientError::Status(422)) if transport == "ipc" => {
                assert!(
                    tokio::time::Instant::now() < admission_deadline,
                    "Embedded actor index observation did not settle before reindex admission"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(error) => panic!("{transport}: submit reindex actors: {error}"),
        }
    };
    assert_eq!(
        reindex_receipt.operation,
        MaintenanceOperation::ReindexActors
    );
    if reindex_receipt.already_completed {
        assert_eq!(transport, "ipc", "only Embedded reuses a serving index");
        assert!(reindex_receipt.reused && reindex_receipt.job_id.is_empty());
    } else {
        assert!(!reindex_receipt.job_id.is_empty());
    }
    let reindex_replay = client
        .reindex_actors(&reindex_req)
        .await
        .expect("replay reindex actors");
    assert_eq!(reindex_replay.job_id, reindex_receipt.job_id);
    assert_eq!(reindex_replay, reindex_receipt);
    record(
        &mut *report,
        "reindex_actors",
        "live_parity",
        "reindex admission and idempotent replay parity",
    );

    let search_deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let search_query = ActorSearchQuery::new("Harbor Beacon".to_owned(), 10).expect("search query");
    let search_resp = loop {
        let resp: ActorSearchResponse = client
            .get_with_query(ACTOR_SEARCH_PATH, &search_query)
            .await
            .expect("actor search");
        if resp.index.state == IndexState::Ready && !resp.rows.is_empty() {
            break resp;
        }
        assert!(
            tokio::time::Instant::now() < search_deadline,
            "{transport}: timed out waiting for canonical actor search result"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let canonical_actor_id = search_resp.rows[0].actor_id.clone();
    assert!(!canonical_actor_id.is_empty());
    assert!(search_resp.rows[0]
        .label
        .contains("Harbor Beacon Collective"));
    record(
        &mut *report,
        "actors_search",
        "live_parity",
        "canonical actor indexing and search visibility parity",
    );

    let actor_detail: serde_json::Value = client
        .get_with_query(ACTOR_DETAIL_PATH, &[("id", canonical_actor_id.as_str())])
        .await
        .expect("actor detail");
    assert_eq!(actor_detail["id"], canonical_actor_id);
    assert_eq!(actor_detail["display_name"], "Harbor Beacon Collective");
    record(
        &mut *report,
        "actor_detail",
        "live_parity",
        "actor detail parity",
    );

    let readiness: WorkstationReadiness = client
        .workstation_readiness()
        .await
        .expect("workstation readiness");
    assert_eq!(readiness.deployment_id, registered_context.deployment_id);
    assert_eq!(
        readiness.declared_atlas_bert_candidate.as_ref(),
        expected_candidate,
        "{transport}: operator and registered-client candidate revision must agree"
    );
    assert!(readiness.checks.iter().any(|check| {
        check.id == ReadinessCheckId::ClassifierInference && check.state != ReadinessState::Ready
    }));
    if local {
        assert!(readiness.atlas_preflight.is_none());
    }

    assert!(readiness
        .checks
        .iter()
        .any(|c| c.id == ReadinessCheckId::ActorCatalog && c.state == ReadinessState::Ready));
    assert_eq!(
        readiness
            .checks
            .iter()
            .any(|check| check.id == ReadinessCheckId::Nlp && check.state == ReadinessState::Ready),
        local_nlp,
        "{transport}: readiness must reflect the bundled NLP startup probe"
    );
    assert!(readiness.checks.iter().any(|check| {
        check.id == ReadinessCheckId::ClassifierTraining
            && check.state == ReadinessState::NotApplicable
            && check.actions.is_empty()
    }));
    let unregistered = base_client
        .workstation_readiness()
        .await
        .expect("unregistered workstation may read general diagnostics");
    assert!(
        unregistered.declared_atlas_bert_candidate.is_none(),
        "{transport}: a registered Native context is required for candidate readback"
    );

    record(
        &mut *report,
        "workstation_readiness",
        "live_parity",
        "post-seed/index readiness and declared Atlas BERT candidate revision parity",
    );

    // Articles & cycle history & negative lifecycle parity checks.
    let articles = client
        .get_with_query::<serde_json::Value>(
            ARTICLES_PATH,
            &json!({
                "day": "2026-11-15",
                "category": "all",
                "search": "",
                "sort": "latest",
                "limit": 20,
            }),
        )
        .await
        .expect("articles list");
    assert!(articles.is_object() || articles.is_array());
    record(
        &mut *report,
        "articles_list",
        "live_parity",
        "articles read parity",
    );

    let found = client
        .get_with_query::<serde_json::Value>(
            ARTICLES_SEARCH_PATH,
            &json!({"query": "harbor", "limit": 5}),
        )
        .await
        .expect("articles search");
    assert!(
        found["hits"].is_array(),
        "{transport}: search page has hits"
    );
    assert_eq!(found["truncated"], json!(false));
    assert_eq!(
        client
            .get_with_query::<serde_json::Value>(ARTICLES_SEARCH_PATH, &json!({"query": "  "}))
            .await
            .unwrap_err(),
        ClientError::Status(400),
        "{transport}: blank search returns 400"
    );
    record(
        &mut *report,
        "articles_search",
        "live_parity",
        "indexed article search page and blank-query 400 parity",
    );

    let cycles = client
        .cycle_history_page(10, None)
        .await
        .expect("cycle history");
    assert!(cycles.rows.is_empty());
    record(
        &mut *report,
        "cycle_history",
        "live_parity",
        "cycle history pagination parity",
    );

    assert_eq!(
        client
            .delete_cycle(&DeleteCycleRequest::new(
                format!("parity-del-missing-{transport}"),
                stale_mutation.clone(),
                "missing-cycle-job".to_owned(),
                1,
            ))
            .await
            .unwrap_err(),
        ClientError::Status(409),
        "{transport}: deleting nonexistent or stale-revision cycle returns 409"
    );
    record(
        &mut *report,
        "cycle_delete",
        "live_parity",
        "missing/stale cycle delete 409 parity",
    );

    let wrong_ctx = MutationContext::new(&registered_context)
        .with_configuration_revision(import_receipt.configuration_revision + 99);
    let atlas_req = AtlasCycleRequest::new(format!("parity_atlas_{transport}"))
        .expect("atlas req")
        .with_context(wrong_ctx);
    assert_eq!(
        client
            .post::<serde_json::Value>(ATLAS_CYCLES_PATH, &atlas_req)
            .await
            .unwrap_err(),
        ClientError::Status(409),
        "{transport}: stale configuration revision on atlas cycle returns 409"
    );
    record(
        &mut *report,
        "atlas_cycle_submit",
        "live_parity",
        "stale-revision 409 conflict parity on atlas cycle admission",
    );

    assert_eq!(
        client
            .post::<serde_json::Value>(
                TOOL_RUN_PATH,
                &ToolRunRequest::new(
                    format!("parity-tool-run-{transport}"),
                    "nonexistent-tool".to_owned(),
                    json!({}),
                )
                .with_context(stale_mutation.clone()),
            )
            .await
            .unwrap_err(),
        ClientError::Status(400),
        "{transport}: unknown tool_run returns 400"
    );
    record(
        &mut *report,
        "tool_run",
        "live_parity",
        "unknown tool_run 400 parity",
    );

    let chat_turn = |input: &str| {
        serde_json::json!({
            "request_id": format!("parity-chat-{transport}"),
            "context": MutationContext::new(&registered_context),
            "source": {"kind": "job", "id": "parity-chat-source"},
            "input": input,
        })
    };
    let ack: ChatTurnAck = client
        .post(CHAT_TURNS_PATH, &chat_turn("Is this reachable?"))
        .await
        .expect("chat turn acknowledged");
    assert!(ack.is_ok(), "{transport}: chat turn answers ok");
    assert_eq!(
        client
            .post::<serde_json::Value>(CHAT_TURNS_PATH, &chat_turn("   "))
            .await
            .unwrap_err(),
        ClientError::Status(400),
        "{transport}: empty chat turn returns 400"
    );
    record(
        &mut *report,
        "chat_turn_submit",
        "live_parity",
        "chat turn ok acknowledgement and empty-input 400 parity",
    );

    let qa_settings = client.qa_settings().await.expect("registered QA settings");
    assert!(
        !qa_settings.enabled,
        "{transport}: Inbox QA defaults to Off"
    );
    assert_eq!(qa_settings.phase, QaPhase::Off);
    assert!(qa_settings.can_manage);
    assert!(
        client
            .latest_qa_turn()
            .await
            .expect("latest Inbox exchange")
            .is_none(),
        "{transport}: a fresh registration has no accepted QA exchange",
    );
    let qa_context = MutationContext::new(&registered_context);
    assert_eq!(
        client
            .update_qa_settings(&QaSettingsUpdate::new(
                qa_context.clone(),
                qa_settings.revision + 1,
                true,
            ))
            .await
            .unwrap_err(),
        ClientError::Status(409),
    );
    let qa_turn = QaTurnRequest::new(
        format!("parity-inbox-qa-{transport}"),
        qa_context.clone(),
        "What is the state of the gateway?".to_owned(),
        None,
    )
    .expect("valid QA turn");
    assert_eq!(
        client.submit_qa_turn(&qa_turn).await.unwrap_err(),
        ClientError::Status(503),
    );
    let missing_turn_id = "missing-parity-turn";
    assert_eq!(
        client
            .qa_events(&QaEventQuery::new(missing_turn_id.to_owned(), 0, 0))
            .await
            .unwrap_err(),
        ClientError::Status(404),
    );
    assert_eq!(
        client
            .cancel_qa_turn(&QaCancelRequest::new(
                qa_context,
                missing_turn_id.to_owned()
            ))
            .await
            .unwrap_err(),
        ClientError::Status(404),
    );
    for (operation, summary) in [
        ("qa_settings", "registered backend defaults to Off"),
        ("qa_settings_update", "stale setting revision conflict"),
        ("qa_turn_submit", "disabled inference rejects turn"),
        ("qa_latest", "fresh registration has no latest exchange"),
        ("qa_events", "unknown turn rejects event read"),
        ("qa_cancel", "unknown turn rejects Stop"),
    ] {
        record(report, operation, "live_parity", summary);
    }

    assert_eq!(
        client
            .get_with_query::<serde_json::Value>(
                TOOL_RESULT_PATH,
                &[("id", "fleet-tool-missing-1")]
            )
            .await
            .unwrap_err(),
        ClientError::Status(404),
        "{transport}: missing tool_result returns 404"
    );
    record(
        &mut *report,
        "tool_result",
        "live_parity",
        "missing tool_result 404 parity",
    );

    let missing_job_action = json!({
        "id": "missing-job-1",
        "request_id": format!("parity-job-act-{transport}"),
    });
    assert_eq!(
        client
            .post::<serde_json::Value>(JOB_RETRY_PATH, &missing_job_action)
            .await
            .unwrap_err(),
        ClientError::Status(404),
        "{transport}: missing job_retry returns 404"
    );
    record(
        &mut *report,
        "job_retry",
        "live_parity",
        "missing job_retry 404 parity",
    );

    assert_eq!(
        client
            .post::<serde_json::Value>(JOB_CANCEL_PATH, &missing_job_action)
            .await
            .unwrap_err(),
        ClientError::Status(404),
        "{transport}: missing job_cancel returns 404"
    );
    record(
        &mut *report,
        "job_cancel",
        "live_parity",
        "missing job_cancel 404 parity",
    );

    // Both transports retire Natural writes and retain protected historical reads.
    let candidate = CandidateIdentity::new(
        "candidate-1".to_owned(),
        "train-job-1".to_owned(),
        "a".repeat(64),
        "b".repeat(64),
        "c".repeat(64),
        "d".repeat(64),
    )
    .expect("candidate id");
    let activate_req = ActivationRequest::new(
        format!("parity-activate-{transport}"),
        stale_mutation.clone(),
        candidate,
        ActiveModelRevision::NeverActivated,
        SyntheticBaselineConsent::NotGranted,
    )
    .expect("activation request");
    assert_eq!(
        client
            .post::<serde_json::Value>(ACTIVATION_PATH, &activate_req)
            .await
            .unwrap_err(),
        ClientError::Status(410),
        "{transport}: Natural activation is retired"
    );
    record(
        &mut *report,
        "classifier_activate",
        "live_parity",
        "legacy activation returns 410",
    );

    assert_eq!(
        client
            .get_with_query::<serde_json::Value>(
                CANDIDATE_DETAIL_PATH,
                &json!({"training_job_id": "train-job-1", "model_id": "candidate-1"}),
            )
            .await
            .unwrap_err(),
        ClientError::Status(404),
        "{transport}: protected readback rejects an absent historical candidate"
    );
    record(
        &mut *report,
        "classifier_candidate_detail",
        "live_parity",
        "missing historical candidate returns 404",
    );

    let act_result_err = client
        .get_with_query::<serde_json::Value>(ACTIVATION_RESULT_PATH, &[("job_id", "missing-job")])
        .await
        .unwrap_err();
    assert_eq!(
        act_result_err,
        ClientError::Status(404),
        "{transport}: protected readback requires an existing activation job"
    );
    record(
        &mut *report,
        "classifier_activation_result",
        "live_parity",
        "missing historical activation job returns 404",
    );

    let train_dataset = DatasetRef::new(
        "missing-train-ds".to_owned(),
        DatasetKind::ClassifierTraining,
        "a".repeat(64),
        1024,
    )
    .expect("train dataset ref");
    let train_req = MaintenanceRequest::for_training(
        format!("parity-train-{transport}"),
        stale_mutation.clone(),
        train_dataset,
    )
    .expect("build train request");
    let train_err = client
        .post::<serde_json::Value>(CLASSIFIER_TRAIN_PATH, &train_req)
        .await
        .unwrap_err();
    assert_eq!(
        train_err,
        ClientError::Status(410),
        "{transport}: Natural training is retired"
    );
    record(
        &mut *report,
        "train_classifier",
        "live_parity",
        "legacy training returns 410 before staging or training",
    );
    let logs_follow_op = IpcOperation::from_get_with_query(LOGS_FOLLOW_PATH, &LogQuery::default())
        .expect("logs/follow route mapping");
    assert_eq!(
        logs_follow_op.into_typed_route(),
        (IpcMethod::Get, LOGS_FOLLOW_PATH)
    );
    assert_eq!(
        client
            .get_with_query::<serde_json::Value>(LOGS_FOLLOW_PATH, &LogQuery::default())
            .await
            .unwrap_err(),
        ClientError::Status(400),
        "{transport}: /api/v1/logs/follow without cursor must return 400"
    );
    let fresh_logs: serde_json::Value = client
        .get_with_query(LOGS_PATH, &LogQuery::default())
        .await
        .expect("fresh logs page for follow_cursor");
    if let Some(follow_cursor) = fresh_logs["follow_cursor"].as_str() {
        let mut follow_query = LogQuery::default();
        follow_query.cursor = Some(follow_cursor.to_owned());
        follow_query.wait_ms = Some(1);
        let follow_resp: serde_json::Value = client
            .get_with_query(LOGS_FOLLOW_PATH, &follow_query)
            .await
            .expect("logs/follow with cursor");
        assert!(follow_resp["rows"].is_array());
    }
    record(
        &mut *report,
        "logs_follow",
        "live_parity",
        "bounded logs/follow cursor enforcement and tail snapshot parity",
    );

    SharedFlowResult {
        deployment_id: registered_context.deployment_id.clone(),
        registered_context,
        seed_job_id: seed_receipt.job_id,
        seed_request_id,
        canonical_actor_id,
    }
}

async fn verify_ipc_protocol_mismatch_exception(ipc_dir: &Path) {
    let (mut child, mut socket) =
        fleetworks_contracts::ipc::spawn_companion_socket(spawn_local_ipc_command(ipc_dir))
            .expect("spawn local-ipc for version mismatch check");

    let mut hello = IpcHello::new("parity-version-probe".to_owned(), "99.0.0".to_owned());
    hello.protocol_version = 99;
    hello.min_protocol_version = 99;
    let incompatible = IpcEnvelope::Hello(hello);
    write_frame(&mut socket, &incompatible)
        .await
        .expect("write incompatible hello");

    let ack_frame = read_frame(&mut socket)
        .await
        .expect("read HelloAck frame")
        .expect("non-empty HelloAck frame");
    match ack_frame {
        IpcEnvelope::HelloAck(ack) => {
            assert!(
                !ack.compatible,
                "incompatible protocol range must receive compatible=false HelloAck"
            );
            assert!(ack.capabilities.is_none());
        }
        other => panic!("expected HelloAck, got {other:?}"),
    }
    let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .expect("child exits cleanly after incompatible HelloAck")
        .expect("wait status");
    assert!(
        status.success() || status.code().is_some(),
        "incompatible handshake child exited abnormally: {status:?}"
    );
}

#[tokio::test]
async fn http_and_local_ipc_backends_satisfy_parity_and_intentional_exceptions() {
    let mut report = ParityReport::new();

    // Create deterministic actor catalog CSV fixture.
    let fixture_dir = create_private_dir("fleetworks-parity-fixtures");
    let csv_path = fixture_dir.join("actors.csv");
    std::fs::write(
        &csv_path,
        b"actor_id,actor_name,actor_type,aliases,name_normalized\n\
          actor-harbor-1,Harbor Beacon Collective,organization,[],harbor beacon collective\n",
    )
    .expect("write fixture csv");

    // 1. Run full shared scenario suite against real Axum HTTP gateway + LocalNativeWorker.
    #[cfg(all(feature = "classifier", unix))]
    let classifier = Some(declared_classifier_fixture::start().await);
    #[cfg(not(all(feature = "classifier", unix)))]
    let classifier: Option<DeclaredClassifierHarness> = None;
    let expected_candidate = classifier.as_ref().map(|owner| &owner.expected);
    let probes = classifier.iter().map(|owner| owner.probe.clone()).collect();
    let http = HttpHarness::start(probes).await;
    let http_client = Client::new(&http.origin, Some(TOKEN)).expect("HTTP native client");
    let http_unauth = Client::new(&http.origin, None).expect("HTTP unauth client");
    let http_res = run_shared_flow(
        &http_client,
        Some(&http_unauth),
        &csv_path,
        "http",
        expected_candidate,
        &mut report,
    )
    .await;
    // Verify durable control/seed state persists on local disk in HTTP embedded repo.
    let http_seed_obs = core_setup_jobs::actor_seed_observation(&http.repo)
        .await
        .expect("observe durable HTTP actor seed receipt");
    assert!(
        http_seed_obs
            .as_ref()
            .is_some_and(|obs| obs.status == "complete"),
        "durable setup_jobs seed observation must persist in embedded SurrealDB"
    );
    http.shutdown().await;

    // 2. Run exact same shared scenario suite against compiled `argos-api-host --role local-ipc`.
    let ipc_root = classifier.as_ref().map_or_else(
        || create_private_dir("fleetworks-ipc-parity"),
        |owner| owner.home.path().to_owned(),
    );
    let cmd1 = spawn_local_ipc_command(&ipc_root);
    let ipc1 = Arc::new(
        LocalIpcClient::spawn_command(cmd1, "http-ipc-parity", "1.0.0")
            .await
            .expect("spawn argos-api-host --role local-ipc session 1"),
    );
    let ipc_client1 = Client::from_local_ipc(Arc::clone(&ipc1));
    let ipc_res = run_shared_flow(
        &ipc_client1,
        None,
        &csv_path,
        "ipc",
        expected_candidate,
        &mut report,
    )
    .await;

    // Graceful IPC shutdown relinquishes leases and checkpoints local durable stores.
    drop(ipc_client1);
    let shutdown_ack = ipc1
        .shutdown("parity_restart_recovery")
        .await
        .expect("IPC graceful shutdown ack");
    assert!(shutdown_ack.checkpointed);

    // Check protocol mismatch intentional exception on fresh non-destructive handshake probe.
    let mismatch_dir = create_private_dir("fleetworks-ipc-mismatch");
    verify_ipc_protocol_mismatch_exception(&mismatch_dir).await;
    let _ = std::fs::remove_dir_all(&mismatch_dir);

    // 3. Restart real `argos-api-host --role local-ipc` child on SAME state directory (no network storage)
    // and verify durable state, request_id -> job_id preservation, replay idempotency, and search visibility.
    let cmd2 = spawn_local_ipc_command(&ipc_root);
    let ipc2 = Arc::new(
        LocalIpcClient::spawn_command(cmd2, "http-ipc-parity-restart", "1.0.0")
            .await
            .expect("spawn argos-api-host --role local-ipc session 2"),
    );
    let hello_caps2 = ipc2.capabilities().expect("HelloAck 2 capabilities");
    assert_eq!(
        hello_caps2.deployment_id.as_deref(),
        Some(ipc_res.deployment_id.as_str()),
        "deployment_id must persist on local disk across IPC restarts"
    );
    let ipc_client2 = Client::from_local_ipc(Arc::clone(&ipc2))
        .with_registered_context(&ipc_res.registered_context)
        .expect("attach saved registered context after restart");

    let restarted_readiness = ipc_client2
        .workstation_readiness()
        .await
        .expect("workstation readiness across companion restart");
    assert_eq!(
        restarted_readiness.declared_atlas_bert_candidate.as_ref(),
        expected_candidate
    );
    assert!(restarted_readiness.atlas_preflight.is_none());

    let assoc_after_restart = ipc_client2
        .request_detail(&WorkstationRequestDetailQuery::new(
            ipc_res.seed_request_id.clone(),
            "actor_seed".to_owned(),
            None,
        ))
        .await
        .expect("request detail after IPC restart");
    assert_eq!(assoc_after_restart.job_id, ipc_res.seed_job_id);

    let jobs_after_restart: Jobs = ipc_client2
        .get(JOBS_PATH)
        .await
        .expect("jobs after restart");
    assert_eq!(
        jobs_after_restart
            .rows
            .iter()
            .filter(|row| row.id == ipc_res.seed_job_id)
            .count(),
        1,
        "durable seed job must survive IPC restart with no duplicates"
    );

    let restart_search_deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let search_after_restart = loop {
        let resp: ActorSearchResponse = ipc_client2
            .get_with_query(
                ACTOR_SEARCH_PATH,
                &ActorSearchQuery::new("Harbor Beacon".to_owned(), 10).expect("query"),
            )
            .await
            .expect("search after IPC restart");
        if resp.index.state == IndexState::Ready && !resp.rows.is_empty() {
            break resp;
        }
        assert!(
            tokio::time::Instant::now() < restart_search_deadline,
            "search index did not settle to Ready after IPC restart: state={:?} rows={}",
            resp.index.state,
            resp.rows.len()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(search_after_restart.index.state, IndexState::Ready);
    assert_eq!(
        search_after_restart.rows[0].actor_id,
        ipc_res.canonical_actor_id
    );
    assert_eq!(
        http_res.canonical_actor_id, ipc_res.canonical_actor_id,
        "HTTP and IPC must derive identical deterministic canonical actor IDs for the same CSV input"
    );

    // Record executable intentional-exception scenarios.
    report.record_exception(IntentionalExceptionRecord {
        id: "handshake_protocol_vs_http_auth".to_owned(),
        operations: vec!["ready".to_owned(), "capabilities".to_owned()],
        reason: "Local IPC negotiates frame protocol versions in IpcHello/IpcHelloAck, whereas HTTP authenticates per-request Bearer tokens via NetworkPolicy without a handshake frame."
            .to_owned(),
        http_behavior: "Unauthenticated GET /api/v1/jobs returns 401; ready/capabilities omit HelloAck framing."
            .to_owned(),
        ipc_behavior: "Incompatible IpcHello (v99) returns HelloAck { compatible: false, capabilities: None } and exits without opening authenticated dispatch."
            .to_owned(),
        verified: true,
    });
    report.record_exception(IntentionalExceptionRecord {
        id: "optional_nlp_capabilities".to_owned(),
        operations: vec![
            "capabilities".to_owned(),
            "workstation_readiness".to_owned(),
        ],
        reason: "Embedded IPC advertises bundled NLP after a startup probe. Distributed HTTP clients use readiness for independently probed services."
            .to_owned(),
        http_behavior: "The HTTP fixture has no private NLP probe and does not advertise bundled NLP."
            .to_owned(),
        ipc_behavior: "Enabled IPC builds report NLP Ready after the bundled probe."
            .to_owned(),
        verified: true,
    });
    report.record_exception(IntentionalExceptionRecord {
        id: "local_worker_shutdown_lease_relinquishment".to_owned(),
        operations: vec!["web_session_create".to_owned()],
        reason: "Local IPC workers configure relinquish_on_shutdown=true and emit IpcShutdownAck { checkpointed: true } on graceful exit, whereas HTTP servers keep in-flight leases until server stop and rely on WebRelay/Native role gating for web sessions."
            .to_owned(),
        http_behavior: "HTTP enforces WebRelay role gating on web_session_create (403 for Native) and shuts down the listener without IpcShutdownAck."
            .to_owned(),
        ipc_behavior: "Local IPC rejects web_session_create with 403 and emits IpcShutdownAck { checkpointed: true } while relinquishing local leases across restarts."
            .to_owned(),
        verified: true,
    });

    drop(ipc_client2);
    let shutdown_ack2 = ipc2.shutdown("parity_complete").await.expect("shutdown 2");
    assert!(shutdown_ack2.checkpointed);

    report.finalize_and_assert_complete();
    let json_report = serde_json::to_string_pretty(&report).expect("serialize parity report");
    if let Ok(out_path) = std::env::var("FLEETWORKS_PARITY_REPORT_PATH") {
        let out = Path::new(&out_path);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).expect("create parity report dir");
        }
        std::fs::write(out, format!("{json_report}\n")).expect("write parity report");
    }
    println!(
        "FLEETWORKS_PARITY_REPORT_SUMMARY covered={}/{} exceptions={}",
        report.covered_ipc_operations,
        report.total_ipc_operations,
        report.intentional_exceptions.len()
    );

    assert!(!http_res.seed_job_id.is_empty());

    let _ = std::fs::remove_dir_all(&ipc_root);
    let _ = std::fs::remove_dir_all(&fixture_dir);
}
