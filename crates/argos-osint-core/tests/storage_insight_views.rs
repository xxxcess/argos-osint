//! Insight reads on the async repository (migration phase 4), the CLI's
//! `argos insights` and recall's `insight_metadata`. Both modes via
//! `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::brain::MemorySource;
use argos_osint_core::storage::{insight_views, memories, memory::memory_revision};
use support::*;

fn source() -> MemorySource {
    MemorySource {
        app: "cli".into(),
        conversation_id: "c1".into(),
        message_id: None,
        reference: None,
    }
}

const ASSESS: &str = r#"{"fidelity":"faithful","relation":"supports","threshold_passed":true,"rationale":"r","gaps":[]}"#;

#[tokio::test]
async fn insights_search_view_and_metadata_through_the_repository() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "insight_views").await;
    let a = memories::add_memory(
        &repo,
        "Acme ships via Rotterdam",
        "fact",
        false,
        source(),
        None,
    )
    .await
    .unwrap();
    let b = memories::add_memory(&repo, "Globex hires pilots", "fact", false, source(), None)
        .await
        .unwrap();
    let rev = memory_revision(&a.text);
    let claim = |fp: &str, mid: &str, entity: &str, topic: &str, at: &str| {
        format!(
            "CREATE insight_claim:{fp} CONTENT {{ memory_id: '{mid}', entity_id: '{entity}', \
             predicate: 'p', object_value: 'o', topic: '{topic}', classification: 'fact', \
             confidence: 0.8, created_at: '{at}', updated_at: '{at}' }} RETURN NONE;"
        )
    };
    let sql = [
        claim("fpa", &a.id, "Acme Corp", "Shipping", "2026-01-02"),
        claim("fpb", &b.id, "Globex", "Hiring", "2026-01-03"),
        "CREATE insight_source CONTENT { fingerprint: 'fpa', thread_id: 't1', run_id: NONE, \
         answer_id: 'an', call_id: 'ca', source_url: 'https://x', deleted_origin: false } RETURN NONE;
         CREATE insight_source CONTENT { fingerprint: 'fpa', answer_id: 'an2', call_id: 'cb', \
         deleted_origin: true } RETURN NONE;
         CREATE insight_relation CONTENT { left_fingerprint: 'fpb', right_fingerprint: 'fpa', \
         relation: 'related' } RETURN NONE;
         CREATE insight_snapshot:s1 CONTENT { owner_kind: 'thread', owner_id: 't1', generation: 1, \
         input_key: 'k', inputs_json: '[]', entities_json: '[]', coverage_json: '{}', \
         readiness_json: '{}', contract_version: 'v', policy_version: 'v', created_at: '2026-01-01' } RETURN NONE;
         CREATE insight_snapshot:s2 CONTENT { owner_kind: 'thread', owner_id: 't1', generation: 2, \
         input_key: 'k', inputs_json: '[]', entities_json: '[]', coverage_json: '{}', \
         readiness_json: '{}', contract_version: 'v', policy_version: 'v', created_at: '2025-01-01' } RETURN NONE;
         CREATE type::record('insight_owner_state', ['thread', 't1']) CONTENT { owner_kind: 'thread', \
         owner_id: 't1', active_snapshot_id: 's2', lifecycle: 'ready', updated_at: '2026' } RETURN NONE;"
            .to_string(),
        format!(
            "CREATE insight_derivation:d1 CONTENT {{ snapshot_id: 's1', memory_id: '{0}', \
             insight_kind: 'inference', fingerprint: 'fpa', item_json: '{{}}', assessment_json: $as, \
             status: 'published', revision: '{1}' }} RETURN NONE;
             CREATE insight_derivation:d2 CONTENT {{ snapshot_id: 's2', memory_id: '{0}', \
             insight_kind: 'claim', fingerprint: 'fpa', item_json: '{{}}', assessment_json: $as, \
             status: 'published', revision: '{1}' }} RETURN NONE;
             CREATE insight_derivation:d3 CONTENT {{ snapshot_id: 's2', memory_id: '{0}', \
             insight_kind: 'claim', fingerprint: 'fpa', item_json: '{{}}', assessment_json: $as, \
             status: 'deleted', revision: '{1}' }} RETURN NONE;",
            a.id, rev
        ),
    ]
    .concat();
    repo.db()
        .query(sql)
        .bind(("as", ASSESS.to_string()))
        .await
        .unwrap()
        .check()
        .unwrap();

    // The active snapshot wins over the newer previous one.
    let meta = insight_views::insight_metadata(&repo, &a.id)
        .await
        .unwrap()
        .expect("metadata");
    assert_eq!(meta.snapshot_id, "s2");
    assert_eq!(meta.lifecycle, "ready");
    assert_eq!(meta.insight_kind, "claim");
    assert_eq!(meta.revision, rev);
    assert!(meta.assessment.threshold_passed);
    assert!(insight_views::insight_metadata(&repo, &b.id)
        .await
        .unwrap()
        .is_none());
    assert!(insight_views::insight_metadata(&repo, "nope")
        .await
        .unwrap()
        .is_none());

    let view = insight_views::insight_for_memory(&repo, &a.id)
        .await
        .unwrap()
        .expect("view");
    assert_eq!(view.entity, "Acme Corp");
    assert_eq!(view.sources.len(), 1, "deleted origins are hidden");
    assert_eq!(view.sources[0].thread_id.as_deref(), Some("t1"));
    assert_eq!(view.related, vec!["fpb".to_string()]);
    assert!(insight_views::insight_for_memory(&repo, "nope")
        .await
        .unwrap()
        .is_none());

    let all = insight_views::search_insights(&repo, "", "").await.unwrap();
    assert_eq!(
        all.iter().map(|v| v.memory_id.as_str()).collect::<Vec<_>>(),
        vec![b.id.as_str(), a.id.as_str()],
        "newest first"
    );
    let hit = insight_views::search_insights(&repo, "acme", "SHIP")
        .await
        .unwrap();
    assert_eq!(hit.len(), 1);
    assert_eq!(hit[0].memory_id, a.id);
    assert!(insight_views::search_insights(&repo, "acme", "hiring")
        .await
        .unwrap()
        .is_empty());

    // An edited memory reads as user_edited and unassessed.
    repo.db()
        .query("UPDATE $m SET text = 'Acme ships via Antwerp' RETURN NONE")
        .bind(("m", surrealdb::types::RecordId::new("memory", a.id.clone())))
        .await
        .unwrap()
        .check()
        .unwrap();
    let meta = insight_views::insight_metadata(&repo, &a.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(meta.assessment.fidelity, "user_edited");
    assert_eq!(meta.assessment.relation, "unassessed");
    assert!(!meta.assessment.threshold_passed);
    assert_eq!(meta.revision, memory_revision("Acme ships via Antwerp"));
}
