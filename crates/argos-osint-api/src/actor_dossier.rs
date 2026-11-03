//! Selected actor dossier HTTP read. @tag:intel-brief
use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;

use crate::{ApiResult, ApiState};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActorQuery {
    id: String,
}

pub(crate) async fn read(
    State(state): State<ApiState>,
    Query(query): Query<ActorQuery>,
) -> ApiResult {
    if query.id.trim().is_empty() || query.id.len() > 256 || query.id.chars().any(char::is_control)
    {
        return Err(crate::bad());
    }
    let dossier = argos_osint_core::storage::actor_dossier::read(&state.repo, &query.id)
        .await
        .map_err(crate::internal)?
        .ok_or_else(crate::missing)?;
    Ok(Json(
        serde_json::to_value(dossier).map_err(crate::internal)?,
    ))
}
