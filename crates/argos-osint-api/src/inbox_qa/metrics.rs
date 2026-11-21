//! Numeric observations from the backend host and the registered Jobs dashboard.

use std::time::Duration;

use argos_osint_core::storage::inbox_qa::MAX_EVIDENCE_AGE_MS;
use argos_osint_core::storage::jobs_view;
use chrono::DateTime;
use chrono::Utc;
use fleetworks_contracts::chat::ChatSourceKind;
use fleetworks_contracts::context::Permission;
use tokio::time::Instant;

use super::question::Metric;
use crate::activity;
use crate::host_metrics;
use crate::host_metrics::SharedSampler;
use crate::host_metrics::TimedHostMetrics;
use crate::request_context::ResolvedRequest;
use crate::ApiState;

pub(super) struct Fact {
    pub(super) kind: ChatSourceKind,
    pub(super) id: &'static str,
    pub(super) label: &'static str,
    pub(super) text: String,
    pub(super) observed_at: DateTime<Utc>,
}

pub(super) async fn collect(
    state: &ApiState,
    context: &ResolvedRequest,
    sampler: &SharedSampler,
    metric: Metric,
    boundary: i64,
    deadline: Instant,
) -> Option<Fact> {
    if Instant::now() >= deadline {
        return None;
    }
    match metric {
        Metric::Cpu | Metric::Memory | Metric::Disk => {
            if !context.permits(Permission::ReadBackend) {
                return None;
            }
            observe_host(sampler, metric, boundary, deadline).await
        }
        Metric::QueuedJobs | Metric::RunningJobs | Metric::RetryingJobs => {
            tokio::time::timeout_at(deadline, observe_jobs(state, context, metric, boundary))
                .await
                .ok()
                .flatten()
        }
    }
}

async fn observe_host(
    sampler: &SharedSampler,
    metric: Metric,
    boundary: i64,
    deadline: Instant,
) -> Option<Fact> {
    for can_retry in [true, true, false] {
        let disk_required_after = matches!(metric, Metric::Disk)
            .then(|| boundary.max(Utc::now().timestamp_millis() - MAX_EVIDENCE_AGE_MS));
        let sample =
            host_metrics::sample_timed(sampler.clone(), disk_required_after, deadline).await;
        let next_sample_at = if let Some(sample) = sample {
            if let Some(fact) = host_fact(&sample, &metric, boundary, Utc::now()) {
                return Some(fact);
            }
            Instant::from_std(sample.next_sample_at)
        } else {
            Instant::now() + Duration::from_millis(100)
        };
        if !can_retry || next_sample_at >= deadline {
            break;
        }
        tokio::time::sleep_until(next_sample_at).await;
    }
    None
}

fn host_fact(
    sample: &TimedHostMetrics,
    metric: &Metric,
    boundary: i64,
    now: DateTime<Utc>,
) -> Option<Fact> {
    let (id, label, value, observed_at) = match metric {
        Metric::Cpu => (
            "cpu",
            "API host CPU usage",
            sample.cpu_percent?,
            sample.observed_at,
        ),
        Metric::Memory => (
            "memory",
            "API host memory usage",
            sample.memory_percent?,
            sample.observed_at,
        ),
        Metric::Disk => (
            "disk",
            "API host storage volume usage",
            sample.disk_percent?,
            sample.disk_observed_at?,
        ),
        _ => return None,
    };
    if !(0.0..=100.0).contains(&value) || !fresh(observed_at, boundary, now) {
        return None;
    }
    Some(Fact {
        kind: ChatSourceKind::HostMetric,
        id,
        label,
        text: format!("The {label} is {value:.1}%."),
        observed_at,
    })
}

async fn observe_jobs(
    state: &ApiState,
    context: &ResolvedRequest,
    metric: Metric,
    boundary: i64,
) -> Option<Fact> {
    // The same registered cohort and top-level buckets used by Workstation
    // Status. This path fetches aggregate counts without retrieving any rows.
    let scope = activity::scope_for_request(&state.repo, context, None)
        .await
        .ok()?;
    if Utc::now().timestamp_millis() <= boundary {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let observed_at = Utc::now();
    if observed_at.timestamp_millis() <= boundary {
        return None;
    }
    let counts = jobs_view::activity_job_counts(state.repo.db(), context.scope_id(), &scope)
        .await
        .ok()?;
    if !fresh(observed_at, boundary, Utc::now()) {
        return None;
    }
    let (id, label, count) = match metric {
        Metric::QueuedJobs => ("queued", "Queued workstation jobs", counts.queued),
        Metric::RunningJobs => ("running", "Running workstation jobs", counts.active),
        Metric::RetryingJobs => ("retrying", "Retrying workstation jobs", counts.retrying),
        _ => return None,
    };
    let count = u64::try_from(count).ok()?;
    let noun = if count == 1 { "job" } else { "jobs" };
    Some(Fact {
        kind: ChatSourceKind::JobCount,
        id,
        label,
        text: format!("This workstation has {count} {id} {noun}."),
        observed_at,
    })
}

fn fresh(observed_at: DateTime<Utc>, boundary: i64, now: DateTime<Utc>) -> bool {
    let observed_ms = observed_at.timestamp_millis();
    let now_ms = now.timestamp_millis();
    observed_ms > boundary && (now_ms - MAX_EVIDENCE_AGE_MS..=now_ms).contains(&observed_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_fact_preserves_zero_and_rejects_missing_or_stale_measurements() {
        let now = DateTime::from_timestamp_millis(10_000).unwrap();
        let before_boundary = DateTime::from_timestamp_millis(9_000).unwrap();
        let sample = TimedHostMetrics {
            cpu_percent: Some(0.0),
            memory_percent: None,
            disk_percent: Some(30.0),
            observed_at: now,
            disk_observed_at: Some(before_boundary),
            next_sample_at: std::time::Instant::now(),
        };

        let boundary = before_boundary.timestamp_millis();
        let cpu = host_fact(&sample, &Metric::Cpu, boundary, now).unwrap();
        assert_eq!(cpu.text, "The API host CPU usage is 0.0%.");
        assert_eq!(cpu.observed_at, now);
        assert!(host_fact(&sample, &Metric::Memory, boundary, now).is_none());
        assert!(host_fact(&sample, &Metric::Disk, boundary, now).is_none());
        let disk = host_fact(&sample, &Metric::Disk, boundary - 1, now).unwrap();
        assert_eq!(disk.observed_at, before_boundary);
        assert!(host_fact(
            &sample,
            &Metric::Disk,
            0,
            DateTime::from_timestamp_millis(14_001).unwrap(),
        )
        .is_none());
    }
}
