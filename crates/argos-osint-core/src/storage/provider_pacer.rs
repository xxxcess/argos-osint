//! Shared send clocks for model providers in a network workspace.

use std::time::Duration;

use surrealdb::engine::any::Any;
use surrealdb::types::Duration as DbDuration;
use surrealdb::{IndexedResults, Surreal};

use super::counters::HOT_RECORD_POLICY;
use super::jobs::{rid, take};
use super::tx::TxError;

pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS model_provider_pacer SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS not_before ON model_provider_pacer TYPE datetime;";

/// Reserve one immediate send, returning `None` on success. A returned wait
/// does not reserve capacity. All clients compare against the database clock.
pub async fn reserve(
    db: &Surreal<Any>,
    provider: &str,
    interval: Duration,
) -> Result<Option<Duration>, TxError> {
    let mut response = update_clock(
        db,
        provider,
        interval,
        "RETURN {
            LET $now = time::now();
            LET $next = (SELECT VALUE not_before FROM ONLY $row) ?? $now;
            IF $next > $now { $next - $now } ELSE {
                UPSERT $row SET not_before = $now + $delay RETURN NONE;
                NONE
            }
        };",
    )
    .await?;
    Ok(take::<Option<DbDuration>>(&mut response, 0)?.map(Duration::from))
}

/// Extend the provider-wide clock after a remote cooldown response. Concurrent
/// reservations and cooldowns serialize on the same provider record.
pub async fn defer(db: &Surreal<Any>, provider: &str, delay: Duration) -> Result<(), TxError> {
    update_clock(
        db,
        provider,
        delay,
        "RETURN {
            LET $until = time::now() + $delay;
            LET $next = SELECT VALUE not_before FROM ONLY $row;
            IF $next = NONE OR $until > $next {
                UPSERT $row SET not_before = $until RETURN NONE;
            };
        };",
    )
    .await?;
    Ok(())
}

// Keep the read and conditional write in one SurrealQL statement. If an
// awaiting caller is cancelled, there is no open transaction on its session.
async fn update_clock(
    db: &Surreal<Any>,
    provider: &str,
    delay: Duration,
    sql: &str,
) -> Result<IndexedResults, TxError> {
    let row = rid("model_provider_pacer", provider);
    let delay = DbDuration::from(delay);
    let policy = HOT_RECORD_POLICY;
    for attempt in 1..=policy.max_attempts {
        let response = db
            .query(sql)
            .bind(("row", row.clone()))
            .bind(("delay", delay))
            .await
            .and_then(IndexedResults::check)
            .map_err(TxError::from);
        match response {
            Err(error) if error.is_conflict() => {
                if attempt < policy.max_attempts {
                    tokio::time::sleep(policy.base).await;
                }
            }
            result => return result,
        }
    }
    Err(TxError::Contention {
        attempts: policy.max_attempts,
    })
}
