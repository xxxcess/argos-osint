//! Recon provider credits behind the repository (P3 item 5): the
//! hold / reconcile / release ledger of `Store::reserve_credits`,
//! `reconcile_credits`, `release_credits` and `release_held_credits`.
//!
//! Record keys: `provider_quota` by provider, `credit_reservation` by hold
//! id. Each operation, including the quota refresh (`touch_quota`), is one
//! transaction, so concurrent holds conflict and retry instead of
//! overdrawing the pool.

use anyhow::Result;
use chrono::Utc;
use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use super::jobs::{bump, rid, run, take};
use super::tx::{run_tx, RetryPolicy, Tx, TxError};
use crate::osint::CreditHold;
use crate::provider::ReconLimits;

/// Credit ledger schema (idempotent).
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS provider_quota SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS allowance ON provider_quota TYPE int;
DEFINE FIELD IF NOT EXISTS trial_remaining ON provider_quota TYPE int;
DEFINE FIELD IF NOT EXISTS trial_seed ON provider_quota TYPE int;
DEFINE FIELD IF NOT EXISTS reserved ON provider_quota TYPE int;
DEFINE FIELD IF NOT EXISTS spent ON provider_quota TYPE int;
DEFINE FIELD IF NOT EXISTS reset_policy ON provider_quota TYPE string;
DEFINE FIELD IF NOT EXISTS period_start ON provider_quota TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON provider_quota TYPE string;

DEFINE TABLE IF NOT EXISTS credit_reservation SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS provider ON credit_reservation TYPE string;
DEFINE FIELD IF NOT EXISTS trial_credits ON credit_reservation TYPE int;
DEFINE FIELD IF NOT EXISTS allowance_credits ON credit_reservation TYPE int;
DEFINE FIELD IF NOT EXISTS state ON credit_reservation TYPE string
  ASSERT $value INSIDE ['held', 'spent', 'released'];
DEFINE FIELD IF NOT EXISTS created_at ON credit_reservation TYPE string;
DEFINE INDEX IF NOT EXISTS credit_reservation_state ON credit_reservation FIELDS state;";

fn now() -> String {
    Utc::now().to_rfc3339()
}

pub(crate) fn same_budget_month(stored: &str) -> bool {
    let now = Utc::now().format("%Y-%m").to_string();
    stored.starts_with(&now)
}

#[derive(SurrealValue)]
struct Quota {
    trial_remaining: i64,
    reserved: i64,
    spent: i64,
    allowance: i64,
}

/// `touch_quota`: create or refresh the provider's pool (allowance, reset
/// policy, trial top-up, monthly reset) and return it.
async fn touch_in(tx: &Tx, provider: &str, limits: &ReconLimits) -> Result<Quota, TxError> {
    let reset = if provider == "whoxy" || limits.credit_reset == "never" {
        "never"
    } else {
        "monthly"
    };
    let allowance = i64::from(limits.allowance(provider));
    let trial = i64::from(limits.trial_grant(provider));
    let mut r = run(
        tx,
        tx.query(
            "IF record::exists($q) {
               UPDATE $q SET allowance = $allowance, reset_policy = $reset, updated_at = $now RETURN NONE;
             } ELSE {
               CREATE $q SET allowance = $allowance, trial_remaining = $trial, trial_seed = $trial,
                 reserved = 0, spent = 0, reset_policy = $reset, period_start = $now,
                 updated_at = $now RETURN NONE;
             };
             SELECT trial_seed, period_start, reset_policy FROM ONLY $q;",
        )
        .bind(("q", rid("provider_quota", provider)))
        .bind(("allowance", allowance))
        .bind(("trial", trial))
        .bind(("reset", reset))
        .bind(("now", now())),
    )
    .await?;
    let row: Option<Seed> = take(&mut r, 1)?;
    let Seed {
        trial_seed: seed,
        period_start: period,
        reset_policy: policy,
    } = row.ok_or_else(|| TxError::Abort(format!("provider quota {provider} missing")))?;
    if trial > seed {
        run(
            tx,
            tx.query(
                "UPDATE $q SET trial_remaining += $add, trial_seed = $trial, updated_at = $now RETURN NONE",
            )
            .bind(("q", rid("provider_quota", provider)))
            .bind(("add", trial - seed))
            .bind(("trial", trial))
            .bind(("now", now())),
        )
        .await?;
    }
    if policy != "never" && !same_budget_month(&period) {
        run(
            tx,
            tx.query("UPDATE $q SET spent = 0, period_start = $now, updated_at = $now RETURN NONE")
                .bind(("q", rid("provider_quota", provider)))
                .bind(("now", now())),
        )
        .await?;
    }
    let mut r = run(
        tx,
        tx.query("SELECT trial_remaining, reserved, spent, allowance FROM ONLY $q")
            .bind(("q", rid("provider_quota", provider))),
    )
    .await?;
    take::<Option<Quota>>(&mut r, 0)?
        .ok_or_else(|| TxError::Abort(format!("provider quota {provider} missing")))
}

#[derive(SurrealValue)]
struct Seed {
    trial_seed: i64,
    period_start: String,
    reset_policy: String,
}

fn allowance_left(q: &Quota) -> i64 {
    q.allowance
        .saturating_sub(q.spent)
        .saturating_sub(q.reserved)
        .max(0)
}

/// `credits_available`: trial credits plus the unspent, unreserved allowance.
pub async fn credits_available(
    db: &Surreal<Any>,
    provider: &str,
    limits: &ReconLimits,
) -> Result<u32> {
    let q = run_tx(db, RetryPolicy::default(), |tx| async move {
        touch_in(&tx, provider, limits).await
    })
    .await?
    .value;
    Ok(u32::try_from(q.trial_remaining.max(0) + allowance_left(&q)).unwrap_or(u32::MAX))
}

/// `reserve_credits`: hold `cost` credits, trial first. `None` when the
/// pool cannot cover it; a zero cost holds nothing.
pub async fn reserve_credits(
    db: &Surreal<Any>,
    provider: &str,
    cost: u32,
    limits: &ReconLimits,
) -> Result<Option<CreditHold>> {
    reserve_named_credits(
        db,
        provider,
        cost,
        limits,
        format!("hold-{}", crate::store::new_id()),
        false,
    )
    .await
}

/// A tool call owns its credit hold independently of Recon recovery.
pub(crate) async fn reserve_manual_credits(
    db: &Surreal<Any>,
    provider: &str,
    cost: u32,
    limits: &ReconLimits,
    call_id: &str,
) -> Result<Option<CreditHold>> {
    anyhow::ensure!(
        crate::osint::manual::valid_call_id(call_id),
        "invalid manual call id"
    );
    reserve_named_credits(db, provider, cost, limits, call_id.to_owned(), true).await
}

async fn reserve_named_credits(
    db: &Surreal<Any>,
    provider: &str,
    cost: u32,
    limits: &ReconLimits,
    hold_id: String,
    manual: bool,
) -> Result<Option<CreditHold>> {
    if cost == 0 {
        return Ok(Some(CreditHold {
            id: String::new(),
            provider: provider.into(),
            trial_credits: 0,
            allowance_credits: 0,
        }));
    }
    let hold = run_tx(db, RetryPolicy::default(), |tx| {
        let hold_id = hold_id.clone();
        async move {
            if manual {
                bump(&tx, "job_guard", &hold_id).await?;
                let mut reply = run(
                    &tx,
                    tx.query(
                        "SELECT VALUE status FROM ONLY $call WHERE origin=$origin",
                    )
                    .bind(("call", rid("osint_call", &hold_id)))
                    .bind(("origin", crate::osint::manual::ORIGIN)),
                )
                .await?;
                let status: Option<String> = take(&mut reply, 0)?;
                if status.as_deref() != Some("running") {
                    return Err(TxError::Abort("manual tool call is no longer running".into()));
                }
            }
            let q = touch_in(&tx, provider, limits).await?;
            let trial_left = u32::try_from(q.trial_remaining.max(0)).unwrap_or(0);
            let allowance = u32::try_from(allowance_left(&q)).unwrap_or(0);
            if trial_left.saturating_add(allowance) < cost {
                return Ok(None);
            }
            let from_trial = cost.min(trial_left);
            let from_allowance = cost - from_trial;
            run(
                &tx,
                tx.query(
                    "UPDATE $q SET trial_remaining -= $t, reserved += $a, updated_at = $now RETURN NONE;
                     CREATE $h SET provider = $p, trial_credits = $t, allowance_credits = $a,
                       state = 'held', created_at = $now RETURN NONE;",
                )
                .bind(("q", rid("provider_quota", provider)))
                .bind(("h", rid("credit_reservation", &hold_id)))
                .bind(("p", provider.to_string()))
                .bind(("t", i64::from(from_trial)))
                .bind(("a", i64::from(from_allowance)))
                .bind(("now", now())),
            )
            .await?;
            Ok(Some(CreditHold {
                id: hold_id,
                provider: provider.into(),
                trial_credits: from_trial,
                allowance_credits: from_allowance,
            }))
        }
    })
    .await?
    .value;
    Ok(hold)
}

/// Return a hold's credits to the pool when it is still held; marks it
/// `state`. Returns whether it was held.
async fn unhold_in(tx: &Tx, hold: &CreditHold, state: &str) -> Result<bool, TxError> {
    let mut r = run(
        tx,
        tx.query(
            "LET $held = (SELECT VALUE state FROM ONLY $h) = 'held';
             IF $held {
               UPDATE $h SET state = $state RETURN NONE;
               UPDATE $q SET trial_remaining += $t,
                 reserved = IF reserved > $a { reserved - $a } ELSE { 0 }, updated_at = $now RETURN NONE;
             };
             RETURN $held;",
        )
        .bind(("h", rid("credit_reservation", &hold.id)))
        .bind(("q", rid("provider_quota", &hold.provider)))
        .bind(("state", state.to_string()))
        .bind(("t", i64::from(hold.trial_credits)))
        .bind(("a", i64::from(hold.allowance_credits)))
        .bind(("now", now())),
    )
    .await?;
    let last = r.num_statements() - 1;
    Ok(take::<Option<bool>>(&mut r, last)? == Some(true))
}

/// `reconcile_credits`: settle a hold at the `actual` cost, trial first.
/// A hold that is no longer held is left alone.
pub async fn reconcile_credits(db: &Surreal<Any>, hold: &CreditHold, actual: u32) -> Result<()> {
    if hold.id.is_empty() {
        return Ok(());
    }
    run_tx(db, RetryPolicy::default(), |tx| async move {
        if !unhold_in(&tx, hold, "spent").await? {
            return Ok(());
        }
        let mut r = run(
            &tx,
            tx.query("SELECT VALUE trial_remaining FROM ONLY $q")
                .bind(("q", rid("provider_quota", &hold.provider))),
        )
        .await?;
        let trial_now: Option<i64> = take(&mut r, 0)?;
        let from_trial = actual.min(u32::try_from(trial_now.unwrap_or(0).max(0)).unwrap_or(0));
        run(
            &tx,
            tx.query(
                "UPDATE $q SET trial_remaining -= $t, spent += $a, updated_at = $now RETURN NONE",
            )
            .bind(("q", rid("provider_quota", &hold.provider)))
            .bind(("t", i64::from(from_trial)))
            .bind(("a", i64::from(actual - from_trial)))
            .bind(("now", now())),
        )
        .await?;
        Ok(())
    })
    .await?;
    Ok(())
}

/// `release_credits`: return a still-held hold to the pool.
pub async fn release_credits(db: &Surreal<Any>, hold: &CreditHold) -> Result<()> {
    if hold.id.is_empty() {
        return Ok(());
    }
    run_tx(db, RetryPolicy::default(), |tx| async move {
        unhold_in(&tx, hold, "released").await.map(|_| ())
    })
    .await?;
    Ok(())
}

/// Conservatively settle an abandoned manual call at its held estimate.
pub(crate) async fn settle_abandoned_manual_call(db: &Surreal<Any>, id: &str) -> Result<()> {
    #[derive(SurrealValue)]
    struct Row {
        provider: String,
        trial_credits: i64,
        allowance_credits: i64,
    }
    let mut reply = db
        .query(
            "SELECT provider, trial_credits, allowance_credits FROM ONLY $hold WHERE state='held'",
        )
        .bind(("hold", rid("credit_reservation", id)))
        .await?
        .check()?;
    let row: Option<Row> = reply.take(0usize)?;
    if let Some(row) = row {
        let hold = CreditHold {
            id: id.to_owned(),
            provider: row.provider,
            trial_credits: row.trial_credits.max(0) as u32,
            allowance_credits: row.allowance_credits.max(0) as u32,
        };
        reconcile_credits(
            db,
            &hold,
            hold.trial_credits.saturating_add(hold.allowance_credits),
        )
        .await?;
    }
    Ok(())
}

/// Run recovery releases old Recon holds, never a Fleetworks manual hold.
pub async fn release_held_credits(db: &Surreal<Any>) -> Result<usize> {
    #[derive(SurrealValue)]
    struct Row {
        id: String,
        provider: String,
        trial_credits: i64,
        allowance_credits: i64,
    }
    let mut r = db
        .query(
            "SELECT record::id(id) AS id, provider, trial_credits, allowance_credits
             FROM credit_reservation WHERE state='held'
               AND !string::starts_with(record::id(id), $manual_prefix)",
        )
        .bind(("manual_prefix", crate::osint::manual::PREFIX))
        .await?
        .check()?;
    let rows: Vec<Row> = r.take(0usize)?;
    let n = rows.len();
    for row in rows {
        release_credits(
            db,
            &CreditHold {
                id: row.id,
                provider: row.provider,
                trial_credits: row.trial_credits.max(0) as u32,
                allowance_credits: row.allowance_credits.max(0) as u32,
            },
        )
        .await?;
    }
    Ok(n)
}
