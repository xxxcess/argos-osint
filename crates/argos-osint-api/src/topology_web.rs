//! Browser topology is addressed by one durable Brain memory ID.

use argos_osint_core::brain_topology::TopologySeed;
use argos_osint_core::memory_metadata::MemoryKind;
use argos_osint_core::storage;
use axum::extract::Query;
use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::ApiResult;
use crate::ApiState;
use crate::TopologyRequest;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemoryQuery {
    seed: String,
}

async fn memory_seed(state: &ApiState, id: &str) -> Result<TopologySeed, crate::ApiError> {
    if id.trim().is_empty() || id.len() > 256 {
        return Err(crate::bad());
    }
    let kind = storage::topology::memory_seed_kind(&state.repo, id)
        .await
        .map_err(crate::internal)?
        .ok_or_else(crate::missing)?;
    match kind {
        MemoryKind::Actor => Ok(TopologySeed::Actor(id.into())),
        MemoryKind::Claim => Ok(TopologySeed::Claim(id.into())),
        MemoryKind::Inference => Ok(TopologySeed::Inference(id.into())),
        _ => Err(crate::bad()),
    }
}

pub(crate) async fn read(
    State(state): State<ApiState>,
    Query(query): Query<MemoryQuery>,
) -> ApiResult {
    let seed = memory_seed(&state, &query.seed).await?;
    let snapshot = storage::topology::topology_snapshot(&state.repo, &seed, &Default::default())
        .await
        .map_err(crate::internal)?;
    if snapshot.root_id.is_empty() {
        return Err(crate::missing());
    }
    Ok(Json(
        serde_json::to_value(snapshot).map_err(crate::internal)?,
    ))
}

fn canonical_id(seed: &TopologySeed) -> Option<&str> {
    match seed {
        TopologySeed::Actor(id) | TopologySeed::Claim(id) | TopologySeed::Inference(id) => Some(id),
        _ => None,
    }
}

/// Resolve an existing brief/graph action before navigation, without creating memories.
pub(crate) async fn resolve(
    State(state): State<ApiState>,
    Json(request): Json<TopologyRequest>,
) -> ApiResult {
    if serde_json::to_string(&request.seed)
        .map_err(|_| crate::bad())?
        .len()
        > 8192
    {
        return Err(crate::bad());
    }
    if let Some(id) = canonical_id(&request.seed) {
        memory_seed(&state, id).await?;
        return Ok(Json(json!({"seed": id})));
    }
    let source = match &request.seed {
        TopologySeed::RetainedActor {
            run_id,
            article_id,
            revision,
            actor_ref,
            ..
        } => Some((run_id, article_id, revision, "actors", actor_ref)),
        TopologySeed::RetainedClaim {
            run_id,
            article_id,
            revision,
            local_id,
            ..
        } => Some((run_id, article_id, revision, "claims", local_id)),
        TopologySeed::RetainedInference {
            run_id,
            article_id,
            revision,
            local_id,
            ..
        } => Some((run_id, article_id, revision, "inferences", local_id)),
        _ => None,
    };
    if let Some((run, id, revision, field, local_id)) = source {
        let article = argos_osint_core::intel_service::article(&state.repo, run, id)
            .await
            .map_err(crate::internal)?
            .ok_or_else(crate::missing)?;
        if argos_osint_core::atlas_metadata::input_hash(&article) != *revision {
            return Err(crate::ApiError(
                axum::http::StatusCode::CONFLICT,
                "The article revision changed; refresh the brief",
            ));
        }
        let brief = argos_osint_core::intel_service::brief(&state.repo, &article)
            .await
            .map_err(crate::internal)?;
        let expected_text = match &request.seed {
            TopologySeed::RetainedActor { name, .. } => name.as_str(),
            TopologySeed::RetainedClaim { text, .. }
            | TopologySeed::RetainedInference { text, .. } => text.as_str(),
            _ => "",
        };
        let text_field = if field == "actors" {
            "display_name"
        } else {
            "text"
        };
        let items: Vec<_> = brief[field]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|item| item[text_field].as_str() == Some(expected_text))
            .collect();
        let item = items
            .iter()
            .find(|item| item["id"] == *local_id)
            .copied()
            .or_else(|| {
                if items.len() == 1 {
                    Some(items[0])
                } else {
                    None
                }
            });
        let seed = item
            .and_then(|item| item.get("topology_seed"))
            .filter(|seed| !seed.is_null())
            .cloned();
        if let Some(seed) = seed {
            let seed: TopologySeed = serde_json::from_value(seed).map_err(crate::internal)?;
            if let Some(id) = canonical_id(&seed) {
                return Ok(Json(json!({"seed":id})));
            }
        }
        return Err(crate::ApiError(
            axum::http::StatusCode::CONFLICT,
            "Brain indexing is pending for this item; navigation is read-only",
        ));
    }
    let snapshot =
        storage::topology::topology_snapshot(&state.repo, &request.seed, &Default::default())
            .await
            .map_err(crate::internal)?;
    let root = snapshot
        .nodes
        .iter()
        .find(|node| node.is_root)
        .and_then(|node| node.topology_seed.as_ref())
        .and_then(canonical_id);
    let article_target = if matches!(request.seed, TopologySeed::Article { .. }) {
        let mut memories: Vec<_> = snapshot
            .nodes
            .iter()
            .filter_map(|node| {
                node.topology_seed
                    .as_ref()
                    .and_then(canonical_id)
                    .map(|id| (node.min_hops, id))
            })
            .collect();
        memories.sort_unstable();
        memories.first().map(|(_, id)| *id)
    } else {
        None
    };
    match root.or(article_target) {
        Some(id) => Ok(Json(json!({"seed":id}))),
        None => Err(crate::ApiError(
            axum::http::StatusCode::CONFLICT,
            "Brain indexing is pending for this item; navigation is read-only",
        )),
    }
}
