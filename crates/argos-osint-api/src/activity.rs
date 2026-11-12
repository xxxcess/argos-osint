//! Workstation activity views, always authorized before payload/pagination.
//!
//! A requester association provides visibility only. Mutation handlers must
//! independently require their permission and revalidate association/proof in
//! the actual durable action transaction. Shared private payload stays closed.

use argos_osint_core::jobs_view::JobFilter;
use argos_osint_core::jobs_view::JobStatusFilter;
use argos_osint_core::provider_diag;
use argos_osint_core::storage;
use argos_osint_core::storage::jobs_view::ActivityJobRow;
use argos_osint_core::storage::repo::Repository;
use axum::extract::Extension;
use axum::extract::Query;
use axum::extract::State;
use axum::Json;
use fleetworks_contracts::context::ActivityScope;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::Permission;
use serde::Deserialize;
use serde_json::json;
use serde_json::Value;

use crate::request_context::ContextFailure;
use crate::request_context::ResolvedRequest;
use crate::ApiResult;
use crate::ApiState;

/// No backend/node/system selector exists. A child filter only narrows scope.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActivityQuery {
    id: Option<String>,
    job_id: Option<String>,
    workspace_id: Option<String>,
    limit: Option<usize>,
    status: Option<String>,
    app: Option<String>,
    operation: Option<String>,
    text: Option<String>,
    since: Option<String>,
}

fn bounded(value: &str, limit: usize) -> bool {
    !value.is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}

fn request(
    extension: &Option<Extension<ResolvedRequest>>,
) -> Result<&ResolvedRequest, ContextFailure> {
    extension
        .as_ref()
        .map(|Extension(request)| request)
        .ok_or_else(ContextFailure::registration_required)
}

fn job_id(query: &ActivityQuery) -> Result<&str, ContextFailure> {
    let value = match (&query.id, &query.job_id) {
        (Some(id), None) | (None, Some(id)) => id.as_str(),
        _ => return Err(ContextFailure::invalid()),
    };
    if !bounded(value, 512) {
        return Err(ContextFailure::invalid());
    }
    Ok(value)
}

fn filter(query: &ActivityQuery) -> Result<(JobFilter, usize), ContextFailure> {
    if query.id.is_some() || query.job_id.is_some() {
        return Err(ContextFailure::invalid());
    }
    let limit = query.limit.unwrap_or(100);
    if !(1..=200).contains(&limit) {
        return Err(ContextFailure::invalid());
    }
    let status = match query.status.as_deref().unwrap_or("all") {
        "all" => JobStatusFilter::All,
        "active" => JobStatusFilter::Active,
        "retrying" => JobStatusFilter::Retrying,
        "failed" => JobStatusFilter::Failed,
        "completed" => JobStatusFilter::Completed,
        "stopped" => JobStatusFilter::Stopped,
        _ => return Err(ContextFailure::invalid()),
    };
    for value in [&query.app, &query.operation, &query.text, &query.since]
        .into_iter()
        .flatten()
    {
        if value.len() > 256 || value.chars().any(char::is_control) {
            return Err(ContextFailure::invalid());
        }
    }
    if query
        .since
        .as_deref()
        .is_some_and(|value| chrono::DateTime::parse_from_rfc3339(value).is_err())
    {
        return Err(ContextFailure::invalid());
    }
    Ok((
        JobFilter {
            status,
            app: query.app.clone().unwrap_or_default(),
            operation: query.operation.clone().unwrap_or_default(),
            text: query.text.clone().unwrap_or_default(),
            since: query.since.clone().unwrap_or_default(),
        },
        limit,
    ))
}

/// Resolve a child against the verified node catalog; a guessed ID never
/// selects another workstation or deployment. Browser sessions stay in their
/// selected workspace; native workstation activity can include local siblings.
pub(crate) async fn scope_for_request(
    repo: &Repository,
    request: &ResolvedRequest,
    workspace_id: Option<&str>,
) -> Result<ActivityScope, ContextFailure> {
    if !request.permits(Permission::ReadWorkstationActivity) {
        return Err(ContextFailure::forbidden());
    }
    request.revalidate(repo).await?;
    let context = request.registered()?;
    let browser = request.purpose() == CredentialPurpose::WebRelay;
    if workspace_id.is_some_and(|id| !bounded(id, 128)) {
        return Err(ContextFailure::invalid());
    }
    if browser && workspace_id.is_some_and(|id| id != context.workspace_id) {
        return Err(ContextFailure::missing());
    }
    let mut scope = if browser {
        ActivityScope::for_selected_workspace(context)
    } else {
        ActivityScope::for_workstation(context)
    };
    if let Some(workspace_id) = workspace_id {
        // Historical retired children are still valid narrow activity filters.
        // Current request registration itself was independently revalidated.
        let rows: Vec<Value> = repo.db().query("SELECT VALUE workspace_id FROM connect_workspace WHERE workspace_id = $workspace AND scope_id = $scope AND workstation_id = $node LIMIT 1")
            .bind(("workspace", workspace_id.to_owned())).bind(("scope", request.scope_id().to_owned()))
            .bind(("node", context.workstation_id.clone())).await.map_err(|_| ContextFailure::unavailable())?
            .check().map_err(|_| ContextFailure::unavailable())?.take(0).map_err(|_| ContextFailure::unavailable())?;
        if rows.is_empty() {
            return Err(ContextFailure::missing());
        }
        scope.workspace_id = Some(workspace_id.to_owned());
    }
    Ok(scope)
}

/// Membership plus conservative shared-origin privacy check, before payload.
pub(crate) async fn authorized_job(
    repo: &Repository,
    request: &ResolvedRequest,
    scope: &ActivityScope,
    id: &str,
) -> Result<ActivityJobRow, ContextFailure> {
    if !request.permits(Permission::ReadWorkstationActivity) {
        return Err(ContextFailure::forbidden());
    }
    request.revalidate(repo).await?;
    let context = request.registered()?;
    if scope.deployment_id != context.deployment_id
        || scope.workstation_id != context.workstation_id
        || scope.scope_revision != context.scope_revision
        || (request.purpose() == CredentialPurpose::WebRelay
            && scope.workspace_id.as_deref() != Some(context.workspace_id.as_str()))
    {
        return Err(ContextFailure::forbidden());
    }
    storage::jobs_view::get_activity_job(repo.db(), request.scope_id(), scope, id)
        .await
        .map_err(|_| ContextFailure::unavailable())?
        .ok_or_else(ContextFailure::missing)
}

pub(crate) async fn action_availability(
    repo: &Repository,
    request: &ResolvedRequest,
    rows: &mut [ActivityJobRow],
) -> Result<(), ContextFailure> {
    let cancel = request.permits(Permission::CancelJob);
    let retry = request.permits(Permission::RetryJob);
    if !cancel && !retry {
        for row in rows {
            row.action_reason_code = "action_permission_required".into();
        }
        return Ok(());
    }
    let ids: Vec<_> = rows
        .iter()
        .filter(|row| row.private_details_available)
        .map(|row| row.job.id.clone())
        .collect();
    let availability =
        storage::job_actions::availability(repo, request.scope_id(), request.registered()?, &ids)
            .await
            .map_err(|_| ContextFailure::unavailable())?;
    let availability: std::collections::HashMap<_, _> = availability
        .into_iter()
        .map(|value| (value.job_id.clone(), value))
        .collect();
    for row in rows {
        if let Some(value) = availability.get(&row.job.id) {
            row.can_cancel = cancel && value.can_cancel;
            row.can_retry = retry && value.can_retry;
            row.activity_actions_available = row.can_cancel || row.can_retry;
            row.job.cancellable = row.can_cancel;
            row.action_reason_code = value.reason_code.clone();
        }
    }
    Ok(())
}

fn maintenance_result(result: Value) -> Option<Value> {
    let operation = result.get("operation")?.as_str()?;
    if !matches!(
        operation,
        "actor_seed" | "classifier_train" | "actor_reindex" | "classifier_activate"
    ) {
        return None;
    }
    let mut safe =
        serde_json::Map::from_iter([("operation".to_owned(), Value::String(operation.to_owned()))]);
    for name in [
        "engine",
        "atlas_engine",
        "candidate_model_id",
        "active_model_id",
        "candidate_sha256",
        "report_sha256",
        "dataset_sha256",
        "state",
        "actor_search_state",
        "index_generation",
    ] {
        if let Some(value) = result
            .get(name)
            .and_then(Value::as_str)
            .filter(|value| bounded(value, 256))
        {
            safe.insert(name.to_owned(), Value::String(value.to_owned()));
        }
    }
    for name in [
        "processed",
        "total",
        "aliases",
        "visible_checkpoint",
        "activation_generation",
    ] {
        if let Some(value) = result.get(name).and_then(Value::as_u64) {
            safe.insert(name.to_owned(), Value::from(value));
        }
    }
    for name in ["activation_requested", "database_seeded"] {
        if let Some(value) = result.get(name).and_then(Value::as_bool) {
            safe.insert(name.to_owned(), Value::Bool(value));
        }
    }
    Some(Value::Object(safe))
}

pub(crate) async fn jobs(
    State(state): State<ApiState>,
    extension: Option<Extension<ResolvedRequest>>,
    Query(query): Query<ActivityQuery>,
) -> ApiResult {
    let request = request(&extension)?;
    let (filter, limit) = filter(&query)?;
    let scope = scope_for_request(&state.repo, request, query.workspace_id.as_deref()).await?;
    let mut rows = storage::jobs_view::list_activity_jobs(
        state.repo.db(),
        request.scope_id(),
        &scope,
        &filter,
        limit,
    )
    .await
    .map_err(|_| ContextFailure::unavailable())?;
    let counts =
        storage::jobs_view::activity_job_counts(state.repo.db(), request.scope_id(), &scope)
            .await
            .map_err(|_| ContextFailure::unavailable())?;
    let apps = storage::jobs_view::activity_job_apps(state.repo.db(), request.scope_id(), &scope)
        .await
        .map_err(|_| ContextFailure::unavailable())?;
    for row in &mut rows {
        row.job.error_summary = provider_diag::sanitize(&row.job.error_summary);
    }
    action_availability(&state.repo, request, &mut rows).await?;
    request.revalidate(&state.repo).await?;
    Ok(Json(
        json!({"rows":rows,"counts":counts,"apps":apps,"activity_scope":scope,
        "relation_counts_available":false,"activity_actions_available":rows.iter().any(|row|row.activity_actions_available)}),
    ))
}

pub(crate) async fn job(
    State(state): State<ApiState>,
    extension: Option<Extension<ResolvedRequest>>,
    Query(query): Query<ActivityQuery>,
) -> ApiResult {
    let request = request(&extension)?;
    let scope = scope_for_request(&state.repo, request, query.workspace_id.as_deref()).await?;
    let id = job_id(&query)?;
    let mut job = authorized_job(&state.repo, request, &scope, id).await?;
    let stages = if job.private_details_available {
        crate::safe_stages(&state.repo, id).await?
    } else {
        Vec::new()
    };
    let maintenance = if job.private_details_available {
        storage::setup_jobs::result_for_job(&state.repo, id)
            .await
            .map_err(crate::internal)?
            .and_then(maintenance_result)
    } else {
        None
    };
    // Another requester may have joined while the payload was loaded.
    let current = authorized_job(&state.repo, request, &scope, id).await?;
    if current.private_details_available != job.private_details_available {
        return Err(ContextFailure::changed().into());
    }
    job.job.error_summary = provider_diag::sanitize(&job.job.error_summary);
    action_availability(&state.repo, request, std::slice::from_mut(&mut job)).await?;
    Ok(Json(
        json!({"job":job,"stages":stages,"maintenance_result":maintenance,"private_details_available":job.private_details_available,
        "relation_counts_available":false,"activity_actions_available":job.activity_actions_available}),
    ))
}

pub(crate) async fn stages(
    State(state): State<ApiState>,
    extension: Option<Extension<ResolvedRequest>>,
    Query(query): Query<ActivityQuery>,
) -> ApiResult {
    let request = request(&extension)?;
    let scope = scope_for_request(&state.repo, request, query.workspace_id.as_deref()).await?;
    let id = job_id(&query)?;
    let job = authorized_job(&state.repo, request, &scope, id).await?;
    if !job.private_details_available {
        return Ok(Json(
            json!({"stages":[],"outputs":{},"private_details_available":false}),
        ));
    }
    let stages = crate::safe_stages(&state.repo, id).await?;
    let outputs = storage::brief_stages::stage_outputs(&state.repo, id)
        .await
        .map_err(crate::internal)?;
    if !authorized_job(&state.repo, request, &scope, id)
        .await?
        .private_details_available
    {
        return Err(ContextFailure::changed().into());
    }
    Ok(Json(
        json!({"stages":stages,"outputs":outputs,"private_details_available":true}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maintenance_results_expose_safe_metadata_without_worker_paths_or_raw_reports() {
        let projected = maintenance_result(json!({"operation":"classifier_train","engine":"natural","candidate_model_id":"synthetic-candidate","active_model_id":null,"activation_requested":false,"dataset_sha256":"a".repeat(64),"raw_report":{"api_key":"synthetic-private"},"server_path":"/private/server","state":"trained_not_activated"})).expect("known maintenance result");
        assert_eq!(projected["candidate_model_id"], "synthetic-candidate");
        assert_eq!(projected["activation_requested"], false);
        let encoded = projected.to_string();
        assert!(
            !encoded.contains("synthetic-private")
                && !encoded.contains("/private/server")
                && !encoded.contains("raw_report")
        );
        assert!(maintenance_result(json!({"operation":"unknown","state":"complete"})).is_none());
    }

    #[test]
    fn activity_query_cannot_broaden_to_another_node_or_system() {
        for query in [
            json!({"workstation_id":"other"}),
            json!({"deployment_id":"other"}),
            json!({"source":"system"}),
            json!({"all":true}),
        ] {
            assert!(serde_json::from_value::<ActivityQuery>(query).is_err());
        }
        assert!(filter(&ActivityQuery {
            limit: Some(201),
            ..Default::default()
        })
        .is_err());
        assert!(filter(&ActivityQuery {
            since: Some("yesterday".into()),
            ..Default::default()
        })
        .is_err());
        assert!(job_id(&ActivityQuery {
            id: Some("a".into()),
            job_id: Some("b".into()),
            ..Default::default()
        })
        .is_err());
    }
}
