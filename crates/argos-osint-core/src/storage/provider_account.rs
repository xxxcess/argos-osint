//! Database-clock tickets for shared account admission.

use std::time::Duration;

use sha2::{Digest, Sha256};
use surrealdb::engine::any::Any;
use surrealdb::types::{Duration as DbDuration, RecordId};
use surrealdb::{IndexedResults, Surreal};

use super::counters::HOT_RECORD_POLICY;
use super::jobs::{rid, take};
use super::tx::TxError;

pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS model_account_gate SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS rev ON model_account_gate TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS cooldown_until ON model_account_gate TYPE option<datetime>;
DEFINE TABLE IF NOT EXISTS model_account_ticket SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS account ON model_account_ticket TYPE record<model_account_gate>;
DEFINE FIELD IF NOT EXISTS state ON model_account_ticket TYPE 'waiting' | 'active';
DEFINE FIELD IF NOT EXISTS priority ON model_account_ticket TYPE int;
DEFINE FIELD IF NOT EXISTS queued_at ON model_account_ticket TYPE datetime;
DEFINE FIELD IF NOT EXISTS expires_at ON model_account_ticket TYPE datetime;
DEFINE INDEX IF NOT EXISTS model_account_expiry ON model_account_ticket FIELDS account, expires_at;
DEFINE INDEX IF NOT EXISTS model_account_queue ON model_account_ticket FIELDS account, state, priority, queued_at;";

#[derive(Clone)]
pub struct Ticket {
    gate: RecordId,
    id: RecordId,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Admission {
    Waiting,
    Active,
    Lost,
}

fn gate(account: &str) -> RecordId {
    rid(
        "model_account_gate",
        &format!("{:x}", Sha256::digest(account.as_bytes())),
    )
}

impl Ticket {
    /// Mint an immutable owner token for a configured account identity.
    pub fn new(account: &str) -> Self {
        Self {
            gate: gate(account),
            id: rid("model_account_ticket", &crate::store::new_id()),
        }
    }

    /// Register the waiter before it can contend for an active slot.
    pub async fn register(
        &self,
        db: &Surreal<Any>,
        interactive: bool,
        ttl: Duration,
    ) -> Result<(), TxError> {
        apply(
            db,
            &self.gate,
            Some(&self.id),
            if interactive { 1 } else { 0 },
            ttl,
            "RETURN {
                UPSERT $gate SET rev = (rev ?? 0) + 1 RETURN NONE;
                LET $now = time::now();
                DELETE model_account_ticket WHERE account = $gate AND expires_at <= $now RETURN NONE;
                CREATE $ticket SET account = $gate, state = 'waiting', priority = $priority,
                    queued_at = $now, expires_at = $now + $ttl RETURN NONE;
            };",
        )
        .await?;
        Ok(())
    }

    /// Refresh a live waiter, promoting the highest-priority oldest ticket
    /// if the account has a free slot and the cooldown has expired.
    pub async fn poll(&self, db: &Surreal<Any>, ttl: Duration) -> Result<Admission, TxError> {
        let mut response = apply(
            db,
            &self.gate,
            Some(&self.id),
            0,
            ttl,
            "RETURN {
                UPSERT $gate SET rev = (rev ?? 0) + 1 RETURN NONE;
                LET $now = time::now();
                LET $live = SELECT VALUE id FROM ONLY $ticket
                    WHERE account = $gate AND state = 'waiting' AND expires_at > $now;
                IF $live = NONE { 'lost' } ELSE {
                    UPDATE $ticket SET expires_at = $now + $ttl RETURN NONE;
                    LET $cooldown = (SELECT VALUE cooldown_until FROM ONLY $gate) ?? $now;
                    LET $active = count(SELECT VALUE id FROM model_account_ticket
                        WHERE account = $gate AND state = 'active' AND expires_at > $now LIMIT 2);
                    LET $head = SELECT id, priority, queued_at FROM model_account_ticket
                        WHERE account = $gate AND state = 'waiting' AND expires_at > $now
                        ORDER BY priority DESC, queued_at ASC, id ASC LIMIT 1;
                    IF $cooldown <= $now AND $active < 2 AND $head[0].id = $ticket {
                        UPDATE $ticket SET state = 'active' RETURN NONE;
                        'active'
                    } ELSE { 'waiting' }
                }
            };",
        )
        .await?;
        match take::<Option<String>>(&mut response, 0)?.as_deref() {
            Some("waiting") => Ok(Admission::Waiting),
            Some("active") => Ok(Admission::Active),
            Some("lost") => Ok(Admission::Lost),
            _ => Err(TxError::Abort(
                "model account admission returned no state".into(),
            )),
        }
    }

    /// Extend only an active unexpired lease with this ticket's owner token.
    pub async fn renew(&self, db: &Surreal<Any>, ttl: Duration) -> Result<bool, TxError> {
        let mut response = apply(
            db,
            &self.gate,
            Some(&self.id),
            0,
            ttl,
            "RETURN {
                UPSERT $gate SET rev = (rev ?? 0) + 1 RETURN NONE;
                LET $now = time::now();
                LET $lease = UPDATE $ticket SET expires_at = $now + $ttl
                    WHERE account = $gate AND state = 'active' AND expires_at > $now RETURN VALUE id;
                count($lease) = 1
            };",
        )
        .await?;
        Ok(take::<Option<bool>>(&mut response, 0)? == Some(true))
    }

    /// Drop only this ticket. Expiry recovers crashes and failed cleanup.
    pub async fn release(&self, db: &Surreal<Any>) -> Result<(), TxError> {
        apply(
            db,
            &self.gate,
            Some(&self.id),
            0,
            Duration::ZERO,
            "RETURN {
                UPSERT $gate SET rev = (rev ?? 0) + 1 RETURN NONE;
                DELETE $ticket WHERE account = $gate RETURN NONE;
            };",
        )
        .await?;
        Ok(())
    }
}

/// Extend an account cooldown. Shorter feedback cannot clear a longer defer.
pub async fn defer(db: &Surreal<Any>, account: &str, delay: Duration) -> Result<(), TxError> {
    apply(
        db,
        &gate(account),
        None,
        0,
        delay,
        "RETURN {
            LET $until = time::now() + $ttl;
            UPSERT $gate SET rev = (rev ?? 0) + 1,
                cooldown_until = IF cooldown_until = NONE OR cooldown_until < $until { $until }
                    ELSE { cooldown_until } RETURN NONE;
        };",
    )
    .await?;
    Ok(())
}

// One atomic statement avoids an open server transaction on dropped futures.
// Every state transition writes the account guard, preventing snapshot races.
async fn apply(
    db: &Surreal<Any>,
    gate: &RecordId,
    ticket: Option<&RecordId>,
    priority: i64,
    ttl: Duration,
    sql: &str,
) -> Result<IndexedResults, TxError> {
    let policy = HOT_RECORD_POLICY;
    for attempt in 1..=policy.max_attempts {
        let response = db
            .query(sql)
            .bind(("gate", gate.clone()))
            .bind(("ticket", ticket.cloned()))
            .bind(("priority", priority))
            .bind(("ttl", DbDuration::from(ttl)))
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
