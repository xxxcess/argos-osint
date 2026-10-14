//! Wire-level adapter fixtures through the shared mock provider (spec §9:
//! per-capability native/chat/schema/tool bodies, declared total ceilings,
//! scoped unsupported-option learning, typed native errors).
use super::mock::*;
use super::*;
use crate::provider_adapters::{ResponseContract, OUTPUT_TOOL};
use crate::request_policy::{ReasoningEffort, RequestPolicy};

fn msgs() -> Vec<ChatMessage> {
    vec![
        ChatMessage {
            role: "system".into(),
            content: "Rules".into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
        ChatMessage {
            role: "user".into(),
            content: "Extract".into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
    ]
}

fn body_of(raw: &str) -> Value {
    serde_json::from_str(&raw[raw.find("\r\n\r\n").unwrap() + 4..]).unwrap()
}

fn request_line(raw: &str) -> &str {
    raw.lines().next().unwrap_or("")
}

fn kinded(base: &str, kind: &str, model: &str) -> ProviderSecret {
    let mut s = secret(base);
    s.kind = kind.into();
    s.model = model.into();
    s
}

fn policy(effort: ReasoningEffort) -> RequestPolicy {
    RequestPolicy {
        max_output_tokens: Some(2048),
        thinking_ceiling: Some(4096),
        json_object: true,
        reasoning: effort,
        ..Default::default()
    }
}

fn contract() -> ResponseContract {
    crate::intel_recon::brief_contracts::claims_contract(&["t1"])
}

async fn send(secret: &ProviderSecret, policy: &RequestPolicy) -> AttemptReport {
    attempt_insight(secret, &msgs(), Transport::NonStream, policy, None, None).await
}

async fn send_contract(
    secret: &ProviderSecret,
    policy: &RequestPolicy,
    transport: Transport,
) -> AttemptReport {
    attempt_contract(
        secret,
        &msgs(),
        transport,
        Deadlines::insight_non_stream(),
        None,
        None,
        policy,
        &contract(),
    )
    .await
}

const CLAIMS: &str = r#"{"claims":[],"limit_reached":false,"empty_reason":"none"}"#;

fn tool_reply(args: &str) -> Reply {
    Reply::Json(
        200,
        serde_json::json!({"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","content":null,
            "tool_calls":[{"id":"call_1","type":"function","function":{"name":OUTPUT_TOOL,"arguments":args}}]}}],
            "usage":{"prompt_tokens":40,"completion_tokens":12}})
        .to_string(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stage_ceilings_reach_google_nvidia_and_openrouter_on_the_wire() {
    // Super: strict schema + require_parameters; declared ceiling, no reserve.
    let server = serve(vec![ok_json(CLAIMS, "stop")]).await;
    let s = kinded(
        &server.base_url,
        "openrouter",
        "nvidia/nemotron-3-super-120b-a12b:free",
    );
    let report = send_contract(&s, &policy(ReasoningEffort::Default), Transport::NonStream).await;
    assert!(report.outcome.is_ok(), "{:?}", report.outcome.err());
    let body = body_of(&server.requests.lock().unwrap()[0]);
    assert_eq!(body["max_tokens"], 2048);
    assert_eq!(body["response_format"]["type"], "json_schema");
    assert_eq!(body["provider"]["require_parameters"], true);
    assert!(body.get("temperature").is_none());
    assert_eq!(server.hits(), 1);

    // Ultra: one forced output tool; its arguments are the answer (1 request).
    let server = serve(vec![tool_reply(CLAIMS)]).await;
    let s = kinded(
        &server.base_url,
        "openrouter",
        "nvidia/nemotron-3-ultra-550b-a55b:free",
    );
    let report = send_contract(&s, &policy(ReasoningEffort::Default), Transport::NonStream).await;
    let c = report.outcome.expect("output tool answer");
    assert_eq!(c.content, CLAIMS);
    assert_eq!(c.usage.unwrap().prompt_tokens, Some(40));
    assert_eq!(server.hits(), 1, "no second round trip for the output tool");
    let body = body_of(&server.requests.lock().unwrap()[0]);
    assert!(body.get("response_format").is_none());
    assert_eq!(body["tool_choice"]["function"]["name"], OUTPUT_TOOL);
    assert_eq!(
        report.effective.unwrap().structured,
        crate::provider_adapters::StructuredMode::OutputTool
    );

    // Nvidia Ultra hosted: model-specific effort and the thinking ceiling.
    let server = serve(vec![ok_json(CLAIMS, "stop")]).await;
    let s = kinded(
        &server.base_url,
        "nvidia",
        "nvidia/nemotron-3-ultra-550b-a55b",
    );
    let report = send(&s, &policy(ReasoningEffort::Efficient)).await;
    assert!(report.outcome.is_ok());
    let body = body_of(&server.requests.lock().unwrap()[0]);
    assert_eq!(body["reasoning_effort"], "medium");
    assert_eq!(body["max_tokens"], 4096);
    assert!(
        body.get("response_format").is_none(),
        "unconfirmed JSON mode omitted"
    );

    // Google native Flash on an explicit native base.
    let reply = Reply::Json(
        200,
        serde_json::json!({"candidates":[{"content":{"parts":[{"text":CLAIMS}]},"finishReason":"STOP"}],
            "usageMetadata":{"promptTokenCount":9,"candidatesTokenCount":4,"thoughtsTokenCount":6}})
        .to_string(),
    );
    let server = serve(vec![reply]).await;
    let s = kinded(&server.base_url, "google", "models/gemini-3.8-flash");
    let report = send_contract(&s, &policy(ReasoningEffort::Off), Transport::NonStream).await;
    let c = report.outcome.expect("native answer");
    assert_eq!(c.content, CLAIMS);
    assert_eq!(c.usage.unwrap().reasoning_tokens, Some(6));
    let raw = server.requests.lock().unwrap()[0].clone();
    assert!(
        request_line(&raw).starts_with("POST /v1/models/gemini-3.8-flash:generateContent "),
        "{}",
        request_line(&raw)
    );
    assert!(raw.to_ascii_lowercase().contains("x-goog-api-key: sk-mock"));
    assert!(!raw.to_ascii_lowercase().contains("authorization: bearer"));
    let body = body_of(&raw);
    assert_eq!(
        body["generationConfig"]["thinkingConfig"]["thinkingLevel"],
        "low"
    );
    assert_eq!(body["generationConfig"]["maxOutputTokens"], 4096);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reasoning_effort_reaches_the_wire_with_reserve() {
    // Renamed semantics: the declared total ceiling, never an implicit reserve.
    let server = serve(vec![ok_json("{}", "stop")]).await;
    let s = kinded(
        &server.base_url,
        "nvidia",
        "nvidia/nemotron-3-super-120b-a12b",
    );
    send(&s, &policy(ReasoningEffort::Efficient)).await;
    let body = body_of(&server.requests.lock().unwrap()[0]);
    assert_eq!(body["reasoning_effort"], "low");
    assert_eq!(
        body["max_tokens"], 4096,
        "declared thinking ceiling, no reserve"
    );
    // An unknown model receives no unverified reasoning field.
    let server = serve(vec![ok_json("{}", "stop")]).await;
    let s = kinded(&server.base_url, "nvidia", "vendor/unseen");
    let report = send(&s, &policy(ReasoningEffort::High)).await;
    let body = body_of(&server.requests.lock().unwrap()[0]);
    assert!(body.get("reasoning_effort").is_none());
    assert_eq!(body["max_tokens"], 2048);
    assert!(report
        .policy_diagnostics
        .iter()
        .any(|d| d.contains("unsupported-policy:reasoning=high")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsupported_option_is_learned_and_resent_once() {
    let reject = Reply::Json(
        400,
        r#"{"error":{"message":"Invalid value for reasoning_effort: 'medium' is not supported"}}"#
            .into(),
    );
    let server = serve(vec![reject, ok_json("{}", "stop")]).await;
    let secret = kinded(
        &server.base_url,
        "nvidia",
        "nvidia/nemotron-3-ultra-550b-a55b",
    );
    let report = send(&secret, &policy(ReasoningEffort::Efficient)).await;
    assert!(report.outcome.is_ok(), "{:?}", report.outcome.err());
    assert_eq!(server.hits(), 2);
    let requests = server.requests.lock().unwrap().clone();
    assert_eq!(body_of(&requests[0])["reasoning_effort"], "medium");
    let resent = body_of(&requests[1]);
    assert!(resent.get("reasoning_effort").is_none());
    assert!(resent.get("max_tokens").is_some(), "ceiling never removed");
    assert!(report
        .policy_diagnostics
        .iter()
        .any(|d| d.contains("reasoning_effort=medium rejected")));
    // The next dispatch skips that value before sending, but the field's
    // other valid values remain usable.
    let report = send(&secret, &policy(ReasoningEffort::Efficient)).await;
    assert!(report.outcome.is_ok());
    assert_eq!(server.hits(), 3);
    assert!(body_of(&server.requests.lock().unwrap()[2])
        .get("reasoning_effort")
        .is_none());
    let report = send(&secret, &policy(ReasoningEffort::Off)).await;
    assert!(report.outcome.is_ok());
    assert_eq!(
        body_of(&server.requests.lock().unwrap()[3])["reasoning_effort"],
        "none"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unrelated_400_is_not_retried() {
    let reject = Reply::Json(
        400,
        r#"{"error":{"message":"context length exceeded"}}"#.into(),
    );
    let server = serve(vec![reject, ok_json("{}", "stop")]).await;
    let report = send(
        &kinded(
            &server.base_url,
            "nvidia",
            "nvidia/nemotron-3-super-120b-a12b",
        ),
        &policy(ReasoningEffort::Efficient),
    )
    .await;
    assert!(report.outcome.is_err());
    assert_eq!(server.hits(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decision_only_target_fails_before_network_access() {
    let server = serve(vec![ok_json("{}", "stop")]).await;
    let jev = kinded(&server.base_url, "openrouter", "typesafe/jev-1.13");
    let report = send(&jev, &policy(ReasoningEffort::Default)).await;
    let f = report.outcome.unwrap_err();
    assert_eq!(f.category, Category::Unsupported);
    assert!(!f.retryable);
    assert!(f.message.contains("capability skip"));
    let err = crate::provider::complete_one(&jev, &msgs(), &[])
        .await
        .unwrap_err();
    let f = err.downcast_ref::<ProviderFailure>().expect("typed");
    assert_eq!(f.category, Category::Unsupported);
    assert!(f.message.contains("decision-only"), "{}", f.message);
    assert_eq!(server.hits(), 0, "zero dispatched requests");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streamed_output_tool_with_fragmented_arguments_needs_one_request() {
    let ev = |v: Value| format!("data: {v}\n\n");
    let chunks = vec![
        ev(serde_json::json!({"choices":[{"delta":{"role":"assistant","content":""}}]})),
        ev(
            serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":OUTPUT_TOOL,"arguments":"{\"claims\":[],"}}]}}]}),
        ),
        ev(
            serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"limit_reached\":false,\"empty_reason\":\"none\"}"}}]}}]}),
        ),
        ev(serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]})),
        ev(serde_json::json!({"choices":[],"usage":{"prompt_tokens":5,"completion_tokens":3}})),
        "data: [DONE]\n\n".to_string(),
    ];
    let server = serve(vec![Reply::Sse(chunks)]).await;
    let s = kinded(
        &server.base_url,
        "openrouter",
        "cohere/north-mini-code:free",
    );
    let report = send_contract(&s, &policy(ReasoningEffort::Default), Transport::Stream).await;
    let c = report.outcome.expect("complete output tool");
    assert_eq!(c.content, CLAIMS);
    assert_eq!(c.usage.unwrap().completion_tokens, Some(3));
    assert_eq!(server.hits(), 1);

    // Two output calls, an unknown tool, or partial JSON are rejected.
    for args in [r#"{"claims":["#] {
        let server = serve(vec![tool_reply(args)]).await;
        let s = kinded(
            &server.base_url,
            "openrouter",
            "cohere/north-mini-code:free",
        );
        let report =
            send_contract(&s, &policy(ReasoningEffort::Default), Transport::NonStream).await;
        assert_eq!(
            report.outcome.unwrap_err().category,
            Category::InvalidResult
        );
    }
    let multi = Reply::Json(
        200,
        serde_json::json!({"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","content":null,
            "tool_calls":[{"id":"a","type":"function","function":{"name":OUTPUT_TOOL,"arguments":"{}"}},
                          {"id":"b","type":"function","function":{"name":"search","arguments":"{}"}}]}}]})
        .to_string(),
    );
    let server = serve(vec![multi]).await;
    let s = kinded(
        &server.base_url,
        "openrouter",
        "cohere/north-mini-code:free",
    );
    let report = send_contract(&s, &policy(ReasoningEffort::Default), Transport::NonStream).await;
    assert!(report.outcome.unwrap_err().message.contains("got 2"));
    // A truncated stream is never accepted.
    let server = serve(vec![Reply::Truncated(format!(
        "data: {}\n\n",
        serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":OUTPUT_TOOL,"arguments":"{\"claims\""}}]}}]})
    ))])
    .await;
    let s = kinded(
        &server.base_url,
        "openrouter",
        "cohere/north-mini-code:free",
    );
    let report = send_contract(&s, &policy(ReasoningEffort::Default), Transport::Stream).await;
    assert!(report.outcome.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn google_native_stream_uses_sse_and_reads_usage_metadata() {
    let ev = |v: Value| format!("data: {v}\r\n\r\n");
    let chunks = vec![
        ev(
            serde_json::json!({"candidates":[{"content":{"role":"model","parts":[{"text":"plan","thought":true}]}}]}),
        ),
        ev(
            serde_json::json!({"candidates":[{"content":{"role":"model","parts":[{"text":"Hello "}]}}]}),
        ),
        ev(
            serde_json::json!({"candidates":[{"content":{"role":"model","parts":[{"text":"world"}]},"finishReason":"STOP"}],
            "usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":2,"thoughtsTokenCount":1}}),
        ),
    ];
    let server = serve(vec![Reply::Sse(chunks)]).await;
    let s = kinded(&server.base_url, "google", "models/gemma-4-26b-a4b-it");
    let report = attempt_insight(
        &s,
        &msgs(),
        Transport::Stream,
        &RequestPolicy::default(),
        None,
        None,
    )
    .await;
    let c = report.outcome.expect("native stream");
    assert_eq!(c.content, "Hello world");
    assert_eq!(c.reasoning, "plan");
    assert_eq!(c.usage.unwrap().visible_tokens, Some(2));
    let raw = server.requests.lock().unwrap()[0].clone();
    assert!(request_line(&raw)
        .starts_with("POST /v1/models/gemma-4-26b-a4b-it:streamGenerateContent?alt=sse "));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_jev_http_errors_are_typed() {
    for (status, category) in [
        (400u16, Category::MalformedRequest),
        (401, Category::Auth),
        (429, Category::RateLimited),
        (504, Category::Server),
    ] {
        let body = format!(r#"{{"error":{{"code":"e{status}","message":"failed now"}}}}"#);
        let reply = Reply::Raw(format!(
            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\nx-request-id: req-{status}\r\nretry-after: 7\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        ));
        let server = serve(vec![reply]).await;
        let jev = kinded(&server.base_url, "openrouter", "typesafe/jev-1.13");
        let err = crate::provider::decide(&jev, &serde_json::json!({}), &serde_json::json!({}))
            .await
            .unwrap_err();
        let f = err
            .downcast_ref::<ProviderFailure>()
            .unwrap_or_else(|| panic!("{status} untyped: {err}"));
        assert_eq!(f.http_status, Some(status));
        assert_eq!(f.category, category, "{status}");
        assert_eq!(f.request_id.as_deref(), Some(&*format!("req-{status}")));
        if status == 429 {
            assert_eq!(f.retry_after_ms, Some(7000));
        }
        let raw = server.requests.lock().unwrap()[0].clone();
        assert!(request_line(&raw).starts_with("POST /alpha/decisions "));
    }
}
