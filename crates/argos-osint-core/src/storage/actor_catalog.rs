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
    let ids: Vec<String> = take(&mut r, 0)?;
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
    tokio::task::spawn_blocking(move || {
        let actors: Vec<Actor> = rows
            .into_iter()
            .map(|s| serde_json::from_str(&s))
            .collect::<std::result::Result<_, _>>()?;
        Matcher::new(actors)
    })
    .await?
}

/// Current catalog actors for all article consumers, including before NLP
/// metadata is available. None preserves a pre-catalog home's legacy view.
pub async fn article_actors(
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
    if actors.is_empty() {
        bail!("actor catalog cannot be empty");
    }
    let fingerprint = fingerprint.to_owned();
    let owner = format!(
        "{}:{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    );
    let now = chrono::Utc::now().to_rfc3339();
    repo.db().query("BEGIN; IF (SELECT VALUE status FROM ONLY brain_actor_seed:catalog) = 'running' AND (SELECT VALUE lease_until FROM ONLY brain_actor_seed:catalog) > $now { THROW 'actor seed is already running'; }; UPSERT brain_actor_seed:catalog SET status = 'running', fingerprint = $hash, processed = 0, total = $total, aliases = 0, owner = $owner, lease_until = $lease, updated_at = $now; COMMIT;")
        .bind(("now", now.clone())).bind(("hash", fingerprint.clone())).bind(("total", actors.len())).bind(("owner", owner.clone()))
        .bind(("lease", (chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339())).await?.check()?;
    let result = ingest(repo, &actors, &owner, cancel, &report).await;
    if result.is_err() {
        repo.db().query("UPDATE brain_actor_seed:catalog SET status = 'failed', lease_until = '' WHERE owner = $owner RETURN NONE")
            .bind(("owner", owner)).await?.check()?;
    }
    result
}

async fn ingest(
    repo: &Repository,
    actors: &[Actor],
    owner: &str,
    cancel: &AtomicBool,
    report: &impl Fn(Progress),
) -> Result<Progress> {
    let mut progress = Progress {
        total: actors.len(),
        ..Progress::default()
    };
    report(progress.clone());
    for chunk in actors.chunks(BATCH) {
        if cancel.load(Ordering::Acquire) {
            bail!("actor seeding cancelled; committed batches can be retried");
        }
        let now = chrono::Utc::now().to_rfc3339();
        let mut actor_rows = Vec::new();
        let mut memories = Vec::new();
        let mut aliases = Vec::new();
        let mut graph = Vec::new();
        for actor in chunk {
            let provenance = serde_json::to_string(&actor.metadata)?;
            actor_rows.push(ActorRow {
                id: rid("brain_actor", &actor.id),
                subtype: actor.subtype.clone(),
                display_name: actor.name.clone(),
                identity_state: "resolved".into(),
                scope_key: actor_catalog::SCOPE.into(),
                revision: 1,
                provenance_json: provenance.clone(),
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
            for alias in &actor.aliases {
                let normalized = brain_actors::normalize_alias(alias);
                if normalized.chars().count() < 3 || !keys.insert(normalized.clone()) {
                    continue;
                }
                aliases.push(AliasRow {
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
                    alias_type: "name".into(),
                    provenance_json: provenance.clone(),
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
        let data = Arc::new((actor_rows, memories, aliases, graph));
        run_tx(repo.db(), RetryPolicy::default(), |tx| {
            let data = data.clone(); let owner = owner.to_owned(); let next = next.clone(); let now = now.clone();
            async move {
                run(&tx, tx.query("IF (SELECT VALUE owner FROM ONLY brain_actor_seed:catalog) != $owner { THROW 'actor seed lease changed'; }; INSERT IGNORE INTO brain_actor $actors RETURN NONE; INSERT IGNORE INTO memory $memories RETURN NONE; INSERT IGNORE INTO brain_actor_alias $aliases RETURN NONE; INSERT IGNORE INTO brain_graph_node $graph RETURN NONE; UPDATE brain_actor_seed:catalog SET processed = $done, aliases = $alias_count, updated_at = $now, lease_until = $lease RETURN NONE;")
                    .bind(("owner", owner)).bind(("actors", data.0.clone())).bind(("memories", data.1.clone())).bind(("aliases", data.2.clone())).bind(("graph", data.3.clone()))
                    .bind(("done", next.processed)).bind(("alias_count", next.aliases)).bind(("now", now)).bind(("lease", (chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339()))).await?;
                Ok(())
            }
        }).await.map_err(|e| anyhow::anyhow!("actor seed batch: {e}"))?;
        progress = next;
        report(progress.clone());
    }
    repo.db().query("BEGIN; IF (SELECT VALUE owner FROM ONLY brain_actor_seed:catalog) != $owner OR (SELECT VALUE processed FROM ONLY brain_actor_seed:catalog) != $total { THROW 'actor seed completion lost ownership'; }; UPDATE brain_actor_seed:catalog SET status = 'complete', lease_until = '' RETURN NONE; COMMIT;")
        .bind(("owner", owner.to_owned())).bind(("total", progress.total)).await?.check()?;
    progress.complete = true;
    report(progress.clone());
    Ok(progress)
}
