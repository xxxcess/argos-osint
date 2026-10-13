//! Compact model-facing evidence: Rust owns immutable acquisition metadata.
use super::insights::{EvidenceLocation, EvidenceRef, PipelineInput, RetainedEvidence};
use anyhow::{ensure, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Give pointer windows exact global coordinates, including preceding siblings.
pub(super) fn source_span(
    source: &RetainedEvidence,
    reference: &EvidenceRef,
) -> Result<crate::nlp::Span> {
    if let EvidenceLocation::Range(span) = &reference.location {
        return Ok(*span);
    }
    let EvidenceLocation::JsonPointer(target) = &reference.location else {
        unreachable!()
    };
    let native = source
        .native
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("native source unavailable"))?;
    ensure!(
        serde_json::to_string(native)? == source.text,
        "native artifact serialization changed"
    );
    fn locate(
        value: &Value,
        path: &str,
        target: &str,
        start: usize,
    ) -> Result<Option<crate::nlp::Span>> {
        if path == target {
            return Ok(Some(crate::nlp::Span {
                start_byte: start,
                end_byte: start + serde_json::to_vec(value)?.len(),
            }));
        }
        if !target.starts_with(&format!("{path}/")) {
            return Ok(None);
        }
        let mut cursor = start + 1;
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    cursor += serde_json::to_vec(key)?.len() + 1;
                    let path = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                    if let Some(span) = locate(child, &path, target, cursor)? {
                        return Ok(Some(span));
                    }
                    cursor += serde_json::to_vec(child)?.len() + 1;
                }
            }
            Value::Array(array) => {
                for (index, child) in array.iter().enumerate() {
                    if let Some(span) = locate(child, &format!("{path}/{index}"), target, cursor)? {
                        return Ok(Some(span));
                    }
                    cursor += serde_json::to_vec(child)?.len() + 1;
                }
            }
            _ => {}
        }
        Ok(None)
    }
    let span = locate(native, "", target, 0)?
        .ok_or_else(|| anyhow::anyhow!("unknown native window pointer"))?;
    ensure!(
        span.slice(&source.text)
            == Some(serde_json::to_string(native.pointer(target).unwrap())?.as_str()),
        "native window byte mapping mismatch"
    );
    Ok(span)
}

/// The model uses source-absolute UTF-8 byte offsets, never offsets relative to a
/// window. Native references use exact JSON pointers; metadata is supplied locally.
pub(super) fn compact(
    value: &mut Value,
    dictionary: &mut BTreeMap<String, EvidenceRef>,
) -> Result<()> {
    match value {
        Value::Object(map)
            if map.contains_key("artifact_hash")
                && map.contains_key("source_id")
                && map.contains_key("location") =>
        {
            let reference: EvidenceRef = serde_json::from_value(value.clone())?;
            let id = crate::nlp::cache::key(&json!(reference));
            dictionary.insert(id.clone(), reference.clone());
            *value = json!({"evidence_id":id,"location":reference.location});
        }
        Value::Object(map) => {
            // Assessments and snapshot identity are server-owned on proposals,
            // but retained when they describe already assessed input claims.
            for child in map.values_mut() {
                compact(child, dictionary)?;
            }
        }
        Value::Array(array) => {
            for child in array {
                compact(child, dictionary)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn hydrate(
    mut value: Value,
    dictionary: &BTreeMap<String, EvidenceRef>,
    input: &PipelineInput,
) -> Result<Value> {
    fn visit(
        value: &mut Value,
        dictionary: &BTreeMap<String, EvidenceRef>,
        input: &PipelineInput,
    ) -> Result<()> {
        match value {
            Value::Object(map) if map.contains_key("evidence_id") => {
                let id = map["evidence_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("evidence_id must be a string"))?;
                let original = dictionary
                    .get(id)
                    .ok_or_else(|| anyhow::anyhow!("unknown evidence ID {id}"))?;
                let mut reference = original.clone();
                if let Some(location) = map.get("location") {
                    reference.location = serde_json::from_value(location.clone())?;
                }
                let in_window = match (&original.location, &reference.location) {
                    (EvidenceLocation::Range(a), EvidenceLocation::Range(b)) => {
                        b.start_byte >= a.start_byte && b.end_byte <= a.end_byte
                    }
                    (EvidenceLocation::JsonPointer(a), EvidenceLocation::JsonPointer(b)) => {
                        b == a || b.starts_with(&format!("{a}/"))
                    }
                    _ => false,
                };
                ensure!(
                    in_window && input.evidence.iter().any(|e| e.validate_ref(&reference)),
                    "invalid UTF-8 span/pointer or citation outside supplied window"
                );
                *value = serde_json::to_value(reference)?;
            }
            Value::Object(map) => {
                for child in map.values_mut() {
                    visit(child, dictionary, input)?;
                }
            }
            Value::Array(array) => {
                for child in array {
                    visit(child, dictionary, input)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    visit(&mut value, dictionary, input)?;
    for array in ["claims", "inferences", "links"] {
        if let Some(items) = value[array].as_array_mut() {
            for item in items {
                item["assessment"] = json!({"fidelity":"unassessed","relation":"insufficient","threshold_passed":false,"rationale":"","gaps":[]});
                if array == "claims" {
                    item["support_receipts"] = json!([]);
                } else {
                    item["snapshot_key"] = json!(input.snapshot.key);
                }
            }
        }
    }
    Ok(value)
}
