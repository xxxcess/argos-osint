//! Markdown reports written for a case.
//!
//! Every file uses the same OSINT BLUF product: bottom line first, then the
//! requirement, source-derived judgments, evidence, gaps, and a source list.
//! The collector does not invent claims beyond the excerpts it cites.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::search::SearchHit;
use crate::store::new_id;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReportMeta {
    pub id: String,
    pub case_id: Option<String>,
    pub title: String,
    pub path: String,
    pub created_at: String,
}

pub fn render_report(
    title: &str,
    case_id: Option<&str>,
    question: &str,
    body: &str,
    hits: &[SearchHit],
) -> String {
    let created = chrono::Utc::now()
        .format("%Y-%m-%d %I:%M:%S %p UTC")
        .to_string();
    let requirement = if question.trim().is_empty() {
        title.trim()
    } else {
        question.trim()
    };
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str("product: BLUF\n");
    out.push_str("handling: public OSINT\n");
    out.push_str(&format!("title: {}\n", yaml_escape(title)));
    if let Some(id) = case_id {
        out.push_str(&format!("case: {id}\n"));
    }
    out.push_str(&format!("requirement: {}\n", yaml_escape(requirement)));
    out.push_str(&format!("sources: {}\n", hits.len()));
    out.push_str("confidence: low\n");
    out.push_str(&format!("created: {created}\n"));
    out.push_str("---\n\n");
    out.push_str(&format!("# {}\n\n", title.trim()));
    out.push_str("## BLUF\n\n");
    out.push_str(&bluf_line(requirement, hits));
    out.push_str("\n\n## Requirement\n\n");
    out.push_str(requirement);
    out.push_str("\n\n## Key judgments\n\n");
    out.push_str(&key_judgments(hits));
    out.push_str("\n\n## Evidence\n\n");
    if hits.is_empty() {
        out.push_str("No public source was attached. Nothing in this file is cited to a URL.\n");
    } else {
        for (i, hit) in hits.iter().enumerate() {
            let n = i + 1;
            let title = one_line(&hit.title);
            out.push_str(&format!("### {n}. {title}\n\n"));
            out.push_str(&format!("- URL: {}\n", hit.url.trim()));
            out.push_str(&format!("- Retrieved: {created}\n"));
            out.push_str(&format!(
                "- Use: Judgment {n} restates this source only.\n\n"
            ));
        }
    }
    let note = body.trim();
    if !note.is_empty() {
        out.push_str("## Analyst note\n\n");
        out.push_str(note);
        out.push_str("\n\nThe note is not a substitute for the excerpts above.\n\n");
    }
    out.push_str("## Gaps and assumptions\n\n");
    out.push_str(&format!(
        "- Collection is public OSINT only, retrieved {created}. Closed sources, leaked data, and account access were not used.\n"
    ));
    out.push_str(
        "- Confidence is low. Each judgment is single-source and has not been graded for reliability or credibility.\n",
    );
    out.push_str(
        "- Absence of a source is not evidence that a claim is false. An empty excerpt means the collector stored a link and no text.\n",
    );
    out.push_str("\n## Source list\n\n");
    if hits.is_empty() {
        out.push_str("_No public sources were attached to this report._\n");
    } else {
        for (i, hit) in hits.iter().enumerate() {
            out.push_str(&format!(
                "{}. [{}]({}) — retrieved {created}\n",
                i + 1,
                one_line(&hit.title),
                hit.url.trim()
            ));
        }
    }
    out
}

fn bluf_line(requirement: &str, hits: &[SearchHit]) -> String {
    if hits.is_empty() {
        return format!(
            "No public source was attached for “{requirement}”. Nothing was discovered that can be stated as a finding. Confidence: low."
        );
    }
    let discovered = discovery_summary(hits);
    if discovered.is_empty() {
        let n = hits.len();
        let noun = if n == 1 { "link" } else { "links" };
        return format!(
            "On “{requirement}”, public sources returned {n} {noun} and no excerpt, so no finding can be stated. Confidence: low."
        );
    }
    format!("On “{requirement}”, public sources report: {discovered} Confidence: low.")
}

/// One short sentence for every source. Identical excerpts are kept once so
/// the bottom line states all of the evidence without repeating it.
fn discovery_summary(hits: &[SearchHit]) -> String {
    let mut sentences = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for hit in hits {
        let sentence = if hit.snippet.trim().is_empty() {
            let title = one_line(&hit.title);
            if title.is_empty() {
                String::new()
            } else {
                format!("{title}.")
            }
        } else {
            first_sentence(&hit.snippet)
        };
        if sentence.is_empty() {
            continue;
        }
        let key = sentence.to_lowercase();
        if !seen.insert(key) {
            continue;
        }
        sentences.push(sentence);
    }
    sentences.join(" ")
}

fn first_sentence(text: &str) -> String {
    let line = one_line(text);
    if line.is_empty() {
        return String::new();
    }
    let mut end = line.len();
    for (index, ch) in line.char_indices() {
        if matches!(ch, '.' | '!' | '?') && index >= 40 {
            end = index + ch.len_utf8();
            break;
        }
        if index >= 220 {
            end = index;
            break;
        }
    }
    let mut sentence = line[..end].trim().to_string();
    if !sentence.ends_with(['.', '!', '?']) {
        sentence.push('.');
    }
    sentence
}

fn key_judgments(hits: &[SearchHit]) -> String {
    if hits.is_empty() {
        return "No source-derived judgment. Confidence: low.".into();
    }
    let mut out = String::new();
    for (i, hit) in hits.iter().enumerate() {
        let excerpt = one_line(&hit.snippet);
        let stated = if excerpt.is_empty() {
            format!(
                "provides a link titled “{}” and no excerpt",
                one_line(&hit.title)
            )
        } else {
            format!("states: {excerpt}")
        };
        out.push_str(&format!(
            "{}. Source {} {stated} Confidence: low.\n",
            i + 1,
            i + 1
        ));
    }
    out
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `Sep 24 02:05 PM` from a stored `YYYY-MM-DD HH:MM:SS` stamp.
pub fn short_when(created_at: &str) -> String {
    chrono::NaiveDateTime::parse_from_str(created_at.trim(), "%Y-%m-%d %H:%M:%S")
        .map(|dt| dt.format("%b %d %I:%M %p").to_string())
        .unwrap_or_else(|_| created_at.chars().take(22).collect())
}

pub fn source_pack(
    title: &str,
    case_id: Option<&str>,
    question: &str,
    hits: &[SearchHit],
) -> String {
    render_report(title, case_id, question, "", hits)
}

fn yaml_escape(text: &str) -> String {
    let t = text.replace('\n', " ").trim().to_string();
    if t.chars().any(|c| ":#{}[]&*!|>%@`'\"".contains(c)) {
        format!("\"{}\"", t.replace('"', "\\\""))
    } else {
        t
    }
}

pub fn write_report(
    dir: &Path,
    title: &str,
    case_id: Option<&str>,
    markdown: &str,
) -> std::io::Result<ReportMeta> {
    fs::create_dir_all(dir)?;
    let id = new_id("report");
    let slug = slugify(title);
    let path: PathBuf = dir.join(format!("{id}-{slug}.md"));
    fs::write(&path, markdown)?;
    Ok(ReportMeta {
        id,
        case_id: case_id.map(|s| s.to_string()),
        title: title.to_string(),
        path: path.display().to_string(),
        created_at: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    })
}

/// How closely a desk question matches a report title. `0.0` is no overlap.
/// Stopwords such as "who" and "about" are ignored so "what did we learn
/// about elon musk" still matches a report titled "who is elon musk?".
pub fn title_score(query: &str, title: &str) -> f32 {
    let query_tokens = content_tokens(query);
    let title_tokens = content_tokens(title);
    if query_tokens.is_empty() || title_tokens.is_empty() {
        return 0.0;
    }
    let query_set: std::collections::HashSet<&str> =
        query_tokens.iter().map(|s| s.as_str()).collect();
    let title_set: std::collections::HashSet<&str> =
        title_tokens.iter().map(|s| s.as_str()).collect();
    let shared = query_set.intersection(&title_set).count() as f32;
    if shared == 0.0 {
        return 0.0;
    }
    let union = query_set.union(&title_set).count() as f32;
    let jaccard = shared / union;
    let cover = shared / title_set.len() as f32;
    jaccard.max(cover)
}

/// A title is relevant when at least half of its content words appear in the question.
pub fn title_matches(query: &str, title: &str) -> bool {
    title_score(query, title) >= 0.5
}

fn content_tokens(text: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "about",
        "already",
        "an",
        "and",
        "any",
        "are",
        "at",
        "by",
        "case",
        "did",
        "do",
        "does",
        "earlier",
        "find",
        "for",
        "from",
        "had",
        "have",
        "how",
        "in",
        "info",
        "information",
        "is",
        "know",
        "learn",
        "learned",
        "look",
        "me",
        "my",
        "of",
        "on",
        "or",
        "our",
        "please",
        "previous",
        "report",
        "reports",
        "tell",
        "that",
        "the",
        "this",
        "to",
        "up",
        "was",
        "we",
        "were",
        "what",
        "when",
        "where",
        "who",
        "why",
        "with",
        "you",
        "your",
    ];
    crate::brain::tokenize(text)
        .into_iter()
        .filter(|token| !STOP.iter().any(|stop| stop == token))
        .collect()
}

/// `true` when the model reply asks to open a new case worker.
pub fn route_wants_new_case(reply: &str) -> bool {
    let word = reply
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|ch: char| !ch.is_ascii_alphabetic())
        .to_ascii_uppercase();
    word.starts_with("NEW")
}

fn slugify(title: &str) -> String {
    let mut out = String::new();
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if out.chars().last().is_some_and(|c| c != '-') {
            out.push('-');
        }
        if out.len() >= 40 {
            break;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "note".into()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_contains_front_matter_and_sources() {
        let hits = vec![
            SearchHit {
                title: "Example".into(),
                url: "https://example.com".into(),
                snippet: "A page about the harbor.".into(),
            },
            SearchHit {
                title: "Second".into(),
                url: "https://example.org".into(),
                snippet: "The channel closed overnight.".into(),
            },
        ];
        let md = render_report(
            "Harbor",
            Some("case-1"),
            "what changed?",
            "The tide turned.",
            &hits,
        );
        assert!(md.starts_with("---\n"));
        assert!(md.contains("product: BLUF"));
        assert!(md.contains("case: case-1"));
        assert!(md.contains("## BLUF"));
        assert!(md.contains("## Key judgments"));
        assert!(md.contains("## Evidence"));
        assert!(md.contains("## Gaps and assumptions"));
        assert!(md.contains("## Source list"));
        assert!(md.find("## BLUF").unwrap() < md.find("## Key judgments").unwrap());
        let bluf = &md[md.find("## BLUF").unwrap()..md.find("## Key judgments").unwrap()];
        assert!(bluf.contains("what changed?"));
        assert!(bluf.contains("A page about the harbor."));
        assert!(bluf.contains("The channel closed overnight."));
        let evidence = &md[md.find("## Evidence").unwrap()..md.find("## Analyst note").unwrap()];
        assert!(!evidence.contains("Excerpt"));
        assert!(!evidence.contains("A page about the harbor."));
        assert!(md.contains("https://example.com"));
        assert!(md.contains("The tide turned."));
        assert!(md.contains("Confidence: low"));
        assert!(md.contains("AM") || md.contains("PM"));
        let shown = short_when("2026-09-24 14:05:00");
        assert_eq!(shown, "Sep 24 02:05 PM");
    }

    #[test]
    fn title_match_ignores_question_words() {
        assert!(title_matches(
            "what did we learn about elon musk",
            "who is elon musk?"
        ));
        assert!(!title_matches("hello", "who is elon musk?"));
        assert!(route_wants_new_case("NEW\n"));
        assert!(!route_wants_new_case("CHAT"));
    }
}
