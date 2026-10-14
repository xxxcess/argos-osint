//! Stage executors for the Intel brief preparation chain (spec §2/§4).
//!
//! Each stage receives only its permitted analytical inputs, a compact prompt
//! and a per-stage completion ceiling. Models return short source/sentence IDs;
//! Rust validates IDs, field/count limits and duplicates, then resolves UTF-8
//! spans. A lease heartbeat covers the stage from claim through commit and
//! acknowledgement; cancellation or a lost lease drops the in-flight request.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, ensure, Result};
use rusqlite::OptionalExtension;
use serde_json::{json, Value};

use super::brief_pipeline;
use super::clean_body::{stage_one_admissible, CleanArtifact};
use crate::store::{AtlasArticleRow, Store};
use crate::tasks::{self, ClaimedTask};

pub const CLAIMS_PROMPT: &str = "Extract only assertions explicitly stated in TITLE and DESCRIPTION. Preserve speaker, allegation or denial, uncertainty, and time. Produce up to six distinct claims, each at most 35 words. Cite its title/description sentence IDs. Do not add background knowledge, explanations, inferences, article-body claims, or article links. Source assertion does not mean verified fact. Return an empty array when no explicit claim is present.";
pub const INFERENCES_PROMPT: &str = "Use the supplied claims as the questions to examine. Use only CLEAN_ARTICLE_BODY as evidence. Identify zero to five distinct conclusions that reasonably follow from its details; do not restate claims. For each, reference relevant claim IDs and precise body sentence IDs, explain the reasoning briefly, and label support as supported or conditional. Preserve corrections, denials, time, uncertainty, and counterevidence. Conditional conclusions must name the unresolved condition. Do not treat an allegation as established or infer intent, causation, or identity from coincidence. Reject conclusions without body support. Earlier claims are source assertions, not independent corroboration. Return no article links. Prefer an empty array to speculation.";
pub const LINKS_PROMPT: &str = "Select up to five candidate article–inference pairs whose TITLE or DESCRIPTION contains a specific assertion or detail that could corroborate the inference. Shared names, country, or broad topic alone are insufficient. For each, return the candidate ID, inference ID, matching metadata sentence IDs, and a ≤40-word interpretation explaining the overlap and what still requires verification. Titles/descriptions establish a potential lead, not confirmed corroboration. Use only supplied candidates; invent no article, URL, body detail, or evidence. Return an empty array when no plausible match exists.";

const STRICT_JSON: &str = " Return strict JSON only, with explicit arrays (empty arrays are valid). No tables, introductions, Markdown fences, or repeated input text.";
const CLAIMS_SHAPE: &str = r#" Shape: {"claims":[{"id":"c1","text":"","subject_id":"","predicate":"","object":"","attributed_to":"","modality":"asserted|alleged|denied|uncertain","polarity":"positive|negative","time":"","evidence_ids":["t1"]}],"limit_reached":false,"empty_reason":""}"#;
const INFERENCES_SHAPE: &str = r#" Shape: {"inferences":[{"id":"i1","text":"","premise_claim_ids":["c1"],"body_evidence_ids":["b1"],"support":"supported|conditional","why":"","limitations":[""],"counterevidence_ids":[]}],"empty_reason":""}"#;
const LEADS_SHAPE: &str = r#" Shape: {"leads":[{"article_id":"r1","inference_id":"i1","metadata_evidence_ids":["r1.t1"],"why":"","verification_needed":""}],"empty_reason":""}"#;

/// Spec §4 initial completion ceilings.
pub const CLAIMS_TOKENS: u32 = 2048;
pub const INFERENCES_TOKENS: u32 = 3072;
pub const LINKS_TOKENS: u32 = 1536;

pub const MAX_CLAIMS: usize = 6;
pub const MAX_INFERENCES: usize = 5;
pub const MAX_LEADS: usize = 5;
pub const MAX_CANDIDATES: usize = 12;
/// Body bytes supplied in one Inferences request; longer bodies are covered
/// by ordered windows within Stage 3, never truncated.
pub const BODY_WINDOW_BYTES: usize = 48_000;
/// Budget for one *serialized compiled* Stage-3 request (system + user
/// message), spec §2: escaping and sentence-ID overhead count against it.
pub const REQUEST_WINDOW_BYTES: usize = 56_000;
/// Reserved for carried prior-window proposals (≤5 inferences).
pub const CARRIED_RESERVE_BYTES: usize = 6_000;

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub type Validator = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// Immutable execution context of one claimed stage (spec §6). Built once at
/// claim time and passed to every backend call, so parallel branches never
/// reload Defaults or mutable article metadata mid-stage.
#[derive(Clone, Debug, Default)]
pub struct StageContext {
    pub job_id: String,
    pub task_id: String,
    /// Checked brief generation of the owning job.
    pub job_generation: i64,
    /// Lease epoch of this execution (ownership/telemetry, never a cache key).
    pub lease_epoch: i64,
    /// Captured title/description/published/source revision.
    pub metadata_snapshot: Value,
    /// Captured role defaults (JSON) used to route every dispatch.
    pub route_snapshot: String,
}

impl StageContext {
    /// Execution-scoped operation id: `{task_id}:e{epoch}:{key}:{payload_hash}`.
    pub fn operation_id(&self, key: &str, payload: &str) -> String {
        format!(
            "{}:e{}:{key}:{}",
            self.task_id,
            self.lease_epoch,
            crate::nlp::cache::hash(payload)
        )
    }

    /// Bind a scope to this execution: task, epoch and job generation.
    pub fn scope<'a>(
        &'a self,
        scope: crate::recon::model_exec::OperationScope<'a>,
    ) -> crate::recon::model_exec::OperationScope<'a> {
        scope
            .with_task(&self.task_id)
            .with_task_epoch(self.lease_epoch)
            .with_generation(u32::try_from(self.job_generation.max(0)).unwrap_or(u32::MAX))
    }

    /// Captured defaults applied over `settings` (credentials stay in the
    /// account store; only routing comes from the snapshot).
    pub fn routed(
        &self,
        settings: &crate::provider::SettingsFile,
    ) -> crate::provider::SettingsFile {
        let mut routed = settings.clone();
        if let Ok(defaults) =
            serde_json::from_str::<crate::provider::RoleDefaults>(&self.route_snapshot)
        {
            routed.defaults = defaults;
        }
        routed
    }

    /// Resolved route of one operation in the captured snapshot, as a
    /// receipt component (route only, never the whole Defaults).
    pub fn route_key(&self, operation: Option<&str>, role: &str) -> String {
        let defaults: crate::provider::RoleDefaults =
            serde_json::from_str(&self.route_snapshot).unwrap_or_default();
        crate::nlp::cache::hash(
            &serde_json::to_string(&defaults.resolve_operation(operation, role).0)
                .unwrap_or_default(),
        )
    }
}

/// One model request for a stage.
pub struct StageRequest {
    pub ctx: StageContext,
    pub task_id: String,
    pub stage: String,
    pub role: &'static str,
    pub system: String,
    pub user: String,
    pub completion_tokens: u32,
    /// Stage reasoning effort (spec §2: extraction/linking low, inference default).
    pub reasoning: crate::request_policy::ReasoningEffort,
    /// Operation key for `defaults.operation_overrides`.
    pub operation_key: &'static str,
    pub validator: Validator,
    /// Stage budget-table row (spec §5).
    pub budget: crate::request_policy::StageBudget,
    /// Authoritative output schema (strict schema / output tool / JSON mode).
    pub contract: Option<crate::provider_adapters::ResponseContract>,
}

/// Provider/retrieval boundary for stage execution (runtime or test double).
pub trait StageBackend: Send {
    /// Stage 1: retrieve (or reuse) and clean the article body.
    fn prepare_body<'a>(
        &'a mut self,
        ctx: &'a StageContext,
        article: &'a AtlasArticleRow,
        force: bool,
        cancel: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<CleanArtifact>>;
    /// Batched Claim-assessor fidelity decisions (native Jev or constrained chat).
    fn assess<'a>(
        &'a mut self,
        ctx: &'a StageContext,
        items: Vec<super::brief_fidelity::FidelityItem>,
        cancel: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<HashMap<String, String>>>;
    /// Existing related articles for Stage 4 (metadata only).
    fn related(&mut self, article: &AtlasArticleRow) -> Result<Vec<AtlasArticleRow>>;
    /// Measured focused oversized-story path enabled (default off).
    fn focused_oversized(&self) -> bool {
        false
    }
    /// Generated JSON request; must honour `cancel`.
    fn generate<'a>(
        &'a mut self,
        request: StageRequest,
        cancel: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<String>>;
}

#[derive(Clone, Debug)]
pub struct ExecConfig {
    pub lease_secs: i64,
    pub heartbeat: Duration,
}

impl Default for ExecConfig {
    fn default() -> Self {
        Self {
            lease_secs: 120,
            heartbeat: Duration::from_secs(30),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StageOutcome {
    Committed {
        operation: String,
        items: usize,
    },
    Failed {
        operation: String,
        reason: String,
    },
    /// Lease lost or chain cancelled/superseded: nothing was written.
    Stale {
        operation: String,
    },
}

// ---------- sentence identities ----------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sentence {
    pub id: String,
    pub text: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// Split text into sentences with absolute UTF-8 byte offsets.
pub fn sentences(prefix: &str, text: &str) -> Vec<Sentence> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let bytes: Vec<(usize, char)> = text.char_indices().collect();
    let push = |s: usize, e: usize, out: &mut Vec<Sentence>| {
        let slice = &text[s..e];
        let lead = slice.len() - slice.trim_start().len();
        let trimmed = slice.trim();
        if !trimmed.is_empty() {
            let s = s + lead;
            out.push(Sentence {
                id: format!("{prefix}{}", out.len() + 1),
                text: trimmed.to_string(),
                start_byte: s,
                end_byte: s + trimmed.len(),
            });
        }
    };
    for (i, (b, ch)) in bytes.iter().enumerate() {
        let next = bytes.get(i + 1).map(|(_, c)| *c);
        let boundary = (matches!(ch, '.' | '!' | '?' | '。')
            && next.is_none_or(char::is_whitespace))
            || (*ch == '\n' && next == Some('\n'));
        if boundary {
            let end = b + ch.len_utf8();
            push(start, end, &mut out);
            start = end;
        }
    }
    push(start, text.len(), &mut out);
    out
}

fn words(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Strict JSON: no fences or prose around the object.
pub fn strict_json(raw: &str) -> Result<Value> {
    let trimmed = raw.trim();
    ensure!(
        !trimmed.starts_with("```"),
        "markdown fences are not allowed"
    );
    ensure!(trimmed.starts_with('{'), "response must be a JSON object");
    Ok(serde_json::from_str(trimmed)?)
}

fn str_field<'a>(item: &'a Value, key: &str) -> &'a str {
    item.get(key).and_then(Value::as_str).unwrap_or("")
}

fn ids(item: &Value, key: &str) -> Result<Vec<String>> {
    match item.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| anyhow!("{key} must contain string IDs"))
            })
            .collect(),
        _ => bail!("{key} must be an array"),
    }
}

fn spans(ids: &[String], index: &HashMap<&str, &Sentence>) -> Value {
    Value::Array(
        ids.iter()
            .filter_map(|id| index.get(id.as_str()))
            .map(|s| json!({"id":s.id,"start_byte":s.start_byte,"end_byte":s.end_byte}))
            .collect(),
    )
}

fn norm(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn empty_reason(value: &Value) -> Result<String> {
    let reason = str_field(value, "empty_reason").trim().to_string();
    ensure!(
        words(&reason) <= super::brief_contracts::EMPTY_REASON_WORDS,
        "empty_reason exceeds 25 words"
    );
    Ok(reason)
}

// ---------- Stage 2: Claims ----------

pub fn metadata_sentences(article: &AtlasArticleRow) -> Vec<Sentence> {
    super::sentence_index::metadata_wire(&article.title, &article.description)
}

/// Deterministic ID order for schema enums (stable request bodies).
fn sorted_ids(ids: &HashSet<String>) -> Vec<&str> {
    let mut out: Vec<&str> = ids.iter().map(String::as_str).collect();
    out.sort_unstable();
    out
}

pub fn claims_user(article: &AtlasArticleRow) -> String {
    let meta = metadata_sentences(article);
    let pick = |p: &str| -> Vec<Value> {
        meta.iter()
            .filter(|s| s.id.starts_with(p))
            .map(|s| json!({"id":s.id,"text":s.text}))
            .collect()
    };
    json!({"TITLE":pick("t"),"DESCRIPTION":pick("d")}).to_string()
}

pub fn validate_claims(raw: &str, meta: &[Sentence]) -> Result<Value> {
    let value = strict_json(raw)?;
    let items = value
        .get("claims")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("claims must be an array"))?;
    ensure!(items.len() <= MAX_CLAIMS, "more than six claims");
    let index: HashMap<&str, &Sentence> = meta.iter().map(|s| (s.id.as_str(), s)).collect();
    let mut seen = HashSet::new();
    let mut ids_seen = HashSet::new();
    let mut out = Vec::new();
    for (n, item) in items.iter().enumerate() {
        let text = str_field(item, "text").trim();
        ensure!(!text.is_empty(), "claim text missing");
        ensure!(
            words(text) <= super::brief_contracts::CLAIM_TEXT_WORDS,
            "claim exceeds 35 words"
        );
        let evidence = ids(item, "evidence_ids")?;
        ensure!(
            !evidence.is_empty(),
            "claim cites no title/description sentence"
        );
        for id in &evidence {
            ensure!(
                index.contains_key(id.as_str()),
                "unknown metadata sentence id {id}"
            );
        }
        ensure!(seen.insert(norm(text)), "duplicate claim");
        let id = match str_field(item, "id") {
            "" => format!("c{}", n + 1),
            id => id.to_string(),
        };
        ensure!(ids_seen.insert(id.clone()), "duplicate claim id");
        let mut claim = item.clone();
        claim["id"] = json!(id);
        claim["evidence"] = spans(&evidence, &index);
        out.push(claim);
    }
    let reason = empty_reason(&value)?;
    Ok(json!({
        "claims": out,
        "limit_reached": value.get("limit_reached").and_then(Value::as_bool).unwrap_or(false),
        "empty_reason": if out.is_empty() && reason.is_empty() { "no explicit claim in title or description".into() } else { reason },
    }))
}

// ---------- Stage 3: Inferences ----------

pub fn inferences_user(claims: &Value, body: &[Sentence], carried: &Value) -> String {
    let claims: Vec<Value> = claims
        .get("claims")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| json!({"id":c["id"],"text":c["text"],"attributed_to":c.get("attributed_to"),"modality":c.get("modality")}))
        .collect();
    let mut user = json!({
        "CLAIMS": claims,
        "CLEAN_ARTICLE_BODY": body.iter().map(|s| json!({"id":s.id,"text":s.text})).collect::<Vec<_>>(),
    });
    if carried.as_array().is_some_and(|a| !a.is_empty()) {
        user["PRIOR_WINDOW_PROPOSALS"] = carried.clone();
        user["NOTE"] = json!(
            "Revise, keep or reject prior proposals; return the full revised set (at most five)."
        );
    }
    user.to_string()
}

pub fn validate_inferences(
    raw: &str,
    claim_ids: &HashSet<String>,
    body: &[Sentence],
    prior: &[Sentence],
) -> Result<Value> {
    let value = strict_json(raw)?;
    ensure!(
        value.get("links").is_none() && value.get("leads").is_none(),
        "inference stage must not return article links"
    );
    let items = value
        .get("inferences")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("inferences must be an array"))?;
    ensure!(items.len() <= MAX_INFERENCES, "more than five inferences");
    let index: HashMap<&str, &Sentence> = body
        .iter()
        .chain(prior)
        .map(|s| (s.id.as_str(), s))
        .collect();
    let mut seen = HashSet::new();
    let mut ids_seen = HashSet::new();
    let mut out = Vec::new();
    for (n, item) in items.iter().enumerate() {
        let text = str_field(item, "text").trim();
        ensure!(
            !text.is_empty() && words(text) <= super::brief_contracts::INFERENCE_TEXT_WORDS,
            "inference text missing or over 35 words"
        );
        let why = str_field(item, "why").trim();
        ensure!(
            !why.is_empty() && words(why) <= super::brief_contracts::INFERENCE_WHY_WORDS,
            "reasoning missing or over 45 words"
        );
        let support = str_field(item, "support");
        ensure!(
            matches!(support, "supported" | "conditional"),
            "support must be supported or conditional"
        );
        let limitations: Vec<String> = ids(item, "limitations")?;
        ensure!(
            limitations.len() <= super::brief_contracts::MAX_LIMITATIONS,
            "at most two limitations"
        );
        ensure!(
            limitations
                .iter()
                .all(|l| words(l) <= super::brief_contracts::LIMITATION_WORDS),
            "limitation over 20 words"
        );
        ensure!(
            support != "conditional" || limitations.iter().any(|l| !l.trim().is_empty()),
            "conditional inference must name its unresolved condition"
        );
        let body_ids = ids(item, "body_evidence_ids")?;
        ensure!(!body_ids.is_empty(), "inference lacks body support");
        let counter = ids(item, "counterevidence_ids")?;
        for id in body_ids.iter().chain(&counter) {
            ensure!(
                index.contains_key(id.as_str()),
                "unknown body sentence id {id}"
            );
        }
        let premises = ids(item, "premise_claim_ids")?;
        for id in &premises {
            ensure!(claim_ids.contains(id), "unknown claim id {id}");
        }
        ensure!(seen.insert(norm(text)), "duplicate inference");
        let id = match str_field(item, "id") {
            "" => format!("i{}", n + 1),
            id => id.to_string(),
        };
        ensure!(ids_seen.insert(id.clone()), "duplicate inference id");
        let mut inference = item.clone();
        inference["id"] = json!(id);
        inference["evidence"] = spans(&body_ids, &index);
        inference["counterevidence"] = spans(&counter, &index);
        out.push(inference);
    }
    let reason = empty_reason(&value)?;
    Ok(json!({
        "inferences": out,
        "empty_reason": if out.is_empty() && reason.is_empty() { "no supported inferences".into() } else { reason },
    }))
}

/// Ordered windows of whole sentences, each within [`BODY_WINDOW_BYTES`].
pub fn body_windows(body: &[Sentence], window_bytes: usize) -> Vec<Vec<Sentence>> {
    let mut windows: Vec<Vec<Sentence>> = Vec::new();
    let mut current = Vec::new();
    let mut size = 0;
    for s in body {
        if size + s.text.len() > window_bytes && !current.is_empty() {
            windows.push(std::mem::take(&mut current));
            size = 0;
        }
        size += s.text.len() + 1;
        current.push(s.clone());
    }
    if !current.is_empty() || windows.is_empty() {
        windows.push(current);
    }
    windows
}

/// Serialized size of one body sentence as it appears in the request.
fn wire_cost(s: &Sentence) -> usize {
    json!({"id": s.id, "text": s.text}).to_string().len() + 1
}

/// Ordered windows sized against the serialized compiled request: each
/// window's `system` + `inferences_user(claims, window, carried)` stays within
/// `budget` when carried proposals fit [`CARRIED_RESERVE_BYTES`]. A single
/// sentence larger than the remaining budget still gets its own window (never
/// truncated).
pub fn request_windows(
    body: &[Sentence],
    system: &str,
    claims: &Value,
    budget: usize,
) -> Vec<Vec<Sentence>> {
    let fixed =
        system.len() + inferences_user(claims, &[], &json!([])).len() + CARRIED_RESERVE_BYTES;
    let body_budget = budget.saturating_sub(fixed).max(2_000);
    let mut windows: Vec<Vec<Sentence>> = Vec::new();
    let mut current = Vec::new();
    let mut size = 0;
    for s in body {
        let cost = wire_cost(s);
        if size + cost > body_budget && !current.is_empty() {
            windows.push(std::mem::take(&mut current));
            size = 0;
        }
        size += cost;
        current.push(s.clone());
    }
    if !current.is_empty() || windows.is_empty() {
        windows.push(current);
    }
    windows
}

// ---------- Stage 4: Links ----------

#[derive(Clone, Debug)]
pub struct Candidate {
    pub id: String,
    pub article: AtlasArticleRow,
    pub sentences: Vec<Sentence>,
}

fn canonical_url(url: &str) -> String {
    let url = url.split(['?', '#']).next().unwrap_or(url);
    url.trim_end_matches('/')
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.")
        .to_lowercase()
}

/// Local shortlist: exclude the source, dedupe canonical URLs and syndicated
/// copies (same normalized title), cap at [`MAX_CANDIDATES`].
pub fn shortlist(source: &AtlasArticleRow, related: Vec<AtlasArticleRow>) -> Vec<Candidate> {
    let mut urls = HashSet::from([canonical_url(&source.url)]);
    let mut titles = HashSet::from([norm(&source.title)]);
    let mut out = Vec::new();
    for article in related {
        if article.id == source.id
            || !urls.insert(canonical_url(&article.url))
            || !titles.insert(norm(&article.title))
        {
            continue;
        }
        let id = format!("r{}", out.len() + 1);
        let mut sentences_ = sentences(&format!("{id}.t"), &article.title);
        sentences_.extend(sentences(&format!("{id}.d"), &article.description));
        out.push(Candidate {
            id,
            article,
            sentences: sentences_,
        });
        if out.len() == MAX_CANDIDATES {
            break;
        }
    }
    out
}

pub fn links_user(inferences: &Value, candidates: &[Candidate]) -> String {
    json!({
        "INFERENCES": inferences.get("inferences").and_then(Value::as_array).into_iter().flatten().map(|i| json!({"id":i["id"],"text":i["text"]})).collect::<Vec<_>>(),
        "CANDIDATES": candidates.iter().map(|c| json!({
            "id": c.id,
            "source": c.article.source_name,
            "published_at": c.article.published_at,
            "metadata": c.sentences.iter().map(|s| json!({"id":s.id,"text":s.text})).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
    .to_string()
}

pub fn validate_leads(
    raw: &str,
    inference_ids: &HashSet<String>,
    candidates: &[Candidate],
) -> Result<Value> {
    let value = strict_json(raw)?;
    let items = value
        .get("leads")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("leads must be an array"))?;
    ensure!(items.len() <= MAX_LEADS, "more than five leads");
    let by_id: HashMap<&str, &Candidate> = candidates.iter().map(|c| (c.id.as_str(), c)).collect();
    let mut pairs = HashSet::new();
    let mut out = Vec::new();
    for item in items {
        let candidate = by_id
            .get(str_field(item, "article_id"))
            .ok_or_else(|| anyhow!("unknown candidate id"))?;
        let inference = str_field(item, "inference_id");
        ensure!(
            inference_ids.contains(inference),
            "unknown inference id {inference}"
        );
        let why = str_field(item, "why").trim();
        ensure!(
            !why.is_empty() && words(why) <= super::brief_contracts::LEAD_WHY_WORDS,
            "interpretation missing or over 40 words"
        );
        let evidence = ids(item, "metadata_evidence_ids")?;
        ensure!(!evidence.is_empty(), "lead cites no candidate metadata");
        let index: HashMap<&str, &Sentence> = candidate
            .sentences
            .iter()
            .map(|s| (s.id.as_str(), s))
            .collect();
        for id in &evidence {
            ensure!(
                index.contains_key(id.as_str()),
                "metadata id {id} not from candidate"
            );
        }
        ensure!(
            pairs.insert((candidate.id.clone(), inference.to_string())),
            "duplicate lead"
        );
        out.push(json!({
            "article_id": candidate.article.id,
            "article_run_id": candidate.article.run_id,
            "metadata_revision": crate::nlp::cache::hash(&format!("{}\n{}", candidate.article.title, candidate.article.description)),
            "inference_id": inference,
            "why": why,
            "verification_needed": str_field(item, "verification_needed"),
            "metadata_evidence": spans(&evidence, &index),
        }));
    }
    let reason = empty_reason(&value)?;
    Ok(json!({
        "leads": out,
        "empty_reason": if out.is_empty() && reason.is_empty() { "no matching articles".into() } else { reason },
    }))
}

/// Content terms for lead overlap: lower-case words of 4+ characters that are
/// neither stop words nor country names (geography alone is not an overlap).
fn content_terms(text: &str) -> HashSet<String> {
    const STOP: &[&str] = &[
        "that", "this", "with", "from", "have", "after", "before", "said", "says", "will", "would",
        "could", "their", "there", "they", "were", "been", "being", "into", "over", "under",
        "about", "also", "more", "than", "when", "which", "while", "only", "other", "report",
        "reports", "news", "week", "year", "today",
    ];
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 4)
        .filter(|w| crate::iso3166::code_for_name(w).is_none())
        .map(str::to_lowercase)
        .filter(|w| !STOP.contains(&w.as_str()))
        .collect()
}

/// Local semantic gate after format validation (§8.7): a lead must share an
/// inference-specific term with the metadata it cites (shared country alone
/// yields no lead) and must name its verification gap. Dropped leads are
/// counted, never silently promoted.
pub fn filter_leads(
    out: &mut Value,
    inference_texts: &HashMap<String, String>,
    candidates: &[Candidate],
) {
    let leads = out["leads"].as_array().cloned().unwrap_or_default();
    let mut kept = Vec::new();
    for lead in leads {
        let inference = inference_texts
            .get(lead["inference_id"].as_str().unwrap_or(""))
            .map(|t| content_terms(t))
            .unwrap_or_default();
        let cited: String = candidates
            .iter()
            .find(|c| lead["article_id"] == c.article.id.as_str())
            .map(|c| {
                lead["metadata_evidence"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| c.sentences.iter().find(|s| e["id"] == s.id.as_str()))
                    .map(|s| s.text.clone())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        let overlap = !inference.is_disjoint(&content_terms(&cited));
        let gap = !lead["verification_needed"]
            .as_str()
            .unwrap_or("")
            .trim()
            .is_empty();
        if overlap && gap {
            kept.push(lead);
        }
    }
    let dropped = out["leads"].as_array().map_or(0, Vec::len) - kept.len();
    if kept.is_empty() && dropped > 0 {
        out["empty_reason"] = json!("no inference-specific overlap");
    }
    out["leads"] = Value::Array(kept);
    out["withheld"] = json!(dropped);
}

// ---------- executor ----------

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn article_for(store: &Store, article_id: &str) -> Result<AtlasArticleRow> {
    let run: Option<String> = store
        .conn
        .query_row(
            "SELECT run_id FROM atlas_articles WHERE article_id=?1 ORDER BY seen_at DESC LIMIT 1",
            [article_id],
            |r| r.get(0),
        )
        .optional()?;
    let run = run.ok_or_else(|| anyhow!("article {article_id} no longer exists"))?;
    store
        .atlas_article(&run, article_id)?
        .ok_or_else(|| anyhow!("article {article_id} no longer exists"))
}

fn stage_output(store: &Store, job_id: &str, operation: &str) -> Result<Value> {
    let raw: String = store
        .conn
        .query_row(
            "SELECT output_json FROM intel_brief_stage_outputs WHERE task_id=?1",
            [brief_pipeline::task_id(job_id, operation)],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| anyhow!("{operation} output missing"))?;
    Ok(serde_json::from_str(&raw)?)
}

fn article_id_of(claimed: &ClaimedTask, store: &Store) -> Result<String> {
    store
        .conn
        .query_row(
            "SELECT article_id FROM intel_brief_runs WHERE job_id=?1",
            [&claimed.job_id],
            |r| r.get(0),
        )
        .map_err(Into::into)
}

/// Await until `flag` is set.
async fn cancelled(flag: Arc<AtomicBool>) {
    while !flag.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn activity(db_path: &Path, task: &str, label: &str) {
    if let Ok(store) = Store::open_light(db_path) {
        let _ = brief_pipeline::set_activity(&store.conn, task, label);
    }
}

async fn generate(
    db_path: &Path,
    backend: &mut dyn StageBackend,
    cancel: &Arc<AtomicBool>,
    request: StageRequest,
) -> Result<Value> {
    let validator = request.validator.clone();
    let task = request.task_id.clone();
    activity(db_path, &task, "waiting for model");
    let raw = super::brief_timing::timed(
        super::brief_timing::Phase::Model,
        backend.generate(request, cancel),
    )
    .await?;
    activity(db_path, &task, "validating");
    validator(&raw).map_err(|e| anyhow!("invalid stage output: {e}"))?;
    strict_json(&raw)
}

/// Run one stage: Ok(value, items, empty_reason).
/// Build the immutable context for a claimed stage on its own connection.
pub fn stage_context(store: &Store, claimed: &ClaimedTask) -> Result<StageContext> {
    let (job_generation, route_snapshot): (i64, String) = store.conn.query_row(
        "SELECT r.generation, t.role_snapshot FROM intel_brief_runs r JOIN argos_tasks t ON t.job_id=r.job_id
         WHERE r.job_id=?1 AND t.id=?2",
        [&claimed.job_id, &claimed.id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    // Roots carry the captured metadata; descendants read the Claims root's.
    let own: Value = serde_json::from_str(&claimed.input_ref).unwrap_or(Value::Null);
    let metadata_snapshot = if own["metadata"].is_object() {
        own["metadata"].clone()
    } else {
        let raw: Option<String> = store
            .conn
            .query_row(
                "SELECT input_ref FROM argos_tasks WHERE id=?1",
                [brief_pipeline::task_id(
                    &claimed.job_id,
                    "intel_brief_claims",
                )],
                |r| r.get(0),
            )
            .optional()?;
        raw.and_then(|r| serde_json::from_str::<Value>(&r).ok())
            .map(|v| v["metadata"].clone())
            .unwrap_or(Value::Null)
    };
    Ok(StageContext {
        job_id: claimed.job_id.clone(),
        task_id: claimed.id.clone(),
        job_generation,
        lease_epoch: claimed.epoch,
        metadata_snapshot,
        route_snapshot,
    })
}

/// The article row with its title/description replaced by the captured
/// snapshot, so a mid-chain metadata edit never reaches a running branch.
fn snapshot_article(mut article: AtlasArticleRow, ctx: &StageContext) -> AtlasArticleRow {
    let m = &ctx.metadata_snapshot;
    if let Some(t) = m["title"].as_str() {
        article.title = t.to_string();
    }
    if let Some(d) = m["description"].as_str() {
        article.description = d.to_string();
    }
    if let Some(p) = m["published_at"].as_str() {
        article.published_at = p.to_string();
    }
    article
}

async fn execute(
    db_path: &Path,
    claimed: &ClaimedTask,
    backend: &mut dyn StageBackend,
    cancel: &Arc<AtomicBool>,
) -> Result<(Value, usize, String)> {
    let (article, job, ctx) = {
        let store = Store::open(db_path)?;
        let ctx = stage_context(&store, claimed)?;
        (
            snapshot_article(article_for(&store, &article_id_of(claimed, &store)?)?, &ctx),
            claimed.job_id.clone(),
            ctx,
        )
    };
    match claimed.operation.as_str() {
        "intel_brief_body" => {
            let force = serde_json::from_str::<Value>(&claimed.input_ref)
                .ok()
                .and_then(|v| v["force_body"].as_bool())
                .unwrap_or(false);
            activity(db_path, &claimed.id, "retrieving / cleaning");
            let artifact = backend.prepare_body(&ctx, &article, force, cancel).await?;
            activity(db_path, &claimed.id, "validating");
            stage_one_admissible(&artifact, artifact.acquisition_complete)?;
            // Row: "Completed · validated clean body" (plus any diagnostics).
            let mut reason = String::from("validated clean body");
            if !artifact.diagnostics.is_empty() {
                reason.push_str(&format!(" · {}", artifact.diagnostics.join("; ")));
            }
            Ok((serde_json::to_value(&artifact)?, 0, reason))
        }
        "intel_brief_claims" => {
            let meta = metadata_sentences(&article);
            if meta.is_empty() {
                return Ok((
                    json!({"claims":[],"empty_reason":"no title or description"}),
                    0,
                    "no title or description".into(),
                ));
            }
            let meta_v = meta.clone();
            let validator: Validator = Arc::new(move |raw| {
                validate_claims(raw, &meta_v)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            });
            let raw = generate(
                db_path,
                backend,
                cancel,
                StageRequest {
                    ctx: ctx.clone(),
                    task_id: claimed.id.clone(),
                    stage: claimed.operation.clone(),
                    role: "claim_assessor",
                    system: format!(
                        "{CLAIMS_PROMPT}{}{STRICT_JSON}{CLAIMS_SHAPE}",
                        super::brief_contracts::claims_limits()
                    ),
                    user: claims_user(&article),
                    completion_tokens: CLAIMS_TOKENS,
                    reasoning: crate::request_policy::CLAIMS_BUDGET.reasoning,
                    operation_key: crate::provider::OP_CLAIMS_GENERATE,
                    validator,
                    budget: crate::request_policy::CLAIMS_BUDGET,
                    contract: Some(super::brief_contracts::claims_contract(
                        &meta.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
                    )),
                },
            )
            .await?;
            let mut out = validate_claims(&raw.to_string(), &meta)?;
            let index: HashMap<&str, &Sentence> = meta.iter().map(|s| (s.id.as_str(), s)).collect();
            apply_fidelity(
                db_path,
                backend,
                &ctx,
                &ctx.route_key(None, "evidence_curator"),
                cancel,
                &mut out,
                "claims",
                "claim",
                "evidence_ids",
                &index,
                "no faithful claims in title or description",
            )
            .await?;
            let n = out["claims"].as_array().map_or(0, Vec::len);
            let reason = if n == 0 {
                out["empty_reason"].as_str().unwrap_or("").to_string()
            } else {
                String::new()
            };
            Ok((out, n, reason))
        }
        "intel_brief_inferences" => {
            let (body, claims) = {
                let store = Store::open(db_path)?;
                (
                    stage_output(&store, &job, "intel_brief_body")?,
                    stage_output(&store, &job, "intel_brief_claims")?,
                )
            };
            let (all, index_full) = super::sentence_index::body_sentences(&body);
            let claim_ids: HashSet<String> = claims["claims"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c["id"].as_str().map(str::to_string))
                .collect();
            let system = format!(
                "{INFERENCES_PROMPT}{}{STRICT_JSON}{INFERENCES_SHAPE}",
                super::brief_contracts::inferences_limits()
            );
            let mut windows = request_windows(&all, &system, &claims, REQUEST_WINDOW_BYTES);
            // Oversized stories may read actor-focused packets (measured path,
            // opt-in). Fidelity below still reviews against the whole body.
            let mut focused: Option<super::passage_packets::PacketSet> = None;
            if windows.len() > 1 && backend.focused_oversized() {
                let actors: Vec<String> = {
                    let store = Store::open(db_path)?;
                    let article = article_for(&store, &article_id_of(claimed, &store)?)?;
                    store
                        .atlas_metadata(&article.run_id, &article.id)?
                        .map(|m| {
                            crate::atlas_metadata::article_actors(&article, &m)
                                .into_iter()
                                .map(|a| a.display_name)
                                .collect()
                        })
                        .unwrap_or_default()
                };
                let markdown = body["markdown"].as_str().unwrap_or("");
                let query = super::passage_packets::claims_query(&actors, &claims);
                if let Some((selected, set)) =
                    super::passage_packets::focused_sentences(&index_full, markdown, &query)
                {
                    let packed = request_windows(&selected, &system, &claims, REQUEST_WINDOW_BYTES);
                    if packed.len() < windows.len() {
                        windows = packed;
                        focused = Some(set);
                    }
                }
            }
            let mut carried = json!([]);
            let mut covered: Vec<Sentence> = Vec::new();
            let mut last = json!({"inferences":[],"empty_reason":"no supported inferences"});
            for window in &windows {
                let (cid, w, prior) = (claim_ids.clone(), window.clone(), covered.clone());
                let validator: Validator = Arc::new(move |raw| {
                    validate_inferences(raw, &cid, &w, &prior)
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                });
                let raw = generate(
                    db_path,
                    backend,
                    cancel,
                    StageRequest {
                        ctx: ctx.clone(),
                        task_id: claimed.id.clone(),
                        stage: claimed.operation.clone(),
                        role: "synthesis",
                        system: system.clone(),
                        user: inferences_user(&claims, window, &carried),
                        completion_tokens: INFERENCES_TOKENS,
                        reasoning: crate::request_policy::INFERENCES_BUDGET.reasoning,
                        operation_key: crate::provider::OP_INFERENCES_GENERATE,
                        validator,
                        budget: crate::request_policy::INFERENCES_BUDGET,
                        contract: Some(super::brief_contracts::inferences_contract(
                            &sorted_ids(&claim_ids),
                            &window.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
                        )),
                    },
                )
                .await?;
                last = validate_inferences(&raw.to_string(), &claim_ids, window, &covered)?;
                covered.extend(window.iter().cloned());
                carried = json!(last["inferences"].as_array().into_iter().flatten().map(|i| json!({"id":i["id"],"text":i["text"],"body_evidence_ids":i["body_evidence_ids"],"support":i["support"]})).collect::<Vec<_>>());
            }
            let index: HashMap<&str, &Sentence> = all.iter().map(|s| (s.id.as_str(), s)).collect();
            apply_fidelity(
                db_path,
                backend,
                &ctx,
                &ctx.route_key(
                    Some(crate::provider::OP_INFERENCES_ASSESS),
                    "claim_assessor",
                ),
                cancel,
                &mut last,
                "inferences",
                "inference",
                "body_evidence_ids",
                &index,
                "no supported inferences",
            )
            .await?;
            last["windows"] = json!(windows.len());
            last["path"] = json!(if focused.is_some() { "focused" } else { "full" });
            if let Some(set) = &focused {
                last["packet_coverage"] = json!(set.coverage);
                last["packet_policy"] = json!(set.policy);
                last["omitted_ids"] = json!(set.omitted);
            }
            last["body_hash"] = body["content_hash"].clone();
            let n = last["inferences"].as_array().map_or(0, Vec::len);
            let reason = if n == 0 {
                last["empty_reason"].as_str().unwrap_or("").to_string()
            } else {
                String::new()
            };
            Ok((last, n, reason))
        }
        "intel_brief_links" => {
            let inferences = {
                let store = Store::open(db_path)?;
                stage_output(&store, &job, "intel_brief_inferences")?
            };
            let inference_ids: HashSet<String> = inferences["inferences"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|i| i["id"].as_str().map(str::to_string))
                .collect();
            if inference_ids.is_empty() {
                return Ok((
                    json!({"leads":[],"empty_reason":"no supported inferences"}),
                    0,
                    "no supported inferences".into(),
                ));
            }
            let related = backend.related(&article)?;
            let candidates = shortlist(&article, related);
            if candidates.is_empty() {
                return Ok((
                    json!({"leads":[],"empty_reason":"no matching articles"}),
                    0,
                    "no matching articles".into(),
                ));
            }
            let (iid, cands) = (inference_ids.clone(), candidates.clone());
            let validator: Validator = Arc::new(move |raw| {
                validate_leads(raw, &iid, &cands)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            });
            let raw = generate(
                db_path,
                backend,
                cancel,
                StageRequest {
                    ctx: ctx.clone(),
                    task_id: claimed.id.clone(),
                    stage: claimed.operation.clone(),
                    role: "synthesis",
                    system: format!(
                        "{LINKS_PROMPT}{}{STRICT_JSON}{LEADS_SHAPE}",
                        super::brief_contracts::leads_limits()
                    ),
                    user: links_user(&inferences, &candidates),
                    completion_tokens: LINKS_TOKENS,
                    reasoning: crate::request_policy::LINKS_BUDGET.reasoning,
                    operation_key: crate::provider::OP_LINKS_GENERATE,
                    validator,
                    budget: crate::request_policy::LINKS_BUDGET,
                    contract: Some(super::brief_contracts::leads_contract(
                        &candidates.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
                        &sorted_ids(&inference_ids),
                        &candidates
                            .iter()
                            .flat_map(|c| c.sentences.iter().map(|s| s.id.as_str()))
                            .collect::<Vec<_>>(),
                    )),
                },
            )
            .await?;
            let mut out = validate_leads(&raw.to_string(), &inference_ids, &candidates)?;
            let texts: HashMap<String, String> = inferences["inferences"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|i| {
                    Some((
                        i["id"].as_str()?.to_string(),
                        i["text"].as_str()?.to_string(),
                    ))
                })
                .collect();
            filter_leads(&mut out, &texts, &candidates);
            let n = out["leads"].as_array().map_or(0, Vec::len);
            let reason = if n == 0 {
                out["empty_reason"].as_str().unwrap_or("").to_string()
            } else {
                String::new()
            };
            Ok((out, n, reason))
        }
        other => bail!("unknown brief stage {other}"),
    }
}

/// Canonical inputs for the final commit; any preparation error becomes the
/// Index footer reason rather than a stage failure.
fn prepare_canonical(
    store: &Store,
    claimed: &ClaimedTask,
) -> std::result::Result<super::brief_canonical::CanonicalBrief, String> {
    let prepared = (|| -> Result<_> {
        let article = article_for(store, &article_id_of(claimed, store)?)?;
        super::brief_canonical::prepare(
            store,
            &article,
            &stage_output(store, &claimed.job_id, "intel_brief_body")?,
            &stage_output(store, &claimed.job_id, "intel_brief_claims")?,
            &stage_output(store, &claimed.job_id, "intel_brief_inferences")?,
        )
    })();
    match prepared {
        Ok(result) => result,
        Err(error) => Err(error.to_string()),
    }
}

/// Fidelity decisions with cached receipts; uncached items go to the
/// Claim assessor in batches of at most [`super::brief_fidelity::BATCH`].
/// Fidelity receipts are keyed by the item's source/contract receipt plus
/// the actual resolved route, so a route change re-assesses.
pub async fn assess_cached(
    db_path: &Path,
    backend: &mut dyn StageBackend,
    ctx: &StageContext,
    route_key: &str,
    items: Vec<super::brief_fidelity::FidelityItem>,
    cancel: &Arc<AtomicBool>,
) -> Result<HashMap<String, String>> {
    let receipt = |item: &super::brief_fidelity::FidelityItem| {
        format!("intel-fidelity:{}:{route_key}", item.receipt_key())
    };
    let mut labels = HashMap::new();
    let mut pending = Vec::new();
    {
        let store = Store::open(db_path)?;
        for item in items {
            match store.app_state_get(&receipt(&item))? {
                Some(label) => {
                    labels.insert(item.id.clone(), label);
                }
                None => pending.push(item),
            }
        }
    }
    for batch in pending.chunks(super::brief_fidelity::BATCH) {
        let decided = backend.assess(ctx, batch.to_vec(), cancel).await?;
        let store = Store::open(db_path)?;
        for item in batch {
            let label = decided
                .get(&item.id)
                .cloned()
                .ok_or_else(|| anyhow!("fidelity decision missing for {}", item.id))?;
            store.app_state_set(&receipt(item), &label)?;
            labels.insert(item.id.clone(), label);
        }
    }
    Ok(labels)
}

/// Keep only items the Claim assessor judged faithful to their own passages.
#[allow(clippy::too_many_arguments)]
async fn apply_fidelity(
    db_path: &Path,
    backend: &mut dyn StageBackend,
    ctx: &StageContext,
    route_key: &str,
    cancel: &Arc<AtomicBool>,
    out: &mut Value,
    key: &str,
    kind: &'static str,
    ids_key: &str,
    index: &HashMap<&str, &Sentence>,
    empty: &str,
) -> Result<()> {
    let list = out[key].as_array().cloned().unwrap_or_default();
    if list.is_empty() {
        return Ok(());
    }
    let items: Vec<super::brief_fidelity::FidelityItem> = list
        .iter()
        .map(|item| super::brief_fidelity::FidelityItem {
            id: item["id"].as_str().unwrap_or("").to_string(),
            kind,
            text: item["text"].as_str().unwrap_or("").to_string(),
            passages: item[ids_key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|id| index.get(id.as_str()?).map(|s| s.text.clone()))
                .collect(),
        })
        .collect();
    activity(db_path, &ctx.task_id, "checking fidelity");
    let labels = super::brief_timing::timed(
        super::brief_timing::Phase::Fidelity,
        assess_cached(db_path, backend, ctx, route_key, items, cancel),
    )
    .await?;
    let mut kept = Vec::new();
    for mut item in list {
        let label = labels
            .get(item["id"].as_str().unwrap_or(""))
            .cloned()
            .unwrap_or_default();
        if label == "faithful" {
            item["fidelity"] = json!(label);
            kept.push(item);
        }
    }
    let withheld = out[key].as_array().map_or(0, Vec::len) - kept.len();
    if kept.is_empty() {
        out["empty_reason"] = json!(empty);
    }
    out[key] = Value::Array(kept);
    out["withheld"] = json!(withheld);
    Ok(())
}

/// Claim and run the next eligible brief stage. Returns None when idle.
///
/// The heartbeat renews the lease from claim through commit/acknowledgement on
/// its own connection; if renewal fails or the chain is cancelled, the stage's
/// cancel flag is raised and the in-flight request future is dropped.
pub async fn run_next_stage(
    db_path: &Path,
    owner: &str,
    backend: &mut dyn StageBackend,
    parent_cancel: &Arc<AtomicBool>,
    config: &ExecConfig,
) -> Result<Option<StageOutcome>> {
    let Some(claimed) = claim_next(db_path, owner, config)? else {
        return Ok(None);
    };
    run_claimed(db_path, claimed, backend, parent_cancel, config)
        .await
        .map(Some)
}

/// Reap expired leases and claim one ready stage on a fresh connection.
pub fn claim_next(db_path: &Path, owner: &str, config: &ExecConfig) -> Result<Option<ClaimedTask>> {
    let store = Store::open_initialized_light(db_path)?;
    tasks::interrupt_expired_leases(&store.conn, &now())?;
    brief_pipeline::claim_stage(&store.conn, owner, config.lease_secs, &now())
}

/// Execute one already-claimed stage through commit/acknowledgement. Every
/// call opens its own connections, so concurrent stages never share a
/// mutable SQLite handle.
pub async fn run_claimed(
    db_path: &Path,
    claimed: ClaimedTask,
    backend: &mut dyn StageBackend,
    parent_cancel: &Arc<AtomicBool>,
    config: &ExecConfig,
) -> Result<StageOutcome> {
    let operation = claimed.operation.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let lost = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let heartbeat = {
        let (db, c, cancel, lost, done, parent) = (
            db_path.to_path_buf(),
            claimed.clone(),
            cancel.clone(),
            lost.clone(),
            done.clone(),
            parent_cancel.clone(),
        );
        let (interval, lease) = (config.heartbeat, config.lease_secs);
        tokio::spawn(async move {
            while !done.load(Ordering::Acquire) {
                let step = Duration::from_millis(50).min(interval);
                let mut waited = Duration::ZERO;
                while waited < interval && !done.load(Ordering::Acquire) {
                    if parent.load(Ordering::Acquire) {
                        cancel.store(true, Ordering::Release);
                    }
                    tokio::time::sleep(step).await;
                    waited += step;
                }
                if done.load(Ordering::Acquire) {
                    break;
                }
                let renewed = renew(&db, &c, lease);
                if !renewed {
                    note_heartbeat(&db, &c.id, "lease renewal failed; stage cancelled");
                    lost.store(true, Ordering::Release);
                    cancel.store(true, Ordering::Release);
                    break;
                }
            }
        })
    };
    let stage_clock = std::time::Instant::now();
    let clock = super::brief_timing::StageClock::default();
    let result = tokio::select! {
        r = super::brief_timing::with_clock(
            clock.clone(),
            execute(db_path, &claimed, backend, &cancel),
        ) => r,
        _ = cancelled(cancel.clone()) => Err(anyhow!("cancelled")),
    };
    {
        // Local work = stage wall time not spent in model/fidelity chains.
        use super::brief_timing::{note_on, peek, Phase};
        let remote = peek(&clock, Phase::Model) + peek(&clock, Phase::Fidelity);
        note_on(
            &clock,
            Phase::Local,
            stage_clock.elapsed().saturating_sub(remote),
        );
    }
    let persist_clock = std::time::Instant::now();
    // Commit while the heartbeat still covers the lease, then stop it.
    let outcome = {
        let store = Store::open(db_path)?;
        let at = now();
        match result {
            _ if lost.load(Ordering::Acquire) || cancel.load(Ordering::Acquire) => {
                StageOutcome::Stale {
                    operation: operation.clone(),
                }
            }
            Ok((value, items, reason)) => {
                let canonical = if operation == "intel_brief_links" {
                    Some(prepare_canonical(&store, &claimed))
                } else {
                    None
                };
                if brief_pipeline::commit_stage_with(
                    &store.conn,
                    &claimed,
                    &value.to_string(),
                    items,
                    &reason,
                    &at,
                    canonical.as_ref(),
                )? {
                    StageOutcome::Committed {
                        operation: operation.clone(),
                        items,
                    }
                } else {
                    StageOutcome::Stale {
                        operation: operation.clone(),
                    }
                }
            }
            Err(error) => {
                // The model chain is the single retry owner; a stage failure
                // waits for explicit Retry.
                let mut once = claimed.clone();
                once.max_attempts = 1;
                let reason = error.to_string();
                let category = tasks::ErrorCategory::InvalidResult;
                match tasks::fail_claimed(
                    &store.conn,
                    &once,
                    tasks::OperationKind::OtherLlm,
                    &tasks::TaskError {
                        category,
                        message: reason.clone(),
                    },
                    &at,
                )? {
                    Some(_) => StageOutcome::Failed {
                        operation: operation.clone(),
                        reason,
                    },
                    None => StageOutcome::Stale {
                        operation: operation.clone(),
                    },
                }
            }
        }
    };
    let persistence = persist_clock.elapsed();
    done.store(true, Ordering::Release);
    let _ = heartbeat.await;
    // Logs carry the same stage row as the brief panel and Jobs.
    if let Ok(store) = Store::open_light(db_path) {
        let _ = brief_pipeline::set_activity(&store.conn, &claimed.id, "");
        let outcome_label = match &outcome {
            StageOutcome::Committed { .. } => "committed",
            StageOutcome::Failed { .. } => "failed",
            StageOutcome::Stale { .. } => "stale",
        };
        if let Ok(timing) = super::brief_timing::collect(
            &store.conn,
            &claimed,
            &clock,
            stage_clock.elapsed(),
            persistence,
            outcome_label,
        ) {
            let _ = super::brief_timing::persist(&store.conn, &timing, &now());
            let _ = crate::events::record_event(
                &store.conn,
                &crate::events::NewEvent {
                    severity: None,
                    app: "intel".into(),
                    event_type: "intel_brief.timing".into(),
                    message: format!(
                        "{} · {}",
                        claimed.operation,
                        super::brief_timing::summary(&timing)
                    ),
                    job_id: claimed.job_id.clone(),
                    task_id: claimed.id.clone(),
                    ..Default::default()
                },
            );
        }
        if let Some(view) = brief_pipeline::stage_views(&store.conn, &claimed.job_id)
            .ok()
            .and_then(|v| v.into_iter().find(|v| v.operation == operation))
        {
            let _ = crate::events::record_event(
                &store.conn,
                &crate::events::NewEvent {
                    severity: None,
                    app: "intel".into(),
                    event_type: "intel_brief.stage".into(),
                    message: format!("{} · {}", view.label, view.row),
                    job_id: claimed.job_id.clone(),
                    task_id: claimed.id.clone(),
                    ..Default::default()
                },
            );
        }
    }
    Ok(outcome)
}

/// Durable heartbeat diagnostics (renewal failures and heartbeat age),
/// surfaced in Logs alongside the task's own `heartbeat_at`.
fn note_heartbeat(db: &Path, task: &str, message: &str) {
    if let Ok(store) = Store::open_light(db) {
        let age: Option<f64> = store
            .conn
            .query_row(
                "SELECT (julianday(?2)-julianday(heartbeat_at))*86400.0 FROM argos_tasks WHERE id=?1 AND heartbeat_at<>''",
                rusqlite::params![task, now()],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten();
        let _ = store.app_state_set(
            &format!("intel-brief-heartbeat:{task}"),
            &json!({"at": now(), "message": message, "heartbeat_age_secs": age}).to_string(),
        );
    }
}

fn renew(db: &Path, claimed: &ClaimedTask, lease: i64) -> bool {
    for _ in 0..5 {
        match Store::open_light(db).and_then(|s| {
            tasks::renew_lease(
                &s.conn,
                &claimed.id,
                &claimed.owner,
                claimed.epoch,
                lease,
                &now(),
            )
        }) {
            Ok(ok) => return ok,
            Err(error) => {
                note_heartbeat(db, &claimed.id, &format!("renewal retry: {error}"));
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    false
}

/// Serial drain with one backend: run ready stages until none is claimable.
/// A failed branch no longer stops the drain: its independent sibling still
/// runs (spec §6). Stops after a stale outcome (cancel/supersede/lost lease).
pub async fn drain(
    db_path: &Path,
    owner: &str,
    backend: &mut dyn StageBackend,
    cancel: &Arc<AtomicBool>,
    config: &ExecConfig,
) -> Result<Vec<StageOutcome>> {
    let mut outcomes = Vec::new();
    while !cancel.load(Ordering::Acquire) {
        let Some(outcome) = run_next_stage(db_path, owner, backend, cancel, config).await? else {
            break;
        };
        let stop = matches!(outcome, StageOutcome::Stale { .. });
        outcomes.push(outcome);
        if stop {
            break;
        }
    }
    Ok(outcomes)
}

/// Stage slots of the dedicated drainer: the two independent v2 roots.
pub const STAGE_SLOTS: usize = 2;

/// Bounded parallel drain (spec §6). A slot is taken *before* a lease is
/// claimed, each claimed stage runs on its own Tokio task with its own
/// backend (from `make_backend`) and connections, and task-local timing and
/// cancellation stay per stage. Returns when nothing is running and nothing
/// is claimable.
pub async fn drain_parallel<F>(
    db_path: &Path,
    owner: &str,
    make_backend: F,
    cancel: &Arc<AtomicBool>,
    config: &ExecConfig,
    slots: usize,
) -> Result<Vec<StageOutcome>>
where
    F: Fn() -> Box<dyn StageBackend + Send + 'static>,
{
    let mut running: tokio::task::JoinSet<Result<StageOutcome>> = tokio::task::JoinSet::new();
    let mut outcomes = Vec::new();
    loop {
        // Fill free slots; each slot is reserved before its claim.
        while running.len() < slots.max(1) && !cancel.load(Ordering::Acquire) {
            let Some(claimed) = claim_next(db_path, owner, config)? else {
                break;
            };
            let (db, parent, cfg) = (db_path.to_path_buf(), cancel.clone(), config.clone());
            let mut backend = make_backend();
            running.spawn(async move {
                run_claimed(&db, claimed, backend.as_mut(), &parent, &cfg).await
            });
        }
        match running.join_next().await {
            Some(joined) => {
                let outcome = joined.map_err(|e| anyhow!("stage task panicked: {e}"))??;
                outcomes.push(outcome);
            }
            None => break,
        }
    }
    Ok(outcomes)
}

// ---------- runtime backend ----------

/// Production backend: Stage 1 reuses the body retrieval/cleanup path under
/// the brief job; generated stages use the captured role chain with the
/// per-stage completion ceiling.
pub struct RuntimeBackend {
    pub db_path: PathBuf,
    pub keys: crate::osint::ProviderKeys,
    pub auth: crate::secrets::AuthFile,
    pub settings: crate::provider::SettingsFile,
    pub user_agent: Option<String>,
}

impl RuntimeBackend {
    fn job_runtime(
        &self,
        ctx: &StageContext,
        article: &AtlasArticleRow,
        cancel: &Arc<AtomicBool>,
    ) -> super::worker::JobRuntime {
        let settings = ctx.routed(&self.settings);
        super::worker::JobRuntime {
            stage: Some(ctx.clone()),
            db_path: self.db_path.clone(),
            job_id: ctx.job_id.clone(),
            article_title: article.title.clone(),
            article_url: article.url.clone(),
            article_preview: article.description.clone(),
            article_published: article.published_at.clone(),
            article_domain: article.source_domain.clone(),
            run_id: article.run_id.clone(),
            keys: self.keys.clone(),
            auth: self.auth.clone(),
            synthesis_secret: None,
            classifier_secret: crate::provider::role_secret(&self.auth, &settings, "classifier")
                .ok(),
            settings,
            cancel: cancel.clone(),
        }
    }
}

pub fn load_clean_artifact(
    store: &Store,
    article_id: &str,
    generation: i64,
) -> Result<Option<CleanArtifact>> {
    Ok(store
        .app_state_get(&format!("intel-clean-body:{article_id}:{generation}"))?
        .and_then(|json| serde_json::from_str(&json).ok()))
}

impl StageBackend for RuntimeBackend {
    fn prepare_body<'a>(
        &'a mut self,
        ctx: &'a StageContext,
        article: &'a AtlasArticleRow,
        force_reload: bool,
        cancel: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<CleanArtifact>> {
        Box::pin(async move {
            let outcome = {
                let store = Store::open(&self.db_path)?;
                super::body::enqueue_article_body(&store, article, force_reload)?
            };
            let (body_id, generation, force) = match outcome {
                super::body::EnqueueOutcome::Cached(body_id) => {
                    let store = Store::open(&self.db_path)?;
                    let body = store
                        .article_body_by_id(&body_id)?
                        .ok_or_else(|| anyhow!("body missing"))?;
                    if let Some(artifact) =
                        load_clean_artifact(&store, &article.id, body.body_version)?
                    {
                        return Ok(artifact);
                    }
                    // Legacy display body without a clean artifact: re-derive.
                    (body_id, body.body_version + 1, true)
                }
                super::body::EnqueueOutcome::Start {
                    body_id,
                    generation,
                    force_refresh,
                } => (body_id, generation, force_refresh),
                super::body::EnqueueOutcome::AlreadyRunning { .. } => {
                    bail!("article retrieval already running")
                }
                super::body::EnqueueOutcome::Cooldown { reason, .. } => {
                    bail!("story unavailable: {reason}")
                }
            };
            if force {
                let store = Store::open(&self.db_path)?;
                store.clear_article_body_content(&body_id)?;
            }
            let runtime = self.job_runtime(ctx, article, cancel);
            super::body::fetch_article_body_with_runtime(
                &self.db_path,
                &body_id,
                &article.title,
                &article.description,
                self.keys.clone(),
                None,
                runtime.classifier_secret.clone(),
                self.user_agent.clone(),
                force,
                cancel.clone(),
                |_| {},
                Some(&runtime),
            )
            .await?;
            let store = Store::open(&self.db_path)?;
            load_clean_artifact(&store, &article.id, generation)?
                .ok_or_else(|| anyhow!("story missing: retrieval produced no validated body"))
        })
    }

    fn assess<'a>(
        &'a mut self,
        ctx: &'a StageContext,
        items: Vec<super::brief_fidelity::FidelityItem>,
        cancel: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<HashMap<String, String>>> {
        Box::pin(async move {
            let task_id = ctx.task_id.as_str();
            let operation_id = ctx.operation_id(
                "fidelity",
                &items.iter().map(|i| i.receipt_key()).collect::<String>(),
            );
            let settings = ctx.routed(&self.settings);
            let adapter = super::brief_fidelity::FidelityDecisions { items: &items };
            let mode = adapter.mode();
            let mut scope = ctx
                .scope(crate::recon::model_exec::OperationScope::new(
                    &operation_id,
                    mode.role(),
                ))
                .with_budget(match mode {
                    super::brief_fidelity::FidelityMode::Extraction => {
                        crate::request_policy::FIDELITY_BUDGET
                    }
                    super::brief_fidelity::FidelityMode::Support => {
                        crate::request_policy::SUPPORT_BUDGET
                    }
                })
                .with_format(crate::recon::model_exec::ResponseFormat::BoundedDecision)
                .with_interactive();
            if let Some(key) = mode.operation_key() {
                scope = scope.with_operation(key);
            }
            let (labels, _) = crate::recon::model_exec::execute_decisions_or_chat(
                &self.auth,
                &settings,
                &scope,
                &adapter,
                cancel,
                Some(super::brief_timing::activity_sink(
                    &self.db_path,
                    task_id,
                    "checking fidelity",
                    scope.role,
                )),
                Some(self.db_path.as_path()),
            )
            .await?;
            Ok(labels)
        })
    }

    fn focused_oversized(&self) -> bool {
        self.settings.defaults.focused_oversized_brief
    }

    fn related(&mut self, article: &AtlasArticleRow) -> Result<Vec<AtlasArticleRow>> {
        let store = Store::open(&self.db_path)?;
        // All retained cycles by default, one row per URL.
        Ok(store
            .related_articles_global(
                &article.run_id,
                &article.id,
                crate::related_global::RelatedScope::AllRetained,
                None,
                50,
            )?
            .rows
            .into_iter()
            .map(|r| r.article)
            .collect())
    }

    fn generate<'a>(
        &'a mut self,
        request: StageRequest,
        cancel: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<String>> {
        Box::pin(async move {
            let operation_id = request
                .ctx
                .operation_id(request.operation_key, &request.user);
            let settings = request.ctx.routed(&self.settings);
            let mut scope = request
                .ctx
                .scope(crate::recon::model_exec::OperationScope::new(
                    &operation_id,
                    request.role,
                ))
                .with_insight_contract()
                .with_budget(request.budget)
                .with_operation(request.operation_key)
                .with_format(crate::recon::model_exec::ResponseFormat::GeneratedJson)
                .with_interactive();
            if let Some(contract) = request.contract.clone() {
                scope = scope.with_contract(contract);
            }
            let messages = [
                crate::provider::ChatMessage {
                    role: "system".into(),
                    content: request.system.clone(),
                    tool_call_id: None,
                    tool_calls: vec![],
                },
                crate::provider::ChatMessage {
                    role: "user".into(),
                    content: request.user.clone(),
                    tool_call_id: None,
                    tool_calls: vec![],
                },
            ];
            let validator = request.validator.clone();
            let (text, _) = crate::recon::model_exec::execute_chat(
                &self.auth,
                &settings,
                &scope,
                &messages,
                move |raw| validator(raw),
                cancel,
                Some(super::brief_timing::activity_sink(
                    &self.db_path,
                    &request.task_id,
                    &request.stage,
                    request.role,
                )),
                None,
                Some(self.db_path.as_path()),
                true,
            )
            .await?;
            Ok(text)
        })
    }
}

/// Production construction from the configured settings.
impl RuntimeBackend {
    pub fn load(db_path: &Path) -> Self {
        let settings = crate::provider::SettingsFile::load().unwrap_or_default();
        Self {
            db_path: db_path.to_path_buf(),
            keys: crate::osint::ProviderKeys::from_settings(&settings),
            auth: crate::secrets::AuthFile::load().unwrap_or_default(),
            user_agent: Some(settings.osint_user_agent.clone()).filter(|ua| !ua.trim().is_empty()),
            settings,
        }
    }
}

/// Per-node semantic revisions in stage order (spec §6). Each node is keyed
/// independently by its own source revision, contract/prompt version and
/// resolved route/policy; prerequisite outputs enter descendant keys through
/// the join (bound output hashes), never through these revisions. Claims
/// therefore carry title/description and their own routes but no body
/// retrieval/cleanup hash: a body-only refresh reuses Claims.
pub fn stage_revisions(
    article: &AtlasArticleRow,
    settings: &crate::provider::SettingsFile,
) -> Vec<String> {
    use crate::provider::{
        OP_CLAIMS_GENERATE, OP_INFERENCES_ASSESS, OP_INFERENCES_GENERATE, OP_LINKS_GENERATE,
    };
    let d = &settings.defaults;
    let route = |op: Option<&str>, role: &str| {
        serde_json::to_value(d.resolve_operation(op, role).0).unwrap_or(Value::Null)
    };
    let h = |v: Value| crate::nlp::cache::hash(&v.to_string());
    let body = h(json!({
        "contract": brief_pipeline::CONTRACT,
        "url": article.url,
        // Cleanup anchors on title/description.
        "title": article.title,
        "description": article.description,
        "transform": super::clean_body::TRANSFORM,
        "index": super::sentence_index::INDEX_VERSION,
        "classifier": route(None, "classifier"),
    }));
    let claims = h(json!({
        "contract": brief_pipeline::CONTRACT,
        "title": article.title,
        "description": article.description,
        "published_at": article.published_at,
        "prompt": CLAIMS_PROMPT,
        "generate": route(Some(OP_CLAIMS_GENERATE), "claim_assessor"),
        "fidelity": route(None, "evidence_curator"),
        "fidelity_contract": format!("extraction_fidelity:{}", crate::investigation::decisions::templates::TEMPLATE_VERSION),
    }));
    let inferences = h(json!({
        "contract": brief_pipeline::CONTRACT,
        "prompt": INFERENCES_PROMPT,
        "window": REQUEST_WINDOW_BYTES,
        "generate": route(Some(OP_INFERENCES_GENERATE), "synthesis"),
        "assess": route(Some(OP_INFERENCES_ASSESS), "claim_assessor"),
        "assess_prompt": super::brief_fidelity::FIDELITY_PROMPT,
        "focused_oversized": d.focused_oversized_brief,
        "packet_policy": super::passage_packets::PACKET_POLICY,
    }));
    let links = h(json!({
        "contract": brief_pipeline::CONTRACT,
        "prompt": LINKS_PROMPT,
        "generate": route(Some(OP_LINKS_GENERATE), "synthesis"),
        // Link candidates come from Brain actors and all-retained Related.
        "identity_index": crate::brain_actors::IDENTITY_INDEX_VERSION,
        "actor_index": crate::atlas_actor_index::INDEX_CONTRACT,
        "related_scope": "all_retained",
        "related_match": crate::related_articles::MATCH_VERSION,
    }));
    vec![body, claims, inferences, links]
}

/// Captured metadata snapshot persisted in the root task inputs.
pub fn metadata_snapshot(article: &AtlasArticleRow) -> Value {
    json!({
        "title": article.title,
        "description": article.description,
        "published_at": article.published_at,
        "source_revision": crate::nlp::cache::hash(&json!([article.title, article.description, article.published_at, article.url]).to_string()),
    })
}

/// Chain input revision: every node revision folded together, so any node
/// change admits a new generation (unchanged nodes are then reused).
pub fn input_revision(
    article: &AtlasArticleRow,
    settings: &crate::provider::SettingsFile,
) -> String {
    crate::nlp::cache::hash(&stage_revisions(article, settings).join(":"))
}

/// Reload: supersede the chain with a forced re-acquisition of the body.
/// Later visits attach to this chain (same base inputs).
pub fn reload_intel_brief_pipeline(
    store: &Store,
    article: &AtlasArticleRow,
    settings: &crate::provider::SettingsFile,
) -> Result<String> {
    brief_pipeline::ensure_chain(
        &store.conn,
        &brief_pipeline::BriefInputs {
            article_id: article.id.clone(),
            input_revision: format!("{}:reload:{}", input_revision(article, settings), now()),
            role_snapshot: serde_json::to_string(&settings.defaults).unwrap_or_default(),
            title: article.title.clone(),
            stage_revisions: stage_revisions(article, settings),
            metadata: metadata_snapshot(article),
        },
        &now(),
    )
}

/// The single idempotent entry point for opening a brief, body-ready events
/// and recovery: attach to (or admit) this article's preparation chain.
pub fn ensure_intel_brief_pipeline(
    store: &Store,
    article: &AtlasArticleRow,
    settings: &crate::provider::SettingsFile,
) -> Result<String> {
    let job = brief_pipeline::ensure_chain(
        &store.conn,
        &brief_pipeline::BriefInputs {
            article_id: article.id.clone(),
            input_revision: input_revision(article, settings),
            role_snapshot: serde_json::to_string(&settings.defaults).unwrap_or_default(),
            title: article.title.clone(),
            stage_revisions: stage_revisions(article, settings),
            metadata: metadata_snapshot(article),
        },
        &now(),
    )?;
    super::brief_canonical::refresh_cached_publication(store, article, &job)?;
    Ok(job)
}

#[cfg(test)]
#[path = "brief_stages_tests.rs"]
mod tests;
