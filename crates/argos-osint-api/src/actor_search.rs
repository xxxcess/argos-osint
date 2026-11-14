//! Discovery transport and current canonical filtering at the authenticated gateway.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::time::Duration;

use argos_osint_core::storage::actor_search_outbox;
use axum::http::header;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use fleetworks_contracts::actor_index::ActorIndexQuery;
use fleetworks_contracts::actor_index::ActorIndexQueryResponse;
use fleetworks_contracts::actor_index::ActorIndexStatus;
use fleetworks_contracts::actor_index::ACTOR_INDEX_ANALYZER;
use fleetworks_contracts::actor_index::ACTOR_INDEX_SCHEMA;
use fleetworks_contracts::actor_index::MAX_QUERY_RESULTS;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::search::ActorSearchHit;
use fleetworks_contracts::search::ActorSearchResponse;
use fleetworks_contracts::search::IndexState;
use fleetworks_contracts::search::IndexStatus;
use serde::de::DeserializeOwned;
use url::Url;

use crate::ApiError;
use crate::ApiState;
use crate::AuthenticatedPrincipal;

const MAX_RESPONSE: usize = 1024 * 1024;

/// An explicitly configured, pooled private reader. Public request fields never
/// select its origin, credential, filesystem path or authority.
#[derive(Clone)]
pub struct SearchClient {
    origin: Url,
    authorization: HeaderValue,
    http: reqwest::Client,
}

impl SearchClient {
    pub fn new(origin: &str, token: &str) -> anyhow::Result<Self> {
        let origin = crate::network::validate_origin(origin)?;
        anyhow::ensure!(
            crate::network::valid_token(token.as_bytes()),
            "invalid search reader credential"
        );
        let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))?;
        authorization.set_sensitive(true);
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(8))
            .build()?;
        Ok(Self {
            origin,
            authorization,
            http,
        })
    }

    fn endpoint(&self, path: &str) -> Url {
        let mut endpoint = self.origin.clone();
        endpoint.set_path(path);
        endpoint
    }

    /// Probe configured private status without triggering an ingest or rebuild.
    pub async fn status(&self) -> anyhow::Result<ActorIndexStatus> {
        let response = self
            .http
            .get(self.endpoint("/internal/v1/actors/status"))
            .header(header::AUTHORIZATION, self.authorization.clone())
            .send()
            .await?;
        bounded_json(response).await
    }

    async fn query(&self, query: &ActorIndexQuery) -> anyhow::Result<ActorIndexQueryResponse> {
        query.validate()?;
        let response = self
            .http
            .post(self.endpoint("/internal/v1/actors/query"))
            .header(header::AUTHORIZATION, self.authorization.clone())
            .json(query)
            .send()
            .await?;
        bounded_json(response).await
    }
}

async fn bounded_json<T: DeserializeOwned + Send + 'static>(
    mut response: reqwest::Response,
) -> anyhow::Result<T> {
    anyhow::ensure!(response.status().is_success(), "search service unavailable");
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            chunk.len() <= MAX_RESPONSE - bytes.len(),
            "search response exceeds limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(tokio::task::spawn_blocking(move || serde_json::from_slice(&bytes)).await??)
}

fn empty(state: IndexState, reason: &str) -> ActorSearchResponse {
    let mut index = IndexStatus::unknown(chrono::Utc::now().to_rfc3339());
    index.state = state;
    index.reason_code = Some(reason.into());
    ActorSearchResponse::new(Vec::new(), index)
}

pub(crate) async fn discover(
    state: &ApiState,
    client: Option<&SearchClient>,
    principal: Option<&AuthenticatedPrincipal>,
    query: &str,
    limit: usize,
) -> Result<ActorSearchResponse, ApiError> {
    if !principal.is_some_and(|principal| principal.permits(Permission::ReadBackend)) {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Backend read permission required",
        ));
    }
    let request = ActorIndexQuery {
        query: query.to_owned(),
        limit: MAX_QUERY_RESULTS,
    };
    request.validate().map_err(|_| crate::bad())?;
    let Some(client) = client else {
        return Ok(empty(IndexState::NotBuilt, "actor_search_not_configured"));
    };
    let observation = actor_search_outbox::observe(&state.repo)
        .await
        .map_err(crate::internal)?;
    let Some(observation) = observation else {
        return Ok(empty(IndexState::NotBuilt, "actor_projection_not_started"));
    };
    let response = match client.query(&request).await {
        Ok(response) => response,
        Err(_) => return Ok(empty(IndexState::Unavailable, "actor_search_unavailable")),
    };
    if response.deployment_id != observation.deployment_id
        || response.source_epoch != observation.source_epoch
        || response.index_generation != observation.index_generation
        || response.hits.len() > usize::from(MAX_QUERY_RESULTS)
        || observation.acknowledged_sequence < 0
        || observation.next_sequence < observation.acknowledged_sequence
    {
        return Ok(empty(
            IndexState::Unavailable,
            "actor_search_source_mismatch",
        ));
    }
    // Read current canonical rows before copying *any* private label/snippet.
    // A stale alias/scope/tombstone cannot pass solely because the index said
    // it was eligible. One bounded DB batch replaces per-hit HTTP/DB fanout.
    let ids: Vec<String> = response
        .hits
        .iter()
        .map(|hit| hit.actor_id.clone())
        .collect();
    let canonical = actor_search_outbox::canonical_many(&state.repo, &ids)
        .await
        .map_err(crate::internal)?;
    let canonical: BTreeMap<_, _> = canonical
        .into_iter()
        .map(|row| (row.actor_id.clone(), row))
        .collect();
    Ok(project(response, canonical, &observation, limit))
}

fn project(
    response: ActorIndexQueryResponse,
    canonical: BTreeMap<String, actor_search_outbox::CanonicalProjection>,
    observation: &actor_search_outbox::Observation,
    limit: usize,
) -> ActorSearchResponse {
    let mut seen = BTreeSet::new();
    let mut stale = false;
    let mut rows = Vec::new();
    for hit in response.hits {
        let Some(current) = canonical.get(&hit.actor_id) else {
            stale = true;
            continue;
        };
        let Some(projection) = current.projection.as_ref() else {
            stale = true;
            continue;
        };
        if !seen.insert(hit.actor_id.clone())
            || current.revision != hit.revision
            || *projection != hit.projection
            || !projection.eligible
            || !projection
                .authority_scopes
                .iter()
                .any(|scope| scope == "backend")
            || !hit.score.is_finite()
        {
            stale = true;
            continue;
        }
        let mut row = ActorSearchHit::new(
            hit.actor_id,
            projection.display_name.clone(),
            current.revision,
            response.index_generation.clone(),
        );
        row.summary = projection.summary.clone();
        row.actor_type = projection.actor_type.clone();
        row.countries = projection.countries.clone();
        row.matched_field = matches!(
            hit.matched_field.as_str(),
            "name_exact" | "alias_exact" | "name_or_alias" | "prefix" | "description"
        )
        .then_some(hit.matched_field);
        row.score = hit.score;
        rows.push(row);
    }
    let mut index = IndexStatus::unknown(chrono::Utc::now().to_rfc3339());
    let pending = (observation.next_sequence as u64).saturating_sub(response.visible_checkpoint);
    index.state =
        if stale || response.serving_state == "stale" || response.serving_state == "rebuilding" {
            IndexState::Stale
        } else if observation.blocked {
            IndexState::Paused
        } else if !response.source_complete
            || !observation.backfill_complete
            || !observation.seed_complete
            || pending > 0
        {
            IndexState::Indexing
        } else if response.serving_state == "ready" {
            IndexState::Ready
        } else {
            IndexState::Degraded
        };
    index.generation = Some(response.index_generation);
    index.source_generation = Some(response.source_epoch);
    index.schema_version = ACTOR_INDEX_SCHEMA;
    index.analyzer_version = ACTOR_INDEX_ANALYZER;
    index.visible_checkpoint = Some(response.visible_checkpoint);
    index.pending_work = Some(pending);
    if stale {
        index.reason_code = Some("actor_candidates_changed_since_index".into());
    }
    let truncated = response.truncated || stale || rows.len() > limit;
    rows.truncate(limit);
    let mut result = ActorSearchResponse::new(rows, index);
    result.truncated = truncated;
    result
}

#[cfg(test)]
mod tests {
    use fleetworks_contracts::actor_index::ActorIndexHit;
    use fleetworks_contracts::actor_index::ActorProjection;

    use super::*;

    fn projection() -> ActorProjection {
        ActorProjection {
            display_name: "Synthetic 港 Union".into(),
            aliases: Vec::new(),
            summary: Some("Synthetic dossier summary".into()),
            actor_type: Some("organization".into()),
            countries: vec!["ZZ".into()],
            source_id: Some("fixture:union".into()),
            authority_scopes: vec!["backend".into()],
            eligible: true,
        }
    }

    fn response() -> ActorIndexQueryResponse {
        ActorIndexQueryResponse {
            deployment_id: "deployment".into(),
            source_epoch: "source".into(),
            index_generation: "g1".into(),
            visible_checkpoint: 10,
            source_complete: true,
            serving_state: "ready".into(),
            hits: vec![ActorIndexHit {
                actor_id: "union".into(),
                revision: 3,
                projection: projection(),
                matched_field: "alias_exact".into(),
                score: 12.0,
            }],
            truncated: false,
        }
    }

    fn observation() -> actor_search_outbox::Observation {
        actor_search_outbox::Observation {
            deployment_id: "deployment".into(),
            source_epoch: "source".into(),
            index_generation: "g1".into(),
            next_sequence: 10,
            acknowledged_sequence: 10,
            blocked: false,
            backfill_complete: true,
            seed_complete: true,
            pressure_paused: false,
            pressure_reason: String::new(),
            backlog_paused: false,
        }
    }

    #[test]
    fn only_matching_current_canonical_projection_can_supply_a_label() {
        let canonical = BTreeMap::from([(
            "union".into(),
            actor_search_outbox::CanonicalProjection {
                actor_id: "union".into(),
                revision: 3,
                projection: Some(projection()),
            },
        )]);
        let result = project(response(), canonical.clone(), &observation(), 10);
        assert_eq!(result.rows[0].label, "Synthetic 港 Union");
        assert_eq!(result.rows[0].matched_field.as_deref(), Some("alias_exact"));
        assert_eq!(result.index.state, IndexState::Ready);
        let mut forged = response();
        forged.hits[0].projection.display_name = "stale private label".into();
        let filtered = project(forged, canonical, &observation(), 10);
        assert!(filtered.rows.is_empty());
        assert_eq!(filtered.index.state, IndexState::Stale);
        assert!(
            filtered.truncated,
            "an authority-filtered page must not claim completeness"
        );
    }

    #[test]
    fn retirement_or_scope_change_excludes_private_hits_before_projection() {
        for projection in [
            None,
            Some(ActorProjection {
                authority_scopes: vec!["other-scope".into()],
                ..projection()
            }),
        ] {
            let canonical = BTreeMap::from([(
                "union".into(),
                actor_search_outbox::CanonicalProjection {
                    actor_id: "union".into(),
                    revision: 4,
                    projection,
                },
            )]);
            let result = project(response(), canonical, &observation(), 10);
            assert!(result.rows.is_empty());
            assert_eq!(result.index.state, IndexState::Stale);
        }
    }
}
