//! Atlas memory-phase reads on SurrealDB (`storage::atlas_memory`): stored
//! publication receipts and their verification match SQLite for a run with
//! no publication and for a receipt whose memories are gone.
//! Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::atlas_store::AtlasStore;
use argos_osint_core::storage::bootstrap;
use argos_osint_core::store::PublicationReceipt;
use support::*;

fn receipt() -> PublicationReceipt {
    PublicationReceipt {
        run_id: "run-m".into(),
        revision: "r1".into(),
        input_claims: 1,
        accepted_claims: 1,
        created: 1,
        created_memory_ids: vec!["mem-gone".into()],
        claim_memory_ids: vec![("fp-gone".into(), "mem-gone".into())],
        affected_memory_ids: vec!["mem-gone".into()],
        ..Default::default()
    }
}

async fn script(store: &AtlasStore) -> serde_json::Value {
    let missing = store.publication_receipt("run-m").await.unwrap();
    let verified = store.verify_publication(&receipt()).await.unwrap();
    serde_json::json!({
        "receipt": missing.is_some(),
        "missing": verified.missing_claim_memories,
        "broken": verified.broken_source_links,
        "reconciled": verified.reconciled,
        "missing_rows": verified.coverage.missing_rows,
        "memories_ok": verified.memories_ok(),
    })
}

#[tokio::test]
async fn publication_verification_reports_gaps() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "atlas_memory").await;
    bootstrap::ensure_schema(repo.db()).await.unwrap();
    let surreal = AtlasStore::from_repository(repo);

    let s = script(&surreal).await;
    assert_eq!(s["receipt"], false);
    assert_eq!(s["missing"][0], "mem-gone");
    assert_eq!(s["broken"][0], "fp-gone");
    assert_eq!(s["memories_ok"], false);
}
