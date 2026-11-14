//! Bound discovery fields without silently changing canonical identity text.

use std::collections::BTreeSet;

use fleetworks_contracts::actor_index::ActorIndexAlias;
use unicode_normalization::UnicodeNormalization;

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

pub(super) fn aliases(
    display_name: &str,
    rows: Vec<AliasRow>,
) -> Result<Vec<ActorIndexAlias>, TxError> {
    // Aliases are optional discovery terms. The canonical name and all source
    // aliases remain in storage even when the index cannot accept every term.
    // Keep these word and prefix limits aligned with search-index::schema.
    let mut words: BTreeSet<String> = tokens(display_name)
        .ok_or_else(|| invalid("actor_name_token_bound"))?
        .into_iter()
        .collect();
    if words.len() > 128 || prefixes(&words).len() > 1024 {
        return Err(invalid("actor_name_expansion_bound"));
    }
    let mut selected = Vec::new();
    for row in rows {
        if selected.len() == 32 {
            break;
        }
        let Some(value) = row
            .alias
            .as_deref()
            .and_then(|value| identity(value, 256).ok())
        else {
            continue;
        };
        let Some(alias_words) = tokens(&value) else {
            continue;
        };
        let additional_words: BTreeSet<String> = alias_words.into_iter().collect();
        let next_words: BTreeSet<String> = words.union(&additional_words).cloned().collect();
        if next_words.len() > 128 {
            continue;
        }
        let next_prefixes = prefixes(&next_words);
        if next_prefixes.len() > 1024 {
            continue;
        }
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
        selected.push(ActorIndexAlias {
            value,
            strength: strength.into(),
            provenance: provenance["source_id"]
                .as_str()
                .filter(|value| super::valid(value))
                .map(str::to_owned),
        });
        words = next_words;
    }
    Ok(selected)
}

pub(super) fn indexable_text(value: &str) -> bool {
    tokens(value).is_some()
}

fn tokens(value: &str) -> Option<Vec<String>> {
    let normalized: String = value.trim().nfc().flat_map(char::to_lowercase).collect();
    let mut result = Vec::new();
    for token in normalized
        .split(|character: char| {
            !character.is_alphanumeric()
                && unicode_normalization::char::canonical_combining_class(character) == 0
        })
        .filter(|token| !token.is_empty())
    {
        if token.chars().count() > 64 {
            return None;
        }
        result.push(token.to_owned());
    }
    Some(result)
}

fn prefixes(words: &BTreeSet<String>) -> BTreeSet<String> {
    let mut prefixes = BTreeSet::new();
    for word in words {
        let characters: Vec<char> = word.chars().collect();
        for end in 2..=characters.len().min(24) {
            prefixes.insert(characters[..end].iter().collect());
            if prefixes.len() > 1024 {
                return prefixes;
            }
        }
    }
    prefixes
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
    // Countries are discovery facets. Keep a stable bounded subset in the
    // index while the complete source list remains in canonical metadata.
    Ok(countries.into_iter().take(32).collect())
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
        let rows = (0..33)
            .map(|number| AliasRow {
                actor_id: "synthetic".into(),
                alias: Some(format!("alias-{number}")),
                provenance_json: None,
            })
            .collect();
        assert_eq!(
            aliases("canonical name", rows)
                .expect("bounded aliases")
                .len(),
            32
        );
        let rows = vec![AliasRow {
            actor_id: "synthetic".into(),
            alias: Some("港".repeat(86)),
            provenance_json: None,
        }];
        assert!(aliases("canonical name", rows)
            .expect("bounded aliases")
            .is_empty());
        assert!(aliases(&"x".repeat(65), Vec::new()).is_err());
    }

    #[test]
    fn alias_selection_respects_search_expansion_budgets() {
        let rows = (0..40)
            .map(|number| AliasRow {
                actor_id: "synthetic".into(),
                alias: Some(
                    (0..12)
                        .map(|part| format!("term{number}x{part}"))
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
                provenance_json: None,
            })
            .collect();
        let selected = aliases("Canonical name", rows).expect("bounded aliases");
        assert!(selected.len() < 32);
        let words: BTreeSet<String> = selected
            .iter()
            .flat_map(|alias| tokens(&alias.value).expect("indexable alias"))
            .chain(tokens("Canonical name").expect("indexable name"))
            .collect();
        assert!(words.len() <= 128);
        assert!(prefixes(&words).len() <= 1024);
    }

    #[test]
    fn oversized_country_list_keeps_stable_bounded_facets() {
        let source_countries: Vec<String> = (0..43)
            .map(|number| format!("Country {number:02}"))
            .collect();
        let metadata = serde_json::json!({
            "countries_operated_in": serde_json::to_string(&source_countries)
                .expect("source countries")
        });
        let projected = countries(&metadata).expect("bounded optional facets");
        assert_eq!(projected, source_countries[..32].to_vec());
        assert_eq!(source_countries.len(), 43, "canonical source is unchanged");
    }
}
