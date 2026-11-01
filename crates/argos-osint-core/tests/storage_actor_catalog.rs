//! Offline catalog seed/retry and article association contracts.
#![cfg(feature = "surreal")]
mod support;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use argos_osint_core::actor_catalog::Actor;
use argos_osint_core::atlas_actor_index::surreal;
use argos_osint_core::storage::tx::{run_tx, RetryPolicy};
use argos_osint_core::storage::{actor_bind, actor_catalog};
use argos_osint_core::store::AtlasArticleRow;

fn actors(count: usize) -> Vec<Actor> {
    (0..count)
        .map(|i| Actor {
            id: format!("fixture-actor-{i}"),
            source_id: format!("fixture:{i}"),
            name: format!("Fixture Union {i}"),
            subtype: "organization".into(),
            aliases: vec![
                format!("Fixture Union {i}"),
                format!("Harbor Alias {i}"),
                "Shared Alias".into(),
            ],
            metadata: BTreeMap::from([("license".into(), "fixture-only".into())]),
        })
        .collect()
}

#[tokio::test]
async fn partial_seed_retries_without_duplicates_and_only_complete_receipts_hide_action() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("catalog", "retry").await;
    assert!(
        actor_catalog::status(repo.db())
            .await
            .expect("initial status")
            .needs_seed
    );
    let cancel = AtomicBool::new(false);
    let result = actor_catalog::seed_actors(&repo, actors(129), "fixture-v1", &cancel, |p| {
        if p.processed == 128 {
            cancel.store(true, Ordering::Release);
        }
    })
    .await;
    assert!(result.is_err());
    assert!(
        actor_catalog::status(repo.db())
            .await
            .expect("partial status")
            .needs_seed
    );
    cancel.store(false, Ordering::Release);
    let progress = actor_catalog::seed_actors(&repo, actors(129), "fixture-v1", &cancel, |_| {})
        .await
        .expect("retry seed");
    assert!(progress.complete);
    assert_eq!(progress.processed, 129);
    assert!(
        !actor_catalog::status(repo.db())
            .await
            .expect("complete status")
            .needs_seed
    );
    actor_catalog::seed_actors(&repo, actors(129), "fixture-v1", &cancel, |_| {})
        .await
        .expect("repeat seed");
    let mut r = repo.db().query("RETURN count(SELECT id FROM brain_actor); RETURN count(SELECT id FROM memory WHERE memory_kind = 'actor'); RETURN count(SELECT id FROM brain_actor_alias);").await.expect("counts").check().expect("valid counts");
    assert_eq!(r.take::<Option<usize>>(0).expect("actors"), Some(129));
    assert_eq!(r.take::<Option<usize>>(1).expect("memories"), Some(129));
    assert_eq!(r.take::<Option<usize>>(2).expect("aliases"), Some(387));
    repo.db()
        .query("DELETE brain_actor")
        .await
        .expect("empty actors")
        .check()
        .expect("delete");
    assert!(
        actor_catalog::status(repo.db())
            .await
            .expect("empty status")
            .needs_seed
    );
}

#[tokio::test]
async fn articles_match_catalog_aliases_without_nlp_and_never_create_unknown_identities() {
    let Some(server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("catalog", "matching").await;
    actor_catalog::seed_actors(
        &repo,
        actors(2),
        "fixture-v1",
        &AtomicBool::new(false),
        |_| {},
    )
    .await
    .expect("seed");
    let article = AtlasArticleRow {
        id: "article".into(),
        run_id: "run".into(),
        title: "Harbor Alias 0 meets Unknown Person and Shared Alias".into(),
        ..Default::default()
    };
    argos_osint_core::storage::atlas::upsert_article(repo.db(), &article)
        .await
        .expect("article");
    let matched = actor_catalog::article_actors(&repo, &article)
        .await
        .expect("catalog projection")
        .expect("catalog enabled");
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].id, "fixture-actor-0");
    surreal::index_occurrence(&repo, &article, None, "revision", "2026-11-01T00:00:00Z")
        .await
        .expect("index without NLP");
    let mut r = repo.db().query("SELECT VALUE actor_id FROM brain_actor_mention; RETURN count(SELECT id FROM brain_actor);").await.expect("mentions").check().expect("valid mentions");
    assert_eq!(
        r.take::<Vec<String>>(0).expect("actors"),
        vec!["fixture-actor-0"]
    );
    assert_eq!(r.take::<Option<usize>>(1).expect("count"), Some(2));
    run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        assert_eq!(
            actor_catalog::lookup_in(&tx, "HARBOR ALIAS 0").await?,
            Some("fixture-actor-0".into())
        );
        assert!(actor_catalog::lookup_in(&tx, "Shared Alias")
            .await?
            .is_none());
        let req = argos_osint_core::brain_actors::BindRequest {
            name: "Unknown Person",
            subtype: argos_osint_core::memory_metadata::ActorSubtype::Person,
            scope: "article:fixture",
            identifiers: &[],
            external: None,
            provenance: serde_json::json!({}),
        };
        assert!(
            actor_bind::bind_optional_in(&tx, &req, "2026-11-01T00:00:00Z")
                .await?
                .is_none()
        );
        Ok(())
    })
    .await
    .expect("binding without new actors");
    run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        argos_osint_core::storage::lifecycle::retire_origin_in(&tx, "article", "run").await?;
        Ok(())
    })
    .await
    .expect("retire article origin");
    let mut r = repo
        .db()
        .query("SELECT VALUE lifecycle FROM ONLY type::record('memory', 'fixture-actor-0')")
        .await
        .expect("actor lifecycle")
        .check()
        .expect("read lifecycle");
    assert_eq!(
        r.take::<Option<String>>(0).expect("lifecycle"),
        Some("active".into())
    );
    assert!(
        argos_osint_core::storage::actor_dedup::candidates(&repo, 10)
            .await
            .expect("identity candidates")
            .is_empty()
    );
}
