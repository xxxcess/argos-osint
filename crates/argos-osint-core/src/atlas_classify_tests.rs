//! Offline loopback fixtures for actual classifier wire attempts and receipts.
use std::sync::{atomic::AtomicBool, Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::atlas_classify::BatchAdapter;
use crate::provider::DecisionsResponse;
use crate::recon::model_exec::DecisionsAdapter;
use crate::secrets::ProviderSecret;
use crate::store::AtlasArticleRow;

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
                let compiled: Value =
                    serde_json::from_str(request["messages"][1]["content"].as_str().unwrap())
                        .unwrap();
                compiled["questions"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .cloned()
                    .collect()
            };
            if partial_first && attempt == 0 {
                ids.truncate(1);
            }
            let response = if native {
                let answers: serde_json::Map<String, Value> = ids
                    .iter()
                    .map(|id| {
                        // Rank 1 is the miss (`unk`); later ranks stop.
                        let choice = if id.ends_with("#category_1") {
                            "unk"
                        } else {
                            "done"
                        };
                        (id.clone(), json!({"type":"choice","choice":choice}))
                    })
                    .collect();
                json!({"id":"loopback","answers":answers,"model":"typesafe/jev-1.13"})
            } else {
                let mut content = String::from("argos-text-v1\n");
                for id in &ids {
                    let choice = if id.ends_with("#category_1") {
                        "unk"
                    } else {
                        "done"
                    };
                    content.push_str(&format!(
                        "## Decision\nQuestion: {id}\nChoice: {choice}\nEnd\n"
                    ));
                }
                json!({"id":"loopback","choices":[{"message":{"role":"assistant","content":content},"finish_reason":"stop"}]})
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
    let ranks = crate::atlas::RANKED_PICKS;
    let mut answers = serde_json::Map::new();
    for rank in 1..=ranks {
        let choice = if rank == 1 { "unk" } else { "done" };
        answers.insert(
            format!("a1#category_{rank}"),
            json!({"type":"choice","choice":choice}),
        );
    }
    let partial: DecisionsResponse = serde_json::from_value(json!({"answers":answers})).unwrap();
    assert!(adapter.parse_native(&partial).is_err());
    assert_eq!(adapter.receipt()["a1"], "unk");
    let (_, questions) = adapter.compile_native();
    let keys: Vec<String> = questions.as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys.len(), ranks);
    assert!(keys.iter().all(|key| key.starts_with("a2#category_")));
    let mut labeled = String::from("argos-text-v1\n");
    for rank in 1..=ranks {
        let choice = if rank == 1 { "unk" } else { "done" };
        labeled.push_str(&format!(
            "## Decision\nQuestion: a2#category_{rank}\nChoice: {choice}\nEnd\n"
        ));
    }
    assert!(adapter.parse_chat(&labeled).is_ok());
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
