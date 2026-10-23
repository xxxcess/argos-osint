//! Memory recall over SurrealDB: DISKANN vector search with stale-hit
//! fencing, a paged lexical fallback, and the recall caps.
//!
//! A vector hit is used only when its embedding row was computed for the
//! memory's *current* text (`revision` = [`memory_revision`] of the stored
//! text), with the query's embedding model and the current generation, and
//! the memory is still active and of an allowed kind. Anything else is a
//! stale hit and is dropped, never shown. Generation and model are filters
//! inside the DISKANN search, so neighbours from another generation or model
//! never crowd fresh ones out of the k nearest; revision, lifecycle and kind
//! are checked on the returned rows. When fewer than the requested number of
//! vector hits survive (none, a failed search or no query vector included),
//! recall tops up with token-overlap (Jaccard) hits, paged through the
//! active memories of the allowed kinds via the `memory_lifecycle_kind`
//! index with a bounded top-k heap and at most [`LEXICAL_SCAN_CAP`] rows read.
//! Vector hits rank ahead of lexical ones; ties order by memory id, so
//! results are deterministic.

use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashSet};

use surrealdb::engine::any::Any;
use surrealdb::types::{RecordId, SurrealValue};
use surrealdb::Surreal;

use super::memory::memory_revision;
use super::tx::TxError;
use crate::memory_metadata::MemoryKind;

/// Most memories one recall returns.
pub const MAX_MEMORIES: usize = 5;
/// Most estimated tokens one recall returns.
pub const MAX_TOKENS: usize = 1_500;
/// Rows read per lexical page.
pub const LEXICAL_PAGE: usize = 256;
/// Most rows one lexical pass reads (16 pages), so fallback cost stays
/// bounded however large the store grows.
pub const LEXICAL_SCAN_CAP: usize = LEXICAL_PAGE * 16;
/// Kinds left out of default recall: actors serve identity lookup and
/// Link/Related records serve exploration, not independent evidence.
pub const DEFAULT_EXCLUDED: [MemoryKind; 3] =
    [MemoryKind::Actor, MemoryKind::Link, MemoryKind::Related];
const KNN_EF: usize = 100;

/// One recall request.
#[derive(Debug, Clone, Default)]
pub struct RecallQuery {
    pub text: String,
    /// Query vector; `None` goes straight to lexical recall.
    pub embedding: Option<Vec<f32>>,
    /// Fingerprint of the model that produced `embedding`.
    pub model: String,
    /// Current embedding generation; older rows are stale.
    pub generation: i64,
    /// Kinds to consider; empty = every kind except [`DEFAULT_EXCLUDED`].
    pub allowed_kinds: Vec<MemoryKind>,
    /// Capped at [`MAX_MEMORIES`]; 0 = the cap.
    pub max_memories: usize,
    /// Capped at [`MAX_TOKENS`]; 0 = the cap.
    pub token_budget: usize,
    /// The caller opted out of memory recall: return nothing, query nothing.
    pub opt_out: bool,
}

/// Which path produced a hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecallSource {
    Vector,
    Lexical,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecallHit {
    pub id: String,
    pub text: String,
    pub kind: String,
    /// Cosine similarity (vector) or Jaccard overlap (lexical), 0..=1.
    pub score: f32,
    pub source: RecallSource,
    /// Estimated tokens ([`estimate_tokens`]).
    pub tokens: usize,
}

/// Rough token count used for the budget: one token per 4 characters.
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

fn allowed_kinds(q: &RecallQuery) -> Vec<String> {
    let kinds: Vec<MemoryKind> = if q.allowed_kinds.is_empty() {
        MemoryKind::ALL
            .iter()
            .copied()
            .filter(|k| !DEFAULT_EXCLUDED.contains(k))
            .collect()
    } else {
        q.allowed_kinds.clone()
    };
    kinds.iter().map(|k| k.as_str().to_string()).collect()
}

/// Recall memories for `q`. A vector-index failure falls back to lexical
/// recall (reported on stderr); only lexical-path database errors are returned.
pub async fn recall(db: &Surreal<Any>, q: &RecallQuery) -> Result<Vec<RecallHit>, TxError> {
    if q.opt_out || q.text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let max = cap(q.max_memories, MAX_MEMORIES);
    let budget = cap(q.token_budget, MAX_TOKENS);
    let kinds = allowed_kinds(q);
    // Rank more than we return so the token budget can skip long ones.
    let k = max * 4;
    let mut ranked = Vec::new();
    if let Some(v) = &q.embedding {
        match vector_candidates(db, q, v, &kinds, k).await {
            Ok(hits) => ranked = hits,
            Err(e) => eprintln!("vector recall failed; using lexical recall: {e}"),
        }
    }
    // The top-up counts hits before the token budget is applied, so
    // `apply_caps` may still drop over-budget hits and return fewer than
    // `max` even when more lexical matches exist past the `k` ranked here.
    if ranked.len() < max {
        let have: HashSet<String> = ranked.iter().map(|h| h.id.clone()).collect();
        let lexical = lexical_candidates(db, &q.text, &kinds, k).await?;
        ranked.extend(lexical.into_iter().filter(|h| !have.contains(&h.id)));
    }
    Ok(apply_caps(ranked, max, budget))
}

fn cap(asked: usize, limit: usize) -> usize {
    if asked == 0 {
        limit
    } else {
        asked.min(limit)
    }
}

/// Best first, keeping at most `max` hits within `budget` tokens. A hit too
/// long for the remaining budget is skipped so shorter ones can still fit.
pub fn apply_caps(ranked: Vec<RecallHit>, max: usize, budget: usize) -> Vec<RecallHit> {
    let mut out = Vec::new();
    let mut used = 0;
    for hit in ranked {
        if out.len() == max {
            break;
        }
        if used + hit.tokens <= budget {
            used += hit.tokens;
            out.push(hit);
        }
    }
    out
}

#[derive(Debug, SurrealValue)]
struct KnnRow {
    key: String,
    revision: String,
    d: f64,
}

#[derive(Debug, SurrealValue)]
struct MemoryRow {
    id: String,
    text: String,
    memory_kind: String,
    lifecycle: String,
}

/// The DISKANN query for `k` neighbours; `k` is a literal in the KNN
/// operator (a bounded usize). Public for the plan test.
pub fn vector_sql(k: usize) -> String {
    format!(
        "SELECT key, revision, vector::distance::knn() AS d FROM memory_embeddings \
         WHERE generation = $g AND model = $model AND embedding <|{k},{KNN_EF}|> $v ORDER BY d"
    )
}

/// One lexical page; `after` adds the `id > $after` cursor. Public for the
/// plan test.
pub fn lexical_page_sql(after: bool) -> String {
    let cursor = if after { " AND id > $after" } else { "" };
    format!(
        "SELECT record::id(id) AS id, text, memory_kind, lifecycle FROM memory \
         WHERE lifecycle = 'active' AND memory_kind IN $kinds{cursor} \
         ORDER BY id LIMIT {LEXICAL_PAGE}"
    )
}

async fn vector_candidates(
    db: &Surreal<Any>,
    q: &RecallQuery,
    v: &[f32],
    kinds: &[String],
    k: usize,
) -> Result<Vec<RecallHit>, TxError> {
    let mut res = db
        .query(vector_sql(k))
        .bind(("g", q.generation))
        .bind(("model", q.model.clone()))
        .bind(("v", v.to_vec()))
        .await?
        .check()?;
    let knn: Vec<KnnRow> = res.take(0)?;
    if knn.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<RecordId> = knn
        .iter()
        .map(|r| RecordId::new("memory", r.key.as_str()))
        .collect();
    let mut res = db
        .query("SELECT record::id(id) AS id, text, memory_kind, lifecycle FROM $ids")
        .bind(("ids", ids))
        .await?
        .check()?;
    let rows: Vec<MemoryRow> = res.take(0)?;
    let kinds: HashSet<&str> = kinds.iter().map(String::as_str).collect();
    let mut heap = TopK::new(k);
    for row in rows {
        let Some(hit) = knn.iter().find(|h| h.key == row.id) else {
            continue;
        };
        let fresh = row.lifecycle == "active"
            && kinds.contains(row.memory_kind.as_str())
            && hit.revision == memory_revision(&row.text);
        if !fresh {
            continue;
        }
        // Cosine distance is 0..=2; similarity 1 - d, floored at 0.
        let score = (1.0 - hit.d).clamp(0.0, 1.0) as f32;
        heap.push(make_hit(row, score, RecallSource::Vector));
    }
    Ok(heap.into_sorted())
}

async fn lexical_candidates(
    db: &Surreal<Any>,
    text: &str,
    kinds: &[String],
    k: usize,
) -> Result<Vec<RecallHit>, TxError> {
    let query: HashSet<String> = crate::brain::tokenize(text).into_iter().collect();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let mut heap = TopK::new(k);
    let mut after: Option<RecordId> = None;
    let mut read = 0;
    loop {
        let mut req = db
            .query(lexical_page_sql(after.is_some()))
            .bind(("kinds", kinds.to_vec()));
        if let Some(a) = &after {
            req = req.bind(("after", a.clone()));
        }
        let rows: Vec<MemoryRow> = req.await?.check()?.take(0)?;
        let n = rows.len();
        read += n;
        if let Some(last) = rows.last() {
            after = Some(RecordId::new("memory", last.id.as_str()));
        }
        for row in rows {
            let score = jaccard(&query, &row.text);
            if score > 0.0 {
                heap.push(make_hit(row, score, RecallSource::Lexical));
            }
        }
        if n < LEXICAL_PAGE {
            break;
        }
        if read >= LEXICAL_SCAN_CAP {
            eprintln!(
                "lexical recall stopped at the {LEXICAL_SCAN_CAP}-row scan cap; \
                 later active memories were not scored"
            );
            break;
        }
    }
    Ok(heap.into_sorted())
}

fn jaccard(query: &HashSet<String>, text: &str) -> f32 {
    let doc: HashSet<String> = crate::brain::tokenize(text).into_iter().collect();
    let inter = query.intersection(&doc).count();
    let union = query.union(&doc).count();
    if union == 0 {
        0.0
    } else {
        inter as f32 / union as f32
    }
}

fn make_hit(row: MemoryRow, score: f32, source: RecallSource) -> RecallHit {
    RecallHit {
        tokens: estimate_tokens(&row.text),
        id: row.id,
        text: row.text,
        kind: row.memory_kind,
        score,
        source,
    }
}

/// Ranking order: higher score first, then lower id.
struct Ranked(RecallHit);

impl Ord for Ranked {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .score
            .total_cmp(&other.0.score)
            .then_with(|| other.0.id.cmp(&self.0.id))
    }
}
impl PartialOrd for Ranked {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for Ranked {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Ranked {}

/// Keeps the best `k` hits; the worst kept hit sits on top of the min-heap.
pub struct TopK {
    k: usize,
    heap: BinaryHeap<Reverse<Ranked>>,
}

impl TopK {
    pub fn new(k: usize) -> Self {
        TopK {
            k,
            heap: BinaryHeap::with_capacity(k + 1),
        }
    }

    pub fn push(&mut self, hit: RecallHit) {
        if self.k == 0 {
            return;
        }
        self.heap.push(Reverse(Ranked(hit)));
        if self.heap.len() > self.k {
            self.heap.pop();
        }
    }

    /// Best first.
    pub fn into_sorted(self) -> Vec<RecallHit> {
        // Ascending order of Reverse = best first.
        self.heap
            .into_sorted_vec()
            .into_iter()
            .map(|Reverse(Ranked(h))| h)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(id: &str, score: f32, tokens: usize) -> RecallHit {
        RecallHit {
            id: id.into(),
            text: String::new(),
            kind: "claim".into(),
            score,
            source: RecallSource::Lexical,
            tokens,
        }
    }

    #[test]
    fn top_k_keeps_the_best_with_id_tie_break() {
        let mut t = TopK::new(3);
        for (id, s) in [("d", 0.5), ("a", 0.5), ("z", 0.9), ("b", 0.1), ("c", 0.5)] {
            t.push(hit(id, s, 1));
        }
        let ids: Vec<String> = t.into_sorted().into_iter().map(|h| h.id).collect();
        assert_eq!(ids, vec!["z", "a", "c"]);
    }

    #[test]
    fn caps_limit_count_and_skip_hits_over_budget() {
        let ranked = vec![
            hit("a", 0.9, 1000),
            hit("b", 0.8, 600),
            hit("c", 0.7, 400),
            hit("d", 0.6, 50),
            hit("e", 0.5, 10),
            hit("f", 0.4, 10),
            hit("g", 0.3, 10),
            hit("h", 0.2, 10),
        ];
        let kept: Vec<String> = apply_caps(ranked, 5, 1500)
            .into_iter()
            .map(|h| h.id)
            .collect();
        assert_eq!(kept, vec!["a", "c", "d", "e", "f"]);
    }

    #[test]
    fn all_lists_every_memory_kind_once() {
        // Exhaustive match: a new MemoryKind variant fails to compile here
        // until it is added to ALL.
        fn listed(k: MemoryKind) -> bool {
            match k {
                MemoryKind::Actor
                | MemoryKind::Claim
                | MemoryKind::Inference
                | MemoryKind::Link
                | MemoryKind::Related
                | MemoryKind::CycleBrief
                | MemoryKind::InvestigationDigest
                | MemoryKind::SourceSummary
                | MemoryKind::ManualNote
                | MemoryKind::UserProfile => MemoryKind::ALL.contains(&k),
            }
        }
        assert!(MemoryKind::ALL.iter().all(|&k| listed(k)));
        let names: HashSet<&str> = MemoryKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(names.len(), MemoryKind::ALL.len(), "no duplicates");
        // Every listed kind round-trips through parse.
        for k in MemoryKind::ALL {
            assert_eq!(MemoryKind::parse(k.as_str()), Some(k));
        }
    }

    #[test]
    fn default_kinds_exclude_structural_memories() {
        let kinds = allowed_kinds(&RecallQuery::default());
        for k in DEFAULT_EXCLUDED {
            assert!(!kinds.contains(&k.as_str().to_string()));
        }
        assert!(kinds.contains(&"claim".to_string()));
        let explicit = allowed_kinds(&RecallQuery {
            allowed_kinds: vec![MemoryKind::Actor],
            ..Default::default()
        });
        assert_eq!(explicit, vec!["actor".to_string()]);
    }
}
