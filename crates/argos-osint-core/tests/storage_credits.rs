//! Recon provider credits on SurrealDB (P3 item 5).
//! Run through `python3 scripts/test_surreal.py --mode both`.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::provider::ReconLimits;
use argos_osint_core::storage::credits::*;
use support::*;

#[tokio::test]
async fn holds_reconcile_release_and_reset_like_sqlite() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.root("argos", "credits").await;
    let mut limits = ReconLimits {
        hunter_credits: 2,
        hunter_trial_credits: 1,
        credit_reset: "never".into(),
        ..ReconLimits::default()
    };
    let hold = reserve_credits(&db, "hunter", 1, &limits)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((hold.trial_credits, hold.allowance_credits), (1, 0));
    reconcile_credits(&db, &hold, 1).await.unwrap();
    // A second reconcile of a settled hold is a no-op.
    reconcile_credits(&db, &hold, 1).await.unwrap();
    let hold = reserve_credits(&db, "hunter", 1, &limits)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(hold.allowance_credits, 1);
    release_credits(&db, &hold).await.unwrap();
    release_credits(&db, &hold).await.unwrap();
    for _ in 0..2 {
        let hold = reserve_credits(&db, "hunter", 1, &limits)
            .await
            .unwrap()
            .unwrap();
        reconcile_credits(&db, &hold, 1).await.unwrap();
    }
    assert!(reserve_credits(&db, "hunter", 1, &limits)
        .await
        .unwrap()
        .is_none());
    assert_eq!(credits_available(&db, "hunter", &limits).await.unwrap(), 0);
    // Zero cost holds nothing.
    let free = reserve_credits(&db, "hunter", 0, &limits)
        .await
        .unwrap()
        .unwrap();
    assert!(free.id.is_empty());

    db.query(
        "UPDATE provider_quota:hunter SET spent = 5, period_start = '2020-01-01T00:00:00+00:00'",
    )
    .await
    .unwrap()
    .check()
    .unwrap();
    limits.credit_reset = "monthly".into();
    assert_eq!(credits_available(&db, "hunter", &limits).await.unwrap(), 2);

    // Whoxy is a prepaid pool: never reset monthly.
    let whoxy = ReconLimits {
        whoxy_credits: 2,
        credit_reset: "monthly".into(),
        ..ReconLimits::default()
    };
    let hold = reserve_credits(&db, "whoxy", 1, &whoxy)
        .await
        .unwrap()
        .unwrap();
    reconcile_credits(&db, &hold, 1).await.unwrap();
    db.query(
        "UPDATE provider_quota:whoxy SET spent = 2, period_start = '2020-01-01T00:00:00+00:00'",
    )
    .await
    .unwrap()
    .check()
    .unwrap();
    assert_eq!(credits_available(&db, "whoxy", &whoxy).await.unwrap(), 0);
}

#[tokio::test]
async fn concurrent_holds_never_overdraw_and_recovery_releases_held() {
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.root("argos", "credits_race").await;
    let limits = ReconLimits {
        hunter_credits: 3,
        hunter_trial_credits: 0,
        credit_reset: "never".into(),
        ..ReconLimits::default()
    };
    credits_available(&db, "hunter", &limits).await.unwrap();
    let tasks: Vec<_> = (0..6)
        .map(|_| {
            let (db, limits) = (db.clone(), limits.clone());
            tokio::spawn(async move { reserve_credits(&db, "hunter", 1, &limits).await })
        })
        .collect();
    let mut held = 0;
    for t in tasks {
        if t.await.unwrap().unwrap().is_some() {
            held += 1;
        }
    }
    assert_eq!(held, 3, "exactly the pool size is held");
    assert_eq!(credits_available(&db, "hunter", &limits).await.unwrap(), 0);
    assert_eq!(release_held_credits(&db).await.unwrap(), 3);
    assert_eq!(release_held_credits(&db).await.unwrap(), 0);
    assert_eq!(credits_available(&db, "hunter", &limits).await.unwrap(), 3);
}
