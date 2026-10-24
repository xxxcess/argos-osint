//! Offline loopback fixtures for actual classifier wire attempts and receipts.
use std::sync::{atomic::AtomicBool, Arc, Mutex};
use std::time::Instant;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::atlas_classify::{batches, execute, BatchAdapter};
use crate::provider::{DecisionsResponse, SettingsFile};
use crate::recon::model_exec::DecisionsAdapter;
use crate::secrets::ProviderSecret;
use crate::store::{AtlasArticleRow, Store};

fn article(id: usize) -> AtlasArticleRow {
    serde_json::from_value(json!({"run_id":"fixture","id":format!("a{id}"),"title":format!("Acme article {id}"),"description":"Jane Doe announces an office","url":format!("https://example.org/{id}"),"country":"us","source_name":"Desk","source_domain":"example.org","published_at":"","provider":"newsapi","temperature":1.0,"category":"unk","seen_at":"","author":"","image_url":""})).unwrap()
}

async fn read_request(socket: &mut tokio::net::TcpStream) -> (String, Value) {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let n = tokio::time::timeout(std::time::Duration::from_secs(5), socket.read(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        assert!(n > 0, "request ended before its body");
        bytes.extend_from_slice(&buffer[..n]);
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|n| n.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            if bytes.len() >= end + 4 + length {
                return (
                    headers,
                    serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap(),
                );
            }
        }
    }
}

async fn server(
    requests: usize,
    native: bool,
    partial_first: bool,
) -> (
    ProviderSecret,
    Arc<Mutex<Vec<Value>>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let capture = recorded.clone();
    let task = tokio::spawn(async move {
        for attempt in 0..requests {
            let (mut socket, _) =
                tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
            let (headers, request) = read_request(&mut socket).await;
            let mut ids: Vec<String> = if native {
                assert!(headers.starts_with("POST /api/alpha/decisions "));
                request["questions"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .cloned()
                    .collect()
            } else {
                assert!(headers.starts_with("POST /api/v1/chat/completions "));
                let rows: Value =
                    serde_json::from_str(request["messages"][1]["content"].as_str().unwrap())
                        .unwrap();
                rows.as_array()
                    .unwrap()
                    .iter()
                    .map(|r| r["article_id"].as_str().unwrap().into())
                    .collect()
            };
            if partial_first && attempt == 0 {
                ids.truncate(1);
            }
            let response = if native {
                let answers: serde_json::Map<String, Value> = ids
                    .iter()
                    .map(|id| (id.clone(), json!({"type":"choice","choice":"unk"})))
                    .collect();
                json!({"id":"loopback","answers":answers,"model":"typesafe/jev-1.13"})
            } else {
                let rows: Vec<_> = ids
                    .iter()
                    .map(|id| json!({"article_id":id,"category":"unk"}))
                    .collect();
                json!({"id":"loopback","choices":[{"message":{"role":"assistant","content":serde_json::to_string(&rows).unwrap()},"finish_reason":"stop"}]})
            };
            capture.lock().unwrap().push(request);
            let body = response.to_string();
            let reply=format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",body.len());
            socket.write_all(reply.as_bytes()).await.unwrap();
        }
    });
    (
        ProviderSecret {
            kind: "openrouter".into(),
            base_url: format!("http://{addr}/api/v1"),
            model: if native {
                "typesafe/jev-1.13".into()
            } else {
                "fixture/chat".into()
            },
            api_key: Some("offline-fixture".into()),
            stt_model: None,
            device: None,
        },
        recorded,
        task,
    )
}

#[test]
fn partial_native_receipt_recompiles_missing_ids_for_both_transports() {
    let adapter = BatchAdapter::new(vec![article(1), article(2)]);
    let partial: DecisionsResponse =
        serde_json::from_value(json!({"answers":{"a1":{"type":"choice","choice":"economic"}}}))
            .unwrap();
    assert!(adapter.parse_native(&partial).is_err());
    assert_eq!(adapter.receipt()["a1"], "economic");
    let (_, questions) = adapter.compile_native();
    assert_eq!(
        questions.as_object().unwrap().keys().collect::<Vec<_>>(),
        vec!["a2"]
    );
    let rows: Value = serde_json::from_str(&adapter.compile_chat()[1].content).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["article_id"], "a2");
    assert!(adapter
        .parse_chat(r#"[{"article_id":"a2","category":"unk"}]"#)
        .is_ok());
}

async fn run_phase6(db: &std::path::Path, run: &str) -> crate::atlas::Stop {
    use crate::atlas::{run_atlas, RunInput};
    let pause = AtomicBool::new(false);
    let keys = crate::osint::ProviderKeys::default();
    run_atlas(
        RunInput {
            db_path: db,
            pause: &pause,
            keys: &keys,
            user_agent: "Argos offline",
            pace: false,
            resume: true,
            feed: &[],
            classifier: None,
            synthesizer: None,
            synthesizer_fallbacks: &[],
            run_id: Some(run),
        },
        |_| {},
        |_| Box::pin(async { panic!("phase-6 resume must not discover") }),
    )
    .await
    .unwrap()
}
