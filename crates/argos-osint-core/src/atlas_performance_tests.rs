//! Offline contracts for production Atlas measurement and fault redaction.
use super::*;

#[test]
fn classifier_latency_is_a_paired_batch_distribution_and_queue_is_observed() {
    let before = crate::classifier::ClassifierPerformance {
        queue_micros: 250,
        batch_execution_micros: Vec::new(),
        batch_sizes: Vec::new(),
        js_heap_current_bytes: 1,
        js_heap_peak_bytes: 1,
        runtime_reserved_bytes: 128 * 1024 * 1024,
        stack_limit_bytes: 1024 * 1024,
    };
    let mut after = before.clone();
    after.queue_micros = 1750;
    after.batch_execution_micros = vec![1000, 64000, 2000];
    after.batch_sizes = vec![1, 64, 1];
    let metrics = classifier_worker_metrics(Some(&before), Ok(&after));
    assert_eq!(metrics["atlas.classifier.queue_ms"], 1.5);
    assert_eq!(metrics["atlas.classifier.p50_ms"], 2.0);
    assert_eq!(
        metrics["atlas.classifier.p95_ms"], 64.0,
        "a 64-item batch is not silently divided into a per-article latency"
    );
    assert_eq!(metrics["atlas.classifier.latency_unit"], "batch");
    assert_eq!(metrics["atlas.classifier.samples"], 3);
    assert_eq!(metrics["atlas.classifier.batch_sizes"][0], 2);
    assert_eq!(metrics["atlas.classifier.batch_sizes"][63], 1);
    after.queue_micros = 100;
    let reset = classifier_worker_metrics(Some(&before), Ok(&after));
    assert!(reset["atlas.classifier.queue_ms"].is_null());
    assert!(reset["atlas.classifier.queue_reason"].as_str().is_some());
    let missing = classifier_worker_metrics(None, Err("worker_performance_unavailable"));
    assert!(missing["atlas.classifier.p95_ms"].is_null());
    assert!(missing["atlas.classifier.js_heap_current_bytes"].is_null());
    assert_eq!(
        missing["atlas.classifier.unavailable_reason"],
        "worker_performance_unavailable"
    );
}

#[test]
fn provider_diagnostics_never_reflect_credentials_response_text_or_urls() {
    let secret = "fixture-credential-never-log";
    let raw=format!("{{\"error\":\"quota exceeded {secret}\",\"url\":\"https://fixture.invalid/?api_key={secret}\"}}");
    for status in [401, 403, 429, 500] {
        let fault = provider_fault("newsdata", status, &raw);
        let shown = format!("{fault:?}");
        assert!(!shown.contains(secret));
        assert!(!shown.contains("https://"));
        assert!(!fault.body.is_empty());
    }
    let event = parse_fault(
        "newsdata",
        format!("error {secret}"),
        &serde_json::json!({"Authorization":secret,"response":{"token":secret}}),
    );
    let AtlasEvent::Fault(fault) = event else {
        panic!("schema failure must produce a typed fault")
    };
    assert!(!format!("{fault:?}").contains(secret));
    let body: serde_json::Value = serde_json::from_str(&fault.body).unwrap();
    assert_eq!(body["code"], "invalid_response_schema");
    assert_eq!(body["top_level_fields"], 2);
}

#[tokio::test]
async fn provider_counts_measure_actual_transport_retry_and_redirect_edges() {
    let mut replies = std::collections::VecDeque::from([
        HttpReply {
            status: 503,
            body: "temporary fixture failure".into(),
            location: None,
        },
        HttpReply {
            status: 302,
            body: String::new(),
            location: Some("/next".into()),
        },
        HttpReply {
            status: 200,
            body: "{\"ok\":true}".into(),
            location: None,
        },
    ]);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = calls.clone();
    let mut fetch = move |_call: HttpCall| -> FetchFut {
        counted.fetch_add(1, Ordering::SeqCst);
        let reply = replies.pop_front().expect("only scripted transport calls");
        Box::pin(async move { Ok(reply) })
    };
    let mut observed = ProviderCallCounts::default();
    let value = dispatch_counted(
        &mut fetch,
        CallSpec {
            provider: "newsdata",
            url: "https://fixture.invalid/start".into(),
            headers: Vec::new(),
        },
        "fixture",
        &mut observed,
    )
    .await
    .unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(observed.calls, 3);
    assert_eq!(observed.transport_retries, 1);
    assert_eq!(observed.redirects, 1);
    assert_eq!(
        observed.fallbacks, 0,
        "fallback-account attempts are counted only by their real caller edge"
    );
}
