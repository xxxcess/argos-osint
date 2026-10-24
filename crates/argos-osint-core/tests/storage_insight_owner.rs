//! Insight owner state on SurrealDB for Recon owners (Recon switch, step 2):
//! generations, refresh enqueue, Recon live evidence and opt-in,
//! `active_output` and `reconcile_deleted_memories`. Both modes via
//! `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::investigation::insights::{
    snapshot, Assessment, EvidenceLocation, EvidenceRef, PipelineInput, RetainedEvidence,
};
use argos_osint_core::nlp::cache::hash;
use argos_osint_core::provider::SettingsFile;
use argos_osint_core::storage::insight_owner::*;
use argos_osint_core::storage::live_evidence::validate_in;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::tx::{run_tx, RetryPolicy};
use serde_json::json;
use support::*;
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

async fn sql(db: &Surreal<Any>, q: &str) {
    db.query(q).await.unwrap().check().unwrap();
}

async fn scalar<T: SurrealValue>(db: &Surreal<Any>, q: &str) -> Option<T> {
    let mut r = db.query(q).await.unwrap().check().unwrap();
    r.take::<Option<T>>(0usize).unwrap()
}

const RESULT: &str = "{\"status\":\"completed\",\"raw\":\"r\"}";

/// Thread t1 (opted in) with live run run1 and completed call c1.
async fn recon_fixture(db: &Surreal<Any>) {
    db.query(
        "CREATE recon_thread:t1 SET title = 'T', created_at = '', updated_at = '', recall_insights = true;
         CREATE recon_run:run1 SET thread_id = 't1', turn_id = 'u1', state = 'running', stage = '',
           recon_model = '', synthesis_model = '', max_rounds = 1, max_calls = 1, turn_seconds = 300,
           created_at = '', updated_at = '';
         CREATE osint_call:c1 SET tool_id = 'x', run_id = 'run1', thread_id = 't1', origin = 'recon',
           inputs_json = '{}', status = 'completed', result_json = $res, started_at = '';
         CREATE type::record('nlp_artifact', $h) SET text = 'call text', byte_len = 1, created_at = '';",
    )
    .bind(("res", RESULT))
    .bind(("h", hash("call text")))
    .await
    .unwrap()
    .check()
    .unwrap();
}

fn call_evidence() -> Vec<RetainedEvidence> {
    vec![RetainedEvidence {
        reference: EvidenceRef {
            source_id: "call:c1#page1".into(),
            revision: hash(RESULT),
            artifact_hash: hash("call text"),
            acquisition_id: String::new(),
            origin_root: String::new(),
            url: String::new(),
            published_at: None,
            retrieved_at: String::new(),
            location: EvidenceLocation::JsonPointer(String::new()),
        },
        text: "call text".into(),
        native: None,
        substantive: true,
        relevant: true,
        complete_story: true,
        known_truncation: false,
    }]
}

fn recon_input(generation: u32) -> PipelineInput {
    let evidence = call_evidence();
    PipelineInput {
        snapshot: snapshot("recon", "run1", generation, &evidence),
        evidence,
        objective: "Answer.".into(),
        surface: "recon".into(),
        required_tasks_complete: true,
        save_insights: true,
        recall_enabled: true,
    }
}

async fn validate(repo: &Repository, input: &PipelineInput) -> Result<(), String> {
    let snap = input.snapshot.clone();
    run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let snap = snap.clone();
        async move { validate_in(&tx, &snap).await }
    })
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}

#[tokio::test]
async fn recon_generations_and_refresh_enqueue() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "insight_owner_enqueue").await;
    let db = repo.db();
    recon_fixture(db).await;
    assert_eq!(desired_generation(&repo, "recon", "run1").await.unwrap(), 0);
    assert_eq!(
        advance_generation(&repo, "recon", "run1", "t0")
            .await
            .unwrap(),
        1
    );
    sql(
        db,
        "UPDATE insight_owner_state SET active_snapshot_id = 'keep', lifecycle = 'active'",
    )
    .await;
    assert_eq!(
        advance_generation(&repo, "recon", "run1", "t1")
            .await
            .unwrap(),
        2
    );
    assert_eq!(desired_generation(&repo, "recon", "run1").await.unwrap(), 2);
    assert_eq!(
        scalar::<String>(
            db,
            "SELECT VALUE lifecycle FROM ONLY insight_owner_state:['recon','run1']"
        )
        .await,
        Some("pending".into())
    );
    assert_eq!(
        scalar::<String>(
            db,
            "SELECT VALUE active_snapshot_id FROM ONLY insight_owner_state:['recon','run1']"
        )
        .await,
        Some("keep".into()),
        "a new generation keeps the active pointer"
    );

    let settings = SettingsFile::default();
    let mut input = recon_input(2);
    let id = enqueue_refresh(&repo, &mut input, &settings).await.unwrap();
    assert!(!input.snapshot.key.is_empty());
    assert_eq!(
        id,
        format!("insight-refresh:recon:run1:2:{}", input.snapshot.key)
    );
    assert_eq!(
        scalar::<String>(
            db,
            "SELECT VALUE refresh_job_id FROM ONLY insight_owner_state:['recon','run1']"
        )
        .await,
        Some("insight-refresh:recon:run1:2".into())
    );
    assert_eq!(
        scalar::<String>(
            db,
            &format!(
                "SELECT VALUE owner_scope FROM ONLY type::record('insight_refresh_task', '{id}')"
            )
        )
        .await,
        Some("recon:run1".into())
    );
    // Idempotent: the same input maps to the same task.
    let again = enqueue_refresh(&repo, &mut recon_input(2), &settings)
        .await
        .unwrap();
    assert_eq!(again, id);
    assert_eq!(count(db, "insight_refresh_task").await, 1);
    // A stale generation does not take over the owner's job pointer.
    enqueue_refresh(&repo, &mut recon_input(1), &settings)
        .await
        .unwrap();
    assert_eq!(
        scalar::<String>(
            db,
            "SELECT VALUE refresh_job_id FROM ONLY insight_owner_state:['recon','run1']"
        )
        .await,
        Some("insight-refresh:recon:run1:2".into())
    );

    let mut atlas = recon_input(2);
    atlas.snapshot.owner_kind = "atlas".into();
    assert!(enqueue_refresh(&repo, &mut atlas, &settings)
        .await
        .unwrap_err()
        .to_string()
        .contains("Atlas cycles cannot enqueue"));
    let mut report = recon_input(2);
    report.surface = "recon_report".into();
    assert!(enqueue_refresh(&repo, &mut report, &settings)
        .await
        .unwrap_err()
        .to_string()
        .contains("retired"));
}

#[tokio::test]
async fn recon_live_evidence_and_opt_in() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "insight_owner_live").await;
    let db = repo.db();
    recon_fixture(db).await;
    let input = recon_input(1);
    validate(&repo, &input).await.unwrap();

    let ctx = recon_context(&repo, "run1", true).await.unwrap();
    assert!(ctx.save_insights);
    assert_eq!(ctx.recon_thread.as_deref(), Some("t1"));
    assert!(
        !recon_context(&repo, "run1", false)
            .await
            .unwrap()
            .save_insights
    );
    let missing = recon_context(&repo, "nope", true).await.unwrap();
    assert!(!missing.save_insights && missing.recon_thread.is_none());

    sql(db, "UPDATE osint_call:c1 SET result_json = '{}'").await;
    assert!(validate(&repo, &input)
        .await
        .unwrap_err()
        .contains("tool evidence revision changed"));
    sql(
        db,
        "UPDATE osint_call:c1 SET result_json = $r, status = 'failed'"
            .replace("$r", &format!("'{RESULT}'"))
            .as_str(),
    )
    .await;
    assert!(validate(&repo, &input)
        .await
        .unwrap_err()
        .contains("tool evidence revision changed"));
    sql(db, "UPDATE osint_call:c1 SET status = 'completed'").await;
    validate(&repo, &input).await.unwrap();

    sql(db, "UPDATE recon_thread:t1 SET recall_insights = false").await;
    assert!(
        !recon_context(&repo, "run1", true)
            .await
            .unwrap()
            .save_insights
    );
    sql(
        db,
        "UPDATE recon_thread:t1 SET recall_insights = true, deleted = true",
    )
    .await;
    assert!(
        !recon_context(&repo, "run1", true)
            .await
            .unwrap()
            .save_insights
    );
    assert!(validate(&repo, &input)
        .await
        .unwrap_err()
        .contains("Recon owner no longer active"));
    sql(
        db,
        "UPDATE recon_thread:t1 SET deleted = false; UPDATE recon_run:run1 SET state = 'cancelled'",
    )
    .await;
    assert!(validate(&repo, &input)
        .await
        .unwrap_err()
        .contains("Recon owner no longer active"));
    sql(db, "UPDATE recon_run:run1 SET state = 'completed'").await;
    validate(&repo, &input).await.unwrap();
    sql(db, "DELETE recon_run:run1").await;
    assert!(validate(&repo, &input)
        .await
        .unwrap_err()
        .contains("Recon owner no longer active"));
}

fn assessment() -> serde_json::Value {
    serde_json::to_value(Assessment::default()).unwrap()
}

fn claim(id: &str) -> String {
    json!({"id": id, "subject_ref": "e1", "predicate": "p", "object": "o", "text": id,
        "polarity": "", "modality": "", "attributed_to": "", "temporal_scope": "",
        "evidence_refs": [], "extraction_scope": "", "assessment": assessment()})
    .to_string()
}

fn inference(id: &str) -> String {
    json!({"id": id, "text": id, "entity_refs": [], "premise_claim_ids": ["cl1"],
        "evidence_refs": [], "rationale": "", "limitations": [], "snapshot_key": "k1",
        "assessment": assessment()})
    .to_string()
}

/// Owner recon:run1 active on snapshot s1: claims cl1 (memory m1) and cl2
/// (memory m2), inference in1, a deleted claim, and two links.
async fn output_fixture(db: &Surreal<Any>) {
    db.query(
        "CREATE memory:m1 SET text = 'one'; CREATE memory:m2 SET text = 'two';
         CREATE insight_owner_state:['recon','run1'] SET owner_kind = 'recon', owner_id = 'run1',
           desired_generation = 1, active_snapshot_id = 's1', lifecycle = 'active', updated_at = '';
         CREATE insight_owner_state:['recon','run2'] SET owner_kind = 'recon', owner_id = 'run2',
           desired_generation = 1, active_snapshot_id = 's2', lifecycle = 'active', updated_at = '';
         CREATE insight_snapshot:s1 SET owner_kind = 'recon', owner_id = 'run1', generation = 1,
           input_key = 'k1', inputs_json = '{}', entities_json = '[]', coverage_json = '[]',
           readiness_json = $ready, contract_version = 'c', policy_version = 'p', created_at = '';
         CREATE insight_snapshot:s2 SET owner_kind = 'recon', owner_id = 'run2', generation = 1,
           input_key = 'k2', inputs_json = '{}', entities_json = '[]', coverage_json = '[]',
           readiness_json = '{}', contract_version = 'c', policy_version = 'p', created_at = '';
         CREATE insight_derivation:d1 SET snapshot_id = 's1', memory_id = 'm1', insight_kind = 'claim',
           fingerprint = 'f1', item_json = $c1, assessment_json = '{}', status = 'assessed', revision = '';
         CREATE insight_derivation:d2 SET snapshot_id = 's1', memory_id = 'm2', insight_kind = 'claim',
           fingerprint = 'f2', item_json = $c2, assessment_json = '{}', status = 'assessed', revision = '';
         CREATE insight_derivation:d3 SET snapshot_id = 's1', memory_id = NONE, insight_kind = 'inference',
           fingerprint = 'f3', item_json = $i1, assessment_json = '{}', status = 'assessed', revision = '';
         CREATE insight_derivation:d4 SET snapshot_id = 's1', memory_id = NONE, insight_kind = 'claim',
           fingerprint = 'f4', item_json = $c4, assessment_json = '{}', status = 'deleted', revision = '';
         CREATE insight_derivation:d5 SET snapshot_id = 's2', memory_id = 'm2', insight_kind = 'claim',
           fingerprint = 'f5', item_json = $c5, assessment_json = '{}', status = 'assessed', revision = '';
         CREATE insight_derivation_link:l1 SET snapshot_id = 's1', left_kind = 'claim', left_id = 'cl1',
           right_kind = 'inference', right_id = 'in1', relation = 'supports', rationale = 'r',
           assessment_json = $assess, evidence_json = '[]', status = 'assessed', revision = '';
         CREATE insight_derivation_link:l2 SET snapshot_id = 's1', left_kind = 'claim', left_id = 'cl2',
           right_kind = 'claim', right_id = 'cl1', relation = 'x', rationale = '',
           assessment_json = $assess, evidence_json = '[]', status = 'retired', revision = '';
         CREATE insight_derivation_evidence:e1 SET derivation_id = 'd1', source_id = 'call:c1',
           source_revision = '', memory_id = 'm1', evidence_json = '{}';
         CREATE insight_derivation_evidence:e2 SET derivation_id = 'd2', source_id = 'call:c1',
           source_revision = '', memory_id = 'm2', evidence_json = '{}';",
    )
    .bind(("ready", json!({"limit_reached": true, "withheld_count": 3, "item_gaps": ["g"]}).to_string()))
    .bind(("assess", assessment().to_string()))
    .bind(("c1", claim("cl1")))
    .bind(("c2", claim("cl2")))
    .bind(("c4", claim("cl4")))
    .bind(("c5", claim("cl5")))
    .bind(("i1", inference("in1")))
    .await
    .unwrap()
    .check()
    .unwrap();
}

#[tokio::test]
async fn active_output_reads_the_active_snapshot() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "insight_owner_output").await;
    let db = repo.db();
    assert!(active_output(&repo, "recon", "run1")
        .await
        .unwrap()
        .is_none());
    output_fixture(db).await;
    let out = active_output(&repo, "recon", "run1")
        .await
        .unwrap()
        .unwrap();
    let ids: Vec<_> = out.claims.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["cl1", "cl2"], "deleted derivations are skipped");
    assert_eq!(out.inferences.len(), 1);
    assert_eq!(out.links.len(), 1, "only assessed links");
    assert_eq!(out.links[0].id, "l1");
    assert_eq!(out.links[0].snapshot_key, "k1");
    assert_eq!(out.links[0].typed_endpoints[1].id, "in1");
    assert!(out.limit_reached && out.withheld_count == 3 && out.item_gaps == ["g"]);
    assert!(out.readiness.input_eligible && out.readiness.publication_eligible);

    sql(
        db,
        "UPDATE insight_owner_state:['recon','run1'] SET lifecycle = 'previous'",
    )
    .await;
    let out = active_output(&repo, "recon", "run1")
        .await
        .unwrap()
        .unwrap();
    assert!(!out.readiness.input_eligible && !out.readiness.publication_eligible);
    assert!(out
        .readiness
        .gaps
        .iter()
        .any(|g| g == "insight lifecycle: previous"));

    sql(
        db,
        "UPDATE insight_owner_state:['recon','run1'] SET active_snapshot_id = ''",
    )
    .await;
    assert!(active_output(&repo, "recon", "run1")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn reconcile_deleted_memories_retires_dangling_derivations() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "insight_owner_reconcile").await;
    let db = repo.db();
    output_fixture(db).await;
    // A deleted derivation on the active snapshot already retires its owner
    // (as in SQLite); drop it so the first pass has nothing to do.
    sql(db, "DELETE insight_derivation:d4").await;
    // Nothing dangling: nothing changes.
    reconcile_deleted_memories(&repo).await.unwrap();
    assert_eq!(
        scalar::<String>(
            db,
            "SELECT VALUE lifecycle FROM ONLY insight_owner_state:['recon','run1']"
        )
        .await,
        Some("active".into())
    );
    assert_eq!(
        scalar::<String>(
            db,
            "SELECT VALUE status FROM ONLY insight_derivation_link:l1"
        )
        .await,
        Some("assessed".into())
    );

    sql(db, "DELETE memory:m1").await;
    reconcile_deleted_memories(&repo).await.unwrap();
    assert_eq!(
        scalar::<String>(db, "SELECT VALUE status FROM ONLY insight_derivation:d1").await,
        Some("deleted".into())
    );
    assert_eq!(
        scalar::<String>(db, "SELECT VALUE memory_id FROM ONLY insight_derivation:d1").await,
        None
    );
    assert_eq!(
        scalar::<String>(db, "SELECT VALUE status FROM ONLY insight_derivation:d2").await,
        Some("assessed".into())
    );
    for link in ["l1", "l2"] {
        assert_eq!(
            scalar::<String>(
                db,
                &format!("SELECT VALUE status FROM ONLY insight_derivation_link:{link}")
            )
            .await,
            Some("deleted_endpoint".into()),
            "{link} touches cl1"
        );
    }
    assert_eq!(
        scalar::<String>(
            db,
            "SELECT VALUE last_error FROM ONLY insight_owner_state:['recon','run1']"
        )
        .await,
        Some("memory endpoint removed".into())
    );
    assert_eq!(
        scalar::<String>(
            db,
            "SELECT VALUE lifecycle FROM ONLY insight_owner_state:['recon','run2']"
        )
        .await,
        Some("active".into()),
        "an owner without deleted derivations stays active"
    );
    assert_eq!(
        scalar::<String>(
            db,
            "SELECT VALUE memory_id FROM ONLY insight_derivation_evidence:e1"
        )
        .await,
        None
    );
    assert_eq!(
        scalar::<String>(
            db,
            "SELECT VALUE memory_id FROM ONLY insight_derivation_evidence:e2"
        )
        .await,
        Some("m2".into())
    );
    assert_eq!(
        count(db, "insight_derivation_evidence").await,
        2,
        "evidence is retained"
    );
}
