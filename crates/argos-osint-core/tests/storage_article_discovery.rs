//! Brain discovery contracts on real storage: `search_articles` over the
//! admitted-only canonical corpus and the indexed actor name/alias services.
//! Article discovery reads the two schema-v2 FULLTEXT BM25 indexes; actor
//! discovery reads the `brain_actor_alias_normalized` and `brain_actor_catalog`
//! indexes with point reads only.
#![cfg(feature = "surreal")]

#[path = "support/current_admission.rs"]
mod current_admission;
mod support;

use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;

use argos_osint_core::actor_catalog::Actor;
use argos_osint_core::storage::article_discovery::{self, ArticleSearchQuery, MatchedField};
use argos_osint_core::storage::{actor_catalog, atlas, atlas_delete, repo::Repository};
use argos_osint_core::store::AtlasArticleRow;

/// Admit through real local scoring; fixture metadata selects a fitted pair.
#[allow(clippy::too_many_arguments)]
async fn admit(
    repo: &Repository,
    run: &str,
    id: &str,
    title: &str,
    description: &str,
    categories: &[&str],
    published: &str,
    seen: &str,
) {
    let mut row = AtlasArticleRow {
        run_id: run.into(),
        id: id.into(),
        title: title.into(),
        description: description.into(),
        url: format!("https://example.com/{id}"),
        source_name: "Wire".into(),
        source_domain: "example.com".into(),
        published_at: published.into(),
        provider: "newsapi".into(),
        temperature: 1.0,
        category: categories.first().copied().unwrap_or("unk").into(),
        seen_at: seen.into(),
        categories: categories.iter().map(|tag| tag.to_string()).collect(),
        ..Default::default()
    };
    let vocabulary = if categories.contains(&"environmental") {
        "River sensor common"
    } else {
        "Freight clinic common"
    };
    current_admission::admit(repo, &mut row, vocabulary).await;
    assert_eq!(
        row.categories, categories,
        "fixture worker scores must select the fixture pair"
    );
}

async fn search(repo: &Repository, query: &str) -> article_discovery::ArticleSearchPage {
    let q = ArticleSearchQuery {
        query: query.into(),
        ..Default::default()
    };
    article_discovery::search_articles(repo, &q)
        .await
        .expect("search articles")
}

#[tokio::test]
async fn titles_rank_above_description_only_matches_with_field_attribution() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("discovery", "field_weighting").await;
    atlas::insert_run(repo.db(), "r1", "{}", "{}")
        .await
        .expect("insert run");
    admit(
        &repo,
        "r1",
        "title-hit",
        "Harbor Union approves the merger",
        "Unrelated description text",
        &["economic", "human"],
        "2026-01-03T00:00:00+00:00",
        "2026-01-03T00:00:00+00:00",
    )
    .await;
    admit(
        &repo,
        "r1",
        "description-hit",
        "Completely different headline",
        "The harbor union met behind closed doors",
        &["economic", "human"],
        "2026-01-03T00:00:00+00:00",
        "2026-01-03T00:00:00+00:00",
    )
    .await;
    let page = search(&repo, "harbor union").await;
    assert_eq!(page.hits.len(), 2, "{:?}", page.hits);
    assert_eq!(page.hits[0].article_id, "title-hit");
    assert_eq!(page.hits[0].matched_field, MatchedField::Title);
    assert!(page.hits[0].excerpt.to_lowercase().contains("harbor"));
    assert_eq!(page.hits[1].article_id, "description-hit");
    assert_eq!(page.hits[1].matched_field, MatchedField::Description);
    assert!(page.next_cursor.is_none());
    assert!(!page.revision.is_empty());
    // Every hit pins the occurrence's input revision.
    for hit in &page.hits {
        assert!(!hit.revision.is_empty());
        assert_eq!(hit.categories.first().map(String::as_str), Some("economic"));
        assert_eq!(hit.source_domain, "example.com");
    }
}

#[tokio::test]
async fn occurrences_deduplicate_to_the_newest_and_legacy_rows_stay_excluded() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("discovery", "dedup").await;
    for run in ["r1", "r2"] {
        atlas::insert_run(repo.db(), run, "{}", "{}")
            .await
            .expect("insert run");
    }
    admit(
        &repo,
        "r1",
        "dup",
        "Harbor Union merger approved",
        "First occurrence",
        &["economic", "human"],
        "2026-01-03T00:00:00+00:00",
        "2026-01-03T00:00:00+00:00",
    )
    .await;
    admit(
        &repo,
        "r2",
        "dup",
        "Harbor Union merger approved",
        "Newer occurrence",
        &["economic", "human"],
        "2026-01-03T00:00:00+00:00",
        "2026-01-05T00:00:00+00:00",
    )
    .await;
    // A legacy row (never admitted under the current contract) is pending migration, never a hit.
    atlas::upsert_article(
        repo.db(),
        &AtlasArticleRow {
            run_id: "r1".into(),
            id: "legacy".into(),
            title: "Harbor Union legacy row".into(),
            ..Default::default()
        },
    )
    .await
    .expect("insert legacy row");
    let page = search(&repo, "harbor union").await;
    assert_eq!(
        page.hits
            .iter()
            .map(|h| h.article_id.as_str())
            .collect::<Vec<_>>(),
        ["dup"],
        "one identity survives dedup; the unmigrated legacy row is not a hit"
    );
    assert_eq!(page.hits[0].run_id, "r2", "the newest occurrence wins");
}

#[tokio::test]
async fn filters_narrow_by_category_day_source_and_run() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("discovery", "filters").await;
    admit(
        &repo,
        "r1",
        "in-day",
        "Harbor Union meeting",
        "Scoped description",
        &["economic", "human"],
        "2026-01-03T00:00:00+00:00",
        "2026-01-03T00:00:00+00:00",
    )
    .await;
    admit(
        &repo,
        "r2",
        "out-of-day",
        "Harbor Union meeting",
        "Scoped description",
        &["environmental", "scientific"],
        "2026-01-05T00:00:00+00:00",
        "2026-01-05T00:00:00+00:00",
    )
    .await;
    admit(
        &repo,
        "r2",
        "other-source",
        "Harbor Union meeting",
        "Scoped description",
        &["economic", "human"],
        "2026-01-05T00:00:00+00:00",
        "2026-01-05T00:00:00+00:00",
    )
    .await;
    let mut other = atlas::article(repo.db(), "r2", "other-source")
        .await
        .unwrap()
        .unwrap();
    other.source_domain = "other.example".into();
    current_admission::admit(&repo, &mut other, "Freight clinic common").await;
    for (run, day) in [("r1", "2026-01-02"), ("r2", "2026-01-03")] {
        repo.db()
            .query("UPDATE type::record('atlas_run', $run) SET started_at = $day")
            .bind(("run", run.to_string()))
            .bind(("day", day.to_string()))
            .await
            .expect("set run day")
            .check()
            .expect("set run day");
    }
    let day = ArticleSearchQuery {
        query: "harbor union".into(),
        day: "2026-01-03".into(),
        ..Default::default()
    };
    let page = article_discovery::search_articles(&repo, &day)
        .await
        .expect("day filter");
    assert_eq!(
        page.hits
            .iter()
            .map(|h| h.article_id.as_str())
            .collect::<Vec<_>>(),
        ["in-day"]
    );
    let category = ArticleSearchQuery {
        query: "harbor union".into(),
        category: "environmental".into(),
        ..Default::default()
    };
    let page = article_discovery::search_articles(&repo, &category)
        .await
        .expect("category filter");
    assert_eq!(
        page.hits
            .iter()
            .map(|h| h.article_id.as_str())
            .collect::<Vec<_>>(),
        ["out-of-day"]
    );
    let source = ArticleSearchQuery {
        query: "harbor union".into(),
        source: "example.com".into(),
        ..Default::default()
    };
    let page = article_discovery::search_articles(&repo, &source)
        .await
        .expect("source filter");
    assert_eq!(page.hits.len(), 2, "other.example is filtered out");
    let run = ArticleSearchQuery {
        query: "harbor union".into(),
        run: "r2".into(),
        ..Default::default()
    };
    let page = article_discovery::search_articles(&repo, &run)
        .await
        .expect("run filter");
    assert_eq!(page.hits.len(), 2);
}

#[tokio::test]
async fn pagination_is_bounded_and_deterministic() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("discovery", "pagination").await;
    atlas::insert_run(repo.db(), "r1", "{}", "{}")
        .await
        .expect("insert run");
    for i in 0..5 {
        admit(
            &repo,
            "r1",
            &format!("a{i}"),
            &format!("Harbor Union item {i}"),
            "Shared description body",
            &["economic", "human"],
            "2026-01-03T00:00:00+00:00",
            "2026-01-03T00:00:00+00:00",
        )
        .await;
    }
    let mut q = ArticleSearchQuery {
        query: "harbor union".into(),
        limit: Some(2),
        ..Default::default()
    };
    let mut seen = Vec::new();
    loop {
        let page = article_discovery::search_articles(&repo, &q)
            .await
            .expect("page");
        assert!(page.hits.len() <= 2);
        seen.extend(page.hits.iter().map(|h| h.article_id.clone()));
        match page.next_cursor {
            Some(cursor) => q.cursor = Some(cursor),
            None => break,
        }
        assert!(seen.len() < 10, "pagination must terminate");
    }
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 5, "every hit is reachable exactly once");
    // A cursor beyond the result set is an honest empty page.
    let q = ArticleSearchQuery {
        query: "harbor union".into(),
        cursor: Some("4000".into()),
        ..Default::default()
    };
    let page = article_discovery::search_articles(&repo, &q)
        .await
        .expect("beyond-end page");
    assert!(page.hits.is_empty() && page.next_cursor.is_none());
}

#[tokio::test]
async fn deleted_articles_leave_the_discovery_index() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("discovery", "deletion").await;
    atlas::insert_run(repo.db(), "r1", "{}", "{}")
        .await
        .expect("insert run");
    admit(
        &repo,
        "r1",
        "gone",
        "Harbor Union secret meeting",
        "Sensitive description",
        &["economic", "human"],
        "2026-01-03T00:00:00+00:00",
        "2026-01-03T00:00:00+00:00",
    )
    .await;
    assert_eq!(search(&repo, "harbor union").await.hits.len(), 1);
    atlas_delete::delete_article(repo.publisher(), "r1", "gone")
        .await
        .expect("delete article");
    assert!(
        search(&repo, "harbor union").await.hits.is_empty(),
        "a deleted article is not discoverable (atomic index maintenance)"
    );
}

#[tokio::test]
async fn re_admission_updates_the_index_and_moves_the_revision() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("discovery", "re_admission").await;
    atlas::insert_run(repo.db(), "r1", "{}", "{}")
        .await
        .expect("insert run");
    admit(
        &repo,
        "r1",
        "story",
        "Harbor Union vote scheduled",
        "Original description",
        &["economic", "human"],
        "2026-01-03T00:00:00+00:00",
        "2026-01-03T00:00:00+00:00",
    )
    .await;
    let before = search(&repo, "harbor union").await;
    assert_eq!(before.hits[0].title, "Harbor Union vote scheduled");
    admit(
        &repo,
        "r1",
        "story",
        "Harbor Union vote cancelled",
        "Revised description",
        &["economic", "human"],
        "2026-01-03T00:00:00+00:00",
        "2026-01-04T00:00:00+00:00",
    )
    .await;
    let after = search(&repo, "harbor union").await;
    assert_eq!(after.hits[0].title, "Harbor Union vote cancelled");
    assert_ne!(
        before.revision, after.revision,
        "a content change moves the fingerprint"
    );
    assert_ne!(before.hits[0].revision, after.hits[0].revision);
}

#[tokio::test]
async fn related_candidates_exclude_the_seed() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("discovery", "related").await;
    atlas::insert_run(repo.db(), "r1", "{}", "{}")
        .await
        .expect("insert run");
    admit(
        &repo,
        "r1",
        "seed",
        "Harbor Union merger confirmed by regulators",
        "The harbor union merger cleared review",
        &["economic", "human"],
        "2026-01-03T00:00:00+00:00",
        "2026-01-03T00:00:00+00:00",
    )
    .await;
    admit(
        &repo,
        "r1",
        "lead",
        "Harbor Union merger faces a second review",
        "A later development",
        &["economic", "human"],
        "2026-01-04T00:00:00+00:00",
        "2026-01-04T00:00:00+00:00",
    )
    .await;
    let seed = atlas::article(repo.db(), "r1", "seed")
        .await
        .expect("read seed")
        .expect("seed exists");
    let related = article_discovery::related_from_seed(&repo, &seed, 10)
        .await
        .expect("related candidates");
    assert_eq!(related.len(), 1);
    assert_eq!(
        related[0].article_id, "lead",
        "the seed never returns as its own lead"
    );
}

#[tokio::test]
async fn unicode_titles_match_folded_and_unfolded_queries() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("discovery", "unicode").await;
    atlas::insert_run(repo.db(), "r1", "{}", "{}")
        .await
        .expect("insert run");
    admit(
        &repo,
        "r1",
        "unicode",
        "München Übereinkunft 東京合意 signed",
        "Folded and unfolded queries must agree",
        &["economic", "human"],
        "2026-01-03T00:00:00+00:00",
        "2026-01-03T00:00:00+00:00",
    )
    .await;
    // The `atlas_article_text` analyzer (class/lowercase/ascii) folds
    // documents and queries with the same pipeline, so the unfolded
    // diacritic form, the ASCII-folded form, the uppercase form and the CJK
    // run all resolve to the same document — the mention-matcher's
    // normalization contract applied to the article index.
    for query in ["münchen", "munchen", "MÜNCHEN", "東京合意"] {
        let page = search(&repo, query).await;
        assert_eq!(
            page.hits
                .iter()
                .map(|hit| hit.article_id.as_str())
                .collect::<Vec<_>>(),
            ["unicode"],
            "query {query:?} must fold like the indexed document"
        );
    }
}

fn actor(id: &str, source: &str, name: &str, aliases: &[&str]) -> Actor {
    Actor {
        id: id.into(),
        source_id: source.into(),
        name: name.into(),
        subtype: "organization".into(),
        aliases: aliases.iter().map(|s| s.to_string()).collect(),
        metadata: BTreeMap::from([("actor_id".into(), source.into())]),
    }
}

#[tokio::test]
async fn unseeded_catalog_reports_unavailable_not_empty_success() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("discovery", "unseeded").await;
    let names = actor_catalog::search_actor_names(&repo, "Harbor Union", 10)
        .await
        .expect("name search");
    assert_eq!(names.diagnostic, "catalog unavailable");
    assert!(names.matches.is_empty() && names.collisions.is_empty());
    let aliases = actor_catalog::search_actor_aliases(&repo, "Harbor Union", 10)
        .await
        .expect("alias search");
    assert_eq!(aliases.diagnostic, "catalog unavailable");
}

#[tokio::test]
async fn name_search_resolves_exact_normalized_and_curated_forms_and_reports_collisions() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("discovery", "names").await;
    let mut trump = actor("donald", "os:Q22686", "Donald Trump", &[]);
    trump.subtype = "person".into();
    actor_catalog::seed_actors(
        &repo,
        vec![
            trump,
            actor("one", "fixture:1", "Harbor Union", &["HU Lite"]),
            actor("two", "fixture:2", "Other Group", &["Harbor Union"]),
            actor("three", "fixture:3", "Harbor Union Ltd", &["harbor union"]),
        ],
        "fixture-v1",
        &AtomicBool::new(false),
        |_| {},
    )
    .await
    .expect("seed catalog");

    // Exact display name, one active canonical owner.
    let found = actor_catalog::search_actor_names(&repo, "Donald Trump", 10)
        .await
        .expect("exact name");
    assert_eq!(found.matches.len(), 1);
    assert_eq!(found.matches[0].reason, "exact name");
    assert_eq!(found.matches[0].catalog_id, "os:Q22686");
    assert_eq!(found.matches[0].subtype, "person");
    assert!(found.diagnostic.is_empty());
    assert!(
        !found.revision.is_empty(),
        "the search pins the catalog fingerprint"
    );

    // The curated short form resolves through the reverse lookup with its own
    // reason, and the backfill (via the matcher) writes the provenance-stamped row.
    let curated = actor_catalog::search_actor_names(&repo, "Trump", 10)
        .await
        .expect("curated form");
    assert_eq!(curated.matches.len(), 1);
    assert_eq!(curated.matches[0].reason, "curated form");
    assert_eq!(curated.matches[0].actor_id, "donald");
    let _ = repo.actor_matcher().await.expect("matcher with backfill");
    let backfilled = actor_catalog::search_actor_aliases(&repo, "Trump", 10)
        .await
        .expect("backfilled alias");
    assert!(backfilled.exact.iter().any(|hit| hit.strength == "curated"));

    // A name two catalog actors share is a collision, never a match —
    // ambiguity is not resolved by ranking.
    let collision = actor_catalog::search_actor_names(&repo, "harbor union", 10)
        .await
        .expect("collision");
    assert!(collision.matches.is_empty());
    assert_eq!(collision.collisions.len(), 3, "{:?}", collision.collisions);

    // Risky short queries and identifiers are guarded with diagnostics.
    let short = actor_catalog::search_actor_names(&repo, "hu", 10)
        .await
        .expect("short query");
    assert_eq!(short.diagnostic, "query too short");
    let identifier = actor_catalog::search_actor_names(&repo, "10.0.0.1", 10)
        .await
        .expect("identifier");
    assert_eq!(identifier.diagnostic, "unsupported type");
}

#[tokio::test]
async fn alias_search_reports_strength_provenance_prefix_candidates_and_ambiguity() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("discovery", "aliases").await;
    actor_catalog::seed_actors(
        &repo,
        vec![
            actor(
                "one",
                "fixture:1",
                "Harbor Union",
                &["Harbor Union", "Shared"],
            ),
            actor("two", "fixture:2", "Other Group", &["Harbor Group"]),
        ],
        "fixture-v1",
        &AtomicBool::new(false),
        |_| {},
    )
    .await
    .expect("seed catalog");

    let found = actor_catalog::search_actor_aliases(&repo, "harbor union", 10)
        .await
        .expect("alias search");
    assert!(found.diagnostic.is_empty());
    assert!(!found.revision.is_empty());
    let exact = found
        .exact
        .iter()
        .find(|hit| hit.normalized == "harbor union")
        .expect("exact alias");
    assert_eq!(
        exact.strength, "catalog",
        "seeded rows carry the CSV provenance"
    );
    assert_eq!(
        exact.provenance.get("actor_id").and_then(|v| v.as_str()),
        Some("fixture:1")
    );
    assert!(!exact.ambiguous);
    assert_eq!(exact.owners, ["one".to_owned()]);

    // Prefix candidates come from the ordered alias index, bounded.
    let prefix = actor_catalog::search_actor_aliases(&repo, "harbor", 10)
        .await
        .expect("prefix search");
    assert!(prefix.exact.is_empty(), "no exact row is named 'harbor'");
    let candidates: Vec<&str> = prefix
        .prefix
        .iter()
        .map(|hit| hit.normalized.as_str())
        .collect();
    assert!(candidates.contains(&"harbor union"), "{candidates:?}");
    assert!(candidates.contains(&"harbor group"), "{candidates:?}");

    // A two-letter query never returns candidates.
    let short = actor_catalog::search_actor_aliases(&repo, "hu", 10)
        .await
        .expect("short query");
    assert_eq!(short.diagnostic, "query too short");
}

#[tokio::test]
async fn retired_owners_leave_the_alias_and_name_indexes() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("discovery", "retirement").await;
    actor_catalog::seed_actors(
        &repo,
        vec![actor("one", "fixture:1", "Harbor Union", &["Harbor Union"])],
        "fixture-v1",
        &AtomicBool::new(false),
        |_| {},
    )
    .await
    .expect("seed catalog");
    let found = actor_catalog::search_actor_names(&repo, "Harbor Union", 10)
        .await
        .expect("name search");
    assert_eq!(found.matches.len(), 1);
    // Retire the actor's memory: the active-lifecycle filter drops it.
    repo.db()
        .query("UPDATE type::record('memory', $m) SET lifecycle = 'retired' RETURN NONE")
        .bind(("m", "one".to_string()))
        .await
        .expect("retire")
        .check()
        .expect("retire");
    let found = actor_catalog::search_actor_names(&repo, "Harbor Union", 10)
        .await
        .expect("name search after retirement");
    assert!(
        found.matches.is_empty() && found.collisions.is_empty(),
        "{found:?}"
    );
    let aliases = actor_catalog::search_actor_aliases(&repo, "harbor union", 10)
        .await
        .expect("alias search after retirement");
    assert!(
        aliases.exact.iter().all(|hit| hit.owners.is_empty()),
        "{:?}",
        aliases.exact
    );
}
