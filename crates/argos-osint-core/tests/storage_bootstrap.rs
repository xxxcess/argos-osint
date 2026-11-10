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
        "nlp_feature",
        "derived_summary",
        "summary_flush_request",
        "brain_scope_request",
        "atlas_reclassification_job",
        "classifier_calibration_job",
        "storage_meta",
        "model_provider_pacer",
        "brief_publication_work",
    ] {
        assert!(info.contains(&format!("\"{table}\"")), "missing {table}");
    }
    // Idempotent re-run.
    let again = ensure_schema(&db).await.unwrap();
    assert_eq!(again.previous, SCHEMA_VERSION);
    assert_eq!(bootstrap::schemas().len(), 47);

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

/// The atlas module schema as schema version 1 wrote it (`main@bfe994f4`):
/// no `categories`/`admitted_at` fields, no `categories.*` membership index,
/// no `atlas_article_text` analyzer, no article full-text indexes and no
/// admission/rejection tables. A real v1 home has exactly these atlas
/// definitions plus data rows.
const V1_ATLAS_SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS atlas_run SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS state ON atlas_run TYPE string;
DEFINE FIELD IF NOT EXISTS phase ON atlas_run TYPE int DEFAULT 1;
DEFINE FIELD IF NOT EXISTS cursor_json ON atlas_run TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS stats_json ON atlas_run TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS note ON atlas_run TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS started_at ON atlas_run TYPE string;
DEFINE FIELD IF NOT EXISTS finished_at ON atlas_run TYPE string DEFAULT '';
DEFINE INDEX IF NOT EXISTS atlas_run_started ON atlas_run FIELDS started_at;
DEFINE INDEX IF NOT EXISTS atlas_run_state ON atlas_run FIELDS state;
DEFINE TABLE IF NOT EXISTS atlas_article SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS run_id ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS article_id ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS title ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS description ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS url ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS country ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS source_name ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS source_domain ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS published_at ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS provider ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS temperature ON atlas_article TYPE float;
DEFINE FIELD IF NOT EXISTS category ON atlas_article TYPE string DEFAULT 'unk';
DEFINE FIELD IF NOT EXISTS seen_at ON atlas_article TYPE string;
DEFINE FIELD IF NOT EXISTS author ON atlas_article TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS image_url ON atlas_article TYPE string DEFAULT '';
DEFINE INDEX IF NOT EXISTS atlas_article_run ON atlas_article FIELDS run_id;
DEFINE INDEX IF NOT EXISTS atlas_article_id ON atlas_article FIELDS article_id;
DEFINE INDEX IF NOT EXISTS atlas_article_bulletin ON atlas_article FIELDS category, published_at, seen_at, article_id, run_id;
DEFINE INDEX IF NOT EXISTS atlas_article_bulletin_all ON atlas_article FIELDS published_at, seen_at, article_id, run_id;
DEFINE TABLE IF NOT EXISTS atlas_quota SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS used ON atlas_quota TYPE int DEFAULT 0;
DEFINE TABLE IF NOT EXISTS atlas_checkpoint SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS run_id ON atlas_checkpoint TYPE string;
DEFINE FIELD IF NOT EXISTS revision ON atlas_checkpoint TYPE string;
DEFINE FIELD IF NOT EXISTS payload_json ON atlas_checkpoint TYPE string;
DEFINE FIELD IF NOT EXISTS accepted ON atlas_checkpoint TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS rejected ON atlas_checkpoint TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS created_at ON atlas_checkpoint TYPE string;
DEFINE INDEX IF NOT EXISTS atlas_checkpoint_run ON atlas_checkpoint FIELDS run_id;
DEFINE TABLE IF NOT EXISTS atlas_repair SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS status ON atlas_repair TYPE string;
DEFINE FIELD IF NOT EXISTS detail_json ON atlas_repair TYPE string DEFAULT '{}';
DEFINE FIELD IF NOT EXISTS job_id ON atlas_repair TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS updated_at ON atlas_repair TYPE string;";

/// Opening (upgrading) a pre-v2 store in place: the customer regression. The
/// store is built as version 1 — every current module schema except the
/// atlas module, applied with its v1 text ([`V1_ATLAS_SCHEMA`]) — and holds
/// a legacy article row. `ensure_schema` must add the v2 definitions without
/// error and leave the full-text indexes usable over the existing rows;
/// re-running it must stay a no-op that keeps the data.
#[tokio::test]
async fn ensure_schema_upgrades_a_pre_v2_store_with_existing_atlas_rows() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.raw_root("argos", "bootstrap_v1_upgrade").await;

    // The pre-v2 store: v1 atlas definitions, current definitions elsewhere.
    let mut v1_sql = String::new();
    for (name, schema) in bootstrap::schemas() {
        let schema = if name == "atlas" {
            V1_ATLAS_SCHEMA
        } else {
            schema
        };
        v1_sql.push_str(schema.trim().trim_end_matches(';'));
        v1_sql.push_str(";\n");
    }
    db.query(v1_sql.as_str()).await.unwrap().check().unwrap();

    // A legacy occurrence, written before v2 existed (no `categories` array,
    // no `admitted_at`).
    db.query(
        "CREATE atlas_article:legacy1 SET \
         run_id = 'run-v1', article_id = 'a1', \
         title = 'Harbor helicopter drill', description = 'Coastguard exercise', \
         url = 'https://example.test/a1', country = 'us', \
         source_name = 'Example', source_domain = 'example.test', \
         published_at = '2026-10-01T00:00:00Z', provider = 'gdacs', \
         temperature = 0.5, category = 'unk', seen_at = '2026-10-01T01:00:00Z', \
         author = '', image_url = '' RETURN NONE",
    )
    .await
    .unwrap()
    .check()
    .unwrap();
    db.query("UPSERT storage_meta:schema SET version = 1, applied_at = '2026-10-01T00:00:00Z'")
        .await
        .unwrap()
        .check()
        .unwrap();

    // The customer's failing step: open the store, which applies v2.
    let out = ensure_schema(&db)
        .await
        .expect("opening a pre-v2 store must upgrade it in place");
    assert_eq!((out.previous, out.version), (1, SCHEMA_VERSION));

    // The added full-text indexes serve search over the legacy rows.
    let mut r = db
        .query("SELECT VALUE article_id FROM atlas_article WHERE title @1@ 'helicopter'")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(r.take::<Vec<String>>(0usize).unwrap(), vec!["a1"]);

    // Re-running the upgrade is a no-op that keeps the data usable.
    let again = ensure_schema(&db).await.expect("re-run must stay a no-op");
    assert_eq!(
        (again.previous, again.version),
        (SCHEMA_VERSION, SCHEMA_VERSION)
    );
    let mut r = db
        .query("SELECT VALUE article_id FROM atlas_article WHERE title @1@ 'helicopter'")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(r.take::<Vec<String>>(0usize).unwrap(), vec!["a1"]);
}
