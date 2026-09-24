//! User-centric recall. Keyword overlap plus a small identity boost, in the
//! same role Odysseus uses for memory injection: relevant notes are selected
//! before the model sees the turn and placed in the system prompt.

use serde::{Deserialize, Serialize};

/// Memory types from Odysseus (`memory.js` `MEMORY_CATEGORIES` and the
/// extractor in `services/memory/memory_extractor.py`).
pub const CATEGORIES: &[&str] = &[
    "fact",
    "identity",
    "preference",
    "contact",
    "project",
    "goal",
    "task",
];

pub fn category_hint(category: &str) -> &'static str {
    match normalize_category(category) {
        "identity" => "Name, job, city, and what to call you",
        "preference" => "Likes, dislikes, and how you want things done",
        "contact" => "People, email, phone, and where to reach them",
        "project" => "Long-term work you are in the middle of",
        "goal" => "What you are trying to get done",
        "task" => "Todos, reminders, and meetings",
        _ => "A general fact worth recalling later",
    }
}

pub fn normalize_category(category: &str) -> &'static str {
    let compact: String = category
        .trim()
        .to_lowercase()
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '-' && *ch != '_')
        .collect();
    CATEGORIES
        .iter()
        .copied()
        .find(|name| *name == compact)
        .unwrap_or("fact")
}

/// `identity: My name is Ada` files an identity memory. Bare text is a fact.
pub fn parse_typed_memory(input: &str) -> (&'static str, String) {
    let input = input.trim();
    for category in CATEGORIES {
        let prefix = format!("{category}:");
        if let Some(rest) = input
            .get(..prefix.len())
            .filter(|head| head.eq_ignore_ascii_case(&prefix))
        {
            let _ = rest;
            let text = input[prefix.len()..].trim().to_string();
            return (*category, text);
        }
    }
    ("fact", input.to_string())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub text: String,
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default)]
    pub pinned: bool,
    pub created_at: String,
    /// Report whose chat produced this fact, when it came from a report.
    #[serde(default)]
    pub report_id: Option<String>,
}

fn default_category() -> String {
    "fact".into()
}

impl Memory {
    pub fn fact(
        id: impl Into<String>,
        text: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            category: "fact".into(),
            pinned: false,
            created_at: created_at.into(),
            report_id: None,
        }
    }
}

/// One or two short sentences, capped so a report reply can be stored as a fact.
pub fn clip_fact(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let flat = flat.trim().trim_matches('"').trim();
    if flat.is_empty() {
        return String::new();
    }
    let mut end = flat.len();
    let mut sentences = 0;
    for (index, ch) in flat.char_indices() {
        if matches!(ch, '.' | '!' | '?') {
            sentences += 1;
            end = index + ch.len_utf8();
            if sentences == 2 {
                break;
            }
        }
        if index >= 280 {
            end = index;
            break;
        }
    }
    flat[..end].trim().to_string()
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
        if memory.category == kind {
            score += 0.25;
        }
        if memory.pinned {
            score = score.max(0.55);
        }
        if kind == "identity"
            && (memory.category == "identity" || looks_like_identity(&memory.text))
        {
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

/// Facts that came from a completed report and overlap the case-desk question.
/// A pin or a category label does not count as a hit by itself.
pub fn recall_report_facts(memories: &[Memory], query: &str, top_k: usize) -> Vec<ScoredMemory> {
    let query = query.trim();
    if query.is_empty() || top_k == 0 {
        return Vec::new();
    }
    let q_tokens = tokenize(query);
    let mut scored = Vec::new();
    for memory in memories.iter().filter(|memory| memory.report_id.is_some()) {
        let score = jaccard(&q_tokens, &tokenize(&memory.text));
        if score >= 0.12 {
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

/// The user is asking for a fresh case even though facts may already answer it.
pub fn insists_on_new_case(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "new case",
        "case worker",
        "start a case",
        "start research",
        "new research",
        "research again",
        "search again",
        "look it up",
        "look this up",
        "new investigation",
        "ignore the memory",
        "ignore memory",
        "don't use the memory",
        "do not use the memory",
        "fresh search",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
}

pub fn format_injection(hits: &[ScoredMemory]) -> String {
    if hits.is_empty() {
        return String::new();
    }
    let mut out = String::from("USER MEMORY (recall for this turn):\n");
    for hit in hits {
        let pin = if hit.memory.pinned { " pinned" } else { "" };
        out.push_str(&format!(
            "- [{}{pin}] {}\n",
            hit.memory.category,
            hit.memory.text.trim()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(id: &str, text: &str) -> Memory {
        Memory::fact(id, text, "t")
    }

    #[test]
    fn files_each_odysseus_category() {
        assert_eq!(CATEGORIES.len(), 7);
        assert_eq!(
            parse_typed_memory("identity: My name is Ada"),
            ("identity", "My name is Ada".into())
        );
        assert_eq!(
            parse_typed_memory("just a fact"),
            ("fact", "just a fact".into())
        );
        assert_eq!(normalize_category("Project"), "project");
        assert_eq!(normalize_category("nope"), "fact");
        assert_eq!(
            clip_fact("Harbor revenue rose. A second sentence stays. A third does not."),
            "Harbor revenue rose. A second sentence stays."
        );
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
        let mut fact = Memory::fact("m", "Harbor revenue rose (report: harbor budget)", "t");
        fact.report_id = Some("rep".into());
        let hits = recall_report_facts(&[fact], "what about harbor revenue", 3);
        assert_eq!(hits.len(), 1);
        assert!(insists_on_new_case("start a new case worker for harbor"));
        assert!(!insists_on_new_case("what did we learn about harbor"));
        let hits = recall(&all, "markdown source reports", 2);
        assert_eq!(hits[0].memory.id, "1");
    }
}
