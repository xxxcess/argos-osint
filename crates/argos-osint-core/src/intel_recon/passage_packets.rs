//! Actor-focused evidence packets (spec §2 "Actor-focused evidence selection").
//!
//! Actor names/aliases and claim event phrases, dates and quantities are
//! matched as **token-boundary n-grams** over the shared [`SentenceIndex`]
//! (never arbitrary substrings). Each hit starts with the matched sentence
//! and ±`neighbours` sentences in its paragraph, expanding to the whole
//! paragraph for quotation, attribution, pronoun or qualifier context.
//! Denials, corrections and counterevidence anywhere in the story are always
//! protected. Packets keep stable sentence IDs, source spans and an omission
//! manifest; the rendered text marks every gap with the omitted IDs. Rust
//! retrieves the original passages — nothing here summarizes evidence.
//! The full clean story stays the display artifact; packets are only used by
//! the measured oversized-story path.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::sentence_index::SentenceIndex;

pub const PACKET_POLICY: &str = "actor-packets-v1";

#[derive(Clone, Debug, Default)]
pub struct PacketQuery {
    /// Canonical names plus confirmed aliases.
    pub actors: Vec<String>,
    /// Claim predicates / event phrases.
    pub phrases: Vec<String>,
    pub dates: Vec<String>,
    pub quantities: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacketSet {
    pub policy: String,
    pub content_hash: String,
    /// Selected sentence IDs in document order.
    pub selected: Vec<String>,
    /// Omitted sentence IDs in document order.
    pub omitted: Vec<String>,
    /// Why each selected sentence is present (`match`, `neighbour`,
    /// `paragraph`, `protected`).
    pub reasons: Vec<(String, String)>,
    /// Selected / total sentences.
    pub coverage: String,
}

fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '%' && c != '.' && c != ',')
        .map(|t| t.trim_matches(|c: char| c == '.' || c == ','))
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Token-boundary n-gram containment.
pub fn contains_ngram(haystack: &[String], needle: &[String]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

const ATTRIBUTION: &[&str] = &[
    "said",
    "says",
    "told",
    "according",
    "stated",
    "added",
    "claimed",
    "denied",
    "alleged",
];
const PRONOUN_STARTS: &[&str] = &[
    "he", "she", "they", "it", "his", "her", "their", "this", "that",
];

fn needs_paragraph(text: &str, toks: &[String]) -> bool {
    text.contains(['"', '“', '”', '«', '»'])
        || toks.iter().any(|t| ATTRIBUTION.contains(&t.as_str()))
        || toks
            .first()
            .is_some_and(|t| PRONOUN_STARTS.contains(&t.as_str()))
        || crate::nlp::select::marker(&text.to_lowercase())
}

pub fn build_packets(
    index: &SentenceIndex,
    clean: &str,
    query: &PacketQuery,
    neighbours: usize,
) -> PacketSet {
    let sentences = &index.sentences;
    let texts: Vec<&str> = sentences
        .iter()
        .map(|s| &clean[s.start_byte..s.end_byte])
        .collect();
    let toks: Vec<Vec<String>> = texts.iter().map(|t| tokens(t)).collect();
    let needles: Vec<Vec<String>> = query
        .actors
        .iter()
        .chain(&query.phrases)
        .chain(&query.dates)
        .chain(&query.quantities)
        .map(|n| tokens(n))
        .filter(|n| !n.is_empty())
        .collect();
    let mut picked: Vec<Option<&'static str>> = vec![None; sentences.len()];
    let set = |i: usize, why: &'static str, picked: &mut Vec<Option<&'static str>>| {
        if picked[i].is_none() {
            picked[i] = Some(why);
        }
    };
    for (i, t) in toks.iter().enumerate() {
        if !needles.iter().any(|n| contains_ngram(t, n)) {
            continue;
        }
        set(i, "match", &mut picked);
        let para = &sentences[i].paragraph;
        let same_para: Vec<usize> = (0..sentences.len())
            .filter(|j| &sentences[*j].paragraph == para)
            .collect();
        let expand = same_para
            .iter()
            .any(|j| needs_paragraph(texts[*j], &toks[*j]));
        for j in same_para {
            let near = j.abs_diff(i) <= neighbours;
            if expand {
                set(j, "paragraph", &mut picked);
            } else if near {
                set(j, "neighbour", &mut picked);
            }
        }
    }
    // Protect denials/corrections/counterevidence throughout the story.
    for (i, t) in texts.iter().enumerate() {
        if crate::nlp::select::marker(&t.to_lowercase()) {
            set(i, "protected", &mut picked);
        }
    }
    let mut selected = Vec::new();
    let mut omitted = Vec::new();
    let mut reasons = Vec::new();
    for (s, why) in sentences.iter().zip(&picked) {
        match why {
            Some(w) => {
                selected.push(s.id.clone());
                reasons.push((s.id.clone(), (*w).to_string()));
            }
            None => omitted.push(s.id.clone()),
        }
    }
    PacketSet {
        policy: PACKET_POLICY.into(),
        content_hash: index.content_hash.clone(),
        coverage: format!("{} of {} sentences", selected.len(), sentences.len()),
        selected,
        omitted,
        reasons,
    }
}

/// Packet query for an article's Claims: actor names (+ confirmed aliases)
/// from native metadata, claim predicates/objects/attributions as event
/// phrases, claim times as dates, and numbers from claim text as quantities.
pub fn claims_query(actors: &[String], claims: &serde_json::Value) -> PacketQuery {
    let mut q = PacketQuery {
        actors: actors.to_vec(),
        ..PacketQuery::default()
    };
    for c in claims["claims"].as_array().into_iter().flatten() {
        let field = |f: &str| {
            c[f].as_str()
                .filter(|v| v.trim().len() >= 3)
                .map(str::to_string)
        };
        q.phrases.extend(field("predicate"));
        q.phrases.extend(field("object"));
        q.actors.extend(field("attributed_to"));
        q.dates.extend(field("time"));
        for t in c["text"].as_str().unwrap_or("").split_whitespace() {
            let t = t.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '%');
            if t.chars().any(|ch| ch.is_ascii_digit()) {
                q.quantities.push(t.to_string());
            }
        }
    }
    for list in [
        &mut q.actors,
        &mut q.phrases,
        &mut q.dates,
        &mut q.quantities,
    ] {
        list.sort();
        list.dedup();
    }
    q
}

/// Focused oversized-story selection: the packet's sentences in document
/// order (as wire sentences). `None` when nothing matched, so callers keep
/// the full body rather than reasoning over an empty or protected-only set.
pub fn focused_sentences(
    index: &SentenceIndex,
    clean: &str,
    query: &PacketQuery,
) -> Option<(Vec<super::brief_stages::Sentence>, PacketSet)> {
    let set = build_packets(index, clean, query, 2);
    if !set.reasons.iter().any(|(_, why)| why == "match") {
        return None;
    }
    let keep: BTreeSet<&str> = set.selected.iter().map(String::as_str).collect();
    let sentences = index
        .wire_sentences(clean)
        .into_iter()
        .filter(|s| keep.contains(s.id.as_str()))
        .collect();
    Some((sentences, set))
}

impl PacketSet {
    /// Render selected passages in original order; every gap carries an
    /// omission marker naming the omitted original sentence IDs.
    pub fn render(&self, index: &SentenceIndex, clean: &str) -> Vec<serde_json::Value> {
        let selected: BTreeSet<&str> = self.selected.iter().map(String::as_str).collect();
        let mut out = Vec::new();
        let mut gap: Vec<String> = Vec::new();
        for s in &index.sentences {
            if selected.contains(s.id.as_str()) {
                if !gap.is_empty() {
                    out.push(serde_json::json!({"omitted": std::mem::take(&mut gap)}));
                }
                out.push(serde_json::json!({"id": s.id, "text": &clean[s.start_byte..s.end_byte]}));
            } else {
                gap.push(s.id.clone());
            }
        }
        if !gap.is_empty() {
            out.push(serde_json::json!({"omitted": gap}));
        }
        out
    }
}

#[cfg(test)]
#[path = "passage_packets_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "focused_compare_tests.rs"]
mod focused_compare;
