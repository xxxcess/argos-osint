//! Capability routing (spec §3, acceptance case 1/2): Jev family detection,
//! capability skips that send zero requests, first compatible chat route for
//! generated JSON, and native decisions only on supported connections.

use super::*;
use crate::provider::{ModelAssignment, ModelRoute};
use crate::provider_attempt::mock;
use crate::secrets::ProviderSecret;

fn auth(base: &str, openrouter_key: Option<&str>) -> AuthFile {
    let mut auth = AuthFile::default();
    auth.set_account(ProviderSecret {
        kind: "local".into(),
        model: "fixture".into(),
        base_url: base.into(),
        api_key: Some("fixture-only".into()),
        stt_model: None,
        device: None,
    });
    auth.set_account(ProviderSecret {
        kind: "openrouter".into(),
        model: "typesafe/jev-1.13".into(),
        base_url: base.into(),
        api_key: openrouter_key.map(str::to_string),
        stt_model: None,
        device: None,
    });
    auth
}

fn route(provider: &str, model: &str) -> ModelRoute {
    ModelRoute {
        provider: provider.into(),
        model: model.into(),
        ..Default::default()
    }
}

fn assessor(primary: (&str, &str), fallbacks: &[(&str, &str)]) -> SettingsFile {
    let mut settings = SettingsFile::default();
    settings.defaults.claim_assessor = ModelAssignment {
        provider: primary.0.into(),
        model: primary.1.into(),
        account: String::new(),
        fallbacks: fallbacks.iter().map(|(p, m)| route(p, m)).collect(),
    };
    settings
}

fn user(text: &str) -> Vec<ChatMessage> {
    vec![ChatMessage {
        role: "user".into(),
        content: text.into(),
        tool_call_id: None,
        tool_calls: vec![],
    }]
}

#[test]
fn jev_family_detection_is_exact() {
    for id in [
        "typesafe/jev",
        "typesafe/jev-1.13",
        "  TypeSafe/JEV-1.13 ",
        "~typesafe/jev-latest",
        "typesafe/jev-2",
    ] {
        assert!(provider::is_jev_family(id), "{id}");
        assert!(provider::is_decisions_model(id), "{id}");
    }
    for id in [
        "acme/jevons-7b",
        "someone/jev",
        "typesafe/jevons",
        "nvidia/nemotron-3-super-120b-a12b:free",
        "openai/gpt-jev",
        "typesafe/jev-beta",
        "",
    ] {
        assert!(!provider::is_jev_family(id), "{id}");
    }
    assert_eq!(provider::picker_transport("acme/jevons-7b"), "chat");
}

#[tokio::test]
async fn claim_assessor_extraction_skips_jev_and_sends_one_request_to_super() {
    let server = mock::serve(vec![mock::ok_json("{}", "stop")]).await;
    let auth = auth(&server.base_url, Some("or-fixture"));
    let settings = assessor(
        ("openrouter", "typesafe/jev-1.13"),
        &[
            ("openrouter", " ~TypeSafe/JEV-latest "),
            ("local", "super"),
            ("local", "ultra"),
        ],
    );
    let cancel = Arc::new(AtomicBool::new(false));
    let (_, report) = execute_chat(
        &auth,
        &settings,
        &OperationScope::new("extract-fixture", "claim_assessor"),
        &user("Extract claims as JSON"),
        |raw| {
            serde_json::from_str::<Value>(raw)
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        &cancel,
        None,
        None,
        None,
        false,
    )
    .await
    .unwrap();
    assert_eq!(server.hits(), 1, "exactly one chat request, zero Jev");
    assert_eq!(report.effective_route, Some(2), "original route index kept");
    let body = server.requests.lock().unwrap()[0].clone();
    assert!(body.contains("\"model\":\"super\""), "{body}");
    assert!(body.contains("\"messages\""));
    assert!(!body.to_ascii_lowercase().contains("jev"));
}

#[tokio::test]
async fn all_jev_extraction_chain_fails_before_dispatch_with_specific_error() {
    let server = mock::serve(vec![]).await;
    let auth = auth(&server.base_url, Some("or-fixture"));
    let settings = assessor(
        ("openrouter", "typesafe/jev-1.13"),
        &[("openrouter", "~typesafe/jev-latest")],
    );
    let cancel = Arc::new(AtomicBool::new(false));
    let error = execute_chat(
        &auth,
        &settings,
        &OperationScope::new("extract-all-jev", "claim_assessor"),
        &user("Extract"),
        |_| Ok(()),
        &cancel,
        None,
        None,
        None,
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Claim Assessor extraction requires a non-Jev chat fallback"
    );
    assert_eq!(server.hits(), 0);
}

struct CountAnswers;
impl DecisionsAdapter<usize> for CountAnswers {
    fn compile_native(&self) -> (Value, Value) {
        (
            serde_json::json!({"claim":"Acme opened a plant"}),
            serde_json::json!({"q1":{"type":"choice","question":"Supported?","options":{"supported":"Passage states it","insufficient":"Not stated"}}}),
        )
    }
    fn parse_native(&self, resp: &provider::DecisionsResponse) -> Result<usize, String> {
        Ok(resp.answers.len())
    }
    fn compile_chat(&self) -> Vec<ChatMessage> {
        user("Answer {\"answers\":{\"q1\":{\"label\":\"supported|insufficient\"}}}")
    }
    fn parse_chat(&self, raw: &str) -> Result<usize, String> {
        let v: Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
        Ok(v["answers"].as_object().map_or(0, |m| m.len()))
    }
}

fn decisions_reply() -> mock::Reply {
    mock::Reply::Json(
        200,
        serde_json::json!({"id":"d1","model":"typesafe/jev-1.13","answers":{"q1":{"type":"choice","choice":"supported","probabilities":{"supported":0.8,"insufficient":0.2}}}}).to_string(),
    )
}

#[tokio::test]
async fn wrong_provider_jev_is_skipped_and_native_jev_gets_state_and_questions() {
    let server = mock::serve(vec![decisions_reply()]).await;
    let auth = auth(&server.base_url, Some("or-fixture"));
    // Super → Jev(wrong provider) → Jev(OpenRouter): assessment (decision)
    // skips the incompatible Jev without sending it to chat.
    let settings = assessor(
        ("local", "typesafe/jev-1.13"),
        &[("openrouter", "typesafe/jev-1.13")],
    );
    let cancel = Arc::new(AtomicBool::new(false));
    let (answers, report) = execute_decisions_or_chat(
        &auth,
        &settings,
        &OperationScope::new("assess-fixture", "claim_assessor"),
        &CountAnswers,
        &cancel,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(answers, 1);
    assert_eq!(server.hits(), 1);
    assert_eq!(report.effective_route, Some(1));
    let body: Value = serde_json::from_str(
        server.requests.lock().unwrap()[0]
            .split("\r\n\r\n")
            .last()
            .unwrap(),
    )
    .unwrap();
    assert!(
        body.get("messages").is_none(),
        "native has no chat messages"
    );
    assert!(body["questions"]["q1"].is_object());
    assert!(body["state"].is_object());
}

#[tokio::test]
async fn jev_with_missing_key_is_a_capability_skip_not_a_chat_request() {
    let server = mock::serve(vec![mock::ok_json(
        "{\"answers\":{\"q1\":{\"label\":\"insufficient\"}}}",
        "stop",
    )])
    .await;
    let auth = auth(&server.base_url, None);
    let settings = assessor(("openrouter", "typesafe/jev-1.13"), &[("local", "super")]);
    let cancel = Arc::new(AtomicBool::new(false));
    let (answers, report) = execute_decisions_or_chat(
        &auth,
        &settings,
        &OperationScope::new("assess-missing-key", "claim_assessor"),
        &CountAnswers,
        &cancel,
        None,
        None,
    )
    .await
    .unwrap();
    // Valid abstention ("insufficient") is a settled answer: one request only.
    assert_eq!(answers, 1);
    assert_eq!(server.hits(), 1);
    assert_eq!(report.effective_route, Some(1));
    let body = server.requests.lock().unwrap()[0].clone();
    assert!(body.contains("\"model\":\"super\""), "{body}");
}

// §8.6 (brief fidelity): Jev primary without a native connection is a
// capability skip; the compatible chat fallback carries the stage's 1024-token
// ceiling on JSON-mode transports (unknown endpoints get no unsupported
// options), and one successful call is never repeated.
#[tokio::test]
async fn brief_fidelity_falls_back_to_chat_with_the_stage_budget() {
    use crate::intel_recon::brief_fidelity::{FidelityDecisions, FidelityItem, FIDELITY_TOKENS};
    let server = mock::serve(vec![mock::ok_json(
        r#"{"items":[{"id":"c1","fidelity":"faithful"}]}"#,
        "stop",
    )])
    .await;
    let auth = auth(&server.base_url, Some("or-fixture"));
    let settings = assessor(
        ("local", "typesafe/jev-1.13"),
        &[("openrouter", "nvidia/nemotron-3-super-120b-a12b:free")],
    );
    let items = vec![FidelityItem {
        id: "c1".into(),
        kind: "claim",
        text: "The board suspended the ferry.".into(),
        passages: vec!["The board suspended the ferry service.".into()],
    }];
    let cancel = Arc::new(AtomicBool::new(false));
    let (labels, _) = execute_decisions_or_chat(
        &auth,
        &settings,
        &OperationScope::new("brief-fidelity", "claim_assessor")
            .with_completion_tokens(FIDELITY_TOKENS),
        &FidelityDecisions { items: &items },
        &cancel,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(labels.get("c1").map(String::as_str), Some("faithful"));
    assert_eq!(server.hits(), 1, "one chat request, zero Jev");
    let body = server.requests.lock().unwrap()[0].clone();
    assert!(
        body.contains(&format!("\"max_tokens\":{FIDELITY_TOKENS}")),
        "{body}"
    );
    assert!(body.contains("nemotron"), "{body}");
    assert!(!body.contains("typesafe/jev"));
}

// §7: an operation override routes only that operation; others inherit.
#[test]
fn operation_override_routes_only_its_operation() {
    let mut settings = assessor(("local", "fixture"), &[]);
    settings.defaults.synthesis = ModelAssignment {
        provider: "local".into(),
        model: "synth-model".into(),
        ..Default::default()
    };
    settings.defaults.operation_overrides.insert(
        provider::OP_LINKS_GENERATE.into(),
        ModelAssignment {
            provider: "openrouter".into(),
            model: "openai/gpt-4o-mini".into(),
            fallbacks: vec![route("local", "fixture")],
            ..Default::default()
        },
    );
    let a = auth("http://127.0.0.1:9", Some("fixture-only"));
    let models = |settings: &SettingsFile, op: Option<&str>| -> Vec<String> {
        resolve_operation_routes(&a, settings, "synthesis", op)
            .0
            .into_iter()
            .map(|r| format!("{}:{}", r.provider, r.model))
            .collect()
    };
    assert_eq!(
        models(&settings, Some(provider::OP_LINKS_GENERATE)),
        ["openrouter:openai/gpt-4o-mini", "local:fixture"]
    );
    assert_eq!(
        models(&settings, Some(provider::OP_INFERENCES_GENERATE)),
        ["local:synth-model"]
    );
    assert_eq!(models(&settings, None), ["local:synth-model"]);
    settings
        .defaults
        .reset_operation(provider::OP_LINKS_GENERATE);
    assert_eq!(
        models(&settings, Some(provider::OP_LINKS_GENERATE)),
        ["local:synth-model"]
    );
}

// §3: every attempt record carries its dispatch declaration.
#[tokio::test]
async fn attempt_records_persist_the_dispatch_declaration() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("argos.db");
    drop(Store::open(&db).unwrap());
    let server = mock::serve(vec![mock::ok_json("{}", "stop")]).await;
    let auth = auth(&server.base_url, Some("or-fixture"));
    let mut settings = assessor(("local", "fixture"), &[]);
    settings.defaults.synthesis = ModelAssignment {
        provider: "local".into(),
        model: "synth".into(),
        ..Default::default()
    };
    settings.defaults.operation_overrides.insert(
        provider::OP_LINKS_GENERATE.into(),
        ModelAssignment {
            provider: "local".into(),
            model: "links-model".into(),
            ..Default::default()
        },
    );
    let cancel = Arc::new(AtomicBool::new(false));
    let scope = OperationScope::new("decl-op", "synthesis")
        .with_operation(provider::OP_LINKS_GENERATE)
        .with_format(ResponseFormat::GeneratedJson)
        .with_input_revision("rev-7");
    execute_chat(
        &auth,
        &settings,
        &scope,
        &user("Links as JSON"),
        |_| Ok(()),
        &cancel,
        None,
        None,
        Some(&db),
        false,
    )
    .await
    .unwrap();
    let store = Store::open(&db).unwrap();
    let (model, raw): (String, String) = store
        .conn
        .query_row(
            "SELECT model, declaration_json FROM recon_model_attempts WHERE operation_id='decl-op'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(model, "links-model");
    let d: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(d["logical_role"], "synthesis");
    assert_eq!(d["operation_key"], provider::OP_LINKS_GENERATE);
    assert_eq!(d["response_format"], "generated_json");
    assert_eq!(d["input_revision"], "rev-7");
    assert_eq!(d["overridden"], true);
    assert!(d["request_policy"].is_object());
}
