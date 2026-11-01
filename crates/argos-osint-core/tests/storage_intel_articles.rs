//! Article occurrence pagination through the Intel service and real storage.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::intel_service::{articles, ArticleQuery};
use argos_osint_core::storage::{atlas, repo::Repository};
use argos_osint_core::store::AtlasArticleRow;
use support::Server;

fn query(category: &str) -> ArticleQuery {
    ArticleQuery {
        category: category.into(),
        day: "2026-01-02".into(),
        limit: Some(2),
        ..Default::default()
    }
}

#[test]
fn category_remains_required_and_validated() {
    assert!(query("all").validate().is_ok());
    assert!(query("military").validate().is_ok());
    assert!(query("").validate().is_err());
    assert!(query("unknown").validate().is_err());
    let mut invalid = query("all");
    invalid.cursor = Some("not json".into());
    assert!(invalid.validate().is_err());
}

#[tokio::test]
async fn all_pages_preserve_global_order_and_occurrence_identity() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.root("argos", "intel_article_pages").await;
    let repo = Repository::attach(db, "argos", "intel_article_pages")
        .await
        .expect("bootstrap repository");
    for (run, day) in [
        ("r1", "2026-01-02"),
        ("r2", "2026-01-02"),
        ("old", "2026-01-01"),
    ] {
        atlas::insert_run(repo.db(), run, "{}", "{}")
            .await
            .expect("insert run");
        repo.db()
            .query("UPDATE type::record('atlas_run', $run) SET started_at = $day")
            .bind(("run", run.to_string()))
            .bind(("day", day.to_string()))
            .await
            .expect("update run")
            .check()
            .expect("set run day");
    }
    for (run, id, category, published, seen) in [
        ("r1", "new", "economic", "2026-01-04", "2026-01-04"),
        ("r1", "z", "military", "2026-01-03", "2026-01-04"),
        ("r1", "a", "economic", "2026-01-03", "2026-01-03"),
        ("r2", "a", "military", "2026-01-03", "2026-01-03"),
        ("r1", "b", "technology", "2026-01-03", "2026-01-03"),
        ("r1", "uncategorized", "unk", "2026-01-01", "2026-01-01"),
        ("old", "excluded", "economic", "2026-01-05", "2026-01-05"),
    ] {
        let row = AtlasArticleRow {
            run_id: run.into(),
            id: id.into(),
            category: category.into(),
            published_at: published.into(),
            seen_at: seen.into(),
            title: format!("fixture {id}"),
            ..Default::default()
        };
        atlas::upsert_article(repo.db(), &row)
            .await
            .expect("insert article");
    }

    let mut q = query("all");
    let mut occurrences = Vec::new();
    loop {
        let page = articles(&repo, &q).await.expect("read all page");
        assert!(page.rows.len() <= 2);
        occurrences.extend(page.rows.into_iter().map(|row| (row.id, row.run_id)));
        match page.next_cursor {
            Some(cursor) => q.cursor = Some(cursor),
            None => break,
        }
        assert!(occurrences.len() < 10, "pagination must terminate");
    }
    assert_eq!(
        occurrences,
        [
            ("new", "r1"),
            ("z", "r1"),
            ("a", "r1"),
            ("a", "r2"),
            ("b", "r1"),
            ("uncategorized", "r1")
        ]
        .map(|(id, run)| (id.to_string(), run.to_string()))
    );

    let mut filtered = query("economic");
    filtered.search = "FIXTURE".into();
    let page = articles(&repo, &filtered)
        .await
        .expect("read category page");
    assert_eq!(
        page.rows
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        ["new", "a"]
    );
    assert!(page.next_cursor.is_none());
    filtered.search = "missing".into();
    assert!(articles(&repo, &filtered)
        .await
        .expect("empty search")
        .rows
        .is_empty());
}
