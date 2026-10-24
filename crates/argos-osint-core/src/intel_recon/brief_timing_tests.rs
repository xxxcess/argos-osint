use super::*;
use std::time::Duration;

fn claimed(id: &str, started: &str) -> ClaimedTask {
    ClaimedTask {
        id: id.into(),
        job_id: "job-t".into(),
        operation: "intel_brief_claims".into(),
        owner: "w".into(),
        epoch: 1,
        attempt: 1,
        max_attempts: 1,
        input_ref: String::new(),
        input_hash: String::new(),
        source_revision: String::new(),
        attempt_id: String::new(),
        started_at: started.into(),
    }
}

#[test]
fn labels_name_role_and_substep() {
    assert_eq!(role_label("claim_assessor"), "Claim Assessor");
    assert_eq!(role_label("evidence_curator"), "Evidence Curator");
    assert_eq!(substep_label("intel_brief_links"), "selecting links");
}

#[tokio::test]
async fn timed_futures_accumulate_on_their_own_clock_only() {
    let a = StageClock::default();
    let b = StageClock::default();
    let fa = with_clock(a.clone(), async {
        timed(Phase::Model, tokio::time::sleep(Duration::from_millis(30))).await;
    });
    let fb = with_clock(b.clone(), async {
        timed(
            Phase::Fidelity,
            tokio::time::sleep(Duration::from_millis(10)),
        )
        .await;
    });
    tokio::join!(fa, fb);
    assert!(peek(&a, Phase::Model) >= Duration::from_millis(30));
    assert_eq!(peek(&a, Phase::Fidelity), Duration::ZERO);
    assert_eq!(peek(&b, Phase::Model), Duration::ZERO);
    // Outside any clock, notes are dropped rather than misattributed.
    note(Phase::Model, Duration::from_secs(1));
    assert!(peek(&a, Phase::Model) < Duration::from_secs(1));
}
