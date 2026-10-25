//! Related articles across every retained cycle (`RelatedScope::AllRetained`,
//! the default) or one cycle (`ThisCycle`). One row per article URL carries
//! every originating occurrence, the cycles it appears in, each basis, and
//! whether the link is tentative (shared mentions / unconfirmed names) or
//! established by a shared canonical Brain actor.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;

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

/// Host of a canonical URL, lowercased. Empty when `key` is not a URL, so
/// the caller keeps the unbounded scan for that key.
fn url_host(key: &str) -> Option<String> {
    reqwest::Url::parse(key)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
        .filter(|host| !host.is_empty())
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
    for (run, id, url) in
        crate::storage::store_bridge::article_urls_matching(url_host(key).as_deref())?
    {
        if canonical_url(&url) == key {
            if let Some(a) = store.atlas_article(&run, &id)? {
                out.push(a);
            }
        }
    }
    Ok(out)
}

fn actor_name(_store: &Store, actor_id: &str) -> String {
    crate::storage::store_bridge::actor_display_name(actor_id)
        .ok()
        .flatten()
        .unwrap_or_else(|| actor_id.into())
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
        let host = url_host(&key).unwrap_or_default();
        let mut r = db
            .query(
                "SELECT VALUE [run_id, article_id, url] FROM atlas_article \
                 WHERE url != '' AND ($host = '' OR string::lowercase(url) CONTAINS $host)",
            )
            .bind(("host", host))
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
