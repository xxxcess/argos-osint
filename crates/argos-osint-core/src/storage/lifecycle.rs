//! Actor and source lifecycle on SurrealDB (P3): `brain_actor`,
//! `brain_actor_mention` and `brain_actor_redirect`, plus the transactional
//! ports of `brain_actors::retire_origin`, `brain_actors::reconcile_lifecycle`
//! and `brain_source_lifecycle::reconcile_sources`.
//!
//! Actor refs and Link/Related endpoints are not separate tables here: they
//! are the `concerns`, `link` and `related` edges of the graph projection
//! (`storage::graph`), which exist only while their owner memory is active.
//! Every `*_in` function joins the caller's transaction and returns the
//! memory ids whose lifecycle it changed, so a [`super::memory::MemoryWrite`]
//! can hand them to `write_then_index` (index work plus the
//! `memories_changed` bump, both only when something changed).
//!
//! Record keys: `brain_actor` by actor (memory) id, `brain_actor_mention` by
//! mention id, `brain_actor_redirect` by `from_actor`, `brain_actor_alias`
//! by `[actor, normalized, alias_type]` and `brain_actor_external_ref` by
//! `[scope, legacy_id]`.

use std::collections::{BTreeSet, HashMap, HashSet};

use surrealdb::engine::any::Any;
use surrealdb::types::SurrealValue;
use surrealdb::Surreal;

use super::graph::memory_node;
use super::jobs::{rid, run, take};
use super::memory::set_lifecycle_in;
use super::tx::{Tx, TxError};

/// Lifecycle schema (idempotent).
pub const SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS brain_actor SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS subtype ON brain_actor TYPE string DEFAULT 'unknown';
DEFINE FIELD IF NOT EXISTS display_name ON brain_actor TYPE string;
DEFINE FIELD IF NOT EXISTS identity_state ON brain_actor TYPE string DEFAULT 'provisional';
DEFINE FIELD IF NOT EXISTS scope_key ON brain_actor TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS revision ON brain_actor TYPE int DEFAULT 1;
DEFINE FIELD IF NOT EXISTS provenance_json ON brain_actor TYPE string DEFAULT '{}';
DEFINE FIELD IF NOT EXISTS created_at ON brain_actor TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON brain_actor TYPE string;
DEFINE INDEX IF NOT EXISTS brain_actor_scope ON brain_actor FIELDS scope_key;
DEFINE TABLE IF NOT EXISTS brain_actor_mention SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS actor_id ON brain_actor_mention TYPE string;
DEFINE FIELD IF NOT EXISTS origin_kind ON brain_actor_mention TYPE string;
DEFINE FIELD IF NOT EXISTS origin_scope ON brain_actor_mention TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS origin_id ON brain_actor_mention TYPE string;
DEFINE FIELD IF NOT EXISTS origin_revision ON brain_actor_mention TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS spans_json ON brain_actor_mention TYPE string DEFAULT '[]';
DEFINE FIELD IF NOT EXISTS created_at ON brain_actor_mention TYPE string;
DEFINE INDEX IF NOT EXISTS brain_actor_mention_actor ON brain_actor_mention FIELDS actor_id;
DEFINE INDEX IF NOT EXISTS brain_actor_mention_origin ON brain_actor_mention FIELDS origin_kind, origin_scope, origin_id;
DEFINE TABLE IF NOT EXISTS brain_actor_redirect SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS to_actor ON brain_actor_redirect TYPE string;
DEFINE FIELD IF NOT EXISTS receipt_json ON brain_actor_redirect TYPE string DEFAULT '{}';
DEFINE FIELD IF NOT EXISTS created_at ON brain_actor_redirect TYPE string;
DEFINE INDEX IF NOT EXISTS brain_actor_redirect_to ON brain_actor_redirect FIELDS to_actor;
DEFINE INDEX IF NOT EXISTS brain_graph_node_origin ON brain_graph_node FIELDS kind, origin;
DEFINE TABLE IF NOT EXISTS brain_actor_alias SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS actor_id ON brain_actor_alias TYPE string;
DEFINE FIELD IF NOT EXISTS alias ON brain_actor_alias TYPE string;
DEFINE FIELD IF NOT EXISTS normalized ON brain_actor_alias TYPE string;
DEFINE FIELD IF NOT EXISTS alias_type ON brain_actor_alias TYPE string;
DEFINE FIELD IF NOT EXISTS provenance_json ON brain_actor_alias TYPE string DEFAULT '{}';
DEFINE INDEX IF NOT EXISTS brain_actor_alias_normalized ON brain_actor_alias FIELDS normalized, alias_type;
DEFINE TABLE IF NOT EXISTS brain_actor_external_ref SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS actor_id ON brain_actor_external_ref TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON brain_actor_external_ref TYPE string;";

/// Mentions of one origin; pinned to `brain_actor_mention_origin`.
pub const ORIGIN_MENTIONS_SURQL: &str = "SELECT VALUE actor_id FROM brain_actor_mention \
     WHERE origin_kind = $kind AND origin_scope = $scope";
/// Endpoint nodes of one origin; pinned to `brain_graph_node_origin`.
pub const ORIGIN_NODES_SURQL: &str =
    "SELECT VALUE id FROM brain_graph_node WHERE kind = $kind AND origin = $scope";

/// Claim kinds whose `concerns` edge keeps an actor alive.
const CLAIM_KINDS: [&str; 3] = ["claim", "atomic_claim", "inference"];
/// Scope prefixes of the provisional actors lifecycle reconciles.
const RECONCILED_SCOPES: [&str; 7] = [
    "atlas-article:",
    "atlas:",
    "article:",
    "recon:",
    "recon-thread:",
    "recon-deleted:",
    "insight-memory:",
];

pub async fn apply_schema(db: &Surreal<Any>) -> Result<(), TxError> {
    db.query(SCHEMA).await?.check()?;
    Ok(())
}

/// A new or updated actor row (the actor's memory is written separately).
pub async fn upsert_actor_in(
    tx: &Tx,
    actor_id: &str,
    display_name: &str,
    scope_key: &str,
    now: &str,
) -> Result<(), TxError> {
    run(
        tx,
        tx.query(
            "UPSERT $a SET display_name = $name, scope_key = $scope, \
             created_at = created_at ?? $now, updated_at = $now RETURN NONE",
        )
        .bind(("a", rid("brain_actor", actor_id)))
        .bind(("name", display_name.to_string()))
        .bind(("scope", scope_key.to_string()))
        .bind(("now", now.to_string())),
    )
    .await?;
    Ok(())
}

/// One mention of `actor_id` by an origin record.
pub async fn add_mention_in(
    tx: &Tx,
    mention_id: &str,
    actor_id: &str,
    origin_kind: &str,
    origin_scope: &str,
    origin_id: &str,
    now: &str,
) -> Result<(), TxError> {
    run(
        tx,
        tx.query(
            "UPSERT $m SET actor_id = $actor, origin_kind = $kind, origin_scope = $scope, \
             origin_id = $origin, created_at = $now RETURN NONE",
        )
        .bind(("m", rid("brain_actor_mention", mention_id)))
        .bind(("actor", actor_id.to_string()))
        .bind(("kind", origin_kind.to_string()))
        .bind(("scope", origin_scope.to_string()))
        .bind(("origin", origin_id.to_string()))
        .bind(("now", now.to_string())),
    )
    .await?;
    Ok(())
}

pub(crate) async fn redirects(tx: &Tx) -> Result<HashMap<String, String>, TxError> {
    #[derive(SurrealValue)]
    struct Row {
        from: String,
        to: String,
    }
    let mut r = run(
        tx,
        tx.query("SELECT record::id(id) AS from, to_actor AS to FROM brain_actor_redirect"),
    )
    .await?;
    let rows: Vec<Row> = take(&mut r, 0)?;
    Ok(rows.into_iter().map(|r| (r.from, r.to)).collect())
}

pub(crate) fn resolve(map: &HashMap<String, String>, id: &str) -> String {
    let mut cur = id.to_string();
    let mut seen = HashSet::new();
    while let Some(next) = map.get(&cur) {
        if !seen.insert(cur.clone()) {
            break;
        }
        cur = next.clone();
    }
    cur
}

/// Merge `from` into `to`. Aborts when it would make a cycle.
pub async fn add_redirect_in(tx: &Tx, from: &str, to: &str, now: &str) -> Result<(), TxError> {
    let map = redirects(tx).await?;
    if from == to || resolve(&map, to) == from {
        return Err(TxError::Abort(format!(
            "redirect {from} -> {to} makes a cycle"
        )));
    }
    run(
        tx,
        tx.query("UPSERT $r SET to_actor = $to, created_at = $now RETURN NONE")
            .bind(("r", rid("brain_actor_redirect", from)))
            .bind(("to", to.to_string()))
            .bind(("now", now.to_string())),
    )
    .await?;
    Ok(())
}

/// `root` and every actor redirected into it, transitively.
pub(crate) fn family(map: &HashMap<String, String>, root: &str) -> Vec<String> {
    let mut out = vec![root.to_string()];
    let mut i = 0;
    while i < out.len() {
        let cur = out[i].clone();
        for (from, to) in map {
            if *to == cur && !out.contains(from) {
                out.push(from.clone());
            }
        }
        i += 1;
    }
    out
}

/// Which of `ids` still have support: a mention, a `concerns` edge from an
/// active claim/inference other than itself, a pin, or a user edit. One
/// round trip.
async fn supported(tx: &Tx, ids: &[String]) -> Result<HashSet<String>, TxError> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let mut sql = String::new();
    for i in 0..ids.len() {
        sql.push_str(&format!(
            "RETURN count(SELECT id FROM brain_actor_mention WHERE actor_id = $i{i} LIMIT 1) > 0
               OR count(SELECT id FROM brain_graph_edge WHERE out = $n{i} AND meaning = 'concerns'
                    AND in.memory_kind INSIDE $kinds AND owner != $i{i} LIMIT 1) > 0
               OR ((SELECT VALUE pinned FROM ONLY $m{i}) ?? false)
               OR record::exists($e{i});"
        ));
    }
    let mut q = tx
        .query(sql)
        .bind(("kinds", CLAIM_KINDS.map(String::from).to_vec()));
    for (i, id) in ids.iter().enumerate() {
        q = q
            .bind((format!("i{i}"), id.clone()))
            .bind((format!("n{i}"), rid("brain_graph_node", &memory_node(id))))
            .bind((format!("m{i}"), rid("memory", id)))
            .bind((format!("e{i}"), rid("insight_user_edit", id)));
    }
    let mut r = run(tx, q).await?;
    let mut out = HashSet::new();
    for (i, id) in ids.iter().enumerate() {
        if take::<Option<bool>>(&mut r, i)? == Some(true) {
            out.insert(id.clone());
        }
    }
    Ok(out)
}

/// Active Link/Related memories owning an edge touching any of `nodes`.
async fn relationships_touching(
    tx: &Tx,
    where_sql: &str,
    binds: Vec<(&'static str, String)>,
) -> Result<Vec<String>, TxError> {
    let sql = format!(
        "LET $nodes = ({where_sql});
         SELECT VALUE <->brain_graph_edge FROM $nodes;"
    );
    let mut q = tx.query(sql);
    for b in binds {
        q = q.bind(b);
    }
    let mut r = run(tx, q).await?;
    let edges: Vec<surrealdb::types::RecordId> =
        take::<Vec<Vec<surrealdb::types::RecordId>>>(&mut r, 1)?
            .into_iter()
            .flatten()
            .collect();
    if edges.is_empty() {
        return Ok(Vec::new());
    }
    let mut r = run(
        tx,
        tx.query("SELECT VALUE owner FROM $edges WHERE meaning INSIDE ['link', 'related']")
            .bind(("edges", edges)),
    )
    .await?;
    let owners: BTreeSet<String> = take::<Vec<String>>(&mut r, 0)?.into_iter().collect();
    Ok(owners.into_iter().collect())
}

/// `brain_actors::retire_origin`: drop one origin's mentions, retire the
/// Link/Related records with an endpoint of that origin, and retire the
/// origin's actors that have no support left anywhere in their family.
/// Returns (relationships retired, every memory id changed).
pub async fn retire_origin_in(
    tx: &Tx,
    origin_kind: &str,
    origin_scope: &str,
) -> Result<(usize, Vec<String>), TxError> {
    let mut r = run(
        tx,
        tx.query(format!(
            "{ORIGIN_MENTIONS_SURQL}; DELETE brain_actor_mention \
             WHERE origin_kind = $kind AND origin_scope = $scope RETURN NONE;"
        ))
        .bind(("kind", origin_kind.to_string()))
        .bind(("scope", origin_scope.to_string())),
    )
    .await?;
    let affected: Vec<String> = take(&mut r, 0)?;
    let rels = relationships_touching(
        tx,
        ORIGIN_NODES_SURQL,
        vec![
            ("kind", origin_kind.to_string()),
            ("scope", origin_scope.to_string()),
        ],
    )
    .await?;
    let mut changed = set_lifecycle_in(tx, &rels, "retired").await?;
    let retired = changed.len();
    let map = redirects(tx).await?;
    let roots: BTreeSet<String> = affected.iter().map(|a| resolve(&map, a)).collect();
    let mut dead = Vec::new();
    for root in roots {
        let fam = family(&map, &root);
        if supported(tx, &fam).await?.is_empty() {
            dead.push(root);
        }
    }
    changed.extend(set_lifecycle_in(tx, &dead, "retired").await?);
    Ok((retired, changed))
}

#[derive(SurrealValue)]
struct SourceRow {
    id: surrealdb::types::RecordId,
    fingerprint: String,
    run_id: Option<String>,
    answer_id: String,
    call_id: String,
}

fn article_of(call_id: &str) -> &str {
    call_id.strip_prefix("article:").unwrap_or(call_id)
}

/// `brain_source_lifecycle::reconcile_sources`: re-home article sources whose
/// run is gone to the article's latest retained run, drop sources whose
/// article is gone everywhere, and retire article/Recon claims left with no
/// live source. Returns the memory ids retired.
pub async fn reconcile_sources_in(tx: &Tx) -> Result<Vec<String>, TxError> {
    #[derive(SurrealValue)]
    struct Art {
        run_id: String,
        article_id: String,
        seen_at: String,
    }
    let mut r = run(
        tx,
        tx.query(
            "SELECT run_id, article_id, seen_at FROM atlas_article;
             SELECT VALUE record::id(id) FROM insight_snapshot WHERE owner_kind INSIDE ['article', 'atlas'];
             SELECT record::id(id) AS sid, owner_id FROM insight_snapshot WHERE owner_kind INSIDE ['article', 'atlas'];
             SELECT id, fingerprint, run_id, answer_id, call_id FROM insight_source WHERE thread_id = NONE;",
        ),
    )
    .await?;
    let arts: Vec<Art> = take(&mut r, 0)?;
    let snap_ids: HashSet<String> = take::<Vec<String>>(&mut r, 1)?.into_iter().collect();
    #[derive(SurrealValue)]
    struct Snap {
        sid: String,
        owner_id: String,
    }
    let snap_owner: HashMap<String, String> = take::<Vec<Snap>>(&mut r, 2)?
        .into_iter()
        .map(|s| (s.sid, s.owner_id))
        .collect();
    let sources: Vec<SourceRow> = take(&mut r, 3)?;
    let present: HashSet<(String, String)> = arts
        .iter()
        .map(|a| (a.run_id.clone(), a.article_id.clone()))
        .collect();
    // Latest run per article: seen_at DESC, then run_id.
    let mut latest: HashMap<String, (String, String)> = HashMap::new();
    for a in &arts {
        let e = latest
            .entry(a.article_id.clone())
            .or_insert((a.seen_at.clone(), a.run_id.clone()));
        if (a.seen_at.as_str(), std::cmp::Reverse(a.run_id.as_str()))
            > (e.0.as_str(), std::cmp::Reverse(e.1.as_str()))
        {
            *e = (a.seen_at.clone(), a.run_id.clone());
        }
    }
    let mut sql = String::new();
    let mut binds: Vec<(String, surrealdb::types::Value)> = Vec::new();
    let mut n = 0usize;
    for s in sources {
        let article = article_of(&s.call_id).to_string();
        let article_backed = s.call_id.starts_with("article:")
            || s.answer_id.starts_with("atlas-")
            || snap_ids.contains(&s.answer_id);
        let here = s
            .run_id
            .as_ref()
            .is_some_and(|run| present.contains(&(run.clone(), article.clone())));
        let snapshot_gone = snap_owner
            .get(&s.answer_id)
            .is_some_and(|owner| !latest.contains_key(owner));
        let drop = snapshot_gone
            || ((s.call_id.starts_with("article:") || s.answer_id.starts_with("atlas-"))
                && !latest.contains_key(&article));
        if drop {
            sql.push_str(&format!("DELETE $s{n} RETURN NONE;"));
            binds.push((format!("s{n}"), s.id.into_value()));
            n += 1;
        } else if article_backed && !here {
            if let Some((_, run_id)) = latest.get(&article) {
                let answer = if s.answer_id.starts_with("atlas-") {
                    format!("atlas-{run_id}")
                } else {
                    s.answer_id.clone()
                };
                let key = super::codec::encode(&[&s.fingerprint, &answer, &s.call_id]);
                // Re-key; an existing row for the new key wins (UPDATE OR IGNORE).
                sql.push_str(&format!(
                    "LET $o{n} = (SELECT * FROM ONLY $s{n});
                     IF !record::exists(type::record('insight_source', $k{n})) {{
                       CREATE type::record('insight_source', $k{n}) CONTENT
                         object::extend(object::remove($o{n}, 'id'), {{ run_id: $r{n}, answer_id: $a{n} }})
                         RETURN NONE;
                       DELETE $s{n} RETURN NONE;
                     }};"
                ));
                binds.push((format!("s{n}"), s.id.into_value()));
                binds.push((format!("k{n}"), key.into_value()));
                binds.push((format!("r{n}"), run_id.clone().into_value()));
                binds.push((format!("a{n}"), answer.into_value()));
                n += 1;
            }
        }
    }
    if !sql.is_empty() {
        let mut q = tx.query(sql);
        for b in binds {
            q = q.bind(b);
        }
        run(tx, q).await?;
    }
    // Claims of article/Recon memories with no live source retire.
    #[derive(SurrealValue)]
    struct Orphan {
        memory_id: String,
        source_json: Option<String>,
    }
    #[derive(SurrealValue)]
    struct Claim {
        fp: String,
        memory_id: String,
    }
    let mut r = run(
        tx,
        tx.query(
            "SELECT record::id(id) AS fp, memory_id FROM insight_claim;
             SELECT VALUE fingerprint FROM insight_source WHERE deleted_origin = false;",
        ),
    )
    .await?;
    let claims: Vec<Claim> = take(&mut r, 0)?;
    let live: HashSet<String> = take::<Vec<String>>(&mut r, 1)?.into_iter().collect();
    let candidates: Vec<surrealdb::types::RecordId> = claims
        .into_iter()
        .filter(|c| !live.contains(&c.fp))
        .map(|c| rid("memory", &c.memory_id))
        .collect();
    let mut r = run(
        tx,
        tx.query(
            "SELECT record::id(id) AS memory_id, source_json FROM $rids \
             WHERE lifecycle = 'active' AND memory_kind INSIDE $kinds",
        )
        .bind(("rids", candidates))
        .bind(("kinds", CLAIM_KINDS.map(String::from).to_vec())),
    )
    .await?;
    let orphans: Vec<Orphan> = take(&mut r, 0)?;
    let ids: Vec<String> = orphans
        .into_iter()
        .filter(|o| {
            let app = o
                .source_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                .and_then(|v| v.get("app").and_then(|a| a.as_str()).map(str::to_string));
            matches!(
                app.as_deref(),
                Some("article" | "atlas" | "recon" | "recon-deleted")
            )
        })
        .map(|o| o.memory_id)
        .collect();
    set_lifecycle_in(tx, &ids, "retired").await
}

/// `brain_actors::reconcile_lifecycle`: reconcile sources, set each
/// provisional actor family active or retired by its support, then retire
/// Link/Related records whose memory endpoint is no longer active (repeated,
/// since a relationship can be another one's endpoint). Returns every memory
/// id changed.
pub async fn reconcile_lifecycle_in(tx: &Tx) -> Result<Vec<String>, TxError> {
    let mut changed = reconcile_sources_in(tx).await?;
    #[derive(SurrealValue)]
    struct Actor {
        id: String,
        scope_key: String,
    }
    let mut r = run(
        tx,
        tx.query("SELECT record::id(id) AS id, scope_key FROM brain_actor"),
    )
    .await?;
    let actors: Vec<Actor> = take(&mut r, 0)?;
    let map = redirects(tx).await?;
    let scoped: Vec<String> = actors
        .into_iter()
        .filter(|a| {
            a.scope_key.is_empty() || RECONCILED_SCOPES.iter().any(|p| a.scope_key.starts_with(p))
        })
        .map(|a| a.id)
        .filter(|id| !map.contains_key(id))
        .collect();
    let (mut live, mut dead) = (Vec::new(), Vec::new());
    for root in scoped {
        let fam = family(&map, &root);
        if supported(tx, &fam).await?.is_empty() {
            dead.push(root);
        } else {
            live.push(root);
        }
    }
    // Redirected actors keep their lifecycle.
    let skip = |ids: Vec<String>| async move {
        let mut r = run(
            tx,
            tx.query("SELECT VALUE record::id(id) FROM $rids WHERE lifecycle != 'redirected'")
                .bind((
                    "rids",
                    ids.iter().map(|i| rid("memory", i)).collect::<Vec<_>>(),
                )),
        )
        .await?;
        take::<Vec<String>>(&mut r, 0)
    };
    let live = skip(live).await?;
    let dead = skip(dead).await?;
    changed.extend(set_lifecycle_in(tx, &live, "active").await?);
    changed.extend(set_lifecycle_in(tx, &dead, "retired").await?);
    loop {
        let stale = relationships_touching(
            tx,
            "SELECT VALUE id FROM brain_graph_node WHERE kind = 'memory' \
             AND lifecycle NOTINSIDE ['active', '']",
            Vec::new(),
        )
        .await?;
        let retired = set_lifecycle_in(tx, &stale, "retired").await?;
        if retired.is_empty() {
            break;
        }
        changed.extend(retired);
    }
    Ok(changed)
}

// ---- Atlas phase 6 (actor index) ----------------------------------------

/// `atlas_article_index` receipts, keyed `codec::encode([run_id, article_id])`.
/// Register in bootstrap as `("actor_index", lifecycle::ARTICLE_INDEX_SCHEMA)`.
pub const ARTICLE_INDEX_SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS atlas_article_index SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS run_id ON atlas_article_index TYPE string;
DEFINE FIELD IF NOT EXISTS article_id ON atlas_article_index TYPE string;
DEFINE FIELD IF NOT EXISTS input_revision ON atlas_article_index TYPE string;
DEFINE FIELD IF NOT EXISTS index_checksum ON atlas_article_index TYPE string;
DEFINE FIELD IF NOT EXISTS actor_coverage ON atlas_article_index TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS indexed_at ON atlas_article_index TYPE string;
DEFINE INDEX IF NOT EXISTS atlas_article_index_run ON atlas_article_index FIELDS run_id;";

pub async fn apply_article_index_schema(db: &Surreal<Any>) -> Result<(), TxError> {
    db.query(ARTICLE_INDEX_SCHEMA).await?.check()?;
    Ok(())
}

/// One `atlas_article_index` receipt.
#[derive(Debug, Clone, SurrealValue)]
pub struct ArticleIndexRow {
    pub article_id: String,
    pub input_revision: String,
    pub index_checksum: String,
    pub actor_coverage: String,
}

/// Receipts of one run (checksums are checked by the caller).
pub async fn article_index_receipts(
    db: &Surreal<Any>,
    run_id: &str,
) -> Result<Vec<ArticleIndexRow>, TxError> {
    let mut r = db
        .query(
            "SELECT article_id, input_revision, index_checksum, actor_coverage \
             FROM atlas_article_index WHERE run_id = $run",
        )
        .bind(("run", run_id.to_string()))
        .await?
        .check()?;
    Ok(r.take(0)?)
}

/// Upsert one occurrence's receipt inside the caller's transaction.
pub async fn put_article_index_in(
    tx: &Tx,
    run_id: &str,
    article_id: &str,
    row: &ArticleIndexRow,
    now: &str,
) -> Result<(), TxError> {
    if run_id.is_empty() || article_id.is_empty() {
        return Err(TxError::Abort("atlas_article_index: empty id".into()));
    }
    run(
        tx,
        tx.query(
            "UPSERT type::record('atlas_article_index', $key) SET run_id = $run, \
             article_id = $article, input_revision = $rev, index_checksum = $sum, \
             actor_coverage = $cov, indexed_at = $now RETURN NONE",
        )
        .bind(("key", super::codec::encode(&[run_id, article_id])))
        .bind(("run", run_id.to_string()))
        .bind(("article", article_id.to_string()))
        .bind(("rev", row.input_revision.clone()))
        .bind(("sum", row.index_checksum.clone()))
        .bind(("cov", row.actor_coverage.clone()))
        .bind(("now", now.to_string())),
    )
    .await?;
    Ok(())
}

/// Reindex prelude for one article occurrence: drop its mentions and
/// retire Link/Related records with an endpoint on any revision of it.
/// Returns the memory ids retired.
pub async fn retire_article_occurrence_in(
    tx: &Tx,
    run_id: &str,
    article_id: &str,
) -> Result<Vec<String>, TxError> {
    run(
        tx,
        tx.query(
            "DELETE brain_actor_mention WHERE origin_kind = 'article' \
             AND origin_scope = $run AND origin_id = $article RETURN NONE",
        )
        .bind(("run", run_id.to_string()))
        .bind(("article", article_id.to_string())),
    )
    .await?;
    let rels = relationships_touching(
        tx,
        "SELECT VALUE id FROM brain_graph_node WHERE kind = 'article' \
         AND origin = $run AND ref_id = $article",
        vec![
            ("run", run_id.to_string()),
            ("article", article_id.to_string()),
        ],
    )
    .await?;
    set_lifecycle_in(tx, &rels, "retired").await
}

/// `brain_actors::record_mention`: resolve the actor, revive it if
/// retired, and upsert the deterministic mention receipt (spans refreshed).
/// Returns the memory ids whose lifecycle changed.
#[allow(clippy::too_many_arguments)]
pub async fn record_mention_in(
    tx: &Tx,
    actor_id: &str,
    origin_kind: &str,
    origin_scope: &str,
    origin_id: &str,
    origin_revision: &str,
    spans_json: &str,
    now: &str,
) -> Result<Vec<String>, TxError> {
    if actor_id.is_empty() {
        return Err(TxError::Abort("mention: empty actor id".into()));
    }
    let canonical = super::actor_bind::resolve_in(tx, actor_id).await?;
    let changed = set_lifecycle_in(tx, std::slice::from_ref(&canonical), "active").await?;
    let id = crate::brain_actors::hid(
        "mention",
        &[
            &canonical,
            origin_kind,
            origin_scope,
            origin_id,
            origin_revision,
        ],
    );
    run(
        tx,
        tx.query(
            "IF record::exists($m) { UPDATE $m SET spans_json = $spans RETURN NONE } \
             ELSE { CREATE $m SET actor_id = $actor, origin_kind = $kind, \
             origin_scope = $scope, origin_id = $origin, origin_revision = $rev, \
             spans_json = $spans, created_at = $now RETURN NONE };",
        )
        .bind(("m", rid("brain_actor_mention", &id)))
        .bind(("actor", canonical))
        .bind(("kind", origin_kind.to_string()))
        .bind(("scope", origin_scope.to_string()))
        .bind(("origin", origin_id.to_string()))
        .bind(("rev", origin_revision.to_string()))
        .bind(("spans", spans_json.to_string()))
        .bind(("now", now.to_string())),
    )
    .await?;
    Ok(changed)
}

/// `brain_actors::direct_mentions`: (kind, scope, id, revision) of one actor.
pub async fn direct_mentions_in(
    tx: &Tx,
    actor_id: &str,
) -> Result<Vec<(String, String, String, String)>, TxError> {
    #[derive(SurrealValue)]
    struct Row {
        origin_kind: String,
        origin_scope: String,
        origin_id: String,
        origin_revision: String,
    }
    let mut r = run(
        tx,
        tx.query(
            "SELECT origin_kind, origin_scope, origin_id, origin_revision \
             FROM brain_actor_mention WHERE actor_id = $actor \
             ORDER BY origin_scope, origin_id",
        )
        .bind(("actor", actor_id.to_string())),
    )
    .await?;
    let rows: Vec<Row> = take(&mut r, 0)?;
    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.origin_kind,
                r.origin_scope,
                r.origin_id,
                r.origin_revision,
            )
        })
        .collect())
}

/// `brain_actors::candidates` minus `actor_id`: other canonical actors with
/// the same normalized name (and subtype).
pub async fn name_candidates_in(
    tx: &Tx,
    name: &str,
    subtype: &str,
    actor_id: &str,
) -> Result<Vec<String>, TxError> {
    let key = crate::brain_actors::normalize_alias(name);
    if key.is_empty() {
        return Ok(Vec::new());
    }
    let mut r = run(
        tx,
        tx.query(
            "SELECT VALUE actor_id FROM brain_actor_alias \
             WHERE normalized = $k AND alias_type = 'name'",
        )
        .bind(("k", key)),
    )
    .await?;
    let ids: Vec<String> = take(&mut r, 0)?;
    let mut out = BTreeSet::new();
    for id in ids {
        if !subtype.is_empty() {
            let mut r = run(
                tx,
                tx.query("SELECT VALUE subtype FROM ONLY $a")
                    .bind(("a", rid("brain_actor", &id))),
            )
            .await?;
            if take::<Option<String>>(&mut r, 0)?.as_deref() != Some(subtype) {
                continue;
            }
        }
        let canonical = super::actor_bind::resolve_in(tx, &id).await?;
        if canonical != actor_id {
            out.insert(canonical);
        }
    }
    Ok(out.into_iter().collect())
}
