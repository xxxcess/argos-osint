//! The actor-dedup pool on SurrealDB (`storage::actor_dedup`): local
//! same-scope spelling repair, candidate pairs with context, pair leases,
//! the assessed merge and explicit separations.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::brain_actors::BindRequest;
use argos_osint_core::memory_metadata::ActorSubtype;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::tx::{run_tx, RetryPolicy};
use argos_osint_core::storage::{actor_bind, actor_dedup, bootstrap, entities, lifecycle};
use argos_osint_core::store::AtlasArticleRow;
use serde_json::json;
use support::*;

async fn repo(server: &Server, db: &str) -> Repository {
    let repo = server.repo("argos", db).await;
    bootstrap::ensure_schema(repo.db()).await.unwrap();
    repo
}

async fn bind(repo: &Repository, name: &str, scope: &str) -> String {
    let (name, scope) = (name.to_string(), scope.to_string());
    run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (name, scope) = (name.clone(), scope.clone());
        async move {
            let req = BindRequest {
                name: &name,
                scope: &scope,
                subtype: ActorSubtype::Person,
                identifiers: &[],
                external: None,
                provenance: json!({}),
            };
            let now = chrono::Utc::now().to_rfc3339();
            Ok(actor_bind::bind_in(&tx, &req, &now).await?.0.actor_id)
        }
    })
    .await
    .unwrap()
    .value
}

async fn query<T>(repo: &Repository, sql: &str) -> T
where
    T: surrealdb::types::SurrealValue,
    usize: surrealdb::opt::QueryResult<T>,
{
    let mut r = repo.db().query(sql).await.unwrap().check().unwrap();
    r.take(0usize).unwrap()
}

async fn article_mention(repo: &Repository, actor: &str, id: &str, title: &str) {
    let a = AtlasArticleRow {
        run_id: "r".into(),
        id: id.into(),
        title: title.into(),
        description: "board meeting".into(),
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
    let rev = argos_osint_core::atlas_metadata::input_hash(&a);
    let (actor, id) = (actor.to_string(), id.to_string());
    run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (actor, id, rev) = (actor.clone(), id.clone(), rev.clone());
        async move {
            let now = chrono::Utc::now().to_rfc3339();
            lifecycle::record_mention_in(&tx, &actor, "article", "r", &id, &rev, "[]", &now).await
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn repair_merges_same_scope_spellings_into_the_lowest_id() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = repo(&server, "actor_dedup_repair").await;
    let a = bind(&repo, "Jane Doe", "manual:a").await;
    let b = bind(&repo, "Jane Doe", "manual:b").await;
    assert_ne!(a, b);
    let (low, high) = if a < b { (a, b) } else { (b, a) };
    // A legacy spelling in the same scope as `low`.
    repo.db()
        .query(format!(
            "UPDATE brain_actor:`{high}` SET display_name = \"Jane Doe's\", scope_key = \
             (SELECT VALUE scope_key FROM ONLY brain_actor:`{low}`)"
        ))
        .await
        .unwrap()
        .check()
        .unwrap();
    assert!(actor_dedup::repair_local(&repo, 100).await.unwrap() >= 2);
    let to: Option<String> = query(
        &repo,
        &format!("SELECT VALUE to_actor FROM ONLY brain_actor_redirect:`{high}`"),
    )
    .await;
    assert_eq!(to.as_deref(), Some(low.as_str()));
    let state: Option<String> = query(
        &repo,
        &format!("SELECT VALUE lifecycle FROM ONLY memory:`{high}`"),
    )
    .await;
    assert_eq!(state.as_deref(), Some("redirected"));
    let survivor: Option<String> = query(
        &repo,
        &format!("SELECT VALUE identity_state FROM ONLY brain_actor:`{low}`"),
    )
    .await;
    assert_eq!(survivor.as_deref(), Some("resolved"));
    let name: Option<String> = query(
        &repo,
        &format!("SELECT VALUE display_name FROM ONLY brain_actor:`{high}`"),
    )
    .await;
    assert_eq!(name.as_deref(), Some("Jane Doe"));
    // Receipts are current: a rerun merges nothing more.
    actor_dedup::repair_local(&repo, 100).await.unwrap();
    let n: Option<i64> = query(&repo, "RETURN count(SELECT 1 FROM brain_actor_redirect)").await;
    assert_eq!(n, Some(1));
}

#[tokio::test]
async fn leased_same_decision_merges_and_separations_are_never_proposed() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = repo(&server, "actor_dedup_pairs").await;
    let a = bind(&repo, "Jane Doe", "manual:a").await;
    let b = bind(&repo, "Jane Doe", "manual:b").await;
    article_mention(&repo, &a, "a1", "Jane Doe chaired the Acme board").await;
    article_mention(&repo, &b, "a2", "Jane Doe left the Acme board").await;
    let pairs = actor_dedup::candidates(&repo, 8).await.unwrap();
    assert_eq!(pairs.len(), 1, "{pairs:?}");
    let c = &pairs[0];
    assert!(actor_dedup::acquire(&repo, c, "w1").await.unwrap());
    assert!(
        !actor_dedup::acquire(&repo, c, "w2").await.unwrap(),
        "lease held"
    );
    assert!(actor_dedup::candidates(&repo, 8).await.unwrap().is_empty());
    assert!(actor_dedup::renew(&repo, c, "w1").await.unwrap());
    assert!(!actor_dedup::renew(&repo, c, "w2").await.unwrap());
    // Another owner cannot apply; the holder can.
    assert!(!actor_dedup::apply(&repo, c, "same", "w2", &json!({}))
        .await
        .unwrap());
    assert!(actor_dedup::apply(&repo, c, "same", "w1", &json!({"t":1}))
        .await
        .unwrap());
    let n: Option<i64> = query(&repo, "RETURN count(SELECT 1 FROM brain_actor_redirect)").await;
    assert_eq!(n, Some(1));
    let decision: Option<String> = query(
        &repo,
        "SELECT VALUE decision FROM ONLY brain_actor_dedup_receipt LIMIT 1",
    )
    .await;
    assert_eq!(decision.as_deref(), Some("same"));

    // A fresh pair that an analyst separated is never proposed.
    let x = bind(&repo, "John Roe", "manual:a").await;
    let y = bind(&repo, "John Roe", "manual:b").await;
    article_mention(&repo, &x, "a3", "John Roe met the press").await;
    article_mention(&repo, &y, "a4", "John Roe met investors").await;
    assert_eq!(actor_dedup::candidates(&repo, 8).await.unwrap().len(), 1);
    let (x2, y2) = (x.clone(), y.clone());
    run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (x, y) = (x2.clone(), y2.clone());
        async move { entities::mark_separate_in(&tx, &x, &y, "{}", "now").await }
    })
    .await
    .unwrap();
    assert!(actor_dedup::candidates(&repo, 8).await.unwrap().is_empty());
}
