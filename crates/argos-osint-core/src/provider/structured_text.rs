//! Versioned, bounded model text. JSON is a host serialization, not a model task.
//!
//! Blocks start with `## <kind>` and end with `End`. Fields are single-line
//! `Label: value` pairs; Markdown bold labels, bullets, CRLF and blank lines are
//! accepted, including one complete outer text/Markdown fence. List fields use
//! semicolon-separated values; `none` is an empty list.
//! No partial/truncated block is accepted. Callers validate offered references
//! and source quotations against their captured evidence before publication.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;

use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use serde_json::Value;

pub const VERSION: &str = "argos-text-v1";
pub const MAX_BYTES: usize = 64 * 1024;
pub(crate) const MAX_BLOCKS: usize = 64;
const MAX_FIELD_BYTES: usize = 8192;

/// A parsed block. Domain validation still belongs to the operation's caller.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub kind: String,
    pub fields: BTreeMap<String, String>,
}

/// Validated syntax with the original output retained for audit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    pub raw: String,
    pub blocks: Vec<Block>,
    pub empty_reason: Option<String>,
}

/// Rejection retains raw output without echoing source text into error messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub raw: String,
    pub diagnostic: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.diagnostic)
    }
}

impl std::error::Error for ParseError {}

/// Failure metadata carried back to the durable stage owner. The original
/// provider error remains in the chain for retry/category/cancellation guards.
#[derive(Debug)]
pub struct GenerationDiagnostics {
    pub snapshot: Value,
    pub error: anyhow::Error,
}

impl fmt::Display for GenerationDiagnostics {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.error)
    }
}

impl std::error::Error for GenerationDiagnostics {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.error.as_ref())
    }
}

/// Bound retained diagnostics independently from the provider's wire response.
/// Oversized output is explicitly marked truncated and never treated as valid.
pub fn raw_attempt(raw: &str) -> Value {
    let mut end = raw.len().min(MAX_BYTES);
    while !raw.is_char_boundary(end) {
        end -= 1;
    }
    json!({"raw_text":&raw[..end],"wire_bytes":raw.len(),"truncated":end != raw.len()})
}

fn normalized(raw: &str) -> String {
    raw.trim().trim_matches('*').trim().to_ascii_lowercase()
}

fn contract(kind: &str) -> Option<(&'static [&'static str], &'static [&'static str])> {
    match kind {
        "plan" => Some((
            &["goal", "required evidence", "queries", "depends on"],
            &["entities", "targets", "done when"],
        )),
        "identifier" => Some((
            &["type", "value", "actor", "evidence", "quotation"],
            &["platform"],
        )),
        "claim" => Some((
            &["statement", "evidence"],
            &[
                "subject",
                "predicate",
                "object",
                "attribution",
                "modality",
                "polarity",
                "time",
                "scope",
                "namespace",
                "topic",
                "classification",
            ],
        )),
        "inference" => Some((
            &[
                "conclusion",
                "premises",
                "evidence",
                "conditions",
                "counterevidence",
            ],
            &[
                "rationale",
                "support",
                "entities",
                "kind",
                "domains",
                "limitations",
            ],
        )),
        "related lead" => Some((
            &["article", "findings", "rationale"],
            &[
                "evidence",
                "verification needed",
                "relation",
                "chronology",
                "conditions",
                "limitations",
            ],
        )),
        "overview"
        | "connections"
        | "interpretation"
        | "limits"
        | "current state"
        | "significant developments"
        | "actor relationships"
        | "cross-domain implications"
        | "emerging signals"
        | "conditional outlook"
        | "evidence"
        | "limitations"
        | "current_state"
        | "significant_developments"
        | "actor_relationships"
        | "cross_domain_implications"
        | "emerging_signals"
        | "conditional_outlook" => Some((&["text", "basis"], &[])),
        "decision" => Some((&["question", "choice"], &[])),
        "link" => Some((
            &["endpoints", "relation", "evidence", "rationale"],
            &["chronology", "conditions", "limitations"],
        )),
        "strategy" => Some((&["choice", "rationale"], &[])),
        "queries" => Some((&["identity", "investigative"], &[])),
        "selection" => Some((&["choices", "rationale"], &[])),
        _ => None,
    }
}

impl Block {
    pub fn field(&self, name: &str) -> &str {
        self.fields.get(name).map(String::as_str).unwrap_or("")
    }

    pub fn list(&self, name: &str) -> Vec<String> {
        let text = self.field(name);
        if text.eq_ignore_ascii_case("none") || text.is_empty() {
            return Vec::new();
        }
        text.split(';').map(str::trim).map(str::to_owned).collect()
    }

    /// Stable content identity; a model never supplies a durable record ID.
    pub fn durable_id(&self, namespace: &str) -> String {
        format!("{namespace}:{}", crate::nlp::cache::key(&json!(self)))
    }
}

/// Parse a complete versioned response. No JSON or legacy decoder fallback.
pub fn parse(raw: &str) -> Result<Document, ParseError> {
    let reject = |message: String| ParseError {
        raw: raw_attempt(raw)["raw_text"]
            .as_str()
            .unwrap_or("")
            .to_owned(),
        diagnostic: message,
    };
    if raw.len() > MAX_BYTES {
        return Err(reject("text exceeds 65536-byte response limit".into()));
    }
    let text = raw.trim();
    let payload = if text.starts_with("```") {
        let newline = text
            .find('\n')
            .ok_or_else(|| reject("incomplete outer text fence".into()))?;
        let opening = text[..newline].trim();
        if !["```", "```text", "```plaintext", "```markdown"].contains(&opening)
            || !text.ends_with("```")
            || text.len() < newline + 4
        {
            return Err(reject("unsupported or incomplete outer text fence".into()));
        }
        &text[newline + 1..text.len() - 3]
    } else {
        text
    };
    let mut lines = payload
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    if lines.next() != Some(VERSION) {
        return Err(reject(format!("first line must be {VERSION}")));
    }
    let mut blocks = Vec::new();
    let mut current: Option<Block> = None;
    let mut empty = false;
    let mut reason = None;
    for (index, line) in lines.enumerate() {
        let error = |message: &str| reject(format!("line {}: {message}", index + 2));
        if line.starts_with('#') {
            if current.is_some() || empty || blocks.len() >= MAX_BLOCKS {
                return Err(error("unexpected block boundary or block limit exceeded"));
            }
            let kind = normalized(line.trim_start_matches('#'));
            if contract(&kind).is_none() {
                return Err(error("unknown block kind"));
            }
            current = Some(Block {
                kind,
                fields: BTreeMap::new(),
            });
        } else if line.eq_ignore_ascii_case("end") {
            let block = current.take().ok_or_else(|| error("End without a block"))?;
            let (required, _) = contract(&block.kind).expect("recognized block kind");
            if required
                .iter()
                .any(|field| !block.fields.contains_key(*field))
            {
                return Err(error("missing required field"));
            }
            blocks.push(block);
        } else {
            let line = line.strip_prefix("- ").unwrap_or(line);
            let (label, value) = line
                .split_once(':')
                .ok_or_else(|| error("expected Label: value or End"))?;
            let label = normalized(label);
            let value = value.trim().trim_start_matches("**").trim();
            if value.is_empty() || value.len() > MAX_FIELD_BYTES {
                return Err(error("empty or oversized field"));
            }
            if let Some(block) = current.as_mut() {
                let (required, optional) = contract(&block.kind).expect("recognized block kind");
                if !required.contains(&label.as_str()) && !optional.contains(&label.as_str()) {
                    return Err(error("unknown field"));
                }
                if block.fields.insert(label, value.to_owned()).is_some() {
                    return Err(error("duplicate field"));
                }
            } else if label == "result"
                && value.eq_ignore_ascii_case("none")
                && blocks.is_empty()
                && !empty
            {
                empty = true;
            } else if label == "reason" && empty && reason.is_none() {
                reason = Some(value.to_owned());
            } else {
                return Err(error("unexpected field outside a block"));
            }
        }
    }
    if current.is_some() || (blocks.is_empty() && (!empty || reason.is_none())) {
        return Err(reject(
            "incomplete output; close every block or supply Result: none and Reason".into(),
        ));
    }
    Ok(Document {
        raw: raw.to_owned(),
        blocks,
        empty_reason: reason,
    })
}

/// Template instructions shared by generation callers, not a response schema.
pub fn instructions(kind: &str) -> String {
    let Some((required, optional)) = contract(kind) else {
        return String::new();
    };
    let fields = required
        .iter()
        .chain(optional.iter())
        .map(|field| format!("{field}: <value>"))
        .collect::<Vec<_>>()
        .join("\n");
    format!("Reply in structured text, never JSON. First line: {VERSION}. Repeat complete blocks:\n## {kind}\n{fields}\nEnd\nRequired fields: {}. Optional fields may be omitted. Use semicolons between list values and 'none' for an empty list. Cite only supplied IDs. For no records output {VERSION}\nResult: none\nReason: <specific reason>. Do not output an incomplete block.", required.join(", "))
}

/// Validate short references before domain conversion. Unknown/duplicate refs
/// are errors, never silently dropped by a filter-map.
pub fn references(ids: &[String], offered: &BTreeSet<String>) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for id in ids {
        if !offered.contains(id) || !seen.insert(id) {
            return Err("unknown or duplicate reference".into());
        }
    }
    Ok(())
}

/// Host serialization of labeled chat choices. No confidence is invented.
pub fn choices_value(raw: &str) -> anyhow::Result<Value> {
    let document = parse(raw)?;
    anyhow::ensure!(!document.blocks.is_empty(), "decisions require answers");
    let mut answers = serde_json::Map::new();
    for block in document.blocks {
        anyhow::ensure!(block.kind == "decision", "expected Decision block");
        anyhow::ensure!(
            answers
                .insert(
                    block.field("question").into(),
                    json!({"label":block.field("choice")})
                )
                .is_none(),
            "duplicate question"
        );
    }
    Ok(json!({"answers":answers}))
}

/// Convert Atlas text blocks to the existing typed-artifact wire shape. Domain
/// validators still enforce source scope, offered IDs, word/count bounds.
pub fn brief_value(raw: &str, kind: &str) -> anyhow::Result<Value> {
    let document = parse(raw)?;
    anyhow::ensure!(
        document.blocks.iter().all(|block| block.kind == kind),
        "wrong operation block kind"
    );
    let empty_reason = document.empty_reason.as_deref().unwrap_or("");
    let items: Vec<Value> = document.blocks.iter().map(|block| match kind {
        "claim" => json!({"id":block.durable_id("claim"),"text":block.field("statement"),
            "subject_id":block.field("subject"),"predicate":block.field("predicate"),
            "object":block.field("object"),"attributed_to":block.field("attribution"),
            "modality":if block.field("modality").is_empty() {"asserted"} else {block.field("modality")},
            "polarity":if block.field("polarity").is_empty() {"positive"} else {block.field("polarity")},
            "time":block.field("time"),"evidence_ids":block.list("evidence")}),
        "inference" => json!({"id":block.durable_id("inference"),"text":block.field("conclusion"),
            "premise_claim_ids":block.list("premises"),"body_evidence_ids":block.list("evidence"),
            "support":block.field("support"),"why":block.field("rationale"),
            "limitations":if block.field("limitations").is_empty() {block.list("conditions")} else {block.list("limitations")},
            "conditions":block.list("conditions"),
            "inference_kind":if block.field("kind").is_empty() {Value::Null} else {json!(block.field("kind"))},
            "domains":block.list("domains"),"counterevidence_ids":block.list("counterevidence")}),
        "related lead" => json!({"article_id":block.field("article"),"inference_id":block.field("findings"),
            "metadata_evidence_ids":block.list("evidence"),"why":block.field("rationale"),
            "verification_needed":block.field("verification needed"),
            "proposed_relation":block.field("relation"),"chronology_basis":block.field("chronology"),
            "conditions":block.list("conditions"),"limitations":block.list("limitations")}),
        _ => Value::Null,
    }).collect();
    Ok(match kind {
        "claim" => json!({"claims":items,"limit_reached":false,"empty_reason":empty_reason}),
        "inference" => json!({"inferences":items,"empty_reason":empty_reason}),
        "related lead" => json!({"leads":items,"empty_reason":empty_reason}),
        _ => anyhow::bail!("unsupported Brief text operation"),
    })
}

/// Readable summary sections converted to the existing cached Brief DTO.
pub fn summary_value(raw: &str) -> anyhow::Result<Value> {
    let document = parse(raw)?;
    anyhow::ensure!(
        document.empty_reason.is_none(),
        "a summary requires cited sections"
    );
    let facet_for = |kind: &str| {
        crate::intelligence_contracts::BriefFacet::ALL
            .into_iter()
            .find(|facet| {
                facet.as_str() == kind || facet.display_name().to_ascii_lowercase() == kind
            })
    };
    let faceted = document
        .blocks
        .iter()
        .any(|block| facet_for(&block.kind).is_some());
    let mut value = if faceted {
        json!({"current_state":[],"significant_developments":[],"actor_relationships":[],"cross_domain_implications":[],"emerging_signals":[],"conditional_outlook":[],"evidence":[],"limitations":[]})
    } else {
        json!({"overview":[],"connections":[],"interpretation":[],"limits":[]})
    };
    for block in document.blocks {
        let kind = facet_for(&block.kind)
            .map(|facet| facet.as_str())
            .unwrap_or(&block.kind);
        let array = value
            .get_mut(kind)
            .and_then(Value::as_array_mut)
            .ok_or_else(|| anyhow::anyhow!("unexpected summary section"))?;
        array.push(json!({"text":block.field("text"),"basis":block.list("basis")}));
    }
    Ok(value)
}

/// Host-owned Recon wire conversion. Existing directive/binding validators run
/// after this syntax layer; dependency and quotation checks happen here.
pub fn recon_value(raw: &str, kind: &str) -> anyhow::Result<Value> {
    let document = parse(raw)?;
    anyhow::ensure!(
        document.blocks.iter().all(|block| block.kind == kind),
        "wrong Recon block kind"
    );
    match kind {
        "plan" => {
            let offered: BTreeSet<String> = (1..=document.blocks.len())
                .map(|n| format!("d{n}"))
                .collect();
            let mut directives = Vec::new();
            for (index, block) in document.blocks.iter().enumerate() {
                let dependencies = block.list("depends on");
                references(&dependencies, &offered).map_err(anyhow::Error::msg)?;
                // Dependencies must point backward, so cycles and self edges
                // cannot enter a checkpoint even before Phase 2 consumes them.
                let prior: BTreeSet<_> = (1..=index).map(|n| format!("d{n}")).collect();
                references(&dependencies, &prior).map_err(anyhow::Error::msg)?;
                let queries = block.list("queries");
                directives.push(json!({"id":format!("d{}", index + 1),"goal":block.field("goal"),
                    "entities":block.list("entities"),"targets":block.list("targets"),
                    "done_when":block.field("done when"),"query":queries.first().cloned().unwrap_or_default(),
                    "required_evidence":block.list("required evidence"),"queries":queries,"depends_on":dependencies}));
            }
            Ok(json!({"directives":directives,"empty_reason":document.empty_reason}))
        }
        "identifier" => Ok(json!({"bindings":document.blocks.iter().map(|block|
            json!({"kind":block.field("type"),"value":block.field("value"),"actor":block.field("actor"),
                "evidence_id":block.field("evidence"),"quotation":block.field("quotation"),"platform":block.field("platform")})).collect::<Vec<_>>(),
            "empty_reason":document.empty_reason})),
        "strategy" | "queries" | "selection" => {
            anyhow::ensure!(
                document.blocks.len() == 1,
                "expected one bounded response block"
            );
            let block = &document.blocks[0];
            Ok(match kind {
                "strategy" => {
                    json!({"strategy":block.field("choice"),"rationale":block.field("rationale")})
                }
                "queries" => {
                    json!({"identity_query":block.field("identity"),"investigative_query":block.field("investigative")})
                }
                _ => json!({"selected":block.list("choices")}),
            })
        }
        _ => anyhow::bail!("unknown Recon text operation"),
    }
}

/// Validate an identifier's literal source quotation and actor association.
/// Search decoded source strings, not escaped JSON serialization or summaries.
pub fn validate_identifier_quotes(
    value: &Value,
    evidence_id: &str,
    source: &Value,
) -> anyhow::Result<()> {
    fn contains(source: &Value, quotation: &str) -> bool {
        match source {
            Value::String(text) => text.contains(quotation),
            Value::Array(items) => items.iter().any(|item| contains(item, quotation)),
            Value::Object(fields) => fields.values().any(|item| contains(item, quotation)),
            _ => false,
        }
    }
    let items = value["bindings"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing bindings"))?;
    for item in items {
        let quotation = item["quotation"].as_str().unwrap_or("");
        let value = item["value"].as_str().unwrap_or("");
        let actor = item["actor"].as_str().unwrap_or("");
        anyhow::ensure!(!quotation.is_empty() && !value.is_empty() && !actor.is_empty()
            && quotation.contains(value) && quotation.contains(actor) && contains(source, quotation)
            && item["evidence_id"].as_str() == Some(evidence_id),
            "identifier lacks a literal source quotation/actor association or has an unknown evidence ID");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_text_accepts_markdown_and_assigns_stable_ids() {
        let raw = "argos-text-v1\r\n\r\n### Claim\r\n- **Statement:** Example alleges a delay.\r\n**Evidence**: t1; d2\r\nEnd\r\n";
        let doc = parse(raw).expect("documented Markdown variation");
        assert_eq!(doc.raw, raw);
        assert_eq!(doc.blocks[0].list("evidence"), ["t1", "d2"]);
        assert_eq!(
            doc.blocks[0].durable_id("claim"),
            parse(raw).expect("same text").blocks[0].durable_id("claim")
        );
        let value = brief_value(raw, "claim").expect("host conversion");
        assert_eq!(value["claims"][0]["text"], "Example alleges a delay.");
        let fenced = format!("```text\n{raw}\n```");
        assert_eq!(parse(&fenced).expect("outer text fence").blocks, doc.blocks);
        assert!(parse(&format!("```text\n{raw}")).is_err());
    }

    #[test]
    fn structured_text_rejects_duplicate_missing_truncated_and_unknown_fields() {
        for body in [
            "## Claim\nStatement: A\nStatement: B\nEvidence: t1\nEnd",
            "## Claim\nStatement: A\nEnd",
            "## Claim\nStatement: A\nEvidence: t1",
            "## Claim\nStatement: A\nEvidence: t1\nInvented: B\nEnd",
            "## Claim\nStatement: A\nEvidence: t1\n## Claim",
            "Result: none",
            "",
            "{}",
        ] {
            let raw = format!("{VERSION}\n{body}");
            let err = parse(&raw).expect_err(body);
            assert_eq!(err.raw, raw);
        }
    }

    #[test]
    fn structured_text_empty_is_explicit_and_references_are_closed() {
        let doc = parse("argos-text-v1\nResult: none\nReason: No supported claim.")
            .expect("explicit empty");
        assert_eq!(doc.empty_reason.as_deref(), Some("No supported claim."));
        let offered = BTreeSet::from(["t1".into()]);
        assert!(references(&["fabricated".into()], &offered).is_err());
        assert!(references(&["t1".into(), "t1".into()], &offered).is_err());
        assert!(references(&["t1".into()], &offered).is_ok());
        assert!(parse(&"x".repeat(MAX_BYTES + 1)).is_err());
    }

    #[test]
    fn structured_text_tolerates_documented_variation_but_not_conflicts() {
        // Case-insensitive kinds/labels, bold bullet labels, blank lines and
        // `none` lists are documented variation, not distinct syntax. The
        // contract version line stays exact: it pins the parser contract.
        let doc = parse(
            "argos-text-v1\n\n## CLAIM\n- **Statement:** Example alleges a delay.\n**Evidence**: none\nEnd\n",
        )
        .expect("documented label variation");
        assert_eq!(doc.blocks[0].kind, "claim");
        assert!(doc.blocks[0].list("evidence").is_empty());
        // A completed block plus Result: none is conflicting, never both.
        let conflicting = format!(
            "{VERSION}\n## Claim\nStatement: A\nEvidence: t1\nEnd\nResult: none\nReason: no"
        );
        assert!(parse(&conflicting).is_err());
        // End without an open block and an oversized field are boundary errors.
        assert!(parse(&format!("{VERSION}\nEnd")).is_err());
        let oversized = format!(
            "{VERSION}\n## Claim\nStatement: {}\nEvidence: t1\nEnd",
            "x".repeat(MAX_FIELD_BYTES + 1)
        );
        assert!(parse(&oversized).is_err());
        // A raw error never echoes more than the bounded retained attempt.
        let err = parse("not-a-contract").expect_err("missing version");
        assert!(err.raw.len() <= MAX_BYTES);
        assert!(err.diagnostic.contains("first line"));
    }

    #[test]
    fn structured_text_atlas_conversion_rejects_fabricated_citations_at_domain_boundary() {
        use crate::intel_recon::brief_stages;
        let metadata = brief_stages::sentences("t", "Example reports a delay.");
        let raw = "argos-text-v1\n## Claim\nStatement: Example reports a delay.\nEvidence: t1\nEnd";
        let value = brief_value(raw, "claim").expect("text conversion");
        let validated = brief_stages::validate_claims(&value.to_string(), &metadata)
            .expect("known metadata citation");
        assert_eq!(validated["claims"][0]["evidence"][0]["id"], "t1");
        let fabricated =
            brief_value(&raw.replace("t1", "fabricated"), "claim").expect("valid syntax alone");
        assert!(brief_stages::validate_claims(&fabricated.to_string(), &metadata).is_err());
    }

    #[test]
    fn structured_text_plan_dependencies_are_known_and_acyclic() {
        let raw = "argos-text-v1\n## Plan\nGoal: Establish identity\nRequired evidence: name\nQueries: Example official website\nDepends on: none\nTargets: person_name\nEnd";
        let value = recon_value(raw, "plan").expect("independent first directive");
        assert_eq!(value["directives"][0]["id"], "d1");
        assert!(recon_value(&raw.replace("Depends on: none", "Depends on: d1"), "plan").is_err());
        assert!(recon_value(
            &raw.replace("Depends on: none", "Depends on: fabricated"),
            "plan"
        )
        .is_err());
    }
}
