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
