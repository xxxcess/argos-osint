//! Read-only selected actor profiles and recorded first-degree associations.
//! @tag:brain-memory @tag:intel-brief
use std::collections::BTreeMap;

use anyhow::Result;
use serde::Serialize;
use surrealdb::types::SurrealValue;

use super::jobs::{rid, run, take};
use super::repo::Repository;
use super::tx::{run_tx, RetryPolicy};
use crate::actor_catalog::Actor;
use crate::brain_topology::{
    TopologyEdge, TopologyEdgeKind, TopologyNode, TopologyNodeKind, TopologyOptions, TopologySeed,
};
use crate::store::AtlasArticleRow;

#[derive(SurrealValue)]
struct ProfileRow {
    display_name: String,
    subtype: String,
    identity_state: String,
    catalog_id: Option<String>,
    catalog_json: Option<String>,
    payload_json: String,
}

/// Selected, attributed source field. A malformed encoded value is unavailable.
#[derive(Clone, Debug, Serialize)]
pub struct ProfileField {
    pub key: String,
    pub value: Option<String>,
}

/// Recorded relationship, retaining the topology receipt and source revision.
#[derive(Clone, Debug, Serialize)]
pub struct Association {
    pub node: TopologyNode,
    pub relationship: TopologyEdge,
    pub recorded_at: Option<String>,
    pub article: Option<AtlasArticleRow>,
}

/// Bounded dossier; counts describe loaded records, never global totals.
#[derive(Clone, Debug, Serialize)]
pub struct ActorDossier {
    pub id: String,
    pub display_name: String,
    pub subtype: String,
    pub identity_state: String,
    pub catalog_source_id: Option<String>,
    pub catalog_type: Option<String>,
    pub description: Option<String>,
    pub aliases: Vec<String>,
    pub metadata: Vec<ProfileField>,
    pub claims: Vec<Association>,
    pub briefs: Vec<Association>,
    pub inferences: Vec<Association>,
    pub loaded_claims: usize,
    pub loaded_briefs: usize,
    pub loaded_inferences: usize,
    pub gaps: Vec<String>,
    pub truncated: bool,
}

fn fields(metadata: &BTreeMap<String, String>) -> Vec<ProfileField> {
    let mut result = Vec::new();
    for key in [
        "country_of_origin",
        "countries_operated_in",
        "source_datasets",
        "source_urls",
        "origin_basis",
        "operation_basis",
        "operation_time_scope",
        "license",
    ] {
        let Some(raw) = metadata.get(key).filter(|v| !v.trim().is_empty()) else {
            continue;
        };
        let value = if matches!(
            key,
            "country_of_origin" | "countries_operated_in" | "source_datasets" | "source_urls"
        ) {
            serde_json::from_str::<Vec<String>>(raw)
                .ok()
                .map(|v| v.join(" · "))
        } else {
            Some(raw.clone())
        };
        if value.as_deref() != Some("") {
            result.push(ProfileField {
                key: key.into(),
                value,
            });
        }
    }
    result
}

/// Follow canonical redirects and require an active Actor. Never seeds or matches names.
pub async fn read(repo: &Repository, requested: &str) -> Result<Option<ActorDossier>> {
    let requested = requested.to_owned();
    let profile = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let requested = requested.clone();
        async move {
            let id = super::actor_bind::resolve_in(&tx, &requested).await?;
            let mut response = run(&tx, tx.query("SELECT display_name, subtype, identity_state, catalog_id, catalog_json, type::record('memory', $id).payload_json AS payload_json FROM ONLY $a WHERE type::record('memory', $id).lifecycle = 'active' AND type::record('memory', $id).memory_kind = 'actor'")
                .bind(("a", rid("brain_actor", &id))).bind(("id", id.clone()))).await?;
            let row: Option<ProfileRow> = take(&mut response, 0)?;
            let mut response = run(&tx, tx.query("SELECT VALUE alias FROM brain_actor_alias WHERE actor_id = $id ORDER BY alias").bind(("id", id.clone()))).await?;
            let aliases: Vec<String> = take(&mut response, 0)?;
            Ok(row.map(|row| (id, row, aliases)))
        }
    }).await?;
    let Some((id, row, mut aliases)) = profile.value else {
        return Ok(None);
    };
    let catalog = row
        .catalog_json
        .as_deref()
        .and_then(|v| serde_json::from_str::<Actor>(v).ok());
    if let Some(actor) = &catalog {
        aliases.extend(actor.aliases.clone());
    }
    if let Ok(crate::memory_metadata::MemoryPayload::Actor(payload)) =
        serde_json::from_str(&row.payload_json)
    {
        aliases.extend(payload.aliases);
    }
    aliases.sort();
    aliases.dedup();
    let snapshot = super::topology::topology_snapshot(
        repo,
        &TopologySeed::Actor(id.clone()),
        &TopologyOptions {
            depth: 1,
            ..Default::default()
        },
    )
    .await?;
    let mut claims = Vec::new();
    let mut briefs = Vec::new();
    let mut inferences = Vec::new();
    let mut gaps = snapshot.findings.gaps.clone();
    if row.catalog_json.is_some() && catalog.is_none() {
        gaps.push("Catalog profile unavailable: malformed stored profile".into());
    }
    for relationship in &snapshot.edges {
        let other = if relationship.from == snapshot.root_id {
            &relationship.to
        } else if relationship.to == snapshot.root_id {
            &relationship.from
        } else {
            continue;
        };
        let Some(node) = snapshot.nodes.iter().find(|n| &n.id == other) else {
            continue;
        };
        if node.kind == TopologyNodeKind::Claim
            && relationship.kind == TopologyEdgeKind::Concerns
            && relationship.actor_reference.is_some()
        {
            let recorded_at = snapshot
                .associations
                .claims
                .iter()
                .find(|m| m.id == node.id)
                .map(|m| m.created_at.clone());
            claims.push(Association {
                node: node.clone(),
                relationship: relationship.clone(),
                recorded_at,
                article: None,
            });
        } else if node.kind == TopologyNodeKind::Inference
            && relationship.kind == TopologyEdgeKind::Concerns
            && relationship.actor_reference.is_some()
        {
            inferences.push(Association {
                node: node.clone(),
                relationship: relationship.clone(),
                recorded_at: None,
                article: None,
            });
        } else if node.kind == TopologyNodeKind::Article
            && (relationship.kind == TopologyEdgeKind::MentionedIn
                || (relationship.kind == TopologyEdgeKind::Link
                    && relationship.label == "mentioned_in"))
        {
            // Topology merges native mentions with their materialized Link.
            // Its validated relation remains an article association in either form.
            if let Some(article) = snapshot
                .associations
                .articles
                .iter()
                .find(|a| a.article.run_id == node.run_id && a.article.id == node.article_id)
                .map(|a| a.article.clone())
            {
                briefs.push(Association {
                    node: node.clone(),
                    relationship: relationship.clone(),
                    recorded_at: None,
                    article: Some(article),
                });
            } else {
                gaps.push("Mentioned article occurrence unavailable".into());
            }
        }
    }
    let metadata = catalog
        .as_ref()
        .map(|a| fields(&a.metadata))
        .unwrap_or_default();
    Ok(Some(ActorDossier {
        id,
        display_name: row.display_name,
        subtype: row.subtype,
        identity_state: row.identity_state,
        catalog_source_id: row
            .catalog_id
            .or_else(|| catalog.as_ref().map(|a| a.source_id.clone())),
        catalog_type: catalog
            .as_ref()
            .and_then(|a| a.metadata.get("actor_type").cloned()),
        description: catalog.as_ref().and_then(|a| {
            a.metadata
                .get("description")
                .filter(|s| !s.trim().is_empty())
                .cloned()
        }),
        aliases,
        metadata,
        loaded_claims: claims.len(),
        loaded_briefs: briefs.len(),
        loaded_inferences: inferences.len(),
        inferences,
        claims,
        briefs,
        truncated: snapshot.has_continuation
            || !snapshot.exploration_exhausted
            || snapshot.omitted_nodes > 0
            || snapshot.omitted_edges > 0,
        gaps,
    }))
}
