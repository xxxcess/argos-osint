//! Transactional Inbox state. Model processes never receive this repository.

use std::collections::BTreeSet;
use std::future::Future;

use chrono::DateTime;
use chrono::Utc;
use fleetworks_contracts::chat::ChatSourceKind;
use fleetworks_contracts::chat::ChatSourceReference;
use fleetworks_contracts::chat::QaConversationReply;
use fleetworks_contracts::chat::QaDecisionPreference;
use fleetworks_contracts::chat::QaEvent;
use fleetworks_contracts::chat::QaEvidenceProvenance;
use fleetworks_contracts::chat::QaFinding;
use fleetworks_contracts::chat::QaGeneratorPreference;
use fleetworks_contracts::chat::QaMeasurement;
use fleetworks_contracts::chat::QaModelTarget;
use fleetworks_contracts::chat::QaRelatedActivity;
use fleetworks_contracts::chat::QaTurn;
use fleetworks_contracts::chat::QaTurnLimitation;
use fleetworks_contracts::chat::QaTurnNotice;
use fleetworks_contracts::chat::QaTurnRequest;
use fleetworks_contracts::chat::QaTurnStage;
use fleetworks_contracts::chat::QaTurnState;
use fleetworks_contracts::chat::QA_MAX_FINDINGS;
use fleetworks_contracts::chat::QA_MAX_RETAINED_TURNS;
use fleetworks_contracts::chat::QA_MAX_WORDS;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::qa_worker::valid_dialogue_reply;
use fleetworks_contracts::qa_worker::valid_draft_delta;
use fleetworks_contracts::qa_worker::valid_operational_draft;
use fleetworks_contracts::qa_worker::valid_uncited_reply;
use fleetworks_contracts::qa_worker::verified_operational_sources;
use fleetworks_contracts::qa_worker::QaContextUsage;
use fleetworks_contracts::qa_worker::QaDialogueExchange;
use fleetworks_contracts::qa_worker::QaDialogueKind;
use fleetworks_contracts::qa_worker::QaGuidance;
use fleetworks_contracts::qa_worker::QaWorkerPlan;
use fleetworks_contracts::qa_worker::QaWorkerRequest;
use fleetworks_contracts::qa_worker::QA_MAX_DIALOGUE_HISTORY;
use fleetworks_contracts::qa_worker::QA_MAX_DIALOGUE_MESSAGE_BYTES;
use fleetworks_contracts::qa_worker::QA_MAX_DRAFT_BYTES;
use fleetworks_contracts::qa_worker::QA_MAX_DRAFT_EVENTS;
use futures_util::{Stream, StreamExt};
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use surrealdb::types::RecordId;
use surrealdb::types::SurrealValue;

use crate::nlp::conversation::Focus;
use crate::nlp::conversation::PendingClarification;
use crate::nlp::Status;

use super::connect_registry;
use super::connect_registry::RegistryAuthority;
use super::job_dispatch;
use super::jobs;
use super::jobs_view;
use super::repo::Repository;
use super::tx::run_tx;
use super::tx::Committed;
use super::tx::RetryPolicy;
use super::tx::Tx;
use super::tx::TxError;

/// Shared Jobs dispatch pool, dedicated to QA.
pub const POOL: &str = "assistant_qa";
const REPLICA_LEASE_MS: i64 = 12_000;
const EXCHANGE_TTL_MS: i64 = 15 * 60_000;
const MAX_ACTIVE_AGE_MS: i64 = 90_000;
const MAX_ACTIVE: i64 = 16;
const MAX_PER_REGISTRATION: i64 = QA_MAX_RETAINED_TURNS as i64;
const MAX_RETAINED: i64 = 512;

/// Apply after registry and durable Jobs dispatch schemas.
pub const SCHEMA: &str = r#"
DEFINE TABLE IF NOT EXISTS inbox_qa_settings SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS enabled ON inbox_qa_settings TYPE bool;
DEFINE FIELD IF NOT EXISTS revision ON inbox_qa_settings TYPE int;
DEFINE FIELD IF NOT EXISTS generation ON inbox_qa_settings TYPE int;
DEFINE FIELD IF NOT EXISTS epoch ON inbox_qa_settings TYPE string;
DEFINE FIELD IF NOT EXISTS admission ON inbox_qa_settings TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS changed_at ON inbox_qa_settings TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS model_target ON inbox_qa_settings TYPE option<string>;
DEFINE FIELD IF NOT EXISTS generator_preference ON inbox_qa_settings TYPE option<string>;
DEFINE FIELD IF NOT EXISTS decision_preference ON inbox_qa_settings TYPE option<string>;
DEFINE FIELD IF NOT EXISTS history_generation ON inbox_qa_settings TYPE option<int>;
DEFINE FIELD IF NOT EXISTS decision_generation ON inbox_qa_settings TYPE option<int>;
DEFINE TABLE IF NOT EXISTS inbox_qa_replica SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS generation ON inbox_qa_replica TYPE int;
DEFINE FIELD IF NOT EXISTS owner ON inbox_qa_replica TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS lease_until ON inbox_qa_replica TYPE int;
DEFINE FIELD IF NOT EXISTS ready ON inbox_qa_replica TYPE bool DEFAULT false;
DEFINE INDEX IF NOT EXISTS inbox_qa_replica_ready ON inbox_qa_replica FIELDS generation, lease_until;
DEFINE INDEX IF NOT EXISTS inbox_qa_replica_owner ON inbox_qa_replica FIELDS owner;
DEFINE TABLE IF NOT EXISTS inbox_qa_exchange SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS generation ON inbox_qa_exchange TYPE int;
DEFINE FIELD IF NOT EXISTS owner ON inbox_qa_exchange TYPE string;
DEFINE FIELD IF NOT EXISTS workstation ON inbox_qa_exchange TYPE string;
DEFINE FIELD IF NOT EXISTS active ON inbox_qa_exchange TYPE bool;
DEFINE FIELD IF NOT EXISTS accepted_at ON inbox_qa_exchange TYPE int;
DEFINE FIELD IF NOT EXISTS completed_at ON inbox_qa_exchange TYPE int;
DEFINE FIELD IF NOT EXISTS payload ON inbox_qa_exchange TYPE string;
DEFINE INDEX IF NOT EXISTS inbox_qa_owner ON inbox_qa_exchange FIELDS owner, accepted_at;
DEFINE INDEX IF NOT EXISTS inbox_qa_active ON inbox_qa_exchange FIELDS active, generation;
DEFINE INDEX IF NOT EXISTS inbox_qa_workstation ON inbox_qa_exchange FIELDS workstation, active;
DEFINE INDEX IF NOT EXISTS inbox_qa_expiry ON inbox_qa_exchange FIELDS completed_at;
DEFINE TABLE IF NOT EXISTS inbox_qa_latest SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS generation ON inbox_qa_latest TYPE int;
DEFINE FIELD IF NOT EXISTS turn ON inbox_qa_latest TYPE record<inbox_qa_exchange>;
DEFINE FIELD IF NOT EXISTS required_after ON inbox_qa_latest TYPE int;
DEFINE TABLE IF NOT EXISTS inbox_qa_health SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS required_after ON inbox_qa_health TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS sampled_at ON inbox_qa_health TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS publisher ON inbox_qa_health TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS lease_until ON inbox_qa_health TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS payload ON inbox_qa_health TYPE string DEFAULT '';
DEFINE TABLE IF NOT EXISTS inbox_qa_event SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS event_revision ON inbox_qa_event TYPE int;
"#;

/// Trusted registration identity stored only in the protected coordinator.
#[derive(Clone, Deserialize, Serialize)]
pub struct Actor {
    scope: String,
    credential: String,
    epoch: String,
    context: RegisteredContext,
}

impl Actor {
    /// Construct from authenticated boundary metadata.
    #[must_use]
    pub fn new(
        scope: String,
        credential: String,
        epoch: String,
        context: RegisteredContext,
    ) -> Self {
        Self {
            scope,
            credential,
            epoch,
            context,
        }
    }

    fn key(&self) -> Result<String, TxError> {
        let bytes = serde_json::to_vec(&(
            &self.scope,
            &self.credential,
            &self.epoch,
            &self.context.deployment_id,
            &self.context.registration_id,
            self.context.registration_revision,
            self.context.context_revision,
        ))
        .map_err(|_| abort("invalid_scope"))?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    fn workstation(&self) -> String {
        format!(
            "{:x}",
            Sha256::digest(self.context.workstation_id.as_bytes())
        )
    }

    async fn validate_in(&self, tx: &Tx) -> Result<(), TxError> {
        let authority = RegistryAuthority::new(&self.scope, &self.credential, &self.epoch)
            .map_err(|_| abort("forbidden"))?;
        let current = connect_registry::validate_registration_in(
            tx,
            &authority,
            &self.context.registration_id,
            self.context.registration_revision,
        )
        .await?;
        if !self.context.same_request_context(&current) {
            return Err(abort("forbidden"));
        }
        Ok(())
    }
}

/// Maximum age of evidence admitted to an Inbox turn.
pub const MAX_EVIDENCE_AGE_MS: i64 = 5_000;
/// Label for the approved historical side of an observation comparison.
pub const PREVIOUS_OBSERVATION_LABEL: &str = "Previous observation";
/// The cited aggregate for a Workstation Status check report.
pub const STATUS_CHECKS_SOURCE_ID: &str = "workstation_status_checks";

/// Frozen evidence and metadata retained inside the coordinator during execution.
#[derive(Clone, Deserialize, Serialize)]
pub struct FrozenEvidence {
    pub request: QaWorkerRequest,
    pub sources: Vec<Source>,
    pub sampled_at_ms: i64,
}

/// Label and typed source corresponding to one opaque model passage ID.
#[derive(Clone, Deserialize, Serialize)]
pub struct Source {
    pub opaque_id: String,
    pub reference: ChatSourceReference,
    pub label: String,
    pub observed_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<QaEvidenceProvenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measurement: Option<QaMeasurement>,
    /// For summary selections, display the complete observed state from the chosen passage.
    #[serde(default)]
    pub whole_passage: bool,
}

impl Source {
    /// Verify timing without conflating source time with retrieval time.
    #[must_use]
    pub fn observation_is_valid(&self, sampled_at_ms: i64, now_ms: i64) -> bool {
        DateTime::parse_from_rfc3339(&self.observed_at).is_ok_and(|at| {
            let observed_ms = at.timestamp_millis();
            match self.provenance {
                Some(QaEvidenceProvenance::PreviousObservation) => observed_ms < sampled_at_ms,
                Some(QaEvidenceProvenance::Historical {
                    sequence,
                    coverage_start_ms,
                    ..
                }) => sequence > 0 && (coverage_start_ms..=sampled_at_ms).contains(&observed_ms),
                None | Some(QaEvidenceProvenance::Current) => {
                    (0..=MAX_EVIDENCE_AGE_MS).contains(&now_ms.saturating_sub(observed_ms))
                }
                _ => false,
            }
        })
    }

    /// Attach frozen, server-side citation metadata to an exact source span.
    #[must_use]
    pub fn finding(&self, text: String, start: u32, end: u32) -> QaFinding {
        let mut finding = QaFinding::new(
            self.reference.clone(),
            self.label.clone(),
            text,
            start,
            end,
            self.observed_at.clone(),
        );
        finding.measurement = self.measurement.clone();
        finding.provenance = self.provenance;
        finding
    }
}

/// An independently verified observation, retained for a later explicit recheck.
#[derive(Clone, Deserialize, Serialize)]
pub struct Observation {
    pub source: ChatSourceReference,
    pub observed_at: String,
    pub value: ObservationValue,
}

#[derive(Clone, Deserialize, Serialize)]
pub enum ObservationValue {
    State(String),
    Measurement(QaMeasurement),
}

/// Failed and unresolved checks in the authorized Workstation Status sample.
#[derive(Clone, Copy, Deserialize, Serialize)]
pub struct StatusCheckTally {
    pub failed: usize,
    pub unresolved: usize,
}

/// Proposed conversational state. Only verified terminal replies make it reusable.
#[derive(Clone, Default, Deserialize, Serialize)]
pub struct ConversationState {
    pub focus: Option<Focus>,
    pub pending: Option<PendingClarification>,
    pub parser_status: Option<Status>,
    pub guidance: Option<QaGuidance>,
    pub history_requested: bool,
}

impl ConversationState {
    fn private_source_ids(&self) -> impl Iterator<Item = &str> {
        self.focus
            .iter()
            .chain(self.pending.iter().flat_map(|pending| &pending.choices))
            .filter_map(|focus| focus.source.as_ref())
            .filter(|source| private_subject(source))
            .map(|source| source.id.as_str())
    }
}

/// Lease-scoped inputs for response planning. Facts remain in the frozen snapshot.
pub struct PlanContext {
    pub focus: Option<Focus>,
    pub pending: Option<PendingClarification>,
    pub guidance: Option<QaGuidance>,
    pub history_requested: bool,
    pub decision_preference: QaDecisionPreference,
    pub decision_generation: i64,
}

/// The way the generator may respond to the admitted turn.
#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseMode {
    Conversation,
    Current,
    Historical,
    Recheck,
    Clarification,
}

/// Stable task discriminator for the retained response plan.
#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseTask {
    ResponsePlan,
}

/// Selected plan and exactly one instruction bound to the admitted snapshot.
#[derive(Clone, Deserialize, Serialize)]
pub struct ResponsePlan {
    pub task: ResponseTask,
    pub version: u8,
    pub snapshot: String,
    pub decision_generation: i64,
    pub mode: ResponseMode,
    pub focus: Option<Focus>,
    pub clarification: Option<PendingClarification>,
    pub ordered_passages: Vec<String>,
    pub worker: Option<QaWorkerPlan>,
    pub requires_worker_support: bool,
    pub negotiated: bool,
    pub note: String,
}

impl FrozenEvidence {
    /// Bind the plan to the exact serialized passages and citation metadata.
    #[must_use]
    pub fn binding(&self) -> Option<String> {
        Some(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).ok()?)
        ))
    }
}

impl ResponsePlan {
    fn matches(&self, evidence: &FrozenEvidence, conversation: &ConversationState) -> bool {
        if self.version != 1
            || evidence.binding().as_deref() != Some(self.snapshot.as_str())
            || self.focus != conversation.focus
            || self.clarification != conversation.pending
            || self.note.is_empty()
            || self.note.len() > 256
            || self.ordered_passages.len() > 3
            || self.ordered_passages.iter().collect::<BTreeSet<_>>().len()
                != self.ordered_passages.len()
            || self.ordered_passages.iter().any(|id| {
                !evidence.request.passages.iter().any(|p| &p.source_id == id)
                    || !evidence.sources.iter().any(|s| &s.opaque_id == id)
            })
        {
            return false;
        }
        if let Some(worker) = &self.worker {
            let compact = conversation
                .focus
                .as_ref()
                .map_or_else(String::new, Focus::compact);
            if worker.note != self.note
                || worker.focus != compact
                || worker.guidance.is_some() && worker.guidance != conversation.guidance
                || !self.ordered_passages.starts_with(&worker.required_passages)
            {
                return false;
            }
            return self.request(evidence).is_some();
        }
        !self.requires_worker_support && self.ordered_passages.is_empty()
    }

    /// Reconstruct the negotiated request without changing the frozen snapshot.
    #[must_use]
    pub fn request(&self, evidence: &FrozenEvidence) -> Option<QaWorkerRequest> {
        let mut request = evidence.request.clone();
        if self.negotiated {
            if let Some(worker) = &self.worker {
                request.passages = self
                    .ordered_passages
                    .iter()
                    .map(|id| {
                        evidence
                            .request
                            .passages
                            .iter()
                            .find(|p| &p.source_id == id)
                            .cloned()
                    })
                    .collect::<Option<Vec<_>>>()?;
                request.plan = Some(worker.clone());
            }
        }
        request.validate().ok()?;
        Some(request)
    }
}

/// Trusted request metadata that is independent of any model reply.
pub struct AdmissionMetadata {
    pub subject: Option<ChatSourceReference>,
    pub observation: Option<Observation>,
    pub related_activity: Option<QaRelatedActivity>,
    pub status_check_tally: Option<StatusCheckTally>,
    pub used_previous_exchange: bool,
    pub response_started_at_ms: Option<i64>,
    pub conversation: ConversationState,
}

#[derive(Clone, Deserialize, Serialize)]
struct Exchange {
    actor: Actor,
    source: Option<ChatSourceReference>,
    turn: QaTurn,
    #[serde(skip)]
    accepted_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    response_started_at_ms: Option<i64>,
    events: Vec<QaEvent>,
    evidence: Option<FrozenEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resolved_question: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resolved_source: Option<ChatSourceReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    observation: Option<Observation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status_check_tally: Option<StatusCheckTally>,
    /// Older rows used the history generation for both the worker and transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generator_generation: Option<i64>,
    #[serde(default)]
    conversation: ConversationState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plan: Option<ResponsePlan>,
}

impl Exchange {
    fn response_time_ms(&self, completed_at: i64) -> Option<u64> {
        let started_at = self.response_started_at_ms.unwrap_or(self.accepted_at_ms);
        u64::try_from(completed_at.checked_sub(started_at)?).ok()
    }

    fn subject(&self) -> Option<&ChatSourceReference> {
        self.resolved_source.as_ref().or(self.source.as_ref())
    }

    fn private_citation_ids(&self) -> impl Iterator<Item = &str> {
        let frozen = self
            .evidence
            .iter()
            .flat_map(|evidence| &evidence.sources)
            .map(|source| &source.reference);
        self.subject()
            .into_iter()
            .chain(frozen)
            .chain(self.turn.findings.iter().map(|finding| &finding.source))
            .filter(|source| private_subject(source))
            .map(|source| source.id.as_str())
    }

    fn private_subject_ids(&self) -> impl Iterator<Item = &str> {
        let related = match &self.turn.related_activity {
            Some(QaRelatedActivity::Found { target, .. }) => Some(target),
            _ => None,
        };
        self.private_citation_ids().chain(
            related
                .filter(|source| private_subject(source))
                .map(|source| source.id.as_str()),
        )
    }
}

fn private_subject(source: &ChatSourceReference) -> bool {
    matches!(source.kind, ChatSourceKind::Job | ChatSourceKind::Cycle)
}

async fn private_subjects_visible_in<'a>(
    tx: &Tx,
    actor: &Actor,
    ids: impl Iterator<Item = &'a str>,
) -> Result<bool, TxError> {
    let ids: BTreeSet<_> = ids.collect();
    let jobs = ids.iter().map(|id| jobs::rid("job", id)).collect();
    Ok(visible_private_subjects_in(tx, actor, jobs).await?.len() == ids.len())
}

async fn visible_private_subjects_in(
    tx: &Tx,
    actor: &Actor,
    subjects: Vec<RecordId>,
) -> Result<Vec<String>, TxError> {
    if subjects.is_empty() {
        return Ok(Vec::new());
    }
    let query = format!(
        "SELECT VALUE record::id(id) FROM $subjects WHERE kind != 'service' AND {} AND ({}) AND ({})",
        jobs_view::ELIGIBLE,
        jobs_view::activity_membership(),
        jobs_view::activity_private()
    );
    let mut result = jobs::run(
        tx,
        tx.query(query)
            .bind(("subjects", subjects))
            .bind(("activity_scope", actor.scope.clone()))
            .bind(("activity_deployment", actor.context.deployment_id.clone()))
            .bind(("activity_node", actor.context.workstation_id.clone()))
            .bind((
                "cutoff",
                jobs::ts(Utc::now() - chrono::Duration::hours(jobs::EXECUTION_WINDOW_HOURS)),
            )),
    )
    .await?;
    jobs::take(&mut result, 0)
}

#[derive(Clone, Debug, Default, SurrealValue)]
struct SettingsRow {
    enabled: bool,
    revision: i64,
    generation: i64,
    epoch: String,
    changed_at: i64,
    generator_preference: Option<String>,
    decision_preference: Option<String>,
    history_generation: Option<i64>,
    decision_generation: Option<i64>,
}

#[derive(SurrealValue)]
struct ExchangeRow {
    generation: i64,
    owner: String,
    active: bool,
    accepted_at: i64,
    completed_at: i64,
    payload: String,
}

/// Current setting independent of any particular worker connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Setting {
    pub enabled: bool,
    /// Enablement and generator worker generation.
    pub generation: i64,
    /// Enablement and decision worker generation.
    pub decision_generation: i64,
    /// Enablement generation used to retain transcripts across model changes.
    pub history_generation: i64,
    pub revision: i64,
    pub changed_at_ms: i64,
    pub generator_preference: QaGeneratorPreference,
    pub decision_preference: QaDecisionPreference,
}

impl SettingsRow {
    fn state(&self, epoch: &str) -> Result<Setting, TxError> {
        let generator_preference = match self.generator_preference.as_deref() {
            None | Some("automatic") => QaGeneratorPreference::Automatic,
            Some("flan_t5_small") => QaGeneratorPreference::FlanT5Small,
            Some("flan_t5_base") => QaGeneratorPreference::FlanT5Base,
            Some("gpt2_small") => QaGeneratorPreference::Gpt2Small,
            Some("gpt2_medium") => QaGeneratorPreference::Gpt2Medium,
            Some(_) => return Err(abort("invalid_store")),
        };
        let decision_preference = match self.decision_preference.as_deref() {
            None | Some("rules_only") => QaDecisionPreference::RulesOnly,
            Some("julia1_q8") => QaDecisionPreference::Julia1Q8,
            Some("vllm_d3_edge_q8") => QaDecisionPreference::VllmD3EdgeQ8,
            Some(_) => return Err(abort("invalid_store")),
        };
        Ok(Setting {
            enabled: self.enabled && self.epoch == epoch,
            generation: self.generation,
            decision_generation: self.decision_generation.unwrap_or(self.generation),
            history_generation: self.history_generation.unwrap_or(self.generation),
            revision: self.revision,
            changed_at_ms: self.changed_at,
            generator_preference,
            decision_preference,
        })
    }
}

fn abort(reason: &str) -> TxError {
    TxError::Abort(format!("inbox_qa:{reason}"))
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

impl ExchangeRow {
    fn decode(&self) -> Result<Exchange, TxError> {
        let mut exchange: Exchange =
            serde_json::from_str(&self.payload).map_err(|_| abort("invalid_store"))?;
        // Use row metadata so transcripts retained from earlier versions have the same times.
        exchange.accepted_at_ms = self.accepted_at;
        exchange.turn.accepted_at = Some(self.accepted_at.div_euclid(1_000));
        if exchange.turn.state.is_terminal() && self.completed_at > 0 {
            exchange.turn.completed_at = Some(self.completed_at.div_euclid(1_000));
            exchange.turn.response_time_ms = exchange.response_time_ms(self.completed_at);
        } else {
            exchange.turn.completed_at = None;
            exchange.turn.response_time_ms = None;
        }
        Ok(exchange)
    }
}

async fn settings_row(tx: &Tx) -> Result<SettingsRow, TxError> {
    let mut result = jobs::run(
        tx,
        tx.query(
            "SELECT enabled,revision,generation,epoch,changed_at,
                    (generator_preference ?? 'automatic') AS generator_preference,
                    (decision_preference ?? 'rules_only') AS decision_preference,
                    (history_generation ?? generation) AS history_generation,
                    (decision_generation ?? generation) AS decision_generation
             FROM ONLY inbox_qa_settings:current",
        ),
    )
    .await?;
    Ok(jobs::take::<Option<SettingsRow>>(&mut result, 0)?.unwrap_or_default())
}

async fn settings_in(tx: &Tx) -> Result<Setting, TxError> {
    let row = settings_row(tx).await?;
    let deployment = connect_registry::deployment_state_in(tx).await?;
    let mut observed = row.state(&deployment.configuration_epoch)?;
    observed.enabled &= deployment.admission_open;
    Ok(observed)
}

/// Classify a database failure without exposing its data or diagnostic text.
pub fn storage_error_kind(error: &TxError) -> &'static str {
    use surrealdb::types::QueryError;

    let TxError::Db(error) = error else {
        return "storage";
    };
    match error.query_details() {
        Some(QueryError::TransactionConflict) => "transaction_conflict",
        Some(QueryError::TimedOut { .. }) => "query_timeout",
        Some(QueryError::Cancelled) => "query_cancelled",
        Some(QueryError::NotExecuted) => "query_not_executed",
        _ => error.kind_str(),
    }
}

/// Read the authoritative generation. An absent or reset setting is Off.
pub async fn setting(repo: &Repository) -> Result<Setting, TxError> {
    Ok(run_tx(repo.db(), RetryPolicy::default(), |tx| async move {
        settings_in(&tx).await
    })
    .await?
    .value)
}

// The companion shares its repository between IPC, the executor and the
// health sampler. Queue their writes before opening a database snapshot so
// background work cannot repeatedly invalidate a settings transaction.
// Transcript reads with registration validation also write the registry guard.
// The database still fences independently connected writers.
async fn write<T, F, Fut>(repo: &Repository, body: F) -> Result<Committed<T>, TxError>
where
    F: FnMut(Tx) -> Fut,
    Fut: Future<Output = Result<T, TxError>>,
{
    let _writer = repo.lock_inbox_qa_writer().await;
    run_tx(repo.db(), RetryPolicy::default(), body).await
}

#[derive(SurrealValue)]
struct EventMarker {
    event_revision: i64,
}

async fn bump_event_revision(tx: &Tx) -> Result<(), TxError> {
    jobs::run(
        tx,
        tx.query(
            "UPSERT inbox_qa_event:current SET event_revision=(event_revision ?? 0)+1 RETURN NONE",
        ),
    )
    .await?;
    Ok(())
}

/// A lightweight target-wide wake hint. It contains no authority or exchange data.
pub async fn event_revision(repo: &Repository) -> Result<i64, TxError> {
    let mut reply = repo
        .db()
        .query("SELECT VALUE event_revision FROM ONLY inbox_qa_event:current")
        .await?
        .check()?;
    Ok(reply.take::<Option<i64>>(0)?.unwrap_or_default())
}

/// Observe committed revisions over the pinned SDK's record subscription.
pub async fn event_changes(
    repo: &Repository,
) -> Result<impl Stream<Item = Result<i64, TxError>> + Send + Unpin, TxError> {
    let changes = repo
        .db()
        .select::<Option<EventMarker>>(("inbox_qa_event", "current"))
        .live()
        .await?;
    Ok(changes.map(|result| {
        result
            .map(|notice| notice.data.event_revision)
            .map_err(TxError::from)
    }))
}

/// Toggle backend inference while preserving both saved preferences.
pub async fn set(
    repo: &Repository,
    actor: &Actor,
    expected_revision: i64,
    enabled: bool,
) -> Result<Setting, TxError> {
    update(
        repo,
        actor,
        expected_revision,
        SettingUpdate {
            enabled: Some(enabled),
            ..SettingUpdate::default()
        },
    )
    .await
}

/// Fields provided by an authorized, revisioned settings update.
#[derive(Clone, Copy, Debug, Default)]
pub struct SettingUpdate {
    pub enabled: Option<bool>,
    pub generator_preference: Option<QaGeneratorPreference>,
    pub decision_preference: Option<QaDecisionPreference>,
}

/// Serialize settings updates with native registration rebind and reset.
pub async fn update(
    repo: &Repository,
    actor: &Actor,
    expected_revision: i64,
    update: SettingUpdate,
) -> Result<Setting, TxError> {
    Ok(write(repo, |tx| async move {
        actor.validate_in(&tx).await?;
        let row = settings_row(&tx).await?;
        if row.revision != expected_revision {
            return Err(abort("revision"));
        }
        let deployment = connect_registry::deployment_state_in(&tx).await?;
        let current = row.state(&deployment.configuration_epoch)?;
        let enabled = update.enabled.unwrap_or(current.enabled);
        let generator_preference = update.generator_preference.unwrap_or(current.generator_preference);
        let decision_preference = update.decision_preference.unwrap_or(current.decision_preference);
        let enablement_changed = enabled != current.enabled;
        let generator_changed = generator_preference != current.generator_preference;
        let decision_changed = decision_preference != current.decision_preference;
        if !enablement_changed && !generator_changed && !decision_changed {
            return Ok(current);
        }
        let advance = |value: i64, changed| {
            if changed {
                value.checked_add(1).ok_or_else(|| abort("revision"))
            } else {
                Ok(value)
            }
        };
        let generator_replaced = enablement_changed || generator_changed;
        let next = Setting {
            enabled,
            generator_preference,
            decision_preference,
            revision: advance(current.revision, true)?,
            generation: advance(current.generation, generator_replaced)?,
            decision_generation: advance(current.decision_generation, enablement_changed || decision_changed)?,
            history_generation: advance(current.history_generation, enablement_changed)?,
            changed_at_ms: if generator_replaced { now_ms() } else { current.changed_at_ms },
        };
        let generator_preference = match next.generator_preference {
            QaGeneratorPreference::Automatic => "automatic",
            QaGeneratorPreference::FlanT5Small => "flan_t5_small",
            QaGeneratorPreference::FlanT5Base => "flan_t5_base",
            QaGeneratorPreference::Gpt2Small => "gpt2_small",
            QaGeneratorPreference::Gpt2Medium => "gpt2_medium",
            _ => return Err(abort("invalid_store")),
        };
        let decision_preference = match next.decision_preference {
            QaDecisionPreference::RulesOnly => "rules_only",
            QaDecisionPreference::Julia1Q8 => "julia1_q8",
            QaDecisionPreference::VllmD3EdgeQ8 => "vllm_d3_edge_q8",
            _ => return Err(abort("invalid_store")),
        };
        jobs::run(&tx, tx.query(
            "UPSERT inbox_qa_settings:current SET enabled=$enabled, revision=$revision, generation=$generation,
                    decision_generation=$decision_generation, history_generation=$history_generation,
                    generator_preference=$generator_preference, decision_preference=$decision_preference,
                    epoch=$epoch, changed_at=$at RETURN NONE;",
        ).bind(("enabled", next.enabled)).bind(("revision", next.revision)).bind(("generation", next.generation))
          .bind(("decision_generation", next.decision_generation)).bind(("history_generation", next.history_generation))
          .bind(("generator_preference", generator_preference)).bind(("decision_preference", decision_preference))
          .bind(("epoch", deployment.configuration_epoch)).bind(("at", next.changed_at_ms))).await?;
        if enablement_changed {
            // Master Off retires private transcripts and open Jobs leases.
            let mut reply = jobs::run(&tx, tx.query(
                "SELECT VALUE record::id(id) FROM inbox_qa_exchange LIMIT $limit;",
            ).bind(("limit", MAX_RETAINED))).await?;
            retire_in(&tx, jobs::take::<Vec<String>>(&mut reply, 0)?).await?;
            jobs::run(&tx, tx.query("DELETE inbox_qa_latest RETURN NONE; DELETE inbox_qa_exchange RETURN NONE; DELETE inbox_qa_health:current RETURN NONE;")).await?;
        } else if generator_changed {
            seal_replaced_generator_in(&tx).await?;
        }
        bump_event_revision(&tx).await?;
        Ok(next)
    }).await?.value)
}

async fn seal_replaced_generator_in(tx: &Tx) -> Result<(), TxError> {
    let mut response = jobs::run(
        tx,
        tx.query(
            "SELECT VALUE record::id(id) FROM inbox_qa_exchange WHERE active=true LIMIT $limit",
        )
        .bind(("limit", MAX_ACTIVE)),
    )
    .await?;
    let active = jobs::take::<Vec<String>>(&mut response, 0)?;
    for id in &active {
        if let Some(row) = exchange_in(tx, id).await? {
            let mut exchange = row.decode()?;
            exchange.turn.findings.clear();
            exchange.turn.related_activity = None;
            exchange.events.clear();
            append_in(
                tx,
                &row.owner,
                row.generation,
                &mut exchange,
                QaTurnState::Canceled,
                None,
            )
            .await?;
        }
    }
    retire_in(tx, active).await
}

#[derive(SurrealValue)]
struct ReplicaRow {
    owner: String,
    generation: i64,
    lease_until: i64,
}

/// Permission to open a model session, or to confirm an earlier session was reaped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplicaClaim {
    Wait,
    ConfirmReap,
    Warm,
}

async fn replica_in(tx: &Tx, placement: &str) -> Result<Option<ReplicaRow>, TxError> {
    let mut reply = jobs::run(
        tx,
        tx.query("SELECT owner,generation,lease_until FROM ONLY $worker")
            .bind(("worker", jobs::rid("inbox_qa_replica", placement))),
    )
    .await?;
    jobs::take(&mut reply, 0)
}

/// Reserve this placement. A previous session must be positively reaped before reusing it.
pub async fn claim_replica(
    repo: &Repository,
    placement: &str,
    owner: &str,
    generation: i64,
) -> Result<ReplicaClaim, TxError> {
    Ok(write(repo, |tx| async move {
        let state = settings_in(&tx).await?;
        if state.generation != generation {
            return Ok(ReplicaClaim::Wait);
        }
        let now = now_ms();
        let current = replica_in(&tx, placement).await?;
        let claim = match current {
            Some(row) if row.lease_until > now && row.owner != owner => return Ok(ReplicaClaim::Wait),
            Some(_) => ReplicaClaim::ConfirmReap,
            None if state.enabled => ReplicaClaim::Warm,
            None => return Ok(ReplicaClaim::Wait),
        };
        // This row shares the same write fence as a concurrent enablement transition.
        jobs::run(&tx, tx.query(
            "UPDATE inbox_qa_settings:current SET admission += 1 RETURN NONE;
             UPSERT $worker SET generation=$generation, owner=$owner, ready=false, lease_until=$until RETURN NONE;",
        ).bind(("worker", jobs::rid("inbox_qa_replica", placement))).bind(("owner", owner.to_owned()))
          .bind(("generation", generation)).bind(("until", now + REPLICA_LEASE_MS))).await?;
        Ok(claim)
    }).await?.value)
}

/// Renew the exact open permit. An expired holder must confirm reap before warming again.
pub async fn beat(
    repo: &Repository,
    placement: &str,
    owner: &str,
    generation: i64,
    ready: bool,
) -> Result<bool, TxError> {
    Ok(write(repo, |tx| async move {
        let state = settings_in(&tx).await?;
        if !state.enabled || state.generation != generation {
            return Ok(false);
        }
        let now = now_ms();
        if !replica_in(&tx, placement).await?.is_some_and(|row| {
            row.owner == owner && row.generation == generation && row.lease_until > now
        }) {
            return Ok(false);
        }
        jobs::run(
            &tx,
            tx.query(
                "UPDATE inbox_qa_settings:current SET admission += 1 RETURN NONE;
             UPDATE $worker SET ready=$ready, lease_until=$until RETURN NONE;",
            )
            .bind(("worker", jobs::rid("inbox_qa_replica", placement)))
            .bind(("ready", ready))
            .bind(("until", now + REPLICA_LEASE_MS)),
        )
        .await?;
        Ok(true)
    })
    .await?
    .value)
}

/// Drop this holder's record only after the corresponding model is confirmed reaped.
pub async fn forget_replica(
    repo: &Repository,
    placement: &str,
    owner: &str,
) -> Result<(), TxError> {
    let _writer = repo.lock_inbox_qa_writer().await;
    repo.db()
        .query("DELETE $worker WHERE owner=$owner RETURN NONE")
        .bind(("worker", jobs::rid("inbox_qa_replica", placement)))
        .bind(("owner", owner.to_owned()))
        .await?
        .check()?;
    Ok(())
}

/// Whether any replica has a loaded model and currently holds an enablement lease.
pub async fn replica_present(repo: &Repository, generation: i64) -> Result<bool, TxError> {
    let mut reply = repo.db().query(
        "SELECT VALUE true FROM inbox_qa_replica WHERE generation=$gen AND lease_until>$now AND ready=true LIMIT 1",
    ).bind(("gen", generation)).bind(("now", now_ms())).await?.check()?;
    Ok(!reply.take::<Vec<bool>>(0)?.is_empty())
}

/// Report Stopping until the model supervisor confirms reap, including during a partition.
pub async fn replica_stopping(repo: &Repository) -> Result<bool, TxError> {
    let mut reply = repo
        .db()
        .query("SELECT VALUE true FROM inbox_qa_replica LIMIT 1")
        .await?
        .check()?;
    Ok(!reply.take::<Vec<bool>>(0)?.is_empty())
}

fn turn_id(key: &str, request: &str, generation: i64) -> String {
    format!(
        "qa-{:x}",
        Sha256::digest(format!("{key}:{generation}:{request}"))
    )
}

/// Deterministic, authority-bound identifier for the pending request's frozen snapshot.
pub fn accepted_turn_id(
    actor: &Actor,
    request: &QaTurnRequest,
    history_generation: i64,
) -> Result<String, TxError> {
    Ok(turn_id(
        &actor.key()?,
        &request.request_id,
        history_generation,
    ))
}

async fn exchange_in(tx: &Tx, turn_id: &str) -> Result<Option<ExchangeRow>, TxError> {
    let mut reply = jobs::run(
        tx,
        tx.query("SELECT generation,owner,active,accepted_at,completed_at,payload FROM ONLY $turn")
            .bind(("turn", jobs::rid("inbox_qa_exchange", turn_id))),
    )
    .await?;
    jobs::take(&mut reply, 0)
}

fn matching_request(existing: &Exchange, request: &QaTurnRequest) -> bool {
    existing.turn.question == request.question && existing.source == request.source
}

/// Resolve an accepted request before fetching evidence, including during a temporary outage.
pub async fn replay(
    repo: &Repository,
    actor: &Actor,
    request: &QaTurnRequest,
) -> Result<Option<QaTurn>, TxError> {
    let key = actor.key()?;
    Ok(write(repo, |tx| {
        let key = &key;
        async move {
            actor.validate_in(&tx).await?;
            let state = settings_in(&tx).await?;
            if !state.enabled {
                return Err(abort("off"));
            }
            let id = turn_id(key, &request.request_id, state.history_generation);
            let Some(row) = exchange_in(&tx, &id).await? else {
                return Ok(None);
            };
            if row.owner != *key
                || row.generation != state.history_generation
                || (row.completed_at > 0 && row.completed_at + EXCHANGE_TTL_MS <= now_ms())
            {
                return Ok(None);
            }
            let exchange = row.decode()?;
            if !matching_request(&exchange, request) {
                return Err(abort("id_conflict"));
            }
            if !private_subjects_visible_in(&tx, actor, exchange.private_citation_ids()).await? {
                return Err(abort("missing"));
            }
            Ok(Some(exchange.turn))
        }
    })
    .await?
    .value)
}

/// Timestamp of the last terminal turn in the current generation.
pub async fn freshness_boundary(repo: &Repository, actor: &Actor) -> Result<i64, TxError> {
    let key = actor.key()?;
    Ok(run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let key = &key;
        async move {
            let state = settings_in(&tx).await?;
            boundary_in(&tx, key, state.history_generation).await
        }
    })
    .await?
    .value)
}

async fn boundary_in(tx: &Tx, key: &str, generation: i64) -> Result<i64, TxError> {
    let mut reply = jobs::run(
        tx,
        tx.query("SELECT VALUE required_after FROM ONLY $latest WHERE generation=$gen")
            .bind(("latest", jobs::rid("inbox_qa_latest", key)))
            .bind(("gen", generation)),
    )
    .await?;
    let own = jobs::take::<Option<i64>>(&mut reply, 0)?.unwrap_or_default();
    let mut global = jobs::run(
        tx,
        tx.query("SELECT VALUE required_after FROM ONLY inbox_qa_health:current"),
    )
    .await?;
    Ok(own.max(jobs::take::<Option<i64>>(&mut global, 0)?.unwrap_or_default()))
}

fn dialogue_line(text: &str) -> &str {
    text[..text.floor_char_boundary(QA_MAX_DIALOGUE_MESSAGE_BYTES.min(text.len()))].trim()
}

async fn recent_completed_in(
    tx: &Tx,
    owner: &str,
    generation: i64,
    now: i64,
    limit: i64,
) -> Result<Vec<ExchangeRow>, TxError> {
    let mut reply = jobs::run(
        tx,
        tx.query(
            "SELECT generation,owner,active,accepted_at,completed_at,payload FROM inbox_qa_exchange
         WHERE owner=$owner AND generation=$generation AND active=false AND completed_at>$cutoff
         ORDER BY completed_at DESC, accepted_at DESC, id DESC LIMIT $limit;",
        )
        .bind(("owner", owner.to_owned()))
        .bind(("generation", generation))
        .bind(("cutoff", now - EXCHANGE_TTL_MS))
        .bind(("limit", limit)),
    )
    .await?;
    jobs::take(&mut reply, 0)
}

fn retained_generated_reply(turn: &QaTurn) -> Option<&str> {
    // Earlier verified workers recorded usage before model metadata was added.
    let model_used = turn.context.is_some_and(|usage| {
        usage.model_windows > 0
            && (1..=usage.window_tokens).contains(&usage.peak_input_tokens)
            && QaModelTarget::ALL
                .iter()
                .any(|target| usage.window_tokens == target.input_tokens())
    });
    if turn.state != QaTurnState::Answered || !model_used {
        return None;
    }
    turn.reply
        .as_deref()
        .filter(|reply| valid_dialogue_reply(reply))
}

async fn recent_dialogue_in(
    tx: &Tx,
    owner: &str,
    generation: i64,
    now: i64,
) -> Result<Vec<QaDialogueExchange>, TxError> {
    let retained = recent_completed_in(tx, owner, generation, now, MAX_PER_REGISTRATION).await?;
    let mut history = Vec::with_capacity(QA_MAX_DIALOGUE_HISTORY);
    for row in retained {
        let exchange = row.decode()?;
        // Keep private job and cycle observations outside the model history.
        if exchange.subject().is_some_and(private_subject)
            || exchange.turn.findings.iter().any(|finding| {
                private_subject(&finding.source) || finding.source.kind == ChatSourceKind::JobCount
            })
        {
            continue;
        }
        let Some(reply) = retained_generated_reply(&exchange.turn) else {
            continue;
        };
        history.push(QaDialogueExchange {
            question: dialogue_line(&exchange.turn.question).to_owned(),
            reply: reply.to_owned(),
        });
        if history.len() == QA_MAX_DIALOGUE_HISTORY {
            break;
        }
    }
    history.reverse();
    Ok(history)
}

/// Atomically accept evidence, latest exchange and the work needed to answer it.
pub async fn admit(
    repo: &Repository,
    actor: &Actor,
    request: &QaTurnRequest,
    evidence: FrozenEvidence,
    metadata: AdmissionMetadata,
    expected_generation: i64,
) -> Result<QaTurn, TxError> {
    let key = actor.key()?;
    Ok(write(repo, |tx| {
        let key = &key;
        let evidence = &evidence;
        let metadata = &metadata;
        async move {
            actor.validate_in(&tx).await?;
            let state = settings_in(&tx).await?;
            if !state.enabled || state.generation != expected_generation {
                return Err(abort("off"));
            }
            let generation = state.history_generation;
            let id = turn_id(key, &request.request_id, generation);
            let id = id.as_str();
            if let Some(row) = exchange_in(&tx, id).await? {
                if row.owner != *key || row.generation != generation {
                    return Err(abort("id_conflict"));
                }
                let exchange = row.decode()?;
                if !matching_request(&exchange, request) {
                    return Err(abort("id_conflict"));
                }
                if !private_subjects_visible_in(&tx, actor, exchange.private_citation_ids()).await? {
                    return Err(abort("missing"));
                }
                return Ok(exchange.turn);
            }
            let private_sources = metadata
                .subject
                .iter()
                .chain(evidence.sources.iter().map(|source| &source.reference))
                .filter(|source| private_subject(source))
                .map(|source| source.id.as_str())
                .chain(metadata.conversation.private_source_ids());
            if !private_subjects_visible_in(&tx, actor, private_sources).await? {
                return Err(abort("missing"));
            }
            let boundary = boundary_in(&tx, key, generation).await?;
            let now = Utc::now();
            if evidence.sampled_at_ms <= boundary
                || evidence.sampled_at_ms < now.timestamp_millis() - MAX_EVIDENCE_AGE_MS
                || evidence.sampled_at_ms > now.timestamp_millis()
                || evidence.sources.iter().any(|source| {
                    !source.observation_is_valid(evidence.sampled_at_ms, now.timestamp_millis())
                })
            {
                return Err(abort("freshness"));
            }
            let timestamp = now.timestamp_millis();
            let mut reply = jobs::run(&tx, tx.query(
                "UPDATE inbox_qa_settings:current SET admission += 1 RETURN NONE;
                 SELECT VALUE true FROM inbox_qa_replica WHERE generation=$gen AND lease_until > $now AND ready=true LIMIT $active_cap;
                 SELECT VALUE true FROM inbox_qa_exchange WHERE workstation=$workstation AND active=true LIMIT 1;
                 RETURN count(SELECT VALUE id FROM inbox_qa_exchange WHERE active=true LIMIT $active_cap);
                 RETURN count(SELECT VALUE id FROM inbox_qa_exchange WHERE owner=$owner LIMIT $owner_cap);
                 RETURN count(SELECT VALUE id FROM inbox_qa_exchange LIMIT $retained_cap);",
            ).bind(("gen", state.generation)).bind(("now", timestamp)).bind(("owner", key.clone()))
              .bind(("workstation", actor.workstation())).bind(("active_cap", MAX_ACTIVE)).bind(("owner_cap", MAX_PER_REGISTRATION))
              .bind(("retained_cap", MAX_RETAINED))).await?;
            let ready = jobs::take::<Vec<bool>>(&mut reply, 1)?.len() as i64;
            if ready == 0 {
                return Err(abort("generator_unavailable"));
            }
            if !jobs::take::<Vec<bool>>(&mut reply, 2)?.is_empty() {
                return Err(abort("active"));
            }
            let active = jobs::take::<Option<i64>>(&mut reply, 3)?.ok_or_else(|| abort("invalid_store"))?;
            let owner_retained = jobs::take::<Option<i64>>(&mut reply, 4)?.ok_or_else(|| abort("invalid_store"))?;
            let total_retained = jobs::take::<Option<i64>>(&mut reply, 5)?.ok_or_else(|| abort("invalid_store"))?;
            if active >= MAX_ACTIVE.min(ready * 2)
                || owner_retained >= MAX_PER_REGISTRATION
                || total_retained >= MAX_RETAINED
            {
                return Err(abort("busy"));
            }
            let mut frozen = evidence.clone();
            let dialogue = frozen.request.dialogue.as_mut().ok_or_else(|| abort("invalid_part"))?;
            if dialogue.kind == QaDialogueKind::Operational {
                // The snapshot already resolves the approved subject and comparison.
                frozen.request.question = request.question.clone();
            } else if dialogue.kind == QaDialogueKind::Recall {
                dialogue.history = recent_dialogue_in(&tx, key, generation, timestamp).await?;
                if dialogue.history.is_empty() {
                    dialogue.kind = QaDialogueKind::Clarification;
                }
            }
            frozen.request.validate().map_err(|_| abort("invalid_part"))?;
            let mut exchange = Exchange {
                actor: actor.clone(),
                source: request.source.clone(),
                turn: QaTurn::queued(id.to_owned(), request.request_id.clone(), request.question.clone()),
                accepted_at_ms: timestamp,
                response_started_at_ms: metadata.response_started_at_ms.filter(|started_at| *started_at <= timestamp),
                events: Vec::new(),
                evidence: Some(frozen),
                resolved_question: (request.question != evidence.request.question)
                    .then(|| evidence.request.question.clone()),
                resolved_source: metadata.subject.as_ref().filter(|source| Some(*source) != request.source.as_ref()).cloned(),
                observation: metadata.observation.clone(),
                status_check_tally: metadata.status_check_tally,
                generator_generation: Some(state.generation),
                conversation: metadata.conversation.clone(),
                plan: None,
            };
            exchange.turn.accepted_at = Some(timestamp.div_euclid(1_000));
            exchange.turn.used_previous_exchange = metadata.used_previous_exchange;
            exchange.turn.related_activity = metadata.related_activity.clone();
            let payload = serde_json::to_string(&exchange).map_err(|_| abort("invalid_store"))?;
            jobs::run(&tx, tx.query(
                "CREATE $turn SET generation=$gen, owner=$owner, workstation=$workstation, active=true, accepted_at=$at, completed_at=0, payload=$payload RETURN NONE;
                 UPSERT $latest SET generation=$gen, turn=$turn, required_after=$boundary RETURN NONE;",
            ).bind(("turn", jobs::rid("inbox_qa_exchange", id))).bind(("gen", generation))
              .bind(("owner", key.clone())).bind(("workstation", actor.workstation())).bind(("at", timestamp)).bind(("payload", payload))
              .bind(("latest", jobs::rid("inbox_qa_latest", key))).bind(("boundary", boundary))).await?;
            job_dispatch::submit_in(&tx, &jobs::NewJob {
                id, kind: POOL, operation: POOL, parent: None, created_at: now,
            }, &jobs::NewTask {
                id, job: id, pool: POOL, operation: POOL, priority: 0,
                deps: Vec::new(), input_hash: id, created_at: now,
            }).await?;
            bump_event_revision(&tx).await?;
            Ok(exchange.turn)
        }
    }).await?.value)
}

async fn current_turn_in(tx: &Tx, key: &str, generation: i64) -> Result<Option<String>, TxError> {
    let mut reply = jobs::run(tx, tx.query(
        "SELECT VALUE record::id(turn) FROM ONLY $latest WHERE generation=$gen AND turn.generation=$gen AND turn.owner=$owner",
    ).bind(("latest", jobs::rid("inbox_qa_latest", key)))
      .bind(("gen", generation)).bind(("owner", key.to_owned()))).await?;
    jobs::take(&mut reply, 0)
}

/// Only the current exchange for this authority is available to Inbox event reads.
pub async fn latest(
    repo: &Repository,
    actor: &Actor,
) -> Result<Option<(QaTurn, Vec<QaEvent>)>, TxError> {
    Ok(latest_exchange(repo, actor)
        .await?
        .map(|exchange| (exchange.turn, exchange.events)))
}

/// Read this registration's retained terminal exchanges, newest first.
/// Private subjects and citations must still be visible to the caller.
pub async fn history(repo: &Repository, actor: &Actor) -> Result<Vec<QaTurn>, TxError> {
    let key = actor.key()?;
    Ok(write(repo, |tx| {
        let key = &key;
        async move {
            actor.validate_in(&tx).await?;
            let setting = settings_in(&tx).await?;
            if !setting.enabled {
                return Ok(Vec::new());
            }
            let rows = recent_completed_in(
                &tx,
                key,
                setting.history_generation,
                now_ms(),
                MAX_PER_REGISTRATION,
            )
            .await?;
            let exchanges: Vec<Exchange> = rows
                .into_iter()
                .map(|row| row.decode())
                .collect::<Result<_, _>>()?;
            let subjects: BTreeSet<&str> = exchanges
                .iter()
                .flat_map(Exchange::private_subject_ids)
                .collect();
            let jobs = subjects
                .into_iter()
                .map(|id| jobs::rid("job", id))
                .collect();
            let visible: BTreeSet<String> = visible_private_subjects_in(&tx, actor, jobs)
                .await?
                .into_iter()
                .collect();
            Ok(exchanges
                .into_iter()
                .filter(|exchange| {
                    exchange.turn.state.is_terminal()
                        && exchange
                            .private_subject_ids()
                            .all(|id| visible.contains(id))
                })
                .map(|mut exchange| {
                    exchange.turn.provisional_reply = None;
                    exchange.turn
                })
                .collect())
        }
    })
    .await?
    .value)
}

/// The latest answered intent and its typed subject after registration validation.
pub struct AnsweredPrompt {
    pub question: String,
    pub subject: Option<ChatSourceReference>,
    pub observation: Option<Observation>,
    /// True when an intervening completed turn did not have an answer with citations.
    pub intervening_turn: bool,
    /// The cited aggregate for an answered status report, when present.
    pub status_check_summary: Option<String>,
    pub status_check_tally: Option<StatusCheckTally>,
    pub focus: Option<Focus>,
}

/// Use the most recent cited topic within the short conversation window.
pub async fn last_answered_prompt(
    repo: &Repository,
    actor: &Actor,
) -> Result<Option<AnsweredPrompt>, TxError> {
    let key = actor.key()?;
    Ok(write(repo, |tx| {
        let key = &key;
        async move {
            actor.validate_in(&tx).await?;
            let setting = settings_in(&tx).await?;
            if !setting.enabled {
                return Ok(None);
            }
            for (index, row) in recent_completed_in(
                &tx,
                key,
                setting.history_generation,
                now_ms(),
                QA_MAX_DIALOGUE_HISTORY as i64,
            )
            .await?
            .into_iter()
            .enumerate()
            {
                let exchange = row.decode()?;
                if exchange.turn.state == QaTurnState::Answered
                    && !exchange.turn.findings.is_empty()
                {
                    if !private_subjects_visible_in(&tx, actor, exchange.private_subject_ids())
                        .await?
                    {
                        return Ok(None);
                    }
                    let subject = exchange.subject().cloned();
                    let observation = exchange.observation.filter(|observation| {
                        exchange.turn.findings.iter().any(|finding| {
                            finding.source == observation.source
                                && finding.observed_at == observation.observed_at
                        })
                    });
                    let status_check_summary = exchange.turn.findings.iter().find_map(|finding| {
                        (finding.source.kind == ChatSourceKind::Readiness
                            && finding.source.id == STATUS_CHECKS_SOURCE_ID)
                            .then(|| finding.text.clone())
                    });
                    return Ok(Some(AnsweredPrompt {
                        subject,
                        intervening_turn: index > 0,
                        status_check_summary,
                        status_check_tally: exchange.status_check_tally,
                        focus: exchange.conversation.focus,
                        observation,
                        question: exchange.resolved_question.unwrap_or(exchange.turn.question),
                    }));
                }
            }
            Ok(None)
        }
    })
    .await?
    .value)
}

/// Recover an open clarification from the latest verified reply in this
/// registration and conversation generation.
pub async fn pending_clarification(
    repo: &Repository,
    actor: &Actor,
) -> Result<Option<PendingClarification>, TxError> {
    let key = actor.key()?;
    Ok(write(repo, |tx| {
        let key = &key;
        async move {
            actor.validate_in(&tx).await?;
            let setting = settings_in(&tx).await?;
            if !setting.enabled {
                return Ok(None);
            }
            let completed = recent_completed_in(
                &tx,
                key,
                setting.history_generation,
                now_ms(),
                MAX_PER_REGISTRATION,
            )
            .await?;
            for row in completed {
                let exchange = row.decode()?;
                if exchange.turn.state != QaTurnState::Answered || exchange.turn.reply.is_none() {
                    continue;
                }
                if !private_subjects_visible_in(
                    &tx,
                    actor,
                    exchange.conversation.private_source_ids(),
                )
                .await?
                {
                    return Ok(None);
                }
                // A verified reply supersedes the previous question even when
                // it did not open another clarification.
                return Ok(exchange.conversation.pending);
            }
            Ok(None)
        }
    })
    .await?
    .value)
}

async fn latest_exchange(repo: &Repository, actor: &Actor) -> Result<Option<Exchange>, TxError> {
    let key = actor.key()?;
    Ok(write(repo, |tx| {
        let key = &key;
        async move {
            actor.validate_in(&tx).await?;
            let state = settings_in(&tx).await?;
            if !state.enabled {
                return Ok(None);
            }
            let Some(turn_id) = current_turn_in(&tx, key, state.history_generation).await? else {
                return Ok(None);
            };
            let Some(row) = exchange_in(&tx, &turn_id).await? else {
                return Ok(None);
            };
            let exchange = row.decode()?;
            if !private_subjects_visible_in(&tx, actor, exchange.private_citation_ids()).await? {
                return Ok(None);
            }
            Ok(Some(exchange))
        }
    })
    .await?
    .value)
}

async fn append_in(
    tx: &Tx,
    owner: &str,
    generation: i64,
    exchange: &mut Exchange,
    state: QaTurnState,
    finding: Option<QaFinding>,
) -> Result<(), TxError> {
    append_event_in(tx, owner, generation, exchange, state, finding, None).await
}

async fn append_event_in(
    tx: &Tx,
    owner: &str,
    generation: i64,
    exchange: &mut Exchange,
    state: QaTurnState,
    finding: Option<QaFinding>,
    provisional_delta: Option<String>,
) -> Result<(), TxError> {
    record_event(exchange, state, finding, provisional_delta)?;
    persist_events_in(tx, owner, generation, exchange).await
}

fn record_event(
    exchange: &mut Exchange,
    state: QaTurnState,
    finding: Option<QaFinding>,
    provisional_delta: Option<String>,
) -> Result<(), TxError> {
    if let Some(part) = &finding {
        let words = exchange
            .turn
            .findings
            .iter()
            .map(|f| f.text.split_whitespace().count())
            .sum::<usize>();
        if exchange.turn.findings.len() >= QA_MAX_FINDINGS
            || words + part.text.split_whitespace().count() > QA_MAX_WORDS
            || exchange.turn.findings.contains(part)
        {
            return Err(abort("invalid_part"));
        }
        exchange.turn.findings.push(part.clone());
    }
    exchange.turn.cursor += 1;
    exchange.turn.state = state;
    if state.is_terminal() {
        exchange.turn.stage = None;
        if state != QaTurnState::Answered {
            exchange.conversation.focus = None;
            exchange.conversation.pending = None;
        }
        if exchange.plan.is_some()
            && matches!(
                state,
                QaTurnState::Failed | QaTurnState::InsufficientContext
            )
        {
            exchange.turn.findings.clear();
            for event in &mut exchange.events {
                event.finding = None;
            }
        }
        exchange.evidence = None;
        exchange.turn.provisional_reply = None;
        for event in &mut exchange.events {
            event.provisional_delta = None;
        }
    }
    let mut event = QaEvent::new(exchange.turn.cursor, state, finding);
    event.stage = exchange.turn.stage;
    event.provisional_delta = provisional_delta;
    exchange.events.push(event);
    Ok(())
}

async fn persist_events_in(
    tx: &Tx,
    owner: &str,
    generation: i64,
    exchange: &mut Exchange,
) -> Result<(), TxError> {
    let terminal = exchange.turn.state.is_terminal();
    let at = if terminal { now_ms() } else { 0 };
    exchange.turn.completed_at = terminal.then(|| at.div_euclid(1_000));
    exchange.turn.response_time_ms = if terminal {
        exchange.response_time_ms(at)
    } else {
        None
    };
    let payload = serde_json::to_string(exchange).map_err(|_| abort("invalid_store"))?;
    jobs::run(
        tx,
        // Serialize the publication with a generation transition or placement handover.
        tx.query(
            "UPDATE inbox_qa_settings:current SET admission += 1 RETURN NONE;
             UPDATE $turn SET payload=$payload, active=$active, completed_at=$at RETURN NONE;",
        )
        .bind((
            "turn",
            jobs::rid("inbox_qa_exchange", &exchange.turn.turn_id),
        ))
        .bind(("payload", payload))
        .bind(("active", !terminal))
        .bind(("at", at)),
    )
    .await?;
    if terminal {
        jobs::run(tx, tx.query("UPSERT inbox_qa_health:current SET required_after=math::max([required_after ?? 0,$at]) RETURN NONE; UPDATE $latest SET required_after=$at WHERE generation=$gen AND turn=$turn RETURN NONE;")
            .bind(("latest", jobs::rid("inbox_qa_latest", owner))).bind(("at", at))
            .bind(("gen", generation)).bind(("turn", jobs::rid("inbox_qa_exchange", &exchange.turn.turn_id)))).await?;
    }
    bump_event_revision(tx).await?;
    Ok(())
}

/// Seal the caller's turn without erasing already delivered findings.
pub async fn cancel(repo: &Repository, actor: &Actor, id: &str) -> Result<QaTurn, TxError> {
    let owner = actor.key()?;
    Ok(write(repo, |tx| {
        let owner = &owner;
        async move {
            actor.validate_in(&tx).await?;
            let state = settings_in(&tx).await?;
            if current_turn_in(&tx, owner, state.history_generation)
                .await?
                .as_deref()
                != Some(id)
            {
                return Err(abort("missing"));
            }
            let row = exchange_in(&tx, id)
                .await?
                .filter(|row| {
                    state.enabled
                        && row.generation == state.history_generation
                        && row.owner == *owner
                })
                .ok_or_else(|| abort("missing"))?;
            let mut exchange = row.decode()?;
            if !private_subjects_visible_in(&tx, actor, exchange.private_citation_ids()).await? {
                return Err(abort("missing"));
            }
            if row.active {
                append_in(
                    &tx,
                    &row.owner,
                    row.generation,
                    &mut exchange,
                    QaTurnState::Canceled,
                    None,
                )
                .await?;
                retire_in(&tx, vec![id.to_owned()]).await?;
            }
            Ok(exchange.turn)
        }
    })
    .await?
    .value)
}

async fn live_exchange_in(
    tx: &Tx,
    lease: &job_dispatch::DeliveryLease,
) -> Result<Option<(ExchangeRow, Exchange)>, TxError> {
    let state = settings_in(tx).await?;
    let task = &lease.task;
    let Some(row) = exchange_in(tx, &task.job_id)
        .await?
        .filter(|row| state.enabled && row.generation == state.history_generation && row.active)
    else {
        return Ok(None);
    };
    let mut live = jobs::run(tx, tx.query(
        "SELECT VALUE true FROM ONLY $task WHERE lease_owner=$owner AND lease_epoch=$lease AND lease_until>time::now() AND state='running' AND !job.cancel_requested;
         SELECT VALUE true FROM inbox_qa_replica WHERE owner=$owner AND generation=$generation AND lease_until>$now AND ready=true LIMIT 1;",
    ).bind(("task", jobs::rid("task", &task.id))).bind(("owner", task.lease_owner.clone()))
     .bind(("lease", task.lease_epoch)).bind(("generation", state.generation)).bind(("now", now_ms()))).await?;
    if jobs::take::<Option<bool>>(&mut live, 0)? != Some(true)
        || jobs::take::<Vec<bool>>(&mut live, 1)?.is_empty()
    {
        return Ok(None);
    }
    let mut exchange = row.decode()?;
    if exchange.generator_generation.unwrap_or(row.generation) != state.generation {
        return Ok(None);
    }
    exchange.actor.validate_in(tx).await?;
    if exchange
        .plan
        .as_ref()
        .is_some_and(|plan| plan.decision_generation != state.decision_generation)
        || !private_subjects_visible_in(
            tx,
            &exchange.actor,
            exchange
                .private_citation_ids()
                .chain(exchange.conversation.private_source_ids()),
        )
        .await?
    {
        exchange.turn.findings.clear();
        exchange.turn.related_activity = None;
        exchange.events.clear();
        append_in(
            tx,
            &row.owner,
            row.generation,
            &mut exchange,
            QaTurnState::Canceled,
            None,
        )
        .await?;
        retire_in(tx, vec![exchange.turn.turn_id.clone()]).await?;
        return Ok(None);
    }
    Ok(Some((row, exchange)))
}

/// Load the frozen request under a live lease, then expose Running exactly once.
pub async fn start(
    repo: &Repository,
    lease: &job_dispatch::DeliveryLease,
) -> Result<Option<FrozenEvidence>, TxError> {
    Ok(write(repo, |tx| async move {
        let Some((row, mut exchange)) = live_exchange_in(&tx, lease).await? else {
            return Ok(None);
        };
        if exchange.plan.is_some()
            || !exchange.turn.findings.is_empty()
            || exchange.turn.provisional_reply.is_some()
        {
            // A replacement process must not append to an interrupted stream.
            append_in(
                &tx,
                &row.owner,
                row.generation,
                &mut exchange,
                QaTurnState::Failed,
                None,
            )
            .await?;
            return Ok(None);
        }
        if exchange.turn.state == QaTurnState::Queued {
            exchange.turn.stage = Some(QaTurnStage::Working);
            append_in(
                &tx,
                &row.owner,
                row.generation,
                &mut exchange,
                QaTurnState::Running,
                None,
            )
            .await?;
        }
        Ok(exchange.evidence)
    })
    .await?
    .value)
}

/// Publish the planning stage under the same lease that owns the snapshot.
pub async fn begin_planning(
    repo: &Repository,
    lease: &job_dispatch::DeliveryLease,
) -> Result<Option<PlanContext>, TxError> {
    Ok(write(repo, |tx| async move {
        let Some((row, mut exchange)) = live_exchange_in(&tx, lease).await? else {
            return Ok(None);
        };
        if exchange.turn.state != QaTurnState::Running || exchange.plan.is_some() {
            return Ok(None);
        }
        if exchange.turn.stage != Some(QaTurnStage::Thinking) {
            exchange.turn.stage = Some(QaTurnStage::Thinking);
            append_in(
                &tx,
                &row.owner,
                row.generation,
                &mut exchange,
                QaTurnState::Running,
                None,
            )
            .await?;
        }
        let setting = settings_in(&tx).await?;
        Ok(Some(PlanContext {
            focus: exchange.conversation.focus,
            pending: exchange.conversation.pending,
            guidance: exchange.conversation.guidance,
            history_requested: exchange.conversation.history_requested,
            decision_preference: setting.decision_preference,
            decision_generation: setting.decision_generation,
        }))
    })
    .await?
    .value)
}

/// Accept exactly one bounded plan and move to generator work atomically.
pub async fn accept_plan(
    repo: &Repository,
    lease: &job_dispatch::DeliveryLease,
    plan: &ResponsePlan,
) -> Result<bool, TxError> {
    Ok(write(repo, |tx| async move {
        let Some((row, mut exchange)) = live_exchange_in(&tx, lease).await? else {
            return Ok(false);
        };
        if exchange.turn.stage != Some(QaTurnStage::Thinking) || exchange.plan.is_some() {
            return Ok(false);
        }
        let evidence = exchange
            .evidence
            .as_ref()
            .ok_or_else(|| abort("invalid_part"))?;
        if plan.decision_generation != settings_in(&tx).await?.decision_generation
            || !plan.matches(evidence, &exchange.conversation)
            || plan.requires_worker_support && !plan.negotiated
        {
            return Err(abort("invalid_part"));
        }
        exchange.plan = Some(plan.clone());
        exchange.turn.stage = Some(QaTurnStage::Writing);
        append_in(
            &tx,
            &row.owner,
            row.generation,
            &mut exchange,
            QaTurnState::Running,
            None,
        )
        .await?;
        Ok(true)
    })
    .await?
    .value)
}

/// Record unavailable plan capability or context overflow as terminal status.
pub async fn publish_unavailable(
    repo: &Repository,
    lease: &job_dispatch::DeliveryLease,
    limitation: QaTurnLimitation,
) -> Result<bool, TxError> {
    Ok(write(repo, |tx| async move {
        let Some((row, mut exchange)) = live_exchange_in(&tx, lease).await? else {
            return Ok(false);
        };
        let expected_stage = match limitation {
            QaTurnLimitation::CapabilityUnavailable => QaTurnStage::Thinking,
            QaTurnLimitation::QuestionTooLong => QaTurnStage::Writing,
            _ => return Err(abort("invalid_part")),
        };
        if exchange.turn.stage != Some(expected_stage) {
            return Err(abort("invalid_part"));
        }
        exchange.turn.notice = Some(QaTurnNotice::ObservationUnavailable);
        exchange.turn.limitation = Some(limitation);
        append_in(
            &tx,
            &row.owner,
            row.generation,
            &mut exchange,
            QaTurnState::InsufficientContext,
            None,
        )
        .await?;
        Ok(true)
    })
    .await?
    .value)
}

/// Check that a worker can continue without crossing a Stop, Off or authority fence.
pub async fn is_live(
    repo: &Repository,
    lease: &job_dispatch::DeliveryLease,
) -> Result<bool, TxError> {
    Ok(write(repo, |tx| async move {
        Ok(live_exchange_in(&tx, lease).await?.is_some())
    })
    .await?
    .value)
}

/// Commit one validated part or terminal state under the exact live execution lease.
pub async fn publish(
    repo: &Repository,
    lease: &job_dispatch::DeliveryLease,
    state: QaTurnState,
    finding: Option<QaFinding>,
) -> Result<bool, TxError> {
    publish_with_model_result(repo, lease, state, finding, ModelResult::default()).await
}

/// Publish a bounded draft under the same registration and lease fences as the answer.
pub async fn publish_draft(
    repo: &Repository,
    lease: &job_dispatch::DeliveryLease,
    text: String,
) -> Result<bool, TxError> {
    Ok(write(repo, |tx| {
        let text = text.clone();
        async move {
            let Some((row, mut exchange)) = live_exchange_in(&tx, lease).await? else {
                return Ok(false);
            };
            let Some(evidence) = exchange.evidence.as_ref() else {
                return Err(abort("invalid_part"));
            };
            let Some(dialogue) = evidence.request.dialogue.as_ref() else {
                return Err(abort("invalid_part"));
            };
            let planned_request = exchange
                .plan
                .as_ref()
                .and_then(|plan| plan.request(evidence));
            let request = planned_request.as_ref().unwrap_or(&evidence.request);
            let draft = exchange
                .turn
                .provisional_reply
                .get_or_insert_with(String::new);
            if exchange.turn.state != QaTurnState::Running
                || draft.len() + text.len() > QA_MAX_DRAFT_BYTES
                || exchange
                    .events
                    .iter()
                    .filter(|event| event.provisional_delta.is_some())
                    .count()
                    >= QA_MAX_DRAFT_EVENTS as usize
                || !valid_draft_delta(&text)
            {
                return Err(abort("invalid_part"));
            }
            draft.push_str(&text);
            let valid = if dialogue.kind == QaDialogueKind::Operational {
                valid_operational_draft(request, draft)
            } else {
                valid_uncited_reply(request, draft)
            };
            if !valid {
                return Err(abort("invalid_part"));
            }
            append_event_in(
                &tx,
                &row.owner,
                row.generation,
                &mut exchange,
                QaTurnState::Running,
                None,
                Some(text),
            )
            .await?;
            Ok(true)
        }
    })
    .await?
    .value)
}

/// Verified result of the actual worker handling a terminal turn.
#[derive(Clone, Default)]
pub struct ModelResult {
    pub context: Option<QaContextUsage>,
    pub reply: Option<String>,
    pub model_target: Option<QaModelTarget>,
}

/// Commit model usage with the same terminal transition and execution lease as the answer.
pub async fn publish_with_model_result(
    repo: &Repository,
    lease: &job_dispatch::DeliveryLease,
    state: QaTurnState,
    finding: Option<QaFinding>,
    result: ModelResult,
) -> Result<bool, TxError> {
    publish_parts(repo, lease, state, finding, None, result).await
}

/// Commit generator-selected findings and the terminal state under the live lease.
/// Rechecks operational freshness after acquiring the writer, before recording the batch.
pub async fn publish_findings_and_complete(
    repo: &Repository,
    lease: &job_dispatch::DeliveryLease,
    state: QaTurnState,
    findings: &[QaFinding],
    result: ModelResult,
) -> Result<bool, TxError> {
    if !state.is_terminal() || (state != QaTurnState::Answered && !findings.is_empty()) {
        return Err(abort("invalid_part"));
    }
    publish_parts(repo, lease, state, None, Some(findings), result).await
}

async fn publish_parts(
    repo: &Repository,
    lease: &job_dispatch::DeliveryLease,
    state: QaTurnState,
    finding: Option<QaFinding>,
    terminal_findings: Option<&[QaFinding]>,
    result: ModelResult,
) -> Result<bool, TxError> {
    Ok(write(repo, |tx| {
        let finding = finding.clone();
        let ModelResult {
            context,
            mut reply,
            model_target,
        } = result.clone();
        async move {
            let Some((row, mut exchange)) = live_exchange_in(&tx, lease).await? else {
                return Ok(false);
            };
            let mut state = state;
            if let Some(findings) = terminal_findings {
                let expired = exchange.evidence.as_ref().is_some_and(|evidence| {
                    evidence
                        .request
                        .dialogue
                        .as_ref()
                        .is_some_and(|dialogue| dialogue.kind == QaDialogueKind::Operational)
                        && !(0..=MAX_EVIDENCE_AGE_MS)
                            .contains(&now_ms().saturating_sub(evidence.sampled_at_ms))
                });
                if expired {
                    reply = None;
                    if state == QaTurnState::Answered {
                        state = QaTurnState::InsufficientContext;
                    }
                } else {
                    if !findings.is_empty() {
                        let evidence = exchange
                            .evidence
                            .as_ref()
                            .ok_or_else(|| abort("invalid_part"))?;
                        if findings.iter().any(|finding| {
                            !evidence.sources.iter().any(|source| {
                                source.reference == finding.source
                                    && source.observed_at == finding.observed_at
                                    && source.provenance == finding.provenance
                                    && source.observation_is_valid(evidence.sampled_at_ms, now_ms())
                            })
                        }) {
                            return Err(abort("invalid_part"));
                        }
                    }
                    for finding in findings {
                        record_event(
                            &mut exchange,
                            QaTurnState::Running,
                            Some(finding.clone()),
                            None,
                        )?;
                    }
                }
            }
            let dialogue = exchange
                .evidence
                .as_ref()
                .and_then(|evidence| evidence.request.dialogue.as_ref());
            let has_model_usage = context.is_some_and(|usage| usage.model_windows > 0);
            if state == QaTurnState::Answered {
                if dialogue.is_none()
                    || !has_model_usage
                    || model_target.is_none()
                    || !reply.as_deref().is_some_and(valid_dialogue_reply)
                {
                    return Err(abort("invalid_part"));
                }
                if let Some((plan, evidence)) =
                    exchange.plan.as_ref().zip(exchange.evidence.as_ref())
                {
                    let request = plan
                        .request(evidence)
                        .ok_or_else(|| abort("invalid_part"))?;
                    let valid = reply.as_deref().is_some_and(|reply| {
                        if dialogue
                            .is_some_and(|dialogue| dialogue.kind == QaDialogueKind::Operational)
                        {
                            verified_operational_sources(&request, reply).is_some_and(|passages| {
                                if !plan.negotiated {
                                    return true;
                                }
                                passages.len() == exchange.turn.findings.len()
                                    && passages.into_iter().zip(&exchange.turn.findings).all(
                                        |(passage, finding)| {
                                            evidence
                                                .sources
                                                .iter()
                                                .find(|source| {
                                                    source.opaque_id == passage.source_id
                                                })
                                                .is_some_and(|source| {
                                                    source.observation_is_valid(
                                                        evidence.sampled_at_ms,
                                                        now_ms(),
                                                    ) && *finding
                                                        == source.finding(
                                                            passage.text.clone(),
                                                            0,
                                                            passage.text.chars().count() as u32,
                                                        )
                                                })
                                        },
                                    )
                            })
                        } else {
                            valid_uncited_reply(&request, reply)
                        }
                    });
                    if plan.decision_generation != settings_in(&tx).await?.decision_generation
                        || !valid
                    {
                        return Err(abort("invalid_part"));
                    }
                }
            } else if reply.is_some() {
                return Err(abort("invalid_part"));
            }
            if state.is_terminal() {
                if exchange.plan.as_ref().is_none_or(|plan| !plan.negotiated) {
                    exchange.conversation.pending = None;
                }
                if state == QaTurnState::Answered {
                    match dialogue.map(|dialogue| dialogue.kind) {
                        Some(QaDialogueKind::Greeting) => {
                            exchange.turn.notice = Some(QaTurnNotice::Greeting);
                        }
                        Some(QaDialogueKind::Help) => {
                            exchange.turn.conversation = Some(QaConversationReply::Help);
                        }
                        Some(QaDialogueKind::Acknowledgement) => {
                            exchange.turn.conversation = Some(QaConversationReply::Acknowledgement);
                        }
                        _ => {}
                    }
                }
                if state == QaTurnState::Answered
                    && dialogue.is_some_and(|dialogue| {
                        dialogue.kind == QaDialogueKind::Recall && !dialogue.history.is_empty()
                    })
                {
                    exchange.turn.used_previous_exchange = true;
                }
                exchange.turn.context = context;
                exchange.turn.reply = reply;
                if let Some(target) = model_target.filter(|_| has_model_usage) {
                    exchange.turn.model_target = Some(target);
                    exchange.turn.model_generation =
                        u64::try_from(exchange.generator_generation.unwrap_or(row.generation)).ok();
                }
            }
            append_in(
                &tx,
                &row.owner,
                row.generation,
                &mut exchange,
                state,
                finding,
            )
            .await?;
            Ok(true)
        }
    })
    .await?
    .value)
}

/// One backend-wide sample. It contains allowlisted health facts and no private activity.
#[derive(Clone, Deserialize, Serialize)]
pub struct HealthSample {
    pub sampled_at_ms: i64,
    pub facts: Vec<HealthFact>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct HealthFact {
    pub id: String,
    pub label: String,
    pub status: String,
    pub explanation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(SurrealValue)]
struct HealthRow {
    required_after: i64,
    sampled_at: i64,
    lease_until: i64,
    payload: String,
}

pub enum HealthClaim {
    Ready(HealthSample),
    Pending,
    Probe(i64),
}

/// Heap and probe work are shared across gateways using one short-lived durable claim.
pub async fn claim_health(
    repo: &Repository,
    publisher: &str,
    generation: i64,
    min_after: i64,
    for_admission: bool,
) -> Result<HealthClaim, TxError> {
    Ok(write(repo, |tx| async move {
        let setting = settings_in(&tx).await?;
        if !setting.enabled || setting.generation != generation {
            return Err(abort("off"));
        }
        let now = now_ms();
        let mut reply = jobs::run(
            &tx,
            tx.query("SELECT required_after,sampled_at,lease_until,payload FROM ONLY inbox_qa_health:current"),
        ).await?;
        let health = jobs::take::<Option<HealthRow>>(&mut reply, 0)?;
        let boundary = health.as_ref().map_or(min_after, |row| row.required_after.max(min_after));
        if let Some(row) = health {
            if row.sampled_at > boundary && (!for_admission || row.sampled_at > now - 5_000) {
                return if for_admission {
                    serde_json::from_str(&row.payload).map(HealthClaim::Ready).map_err(|_| abort("invalid_store"))
                } else {
                    Ok(HealthClaim::Pending)
                };
            }
            if row.lease_until > now {
                return Ok(HealthClaim::Pending);
            }
        }
        jobs::run(&tx, tx.query(
            "UPDATE inbox_qa_settings:current SET admission += 1 RETURN NONE;
             UPSERT inbox_qa_health:current SET publisher=$publisher, lease_until=$until RETURN NONE;",
        ).bind(("publisher", publisher.to_owned())).bind(("until", now + 5_000))).await?;
        Ok(HealthClaim::Probe(boundary))
    }).await?.value)
}

/// Publish only into the current enabled generation and only under the probe lease.
pub async fn put_health(
    repo: &Repository,
    publisher: &str,
    generation: i64,
    sample: &HealthSample,
) -> Result<bool, TxError> {
    let payload = serde_json::to_string(sample).map_err(|_| abort("invalid_store"))?;
    Ok(write(repo, |tx| {
        let payload = &payload;
        async move {
            let setting = settings_in(&tx).await?;
            if !setting.enabled || setting.generation != generation {
                return Ok(false);
            }
            let mut reply = jobs::run(&tx, tx.query(
                "UPDATE inbox_qa_settings:current SET admission += 1 RETURN NONE;
                 UPDATE ONLY inbox_qa_health:current SET sampled_at=$sample, payload=$payload, lease_until=0
                 WHERE publisher=$publisher AND lease_until>$now AND required_after<$sample RETURN VALUE true;",
            ).bind(("publisher", publisher.to_owned())).bind(("sample", sample.sampled_at_ms))
              .bind(("payload", payload.clone())).bind(("now", now_ms()))).await?;
            Ok(jobs::take::<Option<bool>>(&mut reply, 1)? == Some(true))
        }
    }).await?.value)
}

/// Release a failed/obsolete probe without canceling a successor's lease.
pub async fn abandon_health(repo: &Repository, publisher: &str) -> Result<(), TxError> {
    let _writer = repo.lock_inbox_qa_writer().await;
    repo.db()
        .query("UPDATE inbox_qa_health:current SET lease_until=0 WHERE publisher=$publisher AND lease_until>0 RETURN NONE")
        .bind(("publisher", publisher.to_owned()))
        .await?
        .check()?;
    Ok(())
}

async fn retire_in(tx: &Tx, ids: Vec<String>) -> Result<(), TxError> {
    if ids.is_empty() {
        return Ok(());
    }
    let roots = ids
        .iter()
        .map(|id| jobs::rid("job", id))
        .collect::<Vec<_>>();
    jobs::run(
        tx,
        tx.query(
            "DELETE job_event WHERE job_id IN $ids RETURN NONE;
         DELETE attempt WHERE root IN $roots RETURN NONE;
         DELETE task WHERE root IN $roots RETURN NONE;
         DELETE job WHERE root IN $roots RETURN NONE;",
        )
        .bind(("roots", roots))
        .bind(("ids", ids)),
    )
    .await?;
    Ok(())
}

/// Retire expired private payloads and ensure an absent executor cannot leave an open turn.
pub async fn cleanup(repo: &Repository) -> Result<(), TxError> {
    write(repo, |tx| async move {
        let enabled = settings_in(&tx).await?.enabled;
        let mut expired = jobs::run(&tx, tx.query(
            "SELECT VALUE record::id(id) FROM inbox_qa_exchange
                WHERE active=true AND (!$enabled OR accepted_at < $active_cutoff) LIMIT 16;
             SELECT VALUE record::id(id) FROM inbox_qa_exchange WHERE completed_at>0 AND completed_at<$retained_cutoff LIMIT 16;",
        ).bind(("enabled", enabled)).bind(("active_cutoff", now_ms() - MAX_ACTIVE_AGE_MS))
         .bind(("retained_cutoff", now_ms() - EXCHANGE_TTL_MS))).await?;
        let mut retire = Vec::new();
        for id in jobs::take::<Vec<String>>(&mut expired, 0)? {
            if let Some(row) = exchange_in(&tx, &id).await?.filter(|row| row.active) {
                let mut exchange = row.decode()?;
                append_in(&tx, &row.owner, row.generation, &mut exchange, QaTurnState::Failed, None).await?;
                retire.push(id);
            }
        }
        let expired_turns = jobs::take::<Vec<String>>(&mut expired, 1)?;
        let turns = expired_turns.iter().map(|id| jobs::rid("inbox_qa_exchange", id)).collect::<Vec<_>>();
        retire.extend(expired_turns);
        retire_in(&tx, retire).await?;
        if !turns.is_empty() {
            jobs::run(&tx, tx.query(
                "DELETE inbox_qa_latest WHERE turn IN $turns AND required_after<$cutoff RETURN NONE;
                 DELETE inbox_qa_exchange WHERE id IN $turns RETURN NONE;",
            ).bind(("turns", turns)).bind(("cutoff", now_ms() - EXCHANGE_TTL_MS))).await?;
            bump_event_revision(&tx).await?;
        }
        Ok(())
    }).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{retained_generated_reply, ChatSourceKind, ChatSourceReference};
    use super::{QaContextUsage, QaFinding, QaTurn, QaTurnNotice, QaTurnState};

    #[test]
    fn recall_requires_a_stored_answer_with_verified_generator_usage() {
        let mut turn = QaTurn::queued("turn".into(), "request".into(), "Check the gateway.".into());
        turn.state = QaTurnState::Answered;
        turn.context = Some(QaContextUsage::new(512, 72, 1));

        turn.notice = Some(QaTurnNotice::Greeting);
        assert_eq!(retained_generated_reply(&turn), None);
        turn.notice = None;
        turn.findings.push(QaFinding::new(
            ChatSourceReference::new(ChatSourceKind::Readiness, "gateway".into()).unwrap(),
            "Gateway".into(),
            "Gateway is ready.".into(),
            0,
            17,
            "2026-11-26T00:00:00Z".into(),
        ));
        assert_eq!(retained_generated_reply(&turn), None);

        turn.reply = Some("Gateway is ready.".into());
        assert!(turn.model_target.is_none());
        assert!(turn.model_generation.is_none());
        assert_eq!(retained_generated_reply(&turn), Some("Gateway is ready."));

        turn.state = QaTurnState::InsufficientContext;
        assert_eq!(retained_generated_reply(&turn), None);
        turn.state = QaTurnState::Answered;
        for usage in [
            None,
            Some(QaContextUsage::new(512, 0, 0)),
            Some(QaContextUsage::new(512, 0, 1)),
            Some(QaContextUsage::new(512, 513, 1)),
        ] {
            turn.context = usage;
            assert_eq!(retained_generated_reply(&turn), None);
        }
        turn.context = Some(QaContextUsage::new(896, 724, 1));
        assert_eq!(retained_generated_reply(&turn), Some("Gateway is ready."));
        for reply in ["", "  ", "Current user: Check the gateway."] {
            turn.reply = Some(reply.into());
            assert_eq!(retained_generated_reply(&turn), None);
        }
    }
}
