//! Replace an article's Atlas insights after full body retrieval.

use std::path::Path;

use anyhow::{anyhow, Result};

use crate::atlas_insights;
use crate::secrets::ProviderSecret;
use crate::store::{AtlasArticleRow, Store};

/// Delete prior article-scoped insights (and orphan Brain memories), then extract
/// fresh claims from title + description + cleaned body and persist them.
///
/// Opens the store only around sync DB work so the async extract stays `Send`.
pub async fn replace_article_insights_from_body(
    db_path: &Path,
    article: &AtlasArticleRow,
    body_markdown: &str,
    synthesis: &ProviderSecret,
    classifier: Option<&ProviderSecret>,
) -> Result<usize> {
    if body_markdown.trim().is_empty() {
        return Err(anyhow!("empty article body"));
    }
    let peers = {
        let store = Store::open(db_path)?;
        let _orphaned = store.delete_article_insights(&article.run_id, &article.id)?;
        store
            .atlas_list_articles(&article.run_id)
            .unwrap_or_default()
            .into_iter()
            .filter(|peer| peer.id != article.id)
            .collect::<Vec<_>>()
    };
    let settled = atlas_insights::extract_for_article_body(
        synthesis,
        classifier,
        article,
        body_markdown,
        &peers,
    )
    .await?;
    let store = Store::open(db_path)?;
    store.persist_atlas_insights(
        &article.run_id,
        &settled.claims,
        &settled.relations,
        "",
        "",
    )?;
    Ok(settled.claims.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::AtlasInsightClaim;

    #[test]
    fn delete_then_persist_replaces_article_claims() {
        let store = Store::memory().unwrap();
        store.atlas_insert_run("run-1", "{}", "{}").unwrap();
        let article = AtlasArticleRow {
            run_id: "run-1".into(),
            id: "art-1".into(),
            title: "Geneva hosts talks".into(),
            description: "Leaders meet at the border.".into(),
            url: "https://example.com/a".into(),
            country: "CH".into(),
            source_name: "Ex".into(),
            source_domain: "example.com".into(),
            published_at: "2026-10-04T12:00:00+00:00".into(),
            provider: "news".into(),
            temperature: 0.4,
            category: "geopolitical".into(),
            seen_at: "".into(),
            author: "".into(),
            image_url: "".into(),
        };
        store.atlas_upsert_article(&article).unwrap();
        let old = AtlasInsightClaim {
            fingerprint: String::new(),
            entity: "geneva".into(),
            namespace: "place".into(),
            predicate: "hosts".into(),
            object: "talks".into(),
            topic: "geopolitical".into(),
            claim: "Geneva hosts talks.".into(),
            classification: "fact".into(),
            confidence: 0.5,
            article_id: "art-1".into(),
            source_url: article.url.clone(),
            published_at: article.published_at.clone(),
            reliability: "B".into(),
            info_credibility: 2,
            admiralty: "B2".into(),
            rsp_status: "".into(),
        };
        store
            .persist_atlas_insights("run-1", &[old], &[], "", "")
            .unwrap();
        assert_eq!(
            store.atlas_claims_for_article("run-1", "art-1").unwrap().len(),
            1
        );
        let orphaned = store.delete_article_insights("run-1", "art-1").unwrap();
        assert_eq!(orphaned, 1);
        let fresh = AtlasInsightClaim {
            fingerprint: String::new(),
            entity: "leaders".into(),
            namespace: "person".into(),
            predicate: "meet".into(),
            object: "border".into(),
            topic: "geopolitical".into(),
            claim: "Leaders meet at the border.".into(),
            classification: "inference".into(),
            confidence: 0.7,
            article_id: "art-1".into(),
            source_url: article.url.clone(),
            published_at: article.published_at.clone(),
            reliability: "B".into(),
            info_credibility: 3,
            admiralty: "B3".into(),
            rsp_status: "".into(),
        };
        store
            .persist_atlas_insights("run-1", &[fresh], &[], "", "")
            .unwrap();
        let rows = store.atlas_claims_for_article("run-1", "art-1").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].entity, "leaders");
    }
}
