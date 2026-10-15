//! Bounded, read-only adjacency queries for the shared topology projection.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use anyhow::{ensure, Result};
use rusqlite::{params, OptionalExtension, Params};
use serde_json::Value;

use super::*;
use crate::brain_actors as actors;
use crate::memory_metadata::{EndpointRef, MemoryKind, MemoryPayload};
use crate::store::Store;

type Adjacency = (TopologyNode, TopologyEdge);

enum ScopedRows<'a> {
    Never,
    Memory(usize),
    Article(usize),
    Mention { kind: usize, run: usize },
    Evidence { source: usize, revision: usize },
    Link(&'a TopologyNode),
}

fn sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Match retained metadata content, URL and publication time, rather than
/// assigning today's article revision to a historical derivation by owner ID.
fn current_metadata_sql(snapshot: &str, article: &str) -> String {
    format!("EXISTS(SELECT 1 FROM json_each(CASE WHEN json_valid({snapshot}.inputs_json) THEN json_extract({snapshot}.inputs_json,'$.source_refs') ELSE '[]' END) topology_ref JOIN nlp_artifacts topology_artifact ON topology_artifact.hash=json_extract(topology_ref.value,'$.artifact_hash') WHERE json_extract(topology_ref.value,'$.source_id')='article:'||{article}.article_id AND topology_artifact.text={article}.title||char(10)||{article}.description AND json_extract(topology_ref.value,'$.url')={article}.url AND COALESCE(json_extract(topology_ref.value,'$.published_at'),'')={article}.published_at)")
}

fn memory_cycle_sql(memory: &str, cycle: &str) -> String {
    let current = current_metadata_sql("topology_snapshot", "topology_article");
    format!("(EXISTS(SELECT 1 FROM insight_derivations topology_derivation JOIN insight_snapshots topology_snapshot ON topology_snapshot.id=topology_derivation.snapshot_id JOIN atlas_articles topology_article ON topology_article.article_id=topology_snapshot.owner_id WHERE topology_derivation.memory_id={memory} AND topology_article.run_id={cycle} AND topology_derivation.status NOT IN ('deleted','retired','rejected') AND {current}) OR EXISTS(SELECT 1 FROM insight_sources topology_source JOIN insight_claims topology_claim ON topology_claim.fingerprint=topology_source.fingerprint WHERE topology_claim.memory_id={memory} AND topology_source.run_id={cycle} AND topology_source.deleted_origin=0) OR EXISTS(SELECT 1 FROM brain_actor_mentions topology_mention WHERE topology_mention.actor_id={memory} AND topology_mention.origin_kind='article' AND topology_mention.origin_scope={cycle}) OR EXISTS(SELECT 1 FROM brain_memory_actor_refs topology_actor_ref JOIN insight_derivations topology_derivation ON topology_derivation.memory_id=topology_actor_ref.memory_id JOIN insight_snapshots topology_snapshot ON topology_snapshot.id=topology_derivation.snapshot_id JOIN atlas_articles topology_article ON topology_article.article_id=topology_snapshot.owner_id WHERE topology_actor_ref.actor_id={memory} AND topology_article.run_id={cycle} AND topology_derivation.status NOT IN ('deleted','retired','rejected') AND {current}) OR EXISTS(SELECT 1 FROM brain_memory_actor_refs topology_actor_ref JOIN insight_claims topology_claim ON topology_claim.memory_id=topology_actor_ref.memory_id JOIN insight_sources topology_source ON topology_source.fingerprint=topology_claim.fingerprint WHERE topology_actor_ref.actor_id={memory} AND topology_source.run_id={cycle} AND topology_source.deleted_origin=0))")
}

fn node(id: String, kind: TopologyNodeKind, label: String) -> TopologyNode {
    TopologyNode {
        id,
        kind,
        detail: label.clone(),
        label,
        subtype: String::new(),
        status: "recorded".into(),
        is_root: false,
        article_id: String::new(),
        run_id: String::new(),
        published_at: String::new(),
        cycle: None,
        member_ids: Vec::new(),
        evidence_count: 0,
        passages: Vec::new(),
        min_hops: 0,
        revision: String::new(),
        predecessor: None,
        topology_seed: None,
    }
}

fn edge(id: String, from: &str, to: &str, kind: TopologyEdgeKind, receipt: String) -> TopologyEdge {
    TopologyEdge {
        id,
        from: from.into(),
        to: to.into(),
        kind,
        label: match kind {
            TopologyEdgeKind::Concerns => "referenced by",
            TopologyEdgeKind::Premise => "premise for",
            TopologyEdgeKind::Source => "source for",
            _ => kind.label(),
        }
        .into(),
        status: "recorded".into(),
        member_ids: Vec::new(),
        receipts: vec![receipt],
        rationale: String::new(),
    }
}

fn edge_priority(e: &TopologyEdge) -> u8 {
    match e.kind {
        TopologyEdgeKind::Source | TopologyEdgeKind::Premise => 0,
        TopologyEdgeKind::Concerns | TopologyEdgeKind::MentionedIn => 1,
        TopologyEdgeKind::Link => 2,
        TopologyEdgeKind::Contains => 3,
        TopologyEdgeKind::Related => 4,
    }
}

fn record_edge(edges: &mut BTreeMap<String, TopologyEdge>, edge: TopologyEdge) {
    match edges.entry(edge.id.clone()) {
        std::collections::btree_map::Entry::Vacant(slot) => {
            slot.insert(edge);
        }
        std::collections::btree_map::Entry::Occupied(mut slot) => {
            let current = slot.get_mut();
            current.receipts.extend(edge.receipts);
            current.receipts.sort();
            current.receipts.dedup();
            current.member_ids.extend(edge.member_ids);
            current.member_ids.sort();
            current.member_ids.dedup();
        }
    }
}

fn meaningful_topic(label: &str) -> bool {
    !matches!(
        label.trim().to_lowercase().as_str(),
        "" | "unk" | "unknown" | "investigation" | "fact" | "claim" | "inference"
    )
}

fn topic_node(label: &str) -> Option<TopologyNode> {
    if !meaningful_topic(label) {
        return None;
    }
    let label = label.trim().to_string();
    let seed = TopologySeed::Topic(label.clone());
    let mut out = node(seed.key(), TopologyNodeKind::Topic, label);
    out.subtype = "derived_topic_v1".into();
    out.status = "recorded attribute".into();
    out.detail = "Read-only projection of recorded topic/category attributes; not a canonical Brain memory or an assessed relationship.".into();
    out.revision = "topic-v1".into();
    out.topology_seed = Some(seed);
    Some(out)
}

fn source_node(origin: &str, source_id: &str, revision: &str) -> TopologyNode {
    let seed = TopologySeed::Source {
        origin: origin.into(),
        source_id: source_id.into(),
        revision: revision.into(),
    };
    let mut out = node(seed.key(), TopologyNodeKind::Source, source_id.into());
    out.subtype = origin.into();
    out.revision = revision.into();
    out.passages.push(source_id.into());
    out.detail = format!("{origin} · {source_id}\nRevision: {revision}");
    out.topology_seed = Some(seed);
    out
}

struct Query<'a> {
    store: &'a Store,
    limit: usize,
    scope: &'a CycleScope,
    exhausted: bool,
    gaps: BTreeSet<String>,
    remaining_rows: usize,
}

impl Query<'_> {
    /// Filter the requested cycle before the branch limit so earlier retained
    /// cycles cannot starve the selected cycle's neighbours.
    fn scoped_rows<const N: usize>(
        &mut self,
        sql: &str,
        parameters: impl Params,
        filter: ScopedRows<'_>,
    ) -> Result<Vec<[String; N]>> {
        let CycleScope::ThisCycle(cycle) = self.scope else {
            return self.rows(sql, parameters);
        };
        let cycle = sql_literal(cycle);
        let predicate = match filter {
            ScopedRows::Never => "0".into(),
            ScopedRows::Memory(column) => memory_cycle_sql(&format!("c{column}"), &cycle),
            ScopedRows::Article(column) => format!("c{column}={cycle}"),
            ScopedRows::Mention { kind, run } => format!("c{kind}='article' AND c{run}={cycle}"),
            ScopedRows::Evidence { source, revision } => {
                let current = current_metadata_sql("topology_snapshot", "topology_article");
                format!("EXISTS(SELECT 1 FROM insight_derivation_evidence topology_evidence JOIN insight_derivations topology_derivation ON topology_derivation.id=topology_evidence.derivation_id JOIN insight_snapshots topology_snapshot ON topology_snapshot.id=topology_derivation.snapshot_id JOIN atlas_articles topology_article ON topology_article.article_id=topology_snapshot.owner_id WHERE topology_evidence.source_id=c{source} AND topology_evidence.source_revision=c{revision} AND topology_article.run_id={cycle} AND topology_derivation.status NOT IN ('deleted','retired','rejected') AND {current})")
            }
            ScopedRows::Link(node) => {
                let mut root_ids = node.member_ids.clone();
                root_ids.push(node.id.clone());
                let root_ids = root_ids
                    .iter()
                    .map(|id| sql_literal(id))
                    .collect::<Vec<_>>()
                    .join(",");
                let memory = memory_cycle_sql("topology_endpoint.ref_id", &cycle);
                let exclude_article = if node.kind == TopologyNodeKind::Article {
                    format!(" AND NOT(topology_endpoint.endpoint_kind='article' AND topology_endpoint.origin={} AND topology_endpoint.ref_id={})",sql_literal(&node.run_id),sql_literal(&node.article_id))
                } else {
                    String::new()
                };
                format!("EXISTS(SELECT 1 FROM brain_memory_endpoints topology_endpoint WHERE topology_endpoint.memory_id=c0 AND NOT(topology_endpoint.endpoint_kind='memory' AND topology_endpoint.ref_id IN ({root_ids})){exclude_article} AND ((topology_endpoint.endpoint_kind='article' AND topology_endpoint.origin={cycle}) OR (topology_endpoint.endpoint_kind='memory' AND {memory})))")
            }
        };
        let columns = (0..N)
            .map(|i| format!("c{i}"))
            .collect::<Vec<_>>()
            .join(",");
        self.rows(&format!("WITH topology_rows({columns}) AS ({sql}) SELECT {columns} FROM topology_rows WHERE {predicate} ORDER BY {columns}"),parameters)
    }

    fn snapshot_matches_article(&self, snapshot: &str, run: &str, article: &str) -> Result<bool> {
        let current = current_metadata_sql("s", "a");
        Ok(self.store.conn.query_row(&format!("SELECT EXISTS(SELECT 1 FROM insight_snapshots s JOIN atlas_articles a ON a.run_id=?2 AND a.article_id=?3 WHERE s.id=?1 AND s.owner_id=a.article_id AND s.owner_kind IN ('article','atlas') AND {current})"),params![snapshot,run,article],|r|r.get(0))?)
    }
    /// Every adjacency branch requests one extra row, so truncation is known.
    /// SQL is internal constant text; only the numeric bound is interpolated.
    fn rows<const N: usize>(
        &mut self,
        sql: &str,
        parameters: impl Params,
    ) -> Result<Vec<[String; N]>> {
        if self.remaining_rows == 0 {
            self.exhausted = false;
            return Ok(Vec::new());
        }
        let limit = self.limit.min(self.remaining_rows);
        let mut statement = self
            .store
            .conn
            .prepare(&format!("{sql} LIMIT {}", limit + 1))?;
        let mut rows = statement
            .query_map(parameters, |r| {
                let mut fields = std::array::from_fn(|_| String::new());
                for (i, field) in fields.iter_mut().enumerate() {
                    *field = r.get(i)?;
                }
                Ok(fields)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.len() > limit {
            self.exhausted = false;
            rows.truncate(limit);
        }
        self.remaining_rows = self.remaining_rows.saturating_sub(rows.len());
        Ok(rows)
    }

    fn memory_node(&mut self, id: &str) -> Result<Option<TopologyNode>> {
        let Some(memory) = self.store.get_memory(id)? else {
            return Ok(None);
        };
        let metadata: Option<(String, String, String)> = self.store.conn.query_row(
            "SELECT memory_kind,lifecycle,CAST(revision AS TEXT) FROM memory_metadata WHERE memory_id=?1",
            [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).optional()?;
        if metadata
            .as_ref()
            .is_some_and(|(_, lifecycle, _)| matches!(lifecycle.as_str(), "retired" | "superseded"))
        {
            return Ok(None);
        }
        let stored_kind = if let Some((kind, _, _)) = &metadata {
            kind.clone()
        } else {
            self.store.conn.query_row("SELECT classification FROM insight_claims WHERE memory_id=?1 ORDER BY fingerprint LIMIT 1", [id], |r| r.get::<_, String>(0)).optional()?.unwrap_or_default()
        };
        let kind = match MemoryKind::parse(&stored_kind) {
            Some(MemoryKind::Actor) => {
                let canonical = actors::resolve(&self.store.conn, id)?;
                let Some(actor) = actors::actor(&self.store.conn, &canonical)? else {
                    return Ok(None);
                };
                let mut out = node(
                    canonical.clone(),
                    TopologyNodeKind::Entity,
                    actor.display_name,
                );
                out.subtype = actor.subtype;
                out.status = actor.identity_state.as_str().into();
                out.member_ids.push(canonical.clone());
                let mut aliases = VecDeque::from([canonical.clone()]);
                while let Some(survivor) = aliases.pop_front() {
                    for [old] in self.rows::<1>("SELECT from_actor FROM brain_actor_redirects WHERE to_actor=?1 ORDER BY from_actor", [&survivor])? {
                        if out.member_ids.contains(&old) { continue; }
                        if out.member_ids.len() >= self.limit { self.exhausted = false; break; }
                        out.member_ids.push(old.clone()); aliases.push_back(old);
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
        let mut out = node(id.into(), kind, memory.text);
        out.subtype = memory.category;
        out.published_at = memory.created_at;
        out.revision = metadata
            .map(|(_, _, revision)| revision)
            .unwrap_or_default();
        out.topology_seed = Some(if kind == TopologyNodeKind::Inference {
            TopologySeed::Inference(id.into())
        } else {
            TopologySeed::Claim(id.into())
        });
        if let Some(payload) = self.store.memory_payload(id)? {
            match payload {
                MemoryPayload::Claim(p) => {
                    out.status = if p.assessment.trim().is_empty()
                        || p.assessment.trim().eq_ignore_ascii_case("unassessed")
                    {
                        "unassessed".into()
                    } else {
                        "extraction assessed".into()
                    };
                    out.passages = p.passages;
                }
                MemoryPayload::Inference(p) => {
                    out.status = p.support_state;
                    out.detail = format!(
                        "{}\nReasoning: {}\nLimitations: {}",
                        out.label, p.reasoning, p.limitations
                    );
                }
                _ => {}
            }
        }
        if kind == TopologyNodeKind::Inference {
            for [json, receipt] in self.rows::<2>("SELECT d.item_json,d.id FROM insight_derivations d WHERE d.memory_id=?1 AND d.status NOT IN ('deleted','retired','rejected') ORDER BY d.id", [id])? {
                if let Ok(item) = serde_json::from_str::<Value>(&json) {
                    out.detail = format!("{}\nReasoning: {}\nLimitations: {}", out.label, item["rationale"].as_str().unwrap_or(""), item["limitations"]);
                    out.passages.push(receipt);
                }
            }
        }
        Ok(Some(out))
    }

    fn article_node(
        &mut self,
        run: &str,
        id: &str,
        expected: &str,
    ) -> Result<Option<TopologyNode>> {
        let Some(article) = self.store.atlas_article(run, id)? else {
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

    fn endpoint_node(&mut self, endpoint: &EndpointRef) -> Result<Option<TopologyNode>> {
        match endpoint {
            EndpointRef::Memory { memory_id, kind } => {
                let result = self.memory_node(memory_id)?;
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
            } => self.article_node(run_id, article_id, revision),
            EndpointRef::Source {
                origin,
                source_id,
                revision,
            } => Ok(Some(source_node(origin, source_id, revision))),
        }
    }

    fn in_scope(&mut self, n: &TopologyNode) -> Result<bool> {
        let CycleScope::ThisCycle(cycle) = self.scope else {
            return Ok(true);
        };
        if n.kind == TopologyNodeKind::Topic {
            return Ok(true);
        }
        if let Some(run) = &n.cycle {
            return Ok(run == cycle);
        }
        if let Some(TopologySeed::Source {
            origin,
            source_id,
            revision,
        }) = &n.topology_seed
        {
            if origin == "evidence" {
                let current = current_metadata_sql("s", "a");
                return Ok(self.store.conn.query_row(&format!("SELECT EXISTS(SELECT 1 FROM insight_derivation_evidence e JOIN insight_derivations d ON d.id=e.derivation_id JOIN insight_snapshots s ON s.id=d.snapshot_id JOIN atlas_articles a ON a.article_id=s.owner_id WHERE e.source_id=?1 AND e.source_revision=?2 AND a.run_id=?3 AND d.status NOT IN ('deleted','retired','rejected') AND {current})"),params![source_id,revision,cycle],|r|r.get(0))?);
            }
            return Ok(false);
        }
        let exists: bool = self.store.conn.query_row(
            &format!("SELECT {}", memory_cycle_sql("?1", "?2")),
            params![n.id, cycle],
            |r| r.get(0),
        )?;
        Ok(exists)
    }

    fn link(&mut self, id: &str) -> Result<Option<(TopologyNode, TopologyNode, TopologyEdge)>> {
        let lifecycle: Option<String> = self
            .store
            .conn
            .query_row(
                "SELECT lifecycle FROM memory_metadata WHERE memory_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        if lifecycle.as_deref() != Some("active") {
            return Ok(None);
        }
        let (source, target, mut e) = match self.store.memory_payload(id)? {
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
        let (Some(s), Some(t)) = (self.endpoint_node(&source)?, self.endpoint_node(&target)?)
        else {
            self.gaps
                .insert(format!("{id}: unresolved relationship endpoint"));
            return Ok(None);
        };
        e.from = s.id.clone();
        e.to = t.id.clone();
        Ok(Some((s, t, e)))
    }

    fn typed_links(&mut self, n: &TopologyNode, out: &mut Vec<Adjacency>) -> Result<()> {
        let (kind, reference, origin) = match &n.topology_seed {
            Some(TopologySeed::Article {
                run_id, article_id, ..
            }) => ("article", article_id.as_str(), run_id.as_str()),
            Some(TopologySeed::Source {
                origin, source_id, ..
            }) => ("source", source_id.as_str(), origin.as_str()),
            _ if matches!(
                n.kind,
                TopologyNodeKind::Entity | TopologyNodeKind::Claim | TopologyNodeKind::Inference
            ) =>
            {
                ("memory", n.id.as_str(), "")
            }
            _ => return Ok(()),
        };
        let references = if n.kind == TopologyNodeKind::Entity {
            n.member_ids.clone()
        } else {
            vec![reference.into()]
        };
        for reference in references {
            for [id] in self.scoped_rows::<1>("SELECT DISTINCT e.memory_id FROM brain_memory_endpoints e JOIN memory_metadata m ON m.memory_id=e.memory_id WHERE e.endpoint_kind=?1 AND e.ref_id=?2 AND (?3='' OR e.origin=?3) AND m.lifecycle='active' AND m.memory_kind IN ('link','related') ORDER BY CASE m.memory_kind WHEN 'link' THEN 0 ELSE 1 END,e.memory_id", params![kind, reference, origin],ScopedRows::Link(n))? {
                if let Some((s, t, e)) = self.link(&id)? {
                    if s.id == n.id { out.push((t, e)); } else if t.id == n.id { out.push((s, e)); }
                }
            }
        }
        Ok(())
    }

    fn neighbours(&mut self, n: &TopologyNode) -> Result<Vec<Adjacency>> {
        let mut out = Vec::new();
        if n.kind == TopologyNodeKind::Article && !self.in_scope(n)? {
            return Ok(out);
        }
        self.typed_links(n, &mut out)?;
        match n.kind {
            TopologyNodeKind::Entity => self.actor_neighbours(n, &mut out)?,
            TopologyNodeKind::Claim | TopologyNodeKind::Inference => {
                self.memory_neighbours(n, &mut out)?
            }
            TopologyNodeKind::Article => self.article_neighbours(n, &mut out)?,
            TopologyNodeKind::Topic => self.topic_neighbours(n, &mut out)?,
            TopologyNodeKind::Source => self.source_neighbours(n, &mut out)?,
            TopologyNodeKind::ActorGroup => {}
        }
        out.sort_by(|(a, e), (b, f)| {
            edge_priority(e)
                .cmp(&edge_priority(f))
                .then_with(|| a.kind.label().cmp(b.kind.label()))
                .then_with(|| a.id.cmp(&b.id))
                .then_with(|| e.id.cmp(&f.id))
        });
        // A materialized mentioned_in Link and its native mention receipt
        // describe the same observation. Preserve the canonical Link ID and
        // both receipts rather than inflating the relationship count.
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

    fn actor_neighbours(&mut self, n: &TopologyNode, out: &mut Vec<Adjacency>) -> Result<()> {
        for actor_id in &n.member_ids {
            for [mid, role, state, provenance] in self.scoped_rows::<4>("SELECT r.memory_id,r.role,r.binding_state,r.provenance_json FROM brain_memory_actor_refs r LEFT JOIN memory_metadata m ON m.memory_id=r.memory_id WHERE r.actor_id=?1 AND (m.lifecycle IS NULL OR m.lifecycle='active') AND (m.memory_kind IS NULL OR m.memory_kind IN ('claim','atomic_claim','inference')) ORDER BY r.memory_id,r.role", [actor_id],ScopedRows::Memory(0))? {
                if let Some(memory) = self.memory_node(&mid)? {
                    let mut e = edge(format!("actor-ref:{mid}:{actor_id}:{role}"), &n.id, &memory.id, TopologyEdgeKind::Concerns, provenance);
                    e.status = format!("recorded · {state}"); e.rationale = format!("Recorded actor role: {role}"); e.member_ids.push(actor_id.clone());
                    out.push((memory, e));
                }
            }
            // Typed payloads remain authoritative when a producer (including
            // scope analysis) has no reverse-reference index row yet. Query
            // the exact canonical actor/redirect member, never a display name.
            for [mid,role,state] in self.scoped_rows::<3>("SELECT DISTINCT m.memory_id,COALESCE(json_extract(reference.value,'$.role'),'mentioned'),COALESCE(json_extract(reference.value,'$.binding_state'),'provisional') FROM memory_metadata m,json_each(CASE WHEN json_valid(m.payload_json) THEN json_extract(m.payload_json,'$.payload.actors') ELSE '[]' END) reference WHERE m.lifecycle='active' AND m.memory_kind IN ('claim','atomic_claim','inference') AND json_extract(reference.value,'$.actor_id')=?1 ORDER BY m.memory_id,2,3",[actor_id],ScopedRows::Memory(0))? {
                if let Some(memory)=self.memory_node(&mid)? {
                    let mut e=edge(format!("actor-ref:{mid}:{actor_id}:{role}"),&n.id,&memory.id,TopologyEdgeKind::Concerns,format!("memory_metadata:{mid}@{}",memory.revision));
                    e.status=format!("recorded · {state}");e.rationale=format!("Recorded actor role: {role}");e.member_ids.push(actor_id.clone());out.push((memory,e));
                }
            }
            for [kind, scope, id, revision, spans] in self.scoped_rows::<5>("SELECT origin_kind,origin_scope,origin_id,origin_revision,spans_json FROM brain_actor_mentions WHERE actor_id=?1 ORDER BY origin_kind,origin_scope,origin_id,origin_revision", [actor_id],ScopedRows::Mention {kind:0,run:1})? {
                let neighbour = if kind == "article" { self.article_node(&scope, &id, &revision)? }
                    else if kind == "thread" { self.store.get_thread(&id)?.map(|t| { let mut s=source_node("thread", &id, &revision); s.label=t.title; s }) }
                    else { Some(source_node(&kind, &id, &revision)) };
                if let Some(other) = neighbour {
                    let mut e = edge(format!("mention:{actor_id}:{kind}:{scope}:{id}:{revision}"), &n.id, &other.id, TopologyEdgeKind::MentionedIn, format!("brain_actor_mentions:{actor_id}:{kind}:{scope}:{id}@{revision}\n{spans}"));
                    e.member_ids.push(actor_id.clone()); out.push((other, e));
                }
            }
        }
        Ok(())
    }

    fn memory_neighbours(&mut self, n: &TopologyNode, out: &mut Vec<Adjacency>) -> Result<()> {
        for [aid, role, state, receipt] in self.scoped_rows::<4>("SELECT actor_id,role,binding_state,provenance_json FROM brain_memory_actor_refs WHERE memory_id=?1 ORDER BY actor_id,role", [&n.id],ScopedRows::Memory(0))? {
            if let Some(actor) = self.memory_node(&aid)? {
                let mut e = edge(format!("actor-ref:{}:{aid}:{role}", n.id), &actor.id, &n.id, TopologyEdgeKind::Concerns, receipt);
                e.member_ids.push(aid); e.status=format!("recorded · {state}"); e.rationale=format!("Recorded actor role: {role}"); out.push((actor,e));
            }
        }
        if let Some(payload) = self.store.memory_payload(&n.id)? {
            let references = match payload {
                MemoryPayload::Claim(p) => p.actors,
                MemoryPayload::Inference(p) => p.actors,
                _ => Vec::new(),
            };
            if references.len() > self.limit {
                self.exhausted = false;
            }
            for reference in references.into_iter().take(self.limit) {
                if let Some(actor) = self.memory_node(&reference.actor_id)? {
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
        }
        // Direct provenance is independent of tentative Related article pairs.
        let current = current_metadata_sql("s", "a");
        let provenance_sql=format!("SELECT DISTINCT a.run_id,a.article_id,'insight_derivations:'||d.id FROM insight_derivations d JOIN insight_snapshots s ON s.id=d.snapshot_id LEFT JOIN insight_derivation_evidence e ON e.derivation_id=d.id JOIN atlas_articles a ON ((s.owner_kind IN ('article','atlas') AND s.owner_id=a.article_id) OR e.source_id='article:'||a.article_id) WHERE d.memory_id=?1 AND d.status NOT IN ('deleted','retired','rejected') AND {current} UNION SELECT a.run_id,a.article_id,'insight_sources:'||s.fingerprint||':'||s.answer_id||':'||s.call_id FROM insight_sources s JOIN insight_claims c ON c.fingerprint=s.fingerprint JOIN atlas_articles a ON (s.call_id=a.article_id OR s.call_id='article:'||a.article_id) AND (s.run_id=a.run_id OR s.run_id=a.article_id OR s.run_id IS NULL) WHERE c.memory_id=?1 AND s.deleted_origin=0 ORDER BY 1,2,3");
        for [run, id, receipt] in
            self.scoped_rows::<3>(&provenance_sql, [&n.id], ScopedRows::Article(0))?
        {
            if let Some(article) = self.article_node(&run, &id, "")? {
                let historical = receipt.starts_with("insight_sources:");
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
                if historical {
                    e.status = "historical origin · revision unavailable".into();
                    e.label = "historical origin".into();
                }
                out.push((article, e));
            }
        }
        for [source,revision,receipt] in self.scoped_rows::<3>("SELECT DISTINCT e.source_id,e.source_revision,e.evidence_json FROM insight_derivation_evidence e JOIN insight_derivations d ON d.id=e.derivation_id WHERE d.memory_id=?1 AND d.status NOT IN ('deleted','retired','rejected') ORDER BY e.source_id,e.source_revision,e.evidence_json", [&n.id],ScopedRows::Evidence {source:0,revision:1})? {
            let s=source_node("evidence",&source,&revision);
            out.push((s.clone(),edge(format!("evidence:{}:{}",s.id,n.id),&s.id,&n.id,TopologyEdgeKind::Source,receipt)));
        }
        for [thread,title] in self.scoped_rows::<2>("SELECT DISTINCT t.id,t.title FROM insight_sources s JOIN insight_claims c ON c.fingerprint=s.fingerprint JOIN recon_threads t ON t.id=s.thread_id WHERE c.memory_id=?1 AND s.deleted_origin=0 AND t.deleted=0 ORDER BY t.id", [&n.id],ScopedRows::Never)? {
            let mut s=source_node("thread",&thread,""); s.label=title;
            out.push((s.clone(),edge(format!("thread-evidence:{thread}:{}",n.id),&s.id,&n.id,TopologyEdgeKind::Source,format!("recon-thread:{thread}"))));
        }
        for [topic,fingerprint] in self.rows::<2>(
            "SELECT DISTINCT topic,fingerprint FROM insight_claims WHERE memory_id=?1 ORDER BY topic,fingerprint",
            [&n.id],
        )? {
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
        self.inference_neighbours(n, out)
    }

    fn inference_neighbours(&mut self, n: &TopologyNode, out: &mut Vec<Adjacency>) -> Result<()> {
        if n.kind == TopologyNodeKind::Inference {
            if let Some(MemoryPayload::Inference(p)) = self.store.memory_payload(&n.id)? {
                if p.premise_claims.len() > self.limit {
                    self.exhausted = false;
                }
                for id in p.premise_claims.into_iter().take(self.limit) {
                    if let Some(c) = self
                        .memory_node(&id)?
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
            for [snapshot,json,receipt] in self.rows::<3>("SELECT snapshot_id,item_json,id FROM insight_derivations WHERE memory_id=?1 AND insight_kind='inference' AND status NOT IN ('deleted','retired','rejected') ORDER BY id", [&n.id])? {
                let value:Value=serde_json::from_str(&json)?;
                if value["premise_claim_ids"].as_array().is_some_and(|premises|premises.len()>self.limit){self.exhausted=false;}
                for premise in value["premise_claim_ids"].as_array().into_iter().flatten().take(self.limit) {
                    let Some(local)=premise.as_str() else {continue;};
                    let id:Option<String>=self.store.conn.query_row("SELECT memory_id FROM insight_derivations WHERE id=?1 AND snapshot_id=?2 AND insight_kind='claim' AND status NOT IN ('deleted','retired','rejected')",params![format!("{snapshot}:{local}"),snapshot],|r| r.get(0)).optional()?.flatten();
                    if let Some(c)=match id {Some(id)=>self.memory_node(&id)?,None=>None} {
                        let mut e=edge(format!("premise:{}:{}",c.id,n.id),&c.id,&n.id,TopologyEdgeKind::Premise,format!("{receipt}:premise={local}"));e.rationale=value["rationale"].as_str().unwrap_or("").into();out.push((c,e));
                    } else {self.gaps.insert(format!("{receipt}: unresolved snapshot-local premise {local}"));}
                }
            }
        } else {
            for [id] in self.scoped_rows::<1>("SELECT DISTINCT m.memory_id FROM memory_metadata m,json_each(CASE WHEN json_valid(m.payload_json) THEN json_extract(m.payload_json,'$.payload.premise_claims') ELSE '[]' END) p WHERE m.memory_kind='inference' AND m.lifecycle='active' AND p.value=?1 ORDER BY m.memory_id",[&n.id],ScopedRows::Memory(0))? {
                if let Some(i)=self.memory_node(&id)? {out.push((i.clone(),edge(format!("premise:{}:{id}",n.id),&n.id,&id,TopologyEdgeKind::Premise,format!("memory_metadata:{id}@{}",i.revision))));}
            }
            for [id,receipt,local] in self.scoped_rows::<3>("SELECT DISTINCT i.memory_id,i.id,p.value FROM insight_derivations c JOIN insight_derivations i ON i.snapshot_id=c.snapshot_id,json_each(CASE WHEN json_valid(i.item_json) THEN json_extract(i.item_json,'$.premise_claim_ids') ELSE '[]' END) p WHERE c.memory_id=?1 AND c.insight_kind='claim' AND i.insight_kind='inference' AND i.memory_id IS NOT NULL AND c.status NOT IN ('deleted','retired','rejected') AND i.status NOT IN ('deleted','retired','rejected') AND c.id=c.snapshot_id||':'||p.value ORDER BY i.memory_id,i.id,p.value",[&n.id],ScopedRows::Memory(0))? {
                if let Some(i)=self.memory_node(&id)? {out.push((i,edge(format!("premise:{}:{id}",n.id),&n.id,&id,TopologyEdgeKind::Premise,format!("{receipt}:premise={local}"))));}
            }
        }
        Ok(())
    }

    fn article_neighbours(&mut self, n: &TopologyNode, out: &mut Vec<Adjacency>) -> Result<()> {
        for [aid,revision,receipt] in self.rows::<3>("SELECT actor_id,origin_revision,spans_json FROM brain_actor_mentions WHERE origin_kind='article' AND origin_scope=?1 AND origin_id=?2 ORDER BY actor_id,origin_revision",params![n.run_id,n.article_id])? {
            if !revision.is_empty() && revision!=n.revision {self.gaps.insert(format!("{}: stale actor mention {aid}@{revision}",n.id));continue;}
            if let Some(a)=self.memory_node(&aid)? {let mut e=edge(format!("mention:{aid}:article:{}:{}:{revision}",n.run_id,n.article_id),&a.id,&n.id,TopologyEdgeKind::MentionedIn,format!("brain_actor_mentions:{aid}:article:{}/{}@{revision}\n{receipt}",n.run_id,n.article_id));e.member_ids.push(aid);out.push((a,e));}
        }
        let current = current_metadata_sql("s", "a");
        let provenance_sql=format!("SELECT DISTINCT d.memory_id,'insight_derivations:'||d.id FROM insight_derivations d JOIN insight_snapshots s ON s.id=d.snapshot_id LEFT JOIN insight_derivation_evidence e ON e.derivation_id=d.id JOIN atlas_articles a ON a.article_id=?1 AND a.run_id=?2 WHERE d.memory_id IS NOT NULL AND d.status NOT IN ('deleted','retired','rejected') AND ((s.owner_kind IN ('article','atlas') AND s.owner_id=?1) OR e.source_id='article:'||?1) AND {current} UNION SELECT c.memory_id,'insight_sources:'||s.fingerprint||':'||s.answer_id||':'||s.call_id FROM insight_claims c JOIN insight_sources s ON s.fingerprint=c.fingerprint WHERE s.deleted_origin=0 AND (s.call_id=?1 OR s.call_id='article:'||?1) AND (s.run_id=?2 OR s.run_id=?1 OR s.run_id IS NULL) ORDER BY 1,2");
        for [id, receipt] in self.scoped_rows::<2>(
            &provenance_sql,
            params![n.article_id, n.run_id],
            ScopedRows::Memory(0),
        )? {
            if let Some(c) = self.memory_node(&id)? {
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
        for [left,right,lr,rr,shared] in self.rows::<5>("SELECT left_id,right_id,left_revision,right_revision,shared_json FROM atlas_related_articles WHERE run_id=?1 AND (left_id=?2 OR right_id=?2) ORDER BY left_id,right_id",params![n.run_id,n.article_id])? {
            let Some(la)=self.store.atlas_article(&n.run_id,&left)? else {continue;};let Some(ra)=self.store.atlas_article(&n.run_id,&right)? else {continue;};
            let related_revision=|a:&crate::store::AtlasArticleRow|crate::nlp::cache::hash(&format!("{}:{}",crate::atlas_metadata::input_hash(a),a.category));
            if related_revision(&la)!=lr || related_revision(&ra)!=rr {self.gaps.insert(format!("related:{left}/{right}: stale retained pair"));continue;}
            let other=if left==n.article_id {&right}else{&left};
            if let Some(a)=self.article_node(&n.run_id,other,"")? {let mut e=edge(format!("atlas-related:{}/{left}/{right}",n.run_id),&format!("article:{}/{left}",n.run_id),&format!("article:{}/{right}",n.run_id),TopologyEdgeKind::Related,shared);e.status="tentative".into();e.label="possible context".into();e.rationale="Retained shared-entity lead; not corroboration.".into();out.push((a,e));}
        }
        Ok(())
    }

    fn topic_neighbours(&mut self, n: &TopologyNode, out: &mut Vec<Adjacency>) -> Result<()> {
        let key = n.label.trim().to_lowercase();
        for [id,topic,fingerprint] in self.scoped_rows::<3>("SELECT DISTINCT c.memory_id,c.topic,c.fingerprint FROM insight_claims c LEFT JOIN memory_metadata m ON m.memory_id=c.memory_id WHERE lower(trim(c.topic))=?1 AND (m.lifecycle IS NULL OR m.lifecycle='active') ORDER BY c.memory_id,c.fingerprint",[&key],ScopedRows::Memory(0))? {
            if let Some(c)=self.memory_node(&id)? {out.push((c,edge(format!("topic:{}:{id}",n.id),&id,&n.id,TopologyEdgeKind::Contains,format!("insight_claims:{fingerprint}:topic={topic}"))));}
        }
        for [run,id,category] in self.scoped_rows::<3>("SELECT run_id,article_id,category FROM atlas_articles WHERE lower(trim(category))=?1 ORDER BY run_id,article_id",[key],ScopedRows::Article(0))? {
            if let Some(a)=self.article_node(&run,&id,"")? {out.push((a.clone(),edge(format!("topic:{}:{}",n.id,a.id),&a.id,&n.id,TopologyEdgeKind::Contains,format!("{}@{}:category={category}",a.id,a.revision))));}
        }
        Ok(())
    }

    fn source_neighbours(&mut self, n: &TopologyNode, out: &mut Vec<Adjacency>) -> Result<()> {
        let Some(TopologySeed::Source {
            origin,
            source_id,
            revision,
        }) = &n.topology_seed
        else {
            return Ok(());
        };
        if origin == "evidence" {
            for [id,receipt] in self.scoped_rows::<2>("SELECT DISTINCT d.memory_id,e.evidence_json FROM insight_derivation_evidence e JOIN insight_derivations d ON d.id=e.derivation_id WHERE e.source_id=?1 AND e.source_revision=?2 AND d.memory_id IS NOT NULL AND d.status NOT IN ('deleted','retired','rejected') ORDER BY d.memory_id,e.evidence_json",params![source_id,revision],ScopedRows::Memory(0))? {
                if let Some(m)=self.memory_node(&id)? {out.push((m,edge(format!("evidence:{}:{id}",n.id),&n.id,&id,TopologyEdgeKind::Source,receipt)));}
            }
        } else if origin == "thread" {
            for [id] in self.scoped_rows::<1>("SELECT DISTINCT c.memory_id FROM insight_sources s JOIN insight_claims c ON c.fingerprint=s.fingerprint JOIN recon_threads t ON t.id=s.thread_id WHERE s.thread_id=?1 AND s.deleted_origin=0 AND t.deleted=0 ORDER BY c.memory_id",[source_id],ScopedRows::Memory(0))? {if let Some(m)=self.memory_node(&id)? {out.push((m,edge(format!("thread-evidence:{source_id}:{id}"),&n.id,&id,TopologyEdgeKind::Source,format!("recon-thread:{source_id}"))));}}
        }
        Ok(())
    }
}

impl Store {
    /// Read one revision-consistent, source-scoped neighbourhood. All query
    /// branches and exploration have hard bounds; omitted counts are discovered
    /// omissions, not guesses about unvisited database rows.
    pub fn topology_snapshot(
        &self,
        seed: &TopologySeed,
        options: &TopologyOptions,
    ) -> Result<TopologySnapshot> {
        ensure!(
            options.continuation_token.is_none(),
            "Topology continuation is unavailable; refresh with a larger bounded limit"
        );
        let transaction = if self.conn.is_autocommit() {
            Some(self.conn.unchecked_transaction()?)
        } else {
            None
        };
        let result = self.topology_snapshot_on(seed, options);
        if let Some(tx) = transaction {
            tx.commit()?;
        }
        result
    }

    fn topology_snapshot_on(
        &self,
        seed: &TopologySeed,
        options: &TopologyOptions,
    ) -> Result<TopologySnapshot> {
        let mut q = Query {
            store: self,
            limit: options.max_nodes.saturating_mul(2).clamp(32, 256),
            scope: &options.cycle_scope,
            exhausted: true,
            gaps: BTreeSet::new(),
            remaining_rows: options.max_nodes.saturating_mul(64).clamp(4096, 32768),
        };
        let mut snapshot = TopologySnapshot {
            seed: Some(seed.clone()),
            memory_changed_seq: self.memories_changed_seq()?.max(0) as u64,
            ..Default::default()
        };
        let mut roots = Vec::new();
        let mut initial_edges = Vec::new();
        let mut group_members = BTreeSet::new();
        let mut retained_article = None;
        let mut retained_actor_mention = false;
        match seed {
            TopologySeed::Claim(id) => {
                if let Some(n) = q
                    .memory_node(id)?
                    .filter(|n| n.kind == TopologyNodeKind::Claim)
                {
                    roots.push(n);
                }
            }
            TopologySeed::Actor(id) => {
                if let Some(n) = q
                    .memory_node(id)?
                    .filter(|n| n.kind == TopologyNodeKind::Entity)
                {
                    roots.push(n);
                }
            }
            TopologySeed::Inference(id) => {
                if let Some(n) = q
                    .memory_node(id)?
                    .filter(|n| n.kind == TopologyNodeKind::Inference)
                {
                    roots.push(n);
                }
            }
            TopologySeed::ActorGroup {
                name_key,
                member_ids,
            } => {
                let mut valid = Vec::new();
                if member_ids.len() > q.limit {
                    q.exhausted = false;
                }
                let mut display_name = name_key.clone();
                for id in member_ids.iter().take(q.limit) {
                    if let Some(n) = q
                        .memory_node(id)?
                        .filter(|n| n.kind == TopologyNodeKind::Entity)
                    {
                        group_members.insert(n.id.clone());
                        display_name = n.label.clone();
                        valid.push(id.clone());
                    }
                }
                if !valid.is_empty() {
                    valid.sort();
                    valid.dedup();
                    let mut n = node(
                        format!("actor-group:{name_key}"),
                        TopologyNodeKind::ActorGroup,
                        display_name,
                    );
                    n.status = "display group · identity unmerged".into();
                    n.subtype = "actor_name_group".into();
                    n.detail = format!(
                        "{} retained member bindings; sharing a name does not establish identity.",
                        valid.len()
                    );
                    n.member_ids = valid;
                    n.topology_seed = Some(seed.clone());
                    roots.push(n);
                }
            }
            TopologySeed::Article {
                run_id,
                article_id,
                revision,
                ..
            } => {
                if let Some(n) = q.article_node(run_id, article_id, revision)? {
                    roots.push(n);
                }
            }
            TopologySeed::Topic(label) => {
                if let Some(n) = topic_node(label) {
                    roots.push(n);
                }
            }
            TopologySeed::Source {
                origin,
                source_id,
                revision,
            } => roots.push(source_node(origin, source_id, revision)),
            TopologySeed::Link(id) | TopologySeed::Related(id) => {
                if let Some((s, t, e)) = q.link(id)? {
                    roots.push(s);
                    roots.push(t);
                    initial_edges.push(e);
                }
            }
            TopologySeed::RetainedClaim {
                run_id,
                article_id,
                revision,
                local_id,
                text,
                job_id,
            } => {
                if let Some(article) = q.article_node(run_id, article_id, revision)? {
                    let publication:Option<(String,String)>=self.conn.query_row("SELECT p.job_id,p.claims_json FROM intel_brief_publications p JOIN intel_brief_runs r ON r.job_id=p.job_id WHERE p.article_id=?1 AND p.job_id=?2 AND r.state='completed' AND NOT EXISTS(SELECT 1 FROM intel_brief_runs newer WHERE newer.article_id=r.article_id AND newer.generation>r.generation)",params![article_id,job_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
                    let matches = publication
                        .as_ref()
                        .and_then(|(_, json)| serde_json::from_str::<Value>(json).ok())
                        .is_some_and(|v| {
                            v["claims"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .any(|c| c["id"] == *local_id && c["text"] == *text)
                        });
                    let mut canonical = None;
                    let mut projection_status = "retained source · not indexed";
                    if matches {
                        if let Some(state) = crate::intel_recon::brief_canonical::index_state(
                            &self.conn, article_id,
                        )?
                        .filter(|s| s.job_id == *job_id && s.state == "indexed")
                        {
                            let id:Option<String>=self.conn.query_row("SELECT memory_id FROM insight_derivations WHERE snapshot_id=?1 AND id=?2 AND insight_kind='claim' AND status='assessed'",params![state.snapshot_id,format!("{}:{local_id}",state.snapshot_id)],|r|r.get(0)).optional()?.flatten();
                            if q.snapshot_matches_article(&state.snapshot_id, run_id, article_id)? {
                                if let Some(id) = id {
                                    canonical = q.memory_node(&id)?;
                                }
                            } else {
                                projection_status =
                                    "retained source · canonical revision unavailable";
                                q.gaps.insert(format!("{job_id}:{local_id}: canonical publication metadata does not match the current retained article or is unavailable"));
                            }
                        }
                    }
                    if let Some(n) = canonical {
                        roots.push(n);
                    } else {
                        let mut n = node(seed.key(), TopologyNodeKind::Claim, text.clone());
                        n.status = projection_status.into();
                        n.subtype = "retained_claim".into();
                        n.article_id = article_id.clone();
                        n.run_id = run_id.clone();
                        n.cycle = Some(run_id.clone());
                        n.revision = revision.clone();
                        n.topology_seed = Some(seed.clone());
                        n.passages.push(format!(
                            "Intel {job_id}:{local_id} · {}/{}@{}",
                            run_id, article_id, revision
                        ));
                        retained_article = Some(article);
                        roots.push(n);
                    }
                }
            }
            TopologySeed::RetainedActor {
                run_id,
                article_id,
                revision,
                actor_ref,
                name,
            } => {
                if let Some(article) = q.article_node(run_id, article_id, revision)? {
                    let mut atlas_ref = String::new();
                    if let (Some(row), Some(metadata)) = (
                        self.atlas_article(run_id, article_id)?,
                        self.atlas_metadata(run_id, article_id)?,
                    ) {
                        if let Some(native) = crate::atlas_metadata::article_actors(&row, &metadata)
                            .into_iter()
                            .find(|candidate| candidate.id == *actor_ref)
                        {
                            retained_actor_mention = true;
                            atlas_ref = format!(
                                "{}:{}",
                                native.type_hint,
                                crate::atlas_metadata::comparison_key(&native.display_name)
                            );
                        }
                    }
                    let current = current_metadata_sql("s", "origin_article");
                    let actor_sql=format!("SELECT DISTINCT a.actor_id FROM brain_actors a WHERE (a.actor_id=?1 OR EXISTS(SELECT 1 FROM brain_actor_external_refs x WHERE x.actor_id=a.actor_id AND ((x.legacy_id=?1 AND x.scope IN ('atlas:'||?3,'recon_entities')) OR (?5<>'' AND x.legacy_id=?5 AND x.scope='atlas:'||?3)))) AND (EXISTS(SELECT 1 FROM brain_actor_mentions m WHERE m.actor_id=a.actor_id AND m.origin_kind='article' AND m.origin_scope=?2 AND m.origin_id=?3 AND m.origin_revision=?4) OR EXISTS(SELECT 1 FROM brain_memory_actor_refs r JOIN insight_derivations d ON d.memory_id=r.memory_id JOIN insight_snapshots s ON s.id=d.snapshot_id JOIN atlas_articles origin_article ON origin_article.run_id=?2 AND origin_article.article_id=?3 WHERE r.actor_id=a.actor_id AND s.owner_kind='article' AND s.owner_id=?3 AND d.status='assessed' AND {current})) ORDER BY a.actor_id");
                    let ids = q.rows::<1>(
                        &actor_sql,
                        params![actor_ref, run_id, article_id, revision, atlas_ref],
                    )?;
                    let mut canonical = BTreeMap::new();
                    for [id] in ids {
                        if let Some(n) = q.memory_node(&id)? {
                            canonical.insert(n.id.clone(), n);
                        }
                    }
                    if canonical.len() == 1 {
                        roots.push(canonical.into_values().next().expect("one canonical actor"));
                    } else {
                        let mut n = node(seed.key(), TopologyNodeKind::Entity, name.clone());
                        n.status = "retained source · provisional".into();
                        n.subtype = "retained_actor".into();
                        n.article_id = article_id.clone();
                        n.run_id = run_id.clone();
                        n.cycle = Some(run_id.clone());
                        n.revision = revision.clone();
                        n.topology_seed = Some(seed.clone());
                        n.passages.push(format!(
                            "Metadata actor {actor_ref} · {run_id}/{article_id}@{revision}"
                        ));
                        retained_article = Some(article);
                        roots.push(n);
                    }
                }
            }
        }
        let Some(root) = roots.first() else {
            snapshot.findings.gaps.push(
                "The selected record is missing, revised, or has no supported typed endpoint."
                    .into(),
            );
            snapshot.unresolved_endpoints = q.gaps.into_iter().collect();
            snapshot.exploration_exhausted = q.exhausted;
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
        let retained_article = match retained_article {
            Some(article) if q.in_scope(&article)? => Some(article),
            _ => None,
        };
        if let Some(mut article) = retained_article {
            article.min_hops = 1;
            article.predecessor = Some(root_id.clone());
            let kind =
                if matches!(seed, TopologySeed::RetainedActor { .. }) && retained_actor_mention {
                    TopologyEdgeKind::MentionedIn
                } else {
                    TopologyEdgeKind::Source
                };
            let mut e = if kind == TopologyEdgeKind::Source {
                edge(
                    format!("retained-evidence:{root_id}"),
                    &article.id,
                    &root_id,
                    kind,
                    format!("{}@{}", article.id, article.revision),
                )
            } else {
                edge(
                    format!("retained-mention:{root_id}"),
                    &root_id,
                    &article.id,
                    kind,
                    format!("{}@{}", article.id, article.revision),
                )
            };
            if matches!(seed, TopologySeed::RetainedActor { .. }) && !retained_actor_mention {
                e.label = "retained source context".into();
                e.status = "unresolved actor binding".into();
                e.rationale="The retained display actor is scoped to this source; no current native mention or canonical identity is asserted.".into();
            }
            edges.insert(e.id.clone(), e);
            queue.push_back(article.id.clone());
            nodes.insert(article.id.clone(), article);
        }
        let candidate_limit = options.max_nodes.saturating_mul(8).clamp(256, 4096);
        let edge_limit = options.max_edges.saturating_mul(8).clamp(512, 8192);
        while let Some(id) = queue.pop_front() {
            let current = nodes[&id].clone();
            if current.min_hops >= options.depth.clamp(1, 3) {
                continue;
            }
            let mut adjacent = if current.kind == TopologyNodeKind::ActorGroup {
                let mut all = Vec::new();
                for member in &group_members {
                    if let Some(actor) = q.memory_node(member)? {
                        all.extend(q.neighbours(&actor)?);
                        if all.len() >= edge_limit {
                            q.exhausted = false;
                            all.truncate(edge_limit);
                            break;
                        }
                    }
                }
                all
            } else {
                q.neighbours(&current)?
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
                if !q.in_scope(&other)? {
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
        snapshot.total_nodes = nodes.len();
        snapshot.total_edges = edges.len();
        snapshot.discovered_nodes = nodes.len();
        snapshot.discovered_edges = edges.len();
        let mut ordered: Vec<_> = nodes.into_values().collect();
        ordered.sort_by(|a, b| {
            b.is_root
                .cmp(&a.is_root)
                .then_with(|| a.min_hops.cmp(&b.min_hops))
                .then_with(|| a.predecessor.cmp(&b.predecessor))
                .then_with(|| a.kind.label().cmp(b.kind.label()))
                .then_with(|| a.id.cmp(&b.id))
        });
        let mut kept = BTreeSet::new();
        let mut chosen_edges = BTreeMap::new();
        // Admit a node only together with its explanatory path. The seed stays
        // visible even when a caller accidentally supplies a zero-node budget.
        for n in ordered {
            if snapshot.nodes.len() >= options.max_nodes.max(1) {
                continue;
            }
            if let Some(parent) = &n.predecessor {
                if !kept.contains(parent) {
                    continue;
                }
                let mut paths: Vec<_> = edges
                    .values()
                    .filter(|e| {
                        (e.from == n.id && e.to == *parent) || (e.to == n.id && e.from == *parent)
                    })
                    .collect();
                paths.sort_by(|a, b| {
                    edge_priority(a)
                        .cmp(&edge_priority(b))
                        .then_with(|| a.id.cmp(&b.id))
                });
                let Some(path) = paths.first() else {
                    continue;
                };
                if chosen_edges.len() >= options.max_edges {
                    continue;
                }
                chosen_edges.insert(path.id.clone(), (*path).clone());
            }
            kept.insert(n.id.clone());
            snapshot.nodes.push(n);
        }
        let mut extra: Vec<_> = edges
            .into_values()
            .filter(|e| kept.contains(&e.from) && kept.contains(&e.to))
            .collect();
        extra.sort_by(|a, b| {
            edge_priority(a)
                .cmp(&edge_priority(b))
                .then_with(|| a.id.cmp(&b.id))
        });
        for e in extra {
            if chosen_edges.len() >= options.max_edges {
                break;
            }
            chosen_edges.entry(e.id.clone()).or_insert(e);
        }
        snapshot.edges = chosen_edges.into_values().collect();
        snapshot.omitted_nodes = snapshot
            .discovered_nodes
            .saturating_sub(snapshot.nodes.len());
        snapshot.omitted_edges = snapshot
            .discovered_edges
            .saturating_sub(snapshot.edges.len());
        snapshot.exploration_exhausted = q.exhausted;
        snapshot.unresolved_endpoints = q.gaps.into_iter().collect();
        snapshot.findings.gaps = snapshot.unresolved_endpoints.clone();
        if snapshot.omitted_nodes > 0
            || snapshot.omitted_edges > 0
            || !snapshot.exploration_exhausted
        {
            snapshot.findings.gaps.push("Bounded projection: some relationships are omitted; increase the bounded limit to expand. No continuation cursor is available.".into());
        }
        if snapshot.nodes.len() == 1 && snapshot.edges.is_empty() {
            snapshot
                .findings
                .coverage
                .push("No indexed neighbours in the selected scope.".into());
        }
        snapshot.findings.coverage.push(format!(
            "{} nodes · {} relationships visible · {} discovered candidates{}",
            snapshot.nodes.len(),
            snapshot.edges.len(),
            snapshot.discovered_nodes,
            if snapshot.exploration_exhausted {
                ""
            } else {
                " · exploration bounded"
            }
        ));
        for e in &snapshot.edges {
            if e.kind == TopologyEdgeKind::Link {
                snapshot
                    .findings
                    .recorded
                    .push(format!("{} · {}", e.id, e.label));
            }
        }
        for n in &mut snapshot.nodes {
            n.evidence_count = snapshot
                .edges
                .iter()
                .filter(|e| e.kind == TopologyEdgeKind::Source && (e.from == n.id || e.to == n.id))
                .count();
            if !n.is_root {
                match n.kind {
                    TopologyNodeKind::Claim => {
                        if let Some(memory) = self.get_memory(&n.id)? {
                            snapshot.associations.claims.push(memory);
                        }
                    }
                    TopologyNodeKind::Entity => {
                        snapshot
                            .associations
                            .actors
                            .push(crate::brain_browse::ActorGroup {
                                name_key: actors::normalize_alias(&n.label),
                                display_name: n.label.clone(),
                                actor_ids: n.member_ids.clone(),
                            })
                    }
                    TopologyNodeKind::Article => {
                        if let Some(article) = self.atlas_article(&n.run_id, &n.article_id)? {
                            snapshot.associations.articles.push(
                                crate::brain_browse::ArticleAssociation {
                                    article,
                                    occurrences: vec![(
                                        n.run_id.clone(),
                                        n.article_id.clone(),
                                        n.revision.clone(),
                                    )],
                                },
                            );
                        }
                    }
                    TopologyNodeKind::Source => {
                        if let Some(TopologySeed::Source {
                            origin, source_id, ..
                        }) = &n.topology_seed
                        {
                            if origin == "thread" {
                                snapshot
                                    .associations
                                    .threads
                                    .push((source_id.clone(), n.label.clone()));
                            }
                        }
                    }
                    _ => {}
                }
            }
            if n.kind == TopologyNodeKind::Inference {
                snapshot
                    .findings
                    .active_inferences
                    .push(format!("{} · {}", n.id, n.label));
            }
        }
        Ok(snapshot)
    }
}
