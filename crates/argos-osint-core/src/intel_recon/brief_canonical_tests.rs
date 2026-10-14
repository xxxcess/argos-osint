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
        &json!({"leads":[]}).to_string(),
        0,
        "no matching articles",
        &now(),
        Some(&prepared)
    )
    .unwrap());
    job
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
fn completed_report_keeps_precedence_and_brief_stages_still_complete() {
    let (store, article) = fixture();
    store.conn.execute("INSERT INTO intel_report_results(job_id,generation,article_id,completion_sequence,source_hash,details_json,completed_at) VALUES('report',1,'a1',1,'h','{}',?1)", [now()]).unwrap();
    let job = complete_chain(&store, &article, "r1");
    let state = index_state(&store.conn, "a1").unwrap().unwrap();
    assert_eq!(state.state, "report_precedence");
    let snapshots: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM insight_snapshots WHERE owner_id='a1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(snapshots, 0, "brief never overwrites the report snapshot");
    let run: String = store
        .conn
        .query_row(
            "SELECT state FROM intel_brief_runs WHERE job_id=?1",
            [&job],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(run, "completed");
    assert!(footer(&store.conn, "a1", &job, false)
        .unwrap()
        .contains("Recon report takes precedence"));
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
