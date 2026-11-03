#![cfg(feature = "embedded")]
//! Offline WebSocket contracts using a temporary repository and loopback listener.

use std::ffi::OsString;
use std::sync::Arc;
use std::time::Duration;

use argos_osint_api::router;
use argos_osint_api::ApiState;
use argos_osint_core::intel_service;
use argos_osint_core::storage::atlas;
use argos_osint_core::storage::open::Target;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::store::AtlasArticleRow;
use futures_util::SinkExt;
use futures_util::StreamExt;
use serde_json::json;
use serde_json::Value;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

struct Home(Option<OsString>);
impl Drop for Home {
    fn drop(&mut self) {
        if let Some(previous) = &self.0 {
            std::env::set_var("ARGOS_HOME", previous);
        } else {
            std::env::remove_var("ARGOS_HOME");
        }
    }
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
    let _home = Home(std::env::var_os("ARGOS_HOME"));
    std::env::set_var("ARGOS_HOME", dir.path().join("home"));
    let repo = Repository::connect(
        &Target::Embedded {
            path: dir.path().join("db"),
        },
        "argos",
        "live_contract",
    )
    .await
    .unwrap();
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
    let job = intel_service::ensure_brief(&repo, &article).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = router(ApiState {
        repo: repo.clone(),
        instance_id: "live-fixture".into(),
        port,
        origins: Arc::new(vec!["http://127.0.0.1:3000".into()]),
    });
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
    let (mut socket, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
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
    assert_eq!(pending["complete"], false);
    assert_eq!(pending["refresh"], true);
    assert_eq!(pending["stages"].as_array().unwrap().len(), 4);

    // Simulate the isolated worker's durable commits. No providers or jobs run.
    repo.db().query("UPDATE type::record('job', $job) SET state = 'completed'; UPDATE task SET state = 'completed' WHERE job = type::record('job', $job);")
        .bind(("job", job)).await.unwrap().check().unwrap();
    eprintln!("waiting for completed brief");
    let completed = receive(&mut socket, "brief").await;
    assert_eq!(completed["complete"], true);
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
    socket.close(None).await.unwrap();
    server.abort();
    let _ = server.await;
}
