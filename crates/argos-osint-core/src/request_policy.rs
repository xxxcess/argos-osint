//! Per-operation request policy: answer target, total completion ceiling,
//! reasoning intent and structured-output request.
//!
//! The policy is provider-neutral. Provider adapters
//! (`crate::provider_adapters`) compile it against the selected model's
//! resolved capabilities (`crate::provider_capabilities`): unknown optional
//! fields are omitted and the answer is validated locally, and every omission
//! is reported as an `unsupported-policy:*` diagnostic.

use serde::{Deserialize, Serialize};

/// Requested reasoning intent. `Default` sends nothing (provider default).
/// `Efficient` resolves to the model profile's low-cost value (for example
/// Nemotron Ultra `medium`, Super `low`, Gemini Flash `low`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    #[default]
    Default,
    Off,
    Efficient,
    Low,
    Medium,
    High,
}

impl ReasoningEffort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Off => "off",
            Self::Efficient => "efficient",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Request policy for one dispatch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequestPolicy {
    /// Total completion ceiling when thinking is off (equals the answer target).
    pub max_output_tokens: Option<u32>,
    /// Total completion ceiling when the model thinks (mandatory thinking or
    /// an effort was sent). Thinking and visible output share this ceiling;
    /// no hidden reserve is added. `None` = `max_output_tokens`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_ceiling: Option<u32>,
    /// Separate reasoning budget, sent only where the profile documents a
    /// budget field. Advisory unless the endpoint enforces it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_budget: Option<u32>,
    /// Ask for JSON output (strict schema / output tool / JSON mode, as the
    /// selected model supports).
    pub json_object: bool,
    pub reasoning: ReasoningEffort,
}

impl RequestPolicy {
    pub fn is_empty(&self) -> bool {
        self.max_output_tokens.is_none()
            && self.thinking_ceiling.is_none()
            && self.reasoning_budget.is_none()
            && !self.json_object
            && self.reasoning == ReasoningEffort::Default
    }
}

/// Which of the options actually sent a 400/422 provider message names.
/// Returns indexes into `sent`. A message must both name the field (or an
/// alias) and use rejection wording; anything else is not guessed.
pub fn rejected_options(
    sent: &[crate::provider_adapters::SentOption],
    message: &str,
) -> Vec<usize> {
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
    if !generic {
        return Vec::new();
    }
    let named: Vec<usize> = sent
        .iter()
        .enumerate()
        .filter(|(_, o)| {
            std::iter::once(o.field.as_str())
                .chain(o.aliases.iter().map(String::as_str))
                .any(|name| lower.contains(&name.to_ascii_lowercase()))
        })
        .map(|(i, _)| i)
        .collect();
    // `reasoning` is a prefix of `reasoning_effort`/`reasoning_budget`: keep
    // the specific field when both match.
    named
        .iter()
        .copied()
        .filter(|&i| {
            let f = &sent[i].field;
            !named.iter().any(|&j| {
                j != i && sent[j].field.len() > f.len() && sent[j].field.starts_with(f.as_str())
            })
        })
        .collect()
}
