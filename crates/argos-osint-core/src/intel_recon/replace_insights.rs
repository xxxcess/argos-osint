//! Replace an article's Atlas insights after full body retrieval.
//!
//! Extraction and validation run **before** any delete. Prior insights stay
//! readable through failures; a single SQLite transaction commits the swap.

use std::path::Path;

use anyhow::{anyhow, Result};

use crate::secrets::ProviderSecret;
#[cfg(test)]
use crate::store::AtlasInsightClaim;
use crate::store::{AtlasArticleRow, Store};

/// Outcome of a staged insight replacement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplaceOutcome {
    pub claims: usize,
    /// True when extraction succeeded with zero supported claims (distinct from failure).
    pub zero_claim_success: bool,
    pub preserved_user_edits: usize,
}

/// Extract and validate candidates from the body, then atomically replace this
/// article's insight sources. Prior insights remain until commit succeeds.
pub async fn replace_article_insights_from_body(
    db_path: &Path,
    article: &AtlasArticleRow,
    body_markdown: &str,
    synthesis: &ProviderSecret,
    classifier: Option<&ProviderSecret>,
) -> Result<ReplaceOutcome> {
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    replace_article_insights_from_body_with_cancel(
        db_path,
        article,
        body_markdown,
        synthesis,
        classifier,
        &cancel,
    )
    .await
}

pub(crate) async fn replace_article_insights_from_body_with_cancel(
    db_path: &Path,
    article: &AtlasArticleRow,
    body_markdown: &str,
    synthesis: &ProviderSecret,
    classifier: Option<&ProviderSecret>,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<ReplaceOutcome> {
    replace_article_insights_with_runtime(
        db_path,
        article,
        body_markdown,
        synthesis,
        classifier,
        cancel,
        None,
    )
    .await
}

pub(crate) async fn replace_article_insights_with_runtime(
    db_path: &Path,
    article: &AtlasArticleRow,
    body_markdown: &str,
    synthesis: &ProviderSecret,
    classifier: Option<&ProviderSecret>,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    runtime: Option<&super::worker::JobRuntime>,
) -> Result<ReplaceOutcome> {
    anyhow::ensure!(
        !cancel.load(std::sync::atomic::Ordering::Acquire),
        "cancelled"
    );
    if body_markdown.trim().is_empty() {
        return Err(anyhow!("empty article body"));
    }

    let store = Store::open(db_path)?;
    let settings = runtime
        .map(|r| r.settings.clone())
        .unwrap_or_else(|| crate::provider::SettingsFile::load().unwrap_or_default());
    let mut auth = runtime
        .map(|r| r.auth.clone())
        .unwrap_or_else(|| crate::secrets::AuthFile::load().unwrap_or_default());
    auth.set_account(synthesis.clone());
    let _ = classifier;
    let Some(task) = crate::investigation::insight_persist::schedule_article_refresh(
        &store, article, &settings, false,
    )?
    else {
        return Err(anyhow!(
            "Waiting for complete retained article details or Focus Brief intent"
        ));
    };
    let protected = 0;
    drop(store);
    let output = crate::investigation::insight_persist::run_refresh_slice(
        db_path, &task, &auth, &settings, cancel,
    )
    .await?
    .ok_or_else(|| anyhow!("insight refresh is already owned by another worker"))?;
    Ok(ReplaceOutcome {
        claims: output
            .claims
            .iter()
            .filter(|c| c.assessment.faithful())
            .count()
            + output.inferences.len(),
        zero_claim_success: !output.claims.iter().any(|c| c.assessment.faithful())
            && output.inferences.is_empty(),
        preserved_user_edits: protected,
    })
}

#[cfg(test)]
fn validate_staged_claims(claims: &[AtlasInsightClaim], article_id: &str) -> Result<()> {
    for claim in claims {
        anyhow::ensure!(
            !claim.entity.trim().is_empty()
                && !claim.predicate.trim().is_empty()
                && !claim.object.trim().is_empty()
                && !claim.claim.trim().is_empty(),
            "staged claim missing required fields; prior insights retained"
        );
        anyhow::ensure!(
            claim.article_id == article_id,
            "staged claim article_id mismatch; prior insights retained"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::AtlasInsightClaim;
    use std::sync::{Arc, Mutex};

    fn article(run: &str, id: &str) -> AtlasArticleRow {
        AtlasArticleRow {
            run_id: run.into(),
            id: id.into(),
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
        }
    }

    fn claim(article_id: &str, entity: &str, predicate: &str, object: &str) -> AtlasInsightClaim {
        AtlasInsightClaim {
            fingerprint: String::new(),
            entity: entity.into(),
            namespace: "place".into(),
            predicate: predicate.into(),
            object: object.into(),
            topic: "geopolitical".into(),
            claim: format!("{entity} {predicate} {object}."),
            classification: "fact".into(),
            confidence: 0.5,
            article_id: article_id.into(),
            source_url: "https://example.com/a".into(),
            published_at: "2026-10-04T12:00:00+00:00".into(),
            reliability: "B".into(),
            info_credibility: 2,
            admiralty: "B2".into(),
            rsp_status: "".into(),
        }
    }
}
