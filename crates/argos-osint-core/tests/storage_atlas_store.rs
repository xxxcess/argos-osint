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
