//! Native graph projection of memory payloads (P2, spec step 5 memory part).
//!
//! `brain_graph_node` holds keys and filter metadata of canonical records
//! (memories, exact article and source revisions); `brain_graph_edge` is a
//! typed relation between nodes. The projection is maintained in the same
//! transaction as the memory it derives from and is never authoritative.
//!
//! Of the seven edge meanings, the four that memory payloads carry are
//! projected here: **Link** and **Related** (edge key = the exact memory id,
//! so parallel relationships between the same endpoints stay distinct),
//! **Concerns** (claim/inference to actor, with role and binding state) and
//! **Premise** (inference to premise claim). **Source**, **MentionedIn** and
//! **Contains** derive from Atlas/insight records and land with those tables
//! in P3; the schema already admits them.
//!
//! Retirement: a memory that is not active keeps its node (lifecycle
//! updated, so traversals can filter it) and loses the edges its own payload
//! produced. Edges other memories own toward it stay, so deleting or
//! retiring one memory never removes another memory's support.

use crate::memory_metadata::{EndpointRef, MemoryKind, MemoryPayload};

use super::codec;
use super::jobs::{rid, run};
use super::tx::{Tx, TxError};

/// The seven edge meanings (`brain_topology::TopologyEdgeKind` labels).
pub const MEANINGS: [&str; 7] = [
    "link",
    "related",
    "concerns",
    "premise",
    "source",
    "mentioned_in",
    "contains",
];

/// Graph schema (idempotent).
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS brain_graph_node SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS kind ON brain_graph_node TYPE string
    ASSERT $value IN ['memory', 'article', 'source'];
DEFINE FIELD IF NOT EXISTS ref_id ON brain_graph_node TYPE string;
DEFINE FIELD IF NOT EXISTS origin ON brain_graph_node TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS revision ON brain_graph_node TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS memory_kind ON brain_graph_node TYPE string DEFAULT '';
-- '' until the node's own record publishes it (endpoint seen first).
DEFINE FIELD IF NOT EXISTS lifecycle ON brain_graph_node TYPE string DEFAULT '';
DEFINE INDEX IF NOT EXISTS brain_graph_node_ref ON brain_graph_node FIELDS kind, ref_id;
DEFINE TABLE IF NOT EXISTS brain_graph_edge TYPE RELATION
    IN brain_graph_node OUT brain_graph_node SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS meaning ON brain_graph_edge TYPE string
    ASSERT $value IN ['link', 'related', 'concerns', 'premise', 'source', 'mentioned_in', 'contains'];
-- Memory whose payload produced the edge.
DEFINE FIELD IF NOT EXISTS owner ON brain_graph_edge TYPE string;
DEFINE FIELD IF NOT EXISTS relation ON brain_graph_edge TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS role ON brain_graph_edge TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS binding_state ON brain_graph_edge TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS status ON brain_graph_edge TYPE string DEFAULT '';
DEFINE INDEX IF NOT EXISTS brain_graph_edge_owner ON brain_graph_edge FIELDS owner;
DEFINE INDEX IF NOT EXISTS brain_graph_edge_meaning ON brain_graph_edge FIELDS meaning;";

/// Node key of a memory.
pub fn memory_node(memory_id: &str) -> String {
    codec::encode(&["memory", memory_id])
}

/// Node key of an endpoint (exact article/source revision).
pub fn endpoint_node(e: &EndpointRef) -> String {
    let (kind, id, origin, revision) = e.index_key();
    match kind {
        "memory" => memory_node(&id),
        _ => codec::encode(&[kind, &origin, &id, &revision]),
    }
}

/// One projected edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub key: String,
    pub meaning: &'static str,
    pub from: EndpointRef,
    pub to: EndpointRef,
    pub relation: String,
    pub role: String,
    pub binding_state: String,
    pub status: String,
}

fn mem(id: &str, kind: MemoryKind) -> EndpointRef {
    EndpointRef::Memory {
        memory_id: id.to_string(),
        kind,
    }
}

/// Edges a memory's payload produces, one per edge key. Pure, so it is
/// unit-testable. A payload may repeat an actor with the same role, or a
/// premise claim; those map to one edge key and the first one wins.
pub fn edges_for(memory_id: &str, payload: &MemoryPayload) -> Vec<Edge> {
    let mut seen = std::collections::HashSet::new();
    let mut edges = payload_edges(memory_id, payload);
    edges.retain(|e| seen.insert(e.key.clone()));
    edges
}

fn payload_edges(memory_id: &str, payload: &MemoryPayload) -> Vec<Edge> {
    let me = mem(memory_id, payload.kind());
    let concerns = |actors: &[crate::memory_metadata::ActorRef]| {
        actors
            .iter()
            .map(|a| Edge {
                key: codec::encode(&[memory_id, "concerns", &a.actor_id, &a.role]),
                meaning: "concerns",
                from: me.clone(),
                to: mem(&a.actor_id, MemoryKind::Actor),
                relation: String::new(),
                role: a.role.clone(),
                binding_state: a.binding_state.as_str().into(),
                status: String::new(),
            })
            .collect::<Vec<_>>()
    };
    match payload {
        MemoryPayload::Actor(_) => Vec::new(),
        MemoryPayload::Claim(c) => concerns(&c.actors),
        MemoryPayload::Inference(i) => {
            let mut out = concerns(&i.actors);
            out.extend(i.premise_claims.iter().map(|c| Edge {
                key: codec::encode(&[memory_id, "premise", c]),
                meaning: "premise",
                from: me.clone(),
                to: mem(c, MemoryKind::Claim),
                relation: String::new(),
                role: String::new(),
                binding_state: String::new(),
                status: i.support_state.clone(),
            }));
            out
        }
        MemoryPayload::Link(l) => vec![Edge {
            key: memory_id.to_string(),
            meaning: "link",
            from: l.source.clone(),
            to: l.target.clone(),
            relation: l.relation.clone(),
            role: String::new(),
            binding_state: String::new(),
            status: "observed".into(),
        }],
        MemoryPayload::Related(r) => vec![Edge {
            key: memory_id.to_string(),
            meaning: "related",
            from: r.source.clone(),
            to: r.target.clone(),
            relation: String::new(),
            role: String::new(),
            binding_state: String::new(),
            status: "tentative".into(),
        }],
    }
}

/// Drops the edges memory `$id` owns, through the `brain_graph_edge_owner`
/// index (pinned by a plan test).
pub const DELETE_OWNED_SURQL: &str = "DELETE brain_graph_edge WHERE owner = $id";

/// Re-project `memory_id` inside the caller's transaction, in one query:
/// upsert its node with `lifecycle`, drop the edges it owns, and (when
/// active) relate the edges its payload produces.
pub async fn project_memory_in(
    tx: &Tx,
    memory_id: &str,
    kind: MemoryKind,
    payload: Option<&MemoryPayload>,
    lifecycle: &str,
) -> Result<(), TxError> {
    let edges = match payload {
        Some(p) if lifecycle == "active" => edges_for(memory_id, p),
        _ => Vec::new(),
    };
    let mut sql = String::from(
        "UPSERT $me SET kind = 'memory', ref_id = $id, memory_kind = $mk, lifecycle = $lc \
         RETURN NONE;",
    );
    sql.push_str(DELETE_OWNED_SURQL);
    sql.push_str(" RETURN NONE;");
    for i in 0..edges.len() {
        for side in ["f", "t"] {
            sql.push_str(&format!(
                "UPSERT ${side}{i} SET kind = ${side}k{i}, ref_id = ${side}r{i}, \
                 origin = ${side}o{i}, revision = ${side}v{i}, \
                 memory_kind = IF memory_kind != NONE AND memory_kind != '' THEN memory_kind \
                     ELSE ${side}mk{i} END RETURN NONE;"
            ));
        }
        sql.push_str(&format!(
            "RELATE $f{i}->$e{i}->$t{i} SET meaning = $m{i}, owner = $id, relation = $rel{i}, \
             role = $role{i}, binding_state = $b{i}, status = $st{i} RETURN NONE;"
        ));
    }
    let mut q = tx
        .query(sql)
        .bind(("me", rid("brain_graph_node", &memory_node(memory_id))))
        .bind(("id", memory_id.to_string()))
        .bind(("mk", kind.as_str().to_string()))
        .bind(("lc", lifecycle.to_string()));
    for (i, e) in edges.iter().enumerate() {
        for (side, end) in [("f", &e.from), ("t", &e.to)] {
            let (k, r, o, v) = end.index_key();
            // A memory endpoint's index origin is its declared kind.
            let (o, mk) = if k == "memory" {
                (String::new(), o)
            } else {
                (o, String::new())
            };
            q = q
                .bind((format!("{side}mk{i}"), mk))
                .bind((
                    format!("{side}{i}"),
                    rid("brain_graph_node", &endpoint_node(end)),
                ))
                .bind((format!("{side}k{i}"), k.to_string()))
                .bind((format!("{side}r{i}"), r))
                .bind((format!("{side}o{i}"), o))
                .bind((format!("{side}v{i}"), v));
        }
        q = q
            .bind((format!("e{i}"), rid("brain_graph_edge", &e.key)))
            .bind((format!("m{i}"), e.meaning.to_string()))
            .bind((format!("rel{i}"), e.relation.clone()))
            .bind((format!("role{i}"), e.role.clone()))
            .bind((format!("b{i}"), e.binding_state.clone()))
            .bind((format!("st{i}"), e.status.clone()));
    }
    run(tx, q).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_metadata::{ActorRef, ClaimPayload, IdentityState, InferencePayload};

    fn actor(id: &str, role: &str, state: IdentityState) -> ActorRef {
        ActorRef {
            actor_id: id.into(),
            role: role.into(),
            binding_state: state,
        }
    }

    #[test]
    fn edges_for_dedupes_repeated_actor_roles_and_premises() {
        let claim = MemoryPayload::Claim(ClaimPayload {
            actors: vec![
                actor("x", "subject", IdentityState::Resolved),
                actor("x", "subject", IdentityState::Provisional),
                actor("x", "object", IdentityState::Resolved),
                actor("y", "subject", IdentityState::Resolved),
            ],
            ..ClaimPayload::default()
        });
        let edges = edges_for("c", &claim);
        let roles: Vec<(String, String)> = edges
            .iter()
            .map(|e| (e.role.clone(), e.binding_state.clone()))
            .collect();
        assert_eq!(edges.len(), 3, "x/subject once, x/object, y/subject");
        // First occurrence wins.
        assert_eq!(
            roles[0],
            ("subject".into(), IdentityState::Resolved.as_str().into())
        );
        let keys: std::collections::HashSet<&str> = edges.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys.len(), edges.len());

        let inference = MemoryPayload::Inference(InferencePayload {
            premise_claims: vec!["c1".into(), "c2".into(), "c1".into()],
            actors: vec![
                actor("x", "relevant", IdentityState::Resolved),
                actor("x", "relevant", IdentityState::Resolved),
            ],
            ..InferencePayload::default()
        });
        let edges = edges_for("i", &inference);
        let count = |m: &str| edges.iter().filter(|e| e.meaning == m).count();
        assert_eq!(count("premise"), 2);
        assert_eq!(count("concerns"), 1);
    }
}
