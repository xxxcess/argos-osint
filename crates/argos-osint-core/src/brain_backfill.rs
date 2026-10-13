//! One resumable local backfill into the Brain actor/relationship registry
//! (spec §4 "Identity, binding and migration", §5 "Backfill and lifecycle").
//!
//! Sources, in order: legacy `recon_entities` and `investigation_entities`
//! (scoped external refs), insight claim subjects (memory actor refs),
//! `insight_derivation_links` (canonical Link memories),
//! `intel_inference_article_leads` (Related memories) and every retained
//! Atlas article occurrence (Actor + mention receipt + observed
//! `mentioned_in` Link + bounded tentative Related records).
//!
//! Each unit carries a content revision; a receipt with the same revision
//! is skipped, a changed one reindexes only that unit. Units commit in
//! bounded transactions (initially 100) with per-unit savepoints, so a crash
//! resumes at the first unreceipted unit and a failing unit never blocks or
//! falsely completes the rest. No model, network, body or embedding request.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::brain_actors::{
    self as actors, BindRequest, IDENTITY_INDEX_VERSION, RELATED_PER_ARTICLE,
};
use crate::memory_metadata::{EndpointRef, LinkPayload, MemoryKind, RelatedPayload};
use crate::store::Store;

pub const BATCH: usize = 100;

pub const SOURCES: &[&str] = &[
    "recon_entities",
    "investigation_entities",
    "insight_claims",
    "insight_derivation_links",
    "intel_inference_article_leads",
    "atlas_articles",
];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SourceCoverage {
    pub source: String,
    pub done: usize,
    pub total: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct BackfillReport {
    pub processed: usize,
    pub failed: usize,
    /// Failing units (retried on the next call; never falsely complete).
    pub errors: Vec<String>,
    pub coverage: Vec<SourceCoverage>,
    pub complete: bool,
}

fn has_table(conn: &Connection, t: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT COUNT(*)>0 FROM sqlite_master WHERE type='table' AND name=?1",
        [t],
        |r| r.get(0),
    )?)
}

fn h(v: &Value) -> String {
    crate::nlp::cache::hash(&format!("{IDENTITY_INDEX_VERSION}|{v}"))
}

/// `(unit_id, revision)` for every unit of a source.
fn units(conn: &Connection, source: &str) -> Result<Vec<(String, String)>> {
    let table = if source == "insight_claims" {
        "insight_claims"
    } else {
        source
    };
    if !has_table(conn, table)? {
        return Ok(vec![]);
    }
    let sql = match source {
        "recon_entities" => "SELECT id, json_array(kind,namespace,canonical,label) FROM recon_entities ORDER BY id",
        "investigation_entities" => "SELECT id, json_array(thread_id,canonical_name,entity_type,identifiers_json) FROM investigation_entities ORDER BY id",
        "insight_claims" => "SELECT fingerprint, json_array(memory_id,entity_id) FROM insight_claims WHERE entity_id<>'' ORDER BY fingerprint",
        "insight_derivation_links" => "SELECT id, json_array(revision,status) FROM insight_derivation_links ORDER BY id",
        "intel_inference_article_leads" => "SELECT id, json_array(candidate_metadata_revision,interpretation,verification_needed) FROM intel_inference_article_leads ORDER BY id",
        "atlas_articles" => "SELECT a.run_id||'/'||a.article_id,
               json_array(a.title,a.description,a.category,
                 (SELECT metadata_json FROM atlas_article_metadata m WHERE m.run_id=a.run_id AND m.article_id=a.article_id))
             FROM atlas_articles a ORDER BY a.run_id, a.article_id",
        _ => return Ok(vec![]),
    };
    if source == "atlas_articles" && !has_table(conn, "atlas_article_metadata")? {
        return Ok(vec![]);
    }
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .map(|(id, raw)| {
            let rev = h(&serde_json::from_str::<Value>(&raw).unwrap_or(Value::String(raw)));
            (id, rev)
        })
        .collect())
}

fn receipts(conn: &Connection, source: &str) -> Result<HashMap<String, String>> {
    let mut stmt =
        conn.prepare("SELECT unit_id,revision FROM brain_backfill_receipts WHERE source=?1")?;
    let rows = stmt
        .query_map([source], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<HashMap<_, _>>>()?;
    Ok(rows)
}

fn identifiers_of(raw: &str) -> Vec<String> {
    let v: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    let mut out = Vec::new();
    if let Some(items) = v.as_array() {
        for i in items {
            if let Some(s) = i.as_str() {
                out.push(s.to_string());
            } else if let Some(s) = i.get("value").and_then(Value::as_str) {
                out.push(s.to_string());
            }
        }
    }
    out
}

/// Bind one claim's subject through the Brain actor service: scoped by the
/// claim memory's own source (app + conversation), referenced as `subject`.
fn claim_subject(conn: &Connection, fingerprint: &str, now: &str) -> Result<String> {
    let row: Option<(String, String, String)> = conn
        .query_row(
            "SELECT c.memory_id, c.entity_id, m.source_json FROM insight_claims c JOIN memories m ON m.id=c.memory_id WHERE c.fingerprint=?1",
            [fingerprint],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((memory_id, entity, source)) = row else {
        return Ok("no memory".into());
    };
    if entity.trim().is_empty() {
        return Ok("no subject".into());
    }
    let source: Value = serde_json::from_str(&source).unwrap_or(Value::Null);
    let app = source.get("app").and_then(Value::as_str).unwrap_or("");
    let conversation = source
        .get("conversation_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let scope = if conversation.is_empty() {
        format!("insight-memory:{memory_id}")
    } else {
        format!("{app}:{conversation}")
    };
    let (binding, _) = actors::bind(
        conn,
        &BindRequest {
            name: &entity,
            subtype: crate::memory_metadata::ActorSubtype::Unknown,
            scope: &scope,
            identifiers: &[],
            external: Some(("insight_claims.entity", fingerprint)),
            provenance: json!({"insight_claim": fingerprint}),
        },
        now,
    )?;
    actors::add_memory_actor_ref(
        conn,
        &memory_id,
        &binding.actor_id,
        "subject",
        binding.state,
        &json!({"fingerprint": fingerprint}),
    )?;
    Ok("subject bound".into())
}

/// Live publication hook: every new insight claim traces its subject through
/// Brain in the publishing transaction (and receipts it for the backfill).
pub fn bind_claim_subject(conn: &Connection, fingerprint: &str) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let rev: Option<(String, String)> = conn
        .query_row(
            "SELECT memory_id, entity_id FROM insight_claims WHERE fingerprint=?1",
            [fingerprint],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((memory_id, entity)) = rev else {
        return Ok(());
    };
    if entity.is_empty() {
        return Ok(());
    }
    let coverage = claim_subject(conn, fingerprint, &now)?;
    conn.execute(
        "INSERT INTO brain_backfill_receipts(source,unit_id,revision,coverage,completed_at) VALUES('insight_claims',?1,?2,?3,?4)
         ON CONFLICT(source,unit_id) DO UPDATE SET revision=excluded.revision, coverage=excluded.coverage, completed_at=excluded.completed_at",
        params![fingerprint, h(&json!([memory_id, entity])), coverage, now],
    )?;
    Ok(())
}

/// Process one unit. Returns the coverage label recorded on its receipt.
fn process(store: &Store, source: &str, unit: &str, now: &str) -> Result<String> {
    let conn = &store.conn;
    match source {
        "recon_entities" => {
            let (kind, ns, canonical, label): (String, String, String, String) = conn.query_row(
                "SELECT kind,namespace,canonical,label FROM recon_entities WHERE id=?1",
                [unit],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
            let subtype = actors::parse_subtype(&kind);
            let idents = vec![canonical.clone()];
            let name = if label.trim().is_empty() {
                &canonical
            } else {
                &label
            };
            let scope = format!("recon:{ns}");
            actors::bind(
                conn,
                &BindRequest {
                    name,
                    subtype,
                    scope: &scope,
                    identifiers: &idents,
                    external: Some(("recon_entities", unit)),
                    provenance: json!({"legacy":"recon_entities","id":unit}),
                },
                now,
            )?;
            Ok("bound".into())
        }
        "investigation_entities" => {
            let (thread, name, kind, idents): (String, String, String, String) = conn.query_row(
                "SELECT thread_id,canonical_name,entity_type,identifiers_json FROM investigation_entities WHERE id=?1",
                [unit],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
            if name.trim().is_empty() {
                return Ok("no name".into());
            }
            let idents = identifiers_of(&idents);
            let scope = format!("recon-thread:{thread}");
            actors::bind(
                conn,
                &BindRequest {
                    name: &name,
                    subtype: actors::parse_subtype(&kind),
                    scope: &scope,
                    identifiers: &idents,
                    external: Some(("investigation_entities", unit)),
                    provenance: json!({"legacy":"investigation_entities","id":unit}),
                },
                now,
            )?;
            Ok("bound".into())
        }
        "insight_claims" => claim_subject(conn, unit, now),
        "insight_derivation_links" => {
            let (snapshot, lk, lid, rk, rid, relation, rationale, rev): (String, String, String, String, String, String, String, String) = conn.query_row(
                "SELECT snapshot_id,left_kind,left_id,right_kind,right_id,relation,rationale,revision FROM insight_derivation_links WHERE id=?1",
                [unit],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
            )?;
            if !actors::LINK_RELATIONS.contains(&relation.as_str()) {
                return Ok(format!("unmapped relation {relation}"));
            }
            let endpoint = |kind: &str, id: &str| -> Result<Option<EndpointRef>> {
                let Some(kind) = MemoryKind::parse(kind)
                    .filter(|k| matches!(k, MemoryKind::Claim | MemoryKind::Inference))
                else {
                    return Ok(None);
                };
                let mem: Option<Option<String>> = conn
                    .query_row(
                        "SELECT memory_id FROM insight_derivations WHERE id=?1",
                        [format!("{snapshot}:{id}")],
                        |r| r.get(0),
                    )
                    .optional()?;
                Ok(mem
                    .flatten()
                    .map(|memory_id| EndpointRef::Memory { memory_id, kind }))
            };
            let (Some(source), Some(target)) = (endpoint(&lk, &lid)?, endpoint(&rk, &rid)?) else {
                return Ok("unmapped endpoints".into());
            };
            let (memory_id, _) = actors::write_link(
                conn,
                &LinkPayload {
                    source,
                    target,
                    relation,
                    rationale,
                    receipts: vec![format!("insight_derivation_links:{unit}:{rev}")],
                    promoted_from: None,
                },
                now,
            )?;
            legacy_map(conn, "insight_derivation_links", unit, &memory_id, now)?;
            Ok("link".into())
        }
        "intel_inference_article_leads" => {
            let (article, cand_article, cand_run, cand_rev, interp, gap, inference, generation): (String, String, String, String, String, String, String, i64) = conn.query_row(
                "SELECT article_id,candidate_article_id,candidate_run_id,candidate_metadata_revision,interpretation,verification_needed,inference_text,source_generation FROM intel_inference_article_leads WHERE id=?1",
                [unit],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
            )?;
            let run: Option<String> = conn
                .query_row("SELECT run_id FROM atlas_articles WHERE article_id=?1 ORDER BY run_id DESC LIMIT 1", [&article], |r| r.get(0))
                .optional()?;
            let Some(run) = run else {
                return Ok("source article not retained".into());
            };
            let target_exists: bool = conn.query_row(
                "SELECT COUNT(*)>0 FROM atlas_articles WHERE run_id=?1 AND article_id=?2",
                params![cand_run, cand_article],
                |r| r.get(0),
            )?;
            if !target_exists {
                return Ok("candidate article not retained".into());
            }
            let (memory_id, _) = actors::write_related(
                conn,
                &RelatedPayload {
                    source: EndpointRef::Article {
                        run_id: run,
                        article_id: article,
                        revision: String::new(),
                    },
                    target: EndpointRef::Article {
                        run_id: cand_run,
                        article_id: cand_article,
                        revision: cand_rev,
                    },
                    shared_actors: vec![],
                    basis: "potential corroboration lead (Intel brief Stage 4)".into(),
                    interpretation: if interp.is_empty() { inference } else { interp },
                    verification_gap: gap,
                    source_revisions: vec![format!("generation:{generation}")],
                },
                now,
            )?;
            legacy_map(conn, "intel_inference_article_leads", unit, &memory_id, now)?;
            Ok("related".into())
        }
        "atlas_articles" => index_article(store, unit, now),
        _ => Ok("unknown source".into()),
    }
}

fn legacy_map(
    conn: &Connection,
    table: &str,
    legacy: &str,
    memory_id: &str,
    now: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO brain_legacy_map(legacy_table,legacy_id,memory_id,migrated_at) VALUES(?1,?2,?3,?4)
         ON CONFLICT(legacy_table,legacy_id) DO UPDATE SET memory_id=excluded.memory_id",
        params![table, legacy, memory_id, now],
    )?;
    Ok(())
}

/// Index one Atlas article occurrence through Brain: bind actors from the
/// validated native title/description mentions, record mention receipts and
/// observed `mentioned_in` Links, then bounded tentative Related records by
/// candidate name (different provisional actors never become "same actor").
pub fn index_article(store: &Store, unit: &str, now: &str) -> Result<String> {
    let conn = &store.conn;
    let Some((run_id, article_id)) = unit.split_once('/') else {
        return Ok("bad unit".into());
    };
    let Some(article) = store.atlas_article(run_id, article_id)? else {
        return Ok("article gone".into());
    };
    let metadata = store.atlas_metadata(run_id, article_id)?;
    let revision = metadata
        .as_ref()
        .map(|m| m.input_hash.clone())
        .unwrap_or_default();
    // Reindex: drop this occurrence's stale contributions first.
    conn.execute(
        "DELETE FROM brain_actor_mentions WHERE origin_kind='article' AND origin_scope=?1 AND origin_id=?2 AND origin_revision<>?3",
        params![run_id, article_id, revision],
    )?;
    conn.execute(
        "UPDATE memory_metadata SET lifecycle='retired', updated_at=?4 WHERE memory_kind IN ('link','related') AND memory_id IN
           (SELECT memory_id FROM brain_memory_endpoints WHERE endpoint_kind='article' AND origin=?1 AND ref_id=?2 AND revision<>?3)",
        params![run_id, article_id, revision, now],
    )?;
    let Some(metadata) = metadata else {
        return Ok("metadata missing".into());
    };
    let found = crate::atlas_metadata::article_actors(&article, &metadata);
    if found.is_empty() {
        return Ok("zero actors".into());
    }
    let scope = format!("atlas-article:{article_id}");
    let article_ep = EndpointRef::Article {
        run_id: run_id.into(),
        article_id: article_id.into(),
        revision: revision.clone(),
    };
    let mut bound: Vec<(String, actors::Binding)> = Vec::new();
    for a in &found {
        let subtype = actors::parse_subtype(&a.type_hint);
        let key = crate::atlas_metadata::comparison_key(&a.display_name);
        let legacy = format!("{}:{key}", a.type_hint);
        let ext_scope = format!("atlas:{article_id}");
        let (binding, _) = actors::bind(
            conn,
            &BindRequest {
                name: &a.display_name,
                subtype,
                scope: &scope,
                identifiers: &[],
                external: Some((&ext_scope, &legacy)),
                provenance: json!({"atlas_run":run_id,"article":article_id}),
            },
            now,
        )?;
        let spans: Vec<Value> = a
            .mentions
            .iter()
            .map(|m| json!({"field":m.field,"start_byte":m.start_byte,"end_byte":m.end_byte}))
            .collect();
        let fields: Vec<&str> = {
            let mut f: Vec<&str> = a.mentions.iter().map(|m| m.field.as_str()).collect();
            f.dedup();
            f
        };
        actors::record_mention(
            conn,
            &binding.actor_id,
            "article",
            run_id,
            article_id,
            &revision,
            &Value::Array(spans),
            now,
        )?;
        actors::write_link(
            conn,
            &LinkPayload {
                source: EndpointRef::Memory {
                    memory_id: binding.actor_id.clone(),
                    kind: MemoryKind::Actor,
                },
                target: article_ep.clone(),
                relation: "mentioned_in".into(),
                rationale: format!("article {} mentions {}", fields.join("/"), a.display_name),
                receipts: vec![format!(
                    "atlas_article_mentions:{run_id}/{article_id}:{revision}"
                )],
                promoted_from: None,
            },
            now,
        )?;
        bound.push((a.display_name.clone(), binding));
    }
    // Bounded tentative Related records (≤ RELATED_PER_ARTICLE), newest
    // origins first; remaining candidates stay derived queries.
    let mut written = 0usize;
    'outer: for (display, binding) in &bound {
        for other in &binding.candidates {
            let mut origins = actors::mentions(conn, other)?;
            origins.retain(|(k, _, id, _)| k == "article" && id != article_id);
            origins.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)));
            for (_, orun, oid, orev) in origins {
                if written >= RELATED_PER_ARTICLE {
                    break 'outer;
                }
                actors::write_related(conn, &RelatedPayload {
                    source: article_ep.clone(),
                    target: EndpointRef::Article { run_id: orun, article_id: oid, revision: orev },
                    shared_actors: vec![],
                    basis: format!("candidate name: {display} (identity unconfirmed)"),
                    interpretation: "Articles mention actors with the same name; identity is not established.".into(),
                    verification_gap: "Confirm both mentions refer to the same entity.".into(),
                    source_revisions: vec![revision.clone()],
                }, now)?;
                written += 1;
            }
        }
    }
    Ok(format!("{} actor(s), {written} related", bound.len()))
}

impl Store {
    /// Run up to `max_units` pending backfill units in transactions of
    /// [`BATCH`]. Safe to call concurrently and repeatedly.
    pub fn brain_backfill(&self, max_units: usize) -> Result<BackfillReport> {
        let mut report = BackfillReport::default();
        let mut budget = max_units;
        for source in SOURCES {
            let pending: Vec<(String, String)> = {
                let have = receipts(&self.conn, source)?;
                units(&self.conn, source)?
                    .into_iter()
                    .filter(|(id, rev)| have.get(id) != Some(rev))
                    .collect()
            };
            for chunk in pending.chunks(BATCH) {
                if budget == 0 {
                    break;
                }
                let take = chunk.len().min(budget);
                budget -= take;
                let now = chrono::Utc::now().to_rfc3339();
                self.write_then_index(|| {
                    for (unit, rev) in &chunk[..take] {
                        self.conn.execute_batch("SAVEPOINT unit")?;
                        match process(self, source, unit, &now) {
                            Ok(coverage) => {
                                self.conn.execute(
                                    "INSERT INTO brain_backfill_receipts(source,unit_id,revision,coverage,completed_at) VALUES(?1,?2,?3,?4,?5)
                                     ON CONFLICT(source,unit_id) DO UPDATE SET revision=excluded.revision, coverage=excluded.coverage, completed_at=excluded.completed_at",
                                    params![source, unit, rev, coverage, now],
                                )?;
                                self.conn.execute_batch("RELEASE unit")?;
                                report.processed += 1;
                            }
                            Err(err) => {
                                self.conn.execute_batch("ROLLBACK TO unit; RELEASE unit")?;
                                report.failed += 1;
                                report.errors.push(format!("{source}/{unit}: {err}"));
                            }
                        }
                    }
                    Ok(((), vec![]))
                })?;
            }
        }
        report.coverage = self.brain_backfill_coverage()?;
        report.complete = report.failed == 0 && report.coverage.iter().all(|c| c.done >= c.total);
        Ok(report)
    }

    /// Per-source historical coverage (receipted units with a current revision).
    pub fn brain_backfill_coverage(&self) -> Result<Vec<SourceCoverage>> {
        let mut out = Vec::new();
        for source in SOURCES {
            let have = receipts(&self.conn, source)?;
            let all = units(&self.conn, source)?;
            out.push(SourceCoverage {
                source: source.to_string(),
                done: all
                    .iter()
                    .filter(|(id, rev)| have.get(id) == Some(rev))
                    .count(),
                total: all.len(),
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
#[path = "brain_backfill_tests.rs"]
mod tests;
