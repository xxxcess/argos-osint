//! Durable scope analysis task `brain_scope_analyze` (spec §3).
//!
//! Executes inference generation, inference assessment, and link generation
//! over a bounded Brain topology scope snapshot.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::brain_topology::{CycleScope, TopologyNodeKind, TopologyOptions, TopologySeed};
use crate::memory_metadata::{EndpointRef, MemoryKind};
use crate::provider::{self, ChatMessage, SettingsFile};
use crate::secrets::{AuthFile, ProviderSecret};

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

#[cfg(test)]
#[path = "brain_scope_tests.rs"]
mod tests;

#[cfg(test)]
mod endpoint_projection_tests {}
