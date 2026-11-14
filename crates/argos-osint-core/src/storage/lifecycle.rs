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
use std::sync::{Arc, Weak};

use serde::{Deserialize, Serialize};
use surrealdb::engine::any::Any;
use surrealdb::types::{RecordId, SurrealValue};
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
DEFINE FIELD IF NOT EXISTS publication_batch ON brain_actor_mention TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS native_input_revision ON brain_actor_mention TYPE string DEFAULT '';
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
DEFINE FIELD IF NOT EXISTS created_at ON brain_actor_external_ref TYPE string;
DEFINE INDEX IF NOT EXISTS insight_source_article_call ON insight_source FIELDS call_id, run_id;
DEFINE INDEX IF NOT EXISTS insight_source_answer_run ON insight_source FIELDS answer_id, run_id;
DEFINE INDEX IF NOT EXISTS insight_snapshot_owner_scope ON insight_snapshot FIELDS owner_kind, owner_id;
DEFINE INDEX IF NOT EXISTS insight_owner_scope ON insight_owner_state FIELDS owner_kind, owner_id;
DEFINE INDEX IF NOT EXISTS brain_graph_node_occurrence ON brain_graph_node FIELDS kind, origin, ref_id;
DEFINE INDEX IF NOT EXISTS brain_graph_edge_input ON brain_graph_edge FIELDS in, meaning;
DEFINE INDEX IF NOT EXISTS brain_graph_edge_output ON brain_graph_edge FIELDS out, meaning;
DEFINE TABLE IF NOT EXISTS lifecycle_reconcile SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS version ON lifecycle_reconcile TYPE string;
DEFINE FIELD IF NOT EXISTS scope_hash ON lifecycle_reconcile TYPE string;
DEFINE FIELD IF NOT EXISTS scope_json ON lifecycle_reconcile TYPE string;
DEFINE FIELD IF NOT EXISTS phase ON lifecycle_reconcile TYPE string;
DEFINE FIELD IF NOT EXISTS revision ON lifecycle_reconcile TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS guards_json ON lifecycle_reconcile TYPE string DEFAULT '{}';
DEFINE TABLE IF NOT EXISTS lifecycle_reconcile_frontier SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS receipt_key ON lifecycle_reconcile_frontier TYPE string;
DEFINE FIELD IF NOT EXISTS phase ON lifecycle_reconcile_frontier TYPE string;
DEFINE FIELD IF NOT EXISTS identity ON lifecycle_reconcile_frontier TYPE string;
DEFINE FIELD IF NOT EXISTS priority ON lifecycle_reconcile_frontier TYPE int;
DEFINE FIELD IF NOT EXISTS payload_json ON lifecycle_reconcile_frontier TYPE string;
DEFINE FIELD IF NOT EXISTS initial_json ON lifecycle_reconcile_frontier TYPE string;
DEFINE FIELD IF NOT EXISTS generation ON lifecycle_reconcile_frontier TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS state ON lifecycle_reconcile_frontier TYPE string DEFAULT 'pending';
DEFINE INDEX IF NOT EXISTS lifecycle_reconcile_pending ON lifecycle_reconcile_frontier
  FIELDS receipt_key, phase, priority, state, generation, identity;
DEFINE INDEX IF NOT EXISTS lifecycle_reconcile_generation ON lifecycle_reconcile_frontier
  FIELDS receipt_key, phase, priority, generation, identity;
DEFINE TABLE IF NOT EXISTS lifecycle_support_epoch SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS revision ON lifecycle_support_epoch TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS last_batch ON lifecycle_support_epoch TYPE string DEFAULT '';
DEFINE FIELD IF NOT EXISTS lifecycle_ingest_batch ON memory TYPE string DEFAULT '';";

/// One logical writer batch. Clone it across a publication's mention/redirect
/// operations so support fencing advances once, without inventing index work.
#[derive(Debug, Clone)]
pub struct SupportBatch {
    nonce: String,
    owner: Weak<surrealdb::method::Transaction<Any>>,
    bumped: Arc<tokio::sync::Mutex<bool>>,
    capture: Option<CapturePermit>,
}

impl SupportBatch {
    /// Returns the operational generation only to this transaction's owner.
    pub(crate) fn publication_id(&self, tx: &Tx) -> Result<&str, TxError> {
        self.check_owner(tx)?;
        Ok(&self.nonce)
    }
    /// Creates one batch inside a transaction attempt. Recreate it on retry;
    /// clones share the actual owner and the once gate within that attempt.
    pub fn new(tx: &Tx) -> Self {
        Self {
            nonce: crate::store::new_id(),
            owner: Arc::downgrade(tx),
            bumped: Arc::new(tokio::sync::Mutex::new(false)),
            capture: None,
        }
    }

    /// Binds a source mutation to a completed capture in this attempt.
    pub fn for_capture(tx: &Tx, capture: &ReconcileCapture) -> Result<Self, TxError> {
        if !capture.ready {
            return Err(reconcile_error("capture_not_ready"));
        }
        let permit = capture
            .permit
            .as_ref()
            .ok_or_else(|| reconcile_error("capture_permit_missing"))?;
        permit.check_owner(tx)?;
        let mut batch = Self::new(tx);
        batch.capture = Some(permit.clone());
        Ok(batch)
    }

    fn check_owner(&self, tx: &Tx) -> Result<(), TxError> {
        let Some(owner) = self.owner.upgrade() else {
            return Err(TxError::Abort("lifecycle_support_batch_expired".into()));
        };
        if !Arc::ptr_eq(&owner, tx) {
            return Err(TxError::Abort(
                "lifecycle_support_batch_transaction_mismatch".into(),
            ));
        }
        Ok(())
    }
}

/// Fences mention-only support mutations in their owning transaction. Canonical
/// memory, pin and user-edit writes additionally carry `memories_changed`.
pub async fn bump_support_epoch_in(tx: &Tx, batch: &SupportBatch) -> Result<(), TxError> {
    batch.check_owner(tx)?;
    let mut bumped = batch.bumped.lock().await;
    if *bumped {
        return Ok(());
    }
    super::classifier_admission::tx_query(
        tx,
        "LET $head = (SELECT revision, last_batch FROM ONLY lifecycle_support_epoch:current);
         IF ($head.last_batch ?? '') != $batch {
           UPSERT lifecycle_support_epoch:current SET revision = ($head.revision ?? 0) + 1,
             last_batch = $batch RETURN NONE;
         };",
    )
    .bind(("batch", batch.nonce.clone()))
    .await?
    .check()?;
    *bumped = true;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SupportFence {
    support: i64,
    memories: i64,
}

async fn support_fence_in(tx: &Tx) -> Result<SupportFence, TxError> {
    let mut result = super::classifier_admission::tx_query(
        tx,
        "UPSERT lifecycle_support_epoch:current SET revision=revision ?? 0,
           last_batch=last_batch ?? '' RETURN NONE;
         UPSERT $counter SET n=n ?? 0 RETURN NONE;
         RETURN (SELECT VALUE revision FROM ONLY lifecycle_support_epoch:current) ?? 0;
         RETURN (SELECT VALUE n FROM ONLY $counter) ?? 0;",
    )
    .bind(("counter", rid("counter", super::publisher::CHANGED_COUNTER)))
    .await?
    .check()?;
    let support = take::<Option<i64>>(&mut result, 2)?
        .ok_or_else(|| TxError::Abort("lifecycle_support_epoch_missing".into()))?;
    let memories = take::<Option<i64>>(&mut result, 3)?
        .ok_or_else(|| TxError::Abort("lifecycle_memory_epoch_missing".into()))?;
    if support < 0 || memories < 0 {
        return Err(TxError::Abort("lifecycle_support_epoch_corrupt".into()));
    }
    Ok(SupportFence { support, memories })
}

/// Persisted frontier protocol, independent of the admission receipt schema.
pub const RECONCILE_VERSION: &str = "atlas-reconcile-frontier-v1";
/// Maximum explicit identities accepted by one affected operation.
pub const RECONCILE_SCOPE_KEYS: usize = 512;
/// Maximum identities processed or discovered by one continuation transaction.
pub const RECONCILE_STEP_IDENTITIES: usize = 256;
const RECONCILE_ITEM_BYTES: usize = 128 * 1024;

/// An identity whose retained support or graph endpoints may have changed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReconcileKey {
    Source { run_id: String, article_id: String },
    Actor { actor_id: String },
    Relationship { memory_id: String },
}

/// Explicit affected identities. Operations validate the bound before any write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconcileScope {
    pub keys: Vec<ReconcileKey>,
}

/// Work committed by one bounded lifecycle continuation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconcileProgress {
    pub changed_memory_ids: Vec<String>,
    pub next: Option<ReconcileCursor>,
    pub examined: u64,
}

/// A continuation bound to the immutable scope and persisted frontier revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconcileCursor {
    pub version: String,
    pub receipt_key: String,
    pub frontier_kind: String,
    pub after_key: String,
}

/// Pre-change capture must settle before a caller removes reverse evidence.
#[derive(Debug, Clone)]
pub struct ReconcileCapture {
    pub ready: bool,
    pub cursor: ReconcileCursor,
    pub examined: u64,
    permit: Option<CapturePermit>,
}

#[derive(Debug, Clone)]
struct CapturePermit {
    owner: Weak<surrealdb::method::Transaction<Any>>,
    receipt_key: String,
    revision: i64,
    examined: usize,
}

impl CapturePermit {
    fn check_owner(&self, tx: &Tx) -> Result<(), TxError> {
        let Some(owner) = self.owner.upgrade() else {
            return Err(reconcile_error("capture_permit_expired"));
        };
        if !Arc::ptr_eq(&owner, tx) {
            return Err(reconcile_error("capture_transaction_mismatch"));
        }
        Ok(())
    }
}

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
    // Scope changes can add/remove catalog support without a canonical write.
    bump_support_epoch_in(tx, &SupportBatch::new(tx)).await?;
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
    super::actor_search_outbox::enqueue_actors_in(
        tx,
        &[actor_id.to_owned()],
        None,
        chrono::Utc::now(),
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
    let batch = SupportBatch::new(tx);
    bump_support_epoch_in(tx, &batch).await?;
    run(
        tx,
        tx.query(
            "UPSERT $m SET actor_id = $actor, origin_kind = $kind, origin_scope = $scope, \
             origin_id = $origin, publication_batch=$batch, created_at = $now RETURN NONE",
        )
        .bind(("m", rid("brain_actor_mention", mention_id)))
        .bind(("actor", actor_id.to_string()))
        .bind(("kind", origin_kind.to_string()))
        .bind(("scope", origin_scope.to_string()))
        .bind(("origin", origin_id.to_string()))
        .bind(("now", now.to_string()))
        .bind(("batch", batch.nonce.clone())),
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

/// Merge `from` into `to`. Aborts when it would make a cycle. A merge
/// touching a catalog-scope actor changes matcher inputs, so the cached
/// matcher epoch is bumped (repositories rebuild single-flight).
pub async fn add_redirect_in(tx: &Tx, from: &str, to: &str, now: &str) -> Result<(), TxError> {
    let map = redirects(tx).await?;
    if from == to || resolve(&map, to) == from {
        return Err(TxError::Abort(format!(
            "redirect {from} -> {to} makes a cycle"
        )));
    }
    bump_support_epoch_in(tx, &SupportBatch::new(tx)).await?;
    run(
        tx,
        tx.query("UPSERT $r SET to_actor = $to, created_at = $now RETURN NONE")
            .bind(("r", rid("brain_actor_redirect", from)))
            .bind(("to", to.to_string()))
            .bind(("now", now.to_string())),
    )
    .await?;
    let mut r = run(
        tx,
        tx.query("SELECT VALUE scope_key FROM brain_actor WHERE record::id(id) IN $ids AND scope_key = $scope")
            .bind((
                "ids",
                vec![from.to_string(), to.to_string()],
            ))
            .bind(("scope", crate::actor_catalog::SCOPE)),
    )
    .await?;
    if !take::<Vec<String>>(&mut r, 0)?.is_empty() {
        crate::actor_catalog::bump_catalog_epoch();
    }
    super::actor_search_outbox::enqueue_actors_in(
        tx,
        &[from.to_owned(), to.to_owned()],
        None,
        chrono::Utc::now(),
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
            "RETURN (SELECT VALUE scope_key FROM ONLY $a{i}) = 'dataset:argos_actors'
               OR count(SELECT id FROM brain_actor_mention WHERE actor_id = $i{i} LIMIT 1) > 0
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
            .bind((format!("a{i}"), rid("brain_actor", id)))
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
    if !affected.is_empty() {
        bump_support_epoch_in(tx, &SupportBatch::new(tx)).await?;
    }
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
        tx.query("SELECT record::id(id) AS id, scope_key FROM brain_actor WHERE scope_key != 'dataset:argos_actors'"),
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
DEFINE FIELD IF NOT EXISTS publication_batch ON atlas_article_index TYPE string DEFAULT '';
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
    pub publication_batch: String,
}

/// Receipts of one run (checksums are checked by the caller).
pub async fn article_index_receipts(
    db: &Surreal<Any>,
    run_id: &str,
) -> Result<Vec<ArticleIndexRow>, TxError> {
    let mut r = db
        .query(
            "SELECT article_id, input_revision, index_checksum, actor_coverage, (publication_batch ?? '') AS publication_batch \
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
    if run_id.is_empty()
        || article_id.is_empty()
        || row.publication_batch.is_empty()
        || row.publication_batch.len() > 256
    {
        return Err(TxError::Abort("atlas_article_index: empty id".into()));
    }
    run(
        tx,
        tx.query(
            "UPSERT type::record('atlas_article_index', $key) SET run_id = $run, \
             article_id = $article, input_revision = $rev, index_checksum = $sum, \
             actor_coverage = $cov,publication_batch=$publication, indexed_at = $now RETURN NONE",
        )
        .bind(("key", super::codec::encode(&[run_id, article_id])))
        .bind(("run", run_id.to_string()))
        .bind(("article", article_id.to_string()))
        .bind(("rev", row.input_revision.clone()))
        .bind(("sum", row.index_checksum.clone()))
        .bind(("cov", row.actor_coverage.clone()))
        .bind(("publication", row.publication_batch.clone()))
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
    retire_article_occurrence_in_batch(tx, run_id, article_id, &SupportBatch::new(tx)).await
}

/// Batched variant: share one support fence across a bounded publication.
pub async fn retire_article_occurrence_in_batch(
    tx: &Tx,
    run_id: &str,
    article_id: &str,
    batch: &SupportBatch,
) -> Result<Vec<String>, TxError> {
    bump_support_epoch_in(tx, batch).await?;
    if batch.capture.is_some() {
        register_mutation_in(
            tx,
            run_id,
            article_id,
            batch,
            MutationIntent::Delete {
                batch: batch.nonce.clone(),
            },
        )
        .await?;
        return Ok(Vec::new());
    }
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
    record_mention_in_batch(
        tx,
        actor_id,
        origin_kind,
        origin_scope,
        origin_id,
        origin_revision,
        spans_json,
        now,
        &SupportBatch::new(tx),
    )
    .await
}

/// Records a mention using the caller's logical support-mutation batch.
#[allow(clippy::too_many_arguments)]
pub async fn record_mention_in_batch(
    tx: &Tx,
    actor_id: &str,
    origin_kind: &str,
    origin_scope: &str,
    origin_id: &str,
    origin_revision: &str,
    spans_json: &str,
    now: &str,
    batch: &SupportBatch,
) -> Result<Vec<String>, TxError> {
    if actor_id.is_empty() {
        return Err(TxError::Abort("mention: empty actor id".into()));
    }
    bump_support_epoch_in(tx, batch).await?;
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
            "IF record::exists($m) { UPDATE $m SET actor_id = $actor, origin_kind = $kind, \
             origin_scope = $scope, origin_id = $origin, origin_revision = $rev, \
             spans_json = $spans, publication_batch=$batch RETURN NONE } \
             ELSE { CREATE $m SET actor_id = $actor, origin_kind = $kind, \
             origin_scope = $scope, origin_id = $origin, origin_revision = $rev, \
             spans_json = $spans, publication_batch=$batch, created_at = $now RETURN NONE };",
        )
        .bind(("m", rid("brain_actor_mention", &id)))
        .bind(("actor", canonical))
        .bind(("kind", origin_kind.to_string()))
        .bind(("scope", origin_scope.to_string()))
        .bind(("origin", origin_id.to_string()))
        .bind(("rev", origin_revision.to_string()))
        .bind(("spans", spans_json.to_string()))
        .bind(("batch", batch.nonce.clone()))
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

// ---- Bounded affected closure -------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReconcileGuards {
    sources: std::collections::BTreeMap<String, String>,
    fence: SupportFence,
    generation: i64,
    own_changes: bool,
    mutations: std::collections::BTreeMap<String, MutationIntent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum MutationIntent {
    Refresh {
        batch: String,
        source_revision: String,
    },
    Delete {
        batch: String,
    },
}

async fn captured_head_in(
    tx: &Tx,
    batch: &SupportBatch,
) -> Result<(String, ReconcileScope, ReconcileHead), TxError> {
    batch.check_owner(tx)?;
    let permit = batch
        .capture
        .as_ref()
        .ok_or_else(|| reconcile_error("capture_permit_required"))?;
    permit.check_owner(tx)?;
    let mut result = super::classifier_admission::tx_query(
        tx,
        "SELECT version,scope_hash,scope_json,phase,revision,guards_json FROM ONLY $row;",
    )
    .bind(("row", rid("lifecycle_reconcile", &permit.receipt_key)))
    .await?
    .check()?;
    let head = take::<Option<ReconcileHead>>(&mut result, 0)?
        .ok_or_else(|| reconcile_error("capture_state_missing"))?;
    if head.version != RECONCILE_VERSION
        || head.phase != "ready"
        || head.revision != permit.revision
    {
        return Err(reconcile_error("capture_permit_stale"));
    }
    let scope: ReconcileScope =
        serde_json::from_str(&head.scope_json).map_err(|_| reconcile_error("scope_corrupt"))?;
    Ok((permit.receipt_key.clone(), scope, head))
}

async fn register_mutation_in(
    tx: &Tx,
    run: &str,
    article: &str,
    batch: &SupportBatch,
    intent: MutationIntent,
) -> Result<(), TxError> {
    let (receipt, scope, mut head) = captured_head_in(tx, batch).await?;
    if !scope.keys.iter().any(|key|matches!(key,ReconcileKey::Source {run_id,article_id} if run_id==run && article_id==article)) {
        return Err(reconcile_error("mutation_outside_scope"));
    }
    let mut guards = head.guards()?;
    let key = super::codec::encode(&[run, article]);
    if guards.mutations.get(&key).is_some_and(|old| old != &intent) {
        return Err(reconcile_error("mutation_intent_conflict"));
    }
    guards.mutations.insert(key, intent);
    head.guards_json = reconcile_json(&guards)?;
    save_head_in(tx, &receipt, &head).await
}

#[derive(SurrealValue)]
struct MaterialSource {
    title: String,
    description: String,
    url: String,
    source_domain: String,
    source_name: String,
}

/// Registers replacement of ingest inventory. The captured old inventory is
/// removed later in bounded chunks; same-ID replacements carry a fresh stamp.
/// User and validation-bearing curated relationships are never blanket-retired.
pub async fn refresh_article_occurrence_in_batch(
    tx: &Tx,
    run: &str,
    article: &str,
    current_source_revision: &str,
    batch: &SupportBatch,
) -> Result<Vec<String>, TxError> {
    let _ = captured_head_in(tx, batch).await?;
    let mut result = super::classifier_admission::tx_query(
        tx,
        "SELECT title,description,url,source_domain,source_name FROM ONLY $article;",
    )
    .bind((
        "article",
        rid("atlas_article", &super::codec::encode(&[run, article])),
    ))
    .await?
    .check()?;
    let actual = take::<Option<MaterialSource>>(&mut result, 0)?
        .ok_or_else(|| reconcile_error("refresh_source_missing"))?;
    let input = crate::store::AtlasArticleRow {
        title: actual.title,
        description: actual.description,
        url: actual.url,
        source_domain: actual.source_domain,
        source_name: actual.source_name,
        ..Default::default()
    };
    if crate::atlas_metadata::input_hash(&input) != current_source_revision {
        return Err(reconcile_error("refresh_source_conflict"));
    }
    bump_support_epoch_in(tx, batch).await?;
    register_mutation_in(
        tx,
        run,
        article,
        batch,
        MutationIntent::Refresh {
            batch: batch.nonce.clone(),
            source_revision: current_source_revision.into(),
        },
    )
    .await?;
    Ok(Vec::new())
}

fn article_endpoint_matches(
    endpoint: &crate::memory_metadata::EndpointRef,
    run: &str,
    article: &str,
) -> bool {
    matches!(endpoint,crate::memory_metadata::EndpointRef::Article {run_id,article_id,..} if run_id==run && article_id==article)
}

fn ingest_owned_for(
    payload: &crate::memory_metadata::MemoryPayload,
    run: &str,
    article: &str,
) -> bool {
    use crate::memory_metadata::MemoryPayload;
    match payload {
        MemoryPayload::Link(link) => {
            link.validation.is_none()
                && link.relation == "mentioned_in"
                && article_endpoint_matches(&link.target, run, article)
                && link.receipts.iter().any(|receipt| {
                    receipt.starts_with(&format!("atlas_article_mentions:{run}/{article}:"))
                })
        }
        MemoryPayload::Related(related) => {
            (article_endpoint_matches(&related.source, run, article)
                || article_endpoint_matches(&related.target, run, article))
                && related
                    .source_revisions
                    .iter()
                    .any(|revision| revision.starts_with(&format!("ingest:{run}/{article}:")))
        }
        _ => false,
    }
}

#[derive(SurrealValue)]
struct IngestProtection {
    pinned: Option<bool>,
    source_json: Option<String>,
    payload_json: Option<String>,
    bounded: bool,
}

/// Whether an ingest producer may replace this memory. Existing manual,
/// confirmed, pinned and user-edited records are not ingest inventory.
pub async fn ingest_memory_writable_in(tx: &Tx, memory_id: &str) -> Result<bool, TxError> {
    let mut result=super::classifier_admission::tx_query(tx,
        "SELECT pinned,
           IF bytes::len(type::bytes(source_json ?? ''))<=16384 {source_json} ELSE {NONE} AS source_json,
           IF bytes::len(type::bytes(payload_json ?? ''))<=65536 {payload_json} ELSE {NONE} AS payload_json,
           (bytes::len(type::bytes(source_json ?? ''))<=16384 AND bytes::len(type::bytes(payload_json ?? ''))<=65536) AS bounded
           FROM ONLY $memory;
         RETURN record::exists($edit);",
    ).bind(("memory",rid("memory",memory_id))).bind(("edit",rid("insight_user_edit",memory_id))).await?.check()?;
    let memory = take::<Option<IngestProtection>>(&mut result, 0)?;
    if take::<Option<bool>>(&mut result, 1)? != Some(false) {
        return Ok(false);
    }
    let Some(memory) = memory else {
        return Ok(true);
    };
    if memory.pinned == Some(true) || !memory.bounded {
        return Ok(false);
    }
    if let Some(source) = memory
        .source_json
        .as_deref()
        .filter(|source| !source.is_empty())
    {
        let Ok(source) = serde_json::from_str::<serde_json::Value>(source) else {
            return Ok(false);
        };
        if source.get("app").and_then(|value| value.as_str()) != Some("atlas_ingest") {
            return Ok(false);
        }
    }
    let Some(payload) = memory
        .payload_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<crate::memory_metadata::MemoryPayload>(raw).ok())
    else {
        return Ok(false);
    };
    use crate::memory_metadata::MemoryPayload;
    Ok(match payload {
        MemoryPayload::Link(link) => {
            link.validation.is_none()
                && link.relation == "mentioned_in"
                && link
                    .receipts
                    .iter()
                    .any(|receipt| receipt.starts_with("atlas_article_mentions:"))
        }
        MemoryPayload::Related(related) => related
            .source_revisions
            .iter()
            .any(|revision| revision.starts_with("ingest:")),
        _ => false,
    })
}

/// Marks only typed ingest inventory created by a captured source refresh.
/// This operational stamp is outside payload/source/semantic/cache identity.
pub async fn mark_ingest_memory_in(
    tx: &Tx,
    memory_id: &str,
    batch: &SupportBatch,
) -> Result<(), TxError> {
    let (_, scope, head) = captured_head_in(tx, batch).await?;
    if !ingest_memory_writable_in(tx, memory_id).await? {
        return Err(reconcile_error("ingest_memory_protected"));
    }
    let guards = head.guards()?;
    let mut result=super::classifier_admission::tx_query(tx,
        "RETURN IF bytes::len(type::bytes((SELECT VALUE payload_json FROM ONLY $memory) ?? ''))<=65536
           {(SELECT VALUE payload_json FROM ONLY $memory)} ELSE {NONE};",
    ).bind(("memory",rid("memory",memory_id))).await?.check()?;
    let raw = take::<Option<String>>(&mut result, 0)?
        .ok_or_else(|| reconcile_error("ingest_payload_missing_or_large"))?;
    let payload: crate::memory_metadata::MemoryPayload =
        serde_json::from_str(&raw).map_err(|_| reconcile_error("ingest_payload_invalid"))?;
    let owned=scope.keys.iter().any(|key| {
        let ReconcileKey::Source {run_id,article_id}=key else{return false};
        matches!(guards.mutations.get(&super::codec::encode(&[run_id,article_id])),Some(MutationIntent::Refresh {batch:owner,..}) if owner==&batch.nonce)
            && ingest_owned_for(&payload,run_id,article_id)
    });
    if !owned {
        return Err(reconcile_error("ingest_stamp_outside_owned_refresh"));
    }
    super::classifier_admission::tx_query(
        tx,
        "UPDATE $memory SET lifecycle_ingest_batch=$batch RETURN NONE;",
    )
    .bind(("memory", rid("memory", memory_id)))
    .bind(("batch", batch.nonce.clone()))
    .await?
    .check()?;
    Ok(())
}

#[derive(SurrealValue)]
struct ReconcileHead {
    version: String,
    scope_hash: String,
    scope_json: String,
    phase: String,
    revision: i64,
    guards_json: String,
}

impl ReconcileHead {
    fn cursor(&self, receipt_key: &str) -> ReconcileCursor {
        ReconcileCursor {
            version: RECONCILE_VERSION.into(),
            receipt_key: receipt_key.into(),
            frontier_kind: self.phase.clone(),
            after_key: format!("{}:{}", self.scope_hash, self.revision),
        }
    }

    fn guards(&self) -> Result<ReconcileGuards, TxError> {
        serde_json::from_str(&self.guards_json).map_err(|_| reconcile_error("guards_corrupt"))
    }
}

fn reconcile_error(code: &str) -> TxError {
    TxError::Abort(format!("lifecycle_reconcile_{code}"))
}

fn reconcile_json<T: Serialize>(value: &T) -> Result<String, TxError> {
    let value = serde_json::to_string(value).map_err(|_| reconcile_error("serialization"))?;
    if value.len() > 2 * 1024 * 1024 {
        return Err(reconcile_error("payload_too_large"));
    }
    Ok(value)
}

fn normalized_scope(scope: &ReconcileScope) -> Result<(ReconcileScope, String, String), TxError> {
    if scope.keys.len() > RECONCILE_SCOPE_KEYS {
        return Err(reconcile_error("scope_too_large"));
    }
    for key in &scope.keys {
        let ids: Vec<&str> = match key {
            ReconcileKey::Source { run_id, article_id } => vec![run_id, article_id],
            ReconcileKey::Actor { actor_id } => vec![actor_id],
            ReconcileKey::Relationship { memory_id } => vec![memory_id],
        };
        if ids.iter().any(|id| id.is_empty()) {
            return Err(reconcile_error("empty_identity"));
        }
    }
    let scope = ReconcileScope {
        keys: scope
            .keys
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    };
    let json = reconcile_json(&scope)?;
    let hash = crate::classifier::artifact::sha256_hex(json.as_bytes());
    Ok((scope, json, hash))
}

async fn source_guards_in(
    tx: &Tx,
    scope: &ReconcileScope,
) -> Result<std::collections::BTreeMap<String, String>, TxError> {
    let mut result = std::collections::BTreeMap::new();
    // This is validation of the explicit <=512-key input, never a table scan.
    // Snapshot/owner headers also fence evidence regenerated without a title change.
    for key in &scope.keys {
        let ReconcileKey::Source { run_id, article_id } = key else {
            continue;
        };
        let occurrence = super::codec::encode(&[run_id, article_id]);
        let mut query = super::classifier_admission::tx_query(
            tx,
            "RETURN crypto::sha256(type::string([
               (SELECT input_revision, source_revision, classification_receipt_key, admission_state
                  FROM ONLY $article),
               (SELECT input_revision, index_checksum,publication_batch FROM ONLY $index),
               (SELECT input_hash, metadata_json FROM ONLY $metadata),
               (SELECT owner_kind, owner_id, desired_generation, active_snapshot_id, lifecycle
                  FROM insight_owner_state WITH INDEX insight_owner_scope
                  WHERE owner_kind = 'article' AND owner_id = $id LIMIT 2),
               (SELECT owner_kind, owner_id, desired_generation, active_snapshot_id, lifecycle
                  FROM insight_owner_state WITH INDEX insight_owner_scope
                  WHERE owner_kind = 'atlas' AND owner_id = $id LIMIT 2)
             ]));",
        )
        .bind(("article", rid("atlas_article", &occurrence)))
        .bind(("index", rid("atlas_article_index", &occurrence)))
        .bind(("metadata", rid("atlas_article_metadata", &occurrence)))
        .bind(("id", article_id.clone()))
        .await?
        .check()?;
        let hash = take::<Option<String>>(&mut query, 0)?
            .ok_or_else(|| reconcile_error("source_guard_missing"))?;
        result.insert(occurrence, hash);
    }
    Ok(result)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CapturedSource {
    key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    record_id: Option<RecordId>,
    fingerprint: String,
    call_id: String,
    answer_id: String,
    article_id: String,
    snapshot_owner: Option<String>,
}

impl CapturedSource {
    fn record_id(&self) -> RecordId {
        self.record_id
            .clone()
            .unwrap_or_else(|| rid("insight_source", &self.key))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CapturedEndpoint {
    key: String,
    kind: String,
    ref_id: String,
    origin: String,
    revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ReconcileWork {
    CaptureSource {
        run_id: String,
        article_id: String,
        part: u8,
        after: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_after: Option<RecordId>,
    },
    CaptureNode {
        node: String,
        side: u8,
        after: String,
    },
    CaptureSnapshot {
        snapshot: String,
        article_id: String,
        after: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_after: Option<RecordId>,
    },
    CaptureRelationship {
        memory_id: String,
        after: String,
    },
    CaptureClaim {
        fingerprint: String,
    },
    CaptureConcerns {
        memory_id: String,
        after: String,
    },
    DeleteMention {
        key: String,
        actor_id: String,
        run_id: String,
        article_id: String,
        old_batch: String,
    },
    Source {
        source: CapturedSource,
    },
    Claim {
        fingerprint: String,
        memory_id: String,
    },
    Actor {
        actor_id: String,
        current: String,
        root: Option<String>,
        supported: bool,
    },
    Member {
        actor_id: String,
        root: String,
        member: String,
        checked: bool,
        after: String,
    },
    ScanNode {
        node: String,
        side: u8,
        after: String,
    },
    Relationship {
        memory_id: String,
        edge: String,
        endpoints: Vec<CapturedEndpoint>,
        owner_revision: String,
        old_ingest_batch: String,
        cleanup_eligible: bool,
    },
    ReloadRelationship {
        memory_id: String,
        revision: String,
        after: String,
    },
    Owner {
        owner_kind: String,
        article_id: String,
        after: String,
    },
    OwnerLinks {
        snapshot: String,
        after: String,
    },
    Seen {
        member: String,
    },
}

impl ReconcileWork {
    fn identity(&self) -> String {
        let fields: Vec<&str> = match self {
            Self::CaptureSource {
                run_id, article_id, ..
            } => vec!["capture_source", run_id, article_id],
            Self::CaptureNode { node, .. } => vec!["capture_node", node],
            Self::CaptureSnapshot { snapshot, .. } => vec!["capture_snapshot", snapshot],
            Self::CaptureRelationship { memory_id, .. } => vec!["capture_relationship", memory_id],
            Self::CaptureClaim { fingerprint } => vec!["capture_claim", fingerprint],
            Self::CaptureConcerns { memory_id, .. } => vec!["capture_concerns", memory_id],
            Self::DeleteMention { key, .. } => vec!["delete_mention", key],
            Self::Source { source } => vec!["source", &source.key],
            Self::Claim { fingerprint, .. } => vec!["claim", fingerprint],
            Self::Actor { actor_id, .. } => vec!["actor", actor_id],
            Self::Member { member, .. } => vec!["member", member],
            Self::ScanNode { node, .. } => vec!["scan_node", node],
            Self::Relationship {
                memory_id,
                edge,
                owner_revision,
                cleanup_eligible,
                ..
            } => vec![
                if *cleanup_eligible {
                    "relationship_cleanup"
                } else {
                    "relationship"
                },
                memory_id,
                edge,
                owner_revision,
            ],
            Self::ReloadRelationship {
                memory_id,
                revision,
                ..
            } => vec!["reload_relationship", memory_id, revision],
            Self::Owner {
                owner_kind,
                article_id,
                ..
            } => vec!["owner", owner_kind, article_id],
            Self::OwnerLinks { snapshot, .. } => vec!["owner_links", snapshot],
            Self::Seen { member } => vec!["seen", member],
        };
        super::codec::encode(&fields)
    }

    fn priority(&self) -> i64 {
        match self {
            Self::CaptureSource { .. }
            | Self::CaptureSnapshot { .. }
            | Self::CaptureClaim { .. } => 0,
            Self::CaptureNode { .. }
            | Self::CaptureRelationship { .. }
            | Self::CaptureConcerns { .. } => 1,
            Self::Source { .. }
            | Self::Owner { .. }
            | Self::OwnerLinks { .. }
            | Self::DeleteMention { .. } => 0,
            Self::Claim { .. } => 1,
            Self::Actor { .. } | Self::Member { .. } => 2,
            Self::ScanNode { .. } => 3,
            Self::Relationship { .. } | Self::ReloadRelationship { .. } => 4,
            Self::Seen { .. } => 5,
        }
    }
}

#[derive(SurrealValue)]
struct FrontierRow {
    identity: String,
    payload_json: String,
    initial_json: String,
    generation: i64,
    priority: i64,
}

async fn put_frontier_in(
    tx: &Tx,
    receipt: &str,
    phase: &str,
    generation: i64,
    work: &ReconcileWork,
    refresh: bool,
) -> Result<(), TxError> {
    if generation < 0 {
        return Err(reconcile_error("negative_frontier_generation"));
    }

    let identity = work.identity();
    let json = reconcile_json(work)?;
    if json.len() > RECONCILE_ITEM_BYTES {
        return Err(reconcile_error("frontier_item_too_large"));
    }
    super::classifier_admission::tx_query(
        tx,
        "IF !record::exists($row) {
           CREATE $row SET receipt_key=$receipt, phase=$phase, identity=$identity,
             priority=$priority, payload_json=$json, initial_json=$json,
             generation=$generation, state='pending' RETURN NONE;
         } ELSE IF $refresh {
           UPDATE $row SET payload_json=initial_json, generation=$generation,
             state='pending' RETURN NONE;
         };",
    )
    .bind((
        "row",
        rid(
            "lifecycle_reconcile_frontier",
            &super::codec::encode(&[receipt, phase, &identity]),
        ),
    ))
    .bind(("receipt", receipt.to_string()))
    .bind(("phase", phase.to_string()))
    .bind(("identity", identity))
    .bind(("priority", work.priority()))
    .bind(("json", json))
    .bind(("generation", generation))
    .bind(("refresh", refresh))
    .await?
    .check()?;
    Ok(())
}

async fn next_frontier_in(
    tx: &Tx,
    receipt: &str,
    phase: &str,
    generation: i64,
) -> Result<Option<ReconcileWork>, TxError> {
    if generation < 0 {
        return Err(reconcile_error("negative_frontier_generation"));
    }
    let started = std::time::Instant::now();
    // Frontier writers only create nonnegative generations. At generation0,
    // no older-generation obligation can exist; retain all six current-work
    // probes and every source/support fence. Later generations keep the full
    // lazy-reset search. Each fixed priority prefix stays index-ordered.
    let statement_count = if generation == 0 { 6usize } else { 12usize };
    let mut sql = String::new();
    for priority in 0..=5 {
        sql.push_str(&format!(
            "SELECT identity,
                 IF bytes::len(type::bytes(payload_json))<=131072 {{payload_json}} ELSE {{NONE}} AS payload_json,
                 IF bytes::len(type::bytes(initial_json))<=131072 {{initial_json}} ELSE {{NONE}} AS initial_json,
                 generation,priority
               FROM lifecycle_reconcile_frontier WITH INDEX lifecycle_reconcile_pending
               WHERE receipt_key=$receipt AND phase=$phase AND priority={priority}
                 AND state='pending' AND generation=$generation ORDER BY identity LIMIT 1;"
        ));
        if generation > 0 {
            sql.push_str(&format!(
                "SELECT identity,
                 IF bytes::len(type::bytes(payload_json))<=131072 {{payload_json}} ELSE {{NONE}} AS payload_json,
                 IF bytes::len(type::bytes(initial_json))<=131072 {{initial_json}} ELSE {{NONE}} AS initial_json,
                 generation,priority
               FROM lifecycle_reconcile_frontier WITH INDEX lifecycle_reconcile_generation
               WHERE receipt_key=$receipt AND phase=$phase AND priority={priority}
                 AND generation<$generation ORDER BY generation,identity LIMIT 1;"
            ));
        }
    }
    let mut result = super::classifier_admission::tx_query(tx, sql)
        .bind(("receipt", receipt.to_string()))
        .bind(("phase", phase.to_string()))
        .bind(("generation", generation))
        .await?
        .check()?;
    let mut rows = Vec::new();
    for index in 0..statement_count {
        rows.extend(take::<Vec<FrontierRow>>(&mut result, index)?);
    }
    let mut decoded = Vec::new();
    for row in rows {
        let json = if row.generation == generation {
            row.payload_json
        } else {
            row.initial_json
        };
        let work: ReconcileWork =
            serde_json::from_str(&json).map_err(|_| reconcile_error("frontier_corrupt"))?;
        if work.identity() != row.identity || work.priority() != row.priority {
            return Err(reconcile_error("frontier_identity_mismatch"));
        }
        decoded.push(work);
    }
    decoded.sort_by_key(|work| (work.priority(), work.identity()));
    tracing::debug!(target:"argos::atlas_derivatives",statement_count=statement_count as u64,query_us=started.elapsed().as_micros() as u64,candidates=decoded.len(),"reconcile_frontier_probe");
    Ok(decoded.into_iter().next())
}

async fn finish_frontier_in(
    tx: &Tx,
    receipt: &str,
    phase: &str,
    generation: i64,
    work: &ReconcileWork,
    done: bool,
) -> Result<(), TxError> {
    if generation < 0 {
        return Err(reconcile_error("negative_frontier_generation"));
    }

    let json = reconcile_json(work)?;
    if json.len() > RECONCILE_ITEM_BYTES {
        return Err(reconcile_error("frontier_item_too_large"));
    }
    super::classifier_admission::tx_query(
        tx,
        "UPDATE $row SET payload_json=$json,generation=$generation,state=$state RETURN NONE;",
    )
    .bind((
        "row",
        rid(
            "lifecycle_reconcile_frontier",
            &super::codec::encode(&[receipt, phase, &work.identity()]),
        ),
    ))
    .bind(("json", json))
    .bind(("generation", generation))
    .bind(("state", if done { "done" } else { "pending" }.to_string()))
    .await?
    .check()?;
    Ok(())
}

async fn save_head_in(tx: &Tx, receipt: &str, head: &ReconcileHead) -> Result<(), TxError> {
    super::classifier_admission::tx_query(
        tx,
        "UPSERT $row SET version=$version,scope_hash=$hash,scope_json=$scope,
           phase=$phase,revision=$revision,guards_json=$guards RETURN NONE;",
    )
    .bind(("row", rid("lifecycle_reconcile", receipt)))
    .bind(("version", head.version.clone()))
    .bind(("hash", head.scope_hash.clone()))
    .bind(("scope", head.scope_json.clone()))
    .bind(("phase", head.phase.clone()))
    .bind(("revision", head.revision))
    .bind(("guards", head.guards_json.clone()))
    .await?
    .check()?;
    Ok(())
}

async fn load_head_in(
    tx: &Tx,
    scope: &ReconcileScope,
    cursor: Option<&ReconcileCursor>,
) -> Result<(String, ReconcileScope, ReconcileHead), TxError> {
    let (scope, scope_json, scope_hash) = normalized_scope(scope)?;
    if cursor.is_some_and(|cursor| {
        cursor.version != RECONCILE_VERSION
            || cursor.after_key.len() > 128
            || !matches!(
                cursor.frontier_kind.as_str(),
                "capture" | "ready" | "reconcile" | "reconcile_restart" | "done"
            )
    }) {
        return Err(reconcile_error("cursor_version_or_kind"));
    }
    let receipt = cursor.map_or_else(crate::store::new_id, |cursor| cursor.receipt_key.clone());
    if receipt.is_empty() || receipt.len() > 256 || receipt.chars().any(char::is_control) {
        return Err(reconcile_error("invalid_receipt_key"));
    }
    let mut result = super::classifier_admission::tx_query(
        tx,
        "SELECT version,scope_hash,scope_json,phase,revision,guards_json FROM ONLY $row;",
    )
    .bind(("row", rid("lifecycle_reconcile", &receipt)))
    .await?
    .check()?;
    if let Some(head) = take::<Option<ReconcileHead>>(&mut result, 0)? {
        if head.revision < 0 || head.guards()?.generation < 0 {
            return Err(reconcile_error("revision_corrupt"));
        }
        if head.version != RECONCILE_VERSION
            || head.scope_hash != scope_hash
            || head.scope_json != scope_json
        {
            return Err(reconcile_error("scope_or_version_mismatch"));
        }
        if let Some(cursor) = cursor {
            if *cursor != head.cursor(&receipt) {
                return Err(reconcile_error("stale_cursor"));
            }
        }
        return Ok((receipt, scope, head));
    }
    let head = ReconcileHead {
        version: RECONCILE_VERSION.into(),
        scope_hash,
        scope_json,
        phase: "capture".into(),
        revision: 0,
        guards_json: reconcile_json(&ReconcileGuards {
            sources: source_guards_in(tx, &scope).await?,
            fence: support_fence_in(tx).await?,
            generation: 0,
            own_changes: false,
            mutations: std::collections::BTreeMap::new(),
        })?,
    };
    if let Some(cursor) = cursor {
        if *cursor != head.cursor(&receipt) {
            return Err(reconcile_error("missing_cursor"));
        }
    }
    save_head_in(tx, &receipt, &head).await?;
    for key in &scope.keys {
        let (phase, work) = match key {
            ReconcileKey::Source { run_id, article_id } => (
                "capture",
                ReconcileWork::CaptureSource {
                    run_id: run_id.clone(),
                    article_id: article_id.clone(),
                    part: 0,
                    after: String::new(),
                    source_after: None,
                },
            ),
            ReconcileKey::Actor { actor_id } => (
                "reconcile",
                ReconcileWork::Actor {
                    actor_id: actor_id.clone(),
                    current: actor_id.clone(),
                    root: None,
                    supported: false,
                },
            ),
            ReconcileKey::Relationship { memory_id } => (
                "capture",
                ReconcileWork::CaptureRelationship {
                    memory_id: memory_id.clone(),
                    after: String::new(),
                },
            ),
        };
        put_frontier_in(tx, &receipt, phase, 0, &work, false).await?;
        if let ReconcileKey::Relationship { memory_id } = key {
            put_frontier_in(
                tx,
                &receipt,
                "reconcile",
                0,
                &ReconcileWork::ScanNode {
                    node: memory_node(memory_id),
                    side: 0,
                    after: String::new(),
                },
                false,
            )
            .await?;
        }
    }
    Ok((receipt, scope, head))
}

fn page_capacity(remaining: usize, cost_per_row: usize) -> usize {
    ((remaining.saturating_sub(1)) / cost_per_row)
        .saturating_sub(1)
        .max(1)
}

#[derive(SurrealValue)]
struct MentionCaptureRow {
    key: String,
    actor_id: String,
    publication_batch: Option<String>,
}
#[derive(SurrealValue)]
struct KeyCaptureRow {
    key: String,
}
#[derive(SurrealValue)]
struct SourceCaptureRow {
    id: RecordId,
    fingerprint: String,
    call_id: String,
    answer_id: String,
    snapshot_owner: Option<String>,
}
#[derive(SurrealValue)]
struct EdgeCaptureRow {
    key: String,
    owner: String,
    from_key: String,
    to_key: String,
    from_kind: Option<String>,
    to_kind: Option<String>,
    from_ref: Option<String>,
    to_ref: Option<String>,
    from_origin: Option<String>,
    to_origin: Option<String>,
    from_revision: Option<String>,
    to_revision: Option<String>,
    owner_revision: Option<String>,
    owner_batch: Option<String>,
}

const CAPTURE_EDGE_FIELDS:&str="id,record::id(id) AS key,owner,
    record::id(in) AS from_key,record::id(out) AS to_key,
    in.kind AS from_kind,out.kind AS to_kind,in.ref_id AS from_ref,out.ref_id AS to_ref,
    in.origin AS from_origin,out.origin AS to_origin,in.revision AS from_revision,out.revision AS to_revision,
    (SELECT VALUE crypto::sha256(type::string([text,payload_json,memory_kind])) FROM ONLY type::record('memory',$parent.owner)) AS owner_revision,
    (SELECT VALUE lifecycle_ingest_batch FROM ONLY type::record('memory',$parent.owner)) AS owner_batch";

async fn capture_edges_in(
    tx: &Tx,
    receipt: &str,
    generation: i64,
    node: Option<&str>,
    owner: Option<&str>,
    side: u8,
    after: &str,
    remaining: usize,
    cleanup_eligible: bool,
) -> Result<(usize, bool, String), TxError> {
    let capacity = page_capacity(remaining, 4);
    let (predicate, index) = if owner.is_some() {
        ("owner=$owner", "brain_graph_edge_owner")
    } else if side == 0 {
        ("in=$node", "brain_graph_edge_input")
    } else if side == 1 {
        ("out=$node", "brain_graph_edge_output")
    } else {
        return Err(reconcile_error("edge_side_corrupt"));
    };
    let sql = format!(
        "SELECT {CAPTURE_EDGE_FIELDS} FROM brain_graph_edge WITH INDEX {index}
        WHERE {predicate} AND meaning INSIDE ['link','related'] AND ($first OR id>$after)
        ORDER BY id LIMIT $limit;"
    );
    let mut result = super::classifier_admission::tx_query(tx, sql)
        .bind(("owner", owner.unwrap_or_default().to_string()))
        .bind(("node", rid("brain_graph_node", node.unwrap_or("_"))))
        .bind(("first", after.is_empty()))
        .bind((
            "after",
            rid(
                "brain_graph_edge",
                if after.is_empty() { "_" } else { after },
            ),
        ))
        .bind(("limit", capacity + 1))
        .await?
        .check()?;
    let mut rows = take::<Vec<EdgeCaptureRow>>(&mut result, 0)?;
    let cost = 1 + rows.len() * 4;
    let more = rows.len() > capacity;
    rows.truncate(capacity);
    let mut last = after.to_string();
    for row in rows {
        if row.owner.is_empty() {
            return Err(reconcile_error("edge_owner_missing"));
        }
        last = row.key.clone();
        let endpoints = vec![
            CapturedEndpoint {
                key: row.from_key,
                kind: row.from_kind.unwrap_or_default(),
                ref_id: row.from_ref.unwrap_or_default(),
                origin: row.from_origin.unwrap_or_default(),
                revision: row.from_revision.unwrap_or_default(),
            },
            CapturedEndpoint {
                key: row.to_key,
                kind: row.to_kind.unwrap_or_default(),
                ref_id: row.to_ref.unwrap_or_default(),
                origin: row.to_origin.unwrap_or_default(),
                revision: row.to_revision.unwrap_or_default(),
            },
        ];
        put_frontier_in(
            tx,
            receipt,
            "reconcile",
            generation,
            &ReconcileWork::Relationship {
                memory_id: row.owner,
                edge: row.key,
                endpoints,
                owner_revision: row.owner_revision.unwrap_or_default(),
                old_ingest_batch: row.owner_batch.unwrap_or_default(),
                cleanup_eligible,
            },
            !cleanup_eligible,
        )
        .await?;
    }
    Ok((cost, more, last))
}

async fn capture_sources_page_in(
    tx: &Tx,
    receipt: &str,
    article: &str,
    call: Option<&str>,
    snapshot: Option<&str>,
    position: (&str, Option<&RecordId>),
    remaining: usize,
) -> Result<(usize, bool, String, Option<RecordId>), TxError> {
    let (after, source_after) = position;
    // A call lookup also inspects the exact snapshot header; charge both identities.
    let weight = 4;
    let capacity = page_capacity(remaining, weight);
    let (predicate, index) = if snapshot.is_some() {
        ("answer_id=$snapshot", "insight_source_answer_run")
    } else {
        ("call_id=$call", "insight_source_article_call")
    };
    let sql = format!(
        "SELECT id,fingerprint,call_id,answer_id,
          (SELECT VALUE owner_id FROM ONLY type::record('insight_snapshot',$parent.answer_id)
             WHERE owner_kind INSIDE ['article','atlas']) AS snapshot_owner
        FROM insight_source WITH INDEX {index} WHERE {predicate} AND thread_id=NONE
          AND ($first OR id>$after) ORDER BY id LIMIT $limit;"
    );
    let mut result = super::classifier_admission::tx_query(tx, sql)
        .bind(("snapshot", snapshot.unwrap_or_default().to_string()))
        .bind(("call", call.unwrap_or_default().to_string()))
        .bind(("first", after.is_empty() && source_after.is_none()))
        .bind((
            "after",
            source_after.cloned().unwrap_or_else(|| {
                rid("insight_source", if after.is_empty() { "_" } else { after })
            }),
        ))
        .bind(("limit", capacity + 1))
        .await?
        .check()?;
    let mut rows = take::<Vec<SourceCaptureRow>>(&mut result, 0)?;
    let cost = 1 + rows.len() * weight;
    let more = rows.len() > capacity;
    rows.truncate(capacity);
    let mut last = after.to_string();
    let mut last_record = source_after.cloned();
    for row in rows {
        if row.id.table.as_str() != "insight_source" {
            return Err(reconcile_error("source_record_table"));
        }
        // JSON preserves key type and escaping for the frontier identity;
        // continuation comparisons bind the typed ID, never this display key.
        last = serde_json::to_string(&row.id).map_err(|_| reconcile_error("source_record_json"))?;
        last_record = Some(row.id.clone());
        let known_owner = snapshot.map(|_| article.to_string()).or(row.snapshot_owner);
        let source = CapturedSource {
            key: last.clone(),
            record_id: Some(row.id),
            fingerprint: row.fingerprint.clone(),
            call_id: row.call_id,
            answer_id: row.answer_id,
            article_id: article.to_string(),
            snapshot_owner: known_owner,
        };
        put_frontier_in(
            tx,
            receipt,
            "reconcile",
            0,
            &ReconcileWork::Source { source },
            false,
        )
        .await?;
        put_frontier_in(
            tx,
            receipt,
            "capture",
            0,
            &ReconcileWork::CaptureClaim {
                fingerprint: row.fingerprint,
            },
            false,
        )
        .await?;
    }
    Ok((cost, more, last, last_record))
}

async fn capture_step_in(
    tx: &Tx,
    receipt: &str,
    work: &mut ReconcileWork,
    remaining: usize,
) -> Result<(bool, usize), TxError> {
    match work.clone() {
        ReconcileWork::CaptureSource {
            run_id,
            article_id,
            part,
            after,
            mut source_after,
        } => {
            let (cost, more, last) = match part {
                0 => {
                    // One old mention plus its deletion and affected actor
                    // descriptors are three identities of bounded work.
                    let capacity = page_capacity(remaining, 3);
                    let mut result=super::classifier_admission::tx_query(tx,
                        "SELECT id,record::id(id) AS key,actor_id,publication_batch FROM brain_actor_mention WITH INDEX brain_actor_mention_origin
                           WHERE origin_kind='article' AND origin_scope=$run AND origin_id=$article
                             AND ($first OR id>$after) ORDER BY id LIMIT $limit;",
                    ).bind(("run",run_id.clone())).bind(("article",article_id.clone()))
                    .bind(("first",after.is_empty())).bind(("after",rid("brain_actor_mention",if after.is_empty(){"_"}else{&after})))
                    .bind(("limit",capacity+1)).await?.check()?;
                    let mut rows = take::<Vec<MentionCaptureRow>>(&mut result, 0)?;
                    let cost = 1 + rows.len() * 3;
                    let more = rows.len() > capacity;
                    rows.truncate(capacity);
                    let mut last = after.clone();
                    for row in rows {
                        if row.actor_id.is_empty() {
                            return Err(reconcile_error("mention_actor_missing"));
                        }
                        last = row.key.clone();
                        put_frontier_in(
                            tx,
                            receipt,
                            "reconcile",
                            0,
                            &ReconcileWork::DeleteMention {
                                key: row.key,
                                actor_id: row.actor_id.clone(),
                                run_id: run_id.clone(),
                                article_id: article_id.clone(),
                                old_batch: row.publication_batch.unwrap_or_default(),
                            },
                            false,
                        )
                        .await?;
                        put_frontier_in(
                            tx,
                            receipt,
                            "reconcile",
                            0,
                            &ReconcileWork::Actor {
                                current: row.actor_id.clone(),
                                actor_id: row.actor_id,
                                root: None,
                                supported: false,
                            },
                            false,
                        )
                        .await?;
                    }
                    (cost, more, last)
                }
                1 => {
                    let capacity = page_capacity(remaining, 2);
                    let mut result=super::classifier_admission::tx_query(tx,
                        "SELECT id,record::id(id) AS key FROM brain_graph_node WITH INDEX brain_graph_node_occurrence
                           WHERE kind='article' AND origin=$run AND ref_id=$article
                             AND ($first OR id>$after) ORDER BY id LIMIT $limit;",
                    ).bind(("run",run_id.clone())).bind(("article",article_id.clone()))
                    .bind(("first",after.is_empty())).bind(("after",rid("brain_graph_node",if after.is_empty(){"_"}else{&after})))
                    .bind(("limit",capacity+1)).await?.check()?;
                    let mut rows = take::<Vec<KeyCaptureRow>>(&mut result, 0)?;
                    let cost = 1 + rows.len() * 2;
                    let more = rows.len() > capacity;
                    rows.truncate(capacity);
                    let mut last = after.clone();
                    for row in rows {
                        last = row.key.clone();
                        put_frontier_in(
                            tx,
                            receipt,
                            "capture",
                            0,
                            &ReconcileWork::CaptureNode {
                                node: row.key.clone(),
                                side: 0,
                                after: String::new(),
                            },
                            false,
                        )
                        .await?;
                        put_frontier_in(
                            tx,
                            receipt,
                            "reconcile",
                            0,
                            &ReconcileWork::ScanNode {
                                node: row.key,
                                side: 0,
                                after: String::new(),
                            },
                            false,
                        )
                        .await?;
                    }
                    (cost, more, last)
                }
                2 | 3 => {
                    let call = if part == 2 {
                        article_id.clone()
                    } else {
                        format!("article:{article_id}")
                    };
                    let (cost, more, last, last_record) = capture_sources_page_in(
                        tx,
                        receipt,
                        &article_id,
                        Some(&call),
                        None,
                        (&after, source_after.as_ref()),
                        remaining,
                    )
                    .await?;
                    source_after = last_record;
                    (cost, more, last)
                }
                4 | 5 => {
                    let kind = if part == 4 { "article" } else { "atlas" };
                    let capacity = page_capacity(remaining, 2);
                    let mut result=super::classifier_admission::tx_query(tx,
                        "SELECT id,record::id(id) AS key FROM insight_snapshot WITH INDEX insight_snapshot_owner_scope
                           WHERE owner_kind=$kind AND owner_id=$article AND ($first OR id>$after)
                           ORDER BY id LIMIT $limit;",
                    ).bind(("kind",kind.to_string())).bind(("article",article_id.clone()))
                    .bind(("first",after.is_empty())).bind(("after",rid("insight_snapshot",if after.is_empty(){"_"}else{&after})))
                    .bind(("limit",capacity+1)).await?.check()?;
                    let mut rows = take::<Vec<KeyCaptureRow>>(&mut result, 0)?;
                    let cost = 1 + rows.len() * 2;
                    let more = rows.len() > capacity;
                    rows.truncate(capacity);
                    let mut last = after.clone();
                    for row in rows {
                        last = row.key.clone();
                        put_frontier_in(
                            tx,
                            receipt,
                            "capture",
                            0,
                            &ReconcileWork::CaptureSnapshot {
                                snapshot: row.key,
                                article_id: article_id.clone(),
                                after: String::new(),
                                source_after: None,
                            },
                            false,
                        )
                        .await?;
                    }
                    put_frontier_in(
                        tx,
                        receipt,
                        "reconcile",
                        0,
                        &ReconcileWork::Owner {
                            owner_kind: kind.into(),
                            article_id: article_id.clone(),
                            after: String::new(),
                        },
                        false,
                    )
                    .await?;
                    (cost, more, last)
                }
                _ => return Err(reconcile_error("source_capture_part_corrupt")),
            };
            *work = ReconcileWork::CaptureSource {
                run_id,
                article_id,
                part: if more { part } else { part + 1 },
                after: if more { last } else { String::new() },
                source_after: if more { source_after } else { None },
            };
            Ok((!more && part == 5, cost))
        }
        ReconcileWork::CaptureNode { node, side, after } => {
            let (cost, more, last) = capture_edges_in(
                tx,
                receipt,
                0,
                Some(&node),
                None,
                side,
                &after,
                remaining,
                true,
            )
            .await?;
            *work = ReconcileWork::CaptureNode {
                node,
                side: if more { side } else { side + 1 },
                after: if more { last } else { String::new() },
            };
            Ok((!more && side == 1, cost))
        }
        ReconcileWork::CaptureSnapshot {
            snapshot,
            article_id,
            after,
            source_after,
        } => {
            let (cost, more, last, last_record) = capture_sources_page_in(
                tx,
                receipt,
                &article_id,
                None,
                Some(&snapshot),
                (&after, source_after.as_ref()),
                remaining,
            )
            .await?;
            *work = ReconcileWork::CaptureSnapshot {
                snapshot,
                article_id,
                after: last,
                source_after: last_record,
            };
            Ok((!more, cost))
        }
        ReconcileWork::CaptureRelationship { memory_id, after } => {
            let (cost, more, last) = capture_edges_in(
                tx,
                receipt,
                0,
                None,
                Some(&memory_id),
                0,
                &after,
                remaining,
                true,
            )
            .await?;
            *work = ReconcileWork::CaptureRelationship {
                memory_id,
                after: last,
            };
            Ok((!more, cost))
        }
        ReconcileWork::CaptureClaim { fingerprint } => {
            let mut result = super::classifier_admission::tx_query(
                tx,
                "SELECT VALUE memory_id FROM ONLY $claim;",
            )
            .bind(("claim", rid("insight_claim", &fingerprint)))
            .await?
            .check()?;
            if let Some(memory_id) = take::<Option<String>>(&mut result, 0)? {
                put_frontier_in(
                    tx,
                    receipt,
                    "reconcile",
                    0,
                    &ReconcileWork::Claim {
                        fingerprint,
                        memory_id: memory_id.clone(),
                    },
                    false,
                )
                .await?;
                put_frontier_in(
                    tx,
                    receipt,
                    "capture",
                    0,
                    &ReconcileWork::CaptureNode {
                        node: memory_node(&memory_id),
                        side: 0,
                        after: String::new(),
                    },
                    false,
                )
                .await?;
                put_frontier_in(
                    tx,
                    receipt,
                    "capture",
                    0,
                    &ReconcileWork::CaptureConcerns {
                        memory_id,
                        after: String::new(),
                    },
                    false,
                )
                .await?;
            }
            Ok((true, 4))
        }
        ReconcileWork::CaptureConcerns { memory_id, after } => {
            let capacity = page_capacity(remaining, 2);
            let mut result=super::classifier_admission::tx_query(tx,
                "SELECT id,record::id(id) AS key,out.ref_id AS actor_id FROM brain_graph_edge WITH INDEX brain_graph_edge_owner
                   WHERE owner=$memory AND meaning='concerns' AND out.kind='memory' AND out.memory_kind='actor'
                     AND ($first OR id>$after) ORDER BY id LIMIT $limit;",
            ).bind(("memory",memory_id.clone())).bind(("first",after.is_empty()))
            .bind(("after",rid("brain_graph_edge",if after.is_empty(){"_"}else{&after})))
            .bind(("limit",capacity+1)).await?.check()?;
            let mut rows = take::<Vec<MentionCaptureRow>>(&mut result, 0)?;
            let cost = 1 + rows.len() * 2;
            let more = rows.len() > capacity;
            rows.truncate(capacity);
            let mut last = after;
            for row in rows {
                last = row.key;
                put_frontier_in(
                    tx,
                    receipt,
                    "reconcile",
                    0,
                    &ReconcileWork::Actor {
                        actor_id: row.actor_id.clone(),
                        current: row.actor_id,
                        root: None,
                        supported: false,
                    },
                    false,
                )
                .await?;
            }
            *work = ReconcileWork::CaptureConcerns {
                memory_id,
                after: last,
            };
            Ok((!more, cost))
        }
        _ => Err(reconcile_error("capture_work_kind_corrupt")),
    }
}

/// Captures reverse evidence in bounded pages before the caller changes it.
/// Commit only the continuation while `ready` is false. A ready result is
/// usable for mutation only in this exact transaction attempt.
pub async fn capture_affected_in(
    tx: &Tx,
    scope: &ReconcileScope,
    cursor: Option<&ReconcileCursor>,
) -> Result<ReconcileCapture, TxError> {
    let (receipt, scope, mut head) = load_head_in(tx, scope, cursor).await?;
    if !matches!(head.phase.as_str(), "capture" | "ready") {
        return Err(reconcile_error("capture_phase_mismatch"));
    }
    let guards = head.guards()?;
    if source_guards_in(tx, &scope).await? != guards.sources
        || support_fence_in(tx).await? != guards.fence
    {
        return Err(reconcile_error("capture_conflict_restart_required"));
    }
    let mut examined = 0;
    while examined + 16 <= RECONCILE_STEP_IDENTITIES && head.phase == "capture" {
        let Some(mut work) = next_frontier_in(tx, &receipt, "capture", 0).await? else {
            head.phase = "ready".into();
            break;
        };
        let (done, cost) = capture_step_in(
            tx,
            &receipt,
            &mut work,
            RECONCILE_STEP_IDENTITIES - examined,
        )
        .await?;
        if cost == 0 || cost > RECONCILE_STEP_IDENTITIES - examined {
            return Err(reconcile_error("capture_budget_violation"));
        }
        examined += cost;
        finish_frontier_in(tx, &receipt, "capture", 0, &work, done).await?;
    }
    if head.phase == "capture"
        && next_frontier_in(tx, &receipt, "capture", 0)
            .await?
            .is_none()
    {
        head.phase = "ready".into();
    }
    head.revision = head
        .revision
        .checked_add(1)
        .ok_or_else(|| reconcile_error("revision_exhausted"))?;
    save_head_in(tx, &receipt, &head).await?;
    let ready = head.phase == "ready";
    Ok(ReconcileCapture {
        ready,
        cursor: head.cursor(&receipt),
        examined: examined as u64,
        permit: ready.then(|| CapturePermit {
            owner: Arc::downgrade(tx),
            receipt_key: receipt,
            revision: head.revision,
            examined,
        }),
    })
}

#[derive(SurrealValue)]
struct RetainedOccurrence {
    run_id: String,
}

async fn latest_retained_in(tx: &Tx, article: &str) -> Result<Option<String>, TxError> {
    let mut result = super::classifier_admission::tx_query(
        tx,
        "SELECT run_id,seen_at FROM atlas_article WITH INDEX atlas_article_id
           WHERE article_id=$article ORDER BY seen_at DESC,run_id LIMIT 1;",
    )
    .bind(("article", article.to_string()))
    .await?
    .check()?;
    Ok(take::<Vec<RetainedOccurrence>>(&mut result, 0)?
        .into_iter()
        .next()
        .map(|row| row.run_id))
}

#[derive(SurrealValue)]
struct LiveSource {
    fingerprint: String,
    call_id: String,
    answer_id: String,
    run_id: Option<String>,
    thread_id: Option<String>,
}

async fn reconcile_source_row_in(tx: &Tx, source: &CapturedSource) -> Result<(), TxError> {
    let source_id = source.record_id();
    let mut result = super::classifier_admission::tx_query(
        tx,
        "SELECT fingerprint,call_id,answer_id,run_id,thread_id FROM ONLY $source;",
    )
    .bind(("source", source_id.clone()))
    .await?
    .check()?;
    let Some(actual) = take::<Option<LiveSource>>(&mut result, 0)? else {
        return Ok(());
    };
    if actual.fingerprint != source.fingerprint
        || actual.call_id != source.call_id
        || actual.answer_id != source.answer_id
        || actual.thread_id.is_some()
    {
        return Err(reconcile_error("source_identity_conflict"));
    }
    let article_backed = actual.call_id.starts_with("article:")
        || actual.answer_id.starts_with("atlas-")
        || source.snapshot_owner.is_some();
    if !article_backed {
        return Ok(());
    }
    let latest = latest_retained_in(tx, &source.article_id).await?;
    let owner_gone = if let Some(owner) = &source.snapshot_owner {
        latest_retained_in(tx, owner).await?.is_none()
    } else {
        false
    };
    if owner_gone || latest.is_none() {
        super::classifier_admission::tx_query(tx, "DELETE $source RETURN NONE;")
            .bind(("source", source_id.clone()))
            .await?
            .check()?;
        return Ok(());
    }
    if let Some(run) = &actual.run_id {
        let mut result =
            super::classifier_admission::tx_query(tx, "RETURN record::exists($article);")
                .bind((
                    "article",
                    rid(
                        "atlas_article",
                        &super::codec::encode(&[run, &source.article_id]),
                    ),
                ))
                .await?
                .check()?;
        if take::<Option<bool>>(&mut result, 0)? == Some(true) {
            return Ok(());
        }
    }
    let run = latest.ok_or_else(|| reconcile_error("retained_occurrence_missing"))?;
    let answer = if actual.answer_id.starts_with("atlas-") {
        format!("atlas-{run}")
    } else {
        actual.answer_id.clone()
    };
    let target = super::codec::encode(&[&actual.fingerprint, &answer, &actual.call_id]);
    // Snapshot answer IDs are retained. If only run_id changes, the key may
    // already be the current row and must never be deleted as a collision.
    if answer == actual.answer_id || rid("insight_source", &target) == source_id {
        super::classifier_admission::tx_query(
            tx,
            "UPDATE $source SET run_id=$run,answer_id=$answer RETURN NONE;",
        )
        .bind(("source", source_id.clone()))
        .bind(("run", run))
        .bind(("answer", answer))
        .await?
        .check()?;
        return Ok(());
    }
    let mut result = super::classifier_admission::tx_query(
        tx,
        "SELECT fingerprint,call_id,answer_id,run_id,thread_id FROM ONLY $target;",
    )
    .bind(("target", rid("insight_source", &target)))
    .await?
    .check()?;
    if let Some(existing) = take::<Option<LiveSource>>(&mut result, 0)? {
        if existing.fingerprint != actual.fingerprint
            || existing.call_id != actual.call_id
            || existing.answer_id != answer
            || existing.run_id.as_deref() != Some(run.as_str())
            || existing.thread_id.is_some()
        {
            return Err(reconcile_error("source_target_collision"));
        }
        // The equivalent winning target remains byte-identical; only the
        // redundant origin is removed, including the old collision case.
        super::classifier_admission::tx_query(tx, "DELETE $source RETURN NONE;")
            .bind(("source", source_id.clone()))
            .await?
            .check()?;
    } else {
        super::classifier_admission::tx_query(tx,
            "LET $old=(SELECT * FROM ONLY $source);
             CREATE $target CONTENT object::extend(object::remove($old,'id'),{run_id:$run,answer_id:$answer}) RETURN NONE;
             DELETE $source RETURN NONE;",
        ).bind(("source",source_id)).bind(("target",rid("insight_source",&target)))
        .bind(("run",run)).bind(("answer",answer)).await?.check()?;
    }
    Ok(())
}

#[derive(SurrealValue)]
struct ClaimHead {
    lifecycle: String,
    memory_kind: String,
    source_json: Option<String>,
}

async fn reconcile_claim_in(
    tx: &Tx,
    receipt: &str,
    generation: i64,
    fingerprint: &str,
    memory_id: &str,
) -> Result<Vec<String>, TxError> {
    let mut result = super::classifier_admission::tx_query(
        tx,
        "SELECT VALUE id FROM insight_source WITH INDEX insight_source_fingerprint
           WHERE fingerprint=$fingerprint AND deleted_origin=false LIMIT 1;
         SELECT lifecycle,memory_kind,source_json FROM ONLY $memory;",
    )
    .bind(("fingerprint", fingerprint.to_string()))
    .bind(("memory", rid("memory", memory_id)))
    .await?
    .check()?;
    let live = take::<Vec<surrealdb::types::RecordId>>(&mut result, 0)?;
    let memory = take::<Option<ClaimHead>>(&mut result, 1)?;
    let Some(memory) = memory else {
        return Ok(Vec::new());
    };
    if memory.lifecycle != "active" {
        put_frontier_in(
            tx,
            receipt,
            "reconcile",
            generation,
            &ReconcileWork::ScanNode {
                node: memory_node(memory_id),
                side: 0,
                after: String::new(),
            },
            false,
        )
        .await?;
        return Ok(Vec::new());
    }
    if !live.is_empty() || !CLAIM_KINDS.contains(&memory.memory_kind.as_str()) {
        return Ok(Vec::new());
    }
    let app = memory
        .source_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|value| {
            value
                .get("app")
                .and_then(|app| app.as_str())
                .map(str::to_string)
        });
    if !matches!(
        app.as_deref(),
        Some("article" | "atlas" | "recon" | "recon-deleted")
    ) {
        return Ok(Vec::new());
    }
    let changed = set_lifecycle_in(tx, &[memory_id.to_string()], "retired").await?;
    put_frontier_in(
        tx,
        receipt,
        "reconcile",
        generation,
        &ReconcileWork::ScanNode {
            node: memory_node(memory_id),
            side: 0,
            after: String::new(),
        },
        !changed.is_empty(),
    )
    .await?;
    Ok(changed)
}

#[derive(SurrealValue)]
struct OwnerHead {
    id: surrealdb::types::RecordId,
    active_snapshot_id: String,
    lifecycle: String,
    desired_generation: i64,
}

async fn reconcile_owner_in(
    tx: &Tx,
    receipt: &str,
    generation: i64,
    kind: &str,
    article: &str,
) -> Result<(), TxError> {
    if !matches!(kind, "article" | "atlas") {
        return Err(reconcile_error("owner_kind_corrupt"));
    }
    if latest_retained_in(tx, article).await?.is_some() {
        return Ok(());
    }
    let mut result=super::classifier_admission::tx_query(tx,
        "SELECT id,active_snapshot_id,lifecycle,desired_generation FROM insight_owner_state WITH INDEX insight_owner_scope
           WHERE owner_kind=$kind AND owner_id=$article LIMIT 2;",
    ).bind(("kind",kind.to_string())).bind(("article",article.to_string())).await?.check()?;
    let rows = take::<Vec<OwnerHead>>(&mut result, 0)?;
    if rows.len() > 1 {
        return Err(reconcile_error("owner_identity_corrupt"));
    }
    if let Some(owner) = rows.into_iter().next() {
        if owner.lifecycle != "previous" {
            let revision = owner
                .desired_generation
                .checked_add(1)
                .ok_or_else(|| reconcile_error("owner_generation_exhausted"))?;
            super::classifier_admission::tx_query(tx,
                "UPDATE $owner SET desired_generation=$generation,lifecycle='previous',last_error='origin article removed',updated_at=$now RETURN NONE;",
            ).bind(("owner",owner.id)).bind(("generation",revision)).bind(("now",chrono::Utc::now().to_rfc3339())).await?.check()?;
        }
        if !owner.active_snapshot_id.is_empty() {
            put_frontier_in(
                tx,
                receipt,
                "reconcile",
                generation,
                &ReconcileWork::OwnerLinks {
                    snapshot: owner.active_snapshot_id,
                    after: String::new(),
                },
                false,
            )
            .await?;
        }
    }
    Ok(())
}

async fn reconcile_owner_links_in(
    tx: &Tx,
    snapshot: &str,
    after: &str,
    remaining: usize,
) -> Result<(bool, usize, String), TxError> {
    let capacity = page_capacity(remaining, 2);
    let mut result=super::classifier_admission::tx_query(tx,
        "SELECT id,record::id(id) AS key FROM insight_derivation_link WITH INDEX insight_links_snapshot
           WHERE snapshot_id=$snapshot AND status!='origin_deleted' AND ($first OR id>$after)
           ORDER BY id LIMIT $limit;",
    ).bind(("snapshot",snapshot.to_string())).bind(("first",after.is_empty()))
    .bind(("after",rid("insight_derivation_link",if after.is_empty(){"_"}else{after})))
    .bind(("limit",capacity+1)).await?.check()?;
    let mut rows = take::<Vec<KeyCaptureRow>>(&mut result, 0)?;
    let cost = 1 + rows.len() * 2;
    let more = rows.len() > capacity;
    rows.truncate(capacity);
    let mut last = after.to_string();
    let keys: Vec<_> = rows
        .iter()
        .map(|row| rid("insight_derivation_link", &row.key))
        .collect();
    if let Some(row) = rows.last() {
        last = row.key.clone();
    }
    if !keys.is_empty() {
        super::classifier_admission::tx_query(
            tx,
            "UPDATE $rows SET status='origin_deleted' RETURN NONE;",
        )
        .bind(("rows", keys))
        .await?
        .check()?;
    }
    Ok((!more, cost, last))
}

#[derive(SurrealValue)]
struct EndpointHead {
    kind: String,
    ref_id: String,
    origin: String,
    lifecycle: String,
}

async fn endpoint_supported_in(tx: &Tx, endpoint: &CapturedEndpoint) -> Result<bool, TxError> {
    if endpoint.key.is_empty() {
        return Err(reconcile_error("endpoint_key_missing"));
    }
    let mut result = super::classifier_admission::tx_query(
        tx,
        "SELECT kind,ref_id,origin,lifecycle FROM ONLY $node;",
    )
    .bind(("node", rid("brain_graph_node", &endpoint.key)))
    .await?
    .check()?;
    let node = take::<Option<EndpointHead>>(&mut result, 0)?;
    let (kind, id, origin) = if !endpoint.kind.is_empty() {
        (
            endpoint.kind.as_str(),
            endpoint.ref_id.as_str(),
            endpoint.origin.as_str(),
        )
    } else if let Some(node) = &node {
        (
            node.kind.as_str(),
            node.ref_id.as_str(),
            node.origin.as_str(),
        )
    } else {
        return Ok(false);
    };
    if id.is_empty() {
        return Err(reconcile_error("endpoint_identity_missing"));
    }
    match kind {
        "memory" => {
            let mut result = super::classifier_admission::tx_query(
                tx,
                "SELECT VALUE lifecycle FROM ONLY $memory;",
            )
            .bind(("memory", rid("memory", id)))
            .await?
            .check()?;
            Ok(match take::<Option<String>>(&mut result, 0)? {
                Some(lifecycle) => matches!(lifecycle.as_str(), "active" | ""),
                // Legacy projection placeholders intentionally have no memory
                // until their owner publishes it; preserve that contract.
                None => node.is_some_and(|node| node.lifecycle.is_empty()),
            })
        }
        "article" => {
            let mut result =
                super::classifier_admission::tx_query(tx, "RETURN record::exists($article);")
                    .bind((
                        "article",
                        rid("atlas_article", &super::codec::encode(&[origin, id])),
                    ))
                    .await?
                    .check()?;
            Ok(take::<Option<bool>>(&mut result, 0)? == Some(true))
        }
        "source" => Ok(node.is_some()),
        _ => Err(reconcile_error("endpoint_kind_corrupt")),
    }
}

#[derive(SurrealValue)]
struct RelationshipHead {
    lifecycle: String,
    memory_kind: String,
}

#[derive(SurrealValue)]
struct LiveRelationship {
    lifecycle: String,
    memory_kind: String,
    payload_json: Option<String>,
    lifecycle_ingest_batch: Option<String>,
    owner_revision: String,
}

fn refresh_for<'a>(
    guards: &'a ReconcileGuards,
    endpoint: &CapturedEndpoint,
) -> Option<&'a MutationIntent> {
    if endpoint.kind != "article" {
        return None;
    }
    guards
        .mutations
        .get(&super::codec::encode(&[&endpoint.origin, &endpoint.ref_id]))
}

async fn reconcile_relationship_in(
    tx: &Tx,
    receipt: &str,
    generation: i64,
    memory_id: &str,
    endpoints: &[CapturedEndpoint],
    owner_revision: &str,
    old_ingest_batch: &str,
    cleanup_eligible: bool,
    guards: &ReconcileGuards,
) -> Result<Vec<String>, TxError> {
    if endpoints.len() > 2 {
        return Err(reconcile_error("relationship_endpoints_corrupt"));
    }
    let mut result=super::classifier_admission::tx_query(tx,
        "SELECT lifecycle,memory_kind,payload_json,lifecycle_ingest_batch,
           crypto::sha256(type::string([text,payload_json,memory_kind])) AS owner_revision FROM ONLY $memory;",
    ).bind(("memory",rid("memory",memory_id))).await?.check()?;
    let Some(memory) = take::<Option<LiveRelationship>>(&mut result, 0)? else {
        return Ok(Vec::new());
    };
    if !matches!(memory.memory_kind.as_str(), "link" | "related") {
        return Err(reconcile_error("relationship_kind_conflict"));
    }
    if memory.owner_revision != owner_revision {
        // A new owner publication/user edit may have reused this memory ID.
        // Re-read its bounded current edges rather than retiring from an old
        // endpoint capture or treating its new ingest stamp as old inventory.
        put_frontier_in(
            tx,
            receipt,
            "reconcile",
            generation,
            &ReconcileWork::ReloadRelationship {
                memory_id: memory_id.into(),
                revision: memory.owner_revision,
                after: String::new(),
            },
            false,
        )
        .await?;
        return Ok(Vec::new());
    }
    if !matches!(memory.lifecycle.as_str(), "active" | "") {
        put_frontier_in(
            tx,
            receipt,
            "reconcile",
            generation,
            &ReconcileWork::ScanNode {
                node: memory_node(memory_id),
                side: 0,
                after: String::new(),
            },
            false,
        )
        .await?;
        return Ok(Vec::new());
    }
    let ingest_cleanup = cleanup_eligible && ingest_memory_writable_in(tx, memory_id).await?;
    let mut unsupported = false;
    for endpoint in endpoints {
        if !endpoint_supported_in(tx, endpoint).await? {
            unsupported = true;
        }
        // A source refresh may retire only captured ingest inventory. A pin,
        // analyst edit or validation gives a relationship its own authority;
        // changing the article revision alone must not retire that evidence.
        if ingest_cleanup {
            if let Some(MutationIntent::Refresh {
                source_revision, ..
            }) = refresh_for(guards, endpoint)
            {
                if !endpoint.revision.is_empty() && &endpoint.revision != source_revision {
                    unsupported = true;
                }
            }
        }
    }
    if ingest_cleanup {
        let payload = memory.payload_json.as_deref().and_then(|raw| {
            serde_json::from_str::<crate::memory_metadata::MemoryPayload>(raw).ok()
        });
        for endpoint in endpoints {
            let Some(MutationIntent::Refresh { batch, .. }) = refresh_for(guards, endpoint) else {
                continue;
            };
            if memory.lifecycle_ingest_batch.as_deref().unwrap_or_default() == old_ingest_batch
                && old_ingest_batch != batch
                && payload.as_ref().is_some_and(|payload| {
                    ingest_owned_for(payload, &endpoint.origin, &endpoint.ref_id)
                })
            {
                unsupported = true;
            }
        }
    }
    if !unsupported {
        return Ok(Vec::new());
    }
    let changed = set_lifecycle_in(tx, &[memory_id.to_string()], "retired").await?;
    put_frontier_in(
        tx,
        receipt,
        "reconcile",
        generation,
        &ReconcileWork::ScanNode {
            node: memory_node(memory_id),
            side: 0,
            after: String::new(),
        },
        !changed.is_empty(),
    )
    .await?;
    Ok(changed)
}

#[derive(SurrealValue)]
struct CapturedMentionHead {
    actor_id: String,
    origin_kind: String,
    origin_scope: String,
    origin_id: String,
    publication_batch: Option<String>,
}

async fn delete_captured_mention_in(
    tx: &Tx,
    work: &ReconcileWork,
    guards: &ReconcileGuards,
    batch: &SupportBatch,
) -> Result<(), TxError> {
    let ReconcileWork::DeleteMention {
        key,
        actor_id,
        run_id,
        article_id,
        old_batch,
    } = work
    else {
        return Err(reconcile_error("mention_work_kind_corrupt"));
    };
    let Some(intent) = guards
        .mutations
        .get(&super::codec::encode(&[run_id, article_id]))
    else {
        return Ok(());
    };
    let owner = match intent {
        MutationIntent::Refresh { batch, .. } | MutationIntent::Delete { batch } => batch,
    };
    let mut result = super::classifier_admission::tx_query(
        tx,
        "SELECT actor_id,origin_kind,origin_scope,origin_id,publication_batch FROM ONLY $mention;",
    )
    .bind(("mention", rid("brain_actor_mention", key)))
    .await?
    .check()?;
    let Some(actual) = take::<Option<CapturedMentionHead>>(&mut result, 0)? else {
        return Ok(());
    };
    if actual.actor_id != *actor_id
        || actual.origin_kind != "article"
        || actual.origin_scope != *run_id
        || actual.origin_id != *article_id
    {
        return Err(reconcile_error("mention_identity_conflict"));
    }
    let stamp = actual.publication_batch.unwrap_or_default();
    if &stamp != old_batch || &stamp == owner {
        return Ok(());
    }
    bump_support_epoch_in(tx, batch).await?;
    super::classifier_admission::tx_query(tx, "DELETE $mention RETURN NONE;")
        .bind(("mention", rid("brain_actor_mention", key)))
        .await?
        .check()?;
    Ok(())
}

fn actor_frontier_phase(kind: &str, actor: &str, generation: i64) -> String {
    format!(
        "{kind}:{}:{generation}",
        crate::classifier::artifact::sha256_hex(actor.as_bytes())
    )
}

async fn actor_member_support_in(tx: &Tx, actor: &str) -> Result<(bool, Option<String>), TxError> {
    let mut result=super::classifier_admission::tx_query(tx,
        "RETURN ((SELECT VALUE scope_key FROM ONLY $actor) ?? '')='dataset:argos_actors'
          OR count(SELECT id FROM brain_actor_mention WITH INDEX brain_actor_mention_actor WHERE actor_id=$id LIMIT 1)>0
          OR count(SELECT id FROM brain_graph_edge WITH INDEX brain_graph_edge_output
               WHERE out=$node AND meaning='concerns' AND owner!=$id
                 AND in.memory_kind INSIDE $kinds AND in.lifecycle INSIDE ['active',''] LIMIT 1)>0
          OR ((SELECT VALUE pinned FROM ONLY $memory) ?? false)
          OR record::exists($edit);
         SELECT VALUE lifecycle FROM ONLY $memory;",
    ).bind(("actor",rid("brain_actor",actor))).bind(("memory",rid("memory",actor)))
    .bind(("node",rid("brain_graph_node",&memory_node(actor)))).bind(("id",actor.to_string()))
    .bind(("kinds",CLAIM_KINDS.map(String::from).to_vec())).bind(("edit",rid("insight_user_edit",actor))).await?.check()?;
    let supported = take::<Option<bool>>(&mut result, 0)?
        .ok_or_else(|| reconcile_error("actor_support_missing"))?;
    let lifecycle = take::<Option<String>>(&mut result, 1)?;
    Ok((supported, lifecycle))
}

async fn actor_member_step_in(
    tx: &Tx,
    receipt: &str,
    generation: i64,
    phase: &str,
    work: &mut ReconcileWork,
    remaining: usize,
) -> Result<(bool, usize, bool), TxError> {
    let ReconcileWork::Member {
        actor_id,
        root,
        member,
        mut checked,
        after,
    } = work.clone()
    else {
        return Err(reconcile_error("family_work_kind_corrupt"));
    };
    let (mut cost, mut supported) = (1, false);
    if !checked {
        let (live, lifecycle) = actor_member_support_in(tx, &member).await?;
        supported = live;
        checked = true;
        cost = 8;
        if lifecycle
            .as_deref()
            .is_some_and(|state| !matches!(state, "active" | ""))
        {
            put_frontier_in(
                tx,
                receipt,
                "reconcile",
                generation,
                &ReconcileWork::ScanNode {
                    node: memory_node(&member),
                    side: 0,
                    after: String::new(),
                },
                false,
            )
            .await?;
        }
    }
    let capacity = page_capacity(remaining.saturating_sub(cost), 2);
    let mut result=super::classifier_admission::tx_query(tx,
        "SELECT id,record::id(id) AS key FROM brain_actor_redirect WITH INDEX brain_actor_redirect_to
           WHERE to_actor=$member AND ($first OR id>$after) ORDER BY id LIMIT $limit;",
    ).bind(("member",member.clone())).bind(("first",after.is_empty()))
    .bind(("after",rid("brain_actor_redirect",if after.is_empty(){"_"}else{&after})))
    .bind(("limit",capacity+1)).await?.check()?;
    let mut rows = take::<Vec<KeyCaptureRow>>(&mut result, 0)?;
    cost += 1 + rows.len() * 2;
    let more = rows.len() > capacity;
    rows.truncate(capacity);
    let mut last = after;
    for child in rows {
        if child.key == root {
            return Err(reconcile_error("redirect_cycle"));
        }
        last = child.key.clone();
        let candidate = ReconcileWork::Member {
            actor_id: actor_id.clone(),
            root: root.clone(),
            member: child.key,
            checked: false,
            after: String::new(),
        };
        let mut seen = super::classifier_admission::tx_query(tx, "RETURN record::exists($row);")
            .bind((
                "row",
                rid(
                    "lifecycle_reconcile_frontier",
                    &super::codec::encode(&[receipt, phase, &candidate.identity()]),
                ),
            ))
            .await?
            .check()?;
        if take::<Option<bool>>(&mut seen, 0)? == Some(true) {
            return Err(reconcile_error("redirect_family_cycle"));
        }
        put_frontier_in(tx, receipt, phase, generation, &candidate, false).await?;
    }
    *work = ReconcileWork::Member {
        actor_id,
        root,
        member,
        checked,
        after: last,
    };
    Ok((!more, cost, supported))
}

#[derive(SurrealValue)]
struct ScopedActorHead {
    scope_key: String,
}

async fn actor_step_in(
    tx: &Tx,
    receipt: &str,
    generation: i64,
    work: &mut ReconcileWork,
    remaining: usize,
) -> Result<(bool, usize, Vec<String>), TxError> {
    let ReconcileWork::Actor {
        actor_id,
        current,
        root,
        mut supported,
    } = work.clone()
    else {
        return Err(reconcile_error("actor_work_kind_corrupt"));
    };
    let Some(root) = root else {
        let phase = actor_frontier_phase("walk", &actor_id, generation);
        let seen = ReconcileWork::Seen {
            member: current.clone(),
        };
        let mut result = super::classifier_admission::tx_query(
            tx,
            "RETURN record::exists($seen); SELECT VALUE to_actor FROM ONLY $redirect;",
        )
        .bind((
            "seen",
            rid(
                "lifecycle_reconcile_frontier",
                &super::codec::encode(&[receipt, &phase, &seen.identity()]),
            ),
        ))
        .bind(("redirect", rid("brain_actor_redirect", &current)))
        .await?
        .check()?;
        if take::<Option<bool>>(&mut result, 0)? == Some(true) {
            return Err(reconcile_error("redirect_cycle"));
        }
        let next = take::<Option<String>>(&mut result, 1)?;
        put_frontier_in(tx, receipt, &phase, generation, &seen, false).await?;
        finish_frontier_in(tx, receipt, &phase, generation, &seen, true).await?;
        if let Some(next) = next {
            if next.is_empty() {
                return Err(reconcile_error("redirect_target_missing"));
            }
            *work = ReconcileWork::Actor {
                actor_id,
                current: next,
                root: None,
                supported: false,
            };
        } else {
            let phase = actor_frontier_phase("family", &actor_id, generation);
            put_frontier_in(
                tx,
                receipt,
                &phase,
                generation,
                &ReconcileWork::Member {
                    actor_id: actor_id.clone(),
                    root: current.clone(),
                    member: current.clone(),
                    checked: false,
                    after: String::new(),
                },
                false,
            )
            .await?;
            *work = ReconcileWork::Actor {
                actor_id,
                current: current.clone(),
                root: Some(current),
                supported: false,
            };
        }
        return Ok((false, 4, Vec::new()));
    };
    let phase = actor_frontier_phase("family", &actor_id, generation);
    if let Some(mut member) = next_frontier_in(tx, receipt, &phase, generation).await? {
        let (done, cost, live) =
            actor_member_step_in(tx, receipt, generation, &phase, &mut member, remaining).await?;
        supported |= live;
        finish_frontier_in(tx, receipt, &phase, generation, &member, done).await?;
        *work = ReconcileWork::Actor {
            actor_id,
            current,
            root: Some(root),
            supported,
        };
        return Ok((false, cost, Vec::new()));
    }
    let mut result = super::classifier_admission::tx_query(
        tx,
        "SELECT scope_key FROM ONLY $actor; SELECT lifecycle,memory_kind FROM ONLY $memory;",
    )
    .bind(("actor", rid("brain_actor", &root)))
    .bind(("memory", rid("memory", &root)))
    .await?
    .check()?;
    let actor = take::<Option<ScopedActorHead>>(&mut result, 0)?;
    let memory = take::<Option<RelationshipHead>>(&mut result, 1)?;
    let mut changed = Vec::new();
    if let (Some(actor), Some(memory)) = (actor, memory) {
        if memory.memory_kind != "actor" {
            return Err(reconcile_error("actor_kind_conflict"));
        }
        let scoped = actor.scope_key.is_empty()
            || RECONCILED_SCOPES
                .iter()
                .any(|prefix| actor.scope_key.starts_with(prefix));
        if scoped && memory.lifecycle != "redirected" {
            changed = set_lifecycle_in(
                tx,
                &[root.clone()],
                if supported { "active" } else { "retired" },
            )
            .await?;
        }
        if !supported || !matches!(memory.lifecycle.as_str(), "active" | "") {
            put_frontier_in(
                tx,
                receipt,
                "reconcile",
                generation,
                &ReconcileWork::ScanNode {
                    node: memory_node(&root),
                    side: 0,
                    after: String::new(),
                },
                !changed.is_empty(),
            )
            .await?;
        }
    }
    Ok((true, 4, changed))
}

async fn reconcile_step_in(
    tx: &Tx,
    receipt: &str,
    generation: i64,
    work: &mut ReconcileWork,
    remaining: usize,
    guards: &ReconcileGuards,
    batch: &SupportBatch,
) -> Result<(bool, usize, Vec<String>), TxError> {
    match work.clone() {
        ReconcileWork::Source { source } => {
            reconcile_source_row_in(tx, &source).await?;
            Ok((true, 12, Vec::new()))
        }
        ReconcileWork::Claim {
            fingerprint,
            memory_id,
        } => Ok((
            true,
            8,
            reconcile_claim_in(tx, receipt, generation, &fingerprint, &memory_id).await?,
        )),
        ReconcileWork::Actor { .. } => {
            actor_step_in(tx, receipt, generation, work, remaining).await
        }
        ReconcileWork::ScanNode { node, side, after } => {
            let (cost, more, last) = capture_edges_in(
                tx,
                receipt,
                generation,
                Some(&node),
                None,
                side,
                &after,
                remaining,
                false,
            )
            .await?;
            *work = ReconcileWork::ScanNode {
                node,
                side: if more { side } else { side + 1 },
                after: if more { last } else { String::new() },
            };
            Ok((!more && side == 1, cost, Vec::new()))
        }
        ReconcileWork::Relationship {
            memory_id,
            endpoints,
            owner_revision,
            old_ingest_batch,
            cleanup_eligible,
            ..
        } => Ok((
            true,
            12,
            reconcile_relationship_in(
                tx,
                receipt,
                generation,
                &memory_id,
                &endpoints,
                &owner_revision,
                &old_ingest_batch,
                cleanup_eligible,
                guards,
            )
            .await?,
        )),
        ReconcileWork::ReloadRelationship {
            memory_id,
            revision,
            after,
        } => {
            let (cost, more, last) = capture_edges_in(
                tx,
                receipt,
                generation,
                None,
                Some(&memory_id),
                0,
                &after,
                remaining,
                false,
            )
            .await?;
            *work = ReconcileWork::ReloadRelationship {
                memory_id,
                revision,
                after: last,
            };
            Ok((!more, cost, Vec::new()))
        }
        ReconcileWork::DeleteMention { .. } => {
            delete_captured_mention_in(tx, work, guards, batch).await?;
            Ok((true, 4, Vec::new()))
        }
        ReconcileWork::Owner {
            owner_kind,
            article_id,
            ..
        } => {
            reconcile_owner_in(tx, receipt, generation, &owner_kind, &article_id).await?;
            Ok((true, 8, Vec::new()))
        }
        ReconcileWork::OwnerLinks { snapshot, after } => {
            let (done, cost, last) =
                reconcile_owner_links_in(tx, &snapshot, &after, remaining).await?;
            *work = ReconcileWork::OwnerLinks {
                snapshot,
                after: last,
            };
            Ok((done, cost, Vec::new()))
        }
        _ => Err(reconcile_error("reconcile_work_kind_corrupt")),
    }
}

async fn run_reconcile_in(
    tx: &Tx,
    receipt: &str,
    scope: &ReconcileScope,
    mut head: ReconcileHead,
    budget: usize,
    caller_mutated: bool,
) -> Result<ReconcileProgress, TxError> {
    let started = std::time::Instant::now();
    let mut guards = head.guards()?;
    if source_guards_in(tx, scope).await? != guards.sources {
        return Err(reconcile_error("source_conflict_restart_required"));
    }
    let fence = support_fence_in(tx).await?;
    let mut restarted = false;
    if fence != guards.fence {
        if !guards.own_changes {
            return Err(reconcile_error("support_conflict_restart_required"));
        }
        // Our prior publication may have advanced memories_changed only after
        // the closure returned. Revisit the saved affected work from fresh
        // points instead of guessing which concurrent increment was ours.
        // New generations lazily reset rows; there is no unbounded UPDATE.
        guards.generation = guards
            .generation
            .checked_add(1)
            .ok_or_else(|| reconcile_error("generation_exhausted"))?;
        restarted = true;
    }
    guards.own_changes = false;
    let batch = SupportBatch::new(tx);
    let mut changed = BTreeSet::new();
    let mut examined = 0;
    let mut drained = false;
    while examined + 16 <= budget {
        let Some(mut work) = next_frontier_in(tx, receipt, "reconcile", guards.generation).await?
        else {
            drained = true;
            break;
        };
        let (done, cost, ids) = reconcile_step_in(
            tx,
            receipt,
            guards.generation,
            &mut work,
            budget - examined,
            &guards,
            &batch,
        )
        .await?;
        if cost == 0 || cost > budget - examined {
            return Err(reconcile_error("frontier_budget_violation"));
        }
        examined += cost;
        changed.extend(ids);
        finish_frontier_in(tx, receipt, "reconcile", guards.generation, &work, done).await?;
    }
    guards.own_changes = caller_mutated || !changed.is_empty();
    guards.sources = source_guards_in(tx, scope).await?;
    guards.fence = support_fence_in(tx).await?;
    let pending = !drained
        && next_frontier_in(tx, receipt, "reconcile", guards.generation)
            .await?
            .is_some();
    head.phase = if pending || guards.own_changes {
        if restarted {
            "reconcile_restart"
        } else {
            "reconcile"
        }
    } else {
        "done"
    }
    .into();
    head.revision = head
        .revision
        .checked_add(1)
        .ok_or_else(|| reconcile_error("revision_exhausted"))?;
    head.guards_json = reconcile_json(&guards)?;
    save_head_in(tx, receipt, &head).await?;
    tracing::debug!(target:"argos::atlas_derivatives",examined=examined as u64,restarted,pending,elapsed_us=started.elapsed().as_micros() as u64,"reconcile_page");
    Ok(ReconcileProgress {
        changed_memory_ids: changed.into_iter().collect(),
        next: (head.phase != "done").then(|| head.cursor(receipt)),
        examined: examined as u64,
    })
}

async fn seal_captured_in(
    tx: &Tx,
    scope: &ReconcileScope,
    capture: &ReconcileCapture,
    caller_mutated: bool,
) -> Result<ReconcileProgress, TxError> {
    if !capture.ready {
        return Err(reconcile_error("capture_not_ready"));
    }
    let permit = capture
        .permit
        .as_ref()
        .ok_or_else(|| reconcile_error("capture_permit_missing"))?;
    permit.check_owner(tx)?;
    let (receipt, scope, mut head) = load_head_in(tx, scope, Some(&capture.cursor)).await?;
    if receipt != permit.receipt_key || head.revision != permit.revision || head.phase != "ready" {
        return Err(reconcile_error("capture_permit_stale"));
    }
    let mut guards = head.guards()?;
    guards.sources = source_guards_in(tx, &scope).await?;
    guards.fence = support_fence_in(tx).await?;
    guards.own_changes = false;
    head.phase = "reconcile".into();
    head.guards_json = reconcile_json(&guards)?;
    let captured = permit.examined;
    if capture.examined != captured as u64 {
        return Err(reconcile_error("capture_budget_violation"));
    }
    let budget = RECONCILE_STEP_IDENTITIES
        .checked_sub(captured)
        .ok_or_else(|| reconcile_error("capture_budget_violation"))?;
    let mut progress = run_reconcile_in(tx, &receipt, &scope, head, budget, caller_mutated).await?;
    progress.examined = progress
        .examined
        .checked_add(capture.examined)
        .ok_or_else(|| reconcile_error("capture_budget_violation"))?;
    Ok(progress)
}

/// Seals a source mutation made after a ready capture in the same transaction.
/// Call this after the caller's final guarded source/mention/index writes, and
/// persist `next` with its derivative checkpoint in that transaction.
pub async fn reconcile_captured_in(
    tx: &Tx,
    scope: &ReconcileScope,
    capture: &ReconcileCapture,
) -> Result<ReconcileProgress, TxError> {
    seal_captured_in(tx, scope, capture, true).await
}

/// Reconciles only the durable affected frontier. The caller supplies its
/// source/run/task fence; every continuation also rechecks captured source and
/// support revisions. Never falls back to the global repair implementation.
pub async fn reconcile_affected_in(
    tx: &Tx,
    scope: &ReconcileScope,
    cursor: Option<&ReconcileCursor>,
) -> Result<ReconcileProgress, TxError> {
    let (receipt, scope, head) = load_head_in(tx, scope, cursor).await?;
    match head.phase.as_str() {
        "done" => Ok(ReconcileProgress {
            changed_memory_ids: Vec::new(),
            next: None,
            examined: 0,
        }),
        "capture" | "ready" => {
            let capture = capture_affected_in(tx, &scope, Some(&head.cursor(&receipt))).await?;
            if !capture.ready {
                return Ok(ReconcileProgress {
                    changed_memory_ids: Vec::new(),
                    next: Some(capture.cursor),
                    examined: capture.examined,
                });
            }
            seal_captured_in(tx, &scope, &capture, false).await
        }
        "reconcile" | "reconcile_restart" => {
            run_reconcile_in(tx, &receipt, &scope, head, RECONCILE_STEP_IDENTITIES, false).await
        }
        _ => Err(reconcile_error("phase_corrupt")),
    }
}

/// Resumes already published inventory cleanup under the caller's current
/// source/task fence. This capability cannot capture or publish native output.
/// The original mutation and initial frontier remain authoritative only while
/// that publication's exact batch is still the occurrence's stored index.
pub(crate) async fn resume_published_in(
    tx: &Tx,
    scope: &ReconcileScope,
    cursor: &ReconcileCursor,
) -> Result<ReconcileProgress, TxError> {
    let (receipt, scope, mut head) = load_head_in(tx, scope, Some(cursor)).await?;
    if head.phase == "done" {
        return Ok(ReconcileProgress {
            changed_memory_ids: Vec::new(),
            next: None,
            examined: 0,
        });
    }
    if !matches!(head.phase.as_str(), "reconcile" | "reconcile_restart") {
        return Err(reconcile_error("cleanup_requires_published_frontier"));
    }
    let [ReconcileKey::Source { run_id, article_id }] = scope.keys.as_slice() else {
        return Err(reconcile_error("cleanup_scope"));
    };
    let mut guards = head.guards()?;
    let key = super::codec::encode(&[run_id, article_id]);
    let Some(MutationIntent::Refresh { batch, .. }) = guards.mutations.get(&key) else {
        return Err(reconcile_error("cleanup_requires_refresh_mutation"));
    };
    let mut current = super::classifier_admission::tx_query(
        tx,
        "RETURN (SELECT VALUE publication_batch FROM ONLY $index)=$batch;",
    )
    .bind(("index", rid("atlas_article_index", &key)))
    .bind(("batch", batch.clone()))
    .await?
    .check()?;
    if batch.is_empty() || take::<Option<bool>>(&mut current, 0)? != Some(true) {
        return Err(reconcile_error("cleanup_publication_superseded"));
    }
    let sources = source_guards_in(tx, &scope).await?;
    let fence = support_fence_in(tx).await?;
    if sources != guards.sources || fence != guards.fence {
        // The source/publication barrier permits adoption of the old cleanup,
        // never its native inputs. Lazy generation reset replays initial work,
        // preserving captured actors/relationships already removed from storage.
        guards.sources = sources;
        guards.fence = fence;
        guards.generation = guards
            .generation
            .checked_add(1)
            .ok_or_else(|| reconcile_error("generation_exhausted"))?;
        guards.own_changes = false;
        head.revision = head
            .revision
            .checked_add(1)
            .ok_or_else(|| reconcile_error("revision_exhausted"))?;
        head.phase = "reconcile_restart".into();
        head.guards_json = reconcile_json(&guards)?;
    }
    run_reconcile_in(tx, &receipt, &scope, head, RECONCILE_STEP_IDENTITIES, false).await
}

/// Runs one bounded affected step through the real publisher and index outbox.
/// Reuse the receipt to advance its persisted cursor until `next` is absent;
/// a new logical source revision needs its own operation receipt.
pub async fn reconcile_affected(
    repo: &super::repo::Repository,
    scope: &ReconcileScope,
    receipt_key: &str,
) -> Result<ReconcileProgress, super::classifier_admission::AdmissionError> {
    let (scope, _, hash) = normalized_scope(scope)?;
    if receipt_key.is_empty()
        || receipt_key.len() > 256
        || receipt_key.chars().any(char::is_control)
    {
        return Err(reconcile_error("invalid_receipt_key").into());
    }
    let receipt = receipt_key.to_string();
    let output = Arc::new(std::sync::Mutex::new(None));
    let slot = output.clone();
    let write: super::memory::MemoryWrite =
        Arc::new(move |tx| {
            let (scope, receipt, hash, slot) =
                (scope.clone(), receipt.clone(), hash.clone(), slot.clone());
            Box::pin(async move {
                let mut result=super::classifier_admission::tx_query(&tx,
                "SELECT version,scope_hash,scope_json,phase,revision,guards_json FROM ONLY $row;",
            ).bind(("row",rid("lifecycle_reconcile",&receipt))).await?.check()?;
                let head = take::<Option<ReconcileHead>>(&mut result, 0)?;
                let cursor = head.map_or_else(
                    || ReconcileCursor {
                        version: RECONCILE_VERSION.into(),
                        receipt_key: receipt.clone(),
                        frontier_kind: "capture".into(),
                        after_key: format!("{hash}:0"),
                    },
                    |head| head.cursor(&receipt),
                );
                let progress = reconcile_affected_in(&tx, &scope, Some(&cursor)).await?;
                let ids = progress.changed_memory_ids.clone();
                *slot
                    .lock()
                    .map_err(|_| reconcile_error("publisher_result_poisoned"))? = Some(progress);
                Ok(ids)
            })
        });
    super::memory::write_then_index(repo.publisher(), write).await?;
    let progress = output
        .lock()
        .map_err(|_| reconcile_error("publisher_result_poisoned"))?
        .take()
        .ok_or_else(|| reconcile_error("publisher_result_missing"))?;
    Ok(progress)
}

/// Explicit maintenance repair of the complete historical lifecycle. Normal
/// ingestion/refresh/read paths must use the affected frontier APIs instead.
pub async fn full_repair(
    repo: &super::repo::Repository,
) -> Result<Vec<String>, super::classifier_admission::AdmissionError> {
    let output = Arc::new(std::sync::Mutex::new(None));
    let slot = output.clone();
    let write: super::memory::MemoryWrite = Arc::new(move |tx| {
        let slot = slot.clone();
        Box::pin(async move {
            let ids = reconcile_lifecycle_in(&tx).await?;
            *slot
                .lock()
                .map_err(|_| reconcile_error("publisher_result_poisoned"))? = Some(ids.clone());
            Ok(ids)
        })
    });
    super::memory::write_then_index(repo.publisher(), write).await?;
    let ids = output
        .lock()
        .map_err(|_| reconcile_error("publisher_result_poisoned"))?
        .take()
        .ok_or_else(|| reconcile_error("publisher_result_missing"))?;
    Ok(ids)
}
