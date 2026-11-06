//! Atlas run bookkeeping on SurrealDB (`atlas_store::AtlasStore`,
//! `storage::atlas::save_with_receipt`, `storage::atlas_work`): the same
//! calls give the same results on the legacy store and SurrealDB.
//! Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

#[path = "support/current_admission.rs"]
mod current_admission;
mod support;

use argos_osint_core::atlas_store::AtlasStore;
use argos_osint_core::atlas_work::UnitManifest;
use argos_osint_core::storage::{atlas, atlas_metadata, atlas_work, pipeline_inputs};
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
    let repo = store.repository().unwrap();
    let key = argos_osint_core::storage::codec::encode(&["run-1", "u1"]);
    repo.db().query("UPDATE type::record('atlas_work_unit', $key) SET owner='native-owner', epoch=7, input_revision=3, source_revision='native-source'")
        .bind(("key", key.clone())).await.unwrap().check().unwrap();
    assert!(
        store
            .save_unit_manifest(&unit("run-1", "u1", 1))
            .await
            .is_err(),
        "legacy writer cannot replace a leased native manifest"
    );
    let mut ownership = repo.db().query("SELECT epoch, input_revision, source_revision FROM ONLY type::record('atlas_work_unit', $key)")
        .bind(("key", key.clone())).await.unwrap().check().unwrap();
    let owned: Option<surrealdb::types::Value> = ownership.take(0).unwrap();
    let owned = owned.unwrap().into_json_value();
    assert_eq!(owned["epoch"], 7);
    assert_eq!(owned["input_revision"], 3);
    assert_eq!(owned["source_revision"], "native-source");
    repo.db()
        .query("UPDATE type::record('atlas_work_unit', $key) SET owner=''")
        .bind(("key", key))
        .await
        .unwrap()
        .check()
        .unwrap();
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

fn related_article(id: &str, title: &str) -> argos_osint_core::store::AtlasArticleRow {
    argos_osint_core::store::AtlasArticleRow {
        run_id: "run-r".into(),
        id: id.into(),
        title: title.into(),
        url: format!("https://example.com/{id}"),
        source_name: "Publisher".into(),
        source_domain: "example.com".into(),
        published_at: "2026-10-12".into(),
        provider: "fixture".into(),
        ..Default::default()
    }
}

fn mentions(
    title: &str,
    names: &[(&str, &str)],
) -> Vec<argos_osint_core::atlas_metadata::ArticleMention> {
    let mut mentions = Vec::new();
    for &(name, kind) in names {
        for (start, text) in title.match_indices(name) {
            mentions.push(argos_osint_core::atlas_metadata::ArticleMention {
                type_hint: kind.into(),
                comparison_key: argos_osint_core::atlas_metadata::comparison_key(name),
                field: "title".into(),
                start_byte: start,
                end_byte: start + text.len(),
                original_text: text.into(),
            });
        }
    }
    mentions
}

/// `related_articles::build_on`: the same pairs on both stores, a cached
/// rebuild, and a pair dropped once one of its articles goes.
/// `related_articles_global` rows as (article id, basis) for both scopes.
async fn global_rows(store: &AtlasStore) -> Vec<(String, Vec<String>, bool)> {
    use argos_osint_core::related_global::{related_articles_global_on, RelatedScope};
    let mut out = Vec::new();
    for scope in [RelatedScope::AllRetained, RelatedScope::ThisCycle] {
        let repo = store.repository().expect("SurrealDB store");
        let page = related_articles_global_on(repo, "run-r", "source", scope, None, 20)
            .await
            .unwrap();
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

/// Three current occurrences with complete local metadata. Repeated Ada
/// mentions still count as one entity, so only source/two form a pair.
async fn related_setup(store: &AtlasStore) -> usize {
    use argos_osint_core::atlas_metadata::{ArticleMetadata, MetadataState};
    store.insert_run("run-r", "{}", "{}").await.unwrap();
    let mut source = related_article("source", "Ada and Ada meet Acme");
    let mut one = related_article("one", "Ada mentions Ada");
    let mut two = related_article("two", "Ada meets Acme");
    for a in [&mut source, &mut one, &mut two] {
        current_admission::admit(store.repository().unwrap(), a, "Ballot patrol common").await;
    }
    for article in [&source, &one, &two] {
        let mut metadata = ArticleMetadata::pending(article);
        metadata.entity_state = MetadataState::Complete;
        metadata.mentions = mentions(
            &article.title,
            &[("Ada", "person"), ("Acme", "organization")],
        );
        assert!(store.save_metadata(&metadata).await.unwrap());
    }
    argos_osint_core::related_articles::build_on(store, "run-r")
        .await
        .unwrap()
}

#[tokio::test]
async fn atlas_store_round_trips() {
    let Some(server) = Server::start() else {
        return;
    };
    let surreal = surreal_store(&server, "atlas_store").await;
    let s = script(&surreal).await;
    assert_eq!(s["receipt"], "{\"ok\":1}");
    assert_eq!(s["stage1"][0][1], "failed");
    assert_eq!(s["all"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn related_articles_build_counts_pairs() {
    let Some(server) = Server::start() else {
        return;
    };
    let surreal = surreal_store(&server, "atlas_related").await;
    let s = related_script(&surreal).await;
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
async fn related_articles_global_rows() {
    let Some(server) = Server::start() else {
        return;
    };
    let surreal = surreal_store(&server, "atlas_related_global").await;
    assert_eq!(related_setup(&surreal).await, 1);
    let s = global_rows(&surreal).await;
    assert_eq!(s.len(), 2, "{s:?}");
    assert_eq!(s[0].0, "two");
    assert!(s.iter().all(|row| row.2), "Related leads stay tentative");
    let repo = surreal.repository().unwrap();
    let source = atlas::article(repo.db(), "run-r", "source")
        .await
        .unwrap()
        .unwrap();
    let mut legacy_source = related_article("legacy-source", "Ada meets Acme");
    legacy_source.url = source.url.clone();
    let mut legacy_target = related_article("legacy-target", "Ada meets Acme");
    for article in [&mut legacy_source, &mut legacy_target] {
        // Modern-looking tags and valid spans do not grant native admission.
        article.category = source.category.clone();
        article.categories = source.categories.clone();
        surreal.upsert_article(article).await.unwrap();
        let mut metadata = argos_osint_core::atlas_metadata::ArticleMetadata::pending(article);
        metadata.entity_state = argos_osint_core::atlas_metadata::MetadataState::Complete;
        metadata.mentions = mentions(
            &article.title,
            &[("Ada", "person"), ("Acme", "organization")],
        );
        assert!(surreal.save_metadata(&metadata).await.unwrap());
    }
    let pair = atlas_metadata::related_pairs(repo.db(), "run-r", "source")
        .await
        .unwrap()
        .pop()
        .unwrap();
    // Simulate retained legacy pairs whose text/category hashes still match.
    // Neither same-URL source aliases nor forged positive targets are current.
    for left in [&source, &legacy_source] {
        let legacy_revision = |article: &argos_osint_core::store::AtlasArticleRow| {
            argos_osint_core::nlp::cache::hash(&format!(
                "{}:{}",
                argos_osint_core::atlas_metadata::input_hash(article),
                serde_json::to_string(&article.categories).unwrap()
            ))
        };
        repo.db().query("CREATE type::record('atlas_related_article', $key) CONTENT {run_id:$run,left_id:$left,right_id:$right,left_revision:$lr,right_revision:$rr,shared_json:$shared,match_version:$version,named_count:2,shared_count:2,rarity:1.0} RETURN NONE")
            .bind(("key", argos_osint_core::storage::codec::encode(&["run-r", &left.id, &legacy_target.id])))
            .bind(("run", "run-r")).bind(("left", left.id.clone())).bind(("right", legacy_target.id.clone()))
            .bind(("lr", legacy_revision(left))).bind(("rr", legacy_revision(&legacy_target)))
            .bind(("shared", pair.shared_json.clone())).bind(("version", argos_osint_core::related_articles::MATCH_VERSION))
            .await.unwrap().check().unwrap();
    }
    use argos_osint_core::related_global::{related_articles_global_on, RelatedScope};
    for scope in [RelatedScope::AllRetained, RelatedScope::ThisCycle] {
        let page = related_articles_global_on(repo, "run-r", "source", scope, None, 1)
            .await
            .unwrap();
        assert_eq!(
            page.total, 1,
            "legacy targets are excluded before pagination"
        );
        assert_eq!(page.rows[0].article.id, "two");
        assert!(page.next_cursor.is_none());
        let legacy = related_articles_global_on(repo, "run-r", &legacy_source.id, scope, None, 1)
            .await
            .unwrap();
        assert_eq!(
            legacy.total, 0,
            "legacy seed cannot borrow a current URL alias"
        );
    }
    assert!(
        argos_osint_core::intel_service::article(repo, "run-r", &legacy_source.id)
            .await
            .unwrap()
            .is_some(),
        "legacy detail remains readable"
    );
    let public = argos_osint_core::intel_service::related(repo, &source, None, 1)
        .await
        .unwrap();
    assert_eq!(public["total"], 1);
    assert_eq!(public["rows"][0]["article"]["id"], "two");
    assert_eq!(public["rows"][0]["tentative"], true);
    let seed = argos_osint_core::brain_topology::TopologySeed::Article {
        run_id: source.run_id.clone(),
        article_id: source.id.clone(),
        revision: argos_osint_core::atlas_metadata::input_hash(&source),
        originating_memory: None,
    };
    let topology =
        argos_osint_core::storage::topology::topology_snapshot(repo, &seed, &Default::default())
            .await
            .unwrap();
    assert_eq!(
        topology
            .edges
            .iter()
            .filter(|edge| edge.kind == argos_osint_core::brain_topology::TopologyEdgeKind::Related)
            .count(),
        1,
        "topology validates the native fingerprint stored alongside legacy text revisions"
    );
    // Retain the pair while changing its native annotation revision, as after
    // interruption between metadata publication and derivative refresh.
    let mut metadata = surreal.metadata("run-r", "source").await.unwrap().unwrap();
    metadata.entity_version.push_str("-revised");
    repo.db().query("UPDATE type::record('atlas_article_metadata', $key) SET metadata_json=$json RETURN NONE")
        .bind(("key", argos_osint_core::storage::codec::encode(&["run-r", "source"])))
        .bind(("json", serde_json::to_string(&metadata).unwrap()))
        .await.unwrap().check().unwrap();
    for scope in [RelatedScope::AllRetained, RelatedScope::ThisCycle] {
        assert_eq!(
            related_articles_global_on(repo, "run-r", "source", scope, None, 1)
                .await
                .unwrap()
                .total,
            0,
            "stale native pair is not a current lead"
        );
    }
    assert_eq!(
        argos_osint_core::intel_service::related(repo, &source, None, 1)
            .await
            .unwrap()["total"],
        0
    );
    let topology =
        argos_osint_core::storage::topology::topology_snapshot(repo, &seed, &Default::default())
            .await
            .unwrap();
    assert!(
        !topology
            .edges
            .iter()
            .any(|edge| edge.kind == argos_osint_core::brain_topology::TopologyEdgeKind::Related),
        "stale native pair is hidden from topology too"
    );
}
