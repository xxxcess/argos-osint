//! Scoped evidence passages for hybrid retrieval (spec §10).
//!
//! SQLite remains authoritative. Passages are rebuildable projections with
//! stable source identity, revision/hash, offsets and provenance.
//!
//! Vector ranking uses **exact** cosine scores supplied by the caller (typically
//! Lance brute-force search). ANN / IVF-PQ / HNSW index creation is **not**
//! enabled here: the locked LanceDB path has no measured corpus-size or
//! recall/latency comparison justifying an ANN policy (spec §19). Callers must
//! not claim unmeasured ANN speedups.

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

/// Indexing policy for passage vectors. Only [`Self::ExactSearch`] is active.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnPolicy {
    /// Brute-force / exact vector search (current default).
    ExactSearch,
    /// Reserved: enable only after measured recall vs exact and latency justify it.
    AnnDeferredUnmeasured,
}

impl AnnPolicy {
    pub fn active() -> Self {
        Self::ExactSearch
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExactSearch => "exact_search",
            Self::AnnDeferredUnmeasured => "ann_deferred_unmeasured",
        }
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
        let both = hits.iter().find(|h| h.passage_id == "p-both").expect("agreement hit");
        assert!(both.why.contains("agreement"), "{}", both.why);
        assert!(both.lexical > 0.0 && both.semantic > 0.0);
        // Exact-only policy: vector-only paraphrase still eligible without ANN.
        assert!(hits.iter().any(|h| h.passage_id == "p-sem"));
        assert!(hits.iter().any(|h| h.passage_id == "p-lex"));
        // Agreement bonus must beat pure lexical of the same query overlap when
        // semantic is present on the agreement candidate.
        let lex_only = hits.iter().find(|h| h.passage_id == "p-lex").unwrap();
        assert!(both.score >= lex_only.score, "both={} lex={}", both.score, lex_only.score);
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
        let covered =
            hybrid_with_identifier_coverage("noise", &passages, &[], "ID-99-ZZ", 1);
        assert_eq!(covered.len(), 1);
        assert!(covered[0].text.contains("ID-99-ZZ"));
    }
}
