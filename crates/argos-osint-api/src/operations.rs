//! Read projections for the Fleetworks operational clients.

use argos_osint_core::events::EventFilter;
use argos_osint_core::provider::SettingsFile;
use argos_osint_core::{intel_service, osint, paths, provider_diag, storage};
use axum::extract::{Extension, Query, State};
use axum::http::StatusCode;
use axum::Json;
use fleetworks_contracts::{AtlasCycleReceipt, AtlasCycleRequest, ATLAS_CYCLE_OPERATION};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::network::AccessMode;
use crate::{bad, internal, storage_ready, ApiError, ApiResult, ApiState};

pub(crate) async fn submit_atlas_cycle(
    State(state): State<ApiState>,
    Json(request): Json<AtlasCycleRequest>,
) -> Result<(StatusCode, Json<AtlasCycleReceipt>), ApiError> {
    use storage::job_dispatch::{self, ATLAS_POOL};
    use storage::jobs::{NewJob, NewTask};
    use storage::tx::TxError;

    let cycle = request.receipt().ok_or_else(bad)?;
    let task_id = cycle.task_id();
    let now = chrono::Utc::now();
    job_dispatch::submit(
        state.repo.db(),
        &NewJob {
            id: &cycle.job_id,
            kind: "task",
            operation: ATLAS_CYCLE_OPERATION,
            parent: None,
            created_at: now,
        },
        &NewTask {
            id: &task_id,
            job: &cycle.job_id,
            pool: ATLAS_POOL,
            operation: ATLAS_CYCLE_OPERATION,
            priority: 100,
            deps: vec![],
            input_hash: &cycle.run_id,
            created_at: now,
        },
    )
    .await
    .map_err(|error| match error {
        TxError::Abort(ref reason)
            if matches!(
                reason.as_str(),
                "dispatch_submission_conflict" | "dispatch_submission_retired"
            ) =>
        {
            ApiError(StatusCode::CONFLICT, "Atlas request ID is already in use")
        }
        other => internal(other),
    })?;
    Ok((StatusCode::ACCEPTED, Json(cycle)))
}

pub(crate) async fn tools() -> Json<Value> {
    Json(json!({"tools": osint::registry()}))
}

pub(crate) async fn model_defaults() -> ApiResult {
    let settings = tokio::task::spawn_blocking(SettingsFile::load)
        .await
        .map_err(|_| internal("model defaults read task failed"))?
        .map_err(|_| internal("could not read model defaults"))?;
    Ok(Json(json!({"defaults": settings.defaults})))
}

pub(crate) async fn profile(
    State(state): State<ApiState>,
    Extension(mode): Extension<AccessMode>,
) -> ApiResult {
    storage_ready(&state.repo).await?;
    let workspace = tokio::task::spawn_blocking(|| std::path::absolute(paths::home_dir()))
        .await
        .map_err(|_| internal("workspace path task failed"))?
        .map_err(|_| internal("could not resolve workspace path"))?;
    Ok(Json(json!({
        "workspace": workspace.to_string_lossy(),
        "ready": true,
        "storage": true,
        "instance_id": state.instance_id,
        "access_mode": mode,
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LogQuery {
    job_id: Option<String>,
    limit: Option<usize>,
}

pub(crate) async fn logs(
    State(state): State<ApiState>,
    Query(query): Query<LogQuery>,
) -> ApiResult {
    let limit = query.limit.unwrap_or(100);
    if !(1..=100).contains(&limit)
        || query
            .job_id
            .as_deref()
            .is_some_and(|id| !intel_service::valid_id(id))
    {
        return Err(bad());
    }
    let filter = EventFilter {
        job_id: query.job_id.unwrap_or_default(),
        ..EventFilter::default()
    };
    let rows = storage::event_log::list_events(state.repo.db(), &filter, limit)
        .await
        .map_err(internal)?;
    // Expose the same durable summaries as Jobs & Logs. Raw details can
    // contain prompts or provider payloads and stay in the owning process.
    let rows = tokio::task::spawn_blocking(move || {
        rows.into_iter()
            .map(|row| {
                json!({
                    "id": row.id,
                    "seq": row.seq,
                    "ts": row.ts,
                    "severity": row.severity,
                    "app": row.app,
                    "event_type": row.event_type,
                    "message": provider_diag::sanitize(&row.message),
                    "job_id": row.job_id,
                })
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|_| internal("log projection task failed"))?;
    Ok(Json(json!({"rows": rows})))
}
