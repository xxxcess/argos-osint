//! Actual transaction barrier for every new nonservice job and causal child.

use std::collections::BTreeMap;

use surrealdb::types::SurrealValue;

use super::super::jobs;
use super::super::tx::Tx;
use super::super::tx::TxError;
use super::ScopeRow;

#[derive(SurrealValue)]
struct SchemaInfo {
    tables: BTreeMap<String, String>,
}

pub(crate) async fn fence_new_work_in(tx: &Tx) -> Result<(), TxError> {
    let mut info = jobs::run(tx, tx.query("INFO FOR DB")).await?;
    let info: SchemaInfo = jobs::take::<Option<SchemaInfo>>(&mut info, 0)?
        .ok_or_else(|| super::abort("unavailable"))?;
    let has_control = info.tables.contains_key("connect_control");
    let has_reset = info
        .tables
        .keys()
        .any(|table| table.starts_with("configuration_reset"));
    if !has_control && !has_reset {
        // Intentional historical jobs-only schema adapter. Full repository /
        // network bootstrap always declares both authorities, even embedded.
        return Ok(());
    }
    for table in [
        "connect_control",
        "connect_registry_guard",
        "configuration_reset",
        "configuration_reset_scope",
        "configuration_reset_revoked",
    ] {
        if !info.tables.contains_key(table) {
            return Err(super::abort("unavailable"));
        }
    }
    super::registry_guard(tx).await?;
    // Decode all bounded scope metadata rather than filtering malformed or
    // missing booleans out of a WHERE predicate. More scopes require review;
    // no unseen page can hide an active reset. Never scan domain/jobs payloads.
    let mut rows = jobs::run(tx, tx.query("SELECT active_reset,access_epoch,first_use_blocked,revoked_json FROM configuration_reset_scope LIMIT 65")).await?;
    let rows: Vec<ScopeRow> = jobs::take(&mut rows, 0)?;
    if rows.len() > 64 {
        return Err(super::abort("unavailable"));
    }
    for scope in rows {
        if !super::valid(&scope.access_epoch)
            || scope.first_use_blocked != !scope.active_reset.is_empty()
            || serde_json::from_str::<std::collections::BTreeSet<String>>(&scope.revoked_json)
                .is_err()
        {
            return Err(super::abort("unavailable"));
        }
        if scope.first_use_blocked {
            return Err(super::abort("forbidden"));
        }
    }
    Ok(())
}
