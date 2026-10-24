//! Recon entities and actor receipts on SurrealDB (P3 item 2, part 1).
//! Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::storage::bootstrap::ensure_schema;
use argos_osint_core::storage::entities::*;
use argos_osint_core::storage::tx::{run_tx, RetryPolicy};
use support::*;

fn entity(id: &str, canonical: &str, label: &str) -> ReconEntity {
    ReconEntity {
        id: id.into(),
        kind: "person".into(),
        namespace: id.into(),
        canonical: canonical.into(),
        label: label.into(),
    }
}

#[tokio::test]
async fn recon_entities_insert_or_ignore_and_label_falls_back_to_canonical() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.root("argos", "entities").await;
    ensure_schema(&db).await.unwrap();
    assert!(ensure_recon_entity(&db, &entity("e1", "ada", "Ada L."))
        .await
        .unwrap());
    // Same id: ignored, the first label stays.
    assert!(!ensure_recon_entity(&db, &entity("e1", "ada", "Other"))
        .await
        .unwrap());
    // Same (kind, namespace, canonical) under another id: ignored too.
    let mut clash = entity("e2", "ada", "Ada");
    clash.namespace = "e1".into();
    assert!(!ensure_recon_entity(&db, &clash).await.unwrap());
    assert!(ensure_recon_entity(&db, &entity("e3", "grace", ""))
        .await
        .unwrap());
    assert_eq!(count(&db, "recon_entity").await, 2);

    let labels = run_tx(&db, RetryPolicy::default(), |tx| async move {
        Ok((
            recon_entity_in(&tx, "e1").await?,
            entity_label_in(&tx, "e1").await?,
            entity_label_in(&tx, "e3").await?,
            entity_label_in(&tx, "missing").await?,
        ))
    })
    .await
    .unwrap()
    .value;
    assert_eq!(labels.0, Some(entity("e1", "ada", "Ada L.")));
    assert_eq!(labels.1.as_deref(), Some("Ada L."));
    assert_eq!(labels.2.as_deref(), Some("grace"));
    assert_eq!(labels.3, None);
}

#[tokio::test]
async fn backfill_and_dedup_receipts_upsert_by_their_sqlite_keys() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.root("argos", "receipts").await;
    ensure_schema(&db).await.unwrap();
    run_tx(&db, RetryPolicy::default(), |tx| async move {
        put_backfill_receipt_in(&tx, "insight_claims", "fp1", "r1", "", "t1").await?;
        put_backfill_receipt_in(&tx, "insight_claims", "fp1", "r2", "bound", "t2").await?;
        put_backfill_receipt_in(&tx, "insight_claims", "fp0", "r0", "", "t1").await?;
        put_backfill_receipt_in(&tx, "recon_entities", "fp1", "x", "", "t1").await?;
        mark_separate_in(
            &tx,
            "actor:b",
            "actor:a",
            "{\"basis\":\"explicit split\"}",
            "t1",
        )
        .await?;
        mark_separate_in(&tx, "actor:a", "actor:b", "{}", "t2").await?;
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(
        backfill_receipts(&db, "insight_claims").await.unwrap(),
        vec![("fp0".into(), "r0".into()), ("fp1".into(), "r2".into())]
    );
    assert_eq!(count(&db, "brain_backfill_receipt").await, 3);
    assert_eq!(pair_key("b", "a"), "a:b");
    assert_eq!(
        separate_pair_keys(&db).await.unwrap(),
        vec!["actor:a:actor:b".to_string()]
    );
}
