//! UI-independent Intel read/action service. No terminal or HTTP types.
//! @tag:intel-brief
use crate::{
    storage::{atlas, brief_stages, intel_brief_view, repo::Repository},
    store::AtlasArticleRow,
};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

pub const CATEGORIES: [&str; 6] = crate::atlas_taxonomy::DOMAIN_IDS;

pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 512 && !id.chars().any(|c| c.is_control())
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ArticleQuery {
    pub category: String,
    pub day: String,
    pub search: String,
    /// Empty/latest orders by publication time; hottest orders by temperature first.
    pub sort: String,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}
impl ArticleQuery {
    pub fn validate(&self) -> Result<()> {
        if self.category != "all"
            && crate::atlas_taxonomy::DomainId::parse(&self.category).is_none()
        {
            bail!("invalid category");
        }
        if chrono::NaiveDate::parse_from_str(&self.day, "%Y-%m-%d").is_err() || self.day.len() != 10
        {
            bail!("invalid day");
        }
        if !matches!(self.sort.as_str(), "" | "latest" | "hottest") {
            bail!("invalid sort");
        }
        if self.search.len() > 256 || !(1..=100).contains(&self.limit.unwrap_or(30)) {
            bail!("invalid search or limit");
        }
        if let Some(c) = &self.cursor {
            if c.len() > 4096 {
                bail!("invalid cursor");
            }
            if self.sort == "hottest" {
                let _: (f64, String, String, String, String) = serde_json::from_str(c)?;
            } else {
                let _: (String, String, String, String) = serde_json::from_str(c)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct ArticlePage {
    #[serde(serialize_with = "serialize_current_articles")]
    pub rows: Vec<AtlasArticleRow>,
    pub next_cursor: Option<String>,
    pub revision: String,
    pub coverage: crate::storage::classifier_admission::TaxonomyCoverage,
}

/// Adds indexed classification identity without expanding the retained row DTO.
fn serialize_current_articles<S: serde::Serializer>(
    rows: &[AtlasArticleRow],
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    struct CurrentArticle<'a> {
        #[serde(flatten)]
        article: &'a AtlasArticleRow,
        taxonomy_version: &'static str,
        admission_state: &'static str,
        publication_version: &'static str,
    }
    rows.iter()
        .map(|article| CurrentArticle {
            article,
            taxonomy_version: crate::atlas_taxonomy::TAXONOMY_VERSION,
            admission_state: "admitted",
            publication_version: crate::storage::classifier_admission::PUBLICATION_VERSION,
        })
        .collect::<Vec<_>>()
        .serialize(serializer)
}

/// Deduplicate article occurrences by identity, newest occurrence first
/// (published/seen descending, then id): the deduplicated All/catalog order.
pub(crate) fn dedup_newest(mut rows: Vec<AtlasArticleRow>) -> Vec<AtlasArticleRow> {
    rows.sort_by(|a, b| {
        b.published_at
            .cmp(&a.published_at)
            .then_with(|| b.seen_at.cmp(&a.seen_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut identities = std::collections::HashSet::new();
    rows.retain(|row| identities.insert(row.id.clone()));
    rows
}

/// Membership counts over deduplicated identities: per-category counts
/// overlap (one article counts in every tag it carries) and the `all` key is
/// the distinct admitted total.
pub(crate) fn overlap_counts(rows: &[AtlasArticleRow]) -> std::collections::BTreeMap<String, u64> {
    let mut counts = std::collections::BTreeMap::new();
    for row in rows {
        for tag in &row.categories {
            *counts.entry(tag.clone()).or_default() += 1;
        }
        if !row.categories.is_empty() {
            *counts.entry("all".to_string()).or_default() += 1;
        }
    }
    counts
}

/// Keyset pagination over one retained day's admitted articles, deduplicated
/// by identity (the newest occurrence of an article id wins), ordered
/// deterministically. A day's rows are bounded by retention, so the fetch
/// loads the day once and dedupes in Rust; category filters use ordered-array
/// membership, and per-category counts overlap while All totals deduplicate.
pub async fn articles(repo: &Repository, q: &ArticleQuery) -> Result<ArticlePage> {
    q.validate()?;
    let hottest = q.sort == "hottest";
    let (heat, after) = if hottest {
        let (heat, p, s, a, r): (f64, String, String, String, String) = q
            .cursor
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?
            .unwrap_or_default();
        (heat, (p, s, a, r))
    } else {
        (
            0.0,
            q.cursor
                .as_deref()
                .map(serde_json::from_str)
                .transpose()?
                .unwrap_or_default(),
        )
    };
    let n = q.limit.unwrap_or(30);
    let scope = crate::storage::classifier_admission::CoverageFilter {
        run_day: Some(q.day.clone()),
        text: Some(q.search.trim().to_lowercase()),
        ..Default::default()
    };
    let coverage = crate::storage::classifier_admission::coverage_filtered(repo, &scope).await?;
    let sql=format!("SELECT * FROM atlas_article WHERE ({}) AND ({}) AND ($category='all' OR category=$category OR categories CONTAINS $category) ORDER BY published_at DESC,seen_at DESC,article_id,run_id", scope.predicate(),atlas::CURRENT_ADMISSION_FILTER);
    let mut response = repo
        .db()
        .query(sql)
        .bind(("scope_started", q.day.clone()))
        .bind(("scope_started_end", format!("{}~", q.day)))
        .bind(("scope_text", q.search.trim().to_lowercase()))
        .bind(("category", q.category.clone()))
        .bind(("taxonomy", crate::atlas_taxonomy::TAXONOMY_VERSION))
        .bind((
            "publication",
            crate::storage::classifier_admission::PUBLICATION_VERSION,
        ))
        .await?
        .check()?;
    let raw: Vec<crate::storage::atlas::ArticleRow> = response.take(0)?;
    let mut rows: Vec<AtlasArticleRow> = dedup_newest(raw.into_iter().map(Into::into).collect());
    if hottest {
        // The fetch orders newest-first; the hot ordering re-sorts the
        // deduplicated page set by temperature first, ties deterministic.
        rows.sort_by(|a, b| {
            b.temperature
                .partial_cmp(&a.temperature)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.published_at.cmp(&a.published_at))
                .then_with(|| b.seen_at.cmp(&a.seen_at))
                .then_with(|| a.id.cmp(&b.id))
                .then_with(|| a.run_id.cmp(&b.run_id))
        });
    }
    // The keyset cursor is the last emitted row's sort tuple; a row passes
    // when it sorts strictly after it. Hot ordering puts temperature first.
    use std::cmp::Reverse;
    let mut page: Vec<AtlasArticleRow> = Vec::new();
    for row in &rows {
        let passed = if q.cursor.is_none() {
            true
        } else if hottest {
            (
                Reverse(row.temperature),
                Reverse(row.published_at.as_str()),
                Reverse(row.seen_at.as_str()),
                row.id.as_str(),
                row.run_id.as_str(),
            ) > (
                Reverse(heat),
                Reverse(after.0.as_str()),
                Reverse(after.1.as_str()),
                after.2.as_str(),
                after.3.as_str(),
            )
        } else {
            (
                Reverse(row.published_at.as_str()),
                Reverse(row.seen_at.as_str()),
                row.id.as_str(),
                row.run_id.as_str(),
            ) > (
                Reverse(after.0.as_str()),
                Reverse(after.1.as_str()),
                after.2.as_str(),
                after.3.as_str(),
            )
        };
        if passed {
            page.push(row.clone());
        }
        if page.len() > n {
            break;
        }
    }
    let more = page.len() > n;
    page.truncate(n);
    let next_cursor = if more {
        page.last()
            .map(|a| {
                if hottest {
                    serde_json::to_string(&(
                        a.temperature,
                        &a.published_at,
                        &a.seen_at,
                        &a.id,
                        &a.run_id,
                    ))
                } else {
                    serde_json::to_string(&(&a.published_at, &a.seen_at, &a.id, &a.run_id))
                }
            })
            .transpose()?
    } else {
        None
    };
    let revision = crate::nlp::cache::hash(&serde_json::to_string(&(&page, &coverage))?);
    Ok(ArticlePage {
        rows: page,
        next_cursor,
        revision,
        coverage,
    })
}

/// Article totals for one retained day, deduplicated by identity (the newest
/// occurrence of an article id wins). Per-category counts overlap — an
/// article with several tags counts in each; the `all` key is the distinct
/// admitted total.
pub async fn category_counts(
    repo: &Repository,
    day: &str,
) -> Result<std::collections::BTreeMap<String, u64>> {
    if chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").is_err() || day.len() != 10 {
        bail!("invalid day");
    }
    let mut response = repo
        .db()
        .query(
            format!("SELECT * FROM atlas_article WHERE run_id IN (SELECT VALUE record::id(id) FROM atlas_run WHERE started_at >= $start AND started_at < $end) AND ({})",atlas::CURRENT_ADMISSION_FILTER),
        )
        .bind(("taxonomy", crate::atlas_taxonomy::TAXONOMY_VERSION))
        .bind(("publication", crate::storage::classifier_admission::PUBLICATION_VERSION))
        .bind(("start", day.to_string()))
        .bind(("end", format!("{day}~")))
        .await?
        .check()?;
    let raw: Vec<crate::storage::atlas::ArticleRow> = response.take(0)?;
    let rows: Vec<AtlasArticleRow> = dedup_newest(raw.into_iter().map(Into::into).collect());
    Ok(overlap_counts(&rows))
}

pub async fn article(repo: &Repository, run: &str, id: &str) -> Result<Option<AtlasArticleRow>> {
    if !valid_id(run) || !valid_id(id) {
        bail!("invalid article identifier");
    }
    Ok(atlas::article(repo.db(), run, id).await?)
}

/// `search_articles`: discovery-compatible article search (Phase 4). The
/// storage service owns the indexed query; this wrapper is the validation
/// and projection seam for the API.
pub async fn article_search(
    repo: &Repository,
    q: &crate::storage::article_discovery::ArticleSearchQuery,
) -> Result<crate::storage::article_discovery::ArticleSearchPage> {
    crate::storage::article_discovery::search_articles(repo, q).await
}

/// Immutable projection shared by any non-terminal client. Retained publication,
/// extraction and indexing are reported separately; absence is never success.
pub async fn brief(repo: &Repository, article: &AtlasArticleRow) -> Result<Value> {
    let source_revision = crate::atlas_metadata::input_hash(article);
    let mut parts = intel_brief_view::brief_parts(repo, article).await?;
    parts.relations.sort();
    // Catalog matching enriches the snapshot; it must not hold retained claims
    // and inferences behind the HTTP deadline while a cold matcher loads.
    let mut actors_pending = false;
    let catalog_actors = match tokio::time::timeout(
        Duration::from_secs(1),
        crate::storage::actor_catalog::metadata_article_actors(repo, article),
    )
    .await
    {
        Ok(result) => result?,
        Err(_) => {
            actors_pending = true;
            Some(Vec::new())
        }
    };
    let catalog_enabled = catalog_actors.is_some();
    let mut actors = catalog_actors.unwrap_or_else(|| {
        parts
            .metadata
            .as_ref()
            .map(|m| crate::atlas_metadata::article_actors(article, m))
            .unwrap_or_default()
    });
    let mut clean_actors_checked = false;
    if catalog_enabled && !actors_pending {
        let body_actors = crate::storage::actor_catalog::body_actors(repo, &article.id).await?;
        clean_actors_checked = body_actors.is_some();
        crate::actor_catalog::append_body_actors(&mut actors, body_actors.unwrap_or_default());
    }
    if let Some(output) = parts.output.as_ref().filter(|_| !catalog_enabled) {
        crate::atlas_metadata::merge_output_actors(&mut actors, &output.entities);
    }
    let links = crate::intel_recon::bucket_extracted_with_explanations(
        &parts.claims,
        &parts.relations,
        &parts.explanations,
    )
    .links;
    use crate::osint::source_eval::SourceReliability;
    let reliability = parts
        .metadata
        .as_ref()
        .and_then(|m| m.publisher_prior.as_ref())
        .and_then(|p| SourceReliability::parse(&p.observation.reliability))
        .unwrap_or(SourceReliability::F);
    let mut confidence = crate::intel_recon::article_confidence_surreal(repo, &article.id)
        .await?
        .or_else(|| {
            parts.output.as_ref().map(|output| {
                crate::intel_recon::EvidenceConfidence::estimate_for_source(
                    output,
                    false,
                    reliability,
                    false,
                )
            })
        });
    let mut confidence_basis = "evidence";
    if confidence.as_ref().and_then(|c| c.score).is_none() {
        let (initial, _) = crate::intel_recon::initial_rating(
            reliability,
            crate::osint::source_eval::InformationCredibility::CannotBeJudged,
        );
        if let Some(score) = initial {
            let estimate =
                confidence.get_or_insert_with(|| crate::intel_recon::EvidenceConfidence {
                    information_grade: 6,
                    provisional: true,
                    ..Default::default()
                });
            estimate.score = Some(score);
            estimate.provisional = true;
            confidence_basis = "publisher_prior";
        } else {
            confidence_basis = "unavailable";
        }
    }
    let confidence_reason = match confidence_basis {
        "publisher_prior" => "Publisher prior only; no scored corroboration yet.",
        "unavailable" => {
            "No scored corroboration or publisher reliability assessment is available."
        }
        _ => "",
    };
    let confidence_grade = format!(
        "{}{}",
        reliability.as_str(),
        confidence.as_ref().map_or(6, |c| c.information_grade)
    );
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
    let inference_rows = parts.output.as_ref().map(|output| output.inferences.iter().map(|item| json!({"id":crate::investigation::insights::inference_fingerprint(item),"text":item.text})).collect::<Vec<_>>());
    let no_actors = actors.is_empty();
    let mut value = json!({"article":article,"source_revision":source_revision,"actors": actors,"claims":claims,"body":parts.body,"output":parts.output,"lifecycle":parts.lifecycle,"status":crate::provider_diag::sanitize(&parts.status),"publication":published,"relations":parts.relations,"links":links,"job_id":job_id,"indexing":indexing,"confidence":confidence,"confidence_grade":confidence_grade,"confidence_basis":confidence_basis,"confidence_reason":confidence_reason,"body_pending":body_pending});
    if let Some(inferences) = inference_rows {
        value["inferences"] = json!(inferences);
    }
    value["actors_pending"] = json!(actors_pending);
    // A running chain already owns storage. Binding navigation seeds here
    // rebuilds a topology snapshot on every progress refresh and pushes the
    // read past the client abort deadline ("signal timed out").
    let chain_stages = match &job_id {
        Some(job) => brief_stages::stage_views(repo, job).await?,
        None => Vec::new(),
    };
    let waiting = |state: &str| {
        matches!(
            state,
            "queued" | "running" | "retry_scheduled" | "waiting" | "pending"
        )
    };
    let brief_running = chain_stages.iter().any(|stage| waiting(&stage.state));
    let body_pending = chain_stages
        .iter()
        .any(|stage| stage.operation == "intel_brief_body" && waiting(&stage.state))
        || (job_id.is_none() && !clean_actors_checked);
    let body_unavailable = chain_stages.iter().any(|stage| {
        stage.operation == "intel_brief_body"
            && matches!(stage.state.as_str(), "failed" | "blocked" | "cancelled")
    });
    value["actors_body_pending"] = json!(body_pending);
    value["actors_body_unavailable"] = json!(body_unavailable);
    value["actors_pending"] = json!(actors_pending || (no_actors && body_pending));
    if !brief_running {
        match tokio::time::timeout(
            Duration::from_secs(1),
            crate::intel_topology::decorate(repo, article, &source_revision, &mut value),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => value["bindings_pending"] = json!(true),
        }
    }
    // A catalog actor exists before article indexing. Do not make dossier
    // access depend on whether the article's topology neighborhood is ready.
    if catalog_enabled {
        crate::intel_topology::decorate_catalog_actors(repo, &mut value).await?;
    }
    value["classification"] = serde_json::to_value(
        crate::storage::classifier_admission::classification_view(
            repo,
            &crate::atlas_classify::OccurrenceKey::new(&article.run_id, &article.id),
        )
        .await?,
    )?;
    value["coverage"] = serde_json::to_value(
        crate::storage::classifier_admission::coverage(repo, Some(&article.run_id), None).await?,
    )?;
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
    let mut inputs = brief_stages::inputs_for_article(repo, article, &settings).await?;
    inputs.input_revision = format!("{}:reload:{key}", inputs.input_revision);
    // An explicit reload reruns nodes, while retaining the captured immutable
    // domain/source packet produced by the shared builder.
    inputs.stage_revisions.clear();
    Ok(brief_stages::ensure(repo, &inputs).await?.job_id)
}

/// Graph of extracted brief items. Indexing is not required: these nodes are
/// the claims, actors and inferences already stored for the article.
pub async fn extracted_topology(repo: &Repository, run: &str, id: &str) -> Result<Value> {
    let Some(article) = article(repo, run, id).await? else {
        bail!("article is not retained");
    };
    let parts = intel_brief_view::brief_parts(repo, &article).await?;
    let actors = parts
        .metadata
        .as_ref()
        .map(|metadata| crate::atlas_metadata::article_actors(&article, metadata))
        .unwrap_or_default();
    Ok(extracted_graph(
        &article.title,
        &article.run_id,
        &article.id,
        &actors
            .iter()
            .map(|actor| (actor.id.as_str(), actor.display_name.as_str()))
            .collect::<Vec<_>>(),
        &parts
            .claims
            .iter()
            .map(|claim| {
                (
                    claim.fingerprint.as_str(),
                    claim.claim.as_str(),
                    claim.classification.as_str(),
                )
            })
            .collect::<Vec<_>>(),
        &parts
            .output
            .as_ref()
            .map(|output| {
                output
                    .inferences
                    .iter()
                    .map(|item| {
                        (
                            item.id.as_str(),
                            item.text.as_str(),
                            item.premise_claim_ids
                                .iter()
                                .map(String::as_str)
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        &parts
            .relations
            .iter()
            .map(|(left, right, relation)| (left.as_str(), right.as_str(), relation.as_str()))
            .collect::<Vec<_>>(),
    ))
}

pub(crate) fn extracted_graph(
    title: &str,
    run: &str,
    id: &str,
    actors: &[(&str, &str)],
    claims: &[(&str, &str, &str)],
    inferences: &[(&str, &str, Vec<&str>)],
    relations: &[(&str, &str, &str)],
) -> Value {
    let root = format!("article:{run}/{id}");
    let mut nodes = vec![json!({
        "id": root,
        "kind": "article",
        "label": title,
        "detail": "Extracted brief",
        "is_root": true,
        "min_hops": 0,
    })];
    let mut edges = Vec::new();
    for (actor_id, name) in actors {
        if name.is_empty() {
            continue;
        }
        let node = format!("actor:{actor_id}");
        nodes.push(json!({
            "id": node,
            "kind": "entity",
            "label": name,
            "detail": "Extracted actor",
            "is_root": false,
            "min_hops": 1,
        }));
        edges.push(json!({
            "id": format!("mention:{node}"),
            "from": root,
            "to": node,
            "label": "mentioned",
        }));
    }
    let mut claim_ids = std::collections::BTreeSet::new();
    for (claim_id, text, classification) in claims {
        if text.is_empty() {
            continue;
        }
        let node = format!("claim:{claim_id}");
        claim_ids.insert(*claim_id);
        let inference = *classification == "inference";
        nodes.push(json!({
            "id": node,
            "kind": if inference { "inference" } else { "claim" },
            "label": text,
            "detail": "Extracted brief item",
            "is_root": false,
            "min_hops": 1,
        }));
        edges.push(json!({
            "id": format!("contains:{node}"),
            "from": root,
            "to": node,
            "label": "extracted",
        }));
    }
    for (inference_id, text, premises) in inferences {
        if text.is_empty() {
            continue;
        }
        let node = format!("inference:{inference_id}");
        nodes.push(json!({
            "id": node,
            "kind": "inference",
            "label": text,
            "detail": "Extracted inference",
            "is_root": false,
            "min_hops": 1,
        }));
        edges.push(json!({
            "id": format!("contains:{node}"),
            "from": root,
            "to": node,
            "label": "extracted",
        }));
        for premise in premises {
            if claim_ids.contains(premise) {
                edges.push(json!({
                    "id": format!("premise:{node}:{premise}"),
                    "from": format!("claim:{premise}"),
                    "to": node,
                    "label": "premise",
                }));
            }
        }
    }
    for (left, right, relation) in relations {
        if claim_ids.contains(left) && claim_ids.contains(right) {
            edges.push(json!({
                "id": format!("relation:{left}:{right}:{relation}"),
                "from": format!("claim:{left}"),
                "to": format!("claim:{right}"),
                "label": relation,
            }));
        }
    }
    json!({
        "root_label": title,
        "nodes": nodes,
        "edges": edges,
        "findings": { "gaps": [] },
        "has_continuation": false,
    })
}

#[cfg(test)]
mod membership_tests {
    use super::*;

    fn row(id: &str, run: &str, seen: &str, categories: &[&str]) -> AtlasArticleRow {
        AtlasArticleRow {
            run_id: run.into(),
            id: id.into(),
            title: "Headline".into(),
            description: String::new(),
            url: "https://example.com".into(),
            country: "us".into(),
            source_name: "Wire".into(),
            source_domain: "example.com".into(),
            published_at: "2026-11-03T00:00:00+00:00".into(),
            provider: "newsapi".into(),
            temperature: 1.0,
            category: categories.first().copied().unwrap_or("unk").into(),
            seen_at: seen.into(),
            author: String::new(),
            image_url: String::new(),
            categories: categories.iter().map(|tag| tag.to_string()).collect(),
            admitted_at: "2026-11-03T01:00:00+00:00".into(),
        }
    }

    #[test]
    fn dedup_keeps_the_newest_occurrence_of_each_identity() {
        let rows = vec![
            row(
                "a",
                "run-1",
                "2026-11-03T02:00:00+00:00",
                &["science", "health"],
            ),
            row("b", "run-1", "2026-11-03T02:00:00+00:00", &["military"]),
            // The same article retained again by a newer run: dedup drops it.
            row("a", "run-2", "2026-11-03T03:00:00+00:00", &["science"]),
        ];
        let deduped = dedup_newest(rows);
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].id, "a");
        assert_eq!(deduped[0].run_id, "run-2", "the newest occurrence wins");
        assert_eq!(deduped[0].categories, ["science"]);
    }

    #[test]
    fn per_category_counts_overlap_and_all_deduplicates_identity() {
        let rows = vec![
            row("a", "run-1", "t1", &["science", "health"]),
            row("b", "run-1", "t2", &["science", "technology", "military"]),
            // A second occurrence of `a` never inflates any count.
            row("a", "run-2", "t3", &["science", "health"]),
        ];
        let deduped = dedup_newest(rows);
        let counts = overlap_counts(&deduped);
        assert_eq!(counts["science"], 2);
        assert_eq!(counts["health"], 1);
        assert_eq!(counts["military"], 1);
        assert_eq!(counts["technology"], 1);
        assert_eq!(counts["all"], 2, "All deduplicates identity");
        // Legacy rows without tags are pending migration: never counted.
        let legacy = vec![row("c", "run-1", "t4", &[])];
        let counts = overlap_counts(&dedup_newest(legacy));
        assert!(!counts.contains_key("all"));
    }

    #[test]
    fn membership_filters_use_array_tags_not_the_scalar() {
        let article = row("a", "run-1", "t1", &["science", "health"]);
        assert!(article.categories.iter().any(|tag| tag == "science"));
        assert!(!article.categories.iter().any(|tag| tag == "information"));
        // The scalar projects the first tag for existing readers.
        assert_eq!(article.projected_category(), "science");
        let legacy = row("c", "run-1", "t4", &[]);
        assert_eq!(legacy.projected_category(), "unk");
    }
}

#[cfg(test)]
mod extracted_graph_tests {
    use super::extracted_graph;
    #[test]
    fn extracted_items_are_navigable_before_indexing() {
        let graph = extracted_graph(
            "Harbor report",
            "run-1",
            "art-1",
            &[("local-actor", "Harbor authority")],
            &[("claim-1", "Retained statement", "fact")],
            &[("inf-1", "Possible outcome", vec!["claim-1"])],
            &[],
        );
        let labels: Vec<_> = graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["label"].as_str().unwrap())
            .collect();
        assert_eq!(
            labels,
            [
                "Harbor report",
                "Harbor authority",
                "Retained statement",
                "Possible outcome"
            ]
        );
        assert!(graph["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edge| edge["label"] == "premise"));
    }
}
