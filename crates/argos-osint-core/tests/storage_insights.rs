//! Insight publication tests (migration phase 3) against a real SurrealDB
//! 3.3.4 disk server or the embedded RocksDB engine (see `support`).
//! Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::storage::insight::{self, commit_article_insight_replacement};
use argos_osint_core::storage::publisher::Publisher;
use argos_osint_core::storage::tx::TxError;
use argos_osint_core::storage::{memory, outbox};
use argos_osint_core::store::AtlasInsightClaim;
use support::*;
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

async fn fresh(server: &Server, name: &str) -> (Surreal<Any>, Publisher) {
    let db = server.root("argos", name).await;
    outbox::apply_schema(&db).await.unwrap();
    memory::apply_schema(&db).await.unwrap();
    insight::apply_schema(&db).await.unwrap();
    (
        db.clone(),
        Publisher::spawn(server.root("argos", name).await),
    )
}

async fn scalar<T: SurrealValue>(db: &Surreal<Any>, sql: &str) -> T {
    let mut r = db.query(sql).await.unwrap().check().unwrap();
    r.take::<Option<T>>(0usize).unwrap().expect("NONE")
}

fn claim(object: &str, article: &str) -> AtlasInsightClaim {
    AtlasInsightClaim {
        fingerprint: String::new(),
        entity: "Acme".into(),
        namespace: "org".into(),
        predicate: "located_in".into(),
        object: object.into(),
        topic: "geo".into(),
        claim: format!("Acme is in {object}."),
        classification: "fact".into(),
        confidence: 0.7,
        article_id: article.into(),
        source_url: "https://example.test/a".into(),
        published_at: "2026-10-01".into(),
        reliability: "B".into(),
        info_credibility: 2,
        admiralty: "B2".into(),
        rsp_status: "ok".into(),
    }
}

async fn counts(db: &Surreal<Any>) -> (i64, i64, i64, i64) {
    (
        scalar(db, "RETURN count(SELECT id FROM insight_claim)").await,
        scalar(db, "RETURN count(SELECT id FROM insight_source)").await,
        scalar(db, "RETURN count(SELECT id FROM memory)").await,
        scalar(db, "RETURN count(SELECT id FROM insight_relation)").await,
    )
}

#[tokio::test]
async fn replacement_swaps_claims_and_queues_index_work_atomically() {
    let Some(server) = Server::start() else {
        return;
    };
    let (db, p) = fresh(&server, "insight_replace").await;
    let a = vec![claim("Paris", "art1"), claim("Lyon", "art1")];
    let out = commit_article_insight_replacement(&p, "run1", "art1", a, vec![])
        .await
        .unwrap();
    assert_eq!((out.claims, out.created.len()), (2, 2));
    let fps = out.created.clone();
    let rel = vec![(fps[0].clone(), fps[1].clone(), "contradicts".into())];
    // Same claims again plus a relation. As in SQLite, the article's old
    // sources go first, so its claims are orphaned and re-created.
    let out = commit_article_insight_replacement(
        &p,
        "run1",
        "art1",
        vec![claim("Paris", "art1"), claim("Lyon", "art1")],
        rel,
    )
    .await
    .unwrap();
    assert_eq!((out.claims, out.created.len()), (2, 2));
    assert_eq!(counts(&db).await, (2, 2, 2, 1));
    let pending: i64 = scalar(&db, "RETURN count(SELECT id FROM index_work)").await;
    assert!(pending >= 2, "index work for created memories is queued");

    // Replace with one claim: Lyon is orphaned, its memory and relation go.
    let out = commit_article_insight_replacement(
        &p,
        "run1",
        "art1",
        vec![claim("Paris", "art1")],
        vec![],
    )
    .await
    .unwrap();
    assert_eq!(out.claims, 1);
    assert_eq!(counts(&db).await, (1, 1, 1, 0));
}

#[tokio::test]
async fn a_user_edited_orphan_is_preserved() {
    let Some(server) = Server::start() else {
        return;
    };
    let (db, p) = fresh(&server, "insight_user_edit").await;
    commit_article_insight_replacement(&p, "r", "art", vec![claim("Oslo", "art")], vec![])
        .await
        .unwrap();
    db.query("CREATE type::record('insight_user_edit', (SELECT VALUE memory_id FROM ONLY insight_claim LIMIT 1))")
        .await
        .unwrap()
        .check()
        .unwrap();
    let out = commit_article_insight_replacement(&p, "r", "art", vec![], vec![])
        .await
        .unwrap();
    assert_eq!(out.preserved_user_edits, 1);
    assert_eq!(counts(&db).await, (1, 0, 1, 0));
}

#[tokio::test]
async fn an_invalid_staged_claim_aborts_the_whole_replacement() {
    let Some(server) = Server::start() else {
        return;
    };
    let (db, p) = fresh(&server, "insight_invalid").await;
    commit_article_insight_replacement(&p, "r", "art", vec![claim("Rome", "art")], vec![])
        .await
        .unwrap();
    let before = counts(&db).await;
    let bad = vec![claim("Milan", "art"), claim("Turin", "other-article")];
    let err = commit_article_insight_replacement(&p, "r", "art", bad, vec![])
        .await
        .unwrap_err();
    assert!(matches!(err, TxError::Abort(_)), "{err:?}");
    assert_eq!(
        counts(&db).await,
        before,
        "nothing from the aborted batch is written"
    );
}

#[tokio::test]
async fn publication_dedups_reuses_tombstones_and_records_its_receipt() {
    use argos_osint_core::storage::insight::publish_atlas_insights;
    use argos_osint_core::store::PublishOptions;
    let Some(server) = Server::start() else {
        return;
    };
    let (db, p) = fresh(&server, "insight_publish").await;
    let mut bad = claim("Oslo", "a1");
    bad.entity.clear();
    let claims = vec![claim("Paris", "a1"), claim("Lyon", "a2"), bad];
    let r = publish_atlas_insights(
        &p,
        "run9",
        claims.clone(),
        vec![],
        "Brief.",
        PublishOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!((r.accepted_claims, r.created, r.rejected.len()), (2, 2, 1));
    assert!(r.reconciles(), "{r:?}");
    assert!(r.brief_memory_id.is_some());
    // Same-entity/predicate claims are linked as conflict_or_revision.
    let rels: i64 = scalar(
        &db,
        "RETURN count(SELECT id FROM insight_relation WHERE relation = 'conflict_or_revision')",
    )
    .await;
    assert_eq!(rels, 1);
    let receipts: i64 = scalar(
        &db,
        "RETURN count(SELECT id FROM atlas_publication WHERE state = 'published')",
    )
    .await;
    assert_eq!(receipts, 1);

    // Republish: everything reused, the brief is updated in place.
    let again = publish_atlas_insights(
        &p,
        "run9",
        claims,
        vec![],
        "Brief v2.",
        PublishOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!((again.created, again.reused), (0, 2));
    assert_eq!(again.brief_memory_id, r.brief_memory_id);
    assert_eq!(
        again.updated_memory_ids,
        vec![r.brief_memory_id.clone().unwrap()]
    );
    assert_eq!(
        scalar::<i64>(&db, "RETURN count(SELECT id FROM memory)").await,
        3
    );

    // A tombstoned fingerprint from this run is not recreated.
    let fp = r.claim_memory_ids[0].0.clone();
    db.query("DELETE insight_claim WHERE record::id(id) = $fp; CREATE memory_tombstone SET memory_id = 'x', fingerprint = $fp, run_ids = ['run9'], created_at = '';")
        .bind(("fp", fp))
        .await
        .unwrap()
        .check()
        .unwrap();
    let t = publish_atlas_insights(
        &p,
        "run9",
        vec![claim("Paris", "a1")],
        vec![],
        "",
        PublishOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!((t.accepted_claims, t.rejected.len()), (0, 1));
}

#[tokio::test]
async fn snapshot_publication_is_fenced_by_lease_and_generation() {
    use argos_osint_core::investigation::insights::EvidenceSnapshot;
    use argos_osint_core::storage::insight::{advance_generation, publish_snapshot, SnapshotStep};
    use argos_osint_core::storage::jobs::{self, NewJob, NewTask};
    use argos_osint_core::storage::tx::{run_tx, RetryPolicy};
    use std::sync::Arc;
    let Some(server) = Server::start() else {
        return;
    };
    let (db, _p) = fresh(&server, "insight_snapshot").await;
    jobs::apply_schema(&db).await.unwrap();
    let now = chrono::Utc::now();
    let job = NewJob {
        id: "j",
        kind: "task",
        operation: "insight_refresh",
        parent: None,
        created_at: now,
    };
    let task = NewTask {
        id: "t",
        job: "j",
        pool: "p",
        operation: "insight_refresh",
        priority: 0,
        deps: vec![],
        input_hash: "h",
        created_at: now,
    };
    jobs::create_job(&db, &job, &[task]).await.unwrap();
    let claimed = jobs::claim_next(&db, "p", "w", 30, now, "k1")
        .await
        .unwrap()
        .value
        .unwrap();
    let gen = run_tx(&db, RetryPolicy::default(), |tx| async move {
        advance_generation(&tx, "recon", "r1", "now").await
    })
    .await
    .unwrap()
    .value;
    let snapshot: EvidenceSnapshot = serde_json::from_value(serde_json::json!({
        "owner_kind": "recon", "owner_id": "r1", "generation": gen, "source_refs": [],
        "entities": [], "tool_refs": [], "coverage_by_stage": [], "assessed_claim_refs": [],
        "memory_refs": [], "key": "k"
    }))
    .unwrap();
    let ok: SnapshotStep = Arc::new(|_, _| Box::pin(async { Ok(()) }));
    let derive: SnapshotStep = Arc::new(|tx, id| {
        Box::pin(async move {
            tx.query("CREATE insight_derivation SET snapshot_id = $s, insight_kind = 'claim', fingerprint = 'f', item_json = '{}', assessment_json = '{}', status = 'active', revision = '1' RETURN NONE")
                .bind(("s", id))
                .await?
                .check()?;
            Ok(())
        })
    });
    // Stale input key: nothing is written.
    assert!(!publish_snapshot(
        &db,
        &claimed,
        &snapshot,
        "other",
        "active",
        now,
        ok.clone(),
        derive.clone()
    )
    .await
    .unwrap());
    let mut lost = claimed.clone();
    lost.lease_epoch += 1;
    assert!(!publish_snapshot(
        &db,
        &lost,
        &snapshot,
        "k",
        "active",
        now,
        ok.clone(),
        derive.clone()
    )
    .await
    .unwrap());
    assert_eq!(
        scalar::<i64>(&db, "RETURN count(SELECT id FROM insight_snapshot)").await,
        0
    );
    // A failing evidence check aborts the publication.
    let dead: SnapshotStep =
        Arc::new(|_, _| Box::pin(async { Err(TxError::Abort("owner gone".into())) }));
    assert!(publish_snapshot(
        &db,
        &claimed,
        &snapshot,
        "k",
        "active",
        now,
        dead,
        derive.clone()
    )
    .await
    .is_err());
    assert_eq!(
        scalar::<i64>(&db, "RETURN count(SELECT id FROM insight_derivation)").await,
        0
    );
    assert!(
        publish_snapshot(&db, &claimed, &snapshot, "k", "active", now, ok, derive)
            .await
            .unwrap()
    );
    assert_eq!(
        scalar::<i64>(&db, "RETURN count(SELECT id FROM insight_derivation)").await,
        1
    );
    let active: String = scalar(
        &db,
        "SELECT VALUE active_snapshot_id FROM ONLY insight_owner_state:['recon', 'r1']",
    )
    .await;
    assert_eq!(active, "isnap:recon:r1:1:k");
    let state: String = scalar(&db, "SELECT VALUE state FROM ONLY task:t").await;
    assert_eq!(state, "completed");
}

async fn plan(db: &Surreal<Any>, sql: &str) -> String {
    let mut r = db
        .query(format!("{sql} EXPLAIN FULL"))
        .bind(("run", "r".to_string()))
        .bind(("answer", "atlas-r".to_string()))
        .bind(("call", "a".to_string()))
        .bind(("fp", "f".to_string()))
        .await
        .unwrap()
        .check()
        .unwrap();
    let plan: surrealdb::types::Value = r.take(0).unwrap();
    format!("{plan:?}")
}

/// The source-by-call lookup and both relation deletes run on an index,
/// never a table scan.
#[tokio::test]
async fn source_and_relation_lookups_use_their_indexes() {
    use argos_osint_core::storage::insight::{
        DELETE_RELATIONS_LEFT_SURQL, DELETE_RELATIONS_RIGHT_SURQL, SOURCES_BY_CALL_SURQL,
    };
    let Some(server) = Server::start() else {
        return;
    };
    let (db, _p) = fresh(&server, "insight_plans").await;
    for (sql, index) in [
        (SOURCES_BY_CALL_SURQL, "insight_source_call"),
        (DELETE_RELATIONS_LEFT_SURQL, "insight_relation_left"),
        (DELETE_RELATIONS_RIGHT_SURQL, "insight_relation_right"),
    ] {
        let p = plan(&db, sql).await;
        assert!(p.contains(index), "{sql}: {p}");
        assert!(!p.contains("Iterate Table"), "{sql}: {p}");
    }
}
