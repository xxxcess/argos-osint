//! Catalog-driven model capabilities (spec §3).
//!
//! Each model's capabilities are resolved from three sources, in increasing
//! precedence:
//! 1. conservative adapter defaults (every optional feature unknown),
//! 2. data-only documented profiles (`provider_profiles.json`, matched on
//!    the exact provider and model ID, never on a model family),
//! 3. cached catalog evidence (OpenRouter endpoint `supported_parameters`,
//!    Google `supportedGenerationMethods` and limits).
//!
//! Scoped rejections observed on the wire are applied last, so a specific
//! observed rejection always beats a broader positive claim. Unknown optional
//! features are never sent; the answer is validated locally instead.
//!
//! [`ResolvedCapabilities::digest`] hashes only the semantic support states,
//! allowed values and limits, in sorted order. Fetch and expiry timestamps
//! and raw descriptions are excluded, so a freshness-only refresh never
//! invalidates an extraction cache.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Tri-state support for one optional feature.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Support {
    #[default]
    Unknown,
    Supported,
    Unsupported,
}

impl Support {
    pub fn is_supported(self) -> bool {
        self == Self::Supported
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Supported => "supported",
            Self::Unsupported => "unsupported",
        }
    }
}

/// Which provider-owned adapter compiles requests for a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterId {
    OpenRouter,
    Google,
    Nvidia,
    /// Any other OpenAI-compatible chat endpoint (x.ai, OpenAI, local, proxies).
    OpenAiCompatible,
}

impl AdapterId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenRouter => "openrouter",
            Self::Google => "google",
            Self::Nvidia => "nvidia",
            Self::OpenAiCompatible => "openai_compatible",
        }
    }

    /// Adapter for a provider kind (already normalized by `effective_kind`).
    pub fn for_kind(kind: &str) -> Self {
        match kind {
            "openrouter" => Self::OpenRouter,
            "google" => Self::Google,
            "nvidia" => Self::Nvidia,
            _ => Self::OpenAiCompatible,
        }
    }

    fn profile_provider(self) -> &'static str {
        self.as_str()
    }
}

/// Wire API used for one request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiDialect {
    /// OpenAI-compatible `POST {base}/chat/completions`.
    Chat,
    /// Google native `models/{id}:generateContent` / `:streamGenerateContent`.
    GoogleGenerateContent,
    /// OpenRouter `POST /api/alpha/decisions`.
    NativeDecisions,
}

impl ApiDialect {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::GoogleGenerateContent => "google_generate_content",
            Self::NativeDecisions => "native_decisions",
        }
    }
}

/// How a model's thinking is controlled on its hosted endpoint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningControl {
    /// No confirmed control: never send a reasoning field.
    #[default]
    None,
    /// OpenRouter `reasoning: {effort | enabled | max_tokens}`.
    OpenRouter,
    /// Top-level `reasoning_effort` string (Nvidia hosted, OpenAI-compatible).
    ReasoningEffort,
    /// `chat_template_kwargs.enable_thinking` (Nvidia Lightning).
    ChatTemplateThinking,
    /// Google native `generationConfig.thinkingConfig.thinkingLevel`.
    GoogleLevel,
}

impl ReasoningControl {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::OpenRouter => "openrouter",
            Self::ReasoningEffort => "reasoning_effort",
            Self::ChatTemplateThinking => "chat_template_thinking",
            Self::GoogleLevel => "google_level",
        }
    }
}

/// Reasoning capabilities. `values` are the exact wire values the endpoint
/// accepts (empty = none confirmed); `off` is the value that disables
/// thinking, if one exists; `efficient` is the profile's low-cost value.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReasoningCaps {
    pub control: ReasoningControl,
    #[serde(default)]
    pub values: Vec<String>,
    #[serde(default)]
    pub off: Option<String>,
    #[serde(default)]
    pub efficient: Option<String>,
    /// Thinking cannot be turned off.
    #[serde(default)]
    pub mandatory: bool,
    /// Separate reasoning budget field (Nvidia `reasoning_budget`).
    #[serde(default)]
    pub budget_field: Option<String>,
    /// Whether the endpoint enforces that budget as a hard cap.
    #[serde(default)]
    pub budget_enforced: bool,
    /// OpenRouter `reasoning.max_tokens` confirmed by the catalog.
    #[serde(default)]
    pub max_tokens: bool,
}

/// Where the resolved capabilities came from.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// `conservative`, `profile`, `catalog`, `observed`, in the order applied.
    pub sources: Vec<String>,
    /// Profile evidence note, if a documented profile matched.
    pub profile: Option<String>,
    /// Catalog fetch time (Unix seconds); excluded from the digest.
    pub catalog_fetched_at: Option<i64>,
}

/// Resolved capabilities for one (provider, base URL, dialect, model, upstream).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolvedCapabilities {
    pub adapter: AdapterId,
    pub model: String,
    /// Native decisions endpoint only; never send generation to it.
    pub decision_only: bool,
    /// Text generation (unknown text models remain usable).
    pub text_generation: Support,
    /// Google native `generateContent` advertised.
    pub generate_content: Support,
    /// Chat `response_format: json_schema` strict structured outputs.
    pub strict_schema: Support,
    /// Google native schema output (`responseFormat`).
    pub native_schema: Support,
    /// JSON mode (`response_format: json_object` / native JSON MIME type).
    pub json_mode: Support,
    pub tools: Support,
    /// Forced function selection (`tool_choice: {type:function,...}`).
    pub forced_tool: Support,
    pub reasoning: ReasoningCaps,
    /// Serving output limit, when advertised.
    pub max_output_tokens: Option<u32>,
    /// Documented sampling default to send; `None` keeps the model default.
    pub temperature: Option<f64>,
    /// Observed rejections `(field, value)`; value `*` rejects the field.
    pub rejected: BTreeSet<(String, String)>,
    pub provenance: Provenance,
}

impl ResolvedCapabilities {
    /// Conservative defaults: every optional feature unknown.
    pub fn conservative(adapter: AdapterId, model: &str) -> Self {
        Self {
            adapter,
            model: model.to_string(),
            decision_only: false,
            text_generation: Support::Unknown,
            generate_content: Support::Unknown,
            strict_schema: Support::Unknown,
            native_schema: Support::Unknown,
            json_mode: Support::Unknown,
            tools: Support::Unknown,
            forced_tool: Support::Unknown,
            reasoning: ReasoningCaps::default(),
            max_output_tokens: None,
            temperature: None,
            rejected: BTreeSet::new(),
            provenance: Provenance {
                sources: vec!["conservative".into()],
                ..Provenance::default()
            },
        }
    }

    /// Whether `field=value` (or the whole field) was rejected on this route.
    pub fn is_rejected(&self, field: &str, value: &str) -> bool {
        self.rejected
            .contains(&(field.to_string(), value.to_string()))
            || self
                .rejected
                .contains(&(field.to_string(), "*".to_string()))
    }

    /// Whether a forced output tool can carry a schema-shaped answer.
    pub fn output_tool_usable(&self) -> bool {
        self.tools.is_supported()
            && self.forced_tool.is_supported()
            && !self.is_rejected("tool_choice", "function")
    }

    /// Semantic digest: sorted support states, allowed values and limits.
    /// Timestamps, provenance and descriptions are excluded.
    pub fn digest(&self) -> String {
        let mut values = self.reasoning.values.clone();
        values.sort();
        let semantic = serde_json::json!({
            "adapter": self.adapter.as_str(),
            "model": self.model,
            "decision_only": self.decision_only,
            "text_generation": self.text_generation.as_str(),
            "generate_content": self.generate_content.as_str(),
            "strict_schema": self.strict_schema.as_str(),
            "native_schema": self.native_schema.as_str(),
            "json_mode": self.json_mode.as_str(),
            "tools": self.tools.as_str(),
            "forced_tool": self.forced_tool.as_str(),
            "reasoning": {
                "control": self.reasoning.control.as_str(),
                "values": values,
                "off": self.reasoning.off,
                "efficient": self.reasoning.efficient,
                "mandatory": self.reasoning.mandatory,
                "budget_field": self.reasoning.budget_field,
                "budget_enforced": self.reasoning.budget_enforced,
                "max_tokens": self.reasoning.max_tokens,
            },
            "max_output_tokens": self.max_output_tokens,
            "temperature": self.temperature.map(|t| format!("{t:.4}")),
            "rejected": self.rejected.iter().map(|(f, v)| format!("{f}={v}")).collect::<Vec<_>>(),
        });
        let digest = Sha256::digest(semantic.to_string().as_bytes());
        digest[..8].iter().map(|b| format!("{b:02x}")).collect()
    }
}

// ---------- Documented profiles ----------

#[derive(Clone, Debug, Default, Deserialize)]
struct ProfileFile {
    #[allow(dead_code)]
    version: u32,
    profiles: Vec<Profile>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    provider: String,
    model: String,
    evidence: String,
    #[serde(default)]
    decision_only: bool,
    #[serde(default)]
    text_generation: Option<Support>,
    #[serde(default)]
    generate_content: Option<Support>,
    #[serde(default)]
    strict_schema: Option<Support>,
    #[serde(default)]
    native_schema: Option<Support>,
    #[serde(default)]
    json_mode: Option<Support>,
    #[serde(default)]
    tools: Option<Support>,
    #[serde(default)]
    forced_tool: Option<Support>,
    #[serde(default)]
    reasoning: Option<ReasoningCaps>,
    #[serde(default)]
    max_output_tokens: Option<u32>,
    #[serde(default)]
    temperature: Option<f64>,
}

fn profiles() -> &'static HashMap<(String, String), Profile> {
    static PROFILES: OnceLock<HashMap<(String, String), Profile>> = OnceLock::new();
    PROFILES.get_or_init(|| {
        let file: ProfileFile = serde_json::from_str(include_str!("provider_profiles.json"))
            .expect("provider_profiles.json is valid");
        file.profiles
            .into_iter()
            .map(|p| ((p.provider.clone(), normalize_model_key(&p.model)), p))
            .collect()
    })
}

/// Profile/cache lookup key for a model: Google `models/` prefix removed.
pub fn normalize_model_key(model: &str) -> String {
    let m = model.trim();
    m.strip_prefix("models/").unwrap_or(m).to_string()
}

/// Number of documented profiles (test/report helper).
pub fn profile_count() -> usize {
    profiles().len()
}

fn apply_profile(caps: &mut ResolvedCapabilities, p: &Profile) {
    caps.decision_only = p.decision_only;
    if p.decision_only {
        caps.text_generation = Support::Unsupported;
    }
    let set = |slot: &mut Support, v: Option<Support>| {
        if let Some(v) = v {
            *slot = v;
        }
    };
    set(&mut caps.text_generation, p.text_generation);
    set(&mut caps.generate_content, p.generate_content);
    set(&mut caps.strict_schema, p.strict_schema);
    set(&mut caps.native_schema, p.native_schema);
    set(&mut caps.json_mode, p.json_mode);
    set(&mut caps.tools, p.tools);
    set(&mut caps.forced_tool, p.forced_tool);
    if let Some(r) = &p.reasoning {
        caps.reasoning = r.clone();
    }
    if p.max_output_tokens.is_some() {
        caps.max_output_tokens = p.max_output_tokens;
    }
    if p.temperature.is_some() {
        caps.temperature = p.temperature;
    }
    caps.provenance.sources.push("profile".into());
    caps.provenance.profile = Some(p.evidence.clone());
}

// ---------- Catalog evidence ----------

/// Normalized catalog metadata for one model endpoint.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CatalogEvidence {
    /// OpenRouter `supported_parameters` (intersection of eligible endpoints).
    #[serde(default)]
    pub supported_parameters: Vec<String>,
    /// Model-specific effort vocabulary from OpenRouter's model catalog.
    /// Parameter support alone does not establish any particular effort.
    #[serde(default)]
    pub reasoning: Option<ReasoningCaps>,
    /// Google `supportedGenerationMethods`.
    #[serde(default)]
    pub generation_methods: Vec<String>,
    #[serde(default)]
    pub output_token_limit: Option<u32>,
    /// Google `thinking: true`.
    #[serde(default)]
    pub thinking: Option<bool>,
    #[serde(default)]
    pub temperature: Option<f64>,
    /// Upstream endpoint/provider name when known (OpenRouter).
    #[serde(default)]
    pub upstream: Option<String>,
}

/// Parse OpenRouter `GET /models/{id}/endpoints`. Only free endpoints are
/// eligible for a `:free` model, and parameters must hold on every eligible
/// endpoint (intersection), because OpenRouter may route to any of them.
pub fn parse_openrouter_endpoints(model: &str, value: &Value) -> Option<CatalogEvidence> {
    let endpoints = value.pointer("/data/endpoints")?.as_array()?;
    let free_only = model.ends_with(":free");
    let mut params: Option<BTreeSet<String>> = None;
    let mut limit: Option<u32> = None;
    let mut upstream = Vec::new();
    for ep in endpoints {
        let free = ["prompt", "completion"].iter().all(|k| {
            ep.pointer(&format!("/pricing/{k}"))
                .and_then(|v| {
                    v.as_str()
                        .and_then(|s| s.parse::<f64>().ok())
                        .or(v.as_f64())
                })
                .is_some_and(|p| p == 0.0)
        });
        if free_only && !free {
            continue;
        }
        let set: BTreeSet<String> = ep
            .get("supported_parameters")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        params = Some(match params {
            None => set,
            Some(p) => p.intersection(&set).cloned().collect(),
        });
        if let Some(l) = ep.get("max_completion_tokens").and_then(Value::as_u64) {
            limit = Some(limit.map_or(l as u32, |x| x.min(l as u32)));
        }
        if let Some(name) = ep.get("provider_name").and_then(Value::as_str) {
            upstream.push(name.to_string());
        }
    }
    let params = params?;
    upstream.sort();
    Some(CatalogEvidence {
        supported_parameters: params.into_iter().collect(),
        reasoning: parse_openrouter_reasoning(&value["data"]),
        output_token_limit: limit,
        upstream: (!upstream.is_empty()).then(|| upstream.join(",")),
        ..CatalogEvidence::default()
    })
}

/// Parse the model catalog's reasoning contract. `null` efforts means all
/// gateway values are accepted; an absent efforts field confirms no values.
/// In particular, Ultra's `medium/high` must never be inferred as `low`.
pub fn parse_openrouter_reasoning(model: &Value) -> Option<ReasoningCaps> {
    let reasoning = model.get("reasoning").filter(|v| v.is_object())?;
    let mut values: Vec<String> = match reasoning.get("supported_efforts") {
        Some(Value::Null) => ["none", "minimal", "low", "medium", "high", "xhigh", "max"]
            .map(String::from)
            .to_vec(),
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    };
    let mandatory = reasoning["mandatory"].as_bool().unwrap_or(false);
    if mandatory {
        values.retain(|v| v != "none");
    }
    values.sort();
    values.dedup();
    let efficient = ["minimal", "low", "medium", "high", "xhigh", "max"]
        .into_iter()
        .find(|v| values.iter().any(|value| value == v))
        .map(str::to_string);
    Some(ReasoningCaps {
        control: ReasoningControl::OpenRouter,
        off: (!mandatory && values.iter().any(|v| v == "none")).then(|| "none".into()),
        values,
        efficient,
        mandatory,
        max_tokens: reasoning["supports_max_tokens"].as_bool().unwrap_or(false),
        ..Default::default()
    })
}

/// Parse Google `GET models/{id}`.
pub fn parse_google_model(value: &Value) -> Option<CatalogEvidence> {
    let methods: Vec<String> = value
        .get("supportedGenerationMethods")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    Some(CatalogEvidence {
        generation_methods: methods,
        output_token_limit: value
            .get("outputTokenLimit")
            .and_then(Value::as_u64)
            .map(|v| v as u32),
        thinking: value.get("thinking").and_then(Value::as_bool),
        temperature: value.get("temperature").and_then(Value::as_f64),
        ..CatalogEvidence::default()
    })
}

fn apply_catalog(caps: &mut ResolvedCapabilities, ev: &CatalogEvidence) {
    let has = |p: &str| ev.supported_parameters.iter().any(|x| x == p);
    match caps.adapter {
        AdapterId::OpenRouter if !ev.supported_parameters.is_empty() => {
            let tri = |b: bool| {
                if b {
                    Support::Supported
                } else {
                    Support::Unsupported
                }
            };
            caps.json_mode = tri(has("response_format"));
            caps.strict_schema = tri(has("structured_outputs") && has("response_format"));
            caps.tools = tri(has("tools"));
            caps.forced_tool = tri(has("tools") && has("tool_choice"));
            if has("reasoning") {
                if let Some(reasoning) = &ev.reasoning {
                    caps.reasoning = reasoning.clone();
                } else {
                    // Retain an exact documented profile if one exists.
                    // Never invent effort values from parameter support.
                    caps.reasoning.control = ReasoningControl::OpenRouter;
                }
            } else {
                // Fresh endpoint evidence can also narrow an exact profile.
                caps.reasoning = ReasoningCaps::default();
            }
        }
        AdapterId::Google if !ev.generation_methods.is_empty() => {
            let gen = ev.generation_methods.iter().any(|m| m == "generateContent");
            caps.generate_content = if gen {
                Support::Supported
            } else {
                Support::Unsupported
            };
            if !gen {
                caps.text_generation = Support::Unsupported;
            }
        }
        _ => {}
    }
    if let Some(limit) = ev.output_token_limit {
        caps.max_output_tokens = Some(limit);
    }
    caps.provenance.sources.push("catalog".into());
}

// ---------- Observed rejections ----------

fn observed() -> &'static Mutex<HashMap<String, HashSet<(String, String)>>> {
    static SET: OnceLock<Mutex<HashMap<String, HashSet<(String, String)>>>> = OnceLock::new();
    SET.get_or_init(|| Mutex::new(HashMap::new()))
}

fn route_key(adapter: AdapterId, base_url: &str, model: &str) -> String {
    format!(
        "{}|{}|{}",
        adapter.as_str(),
        crate::provider::normalize_base(base_url).to_ascii_lowercase(),
        normalize_model_key(model)
    )
}

/// Learn that this route rejects `field=value` (`*` = the whole field).
/// Rejecting one value never disables another valid value of the field.
pub fn learn_rejection(adapter: AdapterId, base_url: &str, model: &str, field: &str, value: &str) {
    if let Ok(mut m) = observed().lock() {
        m.entry(route_key(adapter, base_url, model))
            .or_default()
            .insert((field.to_string(), value.to_string()));
    }
}

// ---------- Catalog cache (app_state) ----------

/// Cache key for a connection/model: hash of provider, base URL, dialect,
/// model and upstream.
pub fn cache_key(
    adapter: AdapterId,
    base_url: &str,
    dialect: ApiDialect,
    model: &str,
    upstream: Option<&str>,
) -> String {
    let raw = format!(
        "{}\n{}\n{}\n{}\n{}",
        adapter.as_str(),
        crate::provider::normalize_base(base_url).to_ascii_lowercase(),
        dialect.as_str(),
        normalize_model_key(model),
        upstream.unwrap_or("")
    );
    let digest = Sha256::digest(raw.as_bytes());
    format!(
        "provider_caps:{}",
        digest[..12]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}

/// One cached catalog entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CachedEvidence {
    pub evidence: CatalogEvidence,
    pub fetched_at: i64,
}

/// Catalog freshness policy.
pub const FRESHNESS_SECS: i64 = 24 * 3600;

impl CachedEvidence {
    pub fn is_fresh(&self, now: i64) -> bool {
        now - self.fetched_at < FRESHNESS_SECS
    }
}

fn memory_cache() -> &'static Mutex<BTreeMap<String, CachedEvidence>> {
    static CACHE: OnceLock<Mutex<BTreeMap<String, CachedEvidence>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Read a cached entry (memory first, then `app_state`).
pub fn cached(store: Option<&crate::store::Store>, key: &str) -> Option<CachedEvidence> {
    if let Some(hit) = memory_cache().lock().ok().and_then(|m| m.get(key).cloned()) {
        return Some(hit);
    }
    let raw = store?.app_state_get(key).ok()??;
    let entry: CachedEvidence = serde_json::from_str(&raw).ok()?;
    if let Ok(mut m) = memory_cache().lock() {
        m.insert(key.to_string(), entry.clone());
    }
    Some(entry)
}

/// Replace a cached entry (refreshes only replace, never block a dispatch).
pub fn store_cached(
    store: Option<&crate::store::Store>,
    key: &str,
    entry: &CachedEvidence,
) -> anyhow::Result<()> {
    if let Some(store) = store {
        store.app_state_set(key, &serde_json::to_string(entry)?)?;
    }
    if let Ok(mut m) = memory_cache().lock() {
        m.insert(key.to_string(), entry.clone());
    }
    Ok(())
}

#[cfg(test)]
pub fn forget_memory_cache() {
    if let Ok(mut m) = memory_cache().lock() {
        m.clear();
    }
}

fn in_flight() -> &'static Mutex<HashSet<String>> {
    static SET: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    SET.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Claim the single background refresh slot for `key`. Returns false when a
/// refresh is already running (coalesced) — callers then use what they have.
pub fn begin_refresh(key: &str) -> bool {
    in_flight()
        .lock()
        .map(|mut s| s.insert(key.to_string()))
        .unwrap_or(false)
}

pub fn end_refresh(key: &str) {
    if let Ok(mut s) = in_flight().lock() {
        s.remove(key);
    }
}

/// Fetch catalog evidence for one connection/model. Never called on the
/// dispatch path: only by background refresh.
pub async fn fetch_evidence(
    secret: &crate::secrets::ProviderSecret,
) -> anyhow::Result<Option<CatalogEvidence>> {
    let kind = crate::provider::effective_kind(secret);
    let adapter = AdapterId::for_kind(&kind);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()?;
    let base = crate::provider::normalize_base(&secret.base_url);
    match adapter {
        AdapterId::OpenRouter => {
            let url = format!("{base}/models/{}/endpoints", secret.model.trim());
            let req = crate::provider::authorize(client.get(&url), secret).await?;
            let resp = req.send().await?;
            if !resp.status().is_success() {
                return Ok(None);
            }
            let v: Value = resp.json().await?;
            let Some(mut evidence) = parse_openrouter_endpoints(&secret.model, &v) else {
                return Ok(None);
            };
            // Endpoint parameter metadata does not enumerate valid efforts.
            // Read the model contract only in this background refresh, never
            // on the generation path. Failure retains the exact profile.
            if evidence.reasoning.is_none() {
                let req = crate::provider::authorize(client.get(format!("{base}/models")), secret)
                    .await?;
                if let Ok(resp) = req.send().await {
                    if resp.status().is_success() {
                        if let Ok(catalog) = resp.json::<Value>().await {
                            evidence.reasoning = catalog["data"]
                                .as_array()
                                .and_then(|models| models.iter().find(|m| m["id"] == secret.model))
                                .and_then(parse_openrouter_reasoning);
                        }
                    }
                }
            }
            Ok(Some(evidence))
        }
        AdapterId::Google => {
            let root = crate::provider_adapters::google::native_root(&base);
            let url = format!(
                "{root}/{}",
                crate::provider_adapters::google::resource_id(&secret.model)
            );
            let mut req = client.get(&url);
            if let Some(key) = crate::provider::resolved_key(secret) {
                req = req.header("x-goog-api-key", key);
            }
            let resp = req.send().await?;
            if !resp.status().is_success() {
                return Ok(None);
            }
            let v: Value = resp.json().await?;
            Ok(parse_google_model(&v))
        }
        // Nvidia catalog entries carry no feature metadata; documented
        // profiles and conservative defaults are used instead.
        _ => Ok(None),
    }
}

/// Request one coalesced background refresh when the cached entry is missing
/// or stale. Never waits; a failed refresh leaves the documented/conservative
/// profile in effect and dispatches no probe.
pub fn refresh_in_background(
    home: Option<std::path::PathBuf>,
    secret: crate::secrets::ProviderSecret,
    now: i64,
) {
    let kind = crate::provider::effective_kind(&secret);
    let adapter = AdapterId::for_kind(&kind);
    if !matches!(adapter, AdapterId::OpenRouter | AdapterId::Google) {
        return;
    }
    let dialect = crate::provider_adapters::default_dialect(adapter, &secret.base_url);
    let key = cache_key(adapter, &secret.base_url, dialect, &secret.model, None);
    let open = || {
        home.as_ref()
            .and_then(|h| crate::store::Store::open(h).ok())
    };
    if cached(open().as_ref(), &key).is_some_and(|c| c.is_fresh(now)) {
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    if !begin_refresh(&key) {
        return;
    }
    handle.spawn(async move {
        if let Ok(Some(evidence)) = fetch_evidence(&secret).await {
            let store = home
                .as_ref()
                .and_then(|h| crate::store::Store::open(h).ok());
            let _ = store_cached(
                store.as_ref(),
                &key,
                &CachedEvidence {
                    evidence,
                    fetched_at: now,
                },
            );
        }
        end_refresh(&key);
    });
}

// ---------- Resolution ----------

/// Resolve capabilities for a connection/model. Pure apart from the
/// in-process observed-rejection and catalog caches; never fetches.
pub fn resolve(adapter: AdapterId, base_url: &str, model: &str) -> ResolvedCapabilities {
    let dialect = crate::provider_adapters::default_dialect(adapter, base_url);
    let key = cache_key(adapter, base_url, dialect, model, None);
    let evidence = cached(None, &key).map(|c| (c.evidence, c.fetched_at));
    resolve_with(
        adapter,
        base_url,
        model,
        evidence.as_ref().map(|(e, t)| (e, *t)),
    )
}

/// Resolution with explicit catalog evidence (tests and admission snapshots).
pub fn resolve_with(
    adapter: AdapterId,
    base_url: &str,
    model: &str,
    evidence: Option<(&CatalogEvidence, i64)>,
) -> ResolvedCapabilities {
    let mut caps = ResolvedCapabilities::conservative(adapter, model);
    if let Some(p) = profiles().get(&(
        adapter.profile_provider().to_string(),
        normalize_model_key(model),
    )) {
        apply_profile(&mut caps, p);
    } else if crate::provider::is_decisions_model(model) && adapter == AdapterId::OpenRouter {
        // Jev family without a profile entry: still decision-only.
        caps.decision_only = true;
        caps.text_generation = Support::Unsupported;
    }
    if let Some((ev, at)) = evidence {
        apply_catalog(&mut caps, ev);
        caps.provenance.catalog_fetched_at = Some(at);
    }
    if let Some(set) = observed()
        .lock()
        .ok()
        .and_then(|m| m.get(&route_key(adapter, base_url, model)).cloned())
    {
        if !set.is_empty() {
            caps.rejected.extend(set);
            caps.provenance.sources.push("observed".into());
        }
    }
    caps
}

/// Resolve for a stored connection.
pub fn for_secret(secret: &crate::secrets::ProviderSecret) -> ResolvedCapabilities {
    let kind = crate::provider::effective_kind(secret);
    resolve(AdapterId::for_kind(&kind), &secret.base_url, &secret.model)
}

/// Merge the current observed rejections into an admission snapshot.
/// Rejections only ever narrow what is sent, so a recovery recompile may
/// use them while catalog evidence stays frozen at admission.
pub fn with_observed(
    mut caps: ResolvedCapabilities,
    base_url: &str,
    model: &str,
) -> ResolvedCapabilities {
    if let Some(set) = observed()
        .lock()
        .ok()
        .and_then(|m| m.get(&route_key(caps.adapter, base_url, model)).cloned())
    {
        if !set.is_empty() {
            caps.rejected.extend(set);
            if !caps.provenance.sources.iter().any(|s| s == "observed") {
                caps.provenance.sources.push("observed".into());
            }
        }
    }
    caps
}

static BACKGROUND_REFRESH: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(!cfg!(test));

/// Enable/disable the background catalog refresh (tests keep it off so a
/// mock server only ever sees operation requests).
pub fn set_background_refresh(enabled: bool) {
    BACKGROUND_REFRESH.store(enabled, std::sync::atomic::Ordering::SeqCst);
}

/// Stage admission (spec §1): request a coalesced background refresh when
/// the cache is missing/stale, then resolve one immutable snapshot. The
/// snapshot (and its digest) is what the whole operation compiles from, so a
/// refresh finishing mid-operation can never change an in-flight request.
pub fn admit(
    secret: &crate::secrets::ProviderSecret,
    home: Option<&std::path::Path>,
) -> ResolvedCapabilities {
    if BACKGROUND_REFRESH.load(std::sync::atomic::Ordering::SeqCst) {
        refresh_in_background(
            home.map(std::path::Path::to_path_buf),
            secret.clone(),
            chrono::Utc::now().timestamp(),
        );
    }
    let kind = crate::provider::effective_kind(secret);
    let adapter = AdapterId::for_kind(&kind);
    let dialect = crate::provider_adapters::default_dialect(adapter, &secret.base_url);
    let key = cache_key(adapter, &secret.base_url, dialect, &secret.model, None);
    let store = home.and_then(|h| crate::store::Store::open(h).ok());
    let evidence = cached(store.as_ref(), &key);
    resolve_with(
        adapter,
        &secret.base_url,
        &secret.model,
        evidence.as_ref().map(|c| (&c.evidence, c.fetched_at)),
    )
}

#[cfg(test)]
#[path = "provider_capabilities_tests.rs"]
mod tests;
