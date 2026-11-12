//! Bounded read-only backend observations for the native Workstation page.
//!
//! Owners are configured at startup; the request cannot choose check URLs.
//! Refresh never seeds/trains/indexes, scans a catalog, warms a model or creates
//! infrastructure. Process reachability and verified inference are distinct.

use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use argos_osint_core::storage::actor_search_outbox;
use argos_osint_core::storage::classifier_activation;
use argos_osint_core::storage::setup_jobs;
use axum::extract::State;
use axum::http::header;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::Extension;
use axum::Json;
use chrono::Utc;
use fleetworks_contracts::actor_index::ActorIndexStatus;
use fleetworks_contracts::actor_index::ACTOR_INDEX_ANALYZER;
use fleetworks_contracts::actor_index::ACTOR_INDEX_PROTOCOL;
use fleetworks_contracts::actor_index::ACTOR_INDEX_SCHEMA;
use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::ReadinessAction;
use fleetworks_contracts::readiness::ReadinessActionId;
use fleetworks_contracts::readiness::ReadinessCheck;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use fleetworks_contracts::readiness::WorkstationReadiness;
use futures_util::future::join_all;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Mutex;

use crate::ApiState;
use crate::AuthenticatedPrincipal;
use crate::SearchClient;

const DEADLINE: Duration = Duration::from_secs(2);
const CACHE_AGE: Duration = Duration::from_secs(5);
const RESPONSE_LIMIT: usize = 64 * 1024;

/// A fixed private ready route with a dedicated configured service credential.
/// Does not implement Debug/Serialize; token-bearing clients stay server-only.
#[derive(Clone)]
pub struct ReadinessProbe {
    id: ReadinessCheckId,
    http: reqwest::Client,
    url: url::Url,
}

impl ReadinessProbe {
    pub fn new(id: ReadinessCheckId, origin: &str, token: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(
            matches!(
                id,
                ReadinessCheckId::Brain
                    | ReadinessCheckId::Control
                    | ReadinessCheckId::Jobs
                    | ReadinessCheckId::Nlp
                    | ReadinessCheckId::Classifier
            ),
            "unsupported readiness owner"
        );
        anyhow::ensure!(
            (32..=256).contains(&token.len())
                && token
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-._~".contains(&byte)),
            "invalid readiness service credential"
        );
        let origin = crate::network_origin(origin)?;
        let mut url = url::Url::parse(&origin)?;
        url.set_path("/api/v1/health/ready");
        let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))?;
        authorization.set_sensitive(true);
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::AUTHORIZATION, authorization);
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(1))
            .timeout(DEADLINE)
            .default_headers(headers)
            .build()?;
        Ok(Self { id, http, url })
    }

    async fn observe(&self, at: &str) -> Vec<ReadinessCheck> {
        let response = self.fetch().await;
        let mut checks = vec![match &response {
            Ok(ready) if ready.ready => check(
                self.id,
                ReadinessState::Ready,
                "process_ready",
                "Configured service reports ready",
                at,
            ),
            Ok(_) => check(
                self.id,
                ReadinessState::NotReady,
                "process_not_ready",
                "Configured service reports not ready",
                at,
            ),
            Err(_) => check(
                self.id,
                ReadinessState::Unavailable,
                "owner_unavailable",
                "Configured service status is unavailable",
                at,
            ),
        }];
        if self.id == ReadinessCheckId::Classifier {
            checks.push(match response {
                Ok(ready) if ready.ready && ready.model_state.as_deref() == Some("verified") => {
                    check(
                        ReadinessCheckId::ClassifierInference,
                        ReadinessState::Ready,
                        "model_verified",
                        "Rust-BERT inference owner reports a verified active model",
                        at,
                    )
                }
                Ok(_) => check(
                    ReadinessCheckId::ClassifierInference,
                    ReadinessState::Unknown,
                    "model_unverified",
                    "Service reachability does not establish verified Rust-BERT inference",
                    at,
                ),
                Err(_) => check(
                    ReadinessCheckId::ClassifierInference,
                    ReadinessState::Unavailable,
                    "inference_owner_unavailable",
                    "Rust-BERT inference status is unavailable",
                    at,
                ),
            });
        }
        checks
    }

    async fn fetch(&self) -> Result<ProcessReady, ()> {
        let mut response = self
            .http
            .get(self.url.clone())
            .send()
            .await
            .map_err(|_| ())?;
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|length| length > RESPONSE_LIMIT as u64)
        {
            return Err(());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
            if chunk.len() > RESPONSE_LIMIT - bytes.len() {
                return Err(());
            }
            bytes.extend_from_slice(&chunk);
        }
        let ready: ProcessReady = serde_json::from_slice(&bytes).map_err(|_| ())?;
        if ready.api_version != 1 {
            return Err(());
        }
        Ok(ready)
    }
}

#[derive(Deserialize)]
struct ProcessReady {
    api_version: u16,
    ready: bool,
    model_state: Option<String>,
}

struct Observations {
    captured: Instant,
    at: String,
    checks: Vec<ReadinessCheck>,
    natural: Option<classifier_activation::NaturalResourceObservation>,
}

/// One coalesced cache for safe backend observations, independent of user actions.
#[derive(Clone)]
pub struct ReadinessOwners {
    deployment_id: String,
    probes: Vec<ReadinessProbe>,
    cache: Arc<Mutex<Option<Observations>>>,
}

impl ReadinessOwners {
    /// Startup obtains the persisted deployment ID before constructing this map.
    pub fn new(deployment_id: String, probes: Vec<ReadinessProbe>) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !deployment_id.is_empty()
                && deployment_id.len() <= 256
                && !deployment_id.chars().any(char::is_control),
            "invalid readiness deployment identity"
        );
        anyhow::ensure!(probes.len() <= 5, "too many readiness owners");
        for (index, probe) in probes.iter().enumerate() {
            anyhow::ensure!(
                !probes[..index].iter().any(|other| other.id == probe.id),
                "duplicate readiness owner"
            );
        }
        Ok(Self {
            deployment_id,
            probes,
            cache: Arc::new(Mutex::new(None)),
        })
    }
}

fn check(
    id: ReadinessCheckId,
    state: ReadinessState,
    code: &str,
    explanation: &str,
    at: &str,
) -> ReadinessCheck {
    let mut check = ReadinessCheck::new(id, state, code.into(), explanation.into(), at.into());
    check.fresh_for_ms = CACHE_AGE.as_millis() as u64;
    check
}

fn unavailable(id: ReadinessCheckId, at: &str) -> ReadinessCheck {
    check(
        id,
        ReadinessState::Unknown,
        "owner_not_configured",
        "No configured status owner; readiness is unknown",
        at,
    )
}

async fn catalog(state: &ApiState, at: &str) -> ReadinessCheck {
    let result =
        tokio::time::timeout(DEADLINE, setup_jobs::actor_seed_observation(&state.repo)).await;
    match result {
        Ok(Ok(Some(receipt))) if receipt.status=="complete" && receipt.total>0 && receipt.processed==receipt.total => check(ReadinessCheckId::ActorCatalog,ReadinessState::Ready,"database_seeded","A durable completed actor seed receipt exists; actor search visibility is a separate check",at),
        Ok(Ok(Some(receipt))) if receipt.status=="running" => check(ReadinessCheckId::ActorCatalog,ReadinessState::Running,"database_seed_running","Actor seed work is running; completion is not implied",at),
        Ok(Ok(_)) => check(ReadinessCheckId::ActorCatalog,ReadinessState::NotReady,"database_seed_incomplete","No current completed seed receipt; existing partial records are retained",at),
        _ => check(ReadinessCheckId::ActorCatalog,ReadinessState::Unavailable,"seed_receipt_unavailable","Cannot read the durable actor seed receipt",at),
    }
}

fn search_observation(
    status: &ActorIndexStatus,
    source: &actor_search_outbox::Observation,
    deployment: &str,
    at: &str,
) -> ReadinessCheck {
    let (state, code, explanation) = if status.protocol != ACTOR_INDEX_PROTOCOL
        || status.schema_version != ACTOR_INDEX_SCHEMA
        || status.analyzer_version != ACTOR_INDEX_ANALYZER
        || status.deployment_id != deployment
        || status.deployment_id != source.deployment_id
        || status.source_epoch != source.source_epoch
        || source.acknowledged_sequence < 0
        || source.next_sequence < source.acknowledged_sequence
    {
        (
            ReadinessState::Unavailable,
            "index_source_mismatch",
            "Actor index status does not match the current backend source",
        )
    } else if let Some(active) = &status.active {
        if active.generation != source.index_generation
            || active.visible_checkpoint > active.committed_checkpoint
            || active.visible_checkpoint > source.next_sequence as u64
        {
            (
                ReadinessState::Unavailable,
                "index_generation_mismatch",
                "Actor index generation or checkpoint does not match the authoritative source",
            )
        } else if source.blocked
            || source.pressure_paused
            || status
                .pressure
                .as_ref()
                .is_some_and(|pressure| pressure.paused)
        {
            (
                ReadinessState::Degraded,
                "index_ingest_paused",
                "Actor index ingestion is paused; existing reader visibility is retained",
            )
        } else if status.recovery_available || status.state == "degraded" {
            (ReadinessState::Degraded, "index_recovery_required", "The actor index owner reports a recoverable generation error; existing reader visibility is retained")
        } else if status.rebuilding.is_some()
            || !active.source_complete
            || !source.backfill_complete
            || !source.seed_complete
            || active.visible_checkpoint < source.next_sequence as u64
            || source.acknowledged_sequence < source.next_sequence
        {
            (
                ReadinessState::Running,
                "index_catching_up",
                "Actor index work is in progress; current source visibility is not complete",
            )
        } else if status.state == "ready" {
            (ReadinessState::Ready, "index_reader_visible", "The current actor generation is reader-visible through the observed source checkpoint")
        } else {
            (
                ReadinessState::Degraded,
                "index_not_ready",
                "The actor index owner does not report a ready serving generation",
            )
        }
    } else if status.rebuilding.is_some() {
        (
            ReadinessState::Running,
            "index_building",
            "Actor index generation is building; no active generation is reader-visible",
        )
    } else {
        (
            ReadinessState::NotReady,
            "index_not_built",
            "No active actor index generation is available",
        )
    };
    let mut observation = check(ReadinessCheckId::ActorSearch, state, code, explanation, at);
    observation.artifact_ids = status
        .active
        .iter()
        .map(|generation| generation.generation.clone())
        .collect();
    observation
}

async fn search(
    state: &ApiState,
    owners: &ReadinessOwners,
    client: Option<&SearchClient>,
    at: &str,
) -> ReadinessCheck {
    let Some(client) = client else {
        return unavailable(ReadinessCheckId::ActorSearch, at);
    };
    let result = tokio::time::timeout(DEADLINE, async {
        tokio::join!(client.status(), actor_search_outbox::observe(&state.repo))
    })
    .await;
    match result {
        Ok((Ok(status), Ok(Some(source)))) => {
            search_observation(&status, &source, &owners.deployment_id, at)
        }
        Ok((Ok(_), Ok(None))) => check(
            ReadinessCheckId::ActorSearch,
            ReadinessState::NotReady,
            "index_source_not_started",
            "Actor indexing has not established a durable source projection",
            at,
        ),
        _ => check(
            ReadinessCheckId::ActorSearch,
            ReadinessState::Unavailable,
            "index_status_unavailable",
            "Current actor index and source status could not be observed",
            at,
        ),
    }
}

async fn collect(
    state: &ApiState,
    owners: &ReadinessOwners,
    client: Option<&SearchClient>,
) -> Observations {
    let at = Utc::now().to_rfc3339();
    let database = tokio::time::timeout(DEADLINE, async {
        state.repo.db().query("RETURN 1").await?.check()
    })
    .await;
    let mut checks = vec![
        check(
            ReadinessCheckId::Gateway,
            ReadinessState::Ready,
            "gateway_reachable",
            "Authenticated gateway request succeeded",
            &at,
        ),
        if matches!(database, Ok(Ok(_))) {
            check(
                ReadinessCheckId::Datastore,
                ReadinessState::Ready,
                "datastore_reachable",
                "Authoritative datastore answered a bounded read",
                &at,
            )
        } else {
            check(
                ReadinessCheckId::Datastore,
                ReadinessState::Unavailable,
                "datastore_unavailable",
                "Authoritative datastore status is unavailable",
                &at,
            )
        },
    ];
    let (catalog, search, services, natural) = tokio::join!(
        catalog(state, &at),
        search(state, owners, client, &at),
        join_all(owners.probes.iter().map(|probe| probe.observe(&at))),
        tokio::time::timeout(DEADLINE, classifier_activation::observation(&state.repo))
    );
    let natural = natural.ok().and_then(Result::ok);
    checks.push(catalog);
    for service in services {
        checks.extend(service);
    }
    for id in [
        ReadinessCheckId::Brain,
        ReadinessCheckId::Control,
        ReadinessCheckId::Jobs,
        ReadinessCheckId::Nlp,
        ReadinessCheckId::Classifier,
    ] {
        if !checks.iter().any(|check| check.id == id) {
            checks.push(unavailable(id, &at));
        }
    }
    if !checks
        .iter()
        .any(|check| check.id == ReadinessCheckId::ClassifierInference)
    {
        checks.push(unavailable(ReadinessCheckId::ClassifierInference, &at));
    }
    checks.push(search);
    let training = match natural.as_ref() {
        Some(resource) if resource.effect_uncertain => check(
            ReadinessCheckId::ClassifierTraining, ReadinessState::Degraded,
            "natural_activation_reconciliation_required",
            "An activation effect needs reconciliation; training and activation remain blocked", &at,
        ),
        Some(resource) if resource.blocked => check(
            ReadinessCheckId::ClassifierTraining, ReadinessState::Running,
            "natural_resource_busy",
            "The shared Natural resource has unfinished work; no candidate or active-model success is inferred", &at,
        ),
        Some(_) => check(
            ReadinessCheckId::ClassifierTraining, ReadinessState::Unknown,
            "natural_resource_idle",
            "No unfinished Natural resource work is observed; select an authorized training job for candidate evidence", &at,
        ),
        None => unavailable(ReadinessCheckId::ClassifierTraining, &at),
    };
    checks.push(training);
    checks.push(check(
        ReadinessCheckId::Registration,
        ReadinessState::Unknown,
        "registration_not_selected",
        "Connection diagnostics are available before a registered workstation is selected",
        &at,
    ));
    Observations {
        captured: Instant::now(),
        at,
        checks,
        natural,
    }
}

fn action(
    id: ReadinessActionId,
    enabled: bool,
    permitted: bool,
    registered: bool,
) -> ReadinessAction {
    let mut action = ReadinessAction::new(
        id,
        Permission::MaintainBackend,
        enabled && permitted && registered,
    );
    action.preconditions = vec![
        "registered_origin".into(),
        "validated_dataset_or_rebuild_context".into(),
    ];
    action.disabled_reason = if !permitted {
        Some("maintenance_permission_required".into())
    } else if !registered {
        Some("registered_origin_required".into())
    } else if !enabled {
        Some("durable_dispatch_not_configured".into())
    } else {
        None
    };
    action
}

/// Global diagnostics require backend read permission, not an arbitrary folder.
pub(crate) async fn read(
    State(state): State<ApiState>,
    principal: Option<Extension<AuthenticatedPrincipal>>,
    owners: Option<Extension<ReadinessOwners>>,
    search: Option<Extension<SearchClient>>,
    context: Option<Extension<RegisteredContext>>,
) -> Response {
    let Some(Extension(principal)) = principal else {
        return reply(
            StatusCode::UNAUTHORIZED,
            json!({"error":"Purpose-scoped gateway credential required"}),
        );
    };
    if !principal.permits(Permission::ReadBackend) {
        return reply(
            StatusCode::FORBIDDEN,
            json!({"error":"Backend diagnostic permission required"}),
        );
    }
    let Some(Extension(owners)) = owners else {
        return reply(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error":"Backend diagnostic owners are not configured"}),
        );
    };
    let mut cache = owners.cache.lock().await;
    if cache
        .as_ref()
        .is_none_or(|cached| cached.captured.elapsed() >= CACHE_AGE)
    {
        *cache = Some(
            collect(
                &state,
                &owners,
                search.as_ref().map(|Extension(client)| client),
            )
            .await,
        );
    }
    let Some(observations) = cache.as_ref() else {
        return reply(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error":"Backend observations unavailable"}),
        );
    };
    let mut checks = observations.checks.clone();
    let observed_at = observations.at.clone();
    let natural = observations.natural.clone();
    drop(cache);
    let permitted = principal.permits(Permission::MaintainBackend);
    let context = context.map(|Extension(context)| context).filter(|context| {
        context.deployment_id == owners.deployment_id
            && context.access_epoch == principal.access_epoch()
    });
    let registered = context.is_some();
    for (check_id, operation, action_id) in [
        (
            ReadinessCheckId::ActorCatalog,
            MaintenanceOperation::SeedActors,
            ReadinessActionId::SeedActors,
        ),
        (
            ReadinessCheckId::ClassifierTraining,
            MaintenanceOperation::TrainClassifier,
            ReadinessActionId::TrainClassifier,
        ),
        (
            ReadinessCheckId::ActorSearch,
            MaintenanceOperation::ReindexActors,
            ReadinessActionId::RebuildActorIndex,
        ),
    ] {
        let enabled = matches!(
            tokio::time::timeout(
                DEADLINE,
                setup_jobs::operation_enabled(&state.repo, operation)
            )
            .await,
            Ok(Ok(true))
        );
        if let Some(check) = checks.iter_mut().find(|check| check.id == check_id) {
            check
                .actions
                .push(action(action_id, enabled, permitted, registered));
        }
    }
    if let Some(check) = checks
        .iter_mut()
        .find(|check| check.id == ReadinessCheckId::ClassifierTraining)
    {
        let native = principal.purpose()
            == fleetworks_contracts::context::CredentialPurpose::Native
            && principal.permits(Permission::ReadWorkstationActivity);
        let enabled = natural
            .as_ref()
            .is_some_and(|resource| resource.consumer_enabled && !resource.blocked);
        let mut activation = action(
            ReadinessActionId::ActivateClassifier,
            enabled,
            permitted && native,
            registered,
        );
        activation.preconditions = vec![
            "registered_origin".into(),
            "configuration_revision".into(),
            "authorized_completed_training_job".into(),
            "exact_candidate_hashes".into(),
            "expected_prior_activation".into(),
            "explicit_synthetic_baseline_consent".into(),
        ];
        if permitted
            && native
            && registered
            && natural.as_ref().is_some_and(|resource| resource.blocked)
        {
            activation.disabled_reason = Some("natural_resource_busy_or_uncertain".into());
        }
        check.actions.push(activation);
        if natural.as_ref().is_some_and(|resource| resource.blocked) {
            for action in &mut check.actions {
                if action.id == ReadinessActionId::TrainClassifier {
                    action.allowed = false;
                    action.disabled_reason = Some("natural_resource_busy_or_uncertain".into());
                }
            }
        }
    }
    let mut response = WorkstationReadiness::new(owners.deployment_id.clone(), observed_at, checks);
    if let Some(context) = context {
        response.registration = Some(RegistrationReceipt::new(
            context.deployment_id,
            context.workstation_id,
            context.workspace_id,
            context.registration_id,
            context.connect_id,
            context.registration_revision,
        ));
        if let Some(check) = response
            .checks
            .iter_mut()
            .find(|check| check.id == ReadinessCheckId::Registration)
        {
            check.state = ReadinessState::Ready;
            check.reason_code = "registration_active".into();
            check.explanation = "Current registered origin verified at the gateway boundary".into();
        }
    }
    match serde_json::to_value(response) {
        Ok(value) => reply(StatusCode::OK, value),
        Err(_) => reply(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error":"Backend observations unavailable"}),
        ),
    }
}

fn reply(status: StatusCode, value: serde_json::Value) -> Response {
    let mut response = (status, Json(value)).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_catalog_requires_matching_reader_visibility_before_search_is_ready() {
        let status: ActorIndexStatus = serde_json::from_value(json!({
            "protocol":ACTOR_INDEX_PROTOCOL,"deployment_id":"fixture-deployment","source_epoch":"fixture-source",
            "schema_version":ACTOR_INDEX_SCHEMA,"analyzer_version":ACTOR_INDEX_ANALYZER,
            "tantivy_version":"0.27.0","index_format_version":1,
            "active":{"generation":"fixture-generation","committed_checkpoint":10,"visible_checkpoint":10,"source_complete":true,"writer_epoch":1},
            "rebuilding":null,"retained_generations":0,"state":"ready","reason_code":null
        })).expect("synthetic owner observation");
        let mut source = actor_search_outbox::Observation {
            deployment_id: "fixture-deployment".into(),
            source_epoch: "fixture-source".into(),
            index_generation: "fixture-generation".into(),
            next_sequence: 10,
            acknowledged_sequence: 10,
            blocked: false,
            backfill_complete: true,
            seed_complete: true,
            pressure_paused: false,
            pressure_reason: String::new(),
            backlog_paused: false,
        };
        let observe = |status: &ActorIndexStatus, source: &actor_search_outbox::Observation| {
            search_observation(status, source, "fixture-deployment", "2026-01-01T00:00:00Z")
        };
        assert_eq!(observe(&status, &source).state, ReadinessState::Ready);
        source.seed_complete = false;
        assert_eq!(observe(&status, &source).state, ReadinessState::Running);
        source.seed_complete = true;
        let mut lagged = status.clone();
        lagged
            .active
            .as_mut()
            .expect("fixture reader")
            .visible_checkpoint = 9;
        assert_eq!(observe(&lagged, &source).state, ReadinessState::Running);
        source.blocked = true;
        assert_eq!(observe(&status, &source).state, ReadinessState::Degraded);
        source.blocked = false;
        let mut recovery = status.clone();
        recovery.recovery_available = true;
        recovery.state = "degraded".into();
        assert_eq!(observe(&recovery, &source).state, ReadinessState::Degraded);
        source.source_epoch = "old-source".into();
        assert_eq!(observe(&status, &source).state, ReadinessState::Unavailable);
        source.source_epoch = "fixture-source".into();
        let mut absent = status;
        absent.active = None;
        assert_eq!(observe(&absent, &source).state, ReadinessState::NotReady);
    }

    #[test]
    fn private_check_urls_are_fixed_and_unknown_owners_are_refused() {
        let token = "synthetic-readiness-token-000000000000000";
        let probe = ReadinessProbe::new(
            ReadinessCheckId::Classifier,
            "http://classifier:8080",
            token,
        )
        .expect("configured classifier");
        assert_eq!(probe.url.path(), "/api/v1/health/ready");
        assert!(ReadinessProbe::new(
            ReadinessCheckId::Classifier,
            "http://classifier:8080/arbitrary",
            token
        )
        .is_err());
        assert!(ReadinessProbe::new(
            ReadinessCheckId::Registration,
            "http://classifier:8080",
            token
        )
        .is_err());
    }

    #[test]
    fn missing_or_unseeded_state_does_not_disable_an_available_repair_permission() {
        let action = action(ReadinessActionId::SeedActors, true, true, true);
        assert!(action.allowed);
        assert!(!super::action(ReadinessActionId::TrainClassifier, false, true, true).allowed);
        assert!(!super::action(ReadinessActionId::SeedActors, true, false, true).allowed);
        assert!(!super::action(ReadinessActionId::SeedActors, true, true, false).allowed);
        assert_eq!(
            unavailable(ReadinessCheckId::ActorSearch, "fixture").state,
            ReadinessState::Unknown
        );
    }
}
