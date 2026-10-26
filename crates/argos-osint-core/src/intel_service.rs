//! UI-independent Intel read/action service. No terminal or HTTP types.
//! @tag:intel-brief
use crate::{
    storage::{atlas, brief_stages, intel_brief_view, repo::Repository},
    store::AtlasArticleRow,
};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const CATEGORIES: [&str; 6] = [
    "geopolitical",
    "economic",
    "military",
    "information",
    "stability",
    "technology",
];

pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 512 && !id.chars().any(|c| c.is_control())
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ArticleQuery {
    pub category: String,
    pub day: String,
    pub search: String,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}
impl ArticleQuery {
    pub fn validate(&self) -> Result<()> {
        if !CATEGORIES.contains(&self.category.as_str()) {
            bail!("invalid category");
        }
        if chrono::NaiveDate::parse_from_str(&self.day, "%Y-%m-%d").is_err() || self.day.len() != 10
        {
            bail!("invalid day");
        }
        if self.search.len() > 256 || !(1..=100).contains(&self.limit.unwrap_or(30)) {
            bail!("invalid search or limit");
        }
        if let Some(c) = &self.cursor {
            if c.len() > 4096 {
                bail!("invalid cursor");
            }
            let _: (String, String, String, String) = serde_json::from_str(c)?;
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct ArticlePage {
    pub rows: Vec<AtlasArticleRow>,
    pub next_cursor: Option<String>,
    pub revision: String,
}

/// Keyset pagination over article occurrences, ordered deterministically. No
/// list_runs/list_articles loop and no offset scan. Search stays inside a day/category.
pub async fn articles(repo: &Repository, q: &ArticleQuery) -> Result<ArticlePage> {
    q.validate()?;
    let after: (String, String, String, String) = q
        .cursor
        .as_deref()
        .map(serde_json::from_str)
        .transpose()?
        .unwrap_or_default();
    let n = q.limit.unwrap_or(30);
    let mut response = repo.db().query(
        "SELECT * FROM atlas_article WHERE category = $category
         AND run_id IN (SELECT VALUE record::id(id) FROM atlas_run WHERE started_at >= $start AND started_at < $end)
         AND ($search = '' OR string::contains(string::lowercase(string::concat(title, ' ', description, ' ', source_name, ' ', source_domain, ' ', url)), $search))
         AND ($first OR published_at < $p OR (published_at = $p AND seen_at < $s)
           OR (published_at = $p AND seen_at = $s AND article_id > $a)
           OR (published_at = $p AND seen_at = $s AND article_id = $a AND run_id > $r))
         ORDER BY published_at DESC, seen_at DESC, article_id, run_id LIMIT $limit"
    ).bind(("category", q.category.clone())).bind(("start", q.day.clone()))
      .bind(("end", format!("{}~", q.day))).bind(("search", q.search.trim().to_lowercase()))
      .bind(("first", q.cursor.is_none())).bind(("p", after.0)).bind(("s", after.1))
      .bind(("a", after.2)).bind(("r", after.3)).bind(("limit", (n + 1) as i64)).await?.check()?;
    let raw: Vec<crate::storage::atlas::ArticleRow> = response.take(0)?;
    let mut rows: Vec<AtlasArticleRow> = raw.into_iter().map(Into::into).collect();
    let more = rows.len() > n;
    rows.truncate(n);
    let next_cursor = if more {
        rows.last()
            .map(|a| serde_json::to_string(&(&a.published_at, &a.seen_at, &a.id, &a.run_id)))
            .transpose()?
    } else {
        None
    };
    let revision = crate::nlp::cache::hash(&serde_json::to_string(&rows)?);
    Ok(ArticlePage {
        rows,
        next_cursor,
        revision,
    })
}

pub async fn article(repo: &Repository, run: &str, id: &str) -> Result<Option<AtlasArticleRow>> {
    if !valid_id(run) || !valid_id(id) {
        bail!("invalid article identifier");
    }
    Ok(atlas::article(repo.db(), run, id).await?)
}

/// Immutable projection shared by any non-terminal client. Retained publication,
/// extraction and indexing are reported separately; absence is never success.
pub async fn brief(repo: &Repository, article: &AtlasArticleRow) -> Result<Value> {
    let source_revision = crate::atlas_metadata::input_hash(article);
    let mut parts = intel_brief_view::brief_parts(repo, article).await?;
    parts.relations.sort();
    let mut actors = parts
        .metadata
        .as_ref()
        .map(|m| crate::atlas_metadata::article_actors(article, m))
        .unwrap_or_default();
    if let Some(output) = &parts.output {
        crate::atlas_metadata::merge_output_actors(&mut actors, &output.entities);
    }
    let links = crate::intel_recon::bucket_extracted_with_explanations(
        &parts.claims,
        &parts.relations,
        &parts.explanations,
    )
    .links;
    let confidence = parts
        .output
        .as_ref()
        .map(|output| crate::intel_recon::EvidenceConfidence::estimate(output, false));
    let mut claims: Vec<Value> = parts.claims.iter().map(|c| json!({"id":c.fingerprint,"text":c.claim,"classification":c.classification,"confidence":c.confidence,"admiralty":c.admiralty,"source_url":c.source_url})).collect();
    let published = crate::storage::brief::published_brief(repo, &article.id).await?;
    if claims.is_empty() {
        if let Some((_, retained, _)) = &published {
            let mut r = repo.db().query("SELECT VALUE job_id FROM ONLY type::record('intel_brief_publication', $article)").bind(("article", article.id.clone())).await?.check()?;
            let published_job: Option<String> = r.take(0)?;
            if let Some(items) = retained.get("claims").and_then(Value::as_array) {
                for item in items {
                    let text = item["text"].as_str().unwrap_or_default();
                    let local_id = item["id"].as_str().unwrap_or_default();
                    let seed = crate::brain_topology::TopologySeed::RetainedClaim {
                        run_id: article.run_id.clone(),
                        article_id: article.id.clone(),
                        revision: source_revision.clone(),
                        local_id: local_id.into(),
                        text: text.into(),
                        job_id: published_job.clone().unwrap_or_default(),
                    };
                    claims.push(json!({"id":local_id,"text":text,"classification":"retained claim","confidence":null,"admiralty":"unassessed","topology_seed":seed}));
                }
            }
        }
    }
    let mut response = repo.db().query("RETURN (SELECT record::id(id) AS job_id, generation FROM intel_brief_run WHERE article_id = $article ORDER BY generation DESC LIMIT 1)[0].job_id").bind(("article", article.id.clone())).await?.check()?;
    let job_id: Option<String> = response.take(0)?;
    let mut body_pending = false;
    if let Some(job) = &job_id {
        let task = brief_stages::task_id(job, "intel_brief_body");
        let mut r = repo
            .db()
            .query("SELECT VALUE input_json FROM ONLY type::record('brief_stage_input', $task)")
            .bind(("task", task))
            .await?
            .check()?;
        let input: Option<String> = r.take(0)?;
        let force = input
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .is_some_and(|v| v["force_body"] == true);
        if force
            && brief_stages::stage_views(repo, job)
                .await?
                .iter()
                .any(|s| s.operation == "intel_brief_body" && s.state != "completed")
        {
            parts.body = None;
            body_pending = true;
        }
    }
    let mut indexing = if let Some(job) = &job_id {
        crate::storage::brief::retained_canonical_state(repo, &article.id, job, "claim").await?
    } else {
        None
    };
    if indexing.is_none() {
        let mut r = repo.db().query("SELECT VALUE state_json FROM ONLY type::record('intel_brief_index_state', $article)").bind(("article", article.id.clone())).await?.check()?;
        let raw: Option<String> = r.take(0)?;
        let current = raw
            .map(|raw| {
                serde_json::from_str::<crate::intel_recon::brief_canonical::IndexState>(&raw)
            })
            .transpose()?;
        indexing = current.filter(|state| Some(&state.job_id) == job_id.as_ref());
    }
    let mut value = json!({"article":article,"source_revision":source_revision,"actors": actors,"claims":claims,"body":parts.body,"output":parts.output,"lifecycle":parts.lifecycle,"status":crate::provider_diag::sanitize(&parts.status),"publication":published,"relations":parts.relations,"links":links,"job_id":job_id,"indexing":indexing,"confidence":confidence,"body_pending":body_pending});
    let revision = crate::nlp::cache::hash(&serde_json::to_string(&value)?);
    value["revision"] = json!(revision);
    Ok(value)
}

pub async fn ensure_brief(repo: &Repository, article: &AtlasArticleRow) -> Result<String> {
    brief_stages::ensure_for_article(repo, article, &crate::provider::SettingsFile::load()?).await
}

pub async fn related(
    repo: &Repository,
    article: &AtlasArticleRow,
    cursor: Option<&str>,
    limit: usize,
) -> Result<Value> {
    if !(1..=100).contains(&limit)
        || cursor.is_some_and(|s| s.len() > 20 || s.parse::<usize>().is_err())
    {
        bail!("invalid related page");
    }
    let page = crate::related_global::related_articles_global_on(
        repo,
        &article.run_id,
        &article.id,
        crate::related_global::RelatedScope::AllRetained,
        cursor,
        limit,
    )
    .await?;
    let rows: Vec<Value> = page.rows.into_iter().map(|r| json!({"article":r.article,"basis":r.basis,"tentative":r.tentative,"cycles":r.cycles})).collect();
    Ok(
        json!({"rows": rows,"next_cursor":page.next_cursor,"total":page.total,"label":"Related leads · not corroboration"}),
    )
}

/// A caller supplied reload key makes network retries idempotent. A new explicit
/// reload uses a new key and fences the previous generation via ensure_chain.
pub async fn reload_brief(
    repo: &Repository,
    article: &AtlasArticleRow,
    key: &str,
) -> Result<String> {
    if key.len() < 8
        || key.len() > 128
        || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        bail!("invalid reload key");
    }
    static RELOAD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = RELOAD.lock().await;
    let mut existing = repo.db().query("RETURN (SELECT record::id(job) AS job_id, generation FROM brief_run WHERE article_id = $article AND string::ends_with(input_revision, $suffix) ORDER BY generation DESC LIMIT 1)[0].job_id")
        .bind(("article", article.id.clone())).bind(("suffix", format!(":reload:{key}"))).await?.check()?;
    let existing: Option<String> = existing.take(0)?;
    if let Some(job) = existing {
        return Ok(job);
    }
    let settings = crate::provider::SettingsFile::load()?;
    let bs = &crate::intel_recon::brief_stages::input_revision(article, &settings);
    let inputs = crate::intel_recon::brief_pipeline::BriefInputs {
        article_id: article.id.clone(),
        input_revision: format!("{bs}:reload:{key}"),
        role_snapshot: serde_json::to_string(&settings.defaults)?,
        title: article.title.clone(),
        stage_revisions: Vec::new(),
        metadata: crate::intel_recon::brief_stages::metadata_snapshot(article),
    };
    Ok(brief_stages::ensure(repo, &inputs).await?.job_id)
}
