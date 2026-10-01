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

#[derive(Clone, Debug)]
pub struct Completion {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReconLimits {
    #[serde(default = "default_max_rounds")]
    pub max_rounds: u8,
    #[serde(default = "default_max_calls")]
    pub max_calls: u8,
    #[serde(default = "default_turn_seconds")]
    pub turn_seconds: u16,
    /// Recurring Firecrawl credits available to automatic investigation.
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
    /// Maximum SociaVault calls on the opening turn.
    #[serde(default = "default_opening_cap")]
    pub opening_sociavault_calls: u8,
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
fn default_firecrawl_credits() -> u32 {
    200
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
            firecrawl_credits: default_firecrawl_credits(),
            hunter_credits: default_hunter_credits(),
            sociavault_credits: default_sociavault_credits(),
            firecrawl_trial_credits: 0,
            hunter_trial_credits: 0,
            sociavault_trial_credits: 0,
            opening_hunter_calls: default_opening_cap(),
            opening_sociavault_calls: default_opening_cap(),
            credit_reset: default_credit_reset(),
            firecrawl_search_cost: default_search_cost(),
            firecrawl_scrape_cost: default_one_cost(),
            hunter_call_cost: default_one_cost(),
            sociavault_call_cost: default_one_cost(),
        }
    }
}

impl ReconLimits {
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
        let priced = |provider: &'static str, credits: u32| Some((provider, credits));
        match tool_id {
            "firecrawl_search" => priced("firecrawl", self.firecrawl_search_cost),
            "firecrawl_scrape" => priced("firecrawl", self.firecrawl_scrape_cost),
            "hunter_domain_search" | "hunter_email_finder" | "hunter_email_verifier"
            | "hunter_tech_lookup" => priced("hunter", self.hunter_call_cost),
            "sociavault_profile" => priced("sociavault", self.sociavault_call_cost),
            _ => None,
        }
    }
}

pub fn role_secret(
    auth: &crate::secrets::AuthFile,
    settings: &SettingsFile,
    role: &str,
) -> Result<ProviderSecret> {
    let assignment = match role {
        "recon" => &settings.defaults.recon,
        "synthesis" => &settings.defaults.synthesis,
        _ => return Err(anyhow!("role must be recon or synthesis")),
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

async fn authorize(
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
        let text = resp.text().await.unwrap_or_default();
        // Some local servers reject stream+tools. Retry once without streaming.
        if status.as_u16() == 400 || status.as_u16() == 404 {
            return complete_once(secret, messages, tools).await;
        }
        return Err(anyhow!("provider {status}: {text}"));
    }
    let mut stream = resp.bytes_stream();
    let mut acc = SseAcc::default();
    let mut buf = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("read provider stream")?;
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(idx) = buf.find('\n') {
            let line = buf[..idx].trim().to_string();
            buf = buf[idx + 1..].to_string();
            if let Some(delta) = acc.push_line(&line) {
                on_delta(&delta);
            }
        }
    }
    if acc.content.is_empty() && acc.tool_calls.is_empty() {
        return complete_once(secret, messages, tools).await;
    }
    Ok(Completion {
        content: acc.content,
        tool_calls: acc.tool_calls,
    })
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
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("provider {status}: {text}"));
    }
    parse_completion(&text)
}

fn chat_body(
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

pub fn parse_completion(text: &str) -> Result<Completion> {
    let v: Value = serde_json::from_str(text).context("provider JSON")?;
    let msg = v
        .pointer("/choices/0/message")
        .cloned()
        .unwrap_or(Value::Null);
    let content = msg
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
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
    Ok(Completion {
        content,
        tool_calls,
    })
}

#[derive(Default)]
struct SseAcc {
    content: String,
    tool_calls: Vec<ToolCall>,
}

impl SseAcc {
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
        let delta = v.pointer("/choices/0/delta")?;
        let mut emitted = None;
        if let Some(text) = delta.get("content").and_then(|c| c.as_str()) {
            if !text.is_empty() {
                self.content.push_str(text);
                emitted = Some(text.to_string());
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
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
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
    /// Hunter API key. A non-empty value overrides `HUNTER_API_KEY`.
    #[serde(default)]
    pub hunter_api_key: String,
    /// SociaVault API key. A non-empty value overrides `SOCIAVAULT_API_KEY`.
    #[serde(default)]
    pub sociavault_api_key: String,
    #[serde(default)]
    pub recon_limits: ReconLimits,
}

impl SettingsFile {
    pub fn load() -> Result<Self> {
        let path = crate::paths::config_path();
        Self::load_from(&path)
    }

    fn load_from(path: &std::path::Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)?;
        if raw.trim().is_empty() {
            return Ok(Self::default());
        }
        let mut settings: Self = toml::from_str(&raw)?;
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

    fn save_to(&self, path: &std::path::Path) -> Result<()> {
        crate::secrets::write_private(path, &toml::to_string_pretty(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
