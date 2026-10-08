//! Local Dork Generator and DorkSearch PRO template catalog.
//!
//! Provides zero-credit, zero-LLM search query composition using captured
//! DorkSearch PRO templates. Generates typed `SearchQueryArtifact`s consumed
//! by Firecrawl Search.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, RwLock};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use url::Url;

use super::dataset::{self, to_hex, DatasetManifest};

pub const TOOL_ID: &str = "dork_generate";
pub const DATASET_NAME: &str = "dorksearch";
pub const SEED_JSON: &str = include_str!("dork_seed.json");
pub const DEFAULT_MAX_QUERIES: usize = 3;
pub const HARD_MAX_QUERIES: usize = 10;

/// Top-level DorkSearch catalog structure.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DorkCatalog {
    pub schema_version: u32,
    pub source_url: String,
    pub capture_date: String,
    pub license_status: String,
    pub category_count: usize,
    pub template_count: usize,
    pub example_count: usize,
    #[serde(default)]
    pub firecrawl_compatibility: String,
    pub categories: Vec<DorkCategory>,
    #[serde(default)]
    pub page_examples: Vec<DorkPageExample>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DorkCategory {
    pub name: String,
    #[serde(default)]
    pub source_name: String,
    pub items: Vec<DorkTemplate>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DorkTemplate {
    pub id: String,
    pub category: String,
    pub label: String,
    pub query_fragment: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub needs_review: bool,
    #[serde(default)]
    pub sample_input: Option<Value>,
    #[serde(default)]
    pub sample_source_preview: Option<String>,
    #[serde(default)]
    pub sample_status: Option<String>,
    #[serde(default)]
    pub required_operand_sample: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DorkPageExample {
    pub query_fragment: String,
    pub label: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub sample_input: Option<Value>,
    #[serde(default)]
    pub sample_status: Option<String>,
}

/// Compute stable template ID according to spec section 17.5:
/// `dsp-` plus the first 16 lowercase hex characters of SHA-256 over UTF-8
/// `trimmed_category + NUL + label + NUL + original_fragment`.
pub fn compute_template_id(category: &str, label: &str, fragment: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(category.trim().as_bytes());
    hasher.update(b"\0");
    hasher.update(label.as_bytes());
    hasher.update(b"\0");
    hasher.update(fragment.as_bytes());
    let hash = hasher.finalize();
    format!("dsp-{}", &to_hex(&hash)[..16])
}

/// Compute query artifact ID:
/// `dqa-` plus first 16 hex chars of SHA256 of `generated_query`.
pub fn compute_query_id(query: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(query.as_bytes());
    format!("dqa-{}", &to_hex(&hasher.finalize())[..16])
}

/// Query compatibility state for Firecrawl / backend execution.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QueryCompatibility {
    Supported,
    NeedsInput,
    Unverified,
    Unsupported,
}

impl QueryCompatibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::NeedsInput => "needs_input",
            Self::Unverified => "unverified",
            Self::Unsupported => "unsupported",
        }
    }
}

/// Execution lifecycle for a query artifact.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QueryExecutionStatus {
    Generated,
    Selected,
    Queued,
    Running,
    Completed,
    NoResults,
    Failed,
    Cancelled,
    UnexecutedBudgetExhausted,
    UnexecutedBlocked,
}

impl QueryExecutionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Generated => "generated",
            Self::Selected => "selected",
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::NoResults => "no_results",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::UnexecutedBudgetExhausted => "unexecuted_budget_exhausted",
            Self::UnexecutedBlocked => "unexecuted_blocked",
        }
    }
}

/// A generated search query artifact consumed by Firecrawl Search.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchQueryArtifact {
    pub query_id: String,
    pub source_template_id: Option<String>,
    pub generated_query: String,
    pub purpose: String,
    pub subject_scope: Option<String>,
    pub directive_id: Option<String>,
    pub compatibility: QueryCompatibility,
    pub generation_call_id: Option<String>,
    pub snapshot_hash: String,
    pub target_tool: String,
    pub firecrawl_args: Value,
    pub execution_status: QueryExecutionStatus,
}

/// Compiled in-memory snapshot for fast O(1) lookups and deterministic iteration.
#[derive(Clone, Debug)]
pub struct DorkSnapshot {
    pub catalog: DorkCatalog,
    pub templates: Vec<DorkTemplate>,
    pub id_to_index: HashMap<String, usize>,
    pub category_to_indexes: HashMap<String, Vec<usize>>,
    pub snapshot_hash: String,
}

impl DorkSnapshot {
    pub fn new(catalog: DorkCatalog, snapshot_hash: String) -> Self {
        let mut templates = Vec::new();
        let mut id_to_index = HashMap::new();
        let mut category_to_indexes = HashMap::new();

        for cat in &catalog.categories {
            let cat_name = cat.name.trim().to_string();
            for item in &cat.items {
                let idx = templates.len();
                id_to_index.insert(item.id.clone(), idx);
                category_to_indexes
                    .entry(cat_name.clone())
                    .or_insert_with(Vec::new)
                    .push(idx);
                templates.push(item.clone());
            }
        }

        Self {
            catalog,
            templates,
            id_to_index,
            category_to_indexes,
            snapshot_hash,
        }
    }

    pub fn get_template(&self, id: &str) -> Option<&DorkTemplate> {
        self.id_to_index.get(id).map(|&idx| &self.templates[idx])
    }
}

static ACTIVE_SNAPSHOT: LazyLock<RwLock<Option<Arc<DorkSnapshot>>>> =
    LazyLock::new(|| RwLock::new(None));

/// Parse and validate catalog JSON.
pub fn parse_catalog(json_bytes: &[u8]) -> Result<DorkCatalog> {
    let catalog: DorkCatalog =
        serde_json::from_slice(json_bytes).context("invalid DorkSearch catalog JSON format")?;

    let mut seen_ids = HashMap::new();
    let mut total_items = 0usize;

    for cat in &catalog.categories {
        for item in &cat.items {
            total_items += 1;
            let computed = compute_template_id(&item.category, &item.label, &item.query_fragment);
            if item.id != computed {
                return Err(anyhow!(
                    "template ID mismatch for '{}' in '{}': expected {}, got {}",
                    item.label,
                    item.category,
                    computed,
                    item.id
                ));
            }
            if let Some(prev) =
                seen_ids.insert(item.id.clone(), (item.category.clone(), item.label.clone()))
            {
                return Err(anyhow!(
                    "collision detected for template ID {}: '{}' and '{}'",
                    item.id,
                    prev.1,
                    item.label
                ));
            }
        }
    }

    if total_items != catalog.template_count {
        return Err(anyhow!(
            "catalog template count declared {} but contains {}",
            catalog.template_count,
            total_items
        ));
    }

    Ok(catalog)
}

/// Ensure and return the active in-memory snapshot.
/// If not loaded, checks `$ARGOS_HOME/datasets/dorksearch/active.json`.
/// If no active on disk, initializes with embedded seed catalog.
pub fn active_snapshot() -> Arc<DorkSnapshot> {
    {
        let reader = ACTIVE_SNAPSHOT.read().unwrap();
        if let Some(ref snap) = *reader {
            return Arc::clone(snap);
        }
    }

    let mut writer = ACTIVE_SNAPSHOT.write().unwrap();
    if let Some(ref snap) = *writer {
        return Arc::clone(snap);
    }

    // Try reading active from disk
    if let Ok(Some((manifest, bytes))) = dataset::read_active_data(DATASET_NAME, "templates.json") {
        if let Ok(catalog) = parse_catalog(&bytes) {
            let snap = Arc::new(DorkSnapshot::new(catalog, manifest.active_version));
            *writer = Some(Arc::clone(&snap));
            return snap;
        }
    }

    // Fall back to embedded seed catalog and activate it
    let seed_catalog =
        parse_catalog(SEED_JSON.as_bytes()).expect("embedded seed catalog must be valid");
    let sha256_hex = to_hex(&Sha256::digest(SEED_JSON.as_bytes()));
    let now = Utc::now().to_rfc3339();
    let manifest = DatasetManifest {
        dataset: DATASET_NAME.to_string(),
        active_version: sha256_hex.clone(),
        retrieved_at: seed_catalog.capture_date.clone(),
        last_checked: now,
        source_url: seed_catalog.source_url.clone(),
        license_status: seed_catalog.license_status.clone(),
        total_count: seed_catalog.template_count,
        supported_count: seed_catalog.template_count,
        skipped_count: 0,
        schema_version: seed_catalog.schema_version,
    };

    let _ = dataset::store_and_activate(
        DATASET_NAME,
        "templates.json",
        SEED_JSON.as_bytes(),
        manifest,
    );

    let snap = Arc::new(DorkSnapshot::new(seed_catalog, sha256_hex));
    *writer = Some(Arc::clone(&snap));
    snap
}

/// Reload active snapshot from disk or set a new candidate.
pub fn reload_active_snapshot() {
    let mut writer = ACTIVE_SNAPSHOT.write().unwrap();
    *writer = None;
}

/// Input parameters for `dork_generate`.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct GenerateInput {
    pub objective: Option<String>,
    pub query: Option<String>,
    pub subject: Option<String>,
    pub domain: Option<String>,
    pub keywords: Option<String>,
    pub categories: Option<Vec<String>>,
    pub template_ids: Option<Vec<String>>,
    pub max_queries: Option<usize>,
    // Template operands
    pub title: Option<String>,
    pub url_text: Option<String>,
    pub excluded_domain: Option<String>,
    pub url: Option<String>,
}

/// Normalize domain input: extracts host without scheme or trailing slash.
pub fn normalize_domain(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(url) = Url::parse(trimmed) {
        if let Some(host) = url.host_str() {
            return Some(host.to_string());
        }
    }
    if let Ok(url) = Url::parse(&format!("https://{trimmed}")) {
        if let Some(host) = url.host_str() {
            return Some(host.to_string());
        }
    }
    Some(trimmed.trim_matches('/').to_string())
}

/// Top-level OR-branch splitting: splits fragments like `A OR B OR C`
/// into individual alternatives `["A", "B", "C"]`.
/// Avoids splitting within quoted substrings like `"A OR B"`.
pub fn split_or_branches(fragment: &str) -> Vec<String> {
    let mut branches = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let chars: Vec<char> = fragment.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            in_quotes = !in_quotes;
            current.push(c);
            i += 1;
        } else if !in_quotes
            && (i + 3 <= chars.len())
            && chars[i] == ' '
            && chars[i + 1] == 'O'
            && chars[i + 2] == 'R'
            && (i + 3 == chars.len() || chars[i + 3] == ' ')
        {
            let trimmed = current.trim();
            if !trimmed.is_empty() {
                branches.push(trimmed.to_string());
            }
            current.clear();
            i += 3;
            if i < chars.len() && chars[i] == ' ' {
                i += 1;
            }
        } else {
            current.push(c);
            i += 1;
        }
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        branches.push(trimmed.to_string());
    }

    if branches.is_empty() {
        vec![fragment.trim().to_string()]
    } else {
        branches
    }
}

/// Evaluate compatibility state of a query branch.
pub fn evaluate_compatibility(fragment: &str, has_unfilled_operands: bool) -> QueryCompatibility {
    if has_unfilled_operands {
        return QueryCompatibility::NeedsInput;
    }
    if fragment.contains("cache:") || fragment.contains("related:") {
        return QueryCompatibility::Unsupported;
    }
    if fragment.starts_with("filetype:") || fragment.starts_with("site:") {
        return QueryCompatibility::Supported;
    }
    QueryCompatibility::Unverified
}

/// Render a single branch of a template with target scope and operands.
pub fn compose_single_query(
    branch: &str,
    domain: Option<&str>,
    keywords: Option<&str>,
    operands: &GenerateInput,
) -> (String, QueryCompatibility) {
    let mut comp_fragment = branch.to_string();
    let mut needs_input = false;

    // 1. Substitute operands
    if comp_fragment.contains("intitle:\"\"") {
        if let Some(ref title) = operands.title {
            comp_fragment = comp_fragment.replace("intitle:\"\"", &format!("intitle:\"{title}\""));
        } else {
            needs_input = true;
        }
    }

    if comp_fragment.contains("inurl:\"\"") {
        if let Some(ref url_text) = operands.url_text {
            comp_fragment = comp_fragment.replace("inurl:\"\"", &format!("inurl:\"{url_text}\""));
        } else {
            needs_input = true;
        }
    }

    if comp_fragment.contains("-example.com") {
        if let Some(ref excluded) = operands.excluded_domain {
            comp_fragment = comp_fragment.replace("-example.com", &format!("-site:{excluded}"));
        } else {
            needs_input = true;
        }
    }

    if comp_fragment == "cache:" {
        if let Some(ref url) = operands.url {
            comp_fragment = format!("cache:{url}");
        } else {
            needs_input = true;
        }
    }

    if comp_fragment == "related:" {
        if let Some(ref dom) = domain {
            comp_fragment = format!("related:{dom}");
        } else {
            needs_input = true;
        }
    }

    // 2. Compose domain and keywords
    // When template already contains positive `site:`, retain source scope and do not duplicate.
    let has_positive_site = comp_fragment
        .split_whitespace()
        .any(|token| token.starts_with("site:") || token.starts_with("\"site:"));
    let mut parts = Vec::new();
    if let Some(dom) = domain {
        if !has_positive_site {
            parts.push(format!("site:{dom}"));
        }
    }

    parts.push(comp_fragment.clone());

    if let Some(kw) = keywords {
        let kw_trimmed = kw.trim();
        if !kw_trimmed.is_empty() {
            if kw_trimmed.starts_with('"') && kw_trimmed.ends_with('"') {
                parts.push(kw_trimmed.to_string());
            } else {
                parts.push(format!("\"{kw_trimmed}\""));
            }
        }
    }

    let final_query = parts.join(" ");
    let compatibility = evaluate_compatibility(&comp_fragment, needs_input);

    (final_query, compatibility)
}

/// Execute local dork generation from inputs.
pub fn generate(input: GenerateInput) -> Result<Vec<SearchQueryArtifact>> {
    let snapshot = active_snapshot();
    let max_queries = input
        .max_queries
        .unwrap_or(DEFAULT_MAX_QUERIES)
        .clamp(1, HARD_MAX_QUERIES);

    let domain = input.domain.as_deref().and_then(normalize_domain);
    let keywords = input.keywords.as_deref().filter(|s| !s.trim().is_empty());

    // Candidate template selection:
    let selected_templates: Vec<&DorkTemplate> = if let Some(ref ids) = input.template_ids {
        ids.iter()
            .filter_map(|id| snapshot.get_template(id))
            .collect()
    } else if let Some(ref cats) = input.categories {
        cats.iter()
            .flat_map(|cat| {
                snapshot
                    .category_to_indexes
                    .get(cat.trim())
                    .into_iter()
                    .flat_map(|indices| indices.iter().map(|&idx| &snapshot.templates[idx]))
            })
            .collect()
    } else {
        // Default deterministic selection: take first available
        snapshot.templates.iter().collect()
    };

    let mut artifacts = Vec::new();
    let _objective_text = input
        .objective
        .as_deref()
        .or(input.query.as_deref())
        .unwrap_or("OSINT structured search");

    for template in selected_templates {
        let branches = split_or_branches(&template.query_fragment);
        for branch in branches {
            let (query_text, compatibility) =
                compose_single_query(&branch, domain.as_deref(), keywords, &input);
            let query_id = compute_query_id(&query_text);

            let status = if artifacts.len() < max_queries {
                QueryExecutionStatus::Generated
            } else {
                QueryExecutionStatus::UnexecutedBudgetExhausted
            };

            artifacts.push(SearchQueryArtifact {
                query_id,
                source_template_id: Some(template.id.clone()),
                generated_query: query_text.clone(),
                purpose: template.label.clone(),
                subject_scope: domain.clone().or_else(|| keywords.map(String::from)),
                directive_id: None,
                compatibility,
                generation_call_id: None,
                snapshot_hash: snapshot.snapshot_hash.clone(),
                target_tool: "firecrawl_search".into(),
                firecrawl_args: json!({
                    "query": query_text,
                    "limit": 5,
                }),
                execution_status: status,
            });

            if artifacts.len() >= max_queries {
                break;
            }
        }
        if artifacts.len() >= max_queries {
            break;
        }
    }

    if artifacts.is_empty() {
        return Err(anyhow!(
            "no templates matched the requested selection. Available categories: {}",
            snapshot
                .category_to_indexes
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    Ok(artifacts)
}

/// Refresh templates from upstream DorkSearch PRO using a default client.
pub async fn refresh() -> Result<DatasetManifest> {
    let client = reqwest::Client::new();
    refresh_from_upstream(&client).await
}

/// Refresh templates from upstream DorkSearch PRO (fetches `script.js` and `index.html`).
pub async fn refresh_from_upstream(client: &reqwest::Client) -> Result<DatasetManifest> {
    refresh_with_progress_and_client(client, None, |_| {}).await
}

/// Refresh templates from upstream DorkSearch PRO with progress and cancellation support.
pub async fn refresh_with_progress<F>(
    cancel: Option<Arc<AtomicBool>>,
    on_phase: F,
) -> Result<DatasetManifest>
where
    F: FnMut(&str) + Send,
{
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    refresh_with_progress_and_client(&client, cancel, on_phase).await
}

/// Refresh templates from upstream DorkSearch PRO with a client, progress, and cancellation support.
pub async fn refresh_with_progress_and_client<F>(
    client: &reqwest::Client,
    cancel: Option<Arc<AtomicBool>>,
    mut on_phase: F,
) -> Result<DatasetManifest>
where
    F: FnMut(&str) + Send,
{
    on_phase("Acquiring templates refresh lease");
    let lease = dataset::acquire_refresh_lease(DATASET_NAME)
        .ok_or_else(|| anyhow!("refresh for {} is already running", DATASET_NAME))?;

    if cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
        anyhow::bail!("cancelled by user");
    }

    on_phase("Fetching script.js from dorksearch.pro");
    let js_url = "https://dorksearch.pro/script.js";
    let resp = client
        .get(js_url)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .context("failed to fetch dorksearch.pro/script.js")?;

    if cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
        anyhow::bail!("cancelled by user");
    }

    on_phase("Reading response script from dorksearch.pro");
    let js_text = resp
        .text()
        .await
        .context("failed to read response text from script.js")?;

    if cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
        anyhow::bail!("cancelled by user");
    }

    on_phase("Parsing and tokenizing dorksData catalog");
    // Inert extraction of dorksData from JS text
    let catalog = parse_dorks_data_from_script(&js_text)
        .context("failed to parse dorksData from script.js")?;

    let json_bytes = serde_json::to_vec_pretty(&catalog)?;
    let sha256_hex = to_hex(&Sha256::digest(&json_bytes));

    if let Some(active) = dataset::load_active_manifest(DATASET_NAME) {
        if active.active_version == sha256_hex {
            dataset::update_last_checked(DATASET_NAME)?;
            on_phase("Templates already up to date");
            return Ok(active);
        }
    }

    on_phase("Activating templates version in persistent storage");
    let now = Utc::now().to_rfc3339();
    let manifest = DatasetManifest {
        dataset: DATASET_NAME.to_string(),
        active_version: sha256_hex.clone(),
        retrieved_at: now.clone(),
        last_checked: now,
        source_url: js_url.to_string(),
        license_status: "No explicit reuse license established; archival reference.".into(),
        total_count: catalog.template_count,
        supported_count: catalog.template_count,
        skipped_count: 0,
        schema_version: catalog.schema_version,
    };

    let activated =
        dataset::store_and_activate(DATASET_NAME, "templates.json", &json_bytes, manifest)?;
    reload_active_snapshot();
    drop(lease);
    on_phase("Templates activated successfully");
    Ok(activated)
}

/// Import DorkSearch templates from a local JSON file.
pub fn import_from_file(path: &std::path::Path) -> Result<DatasetManifest> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read file {}", path.display()))?;
    let catalog: DorkCatalog =
        serde_json::from_slice(&bytes).context("failed to parse DorkCatalog JSON")?;
    let sha256_hex = to_hex(&Sha256::digest(&bytes));

    let now = Utc::now().to_rfc3339();
    let manifest = DatasetManifest {
        dataset: DATASET_NAME.to_string(),
        active_version: sha256_hex,
        retrieved_at: now.clone(),
        last_checked: now,
        source_url: catalog.source_url.clone(),
        license_status: catalog.license_status.clone(),
        total_count: catalog.template_count,
        supported_count: catalog.template_count,
        skipped_count: 0,
        schema_version: catalog.schema_version,
    };

    let activated = dataset::store_and_activate(DATASET_NAME, "templates.json", &bytes, manifest)?;
    reload_active_snapshot();
    Ok(activated)
}

/// Get current status of DorkSearch dataset.
pub fn status() -> dataset::DatasetStatus {
    dataset::get_status(DATASET_NAME)
}

/// Inert tokenizer to extract `dorksData` JSON array from JavaScript text without `eval`.
pub fn parse_dorks_data_from_script(script_text: &str) -> Result<DorkCatalog> {
    let marker = "const dorksData =";
    let start_idx = script_text
        .find(marker)
        .ok_or_else(|| anyhow!("could not find '{}' in script", marker))?;
    let slice = &script_text[start_idx + marker.len()..];

    // Find the opening bracket `[`
    let open_bracket = slice
        .find('[')
        .ok_or_else(|| anyhow!("could not find '[' after dorksData"))?;
    let array_start = &slice[open_bracket..];

    // Inert bracket matching to extract complete array
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escape = false;
    let mut end_pos = None;

    for (pos, ch) in array_start.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if ch == '\\' && in_string {
            escape = true;
            continue;
        }
        if ch == '"' || ch == '\'' || ch == '`' {
            in_string = !in_string;
            continue;
        }
        if !in_string {
            if ch == '[' {
                depth += 1;
            } else if ch == ']' {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    end_pos = Some(pos + 1);
                    break;
                }
            }
        }
    }

    let end_pos = end_pos.ok_or_else(|| anyhow!("unmatched brackets in dorksData array"))?;
    let raw_array_str = &array_start[..end_pos];

    // Normalize JS object syntax to valid JSON (keys without quotes) if needed
    let parsed_val: Value = serde_json::from_str(raw_array_str)
        .or_else(|_| parse_js_object_array(raw_array_str))
        .context("failed to parse extracted dorksData array as JSON")?;

    let array = parsed_val
        .as_array()
        .ok_or_else(|| anyhow!("dorksData is not an array"))?;

    let mut categories = Vec::new();
    let mut total_templates = 0usize;

    for cat_val in array {
        let cat_name = cat_val["category"]
            .as_str()
            .ok_or_else(|| anyhow!("missing category name"))?;
        let items_val = cat_val["items"]
            .as_array()
            .ok_or_else(|| anyhow!("missing items array in category {}", cat_name))?;

        let mut items = Vec::new();
        for item_val in items_val {
            let label = item_val["label"].as_str().unwrap_or("").to_string();
            let dork = item_val["dork"].as_str().unwrap_or("").to_string();
            let id = compute_template_id(cat_name, &label, &dork);
            items.push(DorkTemplate {
                id,
                category: cat_name.trim().to_string(),
                label,
                query_fragment: dork,
                source: "script.js:dorksData".into(),
                needs_review: false,
                sample_input: None,
                sample_source_preview: None,
                sample_status: None,
                required_operand_sample: None,
            });
        }
        total_templates += items.len();
        categories.push(DorkCategory {
            name: cat_name.trim().to_string(),
            source_name: cat_name.to_string(),
            items,
        });
    }

    Ok(DorkCatalog {
        schema_version: 1,
        source_url: "https://dorksearch.pro/script.js".into(),
        capture_date: Utc::now().to_rfc3339(),
        license_status: "Captured upstream reference".into(),
        category_count: categories.len(),
        template_count: total_templates,
        example_count: 0,
        firecrawl_compatibility: "Not tested; original fragments preserved unchanged.".into(),
        categories,
        page_examples: Vec::new(),
    })
}

/// Fallback JS object-literal to JSON parser (e.g. `{ category: "foo" }` -> `{"category": "foo"}`).
fn parse_js_object_array(raw_js: &str) -> Result<Value> {
    let mut out = String::with_capacity(raw_js.len());
    let mut in_str = false;
    let mut str_char = ' ';
    let mut chars = raw_js.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_str {
            if ch == '\\' {
                out.push(ch);
                if let Some(next_ch) = chars.next() {
                    out.push(next_ch);
                }
            } else if ch == str_char {
                in_str = false;
                out.push('"');
            } else {
                out.push(ch);
            }
        } else if ch == '"' || ch == '\'' || ch == '`' {
            in_str = true;
            str_char = ch;
            out.push('"');
        } else if ch.is_ascii_alphabetic() || ch == '_' {
            // Unquoted identifier key
            let mut ident = String::new();
            ident.push(ch);
            while let Some(&next) = chars.peek() {
                if next.is_ascii_alphanumeric() || next == '_' {
                    ident.push(chars.next().unwrap());
                } else {
                    break;
                }
            }
            // Check if followed by ':'
            let mut peek_spaces = String::new();
            while let Some(&next) = chars.peek() {
                if next.is_whitespace() {
                    peek_spaces.push(chars.next().unwrap());
                } else {
                    break;
                }
            }
            if chars.peek() == Some(&':') {
                out.push('"');
                out.push_str(&ident);
                out.push('"');
                out.push_str(&peek_spaces);
            } else {
                out.push_str(&ident);
                out.push_str(&peek_spaces);
            }
        } else {
            out.push(ch);
        }
    }

    serde_json::from_str(&out).context("failed to parse normalized JS object array as JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_catalog_has_exact_counts_and_valid_ids() {
        let catalog = parse_catalog(SEED_JSON.as_bytes()).expect("seed catalog must parse");
        assert_eq!(catalog.category_count, 13);
        assert_eq!(catalog.template_count, 70);
        assert_eq!(catalog.page_examples.len(), 4);

        for cat in &catalog.categories {
            for item in &cat.items {
                let computed =
                    compute_template_id(&item.category, &item.label, &item.query_fragment);
                assert_eq!(item.id, computed, "ID must match SHA256 formula");
            }
        }
    }

    #[test]
    fn fixture_17_1_minimal_annual_report_query() {
        let input = GenerateInput {
            objective: Some("Find public annual reports for the organization".into()),
            domain: Some("example.org".into()),
            keywords: Some("annual report".into()),
            template_ids: Some(vec!["dsp-b5cf476d021e1c5b".into()]),
            max_queries: Some(1),
            ..Default::default()
        };

        let artifacts = generate(input).expect("generation should succeed");
        assert_eq!(artifacts.len(), 1);
        let art = &artifacts[0];
        assert_eq!(
            art.generated_query,
            "site:example.org filetype:pdf \"annual report\""
        );
        assert_eq!(
            art.firecrawl_args["query"],
            "site:example.org filetype:pdf \"annual report\""
        );
        assert_eq!(art.firecrawl_args["limit"], 5);
        assert_eq!(art.target_tool, "firecrawl_search");
        assert_eq!(
            art.source_template_id.as_deref(),
            Some("dsp-b5cf476d021e1c5b")
        );
        assert_eq!(art.compatibility, QueryCompatibility::Supported);
    }

    #[test]
    fn fixture_17_2_multiple_alternatives_split() {
        let input = GenerateInput {
            domain: Some("example.org".into()),
            keywords: Some("research".into()),
            template_ids: Some(vec!["dsp-3bf8eed6c5caa4c1".into()]), // Excel Data
            max_queries: Some(3),
            ..Default::default()
        };

        let artifacts = generate(input).expect("generation should succeed");
        assert_eq!(artifacts.len(), 3);
        assert_eq!(
            artifacts[0].generated_query,
            "site:example.org filetype:xls \"research\""
        );
        assert_eq!(
            artifacts[1].generated_query,
            "site:example.org filetype:xlsx \"research\""
        );
        assert_eq!(
            artifacts[2].generated_query,
            "site:example.org filetype:csv \"research\""
        );
    }

    #[test]
    fn fixture_17_3_required_operands() {
        // Exact Title: dsp-eb615f625e5dbc09
        let input_title = GenerateInput {
            domain: Some("example.org".into()),
            keywords: Some("research".into()),
            template_ids: Some(vec!["dsp-eb615f625e5dbc09".into()]),
            title: Some("Annual report".into()),
            max_queries: Some(1),
            ..Default::default()
        };
        let art_title = generate(input_title).expect("success");
        assert_eq!(
            art_title[0].generated_query,
            "site:example.org intitle:\"Annual report\" \"research\""
        );

        // Exact URL: dsp-41433eae6ec03fa0
        let input_url = GenerateInput {
            domain: Some("example.org".into()),
            keywords: Some("research".into()),
            template_ids: Some(vec!["dsp-41433eae6ec03fa0".into()]),
            url_text: Some("reports".into()),
            max_queries: Some(1),
            ..Default::default()
        };
        let art_url = generate(input_url).expect("success");
        assert_eq!(
            art_url[0].generated_query,
            "site:example.org inurl:\"reports\" \"research\""
        );

        // Exclude Domain: dsp-a5cf8cbda4948284
        let input_exclude = GenerateInput {
            domain: Some("example.org".into()),
            keywords: Some("research".into()),
            template_ids: Some(vec!["dsp-a5cf8cbda4948284".into()]),
            excluded_domain: Some("example.net".into()),
            max_queries: Some(1),
            ..Default::default()
        };
        let art_exclude = generate(input_exclude).expect("success");
        assert_eq!(
            art_exclude[0].generated_query,
            "site:example.org -site:example.net \"research\""
        );

        // Missing operand triggers NeedsInput
        let input_missing = GenerateInput {
            domain: Some("example.org".into()),
            template_ids: Some(vec!["dsp-eb615f625e5dbc09".into()]),
            ..Default::default()
        };
        let art_missing = generate(input_missing).expect("success");
        assert_eq!(art_missing[0].compatibility, QueryCompatibility::NeedsInput);
    }
}
