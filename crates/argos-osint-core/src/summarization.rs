//! Dedicated Summarization role: evidence compression and source-grounded views.
//!
//! Analytical conclusions stay with Synthesis. Summarization must not invent
//! connections, promote inferences to facts, call tools, or decide that a
//! directive is satisfied.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::provider::ChatMessage;
use crate::secrets::ProviderSecret;
use crate::tasks::OperationKind;

mod exec;
pub use exec::{
    admission_account, complete_summary_report, AttemptEvent, AttemptLog, ExecOptions,
    SummaryExecution, MAX_REQUESTS,
};

/// Production modes. Each maps to a real call site.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummarizationMode {
    PageEvidence,
    FollowUpContext,
    ToolObservation,
    InvestigationTitle,
    ReportContext,
    SectionDigest,
    AtlasBrief,
    ArticleDescription,
    /// Typed topology neighborhood packet → structured brief JSON.
    TopologyNeighborhood,
}

impl SummarizationMode {
    pub const ALL: [SummarizationMode; 9] = [
        Self::PageEvidence,
        Self::FollowUpContext,
        Self::ToolObservation,
        Self::InvestigationTitle,
        Self::ReportContext,
        Self::SectionDigest,
        Self::AtlasBrief,
        Self::ArticleDescription,
        Self::TopologyNeighborhood,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::PageEvidence => "page_evidence",
            Self::FollowUpContext => "follow_up_context",
            Self::ToolObservation => "tool_observation",
            Self::InvestigationTitle => "investigation_title",
            Self::ReportContext => "report_context",
            Self::SectionDigest => "section_digest",
            Self::AtlasBrief => "atlas_brief",
            Self::ArticleDescription => "article_description",
            Self::TopologyNeighborhood => "topology_neighborhood",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "page_evidence" | "page-evidence" => Some(Self::PageEvidence),
            "follow_up_context" | "follow-up-context" => Some(Self::FollowUpContext),
            "tool_observation" | "tool-observation" => Some(Self::ToolObservation),
            "investigation_title" | "investigation-title" => Some(Self::InvestigationTitle),
            "report_context" | "report-context" => Some(Self::ReportContext),
            "section_digest" | "section-digest" => Some(Self::SectionDigest),
            "atlas_brief" | "atlas-brief" => Some(Self::AtlasBrief),
            "article_description" | "article-description" => Some(Self::ArticleDescription),
            "topology_neighborhood" | "topology-neighborhood" => Some(Self::TopologyNeighborhood),
            _ => None,
        }
    }

    /// Prompt version string included in cache keys.
    pub fn prompt_version(self) -> &'static str {
        if self == Self::TopologyNeighborhood {
            return "topology-neighborhood.p2.o1";
        }
        "summarization.v2"
    }
}

/// One source record handed to Summarization.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SummarySource {
    pub id: String,
    pub revision: String,
    pub hash: String,
    pub text: String,
    #[serde(default)]
    pub meta: serde_json::Value,
}

/// Request envelope for the shared summarization service.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SummaryRequest {
    pub mode: SummarizationMode,
    pub sources: Vec<SummarySource>,
    pub focus: String,
    pub budget_chars: usize,
    pub required_fields: Vec<String>,
    pub model: String,
    pub provider: String,
    pub prompt_version: String,
    #[serde(default)]
    pub preprocessing: serde_json::Value,
    #[serde(default)]
    pub deterministic: Option<Box<SummaryResult>>,
}

impl SummaryRequest {
    pub fn cache_key(&self) -> String {
        crate::nlp::cache::key(&serde_json::json!({
            "mode":self.mode,"focus":self.focus,"budget":self.budget_chars,
            "provider":self.provider,"model":self.model,"prompt":self.prompt_version,
            "required_fields":self.required_fields,"preprocessing":self.preprocessing,
            "sources":self.sources.iter().map(|s|serde_json::json!({"id":s.id,"revision":s.revision,
                "hash":sha_hex(&s.text),"declared_hash":s.hash,"meta":s.meta})).collect::<Vec<_>>()
        }))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SummaryResult {
    pub content: String,
    pub source_refs: Vec<String>,
    pub source_hash: String,
    pub model: String,
    pub prompt_version: String,
    pub coverage: CoverageMeta,
    pub fallback: bool,
    #[serde(default)]
    pub source_hashes: Vec<String>,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub preprocessing: serde_json::Value,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CoverageMeta {
    pub partial: bool,
    pub omitted: Vec<String>,
    pub notes: String,
}

/// System prompt for a mode. Callers pass sources separately as data.
pub fn system_prompt(mode: SummarizationMode) -> &'static str {
    match mode {
        SummarizationMode::TopologyNeighborhood => crate::brain_neighborhood_brief::MODEL_TASK,
        SummarizationMode::PageEvidence => {
            "Compress the supplied page evidence for investigation use. Preserve attribution, uncertainty, and exact identifiers. Do not invent facts, call tools, or decide directive completion. Write concise Markdown."
        }
        SummarizationMode::FollowUpContext => {
            "Summarize the prior investigation answer for the next turn. Copy supplied directive states exactly; do not reassess completion. Keep entities, identifiers, uncertainty, and unresolved questions."
        }
        SummarizationMode::ToolObservation => {
            "Digest the tool observation for the current question. Preserve tool name, call id, status, counts, errors, and selected record identifiers as structured facts. Do not invent bindings."
        }
        SummarizationMode::InvestigationTitle => {
            "Propose a short recognizable investigation title from the question. No quotes. No trailing punctuation. At most 8 words."
        }
        SummarizationMode::ReportContext => {
            "Compress source material for one report section. Preserve evidence references and disagreements. Do not write final analytical judgments or BLUF."
        }
        SummarizationMode::SectionDigest => {
            "Write a short digest of a completed analytical section for later BLUF use. Do not change the section's conclusions."
        }
        SummarizationMode::AtlasBrief => {
            "Render a concise natural-language brief from the supplied accepted claims. Preserve fact/inference distinctions. Do not create new claims."
        }
        SummarizationMode::ArticleDescription => {
            "Write a concise article description from the supplied title and body excerpt. Do not invent events absent from the source."
        }
    }
}

/// Reject empty or malformed model output for a mode.
pub fn validate_result(
    mode: SummarizationMode,
    content: &str,
    known_ids: &[String],
) -> Result<(), String> {
    let text = content.trim();
    if text.is_empty() {
        return Err("empty summarization output".into());
    }
    match mode {
        SummarizationMode::InvestigationTitle if text.chars().count() > 80 => {
            return Err("title exceeds length budget".into());
        }
        SummarizationMode::TopologyNeighborhood => {
            serde_json::from_str::<crate::brain_neighborhood_brief::NeighborhoodBrief>(text)
                .map_err(|e| format!("malformed neighborhood brief: {e}"))?;
            return Ok(());
        }
        SummarizationMode::PageEvidence if text.chars().count() < 20 => {
            return Err("summary too short to be useful".into());
        }
        _ => {}
    }
    // Unknown evidence IDs in bracket citations are rejected when known_ids is non-empty.
    if !known_ids.is_empty() {
        for token in text.split_whitespace() {
            let trimmed = token.trim_matches(|c: char| matches!(c, '[' | ']' | ',' | ';' | '.'));
            if let Some(id) = trimmed.strip_prefix("call-").or_else(|| {
                if trimmed.starts_with("brain:") {
                    Some(trimmed)
                } else {
                    None
                }
            }) {
                let full = if trimmed.starts_with("brain:") {
                    trimmed.to_string()
                } else {
                    format!("call-{id}")
                };
                if !known_ids.iter().any(|k| k == &full || k == trimmed) {
                    return Err(format!("unknown evidence id {trimmed}"));
                }
            }
        }
    }
    if !known_ids.is_empty() {
        let citations = regex::Regex::new(r"\[([^\[\]]+)\]").unwrap();
        for group in citations.captures_iter(text) {
            for id in group[1].split(|c: char| c.is_whitespace() || c == ',' || c == ';') {
                let id = id.trim_matches('.');
                if [
                    "call-", "brain:", "iev-", "art-", "article:", "source-", "src-", "memory-",
                    "section-",
                ]
                .iter()
                .any(|p| id.starts_with(p))
                    && !known_ids.iter().any(|known| known == id)
                {
                    return Err(format!("unknown evidence id {id}"));
                }
            }
        }
    }
    Ok(())
}

/// Deterministic title fallback when the model is unavailable.
pub fn fallback_title(question: &str) -> String {
    let flat: String = question
        .split_whitespace()
        .take(8)
        .collect::<Vec<_>>()
        .join(" ");
    let mut title = flat.chars().take(72).collect::<String>();
    if title.is_empty() {
        title = "Investigation".into();
    }
    title
}

/// Deterministic page excerpt used when Summarization cannot run.
pub fn fallback_excerpt(text: &str, budget: usize) -> String {
    let budget = budget.max(1);
    let trimmed = text.trim();
    if trimmed.chars().count() <= budget {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(budget.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Look up a cached derived summary by the request cache key fields.
fn cache_get_exact(
    conn: &Connection,
    req: &SummaryRequest,
) -> Result<Option<SummaryResult>, anyhow::Error> {
    if req.sources.iter().any(|s| sha_hex(&s.text) != s.hash) {
        return Ok(None);
    }
    let stored: Option<String> = conn
        .query_row(
            "SELECT result_json FROM argos_derived_summaries WHERE id=?1",
            [format!("sum-{}", req.cache_key())],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(json) = stored {
        if let Ok(result) = serde_json::from_str::<SummaryResult>(&json) {
            return Ok(Some(result));
        }
    }
    let row: Option<(String, String, String, i64, String)> = conn.query_row(
        "SELECT content, source_refs_json, coverage_json, fallback, model FROM argos_derived_summaries WHERE id=?1",
        [format!("sum-{}",req.cache_key())],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
    ).optional()?;
    let Some((content, refs_json, cov_json, fallback, model)) = row else {
        return Ok(None);
    };
    let source_refs: Vec<String> = serde_json::from_str(&refs_json).unwrap_or_default();
    let coverage: CoverageMeta = serde_json::from_str(&cov_json).unwrap_or_default();
    Ok(Some(SummaryResult {
        content,
        source_refs,
        source_hash: req
            .sources
            .first()
            .map(|s| s.hash.clone())
            .unwrap_or_default(),
        model,
        prompt_version: req.prompt_version.clone(),
        coverage,
        source_hashes: req.sources.iter().map(|s| s.hash.clone()).collect(),
        provider: req.provider.clone(),
        preprocessing: req.preprocessing.clone(),
        fallback: fallback != 0,
    }))
}

/// Prefer an exact compatible model result, then its separately keyed local fallback.
pub fn cache_get(conn: &Connection, req: &SummaryRequest) -> anyhow::Result<Option<SummaryResult>> {
    if let Some(result) = cache_get_exact(conn, req)? {
        return Ok(Some(result));
    }
    let mut local = req.clone();
    local.provider = "local".into();
    local.model = "deterministic".into();
    Ok(cache_get_exact(conn, &local)?.filter(|r| r.fallback))
}

pub fn resolved_summary_route() -> (String, String) {
    let auth = crate::secrets::AuthFile::load().unwrap_or_default();
    let settings = crate::provider::SettingsFile::load().unwrap_or_default();
    crate::recon::model_exec::select_role_secret(
        &auth,
        &settings,
        "summarization",
        crate::recon::model_exec::OperationFormat::Prose,
    )
    .map(|s| (s.kind, s.model))
    .ok_or(())
    .unwrap_or_else(|_| ("local".into(), "deterministic".into()))
}

/// Persist a derived summary for later cache hits.
pub fn cache_put(
    conn: &Connection,
    req: &SummaryRequest,
    result: &SummaryResult,
) -> Result<(), anyhow::Error> {
    anyhow::ensure!(
        req.sources.iter().all(|s| sha_hex(&s.text) == s.hash),
        "summary source hash mismatch"
    );
    let now = chrono::Utc::now().to_rfc3339();
    let id = format!("sum-{}", req.cache_key());
    let focus_hash = sha_hex(&req.focus);
    let source_id = req.sources.first().map(|s| s.id.as_str()).unwrap_or("");
    let source_revision = req
        .sources
        .first()
        .map(|s| s.revision.as_str())
        .unwrap_or("");
    let source_hash = req.sources.first().map(|s| s.hash.as_str()).unwrap_or("");
    conn.execute(
        "INSERT INTO argos_derived_summaries(
            id, mode, source_id, source_revision, source_hash, focus_hash, budget,
            model, provider, prompt_version, content, source_refs_json, coverage_json,
            fallback, created_at, updated_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?15)
         ON CONFLICT(id) DO UPDATE SET
            content=excluded.content,
            source_refs_json=excluded.source_refs_json,
            coverage_json=excluded.coverage_json,
            fallback=excluded.fallback,
            updated_at=excluded.updated_at
         WHERE argos_derived_summaries.fallback=1 OR excluded.fallback=0",
        params![
            id,
            req.mode.as_str(),
            source_id,
            source_revision,
            source_hash,
            focus_hash,
            req.budget_chars as i64,
            req.model,
            req.provider,
            req.prompt_version,
            result.content,
            serde_json::to_string(&result.source_refs)?,
            serde_json::to_string(&result.coverage)?,
            if result.fallback { 1 } else { 0 },
            now,
        ],
    )?;
    conn.execute(
        "UPDATE argos_derived_summaries SET result_json=?2 WHERE id=?1 AND (fallback=1 OR ?3=0)",
        params![
            id,
            serde_json::to_string(result)?,
            i64::from(result.fallback)
        ],
    )?;
    Ok(())
}

/// Persist the full SummaryRequest so a worker can run live LLM upgrade later.
pub fn persist_flush_request(conn: &Connection, req: &SummaryRequest) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS argos_summary_flush_requests (
            cache_key TEXT PRIMARY KEY,
            request_json TEXT NOT NULL,
            created_at TEXT NOT NULL
        );",
    )?;
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO argos_summary_flush_requests(cache_key, request_json, created_at)
         VALUES (?1,?2,?3)
         ON CONFLICT(cache_key) DO UPDATE SET request_json=excluded.request_json, created_at=excluded.created_at",
        params![req.cache_key(), serde_json::to_string(req)?, now],
    )?;
    Ok(())
}

pub fn load_flush_request(
    conn: &Connection,
    cache_key: &str,
) -> anyhow::Result<Option<SummaryRequest>> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS argos_summary_flush_requests (
            cache_key TEXT PRIMARY KEY,
            request_json TEXT NOT NULL,
            created_at TEXT NOT NULL
        );",
    )?;
    let row: Option<String> = conn
        .query_row(
            "SELECT request_json FROM argos_summary_flush_requests WHERE cache_key=?1",
            [cache_key],
            |r| r.get(0),
        )
        .optional()?;
    Ok(match row {
        Some(json) => Some(serde_json::from_str(&json)?),
        None => None,
    })
}

/// When a provider secret is present, run [`complete_summary`] (2-attempt policy)
/// and refresh the cache. Without a secret, returns the deterministic/cached value
/// and does **not** invent network success.
pub fn try_live_summary_upgrade(
    conn: &Connection,
    secret: Option<&ProviderSecret>,
    cache_key: &str,
) -> anyhow::Result<(SummaryResult, &'static str)> {
    let Some(req) = load_flush_request(conn, cache_key)? else {
        anyhow::bail!("flush request missing for {cache_key}");
    };
    let cached = cache_get(conn, &req)?
        .or_else(|| req.deterministic.as_deref().cloned())
        .unwrap_or_else(|| SummaryResult {
            content: if req.prompt_version.ends_with(".v1") {
                fallback_excerpt(
                    &req.sources
                        .iter()
                        .map(|s| s.text.as_str())
                        .collect::<Vec<_>>()
                        .join("\n\n"),
                    req.budget_chars,
                )
            } else {
                String::new()
            },
            source_refs: req.sources.iter().map(|s| s.id.clone()).collect(),
            source_hash: req
                .sources
                .first()
                .map(|s| s.hash.clone())
                .unwrap_or_default(),
            model: "deterministic".into(),
            prompt_version: req.prompt_version.clone(),
            coverage: CoverageMeta::default(),
            source_hashes: Vec::new(),
            provider: "local".into(),
            preprocessing: serde_json::Value::Null,
            fallback: true,
        });
    if !cached.fallback {
        return Ok((cached, "already_upgraded"));
    }
    if req.mode == SummarizationMode::TopologyNeighborhood {
        // Structured generation needs the whole role chain, not one secret:
        // `scheduler::drain_summary_flush_with_runtime` dispatches it. This
        // single-secret path never reports it as upgraded.
        return Ok((cached, "role_dispatch_required"));
    }
    let Some(secret) = secret else {
        return Ok((cached, "cached_deterministic_no_secret"));
    };
    if req.prompt_version.ends_with(".v1") {
        // Legacy requests contain a derived source. Their v1 identity stays isolated.
        let mut legacy = req.clone();
        legacy.model = secret.model.clone();
        legacy.provider = secret.kind.clone();
        legacy.preprocessing = serde_json::json!({"legacy_derived_source":true});
        let upgraded =
            crate::brain_lance::block_on(complete_summary(secret, &legacy, cached.clone()));
        cache_put(conn, &legacy, &upgraded)?;
        return Ok((upgraded, "legacy_derived_source"));
    }
    let upgraded = crate::brain_lance::block_on(complete_summary(secret, &req, cached.clone()));
    cache_put(conn, &req, &upgraded)?;
    let outcome = if upgraded.fallback {
        "llm_fallback"
    } else {
        "llm_upgraded"
    };
    Ok((upgraded, outcome))
}

pub(crate) fn sha_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Deterministic FollowUpContext when a model summary is not ready.
pub fn deterministic_follow_up(raw: &str, budget: usize) -> SummaryResult {
    let content = fallback_excerpt(raw, budget.max(200));
    SummaryResult {
        content,
        source_refs: Vec::new(),
        source_hash: sha_hex(raw),
        model: "deterministic".into(),
        prompt_version: SummarizationMode::FollowUpContext.prompt_version().into(),
        coverage: CoverageMeta {
            partial: raw.chars().count() > budget,
            omitted: Vec::new(),
            notes: "deterministic_follow_up".into(),
        },
        source_hashes: Vec::new(),
        provider: "local".into(),
        preprocessing: serde_json::Value::Null,
        fallback: true,
    }
}

/// Structured ToolObservation digest preserving metadata outside prose.
pub fn deterministic_tool_observation(
    tool_id: &str,
    call_id: &str,
    status: &str,
    observation: &serde_json::Value,
    budget: usize,
) -> SummaryResult {
    let mut meta = serde_json::json!({
        "tool_id": tool_id,
        "call_id": call_id,
        "status": status,
    });
    if let Some(err) = observation.get("error") {
        meta["error"] = err.clone();
    }
    if let Some(total) = observation
        .get("total")
        .or_else(|| observation.get("count"))
    {
        meta["count"] = total.clone();
    }
    let preview = crate::summarization::fallback_excerpt(&observation.to_string(), budget);
    let content = format!("tool={tool_id} call={call_id} status={status}\n{}", preview);
    SummaryResult {
        content,
        source_refs: vec![call_id.into()],
        source_hash: sha_hex(&observation.to_string()),
        model: "deterministic".into(),
        prompt_version: SummarizationMode::ToolObservation.prompt_version().into(),
        coverage: CoverageMeta {
            partial: observation.to_string().chars().count() > budget,
            omitted: Vec::new(),
            notes: meta.to_string(),
        },
        source_hashes: Vec::new(),
        provider: "local".into(),
        preprocessing: serde_json::Value::Null,
        fallback: true,
    }
}

/// Deterministic Atlas brief from accepted claim lines.
pub fn deterministic_atlas_brief(lines: &[String]) -> SummaryResult {
    let content = if lines.is_empty() {
        String::new()
    } else {
        let lead = lines.iter().take(2).cloned().collect::<Vec<_>>().join(". ");
        let mut out = vec![format!("{lead}.")];
        for line in lines {
            out.push(format!("- {line}"));
        }
        out.join("\n")
    };
    SummaryResult {
        content,
        source_refs: Vec::new(),
        source_hash: sha_hex(&lines.join("\n")),
        model: "deterministic".into(),
        prompt_version: SummarizationMode::AtlasBrief.prompt_version().into(),
        coverage: CoverageMeta::default(),
        source_hashes: Vec::new(),
        provider: "local".into(),
        preprocessing: serde_json::Value::Null,
        fallback: true,
    }
}

/// Concise article description fallback (keeps original when short).
pub fn deterministic_article_description(
    title: &str,
    description: &str,
    budget: usize,
) -> SummaryResult {
    let src = if description.trim().is_empty() {
        title.to_string()
    } else {
        format!("{title}. {description}")
    };
    let content = fallback_excerpt(&src, budget.max(80));
    SummaryResult {
        content,
        source_refs: Vec::new(),
        source_hash: sha_hex(&src),
        model: "deterministic".into(),
        prompt_version: SummarizationMode::ArticleDescription
            .prompt_version()
            .into(),
        coverage: CoverageMeta {
            partial: src.chars().count() > budget,
            omitted: Vec::new(),
            notes: "article_description".into(),
        },
        source_hashes: Vec::new(),
        provider: "local".into(),
        preprocessing: serde_json::Value::Null,
        fallback: true,
    }
}

/// ReportContext digest: compress body/upstream for one section without judgments.
pub fn deterministic_report_context(label: &str, text: &str, budget: usize) -> SummaryResult {
    let content = format!("{label}: {}", fallback_excerpt(text, budget));
    SummaryResult {
        content,
        source_refs: Vec::new(),
        source_hash: sha_hex(text),
        model: "deterministic".into(),
        prompt_version: SummarizationMode::ReportContext.prompt_version().into(),
        coverage: CoverageMeta {
            partial: text.chars().count() > budget,
            omitted: Vec::new(),
            notes: "report_context".into(),
        },
        source_hashes: Vec::new(),
        provider: "local".into(),
        preprocessing: serde_json::Value::Null,
        fallback: true,
    }
}

/// SectionDigest from completed markdown (does not alter conclusions).
pub fn deterministic_section_digest(
    section_key: &str,
    markdown: &str,
    budget: usize,
) -> SummaryResult {
    let content = fallback_excerpt(markdown, budget.max(120));
    SummaryResult {
        content,
        source_refs: vec![section_key.into()],
        source_hash: sha_hex(markdown),
        model: "deterministic".into(),
        prompt_version: SummarizationMode::SectionDigest.prompt_version().into(),
        coverage: CoverageMeta {
            partial: markdown.chars().count() > budget,
            omitted: Vec::new(),
            notes: "section_digest".into(),
        },
        source_hashes: Vec::new(),
        provider: "local".into(),
        preprocessing: serde_json::Value::Null,
        fallback: true,
    }
}

/// Enqueue a background LLM polish for a mode that already published a
/// deterministic (or cached) result. Uses the shared task schema and
/// summarization attempt cap (2). Dedupes on the request cache key.
pub fn enqueue_summary_flush(conn: &Connection, req: &SummaryRequest) -> anyhow::Result<bool> {
    use crate::tasks::{enqueue_job, enqueue_task, NewJob, NewTask};
    let now = chrono::Utc::now().to_rfc3339();
    let key = req.cache_key();
    let job_id = format!("sum-job-{key}");
    let task_id = format!("sum-task-{key}");
    enqueue_job(
        conn,
        &NewJob {
            id: job_id.clone(),
            kind: "summarization_flush".into(),
            owner_scope: req.mode.as_str().into(),
            input_revision: req
                .sources
                .first()
                .map(|s| s.revision.clone())
                .unwrap_or_default(),
            deadline_at: String::new(),
        },
        &now,
    )?;
    enqueue_task(
        conn,
        &NewTask {
            id: task_id,
            job_id,
            operation: req.mode.as_str().into(),
            dedupe_key: format!("flush:{key}"),
            priority: 50,
            input_ref: req.focus.clone(),
            input_hash: key,
            source_revision: req
                .sources
                .first()
                .map(|s| s.revision.clone())
                .unwrap_or_default(),
            role_snapshot: "summarization".into(),
            max_attempts: OperationKind::Summarization.attempt_cap(),
        },
        &now,
    )
}

/// Cache a deterministic result immediately and enqueue a background flush task
/// under the shared 2-attempt summarization policy. Synchronous callers keep the
/// deterministic text; an elected LLM worker may later upgrade the cache via
/// [`complete_summary`] when a provider secret is available.
pub fn publish_deterministic_and_enqueue(
    conn: &Connection,
    req: &SummaryRequest,
    deterministic: SummaryResult,
) -> anyhow::Result<SummaryResult> {
    let mut req = req.clone();
    // Factory-created requests already carry the route snapshot. Legacy/direct
    // local envelopes resolve once here, before durable identity is calculated.
    if req.provider.is_empty() {
        let (provider, model) = resolved_summary_route();
        req.provider = provider;
        req.model = model;
    }
    if let Some(hit) = cache_get(conn, &req)? {
        if !hit.fallback {
            return Ok(hit);
        }
    }
    let mut deterministic = deterministic;
    deterministic.source_hashes = req.sources.iter().map(|s| s.hash.clone()).collect();
    req.deterministic = Some(Box::new(deterministic.clone()));
    let mut det_req = req.clone();
    det_req.provider = "local".into();
    det_req.model = "deterministic".into();
    cache_put(conn, &det_req, &deterministic)?;
    persist_flush_request(conn, &req)?;
    let _ = enqueue_summary_flush(conn, &req)?;
    Ok(deterministic)
}

/// Build a minimal request for a deterministic mode publish.
pub fn flush_request(
    mode: SummarizationMode,
    source_id: &str,
    revision: &str,
    text: &str,
    focus: &str,
    budget_chars: usize,
) -> SummaryRequest {
    let (provider, model) = resolved_summary_route();
    SummaryRequest {
        mode,
        sources: vec![SummarySource {
            id: source_id.into(),
            revision: revision.into(),
            hash: sha_hex(text),
            text: text.into(),
            meta: serde_json::json!({}),
        }],
        focus: focus.into(),
        budget_chars,
        required_fields: Vec::new(),
        model,
        provider,
        prompt_version: mode.prompt_version().into(),
        preprocessing: serde_json::json!({"feature_key":crate::nlp::cache::feature_key(&crate::nlp::Source::derived(source_id,revision,text),&crate::nlp::ParseContext::default()),"selection_policy":"passage-v1","mode":crate::nlp::configured_mode()}),
        deterministic: None,
    }
}

/// Run a summarization completion under the shared attempt/admission policy.
/// Returns deterministic fallback content when the model cannot produce valid text.
pub async fn complete_summary(
    secret: &ProviderSecret,
    req: &SummaryRequest,
    fallback: SummaryResult,
) -> SummaryResult {
    if req.sources.iter().any(|s| sha_hex(&s.text) != s.hash) {
        return fallback;
    }
    // Snapshot route changes invalidate queued requests, rather than executing on a new route.
    if req.model != "deterministic" && (req.model != secret.model || req.provider != secret.kind) {
        return fallback;
    }
    let mode = req
        .preprocessing
        .get("mode")
        .and_then(|v| serde_json::from_value::<crate::nlp::Mode>(v.clone()).ok())
        .unwrap_or_else(crate::nlp::configured_mode);
    let account = format!("{}:{}", req.provider, secret.kind);
    let system = system_prompt(req.mode);
    let mut parts = vec![format!(
        "Focus: {}\nOutput target (characters, soft): {}\nRequired fields: {}",
        req.focus,
        req.budget_chars,
        req.required_fields.join(", ")
    )];
    let mut manifests = Vec::new();
    let mut omitted = Vec::new();
    let overhead = serde_json::to_vec(&parts).map_or(0, |v| v.len()) + system.len() + 1024;
    let each_budget = (48 * 1024usize).saturating_sub(overhead) / req.sources.len().max(1);
    for source in &req.sources {
        let s = crate::nlp::Source::derived(&source.id, &source.revision, &source.text);
        let prepared = crate::nlp::prepare_mode(
            s,
            "summary",
            &req.focus,
            each_budget,
            each_budget / 4,
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            mode,
        )
        .await;
        if let Some(selection) = &prepared.selection {
            omitted.extend(selection.omitted_sentence_ids.clone());
        } else if prepared.text != source.text {
            omitted.push(source.id.clone());
        }
        manifests.push(serde_json::json!({"source_id":source.id,"hash":source.hash,"selection":prepared.selection,"diagnostic":prepared.diagnostic}));
        parts.push(format!(
            "Source {}@{} metadata {}:\n{}",
            source.id, source.revision, source.meta, prepared.text
        ));
    }
    let user = parts.join("\n\n");
    let messages = [
        ChatMessage {
            role: "system".into(),
            content: system.into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
        ChatMessage {
            role: "user".into(),
            content: user,
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
    ];
    if crate::nlp::message_bytes(&messages, &serde_json::Value::Null) > 48 * 1024 {
        return fallback;
    }
    let known: Vec<String> = req.sources.iter().map(|s| s.id.clone()).collect();
    let mut opts = ExecOptions::for_secret(secret);
    opts.admission_account = account;
    let exec = complete_summary_report(secret, req.mode, &messages, &known, &opts, |_| {}).await;
    match exec.content {
        Some(text) if req.required_fields.iter().all(|f| text.contains(f)) => SummaryResult {
            content: text,
            source_refs: known,
            source_hash: req
                .sources
                .first()
                .map(|s| s.hash.clone())
                .unwrap_or_default(),
            model: secret.model.clone(),
            prompt_version: req.prompt_version.clone(),
            coverage: CoverageMeta {
                partial: !omitted.is_empty(),
                omitted,
                notes: "bounded source view".into(),
            },
            source_hashes: req.sources.iter().map(|s| s.hash.clone()).collect(),
            provider: secret.kind.clone(),
            preprocessing: serde_json::json!(manifests),
            fallback: false,
        },
        _ => fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_summarization_mode_round_trips() {
        assert_eq!(SummarizationMode::ALL.len(), 9);
        for mode in SummarizationMode::ALL {
            assert_eq!(SummarizationMode::parse(mode.as_str()), Some(mode));
            assert!(!system_prompt(mode).is_empty());
        }
    }

    #[test]
    fn validate_rejects_empty_and_unknown_ids() {
        assert!(validate_result(SummarizationMode::AtlasBrief, "  ", &[]).is_err());
        assert!(validate_result(
            SummarizationMode::PageEvidence,
            "See [call-missing] for details that are long enough.",
            &["call-1".into()]
        )
        .is_err());
        assert!(validate_result(
            SummarizationMode::PageEvidence,
            "See [call-1] for details that are long enough.",
            &["call-1".into()]
        )
        .is_ok());
    }

    #[test]
    fn cache_key_changes_with_revision_and_focus() {
        let mut req = SummaryRequest {
            mode: SummarizationMode::PageEvidence,
            sources: vec![SummarySource {
                id: "m1".into(),
                revision: "1".into(),
                hash: sha_hex("x"),
                text: "x".into(),
                meta: serde_json::json!({}),
            }],
            focus: "path-a".into(),
            budget_chars: 800,
            required_fields: vec![],
            model: "m".into(),
            provider: "p".into(),
            prompt_version: "summarization.v2".into(),
            preprocessing: serde_json::Value::Null,
            deterministic: None,
        };
        let a = req.cache_key();
        req.focus = "path-b".into();
        assert_ne!(a, req.cache_key());
        req.focus = "path-a".into();
        req.sources[0].revision = "2".into();
        assert_ne!(a, req.cache_key());
    }

    #[test]
    fn fallbacks_are_non_empty() {
        assert_eq!(
            fallback_title("who runs Example Org today?"),
            "who runs Example Org today?"
        );
        assert!(fallback_excerpt("abcdefghij", 6).ends_with('…'));
    }
}

#[cfg(test)]
mod service_tests {
    use super::*;
    use crate::tasks::migrate_tables;
    use rusqlite::Connection;

    #[test]
    fn try_live_without_secret_keeps_deterministic() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_tables(&conn).unwrap();
        let req = flush_request(
            SummarizationMode::ToolObservation,
            "call-1",
            "1",
            "tool digest",
            "news",
            200,
        );
        let det = deterministic_tool_observation(
            "news",
            "call-1",
            "ok",
            &serde_json::json!({"a": 1}),
            200,
        );
        publish_deterministic_and_enqueue(&conn, &req, det).unwrap();
        let (out, tag) = try_live_summary_upgrade(&conn, None, &req.cache_key()).unwrap();
        assert!(out.fallback);
        assert_eq!(tag, "cached_deterministic_no_secret");
    }

    #[test]
    fn publish_enqueues_deduped_flush_task() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_tables(&conn).unwrap();
        let req = flush_request(
            SummarizationMode::AtlasBrief,
            "brief-1",
            "1",
            "claim line",
            "atlas",
            400,
        );
        let det = deterministic_atlas_brief(&["claim line".into()]);
        let out = publish_deterministic_and_enqueue(&conn, &req, det).unwrap();
        assert!(out.fallback);
        assert!(cache_get(&conn, &req).unwrap().is_some());
        // Second publish coalesces on dedupe key.
        let again = enqueue_summary_flush(&conn, &req).unwrap();
        assert!(!again);
    }

    #[test]
    fn cache_round_trip() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_tables(&conn).unwrap();
        let req = SummaryRequest {
            mode: SummarizationMode::FollowUpContext,
            sources: vec![SummarySource {
                id: "ans-1".into(),
                revision: "1".into(),
                hash: sha_hex("hello"),
                text: "hello".into(),
                meta: serde_json::json!({}),
            }],
            focus: "thread-1".into(),
            budget_chars: 400,
            required_fields: vec![],
            model: "m".into(),
            provider: "p".into(),
            prompt_version: "summarization.v2".into(),
            preprocessing: serde_json::Value::Null,
            deterministic: None,
        };
        assert!(cache_get(&conn, &req).unwrap().is_none());
        let result = deterministic_follow_up("hello world from prior answer", 400);
        cache_put(&conn, &req, &result).unwrap();
        let hit = cache_get(&conn, &req).unwrap().unwrap();
        assert!(hit.fallback);
        assert!(hit.content.contains("hello"));
    }

    #[test]
    fn all_remaining_mode_fallbacks_non_empty() {
        assert!(!deterministic_tool_observation(
            "news",
            "call-1",
            "ok",
            &serde_json::json!({"a":1}),
            100
        )
        .content
        .is_empty());
        assert!(
            !deterministic_atlas_brief(&["Claim one.".into(), "Claim two.".into()])
                .content
                .is_empty()
        );
        assert!(
            !deterministic_article_description("Title", "Long description text here", 40)
                .content
                .is_empty()
        );
        assert!(!deterministic_report_context("body", &"x".repeat(500), 80)
            .content
            .is_empty());
        assert!(
            !deterministic_section_digest("bluf", "# BLUF\n\nJudgment here.", 40)
                .content
                .is_empty()
        );
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    #[test]
    fn all_source_hashes_routes_fields_and_upgrade_guard() {
        let store = crate::store::Store::memory().unwrap();
        let conn = &store.conn;
        let mut req = flush_request(
            SummarizationMode::ReportContext,
            "one",
            "1",
            "original full text",
            "focus",
            400,
        );
        req.sources.push(SummarySource {
            id: "two".into(),
            revision: "1".into(),
            hash: sha_hex("second text"),
            text: "second text".into(),
            meta: serde_json::json!({"title":"two"}),
        });
        let mut result = deterministic_report_context("fixture", "original full text", 400);
        cache_put(conn, &req, &result).unwrap();
        assert!(cache_get(conn, &req).unwrap().is_some());
        let base = req.cache_key();
        let mut changed = req.clone();
        changed.sources[1].text = "changed second text".into();
        changed.sources[1].hash = sha_hex(&changed.sources[1].text);
        assert!(cache_get(conn, &changed).unwrap().is_none());
        changed = req.clone();
        changed.provider = "other".into();
        assert_ne!(base, changed.cache_key());
        changed = req.clone();
        changed.required_fields.push("evidence".into());
        assert_ne!(base, changed.cache_key());
        changed = req.clone();
        changed.sources[1].meta = serde_json::json!({"title":"changed"});
        assert_ne!(base, changed.cache_key());
        changed = req.clone();
        changed.sources[1].hash = "false".into();
        assert!(cache_put(conn, &changed, &result).is_err());
        result.fallback = false;
        result.content = "upgraded result preserves full qualification".into();
        result.source_hashes = req.sources.iter().map(|s| s.hash.clone()).collect();
        cache_put(conn, &req, &result).unwrap();
        let det = deterministic_report_context("fixture", "short", 400);
        cache_put(conn, &req, &det).unwrap();
        assert_eq!(
            cache_get(conn, &req).unwrap().unwrap().content,
            result.content
        );
        let published = publish_deterministic_and_enqueue(conn, &req, det).unwrap();
        assert_eq!(published.content, result.content);
        let mut old = serde_json::to_value(&req).unwrap();
        old.as_object_mut().unwrap().remove("preprocessing");
        old.as_object_mut().unwrap().remove("deterministic");
        old["prompt_version"] = serde_json::json!("summarization.v1");
        let legacy: SummaryRequest = serde_json::from_value(old).unwrap();
        assert_ne!(req.cache_key(), legacy.cache_key());
    }
}

#[cfg(all(test, feature = "nlp"))]
mod preparation_tests {
    use super::*;
    use crate::provider_attempt::mock;
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn modes_preserve_request_count_and_tail_evidence() {
        let text=format!("Alice Smith attributed the Acme project announcement.\n\n{}Correction: Alice Smith denied the $1.2 billion estimate, not $2 billion.\n\nThe estimate remains uncertain.","Background details about unrelated page material.\n\n".repeat(1050));
        assert!(text.len() > 48000);
        let mut wire_sizes = Vec::new();
        for mode in [
            crate::nlp::Mode::Off,
            crate::nlp::Mode::Annotate,
            crate::nlp::Mode::Select,
            crate::nlp::Mode::Auto,
        ] {
            let server = mock::serve(vec![mock::ok_json(
                "[source-one] Alice Smith denied the estimate; the report remains uncertain.",
                "stop",
            )])
            .await;
            let secret = ProviderSecret {
                kind: "local".into(),
                model: "mock-model".into(),
                base_url: server.base_url.clone(),
                api_key: Some("fixture-only".into()),
                stt_model: None,
                device: None,
            };
            let mut req = flush_request(
                SummarizationMode::PageEvidence,
                "source-one",
                "1",
                &text,
                "Acme",
                400,
            );
            req.provider = secret.kind.clone();
            req.model = secret.model.clone();
            req.required_fields = vec!["source-one".into()];
            req.preprocessing["mode"] = serde_json::json!(mode);
            let fallback = deterministic_follow_up(&text, 400);
            let result = complete_summary(&secret, &req, fallback).await;
            assert!(!result.fallback, "{mode:?}: {:?}", result.coverage);
            assert_eq!(server.hits(), 1, "{mode:?}");
            let requests = server.requests.lock().unwrap();
            let body = requests[0].split("\r\n\r\n").nth(1).unwrap();
            wire_sizes.push(body.len());
            if matches!(mode, crate::nlp::Mode::Select | crate::nlp::Mode::Auto) {
                assert!(body.contains("denied the $1.2 billion"));
                assert!(body.contains("attributed"));
                assert!(result.coverage.partial);
            }
            assert_eq!(result.source_hashes, vec![sha_hex(&text)]);
        }
        assert_eq!(wire_sizes[0], wire_sizes[1]);
        assert!(wire_sizes[2] < wire_sizes[0], "{wire_sizes:?}");
        assert!(wire_sizes[3] < wire_sizes[0], "{wire_sizes:?}");
        eprintln!("summary gold source_bytes={} off_wire_bytes={} annotate_wire_bytes={} select_wire_bytes={} auto_wire_bytes={} requests_per_mode=1",text.len(),wire_sizes[0],wire_sizes[1],wire_sizes[2],wire_sizes[3]);
    }
}

/// [`publish_deterministic_and_enqueue`] on the SurrealDB summary queue
/// (`storage::summary_flush::publish`) when SurrealDB is selected; `None`
/// otherwise, and the caller keeps its SQLite path. Nothing here opens
/// SQLite. From synchronous code the publish completes before returning
/// (a cached model result is returned); inside an async context it is
/// queued on the storage runtime and `deterministic` is returned. A failed
/// publish returns `deterministic`.
pub fn publish_selected(
    req: &SummaryRequest,
    deterministic: SummaryResult,
) -> Option<SummaryResult> {
    if !crate::recon::backend::surreal_selected() {
        return None;
    }
    #[cfg(feature = "surreal")]
    {
        let req = req.clone();
        if tokio::runtime::Handle::try_current().is_ok() {
            let det = deterministic.clone();
            crate::storage::runtime::spawn(async move {
                if let Ok(repo) = crate::storage::runtime::repository().await {
                    let _ = crate::storage::summary_flush::publish(&repo, &req, det).await;
                }
            });
            return Some(deterministic);
        }
        let det = deterministic.clone();
        Some(
            crate::storage::runtime::blocking(move |repo| async move {
                crate::storage::summary_flush::publish(&repo, &req, det).await
            })
            .unwrap_or(deterministic),
        )
    }
    #[cfg(not(feature = "surreal"))]
    {
        let _ = req;
        Some(deterministic)
    }
}
