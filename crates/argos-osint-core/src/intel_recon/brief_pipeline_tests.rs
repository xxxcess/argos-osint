use super::*;
use crate::store::Store;

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
fn at(offset_secs: i64) -> String {
    (chrono::Utc::now() + chrono::Duration::seconds(offset_secs)).to_rfc3339()
}
fn seeded() -> Store {
    let store = Store::memory().unwrap();
    seed(&store);
    store
}
fn seed(store: &Store) {
    let now = now();
    store.conn.execute("INSERT OR IGNORE INTO atlas_runs(id,started_at,state) VALUES('fixture',?1,'completed')", [&now]).unwrap();
    store.conn.execute("INSERT INTO atlas_articles(run_id,article_id,title,description,url,country,source_name,source_domain,published_at,provider,temperature,seen_at) VALUES('fixture','a1','Ferry suspension','','https://example.test/a1','','','','','fixture',0,?1)", [&now]).unwrap();
}
fn inputs(rev: &str) -> BriefInputs {
    BriefInputs {
        article_id: "a1".into(),
        input_revision: rev.into(),
        role_snapshot: "{}".into(),
        title: "Ferry suspension".into(),
    }
}
fn state(store: &Store, id: &str) -> String {
    store
        .conn
        .query_row("SELECT state FROM argos_tasks WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .unwrap()
}
fn rows(store: &Store, job: &str) -> Vec<String> {
    stage_views(&store.conn, job)
        .unwrap()
        .into_iter()
        .map(|v| v.row)
        .collect()
}

#[test]
fn repeated_admission_creates_one_chain_and_stages_dispatch_strictly_in_order() {
    let store = seeded();
    let job = ensure_chain(&store.conn, &inputs("r1"), &now()).unwrap();
    // Visits / reopen / body-ready / recovery all attach to the same chain.
    for _ in 0..4 {
        assert_eq!(
            ensure_chain(&store.conn, &inputs("r1"), &now()).unwrap(),
            job
        );
    }
    let count: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM argos_tasks WHERE job_id=?1",
            [&job],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 4, "all four rows exist immediately");
    assert_eq!(
        rows(&store, &job),
        [
            "Queued",
            "Queued · after Full article",
            "Queued · after Claims",
            "Queued · after Inferences"
        ]
    );
    // Unrelated drainers never see brief stages.
    assert!(
        tasks::claim_next_in(&store.conn, &[], "generic", 120, &now())
            .unwrap()
            .is_none()
    );
    assert!(
        tasks::claim_next_in(&store.conn, &[tasks::POOL_LLM], "llm", 120, &now())
            .unwrap()
            .is_none()
    );
    let mut order = Vec::new();
    for _ in 0..4 {
        let claimed = claim_stage(&store.conn, "w", 120, &now()).unwrap().unwrap();
        // Exactly one active stage: nothing else is claimable meanwhile.
        assert!(claim_stage(&store.conn, "w2", 120, &now())
            .unwrap()
            .is_none());
        let views = stage_views(&store.conn, &job).unwrap();
        assert_eq!(views.iter().filter(|v| v.active).count(), 1);
        order.push(claimed.operation.clone());
        let items = if claimed.operation == "intel_brief_links" {
            0
        } else {
            2
        };
        let reason = if items == 0 {
            "no matching articles"
        } else {
            ""
        };
        assert!(commit_stage(
            &store.conn,
            &claimed,
            &format!("{{\"stage\":\"{}\"}}", claimed.operation),
            items,
            reason,
            &now()
        )
        .unwrap());
    }
    assert_eq!(
        order,
        STAGES
            .iter()
            .map(|(op, _)| op.to_string())
            .collect::<Vec<_>>()
    );
    assert!(claim_stage(&store.conn, "w", 120, &now())
        .unwrap()
        .is_none());
    assert_eq!(
        rows(&store, &job),
        [
            "Completed · 2 items",
            "Completed · 2 items",
            "Completed · 2 items",
            "Completed · no matching articles"
        ]
    );
    // Successor inputs are bound to the predecessor's committed hash.
    let (bound, hash): (String, String) = store.conn.query_row(
        "SELECT t.input_hash,o.output_hash FROM argos_tasks t JOIN intel_brief_stage_outputs o ON o.task_id=?2 WHERE t.id=?1",
        [task_id(&job, "intel_brief_claims"), task_id(&job, "intel_brief_body")],
        |r| Ok((r.get(0)?, r.get(1)?)),
    ).unwrap();
    assert_eq!(bound, hash);
    // Completed unchanged visit: same chain, nothing claimable (zero requests).
    assert_eq!(
        ensure_chain(&store.conn, &inputs("r1"), &now()).unwrap(),
        job
    );
    assert!(claim_stage(&store.conn, "w", 120, &now())
        .unwrap()
        .is_none());
}

#[test]
fn failed_stage_blocks_dependents_preserves_predecessors_and_retry_resumes_only_it() {
    let store = seeded();
    let job = ensure_chain(&store.conn, &inputs("r1"), &now()).unwrap();
    for _ in 0..2 {
        let c = claim_stage(&store.conn, "w", 120, &now()).unwrap().unwrap();
        assert!(commit_stage(&store.conn, &c, "{}", 1, "", &now()).unwrap());
    }
    let c = claim_stage(&store.conn, "w", 120, &now()).unwrap().unwrap();
    assert_eq!(c.operation, "intel_brief_inferences");
    store.conn.execute("UPDATE argos_tasks SET state='failed',error_message='token_limit: finish_reason=length; more',lease_owner='',lease_until='' WHERE id=?1", [&c.id]).unwrap();
    assert_eq!(
        rows(&store, &job),
        [
            "Completed · 1 item",
            "Completed · 1 item",
            "Failed · token_limit: finish_reason=length",
            "Queued · blocked by Inferences"
        ]
    );
    // Links stay blocked with no lease; reopening does not reset the chain.
    assert!(claim_stage(&store.conn, "w", 120, &now())
        .unwrap()
        .is_none());
    assert_eq!(
        ensure_chain(&store.conn, &inputs("r1"), &now()).unwrap(),
        job
    );
    assert_eq!(state(&store, &c.id), "failed");
    // Retry resumes Stage 3 only, then continues.
    assert_eq!(
        retry_stage(&store.conn, &job, &now()).unwrap(),
        Some(c.id.clone())
    );
    let again = claim_stage(&store.conn, "w", 120, &now()).unwrap().unwrap();
    assert_eq!(again.operation, "intel_brief_inferences");
    assert_eq!(
        state(&store, &task_id(&job, "intel_brief_body")),
        "completed"
    );
    assert!(commit_stage(
        &store.conn,
        &again,
        "{}",
        0,
        "no supported inferences",
        &now()
    )
    .unwrap());
    let links = claim_stage(&store.conn, "w", 120, &now()).unwrap().unwrap();
    assert_eq!(links.operation, "intel_brief_links");
}

#[test]
fn renewal_between_reaper_selection_and_update_keeps_the_task_running() {
    let store = seeded();
    let job = ensure_chain(&store.conn, &inputs("r1"), &now()).unwrap();
    let c = claim_stage(&store.conn, "w", 120, &now()).unwrap().unwrap();
    // The lease expired from the reaper's point of view (its clock is later).
    store
        .conn
        .execute(
            "UPDATE argos_tasks SET lease_until=?2 WHERE id=?1",
            params![c.id, at(-1)],
        )
        .unwrap();
    let reaper_now = at(1);
    let renew_now = at(-5);
    let mut renewed = false;
    let n = tasks::interrupt_expired_leases_in(&store.conn, &reaper_now, &mut || {
        renewed =
            tasks::renew_lease(&store.conn, &c.id, &c.owner, c.epoch, 120, &renew_now).unwrap();
    })
    .unwrap();
    assert!(renewed, "worker renewed its unexpired lease");
    assert_eq!(n, 0, "renewed lease must not be reaped");
    assert_eq!(state(&store, &c.id), "running");
    assert!(commit_stage(&store.conn, &c, "{}", 1, "", &now()).unwrap());
    let _ = job;
}

#[test]
fn renewal_requires_current_unexpired_ownership() {
    let store = seeded();
    ensure_chain(&store.conn, &inputs("r1"), &now()).unwrap();
    let c = claim_stage(&store.conn, "w", 120, &now()).unwrap().unwrap();
    assert!(!tasks::renew_lease(&store.conn, &c.id, "other", c.epoch, 120, &now()).unwrap());
    assert!(!tasks::renew_lease(&store.conn, &c.id, &c.owner, c.epoch + 1, 120, &now()).unwrap());
    assert!(
        !tasks::renew_lease(&store.conn, &c.id, &c.owner, c.epoch, 120, &at(500)).unwrap(),
        "expired lease cannot be revived"
    );
    assert!(tasks::renew_lease(&store.conn, &c.id, &c.owner, c.epoch, 120, &now()).unwrap());
}

#[test]
fn crash_recovery_fences_stale_worker_refunds_attempt_and_is_bounded() {
    let store = seeded();
    let job = ensure_chain(&store.conn, &inputs("r1"), &now()).unwrap();
    store
        .conn
        .execute(
            "UPDATE argos_tasks SET max_attempts=1 WHERE job_id=?1",
            [&job],
        )
        .unwrap();
    let first = claim_stage(&store.conn, "w", 120, &now()).unwrap().unwrap();
    // Genuine crash: no heartbeat; lease expires.
    assert_eq!(
        tasks::interrupt_expired_leases(&store.conn, &at(200)).unwrap(),
        1
    );
    let (st, attempts, recoveries, epoch, category): (String, i64, i64, i64, String) = store.conn.query_row(
        "SELECT state,attempts,recoveries,lease_epoch,error_category FROM argos_tasks WHERE id=?1",
        [&first.id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    ).unwrap();
    assert_eq!(
        st, "queued",
        "max_attempts=1 is not a permanent failure on first expiry"
    );
    assert_eq!(
        (attempts, recoveries),
        (0, 1),
        "worker recovery is not a provider attempt"
    );
    assert!(epoch > first.epoch, "recovery advances the epoch");
    assert_eq!(category, "lease_expired");
    let outcome: String = store
        .conn
        .query_row(
            "SELECT outcome FROM argos_attempts WHERE task_id=?1 AND worker_epoch=?2",
            params![first.id, first.epoch],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(outcome, "lease_expired");
    // Stale late output from the crashed worker writes nothing.
    assert!(!commit_stage(&store.conn, &first, "{\"stale\":true}", 9, "", &now()).unwrap());
    assert!(!tasks::renew_lease(
        &store.conn,
        &first.id,
        &first.owner,
        first.epoch,
        120,
        &now()
    )
    .unwrap());
    let outputs: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM intel_brief_stage_outputs", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(outputs, 0);
    // Reclaim from the checkpoint; bounded infrastructure recovery.
    for round in 2..=tasks::MAX_WORKER_RECOVERIES {
        let c = claim_stage(&store.conn, "w", 120, &now()).unwrap().unwrap();
        assert_eq!(c.id, first.id);
        tasks::interrupt_expired_leases(&store.conn, &at(200 * round)).unwrap();
    }
    let (st, category): (String, String) = store
        .conn
        .query_row(
            "SELECT state,error_category FROM argos_tasks WHERE id=?1",
            [&first.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (st.as_str(), category.as_str()),
        ("failed", "worker_recovery_exhausted")
    );
}

#[test]
fn new_inputs_supersede_and_cancellation_stops_the_chain() {
    let store = seeded();
    let old = ensure_chain(&store.conn, &inputs("r1"), &now()).unwrap();
    let c = claim_stage(&store.conn, "w", 120, &now()).unwrap().unwrap();
    let new = ensure_chain(&store.conn, &inputs("r2"), &now()).unwrap();
    assert_ne!(old, new);
    assert_eq!(state(&store, &c.id), "superseded");
    assert!(
        !commit_stage(&store.conn, &c, "{}", 1, "", &now()).unwrap(),
        "stale chain cannot commit"
    );
    let active: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM intel_brief_runs WHERE article_id='a1' AND state='active'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(active, 1);
    let c2 = claim_stage(&store.conn, "w", 120, &now()).unwrap().unwrap();
    assert_eq!(c2.job_id, new);
    cancel_chain(&store.conn, &new, &now()).unwrap();
    assert!(!commit_stage(&store.conn, &c2, "{}", 1, "", &now()).unwrap());
    assert!(claim_stage(&store.conn, "w", 120, &now())
        .unwrap()
        .is_none());
}

#[test]
fn overlapping_drainers_never_double_claim_a_stage() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("brief.db");
    let store = Store::open(&path).unwrap();
    seed(&store);
    ensure_chain(&store.conn, &inputs("r1"), &now()).unwrap();
    let handles: Vec<_> = (0..6)
        .map(|i| {
            let path = path.clone();
            std::thread::spawn(move || {
                let s = Store::open(&path).unwrap();
                s.conn
                    .busy_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                claim_stage(&s.conn, &format!("w{i}"), 120, &now())
                    .unwrap()
                    .is_some()
            })
        })
        .collect();
    let claimed = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .filter(|c| *c)
        .count();
    assert_eq!(claimed, 1);
}
