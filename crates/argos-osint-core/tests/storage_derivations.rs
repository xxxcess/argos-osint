//! Canonical derivation writes and article association retirement on
//! SurrealDB (migration phase 4b, part 1), through the `publish` step of
//! `insight::publish_snapshot_on`. Both modes via `scripts/test_surreal.py`.
#![cfg(feature = "surreal")]

mod support;

use std::sync::Arc;

use argos_osint_core::investigation::insights::{
    claim_fingerprint, snapshot, Assessment, EvidenceLocation, EvidenceRef, Inference,
    PipelineInput, PipelineOutput, SourceClaim,
};
use argos_osint_core::storage::derivations::{derivation_step, DerivationContext};
use argos_osint_core::storage::insight::{advance_generation, publish_snapshot_on, SnapshotStep};
use argos_osint_core::storage::tx::{run_tx, RetryPolicy};
use support::*;
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

async fn scalar<T: SurrealValue>(db: &Surreal<Any>, sql: &str) -> T {
    let mut r = db.query(sql).await.unwrap().check().unwrap();
    r.take::<Option<T>>(0usize)
        .unwrap()
        .unwrap_or_else(|| panic!("NONE: {sql}"))
}

fn reference(source: &str) -> EvidenceRef {
    EvidenceRef {
        source_id: source.into(),
        revision: "1".into(),
        artifact_hash: "h".into(),
        acquisition_id: "acq".into(),
        origin_root: "root".into(),
        url: "https://example.test/a".into(),
        published_at: None,
        retrieved_at: String::new(),
        location: EvidenceLocation::JsonPointer("/0".into()),
    }
}

fn claim(id: &str, object: &str, faithful: bool) -> SourceClaim {
    SourceClaim {
        support_receipts: Vec::new(),
        id: id.into(),
        subject_ref: "e:acme".into(),
        predicate: "opened".into(),
        object: object.into(),
        text: format!("Acme opened {object}."),
        polarity: "positive".into(),
        modality: "asserted".into(),
        attributed_to: "Acme".into(),
        temporal_scope: String::new(),
        evidence_refs: vec![reference("body:a1")],
        extraction_scope: "fixture".into(),
        assessment: Assessment {
            fidelity: if faithful { "faithful" } else { "unfaithful" }.into(),
            threshold_passed: true,
            ..Default::default()
        },
    }
}

fn input(article: &str, generation: u32, save: bool) -> PipelineInput {
    PipelineInput {
        snapshot: snapshot("article", article, generation, &[]),
        evidence: vec![],
        objective: "fixture".into(),
        surface: "article".into(),
        required_tasks_complete: true,
        save_insights: save,
        recall_enabled: false,
    }
}

/// Advance the owner's generation and publish one snapshot.
async fn publish(db: &Surreal<Any>, article: &str, output: PipelineOutput, ctx: DerivationContext) {
    let generation = run_tx(db, RetryPolicy::default(), |tx| {
        let article = article.to_string();
        async move { advance_generation(&tx, "article", &article, "now").await }
    })
    .await
    .unwrap()
    .value;
    let input = Arc::new(input(article, generation as u32, true));
    let step = derivation_step(input.clone(), Arc::new(output), None, ctx);
    let ok: SnapshotStep = Arc::new(|_, _| Box::pin(async { Ok(()) }));
    let published = run_tx(db, RetryPolicy::default(), |tx| {
        let (input, step, ok) = (input.clone(), step.clone(), ok.clone());
        async move {
            publish_snapshot_on(
                &tx,
                &input.snapshot,
                &input.snapshot.key,
                "active",
                "2026-10-23T00:00:00Z",
                &ok,
                &step,
            )
            .await
        }
    })
    .await
    .unwrap()
    .value;
    assert!(published.is_some(), "snapshot published");
}

fn saving() -> DerivationContext {
    DerivationContext {
        save_insights: true,
        recon_thread: None,
    }
}

#[tokio::test]
async fn derivations_write_memories_evidence_and_retire_on_refresh() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "derivations").await;
    let db = repo.db().clone();
    let kept = claim("c1", "office", true);
    let rejected = claim("c2", "nothing", false);
    let fp = claim_fingerprint(&kept);
    let inference = Inference {
        id: "i1".into(),
        text: "Acme is expanding.".into(),
        entity_refs: vec!["e:acme".into()],
        premise_claim_ids: vec!["c1".into()],
        evidence_refs: vec![reference("body:a1")],
        rationale: "office".into(),
        limitations: vec![],
        assessment: Assessment {
            fidelity: "faithful".into(),
            threshold_passed: true,
            ..Default::default()
        },
        snapshot_key: String::new(),
    };
    let output = PipelineOutput {
        claims: vec![kept.clone(), rejected],
        inferences: vec![inference],
        ..Default::default()
    };
    publish(&db, "a1", output, saving()).await;

    let mid = format!("mem:{fp}");
    let q = |sql: String| sql.replace("$M", &format!("type::record('memory', '{mid}')"));
    assert_eq!(
        scalar::<String>(&db, &q("SELECT VALUE memory_kind FROM ONLY $M".into())).await,
        "claim"
    );
    assert_eq!(
        scalar::<String>(&db, &q("SELECT VALUE category FROM ONLY $M".into())).await,
        "investigation"
    );
    assert_eq!(
        scalar::<String>(
            &db,
            &format!("SELECT VALUE memory_id FROM ONLY type::record('insight_claim', '{fp}')")
        )
        .await,
        mid
    );
    assert_eq!(
        scalar::<i64>(&db, "RETURN count(SELECT id FROM memory)").await,
        2
    );
    assert_eq!(
        scalar::<i64>(
            &db,
            "RETURN count(SELECT id FROM insight_source WHERE call_id = 'body:a1')"
        )
        .await,
        2
    );
    assert_eq!(
        scalar::<i64>(
            &db,
            "RETURN count(SELECT id FROM insight_derivation WHERE status = 'assessed')"
        )
        .await,
        2
    );
    assert_eq!(scalar::<i64>(&db, "RETURN count(SELECT id FROM insight_derivation WHERE status = 'rejected' AND memory_id = NONE)").await, 1);
    // One evidence row per item ref, one inherited premise row, no memory refs.
    assert_eq!(
        scalar::<i64>(
            &db,
            "RETURN count(SELECT id FROM insight_derivation_evidence WHERE premise_id = '')"
        )
        .await,
        3
    );
    assert_eq!(
        scalar::<String>(&db, "RETURN (SELECT VALUE memory_id FROM insight_derivation_evidence WHERE premise_id = 'c1')[0]").await,
        mid
    );
    assert!(scalar::<String>(
        &db,
        "RETURN (SELECT VALUE readiness_json FROM insight_snapshot)[0]"
    )
    .await
    .contains("brief-rating-v2"));
    // Index work and the change counter joined the publication.
    assert_eq!(
        scalar::<i64>(&db, "RETURN count(SELECT id FROM index_change)").await,
        2
    );
    assert_eq!(
        scalar::<i64>(
            &db,
            "RETURN (SELECT VALUE n FROM ONLY type::record('counter', 'memories_changed'))"
        )
        .await,
        1
    );

    // Refresh keeps the claim but drops the inference: its exclusively
    // owned memory is deleted, the claim memory stays, old rows retire.
    publish(
        &db,
        "a1",
        PipelineOutput {
            claims: vec![kept],
            ..Default::default()
        },
        saving(),
    )
    .await;
    assert_eq!(
        scalar::<i64>(&db, "RETURN count(SELECT id FROM memory)").await,
        1
    );
    assert_eq!(
        scalar::<i64>(&db, "RETURN count(SELECT id FROM insight_claim)").await,
        1
    );
    assert_eq!(
        scalar::<i64>(
            &db,
            "RETURN count(SELECT id FROM insight_derivation WHERE status = 'retired')"
        )
        .await,
        3
    );
    assert_eq!(
        scalar::<i64>(&db, "RETURN count(SELECT id FROM insight_source)").await,
        1
    );
    assert!(scalar::<bool>(&db, &q("RETURN record::exists($M)".into())).await);

    // An empty refresh deletes the article-owned claim memory too.
    publish(&db, "a1", PipelineOutput::default(), saving()).await;
    assert_eq!(
        scalar::<i64>(&db, "RETURN count(SELECT id FROM memory)").await,
        0
    );
    assert_eq!(
        scalar::<i64>(&db, "RETURN count(SELECT id FROM insight_claim)").await,
        0
    );
}

#[tokio::test]
async fn tombstones_and_disabled_saving_write_derivations_only() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "derivations_off").await;
    let db = repo.db().clone();
    let tomb = claim("c1", "office", true);
    let fp = claim_fingerprint(&tomb);
    db.query(
        "CREATE memory_tombstone SET memory_id = 'gone', fingerprint = $fp, created_at = 'now'",
    )
    .bind(("fp", fp))
    .await
    .unwrap()
    .check()
    .unwrap();
    publish(
        &db,
        "a1",
        PipelineOutput {
            claims: vec![tomb],
            ..Default::default()
        },
        saving(),
    )
    .await;
    // Saving rechecked off by the caller (e.g. the Recon opt-in was revoked).
    let off = DerivationContext {
        save_insights: false,
        recon_thread: None,
    };
    publish(
        &db,
        "a2",
        PipelineOutput {
            claims: vec![claim("c9", "store", true)],
            ..Default::default()
        },
        off,
    )
    .await;
    assert_eq!(
        scalar::<i64>(&db, "RETURN count(SELECT id FROM memory)").await,
        0
    );
    assert_eq!(
        scalar::<i64>(&db, "RETURN count(SELECT id FROM insight_source)").await,
        0
    );
    assert_eq!(scalar::<i64>(&db, "RETURN count(SELECT id FROM insight_derivation WHERE status = 'assessed' AND memory_id = NONE)").await, 2);
    assert_eq!(
        scalar::<i64>(&db, "RETURN count(SELECT id FROM index_change)").await,
        0
    );
}
