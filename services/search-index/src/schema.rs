//! Versioned proper-name projection schema and bounded Unicode expansion.

use std::collections::BTreeSet;

use fleetworks_contracts::actor_index::ActorIndexMutation;
use tantivy::schema::Field;
use tantivy::schema::Schema;
use tantivy::schema::FAST;
use tantivy::schema::STORED;
use tantivy::schema::STRING;
use tantivy::TantivyDocument;
use unicode_normalization::UnicodeNormalization;

use crate::IndexError;

#[derive(Clone)]
pub(crate) struct Fields {
    pub actor_id: Field,
    pub eligible: Field,
    pub name_exact: Field,
    pub alias_exact: Field,
    pub names: Field,
    pub prefixes: Field,
    pub description: Field,
    pub record: Field,
}

impl Fields {
    pub fn schema() -> (Schema, Self) {
        let mut schema = Schema::builder();
        let fields = Self {
            actor_id: schema.add_text_field("actor_id", STRING | STORED | FAST),
            eligible: schema.add_text_field("eligible", STRING),
            name_exact: schema.add_text_field("name_exact", STRING),
            alias_exact: schema.add_text_field("alias_exact", STRING),
            names: schema.add_text_field("names", STRING),
            prefixes: schema.add_text_field("prefixes", STRING),
            description: schema.add_text_field("description", STRING),
            record: schema.add_text_field("record", STORED),
        };
        (schema.build(), fields)
    }

    pub fn document(&self, mutation: &ActorIndexMutation) -> Result<TantivyDocument, IndexError> {
        let mut document = TantivyDocument::default();
        document.add_text(self.actor_id, &mutation.actor_id);
        let record = serde_json::to_string(mutation).map_err(IndexError::storage)?;
        document.add_text(self.record, record);
        let Some(projection) = &mutation.projection else {
            return Ok(document);
        };
        if !projection.eligible {
            return Ok(document);
        }
        document.add_text(self.eligible, "yes");
        document.add_text(self.name_exact, normalized(&projection.display_name));
        let mut words = BTreeSet::new();
        words.extend(tokens(&projection.display_name)?);
        for alias in &projection.aliases {
            document.add_text(self.alias_exact, normalized(&alias.value));
            words.extend(tokens(&alias.value)?);
        }
        if words.len() > 128 {
            return Err(IndexError::invalid("projection_expansion_limit"));
        }
        let mut prefixes = BTreeSet::new();
        for word in words {
            document.add_text(self.names, &word);
            let characters: Vec<char> = word.chars().collect();
            for end in 2..=characters.len().min(24) {
                prefixes.insert(characters[..end].iter().collect::<String>());
            }
        }
        if prefixes.len() > 1024 {
            return Err(IndexError::invalid("projection_expansion_limit"));
        }
        for prefix in prefixes {
            document.add_text(self.prefixes, prefix);
        }
        if let Some(summary) = &projection.summary {
            for word in tokens(summary)? {
                document.add_text(self.description, word);
            }
        }
        Ok(document)
    }
}

pub(crate) fn normalized(value: &str) -> String {
    value.trim().nfc().flat_map(char::to_lowercase).collect()
}

pub(crate) fn tokens(value: &str) -> Result<Vec<String>, IndexError> {
    let value = normalized(value);
    let mut result = Vec::new();
    for token in value
        .split(|character: char| {
            !character.is_alphanumeric()
                && unicode_normalization::char::canonical_combining_class(character) == 0
        })
        .filter(|token| !token.is_empty())
    {
        if token.chars().count() > 64 {
            return Err(IndexError::invalid("token_character_limit"));
        }
        result.push(token.to_owned());
    }
    Ok(result)
}
