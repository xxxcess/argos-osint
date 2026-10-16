//! Pure compile/normalize fixtures for the 13 researched routes and an
//! unknown text model (spec §9 verification 1, 3, 8).
use super::*;
use crate::provider_capabilities::{self as caps_mod, AdapterId, ApiDialect};
use crate::request_policy::{ReasoningEffort, RequestPolicy};

fn secret(kind: &str, model: &str) -> ProviderSecret {
    ProviderSecret {
        kind: kind.into(),
        base_url: crate::provider::preset(kind).unwrap().base_url.into(),
        model: model.into(),
        api_key: Some("k".into()),
        stt_model: None,
        device: None,
    }
}

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

fn schema() -> Value {
    crate::intel_recon::brief_contracts::claims_schema(&["t1", "d1"])
}

fn compile_for(
    kind: &str,
    model: &str,
    contract: &ResponseContract,
    policy: &RequestPolicy,
) -> Result<CompiledRequest, CompileError> {
    compile_at(kind, model, None, contract, policy)
}

/// Compile against a test-private base URL so learned rejections stay
/// scoped to that test's route.
fn compile_at(
    kind: &str,
    model: &str,
    base: Option<&str>,
    contract: &ResponseContract,
    policy: &RequestPolicy,
) -> Result<CompiledRequest, CompileError> {
    let mut s = secret(kind, model);
    if let Some(b) = base {
        s.base_url = b.into();
    }
    let caps = caps_mod::for_secret(&s);
    let m = msgs();
    compile(
        &s,
        &OperationRequest {
            messages: &m,
            tools: &[],
            contract,
            policy,
            stream: false,
        },
        &caps,
    )
}

fn json_contract() -> ResponseContract {
    ResponseContract::GeneratedJson {
        name: "brief_claims".into(),
        schema: Some(schema()),
    }
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

const J: (&str, &str) = ("openrouter", "typesafe/jev-1.13");
const S: (&str, &str) = ("openrouter", "nvidia/nemotron-3-super-120b-a12b:free");
const U: (&str, &str) = ("openrouter", "nvidia/nemotron-3-ultra-550b-a55b:free");
const N: (&str, &str) = ("openrouter", "cohere/north-mini-code:free");
const G26: (&str, &str) = ("openrouter", "google/gemma-4-26b-a4b-it:free");
const G31: (&str, &str) = ("openrouter", "google/gemma-4-31b-it:free");
const A: (&str, &str) = ("openrouter", "apodex/apodex-1.1-mini:free");
const F: (&str, &str) = ("google", "models/gemini-3.8-flash");
const D26: (&str, &str) = ("google", "models/gemma-4-26b-a4b-it");
const NS: (&str, &str) = ("nvidia", "nvidia/nemotron-3-super-120b-a12b");
const NU: (&str, &str) = ("nvidia", "nvidia/nemotron-3-ultra-550b-a55b");
const NL: (&str, &str) = ("nvidia", "nvidia/nemotron-3.5-lightning-30b-a3b");
const ND: (&str, &str) = ("nvidia", "deepseek-ai/deepseek-v4.1-flash");

#[test]
fn all_thirteen_routes_compile_their_researched_structured_mode() {
    assert_eq!(caps_mod::profile_count(), 13);
    let c = json_contract();
    let p = policy(ReasoningEffort::Off);
    for ((kind, model), mode, dialect) in [
        (S, StructuredMode::StrictSchema, ApiDialect::Chat),
        (U, StructuredMode::OutputTool, ApiDialect::Chat),
        (N, StructuredMode::OutputTool, ApiDialect::Chat),
        (G26, StructuredMode::JsonMode, ApiDialect::Chat),
        (G31, StructuredMode::JsonMode, ApiDialect::Chat),
        (A, StructuredMode::ValidatedText, ApiDialect::Chat),
        (
            F,
            StructuredMode::NativeSchema,
            ApiDialect::GoogleGenerateContent,
        ),
        (
            D26,
            StructuredMode::ValidatedText,
            ApiDialect::GoogleGenerateContent,
        ),
        (NS, StructuredMode::ValidatedText, ApiDialect::Chat),
        (NU, StructuredMode::ValidatedText, ApiDialect::Chat),
        (NL, StructuredMode::ValidatedText, ApiDialect::Chat),
        (ND, StructuredMode::ValidatedText, ApiDialect::Chat),
    ] {
        let r = compile_for(kind, model, &c, &p).unwrap();
        assert_eq!(r.effective.structured, mode, "{model}");
        assert_eq!(r.dialect, dialect, "{model}");
        // No unconditional temperature.
        assert!(r.body.get("temperature").is_none(), "{model}");
    }
    // Jev never receives generation: compile fails before network access.
    assert_eq!(
        compile_for(J.0, J.1, &c, &p).unwrap_err(),
        CompileError::DecisionOnlyTarget { model: J.1.into() }
    );
}

#[test]
fn north_and_ultra_never_get_response_format_and_force_one_output_tool() {
    for (kind, model) in [U, N] {
        let r = compile_for(
            kind,
            model,
            &json_contract(),
            &policy(ReasoningEffort::Default),
        )
        .unwrap();
        assert!(r.body.get("response_format").is_none(), "{model}");
        let tools = r.body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["function"]["name"], OUTPUT_TOOL);
        assert_eq!(tools[0]["function"]["parameters"], schema());
        assert_eq!(
            r.body["tool_choice"],
            json!({"type":"function","function":{"name":OUTPUT_TOOL}})
        );
        assert_eq!(r.channel, ResponseChannel::OutputTool(OUTPUT_TOOL.into()));
        assert_eq!(r.body["provider"]["require_parameters"], true);
    }
}

#[test]
fn super_uses_strict_schema_and_require_parameters() {
    let r = compile_for(
        S.0,
        S.1,
        &json_contract(),
        &policy(ReasoningEffort::Default),
    )
    .unwrap();
    assert_eq!(r.body["response_format"]["type"], "json_schema");
    assert_eq!(r.body["response_format"]["json_schema"]["strict"], true);
    assert_eq!(r.body["response_format"]["json_schema"]["schema"], schema());
    assert_eq!(r.body["provider"]["require_parameters"], true);
    assert_eq!(r.body["model"], S.1, "exact configured variant kept");
    assert!(r.body.get("models").is_none(), "no hidden model fallback");
}

#[test]
fn nvidia_effort_vocabularies_are_model_specific() {
    let effort = |route: (&str, &str), e| {
        compile_for(route.0, route.1, &json_contract(), &policy(e))
            .unwrap()
            .body
    };
    assert_eq!(effort(NS, ReasoningEffort::Off)["reasoning_effort"], "none");
    assert_eq!(
        effort(NS, ReasoningEffort::Efficient)["reasoning_effort"],
        "low"
    );
    assert_eq!(effort(NU, ReasoningEffort::Off)["reasoning_effort"], "none");
    assert_eq!(
        effort(NU, ReasoningEffort::Efficient)["reasoning_effort"],
        "medium"
    );
    // Low is invalid on Ultra: omitted, not guessed.
    let r = compile_for(NU.0, NU.1, &json_contract(), &policy(ReasoningEffort::Low)).unwrap();
    assert!(r.body.get("reasoning_effort").is_none());
    assert!(r
        .effective
        .diagnostics
        .iter()
        .any(|d| d.contains("reasoning=low")));
    // Lightning: documented thinking toggle.
    assert_eq!(
        effort(NL, ReasoningEffort::Off)["chat_template_kwargs"]["enable_thinking"],
        false
    );
    assert_eq!(
        effort(NL, ReasoningEffort::Efficient)["chat_template_kwargs"]["enable_thinking"],
        true
    );
    // DeepSeek: no unverified effort string; explicit max_tokens kept.
    let nd = effort(ND, ReasoningEffort::Efficient);
    assert!(nd.get("reasoning_effort").is_none());
    assert!(nd.get("chat_template_kwargs").is_none());
    assert_eq!(nd["max_tokens"], 2048);
    // Native Nvidia fields never go through OpenRouter.
    let s = effort(S, ReasoningEffort::Efficient);
    assert!(s.get("reasoning_effort").is_none());
    assert!(s.get("chat_template_kwargs").is_none());
}

#[test]
fn total_ceiling_is_declared_not_reserved() {
    // Off: answer ceiling. Thinking on: the declared thinking ceiling, no
    // hidden reserve.
    let off = compile_for(NU.0, NU.1, &json_contract(), &policy(ReasoningEffort::Off)).unwrap();
    assert_eq!(off.body["max_tokens"], 2048);
    assert!(!off.effective.thinking_on);
    let on = compile_for(
        NU.0,
        NU.1,
        &json_contract(),
        &policy(ReasoningEffort::Efficient),
    )
    .unwrap();
    assert_eq!(on.body["max_tokens"], 4096);
    assert!(on.effective.thinking_on);
    // A documented budget is sent but labelled advisory (not enforced).
    let budget = RequestPolicy {
        reasoning_budget: Some(1024),
        ..policy(ReasoningEffort::Efficient)
    };
    let r = compile_for(NU.0, NU.1, &json_contract(), &budget).unwrap();
    assert_eq!(r.body["reasoning_budget"], 1024);
    assert_eq!(r.effective.budget_kind.as_deref(), Some("advisory"));
}

#[test]
fn google_native_flash_and_gemma_bodies() {
    let f = compile_for(F.0, F.1, &json_contract(), &policy(ReasoningEffort::Off)).unwrap();
    assert_eq!(
        f.endpoint,
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.8-flash:generateContent"
    );
    assert_eq!(f.auth, AuthStyle::GoogleApiKey);
    assert_eq!(f.body["systemInstruction"]["parts"][0]["text"], "Rules");
    assert_eq!(f.body["contents"][0]["role"], "user");
    let cfg = &f.body["generationConfig"];
    // Flash cannot turn thinking off: low, and the thinking ceiling.
    assert_eq!(cfg["thinkingConfig"]["thinkingLevel"], "low");
    assert_eq!(cfg["maxOutputTokens"], 4096);
    assert_eq!(
        cfg["responseFormat"]["text"]["mimeType"],
        "APPLICATION_JSON"
    );
    assert_eq!(cfg["responseFormat"]["text"]["schema"], schema());
    assert!(f.body.get("tools").is_none(), "no grounding/tools added");

    let d = compile_for(
        D26.0,
        D26.1,
        &json_contract(),
        &policy(ReasoningEffort::Off),
    )
    .unwrap();
    assert_eq!(
        d.endpoint,
        "https://generativelanguage.googleapis.com/v1beta/models/gemma-4-26b-a4b-it:generateContent"
    );
    let cfg = &d.body["generationConfig"];
    assert_eq!(cfg["thinkingConfig"]["thinkingLevel"], "minimal");
    assert_eq!(cfg["maxOutputTokens"], 2048);
    assert!(
        cfg.get("responseFormat").is_none(),
        "local validation until confirmed"
    );
    let on = compile_for(
        D26.0,
        D26.1,
        &json_contract(),
        &policy(ReasoningEffort::Efficient),
    )
    .unwrap();
    assert_eq!(
        on.body["generationConfig"]["thinkingConfig"]["thinkingLevel"],
        "high"
    );
}

#[test]
fn google_model_ids_normalize_per_wire_api() {
    assert_eq!(google::resource_id("models/x"), "models/x");
    assert_eq!(google::resource_id("x"), "models/x");
    assert_eq!(google::resource_id("models/models/x"), "models/x");
    // A custom compatibility base keeps chat and gets the bare ID.
    let mut s = secret(F.0, F.1);
    s.base_url = "https://proxy.example/v1beta/openai".into();
    assert_eq!(
        default_dialect(AdapterId::Google, &s.base_url),
        ApiDialect::Chat
    );
    let caps = caps_mod::for_secret(&s);
    let m = msgs();
    let r = compile(
        &s,
        &OperationRequest {
            messages: &m,
            tools: &[],
            contract: &ResponseContract::Prose,
            policy: &RequestPolicy::default(),
            stream: true,
        },
        &caps,
    )
    .unwrap();
    assert_eq!(r.body["model"], "gemini-3.8-flash");
    assert_eq!(
        r.endpoint,
        "https://proxy.example/v1beta/openai/chat/completions"
    );
    // Streaming native uses streamGenerateContent SSE.
    let s = secret(F.0, F.1);
    let caps = caps_mod::for_secret(&s);
    let r = compile(
        &s,
        &OperationRequest {
            messages: &m,
            tools: &[],
            contract: &ResponseContract::Prose,
            policy: &RequestPolicy::default(),
            stream: true,
        },
        &caps,
    )
    .unwrap();
    assert!(r
        .endpoint
        .ends_with("models/gemini-3.8-flash:streamGenerateContent?alt=sse"));
}

#[test]
fn unknown_text_model_is_usable_with_conservative_capabilities() {
    for kind in ["openrouter", "nvidia", "google"] {
        let r = compile_for(
            kind,
            "vendor/unseen-model-9",
            &json_contract(),
            &policy(ReasoningEffort::High),
        )
        .unwrap();
        assert_eq!(
            r.effective.structured,
            StructuredMode::ValidatedText,
            "{kind}"
        );
        for field in ["response_format", "reasoning", "reasoning_effort", "tools"] {
            assert!(r.body.get(field).is_none(), "{kind} {field}");
        }
        assert!(
            r.body.get("provider").is_none(),
            "{kind}: nothing to require"
        );
    }
}

#[test]
fn schema_downgrade_preserves_validation_order() {
    // G26: schema unconfirmed → JSON mode with a downgrade diagnostic.
    let r = compile_for(
        G26.0,
        G26.1,
        &json_contract(),
        &policy(ReasoningEffort::Default),
    )
    .unwrap();
    assert_eq!(r.body["response_format"]["type"], "json_object");
    assert!(r
        .effective
        .diagnostics
        .iter()
        .any(|d| d.contains("downgraded to JSON mode")));
    // A learned json_schema rejection on Super downgrades to JSON mode.
    let base = "https://or-downgrade.test/api/v1";
    caps_mod::learn_rejection(
        AdapterId::OpenRouter,
        base,
        S.1,
        "response_format",
        "json_schema",
    );
    let r = compile_at(
        S.0,
        S.1,
        Some(base),
        &json_contract(),
        &policy(ReasoningEffort::Default),
    )
    .unwrap();
    assert_eq!(r.effective.structured, StructuredMode::JsonMode);
}

#[test]
fn rejecting_one_effort_value_keeps_other_values_of_the_field() {
    let base = "https://nv-value-scope.test/v1";
    caps_mod::learn_rejection(AdapterId::Nvidia, base, NU.1, "reasoning_effort", "none");
    let at = |e| compile_at(NU.0, NU.1, Some(base), &json_contract(), &policy(e)).unwrap();
    assert!(at(ReasoningEffort::Off)
        .body
        .get("reasoning_effort")
        .is_none());
    assert_eq!(
        at(ReasoningEffort::Efficient).body["reasoning_effort"],
        "medium"
    );
}

#[test]
fn prose_and_agent_tools_stay_on_ordinary_chat() {
    let s = secret(F.0, F.1);
    let caps = caps_mod::for_secret(&s);
    let m = msgs();
    let tools = vec![ToolSpec {
        name: "lookup".into(),
        description: "d".into(),
        parameters: json!({"type":"object"}),
    }];
    let r = compile(
        &s,
        &OperationRequest {
            messages: &m,
            tools: &tools,
            contract: &ResponseContract::Prose,
            policy: &RequestPolicy::default(),
            stream: true,
        },
        &caps,
    )
    .unwrap();
    assert_eq!(
        r.dialect,
        ApiDialect::Chat,
        "agent tool loops use compatibility chat"
    );
    assert_eq!(r.body["tool_choice"], "auto");
    assert_eq!(r.channel, ResponseChannel::Content);
}

#[test]
fn native_decisions_receive_no_chat_parameters() {
    let s = secret(J.0, J.1);
    let caps = caps_mod::for_secret(&s);
    let r = compile_decisions(&s, &json!({"items":{}}), &json!({"q":{}}), &caps);
    assert_eq!(r.endpoint, "https://openrouter.ai/api/alpha/decisions");
    assert_eq!(
        r.body
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec!["model", "questions", "state"]
    );
}

#[test]
fn output_tool_normalization_accepts_exactly_one_complete_call() {
    let chan = ResponseChannel::OutputTool(OUTPUT_TOOL.into());
    let call = |name: &str, args: &str| ToolCall {
        id: "c".into(),
        name: name.into(),
        arguments: args.into(),
    };
    let base = |calls: Vec<ToolCall>, finish: &str| Completion {
        tool_calls: calls,
        finish_reason: Some(finish.into()),
        ..Completion::default()
    };
    let ok = finalize_channel(
        base(vec![call(OUTPUT_TOOL, r#"{"claims":[]}"#)], "tool_calls"),
        &chan,
    )
    .unwrap();
    assert_eq!(ok.content, r#"{"claims":[]}"#);
    for (c, why) in [
        (base(vec![], "stop"), "none"),
        (base(vec![call("search", "{}")], "tool_calls"), "unknown"),
        (
            base(
                vec![call(OUTPUT_TOOL, "{}"), call(OUTPUT_TOOL, "{}")],
                "tool_calls",
            ),
            "got 2",
        ),
        (
            base(vec![call(OUTPUT_TOOL, r#"{"claims":["#)], "tool_calls"),
            "not valid JSON",
        ),
        (
            base(vec![call(OUTPUT_TOOL, r#"{"claims":["#)], "length"),
            "truncated",
        ),
    ] {
        let err = finalize_channel(c, &chan).unwrap_err();
        assert!(err.message.contains(why), "{why}: {}", err.message);
    }
    let refused = Completion {
        refusal: Some("no".into()),
        ..base(vec![], "stop")
    };
    assert_eq!(
        finalize_channel(refused, &chan).unwrap_err().category,
        Category::Refused
    );
}

#[test]
fn chat_stream_decoder_assembles_fragmented_tool_arguments_and_usage() {
    let mut d = StreamDecoder::new(ApiDialect::Chat);
    for ev in [
        json!({"choices":[{"delta":{"content":""}}]}),
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"submit_result","arguments":"{\"cla"}}]}}]}),
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ims\":[]}"}}]}}]}),
        json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":7,"completion_tokens_details":{"reasoning_tokens":3},"cost":0.0}}),
    ] {
        d.apply(&ev);
    }
    assert!(d.tool_chars > 0, "argument deltas are meaningful output");
    let c = d.into_completion();
    assert_eq!(c.tool_calls.len(), 1);
    assert_eq!(c.tool_calls[0].arguments, r#"{"claims":[]}"#);
    let u = c.usage.clone().unwrap();
    assert_eq!(
        (
            u.prompt_tokens,
            u.completion_tokens,
            u.reasoning_tokens,
            u.visible_tokens
        ),
        (Some(10), Some(7), Some(3), Some(4))
    );
    let c = finalize_channel(c, &ResponseChannel::OutputTool(OUTPUT_TOOL.into())).unwrap();
    assert_eq!(c.content, r#"{"claims":[]}"#);
}

#[test]
fn google_normalization_keeps_answer_parts_and_usage() {
    let s = secret(F.0, F.1);
    let caps = caps_mod::for_secret(&s);
    let m = msgs();
    let compiled = compile(
        &s,
        &OperationRequest {
            messages: &m,
            tools: &[],
            contract: &ResponseContract::Prose,
            policy: &RequestPolicy::default(),
            stream: false,
        },
        &caps,
    )
    .unwrap();
    let body = json!({
        "candidates":[{"content":{"role":"model","parts":[{"text":"thinking...","thought":true},{"text":"{\"a\":1}"}]},"finishReason":"STOP"}],
        "usageMetadata":{"promptTokenCount":12,"candidatesTokenCount":5,"thoughtsTokenCount":9}
    });
    let c = normalize_final(&compiled, &body.to_string()).unwrap();
    assert_eq!(c.content, "{\"a\":1}");
    assert_eq!(c.reasoning, "thinking...");
    assert_eq!(c.finish_reason.as_deref(), Some("stop"));
    let u = c.usage.unwrap();
    assert_eq!(
        (
            u.prompt_tokens,
            u.visible_tokens,
            u.reasoning_tokens,
            u.completion_tokens,
            u.cost_usd
        ),
        (Some(12), Some(5), Some(9), Some(14), None)
    );
    let max = json!({"candidates":[{"content":{"parts":[{"text":"{\"a\""}]},"finishReason":"MAX_TOKENS"}]});
    assert_eq!(
        normalize_final(&compiled, &max.to_string())
            .unwrap()
            .finish_reason
            .as_deref(),
        Some("length")
    );
    let blocked = json!({"promptFeedback":{"blockReason":"SAFETY"}});
    assert!(normalize_final(&compiled, &blocked.to_string())
        .unwrap()
        .refusal
        .is_some());
}

#[test]
fn model_swaps_recompile_the_same_contract_without_stage_changes() {
    // U→S generation: output tool becomes strict schema.
    let c = json_contract();
    let p = policy(ReasoningEffort::Efficient);
    let u = compile_for(U.0, U.1, &c, &p).unwrap();
    let s = compile_for(S.0, S.1, &c, &p).unwrap();
    assert_eq!(u.effective.structured, StructuredMode::OutputTool);
    assert_eq!(s.effective.structured, StructuredMode::StrictSchema);
    // F→D26 native Google.
    let f = compile_for(F.0, F.1, &c, &p).unwrap();
    let d = compile_for(D26.0, D26.1, &c, &p).unwrap();
    assert_eq!(f.dialect, ApiDialect::GoogleGenerateContent);
    assert_eq!(d.dialect, ApiDialect::GoogleGenerateContent);
    // NU→NS/NL effort translation for the same Efficient intent.
    assert_eq!(
        compile_for(NU.0, NU.1, &c, &p).unwrap().body["reasoning_effort"],
        "medium"
    );
    assert_eq!(
        compile_for(NS.0, NS.1, &c, &p).unwrap().body["reasoning_effort"],
        "low"
    );
    assert_eq!(
        compile_for(NL.0, NL.1, &c, &p).unwrap().body["chat_template_kwargs"]["enable_thinking"],
        true
    );
    // Digests differ per model and are stable across calls.
    assert_ne!(u.effective.capability_digest, s.effective.capability_digest);
    assert_eq!(
        u.effective.capability_digest,
        compile_for(U.0, U.1, &c, &p)
            .unwrap()
            .effective
            .capability_digest
    );
}

#[test]
fn automatic_stages_leave_output_and_reasoning_limits_to_the_provider() {
    use crate::request_policy::*;
    for budget in [
        CLEANUP_FALLBACK_BUDGET,
        CLAIMS_BUDGET,
        FIDELITY_BUDGET,
        ACTOR_IDENTITY_BUDGET,
        INFERENCES_BUDGET,
        SUPPORT_BUDGET,
        LINKS_BUDGET,
    ] {
        let policy = budget.policy(true);
        for (provider, model) in [NU, F, NS, S, U] {
            let compiled = compile_for(provider, model, &json_contract(), &policy).unwrap();
            // No stage ever sends an output or reasoning *ceiling*.
            assert!(
                compiled.body.get("max_tokens").is_none(),
                "{}",
                compiled.body
            );
            assert!(compiled.body.get("max_completion_tokens").is_none());
            assert!(compiled.body["generationConfig"]
                .get("maxOutputTokens")
                .is_none());
            assert!(compiled.body.get("reasoning_budget").is_none());
            assert!(compiled.effective.total_ceiling.is_none());
            assert!(compiled.effective.reasoning_budget.is_none());
            if !budget.thinks() {
                // Non-thinking stages keep the provider default.
                assert!(compiled.body.get("reasoning_effort").is_none());
                assert!(compiled.body["generationConfig"]
                    .get("thinkingConfig")
                    .is_none());
            }
        }
    }
}

/// Inferences always thinks: each model gets its own thinking-on effort, never
/// an off value, and models without a confirmed control omit it (reported).
#[test]
fn inferences_send_a_per_model_thinking_effort_and_never_turn_thinking_off() {
    use crate::request_policy::INFERENCES_BUDGET;
    let policy = INFERENCES_BUDGET.policy(true);
    assert_eq!(policy.reasoning, ReasoningEffort::Efficient);
    let c = json_contract();
    // Nvidia: effort translated per model (Ultra rejects `low`).
    assert_eq!(
        compile_for(NS.0, NS.1, &c, &policy).unwrap().body["reasoning_effort"],
        "low"
    );
    assert_eq!(
        compile_for(NU.0, NU.1, &c, &policy).unwrap().body["reasoning_effort"],
        "medium"
    );
    assert_eq!(
        compile_for(NL.0, NL.1, &c, &policy).unwrap().body["chat_template_kwargs"]
            ["enable_thinking"],
        true
    );
    // Google: Gemini low (thinking mandatory), Gemma binary `high` = on.
    let level = |m: (&str, &str)| {
        compile_for(m.0, m.1, &c, &policy).unwrap().body["generationConfig"]["thinkingConfig"]
            ["thinkingLevel"]
            .as_str()
            .map(str::to_ascii_lowercase)
    };
    assert_eq!(level(F).as_deref(), Some("low"));
    assert_eq!(level(D26).as_deref(), Some("high"));
    // DeepSeek has no documented control: omitted and reported.
    let nd = compile_for(ND.0, ND.1, &c, &policy).unwrap();
    assert!(nd.body.get("reasoning_effort").is_none());
    assert!(nd
        .effective
        .diagnostics
        .iter()
        .any(|d| d.starts_with("unsupported-policy:reasoning=efficient")));
    // OpenRouter: sent only once the catalog confirms `reasoning`.
    let plain = compile_for(S.0, S.1, &c, &policy).unwrap();
    assert!(plain.body.get("reasoning").is_none());
    let s = secret(S.0, S.1);
    let ev = caps_mod::CatalogEvidence {
        supported_parameters: ["reasoning", "response_format", "structured_outputs"]
            .map(String::from)
            .to_vec(),
        ..Default::default()
    };
    let caps = caps_mod::resolve_with(AdapterId::OpenRouter, &s.base_url, S.1, Some((&ev, 1)));
    let m = msgs();
    let r = compile(
        &s,
        &OperationRequest {
            messages: &m,
            tools: &[],
            contract: &c,
            policy: &policy,
            stream: true,
        },
        &caps,
    )
    .unwrap();
    assert_eq!(r.body["reasoning"]["effort"], "low");
    assert!(r.body["reasoning"].get("enabled").is_none());
}
