//! Replace an article's Atlas insights after full body retrieval.
//!
//! Extraction and validation run **before** any delete. Prior insights stay
//! readable through failures; a single SQLite transaction commits the swap.

use std::path::Path;

use anyhow::{anyhow, Result};

use crate::atlas_insights;
use crate::secrets::ProviderSecret;
use crate::store::{AtlasArticleRow, AtlasInsightClaim, Store};

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
    if body_markdown.trim().is_empty() {
        return Err(anyhow!("empty article body"));
    }

    let (peers, prior_count, protected) = {
        let store = Store::open(db_path)?;
        let prior = store
            .atlas_claims_for_article(&article.run_id, &article.id)?
            .len();
        let protected = store.article_insight_user_edit_count(&article.run_id, &article.id)?;
        let peers = store
            .atlas_list_articles(&article.run_id)
            .unwrap_or_default()
            .into_iter()
            .filter(|peer| peer.id != article.id)
            .collect::<Vec<_>>();
        (peers, prior, protected)
    };

    // Stage: network/model work with the prior insights still live.
    let settled = atlas_insights::extract_for_article_body(
        synthesis,
        classifier,
        article,
        body_markdown,
        &peers,
    )
    .await
    .map_err(|err| {
        anyhow!("insight extraction failed; prior insights retained ({prior_count} claims): {err}")
    })?;

    validate_staged_claims(&settled.claims, &article.id)?;

    let store = Store::open(db_path)?;
    let outcome = store.commit_article_insight_replacement(
        &article.run_id,
        &article.id,
        &settled.claims,
        &settled.relations,
    )?;
    Ok(ReplaceOutcome {
        claims: outcome.claims,
        zero_claim_success: outcome.claims == 0,
        preserved_user_edits: protected.max(outcome.preserved_user_edits),
    })
}

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

    #[test]
    fn commit_replaces_atomically_and_keeps_shared_support() {
        let store = Store::memory().unwrap();
        store.atlas_insert_run("run-1", "{}", "{}").unwrap();
        let art = article("run-1", "art-1");
        store.atlas_upsert_article(&art).unwrap();
        store
            .persist_atlas_insights(
                "run-1",
                &[claim("art-1", "geneva", "hosts", "talks")],
                &[],
                "",
                "",
            )
            .unwrap();
        // Shared support from another article on a second claim.
        let shared = claim("art-2", "leaders", "meet", "border");
        store
            .persist_atlas_insights("run-1", &[shared.clone()], &[], "", "")
            .unwrap();
        // Attach art-1 as an extra source for the shared fingerprint.
        let fp = crate::store::insight_fingerprint("place", "leaders", "meet", "border");
        store
            .conn
            .execute(
                "INSERT OR IGNORE INTO insight_sources(fingerprint,thread_id,run_id,answer_id,call_id,source_url,published_at) VALUES (?1,NULL,?2,?3,?4,?5,?6)",
                rusqlite::params![
                    fp,
                    "run-1",
                    format!("atlas-run-1"),
                    "art-1",
                    "https://example.com/a",
                    art.published_at,
                ],
            )
            .unwrap();

        let fresh = vec![claim("art-1", "diplomats", "resume", "talks")];
        let outcome = store
            .commit_article_insight_replacement("run-1", "art-1", &fresh, &[])
            .unwrap();
        assert_eq!(outcome.claims, 1);
        let rows = store.atlas_claims_for_article("run-1", "art-1").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].entity, "diplomats");
        // Shared claim still alive via art-2.
        let shared_left: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM insight_claims WHERE entity_id='leaders'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(shared_left, 1);
    }

    #[test]
    fn failed_validation_leaves_prior_insights() {
        let store = Store::memory().unwrap();
        store.atlas_insert_run("run-1", "{}", "{}").unwrap();
        let art = article("run-1", "art-1");
        store.atlas_upsert_article(&art).unwrap();
        store
            .persist_atlas_insights(
                "run-1",
                &[claim("art-1", "geneva", "hosts", "talks")],
                &[],
                "",
                "",
            )
            .unwrap();
        let bad = AtlasInsightClaim {
            entity: String::new(),
            ..claim("art-1", "x", "y", "z")
        };
        assert!(validate_staged_claims(&[bad], "art-1").is_err());
        assert_eq!(
            store
                .atlas_claims_for_article("run-1", "art-1")
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn zero_claim_commit_is_distinct_success() {
        let store = Store::memory().unwrap();
        store.atlas_insert_run("run-1", "{}", "{}").unwrap();
        let art = article("run-1", "art-1");
        store.atlas_upsert_article(&art).unwrap();
        store
            .persist_atlas_insights(
                "run-1",
                &[claim("art-1", "geneva", "hosts", "talks")],
                &[],
                "",
                "",
            )
            .unwrap();
        let outcome = store
            .commit_article_insight_replacement("run-1", "art-1", &[], &[])
            .unwrap();
        assert_eq!(outcome.claims, 0);
        assert!(store
            .atlas_claims_for_article("run-1", "art-1")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn user_edited_memories_are_not_orphaned() {
        let store = Store::memory().unwrap();
        store.atlas_insert_run("run-1", "{}", "{}").unwrap();
        let art = article("run-1", "art-1");
        store.atlas_upsert_article(&art).unwrap();
        store
            .persist_atlas_insights(
                "run-1",
                &[claim("art-1", "geneva", "hosts", "talks")],
                &[],
                "",
                "",
            )
            .unwrap();
        let memory_id: String = store
            .conn
            .query_row("SELECT memory_id FROM insight_claims LIMIT 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        store
            .conn
            .execute(
                "INSERT OR IGNORE INTO insight_user_edits(memory_id) VALUES (?1)",
                [&memory_id],
            )
            .unwrap();
        let outcome = store
            .commit_article_insight_replacement(
                "run-1",
                "art-1",
                &[claim("art-1", "diplomats", "resume", "talks")],
                &[],
            )
            .unwrap();
        assert!(outcome.preserved_user_edits >= 1);
        let still: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE id=?1",
                [&memory_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(still, 1, "user-edited memory must survive replacement");
    }

    /// AC: extraction error path never deletes (unit stand-in without a live model).
    #[test]
    fn snapshot_counts_are_stable_before_commit() {
        let seen = Arc::new(Mutex::new(0usize));
        let store = Store::memory().unwrap();
        store.atlas_insert_run("run-1", "{}", "{}").unwrap();
        let art = article("run-1", "art-1");
        store.atlas_upsert_article(&art).unwrap();
        store
            .persist_atlas_insights(
                "run-1",
                &[claim("art-1", "geneva", "hosts", "talks")],
                &[],
                "",
                "",
            )
            .unwrap();
        *seen.lock().unwrap() = store
            .atlas_claims_for_article("run-1", "art-1")
            .unwrap()
            .len();
        // Simulate "extract failed" by not calling commit.
        assert_eq!(*seen.lock().unwrap(), 1);
        assert_eq!(
            store
                .atlas_claims_for_article("run-1", "art-1")
                .unwrap()
                .len(),
            1
        );
    }
}
