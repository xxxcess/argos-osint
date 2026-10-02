//! Directives and grounded tool inputs (spec addendum A to #27).
//!
//! Each turn has exactly three directives (`d1`–`d3`): goals that say what to establish,
//! never which tool to use. Search inputs are built only from directive entities (verbatim
//! prompt spans, or the thread's subject on a pronoun follow-up), accepted binding values,
//! and one fixed qualifier per target kind. Search results that do not mention the subject
//! cannot add domain, org_name, email, or url bindings.

use serde_json::Value;

use super::super::{Binding, Directive};
use super::{display_name, emails_in, names_subject, question_bindings};

/// Longest directive goal, in words.
pub const MAX_GOAL_WORDS: usize = 15;
/// Search query shape: at most this many words and characters.
pub const MAX_QUERY_WORDS: usize = 6;
pub const MAX_QUERY_CHARS: usize = 80;

/// The fixed qualifier for each target kind; `person_name` takes none.
pub const QUALIFIERS: &[(&str, &str)] = &[
    ("handle", "official account"),
    ("domain", "official website"),
    ("org_name", "company"),
    ("email", "contact"),
    ("person_name", ""),
];

/// Provider and platform-API names a directive goal (or a search query) may not use.
/// Catalog tool ids are checked as well. Account platforms (Twitter, Instagram, …) are
/// not providers here and stay allowed.
pub const PROVIDER_TERMS: &[&str] = &[
    "firecrawl", "sociavault", "hunter", "hunter.io", "wikidata", "keybase", "crt.sh", "crtsh",
    "mnemonic", "hackertarget", "ripestat", "rdap", "arin", "apnic", "wayback", "common crawl",
    "commoncrawl", "arquivo", "github", "gitlab", "grep.app", "grepapp", "gleif", "edgar",
    "stack exchange", "stackexchange", "wikipedia", "nominatim", "overpass", "blockchain.com",
    "blockstream", "mempool", "mempool.space", "nvd", "osv", "sans isc", "shodan", "internetdb",
    "urlscan", "google", "api",
];

const PRONOUNS: &[&str] = &[
    "he", "him", "his", "himself", "she", "her", "hers", "herself", "they", "them", "their",
    "theirs", "it", "its", "this", "that", "these", "those",
];

const QUESTION_WORDS: &[&str] = &[
    "who", "what", "which", "where", "when", "why", "how", "is", "are", "was", "were", "does",
    "do", "did", "can", "could", "should", "would", "will", "has", "have",
];

/// Prompt kinds a directive must target so their tools stay eligible: an IP, CVE,
/// wallet, coordinates, address, package, email, or domain in the prompt.
const PROMPT_TARGETS: &[&str] = &["ip", "cve", "wallet", "coordinates", "address", "package", "email", "domain"];

fn words_of(text: &str) -> Vec<String> {
    text.to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .map(|word| word.trim_matches('.').to_string())
        .filter(|word| !word.is_empty())
        .collect()
}

/// The catalog tool id or provider name `text` uses, if any.
pub fn names_tool(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    for tool in crate::osint::registry() {
        if lower.contains(tool.id) {
            return Some(tool.id.to_string());
        }
    }
    if lower.contains("hunter_tech_lookup") {
        return Some("hunter_tech_lookup".into());
    }
    let words = format!(" {} ", words_of(text).join(" "));
    PROVIDER_TERMS
        .iter()
        .find(|term| if term.contains('.') { lower.contains(*term) } else { words.contains(&format!(" {term} ")) })
        .map(|term| term.to_string())
}

/// The prompt names an entity of its own: an identifier, an email, or a run of two
/// capitalized words ("Jeff Bezos").
fn own_entity(question: &str) -> bool {
    if !super::super::explicit_entities(question).is_empty() || !emails_in(question).is_empty() {
        return true;
    }
    let words: Vec<&str> = question.split_whitespace().collect();
    words.windows(2).any(|pair| {
        pair.iter().all(|word| {
            let bare = word.trim_matches(|ch: char| !ch.is_alphanumeric());
            bare.chars().next().is_some_and(char::is_uppercase)
                && !QUESTION_WORDS.contains(&bare.to_ascii_lowercase().as_str())
                && !PRONOUNS.contains(&bare.to_ascii_lowercase().as_str())
        })
    })
}

/// Leading verbs and articles a subject phrase can carry ("runs Acme Robotics").
const LEAD_WORDS: &[&str] = &[
    "runs", "run", "owns", "own", "founded", "founds", "leads", "manages", "operates", "controls",
    "created", "built", "made", "wrote", "started", "is", "are", "was", "were", "the", "a", "an",
];

/// The prompt's subject phrase without a leading verb or article, cut before a second
/// clause ("jane example and what are her accounts" -> "jane example").
fn subject_phrase(question: &str) -> String {
    let subject = super::super::question_subject(question);
    let words: Vec<&str> = subject.split_whitespace().collect();
    let bare = |word: &str| word.trim_matches(|ch: char| !ch.is_alphanumeric()).to_ascii_lowercase();
    let mut end = words.len();
    for (index, word) in words.iter().enumerate() {
        let next = words.get(index + 1).map(|next| bare(next)).unwrap_or_default();
        if bare(word) == "and" && (QUESTION_WORDS.contains(&next.as_str()) || PRONOUNS.contains(&next.as_str())) {
            end = index;
            break;
        }
    }
    let mut start = 0;
    while start + 1 < end && LEAD_WORDS.contains(&bare(words[start]).as_str()) {
        start += 1;
    }
    let phrase = words[start..end].join(" ");
    phrase.trim_end_matches("'s").trim_end_matches('\u{2019}').trim().to_string()
}

/// A follow-up that points back at the thread's subject ("what about his companies?").
pub fn refers_back(question: &str) -> bool {
    let tokens = words_of(&subject_phrase(question));
    let pronoun = tokens.is_empty() || tokens.iter().any(|word| PRONOUNS.contains(&word.as_str()));
    pronoun && !own_entity(question)
}

/// Directive entities: an identifier the prompt names (domain, IP, email, URL, CVE), else
/// the prompt's subject as typed (title-cased when all lowercase), or on a follow-up whose
/// prompt names no entity of its own, the thread's subject (the previous turn's directive
/// entities).
pub fn directive_entities(question: &str, thread: &[String]) -> Vec<String> {
    if refers_back(question) && !thread.is_empty() {
        return thread.to_vec();
    }
    if super::target_kind(question) == "other" {
        let mut explicit = super::super::explicit_entities(question);
        explicit.sort();
        if let Some((_, value)) = explicit.into_iter().find(|(kind, _)| matches!(kind.as_str(), "domain" | "ip" | "email" | "url" | "cve")) {
            return vec![value];
        }
        if let Some(email) = emails_in(question).into_iter().next() {
            return vec![email];
        }
    }
    let subject = subject_phrase(question);
    if subject.is_empty() || words_of(&subject).iter().all(|word| PRONOUNS.contains(&word.as_str())) {
        return Vec::new();
    }
    vec![display_name(&subject)]
}

fn prompt_targets(question: &str) -> Vec<String> {
    let mut kinds: Vec<String> = Vec::new();
    for binding in question_bindings(question) {
        if PROMPT_TARGETS.contains(&binding.kind.as_str()) && !kinds.contains(&binding.kind) {
            kinds.push(binding.kind);
        }
    }
    kinds
}

fn directive(id: &str, goal: &str, entities: &[String], targets: &[&str], done_when: &str) -> Directive {
    Directive {
        id: id.into(),
        goal: goal.into(),
        entities: entities.to_vec(),
        targets: targets.iter().map(|kind| kind.to_string()).collect(),
        done_when: done_when.into(),
        query: String::new(),
    }
}

/// Three fixed directives when the Recon model is unavailable or its reply fails twice:
/// identity and roles, official accounts and sites, affiliated organizations and contact
/// domains. A prompt identifier (IP, CVE, wallet, …) joins d1's targets.
pub fn fallback_directives(question: &str, thread: &[String]) -> Vec<Directive> {
    let entities = directive_entities(question, thread);
    let mut first = directive(
        "d1",
        "Establish the subject's identity and public roles",
        &entities,
        &["person_name", "org_name", "url"],
        "the subject's identity is confirmed by at least one accepted record",
    );
    for kind in prompt_targets(question) {
        if !first.targets.contains(&kind) {
            first.targets.push(kind);
        }
    }
    vec![
        first,
        directive(
            "d2",
            "Find the subject's official online accounts and websites",
            &entities,
            &["handle", "domain", "url"],
            "at least one subject-owned handle or domain is accepted",
        ),
        directive(
            "d3",
            "Find organizations affiliated with the subject and their contact domains",
            &entities,
            &["org_name", "domain", "email"],
            "at least one affiliated organization or contact domain is accepted",
        ),
    ]
}

/// Why a goal is not a valid directive goal, if it is not.
pub fn goal_error(goal: &str) -> Option<String> {
    let goal = goal.trim();
    let words: Vec<&str> = goal.split_whitespace().collect();
    if words.is_empty() {
        return Some("the goal is empty".into());
    }
    if words.len() > MAX_GOAL_WORDS {
        return Some(format!("the goal has {} words; at most {MAX_GOAL_WORDS}", words.len()));
    }
    let first = words[0].trim_matches(|ch: char| !ch.is_alphanumeric()).to_ascii_lowercase();
    if goal.ends_with('?') || QUESTION_WORDS.contains(&first.as_str()) {
        return Some("the goal must be an imperative, not a question".into());
    }
    names_tool(goal).map(|term| format!("the goal names a tool or provider ({term})"))
}

/// Validates a Recon reply: exactly three directives `d1`–`d3`, tool-free imperative goals
/// of at most 15 words, entities that are verbatim prompt spans (or the thread subject on
/// a follow-up), and targets from the binding vocabulary. A Recon-written `query` that
/// breaks the grounded-query rules is dropped (the deterministic query is used).
pub fn parse_directives(value: &Value, question: &str, thread: &[String]) -> Result<Vec<Directive>, String> {
    let list = value
        .get("directives")
        .and_then(Value::as_array)
        .ok_or("the reply has no directives array")?;
    if list.len() != 3 {
        return Err(format!("expected exactly 3 directives, got {}", list.len()));
    }
    let lower_question = question.to_ascii_lowercase();
    let fallback_entities = directive_entities(question, thread);
    let mut directives = Vec::new();
    for (index, item) in list.iter().enumerate() {
        let mut parsed: Directive = serde_json::from_value(item.clone()).map_err(|err| format!("directive {} is malformed: {err}", index + 1))?;
        let expected = format!("d{}", index + 1);
        if parsed.id != expected {
            return Err(format!("directive {} must have id {expected}", index + 1));
        }
        if let Some(error) = goal_error(&parsed.goal) {
            return Err(format!("{expected}: {error}"));
        }
        parsed.goal = parsed.goal.trim().to_string();
        let mut entities: Vec<String> = Vec::new();
        for entity in &parsed.entities {
            let entity = entity.trim();
            if entity.is_empty() {
                continue;
            }
            let verbatim = lower_question.contains(&entity.to_ascii_lowercase());
            let thread_subject = thread.iter().any(|known| known.eq_ignore_ascii_case(entity));
            if !verbatim && !thread_subject {
                return Err(format!("{expected}: entity \"{entity}\" is not in the user's prompt"));
            }
            if names_tool(entity).is_some() {
                return Err(format!("{expected}: entity \"{entity}\" names a tool or provider"));
            }
            let shown = display_name(entity);
            if !entities.contains(&shown) {
                entities.push(shown);
            }
        }
        // A pronoun follow-up keeps the thread subject even when Recon copied the pronoun.
        if entities.is_empty() || refers_back(question) && !thread.is_empty() {
            entities = fallback_entities.clone();
        }
        parsed.entities = entities;
        if parsed.targets.is_empty() {
            return Err(format!("{expected} has no targets"));
        }
        for kind in &parsed.targets {
            if !super::known_kind(kind) {
                return Err(format!("{expected} targets {kind}, which is not in the binding vocabulary"));
            }
        }
        if parsed.done_when.chars().count() > 200 {
            parsed.done_when = parsed.done_when.chars().take(200).collect();
        }
        if !parsed.query.is_empty() && !grounded_query(&parsed.query, &parsed.entities, &[]) {
            parsed.query.clear();
        }
        directives.push(parsed);
    }
    for kind in prompt_targets(question) {
        if !directives.iter().any(|item| item.targets.contains(&kind)) {
            directives[0].targets.push(kind);
        }
    }
    Ok(directives)
}

/// The fixed qualifier for a directive: the first of its targets with a qualifier entry.
pub fn qualifier_for(directive: &Directive) -> &'static str {
    directive
        .targets
        .iter()
        .find_map(|kind| QUALIFIERS.iter().find(|(known, _)| known == kind).map(|(_, qualifier)| *qualifier))
        .unwrap_or("")
}

/// Whether a search query keeps to the grounded shape: at most 6 words and 80 characters,
/// built from directive entities or accepted binding values plus at most one fixed
/// qualifier, with no question words and no tool or provider names.
pub fn grounded_query(query: &str, entities: &[String], values: &[String]) -> bool {
    let query = query.trim();
    if query.is_empty() || query.chars().count() > MAX_QUERY_CHARS || query.split_whitespace().count() > MAX_QUERY_WORDS || query.contains('?') {
        return false;
    }
    if names_tool(query).is_some_and(|term| !entities.iter().chain(values).any(|known| known.to_ascii_lowercase().contains(&term))) {
        return false;
    }
    let mut rest = format!(" {} ", query.to_ascii_lowercase());
    let mut grounded: Vec<String> = entities.iter().chain(values).map(|value| value.trim().to_ascii_lowercase()).filter(|value| !value.is_empty()).collect();
    grounded.sort_by_key(|value| std::cmp::Reverse(value.len()));
    let mut anchored = false;
    for value in grounded {
        let needle = format!(" {value} ");
        while let Some(at) = rest.find(&needle) {
            rest.replace_range(at..at + needle.len(), "  ");
            anchored = true;
        }
    }
    let rest = rest.split_whitespace().collect::<Vec<_>>().join(" ");
    anchored && (rest.is_empty() || QUALIFIERS.iter().any(|(_, qualifier)| !qualifier.is_empty() && *qualifier == rest))
}

/// A query and the label of where it came from, e.g. `d2 entity + qualifier`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroundedQuery {
    pub query: String,
    pub source: String,
}

fn clip_words(value: &str, words: usize) -> String {
    let clipped: Vec<&str> = value.split_whitespace().take(words.max(1)).collect();
    clipped.join(" ").chars().take(MAX_QUERY_CHARS).collect::<String>().trim().to_string()
}

/// The query a directive gives a search tool: Recon's query when it is grounded, else
/// `<entity>` or `<entity> <qualifier>`. Platform searches (`qualified = false`) send the
/// entity alone. `None` without a directive entity.
pub fn directive_query(directive: &Directive, qualified: bool) -> Option<GroundedQuery> {
    let entity = directive.entities.first()?;
    let label = |query: &str| {
        let bare = directive.entities.iter().any(|known| known.eq_ignore_ascii_case(query.trim()));
        format!("{} entity{}", directive.id, if bare { "" } else { " + qualifier" })
    };
    if qualified && !directive.query.is_empty() && grounded_query(&directive.query, &directive.entities, &[]) {
        return Some(GroundedQuery { query: directive.query.trim().to_string(), source: label(&directive.query) });
    }
    let qualifier = if qualified { qualifier_for(directive) } else { "" };
    let room = MAX_QUERY_WORDS - qualifier.split_whitespace().count();
    let entity = clip_words(entity, room);
    let query = if qualifier.is_empty() { entity } else { format!("{entity} {qualifier}") };
    let source = label(&query);
    Some(GroundedQuery { query, source })
}

/// The directive a search for `target` serves: the first one targeting that kind.
pub fn directive_for_target<'a>(directives: &'a [Directive], target: &str) -> Option<&'a Directive> {
    directives.iter().find(|item| item.targets.iter().any(|kind| kind == target))
}

/// Binding kinds a search result must earn by mentioning the subject.
pub const GATED_KINDS: &[&str] = &["domain", "org_name", "email", "url"];
/// Tools whose observation is a list of search results.
pub const SEARCH_RESULT_TOOLS: &[&str] = &["firecrawl_search", "sociavault_google_search", "sociavault_search"];

/// The result names the subject: its title, description, or URL contains a directive
/// entity, or every name token of one.
fn result_mentions(row: &Value, entities: &[String]) -> bool {
    let field = |key: &str| row.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let text = [field("title"), field("description"), field("snippet"), field("url")].join(" ");
    let lower = text.to_ascii_lowercase();
    let compact: String = lower.chars().filter(char::is_ascii_alphanumeric).collect();
    entities.iter().any(|entity| {
        let entity_lower = entity.to_ascii_lowercase();
        let entity_compact: String = entity_lower.chars().filter(char::is_ascii_alphanumeric).collect();
        lower.contains(&entity_lower) || !entity_compact.is_empty() && compact.contains(&entity_compact) || names_subject(entity, &text)
    })
}

/// Relevance gate for search results: a domain, org_name, email, or url binding from a
/// search tool is kept only when a result that contains the value also mentions the
/// subject. Returns the kept and the dropped bindings. Other tools pass unchanged.
pub fn relevance_gate(tool_id: &str, entities: &[String], observations: &Value, bindings: Vec<Binding>) -> (Vec<Binding>, Vec<Binding>) {
    let tool = crate::osint::canonical_tool_id(tool_id);
    let rows: Vec<&Value> = observations.get("results").and_then(Value::as_array).map(|rows| rows.iter().collect()).unwrap_or_default();
    if !SEARCH_RESULT_TOOLS.contains(&tool) || rows.is_empty() || entities.is_empty() {
        return (bindings, Vec::new());
    }
    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    for binding in bindings {
        if !GATED_KINDS.contains(&binding.kind.as_str()) {
            kept.push(binding);
            continue;
        }
        let needle = binding.value.to_ascii_lowercase();
        let relevant = rows
            .iter()
            .filter(|row| row.to_string().to_ascii_lowercase().contains(&needle))
            .any(|row| result_mentions(row, entities));
        if relevant {
            kept.push(binding);
        } else {
            dropped.push(binding);
        }
    }
    (kept, dropped)
}

