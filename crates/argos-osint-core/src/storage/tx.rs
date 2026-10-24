//! Bounded-retry transactions over `Surreal<Any>`, identical in server and
//! embedded mode.
//!
//! SurrealDB uses snapshot isolation with optimistic conflict detection, so an
//! invariant that SQLite protected with `BEGIN IMMEDIATE` is protected here by
//! *guard records*: every transaction that must serialize against another one
//! writes the same guard record first, so at most one of them commits. The
//! loser sees a write conflict (normalized by [`super::error`]) and
//! [`run_tx`] retries the whole body with jittered exponential backoff. When
//! the attempts run out it returns [`TxError::Contention`]; it never spins
//! indefinitely.
//!
//! The body must be pure database work: no provider or model call belongs in
//! a closure that can run more than once.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use surrealdb::engine::any::Any;
use surrealdb::method::Transaction;
use surrealdb::{Error, Surreal};

use super::error;

/// One open transaction, shared with the body for the length of one attempt.
pub type Tx = Arc<Transaction<Any>>;

/// Retry limits for [`run_tx`].
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Total attempts, including the first (approved default: 6).
    pub max_attempts: u32,
    /// Backoff for the first retry; doubles per retry up to `cap`.
    pub base: Duration,
    pub cap: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy {
            max_attempts: 6,
            base: Duration::from_millis(5),
            cap: Duration::from_millis(200),
        }
    }
}

/// Why a transaction did not commit.
#[derive(Debug, Clone)]
pub enum TxError {
    /// A database error other than a retryable conflict.
    Db(Error),
    /// Every attempt hit a write conflict.
    Contention { attempts: u32 },
    /// The body chose to abort (validation failed); nothing was written.
    Abort(String),
    /// The publisher was shut down before accepting the write; it never
    /// committed.
    PublisherStopped,
    /// The publisher accepted the write but ended before replying; the write
    /// may have committed.
    PublisherDropped,
}

impl std::fmt::Display for TxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TxError::Db(e) => write!(f, "storage error: {e}"),
            TxError::Contention { attempts } => {
                write!(
                    f,
                    "storage contention: gave up after {attempts} conflicting attempts"
                )
            }
            TxError::Abort(why) => write!(f, "transaction aborted: {why}"),
            TxError::PublisherStopped => {
                write!(f, "memory publisher stopped; the write was not accepted")
            }
            TxError::PublisherDropped => write!(
                f,
                "memory publisher ended before replying; the write may have committed"
            ),
        }
    }
}

impl std::error::Error for TxError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TxError::Db(e) => Some(e),
            _ => None,
        }
    }
}

impl From<Error> for TxError {
    fn from(e: Error) -> Self {
        TxError::Db(error::normalize(e))
    }
}

impl TxError {
    pub fn is_conflict(&self) -> bool {
        matches!(self, TxError::Db(e) if error::is_conflict(e))
    }
}

/// A committed result and how many conflict retries it took.
#[derive(Debug)]
pub struct Committed<T> {
    pub value: T,
    pub retries: u32,
}

/// Run `body` in a transaction, retrying the whole body on a write conflict.
///
/// `body` gets the transaction handle and must drop every clone of it before
/// its future completes. `Ok` commits; `Err` cancels. A conflict raised by any
/// statement or by `COMMIT` itself is retried up to `policy.max_attempts`.
pub async fn run_tx<T, F, Fut>(
    db: &Surreal<Any>,
    policy: RetryPolicy,
    mut body: F,
) -> Result<Committed<T>, TxError>
where
    F: FnMut(Tx) -> Fut,
    Fut: Future<Output = Result<T, TxError>>,
{
    let attempts = policy.max_attempts.max(1);
    for attempt in 0..attempts {
        if attempt > 0 {
            tokio::time::sleep(backoff(policy, attempt)).await;
        }
        let tx: Tx = Arc::new(db.clone().begin().await.map_err(TxError::from)?);
        let result = body(tx.clone()).await;
        let tx = Arc::try_unwrap(tx)
            .unwrap_or_else(|_| panic!("run_tx body kept a transaction handle past its attempt"));
        match result {
            Ok(value) => match tx.commit().await.map_err(TxError::from) {
                Ok(_) => {
                    return Ok(Committed {
                        value,
                        retries: attempt,
                    })
                }
                Err(e) if e.is_conflict() => continue,
                Err(e) => return Err(e),
            },
            Err(e) => {
                let _ = tx.cancel().await;
                if e.is_conflict() {
                    continue;
                }
                return Err(e);
            }
        }
    }
    Err(TxError::Contention { attempts })
}

/// Full-jitter exponential backoff: uniform in `[0, min(cap, base * 2^(n-1))]`.
fn backoff(policy: RetryPolicy, retry: u32) -> Duration {
    let ceiling = policy
        .base
        .saturating_mul(1u32 << (retry - 1).min(16))
        .min(policy.cap);
    let nanos = ceiling.as_nanos() as u64;
    if nanos == 0 {
        return Duration::ZERO;
    }
    Duration::from_nanos(jitter() % (nanos + 1))
}

fn jitter() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default(),
    );
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_is_bounded_by_the_cap() {
        let p = RetryPolicy::default();
        for n in 1..40 {
            assert!(backoff(p, n) <= p.cap);
        }
        assert!(backoff(p, 1) <= p.base);
    }
}

/// Read helpers that run on the client or inside a caller's transaction
/// (one consistent snapshot), e.g. the topology snapshot.
pub trait Reads: Sync {
    fn read<'c>(
        &'c self,
        sql: impl Into<std::borrow::Cow<'c, str>>,
    ) -> surrealdb::method::Query<'c, Any>;
}

impl Reads for Surreal<Any> {
    fn read<'c>(
        &'c self,
        sql: impl Into<std::borrow::Cow<'c, str>>,
    ) -> surrealdb::method::Query<'c, Any> {
        self.query(sql)
    }
}

impl Reads for Transaction<Any> {
    fn read<'c>(
        &'c self,
        sql: impl Into<std::borrow::Cow<'c, str>>,
    ) -> surrealdb::method::Query<'c, Any> {
        self.query(sql)
    }
}
