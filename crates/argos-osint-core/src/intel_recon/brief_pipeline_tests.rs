use super::*;

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
fn at(offset_secs: i64) -> String {
    (chrono::Utc::now() + chrono::Duration::seconds(offset_secs)).to_rfc3339()
}
fn inputs(rev: &str) -> BriefInputs {
    BriefInputs {
        article_id: "a1".into(),
        input_revision: rev.into(),
        role_snapshot: "{}".into(),
        title: "Ferry suspension".into(),
        stage_revisions: vec![],
        metadata: Default::default(),
    }
}

fn revs(rev: &str, nodes: [&str; 4]) -> BriefInputs {
    BriefInputs {
        stage_revisions: nodes.iter().map(|s| s.to_string()).collect(),
        ..inputs(rev)
    }
}

// Phase A: the chain's provider backoff is credited to the task's retry wait
// (fenced by epoch) and rolled into the job.
