//! Brain browse associations on the async repository (migration phase 4a):
//! `Store::brain_browse_memories` (Actor-group claim, thread and article
//! counts), `claim_associations`, `actor_group_associations` and
//! `article_origins_for_memory`, with the SQLite shapes and rules.
//!
//! One read loads the association inputs (redirects, typed claim actors,
//! actor refs, mentions, Link endpoints, insight sources, live derivation
//! evidence, threads and Atlas articles); grouping happens in Rust, so a
//! browse costs a fixed number of queries rather than several per claim.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{anyhow, bail, Result};
use surrealdb::types::SurrealValue;

use super::repo::Repository;
use crate::brain::{Memory, MemorySource};
use crate::brain_browse::{ActorGroup, ArticleAssociation, Associations, BrowseRow};
use crate::memory_metadata::{MemoryKind, MemoryPayload};
use crate::related_articles::{canonical_url, ArticleOrigin};
use crate::store::AtlasArticleRow;

#[derive(Debug, Clone, SurrealValue)]
struct Pair {
    a: String,
    b: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct SourceRow {
    memory_id: Option<String>,
    thread_id: Option<String>,
    run_id: Option<String>,
    call_id: String,
    source_url: Option<String>,
}

#[derive(Debug, Clone, SurrealValue)]
struct DerivationRow {
    memory_id: String,
    owner_kind: Option<String>,
    owner_id: Option<String>,
    source_id: Option<String>,
    evidence_json: Option<String>,
}

#[derive(Debug, Clone, SurrealValue)]
struct EndpointRow {
    memory_id: String,
    endpoint_kind: String,
    ref_id: String,
    origin: String,
    revision: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct MentionRow {
    actor_id: String,
    origin_kind: String,
    origin_scope: String,
    origin_id: String,
    origin_revision: String,
}

/// Everything associations are computed from.
struct Inputs {
    redirects: BTreeMap<String, String>,
    payloads: BTreeMap<String, String>,
    actor_refs: Vec<Pair>,
    mentions: Vec<MentionRow>,
    endpoints: Vec<EndpointRow>,
    active_links: BTreeSet<String>,
    sources: Vec<SourceRow>,
    derivations: Vec<DerivationRow>,
    threads: BTreeMap<String, String>,
    articles: Vec<AtlasArticleRow>,
}

const INPUTS_SURQL: &str = "
SELECT meta::id(id) AS a, to_actor AS b FROM brain_actor_redirect;
SELECT meta::id(id) AS a, payload_json AS b FROM memory WHERE payload_json != '';
SELECT memory_id AS a, actor_id AS b FROM brain_memory_actor_ref;
SELECT actor_id, origin_kind, origin_scope, origin_id, origin_revision FROM brain_actor_mention
  ORDER BY origin_scope, origin_id;
SELECT memory_id, endpoint_kind, ref_id, origin, revision FROM brain_memory_endpoint;
SELECT VALUE meta::id(id) FROM memory WHERE lifecycle = 'active' AND memory_kind = 'link';
SELECT type::record('insight_claim', fingerprint).memory_id AS memory_id, thread_id, run_id,
       call_id, source_url FROM insight_source WHERE deleted_origin = false;
LET $live = (SELECT meta::id(id) AS did, memory_id,
       type::record('insight_snapshot', snapshot_id).owner_kind AS owner_kind,
       type::record('insight_snapshot', snapshot_id).owner_id AS owner_id
  FROM insight_derivation WHERE memory_id != NONE AND status NOTINSIDE ['deleted', 'retired']
    AND record::exists(type::record('insight_snapshot', snapshot_id)));
LET $ev = (SELECT derivation_id, source_id, evidence_json FROM insight_derivation_evidence
  WHERE derivation_id INSIDE $live.did);
RETURN array::flatten($live.map(|$d| {
  LET $rows = $ev.filter(|$e| $e.derivation_id = $d.did);
  RETURN IF array::len($rows) = 0 {
    [{ memory_id: $d.memory_id, owner_kind: $d.owner_kind, owner_id: $d.owner_id,
       source_id: NONE, evidence_json: NONE }]
  } ELSE {
    $rows.map(|$e| { memory_id: $d.memory_id, owner_kind: $d.owner_kind, owner_id: $d.owner_id,
       source_id: $e.source_id, evidence_json: $e.evidence_json })
  };
}));
SELECT meta::id(id) AS a, title AS b FROM recon_thread WHERE deleted = false;";

async fn inputs(repo: &Repository) -> Result<Inputs> {
    let mut r = repo.db().query(INPUTS_SURQL).await?.check()?;
    let redirects: Vec<Pair> = r.take(0usize)?;
    let payloads: Vec<Pair> = r.take(1usize)?;
    let actor_refs: Vec<Pair> = r.take(2usize)?;
    let mentions: Vec<MentionRow> = r.take(3usize)?;
    let endpoints: Vec<EndpointRow> = r.take(4usize)?;
    let active_links: Vec<String> = r.take(5usize)?;
    let sources: Vec<SourceRow> = r.take(6usize)?;
    let derivations: Vec<DerivationRow> = r.take(9usize)?;
    let threads: Vec<Pair> = r.take(10usize)?;
    let mut articles = Vec::new();
    for run in super::atlas::list_runs(repo.db())
        .await
        .map_err(|e| anyhow!("atlas runs: {e}"))?
    {
        articles.extend(
            super::atlas::list_articles(repo.db(), &run.id)
                .await
                .map_err(|e| anyhow!("atlas articles: {e}"))?,
        );
    }
    Ok(Inputs {
        redirects: redirects.into_iter().map(|p| (p.a, p.b)).collect(),
        payloads: payloads.into_iter().map(|p| (p.a, p.b)).collect(),
        actor_refs,
        mentions,
        endpoints,
        active_links: active_links.into_iter().collect(),
        sources,
        derivations,
        threads: threads.into_iter().map(|p| (p.a, p.b)).collect(),
        articles,
    })
}

impl Inputs {
    fn resolve(&self, actor_id: &str) -> Result<String> {
        let mut id = actor_id.to_string();
        for _ in 0..64 {
            match self.redirects.get(&id) {
                Some(n) => id = n.clone(),
                None => return Ok(id),
            }
        }
        bail!("actor redirect chain from {actor_id} does not terminate")
    }

    fn typed_actors(&self, memory_id: &str) -> Result<Option<BTreeSet<String>>> {
        let Some(raw) = self.payloads.get(memory_id) else {
            return Ok(None);
        };
        let payload: MemoryPayload =
            serde_json::from_str(raw).map_err(|e| anyhow!("memory {memory_id} payload: {e}"))?;
        let MemoryPayload::Claim(p) = payload else {
            return Ok(None);
        };
        p.actors
            .iter()
            .map(|a| self.resolve(&a.actor_id))
            .collect::<Result<_>>()
            .map(Some)
    }

    /// Live threads of the claim's insight sources, ordered by title.
    fn claim_threads(&self, memory_id: &str) -> Vec<(String, String)> {
        let mut set = BTreeSet::new();
        for s in &self.sources {
            if s.memory_id.as_deref() != Some(memory_id) {
                continue;
            }
            if let Some(t) = &s.thread_id {
                if let Some(title) = self.threads.get(t) {
                    set.insert((title.clone(), t.clone()));
                }
            }
        }
        set.into_iter().map(|(title, id)| (id, title)).collect()
    }

    /// `Store::article_origins_for_memory`: exact ids first, explicit URLs
    /// only as fallback, per source.
    fn origins(&self, memory_id: &str) -> Vec<ArticleOrigin> {
        let mut refs: Vec<(Option<String>, String, Option<String>)> = Vec::new();
        for d in self.derivations.iter().filter(|d| d.memory_id == memory_id) {
            if d.owner_kind.as_deref() == Some("article") {
                refs.push((None, d.owner_id.clone().unwrap_or_default(), None));
            }
            let url = d
                .evidence_json
                .as_deref()
                .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
                .and_then(|v| v.get("url").and_then(|u| u.as_str()).map(canonical_url));
            let source = d.source_id.clone().unwrap_or_default();
            refs.push((
                None,
                source
                    .strip_prefix("article:")
                    .unwrap_or(&source)
                    .to_string(),
                url,
            ));
        }
        for s in self
            .sources
            .iter()
            .filter(|s| s.memory_id.as_deref() == Some(memory_id))
        {
            refs.push((
                s.run_id.clone(),
                s.call_id
                    .strip_prefix("article:")
                    .unwrap_or(&s.call_id)
                    .to_string(),
                s.source_url.as_deref().map(canonical_url),
            ));
        }
        let mut origins = BTreeSet::new();
        for (run, id, url) in refs {
            let in_run = |a: &&AtlasArticleRow| run.as_ref().is_none_or(|r| a.run_id == *r);
            let exact: Vec<_> = self
                .articles
                .iter()
                .filter(|a| a.id == id)
                .filter(in_run)
                .collect();
            let hits: Vec<_> = if !exact.is_empty() {
                exact
            } else if let Some(url) = url {
                self.articles
                    .iter()
                    .filter(|a| !a.url.is_empty() && canonical_url(&a.url) == url)
                    .filter(in_run)
                    .collect()
            } else {
                vec![]
            };
            for a in hits {
                origins.insert((a.run_id.clone(), a.id.clone()));
            }
        }
        origins
            .into_iter()
            .map(|(run_id, article_id)| ArticleOrigin { run_id, article_id })
            .collect()
    }

    fn article(&self, run: &str, id: &str) -> Option<&AtlasArticleRow> {
        self.articles.iter().find(|a| a.run_id == run && a.id == id)
    }

    /// Group occurrences by canonical URL; a stale revision is skipped.
    fn association_articles(
        &self,
        origins: BTreeSet<(String, String, String)>,
    ) -> Vec<ArticleAssociation> {
        let mut articles: BTreeMap<String, ArticleAssociation> = BTreeMap::new();
        for (run, id, revision) in origins {
            let Some(article) = self.article(&run, &id) else {
                continue;
            };
            let current = crate::atlas_metadata::input_hash(article);
            if !revision.is_empty() && revision != current {
                continue;
            }
            let key = if article.url.is_empty() {
                format!("{run}/{id}")
            } else {
                canonical_url(&article.url)
            };
            articles
                .entry(key)
                .or_insert_with(|| ArticleAssociation {
                    article: article.clone(),
                    occurrences: vec![],
                })
                .occurrences
                .push((run, id, current));
        }
        articles
            .into_values()
            .map(|mut a| {
                a.occurrences.sort();
                a.occurrences.dedup();
                a
            })
            .collect()
    }

    fn claim_origin(&self, memory_id: &str) -> Associations {
        let origins = self
            .origins(memory_id)
            .into_iter()
            .map(|o| (o.run_id, o.article_id, String::new()))
            .collect();
        Associations {
            threads: self.claim_threads(memory_id),
            articles: self.association_articles(origins),
            ..Default::default()
        }
    }
}

struct Context {
    claims: Vec<Memory>,
    typed_actors: BTreeMap<String, BTreeSet<String>>,
    claim_origins: BTreeMap<String, Associations>,
}

async fn context(repo: &Repository, inputs: &Inputs) -> Result<Context> {
    let claims = super::browse::claims(repo).await?;
    let mut typed_actors = BTreeMap::new();
    let mut claim_origins = BTreeMap::new();
    for m in &claims {
        if let Some(ids) = inputs.typed_actors(&m.id)? {
            typed_actors.insert(m.id.clone(), ids);
        }
        claim_origins.insert(m.id.clone(), inputs.claim_origin(&m.id));
    }
    Ok(Context {
        claims,
        typed_actors,
        claim_origins,
    })
}

fn group_associations(group: &ActorGroup, inputs: &Inputs, context: &Context) -> Associations {
    let mut claims = BTreeSet::new();
    let mut threads = BTreeMap::new();
    let mut origins = BTreeSet::new();
    for id in &group.actor_ids {
        // Active Links from this actor to an article.
        for a in inputs
            .endpoints
            .iter()
            .filter(|e| e.endpoint_kind == "memory" && e.ref_id == *id)
        {
            if !inputs.active_links.contains(&a.memory_id) {
                continue;
            }
            for e in inputs
                .endpoints
                .iter()
                .filter(|e| e.memory_id == a.memory_id && e.endpoint_kind == "article")
            {
                origins.insert((e.origin.clone(), e.ref_id.clone(), e.revision.clone()));
            }
        }
        for r in inputs.actor_refs.iter().filter(|r| r.b == *id) {
            if context.claim_origins.contains_key(&r.a) {
                claims.insert(r.a.clone());
            }
        }
        // The group already holds every redirected member: direct mentions only.
        for m in inputs.mentions.iter().filter(|m| m.actor_id == *id) {
            if m.origin_kind == "article" {
                origins.insert((
                    m.origin_scope.clone(),
                    m.origin_id.clone(),
                    m.origin_revision.clone(),
                ));
            } else if m.origin_kind == "thread" {
                if let Some(title) = inputs.threads.get(&m.origin_id) {
                    threads.insert(m.origin_id.clone(), title.clone());
                }
            }
        }
    }
    // Typed references are authoritative.
    for (id, actors) in &context.typed_actors {
        if group.actor_ids.iter().any(|a| actors.contains(a)) {
            claims.insert(id.clone());
        }
    }
    let mut out = Associations::default();
    for id in claims {
        let a = &context.claim_origins[&id];
        for (t, title) in &a.threads {
            threads.insert(t.clone(), title.clone());
        }
        for a in &a.articles {
            origins.extend(a.occurrences.iter().cloned());
        }
        if let Some(m) = context.claims.iter().find(|m| m.id == id) {
            out.claims.push(m.clone());
        }
    }
    out.threads = threads.into_iter().collect();
    out.articles = inputs.association_articles(origins);
    out
}

/// Atlas articles a memory was derived from (live derivations and insight
/// sources), as `Store::article_origins_for_memory`.
pub async fn article_origins_for_memory(
    repo: &Repository,
    memory_id: &str,
) -> Result<Vec<ArticleOrigin>> {
    Ok(inputs(repo).await?.origins(memory_id))
}

/// A claim's Actor groups, threads and articles.
pub async fn claim_associations(repo: &Repository, memory_id: &str) -> Result<Associations> {
    let inputs = inputs(repo).await?;
    let mut actor_ids = BTreeSet::new();
    for r in inputs.actor_refs.iter().filter(|r| r.a == memory_id) {
        actor_ids.insert(inputs.resolve(&r.b)?);
    }
    if let Some(ids) = inputs.typed_actors(memory_id)? {
        actor_ids.extend(ids);
    }
    let mut out = inputs.claim_origin(memory_id);
    out.actors = super::browse::actor_groups(repo)
        .await?
        .into_iter()
        .filter(|g| g.actor_ids.iter().any(|id| actor_ids.contains(id)))
        .collect();
    Ok(out)
}

/// An Actor group's claims, threads and articles.
pub async fn actor_group_associations(
    repo: &Repository,
    group: &ActorGroup,
) -> Result<Associations> {
    let inputs = inputs(repo).await?;
    let context = context(repo, &inputs).await?;
    Ok(group_associations(group, &inputs, &context))
}

/// The TUI list: claims as stored, Actor groups as stable group keys with
/// claim, thread and article counts. Mutations must reject group keys.
pub async fn brain_browse_memories(
    repo: &Repository,
    kind: Option<MemoryKind>,
    query: &str,
) -> Result<Vec<Memory>> {
    let rows = super::browse::brain_browse(repo, kind, query).await?;
    let prepared = if rows.iter().any(|r| matches!(r, BrowseRow::ActorGroup(_))) {
        let inputs = inputs(repo).await?;
        let context = context(repo, &inputs).await?;
        Some((inputs, context))
    } else {
        None
    };
    Ok(rows
        .into_iter()
        .map(|row| match row {
            BrowseRow::Claim(m) => m,
            BrowseRow::ActorGroup(g) => {
                let (inputs, context) = prepared.as_ref().expect("actor context");
                let a = group_associations(&g, inputs, context);
                Memory {
                    id: g.key(),
                    text: g.display_name,
                    category: format!(
                        "{} claims · {} threads · {} articles",
                        a.claims.len(),
                        a.threads.len(),
                        a.articles.len()
                    ),
                    pinned: false,
                    created_at: String::new(),
                    source: MemorySource {
                        app: "brain-actor-group".into(),
                        conversation_id: g.name_key,
                        message_id: None,
                        reference: None,
                    },
                }
            }
        })
        .collect())
}
