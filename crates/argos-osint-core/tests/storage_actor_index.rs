//! Atlas phase 6 (actor index) on SurrealDB (`atlas_actor_index::surreal`,
//! `storage::lifecycle` article-index helpers): every occurrence gets a
//! receipt, zero-actor occurrences included, reruns reuse receipts, and
//! mentions/links land in the repository. Run through
//! `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::atlas_actor_index::{index_run_store, surreal};
use argos_osint_core::atlas_metadata::{ArticleMention, ArticleMetadata, MetadataState};
use argos_osint_core::atlas_store::AtlasStore;
use argos_osint_core::storage::{bootstrap, lifecycle, repo::Repository};
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

async fn count(repo: &Repository, sql: &str) -> i64 {
    let mut r = repo.db().query(sql).await.unwrap().check().unwrap();
    let n: Option<i64> = r.take(0).unwrap();
    n.unwrap_or(0)
}

async fn repo(server: &Server, db: &str) -> Repository {
    let repo = server.repo("argos", db).await;
    bootstrap::ensure_schema(repo.db()).await.unwrap();
    lifecycle::apply_article_index_schema(repo.db())
        .await
        .unwrap();
    repo
}

#[tokio::test]
async fn phase6_receipts_every_occurrence_and_reuses_them() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = repo(&server, "actor_index").await;
    for i in 0..6 {
        if i % 3 == 0 {
            seed(&repo, "r", &format!("a{i}"), "Weather is mild", &[]).await;
        } else {
            seed(
                &repo,
                "r",
                &format!("a{i}"),
                "Jane Doe speaks",
                &["Jane Doe"],
            )
            .await;
        }
    }
    let report = surreal::index_run(&repo, "r", &|| false).await.unwrap();
    assert!(report.complete(), "{report:?}");
    assert_eq!((report.total, report.indexed, report.zero_actor), (6, 6, 2));
    assert_eq!(
        count(&repo, "RETURN count(SELECT 1 FROM atlas_article_index)").await,
        6
    );
    assert_eq!(
        count(
            &repo,
            "RETURN count(SELECT 1 FROM brain_actor_mention WHERE origin_kind = 'article')"
        )
        .await,
        4
    );
    assert!(
        count(
            &repo,
            "RETURN count(SELECT 1 FROM memory WHERE memory_kind = 'link' AND lifecycle = 'active')"
        )
        .await
            >= 4
    );
    assert_eq!(
        count(
            &repo,
            "RETURN count(SELECT 1 FROM brain_backfill_receipt WHERE source = 'atlas_articles')"
        )
        .await,
        6
    );
    let again = surreal::index_run(&repo, "r", &|| false).await.unwrap();
    assert_eq!((again.reused, again.indexed), (6, 0));
    assert_eq!(surreal::receipts(&repo, "r").await.unwrap().len(), 6);
}

#[tokio::test]
async fn phase6_stops_before_a_batch_and_dispatches_through_atlas_store() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = repo(&server, "actor_index_stop").await;
    seed(&repo, "r", "a1", "Jane Doe speaks", &["Jane Doe"]).await;
    let stopped = surreal::index_run(&repo, "r", &|| true).await.unwrap();
    assert!(stopped.stopped && !stopped.complete());
    assert_eq!(stopped.indexed, 0);

    let store = AtlasStore::from_repository(repo.clone());
    let report = index_run_store(&store, "r", &|| false).await.unwrap();
    assert!(report.complete(), "{report:?}");
    // Empty run ids never reach a record id.
    let empty = surreal::index_run(&repo, "", &|| false).await.unwrap();
    assert_eq!(empty.total, 0);
}
