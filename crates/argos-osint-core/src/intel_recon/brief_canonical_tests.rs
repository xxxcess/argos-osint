use super::*;
use crate::intel_recon::brief_pipeline::{self as bp, BriefInputs};
use serde_json::json;

const HEAD: &str = "Baltic board suspended the ferry service on Monday. The harbour master confirmed the insurance certificate expired last week. Officials said the crossing would stay closed until the regulator reviews the lapse.";

fn body_text() -> String {
    format!(
        "{HEAD}\n\n{}",
        "The regulator explained the review timetable, the operator's insurance obligations and the attribution of each statement in detail. ".repeat(30)
    )
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Article with a retained, complete, cleaned acquisition (the same receipts
/// Stage 1 records in production).
fn fixture() -> (Store, AtlasArticleRow) {
    let store = Store::memory().unwrap();
    let at = now();
    store.conn.execute("INSERT OR IGNORE INTO atlas_runs(id,started_at,state) VALUES('fixture',?1,'completed')", [&at]).unwrap();
    store.conn.execute("INSERT INTO atlas_articles(run_id,article_id,title,description,url,country,source_name,source_domain,published_at,provider,temperature,seen_at) VALUES('fixture','a1','Baltic board suspends ferry','The board suspended the ferry service.','https://example.test/a1','lt','Baltic Wire','example.test','2026-10-05T00:00:00Z','fixture',0,?1)", [&at]).unwrap();
    let article = store.atlas_article("fixture", "a1").unwrap().unwrap();
    let body = store
        .ensure_article_body(
            &article.id,
            &article.run_id,
            &article.url,
            &article.source_domain,
            "",
        )
        .unwrap();
    let body_s = body_text();
    let raw = format!("{}\n\n{body_s}", article.title);
    store
        .commit_article_body(
            &body.id,
            body_s.as_str(),
            "h",
            "complete",
            "",
            &article.url,
            "fixture",
            false,
        )
        .unwrap();
    let version = store
        .article_body_by_id(&body.id)
        .unwrap()
        .unwrap()
        .body_version
        .to_string();
    for (transform, text) in [
        ("validated-baseline-v1", body_s.as_str()),
        (crate::intel_recon::clean_body::TRANSFORM, body_s.as_str()),
    ] {
        let mut source = crate::nlp::Source::derived(&body.id, &version, text);
        source.attempt_id = "attempt".into();
        source.raw_hash = Some(crate::nlp::cache::hash(&raw));
        crate::nlp::cache::register(
            &store.conn,
            "article",
            &article.id,
            &source,
            Some(&raw),
            transform,
        )
        .unwrap();
    }
    store
        .app_state_set(
            "article-acquisition:attempt",
            &json!({"artifact_hash":crate::nlp::cache::hash(&raw),"complete_story":true,"known_truncation":false}).to_string(),
        )
        .unwrap();
    (store, article)
}

fn outputs(article: &AtlasArticleRow) -> (Value, Value, Value) {
    let t = article.title.len();
    let body_s = body_text();
    let s2 = body_s.find("The harbour").unwrap();
    let e2 = body_s.find(" Officials").unwrap();
    (
        json!({"markdown":body_s,"content_hash":crate::nlp::cache::hash(&body_s),"quality":"complete","diagnostics":[]}),
        json!({"claims":[
            {"id":"c1","text":"Baltic board suspended the ferry service.","subject_id":"baltic board","predicate":"suspended","object":"ferry service","attributed_to":"Baltic board","modality":"asserted","polarity":"positive","time":"","evidence_ids":["t1"],"evidence":[{"id":"t1","start_byte":0,"end_byte":t}],"fidelity":"faithful"}
        ],"withheld":1}),
        json!({"inferences":[
            {"id":"i1","text":"The suspension may reflect an insurance lapse.","premise_claim_ids":["c1"],"body_evidence_ids":["b2"],"support":"conditional","why":"The harbour master confirmed the certificate expired.","limitations":["Whether the lapse stopped sailings is unconfirmed"],"evidence":[{"id":"b2","start_byte":s2,"end_byte":e2}],"counterevidence":[],"fidelity":"faithful"}
        ],"withheld":0}),
    )
}

/// Drive the chain to Stage 4 and commit it with prepared canonical inputs.
fn complete_chain(store: &Store, article: &AtlasArticleRow, rev: &str) -> String {
    complete_chain_with_leads(store, article, rev, &json!({"leads":[]}))
}

fn complete_chain_with_leads(
    store: &Store,
    article: &AtlasArticleRow,
    rev: &str,
    links: &Value,
) -> String {
    let job = bp::ensure_chain(
        &store.conn,
        &BriefInputs {
            article_id: article.id.clone(),
            input_revision: rev.into(),
            role_snapshot: "{}".into(),
            title: article.title.clone(),
            stage_revisions: vec![],
            metadata: Default::default(),
        },
        &now(),
    )
    .unwrap();
    let (body, claims, inferences) = outputs(article);
    for out in [&body, &claims, &inferences] {
        let c = bp::claim_stage(&store.conn, "w", 120, &now())
            .unwrap()
            .unwrap();
        assert!(bp::commit_stage(&store.conn, &c, &out.to_string(), 1, "", &now()).unwrap());
    }
    let c = bp::claim_stage(&store.conn, "w", 120, &now())
        .unwrap()
        .unwrap();
    assert_eq!(c.operation, "intel_brief_links");
    let prepared = prepare(store, article, &body, &claims, &inferences).unwrap();
    assert!(bp::commit_stage_with(
        &store.conn,
        &c,
        &links.to_string(),
        0,
        "no matching articles",
        &now(),
        Some(&prepared)
    )
    .unwrap());
    job
}

#[test]
fn brief_article_leads_enter_brain_as_tentative_related_and_retire_on_replacement() {
    let (store, article) = fixture();
    let mut candidate = article.clone();
    candidate.id = "a2".into();
    candidate.url = "https://example.test/a2".into();
    candidate.title = "Insurance certificate expired before the suspension".into();
    store.atlas_upsert_article(&candidate).unwrap();
    let links = json!({"leads":[{"inference_id":"i1","article_id":"a2","article_run_id":"fixture",
        "metadata_revision":"revision","why":"Potential matching insurance detail.",
        "verification_needed":"Verify the cause before corroborating.","metadata_evidence":[]}]});
    let job = complete_chain_with_leads(&store, &article, "r1", &links);
    let state = index_state(&store.conn, &article.id).unwrap().unwrap();
    assert_eq!(state.state, "indexed", "{state:?}");
    let (id,payload):(String,String)=store.conn.query_row("SELECT memory_id,payload_json FROM memory_metadata WHERE memory_kind='related' AND lifecycle='active'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let payload: crate::memory_metadata::MemoryPayload = serde_json::from_str(&payload).unwrap();
    let crate::memory_metadata::MemoryPayload::Related(lead) = payload else {
        panic!("tentative Related expected")
    };
    assert!(lead.verification_gap.contains("Verify the cause"));
    let claim_inference:i64=store.conn.query_row("SELECT COUNT(*) FROM insight_derivations WHERE snapshot_id=?1 AND memory_id IS NOT NULL",[&state.snapshot_id],|r|r.get(0)).unwrap();
    assert_eq!(claim_inference, 2);
    let crate::memory_metadata::EndpointRef::Memory { memory_id, kind } = lead.source else {
        panic!("canonical inference endpoint expected")
    };
    assert_eq!(kind, crate::memory_metadata::MemoryKind::Inference);
    let Some(crate::memory_metadata::MemoryPayload::Inference(inference)) =
        store.memory_payload(&memory_id).unwrap()
    else {
        panic!("typed inference metadata must exist before reopening")
    };
    assert_eq!(inference.premise_claims.len(), 1);
    assert!(inference.reasoning.contains("harbour master"));
    let indexed: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM argos_index_changes WHERE record_id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(indexed > 0, "lead enqueued alongside claims/inferences");
    refresh_cached_publication(&store, &article, &job).unwrap();
    let count: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM memory_metadata WHERE memory_kind='related'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "repeat publication is idempotent");
    store
        .conn
        .execute(
            "DELETE FROM intel_inference_article_leads WHERE article_id='a1'",
            [],
        )
        .unwrap();
    publish_leads(&store.conn, "a1", &state.snapshot_id, &now()).unwrap();
    let lifecycle: String = store
        .conn
        .query_row(
            "SELECT lifecycle FROM memory_metadata WHERE memory_id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lifecycle, "retired");
}

#[test]
fn deleting_a_lead_owner_retires_its_related_record_even_when_endpoints_survive() {
    use crate::memory_metadata::{EndpointRef, RelatedPayload};
    let store = Store::memory().unwrap();
    let source = EndpointRef::Source {
        origin: "article".into(),
        source_id: "retained".into(),
        revision: "r1".into(),
    };
    let (id, _) = crate::brain_actors::write_related(
        &store.conn,
        &RelatedPayload {
            source: source.clone(),
            target: source,
            shared_actors: vec![],
            basis: "possible context".into(),
            interpretation: "tentative".into(),
            verification_gap: "check original".into(),
            source_revisions: vec![],
        },
        &now(),
    )
    .unwrap();
    store
        .app_state_set(
            "intel-brief-brain-leads:deleted-owner",
            &json!([id]).to_string(),
        )
        .unwrap();
    insight_persist::reconcile_deleted_articles(&store.conn).unwrap();
    let lifecycle: String = store
        .conn
        .query_row(
            "SELECT lifecycle FROM memory_metadata WHERE memory_id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lifecycle, "retired");
    assert!(
        store.memory_payload(&id).unwrap().is_some(),
        "historical record retained"
    );
}

#[test]
fn lead_index_failure_rolls_back_canonical_claims_inferences_and_outbox() {
    let (store, article) = fixture();
    let mut candidate = article.clone();
    candidate.id = "a2".into();
    candidate.url = "https://example.test/a2".into();
    store.atlas_upsert_article(&candidate).unwrap();
    store.conn.execute_batch("CREATE TRIGGER reject_brief_lead BEFORE INSERT ON memory_metadata WHEN NEW.memory_kind='related' BEGIN SELECT RAISE(ABORT,'fixture lead failure'); END;").unwrap();
    let links = json!({"leads":[{"inference_id":"i1","article_id":"a2","article_run_id":"fixture",
        "metadata_revision":"revision","why":"Potential insurance match.","verification_needed":"Verify cause.","metadata_evidence":[]}]});
    let job = complete_chain_with_leads(&store, &article, "r1", &links);
    let state = index_state(&store.conn, &article.id).unwrap().unwrap();
    assert_eq!(state.state, "failed");
    assert!(state.reason.contains("fixture lead failure"));
    let count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM insight_derivations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    let outbox: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM argos_index_changes", [], |r| r.get(0))
        .unwrap();
    assert_eq!(outbox, 0, "no partial Brain indexing");
    assert!(footer(&store.conn, &article.id, &job, false)
        .unwrap()
        .starts_with("Published"));
}

#[test]
fn brief_publishes_canonical_derivations_and_index_outbox_through_shared_publisher() {
    let (store, article) = fixture();
    let job = complete_chain(&store, &article, "r1");
    let state = index_state(&store.conn, "a1").unwrap().unwrap();
    assert_eq!(state.state, "indexed", "{state:?}");
    let (active, lifecycle): (String, String) = store.conn.query_row("SELECT active_snapshot_id,lifecycle FROM insight_owner_state WHERE owner_kind='article' AND owner_id='a1'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(active, state.snapshot_id);
    assert_eq!(lifecycle, "active");
    let derivations: i64 = store.conn.query_row("SELECT COUNT(*) FROM insight_derivations WHERE snapshot_id=?1 AND memory_id IS NOT NULL", [&active], |r| r.get(0)).unwrap();
    assert_eq!(
        derivations, 2,
        "faithful claim + inference become canonical Brain rows"
    );
    let outbox: i64 = store.conn.query_row("SELECT COUNT(*) FROM argos_index_changes WHERE record_id IN (SELECT memory_id FROM insight_derivations WHERE snapshot_id=?1)", [&active], |r| r.get(0)).unwrap();
    assert!(outbox >= 2, "existing index outbox enqueued");
    assert_eq!(
        footer(&store.conn, "a1", &job, false).unwrap(),
        "Published · Index pending"
    );
    store
        .conn
        .execute("UPDATE argos_index_changes SET state='completed'", [])
        .unwrap();
    assert_eq!(
        footer(&store.conn, "a1", &job, false).unwrap(),
        "Published · Indexed"
    );
    // Provenance: each derivation cites the retained body/metadata passages.
    let evidence: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM insight_derivation_evidence WHERE derivation_id LIKE ?1",
            [format!("{active}:%")],
            |r| r.get(0),
        )
        .unwrap();
    assert!(evidence >= 1);
}

#[test]
fn historical_report_does_not_block_brief_publication() {
    let (store, article) = fixture();
    store.conn.execute("INSERT INTO intel_report_results(job_id,generation,article_id,completion_sequence,source_hash,details_json,completed_at) VALUES('report',1,'a1',1,'h','{}',?1)", [now()]).unwrap();
    let job = complete_chain(&store, &article, "r1");
    let state = index_state(&store.conn, "a1").unwrap().unwrap();
    assert_eq!(state.state, "indexed");
    let snapshots: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM insight_snapshots WHERE owner_id='a1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        snapshots, 1,
        "validated brief publishes alongside historical artifacts"
    );
    let run: String = store
        .conn
        .query_row(
            "SELECT state FROM intel_brief_runs WHERE job_id=?1",
            [&job],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(run, "completed");
    assert_eq!(
        footer(&store.conn, "a1", &job, false).unwrap(),
        "Published · Index pending"
    );
}

#[test]
fn publication_policy_refresh_reuses_cached_stage_outputs_once() {
    let (store, article) = fixture();
    let job = complete_chain(&store, &article, "cached");
    store
        .conn
        .execute(
            "UPDATE app_state SET value=json_remove(value,'$.publication_version') WHERE key=?1",
            [state_key(&article.id)],
        )
        .unwrap();
    let outputs = bp::stage_outputs(&store.conn, &job).unwrap();
    refresh_cached_publication(&store, &article, &job).unwrap();
    let state = index_state(&store.conn, &article.id).unwrap().unwrap();
    assert_eq!(state.publication_version, 3);
    assert_eq!(state.state, "indexed");
    let snapshot = state.snapshot_id;
    refresh_cached_publication(&store, &article, &job).unwrap();
    assert_eq!(
        index_state(&store.conn, &article.id)
            .unwrap()
            .unwrap()
            .snapshot_id,
        snapshot
    );
    assert_eq!(
        bp::stage_outputs(&store.conn, &job).unwrap(),
        outputs,
        "no stage is rerun"
    );
}

#[test]
fn canonical_failure_rolls_back_without_failing_the_stage() {
    let (store, article) = fixture();
    let (body, claims, inferences) = outputs(&article);
    let job = bp::ensure_chain(
        &store.conn,
        &BriefInputs {
            article_id: "a1".into(),
            input_revision: "r1".into(),
            role_snapshot: "{}".into(),
            title: String::new(),
            stage_revisions: vec![],
            metadata: Default::default(),
        },
        &now(),
    )
    .unwrap();
    for out in [&body, &claims, &inferences] {
        let c = bp::claim_stage(&store.conn, "w", 120, &now())
            .unwrap()
            .unwrap();
        assert!(bp::commit_stage(&store.conn, &c, &out.to_string(), 1, "", &now()).unwrap());
    }
    let prepared = prepare(&store, &article, &body, &claims, &inferences).unwrap();
    // Body revision changes between preparation and commit: live evidence fails.
    store
        .conn
        .execute("UPDATE article_bodies SET body_version=body_version+1", [])
        .unwrap();
    let c = bp::claim_stage(&store.conn, "w", 120, &now())
        .unwrap()
        .unwrap();
    assert!(bp::commit_stage_with(
        &store.conn,
        &c,
        &json!({"leads":[]}).to_string(),
        0,
        "",
        &now(),
        Some(&prepared)
    )
    .unwrap());
    let state = index_state(&store.conn, "a1").unwrap().unwrap();
    assert_eq!(state.state, "failed", "{state:?}");
    let gen: i64 = store.conn.query_row("SELECT COALESCE(MAX(desired_generation),0) FROM insight_owner_state WHERE owner_id='a1'", [], |r| r.get(0)).unwrap();
    assert_eq!(gen, 0, "savepoint rolled back the generation bump");
    assert!(footer(&store.conn, "a1", &job, false)
        .unwrap()
        .starts_with("Published · Index failed"));
    assert!(bp::stage_views(&store.conn, &job)
        .unwrap()
        .iter()
        .all(|v| v.state == "completed"));
}

#[test]
fn unfaithful_or_uncovered_briefs_are_not_indexed() {
    let (store, article) = fixture();
    let (body, mut claims, mut inferences) = outputs(&article);
    claims["claims"][0]["fidelity"] = json!("unfaithful");
    inferences["inferences"][0]
        .as_object_mut()
        .unwrap()
        .remove("fidelity");
    let prepared = prepare(&store, &article, &body, &claims, &inferences)
        .unwrap()
        .unwrap();
    assert!(prepared.output.claims.is_empty() && prepared.output.inferences.is_empty());
    assert!(!prepared.output.readiness.publication_eligible);
    // Displayed body differing from the retained artifact cannot map spans.
    let mut other = body.clone();
    other["markdown"] = json!("different text");
    assert!(prepare(&store, &article, &other, &claims, &inferences)
        .unwrap()
        .is_err());
}
