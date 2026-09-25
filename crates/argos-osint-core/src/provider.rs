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
    secret
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
        "openai" => "openai".into(),
        "openrouter" => "openrouter".into(),
        "local" | "ollama" | "llama" | "llamacpp" | "lmstudio" => "local".into(),
        _ => kind.trim().to_lowercase(),
    }
}

/// Provider id used for headers and the environment key. A stored id wins.
/// Older files that only have a base URL are classified from the host.
pub fn effective_kind(secret: &ProviderSecret) -> String {
    let kind = normalize_kind(&secret.kind);
    if preset(&kind).is_some() {
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
    } else if host == "localhost" || host == "127.0.0.1" || host == "::1" || host == "0.0.0.0" {
        "local".into()
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
                "No Grok credentials. Run `grok login` or `argos login`, or set XAI_API_KEY."
            ));
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
    let client = http()?;
    let url = format!("{}/models", normalize_base(&secret.base_url));
    let req = authorize(client.get(&url), secret).await?;
    let resp = req.send().await.with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("models {status}: {text}"));
    }
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Ok(parse_model_catalog(&v))
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SettingsFile {
    #[serde(default)]
    pub searx_url: String,
    #[serde(default)]
    pub report_dir: String,
    #[serde(default = "default_modality")]
    pub modality: String,
    /// Connection model. Empty means the provider's own default, which
    /// is `grok-4.6` until another provider is signed in.
    #[serde(default)]
    pub model: String,
    /// User-facing model. Empty uses `model`.
    #[serde(default)]
    pub writer_model: String,
    /// Tool-calling model. Empty uses `model`.
    #[serde(default)]
    pub tool_model: String,
    /// Wikipedia and Wikidata. `wikipedia` is the old name.
    #[serde(default = "default_true", alias = "wikipedia")]
    pub facts: bool,
    /// SearXNG, or DuckDuckGo, plus Brave and Tavily when a key is set.
    /// `internet` is the old name.
    #[serde(default = "default_true", alias = "internet")]
    pub web: bool,
    #[serde(default = "default_true")]
    pub news: bool,
    #[serde(default = "default_true")]
    pub domain: bool,
    #[serde(default = "default_true")]
    pub social: bool,
    #[serde(default = "default_true")]
    pub identity: bool,
    #[serde(default)]
    pub brave_key: String,
    #[serde(default)]
    pub tavily_key: String,
    #[serde(default)]
    pub youtube_key: String,
    #[serde(default)]
    pub github_token: String,
    /// Extra public sources. Each URL template must contain `{query}`.
    #[serde(default)]
    pub sources: Vec<crate::search::OsintSource>,
}

fn default_true() -> bool {
    true
}

fn default_modality() -> String {
    "text".into()
}

impl Default for SettingsFile {
    fn default() -> Self {
        Self {
            searx_url: String::new(),
            report_dir: String::new(),
            modality: default_modality(),
            model: String::new(),
            writer_model: String::new(),
            tool_model: String::new(),
            facts: true,
            web: true,
            news: true,
            domain: true,
            social: true,
            identity: true,
            brave_key: String::new(),
            tavily_key: String::new(),
            youtube_key: String::new(),
            github_token: String::new(),
            sources: Vec::new(),
        }
    }
}

fn filled(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn secret_or_env(value: &str, env_name: &str) -> Option<String> {
    if let Some(value) = filled(value) {
        return Some(value);
    }
    std::env::var(env_name)
        .ok()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
}

impl SettingsFile {
    /// Global OSINT defaults. Empty key fields fall back to the process environment.
    pub fn source_plan(&self) -> crate::search::SourcePlan {
        crate::search::SourcePlan {
            facts: self.facts,
            web: self.web,
            news: self.news,
            domain: self.domain,
            social: self.social,
            identity: self.identity,
            searx_url: filled(&self.searx_url),
            brave_key: secret_or_env(&self.brave_key, "BRAVE_API_KEY"),
            tavily_key: secret_or_env(&self.tavily_key, "TAVILY_API_KEY"),
            youtube_key: secret_or_env(&self.youtube_key, "YOUTUBE_API_KEY"),
            github_token: secret_or_env(&self.github_token, "GITHUB_TOKEN"),
            extra: self.sources.clone(),
        }
    }

    pub fn load() -> Result<Self> {
        let path = crate::paths::config_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)?;
        if raw.trim().is_empty() {
            return Ok(Self::default());
        }
        Ok(toml::from_str(&raw)?)
    }

    pub fn save(&self) -> Result<()> {
        crate::paths::ensure_home()?;
        std::fs::write(crate::paths::config_path(), toml::to_string_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_osint_toggles_load_as_stages() {
        let settings: SettingsFile =
            toml::from_str("internet = false\nwikipedia = false\n").unwrap();
        assert!(!settings.web);
        assert!(!settings.facts);
        assert!(settings.news);
        assert!(settings.domain);
        assert!(settings.social);
        assert!(settings.identity);
    }

    #[test]
    fn parses_tool_completion_and_sse() {
        let raw = r#"{"choices":[{"message":{"role":"assistant","content":"","tool_calls":[{"id":"c1","type":"function","function":{"name":"web_search","arguments":"{\"query\":\"port\"}"}}]}}]}"#;
        let done = parse_completion(raw).unwrap();
        assert_eq!(done.tool_calls[0].name, "web_search");
        let mut acc = SseAcc::default();
        let d = acc
            .push_line(r#"data: {"choices":[{"delta":{"content":"Hello"}}]}"#)
            .unwrap();
        assert_eq!(d, "Hello");
        acc.push_line(r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"web_search","arguments":"{\"q\":"}}]}}]}"#);
        acc.push_line(r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"1}"}}]}}]}"#);
        assert_eq!(acc.tool_calls[0].arguments, "{\"q\":1}");
    }

    fn secret(kind: &str, base: &str, key: Option<&str>) -> ProviderSecret {
        ProviderSecret {
            kind: kind.into(),
            base_url: base.into(),
            model: "m".into(),
            api_key: key.map(|k| k.to_string()),
            stt_model: None,
            device: None,
        }
    }

    #[test]
    fn presets_cover_cloud_and_local() {
        let ids: Vec<_> = presets().iter().map(|preset| preset.id).collect();
        assert_eq!(ids, vec!["grok", "openai", "openrouter", "local"]);
        assert!(preset("xai").unwrap().base_url.contains("api.x.ai"));
        assert_eq!(normalize_kind("LM Studio"), "local");
        assert!(!preset("local").unwrap().key_required);
        assert!(preset("openrouter").unwrap().key_required);
    }

    #[test]
    fn kind_follows_host_when_the_saved_id_is_generic() {
        let saved = secret("api", "https://openrouter.ai/api/v1", None);
        assert_eq!(effective_kind(&saved), "openrouter");
        let local = secret("api", "http://127.0.0.1:11434/v1", None);
        assert_eq!(effective_kind(&local), "local");
        let named = secret("openai", "https://example.test/v1", None);
        assert_eq!(effective_kind(&named), "openai");
    }

    #[test]
    fn key_prefers_the_file_and_headers_are_only_for_openrouter() {
        let stored = secret("grok", "https://api.x.ai/v1", Some("stored-key"));
        let found = resolved_key_with(&stored, |_| Some("from-env".into()));
        assert_eq!(found.as_deref(), Some("stored-key"));
        assert!(provider_headers(&stored).is_empty());

        let from_env = secret("openrouter", "https://openrouter.ai/api/v1", None);
        let found = resolved_key_with(&from_env, |name| {
            assert_eq!(name, "OPENROUTER_API_KEY");
            Some("or-key".into())
        });
        assert_eq!(found.as_deref(), Some("or-key"));
        let names: Vec<_> = provider_headers(&from_env)
            .iter()
            .map(|(name, _)| *name)
            .collect();
        assert!(names.contains(&"HTTP-Referer"));
        assert!(names.contains(&"X-Title"));
    }

    #[test]
    fn grok_defaults_to_the_current_build_model_and_picks_unambiguously() {
        assert_eq!(default_grok_model(), "grok-4.6");
        assert_eq!(preset("grok").unwrap().text_model, "grok-4.6");
        let choices = grok_models()
            .iter()
            .map(|model| (model.id.to_string(), model.name.to_string()))
            .collect::<Vec<_>>();
        assert_eq!(
            resolve_model_choice(&choices, "grok-4.5").as_deref(),
            Some("grok-4.5")
        );
        assert_eq!(
            resolve_model_choice(&choices, "Grok 4.6").as_deref(),
            Some("grok-4.6")
        );
        assert!(resolve_model_choice(&choices, "grok").is_none());
        assert!(resolve_model_choice(&choices, "  ").is_none());
        let secret = active_text_secret(&crate::secrets::AuthFile::default(), "");
        assert_eq!(secret.kind, "grok");
        assert_eq!(secret.model, "grok-4.6");
        assert_eq!(secret.base_url, "https://api.x.ai/v1");
        let picked = active_text_secret(&crate::secrets::AuthFile::default(), "grok-4.5");
        assert_eq!(picked.model, "grok-4.5");
    }

    #[test]
    fn free_router_opens_onto_concrete_free_models() {
        let raw = r#"{"data":[
            {"id":"openrouter/free","name":"Free Models Router","pricing":{"prompt":"0","completion":"0"}},
            {"id":"meta-llama/llama-3.2-3b-instruct:free","name":"Meta: Llama 3.2 3B Instruct (free)","pricing":{"prompt":"0","completion":"0"}},
            {"id":"openai/gpt-4.1","name":"OpenAI: GPT-4.1","pricing":{"prompt":"0.002","completion":"0.008"}},
            {"id":"stealth/space-bunny","name":"Space Bunny","pricing":{"prompt":"0","completion":"0"}}
        ]}"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let catalog = parse_model_catalog(&value);
        assert!(is_free_router("openrouter/free"));
        assert!(
            !catalog
                .iter()
                .find(|m| m.id == "openrouter/free")
                .unwrap()
                .free
        );
        let free = concrete_free_models(&catalog);
        let ids: Vec<_> = free.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "meta-llama/llama-3.2-3b-instruct:free",
                "stealth/space-bunny"
            ]
        );
        assert!(!ids.contains(&"openai/gpt-4.1"));
        assert!(!ids.contains(&"openrouter/free"));
    }

    #[test]
    fn device_grant_parses() {
        let raw = r#"{"device_code":"d","user_code":"ABCD-EFGH","verification_uri":"https://example.com/device","expires_in":600,"interval":5}"#;
        let g: DeviceGrant = serde_json::from_str(raw).unwrap();
        assert_eq!(g.user_code, "ABCD-EFGH");
        assert!(g.verification_uri.starts_with("https://"));
    }
}
