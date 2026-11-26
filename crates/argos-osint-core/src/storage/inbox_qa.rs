//! Transactional Inbox state. Model processes never receive this repository.

use std::collections::BTreeSet;
use std::future::Future;

use chrono::Utc;
use fleetworks_contracts::chat::ChatSourceKind;
use fleetworks_contracts::chat::ChatSourceReference;
use fleetworks_contracts::chat::QaConversationReply;
use fleetworks_contracts::chat::QaDecisionPreference;
use fleetworks_contracts::chat::QaEvent;
use fleetworks_contracts::chat::QaFinding;
use fleetworks_contracts::chat::QaGeneratorPreference;
use fleetworks_contracts::chat::QaMeasurement;
use fleetworks_contracts::chat::QaModelTarget;
use fleetworks_contracts::chat::QaRelatedActivity;
use fleetworks_contracts::chat::QaTurn;
use fleetworks_contracts::chat::QaTurnNotice;
use fleetworks_contracts::chat::QaTurnRequest;
use fleetworks_contracts::chat::QaTurnState;
use fleetworks_contracts::chat::QA_MAX_FINDINGS;
use fleetworks_contracts::chat::QA_MAX_RETAINED_TURNS;
use fleetworks_contracts::chat::QA_MAX_WORDS;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::qa_worker::valid_dialogue_reply;
use fleetworks_contracts::qa_worker::valid_draft_delta;
use fleetworks_contracts::qa_worker::verified_operational_sources;
use fleetworks_contracts::qa_worker::QaContextUsage;
use fleetworks_contracts::qa_worker::QaDialogueExchange;
use fleetworks_contracts::qa_worker::QaDialogueKind;
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

/// Decide whether a frozen request is answered from an exact fact or sent to the model.
#[derive(Clone, Copy, Default, Deserialize, Serialize)]
pub enum AnswerMode {
    #[default]
    Model,
    /// Publish complete, measured facts without model selection.
    Grounded,
}

/// Frozen evidence and metadata retained inside the coordinator during execution.
#[derive(Clone, Deserialize, Serialize)]
pub struct FrozenEvidence {
    #[serde(default)]
    pub answer_mode: AnswerMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<QaTurnNotice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<QaConversationReply>,
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
    pub measurement: Option<QaMeasurement>,
    /// For summary selections, display the complete observed state from the chosen passage.
    #[serde(default)]
    pub whole_passage: bool,
}

impl Source {
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

/// Trusted request metadata that is independent of any model reply.
pub struct AdmissionMetadata {
    pub subject: Option<ChatSourceReference>,
    pub observation: Option<Observation>,
    pub related_activity: Option<QaRelatedActivity>,
}

#[derive(Clone, Deserialize, Serialize)]
struct Exchange {
    actor: Actor,
    source: Option<ChatSourceReference>,
    turn: QaTurn,
    events: Vec<QaEvent>,
    evidence: Option<FrozenEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resolved_question: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resolved_source: Option<ChatSourceReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    observation: Option<Observation>,
    /// Older rows used the history generation for both the worker and transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generator_generation: Option<i64>,
}

impl Exchange {
    fn subject(&self) -> Option<&ChatSourceReference> {
        self.resolved_source.as_ref().or(self.source.as_ref())
    }

    fn private_subject_ids(&self) -> impl Iterator<Item = &str> {
        let related = match &self.turn.related_activity {
            Some(QaRelatedActivity::Found { target, .. }) => Some(target),
            _ => None,
        };
        self.subject()
            .into_iter()
            .chain(self.turn.findings.iter().map(|finding| &finding.source))
            .chain(related)
            .filter(|source| private_subject(source))
            .map(|source| source.id.as_str())
    }
}

fn private_subject(source: &ChatSourceReference) -> bool {
    matches!(source.kind, ChatSourceKind::Job | ChatSourceKind::Cycle)
}

async fn subject_visible_in(
    tx: &Tx,
    actor: &Actor,
    source: Option<&ChatSourceReference>,
) -> Result<bool, TxError> {
    let Some(source) = source.filter(|source| private_subject(source)) else {
        return Ok(true);
    };
    let jobs = vec![jobs::rid("job", &source.id)];
    Ok(!visible_private_subjects_in(tx, actor, jobs)
        .await?
        .is_empty())
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
        exchange.turn.accepted_at = Some(self.accepted_at.div_euclid(1_000));
        exchange.turn.completed_at = (exchange.turn.state.is_terminal() && self.completed_at > 0)
            .then(|| self.completed_at.div_euclid(1_000));
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
            if !subject_visible_in(&tx, actor, exchange.subject()).await? {
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

fn grounded_findings(evidence: &FrozenEvidence) -> Result<Vec<QaFinding>, TxError> {
    evidence
        .request
        .validate()
        .map_err(|_| abort("invalid_part"))?;
    let passages = &evidence.request.passages;
    if passages.len() != evidence.sources.len() || passages.len() > QA_MAX_FINDINGS {
        return Err(abort("invalid_part"));
    }
    passages
        .iter()
        .zip(&evidence.sources)
        .map(|(passage, source)| {
            if passage.source_id != source.opaque_id {
                return Err(abort("invalid_part"));
            }
            let end =
                u32::try_from(passage.text.chars().count()).map_err(|_| abort("invalid_part"))?;
            Ok(source.finding(passage.text.clone(), 0, end))
        })
        .collect()
}

fn fallback_findings(evidence: &FrozenEvidence) -> Result<Vec<QaFinding>, TxError> {
    evidence
        .request
        .validate()
        .map_err(|_| abort("invalid_part"))?;
    let passages = &evidence.request.passages;
    if passages.len() != evidence.sources.len() {
        return Err(abort("invalid_part"));
    }
    let mut remaining_words = QA_MAX_WORDS;
    let mut findings = Vec::new();
    for (passage, source) in passages.iter().zip(&evidence.sources) {
        if passage.source_id != source.opaque_id {
            return Err(abort("invalid_part"));
        }
        let Some(words) = remaining_words.checked_sub(passage.text.split_whitespace().count())
        else {
            continue;
        };
        let end = u32::try_from(passage.text.chars().count()).map_err(|_| abort("invalid_part"))?;
        findings.push(source.finding(passage.text.clone(), 0, end));
        remaining_words = words;
        if findings.len() == QA_MAX_FINDINGS {
            break;
        }
    }
    Ok(findings)
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
        let turn = exchange.turn;
        let answer = if let Some(reply) = turn.reply {
            reply
        } else if turn.state == QaTurnState::Answered && !turn.findings.is_empty() {
            turn.findings
                .into_iter()
                .map(|finding| finding.text)
                .collect::<Vec<_>>()
                .join(" ")
        } else if turn.notice == Some(QaTurnNotice::Greeting) {
            match turn.conversation {
                Some(QaConversationReply::Acknowledgement) => {
                    "Of course. What else can I help with?"
                }
                Some(QaConversationReply::Help) => {
                    "I can answer questions about workstation status and resources."
                }
                _ => "Hi! How can I help?",
            }
            .to_owned()
        } else {
            continue;
        };
        history.push(QaDialogueExchange {
            question: dialogue_line(&turn.question).to_owned(),
            reply: dialogue_line(&answer).to_owned(),
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
                if !subject_visible_in(&tx, actor, exchange.subject()).await? {
                    return Err(abort("missing"));
                }
                return Ok(exchange.turn);
            }
            if !subject_visible_in(&tx, actor, metadata.subject.as_ref()).await? {
                return Err(abort("missing"));
            }
            let boundary = boundary_in(&tx, key, generation).await?;
            let now = Utc::now();
            if evidence.sampled_at_ms <= boundary
                || evidence.sampled_at_ms < now.timestamp_millis() - MAX_EVIDENCE_AGE_MS
                || evidence.sampled_at_ms > now.timestamp_millis()
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
            let needs_model = matches!(evidence.answer_mode, AnswerMode::Model) && ready > 0;
            if !jobs::take::<Vec<bool>>(&mut reply, 2)?.is_empty() {
                return Err(abort("active"));
            }
            let active = jobs::take::<Option<i64>>(&mut reply, 3)?.ok_or_else(|| abort("invalid_store"))?;
            let owner_retained = jobs::take::<Option<i64>>(&mut reply, 4)?.ok_or_else(|| abort("invalid_store"))?;
            let total_retained = jobs::take::<Option<i64>>(&mut reply, 5)?.ok_or_else(|| abort("invalid_store"))?;
            if (needs_model && active >= MAX_ACTIVE.min(ready * 2))
                || owner_retained >= MAX_PER_REGISTRATION
                || total_retained >= MAX_RETAINED
            {
                return Err(abort("busy"));
            }
            let mut frozen = evidence.clone();
            if let Some(dialogue) = frozen.request.dialogue.as_mut().filter(|_| needs_model) {
                if dialogue.kind == QaDialogueKind::Operational {
                    // The snapshot already resolves the approved subject and comparison.
                    frozen.request.question = request.question.clone();
                } else if dialogue.kind == QaDialogueKind::Recall {
                    dialogue.history = recent_dialogue_in(&tx, key, generation, timestamp).await?;
                }
            }
            let mut exchange = Exchange {
                actor: actor.clone(),
                source: request.source.clone(),
                turn: QaTurn::queued(id.to_owned(), request.request_id.clone(), request.question.clone()),
                events: Vec::new(),
                evidence: Some(frozen),
                resolved_question: (request.question != evidence.request.question)
                    .then(|| evidence.request.question.clone()),
                resolved_source: metadata.subject.as_ref().filter(|source| Some(*source) != request.source.as_ref()).cloned(),
                observation: metadata.observation.clone(),
                generator_generation: Some(state.generation),
            };
            exchange.turn.accepted_at = Some(timestamp.div_euclid(1_000));
            exchange.turn.related_activity = metadata.related_activity.clone();
            let payload = serde_json::to_string(&exchange).map_err(|_| abort("invalid_store"))?;
            jobs::run(&tx, tx.query(
                "CREATE $turn SET generation=$gen, owner=$owner, workstation=$workstation, active=true, accepted_at=$at, completed_at=0, payload=$payload RETURN NONE;
                 UPSERT $latest SET generation=$gen, turn=$turn, required_after=$boundary RETURN NONE;",
            ).bind(("turn", jobs::rid("inbox_qa_exchange", id))).bind(("gen", generation))
              .bind(("owner", key.clone())).bind(("workstation", actor.workstation())).bind(("at", timestamp)).bind(("payload", payload))
              .bind(("latest", jobs::rid("inbox_qa_latest", key))).bind(("boundary", boundary))).await?;
            if !needs_model {
                // Exact facts need no worker lease. Publish the canonical turn and its
                // events in the admission transaction so Off need not race settlement.
                let findings = if (0..=MAX_EVIDENCE_AGE_MS)
                    .contains(&now_ms().saturating_sub(evidence.sampled_at_ms))
                {
                    if matches!(evidence.answer_mode, AnswerMode::Grounded) {
                        grounded_findings(evidence)?
                    } else {
                        fallback_findings(evidence)?
                    }
                } else {
                    Vec::new()
                };
                let terminal = if findings.is_empty() {
                    exchange.turn.notice = evidence.notice.or(Some(QaTurnNotice::ObservationUnavailable));
                    exchange.turn.conversation = evidence.conversation;
                    QaTurnState::InsufficientContext
                } else {
                    for finding in findings {
                        append_in(&tx, key, generation, &mut exchange, QaTurnState::Running, Some(finding)).await?;
                    }
                    QaTurnState::Answered
                };
                exchange.turn.context = Some(QaContextUsage::new(QaModelTarget::default().input_tokens(), 0, 0));
                append_in(&tx, key, generation, &mut exchange, terminal, None).await?;
                return Ok(exchange.turn);
            }
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
            for row in recent_completed_in(
                &tx,
                key,
                setting.history_generation,
                now_ms(),
                QA_MAX_DIALOGUE_HISTORY as i64,
            )
            .await?
            {
                let exchange = row.decode()?;
                if exchange.turn.state == QaTurnState::Answered
                    && !exchange.turn.findings.is_empty()
                {
                    if !subject_visible_in(&tx, actor, exchange.subject()).await? {
                        return Ok(None);
                    }
                    let subject = exchange.subject().cloned();
                    let observation = exchange.observation.filter(|observation| {
                        exchange.turn.findings.iter().any(|finding| {
                            finding.source == observation.source
                                && finding.observed_at == observation.observed_at
                        })
                    });
                    return Ok(Some(AnsweredPrompt {
                        subject,
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
            if !subject_visible_in(&tx, actor, exchange.subject()).await? {
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
        exchange.evidence = None;
        exchange.turn.provisional_reply = None;
        for event in &mut exchange.events {
            event.provisional_delta = None;
        }
    }
    let mut event = QaEvent::new(exchange.turn.cursor, state, finding);
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
            if !subject_visible_in(&tx, actor, exchange.subject()).await? {
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
    if !subject_visible_in(tx, &exchange.actor, exchange.subject()).await? {
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
        if !exchange.turn.findings.is_empty() || exchange.turn.provisional_reply.is_some() {
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
            if dialogue.kind == QaDialogueKind::Operational
                && verified_operational_sources(&evidence.request, draft).is_none()
            {
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

/// Commit fallback findings and the terminal state together under the live lease.
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
                    if state == QaTurnState::Answered && exchange.turn.findings.is_empty() {
                        state = QaTurnState::InsufficientContext;
                    }
                } else {
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
            let dialogue_evidence = exchange
                .evidence
                .as_ref()
                .filter(|evidence| evidence.request.dialogue.is_some());
            if reply.as_deref().is_some_and(|text| {
                dialogue_evidence.is_none()
                    || state != QaTurnState::Answered
                    || !valid_dialogue_reply(text)
            }) || (state == QaTurnState::Answered
                && exchange.turn.findings.is_empty()
                && reply.is_none())
            {
                return Err(abort("invalid_part"));
            }
            if state.is_terminal() {
                exchange.turn.context = context;
                exchange.turn.reply = reply;
                if let Some(target) =
                    model_target.filter(|_| context.is_some_and(|usage| usage.model_windows > 0))
                {
                    exchange.turn.model_target = Some(target);
                    exchange.turn.model_generation =
                        u64::try_from(exchange.generator_generation.unwrap_or(row.generation)).ok();
                }
                if let Some(evidence) = dialogue_evidence.filter(|_| {
                    matches!(
                        state,
                        QaTurnState::Answered | QaTurnState::InsufficientContext
                    )
                }) {
                    exchange.turn.notice = evidence.notice;
                    exchange.turn.conversation = evidence.conversation;
                    if exchange.turn.reply.is_none() {
                        if let Some(dialogue) = evidence
                            .request
                            .dialogue
                            .as_ref()
                            .filter(|dialogue| dialogue.kind == QaDialogueKind::Recall)
                        {
                            exchange.turn.reply = dialogue
                                .history
                                .last()
                                .map(|exchange| format!("You asked “{}”", exchange.question))
                                .filter(|text| valid_dialogue_reply(text));
                        }
                    }
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
