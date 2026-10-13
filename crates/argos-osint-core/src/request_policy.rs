//! Per-operation request policy: output ceilings, structured-output mode and
//! reasoning effort, applied through one capability table per transport.
//!
//! Output-limit support is independent of JSON-mode support (spec §2): every
//! OpenAI-compatible builder we know (OpenRouter, Google, Nvidia, x.ai,
//! OpenAI) receives its ceiling; unknown transports keep strict local
//! validation and record an explicit `unsupported-policy:*` diagnostic. A
//! provider that rejects an option is remembered per (provider, model, field)
//! for the process, so the same route is not failed by it again.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::secrets::ProviderSecret;

/// Requested reasoning effort. `Default` sends nothing (provider default).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    #[default]
    Default,
    Off,
    Low,
    Medium,
    High,
}

impl ReasoningEffort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Off => "off",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    /// Hidden reasoning tokens reserved on top of the answer ceiling where the
    /// provider counts reasoning inside its completion limit.
    pub fn reserve_tokens(self) -> u32 {
        match self {
            Self::Default | Self::Off => 0,
            Self::Low => 1024,
            Self::Medium => 4096,
            Self::High => 8192,
        }
    }
}

/// Request policy for one dispatch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequestPolicy {
    /// Answer ceiling (excluding any reasoning reserve).
    pub max_output_tokens: Option<u32>,
    /// Ask for `response_format: json_object` where the transport supports it.
    pub json_object: bool,
    pub reasoning: ReasoningEffort,
}

impl RequestPolicy {
    pub fn is_empty(&self) -> bool {
        self.max_output_tokens.is_none()
            && !self.json_object
            && self.reasoning == ReasoningEffort::Default
    }
}

/// Fields a policy may add; also the keys used for learned rejections.
pub const FIELD_MAX_TOKENS: &str = "max_tokens";
pub const FIELD_MAX_COMPLETION_TOKENS: &str = "max_completion_tokens";
pub const FIELD_RESPONSE_FORMAT: &str = "response_format";
pub const FIELD_REASONING: &str = "reasoning";
pub const FIELD_REASONING_EFFORT: &str = "reasoning_effort";

/// What one transport accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub output_limit: Option<&'static str>,
    /// `response_format: json_object` (only the JSON-mode adapters).
    pub json_object: bool,
    /// Reasoning dialect: `reasoning:{effort|enabled}` (OpenRouter) or
    /// `reasoning_effort` (OpenAI-compatible Google/OpenAI/x.ai).
    pub reasoning: Option<&'static str>,
    /// Whether `Off` can be expressed.
    pub reasoning_off: bool,
    /// Whether reasoning tokens count inside the completion limit.
    pub reasoning_in_limit: bool,
}

pub fn capabilities(kind: &str, model: &str) -> Capabilities {
    let reasoning_model = {
        let m = model.to_ascii_lowercase();
        m.starts_with('o') && m.chars().nth(1).is_some_and(|c| c.is_ascii_digit())
            || m.starts_with("gpt-5")
    };
    match kind {
        "openrouter" => Capabilities {
            output_limit: Some(FIELD_MAX_TOKENS),
            json_object: true,
            reasoning: Some(FIELD_REASONING),
            reasoning_off: true,
            reasoning_in_limit: true,
        },
        "google" => Capabilities {
            output_limit: Some(FIELD_MAX_TOKENS),
            json_object: false,
            reasoning: Some(FIELD_REASONING_EFFORT),
            reasoning_off: true,
            reasoning_in_limit: true,
        },
        "nvidia" => Capabilities {
            output_limit: Some(FIELD_MAX_TOKENS),
            json_object: false,
            reasoning: None,
            reasoning_off: false,
            reasoning_in_limit: false,
        },
        "grok" => Capabilities {
            output_limit: Some(FIELD_MAX_TOKENS),
            json_object: true,
            reasoning: Some(FIELD_REASONING_EFFORT),
            reasoning_off: false,
            reasoning_in_limit: true,
        },
        "openai" => Capabilities {
            output_limit: Some(FIELD_MAX_COMPLETION_TOKENS),
            json_object: false,
            reasoning: reasoning_model.then_some(FIELD_REASONING_EFFORT),
            reasoning_off: false,
            reasoning_in_limit: true,
        },
        _ => Capabilities {
            output_limit: None,
            json_object: false,
            reasoning: None,
            reasoning_off: false,
            reasoning_in_limit: false,
        },
    }
}

fn learned() -> &'static Mutex<HashSet<(String, String, String)>> {
    static SET: OnceLock<Mutex<HashSet<(String, String, String)>>> = OnceLock::new();
    SET.get_or_init(|| Mutex::new(HashSet::new()))
}

fn rejected(kind: &str, model: &str, field: &str) -> bool {
    learned()
        .lock()
        .map(|s| s.contains(&(kind.into(), model.into(), field.into())))
        .unwrap_or(false)
}

/// Remember that this route rejects `field`.
pub fn learn_unsupported(kind: &str, model: &str, field: &str) {
    if let Ok(mut s) = learned().lock() {
        s.insert((kind.into(), model.into(), field.into()));
    }
}

#[cfg(test)]
pub fn forget_learned() {
    if let Ok(mut s) = learned().lock() {
        s.clear();
    }
}

/// Apply `policy` to a chat body. Returns diagnostics (never secrets/text).
pub fn apply(secret: &ProviderSecret, body: &mut Value, policy: &RequestPolicy) -> Vec<String> {
    use crate::investigation::decisions::{resolve_adapter, DecisionAdapterKind};
    let kind = crate::provider::effective_kind(secret);
    let mut policy = *policy;
    // Structured output stays on the capability-mapped JSON-mode adapters.
    if policy.json_object && resolve_adapter(&secret.model, &kind) != DecisionAdapterKind::JsonMode
    {
        policy.json_object = false;
    }
    apply_for(&kind, &secret.model, body, &policy)
}

pub fn apply_for(kind: &str, model: &str, body: &mut Value, policy: &RequestPolicy) -> Vec<String> {
    let caps = capabilities(kind, model);
    let mut diags = Vec::new();
    let usable = |field: &'static str, diags: &mut Vec<String>| {
        if rejected(kind, model, field) {
            diags.push(format!(
                "unsupported-policy:{field} (rejected earlier by this route)"
            ));
            false
        } else {
            true
        }
    };
    let mut reasoning_sent = false;
    match (policy.reasoning, caps.reasoning) {
        (ReasoningEffort::Default, _) => {}
        (effort, None) => diags.push(format!("unsupported-policy:reasoning={}", effort.as_str())),
        (ReasoningEffort::Off, Some(_)) if !caps.reasoning_off => {
            diags.push("unsupported-policy:reasoning=off".into())
        }
        (effort, Some(field)) => {
            if usable(field, &mut diags) {
                let value = match (field, effort) {
                    (FIELD_REASONING, ReasoningEffort::Off) => json!({"enabled": false}),
                    (FIELD_REASONING, e) => json!({"effort": e.as_str()}),
                    (_, ReasoningEffort::Off) => json!("none"),
                    (_, e) => json!(e.as_str()),
                };
                body[field] = value;
                reasoning_sent = effort != ReasoningEffort::Off;
            }
        }
    }
    if let Some(tokens) = policy.max_output_tokens {
        match caps.output_limit {
            Some(field) if usable(field, &mut diags) => {
                let reserve = if reasoning_sent && caps.reasoning_in_limit {
                    policy.reasoning.reserve_tokens()
                } else {
                    0
                };
                body[field] = json!(tokens + reserve);
            }
            Some(_) => {}
            None => diags.push("unsupported-policy:max_output_tokens".into()),
        }
    }
    if policy.json_object {
        if caps.json_object && usable(FIELD_RESPONSE_FORMAT, &mut diags) {
            body[FIELD_RESPONSE_FORMAT] = json!({"type": "json_object"});
        } else if !caps.json_object {
            diags.push("unsupported-policy:response_format (local validation)".into());
        }
    }
    diags
}

/// Which policy fields a 400/422 provider message names, if any. Only fields
/// actually present in the sent body are candidates.
pub fn rejected_fields(body: &Value, message: &str) -> Vec<&'static str> {
    let lower = message.to_ascii_lowercase();
    let generic = [
        "unsupported",
        "unknown",
        "unrecognized",
        "not supported",
        "invalid",
        "not allowed",
        "extra",
    ]
    .iter()
    .any(|w| lower.contains(w));
    [
        FIELD_REASONING_EFFORT,
        FIELD_REASONING,
        FIELD_MAX_COMPLETION_TOKENS,
        FIELD_MAX_TOKENS,
        FIELD_RESPONSE_FORMAT,
    ]
    .into_iter()
    .filter(|f| body.get(*f).is_some())
    .filter(|f| {
        lower.contains(f)
            || (*f == FIELD_RESPONSE_FORMAT && lower.contains("json mode"))
            || (*f == FIELD_REASONING && lower.contains("thinking"))
    })
    .filter(|_| generic)
    // `reasoning` is a prefix of `reasoning_effort`; keep the specific one.
    .fold(Vec::new(), |mut acc, f| {
        if !(f == FIELD_REASONING && acc.contains(&FIELD_REASONING_EFFORT)) {
            acc.push(f);
        }
        acc
    })
}

#[cfg(test)]
#[path = "request_policy_tests.rs"]
mod tests;
