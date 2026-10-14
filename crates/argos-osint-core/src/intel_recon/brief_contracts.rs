//! Authoritative Claims/Inferences/Links response contracts (spec §4).
//!
//! One set of field/count rules produces the JSON schema sent to the model
//! (strict schema, output tool parameters or Google native schema), the
//! compact prompt instructions and the local validators in `brief_stages`.
//! Adapters never invent a second result shape. Word counts and item counts
//! are enforced locally; offered IDs become schema enums.

use serde_json::{json, Value};

use super::brief_stages::{MAX_CLAIMS, MAX_INFERENCES, MAX_LEADS};
use crate::provider_adapters::ResponseContract;

pub const CLAIM_TEXT_WORDS: usize = 35;
pub const INFERENCE_TEXT_WORDS: usize = 35;
pub const INFERENCE_WHY_WORDS: usize = 45;
pub const MAX_LIMITATIONS: usize = 2;
pub const LIMITATION_WORDS: usize = 20;
pub const EMPTY_REASON_WORDS: usize = 25;
pub const LEAD_WHY_WORDS: usize = 40;

/// Contract schema version (part of stage revisions once wired).
pub const BRIEF_CONTRACT_VERSION: &str = "brief-contracts-v1";

pub const MODALITIES: [&str; 4] = ["asserted", "alleged", "denied", "uncertain"];
pub const POLARITIES: [&str; 2] = ["positive", "negative"];
pub const SUPPORT: [&str; 2] = ["supported", "conditional"];

fn id_array<S: AsRef<str>>(offered: &[S], min_items: usize) -> Value {
    let mut items = json!({"type": "string"});
    if !offered.is_empty() {
        items["enum"] = json!(offered.iter().map(AsRef::as_ref).collect::<Vec<_>>());
    }
    let mut v = json!({"type": "array", "items": items});
    if min_items > 0 {
        v["minItems"] = json!(min_items);
    }
    v
}

fn object(properties: Value) -> Value {
    let required: Vec<String> = properties
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": required,
        "properties": properties,
    })
}

/// Claims schema; `metadata_ids` are the offered title/description sentence IDs.
pub fn claims_schema<S: AsRef<str>>(metadata_ids: &[S]) -> Value {
    let claim = object(json!({
        "id": {"type": "string"},
        "text": {"type": "string"},
        "subject_id": {"type": "string"},
        "predicate": {"type": "string"},
        "object": {"type": "string"},
        "attributed_to": {"type": "string"},
        "modality": {"type": "string", "enum": MODALITIES},
        "polarity": {"type": "string", "enum": POLARITIES},
        "time": {"type": "string"},
        "evidence_ids": id_array(metadata_ids, 1),
    }));
    object(json!({
        "claims": {"type": "array", "items": claim},
        "limit_reached": {"type": "boolean"},
        "empty_reason": {"type": "string"},
    }))
}

/// Inferences schema over offered claim and body sentence IDs.
pub fn inferences_schema<S: AsRef<str>, T: AsRef<str>>(claim_ids: &[S], body_ids: &[T]) -> Value {
    let inference = object(json!({
        "id": {"type": "string"},
        "text": {"type": "string"},
        "premise_claim_ids": id_array(claim_ids, 0),
        "body_evidence_ids": id_array(body_ids, 1),
        "support": {"type": "string", "enum": SUPPORT},
        "why": {"type": "string"},
        "limitations": {"type": "array", "items": {"type": "string"}},
        "counterevidence_ids": id_array(body_ids, 0),
    }));
    object(json!({
        "inferences": {"type": "array", "items": inference},
        "empty_reason": {"type": "string"},
    }))
}

/// Links schema over offered candidate, inference and metadata IDs.
pub fn leads_schema<S: AsRef<str>, T: AsRef<str>, U: AsRef<str>>(
    candidate_ids: &[S],
    inference_ids: &[T],
    metadata_ids: &[U],
) -> Value {
    let mut article = json!({"type": "string"});
    if !candidate_ids.is_empty() {
        article["enum"] = json!(candidate_ids.iter().map(AsRef::as_ref).collect::<Vec<_>>());
    }
    let mut inference = json!({"type": "string"});
    if !inference_ids.is_empty() {
        inference["enum"] = json!(inference_ids.iter().map(AsRef::as_ref).collect::<Vec<_>>());
    }
    let lead = object(json!({
        "article_id": article,
        "inference_id": inference,
        "metadata_evidence_ids": id_array(metadata_ids, 1),
        "why": {"type": "string"},
        "verification_needed": {"type": "string"},
    }));
    object(json!({
        "leads": {"type": "array", "items": lead},
        "empty_reason": {"type": "string"},
    }))
}

/// Compact limits sentence appended to the Claims prompt.
pub fn claims_limits() -> String {
    format!(
        " Limits: at most {MAX_CLAIMS} claims; text at most {CLAIM_TEXT_WORDS} words; \
         evidence_ids only from the offered IDs; empty_reason at most {EMPTY_REASON_WORDS} words."
    )
}

/// Compact limits sentence appended to the Inferences prompt.
pub fn inferences_limits() -> String {
    format!(
        " Limits: at most {MAX_INFERENCES} conclusions; text at most {INFERENCE_TEXT_WORDS} words; \
         why at most {INFERENCE_WHY_WORDS} words; at most {MAX_LIMITATIONS} limitations of at most \
         {LIMITATION_WORDS} words each; premise, body and counterevidence IDs only from the offered \
         IDs; empty_reason at most {EMPTY_REASON_WORDS} words. Cite IDs; never echo passages."
    )
}

/// Compact limits sentence appended to the Links prompt.
pub fn leads_limits() -> String {
    format!(
        " Limits: at most {MAX_LEADS} leads; why at most {LEAD_WHY_WORDS} words; IDs only from the \
         offered candidates, inferences and their metadata; empty_reason at most \
         {EMPTY_REASON_WORDS} words."
    )
}

pub fn claims_contract<S: AsRef<str>>(metadata_ids: &[S]) -> ResponseContract {
    ResponseContract::GeneratedJson {
        name: "brief_claims".into(),
        schema: Some(claims_schema(metadata_ids)),
    }
}

pub fn inferences_contract<S: AsRef<str>, T: AsRef<str>>(
    claim_ids: &[S],
    body_ids: &[T],
) -> ResponseContract {
    ResponseContract::GeneratedJson {
        name: "brief_inferences".into(),
        schema: Some(inferences_schema(claim_ids, body_ids)),
    }
}

pub fn leads_contract<S: AsRef<str>, T: AsRef<str>, U: AsRef<str>>(
    candidate_ids: &[S],
    inference_ids: &[T],
    metadata_ids: &[U],
) -> ResponseContract {
    ResponseContract::GeneratedJson {
        name: "brief_leads".into(),
        schema: Some(leads_schema(candidate_ids, inference_ids, metadata_ids)),
    }
}

#[cfg(test)]
#[path = "brief_contracts_tests.rs"]
mod tests;
