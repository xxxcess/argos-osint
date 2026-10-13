//! Operation format capability selection and single-secret caller alignment
//! (spec §2/§4).

use super::*;
use crate::provider::{ModelAssignment, ModelRoute};
use crate::provider_attempt::mock;

fn auth(base: &str, openrouter_key: Option<&str>) -> AuthFile {
    let mut auth = AuthFile::default();
    for (kind, key) in [
        ("local", Some("fixture-only")),
        ("openrouter", openrouter_key),
    ] {
        auth.set_account(ProviderSecret {
            kind: kind.into(),
            model: "fixture".into(),
            base_url: base.into(),
            api_key: key.map(str::to_string),
            stt_model: None,
            device: None,
        });
    }
    auth
}

fn assignment(primary: (&str, &str), fallbacks: &[(&str, &str)]) -> ModelAssignment {
    ModelAssignment {
        provider: primary.0.into(),
        model: primary.1.into(),
        account: String::new(),
        fallbacks: fallbacks
            .iter()
            .map(|(p, m)| ModelRoute {
                provider: (*p).into(),
                model: (*m).into(),
                ..Default::default()
            })
            .collect(),
    }
}

#[test]
fn generative_formats_skip_jev_and_decisions_need_native_connection() {
    let auth = auth("http://127.0.0.1:9", Some("or"));
    let mut settings = SettingsFile::default();
    settings.defaults.summarization =
        assignment(("openrouter", "typesafe/jev-1.13"), &[("local", "super")]);
    let s = select_role_secret(&auth, &settings, "summarization", OperationFormat::Prose).unwrap();
    assert_eq!(s.model, "super");
    let s = select_role_secret(
        &auth,
        &settings,
        "summarization",
        OperationFormat::BoundedDecision,
    )
    .unwrap();
    assert_eq!(s.model, "typesafe/jev-1.13");

    // Jev on the wrong provider is skipped for decisions; next chat route wins.
    settings.defaults.classifier =
        assignment(("local", "typesafe/jev-1.13"), &[("local", "super")]);
    let s = select_role_secret(
        &auth,
        &settings,
        "classifier",
        OperationFormat::BoundedDecision,
    )
    .unwrap();
    assert_eq!(s.model, "super");

    // All-Jev generative chain: nothing qualifies (no silent cross-role borrow).
    settings.defaults.entity_resolver = assignment(("openrouter", "typesafe/jev-1.13"), &[]);
    assert!(select_role_secret(
        &auth,
        &settings,
        "entity_resolver",
        OperationFormat::GeneratedJson
    )
    .is_none());
    assert!(OperationFormat::GeneratedJson
        .supports(&ProviderSecret {
            kind: "openrouter".into(),
            model: "acme/jevons-7b".into(),
            base_url: String::new(),
            api_key: None,
            stt_model: None,
            device: None,
        })
        .is_ok());
}

#[tokio::test]
async fn mode_classification_uses_full_classifier_chain_both_lanes() {
    use crate::intel_recon::modes::ReportMode;
    use crate::intel_recon::{classify_prompt_mode_for_role, PromptModeClassifyInput};
    let input = PromptModeClassifyInput {
        question: "What happened in Geneva?".into(),
        ..Default::default()
    };
    // Native Jev: questions object keyed by stable id, no chat messages.
    let server = mock::serve(vec![mock::Reply::Json(
        200,
        serde_json::json!({"answers":{"mode":{"type":"choice","choice":"assess_outlook"}}})
            .to_string(),
    )])
    .await;
    let auth_ok = auth(&server.base_url, Some("or"));
    let mut settings = SettingsFile::default();
    settings.defaults.classifier =
        assignment(("openrouter", "typesafe/jev-1.13"), &[("local", "super")]);
    assert_eq!(
        classify_prompt_mode_for_role(&auth_ok, &settings, &input).await,
        ReportMode::AssessOutlook
    );
    let raw = server.requests.lock().unwrap()[0].clone();
    let body: Value = serde_json::from_str(raw.split("\r\n\r\n").last().unwrap()).unwrap();
    assert!(body["questions"]["mode"]["options"].is_object());
    assert!(body.get("messages").is_none());

    // Missing OpenRouter key: Jev skipped, chat fallback compiled for chat.
    let server = mock::serve(vec![
        mock::ok_json(
            "{\"answers\":{\"mode\":{\"label\":\"full_assessment\"}}}",
            "stop",
        ),
        mock::ok_json("{\"answers\":{\"mode\":{\"label\":\"explain\"}}}", "stop"),
    ])
    .await;
    let auth_nokey = auth(&server.base_url, None);
    assert_eq!(
        classify_prompt_mode_for_role(&auth_nokey, &settings, &input).await,
        ReportMode::Explain
    );
    let reqs = server.requests.lock().unwrap().clone();
    assert!(reqs
        .iter()
        .all(|r| r.contains("\"messages\"") && r.contains("\"model\":\"super\"")));
    assert!(reqs.iter().all(|r| !r.contains("typesafe/jev")));
}

#[tokio::test]
async fn legacy_single_secret_mode_helper_never_sends_incapable_jev_to_chat() {
    use crate::intel_recon::{
        classify_prompt_mode, heuristic_prompt_mode, PromptModeClassifyInput,
    };
    let server = mock::serve(vec![]).await;
    let secret = ProviderSecret {
        kind: "local".into(),
        model: "typesafe/jev-1.13".into(),
        base_url: server.base_url.clone(),
        api_key: Some("k".into()),
        stt_model: None,
        device: None,
    };
    let input = PromptModeClassifyInput {
        question: "Is it true they met?".into(),
        ..Default::default()
    };
    assert_eq!(
        classify_prompt_mode(Some(&secret), &input).await,
        heuristic_prompt_mode(&input.question)
    );
    assert_eq!(server.hits(), 0);
}

#[test]
fn block_cleanup_decisions_compute_ranges_in_rust_and_validate_ids() {
    use crate::intel_recon::body_filter::{BlockDecisions, BodyChunk};
    let text = "Café talks resumed — officials denied the report.\n\nSponsored: buy now";
    let split = text.find("\n\n").unwrap();
    let split_chars = text[..split].chars().count();
    let total = text.chars().count();
    let blocks = vec![
        BodyChunk {
            id: 0,
            start: 0,
            end: split_chars,
            text: text[..split].into(),
        },
        BodyChunk {
            id: 1,
            start: split_chars,
            end: total,
            text: text[split..].into(),
        },
    ];
    let adapter = BlockDecisions {
        title: "Talks",
        brief: "Talks resume",
        url: "",
        blocks: &blocks,
    };
    let (_, questions) = adapter.compile_native();
    assert_eq!(questions.as_object().unwrap().len(), 2);
    let resp = |answers: Value| -> provider::DecisionsResponse {
        serde_json::from_value(serde_json::json!({"answers": answers})).unwrap()
    };
    let ranges = adapter
        .parse_native(&resp(serde_json::json!({
            "block_0":{"type":"choice","choice":"keep"},
            "block_1":{"type":"choice","choice":"remove"}})))
        .unwrap();
    let kept = crate::intel_recon::body_filter::remove_char_ranges_verbatim(text, &ranges);
    assert_eq!(kept, "Café talks resumed — officials denied the report.");
    assert!(adapter
        .parse_native(&resp(
            serde_json::json!({"block_0":{"type":"choice","choice":"keep"}})
        ))
        .unwrap_err()
        .contains("missing"));
    assert!(adapter
        .parse_native(&resp(serde_json::json!({
            "block_0":{"type":"choice","choice":"keep"},
            "block_1":{"type":"choice","choice":"keep"},
            "block_9":{"type":"choice","choice":"keep"}})))
        .unwrap_err()
        .contains("unexpected"));
    assert!(adapter
        .parse_native(&resp(serde_json::json!({
            "block_0":{"type":"choice","choice":"rewrite"},
            "block_1":{"type":"choice","choice":"keep"}})))
        .is_err());
    // Chat lane: exact IDs, malformed JSON fails deliberately.
    assert!(adapter.parse_chat("not json").is_err());
    let ranges = adapter
        .parse_chat("{\"blocks\":[{\"id\":0,\"keep\":true},{\"id\":1,\"keep\":false}]}")
        .unwrap();
    assert_eq!(ranges.len(), 1);
}
