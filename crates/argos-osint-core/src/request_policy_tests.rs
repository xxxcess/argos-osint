use super::*;
use serde_json::json;

fn body() -> Value {
    json!({"model": "m", "messages": []})
}

fn policy(tokens: u32, reasoning: ReasoningEffort) -> RequestPolicy {
    RequestPolicy {
        max_output_tokens: Some(tokens),
        json_object: true,
        reasoning,
    }
}

#[test]
fn output_ceiling_is_decoupled_from_json_mode() {
    // Google and Nvidia are ValidatedText (no JSON mode) yet receive ceilings.
    for kind in ["google", "nvidia", "openrouter", "grok"] {
        let mut b = body();
        apply_for(
            kind,
            "policy-ceiling-model",
            &mut b,
            &policy(2048, ReasoningEffort::Default),
        );
        assert_eq!(b["max_tokens"], 2048, "{kind}");
    }
    let mut b = body();
    apply_for(
        "openai",
        "gpt-4o",
        &mut b,
        &policy(900, ReasoningEffort::Default),
    );
    assert_eq!(b["max_completion_tokens"], 900);
    assert!(b.get("max_tokens").is_none());
}

#[test]
fn unknown_transport_keeps_local_validation_with_diagnostic() {
    let mut b = body();
    let d = apply_for("local", "x", &mut b, &policy(512, ReasoningEffort::Low));
    assert_eq!(b, body());
    assert!(d
        .iter()
        .any(|x| x == "unsupported-policy:max_output_tokens"));
    assert!(d.iter().any(|x| x == "unsupported-policy:reasoning=low"));
}

#[test]
fn json_object_only_where_supported() {
    let mut b = body();
    apply_for(
        "openrouter",
        "a/b",
        &mut b,
        &policy(1, ReasoningEffort::Default),
    );
    assert_eq!(b["response_format"]["type"], "json_object");
    let mut b = body();
    let d = apply_for(
        "nvidia",
        "a/b",
        &mut b,
        &policy(1, ReasoningEffort::Default),
    );
    assert!(b.get("response_format").is_none());
    assert!(d
        .iter()
        .any(|x| x.starts_with("unsupported-policy:response_format")));
}

#[test]
fn reasoning_dialects_and_reserve() {
    let mut b = body();
    apply_for(
        "openrouter",
        "r1",
        &mut b,
        &policy(2048, ReasoningEffort::Low),
    );
    assert_eq!(b["reasoning"], json!({"effort": "low"}));
    assert_eq!(b["max_tokens"], 2048 + 1024);

    let mut b = body();
    apply_for(
        "openrouter",
        "r1",
        &mut b,
        &policy(2048, ReasoningEffort::Off),
    );
    assert_eq!(b["reasoning"], json!({"enabled": false}));
    assert_eq!(b["max_tokens"], 2048);

    let mut b = body();
    apply_for(
        "google",
        "gemini-2.5-flash",
        &mut b,
        &policy(1000, ReasoningEffort::Medium),
    );
    assert_eq!(b["reasoning_effort"], "medium");
    assert_eq!(b["max_tokens"], 1000 + 4096);

    let mut b = body();
    apply_for(
        "google",
        "gemini-2.5-flash",
        &mut b,
        &policy(1000, ReasoningEffort::Off),
    );
    assert_eq!(b["reasoning_effort"], "none");

    let mut b = body();
    let d = apply_for("nvidia", "n", &mut b, &policy(1000, ReasoningEffort::High));
    assert!(b.get("reasoning").is_none() && b.get("reasoning_effort").is_none());
    assert_eq!(
        b["max_tokens"], 1000,
        "no reserve when reasoning is not sent"
    );
    assert!(d.iter().any(|x| x == "unsupported-policy:reasoning=high"));

    let mut b = body();
    apply_for(
        "openai",
        "o3-mini",
        &mut b,
        &policy(500, ReasoningEffort::High),
    );
    assert_eq!(b["reasoning_effort"], "high");
    assert_eq!(b["max_completion_tokens"], 500 + 8192);
    let mut b = body();
    apply_for(
        "openai",
        "gpt-4o",
        &mut b,
        &policy(500, ReasoningEffort::High),
    );
    assert!(b.get("reasoning_effort").is_none());
}

#[test]
fn default_policy_is_a_no_op() {
    let mut b = body();
    let d = apply_for("openrouter", "a", &mut b, &RequestPolicy::default());
    assert_eq!(b, body());
    assert!(d.is_empty());
}

#[test]
fn learned_rejections_are_skipped_per_route() {
    learn_unsupported("google", "learn-test-model", FIELD_REASONING_EFFORT);
    let mut b = body();
    let d = apply_for(
        "google",
        "learn-test-model",
        &mut b,
        &policy(64, ReasoningEffort::Low),
    );
    assert!(b.get("reasoning_effort").is_none());
    assert_eq!(b["max_tokens"], 64);
    assert!(d.iter().any(|x| x.contains("rejected earlier")));
    // Another model on the same provider is unaffected.
    let mut b = body();
    apply_for(
        "google",
        "other-learn-model",
        &mut b,
        &policy(64, ReasoningEffort::Low),
    );
    assert_eq!(b["reasoning_effort"], "low");
}

#[test]
fn rejected_fields_parse_provider_messages() {
    let b = json!({"reasoning_effort": "low", "max_tokens": 1});
    assert_eq!(
        rejected_fields(
            &b,
            "Unrecognized request argument supplied: reasoning_effort"
        ),
        vec![FIELD_REASONING_EFFORT]
    );
    assert!(rejected_fields(&b, "max_tokens is too large: 99999 > 8192").is_empty());
    assert!(rejected_fields(&b, "rate limited").is_empty());
    let b = json!({"response_format": {"type": "json_object"}});
    assert_eq!(
        rejected_fields(&b, "JSON mode is not supported for this model"),
        vec![FIELD_RESPONSE_FORMAT]
    );
}
