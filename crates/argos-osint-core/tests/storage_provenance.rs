//! Legacy publication receipts and record-derived edges (migration phase 3,
//! step 4) against a real SurrealDB 3.3.4 disk server or the embedded
//! RocksDB engine (see `support`). Run through
//! `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::storage::insight::{self, publish_atlas_insights};
use argos_osint_core::storage::publisher::Publisher;
use argos_osint_core::storage::{atlas, lifecycle, memory, outbox, provenance};
use argos_osint_core::store::{AtlasArticleRow, AtlasInsightClaim, PublishOptions};
use support::*;
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

async fn fresh(server: &Server, name: &str) -> (Surreal<Any>, Publisher) {
    let db = server.root("argos", name).await;
    outbox::apply_schema(&db).await.unwrap();
    memory::apply_schema(&db).await.unwrap();
    insight::apply_schema(&db).await.unwrap();
    atlas::apply_schema(&db).await.unwrap();
    lifecycle::apply_schema(&db).await.unwrap();
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

#[tokio::test]
async fn legacy_receipt_reuses_live_links_and_rejects_lost_ones() {
    let Some(server) = Server::start() else {
        return;
    };
    let (db, p) = fresh(&server, "prov_legacy").await;
    let r = publish_atlas_insights(
        &p,
        "run1",
        vec![
            claim("Paris", "a1"),
            claim("Lyon", "a2"),
            claim("Rome", "a3"),
        ],
        vec![],
        "Brief.",
        PublishOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(r.created, 3);
    let (fp_paris, mem_paris) = r.claim_memory_ids[0].clone();
    let (fp_lyon, mem_lyon) = r.claim_memory_ids[1].clone();
    // Paris: memory row lost. Lyon: tombstoned and gone.
    db.query("DELETE type::record('memory', $m)")
        .bind(("m", mem_paris.clone()))
        .await
        .unwrap()
        .check()
        .unwrap();
    db.query(
        "DELETE type::record('memory', $m);
         CREATE memory_tombstone SET memory_id = $m, fingerprint = $fp, created_at = '1';",
    )
    .bind(("m", mem_lyon.clone()))
    .bind(("fp", fp_lyon.clone()))
    .await
    .unwrap()
    .check()
    .unwrap();
    // Orphan link: a source row without a claim.
    db.query(
        "CREATE type::record('insight_source', ['zz-orphan', 'atlas-run1', 'a9']) SET
           fingerprint = 'zz-orphan', run_id = 'run1', answer_id = 'atlas-run1', call_id = 'a9';",
    )
    .await
    .unwrap()
    .check()
    .unwrap();

    let legacy = provenance::legacy_receipt(&p, "run1", Some("job7"))
        .await
        .unwrap();
    assert_eq!(legacy.revision, provenance::LEGACY_REVISION);
    assert_eq!(legacy.input_claims, 4);
    assert_eq!((legacy.accepted_claims, legacy.reused), (1, 1));
    let reasons: Vec<(&str, &str)> = legacy
        .rejected
        .iter()
        .map(|c| (c.article_id.as_str(), c.reason.as_str()))
        .collect();
    assert_eq!(reasons.len(), 3, "{reasons:?}");
    assert!(reasons.contains(&(
        "a9",
        "source link has no claim; claim text lost — re-extraction required"
    )));
    assert!(reasons.contains(&("a2", "deleted by user (tombstoned); not recreated")));
    assert!(reasons.contains(&(
        "a1",
        "canonical memory row missing; claim text lost — re-extraction required"
    )));
    let _ = fp_paris;
    assert_eq!(legacy.brief_memory_id, r.brief_memory_id);
    assert_eq!(legacy.affected_memory_ids.len(), 2, "Rome + brief");
    let stored: String = scalar(
        &db,
        "RETURN (SELECT VALUE state FROM ONLY atlas_publication WHERE run_id = 'run1' AND revision = 'legacy' LIMIT 1)",
    )
    .await;
    assert_eq!(stored, "reconstructed");
    // Replay is idempotent: one receipt row.
    provenance::legacy_receipt(&p, "run1", None).await.unwrap();
    assert_eq!(
        scalar::<i64>(
            &db,
            "RETURN count(SELECT id FROM atlas_publication WHERE revision = 'legacy')"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn record_edges_follow_sources_topics_and_mentions() {
    let Some(server) = Server::start() else {
        return;
    };
    let (db, p) = fresh(&server, "prov_edges").await;
    for id in ["a1", "a2", "a3"] {
        atlas::upsert_article(
            &db,
            &AtlasArticleRow {
                run_id: "run1".into(),
                id: id.into(),
                title: "t".into(),
                seen_at: "1".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
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
    let (_, mem) = r.claim_memory_ids[0].clone();
    // A live derivation owned by article a2 with body evidence, and a
    // retired one owned by a1's run that must not count.
    db.query(
        "CREATE type::record('nlp_artifact', 'h2') SET text = 't\n', byte_len = 2, created_at = '1';
         CREATE type::record('insight_snapshot', 's1') SET owner_kind = 'article', owner_id = 'a2',
           generation = 1, input_key = 'k', entities_json = '[]',
           inputs_json = '{\"source_refs\":[{\"source_id\":\"article:a2\",\"url\":\"\",\"artifact_hash\":\"h2\"}]}',
           coverage_json = '{}', readiness_json = '{}', contract_version = 'c',
           policy_version = 'p', created_at = '1';
         CREATE type::record('insight_derivation', 'd1') SET snapshot_id = 's1', memory_id = $m,
           insight_kind = 'claim', fingerprint = 'f', item_json = '{}', assessment_json = '{}',
           status = 'assessed', revision = '1';
         CREATE type::record('insight_derivation', 'd2') SET snapshot_id = 's1', memory_id = $m,
           insight_kind = 'claim', fingerprint = 'f', item_json = '{}', assessment_json = '{}',
           status = 'retired', revision = '1';
         CREATE type::record('insight_snapshot', 's3') SET owner_kind = 'article', owner_id = 'a3',
           generation = 1, input_key = 'k', inputs_json = '{}', entities_json = '[]',
           coverage_json = '{}', readiness_json = '{}', contract_version = 'c',
           policy_version = 'p', created_at = '1';
         CREATE type::record('insight_derivation', 'd3') SET snapshot_id = 's3', memory_id = $m,
           insight_kind = 'claim', fingerprint = 'f', item_json = '{}', assessment_json = '{}',
           status = 'assessed', revision = '1';
         CREATE insight_derivation_evidence SET derivation_id = 'd1', source_id = 'body:a2',
           source_revision = 'r1', memory_id = $m, evidence_json = '{\"q\":1}';
         CREATE insight_derivation_evidence SET derivation_id = 'd2', source_id = 'body:old',
           source_revision = 'r0', memory_id = $m, evidence_json = '{}';",
    )
    .bind(("m", mem.clone()))
    .await
    .unwrap()
    .check()
    .unwrap();
    let edges = provenance::memory_record_edges(&db, &mem).await.unwrap();
    let find = |key: &str| edges.iter().find(|e| e.key == key).cloned();
    let a1 = find(&format!("article-evidence:article:run1/a1:{mem}")).expect("a1 source");
    assert_eq!(a1.meaning, "source");
    assert_eq!(a1.status, "historical origin · revision unavailable");
    let a2 = find(&format!("article-evidence:article:run1/a2:{mem}")).expect("a2 source");
    assert!(a2.status.is_empty() && a2.receipt == "insight_derivations:d1");
    let a3 = find(&format!("article-evidence:article:run1/a3:{mem}")).expect("a3 source");
    assert_eq!(
        a3.status, "historical origin · revision unavailable",
        "a snapshot whose refs no longer describe the article is historical"
    );
    assert!(find(&format!("evidence:source:evidence:body:a2@r1:{mem}")).is_some());
    assert!(
        find(&format!("evidence:source:evidence:body:old@r0:{mem}")).is_none(),
        "retired derivation evidence is not provenance"
    );
    let topic = find(&format!("topic:topic:geo:{mem}")).expect("contains");
    assert_eq!(
        (topic.meaning, topic.from.as_str()),
        ("contains", mem.as_str())
    );
    assert_eq!(edges.len(), 5, "{edges:#?}");

    // MentionedIn: retained article mentions only.
    db.query(
        "CREATE brain_actor_mention SET actor_id = 'act', origin_kind = 'article', origin_scope = 'run1',
           origin_id = 'a1', created_at = '1';
         CREATE brain_actor_mention SET actor_id = 'act', origin_kind = 'article', origin_scope = 'run1',
           origin_id = 'gone', created_at = '1';
         CREATE brain_actor_mention SET actor_id = 'act', origin_kind = 'recon', origin_scope = 'ns',
           origin_id = 'e1', origin_revision = 'v', created_at = '1';",
    )
    .await
    .unwrap()
    .check()
    .unwrap();
    let mentions = provenance::actor_mention_edges(&db, "act").await.unwrap();
    let to: Vec<&str> = mentions.iter().map(|e| e.to.as_str()).collect();
    assert_eq!(to, vec!["article:run1/a1", "source:recon:e1@v"]);
    assert!(mentions.iter().all(|e| e.meaning == "mentioned_in"));
}

#[tokio::test]
async fn stage_outputs_read_a_chains_committed_stages() {
    use argos_osint_core::storage::jobs;
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.root("argos", "prov_stages").await;
    jobs::apply_schema(&db).await.unwrap();
    db.query(
        "CREATE stage_output SET task = task:a, root = job:j1, stage = 'intel_brief_claims',
           output_hash = 'h', output_json = '{\"claims\":[1]}', committed_at = time::now();
         CREATE stage_output SET task = task:b, root = job:j1, stage = 'intel_brief_body',
           output_hash = 'h', output_json = '{\"markdown\":\"x\"}', committed_at = time::now();
         CREATE stage_output SET task = task:c, root = job:j1, stage = 'intel_brief_links',
           output_hash = 'h', output_json = 'not json', committed_at = time::now();
         CREATE stage_output SET task = task:d, root = job:j2, stage = 'intel_brief_inferences',
           output_hash = 'h', output_json = '{}', committed_at = time::now();",
    )
    .await
    .unwrap()
    .check()
    .unwrap();
    let out = provenance::stage_outputs(&db, "j1").await.unwrap();
    assert_eq!(out.len(), 2);
    assert_eq!(out["intel_brief_claims"]["claims"][0], 1);
    assert_eq!(
        provenance::republished_stages(&out),
        vec!["intel_brief_claims"]
    );
}
