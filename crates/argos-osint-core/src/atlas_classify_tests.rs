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

fn setup(db: &std::path::Path) {
    Store::open(db).unwrap();
    // All config/auth reads stay next to the isolated fixture database.
    let mut settings = SettingsFile::default();
    settings.defaults.classifier.fallbacks.clear();
    std::fs::write(
        db.with_file_name("config.toml"),
        toml::to_string(&settings).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn cold_batched_and_singleton_workloads_record_real_attempts_and_warm_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("argos.db");
    setup(&db);
    let articles: Vec<_> = (0..17).map(article).collect();
    let (secret, requests, worker) = server(20, false, false).await;
    let cancel = Arc::new(AtomicBool::new(false));
    let started = Instant::now();
    for row in &articles {
        let (receipt, error) = execute(
            &db,
            "singleton-baseline",
            &secret,
            vec![row.clone()],
            &cancel,
        )
        .await;
        assert!(error.is_none(), "{error:?}");
        assert_eq!(receipt.len(), 1);
    }
    let singleton_ms = started.elapsed().as_millis();
    assert_eq!(requests.lock().unwrap().len(), 17);
    let mut durations = Vec::new();
    for batch in batches(&articles) {
        let started = Instant::now();
        let expected = batch.len();
        let (receipt, error) = execute(&db, "batched", &secret, batch, &cancel).await;
        durations.push(started.elapsed().as_micros());
        assert!(error.is_none(), "{error:?}");
        assert_eq!(receipt.len(), expected);
    }
    assert_eq!(requests.lock().unwrap().len(), 20);
    let warm = Instant::now();
    for batch in batches(&articles) {
        let expected = batch.len();
        let (receipt, error) = execute(&db, "batched", &secret, batch, &cancel).await;
        assert!(error.is_none());
        assert_eq!(receipt.len(), expected);
    }
    let warm_us = warm.elapsed().as_micros();
    assert_eq!(
        requests.lock().unwrap().len(),
        20,
        "warm receipts issued wire requests"
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), worker)
        .await
        .unwrap()
        .unwrap();
    let store = Store::open(&db).unwrap();
    let attempts: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM recon_model_attempts WHERE dispatched=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(attempts, 20);
    let wrong_roles: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM recon_model_operations WHERE role<>'classifier'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(wrong_roles, 0);
    let wrong_routes:i64=store.conn.query_row("SELECT COUNT(*) FROM recon_model_attempts WHERE provider<>'openrouter' OR model<>'fixture/chat' OR route_index<>0 OR outcome<>'ok'",[],|r|r.get(0)).unwrap();
    assert_eq!(wrong_routes, 0);
    let (queue_ms, execution_ms, backoff_ms): (i64, i64, i64) = store.conn.query_row(
        "SELECT COALESCE(SUM(queue_ms),0),COALESCE(SUM(elapsed_ms),0),COALESCE(SUM(CASE WHEN outcome='retry_scheduled' THEN wait_ms ELSE 0 END),0) FROM recon_model_attempts WHERE dispatched=1",
        [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).unwrap();
    store.atlas_insert_run("local", "{}", "{}").unwrap();
    let index =
        crate::osint::wikipedia_rsp::RspIndex::build(Vec::new(), chrono::Utc::now().to_rfc3339());
    let mut local_us = Vec::new();
    let mut entity_ms = 0;
    let mut publisher_ms = 0;
    for mut row in articles.clone() {
        row.run_id = "local".into();
        row.category = "economic".into();
        store.atlas_upsert_article(&row).unwrap();
        let started = Instant::now();
        let metadata = crate::atlas_metadata::collect(&row, Some(&index), cancel.clone()).await;
        local_us.push(started.elapsed().as_micros());
        entity_ms += metadata.entity_ms;
        publisher_ms += metadata.publisher_ms;
        assert!(store.atlas_save_metadata(&metadata).unwrap());
    }
    let matching = Instant::now();
    let pairs = crate::related_articles::build(&store, "local").unwrap();
    let matching_us = matching.elapsed().as_micros();
    let warm_matching = Instant::now();
    assert_eq!(
        crate::related_articles::build(&store, "local").unwrap(),
        pairs
    );
    let warm_matching_us = warm_matching.elapsed().as_micros();
    assert_eq!(
        requests.lock().unwrap().len(),
        20,
        "native metadata sent a model request"
    );
    local_us.sort_unstable();
    durations.sort_unstable();
    eprintln!("offline fixed workload: 17 articles; singleton baseline 17 requests/{singleton_ms}ms; batches 3 requests p50={}us p95={}us; warm 0 requests/{warm_us}us",durations[1],durations[2]);
    eprintln!("route=openrouter/fixture/chat; actual attempts=20; queue={queue_ms}ms execution={execution_ms}ms backoff={backoff_ms}ms; local NLP compiled={}; entity={entity_ms}ms publisher cached lookup={publisher_ms}ms; metadata p50={}us p95={}us; matching={matching_us}us warm matching={warm_matching_us}us pairs={pairs}",cfg!(feature="nlp"),local_us[8],local_us[16]);
}

#[tokio::test]
async fn native_batch_uses_article_specific_questions_and_classifier_route() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("argos.db");
    setup(&db);
    let (secret, requests, worker) = server(1, true, false).await;
    let (receipt, error) = execute(
        &db,
        "native",
        &secret,
        vec![article(1), article(2)],
        &Arc::new(AtomicBool::new(false)),
    )
    .await;
    assert!(error.is_none(), "{error:?}");
    assert_eq!(receipt.len(), 2);
    tokio::time::timeout(std::time::Duration::from_secs(5), worker)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        requests.lock().unwrap()[0]["questions"]
            .as_object()
            .unwrap()
            .len(),
        2
    );
    assert!(receipt.values().all(|c| c == "unk"));
}

#[tokio::test]
async fn cancellation_before_admission_never_dispatches_a_request() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("argos.db");
    setup(&db);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let secret = ProviderSecret {
        kind: "openrouter".into(),
        base_url: format!("http://{}/api/v1", listener.local_addr().unwrap()),
        model: "fixture/chat".into(),
        api_key: Some("offline-fixture".into()),
        stt_model: None,
        device: None,
    };
    let (receipt, error) = execute(
        &db,
        "cancelled",
        &secret,
        vec![article(1)],
        &Arc::new(AtomicBool::new(true)),
    )
    .await;
    assert!(error.is_some());
    assert!(receipt.is_empty());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
    let store = Store::open(&db).unwrap();
    let attempts: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM recon_model_attempts WHERE dispatched=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(attempts, 0);
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

#[tokio::test]
async fn partial_wire_replies_retry_only_unresolved_ids_on_native_and_chat() {
    for native in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("argos.db");
        setup(&db);
        let (secret, requests, worker) = server(2, native, true).await;
        let (receipt, error) = execute(
            &db,
            "partial-wire",
            &secret,
            vec![article(1), article(2)],
            &Arc::new(AtomicBool::new(false)),
        )
        .await;
        assert!(error.is_none(), "{error:?}");
        assert_eq!(receipt.len(), 2);
        tokio::time::timeout(std::time::Duration::from_secs(5), worker)
            .await
            .unwrap()
            .unwrap();
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        if native {
            assert_eq!(requests[0]["questions"].as_object().unwrap().len(), 2);
            assert_eq!(
                requests[1]["questions"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .collect::<Vec<_>>(),
                vec!["a2"]
            );
        } else {
            let subset: Value =
                serde_json::from_str(requests[1]["messages"][1]["content"].as_str().unwrap())
                    .unwrap();
            assert_eq!(subset.as_array().unwrap().len(), 1);
            assert_eq!(subset[0]["article_id"], "a2");
        }
        let store = Store::open(&db).unwrap();
        let (attempts, successes): (i64, i64) = store
            .conn
            .query_row(
                "SELECT COUNT(*),SUM(outcome='ok') FROM recon_model_attempts WHERE dispatched=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((attempts, successes), (2, 1));
    }
}

#[tokio::test]
async fn configured_classifier_fallback_runs_native_or_chat_without_other_roles() {
    for native in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("argos.db");
        setup(&db);
        let (fallback, requests, worker) = server(1, native, false).await;
        let mut auth = crate::secrets::AuthFile::default();
        auth.set_account(fallback.clone());
        auth.save_to(&db.with_file_name("auth.json")).unwrap();
        let mut settings = SettingsFile::default();
        settings.defaults.classifier.provider = "google".into();
        settings.defaults.classifier.model = "fixture-missing".into();
        settings.defaults.classifier.fallbacks = vec![crate::provider::ModelRoute {
            provider: "openrouter".into(),
            model: fallback.model.clone(),
            account: String::new(),
        }];
        std::fs::write(
            db.with_file_name("config.toml"),
            toml::to_string(&settings).unwrap(),
        )
        .unwrap();
        let missing = ProviderSecret {
            kind: "google".into(),
            base_url: "http://127.0.0.1:1".into(),
            model: "fixture-missing".into(),
            api_key: None,
            stt_model: None,
            device: None,
        };
        let (receipt, error) = execute(
            &db,
            "fallback",
            &missing,
            vec![article(1)],
            &Arc::new(AtomicBool::new(false)),
        )
        .await;
        assert!(error.is_none(), "{error:?}");
        assert_eq!(receipt.len(), 1);
        tokio::time::timeout(std::time::Duration::from_secs(5), worker)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(requests.lock().unwrap().len(), 1);
        let store = Store::open(&db).unwrap();
        let actual: (String, String, i64) = store
            .conn
            .query_row(
                "SELECT provider,model,route_index FROM recon_model_attempts WHERE dispatched=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(actual, ("openrouter".into(), fallback.model, 1));
        let other_roles: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM recon_model_operations WHERE role<>'classifier'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(other_roles, 0);
    }
}

#[tokio::test]
async fn changed_source_revision_invalidates_only_its_cached_batch_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("argos.db");
    setup(&db);
    let (secret, requests, worker) = server(2, false, false).await;
    let cancel = Arc::new(AtomicBool::new(false));
    let original = vec![article(1), article(2)];
    for _ in 0..2 {
        let (receipt, error) = execute(&db, "revision", &secret, original.clone(), &cancel).await;
        assert!(error.is_none());
        assert_eq!(receipt.len(), 2);
    }
    assert_eq!(requests.lock().unwrap().len(), 1);
    let mut changed = original;
    changed[1]
        .description
        .push_str(" with a revised source description");
    let (receipt, error) = execute(&db, "revision", &secret, changed, &cancel).await;
    assert!(error.is_none());
    assert_eq!(receipt.len(), 2);
    tokio::time::timeout(std::time::Duration::from_secs(5), worker)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn cancel_inflight_keeps_partial_receipt_and_resume_requests_only_missing_id() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("argos.db");
    setup(&db);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let secret = ProviderSecret {
        kind: "openrouter".into(),
        base_url: format!("http://{}/api/v1", listener.local_addr().unwrap()),
        model: "fixture/chat".into(),
        api_key: Some("offline-fixture".into()),
        stt_model: None,
        device: None,
    };
    let (inflight_tx, inflight_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let worker = tokio::spawn(async move {
        let mut inflight_tx = Some(inflight_tx);
        let mut release_rx = Some(release_rx);
        for attempt in 0..3 {
            let (mut socket, _) =
                tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
            let (_, request) = read_request(&mut socket).await;
            let rows: Value =
                serde_json::from_str(request["messages"][1]["content"].as_str().unwrap()).unwrap();
            if attempt == 0 {
                assert_eq!(rows.as_array().unwrap().len(), 2);
            } else {
                assert_eq!(rows.as_array().unwrap().len(), 1);
                assert_eq!(rows[0]["article_id"], "a2");
            }
            if attempt == 1 {
                inflight_tx.take().unwrap().send(()).unwrap();
                release_rx.take().unwrap().await.unwrap();
                continue;
            }
            let id = if attempt == 0 { "a1" } else { "a2" };
            let body=json!({"choices":[{"message":{"role":"assistant","content":format!(r#"[{{"article_id":"{id}","category":"unk"}}]"#)},"finish_reason":"stop"}]}).to_string();
            let reply=format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",body.len());
            socket.write_all(reply.as_bytes()).await.unwrap();
        }
    });
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let work = execute(
        &db,
        "partial-cancel",
        &secret,
        vec![article(1), article(2)],
        &cancel,
    );
    let signal = async move {
        tokio::time::timeout(std::time::Duration::from_secs(5), inflight_rx)
            .await
            .unwrap()
            .unwrap();
        flag.store(true, std::sync::atomic::Ordering::Release);
    };
    let ((receipt, error), ()) = tokio::join!(work, signal);
    assert!(error.is_some());
    assert_eq!(receipt.len(), 1);
    assert_eq!(receipt["a1"], "unk");
    release_tx.send(()).unwrap();
    cancel.store(false, std::sync::atomic::Ordering::Release);
    let (resumed, error) = execute(
        &db,
        "partial-cancel",
        &secret,
        vec![article(1), article(2)],
        &cancel,
    )
    .await;
    assert!(error.is_none(), "{error:?}");
    assert_eq!(resumed.len(), 2);
    tokio::time::timeout(std::time::Duration::from_secs(5), worker)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn all_unk_cycle_classifies_and_finishes_without_brain_or_nonclassifier_calls() {
    use crate::atlas::{run_atlas, AtlasEvent, Cursor, RunInput, RunStats, Stop};
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("argos.db");
    setup(&db);
    let (secret, requests, worker) = server(1, false, false).await;
    let store = Store::open(&db).unwrap();
    let stats = RunStats {
        contract_version: crate::atlas_work::COLLECTION_CONTRACT.into(),
        ..RunStats::default()
    };
    let cursor = Cursor {
        phase: 3,
        leg: "classify".into(),
        ..Cursor::default()
    };
    store
        .atlas_insert_run(
            "all-unk",
            &serde_json::to_string(&cursor).unwrap(),
            &serde_json::to_string(&stats).unwrap(),
        )
        .unwrap();
    store
        .atlas_set_state("all-unk", "paused", "", false)
        .unwrap();
    for i in 0..2 {
        let mut row = article(i);
        row.run_id = "all-unk".into();
        store.atlas_upsert_article(&row).unwrap();
    }
    let pause = AtomicBool::new(false);
    let keys = crate::osint::ProviderKeys::default();
    let mut events = Vec::new();
    let stop = run_atlas(
        RunInput {
            db_path: &db,
            pause: &pause,
            keys: &keys,
            user_agent: "Argos offline",
            pace: false,
            resume: true,
            feed: &[],
            classifier: Some(secret),
            synthesizer: None,
            synthesizer_fallbacks: &[],
            run_id: Some("all-unk"),
        },
        |event| events.push(event),
        |_| Box::pin(async { panic!("retained cycle must not discover") }),
    )
    .await
    .unwrap();
    assert_eq!(stop, Stop::Finished);
    tokio::time::timeout(std::time::Duration::from_secs(5), worker)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(
        store.atlas_latest_run().unwrap().unwrap().state,
        "completed"
    );
    for row in store.atlas_list_articles("all-unk").unwrap() {
        assert_eq!(row.category, "unk");
        assert_eq!(
            store
                .atlas_metadata("all-unk", &row.id)
                .unwrap()
                .unwrap()
                .classification_status,
            "classified"
        );
    }
    assert!(store.list_memories().unwrap().is_empty());
    for table in [
        "argos_memory_index_state",
        "argos_index_changes",
        "insight_snapshots",
    ] {
        let count: i64 = store
            .conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
    let other_roles: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM recon_model_operations WHERE role<>'classifier'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(other_roles, 0);
    assert!(!events.iter().any(|e| matches!(
        e,
        AtlasEvent::MemoryProgress { .. } | AtlasEvent::MemoriesChanged { .. }
    )));
}
