//! Provider send reservations and cooldowns shared by database clients.

#![cfg(feature = "surreal")]

mod support;

use std::time::Duration;

use argos_osint_core::storage::provider_pacer;
use surrealdb::engine::any::Any;
use surrealdb::types::Datetime;
use surrealdb::Surreal;

use support::Server;

async fn openrouter_clock(db: &Surreal<Any>) -> Datetime {
    db.query("SELECT VALUE not_before FROM ONLY model_provider_pacer:openrouter")
        .await
        .unwrap()
        .check()
        .unwrap()
        .take::<Option<Datetime>>(0)
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn clients_share_send_reservations_and_cooldowns() {
    let Some(server) = Server::start() else {
        return;
    };
    let a = server.root("argos", "provider_pacing").await;
    let b = server.root("argos", "provider_pacing").await;
    let interval = Duration::from_secs(60);

    let (first, second) = tokio::join!(
        provider_pacer::reserve(&a, "openrouter", interval),
        provider_pacer::reserve(&b, "openrouter", interval)
    );
    match (first.unwrap(), second.unwrap()) {
        (None, Some(wait)) | (Some(wait), None) => {
            assert!(!wait.is_zero() && wait <= interval);
        }
        result => panic!("exactly one client must reserve the send: {result:?}"),
    }

    let original = openrouter_clock(&a).await;
    let wait = provider_pacer::reserve(&b, "openrouter", interval)
        .await
        .unwrap();
    assert!(wait.is_some());
    assert_eq!(openrouter_clock(&b).await, original, "waiting uses no send");

    provider_pacer::defer(&b, "openrouter", Duration::from_secs(600))
        .await
        .unwrap();
    let extended = openrouter_clock(&a).await;
    assert!(extended > original);
    provider_pacer::defer(&a, "openrouter", Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(openrouter_clock(&b).await, extended);
    let wait = provider_pacer::reserve(&a, "openrouter", interval)
        .await
        .unwrap()
        .unwrap();
    assert!(wait > interval);

    assert_eq!(
        provider_pacer::reserve(&b, "google", interval)
            .await
            .unwrap(),
        None
    );
}
