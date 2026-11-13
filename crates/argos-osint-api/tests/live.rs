#![cfg(feature = "embedded")]
//! Offline WebSocket contracts using a temporary repository and loopback listener.

use std::ffi::OsString;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use argos_osint_api::network_router;
use argos_osint_api::ApiState;
use argos_osint_api::NetworkPolicy;
use argos_osint_api::ScopedCredential;
use argos_osint_core::storage::atlas;
use argos_osint_core::storage::bootstrap;
use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::open::Target;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::store::AtlasArticleRow;
use axum::body::to_bytes;
use axum::body::Body;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::Router;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::Permission;
use futures_util::SinkExt;
use futures_util::StreamExt;
use serde_json::json;
use serde_json::Value;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tower::ServiceExt;

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

const NATIVE_TOKEN: &str = "live-native-purpose-0123456789abcdefghijkl";

struct Home(Vec<(&'static str, Option<OsString>)>);
impl Home {
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
impl Drop for Home {
    fn drop(&mut self) {
        for (key, previous) in &self.0 {
            match previous {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

async fn setup_post(
    app: &Router,
    host: &str,
    path: &str,
    registration: Option<&RegistrationReceipt>,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method("POST")
        .uri(path)
        .header("host", host)
        .header("authorization", format!("Bearer {NATIVE_TOKEN}"))
        .header("content-type", "application/json");
    if let Some(registration) = registration {
        request = request
            .header("x-fleetworks-registration", &registration.registration_id)
            .header(
                "x-fleetworks-registration-revision",
                registration.revision.to_string(),
            );
    }
    let response = app
        .clone()
        .oneshot(
            request
                .body(Body::from(body.to_string()))
                .expect("setup request"),
        )
        .await
        .expect("setup response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 16 * 1024)
        .await
        .expect("bounded setup response");
    (status, serde_json::from_slice(&bytes).expect("setup JSON"))
}

async fn receive(socket: &mut Socket, kind: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let message = socket.next().await.unwrap().unwrap();
            if let Message::Text(text) = message {
                let value: Value = serde_json::from_str(&text).unwrap();
                eprintln!("waiting for {kind}, received {}", value["type"]);
                assert_ne!(value["type"], "error", "{value}");
                if value["type"] == kind {
                    return value;
                }
            }
        }
    })
    .await
    .unwrap_or_else(|error| panic!("{kind} live event did not arrive: {error}"))
}

#[tokio::test]
async fn streams_selected_channels_and_settles_the_brief_without_http_polling() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir(&home).expect("isolated configuration owner");
    let home = home.canonicalize().expect("canonical configuration owner");
    let _home = Home::install(&home);
    let repo = Repository::connect(
        &Target::Embedded {
            path: dir.path().join("db"),
        },
        "argos",
        "live_contract",
    )
    .await
    .unwrap();
    bootstrap::ensure_network_identity(repo.db())
        .await
        .expect("network identity");
    connect_registry::initialize_control(repo.db())
        .await
        .expect("registration control");
    atlas::insert_run(repo.db(), "fixture-run", "{}", "{}")
        .await
        .unwrap();
    let article = AtlasArticleRow {
        run_id: "fixture-run".into(),
        id: "fixture-article".into(),
        title: "Offline harbor fixture".into(),
        description: "Retained fixture evidence".into(),
        category: "geopolitical".into(),
        ..Default::default()
    };
    atlas::upsert_article(repo.db(), &article).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let host = format!("127.0.0.1:{port}");
    let native = ScopedCredential::new(
        "live-native".into(),
        "live-scope".into(),
        "live-access".into(),
        CredentialPurpose::Native,
        [
            Permission::ReadBackend,
            Permission::RegisterWorkstation,
            Permission::SubmitInteractive,
            Permission::ReadWorkstationActivity,
        ]
        .into_iter()
        .collect(),
        NATIVE_TOKEN,
    )
    .expect("explicit Native authority");
    let requests = Arc::new(AtomicUsize::new(0));
    let counted = requests.clone();
    // With no private Brain client installed, this gateway uses the actual local
    // Brain handlers. Setup still traverses authenticated public admission.
    let app = network_router(
        ApiState {
            repo: repo.clone(),
            instance_id: "live-fixture".into(),
            port,
            origins: Arc::new(vec!["http://127.0.0.1:3000".into()]),
        },
        NetworkPolicy::with_credentials(std::slice::from_ref(&host), vec![native])
            .expect("native policy"),
    )
    .layer(axum::middleware::from_fn(
        move |request: Request, next: Next| {
            let counted = counted.clone();
            async move {
                counted.fetch_add(1, Ordering::SeqCst);
                next.run(request).await
            }
        },
    ));
    let (status, registration) = setup_post(
        &app,
        &host,
        "/api/v1/connect/registrations/ensure",
        None,
        serde_json::to_value(RegistrationEnsureRequest::new(
            "live-registration".into(),
            None,
            "live-installation-marker".into(),
        ))
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{registration}");
    let registration: RegistrationReceipt =
        serde_json::from_value(registration).expect("real workstation proof");
    let (status, receipt) = setup_post(
        &app,
        &host,
        "/api/v1/brief/ensure",
        Some(&registration),
        json!({"run":"fixture-run","id":"fixture-article","request_id":"live-brief-request"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    let job = receipt["job_id"]
        .as_str()
        .expect("attributed brief job")
        .to_owned();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let url = format!("ws://127.0.0.1:{port}/api/v1/live");

    let mut refused = url.clone().into_client_request().unwrap();
    refused
        .headers_mut()
        .insert("origin", "http://untrusted.example".parse().unwrap());
    match tokio_tungstenite::connect_async(refused).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            assert_eq!(response.status().as_u16(), 403)
        }
        _ => panic!("foreign WebSocket origin was not refused"),
    }
    match tokio_tungstenite::connect_async(&url).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            assert_eq!(response.status().as_u16(), 401)
        }
        _ => panic!("unauthenticated WebSocket was not refused"),
    }
    let mut authorized = url.into_client_request().unwrap();
    authorized.headers_mut().insert(
        "authorization",
        format!("Bearer {NATIVE_TOKEN}").parse().unwrap(),
    );
    authorized.headers_mut().insert(
        "x-fleetworks-registration",
        registration.registration_id.parse().unwrap(),
    );
    authorized.headers_mut().insert(
        "x-fleetworks-registration-revision",
        registration.revision.to_string().parse().unwrap(),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(authorized).await.unwrap();
    // Exclude one-time admission and upgrade requests from the no-polling check.
    let baseline_requests = requests.load(Ordering::SeqCst);
    assert!(
        baseline_requests > 0,
        "HTTP counting must observe setup and upgrade"
    );
    socket
        .send(Message::Text(
            json!({"type":"subscribe","metrics":true,"brief":null})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let metric = receive(&mut socket, "metrics").await;
    for key in [
        "cpu_percent",
        "memory_percent",
        "disk_percent",
        "network_kbps",
    ] {
        assert!(
            metric["data"][key].is_null()
                || metric["data"][key]
                    .as_f64()
                    .is_some_and(|value| value.is_finite() && value >= 0.0)
        );
    }
    assert!(receive(&mut socket, "metrics").await["data"].is_object());

    socket.send(Message::Text(json!({"type":"subscribe","metrics":false,"brief":{"run":"fixture-run","id":"fixture-article","watch":"generation-1"}}).to_string().into())).await.unwrap();
    eprintln!("waiting for initial brief");
    let pending = receive(&mut socket, "brief").await;
    assert_eq!(pending["job_id"], job);
    assert_eq!(pending["private_details_available"], true);
    assert_eq!(pending["complete"], false);
    assert_eq!(pending["refresh"], true);
    assert_eq!(pending["stages"].as_array().unwrap().len(), 4);

    // Simulate the isolated worker's durable commits. No providers or jobs run.
    repo.db().query("UPDATE type::record('job', $job) SET state = 'completed'; UPDATE task SET state = 'completed' WHERE job = type::record('job', $job);")
        .bind(("job", job)).await.unwrap().check().unwrap();
    eprintln!("waiting for completed brief");
    let completed = receive(&mut socket, "brief").await;
    assert_eq!(completed["complete"], true);
    assert_eq!(completed["private_details_available"], true);
    assert_eq!(completed["stages"].as_array().unwrap().len(), 4);
    assert_eq!(completed["refresh"], true);
    assert!(completed["stages"]
        .as_array()
        .unwrap()
        .iter()
        .all(|stage| stage["state"] == "completed"));
    assert!(
        tokio::time::timeout(Duration::from_millis(1200), socket.next())
            .await
            .is_err(),
        "a settled brief must stop producing updates"
    );
    assert_eq!(
        requests.load(Ordering::SeqCst),
        baseline_requests,
        "WebSocket subscriptions and settlement must not issue HTTP polling requests"
    );
    socket.close(None).await.unwrap();
    server.abort();
    let _ = server.await;
}
