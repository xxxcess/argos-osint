use super::*;
use crate::store::AtlasArticleRow;
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

#[test]
fn canonical_projection_preserves_true_support_kind_domains_and_conditions() {
    let article = AtlasArticleRow {
        id: "a1".into(),
        run_id: "r1".into(),
        title: "Baltic board suspended the ferry service.".into(),
        url: "https://example.invalid/ferry".into(),
        ..Default::default()
    };
    let (body, claims, mut inferences) = outputs(&article);
    inferences["inferences"][0]["inference_kind"] = json!("conditional_outlook");
    inferences["inferences"][0]["domains"] = json!(["security", "economic"]);
    inferences["inferences"][0]["conditions"] =
        json!(["If the insurance lapse prevented sailings"]);
    let evidence = |id: &str, text: String, complete: bool| RetainedEvidence {
        reference: EvidenceRef {
            source_id: id.into(),
            revision: "immutable-v1".into(),
            artifact_hash: crate::nlp::cache::hash(&text),
            acquisition_id: "acquisition-v1".into(),
            origin_root: if complete {
                "independent-body-root".into()
            } else {
                String::new()
            },
            url: article.url.clone(),
            published_at: None,
            retrieved_at: now(),
            location: EvidenceLocation::Range(crate::nlp::Span {
                start_byte: 0,
                end_byte: text.len(),
            }),
        },
        text,
        native: None,
        substantive: complete,
        relevant: true,
        complete_story: complete,
        known_truncation: !complete,
    };
    let retained = vec![
        evidence("article:a1", article.title.clone(), false),
        evidence("body:a1", body_text(), true),
    ];
    let input = PipelineInput {
        snapshot: crate::investigation::insights::snapshot("article", "a1", 1, &retained),
        evidence: retained,
        objective: "Isolated canonical contract".into(),
        surface: "article".into(),
        required_tasks_complete: true,
        save_insights: true,
        recall_enabled: false,
    };
    let projected = prepare_for_stage(
        input.clone(),
        &article,
        &body,
        &claims,
        &inferences,
        "intel_brief_inferences",
    )
    .unwrap();
    let inference = &projected.output.inferences[0];
    assert_eq!(
        inference.inference_kind,
        Some(crate::intelligence_contracts::InferenceKind::ConditionalOutlook)
    );
    assert_eq!(
        inference.domains,
        [
            crate::atlas_taxonomy::DomainId::Security,
            crate::atlas_taxonomy::DomainId::Economic
        ]
    );
    assert_eq!(inference.support_state, "conditional");
    assert_eq!(
        inference.conditions,
        ["If the insurance lapse prevented sailings"]
    );
    assert_eq!(inference.assessment.fidelity, "faithful");
    assert!(
        projected.output.readiness.publication_eligible,
        "{:?}",
        projected.output.readiness.gaps
    );
    inferences["inferences"][0]["conditions"] = json!([]);
    let invalid = prepare_for_stage(
        input,
        &article,
        &body,
        &claims,
        &inferences,
        "intel_brief_inferences",
    )
    .unwrap();
    assert!(!invalid.output.readiness.publication_eligible);
}
