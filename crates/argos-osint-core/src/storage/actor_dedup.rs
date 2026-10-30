//! The actor-dedup pool on SurrealDB (`brain_actor_dedup` on the
//! repository): local spelling repair, candidate pairs with their sampled
//! source context, durable pair leases on `brain_actor_dedup_receipt`, and
//! the assessed merge. The prompt, decision adapter, pair generation and
//! input hash are shared with the legacy store, so a pair hashes the same on both.
//!
//! the legacy store's repair also deletes pre-upgrade debris (mentions and index
//! receipts of deleted articles); a SurrealDB store starts empty and its
//! delete paths remove those rows, so there is none to repair.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use serde_json::{json, Value};
use surrealdb::types::SurrealValue;

use super::actor_bind::{add_alias_in, resolve_in};
use super::jobs::{rid, run, take};
use super::lifecycle::{family, reconcile_lifecycle_in, redirects};
use super::memory::{put_typed_in, set_lifecycle_in, write_then_index, MemoryWrite, TypedMemory};
use super::repo::Repository;
use super::tx::{run_tx, RetryPolicy, Tx, TxError};
use crate::brain_actor_dedup::{
    candidate_pairs, candidate_snapshot, Candidate, IdentityDecision, BATCH, PAIRS, VERSION,
};
use crate::brain_actors::{normalize_alias, normalize_name};
use crate::memory_metadata::{IdentityState, MemoryKind, MemoryPayload};
use crate::recon::model_exec;

const LEASE_MINUTES: i64 = 10;
const IDENTIFIER: [&str; 3] = ["account", "domain", "ip"];

fn abort(e: impl std::fmt::Display) -> TxError {
    TxError::Abort(e.to_string())
}

#[derive(SurrealValue)]
struct ActorRow {
    id: String,
    subtype: String,
    display_name: String,
    scope_key: String,
    revision: i64,
}

async fn actor_in(tx: &Tx, id: &str) -> Result<Option<ActorRow>, TxError> {
    let mut r = run(
        tx,
        tx.query(
            "SELECT record::id(id) AS id, subtype, display_name, scope_key, revision \
             FROM ONLY $a",
        )
        .bind(("a", rid("brain_actor", id))),
    )
    .await?;
    take(&mut r, 0)
}

#[derive(SurrealValue)]
struct MemoryRow {
    text: String,
    lifecycle: String,
    payload_json: String,
}

async fn memory_in(tx: &Tx, id: &str) -> Result<Option<MemoryRow>, TxError> {
    let mut r = run(
        tx,
        tx.query("SELECT text, lifecycle, payload_json FROM ONLY $m")
            .bind(("m", rid("memory", id))),
    )
    .await?;
    take(&mut r, 0)
}

/// Rewrite the actor's memory payload through `edit` (graph re-projected).
async fn edit_actor_memory_in(
    tx: &Tx,
    id: &str,
    text: Option<String>,
    edit: impl FnOnce(&mut crate::memory_metadata::ActorPayload),
) -> Result<bool, TxError> {
    let Some(m) = memory_in(tx, id).await? else {
        return Ok(false);
    };
    let Ok(MemoryPayload::Actor(mut p)) = serde_json::from_str::<MemoryPayload>(&m.payload_json)
    else {
        return Ok(false);
    };
    edit(&mut p);
    put_typed_in(
        tx,
        &TypedMemory {
            id: id.to_string(),
            text: text.unwrap_or(m.text),
            kind: MemoryKind::Actor,
            payload: Some(MemoryPayload::Actor(p)),
            lifecycle: m.lifecycle,
        },
    )
    .await?;
    Ok(true)
}

/// `brain_actors::separated_pairs`: explicit splits, resolved and sorted.
pub async fn separated_pairs_in(tx: &Tx) -> Result<BTreeSet<(String, String)>, TxError> {
    let mut r = run(
        tx,
        tx.query(
            "SELECT VALUE record::id(id) FROM brain_actor_dedup_receipt \
             WHERE decision = 'separate'",
        ),
    )
    .await?;
    let keys: Vec<String> = take(&mut r, 0)?;
    let mut out = BTreeSet::new();
    for key in keys {
        if let Some((l, r)) = key.split_once(':') {
            let mut ids = [resolve_in(tx, l).await?, resolve_in(tx, r).await?];
            ids.sort();
            let [a, b] = ids;
            out.insert((a, b));
        }
    }
    Ok(out)
}

/// `brain_actors::merge`: `from` redirects to `to`, the survivor becomes
/// resolved (revision bumped, payload updated) and active, `from` is
/// redirected. Returns the memory ids changed.
pub async fn merge_in(
    tx: &Tx,
    from: &str,
    to: &str,
    receipt: &Value,
    now: &str,
) -> Result<Vec<String>, TxError> {
    let from = resolve_in(tx, from).await?;
    let to = resolve_in(tx, to).await?;
    if from == to {
        return Err(abort("cannot merge an actor into itself"));
    }
    if actor_in(tx, &from).await?.is_none() || actor_in(tx, &to).await?.is_none() {
        return Err(abort("unknown actor"));
    }
    super::lifecycle::add_redirect_in(tx, &from, &to, now).await?;
    run(
        tx,
        tx.query(
            "UPDATE $r SET receipt_json = $receipt RETURN NONE;
             UPDATE $a SET identity_state = 'resolved', revision += 1, updated_at = $now
               RETURN NONE;",
        )
        .bind(("r", rid("brain_actor_redirect", &from)))
        .bind(("receipt", receipt.to_string()))
        .bind(("a", rid("brain_actor", &to)))
        .bind(("now", now.to_string())),
    )
    .await?;
    let mut changed = vec![to.clone(), from.clone()];
    edit_actor_memory_in(tx, &to, None, |p| {
        p.identity_state = IdentityState::Resolved
    })
    .await?;
    changed.extend(set_lifecycle_in(tx, std::slice::from_ref(&to), "active").await?);
    changed.extend(set_lifecycle_in(tx, std::slice::from_ref(&from), "redirected").await?);
    Ok(changed)
}

/// The sampled identity context of one actor family (legacy store `context`).
async fn context_in(tx: &Tx, id: &str) -> Result<Value, TxError> {
    let root = resolve_in(tx, id).await?;
    let Some(a) = actor_in(tx, &root).await? else {
        return Ok(Value::Null);
    };
    let lifecycle = memory_in(tx, &root)
        .await?
        .map(|m| m.lifecycle)
        .unwrap_or_default();
    let map = redirects(tx).await?;
    let fam = family(&map, id);
    #[derive(SurrealValue)]
    struct Mention {
        origin_kind: String,
        origin_scope: String,
        origin_id: String,
        origin_revision: String,
    }
    let mut r = run(
        tx,
        tx.query(
            "SELECT origin_kind, origin_scope, origin_id, origin_revision, created_at \
             FROM brain_actor_mention WHERE actor_id INSIDE $fam \
             ORDER BY created_at DESC, id DESC;
             SELECT text, created_at FROM memory WHERE lifecycle = 'active' \
               AND memory_kind = 'claim' AND record::id(id) INSIDE \
               (SELECT VALUE memory_id FROM brain_memory_actor_ref WHERE actor_id INSIDE $fam) \
             ORDER BY created_at DESC, id DESC LIMIT 3;
             SELECT alias, alias_type, normalized FROM brain_actor_alias \
             WHERE actor_id INSIDE $fam ORDER BY alias_type, normalized;",
        )
        .bind(("fam", fam)),
    )
    .await?;
    let mentions: Vec<Mention> = take(&mut r, 0)?;
    #[derive(SurrealValue)]
    struct Text {
        text: String,
    }
    let claims: Vec<Text> = take(&mut r, 1)?;
    #[derive(SurrealValue)]
    struct Alias {
        alias: String,
        alias_type: String,
    }
    let aliases: Vec<Alias> = take(&mut r, 2)?;
    let mut passages = BTreeSet::new();
    for m in mentions {
        if m.origin_kind == "article" {
            if let Some(article) =
                super::atlas::article_in(tx, &m.origin_scope, &m.origin_id).await?
            {
                if m.origin_revision.is_empty()
                    || m.origin_revision == crate::atlas_metadata::input_hash(&article)
                {
                    passages.insert(format!("{} — {}", article.title, article.description));
                }
            }
        } else if m.origin_kind == "thread" {
            if let Some(t) = super::recon::get_thread_on(tx.as_ref(), &m.origin_id)
                .await
                .map_err(abort)?
            {
                passages.insert(t.title);
            }
        }
        if passages.len() >= 3 {
            break;
        }
    }
    for c in claims {
        if passages.len() >= 4 {
            break;
        }
        passages.insert(c.text);
    }
    let mut seen = BTreeSet::new();
    let aliases: Vec<Value> = aliases
        .into_iter()
        .filter(|a| seen.insert((a.alias.clone(), a.alias_type.clone())))
        .take(20)
        .map(|a| json!({"value": a.alias, "type": a.alias_type}))
        .collect();
    Ok(
        json!({"name":a.display_name,"type":a.subtype,"revision":a.revision,
        "lifecycle":lifecycle,"scope":a.scope_key,"aliases":aliases,
        "passages":passages.into_iter().take(4)
            .map(|s|s.chars().take(1000).collect::<String>()).collect::<Vec<_>>()}),
    )
}

async fn candidate_in(tx: &Tx, left: &str, right: &str) -> Result<Candidate, TxError> {
    let context =
        json!({"left": context_in(tx, left).await?, "right": context_in(tx, right).await?});
    Ok(candidate_snapshot(left, right, context))
}

/// `brain_actor_dedup::candidates`: up to `limit` uncached pairs with
/// retained context on both sides.
pub async fn candidates(repo: &Repository, limit: usize) -> Result<Vec<Candidate>> {
    let now = chrono::Utc::now().to_rfc3339();
    Ok(run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let now = now.clone();
        async move {
            #[derive(SurrealValue)]
            struct Root {
                id: String,
                subtype: String,
                lifecycle: Option<String>,
            }
            let mut r = run(
                &tx,
                tx.query(
                    "SELECT record::id(id) AS id, subtype, \
                       (SELECT VALUE lifecycle FROM ONLY type::record('memory', record::id($parent.id))) \
                         AS lifecycle FROM brain_actor ORDER BY id",
                ),
            )
            .await?;
            let roots: Vec<Root> = take(&mut r, 0)?;
            let map = redirects(&tx).await?;
            let roots = roots
                .into_iter()
                .filter(|r| !map.contains_key(&r.id) && r.lifecycle.as_deref() == Some("active"));
            let mut rows = Vec::new();
            for root in roots {
                let fam = family(&map, &root.id);
                let mut r = run(
                    &tx,
                    tx.query(
                        "SELECT VALUE alias FROM brain_actor_alias \
                         WHERE actor_id INSIDE $fam AND alias_type = 'name'",
                    )
                    .bind(("fam", fam)),
                )
                .await?;
                let names: BTreeSet<String> = take::<Vec<String>>(&mut r, 0)?.into_iter().collect();
                rows.extend(
                    names
                        .into_iter()
                        .map(|n| (root.id.clone(), n, root.subtype.clone())),
                );
            }
            let separated = separated_pairs_in(&tx).await?;
            let mut contexts: BTreeMap<String, Value> = BTreeMap::new();
            let mut out = Vec::new();
            for (left, right) in candidate_pairs(rows) {
                if separated.contains(&(left.clone(), right.clone())) {
                    continue;
                }
                if out.len() >= limit.min(PAIRS) {
                    break;
                }
                for id in [&left, &right] {
                    if !contexts.contains_key(id) {
                        contexts.insert(id.clone(), context_in(&tx, id).await?);
                    }
                }
                let c = candidate_snapshot(
                    &left,
                    &right,
                    json!({"left": contexts[&left], "right": contexts[&right]}),
                );
                let empty = |side: &str| {
                    c.context[side]["passages"]
                        .as_array()
                        .is_none_or(|p| p.is_empty())
                };
                if empty("left") || empty("right") {
                    continue;
                }
                let mut r = run(
                    &tx,
                    tx.query(
                        "RETURN (SELECT VALUE decision = 'separate' OR (input_hash = $h AND \
                           (decision INSIDE ['same', 'different', 'uncertain'] OR lease_until > $now)) \
                         FROM ONLY $r) ?? false",
                    )
                    .bind(("r", rid("brain_actor_dedup_receipt", &c.key())))
                    .bind(("h", c.input_hash.clone()))
                    .bind(("now", now.clone())),
                )
                .await?;
                if take::<Option<bool>>(&mut r, 0)? != Some(true) {
                    out.push(c);
                }
            }
            Ok(out)
        }
    })
    .await?
    .value)
}

/// Take the pair lease unless it is held, explicitly separated, or already
/// decided for this input. Returns whether this worker now holds it.
pub async fn acquire(repo: &Repository, c: &Candidate, owner: &str) -> Result<bool> {
    let now = chrono::Utc::now();
    let until = (now + chrono::Duration::minutes(LEASE_MINUTES)).to_rfc3339();
    let now = now.to_rfc3339();
    let (key, hash, owner) = (c.key(), c.input_hash.clone(), owner.to_string());
    Ok(run_tx(repo.db(), RetryPolicy::default(), |tx| {
        let (key, hash, owner, now, until) = (
            key.clone(),
            hash.clone(),
            owner.clone(),
            now.clone(),
            until.clone(),
        );
        async move {
            let mut r = run(
                &tx,
                tx.query(
                    "LET $cur = (SELECT * FROM ONLY $r);
                     IF $cur = NONE OR ($cur.lease_until <= $now AND $cur.decision != 'separate'
                         AND ($cur.input_hash != $h
                              OR $cur.decision NOTINSIDE ['same', 'different', 'uncertain'])) {
                       UPSERT $r SET input_hash = $h, decision = 'pending', lease_owner = $o,
                         lease_until = $until, updated_at = $now RETURN NONE;
                       RETURN true;
                     } ELSE { RETURN false; };",
                )
                .bind(("r", rid("brain_actor_dedup_receipt", &key)))
                .bind(("h", hash))
                .bind(("o", owner))
                .bind(("until", until))
                .bind(("now", now)),
            )
            .await?;
            Ok(take::<Option<bool>>(&mut r, 1)? == Some(true))
        }
    })
    .await?
    .value)
}

/// Extend a held lease; false when it was lost or expired.
pub async fn renew(repo: &Repository, c: &Candidate, owner: &str) -> Result<bool> {
    let now = chrono::Utc::now();
    let mut r = repo
        .db()
        .query(
            "UPDATE $r SET lease_until = $until WHERE lease_owner = $o AND lease_until > $now \
             RETURN VALUE true",
        )
        .bind(("r", rid("brain_actor_dedup_receipt", &c.key())))
        .bind(("o", owner.to_string()))
        .bind((
            "until",
            (now + chrono::Duration::minutes(LEASE_MINUTES)).to_rfc3339(),
        ))
        .bind(("now", now.to_rfc3339()))
        .await?
        .check()?;
    let rows: Vec<bool> = r.take(0usize)?;
    Ok(!rows.is_empty())
}

/// Record a failed assessment on a lease this worker holds.
pub async fn fail(repo: &Repository, c: &Candidate, owner: &str, error: &str) -> Result<()> {
    repo.db()
        .query(
            "UPDATE $r SET decision = 'failed', receipt_json = $j, updated_at = $now \
             WHERE lease_owner = $o RETURN NONE",
        )
        .bind(("r", rid("brain_actor_dedup_receipt", &c.key())))
        .bind(("o", owner.to_string()))
        .bind(("j", json!({ "error": error }).to_string()))
        .bind(("now", chrono::Utc::now().to_rfc3339()))
        .await?
        .check()?;
    Ok(())
}

/// `brain_actor_dedup::apply`: only while both actors are canonical, the
/// context still hashes the same and this worker holds the unexpired lease.
/// A `same` decision merges the shorter name into the longer one.
pub async fn apply(
    repo: &Repository,
    c: &Candidate,
    decision: &str,
    owner: &str,
    receipt: &Value,
) -> Result<bool> {
    anyhow::ensure!(
        matches!(decision, "same" | "different" | "uncertain"),
        "invalid identity decision"
    );
    let applied = Arc::new(Mutex::new(false));
    let slot = applied.clone();
    let (c, decision, owner, receipt) = (
        c.clone(),
        decision.to_string(),
        owner.to_string(),
        receipt.clone(),
    );
    let write: MemoryWrite = Arc::new(move |tx: Tx| {
        let (c, decision, owner, receipt, slot) = (
            c.clone(),
            decision.clone(),
            owner.clone(),
            receipt.clone(),
            slot.clone(),
        );
        Box::pin(async move {
            *slot.lock().expect("apply slot") = false;
            let now = chrono::Utc::now().to_rfc3339();
            if separated_pairs_in(&tx)
                .await?
                .contains(&(c.left.clone(), c.right.clone()))
                || resolve_in(&tx, &c.left).await? != c.left
                || resolve_in(&tx, &c.right).await? != c.right
                || candidate_in(&tx, &c.left, &c.right).await?.input_hash != c.input_hash
            {
                return Ok(Vec::new());
            }
            let mut r = run(
                &tx,
                tx.query(
                    "UPDATE $r SET decision = $d, receipt_json = $j, lease_owner = '',
                       lease_until = '', updated_at = $now
                     WHERE input_hash = $h AND lease_owner = $o AND lease_until > $now
                     RETURN VALUE true",
                )
                .bind(("r", rid("brain_actor_dedup_receipt", &c.key())))
                .bind(("d", decision.clone()))
                .bind(("j", receipt.to_string()))
                .bind(("h", c.input_hash.clone()))
                .bind(("o", owner))
                .bind(("now", now.clone())),
            )
            .await?;
            if take::<Vec<bool>>(&mut r, 0)?.is_empty() {
                return Ok(Vec::new());
            }
            *slot.lock().expect("apply slot") = true;
            if decision != "same" {
                return Ok(Vec::new());
            }
            let name = |side: &str| {
                c.context[side]["name"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .count()
            };
            let (from, to) = if name("right") > name("left") {
                (&c.left, &c.right)
            } else {
                (&c.right, &c.left)
            };
            merge_in(&tx, from, to, &receipt, &now).await
        })
    });
    write_then_index(repo.publisher(), write).await?;
    let ok = *applied.lock().expect("apply slot");
    Ok(ok)
}

/// `brain_actor_dedup::repair_local` on the repository: normalize up to
/// `limit` actors whose receipt is behind their revision, add the clean
/// spelling as an alias, rename, merge an equal same-scope spelling into the
/// lowest id, then reconcile lifecycle. Returns actors examined.
pub async fn repair_local(repo: &Repository, limit: usize) -> Result<usize> {
    let nlp_enabled =
        cfg!(feature = "nlp") && crate::nlp::configured_mode() != crate::nlp::Mode::Off;
    let version = if nlp_enabled {
        "actor-dedup-v2-nlp"
    } else {
        "actor-dedup-v2-lexical"
    };
    let mut r = repo
        .db()
        .query(
            "SELECT record::id(id) AS id, subtype, display_name, scope_key, revision \
             FROM brain_actor ORDER BY id;
             SELECT VALUE record::id(id) FROM brain_actor_redirect;
             SELECT unit_id, revision FROM brain_backfill_receipt \
             WHERE source = 'actor_normalization';",
        )
        .await?
        .check()?;
    let all: Vec<ActorRow> = r.take(0usize)?;
    let redirected: BTreeSet<String> = r.take::<Vec<String>>(1usize)?.into_iter().collect();
    #[derive(SurrealValue)]
    struct Receipt {
        unit_id: String,
        revision: String,
    }
    let receipts: HashMap<String, String> = r
        .take::<Vec<Receipt>>(2usize)?
        .into_iter()
        .map(|r| (r.unit_id, r.revision))
        .collect();
    let rows: Vec<ActorRow> = all
        .into_iter()
        .filter(|a| {
            !redirected.contains(&a.id)
                && receipts.get(&a.id) != Some(&format!("{version}:{}", a.revision))
        })
        .take(limit.min(BATCH))
        .collect();
    if rows.is_empty() {
        return Ok(0);
    }
    let names: Vec<String> = rows
        .iter()
        .map(|a| {
            if a.display_name.len() > 1024 || IDENTIFIER.contains(&a.subtype.as_str()) {
                String::new()
            } else {
                normalize_name(&a.display_name)
            }
        })
        .collect();
    let parsed = if nlp_enabled {
        tokio::task::spawn_blocking(move || crate::nlp::Service::shared().actor_names(names))
            .await
            .unwrap_or(None)
    } else {
        Some(names)
    };
    let n = rows.len();
    let rows = Arc::new(rows);
    let parsed = Arc::new(parsed);
    let version = version.to_string();
    let write: MemoryWrite = Arc::new(move |tx: Tx| {
        let (rows, parsed, version) = (rows.clone(), parsed.clone(), version.clone());
        Box::pin(async move {
            let now = chrono::Utc::now().to_rfc3339();
            let mut changed = Vec::new();
            let separated = separated_pairs_in(&tx).await?;
            for (pos, a) in rows.iter().enumerate() {
                let Some(cur) = actor_in(&tx, &a.id).await? else {
                    continue;
                };
                if resolve_in(&tx, &a.id).await? != a.id
                    || cur.revision != a.revision
                    || cur.display_name != a.display_name
                {
                    continue;
                }
                let identifier = IDENTIFIER.contains(&a.subtype.as_str());
                let clean = if identifier {
                    a.display_name.clone()
                } else {
                    parsed
                        .as_ref()
                        .as_ref()
                        .and_then(|n| n.get(pos))
                        .cloned()
                        .filter(|n| !n.trim().is_empty())
                        .unwrap_or_else(|| normalize_name(&a.display_name))
                };
                add_alias_in(&tx, &a.id, &clean, &json!({})).await?;
                if clean != a.display_name {
                    run(
                        &tx,
                        tx.query(
                            "UPDATE $a SET display_name = $n, revision += 1, updated_at = $now \
                             RETURN NONE",
                        )
                        .bind(("a", rid("brain_actor", &a.id)))
                        .bind(("n", clean.clone()))
                        .bind(("now", now.clone())),
                    )
                    .await?;
                    let old = a.display_name.clone();
                    let text = format!("Actor: {clean} ({})", a.subtype);
                    let renamed = clean.clone();
                    if edit_actor_memory_in(&tx, &a.id, Some(text), move |p| {
                        p.display_name = renamed;
                        if !p.aliases.contains(&old) {
                            p.aliases.push(old);
                        }
                    })
                    .await?
                    {
                        changed.push(a.id.clone());
                    }
                }
                // Same-source spelling normalization is the only automatic merge.
                if !a.scope_key.is_empty() && !identifier {
                    let mut r = run(
                        &tx,
                        tx.query(
                            "SELECT VALUE actor_id FROM brain_actor_alias \
                             WHERE normalized = $k AND alias_type = 'name' AND actor_id < $id",
                        )
                        .bind(("k", normalize_alias(&clean)))
                        .bind(("id", a.id.clone())),
                    )
                    .await?;
                    let ids: BTreeSet<String> =
                        take::<Vec<String>>(&mut r, 0)?.into_iter().collect();
                    let map: HashMap<String, String> = redirects(&tx).await?;
                    let mut peer = None;
                    for id in ids {
                        if map.contains_key(&id) {
                            continue;
                        }
                        let Some(p) = actor_in(&tx, &id).await? else {
                            continue;
                        };
                        let pair = (id.clone(), a.id.clone());
                        if p.scope_key == a.scope_key
                            && p.subtype == a.subtype
                            && !separated.contains(&pair)
                        {
                            peer = Some(id);
                            break;
                        }
                    }
                    if let Some(peer) = peer {
                        changed.extend(
                            merge_in(
                                &tx,
                                &a.id,
                                &peer,
                                &json!({"contract":VERSION,"basis":"same-source normalized spelling"}),
                                &now,
                            )
                            .await?,
                        );
                    }
                }
                if parsed.is_some() || identifier {
                    let mut r = run(
                        &tx,
                        tx.query("SELECT VALUE revision FROM ONLY $a")
                            .bind(("a", rid("brain_actor", &a.id))),
                    )
                    .await?;
                    let rev: Option<i64> = take(&mut r, 0)?;
                    if let Some(rev) = rev {
                        run(
                            &tx,
                            tx.query(
                                "UPSERT $r SET source = 'actor_normalization', \
                                   unit_id = $id, revision = $rev, coverage = 'normalized', \
                                   completed_at = $now RETURN NONE",
                            )
                            .bind((
                                "r",
                                rid(
                                    "brain_backfill_receipt",
                                    &super::codec::encode(&["actor_normalization", &a.id]),
                                ),
                            ))
                            .bind(("id", a.id.clone()))
                            .bind(("rev", format!("{version}:{rev}")))
                            .bind(("now", now.clone())),
                        )
                        .await?;
                    }
                }
            }
            changed.extend(reconcile_lifecycle_in(&tx).await?);
            changed.sort();
            changed.dedup();
            Ok(changed)
        })
    });
    write_then_index(repo.publisher(), write).await?;
    Ok(n)
}

/// One bounded worker tick (`brain_actor_dedup::drain`): repair, then up to
/// `PAIRS` leased identity decisions. No transaction waits on a model.
pub async fn drain(
    repo: &Repository,
    owner: &str,
    cancel: &Arc<AtomicBool>,
    auth: &crate::secrets::AuthFile,
    settings: &crate::provider::SettingsFile,
) -> Result<usize> {
    repair_local(repo, BATCH).await?;
    if model_exec::select_role_secret(
        auth,
        settings,
        "entity_resolver",
        model_exec::OperationFormat::BoundedDecision,
    )
    .is_none()
    {
        return Ok(0);
    }
    let mut done = 0;
    for c in candidates(repo, PAIRS).await? {
        if cancel.load(Ordering::Acquire) {
            break;
        }
        let lease_owner = format!("{owner}:{}", crate::job_registry::new_job_id("actor-pair"));
        if !acquire(repo, &c, &lease_owner).await? {
            continue;
        }
        let op = format!("actor-dedup:{}:{}:{lease_owner}", c.key(), c.input_hash);
        let scope = model_exec::OperationScope::new(&op, "entity_resolver")
            .with_format(model_exec::ResponseFormat::BoundedDecision)
            .with_input_revision(&c.input_hash)
            .with_budget(crate::request_policy::ACTOR_IDENTITY_BUDGET);
        let operation_cancel = Arc::new(AtomicBool::new(false));
        let adapter = IdentityDecision(&c);
        let evaluation = model_exec::execute_decisions_or_chat(
            auth,
            settings,
            &scope,
            &adapter,
            &operation_cancel,
            None,
            None,
        );
        tokio::pin!(evaluation);
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
        let mut ticks = 0u32;
        let result = loop {
            tokio::select! {
                result = &mut evaluation => break result,
                _ = tick.tick() => {
                    if cancel.load(Ordering::Acquire) {
                        operation_cancel.store(true, Ordering::Release);
                    }
                    ticks += 1;
                    if ticks.is_multiple_of(10) && !matches!(renew(repo, &c, &lease_owner).await, Ok(true)) {
                        operation_cancel.store(true, Ordering::Release);
                    }
                }
            }
        };
        match result {
            Ok((decision, report)) => {
                let receipt = json!({"contract":VERSION,"input_hash":c.input_hash,
                    "decision":decision,"context":c.context,"operation":op,
                    "model":report.effective_model,"route":report.effective_route,
                    "requests":report.requests});
                if !cancel.load(Ordering::Acquire)
                    && apply(repo, &c, &decision, &lease_owner, &receipt).await?
                {
                    done += 1;
                }
            }
            Err(e) => fail(repo, &c, &lease_owner, &e.to_string()).await?,
        }
    }
    Ok(done)
}
