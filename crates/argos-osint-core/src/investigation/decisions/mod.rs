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

use anyhow::Result;
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

use crate::provider::{ChatMessage, SettingsFile};
use crate::recon::model_exec::{self, DecisionsAdapter, OperationScope};
use crate::secrets::AuthFile;
use crate::secrets::ProviderSecret;

pub struct ContractAdapter<'a> {
    pub contract: &'a DecisionContract,
    pub state: &'a DecisionState,
}
impl DecisionsAdapter<NormalizedDecisionResult> for ContractAdapter<'_> {
    fn compile_native(&self) -> (serde_json::Value, serde_json::Value) {
        compile_native(self.contract, &self.state.to_value())
    }
    fn parse_native(
        &self,
        response: &crate::provider::DecisionsResponse,
    ) -> Result<NormalizedDecisionResult, String> {
        parse_native_response(response, self.contract).map_err(|e| e.to_string())
    }
    fn compile_chat(&self) -> Vec<ChatMessage> {
        let (system, user) = compile_general_model_prompt(self.contract, &self.state.to_value());
        vec![
            ChatMessage {
                role: "system".into(),
                content: system,
                tool_call_id: None,
                tool_calls: vec![],
            },
            ChatMessage {
                role: "user".into(),
                content: user,
                tool_call_id: None,
                tool_calls: vec![],
            },
        ]
    }
    fn parse_chat(&self, raw: &str) -> Result<NormalizedDecisionResult, String> {
        parse_general_model_response(raw, self.contract, "", DecisionAdapterKind::ValidatedText)
            .map_err(|e| e.to_string())
    }
}

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
        let mut auth = AuthFile::default();
        auth.set_account(secret.clone());
        let mut settings = SettingsFile::default();
        let assignment = crate::provider::ModelAssignment {
            provider: secret.kind.clone(),
            model: secret.model.clone(),
            ..Default::default()
        };
        // This compatibility entrypoint receives an already resolved route.
        settings.defaults.synthesis = assignment.clone();
        settings.defaults.classifier = assignment.clone();
        settings.defaults.recon = assignment;
        let id = format!(
            "decision:{}:{}",
            contract.template_name,
            crate::nlp::cache::hash(&state.to_value().to_string())
        );
        let scope = OperationScope::new(&id, &contract.role_id);
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.evaluate_shared(
            &auth, &settings, &scope, contract, state, policy, &cancel, None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn evaluate_shared(
        &self,
        auth: &AuthFile,
        settings: &SettingsFile,
        scope: &OperationScope<'_>,
        contract: &DecisionContract,
        state: &DecisionState,
        policy: &DecisionPolicy,
        cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        db_path: Option<&std::path::Path>,
    ) -> Result<(
        NormalizedDecisionResult,
        BTreeMap<String, EnforcementOutcome>,
    )> {
        let start = Instant::now();
        let adapter = ContractAdapter { contract, state };
        let (mut result, report) = model_exec::execute_decisions_or_chat(
            auth, settings, scope, &adapter, cancel, None, db_path,
        )
        .await?;
        result.model_used = report.effective_model;
        result.latency_ms = start.elapsed().as_millis() as u64;

        let mut outcomes = BTreeMap::new();
        for (qid, answer) in &result.answers {
            let outcome = policy.evaluate(qid, answer);
            outcomes.insert(qid.clone(), outcome);
        }

        Ok((result, outcomes))
    }
}
