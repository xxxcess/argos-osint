//! Related articles across every retained cycle (`RelatedScope::AllRetained`,
//! the default) or one cycle (`ThisCycle`). One row per article URL carries
//! every originating occurrence, the cycles it appears in, each basis, and
//! whether the link is tentative (shared mentions / unconfirmed names) or
//! established by a shared canonical Brain actor.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use rusqlite::params;

use crate::brain_actors as actors;
use crate::related_articles::{canonical_url, ArticleOrigin};
use crate::store::{AtlasArticleRow, Store};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RelatedScope {
    #[default]
    AllRetained,
    ThisCycle,
}

impl RelatedScope {
    pub fn label(self) -> &'static str {
        match self {
            RelatedScope::AllRetained => "All retained cycles",
            RelatedScope::ThisCycle => "This cycle",
        }
    }
    pub fn toggle(self) -> Self {
        match self {
            RelatedScope::AllRetained => RelatedScope::ThisCycle,
            RelatedScope::ThisCycle => RelatedScope::AllRetained,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GlobalRelated {
    /// Newest occurrence (display row).
    pub article: AtlasArticleRow,
    pub url_key: String,
    pub origins: Vec<ArticleOrigin>,
    pub cycles: usize,
    pub basis: Vec<String>,
    /// False only when a shared canonical actor established the link.
    pub tentative: bool,
}

impl GlobalRelated {
    pub fn summary(&self) -> String {
        let date = self.article.published_at.get(..10).unwrap_or("");
        format!(
            "{date} · {} cycle{} · {}{}",
            self.cycles,
            if self.cycles == 1 { "" } else { "s" },
            self.basis.join("; "),
            if self.tentative { " · tentative" } else { "" }
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlobalRelatedPage {
    pub rows: Vec<GlobalRelated>,
    pub total: usize,
    /// Opaque continuation; `None` at the end.
    pub next_cursor: Option<String>,
    pub scope: RelatedScope,
}

impl GlobalRelatedPage {
    /// Presentation page shared by Intel and Brain (offset = cursor).
    pub fn into_page(self, offset: usize) -> crate::related_articles::RelatedArticlePage {
        let has_more = self.next_cursor.is_some();
        crate::related_articles::RelatedArticlePage {
            total: self.total,
            offset,
            has_more,
            coverage: match self.scope {
                RelatedScope::AllRetained => "all retained cycles",
                RelatedScope::ThisCycle => "this cycle",
            }
            .into(),
            rows: self
                .rows
                .into_iter()
                .map(|r| {
                    let mut runs: Vec<String> =
                        r.origins.iter().map(|o| o.run_id.clone()).collect();
                    runs.dedup();
                    crate::related_articles::RelatedArticle {
                        article: r.article,
                        shared_mentions: r.basis,
                        originating_run_ids: runs,
                        place_only: false,
                        origins: r.origins,
                        established: !r.tentative,
                    }
                })
                .collect(),
        }
    }
}

fn url_key(a: &AtlasArticleRow) -> String {
    if a.url.trim().is_empty() {
        format!("occurrence:{}/{}", a.run_id, a.id)
    } else {
        canonical_url(&a.url)
    }
}

#[derive(Default)]
struct Acc {
    occurrences: BTreeMap<ArticleOrigin, AtlasArticleRow>,
    basis: BTreeSet<String>,
    established: bool,
}

struct Collector<'a> {
    store: &'a Store,
    exclude: BTreeSet<String>,
    rows: BTreeMap<String, Acc>,
}

impl Collector<'_> {
    fn add_related(
        &mut self,
        source: &AtlasArticleRow,
        run: &str,
        id: &str,
        basis: String,
        established: bool,
    ) -> Result<()> {
        let Some(candidate) = self.store.atlas_article(run, id)? else {
            return Ok(());
        };
        if crate::related_articles::shared_entity_mentions(self.store, source, &candidate)?.len()
            < crate::related_articles::MIN_SHARED_ENTITIES
        {
            return Ok(());
        }
        self.add(run, id, basis, established)
    }
    fn add(&mut self, run: &str, id: &str, basis: String, established: bool) -> Result<()> {
        let Some(article) = self.store.atlas_article(run, id)? else {
            return Ok(());
        };
        insert(&self.exclude, &mut self.rows, article, basis, established);
        Ok(())
    }

    fn finish(
        self,
        cursor: Option<&str>,
        limit: usize,
        scope: RelatedScope,
    ) -> Result<GlobalRelatedPage> {
        Ok(finish(self.rows, cursor, limit, scope))
    }
}

/// Group one occurrence under its URL key unless the key is excluded.
fn insert(
    exclude: &BTreeSet<String>,
    rows: &mut BTreeMap<String, Acc>,
    article: AtlasArticleRow,
    basis: String,
    established: bool,
) {
    let key = url_key(&article);
    if exclude.contains(&key) {
        return;
    }
    let acc = rows.entry(key).or_default();
    acc.basis.insert(basis);
    acc.established |= established;
    acc.occurrences.insert(
        ArticleOrigin {
            run_id: article.run_id.clone(),
            article_id: article.id.clone(),
        },
        article,
    );
}

/// Rank grouped rows (established first, then basis count, recency, URL) and
/// cut one page at the numeric `cursor`.
fn finish(
    grouped: BTreeMap<String, Acc>,
    cursor: Option<&str>,
    limit: usize,
    scope: RelatedScope,
) -> GlobalRelatedPage {
    {
        let mut rows = Vec::new();
        for (key, acc) in grouped {
            let newest = acc
                .occurrences
                .values()
                .max_by(|a, b| {
                    a.published_at
                        .cmp(&b.published_at)
                        .then(a.run_id.cmp(&b.run_id))
                })
                .cloned()
                .unwrap_or_default();
            let cycles: BTreeSet<_> = acc.occurrences.keys().map(|o| o.run_id.clone()).collect();
            rows.push(GlobalRelated {
                article: newest,
                url_key: key,
                origins: acc.occurrences.into_keys().collect(),
                cycles: cycles.len(),
                basis: acc.basis.into_iter().collect(),
                tentative: !acc.established,
            });
        }
        rows.sort_by(|a, b| {
            a.tentative
                .cmp(&b.tentative)
                .then_with(|| b.basis.len().cmp(&a.basis.len()))
                .then_with(|| b.article.published_at.cmp(&a.article.published_at))
                .then_with(|| a.url_key.cmp(&b.url_key))
        });
        let total = rows.len();
        let offset: usize = cursor.and_then(|c| c.parse().ok()).unwrap_or(0);
        let limit = limit.clamp(1, 100);
        let end = offset.saturating_add(limit);
        GlobalRelatedPage {
            rows: rows.into_iter().skip(offset).take(limit).collect(),
            total,
            next_cursor: (end < total).then(|| end.to_string()),
            scope,
        }
    }
}

fn occurrences_of_url(store: &Store, key: &str) -> Result<Vec<AtlasArticleRow>> {
    let mut out = Vec::new();
    for (run, id, url) in crate::storage::store_bridge::article_urls()? {
        if canonical_url(&url) == key {
            if let Some(a) = store.atlas_article(&run, &id)? {
                out.push(a);
            }
        }
    }
    Ok(out)
}

fn actor_name(store: &Store, actor_id: &str) -> String {
    crate::storage::store_bridge::actor_display_name(actor_id)
        .ok()
        .flatten()
        .unwrap_or_else(|| actor_id.into())
}

impl Store {
    /// Related articles for one occurrence, grouped by URL. `AllRetained`
    /// starts from every retained occurrence of the source URL and adds Brain
    /// Related records and articles sharing a canonical actor; `ThisCycle`
    /// keeps only the source cycle's shared-mention leads.
    pub fn related_articles_global(
        &self,
        source_run: &str,
        source_article: &str,
        scope: RelatedScope,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<GlobalRelatedPage> {
        let Some(source) = self.atlas_article(source_run, source_article)? else {
            return Ok(GlobalRelatedPage {
                scope,
                ..GlobalRelatedPage::default()
            });
        };
        let exclude = url_key(&source);
        let sources = self.occurrences_for(&source, scope)?;
        let mut c = Collector {
            store: self,
            exclude: BTreeSet::from([exclude]),
            rows: BTreeMap::new(),
        };
        for s in &sources {
            self.gather(&mut c, s, scope)?;
        }
        c.finish(cursor, limit, scope)
    }

    fn occurrences_for(
        &self,
        source: &AtlasArticleRow,
        scope: RelatedScope,
    ) -> Result<Vec<AtlasArticleRow>> {
        let key = url_key(source);
        Ok(match scope {
            RelatedScope::AllRetained if !key.starts_with("occurrence:") => {
                occurrences_of_url(self, &key)?
            }
            _ => vec![source.clone()],
        })
    }

    fn gather(
        &self,
        c: &mut Collector<'_>,
        s: &AtlasArticleRow,
        scope: RelatedScope,
    ) -> Result<()> {
        let mut start = 0;
        loop {
            let page = self.related_articles(&s.run_id, &s.id, start, 100)?;
            for row in &page.rows {
                c.add_related(s, &row.article.run_id, &row.article.id, row.reason(), false)?;
            }
            if !page.has_more {
                break;
            }
            start += 100;
        }
        if scope == RelatedScope::ThisCycle {
            return Ok(());
        }
        // Brain Related records touching this occurrence (active only).
        let surreal_hits = {
            Some(crate::storage::store_bridge::related_endpoint_candidates(
                &s.run_id, &s.id, false,
            )?)
        };
        #[cfg(not(feature = "surreal"))]
        let surreal_hits: Option<Vec<(String, String, String)>> = None;
        let hits = match surreal_hits {
            Some(h) => h,
            None => self.related_hits_sqlite(s)?,
        };
        for (run, id, memory) in hits {
            let basis = match self.memory_payload(&memory)? {
                Some(crate::memory_metadata::MemoryPayload::Related(p))
                    if crate::related_articles::current_ingest_related(self, &p)? =>
                {
                    p.basis
                }
                _ => continue,
            };
            c.add_related(s, &run, &id, basis, false)?;
        }
        Ok(())
    }

    fn related_hits_sqlite(&self, s: &AtlasArticleRow) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
                "SELECT o.origin,o.ref_id,mm.memory_id FROM brain_memory_endpoints e
                 JOIN brain_memory_endpoints o ON o.memory_id=e.memory_id AND o.side<>e.side AND o.endpoint_kind='article'
                 JOIN memory_metadata mm ON mm.memory_id=e.memory_id AND mm.memory_kind='related' AND mm.lifecycle<>'retired'
                 WHERE e.endpoint_kind='article' AND e.origin=?1 AND e.ref_id=?2",
            )?;
        let hits = stmt
            .query_map(params![s.run_id, s.id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(hits)
    }

    /// Related articles for a memory: the union over every article origin of
    /// the memory, one row per URL, excluding the origins themselves.
    pub fn related_articles_global_for_memory(
        &self,
        memory_id: &str,
        scope: RelatedScope,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<GlobalRelatedPage> {
        // An Actor's Related Articles are the articles that mention it.
        if crate::brain_kinds::memory_kind(self, memory_id)?
            == crate::memory_metadata::MemoryKind::Actor
        {
            return self.actor_articles(memory_id, cursor, limit);
        }
        let mut sources = Vec::new();
        let mut exclude = BTreeSet::new();
        let mut origins = self.article_origins_for_memory(memory_id)?;
        // Link/Related records start from their article endpoints.
        {
            origins.extend(crate::storage::store_bridge::memory_article_endpoints(
                memory_id,
            )?);
        }
        #[cfg(not(feature = "surreal"))]
        origins.extend(self.memory_article_endpoints_sqlite(memory_id)?);
        for origin in origins {
            if let Some(a) = self.atlas_article(&origin.run_id, &origin.article_id)? {
                exclude.insert(url_key(&a));
                sources.extend(self.occurrences_for(&a, scope)?);
            }
        }
        let mut c = Collector {
            store: self,
            exclude,
            rows: BTreeMap::new(),
        };
        for s in &sources {
            self.gather(&mut c, s, scope)?;
        }
        c.finish(cursor, limit, scope)
    }

    fn memory_article_endpoints_sqlite(&self, memory_id: &str) -> Result<Vec<ArticleOrigin>> {
        let mut stmt = self.conn.prepare(
            "SELECT origin,ref_id FROM brain_memory_endpoints WHERE memory_id=?1 AND endpoint_kind='article'",
        )?;
        let rows = stmt
            .query_map([memory_id], |r| {
                Ok(ArticleOrigin {
                    run_id: r.get(0)?,
                    article_id: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Articles mentioning a canonical actor (and everything merged into it),
    /// one row per URL across all retained cycles.
    pub fn actor_articles(
        &self,
        actor_id: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<GlobalRelatedPage> {
        let canonical = crate::brain_kinds::actor_resolve(self, actor_id)?;
        let name = actor_name(self, &canonical);
        let mut c = Collector {
            store: self,
            exclude: BTreeSet::new(),
            rows: BTreeMap::new(),
        };
        for (kind, run, id, revision) in crate::brain_kinds::actor_mentions(self, &canonical)? {
            if kind == "article" {
                let Some(article) = self.atlas_article(&run, &id)? else {
                    continue;
                };
                if !revision.is_empty() && revision != crate::atlas_metadata::input_hash(&article) {
                    continue;
                }
                c.add(&run, &id, format!("mentions {name}"), true)?;
            }
        }
        c.finish(cursor, limit, RelatedScope::AllRetained)
    }
}

/// `Store::related_articles_global` on SurrealDB: URL occurrences across
/// retained cycles plus each occurrence's current shared-mention leads.
/// Brain Related records and canonical-actor leads are not stored on
/// SurrealDB yet, so every row stays tentative.
#[cfg(feature = "surreal")]
pub async fn related_articles_global_on(
    repo: &crate::storage::repo::Repository,
    source_run: &str,
    source_article: &str,
    scope: RelatedScope,
    cursor: Option<&str>,
    limit: usize,
) -> Result<GlobalRelatedPage> {
    use crate::storage::{atlas, atlas_metadata};
    use surrealdb::types::SurrealValue;
    #[derive(SurrealValue)]
    struct Pair {
        left_id: String,
        right_id: String,
        left_revision: String,
        right_revision: String,
        shared_json: String,
    }
    let db = repo.db();
    let Some(source) = atlas::article(db, source_run, source_article).await? else {
        return Ok(GlobalRelatedPage {
            scope,
            ..GlobalRelatedPage::default()
        });
    };
    let key = url_key(&source);
    let sources = if scope == RelatedScope::AllRetained && !key.starts_with("occurrence:") {
        let mut r = db
            .query("SELECT VALUE [run_id, article_id, url] FROM atlas_article WHERE url != ''")
            .await?
            .check()?;
        let hits: Vec<Vec<String>> = r.take(0usize)?;
        let mut out = Vec::new();
        for hit in hits
            .iter()
            .filter(|h| h.len() == 3 && canonical_url(&h[2]) == key)
        {
            if let Some(a) = atlas::article(db, &hit[0], &hit[1]).await? {
                out.push(a);
            }
        }
        out
    } else {
        vec![source.clone()]
    };
    let exclude = BTreeSet::from([key]);
    let mut grouped = BTreeMap::new();
    for s in &sources {
        let mut r = db
            .query(
                "SELECT left_id, right_id, left_revision, right_revision, shared_json
                   FROM atlas_related_article WHERE run_id = $r AND left_id = $a;
                 SELECT left_id, right_id, left_revision, right_revision, shared_json
                   FROM atlas_related_article WHERE run_id = $r AND right_id = $a;",
            )
            .bind(("r", s.run_id.clone()))
            .bind(("a", s.id.clone()))
            .await?
            .check()?;
        let mut pairs: Vec<Pair> = r.take(0usize)?;
        pairs.extend(r.take::<Vec<Pair>>(1usize)?);
        for p in pairs {
            let (Some(a), Some(b)) = (
                atlas::article(db, &s.run_id, &p.left_id).await?,
                atlas::article(db, &s.run_id, &p.right_id).await?,
            ) else {
                continue;
            };
            if crate::related_articles::revision(&a) != p.left_revision
                || crate::related_articles::revision(&b) != p.right_revision
            {
                continue;
            }
            let am = atlas_metadata::metadata(db, &a.run_id, &a.id).await?;
            let bm = atlas_metadata::metadata(db, &b.run_id, &b.id).await?;
            if crate::related_articles::shared_entity_mentions_with(
                &a,
                am.as_ref(),
                &b,
                bm.as_ref(),
            )
            .len()
                < crate::related_articles::MIN_SHARED_ENTITIES
            {
                continue;
            }
            let shared: Vec<crate::related_articles::SharedMention> =
                serde_json::from_str(&p.shared_json)?;
            let basis = format!(
                "Shared mentions: {}",
                shared
                    .into_iter()
                    .map(|m| m.original_text)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let target = if p.left_id == s.id { b } else { a };
            insert(&exclude, &mut grouped, target, basis, false);
        }
    }
    Ok(finish(grouped, cursor, limit, scope))
}

#[cfg(test)]
#[path = "related_global_tests.rs"]
mod tests;
