//! Deterministic document cleanup, independent of the OSINT question parser.
use std::net::IpAddr;

use regex::Regex;

use super::corpus::CorpusDoc;
use super::extract::{extract_occurrences, Occurrence};
use super::types::{TnaDecision, TnaNodeKind};

/// Accepted occurrences and an audit of extraction decisions. Offsets refer to
/// the original report, not a rewritten or model-generated entity string.
pub(super) fn clean_document(doc: &CorpusDoc) -> (Vec<Occurrence>, Vec<TnaDecision>) {
    let mut accepted = Vec::new();
    let mut decisions = Vec::new();
    let mut section = "Evidence".to_string();
    let mut frontmatter = false;
    let mut offset = 0;
    let links = Regex::new(r"\[[^\]]*\]\([^)]*\)|\[\d+\]").unwrap();
    let references = Regex::new(r"^\[[^\]]+\]:").unwrap();
    let mut seen_lines = std::collections::HashSet::new();
    for line in doc.text.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed == "---" {
            frontmatter = !frontmatter;
            offset += line.len();
            continue;
        }
        if trimmed.starts_with("## ") {
            section = trimmed.trim_start_matches('#').trim().to_string();
        }
        let lower = trimmed.to_ascii_lowercase();
        let explicit_theme = lower.starts_with("theme:")
            || lower.starts_with("themes:")
            || lower.starts_with("- theme:")
            || lower.starts_with("- themes:");
        let usable = !trimmed.starts_with('#')
            && (!frontmatter || explicit_theme)
            && (matches!(
                section.to_ascii_lowercase().as_str(),
                "evidence" | "requirement" | "analyst note" | "themes" | "theme"
            ))
            && ![
                "- url:",
                "- retrieved:",
                "- use:",
                "the note is not a substitute",
                "- excerpt: no excerpt",
                "url:",
                "source:",
                "sources:",
            ]
            .iter()
            .any(|p| lower.starts_with(p));
        if !usable || trimmed.is_empty() || references.is_match(trimmed) {
            offset += line.len();
            continue;
        }
        // Duplicate excerpts/citations must not inflate mention counts or degree.
        if !seen_lines.insert((section.clone(), lower.clone())) {
            offset += line.len();
            continue;
        }
        let mut working = line.as_bytes().to_vec();
        // Blank only citation syntax/destinations; preserve nearby source-backed names.
        for m in links.find_iter(line) {
            if let Some(split) = m.as_str().find("](") {
                working[m.start()] = b' ';
                working[m.start() + split..m.end()].fill(b' ');
            } else {
                working[m.start()..m.end()].fill(b' ');
            }
        }
        if let Some(pos) = line.find("- Excerpt:") {
            working[pos..pos + "- Excerpt:".len()].fill(b' ');
        }
        let working = String::from_utf8(working).expect("mask preserves UTF-8");
        let mut candidates = extract_occurrences(&working);
        if explicit_theme || matches!(section.to_ascii_lowercase().as_str(), "theme" | "themes") {
            let value_start = if explicit_theme {
                line.find(':').unwrap() + 1
            } else {
                0
            };
            let mut pos = value_start;
            for value in line[value_start..].split([',', ';', '\n']) {
                let label = value.trim().trim_matches(['"', '\'']);
                if !label.is_empty() {
                    candidates.push(Occurrence {
                        kind: TnaNodeKind::Topic,
                        label: label.into(),
                        start: pos + value.find(label).unwrap_or(0),
                    });
                }
                pos += value.len() + 1;
            }
        }
        for mut item in candidates {
            // URL-host candidates may point at the URL start. Align them with
            // the host span so URL and bare-host extraction deduplicate.
            if working[item.start..]
                .to_ascii_lowercase()
                .starts_with("http")
            {
                if let Some(relative) = working[item.start..]
                    .to_ascii_lowercase()
                    .find(&item.label.to_ascii_lowercase())
                {
                    item.start += relative;
                }
            }
            let start = item.start;
            let handle_prefix =
                usize::from(item.kind == TnaNodeKind::Handle && line[start..].starts_with('@'));
            let end = (start + item.label.len() + handle_prefix).min(line.len());
            let original = line.get(start..end).unwrap_or(&item.label).to_string();
            let result = normalize(
                &item,
                line,
                explicit_theme
                    || matches!(section.to_ascii_lowercase().as_str(), "theme" | "themes"),
            );
            let (label, reason) = match result {
                Some(label) => {
                    let reason = if label == original.trim_start_matches('@') {
                        "accepted: validated source span"
                    } else {
                        "normalized: canonical spelling"
                    };
                    (Some(label), reason)
                }
                None => (
                    None,
                    match item.kind {
                        TnaNodeKind::Domain => {
                            "rejected: invalid hostname, file/path, or citation/email context"
                        }
                        TnaNodeKind::Topic => {
                            "rejected: no concise explicit theme or boilerplate phrase"
                        }
                        TnaNodeKind::Person | TnaNodeKind::Org => {
                            "rejected: uncertain name, boilerplate, fragment, or typed theme field"
                        }
                        _ => "rejected: invalid observable or URL/path context",
                    },
                ),
            };
            let canonical_id = label
                .as_ref()
                .map(|l| format!("{}:{}", item.kind.as_str(), canonical(l)));
            decisions.push(TnaDecision {
                report_id: doc.report_id.clone(),
                section: if frontmatter {
                    "frontmatter theme".into()
                } else {
                    section.clone()
                },
                start: offset + start,
                end: offset + end,
                original,
                kind: item.kind,
                label: label.clone(),
                canonical_id,
                reason: reason.into(),
            });
            if let Some(label) = label {
                item.label = label;
                item.start += offset;
                if !accepted.iter().any(|a: &Occurrence| {
                    a.kind == item.kind
                        && a.start == item.start
                        && canonical(&a.label) == canonical(&item.label)
                }) {
                    accepted.push(item);
                }
            }
        }
        offset += line.len();
    }
    accepted.sort_by_key(|o| o.start);
    (accepted, decisions)
}

pub(super) fn canonical(label: &str) -> String {
    label
        .trim()
        .trim_end_matches('.')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn host(raw: &str) -> Option<String> {
    let name = raw.trim_end_matches('.').to_ascii_lowercase();
    let labels: Vec<_> = name.split('.').collect();
    let tld = *labels.last()?;
    // Conservative offline suffix policy; no DNS/network or LLM required.
    if labels.len() < 2
        || name.len() > 253
        || name.parse::<IpAddr>().is_ok()
        || !((tld.len() == 2 && !matches!(tld, "md" | "js"))
            || [
                "com", "org", "net", "edu", "gov", "mil", "int", "info", "biz", "online", "site",
                "app", "dev", "social", "io", "ai", "cloud", "tech", "xyz", "example",
            ]
            .contains(&tld))
        || !labels.iter().all(|s| {
            !s.is_empty()
                && s.len() <= 63
                && !s.starts_with('-')
                && !s.ends_with('-')
                && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return None;
    }
    Some(name)
}

fn normalize(item: &Occurrence, line: &str, explicit_theme: bool) -> Option<String> {
    // An explicit theme field is typed data, not a source of invented names.
    if explicit_theme && item.kind != TnaNodeKind::Topic {
        return None;
    }
    let label = item.label.trim().trim_end_matches(['.', ',', ';']);
    let before = &line[..item.start.min(line.len())];
    let token_start = before
        .rfind(char::is_whitespace)
        .map(|p| p + 1)
        .unwrap_or(0);
    let token_end = line[item.start..]
        .find(char::is_whitespace)
        .map(|p| item.start + p)
        .unwrap_or(line.len());
    let token = &line[token_start..token_end];
    // Observables in a URL path are not independent subjects. Only its actual
    // host may survive, and Markdown destinations were already excluded.
    if let Some(url_start) = token.to_ascii_lowercase().find("http") {
        let url =
            url::Url::parse(token[url_start..].trim_end_matches([')', ',', ';', '.'])).ok()?;
        if !matches!(item.kind, TnaNodeKind::Domain | TnaNodeKind::Ip)
            || url
                .host_str()?
                .trim_matches(['[', ']'])
                .to_ascii_lowercase()
                != label.to_ascii_lowercase()
        {
            return None;
        }
    }
    match item.kind {
        TnaNodeKind::Domain => {
            // A host inside an email is identity context, not subject infrastructure.
            if token.contains('@')
                || (token.contains('/') && !token.to_ascii_lowercase().starts_with("http"))
            {
                return None;
            }
            if token.to_ascii_lowercase().starts_with("http") {
                let url = url::Url::parse(token.trim_matches(['(', ')', ',', ';'])).ok()?;
                if url.host_str()?.to_ascii_lowercase() != label.to_ascii_lowercase() {
                    return None;
                }
            } else if before.ends_with(['-', '_', '.']) {
                return None;
            }
            host(label)
        }
        TnaNodeKind::Ip => label
            .parse::<IpAddr>()
            .ok()
            .filter(|ip| !crate::search::ip_blocked(*ip))
            .map(|ip| ip.to_string()),
        TnaNodeKind::Email => {
            let (local, domain) = label.split_once('@')?;
            if local.is_empty()
                || local.starts_with('.')
                || local.ends_with('.')
                || local.contains("..")
            {
                return None;
            }
            Some(format!("{}@{}", local.to_ascii_lowercase(), host(domain)?))
        }
        TnaNodeKind::Handle => {
            if token.contains('/') || token.contains('@') && !token.starts_with('@') {
                return None;
            }
            (label.len() >= 2
                && label.len() <= 39
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_'))
            .then(|| label.to_ascii_lowercase())
        }
        TnaNodeKind::Person | TnaNodeKind::Org | TnaNodeKind::Topic => {
            if item.kind == TnaNodeKind::Topic
                && (!explicit_theme || item.start < line.find(':').map(|p| p + 1).unwrap_or(0))
            {
                return None;
            }
            let words: Vec<_> = label.split_whitespace().collect();
            let max = if item.kind == TnaNodeKind::Topic {
                6
            } else {
                5
            };
            if words.is_empty()
                || words.len() > max
                || label.len() > 72
                || (item.kind == TnaNodeKind::Person && words.len() < 2)
                || label
                    .chars()
                    .any(|c| !c.is_alphabetic() && !" '-’&".contains(c))
                || token.contains('@')
                || token.contains('/')
            {
                return None;
            }
            const STOP: &[&str] = &[
                "report",
                "reports",
                "evidence",
                "requirement",
                "analyst",
                "note",
                "source",
                "sources",
                "list",
                "key",
                "judgments",
                "confidence",
                "public",
                "osint",
                "case",
                "desk",
                "text",
                "network",
                "analysis",
                "strategic",
                "anchor",
                "anchors",
                "selected",
                "graph",
                "outline",
                "table",
                "extract",
                "extraction",
                "prompt",
                "person",
                "organization",
                "theme",
                "themes",
                "no",
                "nothing",
                "who",
                "what",
                "is",
                "are",
                "was",
                "the",
                "this",
                "that",
                "and",
                "for",
                "from",
                "in",
                "on",
                "at",
                "to",
                "with",
                "about",
                "research",
                "generic",
                "unknown",
                "gaps",
                "assumptions",
                "incidental",
                "subject",
                "host",
                "retrieved",
                "a",
                "an",
                "find",
                "findings",
                "review",
                "information",
                "intelligence",
                "financial",
                "assets",
                "rights",
                "reserved",
                "read",
                "more",
                "search",
                "results",
                "social",
                "media",
                "marketing",
                "prof",
                "school",
                "business",
                "data",
                "collection",
                "operations",
                "activity",
                "summary",
                "analysis",
                "detected",
                "community",
                "communities",
                "entity",
                "entities",
                "operates",
                "works",
                "affects",
                "owns",
                "contains",
                "returned",
                "states",
            ];
            if words
                .iter()
                .any(|w| STOP.contains(&w.to_ascii_lowercase().as_str()))
            {
                return None;
            }
            if item.kind == TnaNodeKind::Org
                && words.iter().any(|w| {
                    !["of", "the", "and"].contains(&w.to_ascii_lowercase().as_str())
                        && !w.chars().next().is_some_and(char::is_uppercase)
                })
            {
                return None;
            }
            if item.kind == TnaNodeKind::Person
                && words.iter().any(|w| {
                    [
                        "inc",
                        "llc",
                        "ltd",
                        "corp",
                        "corporation",
                        "foundation",
                        "university",
                        "company",
                    ]
                    .contains(&w.to_ascii_lowercase().as_str())
                })
            {
                return None;
            }
            Some(
                words
                    .iter()
                    .map(|word| {
                        if item.kind == TnaNodeKind::Person
                            && word.chars().all(|c| !c.is_alphabetic() || c.is_lowercase())
                        {
                            let mut chars = word.chars();
                            chars
                                .next()
                                .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
                                .unwrap_or_default()
                        } else {
                            word.to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        }
        TnaNodeKind::Doc => None,
    }
}
