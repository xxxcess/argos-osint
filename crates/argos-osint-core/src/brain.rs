//! User-centric recall. Keyword overlap plus a small identity boost, in the
//! same role Odysseus uses for memory injection: relevant notes are selected
//! before the model sees the turn and placed in the system prompt.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub text: String,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScoredMemory {
    pub memory: Memory,
    pub score: f32,
}

pub fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 1)
        .map(|w| w.to_string())
        .collect()
}

fn jaccard(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let aset: std::collections::HashSet<&str> = a.iter().map(|s| s.as_str()).collect();
    let bset: std::collections::HashSet<&str> = b.iter().map(|s| s.as_str()).collect();
    let inter = aset.intersection(&bset).count() as f32;
    let union = aset.union(&bset).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn looks_like_identity(text: &str) -> bool {
    let l = text.to_lowercase();
    ["my name", "i am", "i'm", "called", "call me", "name is"]
        .iter()
        .any(|p| l.contains(p))
}

fn query_kind(query: &str) -> &'static str {
    let l = query.to_lowercase();
    if ["name", "who am", "who i", "identity", "call me"]
        .iter()
        .any(|w| l.contains(w))
    {
        "identity"
    } else if ["email", "phone", "address", "contact"]
        .iter()
        .any(|w| l.contains(w))
    {
        "contact"
    } else if ["prefer", "like", "favorite", "love", "hate"]
        .iter()
        .any(|w| l.contains(w))
    {
        "preference"
    } else if ["todo", "task", "remind", "meeting"]
        .iter()
        .any(|w| l.contains(w))
    {
        "task"
    } else {
        "fact"
    }
}

/// Rank memories for injection. Empty queries return nothing.
pub fn recall(memories: &[Memory], query: &str, top_k: usize) -> Vec<ScoredMemory> {
    let query = query.trim();
    if query.is_empty() || memories.is_empty() || top_k == 0 {
        return Vec::new();
    }
    let kind = query_kind(query);
    let q_tokens = tokenize(query);
    let mut scored = Vec::new();
    for memory in memories {
        let mut score = jaccard(&q_tokens, &tokenize(&memory.text));
        if kind == "identity" && looks_like_identity(&memory.text) {
            score = score.max(0.9);
        } else if kind == "contact" && memory.text.to_lowercase().contains("email") {
            score += 0.15;
        } else if kind == "preference"
            && ["prefer", "likes", "favorite"]
                .iter()
                .any(|w| memory.text.to_lowercase().contains(w))
        {
            score += 0.15;
        } else if kind == "task"
            && ["todo", "remind", "task"]
                .iter()
                .any(|w| memory.text.to_lowercase().contains(w))
        {
            score += 0.15;
        }
        if score >= 0.05 {
            scored.push(ScoredMemory {
                memory: memory.clone(),
                score,
            });
        }
    }
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(top_k);
    scored
}

pub fn format_injection(hits: &[ScoredMemory]) -> String {
    if hits.is_empty() {
        return String::new();
    }
    let mut out = String::from("USER MEMORY (recall for this turn):\n");
    for hit in hits {
        out.push_str("- ");
        out.push_str(hit.memory.text.trim());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(id: &str, text: &str) -> Memory {
        Memory {
            id: id.into(),
            text: text.into(),
            created_at: "t".into(),
        }
    }

    #[test]
    fn identity_query_prefers_name_memory() {
        let all = vec![
            mem("1", "The harbor report is filed under northwind"),
            mem("2", "My name is Sakie and I work the night desk"),
        ];
        let hits = recall(&all, "who am I", 3);
        assert_eq!(hits[0].memory.id, "2");
        assert!(hits[0].score >= 0.9);
    }

    #[test]
    fn overlap_ranks_and_empty_query_is_empty() {
        let all = vec![
            mem("1", "Prefers markdown reports with source urls"),
            mem("2", "The kettle is in the galley"),
        ];
        assert!(recall(&all, "   ", 4).is_empty());
        let hits = recall(&all, "markdown source reports", 2);
        assert_eq!(hits[0].memory.id, "1");
    }
}
