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
        let prepared =
            prepare_for_stage(store, article, &body, &claims, &inferences, &c.operation).unwrap();
        assert!(bp::commit_stage_with(
            &store.conn,
            &c,
            &out.to_string(),
            1,
            "",
            &now(),
            Some(&prepared)
        )
        .unwrap());
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
fn lead_index_failure_preserves_completed_claims_and_inferences() {
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
    assert_eq!(
        count, 3,
        "claims snapshot and cumulative inference snapshot survive"
    );
    let outbox: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM argos_index_changes", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        outbox, 3,
        "prior claim and inference indexing survives; failed lead rolls back"
    );
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
        .execute("INSERT INTO argos_memory_index_state(memory_id,revision,fingerprint,generation,indexed_at) SELECT record_id,revision,'fixture',1,?1 FROM argos_index_changes",[now()])
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
        snapshots, 2,
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
    assert_eq!(state.publication_version, PUBLICATION_VERSION);
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
        let prepared =
            prepare_for_stage(&store, &article, &body, &claims, &inferences, &c.operation).unwrap();
        assert!(bp::commit_stage_with(
            &store.conn,
            &c,
            &out.to_string(),
            1,
            "",
            &now(),
            Some(&prepared)
        )
        .unwrap());
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
    assert_eq!(
        gen, 2,
        "failed links preserve the two prior canonical publications"
    );
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
    assert!(
        prepared.output.readiness.publication_eligible,
        "valid completed extraction can replace the prior same-kind set with no faithful rows"
    );
    // Displayed body differing from the retained artifact cannot map spans.
    let mut other = body.clone();
    other["markdown"] = json!("different text");
    assert!(prepare(&store, &article, &other, &claims, &inferences)
        .unwrap()
        .is_err());
}

#[test]
fn claims_publish_before_body_and_survive_failed_inference_then_retry() {
    let (store, article) = fixture();
    let (body, claims, inferences) = outputs(&article);
    let job = bp::ensure_chain(
        &store.conn,
        &BriefInputs {
            article_id: article.id.clone(),
            input_revision: "claims-first".into(),
            role_snapshot: "{}".into(),
            title: article.title.clone(),
            ..Default::default()
        },
        &now(),
    )
    .unwrap();
    let body_task = bp::claim_stage(&store.conn, "body", 120, &now())
        .unwrap()
        .unwrap();
    let claims_task = bp::claim_stage(&store.conn, "claims", 120, &now())
        .unwrap()
        .unwrap();
    assert_eq!(claims_task.operation, "intel_brief_claims");
    let prepared = prepare_for_stage(
        &store,
        &article,
        &Value::Null,
        &claims,
        &Value::Null,
        &claims_task.operation,
    )
    .unwrap();
    assert!(
        prepared
            .as_ref()
            .unwrap()
            .output
            .readiness
            .publication_eligible
    );
    assert!(bp::commit_stage_with(
        &store.conn,
        &claims_task,
        &claims.to_string(),
        1,
        "",
        &now(),
        Some(&prepared)
    )
    .unwrap());
    let state = index_state(&store.conn, &article.id).unwrap().unwrap();
    assert_eq!(state.state, "indexed", "{state:?}");
    assert_eq!(
        state.required.len(),
        2,
        "claim and its canonical subject actor"
    );
    assert!(
        bp::published_brief(&store.conn, &article.id)
            .unwrap()
            .unwrap()
            .1["claims"]
            .as_array()
            .unwrap()
            .len()
            == 1
    );
    assert_eq!(
        footer(&store.conn, &article.id, &job, false).unwrap(),
        "Published · Index pending"
    );
    assert!(bp::commit_stage(&store.conn, &body_task, &body.to_string(), 1, "", &now()).unwrap());
    let failed = bp::claim_stage(&store.conn, "inference", 120, &now())
        .unwrap()
        .unwrap();
    store.conn.execute("UPDATE argos_tasks SET state='failed',error_message='fixture model timeout',lease_owner='',lease_until='' WHERE id=?1",[&failed.id]).unwrap();
    crate::tasks::refresh_job_state(&store.conn, &job, &now()).unwrap();
    assert_eq!(
        index_state(&store.conn, &article.id).unwrap().unwrap(),
        state
    );
    assert_eq!(
        footer(&store.conn, &article.id, &job, true).unwrap(),
        "Published · Index pending"
    );
    assert_eq!(
        bp::retry_stage(&store.conn, &job, &now()).unwrap(),
        Some(failed.id.clone())
    );
    let retry = bp::claim_stage(&store.conn, "retry", 120, &now())
        .unwrap()
        .unwrap();
    let prepared = prepare_for_stage(
        &store,
        &article,
        &body,
        &claims,
        &inferences,
        &retry.operation,
    )
    .unwrap();
    assert!(bp::commit_stage_with(
        &store.conn,
        &retry,
        &inferences.to_string(),
        1,
        "",
        &now(),
        Some(&prepared)
    )
    .unwrap());
    let current = index_state(&store.conn, &article.id).unwrap().unwrap();
    assert_eq!(current.required.len(), 3);
    assert!(
        current.required.contains(&state.required[0]),
        "claim id/revision survives inference replacement"
    );
    assert!(
        !bp::commit_stage_with(
            &store.conn,
            &failed,
            &inferences.to_string(),
            1,
            "",
            &now(),
            Some(&prepared)
        )
        .unwrap(),
        "old lease cannot republish"
    );
    assert_eq!(
        index_state(&store.conn, &article.id).unwrap().unwrap(),
        current
    );
}

#[test]
fn three_publication_points_enqueue_only_the_completed_kind_and_keep_premises() {
    let (store, article) = fixture();
    let mut candidate = article.clone();
    candidate.id = "a2".into();
    candidate.url = "https://example.test/a2".into();
    store.atlas_upsert_article(&candidate).unwrap();
    store.conn.execute_batch("CREATE TABLE brief_index_probe(kind TEXT); CREATE TRIGGER probe_brief_index AFTER INSERT ON argos_index_changes BEGIN INSERT INTO brief_index_probe SELECT memory_kind FROM memory_metadata WHERE memory_id=NEW.record_id; END;").unwrap();
    let links = json!({"leads":[{"inference_id":"i1","article_id":"a2","article_run_id":"fixture","metadata_revision":"r1","why":"Possible insurance context.","verification_needed":"Check original reporting.","metadata_evidence":[]}]});
    let job = complete_chain_with_leads(&store, &article, "three-stages", &links);
    let mut statement = store
        .conn
        .prepare("SELECT kind FROM brief_index_probe ORDER BY rowid")
        .unwrap();
    let kinds = statement
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        kinds,
        ["actor", "claim", "inference", "related"],
        "claim publication also indexes its new subject; later stages enqueue only their own kind"
    );
    let receipts: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM app_state WHERE key LIKE ?1",
            [format!("intel-brief-canonical-stage:{job}:%")],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(receipts, 3);
    let state = index_state(&store.conn, &article.id).unwrap().unwrap();
    assert_eq!(
        state.required.len(),
        4,
        "actors, claims, inferences and tentative leads all require index acknowledgements"
    );
    assert_eq!(
        store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM insight_snapshots WHERE owner_id=?1",
                [&article.id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2,
        "links reuse inference snapshot"
    );
    // A completed outbox row is insufficient when its exact revision has not
    // been acknowledged by the index writer.
    store
        .conn
        .execute("UPDATE argos_index_changes SET state='completed'", [])
        .unwrap();
    assert_eq!(
        footer(&store.conn, &article.id, &job, false).unwrap(),
        "Published · Index pending"
    );
    for receipt in &state.required {
        store.conn.execute("INSERT INTO argos_memory_index_state(memory_id,revision,indexed_at) VALUES(?1,?2,?3)",params![receipt.memory_id,receipt.revision,now()]).unwrap();
    }
    assert_eq!(
        footer(&store.conn, &article.id, &job, false).unwrap(),
        "Published · Indexed"
    );
    store
        .conn
        .execute(
            "UPDATE argos_memory_index_state SET revision='older' WHERE memory_id=?1",
            [&state.required[0].memory_id],
        )
        .unwrap();
    assert_eq!(
        footer(&store.conn, &article.id, &job, false).unwrap(),
        "Published · Index pending"
    );
    let before: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM brief_index_probe", [], |r| r.get(0))
        .unwrap();
    refresh_cached_publication(&store, &article, &job).unwrap();
    assert_eq!(
        store
            .conn
            .query_row("SELECT COUNT(*) FROM brief_index_probe", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        before,
        "cache refresh never requeues successful kinds"
    );
}

#[test]
fn focused_inference_publication_requires_actual_analysis_sentence_coverage() {
    let (store, article) = fixture();
    let (body, claims, mut inferences) = outputs(&article);
    inferences["path"] = json!("claim_context");
    inferences["analysis_sentence_ids"] = json!([]);
    let prepared = prepare(&store, &article, &body, &claims, &inferences)
        .unwrap()
        .unwrap();
    assert!(!prepared.output.readiness.publication_eligible);
    assert!(prepared
        .output
        .readiness
        .gaps
        .iter()
        .any(|g| g.contains("cited inference passage")));
    inferences["analysis_sentence_ids"] = json!(["b2"]);
    let prepared = prepare(&store, &article, &body, &claims, &inferences)
        .unwrap()
        .unwrap();
    assert!(
        prepared.output.readiness.publication_eligible,
        "{:?}",
        prepared.output.readiness.gaps
    );
    let synthesis = prepared
        .input
        .snapshot
        .coverage_by_stage
        .iter()
        .find(|c| c.stage == "synthesis")
        .unwrap();
    assert_eq!(synthesis.ranges.len(), 1);
    assert!(
        synthesis.ranges[0].end_byte < body["markdown"].as_str().unwrap().len(),
        "focused coverage never claims the entire story was sent"
    );
}

#[test]
fn cached_old_generation_cannot_replace_a_new_claim_publication() {
    let (store, article) = fixture();
    let old = complete_chain(&store, &article, "old");
    let new = bp::ensure_chain(
        &store.conn,
        &BriefInputs {
            article_id: article.id.clone(),
            input_revision: "new".into(),
            role_snapshot: "{}".into(),
            title: article.title.clone(),
            ..Default::default()
        },
        &now(),
    )
    .unwrap();
    let (body, claims, inferences) = outputs(&article);
    let _body_task = bp::claim_stage(&store.conn, "body", 120, &now())
        .unwrap()
        .unwrap();
    let claims_task = bp::claim_stage(&store.conn, "claims", 120, &now())
        .unwrap()
        .unwrap();
    let prepared = prepare_for_stage(
        &store,
        &article,
        &body,
        &claims,
        &inferences,
        &claims_task.operation,
    )
    .unwrap();
    assert!(bp::commit_stage_with(
        &store.conn,
        &claims_task,
        &claims.to_string(),
        1,
        "",
        &now(),
        Some(&prepared)
    )
    .unwrap());
    let state = index_state(&store.conn, &article.id).unwrap().unwrap();
    assert_eq!(state.job_id, new);
    refresh_cached_publication(&store, &article, &old).unwrap();
    assert_eq!(
        index_state(&store.conn, &article.id).unwrap().unwrap(),
        state
    );
    assert_eq!(
        bp::published_brief(&store.conn, &article.id)
            .unwrap()
            .unwrap()
            .0,
        2
    );
}

#[test]
fn carried_inference_keeps_original_provenance_and_empty_stages_replace_only_their_kind() {
    let (store, article) = fixture();
    complete_chain(&store, &article, "complete");
    let original = index_state(&store.conn, &article.id).unwrap().unwrap();
    let inference_memory:String=store.conn.query_row("SELECT memory_id FROM insight_derivations WHERE snapshot_id=?1 AND insight_kind='inference'",[&original.snapshot_id],|r|r.get(0)).unwrap();
    let claim_memory:String=store.conn.query_row("SELECT memory_id FROM insight_derivations WHERE snapshot_id=?1 AND insight_kind='claim'",[&original.snapshot_id],|r|r.get(0)).unwrap();
    let Some(crate::memory_metadata::MemoryPayload::Inference(payload)) =
        store.memory_payload(&inference_memory).unwrap()
    else {
        panic!("typed inference")
    };
    assert_eq!(payload.premise_claims, vec![claim_memory.clone()]);
    assert!(
        !payload.actors.is_empty(),
        "inference inherits explicit canonical premise actor associations"
    );
    for actor in &payload.actors {
        assert_eq!(actor.role, "premise");
        assert_eq!(store.conn.query_row("SELECT COUNT(*) FROM brain_memory_actor_refs WHERE memory_id=?1 AND actor_id=?2 AND role='premise'",params![inference_memory,actor.actor_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    }
    let job = bp::ensure_chain(
        &store.conn,
        &BriefInputs {
            article_id: article.id.clone(),
            input_revision: "empty".into(),
            role_snapshot: "{}".into(),
            title: article.title.clone(),
            ..Default::default()
        },
        &now(),
    )
    .unwrap();
    let (body, _, _) = outputs(&article);
    let body_task = bp::claim_stage(&store.conn, "body", 120, &now())
        .unwrap()
        .unwrap();
    let claims_task = bp::claim_stage(&store.conn, "claims", 120, &now())
        .unwrap()
        .unwrap();
    let claims = json!({"claims":[],"empty_reason":"No faithful assertions","withheld":0});
    let prepared = prepare_for_stage(
        &store,
        &article,
        &Value::Null,
        &claims,
        &Value::Null,
        &claims_task.operation,
    )
    .unwrap();
    assert!(bp::commit_stage_with(
        &store.conn,
        &claims_task,
        &claims.to_string(),
        0,
        "No faithful assertions",
        &now(),
        Some(&prepared)
    )
    .unwrap());
    let partial = index_state(&store.conn, &article.id).unwrap().unwrap();
    assert_eq!(partial.state, "indexed");
    let metadata: String = store
        .conn
        .query_row(
            "SELECT readiness_json FROM insight_snapshots WHERE id=?1",
            [&partial.snapshot_id],
            |r| r.get(0),
        )
        .unwrap();
    let metadata: Value = serde_json::from_str(&metadata).unwrap();
    let carried = &metadata["carried_kinds"][0];
    assert_eq!(carried["kind"], "inference");
    assert_eq!(carried["source_snapshot_id"], original.snapshot_id);
    assert!(
        carried["inputs"]["source_refs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["source_id"].as_str().unwrap_or("").starts_with("body:")),
        "historical body refs remain bound to their original snapshot"
    );
    let kinds:Vec<String>=store.conn.prepare("SELECT insight_kind FROM insight_derivations WHERE snapshot_id=?1 AND status='assessed' ORDER BY insight_kind").unwrap().query_map([&partial.snapshot_id],|r|r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(
        kinds,
        vec!["inference"],
        "empty Claims retire prior claim associations without touching Inferences"
    );
    assert!(
        store.memory_payload(&claim_memory).unwrap().is_some(),
        "a historical premise remains reachable from the carried inference"
    );
    assert!(bp::commit_stage(&store.conn, &body_task, &body.to_string(), 1, "", &now()).unwrap());
    let inference_task = bp::claim_stage(&store.conn, "inference", 120, &now())
        .unwrap()
        .unwrap();
    let inferences = json!({"inferences":[],"path":"claim_context","analysis_sentence_ids":[],"empty_reason":"No supported inference"});
    let prepared = prepare_for_stage(
        &store,
        &article,
        &body,
        &claims,
        &inferences,
        &inference_task.operation,
    )
    .unwrap();
    assert!(bp::commit_stage_with(
        &store.conn,
        &inference_task,
        &inferences.to_string(),
        0,
        "No supported inference",
        &now(),
        Some(&prepared)
    )
    .unwrap());
    let current = index_state(&store.conn, &article.id).unwrap().unwrap();
    assert_eq!(current.state, "indexed", "{current:?}");
    assert_eq!(
        store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM insight_derivations WHERE snapshot_id=?1",
                [&current.snapshot_id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert!(store.conn.query_row("SELECT EXISTS(SELECT 1 FROM argos_index_changes WHERE record_id=?1 AND operation='index_remove')",[&inference_memory],|r|r.get::<_,bool>(0)).unwrap());
    assert!(
        current
            .required
            .iter()
            .any(|receipt| receipt.memory_id == inference_memory
                && receipt.revision == crate::store::DELETED_REVISION),
        "empty replacement waits for exact inference removal acknowledgement"
    );
    assert_eq!(current.job_id, job);
}

#[test]
fn failed_inference_publication_cannot_be_hidden_by_a_successful_links_extraction() {
    let (store, article) = fixture();
    store.conn.execute_batch("CREATE TRIGGER reject_inference BEFORE INSERT ON memory_metadata WHEN NEW.memory_kind='inference' BEGIN SELECT RAISE(ABORT,'fixture inference publication failure'); END;").unwrap();
    let mut candidate = article.clone();
    candidate.id = "a2".into();
    candidate.url = "https://example.test/a2".into();
    store.atlas_upsert_article(&candidate).unwrap();
    let links = json!({"leads":[{"inference_id":"i1","article_id":"a2","article_run_id":"fixture","metadata_revision":"r1","why":"Potential context.","verification_needed":"Check reporting.","metadata_evidence":[]}]});
    let job = complete_chain_with_leads(&store, &article, "canonical-inference-failed", &links);
    let state = index_state(&store.conn, &article.id).unwrap().unwrap();
    assert_eq!(state.state, "failed");
    assert!(
        state
            .reason
            .contains("canonical inference publication unavailable"),
        "{state:?}"
    );
    assert!(footer(&store.conn, &article.id, &job, false)
        .unwrap()
        .contains("Index failed"));
    let claims_receipt: String = store
        .conn
        .query_row(
            "SELECT value FROM app_state WHERE key=?1",
            [format!(
                "intel-brief-canonical-stage:{job}:intel_brief_claims"
            )],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&claims_receipt).unwrap()["state"]["state"],
        "indexed",
        "prior Claims stay published"
    );
    assert_eq!(
        store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM memory_metadata WHERE memory_kind='related'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0,
        "missing inference endpoints never produce Related rows"
    );
}

#[test]
fn indexed_premise_receipts_bind_canonical_claim_memory_ids() {
    let (store, article) = fixture();
    complete_chain(&store, &article, "indexed-premises");
    let state = index_state(&store.conn, &article.id).unwrap().unwrap();
    let claim:String=store.conn.query_row("SELECT memory_id FROM insight_derivations WHERE snapshot_id=?1 AND insight_kind='claim'",[&state.snapshot_id],|r|r.get(0)).unwrap();
    let inference:String=store.conn.query_row("SELECT memory_id FROM insight_derivations WHERE snapshot_id=?1 AND insight_kind='inference'",[&state.snapshot_id],|r|r.get(0)).unwrap();
    let sql="SELECT DISTINCT d.memory_id FROM insight_derivation_evidence e JOIN insight_derivations d ON d.id=e.derivation_id WHERE e.memory_id=?1 AND e.premise_id<>'' AND d.insight_kind='inference' AND d.status='assessed'";
    let rows = store
        .conn
        .prepare(sql)
        .unwrap()
        .query_map([&claim], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(rows, vec![inference]);
    let plan = store
        .conn
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap()
        .query_map([&claim], |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert!(
        plan.iter()
            .any(|step| step.contains("insight_evidence_memory")),
        "{plan:?}"
    );
}

#[test]
fn tombstoned_claim_with_assessed_null_memory_never_publishes_an_orphan_inference() {
    let (store, article) = fixture();
    let (body, claims, inferences) = outputs(&article);
    bp::ensure_chain(
        &store.conn,
        &BriefInputs {
            article_id: article.id.clone(),
            input_revision: "before-delete".into(),
            role_snapshot: "{}".into(),
            ..Default::default()
        },
        &now(),
    )
    .unwrap();
    for output in [&body, &claims] {
        let task = bp::claim_stage(&store.conn, "first", 120, &now())
            .unwrap()
            .unwrap();
        let prepared = prepare_for_stage(
            &store,
            &article,
            &body,
            &claims,
            &inferences,
            &task.operation,
        )
        .unwrap();
        assert!(bp::commit_stage_with(
            &store.conn,
            &task,
            &output.to_string(),
            1,
            "",
            &now(),
            Some(&prepared)
        )
        .unwrap());
    }
    let state = index_state(&store.conn, &article.id).unwrap().unwrap();
    let claim_memory:String=store.conn.query_row("SELECT memory_id FROM insight_derivations WHERE snapshot_id=?1 AND insight_kind='claim'",[&state.snapshot_id],|r|r.get(0)).unwrap();
    store.delete_memory(&claim_memory).unwrap();
    let job = complete_chain(&store, &article, "after-delete");
    let receipt: String = store
        .conn
        .query_row(
            "SELECT value FROM app_state WHERE key=?1",
            [format!(
                "intel-brief-canonical-stage:{job}:intel_brief_inferences"
            )],
            |r| r.get(0),
        )
        .unwrap();
    let receipt: Value = serde_json::from_str(&receipt).unwrap();
    assert_eq!(receipt["state"]["state"], "failed");
    assert!(receipt["state"]["reason"]
        .as_str()
        .unwrap()
        .contains("canonical premise claim"));
    assert!(store.conn.query_row("SELECT EXISTS(SELECT 1 FROM insight_derivations d JOIN insight_snapshots s ON s.id=d.snapshot_id WHERE s.owner_id=?1 AND d.insight_kind='claim' AND d.status='assessed' AND d.memory_id IS NULL)",[&article.id],|r|r.get::<_,bool>(0)).unwrap());
    assert_eq!(
        store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM memory_metadata WHERE memory_kind='inference'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn canonical_inference_preserves_counterevidence_without_asserting_its_support() {
    let (store, article) = fixture();
    let (body, claims, mut inferences) = outputs(&article);
    let sentences = super::super::sentence_index::body_sentences(&body).0;
    let contrary = &sentences[2];
    inferences["inferences"][0]["counterevidence"] =
        json!([{"id":contrary.id,"start_byte":contrary.start_byte,"end_byte":contrary.end_byte}]);
    inferences["path"] = json!("claim_context");
    inferences["analysis_sentence_ids"] = json!(["b2", contrary.id]);
    let prepared = prepare(&store, &article, &body, &claims, &inferences)
        .unwrap()
        .unwrap();
    assert!(
        prepared.output.readiness.publication_eligible,
        "{:?}",
        prepared.output.readiness.gaps
    );
    assert_eq!(prepared.output.inferences[0].evidence_refs.len(), 2);
    assert!(prepared.output.inferences[0].evidence_refs.iter().any(|reference|matches!(reference.location,EvidenceLocation::Range(span) if span.start_byte==contrary.start_byte && span.end_byte==contrary.end_byte)));
    assert!(
        prepared.output.links.is_empty(),
        "retaining contrary provenance creates no corroboration assertion"
    );
}

#[test]
fn footer_reports_current_failed_disabled_and_deferred_index_work() {
    let (store, article) = fixture();
    let job = complete_chain(&store, &article, "index-outcomes");
    let state = index_state(&store.conn, &article.id).unwrap().unwrap();
    for receipt in &state.required {
        store.conn.execute("INSERT INTO argos_memory_index_state(memory_id,revision,indexed_at) VALUES(?1,?2,?3)",params![receipt.memory_id,receipt.revision,now()]).unwrap();
    }
    let target = &state.required[0];
    store
        .conn
        .execute(
            "UPDATE argos_memory_index_state SET revision='older' WHERE memory_id=?1",
            [&target.memory_id],
        )
        .unwrap();
    store.conn.execute("INSERT INTO argos_index_changes(record_kind,record_id,revision,operation,state,outcome,created_at,updated_at) VALUES('memory',?1,'older','index_upsert','completed','{}',?2,?2)",params![target.memory_id,now()]).unwrap();
    assert_eq!(
        footer(&store.conn, &article.id, &job, false).unwrap(),
        "Published · Index pending",
        "completed stale revision never acknowledges current required revision"
    );
    store.conn.execute("UPDATE argos_index_changes SET state='failed',error_message='vector store write failed',error_category='permanent' WHERE record_id=?1 AND revision=?2",params![target.memory_id,target.revision]).unwrap();
    assert_eq!(
        footer(&store.conn, &article.id, &job, false).unwrap(),
        "Published · Index failed · vector store write failed"
    );
    let disabled = crate::tasks::IndexOutcome::Disabled {
        reason: "semantic indexing disabled (ARGOS_EMBED=0)".into(),
    };
    store.conn.execute("UPDATE argos_index_changes SET state='blocked',error_message='',error_category='configuration_missing',outcome=?3 WHERE record_id=?1 AND revision=?2",params![target.memory_id,target.revision,disabled.to_json()]).unwrap();
    assert_eq!(
        footer(&store.conn, &article.id, &job, false).unwrap(),
        "Published · Not indexed · semantic indexing disabled (ARGOS_EMBED=0)"
    );
    store.conn.execute("UPDATE argos_index_changes SET state='deferred',outcome=?3,error_message='',error_category='' WHERE record_id=?1 AND revision=?2",params![target.memory_id,target.revision,json!({"status":"deferred","reason":"embedding model unavailable"}).to_string()]).unwrap();
    assert_eq!(
        footer(&store.conn, &article.id, &job, false).unwrap(),
        "Published · Not indexed · embedding model unavailable"
    );
    store
        .conn
        .execute(
            "UPDATE argos_index_changes SET state='superseded' WHERE record_id=?1 AND revision=?2",
            params![target.memory_id, target.revision],
        )
        .unwrap();
    assert_eq!(
        footer(&store.conn, &article.id, &job, false).unwrap(),
        "Published · Index pending",
        "superseded work cannot poison a current revision"
    );
    store
        .conn
        .execute(
            "UPDATE argos_memory_index_state SET revision=?2 WHERE memory_id=?1",
            params![target.memory_id, target.revision],
        )
        .unwrap();
    store.conn.execute("UPDATE argos_index_changes SET state='failed',error_message='stale failure' WHERE record_id=?1",[&target.memory_id]).unwrap();
    assert_eq!(
        footer(&store.conn, &article.id, &job, false).unwrap(),
        "Published · Indexed",
        "exact current acknowledgement wins over stale failed or blocked work"
    );
}

#[test]
fn first_claim_canonical_failure_is_not_reported_as_published() {
    let (store, article) = fixture();
    let (_, claims, _) = outputs(&article);
    store.conn.execute_batch("CREATE TRIGGER reject_claim BEFORE INSERT ON memory_metadata WHEN NEW.memory_kind='claim' BEGIN SELECT RAISE(ABORT,'fixture first claim publication failure'); END;").unwrap();
    let job = bp::ensure_chain(
        &store.conn,
        &BriefInputs {
            article_id: article.id.clone(),
            input_revision: "first-publication-failed".into(),
            role_snapshot: "{}".into(),
            ..Default::default()
        },
        &now(),
    )
    .unwrap();
    let _body = bp::claim_stage(&store.conn, "body", 120, &now())
        .unwrap()
        .unwrap();
    let task = bp::claim_stage(&store.conn, "claims", 120, &now())
        .unwrap()
        .unwrap();
    let prepared = prepare_for_stage(
        &store,
        &article,
        &Value::Null,
        &claims,
        &Value::Null,
        &task.operation,
    )
    .unwrap();
    assert!(bp::commit_stage_with(
        &store.conn,
        &task,
        &claims.to_string(),
        1,
        "",
        &now(),
        Some(&prepared)
    )
    .unwrap());
    assert!(
        bp::published_brief(&store.conn, &article.id)
            .unwrap()
            .is_some(),
        "validated extraction display committed"
    );
    assert!(index_state(&store.conn, &article.id)
        .unwrap()
        .unwrap()
        .snapshot_id
        .is_empty());
    assert_eq!(
        footer(&store.conn, &article.id, &job, false).unwrap(),
        "Not published · fixture first claim publication failure"
    );
    assert_eq!(
        store
            .conn
            .query_row("SELECT COUNT(*) FROM insight_snapshots", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0,
        "failed canonical transaction left no published snapshot"
    );
}

#[test]
fn earlier_successful_kind_receipt_preserves_published_status_after_later_failure() {
    let (store, article) = fixture();
    let job = complete_chain(&store, &article, "published-before-failure");
    store.conn.execute("UPDATE app_state SET value=json_set(value,'$.snapshot_id','','$.state','failed','$.reason','later kind failure') WHERE key=?1",[state_key(&article.id)]).unwrap();
    assert_eq!(footer(&store.conn,&article.id,&job,false).unwrap(),"Published · Index failed · later kind failure","successful earlier kind receipt confirms canonical publication even when a later footer omits its snapshot id");
}
