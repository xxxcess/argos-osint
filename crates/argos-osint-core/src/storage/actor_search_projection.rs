//! Bound discovery fields without silently changing canonical identity text.

use std::collections::BTreeSet;

use fleetworks_contracts::actor_index::ActorIndexAlias;

use super::invalid;
use super::AliasRow;
use super::TxError;

pub(super) fn identity(value: &str, bytes: usize) -> Result<String, TxError> {
    let value = value.trim();
    if value.is_empty() || value.len() > bytes || value.chars().any(char::is_control) {
        return Err(invalid("actor_identity_field_bound"));
    }
    Ok(value.to_owned())
}

pub(super) fn optional_identity(value: &str, bytes: usize) -> Result<Option<String>, TxError> {
    if value.trim().is_empty() {
        Ok(None)
    } else {
        identity(value, bytes).map(Some)
    }
}

pub(super) fn aliases(rows: Vec<AliasRow>) -> Result<Vec<ActorIndexAlias>, TxError> {
    // The query reads one sentinel above the accepted count. Silently keeping
    // the first N aliases would hide an unresolved oversized identity.
    if rows.len() > 32 {
        return Err(invalid("actor_alias_count_bound"));
    }
    rows.into_iter()
        .map(|row| {
            let provenance: serde_json::Value = row
                .provenance_json
                .as_deref()
                .and_then(|value| serde_json::from_str(value).ok())
                .unwrap_or_default();
            let strength = if provenance["curated"].as_bool() == Some(true) {
                "curated"
            } else if provenance["source_id"].is_string() || provenance["actor_id"].is_string() {
                "catalog"
            } else {
                "learned"
            };
            Ok(ActorIndexAlias {
                value: identity(
                    row.alias
                        .as_deref()
                        .ok_or_else(|| invalid("actor_alias_field_bound"))?,
                    256,
                )?,
                strength: strength.into(),
                provenance: provenance["source_id"]
                    .as_str()
                    .filter(|value| super::valid(value))
                    .map(str::to_owned),
            })
        })
        .collect()
}

pub(super) fn countries(metadata: &serde_json::Value) -> Result<Vec<String>, TxError> {
    let mut countries = BTreeSet::new();
    for key in ["country_of_origin", "countries_operated_in"] {
        if let Some(value) = metadata[key].as_str() {
            let values: Vec<String> = serde_json::from_str(value).unwrap_or_default();
            for value in values {
                countries.insert(identity(&value, 64)?);
            }
        }
    }
    if countries.len() > 32 {
        return Err(invalid("actor_country_count_bound"));
    }
    Ok(countries.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_identity_is_preserved_and_oversized_or_control_data_is_unresolved() {
        assert_eq!(
            identity("  港 Union  ", 64).expect("bounded identity"),
            "港 Union"
        );
        for value in ["港".repeat(22), "bad\nidentity".into(), " ".into()] {
            assert!(identity(&value, 64).is_err());
        }
        assert!(aliases(
            (0..33)
                .map(|number| AliasRow {
                    actor_id: "synthetic".into(),
                    alias: Some(format!("alias-{number}")),
                    provenance_json: None
                })
                .collect()
        )
        .is_err());
    }
}
