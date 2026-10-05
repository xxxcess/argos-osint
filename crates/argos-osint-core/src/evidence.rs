//! Scoped evidence passages for hybrid retrieval (spec §10).
//!
//! SQLite remains authoritative. Passages are rebuildable projections with
//! stable source identity, revision/hash, offsets and provenance.

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

/// Token-aware-ish char chunks with overlap. Does not call the embedder.
pub fn chunk_text(source_id: &str, revision: &str, kind: RecordKind, text: &str, size: usize, overlap: usize) -> Vec<EvidencePassage> {
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
}
