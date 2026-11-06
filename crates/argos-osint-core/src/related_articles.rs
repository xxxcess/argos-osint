//! Tentative headline/description leads. Shared mentions never form evidence edges.
use crate::{
    atlas_metadata::{self, ArticleMention, MetadataState},
    store::{AtlasArticleRow, Store},
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
pub const MATCH_VERSION: &str = "shared-headline-entities-v4-possessive";
pub const MIN_SHARED_ENTITIES: usize = 2;
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RelatedArticle {
    pub article: AtlasArticleRow,
    /// Shared mentions (per-cycle leads) or labelled bases (all-retained).
    pub shared_mentions: Vec<String>,
    pub originating_run_ids: Vec<String>,
    pub place_only: bool,
    /// All-retained rows: every occurrence of this URL.
    pub origins: Vec<ArticleOrigin>,
    /// False only when a shared canonical actor established the row.
    pub established: bool,
}
impl RelatedArticle {
    pub fn reason(&self) -> String {
        format!("Shared mentions: {}", self.shared_mentions.join(", "))
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RelatedArticlePage {
    pub rows: Vec<RelatedArticle>,
    pub total: usize,
    pub offset: usize,
    pub has_more: bool,
    pub coverage: String,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArticleOrigin {
    pub run_id: String,
    pub article_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SharedMention {
    key: String,
    type_hint: String,
    pub(crate) original_text: String,
    left: ArticleMention,
    right: ArticleMention,
}
fn named(kind: &str) -> bool {
    matches!(kind, "person" | "organization" | "place")
}
fn recognized(category: &str) -> bool {
    crate::atlas_taxonomy::DOMAIN_IDS.contains(&category)
        || matches!(
            category,
            "geopolitical"
                | "economic"
                | "military"
                | "science"
                | "health"
                | "stability"
                | "technology"
        )
}

fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '@'
}
/// Unicode case expansion retains a byte map into the original source.
fn find_exact(text: &str, key: &str, kind: &str, field: &str) -> Option<ArticleMention> {
    let mut lowered = String::new();
    let mut starts = HashMap::new();
    let mut ends = HashMap::new();
    let mut prior_space = false;
    for (start, c) in text.char_indices() {
        if c.is_whitespace() && prior_space {
            ends.insert(lowered.len(), start + c.len_utf8());
            continue;
        }
        let at = lowered.len();
        if c.is_whitespace() {
            lowered.push(' ');
        } else {
            lowered.extend(c.to_lowercase());
        }
        prior_space = c.is_whitespace();
        starts.insert(at, start);
        ends.insert(lowered.len(), start + c.len_utf8());
    }
    for (at, _) in lowered.match_indices(key) {
        let Some(&start) = starts.get(&at) else {
            continue;
        };
        let Some(&end) = ends.get(&(at + key.len())) else {
            continue;
        };
        if text[..start].chars().next_back().is_some_and(word)
            || text[end..].chars().next().is_some_and(word)
        {
            continue;
        }
        return Some(ArticleMention {
            type_hint: kind.into(),
            comparison_key: key.into(),
            field: field.into(),
            start_byte: start,
            end_byte: end,
            original_text: text[start..end].into(),
        });
    }
    None
}
fn valid(article: &AtlasArticleRow, m: &ArticleMention) -> bool {
    let text = match m.field.as_str() {
        "title" => &article.title,
        "description" => &article.description,
        _ => return false,
    };
    text.get(m.start_byte..m.end_byte) == Some(m.original_text.as_str())
        && atlas_metadata::comparison_key(&m.original_text) == m.comparison_key
}

fn mention_key(mention: &ArticleMention) -> String {
    atlas_metadata::comparison_key(&crate::brain_actors::normalize_name(&mention.original_text))
}

/// Distinct, span-backed names shared by two exact article occurrences.
/// Repeated mentions of one name count once; metadata country/category fields
/// never count. A retained typed anchor may match exact prospective text.
pub fn shared_entity_mentions(
    store: &Store,
    left: &AtlasArticleRow,
    right: &AtlasArticleRow,
) -> Result<Vec<String>> {
    let metadata = [
        store.atlas_metadata(&left.run_id, &left.id)?,
        store.atlas_metadata(&right.run_id, &right.id)?,
    ];
    Ok(shared_entity_mentions_with(
        left,
        metadata[0].as_ref(),
        right,
        metadata[1].as_ref(),
    ))
}

/// [`shared_entity_mentions`] on metadata the caller already read (the
/// SurrealDB actor index reads it from the repository).
pub fn shared_entity_mentions_with(
    left: &AtlasArticleRow,
    left_metadata: Option<&crate::atlas_metadata::ArticleMetadata>,
    right: &AtlasArticleRow,
    right_metadata: Option<&crate::atlas_metadata::ArticleMetadata>,
) -> Vec<String> {
    let metadata = [left_metadata.cloned(), right_metadata.cloned()];
    let articles = [left, right];
    let mut shared = BTreeMap::new();
    for side in 0..2 {
        let Some(m) = metadata[side].as_ref().filter(|m| {
            m.entity_state == MetadataState::Complete
                && m.input_hash == atlas_metadata::input_hash(articles[side])
        }) else {
            continue;
        };
        for mention in m.mentions.iter().filter(|mention| {
            let text = if mention.field == "title" {
                &articles[side].title
            } else {
                &articles[side].description
            };
            named(&mention.type_hint)
                && valid(articles[side], mention)
                && !atlas_metadata::temporal_person_mention(text, mention, &m.mentions)
        }) {
            let target = articles[1 - side];
            if metadata[1 - side].as_ref().is_some_and(|m| {
                m.mentions.iter().any(|other| {
                    mention_key(other) == mention_key(mention)
                        && named(&other.type_hint)
                        && other.type_hint != mention.type_hint
                })
            }) {
                continue;
            }
            let found = find_exact(
                &target.title,
                &mention_key(mention),
                &mention.type_hint,
                "title",
            )
            .or_else(|| {
                find_exact(
                    &target.description,
                    &mention_key(mention),
                    &mention.type_hint,
                    "description",
                )
            });
            if let Some(found) = found {
                let target_text = if found.field == "title" {
                    &target.title
                } else {
                    &target.description
                };
                if atlas_metadata::temporal_person_mention(
                    target_text,
                    &found,
                    metadata[1 - side]
                        .as_ref()
                        .map_or(&[], |m| m.mentions.as_slice()),
                ) {
                    continue;
                }
                if metadata[1 - side].as_ref().is_some_and(|m| {
                    m.mentions.iter().any(|other| {
                        named(&other.type_hint)
                            && other.field == found.field
                            && mention_key(other) != mention_key(mention)
                            && other.start_byte < found.end_byte
                            && found.start_byte < other.end_byte
                    })
                }) {
                    continue;
                }
                shared
                    .entry(mention_key(mention))
                    .or_insert_with(|| mention.original_text.clone());
            }
        }
    }
    shared.into_values().collect()
}
/// Only ingest-owned Related records with both exact current endpoints qualify.
pub(crate) fn current_ingest_related(
    store: &Store,
    payload: &crate::memory_metadata::RelatedPayload,
) -> Result<bool> {
    if !payload
        .source_revisions
        .iter()
        .any(|r| r.starts_with("ingest:"))
    {
        return Ok(false);
    }
    for endpoint in [&payload.source, &payload.target] {
        let crate::memory_metadata::EndpointRef::Article {
            run_id,
            article_id,
            revision,
        } = endpoint
        else {
            return Ok(false);
        };
        let Some(article) = store.atlas_article(run_id, article_id)? else {
            return Ok(false);
        };
        if revision != &atlas_metadata::input_hash(&article) {
            return Ok(false);
        }
    }
    Ok(true)
}
/// The cache key and input revision of one run's Related build: equal
/// revisions (and an equal stored pair count) mean the stored pairs are current.
pub fn build_input(
    run_id: &str,
    articles: &[AtlasArticleRow],
    metadata: &[Option<crate::atlas_metadata::ArticleMetadata>],
) -> Result<(String, String)> {
    let build_key = format!("atlas-related-input:{run_id}");
    let input_revision = crate::nlp::cache::hash(&serde_json::to_string(&(
        MATCH_VERSION,
        articles
            .iter()
            .zip(metadata.iter())
            .map(|(a, m)| {
                (
                    &a.id,
                    atlas_metadata::input_hash(a),
                    &a.category,
                    m.as_ref().map(|m| (&m.entity_state, &m.mentions)),
                )
            })
            .collect::<Vec<_>>(),
    ))?);
    Ok((build_key, input_revision))
}

/// Whether a stored build marker matches this revision and pair count.
pub fn build_is_current(marker: Option<&str>, input_revision: &str, count: usize) -> bool {
    marker
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .is_some_and(|cached| {
            cached.get("revision").and_then(|v| v.as_str()) == Some(input_revision)
                && cached.get("count").and_then(|v| v.as_u64()) == Some(count as u64)
        })
}

/// The build marker stored after publishing `count` pairs.
pub fn build_marker(input_revision: &str, count: usize) -> Result<String> {
    Ok(serde_json::to_string(
        &serde_json::json!({"revision":input_revision,"count":count}),
    )?)
}

/// One Related pair ready to store: `left`/`right` index the input articles
/// (`left.id < right.id`); the caller checks both are still current.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedPair {
    pub left: usize,
    pub right: usize,
    pub left_revision: String,
    pub right_revision: String,
    pub shared_json: String,
    pub named_count: i64,
    pub shared_count: i64,
    pub rarity: f64,
}

/// Inverted token and entity postings avoid comparing all article pairs.
pub fn build(_store: &Store, run_id: &str) -> Result<usize> {
    let run_id = run_id.to_owned();
    crate::storage::runtime::blocking(move |repo| async move {
        let store = crate::atlas_store::AtlasStore::from_repository(repo.clone());
        build_surreal(&store, &repo, &run_id).await
    })
}

/// [`build`] on the SurrealDB repository: same plan, same input marker
/// (`app_state`), published by `storage::atlas_metadata::replace_related`.
pub async fn build_on(store: &crate::atlas_store::AtlasStore, run_id: &str) -> Result<usize> {
    let Some(repo) = store.repository() else {
        anyhow::bail!("SurrealDB storage is not configured");
    };
    build_surreal(store, repo, run_id).await
}

#[cfg(feature = "surreal")]
async fn build_surreal(
    _store: &crate::atlas_store::AtlasStore,
    repo: &crate::storage::repo::Repository,
    run_id: &str,
) -> Result<usize> {
    use crate::storage::classifier_admission::{self, DerivativeStage};
    let owner = classifier_admission::AdmissionOwner::new(repo.clone());
    let result = async {
        let mut after = None;
        let mut processed = 0usize;
        let cancel = std::sync::atomic::AtomicBool::new(false);
        loop {
            let keys = crate::storage::atlas::current_occurrence_keys_page(
                repo,
                Some(run_id),
                None,
                after.as_ref(),
            )
            .await?;
            if keys.is_empty() {
                break;
            }
            after = keys.last().cloned();
            processed += keys.len();
            let report = classifier_admission::process_native_refresh_stage(
                &owner,
                &keys,
                DerivativeStage::Related,
                &cancel,
            )
            .await?;
            if report.states.iter().any(|work| {
                !work
                    .stages
                    .iter()
                    .find(|stage| stage.stage == DerivativeStage::Related)
                    .and_then(|stage| stage.disposition.as_ref())
                    .is_some_and(classifier_admission::DerivativeDisposition::terminal)
            }) {
                anyhow::bail!("Related native inputs remain unavailable or changed");
            }
        }
        let mut response = repo
            .db()
            .query("RETURN count(SELECT id FROM atlas_article WHERE run_id=$run);")
            .bind(("run", run_id.to_owned()))
            .await?
            .check()?;
        let retained = response
            .take::<Option<i64>>(0)?
            .ok_or_else(|| anyhow::anyhow!("missing retained Related count"))?;
        if usize::try_from(retained).ok() != Some(processed) {
            anyhow::bail!("Related refresh ineligible for legacy, rejected or pending occurrences");
        }
        crate::storage::atlas_metadata::related_count(repo.db(), run_id)
            .await
            .map_err(anyhow::Error::from)
    }
    .await;
    let cleanup = owner.shutdown().await;
    let result = result?;
    cleanup?;
    Ok(result)
}

/// `Store::related_articles` on the selected Atlas store: under SurrealDB
/// the pairs come from `storage::atlas_metadata::related_pairs` and the
/// articles and metadata from the repository; otherwise the legacy store.
pub async fn related_articles_on(
    store: &crate::atlas_store::AtlasStore,
    run_id: &str,
    article_id: &str,
    offset: usize,
    limit: usize,
) -> Result<RelatedArticlePage> {
    let Some(repo) = store.repository() else {
        anyhow::bail!("SurrealDB storage is not configured");
    };
    related_surreal(store, repo, run_id, article_id, offset, limit).await
}

#[cfg(feature = "surreal")]
async fn related_surreal(
    store: &crate::atlas_store::AtlasStore,
    repo: &crate::storage::repo::Repository,
    run_id: &str,
    article_id: &str,
    offset: usize,
    limit: usize,
) -> Result<RelatedArticlePage> {
    let limit = limit.clamp(1, 100);
    let articles = store.current_articles(run_id).await?;
    let lookup: HashMap<_, _> = articles.iter().map(|a| (a.id.as_str(), a)).collect();
    let mut pairs = crate::storage::atlas_metadata::related_pairs(repo.db(), run_id, article_id)
        .await
        .map_err(|e| anyhow::anyhow!("atlas related pairs: {e}"))?;
    // legacy store joins the counterpart article; pairs whose counterpart is gone drop out.
    let other = |p: &crate::storage::atlas_metadata::RelatedPairRow| {
        if p.left_id == article_id {
            p.right_id.clone()
        } else {
            p.left_id.clone()
        }
    };
    pairs.retain(|p| lookup.contains_key(other(p).as_str()));
    pairs.sort_by(|a, b| {
        let (oa, ob) = (lookup[other(a).as_str()], lookup[other(b).as_str()]);
        (a.named_count == 0)
            .cmp(&(b.named_count == 0))
            .then_with(|| b.shared_count.cmp(&a.shared_count))
            .then_with(|| b.rarity.total_cmp(&a.rarity))
            .then_with(|| ob.published_at.cmp(&oa.published_at))
            .then_with(|| oa.id.cmp(&ob.id))
    });
    let mut metadata: HashMap<String, Option<crate::atlas_metadata::ArticleMetadata>> =
        HashMap::new();
    let mut rows = vec![];
    for p in pairs {
        let (Some(a), Some(b)) = (
            lookup.get(p.left_id.as_str()),
            lookup.get(p.right_id.as_str()),
        ) else {
            continue;
        };
        if revision(a) != p.left_revision || revision(b) != p.right_revision {
            continue;
        }
        for id in [&a.id, &b.id] {
            if !metadata.contains_key(id) {
                metadata.insert(id.clone(), store.metadata(run_id, id).await?);
            }
        }
        let shared: Vec<SharedMention> = serde_json::from_str(&p.shared_json)?;
        if shared_entity_mentions_with(a, metadata[&a.id].as_ref(), b, metadata[&b.id].as_ref())
            .len()
            < MIN_SHARED_ENTITIES
        {
            continue;
        }
        let target = if p.left_id == article_id { b } else { a };
        rows.push(RelatedArticle {
            article: (*target).clone(),
            shared_mentions: shared.into_iter().map(|m| m.original_text).collect(),
            originating_run_ids: vec![run_id.into()],
            place_only: p.named_count == 0,
            ..RelatedArticle::default()
        });
    }
    let total = rows.len();
    let rows = rows.into_iter().skip(offset).take(limit).collect();
    let coverage = match store.metadata(run_id, article_id).await? {
        Some(m) if m.entity_state == MetadataState::Complete => "available",
        Some(m) if m.entity_state == MetadataState::Pending => "loading",
        _ => "unavailable",
    }
    .into();
    Ok(RelatedArticlePage {
        rows,
        total,
        offset,
        has_more: offset.saturating_add(limit) < total,
        coverage,
    })
}

/// `Store::related_articles_for_memory` on the selected Atlas store: the
/// memory's origins (`storage::browse_assoc::article_origins_for_memory`
/// under SurrealDB) and the union of their Related pages.
pub async fn related_articles_for_memory_on(
    store: &crate::atlas_store::AtlasStore,
    memory_id: &str,
    offset: usize,
    limit: usize,
) -> Result<RelatedArticlePage> {
    let Some(repo) = store.repository() else {
        anyhow::bail!("SurrealDB storage is not configured");
    };
    let origins = crate::storage::browse_assoc::article_origins_for_memory(repo, memory_id).await?;
    let mut pages = Vec::new();
    for origin in &origins {
        let mut start = 0;
        loop {
            let page =
                related_articles_on(store, &origin.run_id, &origin.article_id, start, 100).await?;
            let more = page.has_more;
            pages.push(page);
            if !more {
                break;
            }
            start += 100;
        }
    }
    Ok(union_pages(&origins, pages, offset, limit))
}

/// Merge the Related pages of a memory's origins (shared by both stores).
pub(crate) fn union_pages(
    origins: &[ArticleOrigin],
    pages: Vec<RelatedArticlePage>,
    offset: usize,
    limit: usize,
) -> RelatedArticlePage {
    let excluded: BTreeSet<_> = origins.iter().map(|o| o.article_id.as_str()).collect();
    let mut union: BTreeMap<String, RelatedArticle> = BTreeMap::new();
    let mut available = false;
    for page in pages {
        available |= page.coverage == "available";
        for row in page.rows {
            if excluded.contains(row.article.id.as_str()) {
                continue;
            }
            union
                .entry(row.article.id.clone())
                .and_modify(|r| {
                    for mention in &row.shared_mentions {
                        if !r.shared_mentions.contains(mention) {
                            r.shared_mentions.push(mention.clone());
                        }
                    }
                    for run in &row.originating_run_ids {
                        if !r.originating_run_ids.contains(run) {
                            r.originating_run_ids.push(run.clone());
                        }
                    }
                    r.place_only &= row.place_only;
                })
                .or_insert(row);
        }
    }
    let mut rows: Vec<_> = union.into_values().collect();
    rows.sort_by(|a, b| {
        a.place_only
            .cmp(&b.place_only)
            .then_with(|| b.shared_mentions.len().cmp(&a.shared_mentions.len()))
            .then_with(|| b.article.published_at.cmp(&a.article.published_at))
            .then_with(|| a.article.id.cmp(&b.article.id))
    });
    let total = rows.len();
    let limit = limit.clamp(1, 100);
    let rows = rows.into_iter().skip(offset).take(limit).collect();
    RelatedArticlePage {
        rows,
        total,
        offset,
        has_more: offset.saturating_add(limit) < total,
        coverage: if available {
            "available"
        } else {
            "unavailable"
        }
        .into(),
    }
}

/// The Related pairs of one run's articles and metadata (pure; both stores
/// publish its output).
pub fn plan(
    articles: &[AtlasArticleRow],
    metadata: &[Option<crate::atlas_metadata::ArticleMetadata>],
) -> Result<Vec<PlannedPair>> {
    let mut tokens: HashMap<String, BTreeSet<usize>> = HashMap::new();
    for (i, a) in articles.iter().enumerate() {
        for token in a
            .title
            .split(|c: char| !word(c))
            .chain(a.description.split(|c: char| !word(c)))
            .filter(|s| !s.is_empty())
        {
            tokens.entry(token.to_lowercase()).or_default().insert(i);
        }
    }
    let mut anchors: BTreeMap<(String, String), Vec<(usize, ArticleMention)>> = BTreeMap::new();
    for (i, m) in metadata.iter().enumerate() {
        if let Some(m) = m {
            if m.entity_state == MetadataState::Complete
                && m.input_hash == atlas_metadata::input_hash(&articles[i])
            {
                for mention in &m.mentions {
                    let text = if mention.field == "title" {
                        &articles[i].title
                    } else {
                        &articles[i].description
                    };
                    if named(&mention.type_hint)
                        && valid(&articles[i], mention)
                        && !atlas_metadata::temporal_person_mention(text, mention, &m.mentions)
                    {
                        anchors
                            .entry((mention.type_hint.clone(), mention_key(mention)))
                            .or_default()
                            .push((i, mention.clone()));
                    }
                }
            }
        }
    }
    let mut pairs: BTreeMap<(usize, usize), Vec<(SharedMention, f64)>> = BTreeMap::new();
    for ((kind, key), posting) in anchors {
        let first = key
            .split(|c: char| !word(c))
            .find(|s| !s.is_empty())
            .unwrap_or("");
        let targets = tokens.get(first).cloned().unwrap_or_default();
        let seeds: BTreeSet<usize> = posting
            .iter()
            .filter(|(i, _)| recognized(&articles[*i].category))
            .map(|(i, _)| *i)
            .collect();
        let mut mentions: BTreeMap<usize, ArticleMention> = posting.into_iter().collect();
        for i in targets {
            if mentions.contains_key(&i) {
                continue;
            }
            // An explicitly incompatible annotation cannot be overridden by textual fallback.
            if metadata[i].as_ref().is_some_and(|m| {
                m.mentions
                    .iter()
                    .any(|m| mention_key(m) == key && named(&m.type_hint) && m.type_hint != kind)
            }) {
                continue;
            }
            if let Some(m) = find_exact(&articles[i].title, &key, &kind, "title")
                .or_else(|| find_exact(&articles[i].description, &key, &kind, "description"))
            {
                let text = if m.field == "title" {
                    &articles[i].title
                } else {
                    &articles[i].description
                };
                if atlas_metadata::temporal_person_mention(
                    text,
                    &m,
                    metadata[i].as_ref().map_or(&[], |m| m.mentions.as_slice()),
                ) {
                    continue;
                }
                let conflicting_span = metadata[i].as_ref().is_some_and(|metadata| {
                    metadata.mentions.iter().any(|other| {
                        named(&other.type_hint)
                            && other.field == m.field
                            && mention_key(other) != mention_key(&m)
                            && other.start_byte < m.end_byte
                            && m.start_byte < other.end_byte
                    })
                });
                if !conflicting_span {
                    mentions.insert(i, m);
                }
            }
        }
        let rarity = 1.0 / (mentions.len().max(1) as f64);
        let postings: Vec<_> = mentions.into_iter().collect();
        for (p, (i, left)) in postings.iter().enumerate() {
            for (j, right) in postings.iter().skip(p + 1) {
                let a = &articles[*i];
                let b = &articles[*j];
                if !(seeds.contains(i) || seeds.contains(j))
                    || a.id == b.id
                    || (!a.url.is_empty() && canonical_url(&a.url) == canonical_url(&b.url))
                    || (!recognized(&a.category) && !recognized(&b.category))
                {
                    continue;
                }
                if !valid(a, left) || !valid(b, right) {
                    continue;
                }
                pairs.entry((*i, *j)).or_default().push((
                    SharedMention {
                        key: key.clone(),
                        type_hint: kind.clone(),
                        original_text: left.original_text.clone(),
                        left: left.clone(),
                        right: right.clone(),
                    },
                    rarity,
                ));
            }
        }
    }
    let mut out = Vec::new();
    for ((i, j), mut shared) in pairs {
        shared.sort_by(|a, b| a.0.key.cmp(&b.0.key));
        shared.dedup_by(|a, b| a.0.key == b.0.key && a.0.type_hint == b.0.type_hint);
        if shared
            .iter()
            .map(|(s, _)| &s.key)
            .collect::<BTreeSet<_>>()
            .len()
            < MIN_SHARED_ENTITIES
        {
            continue;
        }
        let (left, right) = if articles[i].id < articles[j].id {
            (i, j)
        } else {
            for (s, _) in &mut shared {
                std::mem::swap(&mut s.left, &mut s.right);
            }
            (j, i)
        };
        let named_count = shared
            .iter()
            .filter(|(s, _)| s.type_hint != "place")
            .count();
        let rarity: f64 = shared.iter().map(|(_, r)| r).sum();
        out.push(PlannedPair {
            left,
            right,
            left_revision: revision(&articles[left]),
            right_revision: revision(&articles[right]),
            shared_json: serde_json::to_string(&shared.iter().map(|(s, _)| s).collect::<Vec<_>>())?,
            named_count: named_count as i64,
            shared_count: shared.len() as i64,
            rarity,
        });
    }
    Ok(out)
}
pub(crate) fn revision(a: &AtlasArticleRow) -> String {
    // The ordered categories are part of the pair's identity: a re-admission
    // with different tags refreshes the planned pair revisions.
    crate::nlp::cache::hash(&format!(
        "{}:{}",
        atlas_metadata::input_hash(a),
        serde_json::to_string(&a.categories).expect("domain strings serialize")
    ))
}
pub(crate) fn canonical_url(raw: &str) -> String {
    reqwest::Url::parse(raw)
        .map(|mut u| {
            u.set_fragment(None);
            u.to_string()
        })
        .unwrap_or_else(|_| raw.into())
}

/// Rebuilds only pairs touching one captured occurrence. Candidate retrieval
/// is indexed and bounded to 100 raw hits; a continuation remains durable.
pub(crate) async fn process_derivative(
    owner: &crate::storage::classifier_admission::AdmissionOwner,
    fence: &crate::storage::classifier_admission::DerivativeFence,
    claim: &crate::storage::classifier_admission::DerivativeClaim,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<crate::storage::classifier_admission::DerivativeDisposition> {
    use crate::storage::classifier_admission::{self, DerivativeDisposition, DerivativeStage};
    use crate::storage::tx::{RetryPolicy, TxError};
    if cancel.load(std::sync::atomic::Ordering::Acquire) {
        return Ok(DerivativeDisposition::Retryable {
            code: "cancelled".into(),
        });
    }
    let Some(metadata) = claim.metadata.as_ref() else {
        return Ok(classifier_admission::release_derivative(
            owner,
            fence,
            claim,
            DerivativeDisposition::Retryable {
                code: "related_metadata_missing".into(),
            },
        )
        .await?);
    };
    if metadata.entity_state == MetadataState::Skipped {
        let fence = fence.clone();
        let claim = claim.clone();
        return Ok(
            classifier_admission::owned_tx(owner, RetryPolicy::default(), move |tx| {
                let fence = fence.clone();
                let claim = claim.clone();
                async move {
                    let now = chrono::Utc::now();
                    if !classifier_admission::check_derivative_in(&tx, &fence, &claim, now).await? {
                        return Ok(DerivativeDisposition::StaleInput);
                    }
                    let disposition = DerivativeDisposition::Disabled {
                        reason: "local_entities_disabled".into(),
                    };
                    if !classifier_admission::complete_derivative_in(
                        &tx,
                        &fence,
                        &claim,
                        &disposition,
                        "",
                        now,
                    )
                    .await?
                    {
                        return Err(TxError::Abort("related_disabled_lease_lost".into()));
                    }
                    Ok(disposition)
                }
            })
            .await?
            .value,
        );
    }
    if metadata.entity_state != MetadataState::Complete
        && metadata.entity_state != MetadataState::Unsupported
    {
        return Ok(classifier_admission::release_derivative(
            owner,
            fence,
            claim,
            DerivativeDisposition::Retryable {
                code: "related_entities_unavailable".into(),
            },
        )
        .await?);
    }
    let cursor = if claim.checkpoint_json.is_empty() {
        None
    } else {
        Some(claim.checkpoint_json.as_str())
    };
    let candidates = crate::storage::article_discovery::related_from_seed_bounded_after(
        owner.repository(),
        &claim.article,
        100,
        cursor,
    )
    .await?;
    let keys: Vec<_> = candidates
        .hits
        .iter()
        .filter(|hit| hit.run_id == claim.article.run_id && hit.article_id != claim.article.id)
        .map(|hit| crate::atlas_classify::OccurrenceKey::new(&hit.run_id, &hit.article_id))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let first = claim.checkpoint_json.is_empty();
    let next = candidates.next_cursor.clone();
    let truncated = candidates.truncated;
    let captured = classifier_admission::owned_tx(owner, RetryPolicy::default(), move |tx| {
        let keys = keys.clone();
        async move {
            let mut rows = BTreeMap::new();
            for keys in keys.chunks(64) {
                rows.extend(
                    classifier_admission::derivative_snapshots_in(
                        &tx,
                        keys,
                        DerivativeStage::Related,
                    )
                    .await?,
                );
            }
            Ok(rows)
        }
    })
    .await?
    .value;
    let mut articles = vec![claim.article.clone()];
    let mut metadata = vec![claim.metadata.clone()];
    let mut captures = vec![claim.snapshot.clone()];
    for (snapshot, article, native) in captured.values() {
        if snapshot.admission_state != "admitted" {
            continue;
        }
        articles.push(article.clone());
        metadata.push(native.clone());
        captures.push(snapshot.clone());
    }
    let planned = plan(&articles, &metadata)?;
    let planned: Vec<_> = planned
        .into_iter()
        .filter(|pair| pair.left == 0 || pair.right == 0)
        .collect();
    let fence = fence.clone();
    let claim = claim.clone();
    let committed=classifier_admission::owned_tx(owner,RetryPolicy::default(),move|tx|{let fence=fence.clone();let claim=claim.clone();let articles=articles.clone();let planned=planned.clone();let captures=captures.clone();let next=next.clone();async move {
        let now=chrono::Utc::now();if !classifier_admission::check_derivative_in(&tx,&fence,&claim,now).await?{return Ok(DerivativeDisposition::StaleInput);}
        let keys:Vec<_>=articles.iter().skip(1).map(|article|crate::atlas_classify::OccurrenceKey::new(&article.run_id,&article.id)).collect();
        let mut current=BTreeMap::new();for keys in keys.chunks(64){current.extend(classifier_admission::derivative_snapshots_in(&tx,keys,DerivativeStage::Related).await?);}
        if first {
            crate::storage::jobs::run(&tx,tx.query("DELETE atlas_related_article WHERE run_id=$run AND left_id=$article RETURN NONE;DELETE atlas_related_article WHERE run_id=$run AND right_id=$article RETURN NONE;").bind(("run",claim.article.run_id.clone())).bind(("article",claim.article.id.clone()))).await?;
        }
        let mut conflicted=false;
        for pair in &planned {
            let(left,right)=(&articles[pair.left],&articles[pair.right]);let other=if pair.left==0{pair.right}else{pair.left};
            let key=crate::atlas_classify::OccurrenceKey::new(&articles[other].run_id,&articles[other].id);
            if current.get(&key).map(|(snapshot,_,_)|snapshot.revision(DerivativeStage::Related)).transpose()?.as_deref()!=Some(captures[other].revision(DerivativeStage::Related)?.as_str()){conflicted=true;continue;}
            crate::storage::jobs::run(&tx,tx.query("UPSERT type::record('atlas_related_article',$key) CONTENT {run_id:$run,left_id:$left,right_id:$right,left_revision:$lr,right_revision:$rr,left_native_revision:$ln,right_native_revision:$rn,shared_json:$shared,match_version:$version,named_count:$named,shared_count:$count,rarity:$rarity} RETURN NONE;")
                .bind(("key",crate::storage::codec::encode(&[&left.run_id,&left.id,&right.id]))).bind(("run",left.run_id.clone())).bind(("left",left.id.clone())).bind(("right",right.id.clone())).bind(("lr",revision(left))).bind(("rr",revision(right))).bind(("ln",captures[pair.left].revision(DerivativeStage::Related)?)).bind(("rn",captures[pair.right].revision(DerivativeStage::Related)?)).bind(("shared",pair.shared_json.clone())).bind(("version",MATCH_VERSION)).bind(("named",pair.named_count)).bind(("count",pair.shared_count)).bind(("rarity",pair.rarity))).await?;
        }
        let(disposition,checkpoint)=if conflicted {(DerivativeDisposition::Retryable {code:"related_candidate_changed".into()},String::new())}
            else if let Some(next)=next {(DerivativeDisposition::Retryable {code:"related_continuation".into()},next)}
            else if truncated {(DerivativeDisposition::Retryable {code:"related_discovery_cap".into()},String::new())}
            else {(DerivativeDisposition::Current,String::new())};
        if !classifier_admission::complete_derivative_in(&tx,&fence,&claim,&disposition,&checkpoint,now).await?{return Err(TxError::Abort("related_completion_lease_lost".into()));}Ok(disposition)
    }}).await?;
    Ok(committed.value)
}
