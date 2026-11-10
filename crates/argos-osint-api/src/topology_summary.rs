//! Explicit summary generation and bounded, read-only delivery for one frozen basis.

use argos_osint_core::brain_neighborhood_brief::NeighborhoodBrief;
use argos_osint_core::brain_neighborhood_brief_exec::{self as exec, Delivery, GenerationFailure};
use argos_osint_core::storage::{self, neighborhood_brief};
use axum::extract::rejection::JsonRejection;
use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::{ApiResult, ApiState};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SummaryKey {
    seed: String,
    basis: String,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SummaryView {
    Recorded {
        brief: NeighborhoodBrief,
    },
    Pending {
        brief: NeighborhoodBrief,
    },
    Generated {
        brief: NeighborhoodBrief,
        route: String,
        host_notes: Vec<String>,
    },
    Failed {
        brief: NeighborhoodBrief,
        diagnostic: GenerationFailure,
    },
}

async fn view(state: ApiState, key: SummaryKey, generate: bool) -> ApiResult {
    if key.basis.is_empty() || key.basis.len() > 256 {
        return Err(crate::bad());
    }
    let seed = crate::topology_web::memory_seed(&state, &key.seed).await?;
    let snapshot = storage::topology::topology_snapshot(&state.repo, &seed, &Default::default())
        .await
        .map_err(crate::internal)?;
    let (Some(fallback), Some(input)) = (snapshot.neighborhood_brief, snapshot.neighborhood_input)
    else {
        return Err(crate::missing());
    };
    if fallback.basis_key != key.basis || input.basis_key != key.basis {
        return Err(crate::ApiError(
            axum::http::StatusCode::CONFLICT,
            "Evidence changed; refresh topology before generating",
        ));
    }
    // Local settings/auth file access and route resolution stay off Tokio.
    // No provider/model request runs in this adapter.
    let fingerprint = tokio::task::spawn_blocking(|| {
        let runtime = argos_osint_core::model_exec::RoleRuntime::new(
            argos_osint_core::secrets::AuthFile::load().unwrap_or_default(),
            argos_osint_core::provider::SettingsFile::load().unwrap_or_default(),
        );
        exec::route_fingerprint(&runtime)
    })
    .await
    .map_err(crate::internal)?;
    let (_, delivery) = if generate {
        neighborhood_brief::request_generation(&state.repo, &input, &fallback, &fingerprint).await
    } else {
        neighborhood_brief::publish(&state.repo, &input, &fallback, &fingerprint).await
    }
    .map_err(crate::internal)?;
    let result = match delivery {
        Delivery::Idle => SummaryView::Recorded { brief: fallback },
        Delivery::Pending => SummaryView::Pending { brief: fallback },
        Delivery::Generated(generated) => SummaryView::Generated {
            brief: generated.brief,
            route: generated.route,
            host_notes: generated.host_notes,
        },
        Delivery::Failed(diagnostic) => SummaryView::Failed {
            brief: fallback,
            diagnostic,
        },
    };
    Ok(Json(serde_json::to_value(result).map_err(crate::internal)?))
}

/// Reading a delivery never creates a queue item.
pub(crate) async fn read(
    State(state): State<ApiState>,
    Query(key): Query<SummaryKey>,
) -> ApiResult {
    view(state, key, false).await
}

/// This existing action surface is the only browser authorization to enqueue.
pub(crate) async fn generate(
    State(state): State<ApiState>,
    body: Result<Json<SummaryKey>, JsonRejection>,
) -> ApiResult {
    let Json(key) = body.map_err(|_| crate::bad())?;
    view(state, key, true).await
}
