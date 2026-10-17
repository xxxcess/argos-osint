//! Real `spawn_intel_brief_drainer` against a temp `ARGOS_HOME` with no
//! provider account: Claims must fail visibly, downstream stages must stay
//! queued behind it, and the failure text must reach the UI-facing stage rows.
//! Own test binary, so setting `ARGOS_HOME` cannot race other tests.

use std::time::{Duration, Instant};

use argos_osint_core::intel_recon::brief_pipeline::{ensure_chain, stage_views, BriefInputs};
use argos_osint_core::scheduler::WorkerPool;
use argos_osint_core::store::{AtlasArticleRow, Store};

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[test]
fn no_account_claims_fail_visibly_and_downstream_stays_queued() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("ARGOS_HOME", home.path());
    std::env::remove_var("ARGOS_EMBED");
    let db = home.path().join("argos.db");
    let store = Store::open(&db).unwrap();
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.busy_timeout(Duration::from_secs(5)).unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO atlas_runs(id,started_at,state) VALUES('fixture',?1,'completed')",
        [now()],
    )
    .unwrap();
    store
        .atlas_upsert_article(&AtlasArticleRow {
            run_id: "fixture".into(),
            id: "a1".into(),
            title: "Ferry operator suspends Baltic route".into(),
            description: "The operator said storms forced the suspension.".into(),
            // Refused immediately: the body stage must not depend on the network.
            url: "http://127.0.0.1:1/a1".into(),
            country: String::new(),
            source_name: "Wire".into(),
            source_domain: "example.test".into(),
            published_at: "2026-10-12T08:00:00Z".into(),
            provider: "fixture".into(),
            temperature: 0.0,
            category: String::new(),
            seen_at: now(),
            author: String::new(),
            image_url: String::new(),
        })
        .unwrap();
    let job = ensure_chain(
        &conn,
        &BriefInputs {
            article_id: "a1".into(),
            input_revision: "r1".into(),
            role_snapshot: "{}".into(),
            title: "Ferry operator suspends Baltic route".into(),
            stage_revisions: vec![],
            metadata: Default::default(),
        },
        &now(),
    )
    .unwrap();

    let pool = WorkerPool::spawn_intel_brief_drainer(db.clone(), "test-owner".into());
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut rows = Vec::new();
    while Instant::now() < deadline {
        let views = stage_views(&conn, &job).unwrap();
        rows = views.iter().map(|v| v.row.clone()).collect();
        if views.len() == 4 && views[1].state == "failed" {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    pool.stop();

    assert!(
        rows[1].starts_with("Failed"),
        "Claims must fail without an account, got {rows:?}"
    );
    assert!(
        rows[1].len() > "Failed · ".len(),
        "failure text reaches the row"
    );
    assert!(
        rows[2].starts_with("Queued") && rows[3].starts_with("Queued"),
        "downstream stays queued, got {rows:?}"
    );
    assert!(rows[2].contains("blocked by"), "{rows:?}");
}
