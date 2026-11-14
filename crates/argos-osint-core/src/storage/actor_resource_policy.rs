//! Read the deployment's shared resource policy without creating index intent.
//! Linked through actor_search_outbox only after source checkpoint release.

use fleetworks_contracts::actor_index::ActorIndexLimits;
use surrealdb::types::SurrealValue;

use crate::storage::repo::Repository;
use crate::storage::tx::TxError;

/// A new deployment uses the same validated conservative default policy until
/// its elected coordinator observes service configuration. Never multiply it by
/// native workspace count, or treat corrupt stored configuration as defaults.
pub async fn resource_limits(repo: &Repository) -> Result<ActorIndexLimits, TxError> {
    #[derive(SurrealValue)]
    struct PolicyRow {
        oversized: bool,
        encoded: Option<String>,
    }
    let mut response = repo
        .db()
        .query(
            "SELECT string::len(effective_limits_json)>65536 AS oversized,
        IF string::len(effective_limits_json)<=65536 {effective_limits_json} ELSE {NONE} AS encoded
        FROM ONLY actor_search_control:catalog",
        )
        .await?
        .check()?;
    let row: Option<PolicyRow> = response.take(0)?;
    let limits = match row {
        None => ActorIndexLimits::default(),
        Some(PolicyRow {
            oversized: false,
            encoded: Some(encoded),
        }) if encoded.is_empty() => ActorIndexLimits::default(),
        Some(PolicyRow {
            oversized: false,
            encoded: Some(encoded),
        }) => serde_json::from_str(&encoded)
            .map_err(|_| TxError::Abort("actor_resource_policy_invalid".into()))?,
        Some(_) => return Err(TxError::Abort("actor_resource_policy_invalid".into())),
    };
    limits
        .validate()
        .map_err(|_| TxError::Abort("actor_resource_policy_invalid".into()))?;
    Ok(limits)
}
