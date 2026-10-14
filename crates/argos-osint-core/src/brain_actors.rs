//! Brain actor registry and typed relationship writer (spec §4).
//!
//! One identity authority for every app: Atlas, Intel, Recon, claims,
//! inferences, graphs and Related browsing bind through [`bind`]. Names are
//! search keys, never identities. An Actor is reused only through
//! (1) an existing scoped external reference, (2) a trusted stable identifier
//! (account handle, domain, IP …) with exactly one live owner, or (3) an
//! assessed merge (redirect). Otherwise a deterministic **source-scoped
//! provisional** Actor is created/reused and same-name actors elsewhere are
//! returned as labelled *candidates*, never merged. Reads follow acyclic
//! redirects; merge receipts preserve old IDs, source bindings and history.
//!
//! Actor, Link and Related memories are written by one transactional writer
//! with deterministic IDs, so repeated sightings and restarts never duplicate
//! identities or relationships. Endpoint existence/type/scope and relationship
//! vocabulary are validated before commit.

use anyhow::{bail, ensure, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::memory_metadata::{
    ActorPayload, ActorSubtype, EndpointRef, IdentityState, LinkPayload, MemoryKind, MemoryPayload,
    RelatedPayload,
};

/// Identity/index contract version (folded into recall-cache and Links
/// dependency identity).
pub const IDENTITY_INDEX_VERSION: &str = "brain-actors-v1";
pub const PAYLOAD_SCHEMA: i64 = 1;
/// Bounded materialized Related records per newly indexed article.
pub const RELATED_PER_ARTICLE: usize = 20;
/// Relationship vocabulary for assessed/observed Link memories.
pub const LINK_RELATIONS: &[&str] = &[
    "mentioned_in",
    "supports",
    "contradicts",
    "revises",
    "entity_connection",
    "derived_from",
];

/// Lower-cased, punctuation-free, whitespace-collapsed alias key.
pub fn normalize_alias(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_lowercase().next().unwrap_or(c)
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn hid(prefix: &str, parts: &[&str]) -> String {
    let h = crate::nlp::cache::hash(&parts.join("\u{1f}"));
    format!("{prefix}-{}", &h[..h.len().min(24)])
}

pub fn subtype_str(s: ActorSubtype) -> &'static str {
    match s {
        ActorSubtype::Person => "person",
        ActorSubtype::Organization => "organization",
        ActorSubtype::Place => "place",
        ActorSubtype::Account => "account",
        ActorSubtype::Domain => "domain",
        ActorSubtype::Ip => "ip",
        ActorSubtype::Unknown => "unknown",
    }
}

pub fn parse_subtype(raw: &str) -> ActorSubtype {
    match raw.trim().to_ascii_lowercase().as_str() {
        "person" | "people" | "per" => ActorSubtype::Person,
        "org" | "organization" | "organisation" | "company" => ActorSubtype::Organization,
        "place" | "location" | "gpe" | "loc" | "country" | "city" => ActorSubtype::Place,
        "account" | "username" | "handle" | "email" | "phone" => ActorSubtype::Account,
        "domain" | "website" | "url" => ActorSubtype::Domain,
        "ip" | "ip_address" | "ipaddress" => ActorSubtype::Ip,
        _ => ActorSubtype::Unknown,
    }
}

fn state_str(s: IdentityState) -> &'static str {
    match s {
        IdentityState::Provisional => "provisional",
        IdentityState::Ambiguous => "ambiguous",
        IdentityState::Resolved => "resolved",
    }
}

fn parse_state(raw: &str) -> IdentityState {
    match raw {
        "resolved" => IdentityState::Resolved,
        "ambiguous" => IdentityState::Ambiguous,
        _ => IdentityState::Provisional,
    }
}

/// Stable identifiers are trusted only for subtypes that are themselves
/// identifiers (an account handle, a domain, an IP).
fn trusted_identifier_subtype(s: ActorSubtype) -> bool {
    matches!(
        s,
        ActorSubtype::Account | ActorSubtype::Domain | ActorSubtype::Ip
    )
}

/// One bind/create request from any caller.
#[derive(Clone, Debug)]
pub struct BindRequest<'a> {
    pub name: &'a str,
    pub subtype: ActorSubtype,
    /// Deterministic source scope of a provisional actor, e.g.
    /// `atlas-article:<article_id>` or `recon-thread:<thread_id>`.
    pub scope: &'a str,
    /// Trusted stable identifiers (normalized; only honoured for identifier
    /// subtypes).
    pub identifiers: &'a [String],
    /// Scoped legacy/source-local ID mapped through `brain_actor_external_refs`.
    pub external: Option<(&'a str, &'a str)>,
    pub provenance: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    /// Canonical (redirect-followed) Actor ID.
    pub actor_id: String,
    pub created: bool,
    pub state: IdentityState,
    /// How the actor was chosen: `external_ref`, `identifier`, `scoped`.
    pub basis: &'static str,
    /// Other live actors with the same normalized name/subtype. Candidates
    /// only — never "same actor" facts.
    pub candidates: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActorRow {
    pub actor_id: String,
    pub subtype: String,
    pub display_name: String,
    pub identity_state: IdentityState,
    pub scope_key: String,
    pub revision: i64,
}

/// Follow redirects (acyclic by construction; bounded defensively).
pub fn resolve(conn: &Connection, actor_id: &str) -> Result<String> {
    let mut id = actor_id.to_string();
    for _ in 0..64 {
        let next: Option<String> = conn
            .query_row(
                "SELECT to_actor FROM brain_actor_redirects WHERE from_actor=?1",
                [&id],
                |r| r.get(0),
            )
            .optional()?;
        match next {
            Some(n) => id = n,
            None => return Ok(id),
        }
    }
    bail!("actor redirect chain from {actor_id} does not terminate")
}

pub fn actor(conn: &Connection, actor_id: &str) -> Result<Option<ActorRow>> {
    let id = resolve(conn, actor_id)?;
    Ok(conn
        .query_row(
            "SELECT actor_id,subtype,display_name,identity_state,scope_key,revision FROM brain_actors WHERE actor_id=?1",
            [&id],
            |r| {
                Ok(ActorRow {
                    actor_id: r.get(0)?,
                    subtype: r.get(1)?,
                    display_name: r.get(2)?,
                    identity_state: parse_state(&r.get::<_, String>(3)?),
                    scope_key: r.get(4)?,
                    revision: r.get(5)?,
                })
            },
        )
        .optional()?)
}

/// Live (non-redirected) actors whose alias matches `name` (any subtype when
/// `subtype` is None). Lookup accelerator only.
pub fn candidates(
    conn: &Connection,
    name: &str,
    subtype: Option<ActorSubtype>,
) -> Result<Vec<String>> {
    let key = normalize_alias(name);
    let mut stmt = conn.prepare(
        "SELECT DISTINCT a.actor_id FROM brain_actor_aliases al
         JOIN brain_actors a ON a.actor_id=al.actor_id
         WHERE al.normalized=?1 AND al.alias_type='name'
           AND (?2='' OR a.subtype=?2)
           AND a.actor_id NOT IN (SELECT from_actor FROM brain_actor_redirects)
         ORDER BY a.actor_id",
    )?;
    let rows = stmt
        .query_map(params![key, subtype.map(subtype_str).unwrap_or("")], |r| {
            r.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn text_of(payload: &MemoryPayload) -> String {
    match payload {
        MemoryPayload::Actor(a) => {
            format!("Actor: {} ({})", a.display_name, subtype_str(a.subtype))
        }
        MemoryPayload::Link(l) => format!(
            "Link: {} {} {}",
            l.source.index_key().1,
            l.relation.replace('_', " "),
            l.target.index_key().1
        ),
        MemoryPayload::Related(r) => format!(
            "Related (tentative): {} ↔ {} — {}",
            r.source.index_key().1,
            r.target.index_key().1,
            r.basis
        ),
        MemoryPayload::Claim(_) | MemoryPayload::Inference(_) => String::new(),
    }
}

/// Insert or revise one typed memory row (memories + memory_metadata).
/// Returns true when the stored payload changed.
fn upsert_memory(
    conn: &Connection,
    id: &str,
    payload: &MemoryPayload,
    scope: &str,
    subjects: &[String],
    now: &str,
) -> Result<bool> {
    let kind = payload.kind();
    let payload_json = serde_json::to_string(payload)?;
    let existing: Option<(String, String)> = conn
        .query_row(
            "SELECT memory_kind,payload_json FROM memory_metadata WHERE memory_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((stored_kind, stored)) = &existing {
        ensure!(
            stored_kind == kind.as_str(),
            "memory {id} exists with kind {stored_kind}, not {}",
            kind.as_str()
        );
        if stored == &payload_json {
            return Ok(false);
        }
    }
    let source = json!({"app":"brain","conversation_id": if scope.is_empty() {"brain"} else {scope},"message_id":null,"reference":null});
    conn.execute(
        "INSERT INTO memories(id,text,category,pinned,created_at,source_json) VALUES(?1,?2,'fact',0,?3,?4)
         ON CONFLICT(id) DO UPDATE SET text=excluded.text",
        params![id, text_of(payload), now, source.to_string()],
    )?;
    conn.execute(
        "INSERT INTO memory_metadata(memory_id,memory_kind,subjects_json,payload_json,payload_schema,revision,lifecycle,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?5,1,'active',?6,?6)
         ON CONFLICT(memory_id) DO UPDATE SET payload_json=excluded.payload_json,
           subjects_json=excluded.subjects_json, revision=memory_metadata.revision+1,
           lifecycle='active', updated_at=excluded.updated_at",
        params![
            id,
            kind.as_str(),
            serde_json::to_string(subjects)?,
            payload_json,
            PAYLOAD_SCHEMA,
            now
        ],
    )?;
    Ok(true)
}

fn add_alias(
    conn: &Connection,
    actor_id: &str,
    alias: &str,
    alias_type: &str,
    prov: &Value,
) -> Result<()> {
    let normalized = if alias_type == "name" {
        normalize_alias(alias)
    } else {
        alias.trim().to_lowercase()
    };
    if normalized.is_empty() {
        return Ok(());
    }
    conn.execute(
        "INSERT OR IGNORE INTO brain_actor_aliases(actor_id,alias,normalized,alias_type,provenance_json) VALUES(?1,?2,?3,?4,?5)",
        params![actor_id, alias.trim(), normalized, alias_type, prov.to_string()],
    )?;
    Ok(())
}

/// Candidate lookup → scoped bind/create → canonical reference. Never waits
/// for a model; consequential ambiguity is surfaced through `candidates`.
/// Returns the binding and the memory IDs whose index state changed.
pub fn bind(conn: &Connection, req: &BindRequest<'_>, now: &str) -> Result<(Binding, Vec<String>)> {
    let name = req.name.trim();
    ensure!(!name.is_empty(), "actor name is empty");
    let subtype = req.subtype;
    let others = |conn: &Connection, keep: &str| -> Result<Vec<String>> {
        Ok(candidates(conn, name, Some(subtype))?
            .into_iter()
            .filter(|c| c != keep)
            .collect())
    };
    // 1. Scoped external reference.
    if let Some((scope, legacy)) = req.external {
        let found: Option<String> = conn
            .query_row(
                "SELECT actor_id FROM brain_actor_external_refs WHERE scope=?1 AND legacy_id=?2",
                params![scope, legacy],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(found) = found {
            let id = resolve(conn, &found)?;
            let row =
                actor(conn, &id)?.ok_or_else(|| anyhow::anyhow!("dangling actor ref {id}"))?;
            add_alias(conn, &id, name, "name", &req.provenance)?;
            return Ok((
                Binding {
                    candidates: others(conn, &id)?,
                    actor_id: id,
                    created: false,
                    state: row.identity_state,
                    basis: "external_ref",
                },
                vec![],
            ));
        }
    }
    // 2. Trusted stable identifier with exactly one live owner.
    if trusted_identifier_subtype(subtype) {
        for ident in req.identifiers {
            let key = ident.trim().to_lowercase();
            if key.is_empty() {
                continue;
            }
            let mut stmt = conn.prepare(
                "SELECT DISTINCT a.actor_id FROM brain_actor_aliases al JOIN brain_actors a ON a.actor_id=al.actor_id
                 WHERE al.normalized=?1 AND al.alias_type='identifier' AND a.subtype=?2
                   AND a.actor_id NOT IN (SELECT from_actor FROM brain_actor_redirects)",
            )?;
            let owners = stmt
                .query_map(params![key, subtype_str(subtype)], |r| {
                    r.get::<_, String>(0)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if owners.len() == 1 {
                let id = owners[0].clone();
                add_alias(conn, &id, name, "name", &req.provenance)?;
                link_external(conn, req, &id, now)?;
                return Ok((
                    Binding {
                        candidates: others(conn, &id)?,
                        actor_id: id,
                        created: false,
                        state: IdentityState::Resolved,
                        basis: "identifier",
                    },
                    vec![],
                ));
            }
        }
    }
    // 3. Deterministic source-scoped provisional actor.
    let trusted: Vec<String> = if trusted_identifier_subtype(subtype) {
        req.identifiers
            .iter()
            .map(|i| i.trim().to_lowercase())
            .filter(|i| !i.is_empty())
            .collect()
    } else {
        vec![]
    };
    let (scope_key, state) = if trusted.is_empty() {
        (req.scope.to_string(), IdentityState::Provisional)
    } else {
        (String::new(), IdentityState::Resolved)
    };
    let seed = if trusted.is_empty() {
        normalize_alias(name)
    } else {
        trusted[0].clone()
    };
    let id = hid("actor", &[subtype_str(subtype), &scope_key, &seed]);
    let canonical = resolve(conn, &id)?;
    let exists = actor(conn, &canonical)?.is_some();
    let mut changed = Vec::new();
    if !exists {
        let payload = MemoryPayload::Actor(ActorPayload {
            subtype,
            display_name: name.to_string(),
            aliases: vec![name.to_string()],
            identifiers: trusted.clone(),
            identity_state: state,
        });
        upsert_memory(conn, &id, &payload, req.scope, &[id.clone()], now)?;
        conn.execute(
            "INSERT INTO brain_actors(actor_id,subtype,display_name,identity_state,scope_key,revision,provenance_json,created_at,updated_at)
             VALUES(?1,?2,?3,?4,?5,1,?6,?7,?7)",
            params![id, subtype_str(subtype), name, state_str(state), scope_key, req.provenance.to_string(), now],
        )?;
        for ident in &trusted {
            add_alias(conn, &id, ident, "identifier", &req.provenance)?;
        }
        changed.push(id.clone());
    }
    add_alias(conn, &canonical, name, "name", &req.provenance)?;
    link_external(conn, req, &canonical, now)?;
    let row = actor(conn, &canonical)?.expect("actor just ensured");
    Ok((
        Binding {
            candidates: others(conn, &canonical)?,
            actor_id: canonical,
            created: !exists,
            state: row.identity_state,
            basis: "scoped",
        },
        changed,
    ))
}

fn link_external(conn: &Connection, req: &BindRequest<'_>, id: &str, now: &str) -> Result<()> {
    if let Some((scope, legacy)) = req.external {
        conn.execute(
            "INSERT OR IGNORE INTO brain_actor_external_refs(scope,legacy_id,actor_id,created_at) VALUES(?1,?2,?3,?4)",
            params![scope, legacy, id, now],
        )?;
    }
    Ok(())
}

/// Idempotent mention receipt for an exact Article/Source origin/revision.
#[allow(clippy::too_many_arguments)]
pub fn record_mention(
    conn: &Connection,
    actor_id: &str,
    origin_kind: &str,
    origin_scope: &str,
    origin_id: &str,
    origin_revision: &str,
    spans: &Value,
    now: &str,
) -> Result<String> {
    let id = hid(
        "mention",
        &[
            actor_id,
            origin_kind,
            origin_scope,
            origin_id,
            origin_revision,
        ],
    );
    conn.execute(
        "INSERT INTO brain_actor_mentions(id,actor_id,origin_kind,origin_scope,origin_id,origin_revision,spans_json,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
         ON CONFLICT(id) DO UPDATE SET spans_json=excluded.spans_json",
        params![id, actor_id, origin_kind, origin_scope, origin_id, origin_revision, spans.to_string(), now],
    )?;
    Ok(id)
}

/// Reference an Actor from another memory's payload (claim subject …).
pub fn add_memory_actor_ref(
    conn: &Connection,
    memory_id: &str,
    actor_id: &str,
    role: &str,
    state: IdentityState,
    provenance: &Value,
) -> Result<()> {
    let actor_id = resolve(conn, actor_id)?;
    ensure!(
        actor(conn, &actor_id)?.is_some(),
        "unknown actor {actor_id}"
    );
    conn.execute(
        "INSERT INTO brain_memory_actor_refs(memory_id,actor_id,role,binding_state,provenance_json) VALUES(?1,?2,?3,?4,?5)
         ON CONFLICT(memory_id,actor_id,role) DO UPDATE SET binding_state=excluded.binding_state",
        params![memory_id, actor_id, role, state_str(state), provenance.to_string()],
    )?;
    sync_claim_actor_payload(conn, memory_id)?;
    Ok(())
}

/// Keep the typed Claim payload and reference index in the same transaction.
pub fn sync_claim_actor_payload(conn: &Connection, memory_id: &str) -> Result<()> {
    use crate::memory_metadata::{ActorRef, ClaimPayload};
    let raw: Option<(String, String)> = conn
        .query_row(
            "SELECT memory_kind,payload_json FROM memory_metadata WHERE memory_id=?1",
            [memory_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if raw
        .as_ref()
        .is_some_and(|(kind, _)| MemoryKind::parse(kind) != Some(MemoryKind::Claim))
    {
        return Ok(());
    }
    let mut payload = raw
        .and_then(|(_, json)| serde_json::from_str::<MemoryPayload>(&json).ok())
        .and_then(|p| match p {
            MemoryPayload::Claim(c) => Some(c),
            _ => None,
        })
        .unwrap_or_else(|| ClaimPayload {
            assessment: "unassessed".into(),
            ..Default::default()
        });
    let mut stmt = conn.prepare("SELECT actor_id,role,binding_state FROM brain_memory_actor_refs WHERE memory_id=?1 ORDER BY actor_id,role")?;
    payload.actors = stmt
        .query_map([memory_id], |r| {
            Ok(ActorRef {
                actor_id: r.get(0)?,
                role: r.get(1)?,
                binding_state: parse_state(&r.get::<_, String>(2)?),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute("INSERT INTO memory_metadata(memory_id,memory_kind,payload_json,payload_schema,revision,lifecycle,created_at,updated_at) VALUES(?1,'claim',?2,1,1,'active',?3,?3) ON CONFLICT(memory_id) DO UPDATE SET payload_json=excluded.payload_json,payload_schema=1,revision=memory_metadata.revision+1,updated_at=excluded.updated_at WHERE memory_metadata.payload_json<>excluded.payload_json", params![memory_id,serde_json::to_string(&MemoryPayload::Claim(payload))?,now])?;
    Ok(())
}

fn validate_endpoint(conn: &Connection, e: &EndpointRef) -> Result<()> {
    match e {
        EndpointRef::Memory { memory_id, kind } => {
            let id = if *kind == MemoryKind::Actor {
                resolve(conn, memory_id)?
            } else {
                memory_id.clone()
            };
            let stored: Option<String> = conn
                .query_row(
                    "SELECT memory_kind FROM memory_metadata WHERE memory_id=?1",
                    [&id],
                    |r| r.get(0),
                )
                .optional()?;
            match stored.as_deref().and_then(MemoryKind::parse) {
                Some(k) if k == *kind => Ok(()),
                Some(k) => bail!(
                    "endpoint {id} is a {} memory, not {}",
                    k.as_str(),
                    kind.as_str()
                ),
                None => bail!("endpoint memory {id} does not exist"),
            }
        }
        EndpointRef::Article {
            run_id, article_id, ..
        } => {
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM atlas_articles WHERE article_id=?1 AND (?2='' OR run_id=?2)",
                params![article_id, run_id],
                |r| r.get(0),
            )?;
            ensure!(
                n > 0,
                "endpoint article {run_id}/{article_id} does not exist"
            );
            Ok(())
        }
        EndpointRef::Source {
            origin, source_id, ..
        } => {
            ensure!(
                !origin.trim().is_empty() && !source_id.trim().is_empty(),
                "source endpoint needs origin and id"
            );
            Ok(())
        }
    }
}

fn endpoint_key(e: &EndpointRef) -> String {
    let (k, id, origin, rev) = e.index_key();
    format!("{k}:{origin}:{id}:{rev}")
}

fn write_endpoints(
    conn: &Connection,
    id: &str,
    src: &EndpointRef,
    tgt: &EndpointRef,
) -> Result<()> {
    for (side, e) in [("source", src), ("target", tgt)] {
        let (kind, ref_id, origin, revision) = e.index_key();
        conn.execute(
            "INSERT INTO brain_memory_endpoints(memory_id,side,endpoint_kind,ref_id,origin,revision) VALUES(?1,?2,?3,?4,?5,?6)
             ON CONFLICT(memory_id,side) DO UPDATE SET endpoint_kind=excluded.endpoint_kind, ref_id=excluded.ref_id,
               origin=excluded.origin, revision=excluded.revision",
            params![id, side, kind, ref_id, origin, revision],
        )?;
    }
    Ok(())
}

fn actor_ids(e: &EndpointRef) -> Option<String> {
    match e {
        EndpointRef::Memory {
            memory_id,
            kind: MemoryKind::Actor,
        } => Some(memory_id.clone()),
        _ => None,
    }
}

/// Validated, deterministic Link memory. Same relation + endpoints → same ID.
pub fn write_link(conn: &Connection, link: &LinkPayload, now: &str) -> Result<(String, bool)> {
    ensure!(
        LINK_RELATIONS.contains(&link.relation.as_str()),
        "unknown link relation {}",
        link.relation
    );
    validate_endpoint(conn, &link.source)?;
    validate_endpoint(conn, &link.target)?;
    if let Some(original) = &link.promoted_from {
        let kind: Option<String> = conn
            .query_row(
                "SELECT memory_kind FROM memory_metadata WHERE memory_id=?1",
                [original],
                |r| r.get(0),
            )
            .optional()?;
        ensure!(
            kind.as_deref() == Some("related"),
            "promotion must reference a Related memory"
        );
    }
    let id = hid(
        "link",
        &[
            &link.relation,
            &endpoint_key(&link.source),
            &endpoint_key(&link.target),
        ],
    );
    let subjects: Vec<String> = [&link.source, &link.target]
        .iter()
        .filter_map(|e| actor_ids(e))
        .collect();
    let changed = upsert_memory(
        conn,
        &id,
        &MemoryPayload::Link(link.clone()),
        "",
        &subjects,
        now,
    )?;
    write_endpoints(conn, &id, &link.source, &link.target)?;
    for a in &subjects {
        add_memory_actor_ref(
            conn,
            &id,
            a,
            "endpoint",
            IdentityState::Provisional,
            &json!({}),
        )?;
    }
    Ok((id, changed))
}

/// Validated, deterministic tentative Related memory (endpoint order
/// independent). Supplies no support or identity.
pub fn write_related(
    conn: &Connection,
    related: &RelatedPayload,
    now: &str,
) -> Result<(String, bool)> {
    ensure!(
        !related.basis.trim().is_empty() || !related.shared_actors.is_empty(),
        "a Related record needs shared actors or an explicit basis"
    );
    validate_endpoint(conn, &related.source)?;
    validate_endpoint(conn, &related.target)?;
    let mut keys = [endpoint_key(&related.source), endpoint_key(&related.target)];
    keys.sort();
    let id = hid("related", &[&keys[0], &keys[1], &related.basis]);
    let mut shared = Vec::new();
    for a in &related.shared_actors {
        shared.push(resolve(conn, a)?);
    }
    let changed = upsert_memory(
        conn,
        &id,
        &MemoryPayload::Related(related.clone()),
        "",
        &shared,
        now,
    )?;
    write_endpoints(conn, &id, &related.source, &related.target)?;
    for a in &shared {
        add_memory_actor_ref(
            conn,
            &id,
            a,
            "shared",
            IdentityState::Provisional,
            &json!({}),
        )?;
    }
    Ok((id, changed))
}

/// Assessed merge: `from` redirects to `to`. Acyclic; both must be live.
/// Old IDs, mentions, refs and aliases are preserved (reads follow the
/// redirect); the identity-index revision advances on the survivor.
pub fn merge(conn: &Connection, from: &str, to: &str, receipt: &Value, now: &str) -> Result<()> {
    let from = resolve(conn, from)?;
    let to = resolve(conn, to)?;
    ensure!(from != to, "cannot merge an actor into itself");
    ensure!(
        actor(conn, &from)?.is_some() && actor(conn, &to)?.is_some(),
        "unknown actor"
    );
    conn.execute(
        "INSERT INTO brain_actor_redirects(from_actor,to_actor,receipt_json,created_at) VALUES(?1,?2,?3,?4)",
        params![from, to, receipt.to_string(), now],
    )?;
    conn.execute(
        "UPDATE brain_actors SET identity_state='resolved', revision=revision+1, updated_at=?2 WHERE actor_id=?1",
        params![to, now],
    )?;
    conn.execute(
        "UPDATE memory_metadata SET lifecycle='redirected', updated_at=?2 WHERE memory_id=?1",
        params![from, now],
    )?;
    Ok(())
}

/// Split (undo a merge): `from` becomes live again with its own history.
pub fn split(conn: &Connection, from: &str, now: &str) -> Result<()> {
    let to: Option<String> = conn
        .query_row(
            "SELECT to_actor FROM brain_actor_redirects WHERE from_actor=?1",
            [from],
            |r| r.get(0),
        )
        .optional()?;
    let Some(to) = to else {
        bail!("{from} is not merged")
    };
    conn.execute(
        "DELETE FROM brain_actor_redirects WHERE from_actor=?1",
        [from],
    )?;
    conn.execute(
        "UPDATE memory_metadata SET lifecycle='active', updated_at=?2 WHERE memory_id=?1",
        params![from, now],
    )?;
    conn.execute(
        "UPDATE brain_actors SET revision=revision+1, updated_at=?2 WHERE actor_id IN (?1,?3)",
        params![from, now, to],
    )?;
    Ok(())
}

/// Every actor ID that reads as `actor_id` (itself plus merged-in IDs).
pub fn equivalent_ids(conn: &Connection, actor_id: &str) -> Result<Vec<String>> {
    let root = resolve(conn, actor_id)?;
    let mut out = vec![root.clone()];
    let mut frontier = vec![root];
    while let Some(id) = frontier.pop() {
        let mut stmt =
            conn.prepare("SELECT from_actor FROM brain_actor_redirects WHERE to_actor=?1")?;
        let rows = stmt
            .query_map([&id], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for r in rows {
            if !out.contains(&r) {
                out.push(r.clone());
                frontier.push(r);
            }
        }
    }
    Ok(out)
}

/// `(origin_kind, origin_scope, origin_id, origin_revision)` mentions of an
/// actor and everything merged into it.
pub fn mentions(
    conn: &Connection,
    actor_id: &str,
) -> Result<Vec<(String, String, String, String)>> {
    let ids = equivalent_ids(conn, actor_id)?;
    let mut out = Vec::new();
    for id in ids {
        let mut stmt = conn.prepare(
            "SELECT origin_kind,origin_scope,origin_id,origin_revision FROM brain_actor_mentions WHERE actor_id=?1 ORDER BY origin_scope,origin_id",
        )?;
        let rows = stmt
            .query_map([&id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        out.extend(rows);
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// Remove one origin's contributions (cycle deletion). Shared actors and
/// records supported elsewhere survive; mention Links/Related records whose
/// article endpoint belonged only to this origin are retired. Actors with no
/// remaining mention, ref or user edit are retired (never deleted while
/// pinned or referenced).
pub fn retire_origin(
    conn: &Connection,
    origin_kind: &str,
    origin_scope: &str,
    now: &str,
) -> Result<usize> {
    let affected: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT actor_id FROM brain_actor_mentions WHERE origin_kind=?1 AND origin_scope=?2",
        )?;
        let rows = stmt
            .query_map(params![origin_kind, origin_scope], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    conn.execute(
        "DELETE FROM brain_actor_mentions WHERE origin_kind=?1 AND origin_scope=?2",
        params![origin_kind, origin_scope],
    )?;
    let retired = conn.execute(
        "UPDATE memory_metadata SET lifecycle='retired', updated_at=?3
         WHERE lifecycle='active' AND memory_kind IN ('link','related') AND memory_id IN (
           SELECT memory_id FROM brain_memory_endpoints WHERE endpoint_kind=?1 AND origin=?2)",
        params![origin_kind, origin_scope, now],
    )?;
    for actor_id in affected {
        let alive: i64 = conn.query_row(
            "SELECT (SELECT COUNT(*) FROM brain_actor_mentions WHERE actor_id=?1)
                  + (SELECT COUNT(*) FROM brain_memory_actor_refs r JOIN memory_metadata m ON m.memory_id=r.memory_id
                     WHERE r.actor_id=?1 AND m.lifecycle='active' AND r.memory_id<>?1)
                  + (SELECT COUNT(*) FROM brain_actor_redirects WHERE to_actor=?1)
                  + (SELECT COUNT(*) FROM memories WHERE id=?1 AND pinned=1)",
            [&actor_id],
            |r| r.get(0),
        )?;
        if alive == 0 {
            conn.execute(
                "UPDATE memory_metadata SET lifecycle='retired', updated_at=?2 WHERE memory_id=?1",
                params![actor_id, now],
            )?;
        }
    }
    Ok(retired)
}

impl crate::store::Store {
    /// Transactional bind through [`bind`] (actor memory indexed via the outbox).
    pub fn bind_actor(&self, req: &BindRequest<'_>) -> Result<Binding> {
        let now = chrono::Utc::now().to_rfc3339();
        self.write_then_index(|| bind(&self.conn, req, &now))
    }

    pub fn write_link_memory(&self, link: &LinkPayload) -> Result<String> {
        let now = chrono::Utc::now().to_rfc3339();
        self.write_then_index(|| Ok((write_link(&self.conn, link, &now)?.0, vec![])))
    }

    pub fn write_related_memory(&self, related: &RelatedPayload) -> Result<String> {
        let now = chrono::Utc::now().to_rfc3339();
        self.write_then_index(|| Ok((write_related(&self.conn, related, &now)?.0, vec![])))
    }

    pub fn merge_actors(&self, from: &str, to: &str, receipt: &Value) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.write_then_index(|| Ok((merge(&self.conn, from, to, receipt, &now)?, vec![])))
    }

    pub fn split_actor(&self, from: &str) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.write_then_index(|| Ok((split(&self.conn, from, &now)?, vec![])))
    }

    /// Typed payload of a core memory (validated discriminated decode).
    pub fn memory_payload(&self, memory_id: &str) -> Result<Option<MemoryPayload>> {
        let raw: Option<String> = self
            .conn
            .query_row(
                "SELECT payload_json FROM memory_metadata WHERE memory_id=?1 AND payload_schema>0",
                [memory_id],
                |r| r.get(0),
            )
            .optional()?;
        raw.map(|r| serde_json::from_str(&r).map_err(Into::into))
            .transpose()
    }
}

#[cfg(test)]
#[path = "brain_actors_tests.rs"]
mod tests;
