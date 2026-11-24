//! Numeric observations from the backend host and the registered Jobs dashboard.

use std::time::Duration;

use argos_osint_core::storage::inbox_qa::MAX_EVIDENCE_AGE_MS;
use argos_osint_core::storage::jobs_view;
use chrono::DateTime;
use chrono::Utc;
use fleetworks_contracts::chat::ChatSourceKind;
use fleetworks_contracts::chat::QaMeasurement;
use fleetworks_contracts::chat::QaMeasurementUnit;
use fleetworks_contracts::context::Permission;
use tokio::time::Instant;

use super::question::Metric;
use super::question::Question;
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
    pub(super) measurement: QaMeasurement,
}

/// Capture the available measurements from the authorized host and Jobs scope.
pub(super) async fn collect_snapshot(
    state: &ApiState,
    context: &ResolvedRequest,
    sampler: Option<&SharedSampler>,
    intent: Question,
    boundary: i64,
    deadline: Instant,
) -> Vec<(Metric, Fact)> {
    if Instant::now() >= deadline {
        return Vec::new();
    }
    let (mut facts, jobs) = tokio::join!(
        async {
            if let Some(sampler) = sampler.filter(|_| context.permits(Permission::ReadBackend)) {
                observe_host(sampler, intent, boundary, deadline).await
            } else {
                Vec::new()
            }
        },
        async {
            if [
                Metric::QueuedJobs,
                Metric::RunningJobs,
                Metric::RetryingJobs,
                Metric::FailedJobs,
            ]
            .into_iter()
            .any(|metric| intent.wants_measurement(metric))
            {
                tokio::time::timeout_at(deadline, observe_jobs(state, context, intent, boundary))
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default()
            } else {
                Vec::new()
            }
        },
    );
    facts.extend(jobs);
    let now = Utc::now();
    facts.retain(|(metric, fact)| {
        intent.wants_measurement(*metric) && fresh(fact.observed_at, boundary, now)
    });
    facts
}

async fn observe_host(
    sampler: &SharedSampler,
    intent: Question,
    boundary: i64,
    deadline: Instant,
) -> Vec<(Metric, Fact)> {
    let mut observations = [
        (Metric::Cpu, None),
        (Metric::Memory, None),
        (Metric::Disk, None),
    ];
    let requested_is_complete = |observations: &[(Metric, Option<Fact>)]| {
        observations
            .iter()
            .all(|(metric, fact)| !intent.wants_measurement(*metric) || fact.is_some())
    };
    if requested_is_complete(&observations) {
        return Vec::new();
    }
    for can_retry in [true, true, false] {
        if Instant::now() >= deadline {
            break;
        }
        let needs_disk = intent.wants_measurement(Metric::Disk)
            && observations
                .iter()
                .any(|(metric, fact)| *metric == Metric::Disk && fact.is_none());
        let disk_required_after =
            needs_disk.then(|| boundary.max(Utc::now().timestamp_millis() - MAX_EVIDENCE_AGE_MS));
        let sample =
            host_metrics::sample_timed(sampler.clone(), disk_required_after, deadline).await;
        let retry_floor = Instant::now() + Duration::from_millis(100);
        let next_sample_at = if let Some(sample) = sample {
            let now = Utc::now();
            for (metric, fact) in &mut observations {
                if intent.wants_measurement(*metric) && fact.is_none() {
                    *fact = host_fact(&sample, *metric, boundary, now);
                }
            }
            // A retained disk reading can precede a new CPU baseline.
            Instant::from_std(sample.next_sample_at).max(retry_floor)
        } else {
            retry_floor
        };
        if !can_retry || requested_is_complete(&observations) || next_sample_at >= deadline {
            break;
        }
        tokio::time::sleep_until(next_sample_at).await;
    }
    observations
        .into_iter()
        .filter_map(|(metric, fact)| fact.map(|fact| (metric, fact)))
        .collect()
}

fn host_fact(
    sample: &TimedHostMetrics,
    metric: Metric,
    boundary: i64,
    now: DateTime<Utc>,
) -> Option<Fact> {
    let (id, label, reading) = match metric {
        Metric::Cpu => ("cpu", "API host CPU usage", sample.cpu?),
        Metric::Memory => ("memory", "API host memory usage", sample.memory?),
        Metric::Disk => ("disk", "API host storage volume usage", sample.disk?),
        _ => return None,
    };
    if !(0.0..=100.0).contains(&reading.value) || !fresh(reading.observed_at, boundary, now) {
        return None;
    }
    let value = (reading.value * 10.0).round() as u64;
    Some(Fact {
        kind: ChatSourceKind::HostMetric,
        id,
        label,
        text: format!("The {label} is {}.{}%.", value / 10, value % 10),
        observed_at: reading.observed_at,
        measurement: QaMeasurement::new(value, QaMeasurementUnit::PercentTenths, label.to_owned()),
    })
}

async fn observe_jobs(
    state: &ApiState,
    context: &ResolvedRequest,
    intent: Question,
    boundary: i64,
) -> Option<Vec<(Metric, Fact)>> {
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
    if intent.wants_measurement(Metric::FailedJobs) {
        let count = jobs_view::activity_failed_jobs(
            state.repo.db(),
            context.scope_id(),
            &scope,
            observed_at,
        )
        .await
        .ok()
        .and_then(|count| u64::try_from(count).ok())?;
        let noun = if count == 1 { "job" } else { "jobs" };
        let label = "Failed workstation jobs in the last 24 hours";
        return Some(vec![(
            Metric::FailedJobs,
            Fact {
                kind: ChatSourceKind::JobCount,
                id: "failed_last_24_hours",
                label,
                text: format!("{count} {noun} failed on this workstation in the last 24 hours."),
                observed_at,
                measurement: QaMeasurement::new(count, QaMeasurementUnit::Jobs, label.to_owned()),
            },
        )]);
    }
    let counts = jobs_view::activity_job_counts(state.repo.db(), context.scope_id(), &scope)
        .await
        .ok()?;
    Some(
        [
            (
                Metric::QueuedJobs,
                "queued",
                "Queued workstation jobs",
                counts.queued,
            ),
            (
                Metric::RunningJobs,
                "running",
                "Running workstation jobs",
                counts.active,
            ),
            (
                Metric::RetryingJobs,
                "retrying",
                "Retrying workstation jobs",
                counts.retrying,
            ),
        ]
        .into_iter()
        .filter_map(|(metric, id, label, count)| {
            let count = u64::try_from(count).ok()?;
            let noun = if count == 1 { "job" } else { "jobs" };
            Some((
                metric,
                Fact {
                    kind: ChatSourceKind::JobCount,
                    id,
                    label,
                    text: format!("This workstation has {count} {id} {noun}."),
                    observed_at,
                    measurement: QaMeasurement::new(
                        count,
                        QaMeasurementUnit::Jobs,
                        label.to_owned(),
                    ),
                },
            ))
        })
        .collect(),
    )
}

fn fresh(observed_at: DateTime<Utc>, boundary: i64, now: DateTime<Utc>) -> bool {
    let observed_ms = observed_at.timestamp_millis();
    let now_ms = now.timestamp_millis();
    observed_ms > boundary && (now_ms - MAX_EVIDENCE_AGE_MS..=now_ms).contains(&observed_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_metrics::ObservedPercent;

    #[test]
    fn host_facts_freeze_exact_values_units_subjects_and_independent_timestamps() {
        let now = DateTime::from_timestamp_millis(10_000).unwrap();
        let cpu_at = DateTime::from_timestamp_millis(9_970).unwrap();
        let disk_at = DateTime::from_timestamp_millis(9_000).unwrap();
        let mut sample = TimedHostMetrics {
            cpu: Some(ObservedPercent {
                value: 0.0,
                observed_at: cpu_at,
            }),
            memory: Some(ObservedPercent {
                value: 47.2,
                observed_at: now,
            }),
            disk: Some(ObservedPercent {
                value: 30.5,
                observed_at: disk_at,
            }),
            next_sample_at: std::time::Instant::now(),
        };

        let boundary = disk_at.timestamp_millis();
        let cpu = host_fact(&sample, Metric::Cpu, boundary, now).unwrap();
        assert_eq!(cpu.text, "The API host CPU usage is 0.0%.");
        assert_eq!(
            cpu.measurement,
            QaMeasurement::new(
                0,
                QaMeasurementUnit::PercentTenths,
                "API host CPU usage".into()
            )
        );
        assert_eq!(cpu.observed_at, cpu_at);

        let memory = host_fact(&sample, Metric::Memory, boundary, now).unwrap();
        assert_eq!(memory.text, "The API host memory usage is 47.2%.");
        assert_eq!(
            memory.measurement,
            QaMeasurement::new(
                472,
                QaMeasurementUnit::PercentTenths,
                "API host memory usage".into()
            )
        );
        assert_eq!(memory.observed_at, now);

        assert!(host_fact(&sample, Metric::Disk, boundary, now).is_none());
        let disk = host_fact(&sample, Metric::Disk, boundary - 1, now).unwrap();
        assert_eq!(disk.text, "The API host storage volume usage is 30.5%.");
        assert_eq!(disk.measurement.value, 305);
        assert_eq!(disk.observed_at, disk_at);
        assert!(host_fact(
            &sample,
            Metric::Disk,
            0,
            DateTime::from_timestamp_millis(14_001).unwrap(),
        )
        .is_none());

        let later = DateTime::from_timestamp_millis(11_000).unwrap();
        sample.memory = Some(ObservedPercent {
            value: 51.8,
            observed_at: later,
        });
        sample.cpu = None;
        let next = host_fact(&sample, Metric::Memory, now.timestamp_millis(), later).unwrap();
        assert_eq!(next.measurement.value, 518);
        assert_eq!(next.observed_at, later);
        assert_eq!(
            memory.measurement.value, 472,
            "a later measurement cannot rewrite a frozen fact"
        );
        assert!(host_fact(&sample, Metric::Cpu, boundary, later).is_none());
    }

    #[cfg(feature = "embedded")]
    #[tokio::test]
    async fn job_counts_respect_registered_workstations_and_failure_window() {
        use std::sync::Arc;

        use argos_osint_core::storage::bootstrap;
        use argos_osint_core::storage::connect_registry;
        use argos_osint_core::storage::connect_registry::RegistryAuthority;
        use argos_osint_core::storage::job_context::InteractiveAdmission;
        use argos_osint_core::storage::job_dispatch;
        use argos_osint_core::storage::jobs::NewJob;
        use argos_osint_core::storage::jobs::NewTask;
        use argos_osint_core::storage::open::Target;
        use argos_osint_core::storage::repo::Repository;
        use fleetworks_contracts::connect::RegistrationEnsureRequest;
        use fleetworks_contracts::context::JobOrigin;
        use fleetworks_contracts::context::OriginSource;
        use surrealdb::types::RecordId;

        use crate::request_context;
        use crate::request_context::AdmissionProof;
        use crate::request_context::ContextRequirement;
        use crate::AuthenticatedPrincipal;

        let directory = tempfile::tempdir().unwrap();
        let repo = Repository::connect(
            &Target::Embedded {
                path: directory.path().join("store"),
            },
            "inbox_metric_qa",
            "job_counts",
        )
        .await
        .unwrap();
        bootstrap::ensure_network_identity(repo.db()).await.unwrap();
        connect_registry::initialize_control(repo.db())
            .await
            .unwrap();
        let state = ApiState::new(repo, "fixture-instance".into(), 0, Arc::new(Vec::new()));
        let principal = AuthenticatedPrincipal::for_local_ipc(
            "metric-native",
            "metric-scope",
            "metric-access",
            vec![
                Permission::ReadWorkstationActivity,
                Permission::RegisterWorkstation,
            ],
        )
        .unwrap();
        let authority = RegistryAuthority::new(
            principal.scope_id(),
            principal.credential_id(),
            principal.access_epoch(),
        )
        .unwrap();
        let mut requests = Vec::new();
        for (index, ip) in ["192.0.2.42", "198.51.100.42"].into_iter().enumerate() {
            let receipt = connect_registry::ensure(
                &state.repo,
                &authority,
                &RegistrationEnsureRequest::new(
                    format!("metric-ensure-{index}"),
                    Some(ip.parse().unwrap()),
                    format!("metric-installation-{index}"),
                ),
            )
            .await
            .unwrap()
            .receipt;
            let request = request_context::resolve_local(
                &state.repo,
                &principal,
                AdmissionProof::Native {
                    registration_id: receipt.registration_id,
                    expected_registration_revision: receipt.revision,
                },
                Permission::ReadWorkstationActivity,
                ContextRequirement::Registered,
            )
            .await
            .unwrap();
            requests.push(request);
        }
        let [first, second] = requests.as_slice() else {
            panic!("two registered workstations")
        };
        let first_context = first.registered().unwrap();
        assert_ne!(
            first_context.workstation_id,
            second.registered().unwrap().workstation_id
        );
        let now = Utc::now();
        let origin = JobOrigin::new(
            first_context,
            OriginSource::Native,
            "metric-fixture-request".into(),
            now.to_rfc3339(),
            1,
            Vec::new(),
        );
        let admission =
            InteractiveAdmission::native(authority, first_context.clone(), origin).unwrap();
        let db = state.repo.db();
        job_dispatch::enable_pool(db, "metric_fixture")
            .await
            .unwrap();
        job_dispatch::submit_interactive(
            db,
            &NewJob {
                id: "metric-fixture-job",
                parent: None,
                kind: "task",
                operation: "summaries",
                created_at: now,
            },
            &NewTask {
                id: "metric-fixture-task",
                job: "metric-fixture-job",
                pool: "metric_fixture",
                operation: "summaries",
                input_hash: "metric-fixture-input",
                priority: 0,
                deps: vec![],
                created_at: now,
            },
            &admission,
        )
        .await
        .unwrap();
        // Only job execution state is synthetic. The provenance and membership
        // checked below were produced by ordinary registration/dispatch admission.
        db.query("UPDATE $job SET state='running' RETURN NONE")
            .bind(("job", RecordId::new("job", "metric-fixture-job")))
            .await
            .unwrap()
            .check()
            .unwrap();

        for (request, expected, text) in [
            (first, 1, "This workstation has 1 running job."),
            (second, 0, "This workstation has 0 running jobs."),
        ] {
            let facts = collect_snapshot(
                &state,
                request,
                None,
                Question::JobsSummary,
                0,
                Instant::now() + Duration::from_secs(4),
            )
            .await;
            assert_eq!(facts.len(), 3);
            let observation = |metric| {
                &facts
                    .iter()
                    .find(|(observed, _)| *observed == metric)
                    .unwrap()
                    .1
            };
            let fact = observation(Metric::RunningJobs);
            let scope = activity::scope_for_request(&state.repo, request, None)
                .await
                .unwrap();
            let dashboard = jobs_view::activity_job_counts(db, request.scope_id(), &scope)
                .await
                .unwrap();
            assert_eq!(dashboard.active, expected);
            assert_eq!(
                observation(Metric::QueuedJobs).measurement.value,
                dashboard.queued as u64
            );
            assert_eq!(
                observation(Metric::RetryingJobs).measurement.value,
                dashboard.retrying as u64
            );
            assert!(facts
                .iter()
                .all(|(_, observation)| observation.observed_at == fact.observed_at));
            assert_eq!(
                fact.measurement,
                QaMeasurement::new(
                    expected as u64,
                    QaMeasurementUnit::Jobs,
                    "Running workstation jobs".into()
                )
            );
            assert_eq!(fact.text, text);
        }

        db.query("UPDATE $job SET state='failed', finished_at=<datetime>$at RETURN NONE")
            .bind(("job", RecordId::new("job", "metric-fixture-job")))
            .bind(("at", now.to_rfc3339()))
            .await
            .unwrap()
            .check()
            .unwrap();
        for (request, count, noun) in [(first, 1, "job"), (second, 0, "jobs")] {
            let facts = collect_snapshot(
                &state,
                request,
                None,
                Question::Metric(Metric::FailedJobs),
                0,
                Instant::now() + Duration::from_secs(4),
            )
            .await;
            assert_eq!(facts.len(), 1);
            assert_eq!(facts[0].0, Metric::FailedJobs);
            let fact = &facts[0].1;
            assert_eq!(fact.measurement.value, count);
            assert_eq!(
                fact.text,
                format!("{count} {noun} failed on this workstation in the last 24 hours.")
            );
            assert!(fact.observed_at >= now);
        }

        db.query("UPDATE $job SET finished_at=<datetime>$at RETURN NONE")
            .bind(("job", RecordId::new("job", "metric-fixture-job")))
            .bind(("at", (now - chrono::Duration::hours(25)).to_rfc3339()))
            .await
            .unwrap()
            .check()
            .unwrap();
        let scope = activity::scope_for_request(&state.repo, first, None)
            .await
            .unwrap();
        let count = jobs_view::activity_failed_jobs(db, first.scope_id(), &scope, now)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
}
