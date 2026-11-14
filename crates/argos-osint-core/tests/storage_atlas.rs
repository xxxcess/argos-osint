//! Atlas run storage tests (migration phase 3) against a real SurrealDB
//! 3.3.4 disk server or the embedded RocksDB engine (see `support`).
//! Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

#[path = "support/current_admission.rs"]
mod current_admission;
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
async fn quota_reservations_share_daily_account_caps() {
    let Some(server) = Server::start() else {
        return;
    };
    let first = fresh(&server, "atlas_quota_reservations").await;
    let second = server.root("argos", "atlas_quota_reservations").await;
    let day = "2026-01-01";
    let mut workers = Vec::new();
    for db in [&first, &second].into_iter().cycle().take(8) {
        let db = Surreal::clone(db);
        workers.push(tokio::spawn(async move {
            atlas::quota_reserve(&db, "newsapi", day, 3, None)
                .await
                .unwrap()
        }));
    }
    let mut admitted = 0;
    for worker in workers {
        admitted += u32::from(worker.await.unwrap());
    }
    assert_eq!(admitted, 3, "both workers use one cap");
    assert_eq!(atlas::quota_used(&second, "newsapi", day).await.unwrap(), 3);

    let fallback = "newsapi:fallback";
    assert!(atlas::quota_reserve(&second, fallback, day, 1, None)
        .await
        .unwrap());
    assert!(
        !atlas::quota_reserve(&first, fallback, day, 1, Some(("newsapi", day)))
            .await
            .unwrap()
    );
    assert_eq!(atlas::quota_used(&second, "newsapi", day).await.unwrap(), 3);
    assert_eq!(atlas::quota_used(&first, fallback, day).await.unwrap(), 1);

    let next_day = "2026-01-02";
    assert!(
        atlas::quota_reserve(&first, fallback, next_day, 1, Some(("newsapi", day)))
            .await
            .unwrap()
    );
    assert_eq!(atlas::quota_used(&second, "newsapi", day).await.unwrap(), 2);
    assert_eq!(
        atlas::quota_used(&second, fallback, next_day)
            .await
            .unwrap(),
        1
    );
    assert!(atlas::quota_reserve(&second, "newsapi", day, 3, None)
        .await
        .unwrap());
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

#[tokio::test]
async fn admission_stages_admits_and_discards_with_receipts() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "atlas_admission").await;
    let tax = argos_osint_core::atlas::TAXONOMY_VERSION;

    // Stage → idempotent re-stage keeps one pending row.
    let staged = article("r1", "a", "2026-01-02");
    assert!(atlas::stage_article(&db, &staged, "rev-1", tax)
        .await
        .unwrap());
    assert!(!atlas::stage_article(&db, &staged, "rev-1", tax)
        .await
        .unwrap());
    let rows = atlas::admission_rows(&db, "r1").await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].state.as_str(), rows[0].attempts), ("pending", 0));

    // Evaluating resets when a cycle dies mid-batch, so it retries.
    let hash = rows[0].input_hash.clone();
    assert!(atlas::admission_begin(&db, "r1", "a", &hash).await.unwrap());
    atlas::admission_reset_stale(&db).await.unwrap();
    let rows = atlas::admission_rows(&db, "r1").await.unwrap();
    assert_eq!(rows[0].state.as_str(), "pending");

    // Admit: canonical ordered categories with the scalar projecting the
    // first tag; the staging row is pruned in the same transaction. The
    // two-tag minimum and seven-tag ceiling are storage invariants.
    let mut admitted = staged.clone();
    admitted.categories = vec!["science".into(), "health".into()];
    atlas::admission_admit(&db, &admitted, &hash, tax)
        .await
        .unwrap();
    assert!(atlas::admission_rows(&db, "r1").await.unwrap().is_empty());
    let canonical = atlas::article(&db, "r1", "a")
        .await
        .unwrap()
        .expect("admitted article is canonical");
    assert_eq!(canonical.categories, ["science", "health"]);
    assert_eq!(canonical.category, "science");
    assert!(!canonical.admitted_at.is_empty());
    // Over the ceiling: refused, never written.
    let mut padded = staged.clone();
    padded.categories = (0..8).map(|n| format!("tag{n}")).collect();
    assert!(atlas::admission_admit(&db, &padded, &hash, tax)
        .await
        .is_err());
    // Below the minimum: refused as well.
    let mut single = staged.clone();
    single.categories = vec!["science".into()];
    assert!(atlas::admission_admit(&db, &single, &hash, tax)
        .await
        .is_err());

    // Discard: a minimal rejection receipt; reconsideration requires a
    // changed input or taxonomy.
    let staged2 = article("r1", "b", "2026-01-02");
    atlas::stage_article(&db, &staged2, "rev-2", tax)
        .await
        .unwrap();
    let rows = atlas::admission_rows(&db, "r1").await.unwrap();
    let (id2, hash2, revision2) = (
        rows[0].article_id.clone(),
        rows[0].input_hash.clone(),
        rows[0].source_revision.clone(),
    );
    assert!(!atlas::rejection_exists(&db, &id2, &hash2, tax)
        .await
        .unwrap());
    atlas::admission_discard(
        &db,
        "r1",
        &id2,
        &hash2,
        &revision2,
        tax,
        "fewer than two fitting categories",
    )
    .await
    .unwrap();
    assert!(atlas::admission_rows(&db, "r1").await.unwrap().is_empty());
    assert!(atlas::rejection_exists(&db, &id2, &hash2, tax)
        .await
        .unwrap());
    assert!(!atlas::rejection_exists(&db, &id2, "changed-input", tax)
        .await
        .unwrap());
    assert!(
        !atlas::rejection_exists(&db, &id2, &hash2, "atlas-taxonomy-v1")
            .await
            .unwrap()
    );
    assert!(atlas::article(&db, "r1", &id2).await.unwrap().is_none());

    // A retryable failure keeps the input privately staged with its error.
    let staged3 = article("r1", "c", "2026-01-02");
    atlas::stage_article(&db, &staged3, "rev-3", tax)
        .await
        .unwrap();
    let rows = atlas::admission_rows(&db, "r1").await.unwrap();
    let (id3, hash3) = (rows[0].article_id.clone(), rows[0].input_hash.clone());
    atlas::admission_begin(&db, "r1", &id3, &hash3)
        .await
        .unwrap();
    atlas::admission_failed(&db, "r1", &id3, &hash3, "all routes blocked")
        .await
        .unwrap();
    let rows = atlas::admission_rows(&db, "r1").await.unwrap();
    assert_eq!(
        (
            rows[0].state.as_str(),
            rows[0].attempts,
            rows[0].last_error.as_str()
        ),
        ("failed", 1, "all routes blocked")
    );
    let work = atlas::admission_work(&db).await.unwrap();
    assert_eq!(work.len(), 1, "failed rows stay retryable");
}

#[tokio::test]
async fn article_views_filter_by_category_membership_and_dedup_identity() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "atlas_membership").await;
    let db = repo.db();
    atlas::insert_run(db, "r1", "{}", "{}").await.unwrap();
    let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let mut science = article("r1", "a", "2026-01-02");
    current_admission::admit(&repo, &mut science, "River sensor common").await;
    let mut security = article("r1", "b", "2026-01-02");
    current_admission::admit(&repo, &mut security, "Ballot patrol common").await;
    // Current-looking tags on an unclassified legacy row are not authority.
    let mut legacy = article("r1", "c", "2026-01-02");
    legacy.categories = science.categories.clone();
    atlas::upsert_article(db, &legacy).await.unwrap();

    let science_view = atlas::articles_for_intel(db, "scientific", &day, "")
        .await
        .unwrap();
    assert_eq!(
        science_view
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>(),
        ["a"]
    );
    assert!(
        atlas::articles_for_intel(db, "science", &day, "")
            .await
            .is_err(),
        "obsolete filters are rejected"
    );
    let all = atlas::articles_for_intel(db, "all", &day, "")
        .await
        .unwrap();
    assert_eq!(
        all.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
        ["a", "b"],
        "only real current admissions are listed"
    );
}

#[tokio::test]
async fn article_category_and_fulltext_indexes_are_defined() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "atlas_search_indexes").await;
    // Index definitions live on their table: `INFO FOR DB` maps every table
    // to its bare `DEFINE TABLE` string in SurrealDB 3.3.4 and never lists
    // indexes, so the table info is the catalog to introspect.
    let mut r = db
        .query("INFO FOR TABLE atlas_article")
        .await
        .unwrap()
        .check()
        .unwrap();
    let info: Option<serde_json::Value> = r
        .take::<Option<surrealdb::types::Value>>(0usize)
        .unwrap()
        .map(|v| v.into_json_value());
    let info = info.expect("table info").to_string();
    for index in [
        "atlas_article_categories",
        "atlas_article_title_search",
        "atlas_article_description_search",
    ] {
        assert!(info.contains(index), "missing index {index}");
    }
}

/// The membership predicate `filters()` builds for a named category
/// (storage/article_discovery.rs), pinned so the plan can be inspected.
const MEMBERSHIP_SURQL: &str = "SELECT * FROM atlas_article WHERE categories CONTAINS $category";

#[tokio::test]
async fn category_membership_filters_use_the_categories_index() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "atlas_membership_plans").await;
    let mut r = db
        .query(format!("{MEMBERSHIP_SURQL} EXPLAIN FULL"))
        .bind(("category", "scientific".to_string()))
        .await
        .unwrap()
        .check()
        .unwrap();
    let p = format!("{:?}", r.take::<surrealdb::types::Value>(0).unwrap());
    assert!(
        p.contains("atlas_article_categories"),
        "membership must scan the categories index: {p}"
    );
    assert!(
        !p.contains("TableScan"),
        "membership must not fall back to a table scan: {p}"
    );
}

#[tokio::test]
async fn admission_existence_is_run_scoped_and_includes_all_retained_states() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = fresh(&server, "admission_preflight").await;
    assert!(!atlas::admission_pending_exists(&db, "requested")
        .await
        .unwrap());
    atlas::stage_article(
        &db,
        &article("other", "a", "2026-01-01"),
        "revision-1",
        "fixture-taxonomy",
    )
    .await
    .unwrap();
    assert!(!atlas::admission_pending_exists(&db, "requested")
        .await
        .unwrap());
    assert!(atlas::admission_pending_exists(&db, "other").await.unwrap());
    atlas::stage_article(
        &db,
        &article("requested", "b", "2026-01-01"),
        "revision-1",
        "fixture-taxonomy",
    )
    .await
    .unwrap();
    for state in ["pending", "evaluating", "failed", "future_live_state"] {
        db.query("UPDATE atlas_admission SET state = $state WHERE run_id = 'requested'")
            .bind(("state", state.to_string()))
            .await
            .unwrap()
            .check()
            .unwrap();
        assert!(
            atlas::admission_pending_exists(&db, "requested")
                .await
                .unwrap(),
            "{state}"
        );
    }
    db.query("DELETE atlas_admission WHERE run_id = 'requested'")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert!(!atlas::admission_pending_exists(&db, "requested")
        .await
        .unwrap());
    assert!(atlas::admission_pending_exists(&db, "other").await.unwrap());
}
