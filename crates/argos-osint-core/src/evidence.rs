//! Scoped evidence passages for hybrid retrieval (spec §10).
//!
//! SQLite remains authoritative. Passages are rebuildable projections with
//! stable source identity, revision/hash, offsets and provenance.
//!
//! Vector ranking uses **exact** cosine scores by default (Lance brute-force).
//! ANN / IVF may be enabled only when [`measure_ann_recall`] + [`decide_ann_policy`]
//! meet [`AnnThresholds`] (or an explicit force flag). ExactSearch remains the
//! process default ([`AnnPolicy::active`]); callers must not claim unmeasured
//! ANN speedups (spec §19).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    Memory,
    Claim,
    Passage,
    ToolObservation,
    DerivedSummary,
}

impl RecordKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Claim => "claim",
            Self::Passage => "passage",
            Self::ToolObservation => "tool_observation",
            Self::DerivedSummary => "derived_summary",
        }
    }
}

/// Indexing policy for passage / Brain vectors.
///
/// Exact search is the safe default. [`Self::AnnEnabled`] is only selected when a
/// [`AnnMeasurement`] meets [`AnnThresholds`] (or an explicit force flag). ANN is
/// never claimed from unmeasured production guesses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnPolicy {
    /// Brute-force / exact vector search (safe default).
    ExactSearch,
    /// IVF/ANN may be created and used — only after thresholds are met.
    AnnEnabled,
    /// Explicitly deferred: measurement incomplete or failed criteria.
    AnnDeferredUnmeasured,
}

impl AnnPolicy {
    /// Process default without a measurement: always exact.
    pub fn active() -> Self {
        Self::ExactSearch
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExactSearch => "exact_search",
            Self::AnnEnabled => "ann_enabled",
            Self::AnnDeferredUnmeasured => "ann_deferred_unmeasured",
        }
    }

    pub fn uses_ann(self) -> bool {
        matches!(self, Self::AnnEnabled)
    }
}

/// Conservative gates before enabling ANN (spec §19). Tuned for reproducibility in
/// offline harnesses — not production SLOs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnnThresholds {
    /// Minimum corpus rows before ANN is even considered.
    pub min_corpus_rows: usize,
    /// Minimum fraction of exact top-k IDs recovered by ANN (recall@k).
    pub min_recall_at_k: f32,
    pub k: usize,
    /// Optional: ANN wall time must not exceed exact * this factor (None = ignore).
    pub max_latency_factor: Option<f32>,
}

impl Default for AnnThresholds {
    fn default() -> Self {
        Self {
            min_corpus_rows: 256,
            min_recall_at_k: 0.92,
            k: 10,
            max_latency_factor: Some(1.5),
        }
    }
}

/// One reproducible measurement over a synthetic or fixture corpus.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnnMeasurement {
    pub corpus_size: usize,
    pub queries: usize,
    pub k: usize,
    pub recall_at_k: f32,
    pub exact_ms: f64,
    pub ann_ms: Option<f64>,
    pub justified: bool,
    pub notes: String,
}

/// Decide policy from a measurement. `force_ann` overrides only when measurement
/// has a finite recall (still refuses if corpus is empty).
pub fn decide_ann_policy(
    measurement: &AnnMeasurement,
    thresholds: &AnnThresholds,
    force_ann: bool,
) -> AnnPolicy {
    if measurement.corpus_size == 0 || measurement.queries == 0 {
        return AnnPolicy::AnnDeferredUnmeasured;
    }
    if force_ann && measurement.corpus_size >= thresholds.min_corpus_rows {
        return AnnPolicy::AnnEnabled;
    }
    if measurement.justified {
        AnnPolicy::AnnEnabled
    } else if measurement.corpus_size < thresholds.min_corpus_rows {
        AnnPolicy::ExactSearch
    } else {
        AnnPolicy::AnnDeferredUnmeasured
    }
}

/// Compare exact vs ANN top-k ID sets for each query; return mean recall@k and timings.
///
/// `exact` / `ann` return ordered id lists (best first). This harness does not invent
/// production metrics — callers supply the search functions under test.
pub fn measure_ann_recall<F, G>(
    corpus_size: usize,
    queries: &[Vec<String>],
    k: usize,
    thresholds: &AnnThresholds,
    mut exact: F,
    mut ann: G,
) -> AnnMeasurement
where
    F: FnMut(usize) -> Vec<String>,
    G: FnMut(usize) -> Vec<String>,
{
    use std::time::Instant;
    if queries.is_empty() {
        return AnnMeasurement {
            corpus_size,
            queries: 0,
            k,
            recall_at_k: 0.0,
            exact_ms: 0.0,
            ann_ms: None,
            justified: false,
            notes: "no probe queries".into(),
        };
    }
    let mut recall_sum = 0.0f32;
    let t_exact = Instant::now();
    let exact_hits: Vec<Vec<String>> = (0..queries.len()).map(|i| exact(i)).collect();
    let exact_ms = t_exact.elapsed().as_secs_f64() * 1000.0;
    let t_ann = Instant::now();
    let ann_hits: Vec<Vec<String>> = (0..queries.len()).map(|i| ann(i)).collect();
    let ann_ms = t_ann.elapsed().as_secs_f64() * 1000.0;
    for (e, a) in exact_hits.iter().zip(ann_hits.iter()) {
        let truth: std::collections::HashSet<&str> = e.iter().take(k).map(|s| s.as_str()).collect();
        if truth.is_empty() {
            recall_sum += 1.0;
            continue;
        }
        let hit = a
            .iter()
            .take(k)
            .filter(|id| truth.contains(id.as_str()))
            .count() as f32;
        recall_sum += hit / truth.len() as f32;
    }
    let recall_at_k = recall_sum / queries.len() as f32;
    let mut notes = Vec::new();
    let size_ok = corpus_size >= thresholds.min_corpus_rows;
    let recall_ok = recall_at_k + f32::EPSILON >= thresholds.min_recall_at_k;
    let latency_ok = match thresholds.max_latency_factor {
        None => true,
        Some(_) if exact_ms <= 0.0 => true,
        Some(factor) => ann_ms <= exact_ms * f64::from(factor) + 1e-6,
    };
    if !size_ok {
        notes.push(format!(
            "corpus {corpus_size} < min {}",
            thresholds.min_corpus_rows
        ));
    }
    if !recall_ok {
        notes.push(format!(
            "recall@{k} {recall_at_k:.3} < min {}",
            thresholds.min_recall_at_k
        ));
    }
    if !latency_ok {
        notes.push(format!(
            "ann {ann_ms:.3}ms vs exact {exact_ms:.3}ms exceeds factor"
        ));
    }
    let justified = size_ok && recall_ok && latency_ok;
    if justified {
        notes.push("thresholds met".into());
    }
    AnnMeasurement {
        corpus_size,
        queries: queries.len(),
        k,
        recall_at_k,
        exact_ms,
        ann_ms: Some(ann_ms),
        justified,
        notes: notes.join("; "),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvidencePassage {
    pub id: String,
    pub source_id: String,
    pub revision: String,
    pub kind: RecordKind,
    pub text: String,
    pub start_offset: usize,
    pub end_offset: usize,
    pub content_hash: String,
    #[serde(default)]
    pub meta: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HybridPassageHit {
    pub passage_id: String,
    pub lexical: f32,
    pub semantic: f32,
    pub score: f32,
    pub why: String,
}

/// Floor below which an exact vector hit is treated as noise (aligned with Brain).
pub const PASSAGE_SEMANTIC_FLOOR: f32 = 0.35;
const AGREEMENT_WEIGHT: f32 = 0.1;

/// Token-aware-ish char chunks with overlap. Does not call the embedder.
pub fn chunk_text(
    source_id: &str,
    revision: &str,
    kind: RecordKind,
    text: &str,
    size: usize,
    overlap: usize,
) -> Vec<EvidencePassage> {
    let size = size.max(64);
    let overlap = overlap.min(size / 2);
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut idx = 0usize;
    while start < chars.len() {
        let end = (start + size).min(chars.len());
        let slice: String = chars[start..end].iter().collect();
        let hash = content_hash(&slice);
        out.push(EvidencePassage {
            id: format!("{source_id}:p{idx}"),
            source_id: source_id.into(),
            revision: revision.into(),
            kind: kind.clone(),
            text: slice,
            start_offset: start,
            end_offset: end,
            content_hash: hash,
            meta: serde_json::json!({}),
        });
        if end == chars.len() {
            break;
        }
        start = end.saturating_sub(overlap);
        idx += 1;
    }
    out
}

pub fn content_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn tokenize(text: &str) -> Vec<String> {
    text.to_ascii_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 2)
        .map(|t| t.to_string())
        .collect()
}

fn jaccard(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let set_a: std::collections::HashSet<&String> = a.iter().collect();
    let set_b: std::collections::HashSet<&String> = b.iter().collect();
    let inter = set_a.intersection(&set_b).count() as f32;
    let union = set_a.union(&set_b).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

/// Bounded hybrid candidates over passages: lexical Jaccard + optional exact
/// vector scores. Does **not** build or query an ANN index.
pub fn hybrid_passage_candidates(
    query: &str,
    passages: &[EvidencePassage],
    vector_hits: &[(String, f32)],
    limit: usize,
) -> Vec<HybridPassageHit> {
    let query = query.trim();
    if query.is_empty() || passages.is_empty() || limit == 0 {
        return Vec::new();
    }
    let q_tokens = tokenize(query);
    let mut semantic: std::collections::HashMap<&str, f32> = std::collections::HashMap::new();
    for (id, score) in vector_hits {
        if *score >= PASSAGE_SEMANTIC_FLOOR {
            let slot = semantic.entry(id.as_str()).or_insert(*score);
            *slot = slot.max(*score);
        }
    }
    let mut scored = Vec::new();
    for passage in passages {
        let lex = jaccard(&q_tokens, &tokenize(&passage.text));
        let sem = semantic.get(passage.id.as_str()).copied().unwrap_or(0.0);
        if lex <= 0.0 && sem <= 0.0 {
            continue;
        }
        let score = lex.max(sem) + AGREEMENT_WEIGHT * lex.min(sem);
        let why = match (lex > 0.0, sem > 0.0) {
            (true, true) => "lexical+exact_vector agreement",
            (true, false) => "lexical overlap",
            (false, true) => "exact_vector (no ANN)",
            (false, false) => "none",
        };
        scored.push(HybridPassageHit {
            passage_id: passage.id.clone(),
            lexical: lex,
            semantic: sem,
            score,
            why: why.into(),
        });
    }
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(limit);
    scored
}

/// Prefer passages that contain an exact identifier token even if rank is low.
pub fn ensure_identifier_coverage<'a>(
    ranked: &[&'a EvidencePassage],
    identifier: &str,
    pool: &'a [EvidencePassage],
    limit: usize,
) -> Vec<&'a EvidencePassage> {
    let needle = identifier.trim();
    if needle.is_empty() || limit == 0 {
        return ranked.iter().copied().take(limit).collect();
    }
    let mut out: Vec<&EvidencePassage> = ranked.iter().copied().take(limit).collect();
    if out.iter().any(|p| p.text.contains(needle)) {
        return out;
    }
    if let Some(hit) = pool.iter().find(|p| p.text.contains(needle)) {
        if let Some(last) = out.last_mut() {
            *last = hit;
        } else {
            out.push(hit);
        }
    }
    out
}

/// Rank then force identifier coverage within `limit` (bounded hybrid pipeline).
pub fn hybrid_with_identifier_coverage<'a>(
    query: &str,
    passages: &'a [EvidencePassage],
    vector_hits: &[(String, f32)],
    identifier: &str,
    limit: usize,
) -> Vec<&'a EvidencePassage> {
    let hits = hybrid_passage_candidates(query, passages, vector_hits, limit.max(1) * 2);
    let by_id: std::collections::HashMap<&str, &EvidencePassage> =
        passages.iter().map(|p| (p.id.as_str(), p)).collect();
    let ranked: Vec<&EvidencePassage> = hits
        .iter()
        .filter_map(|h| by_id.get(h.passage_id.as_str()).copied())
        .collect();
    ensure_identifier_coverage(&ranked, identifier, passages, limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_cover_end_of_long_source() {
        let body = format!("{}DECISIVE-TOKEN{}", "a".repeat(500), "b".repeat(50));
        let passages = chunk_text("src-1", "r1", RecordKind::Passage, &body, 200, 40);
        assert!(passages.len() > 1);
        assert!(passages.last().unwrap().text.contains("DECISIVE-TOKEN"));
        let ranked: Vec<&EvidencePassage> = passages.iter().take(1).collect();
        let covered = ensure_identifier_coverage(&ranked, "DECISIVE-TOKEN", &passages, 1);
        assert!(covered[0].text.contains("DECISIVE-TOKEN"));
    }

    #[test]
    fn content_hash_stable() {
        assert_eq!(content_hash("abc"), content_hash("abc"));
        assert_ne!(content_hash("abc"), content_hash("abd"));
    }

    #[test]
    fn hybrid_bounds_and_prefers_agreement() {
        assert_eq!(AnnPolicy::active(), AnnPolicy::ExactSearch);
        let passages = vec![
            EvidencePassage {
                id: "p-lex".into(),
                source_id: "s".into(),
                revision: "1".into(),
                kind: RecordKind::Passage,
                text: "harbor tanker manifests arrived overnight".into(),
                start_offset: 0,
                end_offset: 40,
                content_hash: "a".into(),
                meta: serde_json::json!({}),
            },
            EvidencePassage {
                id: "p-sem".into(),
                source_id: "s".into(),
                revision: "1".into(),
                kind: RecordKind::Passage,
                text: "completely different wording about ships".into(),
                start_offset: 0,
                end_offset: 40,
                content_hash: "b".into(),
                meta: serde_json::json!({}),
            },
            EvidencePassage {
                id: "p-both".into(),
                source_id: "s".into(),
                revision: "1".into(),
                kind: RecordKind::Passage,
                text: "harbor ship arrivals at the quay".into(),
                start_offset: 0,
                end_offset: 30,
                content_hash: "c".into(),
                meta: serde_json::json!({}),
            },
        ];
        let vectors = vec![("p-sem".into(), 0.82_f32), ("p-both".into(), 0.7)];
        let hits = hybrid_passage_candidates("harbor ship arrivals", &passages, &vectors, 3);
        assert_eq!(hits.len(), 3);
        let both = hits
            .iter()
            .find(|h| h.passage_id == "p-both")
            .expect("agreement hit");
        assert!(both.why.contains("agreement"), "{}", both.why);
        assert!(both.lexical > 0.0 && both.semantic > 0.0);
        // Exact-only policy: vector-only paraphrase still eligible without ANN.
        assert!(hits.iter().any(|h| h.passage_id == "p-sem"));
        assert!(hits.iter().any(|h| h.passage_id == "p-lex"));
        // Agreement bonus must beat pure lexical of the same query overlap when
        // semantic is present on the agreement candidate.
        let lex_only = hits.iter().find(|h| h.passage_id == "p-lex").unwrap();
        assert!(
            both.score >= lex_only.score,
            "both={} lex={}",
            both.score,
            lex_only.score
        );
    }

    #[test]
    fn hybrid_with_identifier_forces_token_coverage() {
        let passages = chunk_text(
            "src",
            "1",
            RecordKind::Passage,
            &format!("{} ID-99-ZZ {}", "noise ".repeat(40), "tail"),
            80,
            10,
        );
        let covered = hybrid_with_identifier_coverage("noise", &passages, &[], "ID-99-ZZ", 1);
        assert_eq!(covered.len(), 1);
        assert!(covered[0].text.contains("ID-99-ZZ"));
    }

    #[test]
    fn measure_ann_recall_justifies_only_when_thresholds_met() {
        let corpus = 300usize;
        // Probe queries are unused by the closures below (index-driven).
        let queries: Vec<Vec<String>> = (0..8).map(|i| vec![format!("q{i}")]).collect();
        let exact_lists: Vec<Vec<String>> = (0..8)
            .map(|i| (0..10).map(|j| format!("id-{}-{}", i, j)).collect())
            .collect();
        // Perfect ANN recall.
        let thresholds = AnnThresholds {
            min_corpus_rows: 256,
            min_recall_at_k: 0.92,
            k: 10,
            max_latency_factor: None,
        };
        let perfect = measure_ann_recall(
            corpus,
            &queries,
            10,
            &thresholds,
            |i| exact_lists[i].clone(),
            |i| exact_lists[i].clone(),
        );
        assert!(perfect.justified, "{}", perfect.notes);
        assert_eq!(
            decide_ann_policy(&perfect, &thresholds, false),
            AnnPolicy::AnnEnabled
        );
        // Too-small corpus stays exact even with perfect recall.
        let small = measure_ann_recall(
            40,
            &queries,
            10,
            &thresholds,
            |i| exact_lists[i].clone(),
            |i| exact_lists[i].clone(),
        );
        assert!(!small.justified);
        assert_eq!(
            decide_ann_policy(&small, &thresholds, false),
            AnnPolicy::ExactSearch
        );
        // Poor recall → deferred.
        let poor = measure_ann_recall(
            corpus,
            &queries,
            10,
            &thresholds,
            |i| exact_lists[i].clone(),
            |_| vec!["noise".into()],
        );
        assert!(!poor.justified);
        assert_eq!(
            decide_ann_policy(&poor, &thresholds, false),
            AnnPolicy::AnnDeferredUnmeasured
        );
    }
}
