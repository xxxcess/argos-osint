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

    // memories_changed bumps only when a publish changed something.
    let seq = || scalar::<i64>(&db, "RETURN counter:memories_changed.n ?? 0");
    let bumped = seq().await;
    assert!(bumped > 0, "a publish that created claims bumps");
    let noop = publish_atlas_insights(
        &p,
        "run9",
        vec![claim("Paris", "a1"), claim("Lyon", "a2")],
        vec![],
        "",
        PublishOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!((noop.created, noop.reused), (0, 2));
    assert!(noop.brief_memory_id.is_none() && noop.updated_memory_ids.is_empty());
    assert_eq!(seq().await, bumped, "all-reused publish leaves the counter");

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
        .bind(("item", "i".to_string()))
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
        // delete_memory_in's per-item and sweep statements.
        (
            "SELECT VALUE derivation_id FROM insight_derivation_evidence WHERE premise_id = $item",
            "insight_evidence_premise",
        ),
        (
            "UPDATE insight_derivation_link SET status = 'deleted_endpoint' WHERE left_id = $item RETURN NONE",
            "insight_links_left_id",
        ),
        (
            "UPDATE insight_derivation_link SET status = 'deleted_endpoint' WHERE right_id = $item RETURN NONE",
            "insight_links_right_id",
        ),
        (
            "SELECT VALUE snapshot_id FROM insight_derivation WHERE status = 'blocked_premise'",
            "insight_derivation_status",
        ),
    ] {
        let p = plan(&db, sql).await;
        assert!(p.contains(index), "{sql}: {p}");
        assert!(!p.contains("Iterate Table"), "{sql}: {p}");
    }
    // Derived snapshots are read by record id: no table iteration.
    let mut r = db
        .query("SELECT VALUE snapshot_id FROM [insight_derivation:d1, insight_derivation:d2] EXPLAIN FULL")
        .await
        .unwrap()
        .check()
        .unwrap();
    let p = format!("{:?}", r.take::<surrealdb::types::Value>(0).unwrap());
    assert!(!p.contains("Iterate Table"), "{p}");
}

#[tokio::test]
async fn deleting_a_memory_tombstones_it_and_cleans_its_insights_in_one_commit() {
    use argos_osint_core::storage::insight::{delete_memory, publish_atlas_insights};
    use argos_osint_core::store::PublishOptions;
    let Some(server) = Server::start() else {
        return;
    };
    let (db, p) = fresh(&server, "insight_delete").await;
    let r = publish_atlas_insights(
        &p,
        "run1",
        vec![claim("Paris", "a1"), claim("Lyon", "a1")],
        vec![],
        "",
        PublishOptions::default(),
    )
    .await
    .unwrap();
    let (fp, mem) = r.claim_memory_ids[0].clone();
    db.query(
        "UPSERT insight_owner_state:['recon', 'o'] SET owner_kind = 'recon', owner_id = 'o', active_snapshot_id = 's1', lifecycle = 'active', updated_at = '';
         CREATE insight_derivation:d1 SET snapshot_id = 's1', memory_id = $mem, insight_kind = 'claim', fingerprint = 'x', item_json = '{\"id\":\"item1\"}', assessment_json = '{}', status = 'active', revision = '1';
         CREATE insight_derivation:d2 SET snapshot_id = 's2', insight_kind = 'inference', fingerprint = 'y', item_json = '{\"id\":\"item2\"}', assessment_json = '{}', status = 'active', revision = '1';
         CREATE insight_derivation_evidence SET derivation_id = 'd1', source_id = 'm', source_revision = '1', memory_id = $mem, evidence_json = '{}';
         CREATE insight_derivation_evidence SET derivation_id = 'd2', source_id = 'c', source_revision = '1', premise_id = 'item1', evidence_json = '{}';
         CREATE insight_derivation_link:l1 SET snapshot_id = 's2', left_kind = 'claim', left_id = 'item1', right_kind = 'inference', right_id = 'item2', relation = 'supports', rationale = '', assessment_json = '{}', evidence_json = '{}', status = 'active', revision = '1';
         CREATE memory_graph_summary SET id = type::record('memory_graph_summary', $mem), summary = 's', created_at = '';",
    )
    .bind(("mem", mem.clone()))
    .await
    .unwrap()
    .check()
    .unwrap();

    assert!(delete_memory(&p, &mem).await.unwrap());
    assert!(
        !delete_memory(&p, &mem).await.unwrap(),
        "a second delete is a no-op"
    );
    let q = |sql: &'static str| scalar::<String>(&db, sql);
    assert_eq!(
        q("SELECT VALUE lifecycle FROM ONLY insight_owner_state:['recon', 'o']").await,
        "previous"
    );
    assert_eq!(
        q("SELECT VALUE status FROM ONLY insight_derivation:d1").await,
        "deleted"
    );
    assert_eq!(
        q("SELECT VALUE status FROM ONLY insight_derivation:d2").await,
        "blocked_premise"
    );
    assert_eq!(
        q("SELECT VALUE status FROM ONLY insight_derivation_link:l1").await,
        "deleted_endpoint"
    );
    let fp_left: i64 = scalar(&db, "RETURN count(SELECT id FROM insight_claim)").await;
    assert_eq!(fp_left, 1, "only the other claim remains");
    let tomb: Vec<String> = {
        let mut r = db
            .query("SELECT VALUE run_ids FROM memory_tombstone WHERE fingerprint = $fp")
            .bind(("fp", fp.clone()))
            .await
            .unwrap();
        r.take::<Vec<Vec<String>>>(0usize).unwrap().concat()
    };
    assert_eq!(tomb, vec!["run1".to_string()]);
    assert_eq!(
        scalar::<i64>(&db, "RETURN count(SELECT id FROM memory_graph_summary)").await,
        0
    );
    assert_eq!(
        scalar::<i64>(
            &db,
            "RETURN count(SELECT id FROM insight_source WHERE fingerprint = $fp)"
        )
        .await,
        0
    );
    let removal: i64 = scalar(
        &db,
        &format!("RETURN count(SELECT id FROM index_work WHERE record::id(id) CONTAINS '{mem}')"),
    )
    .await;
    assert!(removal >= 1, "removal index work is queued");
    // The tombstone stops republication of this run's claim.
    let again = publish_atlas_insights(
        &p,
        "run1",
        vec![claim("Paris", "a1")],
        vec![],
        "",
        PublishOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(again.rejected.len(), 1);
}

#[tokio::test]
async fn deleting_a_memory_drops_its_nlp_refs_and_only_unshared_artifacts() {
    use argos_osint_core::storage::insight::{delete_memory, publish_atlas_insights};
    use argos_osint_core::store::PublishOptions;
    let Some(server) = Server::start() else {
        return;
    };
    let (db, p) = fresh(&server, "insight_delete_nlp").await;
    let r = publish_atlas_insights(
        &p,
        "run1",
        vec![claim("Paris", "a1")],
        vec![],
        "",
        PublishOptions::default(),
    )
    .await
    .unwrap();
    let mem = r.claim_memory_ids[0].1.clone();
    let reference =
        "SET owner_kind = $k, owner_id = $o, source_id = 's', revision = '1', attempt_id = 'a',
           raw_hash = $raw, prose_hash = $prose, metadata_json = '{}', transform_version = 't'";
    for h in ["only", "shared", "raw", "evidence"] {
        db.query(
            "CREATE type::record('nlp_artifact', $h) SET text = $h, byte_len = 1, created_at = ''",
        )
        .bind(("h", h))
        .await
        .unwrap()
        .check()
        .unwrap();
    }
    for (k, o, raw, prose) in [
        ("memory", mem.as_str(), Some("raw"), "only"),
        ("memory", mem.as_str(), None, "shared"),
        ("memory", mem.as_str(), None, "evidence"),
        ("article", "a9", None, "shared"),
    ] {
        db.query(format!("CREATE nlp_source_ref {reference}"))
            .bind(("k", k))
            .bind(("o", o.to_string()))
            .bind(("raw", raw))
            .bind(("prose", prose))
            .await
            .unwrap()
            .check()
            .unwrap();
    }
    db.query(
        "CREATE insight_derivation_evidence SET derivation_id = 'dx', source_id = 'm', source_revision = '1',
           evidence_json = '{\"artifact_hash\":\"evidence\"}'",
    )
    .await
    .unwrap()
    .check()
    .unwrap();

    assert!(delete_memory(&p, &mem).await.unwrap());
    let left = |sql: &'static str| {
        let db = db.clone();
        async move {
            let mut r = db.query(sql).await.unwrap().check().unwrap();
            let mut v: Vec<String> = r.take(0usize).unwrap();
            v.sort();
            v
        }
    };
    assert_eq!(
        left("SELECT VALUE record::id(id) FROM nlp_artifact").await,
        vec!["evidence", "shared"],
        "unshared artifacts collected; shared and evidence-retained kept"
    );
    assert_eq!(
        left("SELECT VALUE owner_kind FROM nlp_source_ref").await,
        vec!["article"]
    );
}
