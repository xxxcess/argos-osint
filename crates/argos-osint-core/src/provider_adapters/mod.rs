//! Provider-owned request adapters (spec §3–§4).
//!
//! One immutable operation contract compiles to several wire formats. Each
//! adapter owns endpoint/auth, model ID normalization, parameter names,
//! schema/tool compilation, stream/usage parsing and its error dialect.
//! Model IDs are data: capability differences come from
//! [`ResolvedCapabilities`], never from a switch on model IDs here.
//!
//! [`compile`] is pure: it returns the endpoint, dialect, JSON body, response
//! channel and effective policy. [`normalize_final`] and [`StreamDecoder`]
//! turn a provider response back into one [`Completion`] (answer text or
//! output-tool arguments, finish/refusal status, nullable usage). The shared
//! executor alone performs HTTP, retries and admission.

// `ProviderFailure` is the shared typed diagnostic (see provider_attempt).
#![allow(clippy::result_large_err)]

pub mod google;
pub mod nvidia;
pub mod openrouter;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::provider::{ChatMessage, Completion, ToolCall, ToolSpec, Usage};
use crate::provider_capabilities::{
    AdapterId, ApiDialect, ReasoningControl, ResolvedCapabilities, Support,
};
use crate::provider_diag::{Category, ProviderFailure, Stage};
use crate::request_policy::{ReasoningEffort, RequestPolicy};
use crate::secrets::ProviderSecret;

/// The inert output-channel tool. Never executed; its arguments are the answer.
pub const OUTPUT_TOOL: &str = "submit_result";

/// What an operation must return, independent of adapter.
#[derive(Clone, Debug, PartialEq)]
pub enum ResponseContract {
    /// Closed labels/IDs (native decisions or validated chat labels).
    BoundedDecision,
    /// A JSON object. `schema` is the authoritative operation schema when one
    /// exists (`intel_recon::brief_contracts`); `None` = JSON with local
    /// validation only.
    GeneratedJson { name: String, schema: Option<Value> },
    /// Free text.
    Prose,
}

impl ResponseContract {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BoundedDecision => "bounded_decision",
            Self::GeneratedJson { .. } => "generated_json",
            Self::Prose => "prose",
        }
    }

    /// The legacy mapping for callers that only carry a [`RequestPolicy`].
    pub fn from_policy(policy: &RequestPolicy) -> Self {
        if policy.json_object {
            Self::GeneratedJson {
                name: "result".into(),
                schema: None,
            }
        } else {
            Self::Prose
        }
    }
}

/// How structured output is obtained on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuredMode {
    NativeDecisions,
    /// Chat `response_format: json_schema` (strict).
    StrictSchema,
    /// Google native `responseFormat` schema.
    NativeSchema,
    /// Forced inert `submit_result` function.
    OutputTool,
    /// `response_format: json_object` / native JSON MIME type.
    JsonMode,
    /// Prompt instructions + strict local validation.
    ValidatedText,
    Prose,
}

impl StructuredMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NativeDecisions => "native_decisions",
            Self::StrictSchema => "strict_json_schema",
            Self::NativeSchema => "native_schema",
            Self::OutputTool => "output_tool",
            Self::JsonMode => "json_mode",
            Self::ValidatedText => "validated_text",
            Self::Prose => "prose",
        }
    }

    /// Whether the provider enforces the schema (vs local validation only).
    pub fn schema_enforced(self) -> bool {
        matches!(
            self,
            Self::StrictSchema | Self::NativeSchema | Self::OutputTool | Self::NativeDecisions
        )
    }
}

/// One operation request before compilation.
#[derive(Clone, Copy, Debug)]
pub struct OperationRequest<'a> {
    pub messages: &'a [ChatMessage],
    /// Ordinary tool-calling tools (agent loops); empty for brief operations.
    pub tools: &'a [ToolSpec],
    pub contract: &'a ResponseContract,
    pub policy: &'a RequestPolicy,
    pub stream: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthStyle {
    Bearer,
    /// Google native `x-goog-api-key`.
    GoogleApiKey,
}

/// Where the answer arrives.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResponseChannel {
    Content,
    /// Arguments of exactly one call to this output tool.
    OutputTool(String),
}

/// One optional option actually placed on the wire (for scoped rejection
/// learning by field **and** value).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentOption {
    pub field: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

/// What was actually requested, for telemetry/diagnostics. Never text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectivePolicy {
    pub adapter: String,
    pub dialect: String,
    pub contract: String,
    pub structured: StructuredMode,
    /// Effective reasoning field=value, if any was sent.
    pub reasoning: Option<String>,
    pub thinking_on: bool,
    pub total_ceiling: Option<u32>,
    pub reasoning_budget: Option<u32>,
    /// `hard` when the endpoint enforces the reasoning budget, else `advisory`.
    pub budget_kind: Option<String>,
    pub capability_digest: String,
    pub sent: Vec<SentOption>,
    pub diagnostics: Vec<String>,
}

/// A compiled single request.
#[derive(Clone, Debug)]
pub struct CompiledRequest {
    pub adapter: AdapterId,
    pub dialect: ApiDialect,
    pub endpoint: String,
    pub auth: AuthStyle,
    pub body: Value,
    pub channel: ResponseChannel,
    pub stream: bool,
    pub effective: EffectivePolicy,
}

/// The contract cannot be compiled for this route (no request is sent).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompileError {
    /// Decision-only target (Jev) asked for generation/prose.
    DecisionOnlyTarget {
        model: String,
    },
    /// Catalog says this model does not generate text.
    NotTextGeneration {
        model: String,
    },
    MalformedContract(String),
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DecisionOnlyTarget { model } => write!(
                f,
                "capability skip: {model} is a decision-only route and cannot generate text"
            ),
            Self::NotTextGeneration { model } => write!(
                f,
                "capability skip: {model} does not advertise text generation"
            ),
            Self::MalformedContract(m) => write!(f, "malformed contract: {m}"),
        }
    }
}

impl std::error::Error for CompileError {}

impl CompileError {
    /// Typed, non-retryable failure that moves the chain to the next route.
    pub fn into_failure(self, kind: &str, model: &str) -> ProviderFailure {
        let category = match self {
            Self::MalformedContract(_) => Category::MalformedRequest,
            _ => Category::Unsupported,
        };
        let mut f = ProviderFailure::new(Stage::Configuration, category, self.to_string());
        f.retryable = false;
        f.with_context(kind, model, "", "compile")
    }
}

/// Default wire dialect for a connection. Google's own host (including the
/// preset's `/v1beta/openai` compatibility base) and explicit native bases
/// use native GenerateContent; custom/proxy bases that end in `/openai` keep
/// their explicit compatibility-chat dialect.
pub fn default_dialect(adapter: AdapterId, base_url: &str) -> ApiDialect {
    if adapter != AdapterId::Google {
        return ApiDialect::Chat;
    }
    let base = crate::provider::normalize_base(base_url).to_ascii_lowercase();
    let canonical = url::Url::parse(&base)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(|h| h == "generativelanguage.googleapis.com")
        })
        .unwrap_or(false);
    if canonical || !base.ends_with("/openai") {
        ApiDialect::GoogleGenerateContent
    } else {
        ApiDialect::Chat
    }
}

/// Compile one generation/prose request for `secret` with `caps`.
pub fn compile(
    secret: &ProviderSecret,
    req: &OperationRequest<'_>,
    caps: &ResolvedCapabilities,
) -> Result<CompiledRequest, CompileError> {
    if caps.decision_only {
        return Err(CompileError::DecisionOnlyTarget {
            model: secret.model.clone(),
        });
    }
    if caps.text_generation == Support::Unsupported {
        return Err(CompileError::NotTextGeneration {
            model: secret.model.clone(),
        });
    }
    if let ResponseContract::GeneratedJson {
        schema: Some(schema),
        ..
    } = req.contract
    {
        if schema.get("type").and_then(Value::as_str) != Some("object") {
            return Err(CompileError::MalformedContract(
                "operation schema must be a JSON object schema".into(),
            ));
        }
    }
    let mut dialect = default_dialect(caps.adapter, &secret.base_url);
    if dialect == ApiDialect::GoogleGenerateContent
        && (!req.tools.is_empty() || caps.generate_content == Support::Unsupported)
    {
        // Agent tool loops and non-generateContent models use the supported
        // compatibility chat dialect.
        dialect = ApiDialect::Chat;
    }
    match dialect {
        ApiDialect::GoogleGenerateContent => Ok(google::compile_native(secret, req, caps)),
        _ => Ok(compile_chat(secret, req, caps, dialect)),
    }
}

/// Compile a native decisions request (no chat completion parameters).
pub fn compile_decisions(
    secret: &ProviderSecret,
    state: &Value,
    questions: &Value,
    caps: &ResolvedCapabilities,
) -> CompiledRequest {
    CompiledRequest {
        adapter: caps.adapter,
        dialect: ApiDialect::NativeDecisions,
        endpoint: crate::provider::decisions_url(&secret.base_url),
        auth: AuthStyle::Bearer,
        body: json!({"model": secret.model, "state": state, "questions": questions}),
        channel: ResponseChannel::Content,
        stream: false,
        effective: EffectivePolicy {
            adapter: caps.adapter.as_str().into(),
            dialect: ApiDialect::NativeDecisions.as_str().into(),
            contract: ResponseContract::BoundedDecision.as_str().into(),
            structured: StructuredMode::NativeDecisions,
            reasoning: None,
            thinking_on: false,
            total_ceiling: None,
            reasoning_budget: None,
            budget_kind: None,
            capability_digest: caps.digest(),
            sent: Vec::new(),
            diagnostics: Vec::new(),
        },
    }
}

/// Resolved reasoning plan for one request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ReasoningPlan {
    /// Wire value to send (control-specific), if any.
    pub value: Option<String>,
    pub thinking_on: bool,
}

/// Map an intent onto the model's confirmed values. Never sends a value the
/// profile/catalog does not confirm, or one this route rejected.
pub(crate) fn plan_reasoning(
    caps: &ResolvedCapabilities,
    effort: ReasoningEffort,
    field: &str,
    diags: &mut Vec<String>,
) -> ReasoningPlan {
    let r = &caps.reasoning;
    let mandatory = r.mandatory;
    if effort == ReasoningEffort::Default {
        return ReasoningPlan {
            value: None,
            thinking_on: mandatory,
        };
    }
    if r.control == ReasoningControl::None {
        diags.push(format!("unsupported-policy:reasoning={}", effort.as_str()));
        return ReasoningPlan::default();
    }
    if r.control == ReasoningControl::ChatTemplateThinking {
        let value = if effort == ReasoningEffort::Off {
            "false"
        } else {
            "true"
        };
        if caps.is_rejected(field, value) {
            diags.push(format!(
                "unsupported-policy:{field}={value} (rejected earlier)"
            ));
            return ReasoningPlan::default();
        }
        return ReasoningPlan {
            value: Some(value.into()),
            thinking_on: value == "true",
        };
    }
    let lowest = r.efficient.clone().or_else(|| r.values.first().cloned());
    let wanted: Option<String> = match effort {
        ReasoningEffort::Off => match (&r.off, mandatory) {
            (Some(off), _) => Some(off.clone()),
            (None, true) => {
                diags.push(format!(
                    "reasoning=off unavailable (mandatory thinking); using {}",
                    lowest.as_deref().unwrap_or("provider default")
                ));
                lowest
            }
            (None, false) => None,
        },
        ReasoningEffort::Efficient => r.efficient.clone(),
        e => {
            let v = e.as_str().to_string();
            r.values.contains(&v).then_some(v)
        }
    };
    let Some(value) = wanted else {
        diags.push(format!(
            "unsupported-policy:reasoning={} (not a confirmed value for this model)",
            effort.as_str()
        ));
        return ReasoningPlan {
            value: None,
            thinking_on: mandatory,
        };
    };
    if caps.is_rejected(field, &value) {
        diags.push(format!(
            "unsupported-policy:{field}={value} (rejected earlier by this route)"
        ));
        return ReasoningPlan {
            value: None,
            thinking_on: mandatory,
        };
    }
    let off = r.off.as_deref() == Some(value.as_str());
    ReasoningPlan {
        thinking_on: mandatory || !off,
        value: Some(value),
    }
}

/// Total completion ceiling for the request: thinking and visible output
/// share it. Capped at the advertised serving limit.
pub(crate) fn total_ceiling(
    policy: &RequestPolicy,
    thinking_on: bool,
    caps: &ResolvedCapabilities,
) -> Option<u32> {
    let wanted = if thinking_on {
        policy.thinking_ceiling.or(policy.max_output_tokens)
    } else {
        policy.max_output_tokens
    }?;
    Some(
        caps.max_output_tokens
            .map_or(wanted, |limit| wanted.min(limit)),
    )
}

fn output_limit_field(kind: &str) -> Option<&'static str> {
    match kind {
        "openai" => Some("max_completion_tokens"),
        "openrouter" | "google" | "nvidia" | "grok" => Some("max_tokens"),
        _ => None,
    }
}

pub(crate) fn message_json(m: &ChatMessage) -> Value {
    let mut v = json!({ "role": m.role, "content": m.content });
    if let Some(id) = &m.tool_call_id {
        v["tool_call_id"] = json!(id);
    }
    if !m.tool_calls.is_empty() {
        v["tool_calls"] = json!(m
            .tool_calls
            .iter()
            .map(|c| json!({
                "id": c.id,
                "type": "function",
                "function": { "name": c.name, "arguments": c.arguments }
            }))
            .collect::<Vec<_>>());
    }
    v
}

fn tool_json(t: &ToolSpec) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": t.name,
            "description": t.description,
            "parameters": t.parameters,
        }
    })
}

/// The inert output tool for `schema`.
pub fn output_tool(schema: &Value) -> ToolSpec {
    ToolSpec {
        name: OUTPUT_TOOL.into(),
        description: "Submit the final result. This is an output channel, not an action: \
                      call it exactly once with the complete answer."
            .into(),
        parameters: schema.clone(),
    }
}

/// Choose the structured-output mode for a contract.
pub(crate) fn choose_structured(
    contract: &ResponseContract,
    caps: &ResolvedCapabilities,
    schema_supported: Support,
    schema_field: &str,
    diags: &mut Vec<String>,
) -> StructuredMode {
    match contract {
        ResponseContract::Prose => StructuredMode::Prose,
        ResponseContract::BoundedDecision
        | ResponseContract::GeneratedJson { schema: None, .. } => {
            if caps.json_mode.is_supported() && !caps.is_rejected("response_format", "json_object")
            {
                StructuredMode::JsonMode
            } else {
                diags.push("unsupported-policy:response_format (local validation)".into());
                StructuredMode::ValidatedText
            }
        }
        ResponseContract::GeneratedJson {
            schema: Some(_), ..
        } => {
            if schema_supported.is_supported() && !caps.is_rejected(schema_field, "json_schema") {
                if schema_field == "response_format" {
                    StructuredMode::StrictSchema
                } else {
                    StructuredMode::NativeSchema
                }
            } else if caps.output_tool_usable() {
                StructuredMode::OutputTool
            } else if caps.json_mode.is_supported()
                && !caps.is_rejected("response_format", "json_object")
            {
                diags.push("schema downgraded to JSON mode (local validation)".into());
                StructuredMode::JsonMode
            } else {
                diags.push("schema downgraded to validated text (local validation)".into());
                StructuredMode::ValidatedText
            }
        }
    }
}

fn compile_chat(
    secret: &ProviderSecret,
    req: &OperationRequest<'_>,
    caps: &ResolvedCapabilities,
    dialect: ApiDialect,
) -> CompiledRequest {
    let kind = crate::provider::effective_kind(secret);
    let mut diags = Vec::new();
    let mut sent = Vec::new();
    let model = if caps.adapter == AdapterId::Google {
        crate::provider_capabilities::normalize_model_key(&secret.model)
    } else {
        secret.model.trim().to_string()
    };
    let msgs: Vec<Value> = req.messages.iter().map(message_json).collect();
    let mut body = json!({"model": model, "messages": msgs, "stream": req.stream});
    if let Some(t) = caps.temperature {
        body["temperature"] = json!(t);
    }
    let mut channel = ResponseChannel::Content;
    let structured = choose_structured(
        req.contract,
        caps,
        caps.strict_schema,
        "response_format",
        &mut diags,
    );
    match (structured, req.contract) {
        (StructuredMode::StrictSchema, ResponseContract::GeneratedJson { name, schema }) => {
            body["response_format"] = json!({"type": "json_schema", "json_schema": {
                "name": name, "strict": true, "schema": schema,
            }});
            sent.push(SentOption {
                field: "response_format".into(),
                value: "json_schema".into(),
                aliases: vec!["json_schema".into(), "structured output".into()],
            });
        }
        (
            StructuredMode::OutputTool,
            ResponseContract::GeneratedJson {
                schema: Some(schema),
                ..
            },
        ) => {
            body["tools"] = json!([tool_json(&output_tool(schema))]);
            body["tool_choice"] = json!({"type": "function", "function": {"name": OUTPUT_TOOL}});
            channel = ResponseChannel::OutputTool(OUTPUT_TOOL.into());
            sent.push(SentOption {
                field: "tool_choice".into(),
                value: "function".into(),
                aliases: vec!["tool".into()],
            });
        }
        (StructuredMode::JsonMode, _) => {
            body["response_format"] = json!({"type": "json_object"});
            sent.push(SentOption {
                field: "response_format".into(),
                value: "json_object".into(),
                aliases: vec!["json mode".into(), "json_object".into()],
            });
        }
        _ => {}
    }
    if !req.tools.is_empty() && channel == ResponseChannel::Content {
        body["tools"] = json!(req.tools.iter().map(tool_json).collect::<Vec<_>>());
        body["tool_choice"] = json!("auto");
    }
    let plan = match caps.adapter {
        AdapterId::OpenRouter => {
            openrouter::apply_reasoning(&mut body, req.policy, caps, &mut sent, &mut diags)
        }
        AdapterId::Nvidia | AdapterId::OpenAiCompatible | AdapterId::Google => {
            nvidia::apply_reasoning(&mut body, req.policy, caps, &mut sent, &mut diags)
        }
    };
    let (budget, budget_kind) = budget_fields(&mut body, req.policy, caps, &plan, &mut sent);
    // A *confirmed* (enforced) reasoning budget lets the stage use its
    // tighter budgeted ceiling (spec §5: Inferences 4096 with a hard 1024
    // budget, otherwise the thinking ceiling).
    let ceiling = match (budget_kind.as_deref(), req.policy.budgeted_ceiling) {
        (Some("hard"), Some(tight)) if plan.thinking_on => Some(
            caps.max_output_tokens
                .map_or(tight, |limit| tight.min(limit)),
        ),
        _ => total_ceiling(req.policy, plan.thinking_on, caps),
    };
    if let Some(tokens) = ceiling {
        match output_limit_field(&kind) {
            Some(field) if !caps.is_rejected(field, "*") => {
                body[field] = json!(tokens);
            }
            Some(field) => diags.push(format!("unsupported-policy:{field} (rejected earlier)")),
            None => diags.push("unsupported-policy:max_output_tokens".into()),
        }
    }
    if caps.adapter == AdapterId::OpenRouter && !sent.is_empty() {
        // Never let the gateway silently ignore a requested capability.
        body["provider"] = json!({"require_parameters": true});
    }
    CompiledRequest {
        adapter: caps.adapter,
        dialect,
        endpoint: format!(
            "{}/chat/completions",
            crate::provider::normalize_base(&secret.base_url)
        ),
        auth: AuthStyle::Bearer,
        body,
        channel,
        stream: req.stream,
        effective: EffectivePolicy {
            adapter: caps.adapter.as_str().into(),
            dialect: dialect.as_str().into(),
            contract: req.contract.as_str().into(),
            structured,
            reasoning: reasoning_label(&sent),
            thinking_on: plan.thinking_on,
            total_ceiling: ceiling,
            reasoning_budget: budget,
            budget_kind,
            capability_digest: caps.digest(),
            sent,
            diagnostics: diags,
        },
    }
}

fn budget_fields(
    body: &mut Value,
    policy: &RequestPolicy,
    caps: &ResolvedCapabilities,
    plan: &ReasoningPlan,
    sent: &mut Vec<SentOption>,
) -> (Option<u32>, Option<String>) {
    let (Some(budget), true) = (policy.reasoning_budget, plan.thinking_on) else {
        return (None, None);
    };
    let kind = if caps.reasoning.budget_enforced {
        "hard"
    } else {
        "advisory"
    };
    if let Some(field) = caps.reasoning.budget_field.as_deref() {
        if !caps.is_rejected(field, "*") {
            body[field] = json!(budget);
            sent.push(SentOption {
                field: field.into(),
                value: budget.to_string(),
                aliases: vec!["budget".into()],
            });
            return (Some(budget), Some(kind.into()));
        }
    }
    if caps.reasoning.control == ReasoningControl::OpenRouter && caps.reasoning.max_tokens {
        body["reasoning"]["max_tokens"] = json!(budget);
        return (Some(budget), Some(kind.into()));
    }
    (None, None)
}

fn reasoning_label(sent: &[SentOption]) -> Option<String> {
    sent.iter()
        .find(|o| {
            o.field.starts_with("reasoning") && o.field != "reasoning_budget"
                || o.field.contains("thinking")
        })
        .map(|o| format!("{}={}", o.field, o.value))
}

// ---------- Response normalization ----------

/// Parse a final (non-streamed) body for `compiled` and apply its channel.
pub fn normalize_final(
    compiled: &CompiledRequest,
    text: &str,
) -> Result<Completion, ProviderFailure> {
    let completion = match compiled.dialect {
        ApiDialect::GoogleGenerateContent => google::parse_final(text)?,
        _ => parse_chat_final(text)?,
    };
    finalize_channel(completion, &compiled.channel)
}

pub(crate) fn parse_usage(v: &Value) -> Option<Usage> {
    let u = v.get("usage").filter(|u| u.is_object())?;
    let n = |p: &str| u.pointer(p).and_then(Value::as_u64);
    let completion = n("/completion_tokens");
    let reasoning = n("/completion_tokens_details/reasoning_tokens").or(n("/reasoning_tokens"));
    Some(Usage {
        prompt_tokens: n("/prompt_tokens"),
        completion_tokens: completion,
        reasoning_tokens: reasoning,
        visible_tokens: match (completion, reasoning) {
            (Some(c), Some(r)) => Some(c.saturating_sub(r)),
            _ => None,
        },
        cost_usd: u.get("cost").and_then(Value::as_f64),
    })
}

fn parse_tool_calls(msg: &Value) -> Vec<ToolCall> {
    msg.get("tool_calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|call| ToolCall {
            id: call
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("call")
                .to_string(),
            name: call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            arguments: match call.pointer("/function/arguments") {
                Some(Value::String(s)) => s.clone(),
                Some(v) if v.is_object() => v.to_string(),
                _ => String::new(),
            },
        })
        .collect()
}

/// OpenAI-compatible chat final body (content, reasoning, tool calls, usage).
pub fn parse_chat_final(text: &str) -> Result<Completion, ProviderFailure> {
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
    if v.pointer("/choices/0").is_none() {
        return Err(ProviderFailure::new(
            Stage::Parse,
            Category::MalformedPayload,
            "response JSON has no choices",
        ));
    }
    let msg = v
        .pointer("/choices/0/message")
        .cloned()
        .unwrap_or(Value::Null);
    Ok(Completion {
        content: crate::provider::message_text(&msg),
        reasoning: crate::provider::message_reasoning(&msg),
        tool_calls: parse_tool_calls(&msg),
        finish_reason: v
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        refusal: msg
            .get("refusal")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        usage: parse_usage(&v),
    })
}

/// Apply the response channel: an output-tool answer must be exactly one
/// complete `submit_result` call with a JSON object argument; its arguments
/// become the answer. No tool is executed and no second round trip is made.
pub fn finalize_channel(
    mut c: Completion,
    channel: &ResponseChannel,
) -> Result<Completion, ProviderFailure> {
    let ResponseChannel::OutputTool(name) = channel else {
        return Ok(c);
    };
    let fail = |category: Category, msg: String| {
        let mut f = ProviderFailure::new(Stage::Parse, category, msg);
        f.retryable = false;
        f
    };
    if let Some(r) = c.refusal.as_deref().filter(|r| !r.trim().is_empty()) {
        let mut f = fail(Category::Refused, "model refused".into());
        f.provider_message = Some(crate::provider_diag::sanitize(r));
        return Err(f);
    }
    if c.finish_reason.as_deref() == Some("length") {
        return Err(fail(
            Category::TokenLimit,
            "output tool arguments truncated (finish_reason=length)".into(),
        ));
    }
    match c.tool_calls.as_slice() {
        [] => Err(fail(
            Category::InvalidResult,
            format!("expected one {name} call, got none"),
        )),
        [call] if call.name != *name => Err(fail(
            Category::InvalidResult,
            format!("unknown output tool {:?}", call.name),
        )),
        [call] => {
            let args: Value = serde_json::from_str(call.arguments.trim()).map_err(|e| {
                fail(
                    Category::InvalidResult,
                    format!("output tool arguments are not valid JSON: {e}"),
                )
            })?;
            if !args.is_object() {
                return Err(fail(
                    Category::InvalidResult,
                    "output tool arguments must be a JSON object".into(),
                ));
            }
            c.content = call.arguments.trim().to_string();
            c.tool_calls.clear();
            Ok(c)
        }
        calls => Err(fail(
            Category::InvalidResult,
            format!("expected one {name} call, got {}", calls.len()),
        )),
    }
}

/// Incremental decoder for one streamed response (chat chunks or Google
/// `streamGenerateContent` SSE). Error events are handled by the caller.
#[derive(Clone, Debug)]
pub struct StreamDecoder {
    dialect: ApiDialect,
    pub content: String,
    pub reasoning: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: Option<String>,
    pub refusal: Option<String>,
    pub usage: Option<Usage>,
    /// Characters of output-tool arguments received (meaningful output).
    pub tool_chars: u64,
}

/// What one event added.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecodedDelta {
    pub content: Option<String>,
    pub reasoning: bool,
    pub tool_args: bool,
}

impl StreamDecoder {
    pub fn new(dialect: ApiDialect) -> Self {
        Self {
            dialect,
            content: String::new(),
            reasoning: String::new(),
            tool_calls: Vec::new(),
            finish_reason: None,
            refusal: None,
            usage: None,
            tool_chars: 0,
        }
    }

    /// Apply one parsed `data:` JSON event.
    pub fn apply(&mut self, v: &Value) -> DecodedDelta {
        if self.dialect == ApiDialect::GoogleGenerateContent {
            return google::apply_stream_event(self, v);
        }
        let mut out = DecodedDelta::default();
        if let Some(usage) = parse_usage(v) {
            self.usage = Some(usage);
        }
        if let Some(reason) = v
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            self.finish_reason = Some(reason.to_string());
        }
        let Some(delta) = v.pointer("/choices/0/delta") else {
            return out;
        };
        let text = |key: &str| {
            let t = crate::provider::value_text_pub(delta.get(key));
            (!t.is_empty()).then_some(t)
        };
        if let Some(t) = text("content") {
            self.content.push_str(&t);
            out.content = Some(t);
        }
        for key in ["reasoning_content", "reasoning"] {
            if let Some(t) = text(key) {
                self.reasoning.push_str(&t);
                out.reasoning = true;
            }
        }
        if let Some(r) = delta
            .get("refusal")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|r| !r.is_empty())
        {
            self.refusal = Some(r.to_string());
        }
        for call in delta
            .get("tool_calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let idx = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            // Assemble by index; an ID that names a different call at the
            // same index starts a new call.
            while self.tool_calls.len() <= idx {
                self.tool_calls.push(ToolCall {
                    id: String::new(),
                    name: String::new(),
                    arguments: String::new(),
                });
            }
            let slot = &mut self.tool_calls[idx];
            if let Some(id) = call.get("id").and_then(Value::as_str) {
                if slot.id.is_empty() {
                    slot.id = id.to_string();
                }
            }
            if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                slot.name.push_str(name);
            }
            if let Some(args) = call.pointer("/function/arguments").and_then(Value::as_str) {
                if !args.is_empty() {
                    slot.arguments.push_str(args);
                    self.tool_chars += args.chars().count() as u64;
                    out.tool_args = true;
                }
            }
        }
        out
    }

    pub fn into_completion(self) -> Completion {
        Completion {
            content: self.content,
            reasoning: self.reasoning,
            tool_calls: self.tool_calls,
            finish_reason: self.finish_reason,
            refusal: self.refusal,
            usage: self.usage,
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
