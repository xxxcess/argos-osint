//! Where article body acquisition keeps its state: the legacy SQLite file
//! or the SurrealDB repository. `body` drives one fallback chain against
//! either; each call opens (SQLite) or borrows (SurrealDB) its handle so the
//! fetch future stays `Send`.

use std::path::Path;

use anyhow::Result;

use crate::nlp::{Annotation, ParseContext, Source};
use crate::storage::repo::Repository;
use crate::store::{AtlasArticleRow, Store};

use super::ArticleBodyRow;

#[derive(Clone, Copy)]
pub enum BodyDb<'a> {
    Sqlite(&'a Path),
    Surreal(&'a Repository),
}

use crate::storage::article_body as sb;

impl BodyDb<'_> {
    pub async fn body(&self, id: &str) -> Result<Option<ArticleBodyRow>> {
        match self {
            Self::Sqlite(p) => Store::open(p)?.article_body_by_id(id),
            Self::Surreal(r) => sb::by_id(r, id).await,
        }
    }

    pub async fn ensure(&self, a: &AtlasArticleRow) -> Result<ArticleBodyRow> {
        match self {
            Self::Sqlite(p) => Store::open(p)?.ensure_article_body(
                &a.id,
                &a.run_id,
                &a.url,
                &a.source_domain,
                &a.source_name,
            ),
            Self::Surreal(r) => {
                sb::ensure(
                    r,
                    &a.id,
                    &a.run_id,
                    &a.url,
                    &a.source_domain,
                    &a.source_name,
                )
                .await
            }
        }
    }

    pub async fn set_state(&self, id: &str, state: &str) -> Result<()> {
        match self {
            Self::Sqlite(p) => Store::open(p)?.update_article_body_state(id, state),
            Self::Surreal(r) => sb::set_state(r, id, state).await,
        }
    }

    pub async fn clear(&self, id: &str) -> Result<()> {
        match self {
            Self::Sqlite(p) => Store::open(p)?.clear_article_body_content(id),
            Self::Surreal(r) => sb::clear_content(r, id).await,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn commit(
        &self,
        id: &str,
        markdown: &str,
        content_hash: &str,
        quality: &str,
        rationale: &str,
        resolved_url: &str,
        tool: &str,
        bump: bool,
    ) -> Result<()> {
        match self {
            Self::Sqlite(p) => Store::open(p)?.commit_article_body(
                id,
                markdown,
                content_hash,
                quality,
                rationale,
                resolved_url,
                tool,
                bump,
            ),
            Self::Surreal(r) => {
                sb::commit(
                    r,
                    id,
                    markdown,
                    content_hash,
                    quality,
                    rationale,
                    resolved_url,
                    tool,
                    bump,
                )
                .await?;
                if let Some(body) = sb::by_id(r, id).await? {
                    let source = Source::derived(id, &body.body_version.to_string(), markdown);
                    self.register(&body.article_id, &source, None, "historical-body-v1")
                        .await?;
                }
                Ok(())
            }
        }
    }

    pub async fn mark_failed(
        &self,
        id: &str,
        rationale: &str,
        retry_after: &str,
        partial: &str,
        quality: &str,
    ) -> Result<()> {
        match self {
            Self::Sqlite(p) => Store::open(p)?.mark_article_body_failed(
                id,
                rationale,
                retry_after,
                partial,
                quality,
            ),
            Self::Surreal(r) => {
                sb::mark_failed(r, id, rationale, retry_after, partial, quality).await
            }
        }
    }

    pub async fn insert_attempt(
        &self,
        body_id: &str,
        tool: &str,
        input: &str,
        state: &str,
        reason: &str,
        version: i64,
    ) -> Result<String> {
        match self {
            Self::Sqlite(p) => Store::open(p)?
                .insert_retrieval_attempt(body_id, tool, input, state, reason, "", version),
            Self::Surreal(r) => {
                sb::insert_attempt(r, body_id, tool, input, state, reason, version).await
            }
        }
    }

    pub async fn finish_attempt(&self, id: &str, state: &str, reason: &str) -> Result<()> {
        match self {
            Self::Sqlite(p) => Store::open(p)?.finish_retrieval_attempt(id, state, reason, ""),
            Self::Surreal(r) => sb::finish_attempt(r, id, state, reason).await,
        }
    }

    /// `nlp::cache::register` for an article owner.
    pub async fn register(
        &self,
        article_id: &str,
        source: &Source,
        raw: Option<&str>,
        transform: &str,
    ) -> Result<()> {
        match self {
            Self::Sqlite(p) => crate::nlp::cache::register(
                &Store::open(p)?.conn,
                "article",
                article_id,
                source,
                raw,
                transform,
            ),
            Self::Surreal(r) => {
                crate::storage::pipeline_inputs::register(
                    r, "article", article_id, source, raw, transform,
                )
                .await
            }
        }
    }

    /// The cached NLP annotation. The SurrealDB repository keeps no
    /// annotation cache yet, so the caller re-annotates.
    pub async fn annotation(&self, source: &Source, ctx: &ParseContext) -> Option<Annotation> {
        match self {
            Self::Sqlite(p) => Store::open(p)
                .ok()
                .and_then(|s| crate::nlp::cache::get(&s.conn, source, ctx).ok().flatten()),
            Self::Surreal(_) => None,
        }
    }

    pub async fn put_annotation(
        &self,
        source: &Source,
        ctx: &ParseContext,
        a: &Annotation,
    ) -> Result<()> {
        match self {
            Self::Sqlite(p) => crate::nlp::cache::put(&Store::open(p)?.conn, source, ctx, a),
            Self::Surreal(_) => Ok(()),
        }
    }

    pub async fn app_state_get(&self, key: &str) -> Result<Option<String>> {
        match self {
            Self::Sqlite(p) => Store::open(p)?.app_state_get(key),
            Self::Surreal(r) => crate::storage::pipeline_inputs::app_state_get(r, key).await,
        }
    }

    pub async fn app_state_set(&self, key: &str, value: &str) -> Result<()> {
        match self {
            Self::Sqlite(p) => Store::open(p)?.app_state_set(key, value),
            Self::Surreal(r) => crate::storage::pipeline_inputs::app_state_set(r, key, value).await,
        }
    }

    pub async fn article(&self, run_id: &str, article_id: &str) -> Result<Option<AtlasArticleRow>> {
        match self {
            Self::Sqlite(p) => Store::open(p)?.atlas_article(run_id, article_id),
            Self::Surreal(r) => {
                Ok(crate::storage::atlas::article(r.db(), run_id, article_id).await?)
            }
        }
    }

    /// `insight_persist::schedule_article_refresh`; true when a refresh is
    /// queued or live.
    pub async fn schedule_refresh(
        &self,
        article: &AtlasArticleRow,
        settings: &crate::provider::SettingsFile,
    ) -> Result<bool> {
        match self {
            Self::Sqlite(p) => Ok(
                crate::investigation::insight_persist::schedule_article_refresh(
                    &Store::open(p)?,
                    article,
                    settings,
                    false,
                )?
                .is_some(),
            ),
            Self::Surreal(r) => {
                let input = crate::storage::pipeline_inputs::article_input(r, article, 0, "")
                    .await
                    .ok();
                Ok(crate::storage::refresh::schedule_article_refresh(
                    r.db(),
                    &article.id,
                    input.as_ref(),
                    settings,
                    false,
                )
                .await?
                .is_some())
            }
        }
    }
}
