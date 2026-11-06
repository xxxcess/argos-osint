//! Core-owned article discovery over the admitted-only canonical corpus
//! (schema v2). @tag:atlas-collection @tag:surreal-storage
//!
//! `search_articles` is the third Brain discovery index: admitted titles and
//! descriptions as separate full-text fields, resolved through the two
//! SurrealDB FULLTEXT BM25 indexes on `atlas_article`
//! (`atlas_article_title_search`, `atlas_article_description_search`) — no
//! model calls, no general-memory vectors. Maintenance is atomic: the index
//! is maintained by the engine in the same transaction that writes or deletes
//! the canonical row ([`super::atlas::admission_admit`],
//! [`super::atlas_delete`]), so a hit can never outlive its row and the
//! alternative (idempotent revisioned outbox work) is not needed. Pending
//! rows live only in `atlas_admission` staging, discards keep only
//! `atlas_rejection` receipts, and legacy rows carry an empty `admitted_at`
//! until reclassification — all three are excluded by the `admitted_at != ''`
//! gate, so stale, pending, discarded and deleted inputs never surface.
//!
//! Contracts kept here on purpose: titles rank above description-only
//! matches (relevance is not confidence), ties are deterministic, occurrences
//! deduplicate to the best-ranked one per article id, pagination is bounded,
//! and the page revision fingerprints every hit's input revision. Related
//! candidates derived from a seed article must exclude the seed and are
//! assessed before corroboration ([`related_from_seed`]).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use super::repo::Repository;
use crate::store::AtlasArticleRow;

/// Most distinct query terms one search keeps.
pub const MAX_SEARCH_TERMS: usize = 16;
/// Longest accepted query string.
pub const MAX_QUERY_CHARS: usize = 256;
/// Bounded page size range.
pub const PAGE_LIMIT: std::ops::RangeInclusive<usize> = 1..=50;
/// Per-field candidate fetch cap: the ranked merge reads at most this many
/// rows per field before dedup and pagination.
const FETCH_CAP: usize = 256;
/// Hard offset ceiling; beyond it a page is empty rather than unbounded.
const MAX_OFFSET: usize = 4_096;
/// Excerpt window in characters.
const EXCERPT_CHARS: usize = 160;

/// One `search_articles` request. Empty optional filters are absent.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ArticleSearchQuery {
    pub query: String,
    /// `all` (default) or one category id; membership over the ordered tags.
    pub category: String,
    /// `YYYY-MM-DD` retained-day filter (the occurrence's `seen_at` day).
    pub day: String,
    /// Exact `source_domain` filter.
    pub source: String,
    /// Exact run-occurrence filter.
    pub run: String,
    pub limit: Option<usize>,
    /// Offset cursor from `next_cursor`.
    pub cursor: Option<String>,
}

impl ArticleSearchQuery {
    pub fn validate(&self) -> Result<()> {
        if self.query.trim().is_empty() || self.query.len() > MAX_QUERY_CHARS {
            anyhow::bail!("invalid query");
        }
        if search_terms(&self.query).is_empty() {
            anyhow::bail!("invalid query");
        }
        if !self.category.is_empty()
            && self.category != "all"
            && crate::atlas_taxonomy::DomainId::parse(&self.category).is_none()
        {
            anyhow::bail!("invalid category");
        }
        if !self.day.is_empty()
            && (chrono::NaiveDate::parse_from_str(&self.day, "%Y-%m-%d").is_err()
                || self.day.len() != 10)
        {
            anyhow::bail!("invalid day");
        }
        if self.source.len() > 253 {
            anyhow::bail!("invalid source");
        }
        if !self.run.is_empty() && !crate::intel_service::valid_id(&self.run) {
            anyhow::bail!("invalid run");
        }
        if !PAGE_LIMIT.contains(&self.limit.unwrap_or(20)) {
            anyhow::bail!("invalid limit");
        }
        if let Some(cursor) = &self.cursor {
            let offset: usize = cursor
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid cursor"))?;
            if offset > MAX_OFFSET {
                anyhow::bail!("invalid cursor");
            }
        }
        Ok(())
    }

    fn limit(&self) -> usize {
        self.limit
            .unwrap_or(20)
            .clamp(*PAGE_LIMIT.start(), *PAGE_LIMIT.end())
    }

    fn offset(&self) -> usize {
        self.cursor
            .as_deref()
            .and_then(|c| c.parse().ok())
            .unwrap_or(0)
    }
}

/// Which field a hit matched; titles always outrank description-only hits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchedField {
    Title,
    Description,
}

impl MatchedField {
    pub fn as_str(self) -> &'static str {
        match self {
            MatchedField::Title => "title",
            MatchedField::Description => "description",
        }
    }
}

/// One discovered article occurrence.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ArticleSearchHit {
    pub run_id: String,
    pub article_id: String,
    /// The occurrence's input revision (title/description/url/source hash).
    pub revision: String,
    pub title: String,
    pub description: String,
    /// Bounded excerpt of the matched field around the first query term.
    pub excerpt: String,
    pub matched_field: MatchedField,
    /// The matched field's BM25 score. Relevance is not confidence.
    pub rank: f64,
    pub categories: Vec<String>,
    pub taxonomy_version: String,
    pub admission_state: String,
    pub publication_version: String,
    pub source_name: String,
    pub source_domain: String,
    pub published_at: String,
    pub seen_at: String,
    pub url: String,
    pub provider: String,
}

/// One bounded, deduplicated page.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ArticleSearchPage {
    pub hits: Vec<ArticleSearchHit>,
    pub next_cursor: Option<String>,
    /// Fingerprint over the hits and their input revisions.
    pub revision: String,
    /// `""` — the canonical corpus is always searchable once bootstrapped.
    pub diagnostic: String,
    pub coverage: super::classifier_admission::TaxonomyCoverage,
}

#[derive(SurrealValue)]
struct SearchRow {
    run_id: String,
    article_id: String,
    title: String,
    description: String,
    url: String,
    source_name: String,
    source_domain: String,
    published_at: String,
    seen_at: String,
    provider: String,
    categories: Vec<String>,
    admitted_at: String,
    score: f64,
}

impl SearchRow {
    fn revision(&self) -> String {
        crate::atlas_metadata::input_hash(&self.article())
    }

    fn article(&self) -> AtlasArticleRow {
        AtlasArticleRow {
            run_id: self.run_id.clone(),
            id: self.article_id.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            url: self.url.clone(),
            country: String::new(),
            source_name: self.source_name.clone(),
            source_domain: self.source_domain.clone(),
            published_at: self.published_at.clone(),
            provider: self.provider.clone(),
            temperature: 0.0,
            category: self.categories.first().cloned().unwrap_or_default(),
            seen_at: self.seen_at.clone(),
            author: String::new(),
            image_url: String::new(),
            categories: self.categories.clone(),
            admitted_at: self.admitted_at.clone(),
        }
    }

    fn hit(&self, field: MatchedField, excerpt: String) -> ArticleSearchHit {
        ArticleSearchHit {
            run_id: self.run_id.clone(),
            article_id: self.article_id.clone(),
            revision: self.revision(),
            title: self.title.clone(),
            description: self.description.clone(),
            excerpt,
            matched_field: field,
            rank: self.score,
            categories: self.categories.clone(),
            taxonomy_version: crate::atlas_taxonomy::TAXONOMY_VERSION.into(),
            admission_state: "admitted".into(),
            publication_version: super::classifier_admission::PUBLICATION_VERSION.into(),
            source_name: self.source_name.clone(),
            source_domain: self.source_domain.clone(),
            published_at: self.published_at.clone(),
            seen_at: self.seen_at.clone(),
            url: self.url.clone(),
            provider: self.provider.clone(),
        }
    }
}

/// Distinct lowercased alphanumeric terms of `query`, in order, bounded.
/// The SurrealDB match operator handles multi-term queries; the term list
/// exists for excerpts and honest validation.
pub fn search_terms(query: &str) -> Vec<String> {
    let mut out = Vec::new();
    for term in query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
    {
        if !out.contains(&term) {
            out.push(term);
        }
        if out.len() == MAX_SEARCH_TERMS {
            break;
        }
    }
    out
}

/// Bounded excerpt of `text` around the first `terms` occurrence, with
/// ellipses marking omitted text. Empty when no term appears. Positions come
/// from a per-character lowercase map, so the window lands on original byte
/// offsets even where lowercasing changes byte lengths.
pub fn excerpt_for(text: &str, terms: &[String]) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut lower = String::with_capacity(text.len());
    let mut map: Vec<usize> = Vec::with_capacity(text.len() + 1);
    for (byte, ch) in text.char_indices() {
        for lowered in ch.to_lowercase() {
            map.push(byte);
            lower.push(lowered);
        }
    }
    map.push(text.len());
    let Some(lower_start) = terms
        .iter()
        .filter_map(|term| lower.find(term))
        .min_by_key(|pos| *pos)
    else {
        return String::new();
    };
    let start = map.get(lower_start).copied().unwrap_or(text.len());
    let mut end = EXCERPT_CHARS.min(text.len() - start);
    while !text.is_char_boundary(start + end) {
        end -= 1;
    }
    let mut excerpt = text[start..start + end].trim().to_string();
    if start > 0 {
        excerpt = format!("…{excerpt}");
    }
    if start + end < text.len() {
        excerpt.push('…');
    }
    excerpt
}

/// WHERE fragment for the caller's filters, and its bindings, shared by both
/// field queries.
fn filters(q: &ArticleSearchQuery) -> (String, Vec<(&'static str, String)>) {
    let mut sql = format!("({})", super::atlas::CURRENT_ADMISSION_FILTER);
    let mut binds: Vec<(&'static str, String)> = vec![
        ("taxonomy", crate::atlas_taxonomy::TAXONOMY_VERSION.into()),
        (
            "publication",
            super::classifier_admission::PUBLICATION_VERSION.into(),
        ),
    ];
    if q.category.is_empty() || q.category == "all" {
        // All admitted articles carry at least one tag.
        sql.push_str(" AND categories != []");
    } else {
        sql.push_str(" AND (category=$category OR categories CONTAINS $category)");
        binds.push(("category", q.category.clone()));
    }
    if !q.day.is_empty() {
        // The retained day of the occurrence itself (`seen_at`), compared as
        // RFC3339 strings: every same-day stamp continues with a byte below
        // `~`, so the half-open upper bound needs no date math (the same
        // idiom as `intel_service`).
        sql.push_str(" AND seen_at >= $day_start AND seen_at < $day_end");
        binds.push(("day_start", q.day.clone()));
        binds.push(("day_end", format!("{}~", q.day)));
    }
    if !q.source.is_empty() {
        sql.push_str(" AND source_domain = $source");
        binds.push(("source", q.source.clone()));
    }
    if !q.run.is_empty() {
        sql.push_str(" AND run_id = $run");
        binds.push(("run", q.run.clone()));
    }
    (sql, binds)
}

/// The match expression of a user query: one scored reference, every term
/// required (SurrealDB's default multi-term semantics).
const QUERY_MATCH: &str = "@1@";
/// Related-candidate recall: any seed term matches. The default AND match
/// would drop every lead that shares only the seed's leading terms, and a
/// full-title AND query reduces to the seed itself. BM25 still ranks the
/// overlap; related hits stay leads, never evidence.
const RELATED_MATCH: &str = "@OR@";

async fn field_hits(
    db: &Surreal<Any>,
    field: &'static str,
    q: &ArticleSearchQuery,
    filter_sql: &str,
    binds: &[(&'static str, String)],
    matches: &str,
) -> Result<Vec<SearchRow>> {
    let sql = format!(
        "SELECT run_id, article_id, title, description, url, source_name, source_domain, \
         published_at, seen_at, provider, categories, admitted_at, search::score(1) AS score \
         FROM atlas_article WHERE {field} {matches} $q AND {filter_sql} \
         ORDER BY score DESC, published_at DESC, seen_at DESC, article_id ASC, run_id ASC \
         LIMIT $fetch"
    );
    let mut query = db
        .query(sql)
        .bind(("q", q.query.trim().to_string()))
        .bind(("fetch", FETCH_CAP as i64));
    for (name, value) in binds {
        query = query.bind((name.to_string(), value.clone()));
    }
    let mut response = query.await?.check()?;
    Ok(response.take(0)?)
}

/// Deterministic ranked merge: title matches first, then description-only,
/// each by BM25 score, recency, then identity; occurrences of one article id
/// deduplicate to the best-ranked one (a title occurrence beats a
/// same-scored description occurrence of another run).
fn merge_field_hits(
    mut titles: Vec<ArticleSearchHit>,
    mut descriptions: Vec<ArticleSearchHit>,
) -> Vec<ArticleSearchHit> {
    let order = |a: &ArticleSearchHit, b: &ArticleSearchHit| {
        b.rank
            .partial_cmp(&a.rank)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.published_at.cmp(&a.published_at))
            .then_with(|| b.seen_at.cmp(&a.seen_at))
            .then_with(|| a.article_id.cmp(&b.article_id))
            .then_with(|| a.run_id.cmp(&b.run_id))
    };
    titles.sort_by(order);
    descriptions.sort_by(order);
    let mut merged: Vec<ArticleSearchHit> = Vec::with_capacity(titles.len() + descriptions.len());
    let mut seen = std::collections::HashSet::new();
    for hit in titles.into_iter().chain(descriptions) {
        if seen.insert(hit.article_id.clone()) {
            merged.push(hit);
        }
    }
    merged
}

/// `search_articles`: bounded, deduplicated, weighted discovery over the
/// admitted canonical corpus. Availability is structural (the indexes come up
/// with the schema), so an honest empty page is a real empty page.
pub async fn search_articles(
    repo: &Repository,
    q: &ArticleSearchQuery,
) -> Result<ArticleSearchPage> {
    q.validate()?;
    let scope = super::classifier_admission::CoverageFilter {
        run: (!q.run.is_empty()).then(|| q.run.clone()),
        seen_day: (!q.day.is_empty()).then(|| q.day.clone()),
        source: (!q.source.is_empty()).then(|| q.source.clone()),
        indexed_text: Some(q.query.trim().to_string()),
        ..Default::default()
    };
    let coverage = super::classifier_admission::coverage_filtered(repo, &scope).await?;
    let (filter_sql, binds) = filters(q);
    let titles = field_hits(repo.db(), "title", q, &filter_sql, &binds, QUERY_MATCH).await?;
    let mut hits: Vec<ArticleSearchHit> = Vec::new();
    for row in &titles {
        hits.push(row.hit(
            MatchedField::Title,
            excerpt_for(&row.title, &search_terms(&q.query)),
        ));
    }
    let descriptions = field_hits(
        repo.db(),
        "description",
        q,
        &filter_sql,
        &binds,
        QUERY_MATCH,
    )
    .await?;
    let mut description_hits: Vec<ArticleSearchHit> = Vec::new();
    for row in &descriptions {
        description_hits.push(row.hit(
            MatchedField::Description,
            excerpt_for(&row.description, &search_terms(&q.query)),
        ));
    }
    let merged = merge_field_hits(hits, description_hits);
    let offset = q.offset();
    let page: Vec<ArticleSearchHit> = merged
        .into_iter()
        .skip(offset)
        .take(q.limit() + 1)
        .collect();
    let more = page.len() > q.limit();
    let hits: Vec<ArticleSearchHit> = page.into_iter().take(q.limit()).collect();
    let next_cursor = more.then(|| (offset + q.limit()).to_string());
    let revision = crate::nlp::cache::hash(&serde_json::to_string(&(&hits, &coverage))?);
    Ok(ArticleSearchPage {
        hits,
        next_cursor,
        revision,
        diagnostic: String::new(),
        coverage,
    })
}

/// Related candidates for a seed article, straight from the discovery index:
/// any seed-title term is enough to surface a lead (BM25 ranks the overlap),
/// the seed itself is excluded by identity — never just one occurrence — and
/// every hit stays a lead pending evidence assessment before corroboration.
pub async fn related_from_seed(
    repo: &Repository,
    seed: &AtlasArticleRow,
    limit: usize,
) -> Result<Vec<ArticleSearchHit>> {
    let query = ArticleSearchQuery {
        query: seed.title.trim().to_string(),
        limit: Some(limit.clamp(*PAGE_LIMIT.start(), *PAGE_LIMIT.end())),
        ..Default::default()
    };
    if query.validate().is_err() {
        return Ok(Vec::new());
    }
    let (filter_sql, binds) = filters(&query);
    let terms = search_terms(&query.query);
    let titles = field_hits(
        repo.db(),
        "title",
        &query,
        &filter_sql,
        &binds,
        RELATED_MATCH,
    )
    .await?;
    let descriptions = field_hits(
        repo.db(),
        "description",
        &query,
        &filter_sql,
        &binds,
        RELATED_MATCH,
    )
    .await?;
    let hits: Vec<ArticleSearchHit> = titles
        .iter()
        .map(|row| row.hit(MatchedField::Title, excerpt_for(&row.title, &terms)))
        .chain(descriptions.iter().map(|row| {
            row.hit(
                MatchedField::Description,
                excerpt_for(&row.description, &terms),
            )
        }))
        .collect();
    // The ranked merge deduplicates occurrences to one hit per article id, so
    // dropping the seed's identity cannot leave a second seed occurrence.
    // Related candidates never paginate: the caller takes one page.
    Ok(merge_field_hits(hits, Vec::new())
        .into_iter()
        .filter(|hit| hit.article_id != seed.id)
        .take(query.limit.unwrap_or(20))
        .collect())
}

/// A bounded initial discovery window. `fetched` counts raw field hits before
/// deduplication; callers can share one 100-row budget across independent channels.
#[derive(Clone, Debug, Default)]
pub struct BoundedRelatedCandidates {
    pub hits: Vec<ArticleSearchHit>,
    pub fetched: usize,
    pub truncated: bool,
    pub next_cursor: Option<String>,
    pub revision: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RelatedWindowCursor {
    seed_revision: String,
    title_offset: usize,
    description_offset: usize,
}

/// Indexed all-retained metadata candidates, preserving exact occurrences.
/// The SQL limits apply before metadata leaves storage, never after a 256-row fetch.
pub async fn related_from_seed_bounded(
    repo: &Repository,
    seed: &AtlasArticleRow,
    initial_budget: usize,
) -> Result<BoundedRelatedCandidates> {
    related_from_seed_bounded_after(repo, seed, initial_budget, None).await
}

/// Explicit bounded continuation of the same frozen seed metadata revision.
/// No automatic pagination or model request is performed by this reader.
pub async fn related_from_seed_bounded_after(
    repo: &Repository,
    seed: &AtlasArticleRow,
    initial_budget: usize,
    cursor: Option<&str>,
) -> Result<BoundedRelatedCandidates> {
    anyhow::ensure!(initial_budget <= 100, "related initial budget exceeds 100");
    let seed_revision = crate::atlas_metadata::input_hash(seed);
    let offsets = match cursor {
        Some(raw) => {
            anyhow::ensure!(raw.len() <= 1024, "related cursor exceeds bound");
            let cursor: RelatedWindowCursor = serde_json::from_str(raw)?;
            anyhow::ensure!(
                cursor.seed_revision == seed_revision
                    && cursor.title_offset <= MAX_OFFSET
                    && cursor.description_offset <= MAX_OFFSET,
                "stale or invalid related cursor"
            );
            cursor
        }
        None => RelatedWindowCursor {
            seed_revision: seed_revision.clone(),
            title_offset: 0,
            description_offset: 0,
        },
    };
    if initial_budget == 0 {
        return Ok(BoundedRelatedCandidates {
            revision: seed_revision,
            ..Default::default()
        });
    }
    let source_text = if seed.title.trim().is_empty() {
        &seed.description
    } else {
        &seed.title
    };
    let query = search_terms(source_text).join(" ");
    let query: String = query.chars().take(MAX_QUERY_CHARS).collect();
    if query.trim().is_empty() {
        return Ok(BoundedRelatedCandidates {
            revision: seed_revision,
            ..Default::default()
        });
    }
    let terms = search_terms(&query);
    let title_limit = initial_budget.div_ceil(2);
    let description_limit = initial_budget / 2;
    let titles = bounded_related_field(
        repo,
        seed,
        "title",
        &query,
        title_limit,
        offsets.title_offset,
    )
    .await?;
    let descriptions = bounded_related_field(
        repo,
        seed,
        "description",
        &query,
        description_limit,
        offsets.description_offset,
    )
    .await?;
    let fetched = titles.len() + descriptions.len();
    let truncated = (title_limit > 0 && titles.len() == title_limit)
        || (description_limit > 0 && descriptions.len() == description_limit);
    let next_cursor = if truncated
        && offsets.title_offset + titles.len() <= MAX_OFFSET
        && offsets.description_offset + descriptions.len() <= MAX_OFFSET
    {
        Some(serde_json::to_string(&RelatedWindowCursor {
            seed_revision,
            title_offset: offsets.title_offset + titles.len(),
            description_offset: offsets.description_offset + descriptions.len(),
        })?)
    } else {
        None
    };
    let mut seen = std::collections::BTreeSet::new();
    let hits: Vec<_> = titles
        .iter()
        .map(|row| row.hit(MatchedField::Title, excerpt_for(&row.title, &terms)))
        .chain(descriptions.iter().map(|row| {
            row.hit(
                MatchedField::Description,
                excerpt_for(&row.description, &terms),
            )
        }))
        .filter(|hit| {
            seen.insert((
                hit.run_id.clone(),
                hit.article_id.clone(),
                hit.revision.clone(),
            ))
        })
        .collect();
    let revision = crate::nlp::cache::hash(&serde_json::to_string(&hits)?);
    Ok(BoundedRelatedCandidates {
        hits,
        fetched,
        truncated,
        next_cursor,
        revision,
    })
}

async fn bounded_related_field(
    repo: &Repository,
    seed: &AtlasArticleRow,
    field: &'static str,
    query: &str,
    limit: usize,
    offset: usize,
) -> Result<Vec<SearchRow>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    anyhow::ensure!(
        matches!(field, "title" | "description"),
        "invalid related field"
    );
    let sql = format!("SELECT run_id, article_id, title, description, url, source_name, source_domain, published_at, seen_at, provider, categories, admitted_at, search::score(1) AS score FROM atlas_article WHERE {field} {RELATED_MATCH} $q AND ({}) AND !(run_id = $seed_run AND article_id = $seed_id) ORDER BY score DESC, published_at DESC, seen_at DESC, article_id ASC, run_id ASC LIMIT $limit START $offset",super::atlas::CURRENT_ADMISSION_FILTER);
    let mut result = repo
        .db()
        .query(sql)
        .bind(("taxonomy", crate::atlas_taxonomy::TAXONOMY_VERSION))
        .bind((
            "publication",
            super::classifier_admission::PUBLICATION_VERSION,
        ))
        .bind(("q", query.to_owned()))
        .bind(("seed_run", seed.run_id.clone()))
        .bind(("seed_id", seed.id.clone()))
        .bind(("limit", limit as i64))
        .bind(("offset", offset as i64))
        .await?
        .check()?;
    Ok(result.take(0)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(
        id: &str,
        run: &str,
        field: MatchedField,
        rank: f64,
        published: &str,
    ) -> ArticleSearchHit {
        ArticleSearchHit {
            run_id: run.into(),
            article_id: id.into(),
            revision: format!("rev-{id}-{run}"),
            title: "Title".into(),
            description: "Description".into(),
            excerpt: String::new(),
            matched_field: field,
            rank,
            categories: vec!["scientific".into()],
            taxonomy_version: crate::atlas_taxonomy::TAXONOMY_VERSION.into(),
            admission_state: "admitted".into(),
            publication_version: crate::storage::classifier_admission::PUBLICATION_VERSION.into(),
            source_name: "Wire".into(),
            source_domain: "example.com".into(),
            published_at: published.into(),
            seen_at: published.into(),
            url: "https://example.com".into(),
            provider: "newsapi".into(),
        }
    }

    #[test]
    fn search_terms_are_distinct_lowercased_and_bounded() {
        assert_eq!(
            search_terms("Harbor Union, harbor! HU"),
            ["harbor", "union", "hu"]
        );
        assert!(search_terms(" ... !!! ").is_empty());
        let many = search_terms("a b c d e f g h i j k l m n o p q");
        assert_eq!(many.len(), MAX_SEARCH_TERMS);
    }

    #[test]
    fn titles_rank_above_description_only_matches_with_deterministic_ties() {
        let title_low = hit("t1", "r1", MatchedField::Title, 0.2, "2026-01-01");
        let title_high = hit("t2", "r1", MatchedField::Title, 5.0, "2026-01-01");
        let description_high = hit("d1", "r1", MatchedField::Description, 99.0, "2026-01-01");
        let merged = merge_field_hits(vec![title_low, title_high.clone()], vec![description_high]);
        assert_eq!(
            merged
                .iter()
                .map(|h| h.article_id.as_str())
                .collect::<Vec<_>>(),
            ["t2", "t1", "d1"],
            "field class orders before any description score"
        );
        // Ties: rank, then recency, then identity.
        let tie_a = hit("a", "r1", MatchedField::Title, 1.0, "2026-01-02");
        let tie_b = hit("b", "r1", MatchedField::Title, 1.0, "2026-01-01");
        let merged = merge_field_hits(vec![tie_b, tie_a], vec![]);
        assert_eq!(merged[0].article_id, "a");
        assert_eq!(merged[1].article_id, "b");
    }

    #[test]
    fn occurrences_deduplicate_to_the_best_ranked_one() {
        let old_run = hit("a", "r1", MatchedField::Title, 1.0, "2026-01-01");
        let newest = hit("a", "r2", MatchedField::Description, 1.0, "2026-01-03");
        let merged = merge_field_hits(vec![old_run], vec![newest]);
        assert_eq!(merged.len(), 1, "one article id survives dedup");
        assert_eq!(
            merged[0].run_id, "r1",
            "the title match outranks the same-score description"
        );
    }

    #[test]
    fn excerpts_cut_at_word_boundaries_and_mark_ellipsis() {
        let text = "Officials confirmed the Harbor Union merger after weeks of negotiation";
        let excerpt = excerpt_for(text, &["union".to_string()]);
        assert!(excerpt.contains("Union merger"), "{excerpt}");
        assert!(excerpt.starts_with('…'), "text before the match is marked");
        // The window starts at the first term, so a longer sentence is cut.
        let long = format!(
            "Preamble sentence one. {}",
            "A very long harbor union dossier. ".repeat(12)
        );
        let excerpt = excerpt_for(&long, &["harbor".to_string()]);
        assert!(excerpt.starts_with('…') && excerpt.ends_with('…'));
        assert!(excerpt.chars().count() <= EXCERPT_CHARS + 2);
        assert_eq!(excerpt_for("nothing matches", &["zzz".to_string()]), "");
    }

    #[test]
    fn unicode_terms_excerpt_on_char_boundaries() {
        let text = "Die Stadt München vereinbarte die Harbor Union Übereinkunft gestern";
        let excerpt = excerpt_for(text, &["münchen".to_string()]);
        assert!(excerpt.contains("München"));
        assert!(!excerpt.contains('\u{FFFD}'), "no broken chars");
    }

    #[test]
    fn query_validation_bounds_inputs_and_cursors() {
        let ok = |query: &str| ArticleSearchQuery {
            query: query.into(),
            ..Default::default()
        };
        assert!(ok("harbor union").validate().is_ok());
        assert!(ok("   ").validate().is_err());
        assert!(ok("!!!").validate().is_err());
        assert!(ok(&"x".repeat(MAX_QUERY_CHARS + 1)).validate().is_err());
        let bad_day = ArticleSearchQuery {
            query: "harbor".into(),
            day: "2026-01".into(),
            ..Default::default()
        };
        assert!(bad_day.validate().is_err());
        let bad_cursor = ArticleSearchQuery {
            query: "harbor".into(),
            cursor: Some("not-a-number".into()),
            ..Default::default()
        };
        assert!(bad_cursor.validate().is_err());
        let huge_cursor = ArticleSearchQuery {
            query: "harbor".into(),
            cursor: Some((MAX_OFFSET + 1).to_string()),
            ..Default::default()
        };
        assert!(huge_cursor.validate().is_err());
        let bad_limit = ArticleSearchQuery {
            query: "harbor".into(),
            limit: Some(0),
            ..Default::default()
        };
        assert!(bad_limit.validate().is_err());
    }
}
