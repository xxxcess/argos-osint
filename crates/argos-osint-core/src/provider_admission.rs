//! Shared admission for one configured model account in a network workspace.

use std::future::{self, Future};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::time::Instant;

use crate::provider_diag::{Category, ProviderFailure, Stage};
use crate::secrets::ProviderSecret;
use crate::storage::provider_account::{self, Admission, Ticket};
use crate::storage::repo::Repository;
use crate::storage::{codec, runtime};

const TTL: Duration = Duration::from_secs(30);
const RENEW_INTERVAL: Duration = Duration::from_secs(10);
const QUEUE_POLL: Duration = Duration::from_millis(250);
const STORE_TIMEOUT: Duration = Duration::from_secs(5);
const DIRECT_WAIT: Duration = Duration::from_secs(120);

tokio::task_local! {
    static LEASE_ACCOUNT: String;
}

pub(crate) fn shared() -> bool {
    #[cfg(not(test))]
    {
        runtime::uses_network_storage()
    }
    #[cfg(test)]
    false
}

/// Account identities come from configuration and endpoint origin alone.
pub(crate) fn account_key(secret: &ProviderSecret, configured_account: &str) -> String {
    let provider = crate::provider::effective_kind(secret);
    let origin = url::Url::parse(&secret.base_url)
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_default();
    let account = if configured_account.trim().is_empty() {
        provider.clone()
    } else {
        crate::provider::normalize_kind(configured_account)
    };
    codec::encode(&[&provider, &origin, &account])
}

pub(crate) struct Lease {
    account: String,
    repo: Repository,
    ticket: Ticket,
    renew_by: Instant,
}

fn failure(category: Category, detail: &str) -> ProviderFailure {
    ProviderFailure::new(Stage::Admission, category, detail)
}

async fn stored<T, E>(op: impl Future<Output = Result<T, E>>) -> Result<T, ProviderFailure> {
    match tokio::time::timeout(STORE_TIMEOUT, op).await {
        Ok(Ok(value)) => Ok(value),
        _ => Err(failure(
            Category::Persistence,
            "shared model account admission storage is unavailable",
        )),
    }
}

async fn cancellation(cancel: Option<&AtomicBool>) {
    let Some(cancel) = cancel else {
        return future::pending().await;
    };
    while !cancel.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

pub(crate) async fn acquire(
    account: &str,
    interactive: bool,
    remaining: Option<Duration>,
    cancel: Option<&AtomicBool>,
) -> Result<Lease, ProviderFailure> {
    let deadline = async {
        match remaining {
            Some(left) => tokio::time::sleep(left).await,
            None => future::pending().await,
        }
    };
    let acquire = async {
        let mut lease = Lease {
            account: account.to_string(),
            repo: stored(runtime::repository()).await?,
            ticket: Ticket::new(account),
            renew_by: Instant::now(),
        };
        // Own cleanup before registration. An unknown commit after timeout
        // cannot leave a waiter or slot live beyond its database TTL.
        stored(lease.ticket.register(lease.repo.db(), interactive, TTL)).await?;
        loop {
            let polled_at = Instant::now();
            match stored(lease.ticket.poll(lease.repo.db(), TTL)).await? {
                Admission::Active => {
                    lease.renew_by = polled_at + RENEW_INTERVAL;
                    return Ok(lease);
                }
                Admission::Waiting => tokio::time::sleep(QUEUE_POLL).await,
                Admission::Lost => {
                    return Err(failure(
                        Category::Persistence,
                        "model account wait ticket expired",
                    ))
                }
            }
        }
    };
    tokio::select! {
        biased;
        _ = cancellation(cancel) => Err(failure(Category::Cancelled, "cancelled while waiting for a shared model account slot")),
        _ = deadline => Err(failure(Category::Timeout, "timed out waiting for a shared model account slot")),
        result = acquire => result,
    }
}

impl Lease {
    /// Keep the lease through the entire attempt, including wire pacing.
    /// Stop polling the attempt when the account can no longer renew it.
    pub(crate) async fn run<T>(
        &self,
        operation: impl Future<Output = T>,
        cancel: Option<&AtomicBool>,
    ) -> Result<T, ProviderFailure> {
        let mut operation = std::pin::pin!(LEASE_ACCOUNT.scope(self.account.clone(), operation));
        let mut renew_by = self.renew_by;
        loop {
            tokio::select! {
                biased;
                _ = cancellation(cancel) => return Err(failure(Category::Cancelled, "cancelled during a shared model account attempt")),
                _ = tokio::time::sleep_until(renew_by) => {
                    let polled_at = Instant::now();
                    if !stored(self.ticket.renew(self.repo.db(), TTL)).await? {
                        return Err(failure(Category::Persistence, "shared model account lease was lost"));
                    }
                    renew_by = polled_at + RENEW_INTERVAL;
                }
                value = &mut operation => return Ok(value),
            }
        }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let (repo, ticket) = (self.repo.clone(), self.ticket.clone());
        runtime::spawn_ordered(async move {
            let _ = stored(ticket.release(repo.db())).await;
        });
    }
}

/// Bare transports have the provider's default account. A routed attempt
/// already holds its configured account lease for the same provider origin.
pub(crate) async fn direct<T>(
    secret: &ProviderSecret,
    operation: impl Future<Output = T>,
) -> Result<T, ProviderFailure> {
    if !shared() {
        return Ok(operation.await);
    }
    let account = account_key(secret, "");
    let origin = codec::decode(&account).expect("account key was just encoded");
    let covered = LEASE_ACCOUNT
        .try_with(|key| codec::decode(key).is_some_and(|key| key.get(..2) == origin.get(..2)))
        .unwrap_or(false);
    if covered {
        return Ok(operation.await);
    }
    let lease = acquire(&account, false, Some(DIRECT_WAIT), None).await?;
    lease.run(operation, None).await
}

pub(crate) async fn defer(account: &str, delay: Duration) {
    let result = stored(async {
        let repo = runtime::repository().await?;
        Ok::<_, anyhow::Error>(provider_account::defer(repo.db(), account, delay).await?)
    })
    .await;
    if result.is_err() {
        tracing::warn!("could not persist shared model account cooldown");
    }
}
