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
    for (run, id, categories, published, seen) in [
        ("r1", "new", vec!["economic"], "2026-01-04", "2026-01-04"),
        ("r1", "z", vec!["military"], "2026-01-03", "2026-01-04"),
        (
            "r1",
            "a",
            vec!["economic", "military"],
            "2026-01-03",
            "2026-01-03",
        ),
        // A newer occurrence of the same identity: All deduplicates to it.
        ("r2", "a", vec!["economic"], "2026-01-03", "2026-01-05"),
        ("r1", "b", vec!["technology"], "2026-01-03", "2026-01-03"),
        // A legacy row without v2 tags is pending migration: not admitted.
        ("r1", "uncategorized", vec![], "2026-01-01", "2026-01-01"),
        (
            "old",
            "excluded",
            vec!["economic"],
            "2026-01-05",
            "2026-01-05",
        ),
    ] {
        let row = AtlasArticleRow {
            run_id: run.into(),
            id: id.into(),
            category: categories.first().copied().unwrap_or("unk").into(),
            categories: categories.iter().map(|tag| tag.to_string()).collect(),
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
        [("new", "r1"), ("a", "r2"), ("z", "r1"), ("b", "r1")]
            .map(|(id, run)| (id.to_string(), run.to_string())),
        "All deduplicates identity (newest occurrence wins) and excludes pending migration"
    );

    let mut filtered = query("economic");
    filtered.search = "FIXTURE".into();
    let page = articles(&repo, &filtered)
        .await
        .expect("read category page");
    assert_eq!(
        page.rows
            .iter()
            .map(|row| (row.id.as_str(), row.run_id.as_str()))
            .collect::<Vec<_>>(),
        [("new", "r1"), ("a", "r2")],
        "category filters use array membership on the newest occurrence"
    );
    assert!(page.next_cursor.is_none());
    filtered.search = "missing".into();
    assert!(articles(&repo, &filtered)
        .await
        .expect("empty search")
        .rows
        .is_empty());
}

#[tokio::test]
async fn hottest_pages_order_the_entire_day_with_stable_temperature_ties() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "hottest_pages").await;
    atlas::insert_run(repo.db(), "r", "{}", "{}").await.unwrap();
    repo.db()
        .query("UPDATE atlas_run:r SET started_at = '2026-01-02'")
        .await
        .unwrap()
        .check()
        .unwrap();
    for (id, heat, date) in [
        ("cold-new", 0.1, "2026-01-04"),
        ("hot-old", 0.9, "2026-01-01"),
        ("tie-a", 0.5, "2026-01-03"),
        ("tie-b", 0.5, "2026-01-03"),
    ] {
        atlas::upsert_article(
            repo.db(),
            &AtlasArticleRow {
                run_id: "r".into(),
                id: id.into(),
                category: "military".into(),
                categories: vec!["military".into()],
                title: id.into(),
                temperature: heat,
                published_at: date.into(),
                seen_at: date.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
    let mut q = query("all");
    q.sort = "hottest".into();
    q.limit = Some(1);
    let mut ids = Vec::new();
    loop {
        let page = articles(&repo, &q).await.unwrap();
        ids.extend(page.rows.into_iter().map(|a| a.id));
        match page.next_cursor {
            Some(c) => q.cursor = Some(c),
            None => break,
        }
        assert!(ids.len() <= 4, "cursor must advance across ties");
    }
    assert_eq!(ids, ["hot-old", "tie-a", "tie-b", "cold-new"]);
    q.cursor = None;
    q.sort = "latest".into();
    assert_eq!(articles(&repo, &q).await.unwrap().rows[0].id, "cold-new");
}

#[tokio::test]
async fn brief_uses_a_labeled_publisher_prior_without_fabricating_unknown_scores() {
    use argos_osint_core::atlas_metadata::{self, ArticleMetadata, MetadataState, PublisherPrior};
    use argos_osint_core::osint::wikipedia_rsp::SourceReliabilityObservation;
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "brief_rating_prior").await;
    atlas::insert_run(repo.db(), "r", "{}", "{}").await.unwrap();
    let article = AtlasArticleRow {
        run_id: "r".into(),
        id: "rated".into(),
        title: "A retained report".into(),
        source_domain: "fixture.test".into(),
        ..Default::default()
    };
    atlas::upsert_article(repo.db(), &article).await.unwrap();
    let mut observation =
        SourceReliabilityObservation::from_lookup("fixture.test", "Fixture Wire", None);
    observation.reliability = "B".into();
    let mut metadata = ArticleMetadata {
        run_id: "r".into(),
        article_id: "rated".into(),
        input_hash: atlas_metadata::input_hash(&article),
        entity_state: MetadataState::Complete,
        reliability_state: MetadataState::Complete,
        entity_version: atlas_metadata::ENTITY_VERSION.into(),
        prior_version: atlas_metadata::PRIOR_VERSION.into(),
        classification_status: "fixture".into(),
        classification_receipt: None,
        publisher_prior: Some(PublisherPrior {
            observation,
            tool_id: "fixture".into(),
            matched_domain: Some("fixture.test".into()),
            fetched_at: "2026-11-02T00:00:00Z".into(),
            score: Some(85.0),
            version: "fixture".into(),
            stale: false,
        }),
        mentions: vec![],
        diagnostics: vec![],
        entity_ms: 0,
        publisher_ms: 0,
    };
    argos_osint_core::storage::atlas_metadata::save_metadata(repo.db(), &metadata)
        .await
        .unwrap();
    let view = argos_osint_core::intel_service::brief(&repo, &article)
        .await
        .unwrap();
    assert_eq!(view["confidence"]["score"], 60);
    assert_eq!(view["confidence"]["provisional"], true);
    assert_eq!(view["confidence_grade"], "B6");
    assert_eq!(view["confidence_basis"], "publisher_prior");
    metadata.publisher_prior = None;
    metadata.reliability_state = MetadataState::Unavailable;
    argos_osint_core::storage::atlas_metadata::save_metadata(repo.db(), &metadata)
        .await
        .unwrap();
    let unknown = argos_osint_core::intel_service::brief(&repo, &article)
        .await
        .unwrap();
    assert!(unknown["confidence"]["score"].is_null());
    assert_eq!(unknown["confidence_basis"], "unavailable");
    assert!(unknown["confidence_reason"]
        .as_str()
        .unwrap()
        .contains("publisher reliability"));
}
