//! Wire-level request-policy fixtures (spec §7: stage ceilings for
//! Google/Nvidia/OpenRouter, reasoning options, unsupported-option recovery).
use super::mock::*;
use super::*;
use crate::request_policy::{ReasoningEffort, RequestPolicy};

fn msgs() -> Vec<ChatMessage> {
    vec![ChatMessage {
        role: "user".into(),
        content: "Extract".into(),
        tool_call_id: None,
        tool_calls: Vec::new(),
    }]
}

fn body_of(raw: &str) -> Value {
    serde_json::from_str(&raw[raw.find("\r\n\r\n").unwrap() + 4..]).unwrap()
}

fn kinded(base: &str, kind: &str, model: &str) -> ProviderSecret {
    let mut s = secret(base);
    s.kind = kind.into();
    s.model = model.into();
    s
}

async fn send(secret: &ProviderSecret, policy: &RequestPolicy) -> AttemptReport {
    attempt_insight(secret, &msgs(), Transport::NonStream, policy, None, None).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stage_ceilings_reach_google_nvidia_and_openrouter_on_the_wire() {
    for (kind, model, json) in [
        ("google", "gemini-wire-ceiling", false),
        ("nvidia", "nv/wire-ceiling", false),
        ("openrouter", "or/wire-ceiling", true),
    ] {
        let server = serve(vec![ok_json("{\"claims\":[]}", "stop")]).await;
        let policy = RequestPolicy {
            max_output_tokens: Some(2048),
            json_object: true,
            reasoning: ReasoningEffort::Default,
        };
        let report = send(&kinded(&server.base_url, kind, model), &policy).await;
        assert!(report.outcome.is_ok(), "{kind}");
        let body = body_of(&server.requests.lock().unwrap()[0]);
        assert_eq!(body["max_tokens"], 2048, "{kind}");
        assert_eq!(body.get("response_format").is_some(), json, "{kind}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reasoning_effort_reaches_the_wire_with_reserve() {
    let server = serve(vec![ok_json("{}", "stop")]).await;
    let policy = RequestPolicy {
        max_output_tokens: Some(2048),
        json_object: true,
        reasoning: ReasoningEffort::Low,
    };
    send(
        &kinded(&server.base_url, "openrouter", "or/wire-reasoning"),
        &policy,
    )
    .await;
    let body = body_of(&server.requests.lock().unwrap()[0]);
    assert_eq!(body["reasoning"]["effort"], "low");
    assert_eq!(body["max_tokens"], 2048 + 1024);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsupported_option_is_learned_and_resent_once() {
    let reject = Reply::Json(
        400,
        r#"{"error":{"message":"Unrecognized request argument supplied: reasoning_effort"}}"#
            .into(),
    );
    let server = serve(vec![reject, ok_json("{}", "stop")]).await;
    let secret = kinded(&server.base_url, "google", "gemini-wire-recovery");
    let policy = RequestPolicy {
        max_output_tokens: Some(512),
        json_object: true,
        reasoning: ReasoningEffort::Low,
    };
    let report = send(&secret, &policy).await;
    assert!(report.outcome.is_ok(), "{:?}", report.outcome.err());
    assert_eq!(server.hits(), 2);
    let requests = server.requests.lock().unwrap().clone();
    assert_eq!(body_of(&requests[0])["reasoning_effort"], "low");
    let resent = body_of(&requests[1]);
    assert!(resent.get("reasoning_effort").is_none());
    assert_eq!(resent["max_tokens"], 512, "ceiling kept, reserve dropped");
    assert!(report
        .policy_diagnostics
        .iter()
        .any(|d| d.contains("reasoning_effort rejected")));
    // The next dispatch on the same route skips the field before sending.
    let report = send(&secret, &policy).await;
    assert!(report.outcome.is_ok());
    assert_eq!(server.hits(), 3);
    let third = body_of(&server.requests.lock().unwrap()[2]);
    assert!(third.get("reasoning_effort").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unrelated_400_is_not_retried() {
    let reject = Reply::Json(
        400,
        r#"{"error":{"message":"context length exceeded"}}"#.into(),
    );
    let server = serve(vec![reject, ok_json("{}", "stop")]).await;
    let policy = RequestPolicy {
        max_output_tokens: Some(512),
        json_object: false,
        reasoning: ReasoningEffort::Low,
    };
    let report = send(
        &kinded(&server.base_url, "google", "gemini-wire-400"),
        &policy,
    )
    .await;
    assert!(report.outcome.is_err());
    assert_eq!(server.hits(), 1);
}
