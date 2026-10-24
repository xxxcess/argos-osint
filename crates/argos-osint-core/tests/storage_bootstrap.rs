//! Schema bootstrap (migration phase 3, step 5) against a real SurrealDB
//! 3.3.4 disk server or the embedded RocksDB engine (see `support`). Run
//! through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::storage::bootstrap::{self, ensure_schema, SCHEMA_VERSION};
use argos_osint_core::storage::tx::TxError;
use support::*;

#[tokio::test]
async fn ensure_schema_defines_everything_once_and_refuses_newer_databases() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.raw_root("argos", "bootstrap").await;
    let first = ensure_schema(&db).await.unwrap();
    assert_eq!((first.previous, first.version), (0, SCHEMA_VERSION));
    let mut r = db.query("INFO FOR DB").await.unwrap().check().unwrap();
    let info: Option<serde_json::Value> = r
        .take::<Option<surrealdb::types::Value>>(0usize)
        .unwrap()
        .map(|v| v.into_json_value());
    let info = info.expect("db info").to_string();
    for table in [
        "counter",
        "index_change",
        "memory",
        "brain_graph_edge",
        "memory_embeddings",
        "process",
        "job",
        "task",
        "insight_claim",
        "atlas_article",
        "atlas_article_metadata",
        "intel_investigation",
        "brain_actor",
        "brain_actor_alias",
        "brain_actor_external_ref",
        "insight_refresh_task",
        "recon_thread",
        "recon_message",
        "recon_message_memory",
        "brain_memory_endpoint",
        "brain_memory_actor_ref",
        "article_body",
        "nlp_artifact",
        "nlp_source_ref",
        "app_state",
        "recon_entity",
        "brain_backfill_receipt",
        "brain_actor_dedup_receipt",
        "provider_quota",
        "credit_reservation",
        "telemetry_meta",
        "investigation_task",
        "investigation_task_dep",
        "investigation_passage",
        "investigation_assessment",
        "atlas_work_unit",
        "atlas_article_index",
        "derived_summary",
        "summary_flush_request",
        "brain_scope_request",
        "storage_meta",
    ] {
        assert!(info.contains(&format!("\"{table}\"")), "missing {table}");
    }
    // Idempotent re-run.
    let again = ensure_schema(&db).await.unwrap();
    assert_eq!(again.previous, SCHEMA_VERSION);
    assert_eq!(bootstrap::schemas().len(), 33);

    // A newer database is refused and left untouched.
    db.query("UPDATE storage_meta:schema SET version = 99")
        .await
        .unwrap()
        .check()
        .unwrap();
    match ensure_schema(&db).await {
        Err(TxError::Abort(m)) => assert!(m.contains("newer than this Argos build"), "{m}"),
        other => panic!("expected refusal, got {other:?}"),
    }
    let mut r = db
        .query("SELECT VALUE version FROM storage_meta:schema")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(r.take::<Vec<i64>>(0usize).unwrap(), vec![99]);
}
