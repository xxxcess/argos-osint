//! User-centric recall for conversations in other Argos apps.

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
pub struct MemorySource {
    /// App that contributed the insight.
    pub app: String,
    /// Conversation or context identifier within that app.
    pub conversation_id: String,
    /// Optional message that supports the insight.
    #[serde(default)]
    pub message_id: Option<String>,
    /// Optional deep link or external reference.
    #[serde(default)]
    pub reference: Option<String>,
}

impl MemorySource {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.app.trim().is_empty(), "memory source app is required");
        anyhow::ensure!(
            !self.conversation_id.trim().is_empty(),
            "memory source conversation_id is required"
        );
        Ok(())
    }
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
    pub source: MemorySource,
}

fn default_category() -> String {
    "fact".into()
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
        .filter(|w| {
            w.len() > 1
                && !matches!(
                    *w,
                    "the"
                        | "and"
                        | "for"
                        | "with"
                        | "what"
                        | "who"
                        | "where"
                        | "when"
                        | "how"
                        | "are"
                        | "was"
                        | "were"
                        | "this"
                        | "that"
                        | "about"
                )
        })
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
    } else if ["project", "building", "working on"]
        .iter()
        .any(|w| l.contains(w))
    {
        "project"
    } else if ["goal", "trying to"].iter().any(|w| l.contains(w)) {
        "goal"
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
        let overlap = jaccard(&q_tokens, &tokenize(&memory.text));
        let category_match = kind != "fact" && memory.category == kind;
        let identity_match = kind == "identity" && looks_like_identity(&memory.text);
        if overlap == 0.0 && !category_match && !identity_match {
            continue;
        }
        let mut score = overlap;
        if category_match {
            score += 0.25;
        }
        if memory.pinned {
            score = score.max(0.55);
        }
        if kind == "identity"
            && (memory.category == "identity" || looks_like_identity(&memory.text))
        {
            score = score.max(0.9);
        } else if (kind == "contact" && memory.text.to_lowercase().contains("email"))
            || (kind == "preference"
                && ["prefer", "likes", "favorite"]
                    .iter()
                    .any(|w| memory.text.to_lowercase().contains(w)))
            || (kind == "task"
                && ["todo", "remind", "task"]
                    .iter()
                    .any(|w| memory.text.to_lowercase().contains(w)))
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

/// Cosine similarity below which a vector hit is noise for MiniLM and is dropped.
pub const SEMANTIC_FLOOR: f32 = 0.35;
/// Bonus weight for the weaker signal when lexical and vector recall agree.
const AGREEMENT_WEIGHT: f32 = 0.1;

/// Blends vector hits (`(memory_id, cosine similarity)` from the Lance index) with
/// the Jaccard/category ranking of [`recall`]. Each memory scores the stronger of
/// the two signals plus a small bonus from the weaker one, so a paraphrase with no
/// shared words is still recalled and agreement ranks first. Vector hits below
/// [`SEMANTIC_FLOOR`] or for ids not in `memories` are ignored. With no vector hits
/// this is exactly [`recall`]. Entity-linked insights (`Store::recon_recall`) are
/// merged on top by `recall_for_turn`.
pub fn hybrid_recall(
    memories: &[Memory],
    query: &str,
    vector_hits: &[(String, f32)],
    top_k: usize,
) -> Vec<ScoredMemory> {
    let query = query.trim();
    if query.is_empty() || memories.is_empty() || top_k == 0 {
        return Vec::new();
    }
    let lexical: std::collections::HashMap<String, f32> = recall(memories, query, memories.len())
        .into_iter()
        .map(|hit| (hit.memory.id, hit.score))
        .collect();
    let mut semantic: std::collections::HashMap<&str, f32> = std::collections::HashMap::new();
    for (id, score) in vector_hits {
        if *score >= SEMANTIC_FLOOR {
            let slot = semantic.entry(id.as_str()).or_insert(*score);
            *slot = slot.max(*score);
        }
    }
    let mut scored = Vec::new();
    for memory in memories {
        let lex = lexical.get(&memory.id).copied();
        let sem = semantic.get(memory.id.as_str()).copied();
        if lex.is_none() && sem.is_none() {
            continue;
        }
        let (lex, sem) = (lex.unwrap_or(0.0), sem.unwrap_or(0.0));
        let mut score = lex.max(sem) + AGREEMENT_WEIGHT * lex.min(sem);
        if memory.pinned && sem > 0.0 {
            score = score.max(0.55);
        }
        scored.push(ScoredMemory {
            memory: memory.clone(),
            score,
        });
    }
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(top_k);
    scored
}

/// Format recalled insights with their origin for a downstream chat app.
pub fn format_injection(hits: &[ScoredMemory]) -> String {
    if hits.is_empty() {
        return String::new();
    }
    let mut out = String::from("USER MEMORY (recall for this turn):\n");
    for hit in hits {
        let pin = if hit.memory.pinned { " pinned" } else { "" };
        let message = hit
            .memory
            .source
            .message_id
            .as_deref()
            .unwrap_or("unknown message");
        let reference = hit.memory.source.reference.as_deref().unwrap_or("");
        out.push_str(&format!(
            "- [{}{pin}] {} (from {} / {} / {}{})\n",
            hit.memory.category,
            hit.memory.text.trim(),
            hit.memory.source.app,
            hit.memory.source.conversation_id,
            message,
            if reference.is_empty() {
                String::new()
            } else {
                format!(" / {reference}")
            },
        ));
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
            category: "fact".into(),
            pinned: false,
            created_at: "t".into(),
            source: MemorySource {
                app: "test".into(),
                conversation_id: "test".into(),
                message_id: None,
                reference: None,
            },
        }
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
            mem("1", "The harbor document is filed under northwind"),
            mem("2", "My name is Sakie and I work the night desk"),
        ];
        let hits = recall(&all, "who am I", 3);
        assert_eq!(hits[0].memory.id, "2");
        assert!(hits[0].score >= 0.9);
    }

    #[test]
    fn hybrid_recall_adds_paraphrases_and_rewards_agreement() {
        let all = vec![
            mem("lex", "Harbor tanker manifests list the cargo"),
            mem("sem", "The vessel docked at dawn"),
            mem("both", "Harbor arrivals logged by the port authority"),
            mem("noise", "Kettle in the galley"),
        ];
        let hits = vec![
            ("sem".to_string(), 0.71),
            ("both".to_string(), 0.6),
            ("noise".to_string(), 0.2),
            ("gone".to_string(), 0.99),
        ];
        let ranked = hybrid_recall(&all, "harbor ship arrivals", &hits, 5);
        let ids: Vec<&str> = ranked.iter().map(|h| h.memory.id.as_str()).collect();
        assert_eq!(ids[0], "sem");
        assert!(ids.contains(&"both") && ids.contains(&"lex"));
        assert!(!ids.contains(&"noise"), "below the semantic floor");
        assert!(!ids.contains(&"gone"), "vector ids missing from SQLite are dropped");
        let both = ranked.iter().find(|h| h.memory.id == "both").unwrap().score;
        assert!(both > 0.6, "agreement adds a bonus over the vector score: {both}");
        // No vector hits: identical to Jaccard recall.
        assert_eq!(
            hybrid_recall(&all, "harbor cargo", &[], 3),
            recall(&all, "harbor cargo", 3)
        );
        assert!(hybrid_recall(&all, "  ", &hits, 3).is_empty());
    }

    #[test]
    fn overlap_ranks_and_empty_query_is_empty() {
        let all = vec![
            mem("1", "Prefers markdown documents with source urls"),
            mem("2", "The kettle is in the galley"),
        ];
        assert!(recall(&all, "   ", 4).is_empty());
        let hits = recall(&all, "markdown source documents", 2);
        assert_eq!(hits[0].memory.id, "1");
    }
}
