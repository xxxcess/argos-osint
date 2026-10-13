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
    fn add(&mut self, run: &str, id: &str, basis: String, established: bool) -> Result<()> {
        let Some(article) = self.store.atlas_article(run, id)? else {
            return Ok(());
        };
        let key = url_key(&article);
        if self.exclude.contains(&key) {
            return Ok(());
        }
        let acc = self.rows.entry(key).or_default();
        acc.basis.insert(basis);
        acc.established |= established;
        acc.occurrences.insert(
            ArticleOrigin {
                run_id: run.into(),
                article_id: id.into(),
            },
            article,
        );
        Ok(())
    }

    fn finish(
        self,
        cursor: Option<&str>,
        limit: usize,
        scope: RelatedScope,
    ) -> Result<GlobalRelatedPage> {
        let store = self.store;
        let mut rows = Vec::new();
        for (key, mut acc) in self.rows {
            // Every retained occurrence of the same URL is an origin of the row.
            if !key.starts_with("occurrence:") && scope == RelatedScope::AllRetained {
                for a in occurrences_of_url(store, &key)? {
                    acc.occurrences.insert(
                        ArticleOrigin {
                            run_id: a.run_id.clone(),
                            article_id: a.id.clone(),
                        },
                        a,
                    );
                }
            }
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
        Ok(GlobalRelatedPage {
            rows: rows.into_iter().skip(offset).take(limit).collect(),
            total,
            next_cursor: (end < total).then(|| end.to_string()),
            scope,
        })
    }
}

fn occurrences_of_url(store: &Store, key: &str) -> Result<Vec<AtlasArticleRow>> {
    let mut stmt = store
        .conn
        .prepare("SELECT run_id,article_id,url FROM atlas_articles WHERE url<>''")?;
    let hits: Vec<(String, String)> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .filter(|(_, _, url)| canonical_url(url) == key)
        .map(|(run, id, _)| (run, id))
        .collect();
    let mut out = Vec::new();
    for (run, id) in hits {
        if let Some(a) = store.atlas_article(&run, &id)? {
            out.push(a);
        }
    }
    Ok(out)
}

fn actor_name(store: &Store, actor_id: &str) -> String {
    actors::actor(&store.conn, actor_id)
        .ok()
        .flatten()
        .map(|a| a.display_name)
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
                c.add(&row.article.run_id, &row.article.id, row.reason(), false)?;
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
        for (run, id, memory) in hits {
            let basis = match self.memory_payload(&memory)? {
                Some(crate::memory_metadata::MemoryPayload::Related(p)) => p.basis,
                _ => "Brain Related".into(),
            };
            c.add(&run, &id, basis, false)?;
        }
        // Same canonical actor (explicit identity, not a name match).
        let mut stmt = self.conn.prepare(
                "SELECT DISTINCT actor_id FROM brain_actor_mentions WHERE origin_kind='article' AND origin_scope=?1 AND origin_id=?2",
            )?;
        let ids = stmt
            .query_map(params![s.run_id, s.id], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let canon: BTreeSet<String> = ids
            .iter()
            .map(|id| actors::resolve(&self.conn, id))
            .collect::<Result<_>>()?;
        for actor in canon {
            let name = actor_name(self, &actor);
            for (kind, run, id, _) in actors::mentions(&self.conn, &actor)? {
                if kind == "article" {
                    c.add(&run, &id, format!("same actor: {name}"), true)?;
                }
            }
        }
        Ok(())
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
        for origin in self.article_origins_for_memory(memory_id)? {
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

    /// Articles mentioning a canonical actor (and everything merged into it),
    /// one row per URL across all retained cycles.
    pub fn actor_articles(
        &self,
        actor_id: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<GlobalRelatedPage> {
        let canonical = actors::resolve(&self.conn, actor_id)?;
        let name = actor_name(self, &canonical);
        let mut c = Collector {
            store: self,
            exclude: BTreeSet::new(),
            rows: BTreeMap::new(),
        };
        for (kind, run, id, _) in actors::mentions(&self.conn, &canonical)? {
            if kind == "article" {
                c.add(&run, &id, format!("mentions {name}"), true)?;
            }
        }
        c.finish(cursor, limit, RelatedScope::AllRetained)
    }
}

#[cfg(test)]
#[path = "related_global_tests.rs"]
mod tests;
