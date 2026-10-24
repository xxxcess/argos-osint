//! Insight reads on the async repository, for the CLI and TUI
//! (`argos insights`, the `insight_metadata` of `argos recall`). Same JSON
//! shapes as the SQLite `Store::search_insights`, `insight_for_memory` and
//! `brain_query::insight_metadata`.
//!
//! SurrealDB has no `recon_entities` table, so an insight's entity is its
//! `entity_id` and the entity filter matches that id only.

use anyhow::{anyhow, Result};
use surrealdb::types::SurrealValue;

use super::jobs::rid;
use super::memory::memory_revision;
use super::repo::Repository;
use crate::brain_query::InsightMetadata;
use crate::recon::{InsightSource, InsightView};

#[derive(Debug, Clone, SurrealValue)]
struct DerivationRow {
    id: String,
    insight_kind: String,
    assessment_json: String,
    revision: String,
    snapshot_id: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct SnapshotRow {
    owner_kind: String,
    owner_id: String,
    created_at: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct OwnerRow {
    lifecycle: String,
    active_snapshot_id: String,
}

/// The memory's current insight derivation: the owner's active snapshot
/// first, then the newest. A memory edited since derivation reads as
/// `user_edited` and unassessed.
pub async fn insight_metadata(
    repo: &Repository,
    memory_id: &str,
) -> Result<Option<InsightMetadata>> {
    let db = repo.db();
    let mut r = db
        .query(
            "SELECT VALUE text FROM ONLY $m;
             SELECT meta::id(id) AS id, insight_kind, assessment_json, revision, snapshot_id
               FROM insight_derivation
              WHERE memory_id = $id AND status NOTINSIDE ['deleted', 'retired'];",
        )
        .bind(("m", rid("memory", memory_id)))
        .bind(("id", memory_id.to_string()))
        .await?
        .check()?;
    let text: Option<String> = r.take(0usize)?;
    let Some(text) = text else {
        return Ok(None);
    };
    let derivations: Vec<DerivationRow> = r.take(1usize)?;
    // (is active, snapshot created_at, snapshot id, lifecycle, derivation)
    let mut best: Option<(bool, String, String, String, DerivationRow)> = None;
    for d in derivations {
        let mut r = db
            .query("SELECT owner_kind, owner_id, created_at FROM ONLY $s")
            .bind(("s", rid("insight_snapshot", &d.snapshot_id)))
            .await?
            .check()?;
        let Some(snap): Option<SnapshotRow> = r.take(0usize)? else {
            continue;
        };
        let mut r = db
            .query("SELECT lifecycle, active_snapshot_id FROM ONLY type::record('insight_owner_state', [$k, $o])")
            .bind(("k", snap.owner_kind.clone()))
            .bind(("o", snap.owner_id.clone()))
            .await?
            .check()?;
        let Some(owner): Option<OwnerRow> = r.take(0usize)? else {
            continue;
        };
        let active = owner.active_snapshot_id == d.snapshot_id;
        let lifecycle = if active {
            owner.lifecycle
        } else {
            "previous".into()
        };
        let key = (active, snap.created_at, d.snapshot_id.clone());
        let better = match &best {
            None => true,
            Some((a, c, s, _, _)) => key > (*a, c.clone(), s.clone()),
        };
        if better {
            best = Some((key.0, key.1, key.2, lifecycle, d));
        }
    }
    let Some((_, _, snapshot_id, lifecycle, d)) = best else {
        return Ok(None);
    };
    let current_revision = memory_revision(&text);
    let mut assessment: crate::investigation::insights::Assessment =
        serde_json::from_str(&d.assessment_json)
            .map_err(|e| anyhow!("derivation {} assessment: {e}", d.id))?;
    if current_revision != d.revision {
        assessment.fidelity = "user_edited".into();
        assessment.relation = "unassessed".into();
        assessment.threshold_passed = false;
    }
    Ok(Some(InsightMetadata {
        insight_kind: d.insight_kind,
        assessment,
        lifecycle,
        snapshot_id,
        revision: current_revision,
    }))
}

#[derive(Debug, Clone, SurrealValue)]
struct ClaimRow {
    fingerprint: String,
    memory_id: String,
    entity_id: String,
    predicate: String,
    object_value: String,
    topic: String,
    classification: String,
    confidence: f64,
}

#[derive(Debug, Clone, SurrealValue)]
struct SourceRow {
    thread_id: Option<String>,
    run_id: Option<String>,
    answer_id: String,
    call_id: String,
    source_url: Option<String>,
    deleted_origin: bool,
    published_at: String,
}

const CLAIM: &str = "meta::id(id) AS fingerprint, memory_id, entity_id, predicate, \
    object_value, topic, classification, confidence";

async fn view(repo: &Repository, c: ClaimRow) -> Result<InsightView> {
    let mut r = repo
        .db()
        .query(
            "SELECT thread_id, run_id, answer_id, call_id, source_url, deleted_origin, published_at
               FROM insight_source WHERE fingerprint = $fp AND deleted_origin = false;
             SELECT VALUE IF left_fingerprint = $fp THEN right_fingerprint ELSE left_fingerprint END
               FROM insight_relation WHERE left_fingerprint = $fp OR right_fingerprint = $fp;",
        )
        .bind(("fp", c.fingerprint.clone()))
        .await?
        .check()?;
    let sources: Vec<SourceRow> = r.take(0usize)?;
    let related: Vec<String> = r.take(1usize)?;
    Ok(InsightView {
        metadata: insight_metadata(repo, &c.memory_id).await?,
        memory_id: c.memory_id,
        entity: c.entity_id,
        predicate: c.predicate,
        object_value: c.object_value,
        topic: c.topic,
        classification: c.classification,
        confidence: c.confidence,
        sources: sources
            .into_iter()
            .map(|s| InsightSource {
                thread_id: s.thread_id,
                run_id: s.run_id,
                answer_id: s.answer_id,
                call_id: s.call_id,
                source_url: s.source_url,
                deleted_origin: s.deleted_origin,
                published_at: s.published_at,
            })
            .collect(),
        related,
    })
}

/// The insight that produced this memory, if any.
pub async fn insight_for_memory(repo: &Repository, memory_id: &str) -> Result<Option<InsightView>> {
    let mut r = repo
        .db()
        .query(format!(
            "SELECT {CLAIM} FROM insight_claim WHERE memory_id = $m LIMIT 1"
        ))
        .bind(("m", memory_id.to_string()))
        .await?
        .check()?;
    let claims: Vec<ClaimRow> = r.take(0usize)?;
    match claims.into_iter().next() {
        Some(c) => Ok(Some(view(repo, c).await?)),
        None => Ok(None),
    }
}

/// Insights whose entity id and topic contain the given text
/// (case-insensitive, empty matches all), most recently updated first.
pub async fn search_insights(
    repo: &Repository,
    entity: &str,
    topic: &str,
) -> Result<Vec<InsightView>> {
    let mut r = repo
        .db()
        .query(format!(
            "SELECT {CLAIM}, updated_at FROM insight_claim
              WHERE string::contains(string::lowercase(entity_id), $e)
                AND string::contains(string::lowercase(topic), $t)
              ORDER BY updated_at DESC"
        ))
        .bind(("e", entity.to_lowercase()))
        .bind(("t", topic.to_lowercase()))
        .await?
        .check()?;
    let claims: Vec<ClaimRow> = r.take(0usize)?;
    let mut out = Vec::with_capacity(claims.len());
    for c in claims {
        out.push(view(repo, c).await?);
    }
    Ok(out)
}
