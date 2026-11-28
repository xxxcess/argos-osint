//! Typed access to optional, Embedded-only workstation observations.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use argos_osint_core::storage::jobs_view;
use chrono::DateTime;
use chrono::Utc;
use fleetworks_contracts::chat::QaAvailabilityReason;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::context::ResourceScope;
use fleetworks_contracts::readiness::ReadinessCheck;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use futures_util::stream;
use futures_util::FutureExt;
use futures_util::StreamExt;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;

use crate::activity;
use crate::request_context::ResolvedRequest;
use crate::workstation;
use crate::ApiState;

/// An allowlisted resource, independent of any caller-provided storage expression.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Resource {
    Readiness(ReadinessCheckId),
    Jobs,
    Job(String),
    Backend,
    Generator,
}

impl Resource {
    /// Return the storage family and optional exact resource identity.
    #[must_use]
    pub fn family_and_key(&self) -> (&'static str, Option<&str>) {
        match self {
            Self::Readiness(check) => ("readiness", Some(check.as_str())),
            Self::Jobs => ("jobs", None),
            Self::Job(id) => ("jobs", Some(id)),
            Self::Backend => ("backend", Some("embedded")),
            Self::Generator => ("generator", Some("generator")),
        }
    }

    fn is_job(&self) -> bool {
        matches!(self, Self::Job(_) | Self::Jobs)
    }

    fn includes(&self, other: &Self) -> bool {
        self == other || matches!((self, other), (Self::Jobs, Self::Job(_)))
    }
}

/// An observed lifecycle state, excluding diagnostics and worker output.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Lifecycle {
    Ready,
    Stopping,
}

/// A confirmed Embedded inference lifecycle state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum GeneratorState {
    Ready,
    Unloaded,
    Unavailable,
    Disabled,
}

/// Allowlisted job lifecycle states. Provider errors and job titles are excluded.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum JobState {
    Queued,
    Running,
    RetryScheduled,
    Paused,
    Waiting,
    Completed,
    Partial,
    Failed,
    Cancelled,
    Superseded,
    Blocked,
}

impl JobState {
    fn from_projection(state: &str) -> Option<Self> {
        Some(match state {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "retry_scheduled" => Self::RetryScheduled,
            "paused" => Self::Paused,
            "waiting" => Self::Waiting,
            "completed" | "succeeded" => Self::Completed,
            "partial" => Self::Partial,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "superseded" => Self::Superseded,
            "blocked" => Self::Blocked,
            _ => return None,
        })
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::RetryScheduled => "waiting for retry",
            Self::Paused => "paused",
            Self::Waiting => "waiting",
            Self::Completed => "completed",
            Self::Partial => "partially completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Superseded => "superseded",
            Self::Blocked => "blocked",
        }
    }

    fn terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Partial | Self::Failed | Self::Cancelled | Self::Superseded
        )
    }
}

macro_rules! readiness_reasons {
    ($($variant:ident => $code:literal, $sentence:literal);+ $(;)?) => {
        /// Stable health reason codes with no details from the reporting process.
        #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
        #[serde(rename_all = "snake_case")]
        #[non_exhaustive]
        pub enum ReadinessReason {
            $($variant,)+
        }

        impl ReadinessReason {
            fn from_code(code: &str) -> Option<Self> {
                match code {
                    $($code => Some(Self::$variant),)+
                    _ => None,
                }
            }

            fn passage(self) -> &'static str {
                match self {
                    $(Self::$variant => $sentence,)+
                }
            }
        }
    };
}

readiness_reasons! {
    GatewayReachable => "gateway_reachable", "An authenticated gateway request succeeded.";
    DatastoreReachable => "datastore_reachable", "A bounded datastore read succeeded.";
    DatastoreUnavailable => "datastore_unavailable", "The bounded datastore read failed.";
    ProcessNotReady => "process_not_ready", "The configured process reported that it was unready.";
    OwnerUnavailable => "owner_unavailable", "The configured status probe did not answer.";
    InferenceOwnerUnavailable => "inference_owner_unavailable", "The inference owner did not answer.";
    OwnerNotConfigured => "owner_not_configured", "No status owner was configured.";
    DatabaseSeeded => "database_seeded", "A completed seed and canonical actor records were observed.";
    DatabaseCatalogEmpty => "database_catalog_empty", "The completed seed had no canonical actor records.";
    DatabaseSeedIncomplete => "database_seed_incomplete", "No completed seed was verified.";
    DatabaseSeedRunning => "database_seed_running", "The accepted seed had not completed.";
    SeedReceiptUnavailable => "seed_receipt_unavailable", "The seed receipt could not be checked.";
    IndexReaderVisible => "index_reader_visible", "The index was visible through the observed source checkpoint.";
    IndexBuilding => "index_building", "Index visibility was incomplete.";
    IndexCatchingUp => "index_catching_up", "The index had not caught up to the source.";
    IndexNotBuilt => "index_not_built", "No serving index was observed.";
    IndexSourceNotStarted => "index_source_not_started", "The index source had not started.";
    IndexIngestPaused => "index_ingest_paused", "Index ingestion was paused.";
    IndexRecoveryRequired => "index_recovery_required", "The index owner reported a recoverable generation error.";
    IndexRebuildInterrupted => "index_rebuild_interrupted", "The rebuild stopped before catching up.";
    IndexRebuildCorrectionRequired => "index_rebuild_correction_required", "The rebuild needed a correction to resume.";
    ModelUnverified => "model_unverified", "Reachability alone did not establish verified inference.";
    ModelVerified => "model_verified", "The active model was verified.";
    LegacyClassifierRetired => "legacy_classifier_retired", "Legacy Natural training was retired.";
}

/// Projection permitted in the private journal. Full rows never cross this boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    content = "observation",
    rename_all = "snake_case",
    deny_unknown_fields
)]
#[non_exhaustive]
pub enum ObservationValue {
    Readiness {
        check: ReadinessCheckId,
        state: ReadinessState,
        reason: Option<ReadinessReason>,
    },
    Job {
        id: String,
        state: JobState,
    },
    Backend(Lifecycle),
    Generator {
        state: GeneratorState,
        reason: Option<QaAvailabilityReason>,
    },
}

impl ObservationValue {
    /// Return the stable resource that supplied this fact.
    #[must_use]
    pub fn resource(&self) -> Resource {
        match self {
            Self::Readiness { check, .. } => Resource::Readiness(*check),
            Self::Job { id, .. } => Resource::Job(id.clone()),
            Self::Backend(_) => Resource::Backend,
            Self::Generator { .. } => Resource::Generator,
        }
    }
}

/// A scoped observation staged only by the in-process gateway or companion supervisor.
#[non_exhaustive]
pub struct Record {
    pub scope: String,
    pub generation: i64,
    pub revision: i64,
    pub observed_at_ms: i64,
    pub value: ObservationValue,
    pub terminal: bool,
}

/// A typed current read or requested history interval.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub enum Range {
    Current,
    History { since_ms: i64, until_ms: i64 },
}

/// Internally authorized storage selector. The UI cannot supply scope or resource keys.
#[non_exhaustive]
pub struct Query {
    pub scope: String,
    pub resource: Resource,
    pub range: Range,
}

/// A retained observation, with original source timing separate from collection.
#[non_exhaustive]
pub struct Entry {
    pub sequence: i64,
    pub observed_at_ms: i64,
    pub recorded_at_ms: i64,
    pub value: ObservationValue,
}

impl Entry {
    /// Restore an allowlisted journal value with its source timing.
    #[must_use]
    pub fn new(
        sequence: i64,
        observed_at_ms: i64,
        recorded_at_ms: i64,
        value: ObservationValue,
    ) -> Self {
        Self {
            sequence,
            observed_at_ms,
            recorded_at_ms,
            value,
        }
    }

    /// Render a historical fact without provider text, logs, or model output.
    #[must_use]
    pub fn passage(&self) -> String {
        match &self.value {
            ObservationValue::Readiness { check, state, reason } => {
                let label = workstation::qa_readiness_label(check.as_str()).unwrap_or("Backend check");
                let status = match *state {
                    ReadinessState::NotReady => "not ready",
                    ReadinessState::NotApplicable => "not applicable",
                    other => other.as_str(),
                };
                let mut passage = format!("{label} was observed as {status}.");
                if let Some(reason) = reason {
                    passage.push(' ');
                    passage.push_str(reason.passage());
                }
                passage
            }
            ObservationValue::Job { id, state } if safe_job_id(id) => {
                format!("Job {id} was observed as {}.", state.as_str())
            }
            ObservationValue::Job { state, .. } => format!("A job was observed as {}.", state.as_str()),
            ObservationValue::Backend(Lifecycle::Ready) => "The Embedded companion became ready.".to_owned(),
            ObservationValue::Backend(Lifecycle::Stopping) => "The Embedded companion began shutting down.".to_owned(),
            ObservationValue::Generator { state, reason } => match state {
                GeneratorState::Ready => "The Embedded generator became ready.",
                GeneratorState::Unloaded => "The Embedded generator was unloaded.",
                GeneratorState::Disabled => "Embedded inference was disabled.",
                GeneratorState::Unavailable => match reason {
                    Some(QaAvailabilityReason::PackMissing) => "The Embedded generator was unavailable because no verified pack was installed.",
                    Some(QaAvailabilityReason::PackInvalid) => "The Embedded generator was unavailable because the pack could not be verified.",
                    Some(QaAvailabilityReason::SandboxUnsupported) => "The Embedded generator was unavailable because its sandbox could not be enforced.",
                    Some(QaAvailabilityReason::InsufficientResources) => "The Embedded generator was unavailable because workstation memory was insufficient.",
                    Some(QaAvailabilityReason::ModelProvisionFailed) => "The Embedded generator was unavailable because its model assets could not be verified.",
                    _ => "The Embedded generator was unavailable.",
                },
            }
            .to_owned(),
        }
    }
}

/// Observation results with bounded, conservative collection coverage.
#[non_exhaustive]
pub struct History {
    pub entries: Vec<Entry>,
    pub coverage_started_at_ms: i64,
    pub retrieved_at_ms: i64,
    pub incomplete: bool,
}

impl History {
    /// Restore the journal's coverage separately from individual observation times.
    #[must_use]
    pub fn new(
        entries: Vec<Entry>,
        coverage_started_at_ms: i64,
        retrieved_at_ms: i64,
        incomplete: bool,
    ) -> Self {
        Self {
            entries,
            coverage_started_at_ms,
            retrieved_at_ms,
            incomplete,
        }
    }
}

/// A cancellable lookup supplied by the Embedded host.
pub type QueryFuture<'a> = Pin<Box<dyn Future<Output = Option<History>> + Send + 'a>>;

/// Optional companion-owned observation journal. Writes must never block request handling.
pub trait Journal: Send + Sync {
    fn record(&self, record: Record);
    fn query(&self, query: Query) -> QueryFuture<'_>;
}

/// Authorizes selection and strips operational data down to the journal allowlist.
pub struct Observer {
    journal: Arc<dyn Journal>,
    deployment: String,
    configuration_epoch: String,
    backend_scope: String,
}

impl Observer {
    /// Bind journal operations to the selected Embedded backend and configuration.
    #[must_use]
    pub fn new(journal: Arc<dyn Journal>, deployment: String, configuration_epoch: String) -> Self {
        let backend_scope = scope_key(&[&deployment, &configuration_epoch, "backend"]);
        Self {
            journal,
            deployment,
            configuration_epoch,
            backend_scope,
        }
    }

    fn registered(&self, request: &ResolvedRequest) -> bool {
        request.context().is_some_and(|context| {
            context.deployment_id == self.deployment
                && context.configuration_epoch == self.configuration_epoch
        })
    }

    fn jobs_scope(&self, request: &ResolvedRequest) -> Option<String> {
        let context = request.context()?;
        if !self.registered(request) || !request.permits(Permission::ReadWorkstationActivity) {
            return None;
        }
        Some(scope_key(&[
            &self.deployment,
            &self.configuration_epoch,
            "jobs",
            request.scope_id(),
            request.access_epoch(),
            &context.registration_id,
            &context.workstation_id,
            &context.scope_revision.to_string(),
        ]))
    }

    pub(crate) fn record_readiness(&self, checks: &[ReadinessCheck]) {
        for check in checks {
            if check.scope != ResourceScope::Backend
                || workstation::qa_readiness_label(check.id.as_str()).is_none()
            {
                continue;
            }
            let Some(observed_at_ms) = timestamp(&check.observed_at) else {
                continue;
            };
            self.journal.record(Record {
                scope: self.backend_scope.clone(),
                generation: 0,
                revision: observed_at_ms,
                observed_at_ms,
                value: ObservationValue::Readiness {
                    check: check.id,
                    state: check.state,
                    reason: ReadinessReason::from_code(&check.reason_code),
                },
                terminal: false,
            });
        }
    }

    pub(crate) fn record_jobs(
        &self,
        request: &ResolvedRequest,
        response: &Value,
        fallback_at_ms: i64,
    ) {
        let Some(scope) = self.jobs_scope(request) else {
            return;
        };
        let observed_at_ms = response
            .get("observed_at")
            .and_then(Value::as_str)
            .and_then(timestamp)
            .unwrap_or(fallback_at_ms);
        if let Some(rows) = response.get("rows").and_then(Value::as_array) {
            for row in rows {
                self.record_job(&scope, row, observed_at_ms);
            }
        } else if let Some(job) = response.get("job") {
            self.record_job(&scope, job, observed_at_ms);
        }
    }

    fn record_job(&self, scope: &str, value: &Value, observed_at_ms: i64) {
        let Ok(row) = JobRead::deserialize(value) else {
            return;
        };
        let Some(state) = JobState::from_projection(row.state) else {
            return;
        };
        if !safe_job_id(row.id) {
            return;
        }
        let finished_at_ms = timestamp(row.finished_at);
        // Polling a long-completed job must not recreate its expired event.
        if state.terminal()
            && finished_at_ms.is_some_and(|finished| {
                finished < observed_at_ms.saturating_sub(7 * 24 * 60 * 60 * 1_000)
            })
        {
            return;
        }
        let revision = [
            timestamp(row.created_at),
            timestamp(row.queued_at),
            timestamp(row.started_at),
            timestamp(row.heartbeat_at),
            finished_at_ms,
        ]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(0);
        self.journal.record(Record {
            scope: scope.to_owned(),
            generation: row.attempts_used.max(0),
            revision,
            observed_at_ms,
            value: ObservationValue::Job {
                id: row.id.to_owned(),
                state,
            },
            terminal: state.terminal(),
        });
    }

    /// Record successful Embedded startup or confirmed orderly shutdown.
    pub fn record_backend(&self, lifecycle: Lifecycle) {
        self.record_lifecycle(0, ObservationValue::Backend(lifecycle));
    }

    /// Record a confirmed generator transition, independently of conversation retention.
    pub fn record_generator(
        &self,
        generation: i64,
        state: GeneratorState,
        reason: Option<QaAvailabilityReason>,
    ) {
        self.record_lifecycle(generation, ObservationValue::Generator { state, reason });
    }

    fn record_lifecycle(&self, generation: i64, value: ObservationValue) {
        let now = Utc::now();
        let observed_at_ms = now.timestamp_millis();
        self.journal.record(Record {
            scope: self.backend_scope.clone(),
            generation,
            revision: now.timestamp_nanos_opt().unwrap_or(observed_at_ms),
            observed_at_ms,
            value,
            terminal: false,
        });
    }

    pub(crate) async fn history(
        &self,
        state: &ApiState,
        request: &ResolvedRequest,
        resource: Resource,
        since_ms: i64,
        until_ms: i64,
    ) -> Option<History> {
        if since_ms >= until_ms {
            return None;
        }
        self.query(
            state,
            request,
            resource,
            Range::History { since_ms, until_ms },
        )
        .await
    }

    pub(crate) async fn query(
        &self,
        state: &ApiState,
        request: &ResolvedRequest,
        resource: Resource,
        range: Range,
    ) -> Option<History> {
        if !self.registered(request) {
            return None;
        }
        let scope = if resource.is_job() {
            self.jobs_scope(request)?
        } else {
            if !request.permits(Permission::ReadBackend) {
                return None;
            }
            self.backend_scope.clone()
        };
        request.revalidate(&state.repo).await.ok()?;
        let mut history = self
            .journal
            .query(Query {
                scope,
                resource: resource.clone(),
                range,
            })
            .await?;
        let found = history.entries.len();
        history
            .entries
            .retain(|entry| resource.includes(&entry.value.resource()));
        if resource.is_job() {
            let scope = activity::scope_for_request(&state.repo, request, None)
                .await
                .ok()?;
            let ids: HashSet<String> = history
                .entries
                .iter()
                .filter_map(|entry| match &entry.value {
                    ObservationValue::Job { id, .. } if safe_job_id(id) => Some(id.clone()),
                    _ => None,
                })
                .collect();
            let db = state.repo.db();
            let authority_scope = request.scope_id();
            let visible: HashSet<String> = stream::iter(ids)
                .map(|id| {
                    let scope = &scope;
                    async move {
                        jobs_view::get_activity_job(db, authority_scope, scope, &id)
                            .await
                            .ok()
                            .flatten()
                            .filter(|job| job.private_details_available)
                            .map(|_| id)
                    }
                    .boxed()
                })
                .buffer_unordered(4)
                .filter_map(std::future::ready)
                .collect()
                .await;
            history.entries.retain(|entry| match &entry.value {
                ObservationValue::Job { id, .. } => visible.contains(id),
                _ => false,
            });
        }
        request.revalidate(&state.repo).await.ok()?;
        history.incomplete |= history.entries.len() != found;
        Some(history)
    }
}

#[derive(Deserialize)]
struct JobRead<'a> {
    id: &'a str,
    state: &'a str,
    created_at: &'a str,
    queued_at: &'a str,
    started_at: &'a str,
    heartbeat_at: &'a str,
    finished_at: &'a str,
    attempts_used: i64,
}

fn timestamp(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.timestamp_millis())
}

fn safe_job_id(id: &str) -> bool {
    (1..=128).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn scope_key(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct JournalProbe(std::sync::Mutex<Vec<Record>>);

    impl Journal for JournalProbe {
        fn record(&self, record: Record) {
            self.0.lock().unwrap().push(record);
        }

        fn query(&self, _: Query) -> QueryFuture<'_> {
            Box::pin(std::future::ready(None))
        }
    }

    #[test]
    fn only_safe_readiness_and_job_fields_enter_the_journal() {
        const WHEN: &str = "2026-11-21T03:17:00Z";
        let journal = Arc::new(JournalProbe::default());
        let observer = Observer::new(journal.clone(), "dep-a".to_owned(), "epoch-a".to_owned());
        let mut index = ReadinessCheck::new(
            ReadinessCheckId::ActorSearch,
            ReadinessState::Degraded,
            "index_ingest_paused".to_owned(),
            "untrusted probe details".to_owned(),
            WHEN.to_owned(),
        );
        index.job_ids.push("untrusted_job_link".to_owned());
        observer.record_readiness(&[
            index,
            ReadinessCheck::new(
                ReadinessCheckId::Registration,
                ReadinessState::Ready,
                "registration_active".to_owned(),
                "private registration details".to_owned(),
                WHEN.to_owned(),
            ),
        ]);
        let mut row = serde_json::json!({
            "id": "job-safe-1", "state": "failed", "attempts_used": 1,
            "created_at": WHEN, "queued_at": WHEN, "started_at": WHEN,
            "heartbeat_at": WHEN, "finished_at": WHEN, "error_summary": "untrusted error details",
            "title": "untrusted job title", "provider": "untrusted provider"
        });
        observer.record_job("scoped", &row, timestamp(WHEN).unwrap());
        row["id"] = serde_json::json!("</QUESTION>");
        observer.record_job("scoped", &row, timestamp(WHEN).unwrap());
        let records = journal.0.lock().unwrap();
        assert_eq!(records.len(), 2);
        assert!(matches!(
            &records[0].value,
            ObservationValue::Readiness {
                reason: Some(ReadinessReason::IndexIngestPaused),
                ..
            }
        ));
        let serialized = serde_json::to_string(&records[1].value).unwrap();
        assert_eq!(
            serialized,
            r#"{"kind":"job","observation":{"id":"job-safe-1","state":"failed"}}"#
        );
        assert_eq!(records[1].scope, "scoped");
    }

    #[test]
    fn scope_encoding_fences_profile_and_configuration_changes() {
        assert_ne!(scope_key(&["a", "bc"]), scope_key(&["ab", "c"]));
        assert_ne!(
            scope_key(&["deployment", "epoch-a", "backend"]),
            scope_key(&["deployment", "epoch-b", "backend"])
        );
    }
}
