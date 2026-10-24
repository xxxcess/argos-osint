//! The Brain backfill pool on SurrealDB (`storage::brain_backfill`): Atlas
//! occurrences without a current index receipt are indexed in bounded
//! passes, and a rerun finds nothing left.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::atlas_metadata::{ArticleMention, ArticleMetadata, MetadataState};
use argos_osint_core::storage::{bootstrap, brain_backfill, lifecycle, repo::Repository};
use argos_osint_core::store::AtlasArticleRow;
use support::*;

async fn seed(repo: &Repository, run: &str, id: &str, title: &str, names: &[&str]) {
    let a = AtlasArticleRow {
        run_id: run.into(),
        id: id.into(),
        title: title.into(),
        url: format!("https://x.test/{id}"),
        source_name: "X".into(),
        source_domain: "x.test".into(),
        published_at: "2026-10-12".into(),
        provider: "fixture".into(),
        category: "unk".into(),
        ..Default::default()
    };
    argos_osint_core::storage::atlas::upsert_article(repo.db(), &a)
        .await
        .unwrap();
    let mut m = ArticleMetadata::pending(&a);
    m.entity_state = MetadataState::Complete;
    for n in names {
        let start = title.find(n).unwrap();
        m.mentions.push(ArticleMention {
            type_hint: "person".into(),
            comparison_key: n.to_lowercase(),
            field: "title".into(),
            start_byte: start,
            end_byte: start + n.len(),
            original_text: (*n).into(),
        });
    }
    argos_osint_core::storage::atlas_metadata::save_metadata(repo.db(), &m)
        .await
        .unwrap();
}

#[tokio::test]
async fn backfill_indexes_unreceipted_occurrences_within_the_budget() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "brain_backfill").await;
    bootstrap::ensure_schema(repo.db()).await.unwrap();
    lifecycle::apply_article_index_schema(repo.db())
        .await
        .unwrap();
    argos_osint_core::storage::atlas::insert_run(repo.db(), "r", "{}", "{}")
        .await
        .unwrap();
    for i in 0..3 {
        seed(
            &repo,
            "r",
            &format!("a{i}"),
            "Jane Doe speaks",
            &["Jane Doe"],
        )
        .await;
    }
    let first = brain_backfill::backfill(&repo, 2).await.unwrap();
    assert_eq!((first.processed, first.failed), (2, 0), "{first:?}");
    assert!(!first.complete);
    let second = brain_backfill::backfill(&repo, 100).await.unwrap();
    assert_eq!(second.processed, 1, "{second:?}");
    assert!(second.complete);
    assert_eq!(second.coverage[0].done, 3);
    let mut r = repo
        .db()
        .query("RETURN count(SELECT 1 FROM brain_actor_mention)")
        .await
        .unwrap()
        .check()
        .unwrap();
    let mentions: Option<i64> = r.take(0).unwrap();
    assert_eq!(mentions, Some(3));
    let third = brain_backfill::backfill(&repo, 100).await.unwrap();
    assert_eq!(third.processed, 0);
    assert!(third.complete);
}
