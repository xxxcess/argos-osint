//! Tentative headline/description leads. Shared mentions never form evidence edges.
use crate::{
    atlas_metadata::{self, ArticleMention, MetadataState},
    store::{AtlasArticleRow, Store},
};
use anyhow::Result;
use rusqlite::params;
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
struct SharedMention {
    key: String,
    type_hint: String,
    original_text: String,
    left: ArticleMention,
    right: ArticleMention,
}
fn named(kind: &str) -> bool {
    matches!(kind, "person" | "organization" | "place")
}
fn recognized(category: &str) -> bool {
    matches!(
        category,
        "geopolitical" | "economic" | "military" | "information" | "stability" | "technology"
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
    Ok(shared.into_values().collect())
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
/// Inverted token and entity postings avoid comparing all article pairs.
pub fn build(store: &Store, run_id: &str) -> Result<usize> {
    let articles = store.atlas_list_articles(run_id)?;
    let mut metadata = Vec::new();
    for a in &articles {
        metadata.push(store.atlas_metadata(run_id, &a.id)?);
    }
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
    if let Some(cached) = store
        .app_state_get(&build_key)?
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
    {
        let count: usize = store.conn.query_row(
            "SELECT COUNT(*) FROM atlas_related_articles WHERE run_id=?1",
            [run_id],
            |r| r.get(0),
        )?;
        if cached.get("revision").and_then(|v| v.as_str()) == Some(input_revision.as_str())
            && cached.get("count").and_then(|v| v.as_u64()) == Some(count as u64)
        {
            return Ok(count);
        }
    }
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
    let tx = store.conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM atlas_related_articles WHERE run_id=?1",
        [run_id],
    )?;
    let mut count = 0;
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
        let a = &articles[i];
        let b = &articles[j];
        let (left, right) = if a.id < b.id {
            (a, b)
        } else {
            for (s, _) in &mut shared {
                std::mem::swap(&mut s.left, &mut s.right);
            }
            (b, a)
        };
        // Source/category revisions are checked inside this transaction before publication.
        let current_left = store.atlas_article(run_id, &left.id)?;
        let current_right = store.atlas_article(run_id, &right.id)?;
        if current_left.as_ref() != Some(left) || current_right.as_ref() != Some(right) {
            continue;
        }
        let named_count = shared
            .iter()
            .filter(|(s, _)| s.type_hint != "place")
            .count();
        let rarity: f64 = shared.iter().map(|(_, r)| r).sum();
        tx.execute(
            "INSERT INTO atlas_related_articles VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                run_id,
                left.id,
                right.id,
                revision(left),
                revision(right),
                serde_json::to_string(&shared.iter().map(|(s, _)| s).collect::<Vec<_>>())?,
                MATCH_VERSION,
                named_count as i64,
                shared.len() as i64,
                rarity
            ],
        )?;
        count += 1;
    }
    tx.execute("INSERT INTO app_state(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![build_key,serde_json::to_string(&serde_json::json!({"revision":input_revision,"count":count}))?])?;
    tx.commit()?;
    Ok(count)
}
fn revision(a: &AtlasArticleRow) -> String {
    crate::nlp::cache::hash(&format!("{}:{}", atlas_metadata::input_hash(a), a.category))
}
pub(crate) fn canonical_url(raw: &str) -> String {
    reqwest::Url::parse(raw)
        .map(|mut u| {
            u.set_fragment(None);
            u.to_string()
        })
        .unwrap_or_else(|_| raw.into())
}
impl Store {
    /// Whether an occurrence participates in a retained ingest association
    /// containing this actor name and at least one other distinct entity.
    /// This checks stored pairs, never creates suggestions while browsing.
    pub fn has_ingest_actor_association(
        &self,
        article: &AtlasArticleRow,
        name_key: &str,
    ) -> Result<bool> {
        let mut offset = 0;
        loop {
            let page = self.related_articles(&article.run_id, &article.id, offset, 100)?;
            for row in &page.rows {
                if shared_entity_mentions(self, article, &row.article)?
                    .iter()
                    .any(|name| crate::brain_actors::normalize_alias(name) == name_key)
                {
                    return Ok(true);
                }
            }
            if !page.has_more {
                break;
            }
            offset += 100;
        }
        let mut stmt=self.conn.prepare("SELECT other.origin,other.ref_id,e.memory_id FROM brain_memory_endpoints e JOIN brain_memory_endpoints other ON other.memory_id=e.memory_id AND other.side<>e.side JOIN memory_metadata m ON m.memory_id=e.memory_id WHERE e.endpoint_kind='article' AND e.origin=?1 AND e.ref_id=?2 AND other.endpoint_kind='article' AND m.memory_kind='related' AND m.lifecycle='active'")?;
        let candidates = stmt
            .query_map(params![article.run_id, article.id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (run, id, memory) in candidates {
            let Some(crate::memory_metadata::MemoryPayload::Related(payload)) =
                self.memory_payload(&memory)?
            else {
                continue;
            };
            if !current_ingest_related(self, &payload)? {
                continue;
            }
            let Some(candidate) = self.atlas_article(&run, &id)? else {
                continue;
            };
            let shared = shared_entity_mentions(self, article, &candidate)?;
            if shared.len() >= MIN_SHARED_ENTITIES
                && shared
                    .iter()
                    .any(|name| crate::brain_actors::normalize_alias(name) == name_key)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub fn related_articles(
        &self,
        run_id: &str,
        article_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<RelatedArticlePage> {
        let limit = limit.clamp(1, 100);
        let articles = self.atlas_list_articles(run_id)?;
        let lookup: HashMap<_, _> = articles.iter().map(|a| (a.id.as_str(), a)).collect();
        let mut stmt=self.conn.prepare("SELECT p.left_id,p.right_id,p.left_revision,p.right_revision,p.shared_json,p.named_count FROM atlas_related_articles p JOIN atlas_articles a ON a.run_id=p.run_id AND a.article_id=CASE WHEN p.left_id=?2 THEN p.right_id ELSE p.left_id END WHERE p.run_id=?1 AND (p.left_id=?2 OR p.right_id=?2) ORDER BY CASE WHEN p.named_count>0 THEN 0 ELSE 1 END,p.shared_count DESC,p.rarity DESC,a.published_at DESC,a.article_id")?;
        let records = stmt
            .query_map(params![run_id, article_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut rows = vec![];
        for (left, right, lr, rr, json, named_count) in records {
            let (Some(a), Some(b)) = (lookup.get(left.as_str()), lookup.get(right.as_str())) else {
                continue;
            };
            if revision(a) != lr || revision(b) != rr {
                continue;
            }
            let shared: Vec<SharedMention> = serde_json::from_str(&json)?;
            if shared_entity_mentions(self, a, b)?.len() < MIN_SHARED_ENTITIES {
                continue;
            }
            let target = if left == article_id { b } else { a };
            rows.push(RelatedArticle {
                article: (*target).clone(),
                shared_mentions: shared.into_iter().map(|m| m.original_text).collect(),
                originating_run_ids: vec![run_id.into()],
                place_only: named_count == 0,
                ..RelatedArticle::default()
            });
        }
        let total = rows.len();
        let rows = rows.into_iter().skip(offset).take(limit).collect();
        let coverage = match self.atlas_metadata(run_id, article_id)? {
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
    pub fn article_origins_for_memory(&self, memory_id: &str) -> Result<Vec<ArticleOrigin>> {
        // Resolve each source independently: exact ids first, explicit URLs only as fallback.
        let mut refs: Vec<(Option<String>, String, Option<String>)> = Vec::new();
        let mut stmt=self.conn.prepare("SELECT s.owner_kind,s.owner_id,e.source_id,e.evidence_json FROM insight_derivations d JOIN insight_snapshots s ON s.id=d.snapshot_id LEFT JOIN insight_derivation_evidence e ON e.derivation_id=d.id WHERE d.memory_id=?1 AND d.status NOT IN ('deleted','retired')")?;
        for row in stmt.query_map([memory_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })? {
            let (kind, owner, source, json) = row?;
            if kind == "article" {
                refs.push((None, owner, None));
            }
            let url = json
                .as_deref()
                .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
                .and_then(|v| v.get("url").and_then(|u| u.as_str()).map(canonical_url));
            let source = source.unwrap_or_default();
            refs.push((
                None,
                source
                    .strip_prefix("article:")
                    .unwrap_or(&source)
                    .to_string(),
                url,
            ));
        }
        let mut stmt=self.conn.prepare("SELECT s.run_id,s.call_id,s.source_url FROM insight_sources s JOIN insight_claims c ON c.fingerprint=s.fingerprint WHERE c.memory_id=?1 AND s.deleted_origin=0")?;
        for row in stmt.query_map([memory_id], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })? {
            let (run, id, url) = row?;
            refs.push((
                run,
                id.strip_prefix("article:").unwrap_or(&id).to_string(),
                url.map(|u| canonical_url(&u)),
            ));
        }
        let mut articles = Vec::new();
        for run in self.atlas_list_runs()? {
            articles.extend(self.atlas_list_articles(&run.id)?);
        }
        let mut origins = BTreeSet::new();
        for (run, id, url) in refs {
            let exact: Vec<_> = articles
                .iter()
                .filter(|a| a.id == id && run.as_ref().is_none_or(|r| a.run_id == *r))
                .collect();
            if !exact.is_empty() {
                for a in exact {
                    origins.insert(ArticleOrigin {
                        run_id: a.run_id.clone(),
                        article_id: a.id.clone(),
                    });
                }
            } else if let Some(url) = url {
                for a in articles.iter().filter(|a| {
                    !a.url.is_empty()
                        && canonical_url(&a.url) == url
                        && run.as_ref().is_none_or(|r| a.run_id == *r)
                }) {
                    origins.insert(ArticleOrigin {
                        run_id: a.run_id.clone(),
                        article_id: a.id.clone(),
                    });
                }
            }
        }
        Ok(origins.into_iter().collect())
    }
    pub fn related_articles_for_memory(
        &self,
        memory_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<RelatedArticlePage> {
        let origins = self.article_origins_for_memory(memory_id)?;
        let excluded: BTreeSet<_> = origins.iter().map(|o| o.article_id.as_str()).collect();
        let mut union: BTreeMap<String, RelatedArticle> = BTreeMap::new();
        let mut available = false;
        for origin in origins.iter() {
            let mut start = 0;
            loop {
                let page = self.related_articles(&origin.run_id, &origin.article_id, start, 100)?;
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
                if !page.has_more {
                    break;
                }
                start += 100;
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
        Ok(RelatedArticlePage {
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn related_requires_two_distinct_entities_and_filters_old_single_entity_receipts() {
        let s = store();
        let mut source = article("source", "Ada and Ada meet Acme", "economic");
        source.description.clear();
        let mut one = article("one", "Ada mentions Ada", "unk");
        one.description.clear();
        let mut two = article("two", "Ada meets Acme", "unk");
        two.description.clear();
        for a in [&source, &one, &two] {
            s.atlas_upsert_article(a).unwrap();
        }
        let mut metadata = atlas_metadata::ArticleMetadata::pending(&source);
        metadata.entity_state = MetadataState::Complete;
        for (name, kind) in [("Ada", "person"), ("Acme", "organization")] {
            metadata
                .mentions
                .push(find_exact(&source.title, &name.to_lowercase(), kind, "title").unwrap());
        }
        s.atlas_save_metadata(&metadata).unwrap();
        assert_eq!(shared_entity_mentions(&s, &source, &one).unwrap().len(), 1);
        assert_eq!(shared_entity_mentions(&s, &source, &two).unwrap().len(), 2);
        assert_eq!(build(&s, "r").unwrap(), 1);
        let page = s.related_articles("r", "source", 0, 20).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.rows[0].article.id, "two");
        assert!(s
            .related_articles("r", "one", 0, 20)
            .unwrap()
            .rows
            .is_empty());
    }
    fn article(id: &str, title: &str, category: &str) -> AtlasArticleRow {
        AtlasArticleRow {
            run_id: "r".into(),
            id: id.into(),
            title: title.into(),
            description: "Nairobi".into(),
            url: format!("https://example.com/{id}"),
            country: String::new(),
            source_name: "Publisher".into(),
            source_domain: "example.com".into(),
            published_at: "2026-10-12".into(),
            provider: "fixture".into(),
            temperature: 0.0,
            category: category.into(),
            seen_at: String::new(),
            author: String::new(),
            image_url: String::new(),
        }
    }
    fn inventory(store: &Store, a: &AtlasArticleRow, key: &str, kind: &str) {
        store.atlas_upsert_article(a).unwrap();
        let mut m = atlas_metadata::ArticleMetadata::pending(a);
        m.entity_state = MetadataState::Complete;
        m.reliability_state = MetadataState::Complete;
        if let Some(mention) = find_exact(&a.title, &key.to_lowercase(), kind, "title") {
            m.mentions.push(mention);
        }
        m.mentions
            .push(find_exact(&a.description, "nairobi", "place", "description").unwrap());
        assert!(store.atlas_save_metadata(&m).unwrap());
    }
    fn store() -> Store {
        let s = Store::memory().unwrap();
        s.atlas_insert_run("r", "{}", "{}").unwrap();
        s
    }
    #[test]
    fn known_anchor_to_unk_is_reciprocal_and_boundary_checked() {
        let s = store();
        inventory(
            &s,
            &article("a", "José Acme speaks", "geopolitical"),
            "José Acme",
            "person",
        );
        inventory(&s, &article("b", "José Acme responds", "unk"), "", "person");
        inventory(
            &s,
            &article("c", "José Acmeville responds", "unk"),
            "",
            "person",
        );
        assert_eq!(build(&s, "r").unwrap(), 1);
        let p = s.related_articles("r", "a", 0, 20).unwrap();
        assert_eq!(p.total, 1);
        assert_eq!(p.rows[0].article.id, "b");
        assert_eq!(p.rows[0].shared_mentions, vec!["José Acme", "Nairobi"]);
        assert_eq!(
            s.related_articles("r", "b", 0, 20).unwrap().rows[0]
                .article
                .id,
            "a"
        );
    }
    #[test]
    fn two_unk_or_incompatible_types_never_seed_a_pair() {
        let s = store();
        inventory(&s, &article("a", "Acme opens", "unk"), "Acme", "person");
        inventory(&s, &article("b", "Acme closes", "unk"), "Acme", "person");
        assert_eq!(build(&s, "r").unwrap(), 0);
        s.atlas_set_category("r", "a", "economic").unwrap();
        inventory(
            &s,
            &article("b", "Acme closes", "unk"),
            "Acme",
            "organization",
        );
        assert_eq!(build(&s, "r").unwrap(), 0);
    }
    #[test]
    fn metadata_revisions_category_and_delete_hide_stale_pairs() {
        let s = store();
        let a = article("a", "Jane Doe speaks", "geopolitical");
        inventory(&s, &a, "Jane Doe", "person");
        inventory(
            &s,
            &article("b", "Jane Doe responds", "unk"),
            "Jane Doe",
            "person",
        );
        assert_eq!(build(&s, "r").unwrap(), 1);
        let captured = s.atlas_metadata("r", "a").unwrap().unwrap();
        let mut changed = a.clone();
        changed.title = "Different person speaks".into();
        s.atlas_upsert_article(&changed).unwrap();
        assert!(!s.atlas_save_metadata(&captured).unwrap());
        assert_eq!(s.related_articles("r", "a", 0, 20).unwrap().total, 0);
        inventory(&s, &a, "Jane Doe", "person");
        assert_eq!(build(&s, "r").unwrap(), 1);
        s.atlas_set_category("r", "a", "unk").unwrap();
        assert_eq!(s.related_articles("r", "a", 0, 20).unwrap().total, 0);
        inventory(&s, &a, "Jane Doe", "person");
        assert_eq!(build(&s, "r").unwrap(), 1);
        s.atlas_delete_article("r", "b").unwrap();
        assert_eq!(s.related_articles("r", "a", 0, 20).unwrap().total, 0);
    }
    #[test]
    fn textual_fallback_does_not_shorten_named_people_or_corporate_suffixes() {
        let s = store();
        inventory(
            &s,
            &article("a", "Acme reports", "economic"),
            "Acme",
            "organization",
        );
        inventory(
            &s,
            &article("b", "Acme Corp reports", "unk"),
            "Acme Corp",
            "organization",
        );
        inventory(
            &s,
            &article("c", "Jane reports", "geopolitical"),
            "Jane",
            "person",
        );
        inventory(
            &s,
            &article("d", "Jane Doe reports", "unk"),
            "Jane Doe",
            "person",
        );
        assert_eq!(build(&s, "r").unwrap(), 0);
    }
    #[test]
    fn unknown_candidate_cannot_seed_a_known_article_that_missed_it() {
        let s = store();
        inventory(
            &s,
            &article("a", "Acme speaks", "geopolitical"),
            "",
            "person",
        );
        inventory(&s, &article("b", "Acme responds", "unk"), "Acme", "person");
        assert_eq!(build(&s, "r").unwrap(), 0);
    }
    #[test]
    fn v2_lineage_unions_exact_owner_and_per_source_explicit_url_fallback() {
        let s = store();
        inventory(
            &s,
            &article("a", "Jane Doe speaks", "geopolitical"),
            "Jane Doe",
            "person",
        );
        inventory(
            &s,
            &article("b", "Jane Doe responds", "unk"),
            "Jane Doe",
            "person",
        );
        inventory(
            &s,
            &article("c", "Jane Doe answers", "unk"),
            "Jane Doe",
            "person",
        );
        build(&s, "r").unwrap();
        s.conn.execute_batch("INSERT INTO memories(id,text,category,pinned,created_at,source_json) VALUES('m','Claim','fact',0,'now','{}'); INSERT INTO insight_snapshots(id,owner_kind,owner_id,generation,input_key,inputs_json,entities_json,coverage_json,readiness_json,contract_version,policy_version,created_at) VALUES('s','article','a',1,'key','{}','[]','{}','{}','investigation-v2','v2','now'); INSERT INTO insight_derivations(id,snapshot_id,memory_id,insight_kind,fingerprint,item_json,assessment_json,status,revision) VALUES('d','s','m','claim','f','{}','{}','active','rev'); INSERT INTO insight_derivation_evidence(derivation_id,source_id,source_revision,evidence_json) VALUES('d','missing-acquisition-id','rev','{\"url\":\"https://example.com/b\"}');").unwrap();
        assert_eq!(s.article_origins_for_memory("m").unwrap().len(), 2);
        let page = s.related_articles_for_memory("m", 0, 20).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.rows[0].article.id, "c");
    }
    #[test]
    fn exact_lineage_keeps_all_cycle_memberships() {
        let s = store();
        s.atlas_insert_run("older", "{}", "{}").unwrap();
        let a = article("a", "Jane Doe speaks", "geopolitical");
        s.atlas_upsert_article(&a).unwrap();
        let mut older = a.clone();
        older.run_id = "older".into();
        s.atlas_upsert_article(&older).unwrap();
        s.conn.execute_batch("INSERT INTO memories(id,text,category,pinned,created_at,source_json) VALUES('m','Claim','fact',0,'now','{}'); INSERT INTO insight_claims(fingerprint,memory_id,entity_id,predicate,object_value,topic,classification,confidence,created_at,updated_at) VALUES('fp','m','e','p','o','t','geopolitical',0.5,'now','now'); INSERT INTO insight_sources(fingerprint,run_id,answer_id,call_id) VALUES('fp','older','answer','a');").unwrap();
        assert_eq!(
            s.article_origins_for_memory("m").unwrap(),
            vec![ArticleOrigin {
                run_id: "older".into(),
                article_id: "a".into()
            }]
        );
    }
    #[test]
    fn bounded_pages_have_true_total_and_named_matches_rank_first() {
        let s = store();
        inventory(
            &s,
            &article("a", "Jane Doe visits Geneva", "geopolitical"),
            "Jane Doe",
            "person",
        );
        let mut m = s.atlas_metadata("r", "a").unwrap().unwrap();
        m.mentions
            .push(find_exact("Jane Doe visits Geneva", "geneva", "place", "title").unwrap());
        s.atlas_save_metadata(&m).unwrap();
        inventory(&s, &article("b", "Geneva news", "unk"), "Geneva", "place");
        inventory(
            &s,
            &article("c", "Jane Doe responds", "unk"),
            "Jane Doe",
            "person",
        );
        assert_eq!(build(&s, "r").unwrap(), 2);
        let p = s.related_articles("r", "a", 0, 1).unwrap();
        assert_eq!(p.total, 2);
        assert!(p.has_more);
        assert_eq!(p.rows[0].article.id, "c");
        assert!(!p.rows[0].place_only);
        let p = s.related_articles("r", "a", 1, 1).unwrap();
        assert!(!p.has_more);
        assert!(p.rows[0].place_only);
    }
    #[test]
    fn legacy_multi_article_lineage_uses_exact_ids_and_deduplicates() {
        let s = store();
        let a = article("a", "Jane Doe speaks", "geopolitical");
        inventory(&s, &a, "Jane Doe", "person");
        inventory(
            &s,
            &article("b", "Jane Doe responds", "unk"),
            "Jane Doe",
            "person",
        );
        inventory(
            &s,
            &article("c", "Jane Doe answers", "unk"),
            "Jane Doe",
            "person",
        );
        build(&s, "r").unwrap();
        s.conn.execute_batch("INSERT INTO memories(id,text,category,pinned,created_at,source_json) VALUES('m','Claim','fact',0,'now','{}'); INSERT INTO insight_claims(fingerprint,memory_id,entity_id,predicate,object_value,topic,classification,confidence,created_at,updated_at) VALUES('fp','m','e','p','o','t','geopolitical',0.5,'now','now'); INSERT INTO insight_sources(fingerprint,run_id,answer_id,call_id) VALUES('fp','r','answer','a'),('fp','r','answer','b');").unwrap();
        assert_eq!(s.article_origins_for_memory("m").unwrap().len(), 2);
        let page = s.related_articles_for_memory("m", 0, 20).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.rows[0].article.id, "c");
        assert_eq!(page.rows[0].originating_run_ids, vec!["r"]);
    }
}
