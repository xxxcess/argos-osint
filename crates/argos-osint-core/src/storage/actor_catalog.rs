//! Durable catalog ingestion and identity lookup. @tag:brain-memory
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use surrealdb::engine::any::Any;
use surrealdb::types::{RecordId, SurrealValue};
use surrealdb::Surreal;

use super::jobs::{rid, run, take};
use super::repo::Repository;
use super::tx::{run_tx, RetryPolicy, Tx, TxError};
use crate::actor_catalog::{self, Actor, Matcher};
use crate::brain_actors;
use crate::memory_metadata::{ActorPayload, IdentityState, MemoryPayload};

#[path = "actor_seed_execution.rs"]
mod execution;
pub use execution::SeedExecution;

pub const SCHEMA: &str = "
DEFINE FIELD IF NOT EXISTS catalog_json ON brain_actor TYPE option<string>;
DEFINE FIELD IF NOT EXISTS catalog_id ON brain_actor TYPE option<string>;
DEFINE INDEX IF NOT EXISTS brain_actor_catalog ON brain_actor FIELDS catalog_id;
DEFINE TABLE IF NOT EXISTS brain_actor_seed SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS status ON brain_actor_seed TYPE string;
DEFINE FIELD IF NOT EXISTS fingerprint ON brain_actor_seed TYPE string;
DEFINE FIELD IF NOT EXISTS processed ON brain_actor_seed TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS total ON brain_actor_seed TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS aliases ON brain_actor_seed TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS owner ON brain_actor_seed TYPE string;
DEFINE FIELD IF NOT EXISTS lease_until ON brain_actor_seed TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON brain_actor_seed TYPE string;
";
const BATCH: usize = 128;
/// Conservative retained transaction payload per streamed seed batch.
pub const MAX_SEED_BATCH_BYTES: u64 = 2 * 1024 * 1024;
/// Bound expanded persisted rows retained by one streamed transaction.
pub const MAX_SEED_TRANSACTION_BYTES: u64 = 16 * 1024 * 1024;

/// Conservative expanded row bytes, including per-alias source provenance.
pub fn seed_actor_transaction_bytes(actor: &Actor) -> Result<u64> {
    let provenance = serde_json::to_vec(&actor.metadata)?.len() as u64;
    let mut bytes = actor_catalog::actor_seed_payload_bytes(actor)?;
    let mut keys = std::collections::BTreeSet::new();
    for alias in std::iter::once(&actor.name).chain(&actor.aliases) {
        let normalized = brain_actors::normalize_alias(alias);
        if normalized.chars().count() >= 3 && keys.insert(normalized) {
            bytes = bytes
                .checked_add(
                    provenance
                        .checked_mul(2)
                        .context("actor_seed_batch_budget")?,
                )
                .context("actor_seed_batch_budget")?;
        }
    }
    Ok(bytes)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Status {
    pub needs_seed: bool,
    pub actors: usize,
    pub complete: bool,
}

/// Missing/incomplete receipt or an empty canonical actor table requires seed.
pub async fn status(db: &Surreal<Any>) -> Result<Status> {
    let mut r = db.query("RETURN (SELECT VALUE status FROM ONLY brain_actor_seed:catalog) ?? ''; RETURN count(SELECT id FROM brain_actor WHERE scope_key = $scope);")
        .bind(("scope", actor_catalog::SCOPE)).await?.check()?;
    let state: Option<String> = r.take(0)?;
    let count: Option<usize> = r.take(1)?;
    let actors = count.unwrap_or(0);
    let complete = state.as_deref() == Some("complete");
    Ok(Status {
        needs_seed: !complete || actors == 0,
        actors,
        complete,
    })
}

/// Catalog mode begins with the first seed attempt and never falls back to
/// creating article-scoped actors after an interrupted/failed seed.
pub async fn enabled_in(tx: &Tx) -> Result<bool, TxError> {
    let mut r = run(
        tx,
        tx.query("RETURN record::exists(brain_actor_seed:catalog)"),
    )
    .await?;
    Ok(take::<Option<bool>>(&mut r, 0)?.unwrap_or(false))
}

/// A unique strong alias can resolve to a catalog identity. Shared aliases,
/// weak aliases, retired records and unknown names do not choose an actor.
pub async fn lookup_in(tx: &Tx, name: &str) -> Result<Option<String>, TxError> {
    let mut r = run(tx, tx.query("SELECT VALUE actor_id FROM brain_actor_alias WHERE normalized = $name AND alias_type = 'name' AND type::record('brain_actor', actor_id).scope_key = $scope AND type::record('memory', actor_id).lifecycle = 'active' LIMIT 2")
        .bind(("name", brain_actors::normalize_alias(name)))
        .bind(("scope", actor_catalog::SCOPE))).await?;
    let mut ids: Vec<String> = take(&mut r, 0)?;
    // Existing seeded homes predate curated short forms. Read their source ID
    // directly, retaining ambiguity rejection if another actor owns the alias.
    if brain_actors::normalize_alias(name) == "trump" {
        let mut r = run(tx, tx.query("SELECT VALUE record::id(id) FROM brain_actor WHERE catalog_id = $source AND scope_key = $scope AND type::record('memory', record::id(id)).lifecycle = 'active' LIMIT 2")
            .bind(("source", "os:Q22686"))
            .bind(("scope", actor_catalog::SCOPE))).await?;
        ids.extend(take::<Vec<String>>(&mut r, 0)?);
        ids.sort();
        ids.dedup();
    }
    Ok(match ids.as_slice() {
        [id] => Some(id.clone()),
        _ => None,
    })
}

pub(crate) async fn load_matcher(repo: &Repository) -> Result<Matcher> {
    let mut r = repo
        .db()
        .query("SELECT VALUE catalog_json FROM brain_actor WHERE scope_key = $scope AND type::record('memory', record::id(id)).lifecycle = 'active' ORDER BY id")
        .bind(("scope", actor_catalog::SCOPE))
        .await?
        .check()?;
    let rows: Vec<String> = r.take(0)?;
    let actors: Vec<Actor> = tokio::task::spawn_blocking(move || {
        rows.into_iter()
            .map(|s| serde_json::from_str(&s))
            .collect::<std::result::Result<Vec<Actor>, _>>()
    })
    .await
    .context("actor catalog parse worker")??;
    backfill_curated_aliases(repo, &actors).await?;
    tokio::task::spawn_blocking(move || Matcher::new(actors)).await?
}

/// Homes seeded before a curated short form existed lack its
/// `brain_actor_alias` row, so binding lookups cannot resolve it. Backfill is
/// idempotent and provenance-stamped: the desired rows are known from the
/// loaded catalog, existence is checked by record id, and only missing rows
/// are written in one transaction. Runs with the matcher rebuild (coalesced);
/// the matcher itself is built from code-side curated forms and needs no
/// rebuild for this.
async fn backfill_curated_aliases(repo: &Repository, actors: &[Actor]) -> Result<usize> {
    let mut desired: Vec<(surrealdb::types::RecordId, String, String, String, String)> = Vec::new();
    for actor in actors {
        for curated in actor_catalog::curated_aliases(&actor.source_id) {
            let normalized = brain_actors::normalize_alias(curated);
            if !actor_catalog::matchable_alias(&normalized)
                || actor
                    .aliases
                    .iter()
                    .any(|known| brain_actors::normalize_alias(known) == normalized)
            {
                continue;
            }
            desired.push((
                surrealdb::types::RecordId::new(
                    "brain_actor_alias",
                    surrealdb::types::Array::from(vec![
                        actor.id.clone(),
                        normalized.clone(),
                        "name".to_owned(),
                    ]),
                ),
                actor.id.clone(),
                (*curated).to_owned(),
                normalized,
                actor.source_id.clone(),
            ));
        }
    }
    if desired.is_empty() {
        return Ok(0);
    }
    let mut missing = Vec::new();
    for (id, ..) in &desired {
        let mut r = repo
            .db()
            .query("RETURN record::exists($r)")
            .bind(("r", id.clone()))
            .await?
            .check()?;
        if r.take::<Option<bool>>(0)? != Some(true) {
            missing.push(true);
        } else {
            missing.push(false);
        }
    }
    let pending: Vec<_> = desired
        .into_iter()
        .zip(missing)
        .filter(|(_, was_missing)| *was_missing)
        .collect();
    if pending.is_empty() {
        return Ok(0);
    }
    run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let pending = pending.clone();
        async move {
            let actor_ids: Vec<String> = pending
                .iter()
                .map(|((_, actor_id, _, _, _), _)| actor_id.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            for ((id, actor_id, alias, normalized, source_id), _) in pending {
                run(
                    &tx,
                    tx.query(
                        "IF !record::exists($r) { CREATE $r SET actor_id = $a, \
                          alias = $alias, normalized = $n, alias_type = 'name', \
                          provenance_json = $p RETURN NONE; };",
                    )
                    .bind(("r", id))
                    .bind(("a", actor_id))
                    .bind(("alias", alias))
                    .bind(("n", normalized))
                    .bind((
                        "p",
                        serde_json::json!({"curated": true, "source_id": source_id}).to_string(),
                    )),
                )
                .await?;
            }
            for ids in actor_ids.chunks(500) {
                super::actor_search_outbox::enqueue_actors_in(&tx, ids, None, chrono::Utc::now())
                    .await?;
            }
            Ok(())
        }
    })
    .await?;
    Ok(pending.len())
}

// ---- Brain discovery: search_actor_names / search_actor_aliases ----------
//
// The two indexed actor services (spec Phase 4). Both read only the alias
// index (`brain_actor_alias_normalized`), the catalog-id index
// (`brain_actor_catalog`) and point reads — never a catalog scan — and both
// report catalog readiness honestly: an unseeded catalog is `unavailable`,
// not an empty match. Composition with the shared matcher and article
// discovery happens in Recon's `discover_catalog_actors`/articles seams.

/// Minimum and maximum results one service call may return.
pub const ACTOR_SEARCH_LIMIT: std::ops::RangeInclusive<usize> = 1..=25;

/// One active canonical actor a name resolved to.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ActorNameHit {
    /// Canonical actor id (the memory id), redirect-resolved.
    pub actor_id: String,
    /// The catalog source identifier (e.g. `os:Q22686`), empty for
    /// non-catalog actors.
    pub catalog_id: String,
    pub display_name: String,
    pub subtype: String,
    /// The actor row's live revision.
    pub revision: i64,
    /// `exact name` | `normalized form` | `curated form` | `name alias`.
    pub reason: &'static str,
}

/// Result of [`search_actor_names`]. `matches` are unambiguous (at most one
/// canonical actor owns the name); `collisions` lists every active owner when
/// the name is shared — collisions stay ambiguous regardless of any ranking.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ActorNameSearch {
    pub matches: Vec<ActorNameHit>,
    pub collisions: Vec<ActorNameHit>,
    /// `""` | `catalog unavailable` | `query too short` | `unsupported type`.
    pub diagnostic: &'static str,
    /// Catalog fingerprint the search ran against (`''` when unavailable).
    pub revision: String,
}

impl ActorNameSearch {
    /// Unavailable is not an empty success: callers surface the diagnostic.
    pub fn unavailable(diagnostic: &'static str) -> Self {
        Self {
            matches: Vec::new(),
            collisions: Vec::new(),
            diagnostic,
            revision: String::new(),
        }
    }
}

/// Identifiers the name service refuses: IPs, URLs, CVE and AS numbers are
/// lookup keys, not actor names. Mirrors the Recon extraction shapes.
pub fn unsupported_name_reason(name: &str) -> Option<&'static str> {
    let trimmed = name.trim();
    if trimmed.parse::<std::net::IpAddr>().is_ok() {
        return Some("unsupported type");
    }
    if url::Url::parse(trimmed).is_ok_and(|u| matches!(u.scheme(), "http" | "https")) {
        return Some("unsupported type");
    }
    let upper = trimmed.to_ascii_uppercase();
    if upper.starts_with("CVE-")
        || (upper.starts_with("AS")
            && upper[2..].chars().all(|c| c.is_ascii_digit())
            && upper.len() > 2)
    {
        return Some("unsupported type");
    }
    None
}

/// Why a matched alias row earns its reason, most specific first.
fn name_reason(normalized: &str, display_name: &str, catalog: Option<&Actor>) -> &'static str {
    if brain_actors::normalize_alias(display_name) == normalized {
        "exact name"
    } else if catalog.is_some_and(|actor| {
        actor
            .metadata
            .get("name_normalized")
            .is_some_and(|form| brain_actors::normalize_alias(form) == normalized)
    }) {
        "normalized form"
    } else if catalog.is_some_and(|actor| {
        crate::actor_catalog::curated_aliases(&actor.source_id)
            .iter()
            .any(|alias| brain_actors::normalize_alias(alias) == normalized)
    }) {
        "curated form"
    } else {
        "name alias"
    }
}

/// Catalog fingerprint, or `''` before the first seed.
async fn catalog_revision(db: &Surreal<Any>) -> Result<String> {
    let mut r = db
        .query("SELECT VALUE fingerprint FROM ONLY brain_actor_seed:catalog")
        .await?
        .check()?;
    Ok(r.take::<Option<String>>(0)?.unwrap_or_default())
}

#[derive(SurrealValue)]
struct ActorHead {
    display_name: String,
    subtype: String,
    revision: i64,
    catalog_id: String,
    catalog_json: Option<String>,
}

/// `search_actor_names`: indexed exact resolution of one name over active
/// canonical catalog identities. The candidate set comes from the
/// `brain_actor_alias_normalized` index (plus the curated reverse lookup for
/// homes the backfill has not covered yet); owners are redirect-resolved and
/// filtered to active lifecycles.
pub async fn search_actor_names(
    repo: &Repository,
    name: &str,
    limit: usize,
) -> Result<ActorNameSearch> {
    if let Some(diagnostic) = unsupported_name_reason(name) {
        return Ok(ActorNameSearch::unavailable(diagnostic));
    }
    let normalized = brain_actors::normalize_alias(name);
    if !crate::actor_catalog::matchable_alias(&normalized) {
        return Ok(ActorNameSearch::unavailable("query too short"));
    }
    if status(repo.db()).await?.needs_seed {
        return Ok(ActorNameSearch::unavailable("catalog unavailable"));
    }
    let revision = catalog_revision(repo.db()).await?;
    let limit = limit.clamp(*ACTOR_SEARCH_LIMIT.start(), *ACTOR_SEARCH_LIMIT.end());
    let owned = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let normalized = normalized.clone();
        async move {
            let mut r = run(
                &tx,
                tx.query("SELECT VALUE actor_id FROM brain_actor_alias WHERE normalized = $n AND alias_type = 'name' LIMIT $limit")
                    .bind(("n", normalized.clone()))
                    .bind(("limit", limit as i64)),
            )
            .await?;
            let mut ids: Vec<String> = take(&mut r, 0)?;
            // Curated short forms normalize to a code-known source id; resolve
            // them through the catalog-id index for homes the alias backfill
            // has not covered yet (same fallback as `lookup_in`).
            for source in crate::actor_catalog::curated_sources_for(&normalized) {
                let mut r = run(
                    &tx,
                    tx.query("SELECT VALUE record::id(id) FROM brain_actor WHERE catalog_id = $source AND scope_key = $scope AND type::record('memory', record::id(id)).lifecycle = 'active' LIMIT 2")
                        .bind(("source", source.to_string()))
                        .bind(("scope", actor_catalog::SCOPE)),
                )
                .await?;
                ids.extend(take::<Vec<String>>(&mut r, 0)?);
            }
            let mut heads: Vec<(String, Option<ActorHead>)> = Vec::new();
            let mut seen = std::collections::BTreeSet::new();
            for id in ids {
                let canonical = super::actor_bind::resolve_in(&tx, &id).await?;
                if !seen.insert(canonical.clone()) {
                    continue;
                }
                let mut r = run(
                    &tx,
                    tx.query("SELECT display_name, subtype, revision, catalog_id, catalog_json FROM ONLY $a WHERE type::record('memory', $m).lifecycle = 'active'")
                        .bind(("a", rid("brain_actor", &canonical)))
                        .bind(("m", rid("memory", &canonical))),
                )
                .await?;
                heads.push((canonical, take::<Option<ActorHead>>(&mut r, 0)?));
            }
            Ok(heads)
        }
    })
    .await?
    .value;
    let mut hits: Vec<ActorNameHit> = owned
        .into_iter()
        .filter_map(|(actor_id, head)| {
            let head = head?;
            let catalog: Option<Actor> = head
                .catalog_json
                .as_deref()
                .and_then(|raw| serde_json::from_str(raw).ok());
            Some(ActorNameHit {
                actor_id,
                catalog_id: head.catalog_id,
                display_name: head.display_name.clone(),
                subtype: head.subtype,
                revision: head.revision,
                reason: name_reason(&normalized, &head.display_name, catalog.as_ref()),
            })
        })
        .collect();
    hits.sort_by(|a, b| a.actor_id.cmp(&b.actor_id));
    Ok(if hits.len() == 1 {
        ActorNameSearch {
            matches: hits,
            collisions: Vec::new(),
            diagnostic: "",
            revision,
        }
    } else {
        ActorNameSearch {
            matches: Vec::new(),
            collisions: hits,
            diagnostic: "",
            revision,
        }
    })
}

/// One alias candidate with its strength, provenance and live owners.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ActorAliasHit {
    pub alias: String,
    pub normalized: String,
    /// Row type (today only `name`).
    pub alias_type: String,
    /// `curated` (code-vetted short form) | `catalog` (seeded catalog row) |
    /// `learned` (bound at extraction time).
    pub strength: &'static str,
    /// The row's provenance JSON, verbatim.
    pub provenance: serde_json::Value,
    /// Canonical active owners; more than one is ambiguous and never
    /// auto-binds.
    pub owners: Vec<String>,
    pub ambiguous: bool,
    /// Lowest owner revision, or 0 when no owner is active.
    pub revision: i64,
}

/// Result of [`search_actor_aliases`]: exact-normalized candidates plus
/// bounded prefix candidates from the ordered alias index.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ActorAliasSearch {
    pub exact: Vec<ActorAliasHit>,
    pub prefix: Vec<ActorAliasHit>,
    /// `""` | `catalog unavailable` | `query too short` | `unsupported type`.
    pub diagnostic: &'static str,
    pub revision: String,
}

impl ActorAliasSearch {
    pub fn unavailable(diagnostic: &'static str) -> Self {
        Self {
            exact: Vec::new(),
            prefix: Vec::new(),
            diagnostic,
            revision: String::new(),
        }
    }
}

/// Alias strength from the row's provenance: curated backfills stamp
/// `curated`, seeded catalog rows carry the CSV metadata (with its
/// `actor_id`), everything else was learned at bind time.
fn alias_strength(provenance: &serde_json::Value) -> &'static str {
    if provenance
        .get("curated")
        .is_some_and(|v| v == &serde_json::Value::Bool(true))
    {
        "curated"
    } else if provenance.get("actor_id").is_some() {
        "catalog"
    } else {
        "learned"
    }
}

/// The ordered-range end for `prefix` candidates: the smallest string that
/// sorts after every string starting with `prefix`.
fn prefix_end(prefix: &str) -> String {
    let mut end = prefix.to_string();
    match end.pop() {
        Some(last) => {
            end.push(char::from_u32(last as u32 + 1).unwrap_or(char::MAX));
            end
        }
        None => end,
    }
}

#[derive(SurrealValue)]
struct AliasCandidateRow {
    actor_id: String,
    alias: String,
    normalized: String,
    alias_type: String,
    provenance_json: String,
}

#[derive(SurrealValue)]
struct OwnerRevision {
    revision: i64,
}

/// Resolve one alias row's canonical active owner (if any) and finish its hit.
/// Ambiguity is decided by the caller across the rows of one normalized form.
async fn resolve_alias_row(
    tx: &Tx,
    row: AliasCandidateRow,
) -> Result<(ActorAliasHit, Option<String>), TxError> {
    let provenance: serde_json::Value =
        serde_json::from_str(&row.provenance_json).unwrap_or(serde_json::Value::Null);
    let canonical = super::actor_bind::resolve_in(tx, &row.actor_id).await?;
    let mut r = run(
        tx,
        tx.query(
            "SELECT revision FROM ONLY $a WHERE type::record('memory', $m).lifecycle = 'active'",
        )
        .bind(("a", rid("brain_actor", &canonical)))
        .bind(("m", rid("memory", &canonical))),
    )
    .await?;
    let active: Option<OwnerRevision> = take(&mut r, 0)?;
    let (owner_present, revision) = match &active {
        Some(owner) => (true, owner.revision),
        None => (false, 0),
    };
    let hit = ActorAliasHit {
        alias: row.alias,
        normalized: row.normalized,
        alias_type: row.alias_type,
        strength: alias_strength(&provenance),
        provenance,
        owners: if owner_present {
            vec![canonical.clone()]
        } else {
            Vec::new()
        },
        ambiguous: false,
        revision,
    };
    Ok((hit, active.map(|_| canonical)))
}

/// Merge sibling rows of one normalized form into single hits whose `owners`
/// list every distinct active canonical owner; `ambiguous` marks shared forms
/// (collisions stay ambiguous regardless of rank).
fn merge_alias_owners(hits: Vec<ActorAliasHit>, owners: Vec<Option<String>>) -> Vec<ActorAliasHit> {
    let mut by_owner: Vec<(ActorAliasHit, Vec<String>)> = hits
        .into_iter()
        .zip(owners)
        .map(|(hit, owner)| (hit, owner.into_iter().collect()))
        .collect();
    let mut merged: Vec<ActorAliasHit> = Vec::new();
    for (hit, owners) in by_owner.drain(..) {
        match merged
            .iter_mut()
            .find(|known| known.normalized == hit.normalized)
        {
            Some(known) => {
                for owner in owners {
                    if !known.owners.contains(&owner) {
                        known.owners.push(owner);
                    }
                }
                known.owners.sort();
            }
            None => merged.push(ActorAliasHit { owners, ..hit }),
        }
    }
    for hit in &mut merged {
        hit.ambiguous = hit.owners.len() > 1;
    }
    merged.sort_by(|a, b| a.normalized.cmp(&b.normalized).then(a.alias.cmp(&b.alias)));
    merged
}

/// `search_actor_aliases`: alias candidates with strength, type, provenance
/// and live owners. Exact matches use the `brain_actor_alias_normalized`
/// index equality arm; prefix candidates use its ordered range arm, both
/// bounded by `limit`. Ambiguous owners are reported, never resolved.
pub async fn search_actor_aliases(
    repo: &Repository,
    name: &str,
    limit: usize,
) -> Result<ActorAliasSearch> {
    if let Some(diagnostic) = unsupported_name_reason(name) {
        return Ok(ActorAliasSearch::unavailable(diagnostic));
    }
    let normalized = brain_actors::normalize_alias(name);
    if !crate::actor_catalog::matchable_alias(&normalized) {
        return Ok(ActorAliasSearch::unavailable("query too short"));
    }
    if status(repo.db()).await?.needs_seed {
        return Ok(ActorAliasSearch::unavailable("catalog unavailable"));
    }
    let revision = catalog_revision(repo.db()).await?;
    let limit = limit.clamp(*ACTOR_SEARCH_LIMIT.start(), *ACTOR_SEARCH_LIMIT.end()) as i64;
    let owned = run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let normalized = normalized.clone();
        async move {
            let mut exact_rows: Vec<AliasCandidateRow> = {
                let mut r = run(
                    &tx,
                    tx.query("SELECT actor_id, alias, normalized, alias_type, provenance_json FROM brain_actor_alias WHERE normalized = $n AND alias_type = 'name' LIMIT $limit")
                        .bind(("n", normalized.clone()))
                        .bind(("limit", limit)),
                )
                .await?;
                take(&mut r, 0)?
            };
            // The curated reverse lookup covers homes before the backfill.
            for source in crate::actor_catalog::curated_sources_for(&normalized) {
                let mut r = run(
                    &tx,
                    tx.query("SELECT VALUE catalog_json FROM brain_actor WHERE catalog_id = $source AND scope_key = $scope AND type::record('memory', record::id(id)).lifecycle = 'active' LIMIT 2")
                        .bind(("source", source.to_string()))
                        .bind(("scope", actor_catalog::SCOPE)),
                )
                .await?;
                for raw in take::<Vec<String>>(&mut r, 0)? {
                    let Ok(actor) = serde_json::from_str::<Actor>(&raw) else {
                        continue;
                    };
                    for curated in actor_catalog::curated_aliases(&actor.source_id) {
                        if brain_actors::normalize_alias(curated) != normalized
                            || exact_rows
                                .iter()
                                .any(|row| row.actor_id == actor.id)
                        {
                            continue;
                        }
                        exact_rows.push(AliasCandidateRow {
                            actor_id: actor.id.clone(),
                            alias: (*curated).to_owned(),
                            normalized: normalized.clone(),
                            alias_type: "name".into(),
                            provenance_json: serde_json::json!({
                                "curated": true, "source_id": actor.source_id
                            })
                            .to_string(),
                        });
                    }
                }
            }
            let mut prefix_rows: Vec<AliasCandidateRow> = {
                let mut r = run(
                    &tx,
                    tx.query("SELECT actor_id, alias, normalized, alias_type, provenance_json FROM brain_actor_alias WHERE normalized >= $p AND normalized < $end AND alias_type = 'name' LIMIT $limit")
                        .bind(("p", normalized.clone()))
                        .bind(("end", prefix_end(&normalized)))
                        .bind(("limit", limit)),
                )
                .await?;
                take(&mut r, 0)?
            };
            prefix_rows.retain(|row| row.normalized.starts_with(&normalized));
            let mut exact_hits = Vec::new();
            let mut exact_owners = Vec::new();
            for row in exact_rows {
                let (hit, owner) = resolve_alias_row(&tx, row).await?;
                exact_hits.push(hit);
                exact_owners.push(owner);
            }
            let mut prefix_hits = Vec::new();
            let mut prefix_owners = Vec::new();
            for row in prefix_rows {
                let (hit, owner) = resolve_alias_row(&tx, row).await?;
                prefix_hits.push(hit);
                prefix_owners.push(owner);
            }
            Ok((
                merge_alias_owners(exact_hits, exact_owners),
                merge_alias_owners(prefix_hits, prefix_owners),
            ))
        }
    })
    .await?
    .value;
    Ok(ActorAliasSearch {
        exact: owned.0,
        prefix: owned.1,
        diagnostic: "",
        revision,
    })
}

/// Current catalog actors for all article consumers, including before NLP
/// metadata is available. None preserves a pre-catalog home's legacy view.
pub(crate) async fn metadata_article_actors(
    repo: &Repository,
    article: &crate::store::AtlasArticleRow,
) -> Result<Option<Vec<crate::atlas_metadata::ArticleActor>>> {
    let mut r = repo
        .db()
        .query("SELECT VALUE status FROM ONLY brain_actor_seed:catalog")
        .await?
        .check()?;
    let state: Option<String> = r.take(0)?;
    let Some(state) = state else {
        return Ok(None);
    };
    // The seed receipt is authoritative here. Counting the entire actor table
    // on every brief read turns a local lookup into a catalog-wide scan.
    if state != "complete" {
        return Ok(Some(Vec::new()));
    }
    Ok(Some(repo.actor_matcher().await?.article_actors(article)))
}

/// Current metadata actors followed by actors detected in the retained clean
/// body. Body matching never needs NLP: unambiguous catalog aliases bind
/// exactly; NLP person/organization candidates enrich the resolved spellings
/// when available.
pub async fn article_actors(
    repo: &Repository,
    article: &crate::store::AtlasArticleRow,
) -> Result<Option<Vec<crate::atlas_metadata::ArticleActor>>> {
    let Some(mut actors) = metadata_article_actors(repo, article).await? else {
        return Ok(None);
    };
    crate::actor_catalog::append_body_actors(
        &mut actors,
        body_actors(repo, &article.id).await?.unwrap_or_default(),
    );
    Ok(Some(actors))
}

pub(crate) async fn body_actors(
    repo: &Repository,
    article_id: &str,
) -> Result<Option<Vec<crate::atlas_metadata::ArticleActor>>> {
    let Some(clean) = clean_body(repo, article_id).await? else {
        return Ok(None);
    };
    let mut ready = repo
        .db()
        .query("SELECT VALUE status FROM ONLY brain_actor_seed:catalog")
        .await?
        .check()?;
    if ready.take::<Option<String>>(0)?.as_deref() != Some("complete") {
        return Ok(None);
    }
    // NLP-off matching is exact-only; an incomplete NLP result is retryable
    // and only delays the enrichment, never the exact pass.
    let candidates = clean_body_candidates(repo, &clean)
        .await?
        .unwrap_or_default();
    let matcher = repo.actor_matcher().await?;
    Ok(Some(
        tokio::task::spawn_blocking(move || matcher.body_actors(&clean.text, &candidates)).await?,
    ))
}

/// NLP runs only on the retained cleaned artifact. Incomplete engine results
/// stay retryable and must never become successful empty-match receipts.
pub(crate) async fn clean_body_candidates(
    repo: &Repository,
    clean: &CleanBody,
) -> Result<Option<Vec<crate::nlp::EntityCandidate>>> {
    use crate::nlp::{Mode, Service, Source, Status};
    if crate::nlp::configured_mode() == Mode::Off {
        return Ok(None);
    }
    let source = Source::derived(&clean.source_id, &clean.revision, &clean.text);
    let context = source.context();
    let annotation = match super::nlp_cache::get(repo, &source, &context).await? {
        Some(annotation) => annotation,
        None => {
            let annotation = Service::shared()
                .annotate(
                    source.clone(),
                    context.clone(),
                    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                )
                .await;
            if annotation.status != Status::Complete {
                return Ok(None);
            }
            super::nlp_cache::put(repo, &source, &context, &annotation).await?;
            annotation
        }
    };
    Ok(Some(crate::nlp::entity_candidates(&source, &annotation)))
}

/// Committed work only; the denominator is the fully validated input CSV.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Progress {
    pub processed: usize,
    pub total: usize,
    pub aliases: usize,
    pub complete: bool,
}

#[derive(Clone, SurrealValue)]
struct ActorRow {
    id: RecordId,
    subtype: String,
    display_name: String,
    identity_state: String,
    scope_key: String,
    revision: i64,
    provenance_json: String,
    catalog_json: String,
    catalog_id: String,
    created_at: String,
    updated_at: String,
}
#[derive(Clone, SurrealValue)]
struct MemoryRow {
    id: RecordId,
    text: String,
    category: String,
    pinned: bool,
    lifecycle: String,
    source_json: String,
    memory_kind: String,
    payload_json: String,
}
#[derive(Clone, SurrealValue)]
struct AliasRow {
    id: RecordId,
    actor_id: String,
    alias: String,
    normalized: String,
    alias_type: String,
    provenance_json: String,
}
// Share wide source provenance until a bounded alias INSERT packet is built.
// Every packet remains inside the actor batch's existing transaction.
#[derive(Clone)]
struct SeedAliasPlan {
    id: RecordId,
    actor_id: String,
    alias: String,
    normalized: String,
    provenance_json: Arc<String>,
}

impl SeedAliasPlan {
    fn packet_bytes(&self) -> Result<usize, TxError> {
        self.provenance_json
            .len()
            .checked_mul(2)
            .and_then(|size| {
                size.checked_add(
                    6 * (self.actor_id.len() + self.alias.len() + self.normalized.len()) + 256,
                )
            })
            .ok_or_else(|| TxError::Abort("actor_seed_batch_budget".into()))
    }
    fn row(&self) -> AliasRow {
        AliasRow {
            id: self.id.clone(),
            actor_id: self.actor_id.clone(),
            alias: self.alias.clone(),
            normalized: self.normalized.clone(),
            alias_type: "name".into(),
            provenance_json: self.provenance_json.as_ref().clone(),
        }
    }
}

async fn insert_seed_aliases(
    tx: &Tx,
    aliases: &[SeedAliasPlan],
    execution: Option<&SeedExecution>,
    cancel: &AtomicBool,
) -> Result<(), TxError> {
    let mut position = 0;
    while position < aliases.len() {
        if cancel.load(Ordering::Acquire) {
            return Err(TxError::Abort("actor seeding cancelled".into()));
        }
        let mut packet = Vec::new();
        let mut bytes = 0;
        while let Some(alias) = aliases.get(position) {
            let charge = alias.packet_bytes()?;
            if charge > 1024 * 1024 {
                return Err(TxError::Abort("actor_seed_batch_budget".into()));
            }
            if !packet.is_empty() && bytes + charge > 256 * 1024 {
                break;
            }
            bytes += charge;
            packet.push(alias.row());
            position += 1;
        }
        run(
            tx,
            tx.query("INSERT IGNORE INTO brain_actor_alias $aliases RETURN NONE")
                .bind(("aliases", packet)),
        )
        .await?;
        if let Some(execution) = execution {
            execution.check_in(tx, cancel).await?;
        }
    }
    Ok(())
}

#[derive(Clone, SurrealValue)]
struct GraphRow {
    id: RecordId,
    kind: String,
    ref_id: String,
    memory_kind: String,
    lifecycle: String,
}

/// Validate offline, then insert stable identities in bounded transactions.
/// Retries reuse committed IDs; completion is recorded only after all batches.
/// The callback reports immutable snapshots and never runs inside a transaction.
pub async fn seed_file(
    repo: &Repository,
    path: &Path,
    cancel: &AtomicBool,
    report: impl Fn(Progress),
) -> Result<Progress> {
    let path = path.to_owned();
    let (actors, fingerprint) = tokio::task::spawn_blocking(move || -> Result<_> {
        let bytes = std::fs::read(&path)
            .with_context(|| format!("read actor dataset {}", path.display()))?;
        let fingerprint = crate::evidence::content_hash(std::str::from_utf8(&bytes)?);
        Ok((actor_catalog::read(bytes.as_slice())?, fingerprint))
    })
    .await??;
    seed_actors(repo, actors, &fingerprint, cancel, report).await
}

/// Ingest already validated records (also used by isolated offline fixtures).
pub async fn seed_actors(
    repo: &Repository,
    actors: Vec<Actor>,
    fingerprint: &str,
    cancel: &AtomicBool,
    report: impl Fn(Progress),
) -> Result<Progress> {
    seed_actors_with_source_policy(repo, actors, fingerprint, cancel, report, true, None, None)
        .await
}

/// Seed a new catalog or retry its pinned source without mixing datasets.
pub async fn seed_actors_pinned(
    repo: &Repository,
    actors: Vec<Actor>,
    fingerprint: &str,
    cancel: &AtomicBool,
    report: impl Fn(Progress),
) -> Result<Progress> {
    seed_actors_with_source_policy(repo, actors, fingerprint, cancel, report, false, None, None)
        .await
}

/// Seed from the genuine claimed setup job, preserving its durable child history.
pub async fn seed_actors_pinned_attributed(
    repo: &Repository,
    actors: Vec<Actor>,
    fingerprint: &str,
    cause_job: &str,
    cancel: &AtomicBool,
    report: impl Fn(Progress),
) -> Result<Progress> {
    seed_actors_with_source_policy(
        repo,
        actors,
        fingerprint,
        cancel,
        report,
        false,
        Some(cause_job),
        None,
    )
    .await
}

/// Seed only under the exact actual task lease captured by the executor.
/// The admitted dataset hash and parent job come from opaque server identity.
pub async fn seed_actors_pinned_claimed(
    repo: &Repository,
    actors: Vec<Actor>,
    execution: &SeedExecution,
    cancel: &AtomicBool,
    report: impl Fn(Progress),
) -> Result<Progress> {
    seed_actors_with_source_policy(
        repo,
        actors,
        execution.fingerprint(),
        cancel,
        report,
        false,
        Some(execution.parent_job()),
        Some(execution),
    )
    .await
}

/// Consume validated bounded batches under the existing claimed-task/source fences.
/// The producer must finish full source validation before this function is called.
/// Selected stable identities are upserted; absent identities are retained. The
/// catalog fingerprint describes the latest import, not every retained identity.
pub async fn seed_actors_stream_claimed<S>(
    repo: &Repository,
    total: usize,
    batches: S,
    execution: &SeedExecution,
    cancel: &AtomicBool,
    report: impl Fn(Progress),
) -> Result<Progress>
where
    S: futures_util::Stream<Item = Result<Vec<Actor>>>,
{
    use futures_util::StreamExt;
    let checked = batches.map(|batch| {
        let batch = batch?;
        let bytes = batch.iter().try_fold(0_u64, |total, actor| {
            total
                .checked_add(actor_catalog::actor_seed_payload_bytes(actor)?)
                .context("actor_seed_batch_budget")
        })?;
        anyhow::ensure!(bytes <= MAX_SEED_BATCH_BYTES, "actor_seed_batch_budget");
        let expanded = batch.iter().try_fold(0_u64, |total, actor| {
            total
                .checked_add(seed_actor_transaction_bytes(actor)?)
                .context("actor_seed_batch_budget")
        })?;
        anyhow::ensure!(
            expanded <= MAX_SEED_TRANSACTION_BYTES,
            "actor_seed_batch_budget"
        );
        Ok(batch)
    });
    seed_actor_batches_with_source_policy(
        repo,
        total,
        checked,
        execution.fingerprint(),
        cancel,
        report,
        true,
        true,
        Some(execution.parent_job()),
        Some(execution),
    )
    .await
}

async fn seed_actors_with_source_policy(
    repo: &Repository,
    actors: Vec<Actor>,
    fingerprint: &str,
    cancel: &AtomicBool,
    report: impl Fn(Progress),
    allow_different_source: bool,
    cause_job: Option<&str>,
    execution: Option<&SeedExecution>,
) -> Result<Progress> {
    let total = actors.len();
    let mut remaining = actors.into_iter();
    let batches = futures_util::stream::iter(std::iter::from_fn(move || {
        let batch: Vec<_> = remaining.by_ref().take(BATCH).collect();
        (!batch.is_empty()).then_some(Ok::<_, anyhow::Error>(batch))
    }));
    seed_actor_batches_with_source_policy(
        repo,
        total,
        batches,
        fingerprint,
        cancel,
        report,
        allow_different_source,
        false,
        cause_job,
        execution,
    )
    .await
}

async fn seed_actor_batches_with_source_policy<S>(
    repo: &Repository,
    total: usize,
    batches: S,
    fingerprint: &str,
    cancel: &AtomicBool,
    report: impl Fn(Progress),
    allow_different_source: bool,
    merge_selected: bool,
    cause_job: Option<&str>,
    execution: Option<&SeedExecution>,
) -> Result<Progress>
where
    S: futures_util::Stream<Item = Result<Vec<Actor>>>,
{
    if total == 0 {
        bail!("actor catalog cannot be empty");
    }
    if let Some(execution) = execution {
        if execution.fingerprint() != fingerprint || Some(execution.parent_job()) != cause_job {
            bail!("actor_seed_execution_contract");
        }
    }
    let fingerprint = fingerprint.to_owned();
    let owner = format!(
        "{}:{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    );
    let now = chrono::Utc::now().to_rfc3339();
    run_tx(repo.db(),RetryPolicy::default(),|tx| {
        let fingerprint=fingerprint.clone();let owner=owner.clone();let now=now.clone();
        async move {
            if let Some(execution)=execution {execution.check_in(&tx,cancel).await?;}
            run(&tx,tx.query(
                "LET $previous = (SELECT status, lease_until, fingerprint FROM ONLY brain_actor_seed:catalog);
                 IF $previous.status = 'running' AND $previous.lease_until > $now { THROW 'actor seed is already running'; };
                 IF $allow_different_source = false AND $previous != NONE AND $previous.fingerprint != $hash { THROW 'actor catalog already has a different source fingerprint'; };
                 UPSERT brain_actor_seed:catalog SET status='running',fingerprint=$hash,processed=0,total=$total,aliases=0,owner=$owner,lease_until=$lease,updated_at=$now RETURN NONE;")
                .bind(("allow_different_source",allow_different_source)).bind(("now",now)).bind(("hash",fingerprint)).bind(("total",total)).bind(("owner",owner))
                .bind(("lease",(chrono::Utc::now()+chrono::Duration::minutes(5)).to_rfc3339()))).await?;
            if let Some(execution)=execution {execution.check_in(&tx,cancel).await?;}
            Ok(())
        }
    }).await?;
    let cause = cause_job.map(|parent| super::actor_search_outbox::SourceCause {
        parent_job: parent.to_owned(),
        operation_id: owner.clone(),
    });
    let result = ingest(
        repo,
        total,
        batches,
        &owner,
        cause.as_ref(),
        execution,
        merge_selected,
        cancel,
        &report,
    )
    .await;
    if result.is_err() {
        run_tx(repo.db(),RetryPolicy::default(),|tx| {let owner=owner.clone();let cause=cause.clone();async move {
            let mut response=run(&tx,tx.query("RETURN (SELECT VALUE owner FROM ONLY brain_actor_seed:catalog)=$owner").bind(("owner",owner.clone()))).await?;
            if super::jobs::take::<Option<bool>>(&mut response,0)?==Some(true) {
                run(&tx,tx.query("UPDATE brain_actor_seed:catalog SET status='failed',lease_until='' RETURN NONE")).await?;
                if let Some(cause)=cause {super::actor_search_outbox::close_cause_in(&tx,&cause,true,chrono::Utc::now()).await?;}
            }
            Ok(())
        }}).await?;
    }
    result
}

async fn ingest<S>(
    repo: &Repository,
    total: usize,
    batches: S,
    owner: &str,
    cause: Option<&super::actor_search_outbox::SourceCause>,
    execution: Option<&SeedExecution>,
    merge_selected: bool,
    cancel: &AtomicBool,
    report: &impl Fn(Progress),
) -> Result<Progress>
where
    S: futures_util::Stream<Item = Result<Vec<Actor>>>,
{
    let mut progress = Progress {
        total,
        ..Progress::default()
    };
    report(progress.clone());
    let mut batches = std::pin::pin!(batches);
    while let Some(chunk) = futures_util::StreamExt::next(&mut batches).await {
        let chunk = chunk?;
        anyhow::ensure!(
            !chunk.is_empty() && chunk.len() <= BATCH,
            "actor_seed_batch_budget"
        );
        anyhow::ensure!(
            progress.processed + chunk.len() <= total,
            "actor_seed_count_changed"
        );
        if let Some(execution) = execution {
            super::actor_search_outbox::await_bulk_capacity_claimed(repo, owner, cancel, execution)
                .await?;
        } else {
            super::actor_search_outbox::await_bulk_capacity(repo, owner, cancel).await?;
        }
        if cancel.load(Ordering::Acquire) {
            bail!("actor seeding cancelled; committed batches can be retried");
        }
        let now = chrono::Utc::now().to_rfc3339();
        let mut actor_rows = Vec::new();
        let mut memories = Vec::new();
        let mut aliases = Vec::new();
        let mut graph = Vec::new();
        for actor in &chunk {
            let provenance = Arc::new(serde_json::to_string(&actor.metadata)?);
            actor_rows.push(ActorRow {
                id: rid("brain_actor", &actor.id),
                subtype: actor.subtype.clone(),
                display_name: actor.name.clone(),
                identity_state: "resolved".into(),
                scope_key: actor_catalog::SCOPE.into(),
                revision: 1,
                provenance_json: provenance.as_ref().clone(),
                catalog_json: serde_json::to_string(actor)?,
                catalog_id: actor.source_id.clone(),
                created_at: now.clone(),
                updated_at: now.clone(),
            });
            let payload = MemoryPayload::Actor(ActorPayload {
                subtype: brain_actors::parse_subtype(&actor.subtype),
                display_name: actor.name.clone(),
                aliases: actor.aliases.clone(),
                identifiers: vec![],
                identity_state: IdentityState::Resolved,
            });
            memories.push(MemoryRow {
                id: rid("memory", &actor.id), text: brain_actors::text_of(&payload), category: "fact".into(), pinned: false,
                lifecycle: "active".into(), source_json: serde_json::json!({"app":"brain", "conversation_id": actor_catalog::SCOPE, "message_id":null,"reference":actor.source_id}).to_string(),
                memory_kind: "actor".into(), payload_json: serde_json::to_string(&payload)?,
            });
            let mut keys = std::collections::BTreeSet::new();
            // The canonical display name is itself a name alias (the matcher
            // treats `name` and `aliases` identically), so exact-name search
            // and binding resolve actors that list no explicit alias.
            for alias in std::iter::once(&actor.name).chain(&actor.aliases) {
                let normalized = brain_actors::normalize_alias(alias);
                if normalized.chars().count() < 3 || !keys.insert(normalized.clone()) {
                    continue;
                }
                aliases.push(SeedAliasPlan {
                    id: RecordId::new(
                        "brain_actor_alias",
                        surrealdb::types::Array::from(vec![
                            actor.id.clone(),
                            normalized.clone(),
                            "name".to_owned(),
                        ]),
                    ),
                    actor_id: actor.id.clone(),
                    alias: alias.clone(),
                    normalized,
                    provenance_json: Arc::clone(&provenance),
                });
            }
            graph.push(GraphRow {
                id: rid("brain_graph_node", &super::graph::memory_node(&actor.id)),
                kind: "memory".into(),
                ref_id: actor.id.clone(),
                memory_kind: "actor".into(),
                lifecycle: "active".into(),
            });
        }
        let next = Progress {
            processed: progress.processed + chunk.len(),
            total: progress.total,
            aliases: progress.aliases + aliases.len(),
            complete: false,
        };
        let actor_ids: Vec<String> = chunk.iter().map(|actor| actor.id.clone()).collect();
        let data = Arc::new((actor_rows, memories, aliases, graph, actor_ids));
        run_tx(repo.db(), RetryPolicy::default(), |tx| {
            let data = data.clone(); let owner = owner.to_owned(); let next = next.clone(); let now = now.clone();let cause=cause.cloned();
            async move {
                if let Some(execution)=execution {execution.check_in(&tx,cancel).await?;}
                run(&tx, tx.query("IF (SELECT VALUE owner FROM ONLY brain_actor_seed:catalog) != $owner { THROW 'actor seed lease changed'; };
                    IF $merge_selected {
                        FOR $actor IN $actors {
                            LET $actor_id=$actor.id;
                            UPSERT $actor_id SET subtype=$actor.subtype, display_name=$actor.display_name,
                                identity_state=identity_state ?? $actor.identity_state, scope_key=$actor.scope_key,
                                revision=(revision ?? 0)+1, provenance_json=$actor.provenance_json,
                                catalog_json=$actor.catalog_json, catalog_id=$actor.catalog_id,
                                created_at=created_at ?? $actor.created_at, updated_at=$actor.updated_at RETURN NONE;
                        };
                        FOR $memory IN $memories {
                            LET $memory_id=$memory.id;
                            UPSERT $memory_id SET text=$memory.text, category=category ?? $memory.category,
                                pinned=pinned ?? $memory.pinned, lifecycle=lifecycle ?? $memory.lifecycle,
                                source_json=$memory.source_json, memory_kind=$memory.memory_kind,
                                payload_json=$memory.payload_json RETURN NONE;
                        };
                    } ELSE {
                        INSERT IGNORE INTO brain_actor $actors RETURN NONE;
                        INSERT IGNORE INTO memory $memories RETURN NONE;
                    };
                    INSERT IGNORE INTO brain_graph_node $graph RETURN NONE; UPDATE brain_actor_seed:catalog SET processed = $done, aliases = $alias_count, updated_at = $now, lease_until = $lease RETURN NONE;")
                    .bind(("owner", owner.clone())).bind(("merge_selected", merge_selected)).bind(("actors", data.0.clone())).bind(("memories", data.1.clone())).bind(("graph", data.3.clone()))
                    .bind(("done", next.processed)).bind(("alias_count", next.aliases)).bind(("now", now)).bind(("lease", (chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339()))).await?;
                insert_seed_aliases(&tx, &data.2, execution, cancel).await?;
                super::jobs::bump(&tx,"job_guard","atlas-derivative-catalog-input").await?;
                super::atlas_derivative_invalidation::enqueue_in(&tx,None,&format!("seed:{owner}:{}",next.processed),chrono::Utc::now()).await?;
                super::actor_search_outbox::enqueue_actors_in(&tx,&data.4,cause.as_ref(),chrono::Utc::now()).await?;
                if let Some(execution)=execution {execution.check_in(&tx,cancel).await?;}
                Ok(())
            }
        }).await.map_err(|e| anyhow::anyhow!("actor seed batch: {e}"))?;
        progress = next;
        report(progress.clone());
    }
    run_tx(repo.db(),RetryPolicy::default(),|tx| {let owner=owner.to_owned();let cause=cause.cloned();let total=progress.total;async move {
        if let Some(execution)=execution {execution.check_in(&tx,cancel).await?;}
        run(&tx,tx.query("IF (SELECT VALUE owner FROM ONLY brain_actor_seed:catalog) != $owner OR (SELECT VALUE processed FROM ONLY brain_actor_seed:catalog) != $total { THROW 'actor seed completion lost ownership'; }; UPDATE brain_actor_seed:catalog SET status='complete',lease_until='' RETURN NONE")
            .bind(("owner",owner)).bind(("total",total))).await?;
        if let Some(cause)=cause {super::actor_search_outbox::close_cause_in(&tx,&cause,false,chrono::Utc::now()).await?;}
        if let Some(execution)=execution {execution.check_in(&tx,cancel).await?;}
        Ok(())
    }}).await?;
    // Matcher inputs changed: cached repositories rebuild single-flight.
    crate::actor_catalog::bump_catalog_epoch();
    progress.complete = true;
    report(progress.clone());
    Ok(progress)
}

/// The current registered clean artifact, never raw acquisition markdown.
#[derive(Clone, Debug)]
pub(crate) struct CleanBody {
    pub source_id: String,
    pub revision: String,
    pub artifact_hash: String,
    pub text: String,
}

#[derive(SurrealValue)]
struct BodyHead {
    id: String,
    body_version: i64,
    state: String,
}
#[derive(SurrealValue)]
struct BodyAcquisition {
    attempt_id: String,
    raw_hash: String,
}

/// Select only cleanup output attached to the latest body's baseline acquisition.
pub(crate) async fn clean_body_in(tx: &Tx, article_id: &str) -> Result<Option<CleanBody>, TxError> {
    let mut rows = run(tx, tx.query("SELECT record::id(id) AS id, body_version, state, updated_at FROM article_body WHERE article_id = $a ORDER BY body_version DESC, updated_at DESC LIMIT 1").bind(("a", article_id.to_owned()))).await?;
    let Some(body) = take::<Vec<BodyHead>>(&mut rows, 0)?
        .into_iter()
        .next()
        .filter(|body| body.state == "ready")
    else {
        return Ok(None);
    };
    let revision = body.body_version.to_string();
    let mut rows = run(tx, tx.query("SELECT attempt_id, raw_hash, registered_at FROM nlp_source_ref WHERE owner_kind = 'article' AND owner_id = $a AND source_id = $source AND revision = $revision AND transform_version = 'validated-baseline-v1' AND raw_hash != NONE ORDER BY registered_at DESC LIMIT 1")
        .bind(("a", article_id.to_owned())).bind(("source", body.id.clone())).bind(("revision", revision.clone()))).await?;
    let Some(acquisition) = take::<Vec<BodyAcquisition>>(&mut rows, 0)?
        .into_iter()
        .next()
    else {
        return Ok(None);
    };
    let mut rows = run(tx, tx.query("SELECT VALUE prose_hash FROM nlp_source_ref WHERE owner_kind = 'article' AND owner_id = $a AND source_id = $source AND revision = $revision AND transform_version = $transform AND attempt_id = $attempt AND raw_hash = $raw ORDER BY registered_at DESC LIMIT 1")
        .bind(("a", article_id.to_owned())).bind(("source", body.id.clone())).bind(("revision", revision.clone()))
        .bind(("transform", crate::intel_recon::clean_body::TRANSFORM.to_owned())).bind(("attempt", acquisition.attempt_id)).bind(("raw", acquisition.raw_hash))).await?;
    let Some(artifact_hash) = take::<Vec<String>>(&mut rows, 0)?.into_iter().next() else {
        return Ok(None);
    };
    let mut rows = run(
        tx,
        tx.query("SELECT VALUE text FROM ONLY $artifact")
            .bind(("artifact", rid("nlp_artifact", &artifact_hash))),
    )
    .await?;
    Ok(take::<Option<String>>(&mut rows, 0)?
        .filter(|text| !text.trim().is_empty() && crate::nlp::cache::hash(text) == artifact_hash)
        .map(|text| CleanBody {
            source_id: format!("body:{}", body.id),
            revision,
            artifact_hash,
            text,
        }))
}

pub(crate) async fn clean_body(repo: &Repository, article_id: &str) -> Result<Option<CleanBody>> {
    Ok(run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let article_id = article_id.to_owned();
        async move { clean_body_in(&tx, &article_id).await }
    })
    .await?
    .value)
}

#[cfg(test)]
mod discovery_tests {
    use super::*;

    #[test]
    fn name_reasons_prefer_exact_then_normalized_then_curated() {
        let catalog = Actor {
            id: "catalog-actor-x".into(),
            source_id: "os:Q1".into(),
            name: "Harbor Union".into(),
            subtype: "organization".into(),
            aliases: vec!["harbor union".into()],
            metadata: [("name_normalized".to_owned(), "harbor union".to_owned())]
                .into_iter()
                .collect(),
        };
        assert_eq!(
            name_reason("harbor union", "Harbor Union", None),
            "exact name"
        );
        assert_eq!(
            name_reason("harbor union", "Harbor Union", Some(&catalog)),
            "exact name"
        );
        // A different spelling of the same catalog normalized form.
        assert_eq!(
            name_reason("harbor union", "HARBOR UNION LTD", Some(&catalog)),
            "normalized form"
        );
        assert_eq!(
            name_reason("other", "Harbor Union", Some(&catalog)),
            "name alias"
        );
    }

    #[test]
    fn identifiers_are_unsupported_names_and_short_queries_are_guarded() {
        for name in [
            "10.0.0.1",
            "https://example.com/a",
            "CVE-2024-1234",
            "AS14061",
        ] {
            assert_eq!(
                unsupported_name_reason(name),
                Some("unsupported type"),
                "{name}"
            );
        }
        assert_eq!(unsupported_name_reason("Harbor Union"), None);
        let search = ActorNameSearch::unavailable("query too short");
        assert!(search.matches.is_empty());
        assert_eq!(search.diagnostic, "query too short");
        assert_eq!(search.revision, "");
    }

    #[test]
    fn alias_strength_reads_provenance_and_prefix_end_advances() {
        let curated = serde_json::json!({"curated": true, "source_id": "os:Q22686"});
        let catalog = serde_json::json!({"actor_id": "os:Q22686", "actor_name": "Donald Trump"});
        let learned = serde_json::json!({"insight_claim": "fp"});
        assert_eq!(alias_strength(&curated), "curated");
        assert_eq!(alias_strength(&catalog), "catalog");
        assert_eq!(alias_strength(&learned), "learned");
        assert_eq!(alias_strength(&serde_json::Value::Null), "learned");
        assert_eq!(prefix_end("harbor"), "harbos");
        assert_eq!(prefix_end("münchen"), "müncheo");
    }

    #[test]
    fn alias_owner_merging_marks_shared_forms_ambiguous() {
        let hit = |alias: &str, normalized: &str| ActorAliasHit {
            alias: alias.into(),
            normalized: normalized.into(),
            alias_type: "name".into(),
            strength: "catalog",
            provenance: serde_json::Value::Null,
            owners: Vec::new(),
            ambiguous: false,
            revision: 1,
        };
        // Two rows for one normalized form (two actors): merged, ambiguous.
        let merged = merge_alias_owners(
            vec![hit("Shared", "shared"), hit("Shared", "shared")],
            vec![Some("a".into()), Some("b".into())],
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].owners, ["a".to_owned(), "b".to_owned()]);
        assert!(merged[0].ambiguous);
        // One row, one owner: unambiguous.
        let merged = merge_alias_owners(vec![hit("Solo", "solo")], vec![Some("a".into())]);
        assert!(!merged[0].ambiguous);
        // Retired owners drop out and ambiguity follows the live set.
        let merged = merge_alias_owners(vec![hit("Shared", "shared")], vec![None]);
        assert!(merged[0].owners.is_empty());
        assert!(!merged[0].ambiguous, "no live owner is not an ambiguity");
    }
}

// Appended to storage/actor_catalog.rs in the next repository checkpoint.
/// Verified latest clean-body identities only; never acquires body text.
/// All candidates use the same baseline/transform chain as clean_body_in.
pub(crate) async fn clean_body_heads_many_in(
    tx: &Tx,
    article_ids: &[String],
) -> Result<
    std::collections::BTreeMap<String, crate::storage::classifier_admission::BodyRevision>,
    TxError,
> {
    use crate::storage::classifier_admission::BodyRevision;
    if article_ids.len() > 64 {
        return Err(TxError::Abort("body_head_key_limit".into()));
    }
    let article_ids: Vec<_> = article_ids
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    if article_ids.is_empty() {
        return Ok(std::collections::BTreeMap::new());
    }
    let mut sql = String::new();
    for index in 0..article_ids.len() {
        sql.push_str(&format!("SELECT record::id(id) AS id,body_version,state,updated_at FROM article_body WHERE article_id=$a{index} ORDER BY body_version DESC,updated_at DESC LIMIT 1;"));
    }
    let mut query = tx.query(sql);
    for (index, article) in article_ids.iter().enumerate() {
        query = query.bind((format!("a{index}"), article.clone()));
    }
    let mut response = super::classifier_admission::measured_query(query)
        .await?
        .check()?;
    let mut bodies = Vec::new();
    for (index, article) in article_ids.into_iter().enumerate() {
        if let Some(head) = take::<Vec<BodyHead>>(&mut response, index)?
            .into_iter()
            .next()
            .filter(|head| head.state == "ready")
        {
            bodies.push((article, head));
        }
    }
    if bodies.is_empty() {
        return Ok(std::collections::BTreeMap::new());
    }
    let mut sql = String::new();
    for index in 0..bodies.len() {
        sql.push_str(&format!("SELECT attempt_id,raw_hash,registered_at FROM nlp_source_ref WHERE owner_kind='article' AND owner_id=$a{index} AND source_id=$source{index} AND revision=$revision{index} AND transform_version='validated-baseline-v1' AND raw_hash!=NONE ORDER BY registered_at DESC LIMIT 1;"));
    }
    let mut query = tx.query(sql);
    for (index, (article, head)) in bodies.iter().enumerate() {
        query = query
            .bind((format!("a{index}"), article.clone()))
            .bind((format!("source{index}"), head.id.clone()))
            .bind((format!("revision{index}"), head.body_version.to_string()));
    }
    let mut response = super::classifier_admission::measured_query(query)
        .await?
        .check()?;
    let mut baselines = Vec::new();
    for (index, (article, head)) in bodies.into_iter().enumerate() {
        if let Some(baseline) = take::<Vec<BodyAcquisition>>(&mut response, index)?
            .into_iter()
            .next()
        {
            baselines.push((article, head, baseline));
        }
    }
    if baselines.is_empty() {
        return Ok(std::collections::BTreeMap::new());
    }
    let mut sql = String::new();
    for index in 0..baselines.len() {
        sql.push_str(&format!("SELECT VALUE prose_hash FROM nlp_source_ref WHERE owner_kind='article' AND owner_id=$a{index} AND source_id=$source{index} AND revision=$revision{index} AND transform_version=$transform AND attempt_id=$attempt{index} AND raw_hash=$raw{index} ORDER BY registered_at DESC LIMIT 1;"));
    }
    let mut query = tx.query(sql).bind((
        "transform",
        crate::intel_recon::clean_body::TRANSFORM.to_owned(),
    ));
    for (index, (article, head, baseline)) in baselines.iter().enumerate() {
        query = query
            .bind((format!("a{index}"), article.clone()))
            .bind((format!("source{index}"), head.id.clone()))
            .bind((format!("revision{index}"), head.body_version.to_string()))
            .bind((format!("attempt{index}"), baseline.attempt_id.clone()))
            .bind((format!("raw{index}"), baseline.raw_hash.clone()));
    }
    let mut response = super::classifier_admission::measured_query(query)
        .await?
        .check()?;
    let mut result = std::collections::BTreeMap::new();
    for (index, (article, head, _)) in baselines.into_iter().enumerate() {
        if let Some(hash) = take::<Vec<String>>(&mut response, index)?
            .into_iter()
            .next()
        {
            if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(TxError::Abort(
                    "invalid_clean_body_artifact_identity".into(),
                ));
            }
            result.insert(
                article,
                BodyRevision {
                    source_id: format!("body:{}", head.id),
                    revision: head.body_version.to_string(),
                    artifact_hash: hash,
                    candidates_revision: None,
                },
            );
        }
    }
    verify_body_heads_in(tx, result).await
}

#[derive(SurrealValue)]
struct ArtifactIdentity {
    actual_hash: String,
    nonempty: bool,
}
#[derive(SurrealValue)]
struct CandidateIdentity {
    prose_hash: String,
    revision: String,
}
async fn verify_body_heads_in(
    tx: &Tx,
    heads: std::collections::BTreeMap<String, crate::storage::classifier_admission::BodyRevision>,
) -> Result<
    std::collections::BTreeMap<String, crate::storage::classifier_admission::BodyRevision>,
    TxError,
> {
    if heads.is_empty() {
        return Ok(heads);
    }
    let rows: Vec<_> = heads.into_iter().collect();
    let mut sql = String::new();
    let mut feature_keys = Vec::new();
    for (index, (_, head)) in rows.iter().enumerate() {
        let mut source = crate::nlp::Source::derived(&head.source_id, &head.revision, "");
        source.prose_hash = head.artifact_hash.clone();
        let feature = crate::nlp::cache::feature_key(&source, &source.context());
        super::jobs::bump(tx, "job_guard", &format!("nlp-feature:{feature}")).await?;
        feature_keys.push(feature);
        sql.push_str(&format!("SELECT crypto::sha256(text) AS actual_hash,string::trim(text)!='' AS nonempty FROM ONLY $artifact{index};"));
        if crate::nlp::configured_mode() == crate::nlp::Mode::Off {
            sql.push_str("RETURN NONE;");
        } else {
            sql.push_str(&format!("SELECT prose_hash,crypto::sha256(annotations_json) AS revision FROM ONLY $feature{index};"));
        }
    }
    let mut query = super::classifier_admission::tx_query(tx, sql);
    for (index, (_, head)) in rows.iter().enumerate() {
        query = query
            .bind((
                format!("artifact{index}"),
                rid("nlp_artifact", &head.artifact_hash),
            ))
            .bind((
                format!("feature{index}"),
                rid("nlp_feature", &feature_keys[index]),
            ));
    }
    let mut response = query.await?.check()?;
    let mut out = std::collections::BTreeMap::new();
    for (index, (article, mut head)) in rows.into_iter().enumerate() {
        let artifact: Option<ArtifactIdentity> = take(&mut response, index * 2)?;
        let candidate: Option<CandidateIdentity> = take(&mut response, index * 2 + 1)?;
        if !artifact
            .is_some_and(|artifact| artifact.nonempty && artifact.actual_hash == head.artifact_hash)
        {
            continue;
        }
        head.candidates_revision = candidate
            .filter(|candidate| candidate.prose_hash == head.artifact_hash)
            .map(|candidate| format!("{}:{}", feature_keys[index], candidate.revision));
        out.insert(article, head);
    }
    Ok(out)
}
/// Correlates retained body record IDs to article owners in a bounded group.
/// Callers strip only the documented `body:` evidence prefix before this read.
pub(crate) async fn body_article_ids_many_in(
    tx: &Tx,
    body_ids: &[String],
) -> Result<std::collections::BTreeMap<String, String>, TxError> {
    if body_ids.len() > 64
        || body_ids
            .iter()
            .any(|id| !crate::intel_service::valid_id(id))
    {
        return Err(TxError::Abort("body_owner_key_bounds".into()));
    }
    #[derive(SurrealValue)]
    struct Owner {
        id: String,
        article: Option<String>,
    }
    let mut response=super::classifier_admission::tx_query(tx,"SELECT record::id(id) AS id,IF bytes::len(type::bytes(article_id))<=512 {article_id} ELSE {NONE} AS article FROM $records").bind(("records",body_ids.iter().map(|id|rid("article_body",id)).collect::<Vec<_>>())).await?.check()?;
    let rows: Vec<Owner> = take(&mut response, 0)?;
    let mut out = std::collections::BTreeMap::new();
    for row in rows {
        let article = row
            .article
            .ok_or_else(|| TxError::Abort("body_owner_id_bounds".into()))?;
        if out.insert(row.id, article).is_some() {
            return Err(TxError::Abort("duplicate_body_owner".into()));
        }
    }
    Ok(out)
}
/// A used annotation row must not change between captured matching and commit.
/// The guard is a storage field, not an annotation feature or model change.
pub(crate) async fn fence_body_candidates_in(
    tx: &Tx,
    heads: &[crate::storage::classifier_admission::BodyRevision],
) -> Result<bool, TxError> {
    if heads.len() > 64 {
        return Err(TxError::Abort("body_candidate_fence_limit".into()));
    }
    let mut identities = std::collections::BTreeSet::new();
    for head in heads {
        if let Some(revision) = &head.candidates_revision {
            let (feature, hash) = revision
                .split_once(':')
                .ok_or_else(|| TxError::Abort("invalid_body_candidate_revision".into()))?;
            if [feature, hash].iter().any(|value| {
                value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
            }) {
                return Err(TxError::Abort("invalid_body_candidate_revision".into()));
            }
            identities.insert((
                feature.to_owned(),
                hash.to_owned(),
                head.artifact_hash.clone(),
            ));
        }
    }
    if identities.is_empty() {
        return Ok(true);
    }
    let mut sql = String::new();
    for index in 0..identities.len() {
        sql.push_str(&format!("RETURN count(UPDATE $feature{index} SET atlas_publication_guard=(atlas_publication_guard ?? 0)+1 WHERE prose_hash=$prose{index} AND crypto::sha256(annotations_json)=$hash{index} RETURN VALUE id)=1;"));
    }
    let mut query = super::classifier_admission::tx_query(tx, sql);
    for (index, (feature, hash, prose)) in identities.into_iter().enumerate() {
        query = query
            .bind((format!("feature{index}"), rid("nlp_feature", &feature)))
            .bind((format!("hash{index}"), hash))
            .bind((format!("prose{index}"), prose));
    }
    let mut response = query.await?.check()?;
    for index in 0..response.num_statements() {
        if take::<Option<bool>>(&mut response, index)? != Some(true) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Exact bounded body material, grouped by encoded size before transfer.
/// Missing or invalid candidate material remains unavailable in NLP-on mode.
pub(crate) async fn clean_body_materials_many_in(
    tx: &Tx,
    heads: &std::collections::BTreeMap<String, crate::storage::classifier_admission::BodyRevision>,
) -> Result<
    (
        std::collections::BTreeMap<String, (CleanBody, Vec<crate::nlp::EntityCandidate>)>,
        std::collections::BTreeSet<String>,
    ),
    TxError,
> {
    if heads.len() > 64 {
        return Err(TxError::Abort("body_material_key_bound".into()));
    }
    let entries: Vec<_> = heads.iter().collect();
    if entries.is_empty() {
        return Ok((
            std::collections::BTreeMap::new(),
            std::collections::BTreeSet::new(),
        ));
    }
    let mut sql = String::new();
    for index in 0..entries.len() {
        sql.push_str(&format!(
            "RETURN bytes::len(type::bytes((SELECT VALUE text FROM ONLY $artifact{index})??''));"
        ));
    }
    let mut query = super::classifier_admission::tx_query(tx, sql);
    for (index, (_, head)) in entries.iter().enumerate() {
        query = query.bind((
            format!("artifact{index}"),
            rid("nlp_artifact", &head.artifact_hash),
        ));
    }
    let mut response = query.await?.check()?;
    let mut oversized = std::collections::BTreeSet::new();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut group = Vec::new();
    let mut group_bytes = 0usize;
    for (index, entry) in entries.iter().enumerate() {
        let bytes = take::<Option<i64>>(&mut response, index)?
            .ok_or_else(|| TxError::Abort("missing_body_size".into()))?;
        let bytes =
            usize::try_from(bytes).map_err(|_| TxError::Abort("negative_body_size".into()))?;
        let wire = bytes
            .checked_add(2)
            .and_then(|bytes| bytes.checked_div(3))
            .and_then(|bytes| bytes.checked_mul(4))
            .and_then(|bytes| bytes.checked_add(256 * 1024))
            .ok_or_else(|| TxError::Abort("body_material_size_overflow".into()))?;
        if wire > super::classifier_admission::MAX_RESPONSE_BYTES {
            oversized.insert(entry.0.clone());
            continue;
        }
        if !group.is_empty() && group_bytes + wire > super::classifier_admission::MAX_RESPONSE_BYTES
        {
            groups.push(std::mem::take(&mut group));
            group_bytes = 0;
        }
        group.push(index);
        group_bytes += wire;
    }
    if !group.is_empty() {
        groups.push(group);
    }
    let mut out = std::collections::BTreeMap::new();
    for group in groups {
        let mut sql = String::new();
        for index in &group {
            sql.push_str(&format!("SELECT VALUE encoding::base64::encode(type::bytes(text),true) FROM ONLY $artifact{index};"));
            if entries[*index].1.candidates_revision.is_some() {
                sql.push_str(&format!("SELECT VALUE IF bytes::len(type::bytes(annotations_json))<=131072 {{encoding::base64::encode(type::bytes(annotations_json),true)}} ELSE {{NONE}} FROM ONLY $feature{index};"));
            } else {
                sql.push_str("RETURN NONE;");
            }
        }
        let mut query = super::classifier_admission::tx_query(tx, sql);
        for index in &group {
            let head = entries[*index].1;
            query = query.bind((
                format!("artifact{index}"),
                rid("nlp_artifact", &head.artifact_hash),
            ));
            if let Some(revision) = &head.candidates_revision {
                let (feature, _) = revision
                    .split_once(':')
                    .ok_or_else(|| TxError::Abort("body_candidate_identity".into()))?;
                query = query.bind((format!("feature{index}"), rid("nlp_feature", feature)));
            }
        }
        let mut response = query.await?.check()?;
        for (offset, index) in group.into_iter().enumerate() {
            let (article, head) = entries[index];
            let Some(encoded) = take::<Option<String>>(&mut response, offset * 2)? else {
                continue;
            };
            let bytes = super::classifier_admission::decode_wire_json(
                &encoded,
                super::classifier_admission::MAX_RESPONSE_BYTES,
            )
            .map_err(|_| TxError::Abort("body_material_base64".into()))?;
            let text = String::from_utf8(bytes)
                .map_err(|_| TxError::Abort("body_material_utf8".into()))?;
            if text.trim().is_empty() || crate::nlp::cache::hash(&text) != head.artifact_hash {
                continue;
            }
            let encoded: Option<String> = take(&mut response, offset * 2 + 1)?;
            let candidates = if crate::nlp::configured_mode() == crate::nlp::Mode::Off {
                Vec::new()
            } else {
                let Some(encoded) = encoded else {
                    continue;
                };
                let bytes = super::classifier_admission::decode_wire_json(&encoded, 131072)
                    .map_err(|_| TxError::Abort("body_candidates_base64".into()))?;
                let annotation: crate::nlp::Annotation = serde_json::from_slice(&bytes)
                    .map_err(|_| TxError::Abort("body_candidates_json".into()))?;
                let source = crate::nlp::Source::derived(&head.source_id, &head.revision, &text);
                if annotation.status != crate::nlp::Status::Complete
                    || !annotation.validate(&source)
                    || annotation.feature_key
                        != crate::nlp::cache::feature_key(&source, &source.context())
                {
                    continue;
                }
                crate::nlp::entity_candidates(&source, &annotation)
            };
            out.insert(
                article.clone(),
                (
                    CleanBody {
                        source_id: head.source_id.clone(),
                        revision: head.revision.clone(),
                        artifact_hash: head.artifact_hash.clone(),
                        text,
                    },
                    candidates,
                ),
            );
        }
    }
    Ok((out, oversized))
}
