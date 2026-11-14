use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use fleetworks_contracts::capabilities::ConnectCapabilities;
use fleetworks_contracts::capabilities::ResourceScopes;
use fleetworks_contracts::capabilities::WorkspaceControlCapabilities;
use fleetworks_contracts::capabilities::CONNECT_REGISTRY_FEATURE;
use fleetworks_contracts::pairing::PAIRING_POLL_PATH;
use fleetworks_contracts::pairing::PAIRING_START_PATH;
use fleetworks_contracts::CAPABILITIES_PATH;
use fleetworks_contracts::READY_PATH;
use fleetworks_workstation::native::keychain::KeychainItem;
use fleetworks_workstation::native::PAIRING_FILE;
use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::task::JoinHandle;

use super::*;

const ACCESS: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

#[derive(Default)]
struct StoreState {
    items: BTreeMap<(String, String), SecretToken>,
    locked: bool,
    fail_access_write: bool,
    access_reads: usize,
}

#[derive(Default)]
struct FakeKeychain(Mutex<StoreState>);

impl KeychainStore for FakeKeychain {
    fn get(&self, item: &KeychainItem) -> Result<SecretToken, KeychainError> {
        let mut state = self.0.lock().expect("isolated fake Keychain");
        if state.locked {
            return Err(KeychainError::LockedDenied);
        }
        let token = state
            .items
            .get(&(item.service().into(), item.account().into()))
            .cloned()
            .ok_or(KeychainError::Missing)?;
        if token.expose_secret() == ACCESS {
            state.access_reads += 1;
        }
        Ok(token)
    }

    fn set(&self, item: &KeychainItem, token: &SecretToken) -> Result<(), KeychainError> {
        let mut state = self.0.lock().expect("isolated fake Keychain");
        if state.locked {
            return Err(KeychainError::LockedDenied);
        }
        if state.fail_access_write && token.expose_secret() == ACCESS {
            return Err(KeychainError::Unavailable);
        }
        state.items.insert(
            (item.service().into(), item.account().into()),
            token.clone(),
        );
        Ok(())
    }

    fn delete(&self, item: &KeychainItem) -> Result<(), KeychainError> {
        let mut state = self.0.lock().expect("isolated fake Keychain");
        if state.locked {
            return Err(KeychainError::LockedDenied);
        }
        state
            .items
            .remove(&(item.service().into(), item.account().into()))
            .map(|_| ())
            .ok_or(KeychainError::Missing)
    }
}

struct Request {
    method: String,
    path: String,
    headers: String,
    body: Value,
}

struct Gateway {
    origin: String,
    requests: Arc<Mutex<Vec<Request>>>,
    deployment: Arc<Mutex<String>>,
    task: JoinHandle<()>,
}

impl Gateway {
    async fn start(lose_first_start: bool) -> Self {
        Self::start_with_failure(lose_first_start, None).await
    }

    async fn start_with_failure(
        lose_first_start: bool,
        failure: Option<(&'static str, &'static str)>,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}/", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let deployment = Arc::new(Mutex::new("synthetic-deployment".to_owned()));
        let recorded = requests.clone();
        let current_deployment = deployment.clone();
        let drop_start = AtomicBool::new(lose_first_start);
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_request(&mut socket).await;
                let response = match request.path.as_str() {
                    PAIRING_START_PATH => serde_json::json!({
                        "pairing_id": request.body["request_id"],
                        "approval_code": "b".repeat(32),
                        "expires_at_ms": 2_000_000_000_000_u64,
                        "poll_after_ms": 1000
                    }),
                    PAIRING_POLL_PATH => serde_json::json!({
                        "status": "approved",
                        "credential_id": "paired-synthetic-credential",
                        "access_token": ACCESS,
                        "scope_id": "synthetic-scope",
                        "access_epoch": "synthetic-access"
                    }),
                    READY_PATH => serde_json::json!({
                        "service": "fleetworks-api", "api_version": 1,
                        "ready": true, "storage": true, "instance_id": "fixture"
                    }),
                    CAPABILITIES_PATH => {
                        let deployment = current_deployment
                            .lock()
                            .expect("fixture deployment")
                            .clone();
                        serde_json::json!({
                            "api_version": 1, "auth": true,
                            "deployment_id": deployment,
                            "configuration_epoch": "fixture-config",
                            "features": [CONNECT_REGISTRY_FEATURE],
                            "connect": ConnectCapabilities::default(),
                            "workspace_control": WorkspaceControlCapabilities::default(),
                            "resource_scopes": ResourceScopes::default()
                        })
                    }
                    _ => panic!("native pairing used an unexpected route"),
                };
                let lost =
                    request.path == PAIRING_START_PATH && drop_start.swap(false, Ordering::SeqCst);
                let status = failure
                    .filter(|(path, _)| *path == request.path)
                    .map_or("200 OK", |(_, status)| status);
                recorded.lock().expect("fixture requests").push(request);
                if lost {
                    continue;
                }
                let body = serde_json::to_vec(&response).unwrap();
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(header.as_bytes()).await.unwrap();
                socket.write_all(&body).await.unwrap();
            }
        });
        Self {
            origin,
            requests,
            deployment,
            task,
        }
    }

    async fn finish(mut self) {
        self.task.abort();
        assert!((&mut self.task).await.unwrap_err().is_cancelled());
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn read_request(socket: &mut TcpStream) -> Request {
    let mut bytes = Vec::new();
    loop {
        let mut byte = [0];
        socket.read_exact(&mut byte).await.unwrap();
        bytes.push(byte[0]);
        assert!(bytes.len() <= 8192, "fixture request header is bounded");
        if bytes.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let headers = String::from_utf8(bytes).unwrap().to_ascii_lowercase();
    let mut first = headers.lines().next().unwrap().split_whitespace();
    let method = first.next().unwrap().to_owned();
    let path = first.next().unwrap().to_owned();
    let length = headers
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .map_or(0, |value| value.trim().parse::<usize>().unwrap());
    assert!(length <= 16 * 1024, "fixture body is bounded");
    let mut body = vec![0; length];
    socket.read_exact(&mut body).await.unwrap();
    let body = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap()
    };
    Request {
        method,
        path,
        headers,
        body,
    }
}

fn installation() -> (tempfile::TempDir, Arc<NativeContainer>, Arc<FakeKeychain>) {
    let temporary = tempfile::tempdir().expect("isolated synthetic home");
    let path = temporary.path().canonicalize().unwrap();
    let container = NativeContainer::open_settings(&path).expect("protected test installation");
    (
        temporary,
        Arc::new(container),
        Arc::new(FakeKeychain::default()),
    )
}

fn assert_wire_privacy(requests: &[Request]) {
    for request in requests {
        assert!(!request.path.contains('?'), "no secrets in pairing URLs");
        if matches!(
            request.path.as_str(),
            PAIRING_START_PATH | PAIRING_POLL_PATH
        ) {
            assert_eq!(request.method, "post");
            assert!(!request.headers.contains("authorization:"));
            let secret = request.body["polling_secret"].as_str().unwrap();
            assert_eq!(secret.len(), 64);
            assert!(!request.headers.contains(secret));
        } else {
            assert!(matches!(
                request.path.as_str(),
                READY_PATH | CAPABILITIES_PATH
            ));
            assert_eq!(request.method, "get");
            assert!(request
                .headers
                .contains(&format!("authorization: bearer {ACCESS}\r\n")));
            assert!(request.body.is_null());
        }
    }
}

#[tokio::test]
async fn gateway_failures_after_challenge_preserve_pending_pairing_and_truthful_recovery() {
    for (path, status, expected) in [
        (
            PAIRING_POLL_PATH,
            "503 Service Unavailable",
            PairingFailure::GatewayUnavailable,
        ),
        (
            PAIRING_POLL_PATH,
            "429 Too Many Requests",
            PairingFailure::GatewayUnavailable,
        ),
        (PAIRING_POLL_PATH, "410 Gone", PairingFailure::Expired),
        (
            PAIRING_POLL_PATH,
            "404 Not Found",
            PairingFailure::Unsupported,
        ),
        (
            READY_PATH,
            "503 Service Unavailable",
            PairingFailure::GatewayUnavailable,
        ),
        (
            CAPABILITIES_PATH,
            "503 Service Unavailable",
            PairingFailure::GatewayUnavailable,
        ),
    ] {
        let (_temporary, container, store) = installation();
        let gateway = Gateway::start_with_failure(false, Some((path, status))).await;
        let mut progress = Vec::new();
        let result = connect_or_pair(
            container.clone(),
            gateway.origin.clone(),
            store.clone(),
            false,
            |stage| progress.push(stage),
        )
        .await;
        assert!(
            matches!(result, Err(failure) if failure == expected),
            "{path}: {status}"
        );
        assert!(progress
            .iter()
            .any(|stage| matches!(stage, PairingStage::Waiting(_))));
        let catalog = container.pairing_catalog().unwrap();
        assert_eq!(catalog.pending_pairings.len(), 1);
        assert!(catalog.paired_gateways.is_empty());
        assert!(catalog.selected_origin.is_none());
        assert_eq!(store.0.lock().unwrap().access_reads, 0);
        assert_wire_privacy(&gateway.requests.lock().unwrap());
        gateway.finish().await;
    }
}

#[tokio::test]
async fn approves_authenticates_saves_then_reconnects_from_keychain_without_pairing() {
    let (temporary, container, store) = installation();
    let gateway = Gateway::start(false).await;
    let mut progress = Vec::new();
    let connected = connect_or_pair(
        container.clone(),
        gateway.origin.clone(),
        store.clone(),
        false,
        |stage| progress.push(stage),
    )
    .await
    .expect("approved and authenticated");
    assert_eq!(
        connected.capabilities.deployment_id.as_deref(),
        Some("synthetic-deployment")
    );
    assert!(progress
        .iter()
        .any(|stage| matches!(stage, PairingStage::Waiting(_))));
    let catalog = container.pairing_catalog().unwrap();
    assert_eq!(
        catalog.selected_origin.as_deref(),
        Some(gateway.origin.as_str())
    );
    assert_eq!(catalog.paired_gateways.len(), 1);
    assert!(catalog.pending_pairings.is_empty());
    let poll_secret = {
        let starts = gateway.requests.lock().unwrap();
        starts[0].body["polling_secret"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let bytes = std::fs::read_to_string(container.root().join(PAIRING_FILE)).unwrap();
    assert!(!bytes.contains(ACCESS));
    assert!(!bytes.contains(&poll_secret));
    assert!(!format!("{catalog:?}").contains(ACCESS));
    let reopened = Arc::new(
        NativeContainer::open_settings(&temporary.path().canonicalize().unwrap()).unwrap(),
    );
    let before = store.0.lock().unwrap().access_reads;
    connect_or_pair(
        reopened,
        gateway.origin.clone(),
        store.clone(),
        false,
        |_| {},
    )
    .await
    .expect("saved authenticated reconnect");
    assert!(store.0.lock().unwrap().access_reads > before);
    {
        let requests = gateway.requests.lock().unwrap();
        assert_eq!(requests.len(), 6);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == PAIRING_START_PATH)
                .count(),
            1
        );
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == PAIRING_POLL_PATH)
                .count(),
            1
        );
        assert_wire_privacy(&requests);
    }
    gateway.finish().await;
}

#[tokio::test]
async fn lost_start_response_replays_the_exact_durable_identity_and_secret_after_reopen() {
    let (temporary, container, store) = installation();
    let gateway = Gateway::start(true).await;
    let result = connect_or_pair(
        container.clone(),
        gateway.origin.clone(),
        store.clone(),
        false,
        |_| {},
    )
    .await;
    assert!(matches!(result, Err(PairingFailure::Network)));
    let pending = container.pairing_catalog().unwrap().pending_pairings;
    assert_eq!(pending.len(), 1);
    let reopened = Arc::new(
        NativeContainer::open_settings(&temporary.path().canonicalize().unwrap()).unwrap(),
    );
    connect_or_pair(
        reopened.clone(),
        gateway.origin.clone(),
        store,
        false,
        |_| {},
    )
    .await
    .expect("recovered approval");
    {
        let requests = gateway.requests.lock().unwrap();
        let starts: Vec<_> = requests
            .iter()
            .filter(|request| request.path == PAIRING_START_PATH)
            .collect();
        assert_eq!(starts.len(), 2);
        assert!(
            starts[0].body == starts[1].body,
            "recovery must reuse the exact ID, installation and secret"
        );
        assert!(starts[0].body["request_id"].as_str() == Some(pending[0].request_id.as_str()));
        assert_wire_privacy(&requests);
    }
    assert!(reopened
        .pairing_catalog()
        .unwrap()
        .pending_pairings
        .is_empty());
    gateway.finish().await;
}

#[tokio::test]
async fn locked_saved_keychain_access_fails_before_any_http_request() {
    let (_temporary, container, store) = installation();
    let gateway = Gateway::start(false).await;
    connect_or_pair(
        container.clone(),
        gateway.origin.clone(),
        store.clone(),
        false,
        |_| {},
    )
    .await
    .expect("initial pairing");
    gateway.requests.lock().unwrap().clear();
    store.0.lock().unwrap().locked = true;
    let result = connect_or_pair(container, gateway.origin.clone(), store, false, |_| {}).await;
    assert!(matches!(result, Err(PairingFailure::KeychainDenied)));
    assert!(gateway.requests.lock().unwrap().is_empty());
    gateway.finish().await;
}

#[tokio::test]
async fn failed_final_keychain_write_preserves_recovery_and_never_reports_connected() {
    let (temporary, container, store) = installation();
    let gateway = Gateway::start(false).await;
    store.0.lock().unwrap().fail_access_write = true;
    let result = connect_or_pair(
        container.clone(),
        gateway.origin.clone(),
        store.clone(),
        false,
        |_| {},
    )
    .await;
    assert!(matches!(&result, Err(PairingFailure::KeychainUnavailable)));
    let mut state = State::fixture();
    state.connection.status = ConnectionStatus::Connecting;
    state.complete_pairing_connection(state.connection.epoch, result);
    assert!(matches!(
        state.pairing.stage,
        PairingStage::Failed(PairingFailure::KeychainUnavailable)
    ));
    assert!(!state.connected());
    let catalog = container.pairing_catalog().unwrap();
    assert!(catalog.paired_gateways.is_empty());
    assert!(catalog.selected_origin.is_none());
    assert!(catalog.pending_pairings[0].issued_credential.is_some());
    store.0.lock().unwrap().fail_access_write = false;
    let reopened = Arc::new(
        NativeContainer::open_settings(&temporary.path().canonicalize().unwrap()).unwrap(),
    );
    let recovered = connect_or_pair(
        reopened.clone(),
        gateway.origin.clone(),
        store,
        false,
        |_| {},
    )
    .await;
    state.complete_pairing_connection(state.connection.epoch, recovered);
    assert!(
        state.connected(),
        "Retry completes only after the approved secret is saved"
    );
    assert_eq!(state.pairing.stage, PairingStage::Connected);
    assert!(reopened
        .pairing_catalog()
        .unwrap()
        .pending_pairings
        .is_empty());
    {
        let requests = gateway.requests.lock().unwrap();
        let starts: Vec<_> = requests
            .iter()
            .filter(|request| request.path == PAIRING_START_PATH)
            .collect();
        assert_eq!(starts.len(), 2);
        assert!(
            starts[0].body == starts[1].body,
            "Retry preserves the exact approved request and polling secret"
        );
        assert_wire_privacy(&requests);
    }
    gateway.finish().await;
}

#[tokio::test]
async fn saved_credentials_remain_bound_to_the_exact_origin_and_deployment() {
    let (_temporary, container, store) = installation();
    let first = Gateway::start(false).await;
    connect_or_pair(
        container.clone(),
        first.origin.clone(),
        store.clone(),
        false,
        |_| {},
    )
    .await
    .expect("initial pairing");
    first.requests.lock().unwrap().clear();
    *first.deployment.lock().unwrap() = "replacement-deployment".into();
    let result = connect_or_pair(
        container.clone(),
        first.origin.clone(),
        store.clone(),
        false,
        |_| {},
    )
    .await;
    assert!(matches!(result, Err(PairingFailure::Authentication)));
    {
        let requests = first.requests.lock().unwrap();
        assert_eq!(
            requests.len(),
            2,
            "changed deployment cannot silently pair again"
        );
        assert_wire_privacy(&requests);
    }
    let second = Gateway::start(true).await;
    let result = connect_or_pair(
        container.clone(),
        second.origin.clone(),
        store,
        false,
        |_| {},
    )
    .await;
    assert!(matches!(result, Err(PairingFailure::Network)));
    {
        let requests = second.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, PAIRING_START_PATH);
        assert_wire_privacy(&requests);
    }
    let catalog = container.pairing_catalog().unwrap();
    assert_eq!(catalog.paired_gateways.len(), 1);
    assert_eq!(catalog.paired_gateways[0].origin, first.origin);
    assert_eq!(
        catalog.paired_gateways[0].deployment_id,
        "synthetic-deployment"
    );
    assert_eq!(
        catalog.selected_origin.as_deref(),
        Some(first.origin.as_str())
    );
    assert_eq!(catalog.pending_pairings[0].origin, second.origin);
    first.finish().await;
    second.finish().await;
}

#[test]
fn focus_pause_and_destination_change_fence_pending_pairing_callbacks() {
    let mut state = State::fixture();
    state.setup_gateway = true;
    state.pairing.fixture_enabled = true;
    state.connect();
    let old = state.connection.epoch;
    let challenge = PairingChallenge {
        pairing_id: "a".repeat(32),
        approval_code: "b".repeat(32),
        expires_at_ms: 1,
        poll_after_ms: 2000,
    };
    state.pairing_progress(old, PairingStage::Waiting(challenge.clone()));
    state.pairing_focus(false);
    assert!(state.pairing.paused && !state.pairing.busy());
    state.pairing_progress(old, PairingStage::Failed(PairingFailure::Denied));
    state.complete_pairing_connection(old, Err(PairingFailure::Authentication));
    assert_eq!(state.pairing.stage, PairingStage::Waiting(challenge));
    assert!(state.client.is_none());
    state.pairing_focus(true);
    assert!(!state.pairing.paused && state.connection.epoch > old);
    assert_eq!(
        state.intents,
        vec![Intent::PairingConnect, Intent::PairingConnect]
    );
    let resumed = state.connection.epoch;
    state.navigate(crate::app::Page::Brain);
    state.complete_pairing_connection(resumed, Err(PairingFailure::Denied));
    assert_eq!(state.pairing.stage, PairingStage::Idle);
    assert!(!state.connected() && state.client.is_none());
}

#[test]
fn destination_edit_drops_explicit_operator_file_authority() {
    let mut state = State::fixture();
    state.pairing.fixture_enabled = true;
    state.connection_profiles.operator_token_override = true;
    state.connection.token_file = rune::TextEdit::new("/fixture/private/operator-native-token");
    state.connection.origin = rune::TextEdit::new("https://another.example.test/");
    state.profile_editor_changed(true);
    assert!(!state.connection_profiles.operator_token_override);
    assert!(state.connection.token_file.text.is_empty());
    assert!(state.uses_native_pairing());
}
