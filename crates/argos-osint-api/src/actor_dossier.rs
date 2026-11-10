//! Retained actor discovery and dossier HTTP reads. @tag:intel-brief
use std::collections::BTreeSet;

use argos_osint_core::storage::actor_catalog::{self, ActorAliasSearch, ActorNameSearch};
use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{ApiError, ApiResult, ApiState};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActorQuery {
    id: String,
}

pub(crate) async fn read(
    State(state): State<ApiState>,
    Query(query): Query<ActorQuery>,
) -> ApiResult {
    if !valid_actor_id(&query.id) {
        return Err(crate::bad());
    }
    let dossier = argos_osint_core::storage::actor_dossier::read(&state.repo, &query.id)
        .await
        .map_err(crate::internal)?
        .ok_or_else(crate::missing)?;
    Ok(Json(
        serde_json::to_value(dossier).map_err(crate::internal)?,
    ))
}

fn valid_actor_id(id: &str) -> bool {
    !id.trim().is_empty() && id.len() <= 256 && !id.chars().any(char::is_control)
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SearchMode {
    #[default]
    Name,
    Alias,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActorSearchQuery {
    query: String,
    #[serde(default)]
    mode: SearchMode,
    limit: Option<usize>,
}

impl ActorSearchQuery {
    fn validate(&self) -> Result<(&str, usize), ApiError> {
        let limit = self.limit.unwrap_or(10);
        if !valid_actor_id(&self.query) || !actor_catalog::ACTOR_SEARCH_LIMIT.contains(&limit) {
            return Err(crate::bad());
        }
        Ok((self.query.trim(), limit))
    }
}

pub(crate) async fn search(
    State(state): State<ApiState>,
    Query(request): Query<ActorSearchQuery>,
) -> ApiResult {
    let (query, limit) = request.validate()?;
    // A small display limit must not turn a shared alias into a unique match.
    // Resolve the bounded candidate window before applying the display limit.
    let window = *actor_catalog::ACTOR_SEARCH_LIMIT.end();
    let response = match request.mode {
        SearchMode::Name => {
            let result = actor_catalog::search_actor_names(&state.repo, query, window)
                .await
                .map_err(crate::internal)?;
            project_names(result, limit)
        }
        SearchMode::Alias => {
            let result = actor_catalog::search_actor_aliases(&state.repo, query, window)
                .await
                .map_err(crate::internal)?;
            project_aliases(result, limit)
        }
    };
    Ok(Json(response))
}

fn source_id(value: &str) -> Option<&str> {
    valid_actor_id(value).then_some(value)
}

fn project_names(search: ActorNameSearch, limit: usize) -> Value {
    let rows = search
        .matches
        .iter()
        .map(|hit| (hit, false))
        .chain(search.collisions.iter().map(|hit| (hit, true)))
        .filter(|(hit, _)| valid_actor_id(&hit.actor_id));
    let rows: Vec<_> = rows
        .take(limit + 1)
        .map(|(hit, ambiguous)| {
            json!({
                "actor_id":hit.actor_id,
                "label":hit.display_name,
                "subtype":hit.subtype,
                "kind":"exact_name",
                "reason":hit.reason,
                "ambiguous":ambiguous,
                "actor_revision":hit.revision,
                "catalog_id":source_id(&hit.catalog_id),
            })
        })
        .collect();
    search_response(rows, search.diagnostic, &search.revision, limit)
}

fn project_aliases(search: ActorAliasSearch, limit: usize) -> Value {
    let mut seen = BTreeSet::new();
    let mut rows = Vec::new();
    'groups: for (kind, hits) in [
        ("exact_alias", &search.exact),
        ("prefix_alias", &search.prefix),
    ] {
        for hit in hits {
            for id in &hit.owners {
                if !valid_actor_id(id) || !seen.insert((hit.normalized.as_str(), id.as_str())) {
                    continue;
                }
                // The indexed alias row is the source of this provenance. In
                // a collision it does not identify which owner supplied it.
                let provenance = |key: &str| hit.provenance[key].as_str().and_then(source_id);
                rows.push(json!({
                    "actor_id":id,
                    "label":hit.alias,
                    "kind":kind,
                    "reason":hit.strength,
                    "ambiguous":hit.ambiguous,
                    "alias_provenance":{
                        "catalog_id":provenance("actor_id"),
                        "source_id":provenance("source_id"),
                        "curated":hit.provenance["curated"].as_bool().unwrap_or(false),
                    },
                }));
                if rows.len() > limit {
                    break 'groups;
                }
            }
        }
    }
    search_response(rows, search.diagnostic, &search.revision, limit)
}

fn search_response(mut rows: Vec<Value>, diagnostic: &str, revision: &str, limit: usize) -> Value {
    let truncated = rows.len() > limit;
    rows.truncate(limit);
    json!({"rows":rows,"diagnostic":diagnostic,"catalog_revision":revision,"truncated":truncated})
}

#[cfg(test)]
mod tests {
    use argos_osint_core::storage::actor_catalog::{ActorAliasHit, ActorNameHit};

    use super::*;

    #[test]
    fn actor_search_bounds_query_and_limit() {
        for (query, limit) in [
            ("   ".to_owned(), 10),
            ("a\nb".to_owned(), 10),
            ("é".repeat(129), 10),
            ("Harbor Union".to_owned(), 0),
            ("Harbor Union".to_owned(), 26),
        ] {
            let request = ActorSearchQuery {
                query,
                mode: SearchMode::Alias,
                limit: Some(limit),
            };
            assert!(request.validate().is_err());
        }
        let request = ActorSearchQuery {
            query: " Harbor Union ".into(),
            mode: SearchMode::Name,
            limit: None,
        };
        assert!(matches!(request.validate(), Ok(("Harbor Union", 10))));
    }

    #[test]
    fn a_one_row_limit_keeps_a_name_collision_ambiguous() {
        let hit = |actor: &str| ActorNameHit {
            actor_id: actor.into(),
            catalog_id: format!("fixture:{actor}"),
            display_name: "Harbor Union".into(),
            subtype: "organization".into(),
            revision: 1,
            reason: "exact name",
        };
        let result = project_names(
            ActorNameSearch {
                matches: Vec::new(),
                collisions: vec![hit("actor-a"), hit("actor-b")],
                diagnostic: "",
                revision: "catalog-a".into(),
            },
            1,
        );
        assert_eq!(result["rows"].as_array().unwrap().len(), 1);
        assert_eq!(result["rows"][0]["actor_id"], "actor-a");
        assert_eq!(result["rows"][0]["ambiguous"], true);
        assert_eq!(result["truncated"], true);
        assert_eq!(result["catalog_revision"], "catalog-a");
    }

    #[test]
    fn alias_projection_deduplicates_and_only_exposes_bounded_provenance_fields() {
        let hit = ActorAliasHit {
            alias: "Harbor Union".into(),
            normalized: "harbor union".into(),
            alias_type: "name".into(),
            strength: "catalog",
            provenance: json!({
                "actor_id":"fixture:one",
                "source_id":"not\nvalid",
                "provider_key":"must-not-escape",
            }),
            owners: vec!["actor-1".into(), "actor-2".into()],
            ambiguous: true,
            revision: 1,
        };
        let result = project_aliases(
            ActorAliasSearch {
                exact: vec![hit.clone()],
                prefix: vec![hit],
                diagnostic: "",
                revision: "catalog-b".into(),
            },
            10,
        );
        let rows = result["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["kind"], "exact_alias");
        assert_eq!(rows[0]["ambiguous"], true);
        assert_eq!(rows[0]["alias_provenance"]["catalog_id"], "fixture:one");
        assert_eq!(rows[0]["alias_provenance"]["source_id"], Value::Null);
        assert!(!result.to_string().contains("must-not-escape"));
    }
}
