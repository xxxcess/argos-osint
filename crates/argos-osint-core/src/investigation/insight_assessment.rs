//! Bounded semantic work with shared original evidence and exact item/question IDs.
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use anyhow::{ensure, Result};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

use super::decisions::{
    templates::template_insight_batch, ContractAdapter, DecisionPolicy, DecisionService,
    DecisionState,
};
use super::insights::{Assessment, EvidenceLocation, EvidenceRef, PipelineInput, REQUEST_BYTES};
use crate::recon::model_exec::{DecisionsAdapter, ModelOperationError, OperationScope};

pub(super) const BATCH_LIMIT: usize = 8;

#[derive(Clone)]
pub(super) struct Item {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub refs: Vec<EvidenceRef>,
}

impl Item {
    pub fn new(id: &str, kind: &str, text: &str, refs: &[EvidenceRef]) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            text: text.into(),
            refs: refs.to_vec(),
        }
    }
}

/// Fingerprints retain source revision AND exact span/pointer; no digest is an evidence root.
pub(super) fn dictionary(
    input: &PipelineInput,
    refs: &[EvidenceRef],
) -> Result<BTreeMap<String, Value>> {
    let mut dictionary = BTreeMap::new();
    for reference in refs {
        let source = input
            .evidence
            .iter()
            .find(|e| e.validate_ref(reference))
            .ok_or_else(|| anyhow::anyhow!("unknown/stale assessment evidence"))?;
        let passage = match &reference.location {
            EvidenceLocation::Range(span) => span.slice(&source.text).unwrap_or("").into(),
            EvidenceLocation::JsonPointer(pointer) => source
                .native
                .as_ref()
                .and_then(|v| v.pointer(pointer))
                .map(Value::to_string)
                .unwrap_or_default(),
        };
        dictionary.insert(
            crate::nlp::cache::key(&json!(reference)),
            json!({"reference":reference,"passage":passage}),
        );
    }
    Ok(dictionary)
}

fn compile(
    input: &PipelineInput,
    items: &[Item],
) -> Result<(
    super::decisions::DecisionContract,
    DecisionState,
    DecisionPolicy,
)> {
    let mut policy = DecisionPolicy::default();
    let ids: Vec<_> = items
        .iter()
        .enumerate()
        .map(|(index, item)| (format!("q{index}"), item.kind.clone()))
        .collect();
    ensure!(
        items
            .iter()
            .map(|i| &i.id)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == ids.len(),
        "duplicate assessment item IDs"
    );
    let contract = template_insight_batch(&ids, &mut policy);
    let refs: Vec<_> = items.iter().flat_map(|i| i.refs.clone()).collect();
    let dictionary = dictionary(input, &refs)?;
    let candidates: Vec<_> = items.iter().enumerate().map(|(index,i)| json!({"id":format!("q{index}"),"item_id":i.id,"kind":i.kind,"text":i.text,"evidence_ids":i.refs.iter().map(|r|crate::nlp::cache::key(&json!(r))).collect::<Vec<_>>()})).collect();
    let state = DecisionState::new()
        .with_candidate(json!({"items":candidates}))
        .with_custom("evidence_dictionary", json!(dictionary));
    Ok((contract, state, policy))
}

fn fits(contract: &super::decisions::DecisionContract, state: &DecisionState) -> bool {
    let adapter = ContractAdapter { contract, state };
    let (native, questions) = adapter.compile_native();
    // Reserve actual room for the bounded repair instructions and provider envelope.
    crate::nlp::message_bytes(&adapter.compile_chat(), &Value::Null) + 4096 <= REQUEST_BYTES
        && serde_json::to_vec(&json!([native, questions]))
            .is_ok_and(|b| b.len() + 4096 <= REQUEST_BYTES)
}

fn partition_item(input: &PipelineInput, item: &Item) -> Result<Vec<Item>> {
    let mut references = Vec::new();
    for reference in &item.refs {
        let compiled = compile(
            input,
            &[Item::new(
                &item.id,
                &item.kind,
                &item.text,
                std::slice::from_ref(reference),
            )],
        )?;
        if fits(&compiled.0, &compiled.1) {
            references.push(reference.clone());
            continue;
        }
        let source = input
            .evidence
            .iter()
            .find(|e| e.validate_ref(reference))
            .ok_or_else(|| anyhow::anyhow!("stale oversized evidence"))?;
        match &reference.location {
            EvidenceLocation::Range(span) => {
                let text = span
                    .slice(&source.text)
                    .ok_or_else(|| anyhow::anyhow!("invalid oversized UTF-8 span"))?;
                for part in
                    super::insights::evidence_windows(text, super::insights::WINDOW_BYTES / 4)?
                {
                    let part = crate::nlp::Span {
                        start_byte: part.start_byte + span.start_byte,
                        end_byte: part.end_byte + span.start_byte,
                    };
                    references.push(EvidenceRef {
                        location: EvidenceLocation::Range(part),
                        ..reference.clone()
                    });
                }
            }
            EvidenceLocation::JsonPointer(pointer) => {
                let value = source
                    .native
                    .as_ref()
                    .and_then(|v| v.pointer(pointer))
                    .ok_or_else(|| anyhow::anyhow!("invalid oversized native pointer"))?;
                for part in
                    super::insights::native_windows(value, super::insights::WINDOW_BYTES / 2)?
                {
                    references.push(EvidenceRef {
                        location: EvidenceLocation::JsonPointer(format!("{pointer}{part}")),
                        ..reference.clone()
                    });
                }
            }
        }
    }
    let mut partitions = Vec::new();
    for reference in references {
        let id = format!("{}:part:{}", item.id, partitions.len());
        let mut part = Item::new(&id, &item.kind, &item.text, &[reference]);
        let compiled = compile(input, std::slice::from_ref(&part))?;
        ensure!(
            fits(&compiled.0, &compiled.1),
            "indivisible mandatory assessment fields exceed compiled budget"
        );
        // One original window per partition; no source prefix or new evidence root.
        part.refs.dedup();
        partitions.push(part);
    }
    ensure!(
        partitions.len() > 1,
        "indivisible mandatory assessment input exceeds compiled budget"
    );
    Ok(partitions)
}

/// Configured role that owns each bounded assessment kind: identity binding and
/// recalled-memory identity admission → Entity Resolver; memory
/// predicate/time relevance → Evidence Curator; source claim fidelity,
/// relation and publication checks → Claim Assessor.
pub(crate) fn role_for_kind(kind: &str) -> &'static str {
    match kind {
        "identity" | "memory_identity" => "entity_resolver",
        "memory_relevance" => "evidence_curator",
        _ => "claim_assessor",
    }
}

/// End of the bounded batch starting at `offset`: at most [`BATCH_LIMIT`]
/// items, all requiring the same role.
pub(super) fn role_batch_end(items: &[Item], offset: usize) -> usize {
    let role = role_for_kind(&items[offset].kind);
    offset
        + items[offset..(offset + BATCH_LIMIT).min(items.len())]
            .iter()
            .take_while(|item| role_for_kind(&item.kind) == role)
            .count()
}

pub(super) fn operation_scope<'a>(
    key: &'a str,
    role: &'a str,
    input: &PipelineInput,
    task_id: &'a str,
) -> OperationScope<'a> {
    let mut scope = OperationScope::new(key, role)
        .with_generation(input.snapshot.generation)
        .with_insight_contract();
    if !task_id.is_empty() {
        scope = scope.with_task(task_id);
    }
    scope
}

pub(super) fn task_id(conn: &rusqlite::Connection, input: &PipelineInput) -> Result<String> {
    Ok(conn.query_row("SELECT t.id FROM argos_tasks t JOIN argos_jobs j ON j.id=t.job_id WHERE j.owner_scope=?1 AND t.operation='insight_refresh' AND t.source_revision=?2 AND t.state='running' ORDER BY t.created_at DESC LIMIT 1",
        params![format!("{}:{}",input.snapshot.owner_kind,input.snapshot.owner_id), input.snapshot.generation.to_string()], |r|r.get(0)).optional()?.unwrap_or_default())
}

/// Serial bounded batches share provider admission; optional model unavailability settles
/// items as withheld, while local/stale/persistence/cancellation errors remain terminal.
pub(super) async fn assess(
    input: &PipelineInput,
    items: &[Item],
    auth: &crate::secrets::AuthFile,
    settings: &crate::provider::SettingsFile,
    cancel: &Arc<AtomicBool>,
    db_path: &Path,
) -> Result<BTreeMap<String, Assessment>> {
    let mut output = BTreeMap::new();
    let mut offset = 0;
    while offset < items.len() {
        ensure!(!cancel.load(Ordering::Acquire), "cancelled");
        // Partition batches by the role each item requires so a mixed batch
        // never resolves Claim Assessor for identity/relevance items.
        let role = role_for_kind(&items[offset].kind);
        let mut end = role_batch_end(items, offset);
        let (contract, state, policy) = loop {
            let compiled = compile(input, &items[offset..end])?;
            if fits(&compiled.0, &compiled.1) {
                break compiled;
            }
            if end == offset + 1 {
                break compiled;
            }
            end -= 1;
        };
        if !fits(&contract, &state) {
            let item = &items[offset];
            let parts = partition_item(input, item)?;
            let assessed = Box::pin(assess(input, &parts, auth, settings, cancel, db_path)).await?;
            let mut combined = assessed.values().next().cloned().unwrap_or_default();
            let acceptable = |a: &Assessment| {
                a.threshold_passed
                    && match item.kind.as_str() {
                        "corroboration" => {
                            matches!(a.relation.as_str(), "supports" | "contradicts")
                        }
                        "identity" => a.fidelity == "same_entity",
                        "memory_relevance" => a.fidelity == "relevant",
                        _ => a.faithful(),
                    }
            };
            if let Some(rejection) = assessed.values().find(|a| !acceptable(a)) {
                combined = rejection.clone();
            }
            combined.gaps = assessed.values().flat_map(|a| a.gaps.clone()).collect();
            combined.rationale.push_str(&format!(
                "; all {} original-evidence partitions settled",
                parts.len()
            ));
            output.insert(item.id.clone(), combined);
            offset = end;
            continue;
        }
        let key = crate::nlp::cache::key(&json!([
            input.snapshot.owner_kind,
            input.snapshot.owner_id,
            "assessment-batch",
            contract,
            state,
            policy,
            settings.defaults.resolve_role(role).0
        ]));
        let store = crate::store::Store::open(db_path)?;
        super::insight_persist::validate_dispatch(&store.conn, input)?;
        let saved: Option<(String,String,String)> = store.conn.query_row("SELECT state,output_json,error FROM insight_stage_checkpoints WHERE operation_key=?1",[&key],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        if let Some((status, json, error)) = saved {
            if status == "completed" {
                super::insight_execution::record_reuse(&store.conn, &key, input)?;
                output.extend(serde_json::from_str::<BTreeMap<String, Assessment>>(&json)?);
                offset = end;
                continue;
            }
            if status == "exhausted" {
                for item in &items[offset..end] {
                    output.insert(
                        item.id.clone(),
                        Assessment {
                            gaps: vec![format!("withheld: {error}; Retry extraction required")],
                            ..Default::default()
                        },
                    );
                }
                offset = end;
                continue;
            }
        }
        let task_id = task_id(&store.conn, input)?;
        let execution_id = super::insight_execution::operation_id(&store.conn, &task_id, &key)?;
        super::insight_execution::consume()?;
        super::insight_execution::progress(&store.conn, input, "assessment", None, None)?;
        let window_id = items[offset..end]
            .iter()
            .map(|i| i.id.as_str())
            .collect::<Vec<_>>()
            .join(",");
        store.conn.execute("INSERT INTO insight_stage_checkpoints(operation_key,owner_kind,owner_id,generation,stage,window_id,state,updated_at) VALUES(?1,?2,?3,?4,'assessment',?5,'running',?6) ON CONFLICT(operation_key) DO UPDATE SET state='running',error='',updated_at=excluded.updated_at",params![key,input.snapshot.owner_kind,input.snapshot.owner_id,input.snapshot.generation,window_id,chrono::Utc::now().to_rfc3339()])?;
        drop(store);
        let mut scope = operation_scope(&execution_id, role, input, &task_id);
        if input.snapshot.owner_kind == "recon" {
            scope = scope.with_run(&input.snapshot.owner_id);
        }
        let evaluated = DecisionService::new()
            .evaluate_shared(
                auth,
                settings,
                &scope,
                &contract,
                &state,
                &policy,
                cancel,
                Some(db_path),
            )
            .await;
        let mut assessments = BTreeMap::new();
        let (status, error, route) = match evaluated {
            Ok((result, outcomes)) => {
                for (index, item) in items[offset..end].iter().enumerate() {
                    let question = match item.kind.as_str() {
                        "corroboration" => "claim_relation",
                        "identity" => "entity_binding",
                        "memory_relevance" => "evidence_relevance",
                        _ => "publication",
                    };
                    let qid = format!("q{index}:{question}");
                    let answer = &result.answers[&qid];
                    let passed = outcomes[&qid].is_approved();
                    let mut assessment = Assessment {
                        fidelity: if item.kind == "corroboration" {
                            "unassessed".into()
                        } else {
                            answer.label.clone()
                        },
                        relation: if item.kind == "corroboration" {
                            answer.label.clone()
                        } else {
                            "insufficient".into()
                        },
                        threshold_passed: passed,
                        rationale: format!(
                            "{question}: {} ({})",
                            answer.label,
                            outcomes[&qid].label()
                        ),
                        gaps: vec![],
                    };
                    if item.kind == "claim" {
                        let relation_id = format!("q{index}:claim_relation");
                        assessment.rationale.push_str(&format!("; own-source relation: {} ({}); original source is not independent corroboration",result.answers[&relation_id].label,outcomes[&relation_id].label()));
                    }
                    if !passed
                        || matches!(
                            answer.label.as_str(),
                            "overstated" | "contradictory" | "different_entity" | "irrelevant"
                        )
                    {
                        assessment.gaps.push(format!(
                            "assessment uncertain or rejected: {} ({})",
                            answer.label,
                            outcomes[&qid].label()
                        ));
                    }
                    assessments.insert(item.id.clone(), assessment);
                }
                (
                    "completed",
                    String::new(),
                    json!({"model":result.model_used,"latency_ms":result.latency_ms}).to_string(),
                )
            }
            Err(error) => {
                if cancel.load(Ordering::Acquire)
                    || !error
                        .downcast_ref::<ModelOperationError>()
                        .is_some_and(ModelOperationError::can_withhold)
                {
                    return Err(error);
                }
                let error = crate::events::redact(&error.to_string());
                for item in &items[offset..end] {
                    assessments.insert(
                        item.id.clone(),
                        Assessment {
                            gaps: vec![format!("withheld: {error}")],
                            ..Default::default()
                        },
                    );
                }
                ("exhausted", error, "{}".into())
            }
        };
        let store = crate::store::Store::open(db_path)?;
        super::insight_persist::validate_dispatch(&store.conn, input)?;
        store.conn.execute("UPDATE insight_stage_checkpoints SET state=?2,output_json=?3,error=?4,route_json=?5,updated_at=?6 WHERE operation_key=?1",params![key,status,serde_json::to_string(&assessments)?,error,route,chrono::Utc::now().to_rfc3339()])?;
        super::insight_execution::stage_event(
            &store.conn,
            input,
            &task_id,
            "assessment",
            if error.is_empty() {
                "completed"
            } else {
                "withheld"
            },
            if error.is_empty() {
                "all submitted IDs settled"
            } else {
                &error
            },
            json!({"operation":execution_id,"items":end-offset,"compiled_bytes":crate::nlp::message_bytes(&ContractAdapter {contract:&contract,state:&state}.compile_chat(),&Value::Null)}),
        )?;
        output.extend(assessments);
        offset = end;
        tokio::task::yield_now().await;
    }
    Ok(output)
}

#[cfg(test)]
mod role_partition_tests {
    use super::*;

    #[test]
    fn assessment_kinds_route_to_their_owning_roles() {
        assert_eq!(role_for_kind("identity"), "entity_resolver");
        assert_eq!(role_for_kind("memory_identity"), "entity_resolver");
        assert_eq!(role_for_kind("memory_relevance"), "evidence_curator");
        for kind in ["claim", "inference", "link", "corroboration", "publication"] {
            assert_eq!(role_for_kind(kind), "claim_assessor", "{kind}");
        }
    }

    #[test]
    fn mixed_batches_split_by_role_and_stay_within_eight() {
        let kinds = [
            "claim",
            "claim",
            "identity",
            "identity",
            "memory_relevance",
            "claim",
        ];
        let items: Vec<Item> = kinds
            .iter()
            .enumerate()
            .map(|(i, k)| Item::new(&format!("i{i}"), k, "t", &[]))
            .collect();
        let mut offset = 0;
        let mut batches = vec![];
        while offset < items.len() {
            let end = role_batch_end(&items, offset);
            batches.push((role_for_kind(&items[offset].kind), end - offset));
            offset = end;
        }
        assert_eq!(
            batches,
            vec![
                ("claim_assessor", 2),
                ("entity_resolver", 2),
                ("evidence_curator", 1),
                ("claim_assessor", 1)
            ]
        );
        let many: Vec<Item> = (0..19)
            .map(|i| Item::new(&format!("c{i}"), "claim", "t", &[]))
            .collect();
        assert_eq!(role_batch_end(&many, 0), BATCH_LIMIT);
        assert_eq!(role_batch_end(&many, 16), 19);
    }
}
