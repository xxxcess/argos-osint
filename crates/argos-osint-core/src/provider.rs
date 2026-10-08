//! Text and voice providers.
//!
//! The agent loop speaks one protocol: OpenAI-compatible chat completions,
//! plus `/audio/transcriptions` for voice. Login chooses which vendor fills
//! that protocol: Grok (`api.x.ai`), OpenAI, OpenRouter, or a local server
//! (Ollama, llama.cpp, LM Studio). Cloud providers take an API key typed at
//! the terminal or the vendor's environment variable. A local server may omit
//! the key. OpenRouter also gets its attribution headers.

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::secrets::{DeviceEndpoints, ProviderSecret};

#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    pub tool_call_id: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Clone, Debug)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Clone, Debug, Default)]
pub struct Completion {
    pub content: String,
    pub reasoning: String,
    pub tool_calls: Vec<ToolCall>,
    /// OpenAI-compatible `choices[0].finish_reason` when the provider sent one.
    pub finish_reason: Option<String>,
    /// Provider refusal text when the model declined instead of answering.
    pub refusal: Option<String>,
}

impl Completion {
    /// True when there is neither answer text, reasoning, nor a tool call.
    pub fn is_empty(&self) -> bool {
        self.content.trim().is_empty()
            && self.reasoning.trim().is_empty()
            && self.tool_calls.is_empty()
    }

    /// Error used when a completion that should contain text is blank.
    pub fn empty_error(&self, what: &str) -> anyhow::Error {
        let mut msg = format!("{what} returned an empty completion");
        if let Some(reason) = self
            .finish_reason
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            msg.push_str(&format!(" (finish_reason={reason})"));
        }
        if let Some(refusal) = self
            .refusal
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            msg.push_str(&format!("; refusal: {refusal}"));
        }
        anyhow!(msg)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct DeviceGrant {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    #[serde(default = "default_expires")]
    pub expires_in: u64,
    #[serde(default = "default_interval")]
    pub interval: u64,
}

fn default_expires() -> u64 {
    600
}
fn default_interval() -> u64 {
    5
}

#[derive(Clone, Debug)]
pub enum Poll {
    Pending,
    SlowDown,
    Token(String),
    Denied(String),
}

pub fn normalize_base(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

/// A vendor the desk knows how to sign in. The chat loop does not branch on
/// these ids; only the endpoint, key, and a few headers do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub base_url: &'static str,
    pub text_model: &'static str,
    pub voice_model: &'static str,
    pub env_key: Option<&'static str>,
    pub key_required: bool,
}

pub fn presets() -> &'static [ProviderPreset] {
    &[
        ProviderPreset {
            id: "grok",
            label: "Grok",
            base_url: "https://api.x.ai/v1",
            text_model: "grok-4.6",
            voice_model: "whisper-1",
            env_key: Some("XAI_API_KEY"),
            key_required: true,
        },
        ProviderPreset {
            id: "openai",
            label: "OpenAI",
            base_url: "https://api.openai.com/v1",
            text_model: "gpt-4.1",
            voice_model: "whisper-1",
            env_key: Some("OPENAI_API_KEY"),
            key_required: true,
        },
        ProviderPreset {
            id: "openrouter",
            label: "OpenRouter",
            base_url: "https://openrouter.ai/api/v1",
            text_model: "openai/gpt-4.1",
            voice_model: "openai/whisper-1",
            env_key: Some("OPENROUTER_API_KEY"),
            key_required: true,
        },
        ProviderPreset {
            id: "google",
            label: "Google",
            base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
            text_model: "",
            voice_model: "",
            env_key: Some("GEMINI_API_KEY"),
            key_required: true,
        },
        ProviderPreset {
            id: "nvidia",
            label: "Nvidia",
            base_url: "https://integrate.api.nvidia.com/v1",
            text_model: "",
            voice_model: "",
            env_key: Some("NVIDIA_API_KEY"),
            key_required: true,
        },
        ProviderPreset {
            id: "local",
            label: "Local",
            base_url: "http://127.0.0.1:11434/v1",
            text_model: "llama3.2",
            voice_model: "whisper",
            env_key: None,
            key_required: false,
        },
    ]
}

pub fn preset(id: &str) -> Option<&'static ProviderPreset> {
    let id = normalize_kind(id);
    presets().iter().find(|preset| preset.id == id)
}

/// Built-in Grok catalog. The default matches the model Grok Build starts
/// on (`grok-4.6` in its `default_models.json`). Older ids stay selectable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrokModel {
    pub id: &'static str,
    pub name: &'static str,
    pub detail: &'static str,
}

pub fn grok_models() -> &'static [GrokModel] {
    &[
        GrokModel {
            id: "grok-4.6",
            name: "Grok 4.6",
            detail: "Current default. Frontier model for agents.",
        },
        GrokModel {
            id: "grok-4.5",
            name: "Grok 4.5",
            detail: "Previous Grok Build default.",
        },
    ]
}

pub fn default_grok_model() -> &'static str {
    grok_models()[0].id
}

/// Exact id or display name, then a single unambiguous prefix. Empty and
/// ambiguous queries return nothing, same rule as `/use`.
pub fn resolve_model_choice(choices: &[(String, String)], query: &str) -> Option<String> {
    let query = query.trim();
    if query.is_empty() {
        return None;
    }
    let ql = query.to_lowercase();
    if let Some((id, _)) = choices
        .iter()
        .find(|(id, label)| id.eq_ignore_ascii_case(query) || label.eq_ignore_ascii_case(query))
    {
        return Some(id.clone());
    }
    let prefs: Vec<_> = choices
        .iter()
        .filter(|(id, label)| {
            id.to_lowercase().starts_with(&ql) || label.to_lowercase().starts_with(&ql)
        })
        .collect();
    if prefs.len() == 1 {
        Some(prefs[0].0.clone())
    } else {
        None
    }
}

/// The text provider a turn should call. With nothing saved, that is Grok
/// on `api.x.ai` using the current default model. `selected` (from
/// `/model`, Ctrl+M, or `-m`) wins over the model stored on the provider.
pub fn active_text_secret(
    auth: &crate::secrets::AuthFile,
    selected: &str,
) -> crate::secrets::ProviderSecret {
    use crate::secrets::ProviderSecret;
    let grok = preset("grok").expect("grok preset");
    let mut secret = auth.text.clone().unwrap_or_else(|| ProviderSecret {
        kind: "grok".into(),
        base_url: grok.base_url.into(),
        model: grok.text_model.into(),
        api_key: None,
        stt_model: Some(grok.voice_model.into()),
        device: None,
    });
    if secret.kind.trim().is_empty() {
        secret.kind = "grok".into();
    }
    if secret.base_url.trim().is_empty() {
        if let Some(preset) = preset(&secret.kind) {
            secret.base_url = preset.base_url.into();
        }
    }
    if !selected.trim().is_empty() {
        secret.model = selected.trim().to_string();
    } else if secret.model.trim().is_empty() {
        secret.model = preset(&effective_kind(&secret))
            .map(|preset| preset.text_model)
            .unwrap_or_else(|| default_grok_model())
            .to_string();
    }
    subscription_connection(secret)
}

/// Account defaults contain no credentials from another provider.
pub fn account_secret(auth: &crate::secrets::AuthFile, kind: &str) -> ProviderSecret {
    let kind = normalize_kind(kind);
    if let Some(secret) = auth.account(&kind) {
        return subscription_connection(secret);
    }
    let preset = preset(&kind);
    subscription_connection(ProviderSecret {
        kind: kind.clone(),
        // A transport identifier lets the existing agent distinguish this
        // configured CLI connection from an absent HTTP provider.
        base_url: preset
            .map(|p| p.base_url)
            .unwrap_or(if kind == "openai-chatgpt" {
                "codex://chatgpt"
            } else {
                ""
            })
            .into(),
        model: preset
            .map(|p| p.text_model)
            .unwrap_or(if kind == "openai-chatgpt" {
                "codex-default"
            } else {
                ""
            })
            .into(),
        api_key: None,
        stt_model: None,
        device: None,
    })
}

/// Keep archived API credentials in AuthFile, but Grok account selection
/// always resolves to subscription auth on the official endpoint.
fn subscription_connection(mut secret: ProviderSecret) -> ProviderSecret {
    if effective_kind(&secret) == "grok" {
        secret.kind = "grok-subscription".into();
        secret.api_key = None;
        secret.base_url = preset("grok").expect("Grok preset").base_url.into();
    }
    secret
}

/// Resolve the writer account and model; old configurations retain their text connection.
pub fn writer_secret(auth: &crate::secrets::AuthFile, settings: &SettingsFile) -> ProviderSecret {
    let kind = &settings.writer_provider;
    let model = &settings.writer_model;
    let mut secret = if kind.trim().is_empty() {
        active_text_secret(auth, &settings.model)
    } else {
        account_secret(auth, kind)
    };
    if !model.trim().is_empty() {
        secret.model = model.trim().into();
    }
    secret
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelAssignment {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RoleDefaults {
    #[serde(default)]
    pub recon: ModelAssignment,
    #[serde(default)]
    pub synthesis: ModelAssignment,
    /// Orders catalog tools for the Recon questions. Seeded to OpenRouter Jev.
    #[serde(default)]
    pub tool_picker: ModelAssignment,
    /// Tags Atlas articles with an OSINT category. Seeded to OpenRouter Jev.
    #[serde(default)]
    pub classifier: ModelAssignment,
    /// Evidence compression and source-grounded views. Empty inherits Synthesis.
    #[serde(default)]
    pub summarization: ModelAssignment,
    /// Source-linked passages, observations, dates. Empty inherits Recon.
    #[serde(default)]
    pub evidence_curator: ModelAssignment,
    /// Identity bindings and conflict resolution. Empty inherits Classifier.
    #[serde(default)]
    pub entity_resolver: ModelAssignment,
    /// Per-claim verification and citation evaluation. Empty inherits Synthesis.
    #[serde(default)]
    pub claim_assessor: ModelAssignment,
    /// Checkpoints, next-task decisions, stopping conditions. Empty inherits Recon.
    #[serde(default)]
    pub investigation_controller: ModelAssignment,
    /// Default model for finite decision roles and semantic gates. Seeded to OpenRouter Jev.
    #[serde(default)]
    pub decision_model: ModelAssignment,
    /// Optional compatible fallback for decision roles.
    #[serde(default)]
    pub decision_fallback: Option<ModelAssignment>,
}

/// Pinned default for the tool picker. The `~typesafe/jev-latest` alias is accepted
/// when an operator sets it, but it is never stored as the default.
pub const TOOL_PICKER_PROVIDER: &str = "openrouter";
pub const TOOL_PICKER_MODEL: &str = "typesafe/jev-1.13";
/// Picker models offered for OpenRouter even when `GET /models` omits them.
pub const DECISIONS_MODELS: &[(&str, &str)] = &[(TOOL_PICKER_MODEL, "Jev 1.13 (decisions)")];

/// Jev returns typed decisions, not completions.
pub fn is_decisions_model(model: &str) -> bool {
    let model = model.trim().to_ascii_lowercase();
    model == TOOL_PICKER_MODEL || model == "~typesafe/jev-latest" || model.contains("/jev")
}

/// `decisions` for Jev models, `chat` for any other tool-picker model.
pub fn picker_transport(model: &str) -> &'static str {
    if is_decisions_model(model) {
        "decisions"
    } else {
        "chat"
    }
}

/// Canonical role name, or `None` for an unknown role.
pub fn role_name(role: &str) -> Option<&'static str> {
    match role.trim().to_ascii_lowercase().as_str() {
        "recon" | "planner" => Some("recon"),
        "synthesis" => Some("synthesis"),
        "tool-picker" | "tool_picker" | "toolpicker" | "picker" => Some("tool_picker"),
        "classifier" => Some("classifier"),
        "summarization" | "summary" | "summariser" | "summarizer" => Some("summarization"),
        "evidence_curator" | "evidence-curator" | "curator" => Some("evidence_curator"),
        "entity_resolver" | "entity-resolver" | "resolver" => Some("entity_resolver"),
        "claim_assessor" | "claim-assessor" | "assessor" => Some("claim_assessor"),
        "investigation_controller" | "investigation-controller" | "controller" => {
            Some("investigation_controller")
        }
        "decision_model" | "decision-model" | "decision" => Some("decision_model"),
        _ => None,
    }
}

impl RoleDefaults {
    pub fn role(&self, role: &str) -> Option<&ModelAssignment> {
        match role_name(role)? {
            "recon" => Some(&self.recon),
            "synthesis" => Some(&self.synthesis),
            "classifier" => Some(&self.classifier),
            "summarization" => Some(&self.summarization),
            "tool_picker" => Some(&self.tool_picker),
            "evidence_curator" => Some(&self.evidence_curator),
            "entity_resolver" => Some(&self.entity_resolver),
            "claim_assessor" => Some(&self.claim_assessor),
            "investigation_controller" => Some(&self.investigation_controller),
            "decision_model" => Some(&self.decision_model),
            _ => None,
        }
    }

    pub fn role_mut(&mut self, role: &str) -> Option<&mut ModelAssignment> {
        match role_name(role)? {
            "recon" => Some(&mut self.recon),
            "synthesis" => Some(&mut self.synthesis),
            "classifier" => Some(&mut self.classifier),
            "summarization" => Some(&mut self.summarization),
            "tool_picker" => Some(&mut self.tool_picker),
            "evidence_curator" => Some(&mut self.evidence_curator),
            "entity_resolver" => Some(&mut self.entity_resolver),
            "claim_assessor" => Some(&mut self.claim_assessor),
            "investigation_controller" => Some(&mut self.investigation_controller),
            "decision_model" => Some(&mut self.decision_model),
            _ => None,
        }
    }

    /// Returns the resolved assignment for a role, along with the role it inherited from (if inherited).
    pub fn resolve_role(&self, role: &str) -> (ModelAssignment, Option<&'static str>) {
        let is_empty =
            |a: &ModelAssignment| a.provider.trim().is_empty() && a.model.trim().is_empty();
        match role_name(role).unwrap_or("recon") {
            "evidence_curator" => {
                if is_empty(&self.evidence_curator) {
                    (self.recon.clone(), Some("recon"))
                } else {
                    (self.evidence_curator.clone(), None)
                }
            }
            "entity_resolver" => {
                if is_empty(&self.entity_resolver) {
                    (self.classifier.clone(), Some("classifier"))
                } else {
                    (self.entity_resolver.clone(), None)
                }
            }
            "claim_assessor" => {
                if is_empty(&self.claim_assessor) {
                    (self.synthesis.clone(), Some("synthesis"))
                } else {
                    (self.claim_assessor.clone(), None)
                }
            }
            "investigation_controller" => {
                if is_empty(&self.investigation_controller) {
                    (self.recon.clone(), Some("recon"))
                } else {
                    (self.investigation_controller.clone(), None)
                }
            }
            "summarization" => {
                if is_empty(&self.summarization) {
                    (self.synthesis.clone(), Some("synthesis"))
                } else {
                    (self.summarization.clone(), None)
                }
            }
            "decision_model" => {
                if is_empty(&self.decision_model) {
                    (
                        ModelAssignment {
                            provider: TOOL_PICKER_PROVIDER.into(),
                            model: TOOL_PICKER_MODEL.into(),
                        },
                        Some("tool_picker"),
                    )
                } else {
                    (self.decision_model.clone(), None)
                }
            }
            "recon" => (self.recon.clone(), None),
            "synthesis" => (self.synthesis.clone(), None),
            "classifier" => (self.classifier.clone(), None),
            "tool_picker" => (self.tool_picker.clone(), None),
            _ => (self.recon.clone(), None),
        }
    }

    /// Seeds the default decision model if unset.
    pub fn seed_decision_model(&mut self) -> bool {
        if self.decision_model.provider.trim().is_empty()
            && self.decision_model.model.trim().is_empty()
        {
            self.decision_model = ModelAssignment {
                provider: TOOL_PICKER_PROVIDER.into(),
                model: TOOL_PICKER_MODEL.into(),
            };
            return true;
        }
        false
    }

    /// When Summarization is unset, copy Synthesis so existing installs inherit.
    pub fn inherit_summarization_from_synthesis(&mut self) -> bool {
        if self.summarization.provider.trim().is_empty()
            && self.summarization.model.trim().is_empty()
            && (!self.synthesis.provider.trim().is_empty()
                || !self.synthesis.model.trim().is_empty())
        {
            self.summarization = self.synthesis.clone();
            return true;
        }
        false
    }

    /// Seeds the tool picker only when both of its fields are empty. Never touches
    /// Recon or Synthesis.
    pub fn seed_tool_picker(&mut self) -> bool {
        if self.tool_picker.provider.trim().is_empty() && self.tool_picker.model.trim().is_empty() {
            self.tool_picker = ModelAssignment {
                provider: TOOL_PICKER_PROVIDER.into(),
                model: TOOL_PICKER_MODEL.into(),
            };
            return true;
        }
        false
    }

    /// Seeds the classifier only when both of its fields are empty.
    pub fn seed_classifier(&mut self) -> bool {
        if self.classifier.provider.trim().is_empty() && self.classifier.model.trim().is_empty() {
            self.classifier = ModelAssignment {
                provider: TOOL_PICKER_PROVIDER.into(),
                model: TOOL_PICKER_MODEL.into(),
            };
            return true;
        }
        false
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReconLimits {
    #[serde(default = "default_max_rounds")]
    pub max_rounds: u8,
    #[serde(default = "default_max_calls")]
    pub max_calls: u8,
    #[serde(default = "default_turn_seconds")]
    pub turn_seconds: u16,
    /// Hard ceiling for the whole turn, in seconds. The computed deadline never exceeds
    /// this, and never drops below `turn_seconds`. Missing values load as 900.
    #[serde(default = "default_max_turn_seconds")]
    pub max_turn_seconds: u16,
    /// Recurring Firecrawl credits available to automatic investigation (local Argos
    /// monthly allowance; independent of the Firecrawl account balance).
    #[serde(default = "default_firecrawl_credits")]
    pub firecrawl_credits: u32,
    /// Recurring Hunter credits available to automatic investigation.
    #[serde(default = "default_hunter_credits")]
    pub hunter_credits: u32,
    /// Recurring SociaVault credits available to automatic investigation.
    #[serde(default = "default_sociavault_credits")]
    pub sociavault_credits: u32,
    /// Non-renewing Firecrawl trial credits. Spent before the recurring allowance.
    #[serde(default)]
    pub firecrawl_trial_credits: u32,
    /// Non-renewing Hunter trial credits. Spent before the recurring allowance.
    #[serde(default)]
    pub hunter_trial_credits: u32,
    /// Non-renewing SociaVault trial credits. Spent before the recurring allowance.
    #[serde(default)]
    pub sociavault_trial_credits: u32,
    /// Maximum Hunter calls on the opening turn. Zero disables opening enrichment.
    #[serde(default = "default_opening_cap")]
    pub opening_hunter_calls: u8,
    /// Retired: SociaVault spending per turn is `sociavault_turn_credits_*`. Kept so older
    /// settings files still load; not read.
    #[serde(default = "default_opening_cap", skip_serializing)]
    pub opening_sociavault_calls: u8,
    /// SociaVault credits one turn may spend on a thread's first turn (spec default D4,
    /// to confirm). Still capped by the remaining SociaVault credits.
    #[serde(default = "default_sociavault_turn_credits_opening")]
    pub sociavault_turn_credits_opening: u32,
    /// SociaVault credits one turn may spend on later turns (spec default D4, to confirm).
    #[serde(default = "default_sociavault_turn_credits_later")]
    pub sociavault_turn_credits_later: u32,
    /// Firecrawl search counts as weak below this many results, which makes SociaVault
    /// Google search a fallback candidate (spec default D3, to confirm).
    #[serde(default = "default_google_fallback_min_results")]
    pub google_fallback_min_results: u32,
    /// NewsAPI calls one turn may make (issue #29; the free plan allows 100 a day).
    #[serde(default = "default_news_calls_per_turn")]
    pub news_calls_per_turn: u32,
    /// CourtListener calls one turn may make (issue #29; free tier 5/min, 125/day).
    #[serde(default = "default_legal_calls_per_turn")]
    pub legal_calls_per_turn: u32,
    /// `monthly` restores the recurring allowance. `never` keeps a fixed pool.
    #[serde(default = "default_credit_reset")]
    pub credit_reset: String,
    #[serde(default = "default_search_cost")]
    pub firecrawl_search_cost: u32,
    #[serde(default = "default_one_cost")]
    pub firecrawl_scrape_cost: u32,
    #[serde(default = "default_one_cost")]
    pub hunter_call_cost: u32,
    #[serde(default = "default_one_cost")]
    pub sociavault_call_cost: u32,
}
fn default_max_rounds() -> u8 {
    6
}
fn default_max_calls() -> u8 {
    12
}
fn default_turn_seconds() -> u16 {
    300
}
/// Hard ceiling for one turn. Existing configs that omit the field load this.
pub const DEFAULT_MAX_TURN_SECONDS: u16 = 900;
pub const MIN_MAX_TURN_SECONDS: u16 = 120;
pub const MAX_MAX_TURN_SECONDS: u16 = 1800;
fn default_max_turn_seconds() -> u16 {
    DEFAULT_MAX_TURN_SECONDS
}
fn default_firecrawl_credits() -> u32 {
    1_000
}
fn default_hunter_credits() -> u32 {
    50
}
fn default_sociavault_credits() -> u32 {
    50
}
fn default_opening_cap() -> u8 {
    1
}
/// Spec default D4 (author's default, to confirm): 3 SociaVault credits on the opening turn.
pub const SOCIAVAULT_TURN_CREDITS_OPENING: u32 = 3;
/// Spec default D4 (author's default, to confirm): 8 SociaVault credits on later turns.
pub const SOCIAVAULT_TURN_CREDITS_LATER: u32 = 8;
/// Spec default D3 (author's default, to confirm): Firecrawl search with fewer results is weak.
pub const GOOGLE_FALLBACK_MIN_RESULTS: u32 = 3;
fn default_sociavault_turn_credits_opening() -> u32 {
    SOCIAVAULT_TURN_CREDITS_OPENING
}
fn default_sociavault_turn_credits_later() -> u32 {
    SOCIAVAULT_TURN_CREDITS_LATER
}
fn default_google_fallback_min_results() -> u32 {
    GOOGLE_FALLBACK_MIN_RESULTS
}
/// Issue #29 default: at most 2 NewsAPI calls per turn.
pub const NEWS_CALLS_PER_TURN: u32 = 2;
/// Issue #29 default: at most 3 CourtListener calls per turn.
pub const LEGAL_CALLS_PER_TURN: u32 = 3;
fn default_news_calls_per_turn() -> u32 {
    NEWS_CALLS_PER_TURN
}
fn default_legal_calls_per_turn() -> u32 {
    LEGAL_CALLS_PER_TURN
}
fn default_credit_reset() -> String {
    "monthly".into()
}
fn default_search_cost() -> u32 {
    2
}
fn default_one_cost() -> u32 {
    1
}
impl Default for ReconLimits {
    fn default() -> Self {
        Self {
            max_rounds: default_max_rounds(),
            max_calls: default_max_calls(),
            turn_seconds: default_turn_seconds(),
            max_turn_seconds: default_max_turn_seconds(),
            firecrawl_credits: default_firecrawl_credits(),
            hunter_credits: default_hunter_credits(),
            sociavault_credits: default_sociavault_credits(),
            firecrawl_trial_credits: 0,
            hunter_trial_credits: 0,
            sociavault_trial_credits: 0,
            opening_hunter_calls: default_opening_cap(),
            opening_sociavault_calls: default_opening_cap(),
            sociavault_turn_credits_opening: SOCIAVAULT_TURN_CREDITS_OPENING,
            sociavault_turn_credits_later: SOCIAVAULT_TURN_CREDITS_LATER,
            google_fallback_min_results: GOOGLE_FALLBACK_MIN_RESULTS,
            news_calls_per_turn: NEWS_CALLS_PER_TURN,
            legal_calls_per_turn: LEGAL_CALLS_PER_TURN,
            credit_reset: default_credit_reset(),
            firecrawl_search_cost: default_search_cost(),
            firecrawl_scrape_cost: default_one_cost(),
            hunter_call_cost: default_one_cost(),
            sociavault_call_cost: default_one_cost(),
        }
    }
}

impl ReconLimits {
    /// `max_turn_seconds` clamped to 120–1800. Zero or a missing field is 900.
    pub fn effective_max_turn_seconds(&self) -> u16 {
        let value = if self.max_turn_seconds == 0 {
            DEFAULT_MAX_TURN_SECONDS
        } else {
            self.max_turn_seconds
        };
        value.clamp(MIN_MAX_TURN_SECONDS, MAX_MAX_TURN_SECONDS)
    }

    pub fn allowance(&self, provider: &str) -> u32 {
        match provider {
            "firecrawl" => self.firecrawl_credits,
            "hunter" => self.hunter_credits,
            "sociavault" => self.sociavault_credits,
            _ => 0,
        }
    }

    pub fn trial_grant(&self, provider: &str) -> u32 {
        match provider {
            "firecrawl" => self.firecrawl_trial_credits,
            "hunter" => self.hunter_trial_credits,
            "sociavault" => self.sociavault_trial_credits,
            _ => 0,
        }
    }

    pub fn configured_cost(&self, tool_id: &str) -> Option<(&'static str, u32)> {
        self.configured_cost_for(tool_id, &serde_json::Value::Null)
    }

    /// Credits to hold for one call: the configured per-call price, times the pages a
    /// batch scrape or crawl asks for. Free Hunter reads stay at zero.
    pub fn configured_cost_for(
        &self,
        tool_id: &str,
        args: &serde_json::Value,
    ) -> Option<(&'static str, u32)> {
        let base = crate::osint::estimated_cost(tool_id, args)?;
        let credits = match crate::osint::canonical_tool_id(tool_id) {
            "firecrawl_search" => self.firecrawl_search_cost,
            "firecrawl_scrape" | "firecrawl_map" => self.firecrawl_scrape_cost,
            "firecrawl_batch_scrape" | "firecrawl_crawl" => {
                base.credits * self.firecrawl_scrape_cost
            }
            "firecrawl_extract" => base.credits,
            _ if base.provider == "hunter" && base.credits == 0 => 0,
            _ if base.provider == "hunter" => self.hunter_call_cost,
            _ if base.provider == "sociavault" => self.sociavault_call_cost,
            _ => base.credits,
        };
        Some((base.provider, credits))
    }

    /// SociaVault credits one turn may spend before remaining credits are considered.
    pub fn sociavault_turn_credits(&self, opening: bool) -> u32 {
        if opening {
            self.sociavault_turn_credits_opening
        } else {
            self.sociavault_turn_credits_later
        }
    }
}

pub fn role_secret(
    auth: &crate::secrets::AuthFile,
    settings: &SettingsFile,
    role: &str,
) -> Result<ProviderSecret> {
    let assignment = settings.defaults.role(role).ok_or_else(|| {
        anyhow!("role must be recon, tool-picker, synthesis, classifier, or summarization")
    })?;
    // Summarization inherits Synthesis when both of its fields are empty.
    let assignment = if role_name(role) == Some("summarization")
        && assignment.provider.trim().is_empty()
        && assignment.model.trim().is_empty()
    {
        settings
            .defaults
            .role("synthesis")
            .ok_or_else(|| anyhow!("synthesis role missing"))?
    } else {
        assignment
    };
    let legacy = writer_secret(auth, settings);
    let mut secret = if assignment.provider.is_empty() {
        legacy
    } else {
        account_secret(auth, &assignment.provider)
    };
    if !assignment.model.is_empty() {
        secret.model = assignment.model.clone();
    }
    Ok(secret)
}

/// Map login input onto a known provider id. Unknown text is returned
/// trimmed so a saved custom kind still round-trips.
pub fn normalize_kind(kind: &str) -> String {
    let compact: String = kind
        .trim()
        .to_lowercase()
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '-' && *ch != '_')
        .collect();
    match compact.as_str() {
        "grok" | "xai" | "x.ai" => "grok".into(),
        "groksubscription" => "grok-subscription".into(),
        "openai" => "openai".into(),
        "chatgpt" | "openaichatgpt" => "openai-chatgpt".into(),
        "openrouter" => "openrouter".into(),
        "google" | "gemini" => "google".into(),
        "nvidia" => "nvidia".into(),
        "local" | "ollama" | "llama" | "llamacpp" | "lmstudio" => "local".into(),
        _ => kind.trim().to_lowercase(),
    }
}

/// Provider id used for headers and the environment key. A stored id wins.
/// Older files that only have a base URL are classified from the host.
pub fn effective_kind(secret: &ProviderSecret) -> String {
    let kind = normalize_kind(&secret.kind);
    if kind == "grok-subscription" {
        return "grok".into();
    }
    if kind == "openai-chatgpt" || preset(&kind).is_some() {
        return kind;
    }
    detect_kind_from_url(&secret.base_url)
}

pub fn detect_kind_from_url(url: &str) -> String {
    let host = url::Url::parse(url.trim())
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
        .unwrap_or_default();
    if host == "openrouter.ai" || host.ends_with(".openrouter.ai") {
        "openrouter".into()
    } else if host == "generativelanguage.googleapis.com" {
        "google".into()
    } else if host == "integrate.api.nvidia.com" {
        "nvidia".into()
    } else if host == "api.x.ai" || host == "x.ai" || host.ends_with(".x.ai") {
        "grok".into()
    } else if host == "api.openai.com" || host.ends_with(".openai.com") {
        "openai".into()
    } else {
        "local".into()
    }
}

/// Stored key first, then the vendor environment variable. The environment
/// value is not written back into `auth.json`.
pub fn resolved_key(secret: &ProviderSecret) -> Option<String> {
    resolved_key_with(secret, |name| std::env::var(name).ok())
}

fn resolved_key_with(
    secret: &ProviderSecret,
    lookup: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    if matches!(
        normalize_kind(&secret.kind).as_str(),
        "grok-subscription" | "openai-chatgpt"
    ) {
        return None;
    }
    if let Some(key) = secret
        .api_key
        .as_ref()
        .map(|key| key.trim())
        .filter(|key| !key.is_empty())
    {
        return Some(key.to_string());
    }
    let kind = effective_kind(secret);
    let name = preset(&kind).and_then(|preset| preset.env_key)?;
    lookup(name)
        .or_else(|| {
            (kind == "google")
                .then(|| lookup("GOOGLE_API_KEY"))
                .flatten()
        })
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn provider_headers(secret: &ProviderSecret) -> Vec<(&'static str, &'static str)> {
    if effective_kind(secret) == "openrouter" {
        vec![
            ("HTTP-Referer", "https://github.com/argos-osint"),
            ("X-Title", "Argos OSINT"),
            ("X-OpenRouter-Title", "Argos OSINT"),
        ]
    } else {
        Vec::new()
    }
}

async fn bearer_token(secret: &ProviderSecret) -> Result<Option<String>, String> {
    if normalize_kind(&secret.kind) == "grok-subscription" {
        return crate::grok_oauth::bearer().await;
    }
    if let Some(key) = resolved_key(secret) {
        return Ok(Some(key));
    }
    if effective_kind(secret) == "grok" {
        return crate::grok_oauth::bearer().await;
    }
    Ok(None)
}

pub(crate) async fn authorize(
    mut req: reqwest::RequestBuilder,
    secret: &ProviderSecret,
) -> Result<reqwest::RequestBuilder> {
    match bearer_token(secret).await {
        Ok(Some(key)) => req = req.bearer_auth(key),
        Ok(None) if effective_kind(secret) == "grok" => {
            return Err(anyhow!(
                "No Grok subscription login. Sign in at Providers → Grok, or run `grok login --oauth`."
            ));
        }
        Ok(None) if preset(&effective_kind(secret)).is_some_and(|p| p.key_required) => {
            return Err(anyhow!("{} credentials are not configured. Connect this account in Providers before selecting its models.", effective_kind(secret)));
        }
        Ok(None) => {}
        Err(err) => return Err(anyhow!(err)),
    }
    for (name, value) in provider_headers(secret) {
        req = req.header(name, value);
    }
    Ok(req)
}

pub async fn complete(
    secret: &ProviderSecret,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
    mut on_delta: impl FnMut(&str),
) -> Result<Completion> {
    if effective_kind(secret) == "openai-chatgpt" {
        return crate::subscription::complete(secret, messages, tools, on_delta).await;
    }
    let client = http()?;
    let url = format!("{}/chat/completions", normalize_base(&secret.base_url));
    let body = chat_body(secret, messages, tools, true);
    let req = authorize(client.post(&url).json(&body), secret).await?;
    let resp = req.send().await.with_context(|| format!("POST {url}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let headers = resp.headers().clone();
        let text = resp.text().await.unwrap_or_default();
        // Some local servers reject stream+tools. Retry once without streaming.
        if status.as_u16() == 400 || status.as_u16() == 404 {
            return complete_once(secret, messages, tools).await;
        }
        return Err(typed_http_error(
            secret,
            &url,
            status.as_u16(),
            &headers,
            &text,
            "stream",
        ));
    }
    let mut stream = resp.bytes_stream();
    let mut acc = SseAcc::default();
    let mut buf = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(err) => {
                // Mid-stream drops are common on large synthesis replies; finish without streaming.
                if acc.is_empty() {
                    return complete_once(secret, messages, tools).await;
                }
                return Err(err).context("read provider stream");
            }
        };
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(idx) = buf.find('\n') {
            let line = buf[..idx].trim().to_string();
            buf = buf[idx + 1..].to_string();
            if let Some(delta) = acc.push_line(&line) {
                on_delta(&delta);
            }
        }
    }
    let completion = acc.into_completion();
    if completion.is_empty() {
        // Stream produced no text and no tools (or only metadata). Retry once
        // without streaming; parse_completion also folds reasoning_content.
        return complete_once(secret, messages, tools).await;
    }
    Ok(completion)
}

/// One Decisions answer. `choice` for choice questions, `score` for score questions,
/// `noul` for yes/no questions. `probabilities` maps each option or level to its weight.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct DecisionAnswer {
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub choice: Option<String>,
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub noul: Option<f64>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub probabilities: std::collections::BTreeMap<String, f64>,
}

impl DecisionAnswer {
    /// Probability of the chosen option, else the reported confidence.
    pub fn choice_probability(&self) -> Option<f64> {
        self.choice
            .as_ref()
            .and_then(|choice| self.probabilities.get(choice).copied())
            .or(self.confidence)
    }

    /// Whether the probability distribution is mathematically valid within numeric tolerance.
    pub fn is_valid_distribution(&self, tolerance: f64) -> bool {
        if self.probabilities.len() <= 1 {
            return true;
        }
        let mut sum = 0.0;
        for prob in self.probabilities.values() {
            if !prob.is_finite() || *prob < 0.0 {
                return false;
            }
            sum += *prob;
        }
        (sum - 1.0).abs() <= tolerance
    }

    /// Whether an ordinal score is within range.
    pub fn is_valid_score(&self, min: i64, max: i64) -> bool {
        self.score
            .is_some_and(|s| s.is_finite() && s.round() as i64 >= min && s.round() as i64 <= max)
    }

    /// Whether a noul (yes/no) output is valid.
    pub fn is_valid_noul(&self) -> bool {
        self.noul
            .is_some_and(|n| n.is_finite() && (0.0..=1.0).contains(&n))
            || self.choice.is_some()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct DecisionsResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub answers: std::collections::BTreeMap<String, DecisionAnswer>,
    /// `usage.cost` in USD, when reported.
    #[serde(default)]
    pub cost: Option<f64>,
}

/// `https://openrouter.ai/api/v1` becomes `https://openrouter.ai/api/alpha/decisions`.
pub fn decisions_url(base_url: &str) -> String {
    let base = normalize_base(base_url);
    let root = base.strip_suffix("/v1").unwrap_or(&base);
    format!("{}/alpha/decisions", root.trim_end_matches('/'))
}

pub fn parse_decisions(text: &str) -> Result<DecisionsResponse> {
    let value: Value = serde_json::from_str(text).context("decisions JSON")?;
    let mut response: DecisionsResponse =
        serde_json::from_value(value.clone()).context("decisions response")?;
    response.cost = value.pointer("/usage/cost").and_then(Value::as_f64);
    for (qid, ans) in &response.answers {
        if !ans.is_valid_distribution(0.05) {
            return Err(anyhow!(
                "question '{qid}' has invalid probability distribution"
            ));
        }
    }
    Ok(response)
}

/// Jev on OpenRouter: POST `{model, state, questions}` to the Decisions API. The caller
/// races this against the turn cancel flag; the request has its own timeout.
pub async fn decide(
    secret: &ProviderSecret,
    state: &Value,
    questions: &Value,
) -> Result<DecisionsResponse> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(45))
        .build()
        .context("http client")?;
    let url = decisions_url(&secret.base_url);
    let body = json!({"model": secret.model, "state": state, "questions": questions});
    let req = authorize(client.post(&url).json(&body), secret).await?;
    let resp = req.send().await.with_context(|| format!("POST {url}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("provider {status}: {text}"));
    }
    parse_decisions(&text)
}

async fn complete_once(
    secret: &ProviderSecret,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
) -> Result<Completion> {
    let client = http()?;
    let url = format!("{}/chat/completions", normalize_base(&secret.base_url));
    let body = chat_body(secret, messages, tools, false);
    let req = authorize(client.post(&url).json(&body), secret).await?;
    let resp = req.send().await.with_context(|| format!("POST {url}"))?;
    let status = resp.status();
    let headers = resp.headers().clone();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(typed_http_error(
            secret,
            &url,
            status.as_u16(),
            &headers,
            &text,
            "non_stream",
        ));
    }
    parse_completion(&text)
}

/// Non-success HTTP as a typed, redacted [`crate::provider_diag::ProviderFailure`]
/// (its Display keeps the status and the provider's message).
fn typed_http_error(
    secret: &ProviderSecret,
    url: &str,
    status: u16,
    headers: &reqwest::header::HeaderMap,
    body: &str,
    transport: &str,
) -> anyhow::Error {
    anyhow::Error::new(crate::provider_diag::http_failure(
        status,
        headers,
        body,
        url,
        &effective_kind(secret),
        &secret.model,
        transport,
    ))
}

pub(crate) fn chat_body(
    secret: &ProviderSecret,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
    stream: bool,
) -> Value {
    let msgs: Vec<Value> = messages.iter().map(message_json).collect();
    let mut body = json!({
        "model": secret.model,
        "messages": msgs,
        "stream": stream,
        "temperature": 0.2,
    });
    if !tools.is_empty() {
        body["tools"] = json!(tools.iter().map(tool_json).collect::<Vec<_>>());
        body["tool_choice"] = json!("auto");
    }
    body
}

fn message_json(m: &ChatMessage) -> Value {
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

/// Pulls assistant text from a chat-completions message object.
/// Returns ONLY content; reasoning is kept separate in [`message_reasoning`].
pub fn message_text(msg: &Value) -> String {
    value_text(msg.get("content"))
}

/// Pulls assistant reasoning from a chat-completions message object.
pub fn message_reasoning(msg: &Value) -> String {
    for key in ["reasoning_content", "reasoning"] {
        let text = value_text(msg.get(key));
        if !text.trim().is_empty() {
            return text;
        }
    }
    String::new()
}

fn value_text(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    if let Some(text) = value.as_str() {
        return text.to_string();
    }
    let Some(parts) = value.as_array() else {
        return String::new();
    };
    let mut out = String::new();
    for part in parts {
        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
            out.push_str(text);
            continue;
        }
        if part
            .get("type")
            .and_then(|t| t.as_str())
            .is_some_and(|t| t == "text")
        {
            if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                out.push_str(text);
            }
        }
    }
    out
}

fn delta_text(delta: &Value, key: &str) -> Option<String> {
    let value = delta.get(key)?;
    let text = value_text(Some(value));
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

pub fn parse_completion(text: &str) -> Result<Completion> {
    let v: Value = serde_json::from_str(text).context("provider JSON")?;
    let msg = v
        .pointer("/choices/0/message")
        .cloned()
        .unwrap_or(Value::Null);
    let content = message_text(&msg);
    let reasoning = message_reasoning(&msg);
    let refusal = msg
        .get("refusal")
        .and_then(|c| c.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let finish_reason = v
        .pointer("/choices/0/finish_reason")
        .and_then(|c| c.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let mut tool_calls = Vec::new();
    if let Some(calls) = msg.get("tool_calls").and_then(|c| c.as_array()) {
        for call in calls {
            tool_calls.push(ToolCall {
                id: call
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("call")
                    .to_string(),
                name: call
                    .pointer("/function/name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                arguments: call
                    .pointer("/function/arguments")
                    .and_then(|v| v.as_str())
                    .unwrap_or("{}")
                    .to_string(),
            });
        }
    }
    let completion = Completion {
        content,
        reasoning,
        tool_calls,
        finish_reason,
        refusal,
    };
    if completion.is_empty() {
        return Err(completion.empty_error("provider"));
    }
    Ok(completion)
}

#[derive(Default)]
struct SseAcc {
    content: String,
    reasoning: String,
    tool_calls: Vec<ToolCall>,
    finish_reason: Option<String>,
    refusal: Option<String>,
}

impl SseAcc {
    fn is_empty(&self) -> bool {
        self.content.trim().is_empty()
            && self.reasoning.trim().is_empty()
            && self.tool_calls.is_empty()
    }

    fn into_completion(self) -> Completion {
        Completion {
            content: self.content,
            reasoning: self.reasoning,
            tool_calls: self.tool_calls,
            finish_reason: self.finish_reason,
            refusal: self.refusal,
        }
    }

    fn push_line(&mut self, line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() || line.starts_with(':') {
            return None;
        }
        let data = line.strip_prefix("data:").unwrap_or(line).trim();
        if data == "[DONE]" || data.is_empty() {
            return None;
        }
        let v: Value = serde_json::from_str(data).ok()?;
        if let Some(reason) = v
            .pointer("/choices/0/finish_reason")
            .and_then(|c| c.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            self.finish_reason = Some(reason.to_string());
        }
        let delta = v.pointer("/choices/0/delta")?;
        if let Some(refusal) = delta
            .get("refusal")
            .and_then(|c| c.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            self.refusal = Some(refusal.to_string());
        }
        let mut emitted = None;
        if let Some(text) = delta_text(delta, "content") {
            self.content.push_str(&text);
            emitted = Some(text);
        }
        // Reasoning channel: accumulate separately, do not leak into answer deltas
        for key in ["reasoning_content", "reasoning"] {
            if let Some(text) = delta_text(delta, key) {
                self.reasoning.push_str(&text);
            }
        }
        if let Some(calls) = delta.get("tool_calls").and_then(|c| c.as_array()) {
            for call in calls {
                let idx = call.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                while self.tool_calls.len() <= idx {
                    self.tool_calls.push(ToolCall {
                        id: String::new(),
                        name: String::new(),
                        arguments: String::new(),
                    });
                }
                if let Some(id) = call.get("id").and_then(|v| v.as_str()) {
                    self.tool_calls[idx].id = id.to_string();
                }
                if let Some(name) = call.pointer("/function/name").and_then(|v| v.as_str()) {
                    self.tool_calls[idx].name.push_str(name);
                }
                if let Some(args) = call.pointer("/function/arguments").and_then(|v| v.as_str()) {
                    self.tool_calls[idx].arguments.push_str(args);
                }
            }
        }
        emitted
    }
}

pub async fn start_device(endpoints: &DeviceEndpoints) -> Result<DeviceGrant> {
    let client = http()?;
    let resp = client
        .post(&endpoints.device_auth_url)
        .header("Accept", "application/json")
        .form(&[
            ("client_id", endpoints.client_id.as_str()),
            ("scope", endpoints.scope.as_str()),
        ])
        .send()
        .await
        .with_context(|| format!("POST {}", endpoints.device_auth_url))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("device authorization {status}: {text}"));
    }
    serde_json::from_str(&text).context("device authorization JSON")
}

pub async fn poll_device(endpoints: &DeviceEndpoints, device_code: &str) -> Result<Poll> {
    let client = http()?;
    let resp = client
        .post(&endpoints.token_url)
        .header("Accept", "application/json")
        .form(&[
            ("client_id", endpoints.client_id.as_str()),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])
        .send()
        .await
        .with_context(|| format!("POST {}", endpoints.token_url))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if let Some(token) = v.get("access_token").and_then(|t| t.as_str()) {
        return Ok(Poll::Token(token.to_string()));
    }
    let err = v.get("error").and_then(|e| e.as_str()).unwrap_or("");
    match err {
        "authorization_pending" => Ok(Poll::Pending),
        "slow_down" => Ok(Poll::SlowDown),
        "access_denied" | "expired_token" => Ok(Poll::Denied(err.into())),
        _ if !status.is_success() => Err(anyhow!("token endpoint {status}: {text}")),
        _ => Ok(Poll::Denied(if err.is_empty() { text } else { err.into() })),
    }
}

pub async fn transcribe(secret: &ProviderSecret, wav: &[u8]) -> Result<String> {
    let client = http()?;
    let model = secret
        .stt_model
        .clone()
        .unwrap_or_else(|| "whisper-1".into());
    let part = reqwest::multipart::Part::bytes(wav.to_vec())
        .file_name("speech.wav")
        .mime_str("audio/wav")
        .context("wav part")?;
    let form = reqwest::multipart::Form::new()
        .text("model", model)
        .part("file", part);
    let url = format!("{}/audio/transcriptions", normalize_base(&secret.base_url));
    let req = authorize(client.post(&url).multipart(form), secret).await?;
    let resp = req.send().await.with_context(|| format!("POST {url}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("transcription {status}: {text}"));
    }
    let v: Value = serde_json::from_str(&text).unwrap_or(json!({ "text": text }));
    Ok(v.get("text")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .trim()
        .to_string())
}

/// One model from `GET /models`. `free` is a concrete zero-cost model, not
/// the `openrouter/free` router that picks one of those at random.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedModel {
    pub id: String,
    pub name: String,
    pub free: bool,
}

/// The OpenRouter route that chooses a free model at random.
pub fn is_free_router(id: &str) -> bool {
    id.trim().eq_ignore_ascii_case("openrouter/free")
}

/// Free models a person can pin. The router id itself is not one of them.
pub fn concrete_free_models(models: &[ListedModel]) -> Vec<ListedModel> {
    let mut free: Vec<ListedModel> = models
        .iter()
        .filter(|model| model.free && !is_free_router(&model.id))
        .filter(|model| !model.id.to_ascii_lowercase().starts_with("openrouter/"))
        .cloned()
        .collect();
    free.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    free
}

pub fn parse_model_catalog(value: &Value) -> Vec<ListedModel> {
    let Some(data) = value.get("data").and_then(|d| d.as_array()) else {
        return Vec::new();
    };
    let mut models = Vec::new();
    for item in data {
        let Some(id) = item.get("id").and_then(|v| v.as_str()).map(str::trim) else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(id);
        models.push(ListedModel {
            id: id.to_string(),
            name: name.to_string(),
            free: is_concrete_free(id, item),
        });
    }
    models.sort_by(|a, b| a.id.to_lowercase().cmp(&b.id.to_lowercase()));
    models
}

fn is_concrete_free(id: &str, item: &Value) -> bool {
    if is_free_router(id) || id.to_ascii_lowercase().starts_with("openrouter/") {
        return false;
    }
    if id.to_ascii_lowercase().ends_with(":free") {
        return true;
    }
    let prompt = item.pointer("/pricing/prompt").and_then(json_number);
    let completion = item.pointer("/pricing/completion").and_then(json_number);
    matches!((prompt, completion), (Some(p), Some(c)) if p == 0.0 && c == 0.0)
}

fn json_number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
}

pub async fn list_catalog(secret: &ProviderSecret) -> Result<Vec<ListedModel>> {
    if effective_kind(secret) == "openai-chatgpt" {
        return Err(anyhow!(
            "ChatGPT models are chosen by Codex; enter a model ID or use its default."
        ));
    }
    let client = http()?;
    let url = format!("{}/models", normalize_base(&secret.base_url));
    let req = authorize(client.get(&url), secret).await?;
    let resp = req.send().await.with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(catalog_error(secret, status, &text));
    }
    let v: Value = serde_json::from_str(&text).context("malformed model catalog JSON")?;
    anyhow::ensure!(
        v.get("data").is_some_and(Value::is_array),
        "Model catalog response is missing a data array"
    );
    Ok(parse_model_catalog(&v))
}

fn catalog_error(
    secret: &ProviderSecret,
    status: reqwest::StatusCode,
    body: &str,
) -> anyhow::Error {
    if normalize_kind(&secret.kind) == "grok-subscription" {
        let payload: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        let code = payload
            .get("code")
            .or_else(|| payload.pointer("/error/code"))
            .and_then(Value::as_str);
        if status == reqwest::StatusCode::FORBIDDEN
            && code == Some("personal-team-blocked:spending-limit")
        {
            return anyhow!("Signed in · Grok model access blocked (spending limit). Check this account's subscription/usage at grok.com, then Check existing login.");
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return anyhow!("Grok rejected the saved login (401). Sign in with Grok again; saved API keys are not used.");
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            return anyhow!("Grok denied model access (403). Check the signed-in account's subscription and permissions, then Check existing login.");
        }
    }
    anyhow!("models {status}: {body}")
}

/// OpenRouter's catalog is public. Verify the key against its authenticated
/// endpoint before presenting an account as connected in the setup UI.
pub async fn verified_catalog(secret: &ProviderSecret) -> Result<Vec<ListedModel>> {
    if effective_kind(secret) == "openrouter" {
        let client = http()?;
        let url = format!("{}/key", normalize_base(&secret.base_url));
        let resp = authorize(client.get(&url), secret)
            .await?
            .send()
            .await
            .context("check OpenRouter key")?;
        let status = resp.status();
        if !status.is_success() {
            return Err(anyhow!("OpenRouter rejected this key ({status}). Check the OpenRouter account key in Providers; your other accounts are unchanged."));
        }
        let info: Value = resp
            .json()
            .await
            .context("read OpenRouter key verification")?;
        if !info.get("data").is_some_and(Value::is_object) {
            return Err(anyhow!(
                "OpenRouter did not return key details; check the API endpoint"
            ));
        }
        if info
            .pointer("/data/is_management_key")
            .and_then(Value::as_bool)
            == Some(true)
        {
            return Err(anyhow!(
                "This OpenRouter management key cannot run models. Use an inference API key."
            ));
        }
    }
    list_catalog(secret).await
}

pub async fn list_models(secret: &ProviderSecret) -> Result<Vec<String>> {
    Ok(list_catalog(secret)
        .await?
        .into_iter()
        .map(|model| model.id)
        .collect())
}

fn http() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .context("http client")
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SettingsFile {
    /// User preference; `None` preserves the default visible Recon context pane.
    #[serde(default)]
    pub tui_recon_context: Option<bool>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub writer_model: String,
    #[serde(default)]
    pub writer_provider: String,
    #[serde(default)]
    pub modality: String,
    #[serde(default)]
    pub defaults: RoleDefaults,
    #[serde(default)]
    pub osint_user_agent: String,
    /// Firecrawl API key. A non-empty value overrides `FIRECRAWL_API_KEY`.
    #[serde(default)]
    pub firecrawl_api_key: String,
    /// Second Firecrawl account. Used after the primary key hits a rate or quota limit.
    #[serde(default)]
    pub firecrawl_api_key_fallback: String,
    /// Hunter API key. A non-empty value overrides `HUNTER_API_KEY`.
    #[serde(default)]
    pub hunter_api_key: String,
    /// Second Hunter account. Used after the primary key hits a rate or quota limit.
    #[serde(default)]
    pub hunter_api_key_fallback: String,
    /// SociaVault API key. A non-empty value overrides `SOCIAVAULT_API_KEY`.
    #[serde(default)]
    pub sociavault_api_key: String,
    /// Second SociaVault account. Used after the primary key hits a rate or quota limit.
    #[serde(default)]
    pub sociavault_api_key_fallback: String,
    /// NewsAPI key. A non-empty value overrides `NEWSAPI_API_KEY`.
    #[serde(default)]
    pub newsapi_api_key: String,
    /// Second NewsAPI account. Used after the primary key hits a rate or quota limit.
    #[serde(default)]
    pub newsapi_api_key_fallback: String,
    /// CourtListener API token. A non-empty value overrides `COURTLISTENER_API_TOKEN`.
    #[serde(default)]
    pub courtlistener_api_token: String,
    /// Second CourtListener account. Used after the primary token hits a rate or quota limit.
    #[serde(default)]
    pub courtlistener_api_token_fallback: String,
    /// GNews API key. A non-empty value overrides `GNEWS_API_KEY`.
    #[serde(default)]
    pub gnews_api_key: String,
    /// Second GNews account. Used after the primary key hits a rate or quota limit.
    #[serde(default)]
    pub gnews_api_key_fallback: String,
    /// NewsData.io API key. A non-empty value overrides `NEWSDATA_API_KEY`.
    #[serde(default)]
    pub newsdata_api_key: String,
    /// Second NewsData account. Used after the primary key hits a rate or quota limit.
    #[serde(default)]
    pub newsdata_api_key_fallback: String,
    /// Currents API key. A non-empty value overrides `CURRENTS_API_KEY`.
    #[serde(default)]
    pub currents_api_key: String,
    /// Second Currents account. Used after the primary key hits a rate or quota limit.
    #[serde(default)]
    pub currents_api_key_fallback: String,
    #[serde(default)]
    pub recon_limits: ReconLimits,
}

/// Environment fallbacks for each keyed provider's setting.
pub const KEY_ENV: &[(&str, &str)] = &[
    ("firecrawl", "FIRECRAWL_API_KEY"),
    ("hunter", "HUNTER_API_KEY"),
    ("sociavault", "SOCIAVAULT_API_KEY"),
    ("newsapi", "NEWSAPI_API_KEY"),
    ("courtlistener", "COURTLISTENER_API_TOKEN"),
    ("gnews", "GNEWS_API_KEY"),
    ("newsdata", "NEWSDATA_API_KEY"),
    ("currents", "CURRENTS_API_KEY"),
];

/// Environment fallbacks for each keyed provider's second account.
pub const KEY_FALLBACK_ENV: &[(&str, &str)] = &[
    ("firecrawl", "FIRECRAWL_API_KEY_FALLBACK"),
    ("hunter", "HUNTER_API_KEY_FALLBACK"),
    ("sociavault", "SOCIAVAULT_API_KEY_FALLBACK"),
    ("newsapi", "NEWSAPI_API_KEY_FALLBACK"),
    ("courtlistener", "COURTLISTENER_API_TOKEN_FALLBACK"),
    ("gnews", "GNEWS_API_KEY_FALLBACK"),
    ("newsdata", "NEWSDATA_API_KEY_FALLBACK"),
    ("currents", "CURRENTS_API_KEY_FALLBACK"),
];

impl SettingsFile {
    /// The saved key for a provider, if any (never the environment).
    pub fn saved_key(&self, provider: &str) -> &str {
        match provider {
            "firecrawl" => &self.firecrawl_api_key,
            "hunter" => &self.hunter_api_key,
            "sociavault" => &self.sociavault_api_key,
            "newsapi" => &self.newsapi_api_key,
            "courtlistener" => &self.courtlistener_api_token,
            "gnews" => &self.gnews_api_key,
            "newsdata" => &self.newsdata_api_key,
            "currents" => &self.currents_api_key,
            _ => "",
        }
    }

    /// The saved second key for a provider, if any (never the environment).
    pub fn saved_fallback_key(&self, provider: &str) -> &str {
        match provider {
            "firecrawl" => &self.firecrawl_api_key_fallback,
            "hunter" => &self.hunter_api_key_fallback,
            "sociavault" => &self.sociavault_api_key_fallback,
            "newsapi" => &self.newsapi_api_key_fallback,
            "courtlistener" => &self.courtlistener_api_token_fallback,
            "gnews" => &self.gnews_api_key_fallback,
            "newsdata" => &self.newsdata_api_key_fallback,
            "currents" => &self.currents_api_key_fallback,
            _ => "",
        }
    }

    /// The key a provider's tools use: a non-empty setting, else its environment variable.
    pub fn provider_key(&self, provider: &str) -> String {
        self.provider_key_with(provider, |name| std::env::var(name).ok())
    }

    /// The second account for a provider: a non-empty setting, else its fallback environment variable.
    pub fn provider_fallback_key(&self, provider: &str) -> String {
        self.provider_fallback_key_with(provider, |name| std::env::var(name).ok())
    }

    /// [`Self::provider_key`] with an injected environment (tests).
    pub fn provider_key_with(
        &self,
        provider: &str,
        env: impl Fn(&str) -> Option<String>,
    ) -> String {
        let saved = self.saved_key(provider).trim();
        if !saved.is_empty() {
            return saved.to_string();
        }
        KEY_ENV
            .iter()
            .find(|(known, _)| *known == provider)
            .and_then(|(_, name)| env(name))
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    /// [`Self::provider_fallback_key`] with an injected environment (tests).
    pub fn provider_fallback_key_with(
        &self,
        provider: &str,
        env: impl Fn(&str) -> Option<String>,
    ) -> String {
        let saved = self.saved_fallback_key(provider).trim();
        if !saved.is_empty() {
            return saved.to_string();
        }
        KEY_FALLBACK_ENV
            .iter()
            .find(|(known, _)| *known == provider)
            .and_then(|(_, name)| env(name))
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    pub fn load() -> Result<Self> {
        let path = crate::paths::config_path();
        Self::load_from(&path)
    }

    fn load_from(path: &std::path::Path) -> Result<Self> {
        let seeded = || {
            let mut settings = Self::default();
            settings.defaults.seed_tool_picker();
            settings.defaults.seed_classifier();
            settings.defaults.seed_decision_model();
            settings.defaults.inherit_summarization_from_synthesis();
            settings
        };
        if !path.exists() {
            return Ok(seeded());
        }
        let raw = std::fs::read_to_string(path)?;
        if raw.trim().is_empty() {
            return Ok(seeded());
        }
        let mut settings: Self = toml::from_str(&raw)?;
        // A blank OSINT User-Agent is unset, so requests keep the built-in default.
        settings.osint_user_agent = settings.osint_user_agent.trim().to_string();
        let legacy_provider = settings.writer_provider.clone();
        let legacy_model = if settings.writer_model.is_empty() {
            settings.model.clone()
        } else {
            settings.writer_model.clone()
        };
        let mut migrated = false;
        for role in [
            &mut settings.defaults.recon,
            &mut settings.defaults.synthesis,
        ] {
            if role.provider.is_empty() && !legacy_provider.is_empty() {
                role.provider = legacy_provider.clone();
                migrated = true;
            }
            if role.model.is_empty() && !legacy_model.is_empty() {
                role.model = legacy_model.clone();
                migrated = true;
            }
        }
        // The tool picker is seeded on its own and never from the legacy writer.
        if settings.defaults.seed_tool_picker() {
            migrated = true;
        }
        let mutated = settings.defaults.seed_classifier()
            | settings.defaults.seed_decision_model()
            | settings.defaults.inherit_summarization_from_synthesis();
        if mutated {
            migrated = true;
        }
        // Bump the old product default (200) to the current monthly Firecrawl allowance.
        if settings.recon_limits.firecrawl_credits == 200 {
            settings.recon_limits.firecrawl_credits = default_firecrawl_credits();
            migrated = true;
        }
        let old_keys = toml::from_str::<toml::Value>(&raw)?
            .as_table()
            .is_some_and(|table| {
                table.keys().any(|key| {
                    !matches!(
                        key.as_str(),
                        "model"
                            | "writer_model"
                            | "writer_provider"
                            | "modality"
                            | "defaults"
                            | "osint_user_agent"
                            | "firecrawl_api_key"
                            | "hunter_api_key"
                            | "sociavault_api_key"
                            | "newsapi_api_key"
                            | "courtlistener_api_token"
                            | "gnews_api_key"
                            | "newsdata_api_key"
                            | "currents_api_key"
                            | "firecrawl_api_key_fallback"
                            | "hunter_api_key_fallback"
                            | "sociavault_api_key_fallback"
                            | "newsapi_api_key_fallback"
                            | "courtlistener_api_token_fallback"
                            | "gnews_api_key_fallback"
                            | "newsdata_api_key_fallback"
                            | "currents_api_key_fallback"
                            | "recon_limits"
                    )
                })
            });
        if old_keys || migrated {
            settings.save_to(path)?;
        }
        Ok(settings)
    }

    pub fn save(&self) -> Result<()> {
        crate::paths::ensure_home()?;
        self.save_to(&crate::paths::config_path())
    }

    pub fn save_to(&self, path: &std::path::Path) -> Result<()> {
        crate::secrets::write_private(path, &toml::to_string_pretty(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC2: NewsAPI and CourtListener keys come from settings, else the environment; a
    /// saved key overrides the variable, and both fields survive a settings round trip.
    #[test]
    fn news_and_legal_keys_come_from_settings_then_env() {
        let env = |name: &str| match name {
            "NEWSAPI_API_KEY" => Some(" env-news ".to_string()),
            "COURTLISTENER_API_TOKEN" => Some("env-court".to_string()),
            _ => None,
        };
        let mut settings = SettingsFile::default();
        assert_eq!(settings.provider_key_with("newsapi", env), "env-news");
        assert_eq!(
            settings.provider_key_with("courtlistener", env),
            "env-court"
        );
        assert_eq!(settings.provider_key_with("newsapi", |_| None), "");
        settings.newsapi_api_key = "saved-news".into();
        settings.courtlistener_api_token = "saved-court".into();
        assert_eq!(settings.provider_key_with("newsapi", env), "saved-news");
        assert_eq!(
            settings.provider_key_with("courtlistener", env),
            "saved-court"
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        settings.save_to(&path).unwrap();
        let loaded = SettingsFile::load_from(&path).unwrap();
        assert_eq!(
            (
                loaded.newsapi_api_key.as_str(),
                loaded.courtlistener_api_token.as_str()
            ),
            ("saved-news", "saved-court")
        );
        // Both names are current keys: loading does not rewrite the file as a migration.
        let before = std::fs::read_to_string(&path).unwrap();
        SettingsFile::load_from(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        assert_eq!(
            (
                loaded.recon_limits.news_calls_per_turn,
                loaded.recon_limits.legal_calls_per_turn
            ),
            (2, 3)
        );
    }

    /// A second account is a saved fallback key, else its own environment variable.
    /// The primary key is unchanged.
    #[test]
    fn fallback_keys_come_from_settings_then_env() {
        let env = |name: &str| match name {
            "NEWSAPI_API_KEY" => Some("env-news".to_string()),
            "NEWSAPI_API_KEY_FALLBACK" => Some(" env-spare ".to_string()),
            "COURTLISTENER_API_TOKEN_FALLBACK" => Some("env-court-spare".to_string()),
            _ => None,
        };
        let mut settings = SettingsFile::default();
        assert_eq!(settings.provider_key_with("newsapi", env), "env-news");
        assert_eq!(
            settings.provider_fallback_key_with("newsapi", env),
            "env-spare"
        );
        assert_eq!(
            settings.provider_fallback_key_with("courtlistener", env),
            "env-court-spare"
        );
        assert_eq!(settings.provider_fallback_key_with("gnews", |_| None), "");
        settings.newsapi_api_key_fallback = "saved-spare".into();
        settings.courtlistener_api_token_fallback = "saved-court-spare".into();
        assert_eq!(
            settings.provider_fallback_key_with("newsapi", env),
            "saved-spare"
        );
        assert_eq!(settings.provider_key_with("newsapi", env), "env-news");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        settings.save_to(&path).unwrap();
        let loaded = SettingsFile::load_from(&path).unwrap();
        assert_eq!(
            (
                loaded.newsapi_api_key_fallback.as_str(),
                loaded.courtlistener_api_token_fallback.as_str()
            ),
            ("saved-spare", "saved-court-spare")
        );
        let before = std::fs::read_to_string(&path).unwrap();
        SettingsFile::load_from(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn old_firecrawl_credit_default_migrates_to_1000() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[recon_limits]\nfirecrawl_credits = 200\n").unwrap();
        let loaded = SettingsFile::load_from(&path).unwrap();
        assert_eq!(loaded.recon_limits.firecrawl_credits, 1_000);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(
            saved.contains("firecrawl_credits = 1000"),
            "migration should persist: {saved}"
        );
        assert_eq!(ReconLimits::default().firecrawl_credits, 1_000);
    }

    #[test]
    fn missing_max_turn_seconds_loads_as_900_and_the_range_is_120_to_1800() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[recon_limits]\nturn_seconds = 300\n").unwrap();
        let loaded = SettingsFile::load_from(&path).unwrap();
        assert_eq!(
            loaded.recon_limits.max_turn_seconds,
            DEFAULT_MAX_TURN_SECONDS
        );
        assert_eq!(loaded.recon_limits.effective_max_turn_seconds(), 900);
        let low = ReconLimits {
            max_turn_seconds: 50,
            ..ReconLimits::default()
        };
        assert_eq!(low.effective_max_turn_seconds(), MIN_MAX_TURN_SECONDS);
        let high = ReconLimits {
            max_turn_seconds: 5_000,
            ..ReconLimits::default()
        };
        assert_eq!(high.effective_max_turn_seconds(), MAX_MAX_TURN_SECONDS);
        let mid = ReconLimits {
            max_turn_seconds: 1_200,
            ..ReconLimits::default()
        };
        assert_eq!(mid.effective_max_turn_seconds(), 1_200);
    }

    #[test]
    fn obsolete_research_configuration_is_ignored() {
        let settings: SettingsFile =
            toml::from_str("writer_model = 'grok-4.6'\nsearx_url = 'old'\nreport_dir = 'old'\n")
                .unwrap();
        let saved = toml::to_string(&settings).unwrap();
        assert!(saved.contains("grok-4.6"));
        assert!(!saved.contains("searx_url"));
        assert!(!saved.contains("report_dir"));
    }

    #[test]
    fn role_defaults_migrate_once_and_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path,"writer_provider = 'openrouter'\nwriter_model = 'old-model'\nosint_user_agent = 'Argos test@example.com'\n[defaults.recon]\nprovider = 'local'\nmodel = 'local-model'\n").unwrap();
        let settings = SettingsFile::load_from(&path).unwrap();
        assert_eq!(settings.defaults.recon.model, "local-model");
        assert_eq!(settings.defaults.synthesis.model, "old-model");
        assert_eq!(settings.osint_user_agent, "Argos test@example.com");
        settings.save_to(&path).unwrap();
        let reopened = SettingsFile::load_from(&path).unwrap();
        assert_eq!(reopened.defaults.recon, settings.defaults.recon);
        assert_eq!(reopened.defaults.synthesis, settings.defaults.synthesis);
        assert_eq!(reopened.osint_user_agent, settings.osint_user_agent);
    }

    /// A blank or whitespace `osint_user_agent` loads as unset, so requests keep the
    /// default User-Agent; a custom value is kept (trimmed).
    #[test]
    fn a_blank_osint_user_agent_loads_as_unset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        for raw in ["''", "'   '", "' \t '"] {
            std::fs::write(&path, format!("osint_user_agent = {raw}\n")).unwrap();
            let settings = SettingsFile::load_from(&path).unwrap();
            assert_eq!(settings.osint_user_agent, "", "{raw}");
            assert_eq!(
                crate::osint::effective_user_agent(Some(&settings.osint_user_agent)),
                crate::osint::DEFAULT_USER_AGENT
            );
        }
        std::fs::write(&path, "osint_user_agent = '  Argos test@example.com '\n").unwrap();
        let settings = SettingsFile::load_from(&path).unwrap();
        assert_eq!(
            crate::osint::effective_user_agent(Some(&settings.osint_user_agent)),
            "Argos test@example.com"
        );
    }

    #[test]
    fn empty_config_seeds_tool_picker_and_keeps_synthesis() {
        let dir = tempfile::tempdir().unwrap();
        let missing = SettingsFile::load_from(&dir.path().join("none.toml")).unwrap();
        assert_eq!(missing.defaults.tool_picker.provider, TOOL_PICKER_PROVIDER);
        assert_eq!(missing.defaults.tool_picker.model, TOOL_PICKER_MODEL);
        assert_eq!(missing.defaults.classifier.provider, TOOL_PICKER_PROVIDER);
        assert_eq!(missing.defaults.classifier.model, TOOL_PICKER_MODEL);
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[defaults.synthesis]\nprovider = 'grok'\nmodel = 'grok-4.6'\n",
        )
        .unwrap();
        let settings = SettingsFile::load_from(&path).unwrap();
        assert_eq!(settings.defaults.synthesis.model, "grok-4.6");
        assert_eq!(settings.defaults.synthesis.provider, "grok");
        assert_eq!(settings.defaults.tool_picker.model, TOOL_PICKER_MODEL);
        assert!(settings.defaults.recon.model.is_empty());
        // A saved picker assignment is never overwritten.
        std::fs::write(
            &path,
            "[defaults.tool_picker]\nprovider = 'grok'\nmodel = 'grok-4.6'\n",
        )
        .unwrap();
        let kept = SettingsFile::load_from(&path).unwrap();
        assert_eq!(kept.defaults.tool_picker.model, "grok-4.6");
        assert_eq!(picker_transport(&kept.defaults.tool_picker.model), "chat");
        assert_eq!(picker_transport(TOOL_PICKER_MODEL), "decisions");
        assert_eq!(picker_transport("~typesafe/jev-latest"), "decisions");
    }

    #[test]
    fn tool_picker_role_resolves_and_unknown_roles_error() {
        let mut settings = SettingsFile::default();
        settings.defaults.seed_tool_picker();
        let auth = crate::secrets::AuthFile::default();
        for role in ["tool-picker", "tool_picker"] {
            let secret = role_secret(&auth, &settings, role).unwrap();
            assert_eq!(secret.model, TOOL_PICKER_MODEL);
            assert_eq!(effective_kind(&secret), "openrouter");
        }
        assert!(role_secret(&auth, &settings, "writer").is_err());
        assert!(role_secret(&auth, &settings, "").is_err());
    }

    #[test]
    fn summarization_inherits_synthesis_when_unset() {
        let auth = crate::secrets::AuthFile::default();
        let mut settings = SettingsFile::default();
        settings.defaults.synthesis = ModelAssignment {
            provider: "openrouter".into(),
            model: "test/synth".into(),
        };
        // Empty summarization → inherits at resolve time.
        let secret = role_secret(&auth, &settings, "summarization").unwrap();
        assert_eq!(secret.model, "test/synth");
        assert!(settings.defaults.inherit_summarization_from_synthesis());
        assert_eq!(settings.defaults.summarization.model, "test/synth");
        assert!(!settings.defaults.inherit_summarization_from_synthesis());
    }

    #[tokio::test]
    async fn decisions_client_posts_to_alpha_decisions_and_parses_answers() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let record = seen.clone();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0u8; 65536];
            let mut request = String::new();
            loop {
                let n = socket.read(&mut buffer).await.unwrap();
                request.push_str(&String::from_utf8_lossy(&buffer[..n]));
                let Some(end) = request.find("\r\n\r\n") else {
                    continue;
                };
                let length = request[..end]
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if request.len() >= end + 4 + length || n == 0 {
                    break;
                }
            }
            *record.lock().unwrap() = request;
            let body = r#"{"answers":{"next_tool":{"type":"choice","choice":"firecrawl_search","confidence":0.7,"probabilities":{"firecrawl_search":0.82,"wikidata_entities":0.18}},"depth":{"type":"score","score":1.6,"confidence":0.9,"probabilities":{"0":0.1,"1":0.2,"2":0.7}}},"id":"gen-dec-1","model":"typesafe/jev-1.13-20260917","usage":{"cost":0.00002,"input_tokens":10,"output_tokens":2}}"#;
            let reply = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
            socket.write_all(reply.as_bytes()).await.unwrap();
        });
        let secret = ProviderSecret {
            kind: "openrouter".into(),
            base_url: format!("http://127.0.0.1:{port}/api/v1"),
            model: TOOL_PICKER_MODEL.into(),
            api_key: Some("sk-or-test".into()),
            stt_model: None,
            device: None,
        };
        assert_eq!(
            decisions_url("https://openrouter.ai/api/v1"),
            "https://openrouter.ai/api/alpha/decisions"
        );
        let questions = json!({"next_tool": {"type": "choice", "instructions": "Pick", "criteria": {"firecrawl_search": "search", "wikidata_entities": "record"}}});
        let response = decide(&secret, &json!({"q1": "Who?"}), &questions)
            .await
            .unwrap();
        let request = seen.lock().unwrap().clone();
        assert!(
            request.starts_with("POST /api/alpha/decisions "),
            "{request}"
        );
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-or-test"));
        assert!(request.contains("X-Title") || request.to_ascii_lowercase().contains("x-title"));
        assert!(request.contains("\"model\":\"typesafe/jev-1.13\""));
        let pick = &response.answers["next_tool"];
        assert_eq!(pick.choice.as_deref(), Some("firecrawl_search"));
        assert_eq!(pick.choice_probability(), Some(0.82));
        assert_eq!(response.answers["depth"].score, Some(1.6));
        assert_eq!(response.cost, Some(0.00002));
    }

    #[test]
    fn parse_completion_separates_reasoning_content() {
        let raw = serde_json::json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "role": "assistant",
                    "content": "",
                    "reasoning_content": "## Entity with **predicate**\n\nThe articles support the relation."
                }
            }]
        })
        .to_string();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let msg = v.pointer("/choices/0/message").unwrap();
        assert_eq!(message_text(msg), "");
        let completion = parse_completion(&raw).unwrap();
        assert_eq!(completion.content, "");
        assert!(completion.reasoning.starts_with("## Entity"));
    }

    #[test]
    fn parse_completion_prefers_content_over_reasoning() {
        let raw = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "Visible answer",
                    "reasoning_content": "Hidden chain of thought"
                }
            }]
        })
        .to_string();
        let completion = parse_completion(&raw).unwrap();
        assert_eq!(completion.content, "Visible answer");
        assert_eq!(completion.reasoning, "Hidden chain of thought");
    }

    #[test]
    fn parse_completion_reads_multipart_text_parts() {
        let raw = serde_json::json!({
            "choices": [{
                "message": {
                    "content": [
                        {"type": "text", "text": "Hello "},
                        {"type": "text", "text": "world"}
                    ]
                }
            }]
        })
        .to_string();
        assert_eq!(parse_completion(&raw).unwrap().content, "Hello world");
    }

    #[test]
    fn parse_completion_errors_clearly_when_truly_empty() {
        let raw = serde_json::json!({
            "choices": [{
                "finish_reason": "content_filter",
                "message": {
                    "role": "assistant",
                    "content": null,
                    "refusal": "Policy blocked"
                }
            }]
        })
        .to_string();
        let err = parse_completion(&raw).unwrap_err().to_string();
        assert!(err.contains("empty completion"), "{err}");
        assert!(err.contains("finish_reason=content_filter"), "{err}");
        assert!(err.contains("refusal: Policy blocked"), "{err}");
    }

    #[test]
    fn sse_acc_separates_reasoning_deltas_from_content() {
        let mut acc = SseAcc::default();
        let line1 = format!(
            "data: {}",
            serde_json::json!({"choices":[{"delta":{"reasoning_content":"## Heading"}}]})
        );
        let line2 = format!(
            "data: {}",
            serde_json::json!({"choices":[{"delta":{"reasoning":" body"},"finish_reason":"stop"}]})
        );
        assert!(acc.push_line(&line1).is_none());
        assert!(acc.push_line(&line2).is_none());
        let completion = acc.into_completion();
        assert_eq!(completion.content, "");
        assert_eq!(completion.reasoning, "## Heading body");
        assert_eq!(completion.finish_reason.as_deref(), Some("stop"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn complete_reads_reasoning_only_non_stream_json() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            for _ in 0..2 {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let mut buffer = vec![0u8; 65536];
                let mut request = String::new();
                loop {
                    let n = socket.read(&mut buffer).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    request.push_str(&String::from_utf8_lossy(&buffer[..n]));
                    let Some(end) = request.find("\r\n\r\n") else {
                        continue;
                    };
                    let length = request[..end]
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
                if request.contains("\"stream\":true") {
                    let body = "{\"error\":{\"message\":\"stream unsupported\"}}";
                    let reply = format!(
                        "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(reply.as_bytes()).await;
                    continue;
                }
                let body = serde_json::json!({
                    "choices": [{
                        "finish_reason": "stop",
                        "message": {
                            "role": "assistant",
                            "content": "",
                            "reasoning_content": "## Graph\n\nSupported by the articles."
                        }
                    }]
                })
                .to_string();
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(reply.as_bytes()).await;
            }
        });
        let secret = ProviderSecret {
            kind: "openrouter".into(),
            base_url: format!("http://127.0.0.1:{port}/v1"),
            model: "test-reasoning".into(),
            api_key: Some("sk-test".into()),
            stt_model: None,
            device: None,
        };
        let messages = [ChatMessage {
            role: "user".into(),
            content: "Summarize".into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        }];
        let completion = complete(&secret, &messages, &[], |_| {}).await.unwrap();
        assert_eq!(completion.content, "");
        assert!(
            completion.reasoning.contains("## Graph"),
            "got {:?}",
            completion.reasoning
        );
    }

    /// Streaming path: reasoning deltas separated from content deltas.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn complete_stream_of_reasoning_deltas_yields_text() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buf = vec![0u8; 1 << 16];
            let mut req = Vec::new();
            loop {
                let n = socket.read(&mut buf).await.expect("read");
                if n == 0 {
                    break;
                }
                req.extend_from_slice(&buf[..n]);
                if req.windows(4).any(|w| w == b"\r\n\r\n") {
                    // One shot is enough for this test; ignore remaining body bytes.
                    break;
                }
            }
            let event1 =
                serde_json::json!({"choices":[{"delta":{"reasoning_content":"## Claim thought"}}]});
            let event2 = serde_json::json!({"choices":[{"delta":{"content":"Final answer text"},"finish_reason":"stop"}]});
            let body = format!("data: {event1}\n\ndata: {event2}\n\ndata: [DONE]\n\n");
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(reply.as_bytes()).await.expect("write");
        });
        // Yield so the accept future is polled before the client connects.
        tokio::task::yield_now().await;
        let secret = ProviderSecret {
            kind: "local".into(),
            base_url: format!("http://{addr}/v1"),
            model: "test-reasoning".into(),
            api_key: None,
            stt_model: None,
            device: None,
        };
        let messages = [ChatMessage {
            role: "user".into(),
            content: "Summarize".into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        }];
        let mut deltas = String::new();
        let completion = complete(&secret, &messages, &[], |d| deltas.push_str(d))
            .await
            .expect("complete stream");
        assert_eq!(completion.content, "Final answer text");
        assert_eq!(completion.reasoning, "## Claim thought");
        assert_eq!(deltas, "Final answer text");
        assert!(!deltas.contains("Claim thought"), "{deltas}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn complete_errors_with_finish_reason_when_response_is_empty() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            for _ in 0..2 {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let mut buffer = vec![0u8; 65536];
                let mut request = String::new();
                loop {
                    let n = socket.read(&mut buffer).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    request.push_str(&String::from_utf8_lossy(&buffer[..n]));
                    let Some(end) = request.find("\r\n\r\n") else {
                        continue;
                    };
                    let length = request[..end]
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
                if request.contains("\"stream\":true") {
                    let event = serde_json::json!({"choices":[{"delta":{},"finish_reason":"content_filter"}]});
                    let stream_body = format!("data: {event}\n\ndata: [DONE]\n\n");
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{stream_body}",
                        stream_body.len()
                    );
                    let _ = socket.write_all(reply.as_bytes()).await;
                } else {
                    let body = serde_json::json!({
                        "choices": [{
                            "finish_reason": "content_filter",
                            "message": {
                                "role": "assistant",
                                "content": "",
                                "refusal": "blocked"
                            }
                        }]
                    })
                    .to_string();
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(reply.as_bytes()).await;
                }
            }
        });
        let secret = ProviderSecret {
            kind: "openrouter".into(),
            base_url: format!("http://127.0.0.1:{port}/v1"),
            model: "test-empty".into(),
            api_key: Some("sk-test".into()),
            stt_model: None,
            device: None,
        };
        let messages = [ChatMessage {
            role: "user".into(),
            content: "Summarize".into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        }];
        let err = complete(&secret, &messages, &[], |_| {})
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("empty completion"), "{err}");
        assert!(err.contains("finish_reason=content_filter"), "{err}");
        assert!(err.contains("refusal: blocked"), "{err}");
    }

    /// Live Jev smoke. Runs only with `OPENROUTER_API_KEY` set: `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn live_decisions_smoke() {
        let Ok(key) = std::env::var("OPENROUTER_API_KEY") else {
            return;
        };
        let secret = ProviderSecret {
            kind: "openrouter".into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            model: TOOL_PICKER_MODEL.into(),
            api_key: Some(key),
            stt_model: None,
            device: None,
        };
        let questions = json!({"next_tool": {"type": "choice", "instructions": "Which tool should run first to find the official website of Example Org?", "criteria": {"firecrawl_search": "Web search", "nvd_cve": "CVE record lookup"}}});
        let response = decide(
            &secret,
            &json!({"question": "official website of Example Org"}),
            &questions,
        )
        .await
        .unwrap();
        assert!(response.answers["next_tool"].choice.is_some());
    }
}
