//! OpenRouter adapter: chat completions (and native Decisions via
//! [`super::compile_decisions`]). Reasoning uses OpenRouter's own
//! `reasoning` object with catalog/profile-confirmed efforts only; native
//! Nvidia/Google fields are never sent through OpenRouter. Any used optional
//! capability sets `provider.require_parameters=true` (in `compile_chat`).

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
    if caps.reasoning.control != ReasoningControl::OpenRouter {
        let mut none = caps.clone();
        none.reasoning.control = ReasoningControl::None;
        return plan_reasoning(&none, policy.reasoning, "reasoning", diags);
    }
    let plan = plan_reasoning(caps, policy.reasoning, "reasoning", diags);
    if let Some(value) = &plan.value {
        body["reasoning"] = if caps.reasoning.off.as_deref() == Some(value.as_str()) {
            json!({"enabled": false})
        } else {
            json!({"effort": value})
        };
        sent.push(SentOption {
            field: "reasoning".into(),
            value: value.clone(),
            aliases: vec!["thinking".into(), "effort".into()],
        });
    }
    plan
}
