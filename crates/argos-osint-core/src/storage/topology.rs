//! Brain topology on the async repository (migration phase 5):
//! `Store::topology_snapshot` for the Claim, Actor, Inference, Actor-group,
//! Article, Topic, Source and Link/Related seeds, walked breadth-first to
//! depth 3 with the SQLite node, edge, receipt and admission rules. Admission,
//! findings and the deterministic neighbourhood brief reuse the SQLite code
//! (`brain_topology::admit`, `brain_neighborhood_brief::build`).
//!
//! Gaps (see acceptance.md): the retained Intel display seeds
//! (`RetainedClaim`, `RetainedInference`, `RetainedActor`) are refused, since
//! the Intel brief publications are still SQLite; the `current_metadata_sql`
//! article-revision fence is not applied to derivation provenance or cycle
//! scope; memory nodes carry no revision (SurrealDB memories have none);
//! reads are not one snapshot transaction.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use anyhow::{bail, ensure, Result};
use serde_json::Value;
use surrealdb::types::SurrealValue;

use super::provenance::ACTOR_MENTIONS_SURQL;
use super::publisher::CHANGED_COUNTER;
use super::repo::Repository;
use super::{atlas, counters, recon};
use crate::brain::{Memory, MemorySource};
use crate::brain_neighborhood_brief::{self as brief, BriefInput, MAX_PAYLOAD_READS};
use crate::brain_topology::{
    admit, edge, edge_priority, node, record_edge, source_node, topic_node, CycleScope,
    TopologyEdge, TopologyEdgeKind, TopologyNode, TopologyNodeKind, TopologyOptions, TopologySeed,
    TopologySnapshot,
};
use crate::memory_metadata::{EndpointRef, MemoryKind, MemoryPayload};

type Adjacency = (TopologyNode, TopologyEdge);

/// Derivation statuses that no longer support a node or its scope.
const DEAD: [&str; 3] = ["deleted", "retired", "rejected"];

#[derive(Debug, Clone, SurrealValue)]
struct MemoryRow {
    text: String,
    category: String,
    pinned: bool,
    lifecycle: String,
    memory_kind: String,
    payload_json: String,
    source_json: String,
    created_at: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct ActorRow {
    subtype: String,
    display_name: String,
    identity_state: String,
    revision: i64,
}

#[derive(Debug, Clone, SurrealValue)]
struct ActorRefRow {
    memory_id: String,
    role: String,
    binding_state: String,
    provenance_json: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct PayloadRow {
    id: String,
    payload_json: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct MentionRow {
    origin_kind: String,
    origin_scope: String,
    origin_id: String,
    origin_revision: String,
    spans_json: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct DerivationRow {
    id: String,
    item_json: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct EndpointRow {
    endpoint_kind: String,
    ref_id: String,
    origin: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct EndpointMemoryRow {
    memory_id: String,
    origin: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct MemoryRefRow {
    actor_id: String,
    role: String,
    binding_state: String,
    provenance_json: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct DerivationFull {
    id: String,
    snapshot_id: String,
    memory_id: Option<String>,
    insight_kind: String,
    item_json: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct EvidenceRow {
    derivation_id: String,
    source_id: String,
    source_revision: String,
    memory_id: Option<String>,
    premise_id: String,
    evidence_json: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct SnapshotRow {
    owner_kind: String,
    owner_id: String,
    readiness_json: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct ClaimRow {
    fingerprint: String,
    memory_id: String,
    topic: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct SourceRow {
    fingerprint: String,
    thread_id: Option<String>,
    run_id: Option<String>,
    answer_id: String,
    call_id: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct ArticleMentionRow {
    actor_id: String,
    origin_revision: String,
    spans_json: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct RelatedRow {
    left_id: String,
    right_id: String,
    left_revision: String,
    right_revision: String,
    shared_json: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct ArticleKeyRow {
    run_id: String,
    article_id: String,
    category: String,
}

struct Query<'a> {
    repo: &'a Repository,
    limit: usize,
    scope: &'a CycleScope,
    exhausted: bool,
    gaps: BTreeSet<String>,
    remaining_rows: usize,
    memories: HashMap<String, Option<MemoryRow>>,
}

impl Query<'_> {
    /// `Query::rows` bound: one extra row marks truncation, and every branch
    /// spends the shared row budget.
    fn bound<T>(&mut self, mut rows: Vec<T>) -> Vec<T> {
        if self.remaining_rows == 0 {
            self.exhausted = false;
            return Vec::new();
        }
        let limit = self.limit.min(self.remaining_rows);
        if rows.len() > limit {
            self.exhausted = false;
            rows.truncate(limit);
        }
        self.remaining_rows = self.remaining_rows.saturating_sub(rows.len());
        rows
    }

    fn cycle(&self) -> Option<&str> {
        match self.scope {
            CycleScope::ThisCycle(cycle) => Some(cycle.as_str()),
            CycleScope::AllRetained => None,
        }
    }

    async fn memory(&mut self, id: &str) -> Result<Option<MemoryRow>> {
        if let Some(row) = self.memories.get(id) {
            return Ok(row.clone());
        }
        let mut r = self
            .repo
            .db()
            .query(
                "SELECT text, category, pinned, lifecycle, memory_kind, payload_json, source_json,
                        time::format(created_at, '%+') AS created_at
                 FROM ONLY type::record('memory', $id)",
            )
            .bind(("id", id.to_string()))
            .await?
            .check()?;
        let row: Option<MemoryRow> = r.take(0)?;
        self.memories.insert(id.to_string(), row.clone());
        Ok(row)
    }

    async fn payload(&mut self, id: &str) -> Result<Option<MemoryPayload>> {
        Ok(self
            .memory(id)
            .await?
            .filter(|m| !m.payload_json.is_empty())
            .and_then(|m| serde_json::from_str(&m.payload_json).ok()))
    }

    /// `brain_actors::resolve` on `brain_actor_redirect`.
    async fn resolve(&self, actor_id: &str) -> Result<String> {
        let mut id = actor_id.to_string();
        for _ in 0..64 {
            let mut r = self
                .repo
                .db()
                .query("SELECT VALUE to_actor FROM ONLY type::record('brain_actor_redirect', $id)")
                .bind(("id", id.clone()))
                .await?
                .check()?;
            match r.take::<Option<String>>(0)? {
                Some(next) => id = next,
                None => return Ok(id),
            }
        }
        bail!("actor redirect chain from {actor_id} does not terminate")
    }

    async fn memory_node(&mut self, id: &str) -> Result<Option<TopologyNode>> {
        let Some(memory) = self.memory(id).await? else {
            return Ok(None);
        };
        if matches!(memory.lifecycle.as_str(), "retired" | "superseded") {
            return Ok(None);
        }
        let stored_kind = memory.memory_kind.clone();
        let kind = match MemoryKind::parse(&stored_kind) {
            Some(MemoryKind::Actor) => {
                let canonical = self.resolve(id).await?;
                let mut r = self
                    .repo
                    .db()
                    .query(
                        "SELECT subtype, display_name, identity_state, revision
                         FROM ONLY type::record('brain_actor', $id)",
                    )
                    .bind(("id", canonical.clone()))
                    .await?
                    .check()?;
                let Some(actor) = r.take::<Option<ActorRow>>(0)? else {
                    return Ok(None);
                };
                let mut out = node(
                    canonical.clone(),
                    TopologyNodeKind::Entity,
                    actor.display_name,
                );
                out.subtype = actor.subtype;
                out.status = actor.identity_state;
                out.member_ids.push(canonical.clone());
                let mut aliases = VecDeque::from([canonical.clone()]);
                while let Some(survivor) = aliases.pop_front() {
                    let mut r = self
                        .repo
                        .db()
                        .query(
                            "SELECT VALUE record::id(id) FROM brain_actor_redirect
                             WHERE to_actor = $a",
                        )
                        .bind(("a", survivor))
                        .await?
                        .check()?;
                    let mut olds: Vec<String> = r.take(0)?;
                    olds.sort();
                    for old in self.bound(olds) {
                        if out.member_ids.contains(&old) {
                            continue;
                        }
                        if out.member_ids.len() >= self.limit {
                            self.exhausted = false;
                            break;
                        }
                        out.member_ids.push(old.clone());
                        aliases.push_back(old);
                    }
                }
                out.member_ids.sort();
                out.revision = actor.revision.to_string();
                out.detail = format!("Canonical actor: {canonical}\nIdentity: {}", out.status);
                out.topology_seed = Some(TopologySeed::Actor(canonical));
                return Ok(Some(out));
            }
            Some(MemoryKind::Inference) => TopologyNodeKind::Inference,
            Some(MemoryKind::Claim) => TopologyNodeKind::Claim,
            _ if matches!(stored_kind.to_lowercase().as_str(), "fact" | "claim") => {
                TopologyNodeKind::Claim
            }
            _ => {
                self.gaps
                    .insert(format!("{id}: unsupported memory kind {stored_kind}"));
                return Ok(None);
            }
        };
        let mut out = node(id.into(), kind, memory.text.clone());
        out.subtype = memory.category.clone();
        out.published_at = memory.created_at.clone();
        out.topology_seed = Some(if kind == TopologyNodeKind::Inference {
            TopologySeed::Inference(id.into())
        } else {
            TopologySeed::Claim(id.into())
        });
        match self.payload(id).await? {
            Some(MemoryPayload::Claim(p)) => {
                out.status = if p.assessment.trim().is_empty()
                    || p.assessment.trim().eq_ignore_ascii_case("unassessed")
                {
                    "unassessed".into()
                } else {
                    "extraction assessed".into()
                };
                out.passages = p.passages;
            }
            Some(MemoryPayload::Inference(p)) => {
                out.status = p.support_state;
                out.detail = format!(
                    "{}\nReasoning: {}\nLimitations: {}",
                    out.label, p.reasoning, p.limitations
                );
            }
            _ => {}
        }
        if kind == TopologyNodeKind::Inference {
            let mut r = self
                .repo
                .db()
                .query(
                    "SELECT record::id(id) AS id, item_json FROM insight_derivation
                     WHERE memory_id = $m AND status NOTINSIDE $dead",
                )
                .bind(("m", id.to_string()))
                .bind(("dead", dead()))
                .await?
                .check()?;
            let mut rows: Vec<DerivationRow> = r.take(0)?;
            rows.sort_by(|a, b| a.id.cmp(&b.id));
            for d in self.bound(rows) {
                if let Ok(item) = serde_json::from_str::<Value>(&d.item_json) {
                    out.detail = format!(
                        "{}\nReasoning: {}\nLimitations: {}",
                        out.label,
                        item["rationale"].as_str().unwrap_or(""),
                        item["limitations"]
                    );
                    out.passages.push(d.id);
                }
            }
        }
        Ok(Some(out))
    }

    async fn article_node(
        &mut self,
        run: &str,
        id: &str,
        expected: &str,
    ) -> Result<Option<TopologyNode>> {
        let Some(article) = atlas::article(self.repo.db(), run, id).await? else {
            return Ok(None);
        };
        let revision = crate::atlas_metadata::input_hash(&article);
        if !expected.is_empty() && expected != revision {
            self.gaps
                .insert(format!("article:{run}/{id}: retained revision changed"));
            return Ok(None);
        }
        let mut out = node(
            format!("article:{run}/{id}"),
            TopologyNodeKind::Article,
            article.title,
        );
        out.detail = format!("{}\n{}", article.description, article.url);
        out.subtype = article.category;
        out.article_id = id.into();
        out.run_id = run.into();
        out.cycle = Some(run.into());
        out.published_at = article.published_at;
        out.revision = revision.clone();
        out.topology_seed = Some(TopologySeed::Article {
            run_id: run.into(),
            article_id: id.into(),
            revision,
            originating_memory: None,
        });
        Ok(Some(out))
    }

    async fn endpoint_node(&mut self, endpoint: &EndpointRef) -> Result<Option<TopologyNode>> {
        match endpoint {
            EndpointRef::Memory { memory_id, kind } => {
                let result = Box::pin(self.memory_node(memory_id)).await?;
                let matches = result.as_ref().is_some_and(|n| {
                    matches!(
                        (kind, n.kind),
                        (MemoryKind::Actor, TopologyNodeKind::Entity)
                            | (MemoryKind::Claim, TopologyNodeKind::Claim)
                            | (MemoryKind::Inference, TopologyNodeKind::Inference)
                    )
                });
                if !matches {
                    self.gaps.insert(format!(
                        "{memory_id}: missing or mismatched typed endpoint ({kind})"
                    ));
                }
                Ok(if matches { result } else { None })
            }
            EndpointRef::Article {
                run_id,
                article_id,
                revision,
            } => self.article_node(run_id, article_id, revision).await,
            EndpointRef::Source {
                origin,
                source_id,
                revision,
            } => Ok(Some(source_node(origin, source_id, revision))),
        }
    }

    /// Live derivations or insight sources tie `memory` (directly, or for an
    /// actor through a mention or an actor ref) to `cycle`.
    async fn memory_in_cycle(&mut self, memory: &str, cycle: &str) -> Result<bool> {
        if self.claimed_in_cycle(memory, cycle).await? {
            return Ok(true);
        }
        let mut r = self
            .repo
            .db()
            .query(
                "SELECT VALUE count() FROM brain_actor_mention
                   WHERE actor_id = $m AND origin_kind = 'article' AND origin_scope = $c GROUP ALL;
                 SELECT VALUE memory_id FROM brain_memory_actor_ref WHERE actor_id = $m;",
            )
            .bind(("m", memory.to_string()))
            .bind(("c", cycle.to_string()))
            .await?
            .check()?;
        let mentions: Option<i64> = r.take(0)?;
        if mentions.unwrap_or(0) > 0 {
            return Ok(true);
        }
        let refs: BTreeSet<String> = r.take::<Vec<String>>(1)?.into_iter().collect();
        for m in refs {
            if self.claimed_in_cycle(&m, cycle).await? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn claimed_in_cycle(&mut self, memory: &str, cycle: &str) -> Result<bool> {
        let mut r = self
            .repo
            .db()
            .query(
                "LET $owners = (SELECT VALUE type::record('insight_snapshot', snapshot_id).owner_id
                   FROM insight_derivation WHERE memory_id = $m AND status NOTINSIDE $dead);
                 SELECT VALUE count() FROM atlas_article
                   WHERE run_id = $c AND article_id INSIDE $owners GROUP ALL;
                 LET $fps = (SELECT VALUE record::id(id) FROM insight_claim WHERE memory_id = $m);
                 SELECT VALUE count() FROM insight_source
                   WHERE fingerprint INSIDE $fps AND run_id = $c AND deleted_origin = false
                   GROUP ALL;",
            )
            .bind(("m", memory.to_string()))
            .bind(("c", cycle.to_string()))
            .bind(("dead", dead()))
            .await?
            .check()?;
        let derived: Option<i64> = r.take(1)?;
        let sourced: Option<i64> = r.take(3)?;
        Ok(derived.unwrap_or(0) > 0 || sourced.unwrap_or(0) > 0)
    }

    async fn evidence_in_cycle(
        &mut self,
        source: &str,
        revision: &str,
        cycle: &str,
    ) -> Result<bool> {
        let mut r = self
            .repo
            .db()
            .query(
                "LET $ds = (SELECT VALUE derivation_id FROM insight_derivation_evidence
                   WHERE source_id = $s AND source_revision = $rev);
                 LET $owners = (SELECT VALUE type::record('insight_snapshot', snapshot_id).owner_id
                   FROM insight_derivation WHERE record::id(id) INSIDE $ds AND status NOTINSIDE $dead);
                 SELECT VALUE count() FROM atlas_article
                   WHERE run_id = $c AND article_id INSIDE $owners GROUP ALL;",
            )
            .bind(("s", source.to_string()))
            .bind(("rev", revision.to_string()))
            .bind(("c", cycle.to_string()))
            .bind(("dead", dead()))
            .await?
            .check()?;
        Ok(r.take::<Option<i64>>(2)?.unwrap_or(0) > 0)
    }

    async fn in_scope(&mut self, n: &TopologyNode) -> Result<bool> {
        let Some(cycle) = self.cycle().map(str::to_string) else {
            return Ok(true);
        };
        if n.kind == TopologyNodeKind::Topic {
            return Ok(true);
        }
        if let Some(run) = &n.cycle {
            return Ok(*run == cycle);
        }
        if let Some(TopologySeed::Source {
            origin,
            source_id,
            revision,
        }) = &n.topology_seed
        {
            if origin == "evidence" {
                return self.evidence_in_cycle(source_id, revision, &cycle).await;
            }
            return Ok(false);
        }
        self.memory_in_cycle(&n.id, &cycle).await
    }

    async fn link(
        &mut self,
        id: &str,
    ) -> Result<Option<(TopologyNode, TopologyNode, TopologyEdge)>> {
        if self.memory(id).await?.map(|m| m.lifecycle).as_deref() != Some("active") {
            return Ok(None);
        }
        let (source, target, mut e) = match self.payload(id).await? {
            Some(MemoryPayload::Link(p)) => {
                let mut e = edge(id.into(), "", "", TopologyEdgeKind::Link, id.into());
                e.label = p.relation;
                e.rationale = p.rationale;
                e.status = if e.label == "mentioned_in" {
                    "observed"
                } else {
                    "assessed"
                }
                .into();
                e.receipts.extend(p.receipts);
                (p.source, p.target, e)
            }
            Some(MemoryPayload::Related(p)) => {
                let mut e = edge(id.into(), "", "", TopologyEdgeKind::Related, id.into());
                e.label = "possible context".into();
                e.status = "tentative".into();
                e.rationale = format!(
                    "{}\n{}\nVerification gap: {}",
                    p.basis, p.interpretation, p.verification_gap
                );
                e.member_ids = p.shared_actors;
                e.receipts.extend(p.source_revisions);
                (p.source, p.target, e)
            }
            _ => return Ok(None),
        };
        let (Some(s), Some(t)) = (
            self.endpoint_node(&source).await?,
            self.endpoint_node(&target).await?,
        ) else {
            self.gaps
                .insert(format!("{id}: unresolved relationship endpoint"));
            return Ok(None);
        };
        e.from = s.id.clone();
        e.to = t.id.clone();
        Ok(Some((s, t, e)))
    }

    /// `ScopedRows::Link`: under a cycle, a Link/Related record needs another
    /// endpoint (not the node itself) in that cycle.
    async fn link_in_cycle(&mut self, link: &str, n: &TopologyNode, cycle: &str) -> Result<bool> {
        let mut r = self
            .repo
            .db()
            .query("SELECT endpoint_kind, ref_id, origin FROM brain_memory_endpoint WHERE memory_id = $m")
            .bind(("m", link.to_string()))
            .await?
            .check()?;
        let rows: Vec<EndpointRow> = r.take(0)?;
        for e in rows {
            if e.endpoint_kind == "memory" && (e.ref_id == n.id || n.member_ids.contains(&e.ref_id))
            {
                continue;
            }
            if n.kind == TopologyNodeKind::Article
                && e.endpoint_kind == "article"
                && e.origin == n.run_id
                && e.ref_id == n.article_id
            {
                continue;
            }
            if e.endpoint_kind == "article" && e.origin == cycle {
                return Ok(true);
            }
            if e.endpoint_kind == "memory" && self.memory_in_cycle(&e.ref_id, cycle).await? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `typed_links`: active Link/Related records with an endpoint on the
    /// node (its member ids for an Entity, its article or source otherwise),
    /// Links first.
    async fn typed_links(&mut self, n: &TopologyNode, out: &mut Vec<Adjacency>) -> Result<()> {
        let (kind, reference, origin) = match &n.topology_seed {
            Some(TopologySeed::Article {
                run_id, article_id, ..
            }) => ("article", article_id.clone(), run_id.clone()),
            Some(TopologySeed::Source {
                origin, source_id, ..
            }) => ("source", source_id.clone(), origin.clone()),
            _ if matches!(
                n.kind,
                TopologyNodeKind::Entity | TopologyNodeKind::Claim | TopologyNodeKind::Inference
            ) =>
            {
                ("memory", n.id.clone(), String::new())
            }
            _ => return Ok(()),
        };
        let references = if n.kind == TopologyNodeKind::Entity {
            n.member_ids.clone()
        } else {
            vec![reference]
        };
        for reference in references {
            let mut r = self
                .repo
                .db()
                .query(
                    "SELECT memory_id, origin FROM brain_memory_endpoint
                     WHERE endpoint_kind = $k AND ref_id = $r",
                )
                .bind(("k", kind.to_string()))
                .bind(("r", reference))
                .await?
                .check()?;
            let rows: Vec<EndpointMemoryRow> = r.take(0)?;
            let ids: BTreeSet<String> = rows
                .into_iter()
                .filter(|e| origin.is_empty() || e.origin == origin)
                .map(|e| e.memory_id)
                .collect();
            let mut links = Vec::new();
            for id in ids {
                let Some(m) = self.memory(&id).await? else {
                    continue;
                };
                if m.lifecycle != "active" || !matches!(m.memory_kind.as_str(), "link" | "related")
                {
                    continue;
                }
                if let Some(cycle) = self.cycle().map(str::to_string) {
                    if !self.link_in_cycle(&id, n, &cycle).await? {
                        continue;
                    }
                }
                links.push((m.memory_kind != "link", id));
            }
            links.sort();
            for (_, id) in self.bound(links) {
                if let Some((s, t, e)) = self.link(&id).await? {
                    if s.id == n.id {
                        out.push((t, e));
                    } else if t.id == n.id {
                        out.push((s, e));
                    }
                }
            }
        }
        Ok(())
    }

    async fn memory_scoped(&mut self, memory: &str) -> Result<bool> {
        match self.cycle().map(str::to_string) {
            Some(cycle) => self.memory_in_cycle(memory, &cycle).await,
            None => Ok(true),
        }
    }

    fn run_scoped(&self, run: &str) -> bool {
        self.cycle().is_none_or(|c| c == run)
    }

    /// Keep rows whose memory (selected by `key`) is in the cycle scope.
    async fn scoped<T>(&mut self, rows: Vec<T>, key: impl Fn(&T) -> &str) -> Result<Vec<T>> {
        if self.cycle().is_none() {
            return Ok(rows);
        }
        let mut kept = Vec::new();
        for row in rows {
            if self.memory_scoped(key(&row)).await? {
                kept.push(row);
            }
        }
        Ok(kept)
    }

    async fn derivations(
        &self,
        sql: &str,
        binds: Vec<(&'static str, Value)>,
    ) -> Result<Vec<DerivationFull>> {
        let mut q = self.repo.db().query(sql).bind(("dead", dead()));
        for (k, v) in binds {
            q = q.bind((k, v));
        }
        let mut r = q.await?.check()?;
        let mut rows: Vec<DerivationFull> = r.take(0)?;
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(rows)
    }

    async fn evidence_for(&self, derivations: Vec<String>) -> Result<Vec<EvidenceRow>> {
        if derivations.is_empty() {
            return Ok(Vec::new());
        }
        let mut r = self
            .repo
            .db()
            .query(
                "SELECT derivation_id, source_id, source_revision, memory_id, premise_id, evidence_json
                 FROM insight_derivation_evidence WHERE derivation_id INSIDE $d",
            )
            .bind(("d", derivations))
            .await?
            .check()?;
        Ok(r.take(0)?)
    }

    async fn snapshot_owner(&self, id: &str) -> Result<Option<SnapshotRow>> {
        let mut r = self
            .repo
            .db()
            .query("SELECT owner_kind, owner_id, readiness_json FROM ONLY type::record('insight_snapshot', $id)")
            .bind(("id", id.to_string()))
            .await?
            .check()?;
        Ok(r.take(0)?)
    }

    /// `source_snapshot_sql`: a scoped publication carries the sibling
    /// kind's source snapshot in its readiness receipt.
    async fn origin_snapshot(&self, snapshot: &str, kind: &str) -> Result<String> {
        let readiness = self
            .snapshot_owner(snapshot)
            .await?
            .and_then(|s| serde_json::from_str::<Value>(&s.readiness_json).ok());
        let carried = readiness
            .as_ref()
            .and_then(|r| r["carried_kinds"].as_array().cloned())
            .unwrap_or_default();
        Ok(carried
            .iter()
            .find(|c| c["kind"].as_str() == Some(kind))
            .and_then(|c| c["source_snapshot_id"].as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(snapshot)
            .to_string())
    }

    async fn article_runs(&self, article: &str) -> Result<Vec<String>> {
        let mut r = self
            .repo
            .db()
            .query("SELECT VALUE run_id FROM atlas_article WHERE article_id = $a")
            .bind(("a", article.to_string()))
            .await?
            .check()?;
        let mut runs: Vec<String> = r.take(0)?;
        runs.sort();
        runs.dedup();
        Ok(runs)
    }

    async fn claim_rows(&self, sql: &str, bind: Value) -> Result<Vec<ClaimRow>> {
        let mut r = self.repo.db().query(sql).bind(("v", bind)).await?.check()?;
        Ok(r.take(0)?)
    }

    async fn live_sources(&self, fingerprints: Vec<String>) -> Result<Vec<SourceRow>> {
        if fingerprints.is_empty() {
            return Ok(Vec::new());
        }
        let mut r = self
            .repo
            .db()
            .query(
                "SELECT fingerprint, thread_id, run_id, answer_id, call_id FROM insight_source
                 WHERE fingerprint INSIDE $f AND deleted_origin = false",
            )
            .bind(("f", fingerprints))
            .await?
            .check()?;
        Ok(r.take(0)?)
    }

    async fn actor_neighbours(&mut self, n: &TopologyNode, out: &mut Vec<Adjacency>) -> Result<()> {
        for actor_id in n.member_ids.clone() {
            let mut r = self
                .repo
                .db()
                .query(
                    "SELECT memory_id, role, binding_state, provenance_json
                     FROM brain_memory_actor_ref WHERE actor_id = $a",
                )
                .bind(("a", actor_id.clone()))
                .await?
                .check()?;
            let mut refs: Vec<ActorRefRow> = r.take(0)?;
            refs.sort_by(|a, b| (&a.memory_id, &a.role).cmp(&(&b.memory_id, &b.role)));
            let mut kept = Vec::new();
            for row in refs {
                let live = self.memory(&row.memory_id).await?.is_none_or(|m| {
                    m.lifecycle == "active"
                        && matches!(
                            m.memory_kind.as_str(),
                            "claim" | "atomic_claim" | "inference"
                        )
                });
                if live && self.memory_scoped(&row.memory_id).await? {
                    kept.push(row);
                }
            }
            for row in self.bound(kept) {
                if let Some(memory) = Box::pin(self.memory_node(&row.memory_id)).await? {
                    let mut e = edge(
                        format!("actor-ref:{}:{actor_id}:{}", row.memory_id, row.role),
                        &n.id,
                        &memory.id,
                        TopologyEdgeKind::Concerns,
                        row.provenance_json,
                    );
                    e.status = format!("recorded · {}", row.binding_state);
                    e.rationale = format!("Recorded actor role: {}", row.role);
                    e.member_ids.push(actor_id.clone());
                    out.push((memory, e));
                }
            }
            // Typed payload actors stay authoritative without a ref row.
            let mut r = self
                .repo
                .db()
                .query(
                    "SELECT record::id(id) AS id, payload_json FROM memory
                     WHERE lifecycle = 'active'
                       AND memory_kind INSIDE ['claim', 'atomic_claim', 'inference']
                       AND payload_json CONTAINS $a",
                )
                .bind(("a", actor_id.clone()))
                .await?
                .check()?;
            let rows: Vec<PayloadRow> = r.take(0)?;
            let mut typed = BTreeSet::new();
            for row in rows {
                let Ok(v) = serde_json::from_str::<Value>(&row.payload_json) else {
                    continue;
                };
                for reference in v["payload"]["actors"].as_array().into_iter().flatten() {
                    if reference["actor_id"].as_str() == Some(actor_id.as_str()) {
                        typed.insert((
                            row.id.clone(),
                            reference["role"]
                                .as_str()
                                .unwrap_or("mentioned")
                                .to_string(),
                            reference["binding_state"]
                                .as_str()
                                .unwrap_or("provisional")
                                .to_string(),
                        ));
                    }
                }
            }
            let kept = self.scoped(typed.into_iter().collect(), |t| &t.0).await?;
            for (mid, role, state) in self.bound(kept) {
                if let Some(memory) = Box::pin(self.memory_node(&mid)).await? {
                    let mut e = edge(
                        format!("actor-ref:{mid}:{actor_id}:{role}"),
                        &n.id,
                        &memory.id,
                        TopologyEdgeKind::Concerns,
                        format!("memory_metadata:{mid}@{}", memory.revision),
                    );
                    e.status = format!("recorded · {state}");
                    e.rationale = format!("Recorded actor role: {role}");
                    e.member_ids.push(actor_id.clone());
                    out.push((memory, e));
                }
            }
            let mut r = self
                .repo
                .db()
                .query(ACTOR_MENTIONS_SURQL)
                .bind(("a", actor_id.clone()))
                .await?
                .check()?;
            let mut mentions: Vec<MentionRow> = r.take(0)?;
            if let Some(cycle) = self.cycle() {
                mentions.retain(|m| m.origin_kind == "article" && m.origin_scope == cycle);
            }
            for m in self.bound(mentions) {
                let (kind, scope, id, revision) = (
                    &m.origin_kind,
                    &m.origin_scope,
                    &m.origin_id,
                    &m.origin_revision,
                );
                let neighbour = if kind == "article" {
                    self.article_node(scope, id, revision).await?
                } else if kind == "thread" {
                    recon::get_thread(self.repo, id).await?.map(|t| {
                        let mut s = source_node("thread", id, revision);
                        s.label = t.title;
                        s
                    })
                } else {
                    Some(source_node(kind, id, revision))
                };
                if let Some(other) = neighbour {
                    let mut e = edge(
                        format!("mention:{actor_id}:{kind}:{scope}:{id}:{revision}"),
                        &n.id,
                        &other.id,
                        TopologyEdgeKind::MentionedIn,
                        format!(
                            "brain_actor_mentions:{actor_id}:{kind}:{scope}:{id}@{revision}\n{}",
                            m.spans_json
                        ),
                    );
                    e.member_ids.push(actor_id.clone());
                    out.push((other, e));
                }
            }
        }
        Ok(())
    }

    /// Claim and Inference neighbours: actor refs and typed payload actors,
    /// article provenance, evidence sources, Recon threads, topics and
    /// premises.
    async fn memory_neighbours(
        &mut self,
        n: &TopologyNode,
        out: &mut Vec<Adjacency>,
    ) -> Result<()> {
        let mut r = self
            .repo
            .db()
            .query(
                "SELECT actor_id, role, binding_state, provenance_json
                 FROM brain_memory_actor_ref WHERE memory_id = $m",
            )
            .bind(("m", n.id.clone()))
            .await?
            .check()?;
        let mut refs: Vec<MemoryRefRow> = r.take(0)?;
        refs.sort_by(|a, b| (&a.actor_id, &a.role).cmp(&(&b.actor_id, &b.role)));
        let refs = self.scoped(refs, |r| &r.actor_id).await?;
        for row in self.bound(refs) {
            if let Some(actor) = Box::pin(self.memory_node(&row.actor_id)).await? {
                let mut e = edge(
                    format!("actor-ref:{}:{}:{}", n.id, row.actor_id, row.role),
                    &actor.id,
                    &n.id,
                    TopologyEdgeKind::Concerns,
                    row.provenance_json,
                );
                e.member_ids.push(row.actor_id);
                e.status = format!("recorded · {}", row.binding_state);
                e.rationale = format!("Recorded actor role: {}", row.role);
                out.push((actor, e));
            }
        }
        let references = match self.payload(&n.id).await? {
            Some(MemoryPayload::Claim(p)) => p.actors,
            Some(MemoryPayload::Inference(p)) => p.actors,
            _ => Vec::new(),
        };
        if references.len() > self.limit {
            self.exhausted = false;
        }
        for reference in references.into_iter().take(self.limit) {
            if let Some(actor) = Box::pin(self.memory_node(&reference.actor_id)).await? {
                let mut e = edge(
                    format!(
                        "actor-ref:{}:{}:{}",
                        n.id, reference.actor_id, reference.role
                    ),
                    &actor.id,
                    &n.id,
                    TopologyEdgeKind::Concerns,
                    format!("memory_metadata:{}@{}", n.id, n.revision),
                );
                e.member_ids.push(reference.actor_id);
                e.status = format!("recorded · {}", reference.binding_state.as_str());
                out.push((actor, e));
            }
        }
        // Direct provenance is independent of tentative Related article pairs.
        let derivations = self
            .derivations(
                "SELECT record::id(id) AS id, snapshot_id, memory_id, insight_kind, item_json
                 FROM insight_derivation WHERE memory_id = $m AND status NOTINSIDE $dead",
                vec![("m", Value::String(n.id.clone()))],
            )
            .await?;
        let evidence = self
            .evidence_for(derivations.iter().map(|d| d.id.clone()).collect())
            .await?;
        let mut articles = BTreeSet::new();
        for d in &derivations {
            if let Some(s) = self.snapshot_owner(&d.snapshot_id).await? {
                if matches!(s.owner_kind.as_str(), "article" | "atlas") {
                    for run in self.article_runs(&s.owner_id).await? {
                        articles.insert((
                            run,
                            s.owner_id.clone(),
                            format!("insight_derivations:{}", d.id),
                        ));
                    }
                }
            }
        }
        for e in &evidence {
            if let Some(article) = e.source_id.strip_prefix("article:") {
                for run in self.article_runs(article).await? {
                    articles.insert((
                        run,
                        article.to_string(),
                        format!("insight_derivations:{}", e.derivation_id),
                    ));
                }
            }
        }
        let claims = self
            .claim_rows(
                "SELECT record::id(id) AS fingerprint, memory_id, topic FROM insight_claim WHERE memory_id = $v",
                Value::String(n.id.clone()),
            )
            .await?;
        let sources = self
            .live_sources(claims.iter().map(|c| c.fingerprint.clone()).collect())
            .await?;
        for s in &sources {
            let article = s.call_id.strip_prefix("article:").unwrap_or(&s.call_id);
            for run in self.article_runs(article).await? {
                let matches = s.run_id.as_deref().is_none_or(|r| r == run || r == article);
                if matches {
                    articles.insert((
                        run,
                        article.to_string(),
                        format!(
                            "insight_sources:{}:{}:{}",
                            s.fingerprint, s.answer_id, s.call_id
                        ),
                    ));
                }
            }
        }
        let articles: Vec<_> = articles
            .into_iter()
            .filter(|a| self.run_scoped(&a.0))
            .collect();
        for (run, id, receipt) in self.bound(articles) {
            if let Some(article) = self.article_node(&run, &id, "").await? {
                let mut e = edge(
                    format!("article-evidence:{}:{}", article.id, n.id),
                    &article.id,
                    &n.id,
                    TopologyEdgeKind::Source,
                    format!(
                        "{receipt}\nCurrent retained article:{run}/{id}@{}",
                        article.revision
                    ),
                );
                if receipt.starts_with("insight_sources:") {
                    e.status = "historical origin · revision unavailable".into();
                    e.label = "historical origin".into();
                }
                out.push((article, e));
            }
        }
        let mut sourced = BTreeSet::new();
        for e in &evidence {
            sourced.insert((
                e.source_id.clone(),
                e.source_revision.clone(),
                e.evidence_json.clone(),
            ));
        }
        let mut kept = Vec::new();
        for s in sourced {
            let keep = match self.cycle().map(str::to_string) {
                Some(cycle) => self.evidence_in_cycle(&s.0, &s.1, &cycle).await?,
                None => true,
            };
            if keep {
                kept.push(s);
            }
        }
        for (source, revision, receipt) in self.bound(kept) {
            let s = source_node("evidence", &source, &revision);
            out.push((
                s.clone(),
                edge(
                    format!("evidence:{}:{}", s.id, n.id),
                    &s.id,
                    &n.id,
                    TopologyEdgeKind::Source,
                    receipt,
                ),
            ));
        }
        if self.cycle().is_none() {
            let threads: BTreeSet<String> =
                sources.iter().filter_map(|s| s.thread_id.clone()).collect();
            for thread in self.bound(threads.into_iter().collect()) {
                if let Some(t) = recon::get_thread(self.repo, &thread).await? {
                    let mut s = source_node("thread", &thread, "");
                    s.label = t.title;
                    out.push((
                        s.clone(),
                        edge(
                            format!("thread-evidence:{thread}:{}", n.id),
                            &s.id,
                            &n.id,
                            TopologyEdgeKind::Source,
                            format!("recon-thread:{thread}"),
                        ),
                    ));
                }
            }
        }
        let topics: BTreeSet<(String, String)> = claims
            .iter()
            .map(|c| (c.topic.clone(), c.fingerprint.clone()))
            .collect();
        for (topic, fingerprint) in self.bound(topics.into_iter().collect()) {
            if let Some(t) = topic_node(&topic) {
                out.push((
                    t.clone(),
                    edge(
                        format!("topic:{}:{}", t.id, n.id),
                        &n.id,
                        &t.id,
                        TopologyEdgeKind::Contains,
                        format!("insight_claims:{fingerprint}:topic={topic}"),
                    ),
                ));
            }
        }
        if n.kind == TopologyNodeKind::Inference {
            self.premises(n, &derivations, &evidence, out).await
        } else {
            self.concluded(n, out).await
        }
    }

    /// An Inference's premise Claims: payload premises, canonical evidence
    /// receipts, and only without either the snapshot-local premise ids.
    async fn premises(
        &mut self,
        n: &TopologyNode,
        derivations: &[DerivationFull],
        evidence: &[EvidenceRow],
        out: &mut Vec<Adjacency>,
    ) -> Result<()> {
        let mut canonical = false;
        if let Some(MemoryPayload::Inference(p)) = self.payload(&n.id).await? {
            canonical = !p.premise_claims.is_empty();
            if p.premise_claims.len() > self.limit {
                self.exhausted = false;
            }
            for id in p.premise_claims.into_iter().take(self.limit) {
                if let Some(c) = Box::pin(self.memory_node(&id))
                    .await?
                    .filter(|c| c.kind == TopologyNodeKind::Claim)
                {
                    let mut e = edge(
                        format!("premise:{id}:{}", n.id),
                        &id,
                        &n.id,
                        TopologyEdgeKind::Premise,
                        format!("memory_metadata:{}@{}", n.id, n.revision),
                    );
                    e.rationale = p.reasoning.clone();
                    out.push((c, e));
                } else {
                    self.gaps
                        .insert(format!("{}: unresolved premise {id}", n.id));
                }
            }
        }
        let inference: BTreeSet<&str> = derivations
            .iter()
            .filter(|d| d.insight_kind == "inference")
            .map(|d| d.id.as_str())
            .collect();
        let mut indexed = BTreeSet::new();
        for e in evidence {
            if let Some(m) = &e.memory_id {
                if inference.contains(e.derivation_id.as_str())
                    && !e.premise_id.is_empty()
                    && !e.premise_id.starts_with("memory:")
                {
                    indexed.insert((m.clone(), e.derivation_id.clone(), e.premise_id.clone()));
                }
            }
        }
        for (id, receipt, local) in self.bound(indexed.into_iter().collect()) {
            if let Some(claim) = Box::pin(self.memory_node(&id))
                .await?
                .filter(|c| c.kind == TopologyNodeKind::Claim)
            {
                canonical = true;
                out.push((
                    claim,
                    edge(
                        format!("premise:{id}:{}", n.id),
                        &id,
                        &n.id,
                        TopologyEdgeKind::Premise,
                        format!("{receipt}:premise={local}:canonical={id}"),
                    ),
                ));
            }
        }
        if canonical {
            // Never reinterpret local `c1` against a sibling's new Claims
            // snapshot when canonical premise identities are available.
            return Ok(());
        }
        for d in derivations.iter().filter(|d| d.insight_kind == "inference") {
            let origin = self.origin_snapshot(&d.snapshot_id, "inference").await?;
            let value: Value = serde_json::from_str(&d.item_json)?;
            let premises: Vec<String> = value["premise_claim_ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| p.as_str().map(str::to_string))
                .collect();
            if premises.len() > self.limit {
                self.exhausted = false;
            }
            for local in premises.into_iter().take(self.limit) {
                let mut id = evidence
                    .iter()
                    .filter(|e| e.derivation_id == d.id && e.premise_id == local)
                    .filter_map(|e| e.memory_id.clone())
                    .min();
                if id.is_none() {
                    let mut r = self
                        .repo
                        .db()
                        .query(
                            "SELECT VALUE memory_id FROM ONLY type::record('insight_derivation', $id)
                             WHERE snapshot_id = $s AND insight_kind = 'claim'
                               AND status NOTINSIDE ['deleted', 'rejected']",
                        )
                        .bind(("id", format!("{origin}:{local}")))
                        .bind(("s", origin.clone()))
                        .await?
                        .check()?;
                    id = r.take::<Option<Option<String>>>(0)?.flatten();
                }
                let claim = match id {
                    Some(id) => Box::pin(self.memory_node(&id))
                        .await?
                        .filter(|c| c.kind == TopologyNodeKind::Claim),
                    None => None,
                };
                if let Some(c) = claim {
                    let mut e = edge(
                        format!("premise:{}:{}", c.id, n.id),
                        &c.id,
                        &n.id,
                        TopologyEdgeKind::Premise,
                        format!("{}:premise={local}", d.id),
                    );
                    e.rationale = value["rationale"].as_str().unwrap_or("").into();
                    out.push((c, e));
                } else {
                    self.gaps.insert(format!(
                        "{}: unresolved snapshot-local premise {local}",
                        d.id
                    ));
                }
            }
        }
        Ok(())
    }

    /// The Inferences a Claim is a premise of: canonical evidence receipts,
    /// payload premises without one, and legacy snapshot-local premises.
    async fn concluded(&mut self, n: &TopologyNode, out: &mut Vec<Adjacency>) -> Result<()> {
        let mut r = self
            .repo
            .db()
            .query(
                "SELECT derivation_id, source_id, source_revision, memory_id, premise_id, evidence_json
                 FROM insight_derivation_evidence
                 WHERE memory_id = $m AND premise_id != '' AND !string::starts_with(premise_id, 'memory:')",
            )
            .bind(("m", n.id.clone()))
            .await?
            .check()?;
        let evidence: Vec<EvidenceRow> = r.take(0)?;
        let derivations = self
            .derivations(
                "SELECT record::id(id) AS id, snapshot_id, memory_id, insight_kind, item_json
                 FROM insight_derivation WHERE record::id(id) INSIDE $ids
                   AND insight_kind = 'inference' AND memory_id != NONE AND status NOTINSIDE $dead",
                vec![(
                    "ids",
                    Value::from(
                        evidence
                            .iter()
                            .map(|e| e.derivation_id.clone())
                            .collect::<Vec<_>>(),
                    ),
                )],
            )
            .await?;
        let mut rows = BTreeSet::new();
        for e in &evidence {
            for d in derivations.iter().filter(|d| d.id == e.derivation_id) {
                if let Some(m) = &d.memory_id {
                    rows.insert((m.clone(), d.id.clone(), e.premise_id.clone()));
                }
            }
        }
        let with_receipt: BTreeSet<String> = rows.iter().map(|r| r.0.clone()).collect();
        let rows = self.scoped(rows.into_iter().collect(), |r| &r.0).await?;
        for (id, receipt, local) in self.bound(rows) {
            if let Some(i) = Box::pin(self.memory_node(&id))
                .await?
                .filter(|i| i.kind == TopologyNodeKind::Inference)
            {
                out.push((
                    i,
                    edge(
                        format!("premise:{}:{id}", n.id),
                        &n.id,
                        &id,
                        TopologyEdgeKind::Premise,
                        format!("{receipt}:premise={local}:canonical={}", n.id),
                    ),
                ));
            }
        }
        let mut r = self
            .repo
            .db()
            .query(
                "SELECT record::id(id) AS id, payload_json FROM memory
                 WHERE memory_kind = 'inference' AND lifecycle = 'active' AND payload_json CONTAINS $m",
            )
            .bind(("m", n.id.clone()))
            .await?
            .check()?;
        let candidates: Vec<PayloadRow> = r.take(0)?;
        let mut typed: Vec<String> = candidates
            .into_iter()
            .filter(|row| !with_receipt.contains(&row.id))
            .filter(|row| {
                serde_json::from_str::<Value>(&row.payload_json).is_ok_and(|v| {
                    v["payload"]["premise_claims"]
                        .as_array()
                        .is_some_and(|p| p.iter().any(|c| c.as_str() == Some(n.id.as_str())))
                })
            })
            .map(|row| row.id)
            .collect();
        typed.sort();
        let typed = self.scoped(typed, |id| id).await?;
        for id in self.bound(typed) {
            if let Some(i) = Box::pin(self.memory_node(&id)).await? {
                let receipt = format!("memory_metadata:{id}@{}", i.revision);
                out.push((
                    i,
                    edge(
                        format!("premise:{}:{id}", n.id),
                        &n.id,
                        &id,
                        TopologyEdgeKind::Premise,
                        receipt,
                    ),
                ));
            }
        }
        // Legacy snapshot-local premise ids on the Claim's own derivations.
        let claims = self
            .derivations(
                "SELECT record::id(id) AS id, snapshot_id, memory_id, insight_kind, item_json
                 FROM insight_derivation WHERE memory_id = $m AND insight_kind = 'claim'
                   AND status NOTINSIDE ['deleted', 'rejected']",
                vec![("m", Value::String(n.id.clone()))],
            )
            .await?;
        let mut legacy = BTreeSet::new();
        for c in &claims {
            let Some(local) = c.id.strip_prefix(&format!("{}:", c.snapshot_id)) else {
                continue;
            };
            let inferences = self
                .derivations(
                    "SELECT record::id(id) AS id, snapshot_id, memory_id, insight_kind, item_json
                     FROM insight_derivation WHERE insight_kind = 'inference' AND memory_id != NONE
                       AND status NOTINSIDE $dead AND item_json CONTAINS $local",
                    vec![("local", Value::String(local.to_string()))],
                )
                .await?;
            for i in inferences {
                let Some(memory) = i.memory_id.clone() else {
                    continue;
                };
                let names = serde_json::from_str::<Value>(&i.item_json).is_ok_and(|v| {
                    v["premise_claim_ids"]
                        .as_array()
                        .is_some_and(|p| p.iter().any(|x| x.as_str() == Some(local)))
                });
                if !names
                    || self.origin_snapshot(&i.snapshot_id, "inference").await? != c.snapshot_id
                {
                    continue;
                }
                let has_payload = matches!(
                    self.payload(&memory).await?,
                    Some(MemoryPayload::Inference(p)) if !p.premise_claims.is_empty()
                );
                let receipts = self.evidence_for(vec![i.id.clone()]).await?;
                if has_payload
                    || receipts
                        .iter()
                        .any(|e| e.premise_id == local && e.memory_id.is_some())
                {
                    continue;
                }
                legacy.insert((memory, i.id.clone(), local.to_string()));
            }
        }
        let legacy = self.scoped(legacy.into_iter().collect(), |r| &r.0).await?;
        for (id, receipt, local) in self.bound(legacy) {
            if let Some(i) = Box::pin(self.memory_node(&id)).await? {
                out.push((
                    i,
                    edge(
                        format!("premise:{}:{id}", n.id),
                        &n.id,
                        &id,
                        TopologyEdgeKind::Premise,
                        format!("{receipt}:premise={local}"),
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Article neighbours: native actor mentions, insight provenance, its
    /// category topic and retained Related article pairs.
    async fn article_neighbours(
        &mut self,
        n: &TopologyNode,
        out: &mut Vec<Adjacency>,
    ) -> Result<()> {
        let mut r = self
            .repo
            .db()
            .query(
                "SELECT actor_id, origin_revision, spans_json FROM brain_actor_mention
                 WHERE origin_kind = 'article' AND origin_scope = $r AND origin_id = $a",
            )
            .bind(("r", n.run_id.clone()))
            .bind(("a", n.article_id.clone()))
            .await?
            .check()?;
        let mut mentions: Vec<ArticleMentionRow> = r.take(0)?;
        mentions.sort_by(|a, b| {
            (&a.actor_id, &a.origin_revision).cmp(&(&b.actor_id, &b.origin_revision))
        });
        for m in self.bound(mentions) {
            let (aid, revision) = (&m.actor_id, &m.origin_revision);
            if !revision.is_empty() && *revision != n.revision {
                self.gaps
                    .insert(format!("{}: stale actor mention {aid}@{revision}", n.id));
                continue;
            }
            if let Some(a) = Box::pin(self.memory_node(aid)).await? {
                let mut e = edge(
                    format!(
                        "mention:{aid}:article:{}:{}:{revision}",
                        n.run_id, n.article_id
                    ),
                    &a.id,
                    &n.id,
                    TopologyEdgeKind::MentionedIn,
                    format!(
                        "brain_actor_mentions:{aid}:article:{}/{}@{revision}\n{}",
                        n.run_id, n.article_id, m.spans_json
                    ),
                );
                e.member_ids.push(aid.clone());
                out.push((a, e));
            }
        }
        let mut r = self
            .repo
            .db()
            .query(
                "SELECT VALUE record::id(id) FROM insight_snapshot
                   WHERE owner_id = $a AND owner_kind INSIDE ['article', 'atlas'];
                 SELECT VALUE derivation_id FROM insight_derivation_evidence WHERE source_id = $s;",
            )
            .bind(("a", n.article_id.clone()))
            .bind(("s", format!("article:{}", n.article_id)))
            .await?
            .check()?;
        let snapshots: Vec<String> = r.take(0)?;
        let cited: Vec<String> = r.take(1)?;
        let derivations = self
            .derivations(
                "SELECT record::id(id) AS id, snapshot_id, memory_id, insight_kind, item_json
                 FROM insight_derivation
                 WHERE (snapshot_id INSIDE $snaps OR record::id(id) INSIDE $cited)
                   AND memory_id != NONE AND status NOTINSIDE $dead",
                vec![
                    ("snaps", Value::from(snapshots)),
                    ("cited", Value::from(cited)),
                ],
            )
            .await?;
        let mut rows = BTreeSet::new();
        for d in derivations {
            if let Some(m) = d.memory_id {
                rows.insert((m, format!("insight_derivations:{}", d.id)));
            }
        }
        let mut r = self
            .repo
            .db()
            .query(
                "SELECT fingerprint, thread_id, run_id, answer_id, call_id FROM insight_source
                 WHERE deleted_origin = false AND call_id INSIDE [$a, $s]
                   AND (run_id = NONE OR run_id = $r OR run_id = $a)",
            )
            .bind(("a", n.article_id.clone()))
            .bind(("s", format!("article:{}", n.article_id)))
            .bind(("r", n.run_id.clone()))
            .await?
            .check()?;
        let sources: Vec<SourceRow> = r.take(0)?;
        for s in &sources {
            let claims = self
                .claim_rows(
                    "SELECT record::id(id) AS fingerprint, memory_id, topic FROM insight_claim
                     WHERE record::id(id) = $v",
                    Value::String(s.fingerprint.clone()),
                )
                .await?;
            for c in claims {
                rows.insert((
                    c.memory_id,
                    format!(
                        "insight_sources:{}:{}:{}",
                        s.fingerprint, s.answer_id, s.call_id
                    ),
                ));
            }
        }
        let rows = self.scoped(rows.into_iter().collect(), |r| &r.0).await?;
        for (id, receipt) in self.bound(rows) {
            if let Some(c) = Box::pin(self.memory_node(&id)).await? {
                let mut e = edge(
                    format!("article-evidence:{}:{id}", n.id),
                    &n.id,
                    &id,
                    TopologyEdgeKind::Source,
                    format!("{receipt}\nCurrent retained {}@{}", n.id, n.revision),
                );
                if receipt.starts_with("insight_sources:") {
                    e.status = "historical origin · revision unavailable".into();
                    e.label = "historical origin".into();
                }
                out.push((c, e));
            }
        }
        if let Some(t) = topic_node(&n.subtype) {
            out.push((
                t.clone(),
                edge(
                    format!("topic:{}:{}", t.id, n.id),
                    &n.id,
                    &t.id,
                    TopologyEdgeKind::Contains,
                    format!("{}@{}:category={}", n.id, n.revision, n.subtype),
                ),
            ));
        }
        let mut r = self
            .repo
            .db()
            .query(
                "SELECT left_id, right_id, left_revision, right_revision, shared_json
                 FROM atlas_related_article WHERE run_id = $r AND (left_id = $a OR right_id = $a)",
            )
            .bind(("r", n.run_id.clone()))
            .bind(("a", n.article_id.clone()))
            .await?
            .check()?;
        let mut pairs: Vec<RelatedRow> = r.take(0)?;
        pairs.sort_by(|a, b| (&a.left_id, &a.right_id).cmp(&(&b.left_id, &b.right_id)));
        for p in self.bound(pairs) {
            let (Some(la), Some(ra)) = (
                atlas::article(self.repo.db(), &n.run_id, &p.left_id).await?,
                atlas::article(self.repo.db(), &n.run_id, &p.right_id).await?,
            ) else {
                continue;
            };
            let related_revision = |a: &crate::store::AtlasArticleRow| {
                crate::nlp::cache::hash(&format!(
                    "{}:{}",
                    crate::atlas_metadata::input_hash(a),
                    a.category
                ))
            };
            if related_revision(&la) != p.left_revision || related_revision(&ra) != p.right_revision
            {
                self.gaps.insert(format!(
                    "related:{}/{}: stale retained pair",
                    p.left_id, p.right_id
                ));
                continue;
            }
            let other = if p.left_id == n.article_id {
                &p.right_id
            } else {
                &p.left_id
            };
            if let Some(a) = self.article_node(&n.run_id, other, "").await? {
                let mut e = edge(
                    format!("atlas-related:{}/{}/{}", n.run_id, p.left_id, p.right_id),
                    &format!("article:{}/{}", n.run_id, p.left_id),
                    &format!("article:{}/{}", n.run_id, p.right_id),
                    TopologyEdgeKind::Related,
                    p.shared_json.clone(),
                );
                e.status = "tentative".into();
                e.label = "possible context".into();
                e.rationale = "Retained shared-entity lead; not corroboration.".into();
                out.push((a, e));
            }
        }
        Ok(())
    }

    async fn topic_neighbours(&mut self, n: &TopologyNode, out: &mut Vec<Adjacency>) -> Result<()> {
        let key = n.label.trim().to_lowercase();
        let claims = self
            .claim_rows(
                "SELECT record::id(id) AS fingerprint, memory_id, topic FROM insight_claim
                 WHERE string::lowercase(string::trim(topic)) = $v",
                Value::String(key.clone()),
            )
            .await?;
        let mut rows = BTreeSet::new();
        for c in claims {
            let live = self
                .memory(&c.memory_id)
                .await?
                .is_none_or(|m| m.lifecycle == "active");
            if live {
                rows.insert((c.memory_id, c.fingerprint, c.topic));
            }
        }
        let rows = self.scoped(rows.into_iter().collect(), |r| &r.0).await?;
        for (id, fingerprint, topic) in self.bound(rows) {
            if let Some(c) = Box::pin(self.memory_node(&id)).await? {
                out.push((
                    c,
                    edge(
                        format!("topic:{}:{id}", n.id),
                        &id,
                        &n.id,
                        TopologyEdgeKind::Contains,
                        format!("insight_claims:{fingerprint}:topic={topic}"),
                    ),
                ));
            }
        }
        let mut r = self
            .repo
            .db()
            .query(
                "SELECT run_id, article_id, category FROM atlas_article
                 WHERE string::lowercase(string::trim(category)) = $k",
            )
            .bind(("k", key))
            .await?
            .check()?;
        let mut articles: Vec<ArticleKeyRow> = r.take(0)?;
        articles.retain(|a| self.run_scoped(&a.run_id));
        articles.sort_by(|a, b| (&a.run_id, &a.article_id).cmp(&(&b.run_id, &b.article_id)));
        for row in self.bound(articles) {
            if let Some(a) = self.article_node(&row.run_id, &row.article_id, "").await? {
                let receipt = format!("{}@{}:category={}", a.id, a.revision, row.category);
                out.push((
                    a.clone(),
                    edge(
                        format!("topic:{}:{}", n.id, a.id),
                        &a.id,
                        &n.id,
                        TopologyEdgeKind::Contains,
                        receipt,
                    ),
                ));
            }
        }
        Ok(())
    }

    async fn source_neighbours(
        &mut self,
        n: &TopologyNode,
        out: &mut Vec<Adjacency>,
    ) -> Result<()> {
        let Some(TopologySeed::Source {
            origin,
            source_id,
            revision,
        }) = n.topology_seed.clone()
        else {
            return Ok(());
        };
        if origin == "evidence" {
            let mut r = self
                .repo
                .db()
                .query(
                    "SELECT derivation_id, source_id, source_revision, memory_id, premise_id, evidence_json
                     FROM insight_derivation_evidence WHERE source_id = $s AND source_revision = $rev",
                )
                .bind(("s", source_id.clone()))
                .bind(("rev", revision.clone()))
                .await?
                .check()?;
            let evidence: Vec<EvidenceRow> = r.take(0)?;
            let derivations = self
                .derivations(
                    "SELECT record::id(id) AS id, snapshot_id, memory_id, insight_kind, item_json
                     FROM insight_derivation WHERE record::id(id) INSIDE $ids
                       AND memory_id != NONE AND status NOTINSIDE $dead",
                    vec![(
                        "ids",
                        Value::from(
                            evidence
                                .iter()
                                .map(|e| e.derivation_id.clone())
                                .collect::<Vec<_>>(),
                        ),
                    )],
                )
                .await?;
            let mut rows = BTreeSet::new();
            for e in &evidence {
                for d in derivations.iter().filter(|d| d.id == e.derivation_id) {
                    if let Some(m) = &d.memory_id {
                        rows.insert((m.clone(), e.evidence_json.clone()));
                    }
                }
            }
            let rows = self.scoped(rows.into_iter().collect(), |r| &r.0).await?;
            for (id, receipt) in self.bound(rows) {
                if let Some(m) = Box::pin(self.memory_node(&id)).await? {
                    out.push((
                        m,
                        edge(
                            format!("evidence:{}:{id}", n.id),
                            &n.id,
                            &id,
                            TopologyEdgeKind::Source,
                            receipt,
                        ),
                    ));
                }
            }
        } else if origin == "thread" && recon::get_thread(self.repo, &source_id).await?.is_some() {
            let mut r = self
                .repo
                .db()
                .query(
                    "SELECT VALUE fingerprint FROM insight_source
                     WHERE thread_id = $t AND deleted_origin = false",
                )
                .bind(("t", source_id.clone()))
                .await?
                .check()?;
            let fingerprints: Vec<String> = r.take(0)?;
            let claims = self
                .claim_rows(
                    "SELECT record::id(id) AS fingerprint, memory_id, topic FROM insight_claim
                     WHERE record::id(id) INSIDE $v",
                    Value::from(fingerprints),
                )
                .await?;
            let ids: BTreeSet<String> = claims.into_iter().map(|c| c.memory_id).collect();
            let ids = self.scoped(ids.into_iter().collect(), |id| id).await?;
            for id in self.bound(ids) {
                if let Some(m) = Box::pin(self.memory_node(&id)).await? {
                    out.push((
                        m,
                        edge(
                            format!("thread-evidence:{source_id}:{id}"),
                            &n.id,
                            &id,
                            TopologyEdgeKind::Source,
                            format!("recon-thread:{source_id}"),
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    /// `Query::neighbours`: typed links plus the node kind's own neighbours,
    /// sorted, with a materialized mentioned_in Link merged into its native
    /// mention receipt.
    async fn neighbours(&mut self, n: &TopologyNode) -> Result<Vec<Adjacency>> {
        let mut out = Vec::new();
        if n.kind == TopologyNodeKind::Article && !self.in_scope(n).await? {
            return Ok(out);
        }
        self.typed_links(n, &mut out).await?;
        match n.kind {
            TopologyNodeKind::Entity => self.actor_neighbours(n, &mut out).await?,
            TopologyNodeKind::Claim | TopologyNodeKind::Inference => {
                Box::pin(self.memory_neighbours(n, &mut out)).await?
            }
            TopologyNodeKind::Article => self.article_neighbours(n, &mut out).await?,
            TopologyNodeKind::Topic => self.topic_neighbours(n, &mut out).await?,
            TopologyNodeKind::Source => self.source_neighbours(n, &mut out).await?,
            TopologyNodeKind::ActorGroup => {}
        }
        out.sort_by(|(a, e), (b, f)| {
            edge_priority(e)
                .cmp(&edge_priority(f))
                .then_with(|| a.kind.label().cmp(b.kind.label()))
                .then_with(|| a.id.cmp(&b.id))
                .then_with(|| e.id.cmp(&f.id))
        });
        let mut merged: Vec<Adjacency> = Vec::new();
        for (other, mut e) in out {
            if e.label == "mentioned_in" {
                if let Some((_, prior)) = merged.iter_mut().find(|(n, p)| {
                    n.id == other.id && p.from == e.from && p.to == e.to && p.label == e.label
                }) {
                    if e.kind == TopologyEdgeKind::Link {
                        e.receipts.append(&mut prior.receipts);
                        e.member_ids.append(&mut prior.member_ids);
                        *prior = e;
                    } else {
                        prior.receipts.extend(e.receipts);
                        prior.member_ids.extend(e.member_ids);
                    }
                    prior.receipts.sort();
                    prior.receipts.dedup();
                    prior.member_ids.sort();
                    prior.member_ids.dedup();
                    continue;
                }
            }
            merged.push((other, e));
        }
        Ok(merged)
    }
}

fn dead() -> Vec<String> {
    DEAD.iter().map(|s| s.to_string()).collect()
}

/// The Actor-group root over its live members.
async fn group_root(
    q: &mut Query<'_>,
    seed: &TopologySeed,
    name_key: &str,
    member_ids: &[String],
    group_members: &mut BTreeSet<String>,
) -> Result<Option<TopologyNode>> {
    let mut valid = Vec::new();
    if member_ids.len() > q.limit {
        q.exhausted = false;
    }
    let mut display_name = name_key.to_string();
    for id in member_ids.iter().take(q.limit) {
        if let Some(n) = q
            .memory_node(id)
            .await?
            .filter(|n| n.kind == TopologyNodeKind::Entity)
        {
            group_members.insert(n.id.clone());
            display_name = n.label.clone();
            valid.push(id.clone());
        }
    }
    if valid.is_empty() {
        return Ok(None);
    }
    valid.sort();
    valid.dedup();
    let mut root = node(
        format!("actor-group:{name_key}"),
        TopologyNodeKind::ActorGroup,
        display_name,
    );
    root.status = "display group · identity unmerged".into();
    root.subtype = "actor_name_group".into();
    root.detail = format!(
        "{} retained member bindings; sharing a name does not establish identity.",
        valid.len()
    );
    root.member_ids = valid;
    root.topology_seed = Some(seed.clone());
    Ok(Some(root))
}

/// `Store::topology_snapshot` on SurrealDB: every seed except the retained
/// Intel display seeds, walked breadth-first to `options.depth` (1..=3).
pub async fn topology_snapshot(
    repo: &Repository,
    seed: &TopologySeed,
    options: &TopologyOptions,
) -> Result<TopologySnapshot> {
    ensure!(
        options.continuation_token.is_none(),
        "Topology continuation is unavailable; refresh with a larger bounded limit"
    );
    let depth = options.depth.clamp(1, 3);
    let mut q = Query {
        repo,
        limit: options.max_nodes.saturating_mul(2).clamp(32, 256),
        scope: &options.cycle_scope,
        exhausted: true,
        gaps: BTreeSet::new(),
        remaining_rows: options.max_nodes.saturating_mul(64).clamp(4096, 32768),
        memories: HashMap::new(),
    };
    let mut snapshot = TopologySnapshot {
        seed: Some(seed.clone()),
        memory_changed_seq: counters::current(repo.db(), CHANGED_COUNTER).await?.max(0) as u64,
        ..Default::default()
    };
    let mut roots = Vec::new();
    let mut initial_edges = Vec::new();
    let mut group_members = BTreeSet::new();
    let kind_root = |want: TopologyNodeKind| move |n: &TopologyNode| n.kind == want;
    match seed {
        TopologySeed::Claim(id) => roots.extend(
            q.memory_node(id)
                .await?
                .filter(kind_root(TopologyNodeKind::Claim)),
        ),
        TopologySeed::Actor(id) => roots.extend(
            q.memory_node(id)
                .await?
                .filter(kind_root(TopologyNodeKind::Entity)),
        ),
        TopologySeed::Inference(id) => roots.extend(
            q.memory_node(id)
                .await?
                .filter(kind_root(TopologyNodeKind::Inference)),
        ),
        TopologySeed::ActorGroup {
            name_key,
            member_ids,
        } => {
            roots.extend(group_root(&mut q, seed, name_key, member_ids, &mut group_members).await?)
        }
        TopologySeed::Article {
            run_id,
            article_id,
            revision,
            ..
        } => roots.extend(q.article_node(run_id, article_id, revision).await?),
        TopologySeed::Topic(label) => roots.extend(topic_node(label)),
        TopologySeed::Source {
            origin,
            source_id,
            revision,
        } => roots.push(source_node(origin, source_id, revision)),
        TopologySeed::Link(id) | TopologySeed::Related(id) => {
            if let Some((s, t, e)) = q.link(id).await? {
                roots.push(s);
                roots.push(t);
                initial_edges.push(e);
            }
        }
        TopologySeed::RetainedClaim { .. }
        | TopologySeed::RetainedInference { .. }
        | TopologySeed::RetainedActor { .. } => {
            bail!("{} topology isn't available on SurrealDB yet", seed.key())
        }
    }
    let Some(root) = roots.first() else {
        snapshot.findings.gaps.push(
            "The selected record is missing, revised, or has no supported typed endpoint.".into(),
        );
        snapshot.unresolved_endpoints = q.gaps.into_iter().collect();
        snapshot.exploration_exhausted = q.exhausted;
        snapshot.neighborhood_brief =
            Some(brief::missing(&snapshot, seed, depth, &options.cycle_scope));
        return Ok(snapshot);
    };
    snapshot.root_id = root.id.clone();
    snapshot.root_label = root.label.clone();
    snapshot.root_kind = Some(root.kind);
    let root_id = root.id.clone();
    let mut nodes = BTreeMap::new();
    let mut edges = BTreeMap::new();
    let mut queue = VecDeque::new();
    for mut n in roots {
        n.is_root = true;
        n.min_hops = 0;
        queue.push_back(n.id.clone());
        nodes.insert(n.id.clone(), n);
    }
    for e in initial_edges {
        edges.insert(e.id.clone(), e);
    }
    let candidate_limit = options.max_nodes.saturating_mul(8).clamp(256, 4096);
    let edge_limit = options.max_edges.saturating_mul(8).clamp(512, 8192);
    while let Some(id) = queue.pop_front() {
        let current: TopologyNode = nodes[&id].clone();
        if current.min_hops >= depth {
            continue;
        }
        // The group's neighbours are its members' neighbours, re-pointed at it.
        let mut adjacent = if current.kind == TopologyNodeKind::ActorGroup {
            let mut all = Vec::new();
            for member in &group_members {
                if let Some(actor) = q.memory_node(member).await? {
                    all.extend(q.neighbours(&actor).await?);
                    if all.len() >= edge_limit {
                        q.exhausted = false;
                        all.truncate(edge_limit);
                        break;
                    }
                }
            }
            all
        } else {
            q.neighbours(&current).await?
        };
        adjacent.sort_by(|(a, e), (b, f)| {
            edge_priority(e)
                .cmp(&edge_priority(f))
                .then_with(|| a.id.cmp(&b.id))
                .then_with(|| e.id.cmp(&f.id))
        });
        for (mut other, mut e) in adjacent {
            if group_members.contains(&e.from) {
                e.from = root_id.clone();
            }
            if group_members.contains(&e.to) {
                e.to = root_id.clone();
            }
            if group_members.contains(&other.id) {
                other = nodes[&root_id].clone();
            }
            if other.id == current.id || e.from == e.to {
                continue;
            }
            if !q.in_scope(&other).await? {
                continue;
            }
            if !nodes.contains_key(&other.id) {
                if nodes.len() >= candidate_limit || edges.len() >= edge_limit {
                    q.exhausted = false;
                    continue;
                }
                other.min_hops = current.min_hops + 1;
                other.predecessor = Some(current.id.clone());
                queue.push_back(other.id.clone());
                nodes.insert(other.id.clone(), other);
            }
            if nodes.contains_key(&e.from) && nodes.contains_key(&e.to) {
                if edges.len() < edge_limit {
                    record_edge(&mut edges, e);
                } else {
                    q.exhausted = false;
                }
            }
        }
    }
    let exhausted = q.exhausted;
    let gaps = std::mem::take(&mut q.gaps);
    admit(&mut snapshot, nodes, edges, options, exhausted, gaps);
    for n in snapshot.nodes.clone() {
        if n.is_root {
            continue;
        }
        match n.kind {
            TopologyNodeKind::Claim => {
                if let Some(m) = q.memory(&n.id).await? {
                    snapshot.associations.claims.push(Memory {
                        id: n.id.clone(),
                        text: m.text,
                        category: m.category,
                        pinned: m.pinned,
                        created_at: m.created_at,
                        source: serde_json::from_str::<MemorySource>(&m.source_json)
                            .unwrap_or_else(|_| MemorySource {
                                app: String::new(),
                                conversation_id: String::new(),
                                message_id: None,
                                reference: None,
                            }),
                    });
                }
            }
            TopologyNodeKind::Article => {
                if let Some(article) = atlas::article(repo.db(), &n.run_id, &n.article_id).await? {
                    snapshot
                        .associations
                        .articles
                        .push(crate::brain_browse::ArticleAssociation {
                            article,
                            occurrences: vec![(
                                n.run_id.clone(),
                                n.article_id.clone(),
                                n.revision.clone(),
                            )],
                        });
                }
            }
            _ => {}
        }
    }
    // The deterministic brief from the admitted records' payloads.
    let mut payloads = BTreeMap::new();
    let mut skipped = 0;
    let mut wanted: Vec<String> = snapshot
        .nodes
        .iter()
        .filter(|n| {
            matches!(
                n.kind,
                TopologyNodeKind::Entity | TopologyNodeKind::Claim | TopologyNodeKind::Inference
            )
        })
        .map(|n| n.id.clone())
        .collect();
    wanted.extend(
        snapshot
            .edges
            .iter()
            .filter(|e| matches!(e.kind, TopologyEdgeKind::Link | TopologyEdgeKind::Related))
            .map(|e| e.id.clone()),
    );
    for (i, id) in wanted.iter().enumerate() {
        if i >= MAX_PAYLOAD_READS {
            skipped += 1;
            continue;
        }
        if let Some(p) = q.payload(id).await? {
            payloads.insert(id.clone(), p);
        }
    }
    let (built, packet) = {
        let input = BriefInput {
            snapshot: &snapshot,
            seed,
            depth,
            cycle: &options.cycle_scope,
            payloads,
            payload_reads_skipped: skipped,
        };
        let built = brief::build(&input);
        let packet = brief::build_input(&input, &built.basis_key);
        (built, packet)
    };
    snapshot.neighborhood_brief = Some(built);
    snapshot.neighborhood_input = Some(packet);
    Ok(snapshot)
}
