//! Atlas run storage tests (migration phase 3) against a real SurrealDB
//! 3.3.4 disk server or the embedded RocksDB engine (see `support`).
//! Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::storage::atlas;
use argos_osint_core::store::AtlasArticleRow;
use support::*;
use surrealdb::engine::any::Any;
use surrealdb::Surreal;

async fn fresh(server: &Server, name: &str) -> Surreal<Any> {
    let db = server.root("argos", name).await;
    atlas::apply_schema(&db).await.unwrap();
    db
}

fn article(run: &str, id: &str, seen: &str) -> AtlasArticleRow {
    AtlasArticleRow {
        run_id: run.into(),
        id: id.into(),
        title: format!("t-{id}"),
        url: format!("https://x/{id}"),
        temperature: 0.5,
        category: "unk".into(),
        seen_at: seen.into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn runs_round_trip_state_phase_and_order() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "atlas_runs").await;
    atlas::insert_run(&db, "r1", "{}", "{}").await.unwrap();
    assert!(atlas::insert_run(&db, "r1", "{}", "{}").await.is_err());
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    atlas::insert_run(&db, "r2", "{}", "{}").await.unwrap();
    atlas::save(&db, "r1", r#"{"phase":3}"#, "{\"n\":1}")
        .await
        .unwrap();
    atlas::set_state(&db, "r1", "done", "ok", true)
        .await
        .unwrap();
    atlas::set_state(&db, "r1", "done", "again", false)
        .await
        .unwrap();
    let runs = atlas::list_runs(&db).await.unwrap();
    assert_eq!(
        runs.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["r2", "r1"]
    );
    let r1 = &runs[1];
    assert_eq!(
        (r1.phase, r1.state.as_str(), r1.note.as_str()),
        (3, "done", "again")
    );
    assert!(
        !r1.finished_at.is_empty(),
        "unfinished set_state keeps finished_at"
    );
    atlas::park_running(&db).await.unwrap();
    let latest = atlas::latest_run(&db).await.unwrap().unwrap();
    assert_eq!(
        (latest.id.as_str(), latest.state.as_str()),
        ("r2", "paused")
    );
    assert_eq!(atlas::run_days(&db).await.unwrap().len(), 1);
}

#[tokio::test]
async fn articles_upsert_list_and_latest() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "atlas_articles").await;
    atlas::upsert_article(&db, &article("r1", "b", "2026-01-02"))
        .await
        .unwrap();
    atlas::upsert_article(&db, &article("r1", "a", "2026-01-02"))
        .await
        .unwrap();
    atlas::upsert_article(&db, &article("r2", "a", "2026-01-03"))
        .await
        .unwrap();
    let mut changed = article("r1", "b", "2026-01-01");
    changed.title = "new".into();
    atlas::upsert_article(&db, &changed).await.unwrap();
    atlas::set_category(&db, "r1", "a", "conflict")
        .await
        .unwrap();
    let list = atlas::list_articles(&db, "r1").await.unwrap();
    assert_eq!(
        list.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
        ["b", "a"]
    );
    assert_eq!(list[0], changed);
    assert_eq!(list[1].category, "conflict");
    assert_eq!(
        atlas::article(&db, "r2", "a")
            .await
            .unwrap()
            .unwrap()
            .seen_at,
        "2026-01-03"
    );
    assert!(atlas::article(&db, "r2", "zz").await.unwrap().is_none());
    assert_eq!(
        atlas::latest_article_by_id(&db, "a")
            .await
            .unwrap()
            .unwrap()
            .run_id,
        "r2"
    );
    assert_eq!(atlas::recent_articles(&db).await.unwrap().len(), 3);
}

#[tokio::test]
async fn quota_checkpoints_and_repairs() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "atlas_misc").await;
    assert_eq!(atlas::quota_used(&db, "p", "d").await.unwrap(), 0);
    let mut hs = Vec::new();
    for _ in 0..8 {
        let db = db.clone();
        hs.push(tokio::spawn(async move {
            atlas::quota_bump(&db, "p", "d").await.unwrap()
        }));
    }
    let mut got: Vec<u32> = Vec::new();
    for h in hs {
        got.push(h.await.unwrap());
    }
    got.sort();
    assert_eq!(
        got,
        (1..=8).collect::<Vec<_>>(),
        "concurrent bumps are atomic"
    );
    assert_eq!(atlas::quota_used(&db, "p", "d").await.unwrap(), 8);

    assert!(atlas::save_checkpoint(&db, "r", "v1", "{\"a\":1}", 2, 1)
        .await
        .unwrap());
    assert!(!atlas::save_checkpoint(&db, "r", "v1", "{\"a\":2}", 0, 0)
        .await
        .unwrap());
    assert_eq!(
        atlas::checkpoint(&db, "r", "v1").await.unwrap().as_deref(),
        Some("{\"a\":1}")
    );
    assert!(atlas::checkpoint(&db, "r", "v2").await.unwrap().is_none());
    atlas::set_repair(&db, "r", "pending", "{}", "j")
        .await
        .unwrap();
    atlas::set_repair(&db, "r", "done", "{}", "j")
        .await
        .unwrap();
    assert_eq!(
        atlas::repair_status(&db, "r").await.unwrap().as_deref(),
        Some("done")
    );
}

async fn plan(db: &Surreal<Any>, sql: &str) -> String {
    let mut r = db
        .query(format!("{sql} EXPLAIN FULL"))
        .bind(("run", "r".to_string()))
        .bind(("article", "a".to_string()))
        .await
        .unwrap()
        .check()
        .unwrap();
    format!("{:?}", r.take::<surrealdb::types::Value>(0).unwrap())
}

#[tokio::test]
async fn article_lookups_use_their_indexes() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "atlas_plans").await;
    for (sql, index) in [
        (atlas::RUN_ARTICLES_SURQL, "atlas_article_run"),
        (atlas::LATEST_ARTICLE_SURQL, "atlas_article_id"),
    ] {
        let p = plan(&db, sql).await;
        assert!(p.contains(index), "{sql}: {p}");
        assert!(!p.contains("Iterate Table"), "{sql}: {p}");
    }
}
