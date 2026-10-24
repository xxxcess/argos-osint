//! Durable scope analysis task `brain_scope_analyze` (spec §3).
//!
//! Executes inference generation, inference assessment, and link generation
//! over a bounded Brain topology scope snapshot.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::brain_actors as actors;
use crate::brain_topology::{CycleScope, TopologyNodeKind, TopologyOptions, TopologySeed};
use crate::memory_metadata::{
    ActorRef, EndpointRef, IdentityState, InferencePayload, LinkPayload, MemoryKind, MemoryPayload,
};
use crate::provider::{self, ChatMessage, SettingsFile};
use crate::scheduler::DEFAULT_LEASE_SECS;
use crate::secrets::{AuthFile, ProviderSecret};
use crate::store::Store;
use crate::tasks::{self, JobMeta, NewJob, NewTask, OperationKind, POOL_BRAIN_SCOPE};

fn strip_json_fences(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("```json") {
        rest.strip_suffix("```").unwrap_or(rest).trim().to_string()
    } else if let Some(rest) = trimmed.strip_prefix("```") {
        rest.strip_suffix("```").unwrap_or(rest).trim().to_string()
    } else {
        trimmed.to_string()
    }
}

pub const TASK_KIND_BRAIN_SCOPE_ANALYZE: &str = "brain_scope_analyze";
pub const JOB_KIND_BRAIN_SCOPE: &str = "brain_scope";
pub const SCOPE_INFERENCES_GENERATE: &str = "brain_scope.inferences.generate";
pub const SCOPE_INFERENCES_ASSESS: &str = "brain_scope.inferences.assess";
pub const SCOPE_LINKS_GENERATE: &str = "brain_scope.links.generate";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScopeAnalysisPayload {
    pub seed: TopologySeed,
    pub cycle_scope: CycleScope,
    pub depth: usize,
    pub options: TopologyOptions,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScopeInferenceCandidate {
    pub text: String,
    pub premise_claims: Vec<String>,
    pub actors: Vec<String>,
    pub reasoning: String,
    pub limitations: String,
    pub support_state: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScopeLinkCandidate {
    pub source: EndpointRef,
    pub target: EndpointRef,
    pub relation: String,
    pub rationale: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScopeAnalysisResult {
    pub inferences_created: usize,
    pub links_created: usize,
    pub details: Vec<String>,
}

/// Enqueue a durable scope analysis job and task.
pub fn enqueue_scope_analysis(
    conn: &Connection,
    payload: &ScopeAnalysisPayload,
    now: &str,
) -> Result<String> {
    let input_json = serde_json::to_string(payload)?;
    let seed_key = payload.seed.key();
    let hash = crate::nlp::cache::hash(&input_json);
    let job_id = format!("job-scope-{}", &hash[..12.min(hash.len())]);
    let task_id = format!("task-scope-{}", &hash[..12.min(hash.len())]);

    let job = NewJob {
        id: job_id.clone(),
        kind: JOB_KIND_BRAIN_SCOPE.into(),
        owner_scope: "brain_scope".into(),
        input_revision: String::new(),
        deadline_at: String::new(),
    };

    let meta = JobMeta {
        parent_id: String::new(),
        app: "brain".into(),
        operation: TASK_KIND_BRAIN_SCOPE_ANALYZE.into(),
        title: format!("Analyze scope: {}", payload.seed.display_label()),
        run_ref: String::new(),
        resource_ref: seed_key.clone(),
        correlation_id: hash.clone(),
    };

    tasks::enqueue_job_with(conn, &job, &meta, now)?;

    let task = NewTask {
        id: task_id,
        job_id: job_id.clone(),
        operation: TASK_KIND_BRAIN_SCOPE_ANALYZE.into(),
        dedupe_key: format!("brain_scope:{}", seed_key),
        priority: 5,
        input_ref: input_json,
        input_hash: hash,
        source_revision: String::new(),
        role_snapshot: String::new(),
        max_attempts: 3,
    };

    tasks::enqueue_task(conn, &task, now)?;
    Ok(job_id)
}

/// Resolve provider secret for scope analysis (synthesis role, fallback to recon).
pub fn resolve_scope_secret(auth: &AuthFile, settings: &SettingsFile) -> Option<ProviderSecret> {
    #[cfg(test)]
    {
        let _ = (auth, settings);
        None
    }
    #[cfg(not(test))]
    {
        crate::recon::model_exec::select_role_secret(
            auth,
            settings,
            "synthesis",
            crate::recon::model_exec::OperationFormat::GeneratedJson,
        )
        .or_else(|| {
            crate::recon::model_exec::select_role_secret(
                auth,
                settings,
                "recon",
                crate::recon::model_exec::OperationFormat::GeneratedJson,
            )
        })
    }
}

/// Run scope analysis pipeline directly.
pub async fn run_scope_analysis(
    store: &Store,
    payload: &ScopeAnalysisPayload,
    secret: Option<&ProviderSecret>,
    now: &str,
) -> Result<ScopeAnalysisResult> {
    let mut options = payload.options.clone();
    options.depth = payload.depth;
    options.cycle_scope = payload.cycle_scope.clone();

    let snapshot = store.topology_snapshot(&payload.seed, &options)?;

    let claim_nodes: Vec<_> = snapshot
        .nodes
        .iter()
        .filter(|n| {
            n.kind == TopologyNodeKind::Claim
                && matches!(&n.topology_seed,Some(TopologySeed::Claim(id)) if id==&n.id)
        })
        .collect();
    let entity_nodes: Vec<_> = snapshot
        .nodes
        .iter()
        .filter(|n| {
            n.kind == TopologyNodeKind::Entity
                && matches!(&n.topology_seed,Some(TopologySeed::Actor(id)) if id==&n.id)
        })
        .collect();

    let mut result = ScopeAnalysisResult::default();

    // 1. brain_scope.inferences.generate & assess
    let candidate_inferences = if let Some(sec) = secret {
        match generate_inferences_with_model(sec, &claim_nodes, &entity_nodes).await {
            Ok(inferences) if !inferences.is_empty() => inferences,
            _ => generate_inferences_deterministic(&claim_nodes, &entity_nodes),
        }
    } else {
        generate_inferences_deterministic(&claim_nodes, &entity_nodes)
    };

    // 2. Assess and persist valid inferences
    for inf in candidate_inferences {
        let valid_premises = inf
            .premise_claims
            .iter()
            .all(|cid| claim_nodes.iter().any(|cn| cn.id == *cid));
        if !valid_premises || inf.premise_claims.is_empty() {
            continue;
        }
        let mut still_valid = true;
        for premise in &inf.premise_claims {
            still_valid &= store.is_browse_claim(premise)?;
        }
        for actor in &inf.actors {
            still_valid &= entity_nodes.iter().any(|node| node.id == *actor)
                && actors::actor(&store.conn, actor)?.is_some();
        }
        if !still_valid {
            continue;
        }

        let state = inf.support_state.trim().to_lowercase();
        if state != "supported" && state != "partially_supported" {
            continue;
        }

        let actors: Vec<ActorRef> = inf
            .actors
            .iter()
            .map(|a| ActorRef {
                actor_id: a.clone(),
                role: "subject".into(),
                binding_state: IdentityState::Provisional,
            })
            .collect();

        let inf_payload = InferencePayload {
            premise_claims: inf.premise_claims.clone(),
            actors,
            reasoning: inf.reasoning.clone(),
            limitations: inf.limitations.clone(),
            support_state: inf.support_state.clone(),
            coverage_receipt: format!("scope:{}", payload.seed.key()),
        };

        let mem = store.add_memory(
            &inf.text,
            "inference",
            false,
            crate::brain::MemorySource {
                app: "brain".into(),
                conversation_id: "scope_analysis".into(),
                message_id: None,
                reference: Some(payload.seed.key()),
            },
        )?;

        let payload_json = serde_json::to_string(&MemoryPayload::Inference(inf_payload))?;
        store.conn.execute(
            "INSERT INTO memory_metadata(memory_id,memory_kind,payload_json,payload_schema,revision,lifecycle,created_at,updated_at)
             VALUES (?1,'inference',?2,1,1,'active',?3,?3)
             ON CONFLICT(memory_id) DO UPDATE SET memory_kind='inference',payload_json=excluded.payload_json,payload_schema=1,revision=memory_metadata.revision+1,lifecycle='active',updated_at=excluded.updated_at",
            rusqlite::params![mem.id, payload_json, now],
        )?;

        result.inferences_created += 1;
        result.details.push(format!("Created inference {}", mem.id));
    }

    // 3. brain_scope.links.generate & write
    let link_nodes: Vec<_> = snapshot
        .nodes
        .iter()
        .filter(|node| resolve_node_to_endpoint(&node.id, &snapshot.nodes).is_some())
        .cloned()
        .collect();
    let candidate_links = if let Some(sec) = secret {
        match generate_links_with_model(sec, &link_nodes).await {
            Ok(links) if !links.is_empty() => links,
            _ => generate_links_deterministic(&claim_nodes, &entity_nodes),
        }
    } else {
        generate_links_deterministic(&claim_nodes, &entity_nodes)
    };

    for link in candidate_links {
        let rel = link.relation.trim();
        if !actors::LINK_RELATIONS.contains(&rel) {
            continue;
        }

        let link_payload = LinkPayload {
            source: link.source,
            target: link.target,
            relation: rel.to_string(),
            rationale: link.rationale,
            receipts: vec![format!("scope:{}", payload.seed.key())],
            promoted_from: None,
        };

        if let Ok((link_id, changed)) = actors::write_link(&store.conn, &link_payload, now) {
            if changed {
                result.links_created += 1;
                result.details.push(format!("Created link {}", link_id));
            }
        }
    }

    Ok(result)
}

pub(crate) async fn generate_inferences_with_model(
    secret: &ProviderSecret,
    claims: &[&crate::brain_topology::TopologyNode],
    entities: &[&crate::brain_topology::TopologyNode],
) -> Result<Vec<ScopeInferenceCandidate>> {
    if claims.len() < 2 {
        return Ok(Vec::new());
    }

    let claims_summary: Vec<_> = claims
        .iter()
        .map(|c| serde_json::json!({"id": c.id, "text": c.label}))
        .collect();
    let entities_summary: Vec<_> = entities
        .iter()
        .map(|e| serde_json::json!({"id": e.id, "name": e.label}))
        .collect();

    let system = "You are an investigative reasoning engine. Propose plausible, rigorous inferences from confirmed claims and entities in scope.\n\
    Return JSON only: {\"inferences\": [{\"text\": \"...\", \"premise_claims\": [\"claim_id_1\", ...], \"actors\": [\"actor_id_1\", ...], \"reasoning\": \"...\", \"limitations\": \"...\", \"support_state\": \"supported\"}]}";

    let user = format!(
        "Claims in scope:\n{}\n\nEntities in scope:\n{}\n\nGenerate justified inferences citing exact premise claim IDs. Return JSON only.",
        serde_json::to_string_pretty(&claims_summary)?,
        serde_json::to_string_pretty(&entities_summary)?
    );

    let messages = [
        ChatMessage {
            role: "system".into(),
            content: system.into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
        ChatMessage {
            role: "user".into(),
            content: user,
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
    ];

    let resp = provider::complete(secret, &messages, &[], |_| {}).await?;
    parse_inferences_response(&resp.content)
}

fn parse_inferences_response(response: &str) -> Result<Vec<ScopeInferenceCandidate>> {
    #[derive(Deserialize)]
    struct Raw {
        inferences: Option<Vec<ScopeInferenceCandidate>>,
    }
    let cleaned = strip_json_fences(response);
    let parsed: Raw = serde_json::from_str(&cleaned)?;
    Ok(parsed.inferences.unwrap_or_default())
}

pub(crate) fn generate_inferences_deterministic(
    claims: &[&crate::brain_topology::TopologyNode],
    entities: &[&crate::brain_topology::TopologyNode],
) -> Vec<ScopeInferenceCandidate> {
    if claims.len() < 2 {
        return Vec::new();
    }

    let mut out = Vec::new();
    let c1 = claims[0];
    let c2 = claims[1];
    let actor_ids = entities.iter().map(|e| e.id.clone()).collect();

    out.push(ScopeInferenceCandidate {
        text: format!(
            "Correlation inferred between '{}' and '{}'",
            c1.label.chars().take(40).collect::<String>(),
            c2.label.chars().take(40).collect::<String>()
        ),
        premise_claims: vec![c1.id.clone(), c2.id.clone()],
        actors: actor_ids,
        reasoning: "Co-occurring claims within investigative depth horizon.".into(),
        limitations: "Automated scope derivation; requires independent validation.".into(),
        support_state: "supported".into(),
    });
    out
}

pub(crate) async fn generate_links_with_model(
    secret: &ProviderSecret,
    nodes: &[crate::brain_topology::TopologyNode],
) -> Result<Vec<ScopeLinkCandidate>> {
    let nodes_summary: Vec<_> = nodes
        .iter()
        .map(|n| serde_json::json!({"id": n.id, "kind": n.kind.label(), "label": n.label}))
        .collect();

    let system = "You are an investigative link analyzer. Discover explicit relationships between nodes in scope.\n\
    Valid relations: 'mentioned_in', 'supports', 'contradicts', 'revises', 'entity_connection', 'derived_from'.\n\
    Return JSON only: {\"links\": [{\"source_id\": \"...\", \"target_id\": \"...\", \"relation\": \"supports\", \"rationale\": \"...\"}]}";

    let user = format!(
        "Nodes in scope:\n{}\n\nGenerate valid links. Return JSON only.",
        serde_json::to_string_pretty(&nodes_summary)?
    );

    let messages = [
        ChatMessage {
            role: "system".into(),
            content: system.into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
        ChatMessage {
            role: "user".into(),
            content: user,
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
    ];

    let resp = provider::complete(secret, &messages, &[], |_| {}).await?;
    #[derive(Deserialize)]
    struct RawLink {
        source_id: String,
        target_id: String,
        relation: String,
        rationale: String,
    }
    #[derive(Deserialize)]
    struct RawResp {
        links: Option<Vec<RawLink>>,
    }
    let cleaned = strip_json_fences(&resp.content);
    let parsed: RawResp = serde_json::from_str(&cleaned)?;
    let mut out = Vec::new();
    for rl in parsed.links.unwrap_or_default() {
        let ep_s = resolve_node_to_endpoint(&rl.source_id, nodes);
        let ep_t = resolve_node_to_endpoint(&rl.target_id, nodes);
        if let (Some(source), Some(target)) = (ep_s, ep_t) {
            out.push(ScopeLinkCandidate {
                source,
                target,
                relation: rl.relation,
                rationale: rl.rationale,
            });
        }
    }
    Ok(out)
}

pub(crate) fn resolve_node_to_endpoint(
    node_id: &str,
    nodes: &[crate::brain_topology::TopologyNode],
) -> Option<EndpointRef> {
    let node = nodes.iter().find(|n| n.id == node_id)?;
    match node.kind {
        TopologyNodeKind::Claim => match &node.topology_seed {
            Some(TopologySeed::Claim(id)) if id == &node.id => Some(EndpointRef::Memory {
                memory_id: id.clone(),
                kind: MemoryKind::Claim,
            }),
            _ => None,
        },
        TopologyNodeKind::Inference => node.topology_seed.as_ref().and_then(|seed| {
            if let crate::brain_topology::TopologySeed::Inference(id) = seed {
                Some(EndpointRef::Memory {
                    memory_id: id.clone(),
                    kind: MemoryKind::Inference,
                })
            } else {
                None
            }
        }),
        TopologyNodeKind::ActorGroup => None,
        TopologyNodeKind::Entity => match &node.topology_seed {
            Some(TopologySeed::Actor(id)) if id == &node.id => Some(EndpointRef::Memory {
                memory_id: id.clone(),
                kind: MemoryKind::Actor,
            }),
            _ => None,
        },
        TopologyNodeKind::Article => {
            if !node.run_id.is_empty() && !node.article_id.is_empty() {
                Some(EndpointRef::Article {
                    run_id: node.run_id.clone(),
                    article_id: node.article_id.clone(),
                    revision: node.revision.clone(),
                })
            } else {
                None
            }
        }
        TopologyNodeKind::Source => match &node.topology_seed {
            Some(TopologySeed::Source {
                origin,
                source_id,
                revision,
            }) => Some(EndpointRef::Source {
                origin: origin.clone(),
                source_id: source_id.clone(),
                revision: revision.clone(),
            }),
            _ => None,
        },
        TopologyNodeKind::Topic => None,
    }
}

pub(crate) fn generate_links_deterministic(
    claims: &[&crate::brain_topology::TopologyNode],
    entities: &[&crate::brain_topology::TopologyNode],
) -> Vec<ScopeLinkCandidate> {
    let mut out = Vec::new();
    if claims.len() >= 2 {
        out.push(ScopeLinkCandidate {
            source: EndpointRef::Memory {
                memory_id: claims[0].id.clone(),
                kind: MemoryKind::Claim,
            },
            target: EndpointRef::Memory {
                memory_id: claims[1].id.clone(),
                kind: MemoryKind::Claim,
            },
            relation: "supports".into(),
            rationale: "Co-occurring evidentiary support.".into(),
        });
    }
    for e in entities {
        if let Some(c) = claims.first() {
            let aid = e
                .member_ids
                .first()
                .cloned()
                .unwrap_or_else(|| e.id.clone());
            out.push(ScopeLinkCandidate {
                source: EndpointRef::Memory {
                    memory_id: aid,
                    kind: MemoryKind::Actor,
                },
                target: EndpointRef::Memory {
                    memory_id: c.id.clone(),
                    kind: MemoryKind::Claim,
                },
                relation: "entity_connection".into(),
                rationale: "Entity connected to claim.".into(),
            });
        }
    }
    out
}

/// Drains pending tasks in `POOL_BRAIN_SCOPE`.
pub async fn drain_scope_tasks(db_path: &Path, owner: &str, stop: &AtomicBool) -> Result<usize> {
    let conn = Connection::open(db_path)?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));

    let mut drained = 0usize;
    while !stop.load(Ordering::Acquire) {
        let now = chrono::Utc::now().to_rfc3339();
        let Some(claimed) =
            tasks::claim_next_in(&conn, &[POOL_BRAIN_SCOPE], owner, DEFAULT_LEASE_SECS, &now)?
        else {
            break;
        };

        let result = async {
            let payload: ScopeAnalysisPayload =
                serde_json::from_str(&claimed.input_ref).context("parse scope payload")?;
            let store = Store::open_light(db_path)?;
            let auth = AuthFile::load().unwrap_or_default();
            let settings = SettingsFile::load().unwrap_or_default();
            let secret = resolve_scope_secret(&auth, &settings);
            let analysis = run_scope_analysis(&store, &payload, secret.as_ref(), &now).await?;
            serde_json::to_string(&analysis).context("serialize analysis result")
        }
        .await;

        let finished = chrono::Utc::now().to_rfc3339();
        match result {
            Ok(res_json) => {
                tasks::complete_claimed(&conn, &claimed, &res_json, &finished)?;
            }
            Err(err) => {
                tasks::fail_claimed(
                    &conn,
                    &claimed,
                    OperationKind::OtherLlm,
                    &tasks::TaskError::new(tasks::ErrorCategory::Unknown, format!("{err:#}")),
                    &finished,
                )?;
            }
        }
        drained += 1;
        if drained >= 4 {
            break;
        }
    }
    Ok(drained)
}

#[cfg(test)]
#[path = "brain_scope_tests.rs"]
mod tests;

#[cfg(test)]
mod endpoint_projection_tests {
    use super::*;

    #[test]
    fn source_endpoint_uses_typed_identity_and_exact_revision() {
        let store = Store::memory().unwrap();
        let seed = TopologySeed::Source {
            origin: "evidence".into(),
            source_id: "body:record@literal".into(),
            revision: "revision:exact@1".into(),
        };
        let snapshot = store
            .topology_snapshot(&seed, &TopologyOptions::default())
            .unwrap();
        assert_eq!(
            resolve_node_to_endpoint(&snapshot.root_id, &snapshot.nodes),
            Some(EndpointRef::Source {
                origin: "evidence".into(),
                source_id: "body:record@literal".into(),
                revision: "revision:exact@1".into()
            })
        );
    }
}
