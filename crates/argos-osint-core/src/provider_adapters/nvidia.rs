//! Nvidia hosted chat adapter (also used for generic OpenAI-compatible
//! endpoints). Reasoning controls are profile data: `reasoning_effort` with
//! the model's own vocabulary (Super none/low/high, Ultra none/medium/high),
//! Lightning's `chat_template_kwargs.enable_thinking`, or nothing (DeepSeek
//! and unknown models). Self-hosted NIM parameters are never applied to a
//! hosted endpoint automatically.

use serde_json::{json, Value};

use super::{plan_reasoning, ReasoningPlan, SentOption};
use crate::provider_capabilities::{ReasoningControl, ResolvedCapabilities};
use crate::request_policy::RequestPolicy;

pub(crate) fn apply_reasoning(
    body: &mut Value,
    policy: &RequestPolicy,
    caps: &ResolvedCapabilities,
    sent: &mut Vec<SentOption>,
    diags: &mut Vec<String>,
) -> ReasoningPlan {
    match caps.reasoning.control {
        ReasoningControl::ReasoningEffort => {
            let plan = plan_reasoning(caps, policy.reasoning, "reasoning_effort", diags);
            if let Some(v) = &plan.value {
                body["reasoning_effort"] = json!(v);
                sent.push(SentOption {
                    field: "reasoning_effort".into(),
                    value: v.clone(),
                    aliases: vec!["effort".into()],
                });
            }
            plan
        }
        ReasoningControl::ChatTemplateThinking => {
            let plan = plan_reasoning(caps, policy.reasoning, "enable_thinking", diags);
            if let Some(v) = &plan.value {
                body["chat_template_kwargs"] = json!({"enable_thinking": v == "true"});
                sent.push(SentOption {
                    field: "enable_thinking".into(),
                    value: v.clone(),
                    aliases: vec!["chat_template_kwargs".into(), "thinking".into()],
                });
            }
            plan
        }
        _ => {
            // No chat-dialect reasoning control confirmed (DeepSeek, unknown
            // models, Google levels outside the native API): omit.
            let mut none = caps.clone();
            none.reasoning.control = ReasoningControl::None;
            let mandatory = caps.reasoning.mandatory;
            let mut plan = plan_reasoning(&none, policy.reasoning, "reasoning", diags);
            plan.thinking_on |= mandatory;
            plan
        }
    }
}
