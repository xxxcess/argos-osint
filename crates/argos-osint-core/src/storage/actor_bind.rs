//! Actor binding inside a SurrealDB transaction (migration phase 4b, part
//! 2): the ports of `brain_actors::bind`, `add_memory_actor_ref`,
//! `sync_claim_actor_payload` and `brain_backfill::bind_claim_subject` that
//! insight publication needs.
//!
//! `bind_in` follows the SQLite order: scoped external reference, then the
//! sole source-scoped alias match, then the deterministic provisional actor.
//! Trusted identifiers are refused (publication never passes any); the
//! backfill receipt is `storage::entities::put_backfill_receipt_in`.

use std::collections::BTreeSet;

use serde_json::{json, Value};
use surrealdb::types::SurrealValue;

use super::jobs::{rid, run, take};
use super::memory::{put_typed_in, set_lifecycle_in, TypedMemory};
use super::tx::{Tx, TxError};
use crate::brain_actors::{
    hid, normalize_alias, normalize_name, parse_state, parse_subtype, state_str, subtype_str,
    text_of, BindRequest,
};
use crate::memory_metadata::{
    ActorPayload, ActorRef, ActorSubtype, ClaimPayload, IdentityState, MemoryKind, MemoryPayload,
};

fn json_err(e: serde_json::Error) -> TxError {
    TxError::Abort(format!("actor json: {e}"))
}

/// The canonical actor a binding chose and its identity state.
#[derive(Clone, Debug)]
pub struct Bound {
    pub actor_id: String,
    pub state: IdentityState,
}

/// Follow redirects (acyclic by construction; bounded defensively).
pub async fn resolve_in(tx: &Tx, actor_id: &str) -> Result<String, TxError> {
    let mut id = actor_id.to_string();
    for _ in 0..64 {
        let mut r = run(
            tx,
            tx.query("SELECT VALUE to_actor FROM ONLY $r")
                .bind(("r", rid("brain_actor_redirect", &id))),
        )
        .await?;
        match take::<Option<String>>(&mut r, 0)? {
            Some(next) => id = next,
            None => return Ok(id),
        }
    }
    Err(TxError::Abort(format!(
        "actor redirect chain from {actor_id} does not terminate"
    )))
}

#[derive(SurrealValue)]
struct ActorHead {
    subtype: String,
    scope_key: String,
    identity_state: String,
}

async fn head(tx: &Tx, id: &str) -> Result<Option<ActorHead>, TxError> {
    let mut r = run(
        tx,
        tx.query("SELECT subtype, scope_key, identity_state FROM ONLY $a")
            .bind(("a", rid("brain_actor", id))),
    )
    .await?;
    take(&mut r, 0)
}

async fn add_alias_in(tx: &Tx, actor: &str, alias: &str, prov: &Value) -> Result<(), TxError> {
    let normalized = normalize_alias(alias);
    if normalized.is_empty() {
        return Ok(());
    }
    run(
        tx,
        tx.query(
            "IF !record::exists(type::record('brain_actor_alias', $k)) {
               CREATE type::record('brain_actor_alias', $k) SET actor_id = $a, alias = $alias,
                 normalized = $n, alias_type = 'name', provenance_json = $p RETURN NONE;
             };",
        )
        .bind((
            "k",
            vec![actor.to_string(), normalized.clone(), "name".to_string()],
        ))
        .bind(("a", actor.to_string()))
        .bind(("alias", alias.trim().to_string()))
        .bind(("n", normalized))
        .bind(("p", prov.to_string())),
    )
    .await?;
    Ok(())
}

async fn link_external_in(
    tx: &Tx,
    req: &BindRequest<'_>,
    actor: &str,
    now: &str,
) -> Result<(), TxError> {
    if let Some((scope, legacy)) = req.external {
        run(
            tx,
            tx.query(
                "IF !record::exists(type::record('brain_actor_external_ref', $k)) {
                   CREATE type::record('brain_actor_external_ref', $k) SET actor_id = $a,
                     created_at = $now RETURN NONE;
                 };",
            )
            .bind(("k", vec![scope.to_string(), legacy.to_string()]))
            .bind(("a", actor.to_string()))
            .bind(("now", now.to_string())),
        )
        .await?;
    }
    Ok(())
}

/// Reactivate a retired actor memory; returns the ids that changed.
async fn activate_in(tx: &Tx, id: &str) -> Result<Vec<String>, TxError> {
    let mut r = run(
        tx,
        tx.query("SELECT VALUE lifecycle FROM ONLY $m")
            .bind(("m", rid("memory", id))),
    )
    .await?;
    if take::<Option<String>>(&mut r, 0)?.as_deref() == Some("retired") {
        return set_lifecycle_in(tx, &[id.to_string()], "active").await;
    }
    Ok(Vec::new())
}

/// Live actors whose name alias matches `name` and `subtype`, canonical.
async fn candidates_in(
    tx: &Tx,
    name: &str,
    subtype: ActorSubtype,
) -> Result<BTreeSet<String>, TxError> {
    let mut r = run(
        tx,
        tx.query(
            "SELECT VALUE actor_id FROM brain_actor_alias WHERE normalized = $n AND alias_type = 'name'
               AND type::record('brain_actor', actor_id).subtype = $s",
        )
        .bind(("n", normalize_alias(name)))
        .bind(("s", subtype_str(subtype))),
    )
    .await?;
    let mut out = BTreeSet::new();
    for id in take::<Vec<String>>(&mut r, 0)? {
        out.insert(resolve_in(tx, &id).await?);
    }
    Ok(out)
}

/// Candidate lookup, scoped bind or create, canonical reference. Returns the
/// binding and the memory ids whose index state changed.
pub async fn bind_in(
    tx: &Tx,
    req: &BindRequest<'_>,
    now: &str,
) -> Result<(Bound, Vec<String>), TxError> {
    if req.identifiers.iter().any(|i| !i.trim().is_empty()) {
        return Err(TxError::Abort(
            "trusted actor identifiers are not ported to SurrealDB".into(),
        ));
    }
    let name = normalize_name(req.name);
    if name.is_empty() {
        return Err(TxError::Abort("actor name is empty".into()));
    }
    let subtype = req.subtype;
    if let Some((scope, legacy)) = req.external {
        let mut r = run(
            tx,
            tx.query(
                "SELECT VALUE actor_id FROM ONLY type::record('brain_actor_external_ref', $k)",
            )
            .bind(("k", vec![scope.to_string(), legacy.to_string()])),
        )
        .await?;
        if let Some(found) = take::<Option<String>>(&mut r, 0)? {
            let id = resolve_in(tx, &found).await?;
            let row = head(tx, &id)
                .await?
                .ok_or_else(|| TxError::Abort(format!("dangling actor ref {id}")))?;
            add_alias_in(tx, &id, &name, &req.provenance).await?;
            let changed = activate_in(tx, &id).await?;
            let state = parse_state(&row.identity_state);
            return Ok((
                Bound {
                    actor_id: id,
                    state,
                },
                changed,
            ));
        }
    }
    let mut scoped = Vec::new();
    for id in candidates_in(tx, &name, subtype).await? {
        if let Some(row) = head(tx, &id).await? {
            if row.scope_key == req.scope {
                scoped.push((id, parse_state(&row.identity_state)));
            }
        }
    }
    if let [(id, state)] = scoped.as_slice() {
        add_alias_in(tx, id, &name, &req.provenance).await?;
        link_external_in(tx, req, id, now).await?;
        let changed = activate_in(tx, id).await?;
        return Ok((
            Bound {
                actor_id: id.clone(),
                state: *state,
            },
            changed,
        ));
    }
    let id = hid(
        "actor",
        &[subtype_str(subtype), req.scope, &normalize_alias(&name)],
    );
    let canonical = resolve_in(tx, &id).await?;
    let mut changed = Vec::new();
    if head(tx, &canonical).await?.is_none() {
        let payload = MemoryPayload::Actor(ActorPayload {
            subtype,
            display_name: name.clone(),
            aliases: vec![name.clone()],
            identifiers: vec![],
            identity_state: IdentityState::Provisional,
        });
        let source = json!({"app":"brain","conversation_id": if req.scope.is_empty() {"brain"} else {req.scope},"message_id":null,"reference":null});
        run(
            tx,
            tx.query(
                "UPSERT $m SET text = $text, category = 'fact', pinned = false, created_at = <datetime>$now,
                   source_json = $src RETURN NONE;
                 CREATE $a SET subtype = $sub, display_name = $name, identity_state = 'provisional',
                   scope_key = $scope, revision = 1, provenance_json = $p, created_at = $now,
                   updated_at = $now RETURN NONE;",
            )
            .bind(("m", rid("memory", &id)))
            .bind(("a", rid("brain_actor", &id)))
            .bind(("now", now.to_string()))
            .bind(("src", source.to_string()))
            .bind(("text", text_of(&payload)))
            .bind(("sub", subtype_str(subtype)))
            .bind(("name", name.clone()))
            .bind(("scope", req.scope.to_string()))
            .bind(("p", req.provenance.to_string())),
        )
        .await?;
        put_typed_in(
            tx,
            &TypedMemory {
                id: id.clone(),
                text: text_of(&payload),
                kind: MemoryKind::Actor,
                payload: Some(payload),
                lifecycle: "active".into(),
            },
        )
        .await?;
        changed.push(id.clone());
    }
    add_alias_in(tx, &canonical, &name, &req.provenance).await?;
    link_external_in(tx, req, &canonical, now).await?;
    changed.extend(activate_in(tx, &canonical).await?);
    let state = head(tx, &canonical)
        .await?
        .map(|r| parse_state(&r.identity_state))
        .unwrap_or_default();
    Ok((
        Bound {
            actor_id: canonical,
            state,
        },
        changed,
    ))
}

/// Reference an actor from another memory (claim subject, mention …) and
/// keep a claim's typed payload in step.
pub async fn add_memory_actor_ref_in(
    tx: &Tx,
    memory_id: &str,
    actor_id: &str,
    role: &str,
    state: IdentityState,
    provenance: &Value,
) -> Result<(), TxError> {
    let actor = resolve_in(tx, actor_id).await?;
    if head(tx, &actor).await?.is_none() {
        return Err(TxError::Abort(format!("unknown actor {actor}")));
    }
    run(
        tx,
        tx.query(
            "UPSERT type::record('brain_memory_actor_ref', $k) SET memory_id = $m, actor_id = $a,
               role = $role, binding_state = $state, provenance_json = provenance_json ?? $p
               RETURN NONE",
        )
        .bind((
            "k",
            vec![memory_id.to_string(), actor.clone(), role.to_string()],
        ))
        .bind(("m", memory_id.to_string()))
        .bind(("a", actor))
        .bind(("role", role.to_string()))
        .bind(("state", state_str(state)))
        .bind(("p", provenance.to_string())),
    )
    .await?;
    sync_claim_actor_payload_in(tx, memory_id).await
}

#[derive(SurrealValue)]
struct MemoryHead {
    text: String,
    memory_kind: String,
    payload_json: String,
    lifecycle: String,
}

#[derive(SurrealValue)]
struct RefRow {
    actor_id: String,
    role: String,
    binding_state: String,
}

/// Rewrite a claim memory's typed payload: actors from its refs plus `edit`.
pub async fn update_claim_payload_in(
    tx: &Tx,
    memory_id: &str,
    edit: impl FnOnce(&mut ClaimPayload),
) -> Result<(), TxError> {
    let mut r = run(
        tx,
        tx.query(
            "SELECT text, memory_kind, payload_json, lifecycle FROM ONLY $m;
             SELECT actor_id, role, binding_state FROM brain_memory_actor_ref
               WHERE memory_id = $id ORDER BY actor_id, role;",
        )
        .bind(("m", rid("memory", memory_id)))
        .bind(("id", memory_id.to_string())),
    )
    .await?;
    let Some(memory) = take::<Option<MemoryHead>>(&mut r, 0)? else {
        return Ok(());
    };
    if MemoryKind::parse(&memory.memory_kind) != Some(MemoryKind::Claim) {
        return Ok(());
    }
    let mut payload = serde_json::from_str::<MemoryPayload>(&memory.payload_json)
        .ok()
        .and_then(|p| match p {
            MemoryPayload::Claim(c) => Some(c),
            _ => None,
        })
        .unwrap_or_else(|| ClaimPayload {
            assessment: "unassessed".into(),
            ..Default::default()
        });
    payload.actors = take::<Vec<RefRow>>(&mut r, 1)?
        .into_iter()
        .map(|row| ActorRef {
            actor_id: row.actor_id,
            role: row.role,
            binding_state: parse_state(&row.binding_state),
        })
        .collect();
    edit(&mut payload);
    let payload = MemoryPayload::Claim(payload);
    if serde_json::to_string(&payload).map_err(json_err)? == memory.payload_json {
        return Ok(());
    }
    put_typed_in(
        tx,
        &TypedMemory {
            id: memory_id.to_string(),
            text: memory.text,
            kind: MemoryKind::Claim,
            payload: Some(payload),
            lifecycle: memory.lifecycle,
        },
    )
    .await
}

/// Keep the typed claim payload's actors equal to its refs.
pub async fn sync_claim_actor_payload_in(tx: &Tx, memory_id: &str) -> Result<(), TxError> {
    update_claim_payload_in(tx, memory_id, |_| {}).await
}

#[derive(SurrealValue)]
struct ClaimHead {
    memory_id: String,
    entity_id: String,
    classification: String,
    source_json: Option<String>,
}

#[derive(SurrealValue)]
struct Retained {
    snapshot: String,
    owner_kind: String,
    owner_id: String,
    entities_json: String,
    item_json: String,
}

/// Bind a new insight claim's subject (and the actors its retained
/// extractions reference) in the publishing transaction, and receipt it for
/// the backfill (`brain_backfill_receipt`, source `insight_claims`), as
/// `brain_backfill::bind_claim_subject`. `label` overrides the subject's
/// display label; `None` reads it from `recon_entity` (label, then
/// canonical), falling back to the entity id.
pub async fn bind_claim_subject_in(
    tx: &Tx,
    fingerprint: &str,
    label: Option<&str>,
    now: &str,
) -> Result<Vec<String>, TxError> {
    let Some((head, coverage, changed)) = claim_subject_in(tx, fingerprint, label, now).await?
    else {
        return Ok(Vec::new());
    };
    let revision = crate::brain_backfill::h(&json!([head.memory_id, head.entity_id]));
    super::entities::put_backfill_receipt_in(
        tx,
        "insight_claims",
        fingerprint,
        &revision,
        coverage,
        now,
    )
    .await?;
    Ok(changed)
}

/// `brain_backfill::claim_subject`: `None` when there is no claim or no
/// subject (no receipt), otherwise the coverage label and changed actors.
async fn claim_subject_in(
    tx: &Tx,
    fingerprint: &str,
    label: Option<&str>,
    now: &str,
) -> Result<Option<(ClaimHead, &'static str, Vec<String>)>, TxError> {
    let mut r = run(
        tx,
        tx.query(
            "SELECT memory_id, entity_id, classification,
               type::record('memory', memory_id).source_json AS source_json
             FROM ONLY type::record('insight_claim', $fp)",
        )
        .bind(("fp", fingerprint.to_string())),
    )
    .await?;
    let Some(claim) = take::<Option<ClaimHead>>(&mut r, 0)? else {
        return Ok(None);
    };
    if claim.entity_id.is_empty() {
        return Ok(None);
    }
    if claim.classification.to_lowercase() == "inference" {
        return Ok(Some((claim, "inference retained internally", Vec::new())));
    }
    let Some(source) = claim.source_json.clone() else {
        return Ok(Some((claim, "no memory", Vec::new())));
    };
    let stored = match label {
        Some(_) => None,
        None => super::entities::entity_label_in(tx, &claim.entity_id).await?,
    };
    let entity = label
        .or(stored.as_deref())
        .filter(|l| !l.trim().is_empty())
        .unwrap_or(&claim.entity_id)
        .to_string();
    let source: Value = serde_json::from_str(&source).unwrap_or(Value::Null);
    let app = source["app"].as_str().unwrap_or("");
    let conversation = source["conversation_id"].as_str().unwrap_or("");
    let scope = if conversation.is_empty() {
        format!("insight-memory:{}", claim.memory_id)
    } else {
        format!("{app}:{conversation}")
    };
    let (bound, mut changed) = bind_in(
        tx,
        &BindRequest {
            name: &entity,
            subtype: ActorSubtype::Unknown,
            scope: &scope,
            identifiers: &[],
            external: Some(("insight_claims.entity", fingerprint)),
            provenance: json!({"insight_claim": fingerprint}),
        },
        now,
    )
    .await?;
    if claim.entity_id != entity {
        // Repair only the machine-generated id spelling, keeping analyst names.
        let old = format!(
            "Actor: {} ({})",
            claim.entity_id,
            subtype_str(ActorSubtype::Unknown)
        );
        let mut r = run(
            tx,
            tx.query(
                "UPDATE $a SET display_name = $name, revision += 1, updated_at = $now
                   WHERE display_name = $old_name RETURN NONE;
                 SELECT VALUE payload_json FROM ONLY $m;",
            )
            .bind(("a", rid("brain_actor", &bound.actor_id)))
            .bind(("m", rid("memory", &bound.actor_id)))
            .bind(("name", entity.clone()))
            .bind(("old_name", claim.entity_id.clone()))
            .bind(("now", now.to_string())),
        )
        .await?;
        if let Some(Ok(MemoryPayload::Actor(mut p))) =
            take::<Option<String>>(&mut r, 1)?.map(|raw| serde_json::from_str(&raw))
        {
            if p.display_name == claim.entity_id {
                p.display_name = entity.clone();
                let text = format!("Actor: {} ({})", entity, subtype_str(p.subtype));
                run(
                    tx,
                    tx.query(
                        "UPDATE $m SET payload_json = $p RETURN NONE;
                         UPDATE $m SET text = $text WHERE text = $old RETURN NONE;",
                    )
                    .bind(("m", rid("memory", &bound.actor_id)))
                    .bind((
                        "p",
                        serde_json::to_string(&MemoryPayload::Actor(p)).map_err(json_err)?,
                    ))
                    .bind(("text", text))
                    .bind(("old", old)),
                )
                .await?;
            }
        }
    }
    let provenance = json!({"fingerprint": fingerprint});
    add_memory_actor_ref_in(
        tx,
        &claim.memory_id,
        &bound.actor_id,
        "subject",
        bound.state,
        &provenance,
    )
    .await?;
    // Recover referenced actors from retained extractions; a missing packet
    // is skipped rather than guessed from other article actors.
    let mut r = run(
        tx,
        tx.query(
            "SELECT snapshot_id AS snapshot,
               type::record('insight_snapshot', snapshot_id).owner_kind AS owner_kind,
               type::record('insight_snapshot', snapshot_id).owner_id AS owner_id,
               type::record('insight_snapshot', snapshot_id).entities_json AS entities_json,
               item_json
             FROM insight_derivation WHERE memory_id = $m AND insight_kind = 'claim'
               AND status = 'assessed' AND record::exists(type::record('insight_snapshot', snapshot_id))",
        )
        .bind(("m", claim.memory_id.clone())),
    )
    .await?;
    for row in take::<Vec<Retained>>(&mut r, 0)? {
        let (Ok(entities), Ok(item)) = (
            serde_json::from_str::<Vec<crate::nlp::EntityCandidate>>(&row.entities_json),
            serde_json::from_str::<crate::investigation::insights::SourceClaim>(&row.item_json),
        ) else {
            continue;
        };
        for entity in entities {
            let article = row.owner_kind == "article";
            if !referenced(&entity, &item.subject_ref, &item.evidence_refs, |_| article) {
                continue;
            }
            let Some(name) = entity
                .original_forms
                .first()
                .filter(|n| !n.trim().is_empty())
            else {
                continue;
            };
            let scope = format!("{}:{}", row.owner_kind, row.owner_id);
            let external = format!("snapshot:{}", row.snapshot);
            let provenance = json!({"snapshot": row.snapshot});
            let (b, c) = bind_in(
                tx,
                &BindRequest {
                    name,
                    subtype: parse_subtype(&entity.type_hint),
                    scope: &scope,
                    identifiers: &[],
                    external: Some((&external, &entity.id)),
                    provenance: provenance.clone(),
                },
                now,
            )
            .await?;
            changed.extend(c);
            let role = if entity.id == item.subject_ref {
                "subject"
            } else {
                "mentioned"
            };
            add_memory_actor_ref_in(
                tx,
                &claim.memory_id,
                &b.actor_id,
                role,
                b.state,
                &provenance,
            )
            .await?;
        }
    }
    Ok(Some((claim, "subject bound", changed)))
}

/// Whether a claim references `entity`: its explicit subject, or a mention
/// inside a cited range of the entity's own source (or, for an article
/// owner, of the article body when `article_entity` holds).
pub fn referenced(
    entity: &crate::nlp::EntityCandidate,
    subject: &str,
    refs: &[crate::investigation::insights::EvidenceRef],
    article_entity: impl Fn(&str) -> bool,
) -> bool {
    use crate::investigation::insights::EvidenceLocation;
    entity.id == subject
        || entity.mention_refs.iter().any(|span| {
            refs.iter().any(|r| match &r.location {
                EvidenceLocation::Range(range) => {
                    span.start_byte >= range.start_byte
                        && span.end_byte <= range.end_byte
                        && (entity.id.starts_with(&r.source_id)
                            || (r.source_id.starts_with("article:") && article_entity(&entity.id)))
                }
                _ => false,
            })
        })
}
