//! Text and voice providers.
//!
//! Text chats use the OpenAI-compatible chat completions API (xAI, OpenAI,
//! Ollama, llama.cpp, LM Studio). Voice uses the OpenAI-compatible
//! `/audio/transcriptions` endpoint. Login is either an API key typed at the
//! terminal, a local base URL, or an RFC 8628 device-code grant. Argos shows
//! the verification URL and user code; it does not embed another product's
//! OAuth client id.

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

pub async fn complete(
    secret: &ProviderSecret,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
    mut on_delta: impl FnMut(&str),
) -> Result<Completion> {
    let client = http()?;
    let url = format!("{}/chat/completions", normalize_base(&secret.base_url));
    let body = chat_body(secret, messages, tools, true);
    let mut req = client.post(&url).json(&body);
    if let Some(key) = secret.api_key.as_ref().filter(|k| !k.is_empty()) {
        req = req.bearer_auth(key);
    }
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
    let mut req = client.post(&url).json(&body);
    if let Some(key) = secret.api_key.as_ref().filter(|k| !k.is_empty()) {
        req = req.bearer_auth(key);
    }
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
    let mut req = client.post(&url).multipart(form);
    if let Some(key) = secret.api_key.as_ref().filter(|k| !k.is_empty()) {
        req = req.bearer_auth(key);
    }
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

pub async fn list_models(secret: &ProviderSecret) -> Result<Vec<String>> {
    let client = http()?;
    let url = format!("{}/models", normalize_base(&secret.base_url));
    let mut req = client.get(&url);
    if let Some(key) = secret.api_key.as_ref().filter(|k| !k.is_empty()) {
        req = req.bearer_auth(key);
    }
    let resp = req.send().await.with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("models {status}: {text}"));
    }
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let mut names = Vec::new();
    if let Some(data) = v.get("data").and_then(|d| d.as_array()) {
        for item in data {
            if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                names.push(id.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
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
    #[serde(default = "default_layout")]
    pub layout: String,
    #[serde(default = "default_modality")]
    pub modality: String,
}

fn default_layout() -> String {
    "classic".into()
}
fn default_modality() -> String {
    "text".into()
}

impl Default for SettingsFile {
    fn default() -> Self {
        Self {
            searx_url: String::new(),
            report_dir: String::new(),
            layout: default_layout(),
            modality: default_modality(),
        }
    }
}

impl SettingsFile {
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

    #[test]
    fn device_grant_parses() {
        let raw = r#"{"device_code":"d","user_code":"ABCD-EFGH","verification_uri":"https://example.com/device","expires_in":600,"interval":5}"#;
        let g: DeviceGrant = serde_json::from_str(raw).unwrap();
        assert_eq!(g.user_code, "ABCD-EFGH");
        assert!(g.verification_uri.starts_with("https://"));
    }
}
