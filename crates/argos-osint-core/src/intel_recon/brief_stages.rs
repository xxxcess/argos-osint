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

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub type Validator = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// One model request for a stage.
pub struct StageRequest {
    pub task_id: String,
    pub stage: String,
    pub role: &'static str,
    pub system: String,
    pub user: String,
    pub completion_tokens: u32,
    pub validator: Validator,
}

/// Provider/retrieval boundary for stage execution (runtime or test double).
pub trait StageBackend: Send {
    /// Stage 1: retrieve (or reuse) and clean the article body.
    fn prepare_body<'a>(
        &'a mut self,
        article: &'a AtlasArticleRow,
        cancel: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<CleanArtifact>>;
    /// Existing related articles for Stage 4 (metadata only).
    fn related(&mut self, article: &AtlasArticleRow) -> Result<Vec<AtlasArticleRow>>;
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
    ensure!(words(&reason) <= 25, "empty_reason exceeds 25 words");
    Ok(reason)
}

// ---------- Stage 2: Claims ----------

pub fn metadata_sentences(article: &AtlasArticleRow) -> Vec<Sentence> {
    let mut all = sentences("t", &article.title);
    all.extend(sentences("d", &article.description));
    all
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
        ensure!(words(text) <= 35, "claim exceeds 35 words");
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
            !text.is_empty() && words(text) <= 35,
            "inference text missing or over 35 words"
        );
        let why = str_field(item, "why").trim();
        ensure!(
            !why.is_empty() && words(why) <= 45,
            "reasoning missing or over 45 words"
        );
        let support = str_field(item, "support");
        ensure!(
            matches!(support, "supported" | "conditional"),
            "support must be supported or conditional"
        );
        let limitations: Vec<String> = ids(item, "limitations")?;
        ensure!(limitations.len() <= 2, "at most two limitations");
        ensure!(
            limitations.iter().all(|l| words(l) <= 20),
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
            !why.is_empty() && words(why) <= 40,
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

async fn generate(
    backend: &mut dyn StageBackend,
    cancel: &Arc<AtomicBool>,
    request: StageRequest,
) -> Result<Value> {
    let validator = request.validator.clone();
    let raw = backend.generate(request, cancel).await?;
    validator(&raw).map_err(|e| anyhow!("invalid stage output: {e}"))?;
    strict_json(&raw)
}

/// Run one stage: Ok(value, items, empty_reason).
async fn execute(
    db_path: &Path,
    claimed: &ClaimedTask,
    backend: &mut dyn StageBackend,
    cancel: &Arc<AtomicBool>,
) -> Result<(Value, usize, String)> {
    let (article, job) = {
        let store = Store::open(db_path)?;
        (
            article_for(&store, &article_id_of(claimed, &store)?)?,
            claimed.job_id.clone(),
        )
    };
    match claimed.operation.as_str() {
        "intel_brief_body" => {
            let artifact = backend.prepare_body(&article, cancel).await?;
            stage_one_admissible(&artifact, artifact.acquisition_complete)?;
            let words = artifact.markdown.split_whitespace().count();
            let reason = if artifact.diagnostics.is_empty() {
                String::new()
            } else {
                artifact.diagnostics.join("; ")
            };
            Ok((serde_json::to_value(&artifact)?, words, reason))
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
                backend,
                cancel,
                StageRequest {
                    task_id: claimed.id.clone(),
                    stage: claimed.operation.clone(),
                    role: "claim_assessor",
                    system: format!("{CLAIMS_PROMPT}{STRICT_JSON}{CLAIMS_SHAPE}"),
                    user: claims_user(&article),
                    completion_tokens: CLAIMS_TOKENS,
                    validator,
                },
            )
            .await?;
            let out = validate_claims(&raw.to_string(), &meta)?;
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
            let markdown = body["markdown"].as_str().unwrap_or("").to_string();
            let all = sentences("b", &markdown);
            let claim_ids: HashSet<String> = claims["claims"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c["id"].as_str().map(str::to_string))
                .collect();
            let windows = body_windows(&all, BODY_WINDOW_BYTES);
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
                    backend,
                    cancel,
                    StageRequest {
                        task_id: claimed.id.clone(),
                        stage: claimed.operation.clone(),
                        role: "synthesis",
                        system: format!("{INFERENCES_PROMPT}{STRICT_JSON}{INFERENCES_SHAPE}"),
                        user: inferences_user(&claims, window, &carried),
                        completion_tokens: INFERENCES_TOKENS,
                        validator,
                    },
                )
                .await?;
                last = validate_inferences(&raw.to_string(), &claim_ids, window, &covered)?;
                covered.extend(window.iter().cloned());
                carried = json!(last["inferences"].as_array().into_iter().flatten().map(|i| json!({"id":i["id"],"text":i["text"],"body_evidence_ids":i["body_evidence_ids"],"support":i["support"]})).collect::<Vec<_>>());
            }
            last["windows"] = json!(windows.len());
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
                backend,
                cancel,
                StageRequest {
                    task_id: claimed.id.clone(),
                    stage: claimed.operation.clone(),
                    role: "synthesis",
                    system: format!("{LINKS_PROMPT}{STRICT_JSON}{LEADS_SHAPE}"),
                    user: links_user(&inferences, &candidates),
                    completion_tokens: LINKS_TOKENS,
                    validator,
                },
            )
            .await?;
            let out = validate_leads(&raw.to_string(), &inference_ids, &candidates)?;
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
    let claimed = {
        let store = Store::open(db_path)?;
        tasks::interrupt_expired_leases(&store.conn, &now())?;
        match brief_pipeline::claim_stage(&store.conn, owner, config.lease_secs, &now())? {
            Some(c) => c,
            None => return Ok(None),
        }
    };
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
    let result = tokio::select! {
        r = execute(db_path, &claimed, backend, &cancel) => r,
        _ = cancelled(cancel.clone()) => Err(anyhow!("cancelled")),
    };
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
                if brief_pipeline::commit_stage(
                    &store.conn,
                    &claimed,
                    &value.to_string(),
                    items,
                    &reason,
                    &at,
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
    done.store(true, Ordering::Release);
    let _ = heartbeat.await;
    Ok(Some(outcome))
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

/// Drain the chain until no stage is claimable.
pub async fn drain(
    db_path: &Path,
    owner: &str,
    backend: &mut dyn StageBackend,
    cancel: &Arc<AtomicBool>,
    config: &ExecConfig,
) -> Result<Vec<StageOutcome>> {
    let mut outcomes = Vec::new();
    while let Some(outcome) = run_next_stage(db_path, owner, backend, cancel, config).await? {
        let stop = !matches!(outcome, StageOutcome::Committed { .. });
        outcomes.push(outcome);
        if stop {
            break;
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
    pub job_id: String,
    pub keys: crate::osint::ProviderKeys,
    pub auth: crate::secrets::AuthFile,
    pub settings: crate::provider::SettingsFile,
    pub user_agent: Option<String>,
}

impl RuntimeBackend {
    fn job_runtime(
        &self,
        article: &AtlasArticleRow,
        cancel: &Arc<AtomicBool>,
    ) -> super::worker::JobRuntime {
        super::worker::JobRuntime {
            db_path: self.db_path.clone(),
            job_id: self.job_id.clone(),
            article_title: article.title.clone(),
            article_url: article.url.clone(),
            article_preview: article.description.clone(),
            article_published: article.published_at.clone(),
            article_domain: article.source_domain.clone(),
            run_id: article.run_id.clone(),
            keys: self.keys.clone(),
            auth: self.auth.clone(),
            synthesis_secret: None,
            classifier_secret: crate::provider::role_secret(
                &self.auth,
                &self.settings,
                "classifier",
            )
            .ok(),
            settings: self.settings.clone(),
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
        article: &'a AtlasArticleRow,
        cancel: &'a Arc<AtomicBool>,
    ) -> BoxFut<'a, Result<CleanArtifact>> {
        Box::pin(async move {
            let outcome = {
                let store = Store::open(&self.db_path)?;
                super::body::enqueue_article_body(&store, article, false)?
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
            let runtime = self.job_runtime(article, cancel);
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

    fn related(&mut self, article: &AtlasArticleRow) -> Result<Vec<AtlasArticleRow>> {
        let store = Store::open(&self.db_path)?;
        Ok(store
            .related_articles(&article.run_id, &article.id, 0, 50)?
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
            let operation_id = format!(
                "{}:{}",
                request.task_id,
                crate::nlp::cache::hash(&request.user)
            );
            let scope = crate::recon::model_exec::OperationScope::new(&operation_id, request.role)
                .with_task(&request.task_id)
                .with_insight_contract()
                .with_completion_tokens(request.completion_tokens);
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
                &self.settings,
                &scope,
                &messages,
                move |raw| validator(raw),
                cancel,
                None,
                None,
                Some(self.db_path.as_path()),
                true,
            )
            .await?;
            Ok(text)
        })
    }
}

#[cfg(test)]
#[path = "brief_stages_tests.rs"]
mod tests;
