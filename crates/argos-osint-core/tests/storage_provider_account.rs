//! Cross-session model account capacity and queue semantics.

#![cfg(feature = "surreal")]

mod support;

use std::time::Duration;

use argos_osint_core::storage::provider_account::{self, Admission, Ticket};
use support::Server;

const TTL: Duration = Duration::from_secs(60);

#[tokio::test]
async fn account_slots_respect_priority_fifo_and_lease_ownership() {
    let Some(server) = Server::start() else {
        return;
    };
    let a = server.root("argos", "provider_account").await;
    let b = server.root("argos", "provider_account").await;
    let account = "openrouter:fixture";
    let first = Ticket::new(account);
    let second = Ticket::new(account);
    let older_background = Ticket::new(account);
    let interactive = Ticket::new(account);
    let later_interactive = Ticket::new(account);

    first.register(&a, false, TTL).await.unwrap();
    assert_eq!(first.poll(&a, TTL).await.unwrap(), Admission::Active);
    second.register(&a, false, TTL).await.unwrap();
    older_background.register(&b, false, TTL).await.unwrap();
    let (second_result, background_result) =
        tokio::join!(second.poll(&a, TTL), older_background.poll(&b, TTL),);
    assert_eq!(second_result.unwrap(), Admission::Active);
    assert_eq!(background_result.unwrap(), Admission::Waiting);

    interactive.register(&b, true, TTL).await.unwrap();
    later_interactive.register(&a, true, TTL).await.unwrap();
    assert_eq!(interactive.poll(&b, TTL).await.unwrap(), Admission::Waiting);
    second.release(&a).await.unwrap();
    assert_eq!(
        older_background.poll(&b, TTL).await.unwrap(),
        Admission::Waiting
    );
    assert_eq!(
        later_interactive.poll(&a, TTL).await.unwrap(),
        Admission::Waiting
    );
    assert_eq!(interactive.poll(&b, TTL).await.unwrap(), Admission::Active);
    assert!(interactive.renew(&b, TTL).await.unwrap());
    assert!(!second.renew(&a, TTL).await.unwrap());

    // Drop the next waiting ticket. Expire the active ticket using the DB
    // clock, then prove an old owner cannot renew or release the replacement.
    later_interactive.release(&a).await.unwrap();
    assert!(interactive.renew(&b, Duration::ZERO).await.unwrap());
    assert_eq!(
        older_background.poll(&a, TTL).await.unwrap(),
        Admission::Active
    );
    assert!(!interactive.renew(&b, TTL).await.unwrap());
    interactive.release(&b).await.unwrap();
    assert!(older_background.renew(&a, TTL).await.unwrap());

    first.release(&a).await.unwrap();
    provider_account::defer(&a, account, TTL).await.unwrap();
    let cooled = Ticket::new(account);
    cooled.register(&b, true, TTL).await.unwrap();
    assert_eq!(cooled.poll(&b, TTL).await.unwrap(), Admission::Waiting);
    let other = Ticket::new("google:fixture");
    other.register(&a, false, TTL).await.unwrap();
    assert_eq!(other.poll(&a, TTL).await.unwrap(), Admission::Active);
}
