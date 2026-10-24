//! Atlas run bookkeeping on SurrealDB (`atlas_store::AtlasStore`,
//! `storage::atlas::save_with_receipt`, `storage::atlas_work`): the same
//! calls give the same results on SQLite and SurrealDB.
//! Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::atlas_store::AtlasStore;
use argos_osint_core::atlas_work::UnitManifest;
use argos_osint_core::storage::{atlas, atlas_metadata, atlas_work, pipeline_inputs};
use argos_osint_core::store::Store;
use support::*;

fn unit(run: &str, unit_id: &str, stage: i32) -> UnitManifest {
    UnitManifest {
        run_id: run.into(),
        unit_id: unit_id.into(),
        stage,
        input_ids: vec!["a1".into()],
        input_rev: "rev".into(),
        contract_version: "c1".into(),
        dependency_ids: vec![],
        is_required: false,
        output_refs: vec![format!("metadata:{run}:a1")],
        effective_model: String::new(),
        attempt_history: vec![],
        next_eligible_at: None,
        terminal_reason: None,
        output_json: "{}".into(),
        disposition: "success".into(),
        receipt_json: "{}".into(),
    }
}

async fn surreal_store(server: &Server, db: &str) -> AtlasStore {
    let repo = server.repo("argos", db).await;
    atlas::apply_schema(repo.db()).await.unwrap();
    atlas_metadata::apply_schema(repo.db()).await.unwrap();
    atlas_work::apply_schema(repo.db()).await.unwrap();
    repo.db()
        .query(pipeline_inputs::SCHEMA)
        .await
        .unwrap()
        .check()
        .unwrap();
    AtlasStore::from_repository(repo)
}

/// The same script against either backend; returns a comparable summary.
async fn script(store: &AtlasStore) -> serde_json::Value {
    store.insert_run("run-1", "{}", "{}").await.unwrap();
    store
        .save_with_receipt(
            "run-1",
            "atlas-receipt:run-1",
            "{\"ok\":1}",
            "{\"leg\":\"x\"}",
            "{\"n\":2}",
        )
        .await
        .unwrap();
    let run = store.latest_run().await.unwrap().unwrap();

    store.app_state_set("k", "v").await.unwrap();
    let before = store.app_state_get("k").await.unwrap();
    store.app_state_delete("k").await.unwrap();
    let after = store.app_state_get("k").await.unwrap();
    // Empty keys and ids never reach a record id.
    assert_eq!(store.app_state_get("").await.unwrap(), None);
    store.app_state_delete("").await.unwrap();
    assert!(store.article("", "a1").await.unwrap().is_none());

    store
        .save_unit_manifest(&unit("run-1", "u2", 2))
        .await
        .unwrap();
    store
        .save_unit_manifest(&unit("run-1", "u1", 1))
        .await
        .unwrap();
    let mut replaced = unit("run-1", "u1", 1);
    replaced.disposition = "failed".into();
    store.save_unit_manifest(&replaced).await.unwrap();
    let stage1 = store.unit_manifests("run-1", 1).await.unwrap();
    let all = store.all_unit_manifests("run-1").await.unwrap();

    serde_json::json!({
        "run": [run.id, run.cursor_json, run.stats_json],
        "receipt": store.app_state_get("atlas-receipt:run-1").await.unwrap(),
        "app_state": [before, after],
        "stage1": stage1.iter().map(|u| (u.unit_id.clone(), u.disposition.clone())).collect::<Vec<_>>(),
        "all": all.iter().map(|u| (u.stage, u.unit_id.clone())).collect::<Vec<_>>(),
    })
}

#[tokio::test]
async fn atlas_store_matches_sqlite() {
    let Some(server) = Server::start() else {
        return;
    };
    let surreal = surreal_store(&server, "atlas_store").await;
    assert!(surreal.is_surreal());
    let dir = tempfile::tempdir().unwrap();
    let sqlite = AtlasStore::from_sqlite(Store::open(&dir.path().join("argos.db")).unwrap());

    let s = script(&surreal).await;
    let q = script(&sqlite).await;
    assert_eq!(s, q);
    assert_eq!(s["receipt"], "{\"ok\":1}");
    assert_eq!(s["stage1"][0][1], "failed");
    assert_eq!(s["all"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn atlas_store_surreal_refuses_sqlite_only_work() {
    let Some(server) = Server::start() else {
        return;
    };
    let store = surreal_store(&server, "atlas_store_sqlite_only").await;
    let err = store
        .sqlite("Atlas actor index", |_| Ok(()))
        .unwrap_err()
        .to_string();
    assert!(err.contains("Atlas actor index"), "{err}");
}

#[tokio::test]
async fn work_units_reject_empty_ids() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "atlas_work_empty").await;
    atlas_work::apply_schema(repo.db()).await.unwrap();
    assert!(atlas_work::save_unit_manifest(repo.db(), &unit("", "u", 1))
        .await
        .is_err());
    assert!(atlas_work::save_unit_manifest(repo.db(), &unit("r", "", 1))
        .await
        .is_err());
    assert!(atlas_work::all_unit_manifests(repo.db(), "r")
        .await
        .unwrap()
        .is_empty());
}

fn related_article(
    id: &str,
    title: &str,
    category: &str,
) -> argos_osint_core::store::AtlasArticleRow {
    argos_osint_core::store::AtlasArticleRow {
        run_id: "run-r".into(),
        id: id.into(),
        title: title.into(),
        url: format!("https://example.com/{id}"),
        source_name: "Publisher".into(),
        source_domain: "example.com".into(),
        published_at: "2026-10-12".into(),
        provider: "fixture".into(),
        category: category.into(),
        ..Default::default()
    }
}

fn mention(
    title: &str,
    name: &str,
    kind: &str,
) -> argos_osint_core::atlas_metadata::ArticleMention {
    let start = title.find(name).unwrap();
    argos_osint_core::atlas_metadata::ArticleMention {
        type_hint: kind.into(),
        comparison_key: argos_osint_core::atlas_metadata::comparison_key(name),
        field: "title".into(),
        start_byte: start,
        end_byte: start + name.len(),
        original_text: name.into(),
    }
}

/// `related_articles::build_on`: the same pairs on both stores, a cached
/// rebuild, and a pair dropped once one of its articles goes.
/// `related_articles_global` rows as (article id, basis) for both scopes.
async fn global_rows(store: &AtlasStore) -> Vec<(String, Vec<String>, bool)> {
    use argos_osint_core::related_global::{related_articles_global_on, RelatedScope};
    let mut out = Vec::new();
    for scope in [RelatedScope::AllRetained, RelatedScope::ThisCycle] {
        let page = match store.repository() {
            Some(repo) => related_articles_global_on(repo, "run-r", "source", scope, None, 20)
                .await
                .unwrap(),
            None => store
                .sqlite("global", |s| {
                    s.related_articles_global("run-r", "source", scope, None, 20)
                })
                .unwrap(),
        };
        out.extend(
            page.rows
                .into_iter()
                .map(|r| (r.article.id, r.basis, r.tentative)),
        );
    }
    out
}

async fn related_script(store: &AtlasStore) -> Vec<usize> {
    let first = related_setup(store).await;
    let build = || argos_osint_core::related_articles::build_on(store, "run-r");
    let cached = build().await.unwrap();
    store.delete_article("run-r", "two").await.unwrap();
    let after_delete = build().await.unwrap();
    vec![first, cached, after_delete]
}

/// Three articles in one run with complete metadata on the source; returns
/// the first build's pair count.
async fn related_setup(store: &AtlasStore) -> usize {
    use argos_osint_core::atlas_metadata::{ArticleMetadata, MetadataState};
    store.insert_run("run-r", "{}", "{}").await.unwrap();
    let source = related_article("source", "Ada and Ada meet Acme", "economic");
    let one = related_article("one", "Ada mentions Ada", "unk");
    let two = related_article("two", "Ada meets Acme", "unk");
    for a in [&source, &one, &two] {
        store.upsert_article(a).await.unwrap();
    }
    let mut m = ArticleMetadata::pending(&source);
    m.entity_state = MetadataState::Complete;
    m.mentions.push(mention(&source.title, "Ada", "person"));
    m.mentions
        .push(mention(&source.title, "Acme", "organization"));
    assert!(store.save_metadata(&m).await.unwrap());
    argos_osint_core::related_articles::build_on(store, "run-r")
        .await
        .unwrap()
}

#[tokio::test]
async fn related_articles_build_matches_sqlite() {
    let Some(server) = Server::start() else {
        return;
    };
    let surreal = surreal_store(&server, "atlas_related").await;
    let dir = tempfile::tempdir().unwrap();
    let sqlite = AtlasStore::from_sqlite(Store::open(&dir.path().join("argos.db")).unwrap());
    let s = related_script(&surreal).await;
    assert_eq!(s, related_script(&sqlite).await);
    assert_eq!(s, vec![1, 1, 0]);
    let repo = surreal.repository().unwrap();
    assert_eq!(
        atlas_metadata::related_count(repo.db(), "run-r")
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn related_articles_global_matches_sqlite() {
    let Some(server) = Server::start() else {
        return;
    };
    let surreal = surreal_store(&server, "atlas_related_global").await;
    let dir = tempfile::tempdir().unwrap();
    let sqlite = AtlasStore::from_sqlite(Store::open(&dir.path().join("argos.db")).unwrap());
    assert_eq!(related_setup(&surreal).await, 1);
    assert_eq!(related_setup(&sqlite).await, 1);
    let s = global_rows(&surreal).await;
    assert_eq!(s, global_rows(&sqlite).await);
    assert_eq!(s.len(), 2, "{s:?}");
    assert_eq!(s[0].0, "two");
}
