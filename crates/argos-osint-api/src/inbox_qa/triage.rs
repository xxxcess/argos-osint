//! Read-only triage over the same private activity and health observations as Inbox QA.

use std::time::Duration;

use argos_osint_core::jobs_view::JobFilter;
use argos_osint_core::jobs_view::JobRow;
use argos_osint_core::storage::inbox_qa as store;
use argos_osint_core::storage::jobs_view;
use chrono::Utc;
use fleetworks_contracts::chat::ChatSourceKind;
use fleetworks_contracts::chat::ChatSourceReference;
use fleetworks_contracts::chat::QaDecisionPreference;
use fleetworks_contracts::chat::QaRelatedActivity;
use fleetworks_contracts::chat::QaRelatedActivityRequest;
use fleetworks_contracts::chat::QaRelationshipReason;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::readiness::ReadinessCheckId;
use tokio::time::Instant;

use super::decision::DecisionChoice;
use super::decision::DecisionMenu;
use super::require;
use super::storage_error;
use super::QaTarget;
use crate::activity as activity_api;
use crate::request_context::ResolvedRequest;
use crate::ApiError;
use crate::ApiState;

const MAX_CANDIDATES: usize = 8;

const OPERATIONS: [(ReadinessCheckId, &str); 3] = [
    (ReadinessCheckId::ActorCatalog, "actor_seed"),
    (ReadinessCheckId::ActorSearch, "actor_reindex"),
    (ReadinessCheckId::ClassifierTraining, "classifier_train"),
];

pub(super) struct ActivityFact {
    pub(super) label: &'static str,
    pub(super) status: &'static str,
    pub(super) check: Option<ReadinessCheckId>,
    reason: Option<&'static str>,
}

impl ActivityFact {
    pub(super) fn explanation(&self) -> String {
        let mut text = format!("{} is {}.", self.label, self.status);
        if let Some(reason) = self.reason {
            text.push(' ');
            text.push_str(reason);
        } else if self.status == "failed" {
            text.push_str(" No verified cause was recorded for this job.");
        }
        text
    }
}

/// Retain only safe lifecycle and recorded, allowlisted stage reasons.
pub(super) async fn activity(
    state: &ApiState,
    context: &ResolvedRequest,
    source: &ChatSourceReference,
) -> Result<ActivityFact, ApiError> {
    let scope = activity_api::scope_for_request(&state.repo, context, None).await?;
    let row = activity_api::authorized_job(&state.repo, context, &scope, &source.id).await?;
    if !row.private_details_available {
        return Err(crate::missing());
    }
    let label = if source.kind == ChatSourceKind::Cycle {
        if fleetworks_contracts::AtlasCycleReceipt::from_job_id(&row.job.id).is_none() {
            return Err(crate::missing());
        }
        "Selected cycle"
    } else {
        match row.job.operation.as_str() {
            "actor_reindex" => "Actor indexing job",
            "actor_seed" => "Actor catalog seed job",
            "classifier_train" => "Classifier training job",
            _ => "Selected job",
        }
    };
    Ok(activity_projection(&row.job, label))
}

fn activity_projection(job: &JobRow, label: &'static str) -> ActivityFact {
    let status = match job.state.as_str() {
        "queued" => "queued",
        "running" => "running",
        "retry_scheduled" => "waiting to retry",
        "reconciling" => "reconciling",
        "succeeded" | "completed" => "completed",
        "failed" => "failed",
        "cancelled" | "canceled" => "canceled",
        "partial" => "partially completed",
        "paused" => "paused",
        _ => "unknown",
    };
    let check = OPERATIONS
        .into_iter()
        .find_map(|(check, operation)| (job.operation == operation).then_some(check));
    let reason = if status == "failed" && check.is_some() {
        [job.error_category.as_str(), job.error_summary.as_str()]
            .into_iter()
            .find_map(public_reason)
    } else {
        None
    };
    ActivityFact {
        label,
        status,
        check,
        reason,
    }
}

fn public_reason(code: &str) -> Option<&'static str> {
    Some(match code {
        "setup_worker_memory_pressure" => "The recorded reason was worker memory pressure.",
        "setup_worker_disk_pressure" => "The recorded reason was low workspace disk space.",
        "setup_worker_pressure_unavailable" => "The worker could not verify available resources.",
        "setup_worker_resource_pressure" => "The worker reached its resource limit.",
        "setup_dataset_resource_budget" => "The dataset exceeded an input resource limit.",
        "setup_interrupted" | "actor_reindex_interrupted" => "The worker recorded an interruption.",
        _ => return None,
    })
}

/// Resolve only a current observation of the cited check.
pub(super) async fn current_status(
    target: &QaTarget,
    state: &ApiState,
    generation: i64,
    id: &str,
    deadline: Instant,
) -> Option<QaRelatedActivity> {
    let reference = ChatSourceReference::new(ChatSourceKind::Readiness, id.to_owned()).ok()?;
    let boundary = Utc::now().timestamp_millis() - store::MAX_EVIDENCE_AGE_MS;
    let sample = tokio::time::timeout_at(deadline, async {
        loop {
            if let Some(sample) = target
                .observe(state, generation, boundary, true)
                .await
                .ok()?
            {
                return Some(sample);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .ok()??;
    sample
        .facts
        .iter()
        .any(|fact| fact.id == id)
        .then_some(QaRelatedActivity::Found {
            target: reference,
            reason: QaRelationshipReason::CurrentStatus,
        })
}

/// Resolve a link without writing a transcript or a Jobs entry.
pub(crate) async fn related_activity_request(
    state: &ApiState,
    context: &ResolvedRequest,
    target: &QaTarget,
    request: QaRelatedActivityRequest,
) -> Result<QaRelatedActivity, ApiError> {
    request.validate().map_err(|_| crate::bad())?;
    let source = &request.source;
    let permission = match source.kind {
        ChatSourceKind::Job => Permission::ReadWorkstationActivity,
        ChatSourceKind::Readiness => Permission::ReadBackend,
        _ => return Err(crate::bad()),
    };
    require(context, permission, None)?;
    context.revalidate(&state.repo).await?;
    let setting = store::setting(&state.repo).await.map_err(storage_error)?;
    if !setting.enabled {
        return Ok(QaRelatedActivity::TriageUnavailable);
    }
    let Ok(_permit) = target.probes.try_acquire() else {
        return Ok(QaRelatedActivity::TriageUnavailable);
    };
    let deadline = Instant::now() + Duration::from_secs(4);
    let result = async {
        match source.kind {
            ChatSourceKind::Job => {
                let activity = activity(state, context, source).await?;
                let Some(check) = activity.check else {
                    return Ok(QaRelatedActivity::NoRelatedActivity);
                };
                if !context.permits(Permission::ReadBackend) {
                    return Ok(QaRelatedActivity::NoRelatedActivity);
                }
                let status =
                    current_status(target, state, setting.generation, check.as_str(), deadline)
                        .await;
                if status.is_some() {
                    self::activity(state, context, source).await?;
                }
                Ok(status.unwrap_or(QaRelatedActivity::TriageUnavailable))
            }
            ChatSourceKind::Readiness => {
                if crate::workstation::qa_readiness_label(&source.id).is_none() {
                    return Err(crate::missing());
                }
                let Some(status) =
                    current_status(target, state, setting.generation, &source.id, deadline).await
                else {
                    return Ok(QaRelatedActivity::TriageUnavailable);
                };
                if context.permits(Permission::ReadWorkstationActivity) {
                    if let Some(related) =
                        jobs_for_check(state, context, target, &setting, &source.id).await?
                    {
                        return Ok(related);
                    }
                }
                Ok(status)
            }
            _ => Err(crate::bad()),
        }
    };
    let result = tokio::time::timeout_at(deadline, result)
        .await
        .unwrap_or(Ok(QaRelatedActivity::TriageUnavailable))?;
    context.revalidate(&state.repo).await?;
    let current = store::setting(&state.repo).await.map_err(storage_error)?;
    if !current.enabled
        || current.generation != setting.generation
        || current.decision_generation != setting.decision_generation
    {
        return Ok(QaRelatedActivity::TriageUnavailable);
    }
    if let QaRelatedActivity::Found { target, .. } = &result {
        if target.kind == ChatSourceKind::Job {
            let scope = activity_api::scope_for_request(&state.repo, context, None).await?;
            let job =
                activity_api::authorized_job(&state.repo, context, &scope, &target.id).await?;
            if !job.private_details_available
                || !OPERATIONS.into_iter().any(|(check, operation)| {
                    check.as_str() == source.id && operation == job.job.operation
                })
            {
                return Ok(QaRelatedActivity::NoRelatedActivity);
            }
        }
    }
    Ok(result)
}

async fn jobs_for_check(
    state: &ApiState,
    context: &ResolvedRequest,
    target: &QaTarget,
    setting: &store::Setting,
    id: &str,
) -> Result<Option<QaRelatedActivity>, ApiError> {
    let Some((_, operation)) = OPERATIONS
        .into_iter()
        .find(|(check, _)| check.as_str() == id)
    else {
        return Ok(None);
    };
    let scope = activity_api::scope_for_request(&state.repo, context, None).await?;
    let filter = JobFilter {
        operation: operation.to_owned(),
        ..JobFilter::default()
    };
    let rows = jobs_view::list_activity_jobs(
        state.repo.db(),
        context.scope_id(),
        &scope,
        &filter,
        MAX_CANDIDATES + 1,
    )
    .await
    .map_err(crate::internal)?;
    if rows.len() > MAX_CANDIDATES {
        return Ok(Some(QaRelatedActivity::TriageUnavailable));
    }
    let private = rows
        .iter()
        .filter(|row| row.private_details_available)
        .collect::<Vec<_>>();
    let row = match private.as_slice() {
        [] => return Ok(None),
        [row] => *row,
        candidates => {
            let Some(runtime) = target
                .decision
                .as_ref()
                .filter(|_| setting.decision_preference != QaDecisionPreference::RulesOnly)
            else {
                return Ok(Some(QaRelatedActivity::TriageUnavailable));
            };
            let label = crate::workstation::qa_readiness_label(id).ok_or_else(crate::missing)?;
            let menu = DecisionMenu {
                state: format!(
                    "A workstation readiness notice needs relevant recorded activity for {label}."
                ),
                candidates: candidates
                    .iter()
                    .enumerate()
                    .map(|(index, row)| {
                        let fact = activity_projection(&row.job, "This job");
                        format!(
                            "Job {} in newest-first order. {}",
                            index + 1,
                            fact.explanation()
                        )
                    })
                    .collect(),
            };
            match runtime
                .score(
                    setting.decision_preference,
                    setting.decision_generation,
                    menu,
                )
                .await
            {
                DecisionChoice::Candidate(index) => {
                    let Some(row) = candidates.get(index) else {
                        return Ok(Some(QaRelatedActivity::TriageUnavailable));
                    };
                    *row
                }
                DecisionChoice::Abstain => return Ok(Some(QaRelatedActivity::NoRelatedActivity)),
                DecisionChoice::Unavailable => {
                    return Ok(Some(QaRelatedActivity::TriageUnavailable))
                }
            }
        }
    };
    let target = ChatSourceReference::new(ChatSourceKind::Job, row.job.id.clone())
        .map_err(|_| crate::bad())?;
    Ok(Some(QaRelatedActivity::Found {
        target,
        reason: QaRelationshipReason::SharedOperation,
    }))
}

#[cfg(test)]
mod tests {
    use super::activity_projection;
    use super::JobRow;

    #[test]
    fn projects_only_allowlisted_stage_evidence() {
        let mut job = JobRow {
            operation: "actor_reindex".into(),
            state: "failed".into(),
            error_summary: "https://private.example/user/secret".into(),
            ..JobRow::default()
        };
        assert_eq!(
            activity_projection(&job, "Actor indexing job").explanation(),
            "Actor indexing job is failed. No verified cause was recorded for this job."
        );
        job.error_summary = "setup_worker_memory_pressure".into();
        assert_eq!(
            activity_projection(&job, "Actor indexing job").explanation(),
            "Actor indexing job is failed. The recorded reason was worker memory pressure."
        );
    }
}
