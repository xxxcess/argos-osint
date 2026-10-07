//! Argos adaptive decision roles and strict output contracts.
//!
//! Provides canonical DecisionContract compilation across Jev-native and general-model
//! execution adapters with strict output contracts and threshold policies.

pub mod adapters;
pub mod contracts;
pub mod policy;
pub mod state;
pub mod templates;

#[cfg(test)]
pub mod tests;

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::time::Instant;

pub use adapters::{
    compile_general_model_prompt, compile_native, parse_general_model_response,
    parse_native_response, resolve_adapter, DecisionAdapterKind,
};
pub use contracts::{
    DecisionContract, DecisionQuestionType, DecisionValidationStatus, NormalizedAnswer,
    NormalizedDecisionResult, QuestionSpec,
};
pub use policy::{DecisionPolicy, EnforcementOutcome, ThresholdProfile};
pub use state::DecisionState;

use crate::provider::{complete, decide, ChatMessage};
use crate::secrets::ProviderSecret;

/// Core service for evaluating adaptive decision contracts.
#[derive(Clone, Debug, Default)]
pub struct DecisionService;

impl DecisionService {
    pub fn new() -> Self {
        Self
    }

    /// Execute a decision contract against a configured provider endpoint and evaluate results.
    pub async fn evaluate(
        &self,
        secret: &ProviderSecret,
        contract: &DecisionContract,
        state: &DecisionState,
        policy: &DecisionPolicy,
    ) -> Result<(
        NormalizedDecisionResult,
        BTreeMap<String, EnforcementOutcome>,
    )> {
        let adapter = resolve_adapter(&secret.model, &secret.kind);
        let start = Instant::now();

        let state_val = state.to_value();

        let mut result = match adapter {
            DecisionAdapterKind::NativeDecisions => {
                let (native_state, native_questions) = compile_native(contract, &state_val);
                let resp = decide(secret, &native_state, &native_questions)
                    .await
                    .context("call native decisions API")?;
                parse_native_response(&resp, contract).context("parse native decisions response")?
            }
            _ => {
                let (system_prompt, user_prompt) =
                    compile_general_model_prompt(contract, &state_val);
                let messages = [
                    ChatMessage {
                        role: "system".into(),
                        content: system_prompt,
                        tool_call_id: None,
                        tool_calls: Vec::new(),
                    },
                    ChatMessage {
                        role: "user".into(),
                        content: user_prompt,
                        tool_call_id: None,
                        tool_calls: Vec::new(),
                    },
                ];
                let completion = complete(secret, &messages, &[], |_| {})
                    .await
                    .context("call general model for decision")?;
                parse_general_model_response(&completion.content, contract, &secret.model, adapter)
                    .context("parse general-model decision response")?
            }
        };

        result.latency_ms = start.elapsed().as_millis() as u64;

        let mut outcomes = BTreeMap::new();
        for (qid, answer) in &result.answers {
            let outcome = policy.evaluate(qid, answer);
            outcomes.insert(qid.clone(), outcome);
        }

        Ok((result, outcomes))
    }
}
