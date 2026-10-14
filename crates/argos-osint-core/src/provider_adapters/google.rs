//! Google native GenerateContent adapter (spec §4 "Google provider compiler").
//!
//! Stored `models/...` IDs are kept; the wire path carries exactly one
//! `models/` resource prefix (compatibility chat uses the bare ID). Messages
//! compile into `systemInstruction` and user/model `contents` with
//! `generationConfig.maxOutputTokens`. Thinking maps through the model's
//! confirmed levels (Gemini Flash low/medium/high, mandatory; Gemma
//! minimal=off/high=on). Confirmed native schema output uses
//! `generationConfig.responseFormat.text`. Grounding or tools that would
//! widen an operation's source scope are never enabled.

use serde_json::{json, Map, Value};

use super::{
    choose_structured, plan_reasoning, total_ceiling, AuthStyle, CompiledRequest, DecodedDelta,
    EffectivePolicy, OperationRequest, ResponseChannel, ResponseContract, SentOption,
    StreamDecoder, StructuredMode, OUTPUT_TOOL,
};
use crate::provider::{Completion, ToolCall, Usage};
use crate::provider_capabilities::{ApiDialect, ReasoningControl, ResolvedCapabilities};
use crate::provider_diag::{Category, ProviderFailure, Stage};
use crate::secrets::ProviderSecret;

/// Native API root for a stored base URL (`…/v1beta/openai` → `…/v1beta`).
pub fn native_root(base: &str) -> String {
    let base = crate::provider::normalize_base(base);
    base.strip_suffix("/openai").unwrap_or(&base).to_string()
}

/// `models/{id}` with exactly one resource prefix.
pub fn resource_id(model: &str) -> String {
    let mut id = model.trim();
    while let Some(rest) = id.strip_prefix("models/") {
        id = rest;
    }
    format!("models/{id}")
}

pub(crate) fn compile_native(
    secret: &ProviderSecret,
    req: &OperationRequest<'_>,
    caps: &ResolvedCapabilities,
) -> CompiledRequest {
    let mut diags = Vec::new();
    let mut sent = Vec::new();
    let mut system = Vec::new();
    let mut contents: Vec<Value> = Vec::new();
    for m in req.messages {
        match m.role.as_str() {
            "system" | "developer" => system.push(json!({"text": m.content})),
            "assistant" => contents.push(json!({"role": "model", "parts": [{"text": m.content}]})),
            _ => contents.push(json!({"role": "user", "parts": [{"text": m.content}]})),
        }
    }
    let mut body = json!({"contents": contents});
    if !system.is_empty() {
        body["systemInstruction"] = json!({"parts": system});
    }
    let mut config = Map::new();
    if let Some(t) = caps.temperature {
        config.insert("temperature".into(), json!(t));
    }
    let mut channel = ResponseChannel::Content;
    let structured = choose_structured(
        req.contract,
        caps,
        caps.native_schema,
        "responseFormat",
        &mut diags,
    );
    match (structured, req.contract) {
        (
            StructuredMode::NativeSchema,
            ResponseContract::GeneratedJson {
                schema: Some(schema),
                ..
            },
        ) => {
            config.insert(
                "responseFormat".into(),
                json!({"text": {"mimeType": "APPLICATION_JSON", "schema": schema}}),
            );
            sent.push(SentOption {
                field: "responseFormat".into(),
                value: "json_schema".into(),
                aliases: vec!["response_format".into(), "schema".into()],
            });
        }
        (
            StructuredMode::OutputTool,
            ResponseContract::GeneratedJson {
                schema: Some(schema),
                ..
            },
        ) => {
            body["tools"] = json!([{"functionDeclarations": [{
                "name": OUTPUT_TOOL,
                "description": super::output_tool(schema).description,
                "parameters": schema,
            }]}]);
            body["toolConfig"] = json!({"functionCallingConfig": {
                "mode": "ANY", "allowedFunctionNames": [OUTPUT_TOOL],
            }});
            channel = ResponseChannel::OutputTool(OUTPUT_TOOL.into());
            sent.push(SentOption {
                field: "toolConfig".into(),
                value: "function".into(),
                aliases: vec!["function".into(), "tool".into()],
            });
        }
        (StructuredMode::JsonMode, _) => {
            config.insert("responseMimeType".into(), json!("application/json"));
            sent.push(SentOption {
                field: "responseMimeType".into(),
                value: "json_object".into(),
                aliases: vec!["mime".into(), "json".into()],
            });
        }
        _ => {}
    }
    let plan = if caps.reasoning.control == ReasoningControl::GoogleLevel {
        let plan = plan_reasoning(caps, req.policy.reasoning, "thinkingLevel", &mut diags);
        if let Some(level) = &plan.value {
            config.insert("thinkingConfig".into(), json!({"thinkingLevel": level}));
            sent.push(SentOption {
                field: "thinkingLevel".into(),
                value: level.clone(),
                aliases: vec!["thinking".into(), "thinking_level".into()],
            });
        }
        plan
    } else {
        let mut none = caps.clone();
        none.reasoning.control = ReasoningControl::None;
        let mut plan = plan_reasoning(&none, req.policy.reasoning, "thinkingLevel", &mut diags);
        plan.thinking_on |= caps.reasoning.mandatory;
        plan
    };
    let ceiling = total_ceiling(req.policy, plan.thinking_on, caps);
    if let Some(tokens) = ceiling {
        config.insert("maxOutputTokens".into(), json!(tokens));
    }
    if !config.is_empty() {
        body["generationConfig"] = Value::Object(config);
    }
    let method = if req.stream {
        "streamGenerateContent?alt=sse"
    } else {
        "generateContent"
    };
    CompiledRequest {
        adapter: caps.adapter,
        dialect: ApiDialect::GoogleGenerateContent,
        endpoint: format!(
            "{}/{}:{method}",
            native_root(&secret.base_url),
            resource_id(&secret.model)
        ),
        auth: AuthStyle::GoogleApiKey,
        body,
        channel,
        stream: req.stream,
        effective: EffectivePolicy {
            adapter: caps.adapter.as_str().into(),
            dialect: ApiDialect::GoogleGenerateContent.as_str().into(),
            contract: req.contract.as_str().into(),
            structured,
            reasoning: sent
                .iter()
                .find(|o| o.field == "thinkingLevel")
                .map(|o| format!("thinkingLevel={}", o.value)),
            thinking_on: plan.thinking_on,
            total_ceiling: ceiling,
            reasoning_budget: None,
            budget_kind: None,
            capability_digest: caps.digest(),
            sent,
            diagnostics: diags,
        },
    }
}

fn usage_metadata(v: &Value) -> Option<Usage> {
    let u = v.get("usageMetadata").filter(|u| u.is_object())?;
    let n = |k: &str| u.get(k).and_then(Value::as_u64);
    let visible = n("candidatesTokenCount");
    let reasoning = n("thoughtsTokenCount");
    Some(Usage {
        prompt_tokens: n("promptTokenCount"),
        completion_tokens: match (visible, reasoning) {
            (Some(v), Some(r)) => Some(v + r),
            (Some(v), None) => Some(v),
            _ => None,
        },
        reasoning_tokens: reasoning,
        visible_tokens: visible,
        cost_usd: None,
    })
}

/// Map Google finish reasons to the shared vocabulary.
fn finish(reason: &str) -> (String, bool) {
    match reason {
        "STOP" => ("stop".into(), false),
        "MAX_TOKENS" => ("length".into(), false),
        "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" | "IMAGE_SAFETY" => {
            ("content_filter".into(), true)
        }
        other => (other.to_ascii_lowercase(), false),
    }
}

/// Apply one `GenerateContentResponse` (final body or SSE chunk).
pub(crate) fn apply_stream_event(d: &mut StreamDecoder, v: &Value) -> DecodedDelta {
    let mut out = DecodedDelta::default();
    if let Some(u) = usage_metadata(v) {
        d.usage = Some(u);
    }
    if let Some(block) = v
        .pointer("/promptFeedback/blockReason")
        .and_then(Value::as_str)
    {
        d.refusal = Some(format!("prompt blocked: {block}"));
        d.finish_reason = Some("content_filter".into());
        return out;
    }
    let Some(cand) = v.pointer("/candidates/0") else {
        return out;
    };
    for part in cand
        .pointer("/content/parts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(call) = part.get("functionCall") {
            let args = call.get("args").cloned().unwrap_or(json!({})).to_string();
            d.tool_chars += args.chars().count() as u64;
            d.tool_calls.push(ToolCall {
                id: call
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("call")
                    .to_string(),
                name: call
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                arguments: args,
            });
            out.tool_args = true;
            continue;
        }
        let Some(text) = part.get("text").and_then(Value::as_str) else {
            continue;
        };
        if part.get("thought").and_then(Value::as_bool) == Some(true) {
            d.reasoning.push_str(text);
            out.reasoning = true;
        } else if !text.is_empty() {
            d.content.push_str(text);
            out.content = Some(text.to_string());
        }
    }
    if let Some(reason) = cand.get("finishReason").and_then(Value::as_str) {
        let (mapped, refused) = finish(reason);
        if refused {
            d.refusal = Some(format!("blocked: {reason}"));
        }
        d.finish_reason = Some(mapped);
    }
    out
}

/// Parse a final native body.
pub(crate) fn parse_final(text: &str) -> Result<Completion, ProviderFailure> {
    let v: Value = serde_json::from_str(text)
        .map_err(|e| ProviderFailure::from_error(Stage::Parse, Category::MalformedPayload, &e))?;
    if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
        let (code, message) =
            crate::provider_diag::provider_error(&json!({"error": err}).to_string());
        let mut f = ProviderFailure::new(
            Stage::Parse,
            Category::SseError,
            "provider returned an error payload with HTTP 200",
        );
        f.provider_code = code;
        f.provider_message = message;
        return Err(f);
    }
    if v.get("candidates").is_none() && v.get("promptFeedback").is_none() {
        return Err(ProviderFailure::new(
            Stage::Parse,
            Category::MalformedPayload,
            "response JSON has no candidates",
        ));
    }
    let mut d = StreamDecoder::new(ApiDialect::GoogleGenerateContent);
    apply_stream_event(&mut d, &v);
    Ok(d.into_completion())
}
