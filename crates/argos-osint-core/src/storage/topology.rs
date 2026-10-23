//! Brain topology on the async repository (migration phase 5, first item):
//! `Store::topology_snapshot` for the Actor-group seed
//! (`TopologySeed::ActorGroup`), with the SQLite node, edge, receipt and
//! admission rules. Members resolve through actor redirects; the group's
//! neighbours are its members' typed Links/Related records, actor refs,
//! typed payload actors and native mentions. Admission, findings and the
//! deterministic neighborhood brief reuse the SQLite code
//! (`brain_topology::admit`, `brain_neighborhood_brief::build`).
//!
//! Gaps (see acceptance.md): other seeds are refused; nodes beyond the
//! group's own neighbours are not expanded (depth > 1 is reported as a
//! bounded gap); the `ThisCycle` scope does not apply the current-metadata
//! fence; memory nodes carry no revision (SurrealDB memories have none);
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
    admit, edge, edge_priority, node, record_edge, source_node, CycleScope, TopologyEdge,
    TopologyEdgeKind, TopologyNode, TopologyNodeKind, TopologyOptions, TopologySeed,
    TopologySnapshot,
};
use crate::memory_metadata::{EndpointRef, MemoryKind, MemoryPayload};

type Adjacency = (TopologyNode, TopologyEdge);

/// Derivation statuses that no longer support a node or its scope.
const DEAD: [&str; 3] = ["deleted", "retired", "rejected"];

/// Gap recorded when the walk would expand past the group's neighbours.
pub const DEPTH_GAP: &str =
    "Topology beyond an actor group's own neighbours isn't available on SurrealDB yet.";

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
            if e.endpoint_kind == "article" && e.origin == cycle {
                return Ok(true);
            }
            if e.endpoint_kind == "memory" && self.memory_in_cycle(&e.ref_id, cycle).await? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `typed_links` for an Entity node: active Link/Related records with a
    /// memory endpoint on any of its member ids, Links first.
    async fn typed_links(&mut self, n: &TopologyNode, out: &mut Vec<Adjacency>) -> Result<()> {
        for reference in n.member_ids.clone() {
            let mut r = self
                .repo
                .db()
                .query(
                    "SELECT VALUE memory_id FROM brain_memory_endpoint
                     WHERE endpoint_kind = 'memory' AND ref_id = $r",
                )
                .bind(("r", reference))
                .await?
                .check()?;
            let ids: BTreeSet<String> = r.take::<Vec<String>>(0)?.into_iter().collect();
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
                let live = self.memory(&row.memory_id).await?.is_some_and(|m| {
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
            let mut kept = Vec::new();
            for t in typed {
                if self.memory_scoped(&t.0).await? {
                    kept.push(t);
                }
            }
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

    /// `Query::neighbours` for an actor (member) node: typed links and actor
    /// neighbours, sorted, with a materialized mentioned_in Link merged into
    /// its native mention receipt.
    async fn neighbours(&mut self, n: &TopologyNode) -> Result<Vec<Adjacency>> {
        let mut out = Vec::new();
        self.typed_links(n, &mut out).await?;
        self.actor_neighbours(n, &mut out).await?;
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

/// `Store::topology_snapshot` on SurrealDB for `TopologySeed::ActorGroup`.
/// Other seeds are refused with an error.
pub async fn topology_snapshot(
    repo: &Repository,
    seed: &TopologySeed,
    options: &TopologyOptions,
) -> Result<TopologySnapshot> {
    ensure!(
        options.continuation_token.is_none(),
        "Topology continuation is unavailable; refresh with a larger bounded limit"
    );
    let TopologySeed::ActorGroup {
        name_key,
        member_ids,
    } = seed
    else {
        bail!("{} topology isn't available on SurrealDB yet", seed.key());
    };
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
    let mut group_members = BTreeSet::new();
    let mut valid = Vec::new();
    if member_ids.len() > q.limit {
        q.exhausted = false;
    }
    let mut display_name = name_key.clone();
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
        snapshot.findings.gaps.push(
            "The selected record is missing, revised, or has no supported typed endpoint.".into(),
        );
        snapshot.unresolved_endpoints = q.gaps.into_iter().collect();
        snapshot.exploration_exhausted = q.exhausted;
        snapshot.neighborhood_brief =
            Some(brief::missing(&snapshot, seed, depth, &options.cycle_scope));
        return Ok(snapshot);
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
    root.is_root = true;
    snapshot.root_id = root.id.clone();
    snapshot.root_label = root.label.clone();
    snapshot.root_kind = Some(root.kind);
    let root_id = root.id.clone();
    let mut nodes = BTreeMap::from([(root_id.clone(), root)]);
    let mut edges = BTreeMap::new();
    let candidate_limit = options.max_nodes.saturating_mul(8).clamp(256, 4096);
    let edge_limit = options.max_edges.saturating_mul(8).clamp(512, 8192);
    // The group's neighbours are its members' neighbours, re-pointed at it.
    let mut adjacent = Vec::new();
    for member in &group_members {
        if let Some(actor) = q.memory_node(member).await? {
            adjacent.extend(q.neighbours(&actor).await?);
            if adjacent.len() >= edge_limit {
                q.exhausted = false;
                adjacent.truncate(edge_limit);
                break;
            }
        }
    }
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
        if other.id == root_id || e.from == e.to {
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
            other.min_hops = 1;
            other.predecessor = Some(root_id.clone());
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
    if depth > 1 && nodes.len() > 1 {
        q.exhausted = false;
        q.gaps.insert(DEPTH_GAP.into());
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
