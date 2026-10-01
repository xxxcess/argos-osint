//! Strategy selection, entity ranking, and grounded tool choice for one Recon turn.
//! Priorities are explicit rules. They are not calibrated probabilities.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::osint;

pub const DISCOVERY: &str = "discovery";
pub const HYPOTHESIS: &str = "hypothesis";
pub const ADAPTIVE: &str = "adaptive";

pub fn strategy_label(kind: &str) -> &'static str {
    match kind {
        HYPOTHESIS => "Question and hypothesis testing",
        ADAPTIVE => "Adaptive expansion by information value",
        _ => "Discovery and selective enrichment",
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrategyChoice {
    pub kind: String,
    pub rationale: String,
}

pub fn select_strategy(
    question: &str,
    opening: bool,
    useful_evidence: bool,
    unfamiliar: bool,
) -> StrategyChoice {
    if hypothesis_question(question) {
        return StrategyChoice {
            kind: HYPOTHESIS.into(),
            rationale: "The question poses competing explanations, so this turn looks for evidence that distinguishes them.".into(),
        };
    }
    let broad = super::is_broad_question(question);
    if opening && broad {
        return StrategyChoice {
            kind: DISCOVERY.into(),
            rationale: "The question is broad or the subject is still unfamiliar, so this turn identifies the subject and a few related entities.".into(),
        };
    }
    if opening && unfamiliar && !has_concrete_identifier(question) {
        return StrategyChoice {
            kind: DISCOVERY.into(),
            rationale: "Little is known about the subject yet, so this turn builds a bounded overview before enrichment.".into(),
        };
    }
    if useful_evidence || !broad {
        return StrategyChoice {
            kind: ADAPTIVE.into(),
            rationale: "The question is focused or useful evidence is already on hand, so this turn picks the next lookup by what it can add relative to its cost.".into(),
        };
    }
    StrategyChoice {
        kind: DISCOVERY.into(),
        rationale: "The subject still needs a bounded overview before narrower lookups.".into(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolSuggestion {
    pub tool_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnswerAssessment {
    pub answered: bool,
    pub tools: Vec<ToolSuggestion>,
}

/// Tools that could supply context the current results do not. At least three when the registry allows it.
pub fn additional_tools(
    question: &str,
    used: &HashSet<String>,
    enabled: &HashSet<String>,
) -> Vec<ToolSuggestion> {
    let kind = gap_kind(question);
    let tokens = content_tokens(question);
    let mut ranked: Vec<(i32, &osint::ToolDefinition)> = osint::registry()
        .iter()
        .filter(|tool| enabled.contains(tool.id) && !used.contains(tool.id))
        .map(|tool| {
            let blob = format!(
                "{} {} {}",
                tool.name.to_ascii_lowercase(),
                tool.category.to_ascii_lowercase(),
                tool.description.to_ascii_lowercase()
            );
            let mut score = 0;
            for token in &tokens {
                if blob.contains(token.as_str()) {
                    score += 2;
                }
            }
            if category_for_gap(tool.category, kind) {
                score += 5;
            }
            (score, tool)
        })
        .collect();
    ranked.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.id.cmp(right.1.id)));
    let mut tools: Vec<ToolSuggestion> = ranked
        .into_iter()
        .take(3)
        .map(|(_, tool)| ToolSuggestion {
            tool_id: tool.id.into(),
            reason: format!(
                "{} ({}) can add context the current results do not cover.",
                tool.name, tool.category
            ),
        })
        .collect();
    if tools.len() < 3 {
        for tool in osint::registry() {
            if tools.len() >= 3 {
                break;
            }
            if !enabled.contains(tool.id) || tools.iter().any(|item| item.tool_id == tool.id) {
                continue;
            }
            tools.push(ToolSuggestion {
                tool_id: tool.id.into(),
                reason: format!(
                    "{} ({}) is another enabled source for this question.",
                    tool.name, tool.category
                ),
            });
        }
    }
    tools
}

pub fn assessment_from_model(value: &Value, fallback: &[ToolSuggestion]) -> Option<AnswerAssessment> {
    let answered = value.get("answered")?.as_bool()?;
    if answered {
        return Some(AnswerAssessment {
            answered: true,
            tools: Vec::new(),
        });
    }
    let mut tools: Vec<ToolSuggestion> = Vec::new();
    if let Some(rows) = value.get("tools").and_then(Value::as_array) {
        for row in rows {
            let Some(id) = row.get("tool_id").and_then(Value::as_str) else {
                continue;
            };
            if osint::definition(id).is_none() || tools.iter().any(|tool| tool.tool_id == id) {
                continue;
            }
            let reason = row
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            if reason.is_empty() {
                continue;
            }
            tools.push(ToolSuggestion {
                tool_id: id.into(),
                reason: reason.into(),
            });
        }
    }
    for suggestion in fallback {
        if tools.len() >= 3 {
            break;
        }
        if tools.iter().any(|tool| tool.tool_id == suggestion.tool_id) {
            continue;
        }
        tools.push(suggestion.clone());
    }
    if tools.len() < 3 {
        return None;
    }
    Some(AnswerAssessment {
        answered: false,
        tools,
    })
}

fn category_for_gap(category: &str, kind: &str) -> bool {
    match kind {
        "infrastructure" | "registration" => category == "Domains" || category == "Networks",
        "archive" => category == "Archives",
        "filing" | "ownership" | "identity" | "overview" => category == "Organizations",
        "profile" => category == "Social" || category == "Identities",
        "contacts" | "technology" | "deliverability" => category == "Enrichment",
        "place" => category == "Places",
        "vulnerability" => category == "Vulnerabilities",
        "code" => category == "Code",
        "bitcoin" => category == "Bitcoin",
        _ => category == "Web",
    }
}

pub fn strategy_change_reason(previous: &str, next: &StrategyChoice) -> Option<String> {
    if previous.is_empty() || previous == next.kind {
        None
    } else {
        Some(format!(
            "Strategy changed from {} to {}. {}",
            strategy_label(previous),
            strategy_label(&next.kind),
            next.rationale
        ))
    }
}

fn hypothesis_question(question: &str) -> bool {
    let question = question.to_ascii_lowercase();
    [
        " or ",
        "versus",
        " vs ",
        "which of",
        "same person",
        "same entity",
        "alias",
        "who owns",
        "owner of",
        "belongs to",
        "related to",
        "affiliated",
        "conflicting",
        "really ",
        "actually ",
    ]
    .iter()
    .any(|cue| question.contains(cue))
}

fn has_concrete_identifier(question: &str) -> bool {
    super::explicit_entities(question)
        .iter()
        .any(|(kind, _)| matches!(kind.as_str(), "domain" | "url" | "ip" | "cve"))
        || question.contains('@')
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveryQuery {
    pub role: String,
    pub query: String,
    pub angle: String,
}

pub fn complementary_queries(question: &str, strategy: &str) -> [DiscoveryQuery; 2] {
    let subject = subject_of(question);
    let identity = clip_query(&format!("{subject} official name identifiers"));
    let mut investigative = clip_query(&investigative_angle(question, &subject, strategy));
    if !distinct_queries(&identity, &investigative) {
        investigative = clip_query(&format!(
            "{subject} independent records and contemporaneous reporting"
        ));
    }
    [
        DiscoveryQuery {
            role: "identity".into(),
            query: identity,
            angle: "Establish the subject and its authoritative identifiers.".into(),
        },
        DiscoveryQuery {
            role: "investigative".into(),
            query: investigative,
            angle: "Address the requested relationship, activity, event, or competing explanation."
                .into(),
        },
    ]
}

pub fn distinct_queries(left: &str, right: &str) -> bool {
    let left_tokens = content_tokens(left);
    let right_tokens = content_tokens(right);
    !left.trim().eq_ignore_ascii_case(right.trim())
        && !left_tokens.is_empty()
        && left_tokens != right_tokens
}

fn investigative_angle(question: &str, subject: &str, strategy: &str) -> String {
    let lower = question.to_ascii_lowercase();
    if lower.contains("own") {
        format!("{subject} ownership registration corporate parent")
    } else if lower.contains("email") || lower.contains("contact") {
        format!("{subject} professional contact company domain")
    } else if social_words(&lower) {
        format!("{subject} public social profile accounts")
    } else if lower.contains("where")
        || lower.contains("locat")
        || lower.contains("address")
        || lower.contains("headquarter")
    {
        format!("{subject} headquarters location address")
    } else if lower.contains("when")
        || lower.contains("found")
        || lower.contains("start")
        || lower.contains("history")
    {
        format!("{subject} founding history timeline")
    } else if lower.contains("certificat") || lower.contains("subdomain") || lower.contains("dns") {
        format!("{subject} infrastructure hosts certificates")
    } else if strategy == HYPOTHESIS {
        format!("{subject} conflicting claims compared")
    } else {
        let predicate = predicate_words(question, subject);
        if predicate.is_empty() {
            format!("{subject} recent activity and public records")
        } else {
            format!("{subject} {predicate}")
        }
    }
}

fn social_words(question: &str) -> bool {
    [
        "social", "instagram", "tiktok", "twitter", "facebook", "linkedin", "youtube", "profile",
    ]
    .iter()
    .any(|word| question.contains(word))
}

fn predicate_words(question: &str, subject: &str) -> String {
    let subject_tokens = content_tokens(subject);
    content_tokens(question)
        .into_iter()
        .filter(|token| !subject_tokens.contains(token))
        .take(6)
        .collect::<Vec<_>>()
        .join(" ")
}

fn subject_of(question: &str) -> String {
    let subject = super::question_subject(question);
    let subject = if subject.is_empty() {
        question.trim().to_string()
    } else {
        subject
    };
    clip_query(&subject)
}

fn clip_query(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_control())
        .take(180)
        .collect::<String>()
        .trim()
        .to_string()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvestigationFrame {
    pub subject: String,
    pub objective: String,
    pub constraints: String,
    pub known: Vec<String>,
    pub unresolved: Vec<String>,
}

pub fn investigation_frame(question: &str, known: &[String]) -> InvestigationFrame {
    let subject = subject_of(question);
    let years: Vec<_> = question
        .split_whitespace()
        .filter(|word| {
            let word = word.trim_matches(|ch: char| !ch.is_ascii_digit());
            word.len() == 4 && word.chars().all(|ch| ch.is_ascii_digit())
        })
        .map(|word| word.trim_matches(|ch: char| !ch.is_ascii_digit()).to_string())
        .collect();
    let mut constraints = Vec::new();
    if !years.is_empty() {
        constraints.push(format!("years {}", years.join(", ")));
    }
    let lower = question.to_ascii_lowercase();
    for (needle, label) in [
        ("united states", "United States"),
        (" in the us", "United States"),
        ("europe", "Europe"),
        ("united kingdom", "United Kingdom"),
    ] {
        if lower.contains(needle) {
            constraints.push(label.into());
        }
    }
    InvestigationFrame {
        subject,
        objective: question.trim().to_string(),
        constraints: if constraints.is_empty() {
            "none stated".into()
        } else {
            constraints.join("; ")
        },
        known: known.iter().take(8).cloned().collect(),
        unresolved: vec![question.trim().to_string()],
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub evidence_id: String,
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub retrieved_at: String,
    pub query_role: String,
}

pub fn dedupe_hits(hits: Vec<SearchHit>) -> Vec<SearchHit> {
    let mut seen = HashSet::new();
    let mut kept = Vec::new();
    for hit in hits {
        if hit.url.is_empty() {
            continue;
        }
        if seen.insert(normalize_url(&hit.url)) {
            kept.push(hit);
        }
    }
    kept
}

fn normalize_url(url: &str) -> String {
    let Ok(parsed) = url::Url::parse(url) else {
        return url.to_ascii_lowercase();
    };
    let host = parsed
        .host_str()
        .unwrap_or("")
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    let path = parsed.path().trim_end_matches('/');
    format!("{host}{path}")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityIdentifier {
    pub kind: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedEntity {
    pub canonical_name: String,
    pub entity_type: String,
    pub identifiers: Vec<EntityIdentifier>,
    pub evidence_ids: Vec<String>,
    pub relationships: Vec<String>,
    pub unresolved: Vec<String>,
    pub certainty: String,
    pub why: String,
    pub ambiguous: bool,
    pub selected: bool,
}

pub fn select_entities(question: &str, hits: &[SearchHit]) -> Vec<SelectedEntity> {
    let subject = subject_of(question);
    let mut groups: Vec<Candidate> = Vec::new();
    for hit in hits {
        absorb_hit(&mut groups, &subject, hit);
    }
    let mut entities: Vec<SelectedEntity> = groups
        .into_iter()
        .filter_map(|candidate| candidate.finish(&subject))
        .collect();
    let person_names: Vec<String> = entities
        .iter()
        .filter(|entity| entity.entity_type == "person" && entity.selected)
        .map(|entity| entity.canonical_name.to_ascii_lowercase())
        .collect();
    if person_names.len() > 1
        && person_names
            .iter()
            .all(|name| matches_subject(&subject, name))
    {
        for entity in &mut entities {
            if entity.entity_type == "person" {
                entity.ambiguous = true;
                entity.certainty = "low".into();
                if !entity
                    .unresolved
                    .iter()
                    .any(|item| item == "ambiguous identity")
                {
                    entity
                        .unresolved
                        .push("ambiguous identity; not used as a lookup input".into());
                }
            }
        }
    }
    entities.sort_by_key(rank_key);
    let mut chosen = 0;
    for entity in &mut entities {
        let keep = entity.selected
            && entity.certainty != "low"
            && !entity.ambiguous
            && chosen < 3;
        entity.selected = keep;
        if keep {
            chosen += 1;
        }
    }
    entities
}

fn rank_key(entity: &SelectedEntity) -> (u8, usize, String) {
    let certainty = match entity.certainty.as_str() {
        "high" => 0,
        "medium" => 1,
        _ => 2,
    };
    (
        certainty,
        usize::MAX - entity.evidence_ids.len(),
        entity.canonical_name.to_ascii_lowercase(),
    )
}

struct Candidate {
    name: String,
    entity_type: String,
    identifiers: Vec<EntityIdentifier>,
    evidence_ids: Vec<String>,
    roles: HashSet<String>,
    relationships: Vec<String>,
    why: Vec<String>,
    sources: usize,
    own_domain: bool,
    matched: bool,
}

impl Candidate {
    fn finish(self, subject: &str) -> Option<SelectedEntity> {
        if !self.matched || self.name.trim().is_empty() {
            return None;
        }
        let certainty = if self.ambiguous_source() {
            "low"
        } else if self.own_domain || (self.sources >= 2 && self.matched) {
            "high"
        } else {
            "medium"
        };
        let mut why = self.why;
        if self.roles.len() > 1 {
            why.push(
                "Named in both the identity search and the investigative search.".into(),
            );
        }
        let mut unresolved = Vec::new();
        if certainty == "medium" {
            unresolved.push("needs a second independent source".into());
        }
        if !self.own_domain && self.entity_type == "organization" {
            unresolved.push("no company domain confirmed".into());
        }
        Some(SelectedEntity {
            canonical_name: self.name,
            entity_type: self.entity_type,
            identifiers: self.identifiers,
            evidence_ids: self.evidence_ids,
            relationships: if self.relationships.is_empty() {
                vec![format!("mentioned in connection with {subject}")]
            } else {
                self.relationships
            },
            unresolved,
            certainty: certainty.into(),
            why: why.join(" "),
            ambiguous: false,
            selected: certainty != "low",
        })
    }

    fn ambiguous_source(&self) -> bool {
        !self.matched
    }
}

fn absorb_hit(groups: &mut Vec<Candidate>, subject: &str, hit: &SearchHit) {
    let Ok(url) = url::Url::parse(&hit.url) else {
        return;
    };
    let host = url
        .host_str()
        .unwrap_or("")
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    if let Some(handle) = super::extract_social_handles(std::slice::from_ref(&hit.url))
        .into_iter()
        .next()
    {
        let name = title_name(&hit.title).unwrap_or_else(|| handle.handle.clone());
        if !matches_subject(subject, &name) && !matches_subject(subject, &handle.handle) {
            return;
        }
        let candidate = upsert(groups, &name, "person");
        push_id(
            candidate,
            EntityIdentifier {
                kind: handle.platform,
                value: handle.handle,
            },
        );
        note_hit(candidate, hit, true, false);
        candidate
            .why
            .push("A social profile in the search results matches the subject.".into());
        candidate.relationships.push(format!(
            "public profile discovered for {}",
            candidate.name
        ));
        return;
    }
    if super::enrichable_domain(&host) {
        let title = title_name(&hit.title);
        let name = title
            .filter(|title| matches_subject(subject, title) || matches_subject(subject, &host))
            .unwrap_or_else(|| domain_label(&host));
        let mentioned = matches_subject(subject, &name)
            || matches_subject(subject, &host)
            || matches_subject(subject, &hit.title)
            || matches_subject(subject, &hit.snippet);
        if !mentioned {
            return;
        }
        let candidate = upsert(groups, &name, "organization");
        push_id(
            candidate,
            EntityIdentifier {
                kind: "domain".into(),
                value: host.clone(),
            },
        );
        note_hit(candidate, hit, true, true);
        candidate.why.push(format!(
            "The domain {host} is treated as an identifier of this entity, not of the publisher."
        ));
        candidate
            .relationships
            .push(format!("{host} is a site for {}", candidate.name));
        return;
    }
    if let Some(name) = title_name(&hit.title) {
        if matches_subject(subject, &name) {
            let candidate = upsert(groups, &name, "person");
            note_hit(candidate, hit, true, false);
            candidate.why.push(
                "The subject is named by a source page. The publisher domain is not an identifier."
                    .into(),
            );
        }
    }
}

fn upsert<'a>(groups: &'a mut Vec<Candidate>, name: &str, entity_type: &str) -> &'a mut Candidate {
    let key = name.to_ascii_lowercase();
    if let Some(index) = groups
        .iter()
        .position(|candidate| candidate.name.eq_ignore_ascii_case(&key))
    {
        return &mut groups[index];
    }
    groups.push(Candidate {
        name: name.trim().to_string(),
        entity_type: entity_type.into(),
        identifiers: Vec::new(),
        evidence_ids: Vec::new(),
        roles: HashSet::new(),
        relationships: Vec::new(),
        why: Vec::new(),
        sources: 0,
        own_domain: false,
        matched: false,
    });
    groups.last_mut().expect("candidate just pushed")
}

fn note_hit(candidate: &mut Candidate, hit: &SearchHit, matched: bool, own_domain: bool) {
    if !candidate
        .evidence_ids
        .iter()
        .any(|id| id == &hit.evidence_id)
    {
        candidate.evidence_ids.push(hit.evidence_id.clone());
        candidate.sources += 1;
    }
    if !hit.query_role.is_empty() {
        candidate.roles.insert(hit.query_role.clone());
    }
    candidate.matched |= matched;
    candidate.own_domain |= own_domain;
}

fn push_id(candidate: &mut Candidate, identifier: EntityIdentifier) {
    if !candidate
        .identifiers
        .iter()
        .any(|existing| existing.kind == identifier.kind && existing.value == identifier.value)
    {
        candidate.identifiers.push(identifier);
    }
}

fn title_name(title: &str) -> Option<String> {
    let head = title
        .split(['|', '-', '—', '–', ':'])
        .next()?
        .trim();
    let head = head.split("(@").next()?.trim();
    let head = head.trim_matches(|ch: char| matches!(ch, '"' | '\'' | ',' | '.'));
    let words = head.split_whitespace().count();
    if head.len() > 80 || words == 0 || words > 6 {
        None
    } else {
        Some(head.to_string())
    }
}

fn domain_label(domain: &str) -> String {
    domain
        .split('.')
        .next()
        .unwrap_or(domain)
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect()
}

fn matches_subject(subject: &str, value: &str) -> bool {
    let subject_tokens = content_tokens(subject);
    if subject_tokens.is_empty() {
        return false;
    }
    let value_tokens = content_tokens(value);
    if !subject_tokens.is_disjoint(&value_tokens) {
        return true;
    }
    let compact: String = value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    subject_tokens.iter().all(|token| compact.contains(token))
}

fn content_tokens(value: &str) -> HashSet<String> {
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|word| word.len() > 2)
        .map(|word| word.to_ascii_lowercase())
        .filter(|word| {
            !matches!(
                word.as_str(),
                "the"
                    | "and"
                    | "for"
                    | "who"
                    | "what"
                    | "when"
                    | "where"
                    | "how"
                    | "why"
                    | "are"
                    | "was"
                    | "were"
                    | "did"
                    | "does"
                    | "with"
                    | "from"
                    | "that"
                    | "this"
                    | "official"
                    | "name"
                    | "identifiers"
                    | "about"
                    | "their"
                    | "have"
                    | "has"
            )
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alternative {
    pub id: String,
    pub statement: String,
    pub distinctive: String,
    pub supporting: Vec<String>,
    pub contradicting: Vec<String>,
    pub missing: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HypothesisRecord {
    pub question: String,
    pub alternatives: Vec<Alternative>,
    pub status: String,
}

pub fn draft_hypotheses(question: &str) -> HypothesisRecord {
    let alternatives = if let Some((left, right)) = split_or(question) {
        vec![
            alternative("alt-1", &left.statement, &left.distinctive),
            alternative("alt-2", &right.statement, &right.distinctive),
        ]
    } else if question.to_ascii_lowercase().contains("own") {
        let subject = subject_of(question);
        vec![
            alternative(
                "alt-owner",
                &format!("{subject} is the owner"),
                &subject,
            ),
            alternative(
                "alt-other",
                "A different party is the owner",
                "different party",
            ),
        ]
    } else {
        vec![
            alternative("alt-yes", "The claim in the question is supported", "supported"),
            alternative(
                "alt-no",
                "The claim in the question is not supported",
                "not supported",
            ),
        ]
    };
    HypothesisRecord {
        question: question.trim().to_string(),
        alternatives,
        status: "unresolved".into(),
    }
}

struct NamedAlternative {
    statement: String,
    distinctive: String,
}

fn split_or(question: &str) -> Option<(NamedAlternative, NamedAlternative)> {
    let lower = question.to_ascii_lowercase();
    let index = lower.find(" or ")?;
    let left = question[..index].trim();
    let right = question[index + 4..].trim().trim_end_matches(['?', '.']);
    let left_name = trailing_name(left).unwrap_or_else(|| strip_question(left));
    let (right_name, predicate) = leading_name(right);
    let predicate = predicate.trim().trim_end_matches(['?', '.']);
    let left_statement = if predicate.is_empty() {
        left_name.clone()
    } else {
        format!("{left_name} {predicate}")
    };
    let right_statement = if predicate.is_empty() {
        right_name.clone()
    } else {
        format!("{right_name} {predicate}")
    };
    if left_name.is_empty() || right_name.is_empty() || left_name.eq_ignore_ascii_case(&right_name)
    {
        return None;
    }
    Some((
        NamedAlternative {
            statement: left_statement,
            distinctive: left_name,
        },
        NamedAlternative {
            statement: right_statement,
            distinctive: right_name,
        },
    ))
}

fn trailing_name(value: &str) -> Option<String> {
    let words: Vec<&str> = value.split_whitespace().collect();
    let mut name = Vec::new();
    for word in words.into_iter().rev() {
        let bare = word.trim_matches(|ch: char| !ch.is_ascii_alphanumeric());
        if question_word(bare) {
            if !name.is_empty() {
                break;
            }
            continue;
        }
        if bare.chars().next().is_some_and(|ch| ch.is_uppercase()) {
            name.push(bare);
        } else if !name.is_empty() {
            break;
        }
    }
    if name.is_empty() {
        None
    } else {
        name.reverse();
        Some(name.join(" "))
    }
}

fn leading_name(value: &str) -> (String, String) {
    let mut name = Vec::new();
    let mut rest = Vec::new();
    let mut named = false;
    for word in value.split_whitespace() {
        let bare = word.trim_matches(|ch: char| !ch.is_ascii_alphanumeric());
        if !named && bare.chars().next().is_some_and(|ch| ch.is_uppercase()) {
            name.push(bare);
        } else {
            named = true;
            rest.push(word.trim_matches(|ch: char| matches!(ch, '?' | '.')));
        }
    }
    (name.join(" "), rest.join(" "))
}

fn question_word(word: &str) -> bool {
    let lower = word
        .trim_matches(|ch: char| !ch.is_ascii_alphabetic())
        .to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "who" | "what" | "where" | "when" | "how" | "why" | "does" | "did" | "do" | "is" | "are"
            | "was" | "were" | "has" | "have" | "can"
    )
}

fn strip_question(value: &str) -> String {
    let first = value.split_whitespace().next().unwrap_or("");
    let lower = first
        .trim_matches(|ch: char| !ch.is_ascii_alphabetic())
        .to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "who" | "what" | "where" | "when" | "how" | "why" | "does" | "did" | "is" | "are"
            | "was" | "were"
    ) {
        value.split_whitespace().skip(1).collect::<Vec<_>>().join(" ")
    } else {
        value.to_string()
    }
}

fn alternative(id: &str, statement: &str, distinctive: &str) -> Alternative {
    Alternative {
        id: id.into(),
        statement: statement.into(),
        distinctive: distinctive.into(),
        supporting: Vec::new(),
        contradicting: Vec::new(),
        missing: Vec::new(),
    }
}

pub fn classify_hypothesis(record: &mut HypothesisRecord, notes: &[(String, String)]) {
    for alternative in &mut record.alternatives {
        alternative.supporting.clear();
        alternative.contradicting.clear();
        alternative.missing.clear();
        let tokens = content_tokens(&alternative.distinctive);
        for (id, text) in notes {
            let lower = format!(" {} ", text.to_ascii_lowercase());
            let mentions = !tokens.is_empty()
                && tokens.iter().all(|token| {
                    lower.contains(token)
                        || lower
                            .chars()
                            .filter(|ch| ch.is_ascii_alphanumeric())
                            .collect::<String>()
                            .contains(token)
                });
            if !mentions {
                continue;
            }
            if contradiction(&lower) {
                alternative.contradicting.push(id.clone());
            } else {
                alternative.supporting.push(id.clone());
            }
        }
        if alternative.supporting.is_empty() && alternative.contradicting.is_empty() {
            alternative
                .missing
                .push("No retrieved evidence addresses this alternative.".into());
        }
    }
    record.status = hypothesis_status(&record.alternatives).into();
}

fn contradiction(text: &str) -> bool {
    [
        " not ",
        " no longer",
        "n't ",
        "denied",
        "unrelated",
        "former ",
        "never owned",
        "sold ",
        "does not own",
        "did not own",
        "is not the",
    ]
    .iter()
    .any(|phrase| text.contains(phrase))
}

pub fn hypothesis_status(alternatives: &[Alternative]) -> &'static str {
    if alternatives.is_empty() {
        return "unresolved";
    }
    let supported = alternatives
        .iter()
        .filter(|alternative| {
            !alternative.supporting.is_empty() && alternative.contradicting.is_empty()
        })
        .count();
    let contradicted = alternatives
        .iter()
        .filter(|alternative| {
            !alternative.contradicting.is_empty() && alternative.supporting.is_empty()
        })
        .count();
    if supported == 1 && contradicted == alternatives.len() - 1 {
        "supported"
    } else {
        "unresolved"
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gap {
    pub id: String,
    pub question: String,
    pub kind: String,
}

pub fn gap_kind(question: &str) -> &'static str {
    let question = question.to_ascii_lowercase();
    if question.contains("deliverab")
        || question.contains("bounce")
        || (question.contains("verify") && question.contains("email"))
        || question.contains("valid email")
    {
        "deliverability"
    } else if question.contains("email") || question.contains("contact") {
        "contacts"
    } else if question.contains("technolog")
        || question.contains("tech stack")
        || question.contains("built with")
    {
        "technology"
    } else if question.contains("own") || question.contains("owner") || question.contains("belong")
    {
        "ownership"
    } else if social_words(&question) {
        "profile"
    } else if question.contains("certificat")
        || question.contains("subdomain")
        || question.contains("dns")
        || question.contains("infrastructure")
    {
        "infrastructure"
    } else if question.contains("whois")
        || question.contains("registr")
        || question.contains("rdap")
        || question.contains(" asn")
    {
        "registration"
    } else if question.contains("archive")
        || question.contains("wayback")
        || question.contains("historical snapshot")
    {
        "archive"
    } else if question.contains("filing")
        || question.contains("edgar")
        || question.contains(" sec")
        || question.contains(" lei")
    {
        "filing"
    } else if question.contains("where")
        || question.contains("address")
        || question.contains("headquarter")
        || question.contains("located")
    {
        "place"
    } else if question.contains("cve") || question.contains("vulnerab") {
        "vulnerability"
    } else if question.contains("github")
        || question.contains("repository")
        || question.contains("source code")
    {
        "code"
    } else if question.contains("bitcoin") {
        "bitcoin"
    } else {
        "overview"
    }
}

pub fn focus_entities(question: &str, entities: &mut [SelectedEntity]) {
    let subject = subject_of(question);
    let matches = |entity: &SelectedEntity| {
        matches_subject(&subject, &entity.canonical_name)
            || entity.identifiers.iter().any(|identifier| {
                question
                    .to_ascii_lowercase()
                    .contains(&identifier.value.to_ascii_lowercase())
            })
    };
    if !entities.iter().any(&matches) {
        return;
    }
    for entity in entities.iter_mut() {
        if !matches(entity) {
            entity.selected = false;
        }
    }
}

pub fn gaps_for(
    question: &str,
    strategy: &str,
    entities: &[SelectedEntity],
    hypotheses: Option<&HypothesisRecord>,
) -> Vec<Gap> {
    let mut gaps = vec![Gap {
        id: format!("gap-{}", gap_kind(question)),
        question: question.trim().to_string(),
        kind: gap_kind(question).into(),
    }];
    if entities.iter().all(|entity| !entity.selected) {
        gaps.push(Gap {
            id: "gap-identity".into(),
            question: format!("Who or what is {}?", subject_of(question)),
            kind: "identity".into(),
        });
    }
    if strategy == HYPOTHESIS {
        if let Some(record) = hypotheses {
            for alternative in &record.alternatives {
                gaps.push(Gap {
                    id: format!("gap-{}", alternative.id),
                    question: format!("What distinguishes: {}", alternative.statement),
                    kind: "hypothesis".into(),
                });
            }
        }
    }
    if strategy == DISCOVERY {
        let profile = entities.iter().any(|entity| {
            entity.selected
                && entity
                    .identifiers
                    .iter()
                    .any(|identifier| platform_kind(&identifier.kind))
        });
        if profile && matches!(gap_kind(question), "overview" | "identity" | "profile") {
            gaps.push(Gap {
                id: "gap-profile".into(),
                question: "Which evidence-supported public profile belongs to the subject?".into(),
                kind: "profile".into(),
            });
        }
    }
    gaps
}

fn platform_kind(kind: &str) -> bool {
    matches!(
        kind,
        "twitter"
            | "instagram"
            | "tiktok"
            | "youtube"
            | "facebook"
            | "linkedin"
            | "threads"
            | "twitch"
    )
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProposedAction {
    pub id: String,
    pub tool_id: String,
    pub arguments: Value,
    pub gap_id: String,
    pub purpose: String,
    pub evidence_ids: Vec<String>,
    pub expected: String,
    pub credit_cost: u32,
    pub provider: String,
    pub cache_available: bool,
    pub scarce: bool,
    pub rank_reason: String,
    pub alternative_id: String,
}

impl ProposedAction {
    pub fn spends(&self) -> bool {
        self.scarce && !self.cache_available && self.credit_cost > 0
    }

    pub fn signature(&self) -> String {
        format!("{}:{}", self.tool_id, self.arguments)
    }
}

pub struct SelectionInput<'a> {
    pub question: &'a str,
    pub strategy: &'a str,
    pub opening: bool,
    pub entities: &'a [SelectedEntity],
    pub gaps: &'a [Gap],
    pub enabled: &'a HashSet<String>,
    pub already: &'a HashSet<String>,
    pub cached: &'a HashSet<String>,
    pub hunter_cap: usize,
    pub sociavault_cap: usize,
    pub credits_left: &'a HashMap<String, u32>,
    pub costs: &'a HashMap<String, u32>,
    pub hits: &'a [SearchHit],
}

pub struct Ranked {
    pub actions: Vec<ProposedAction>,
    pub deferred: Vec<ProposedAction>,
    pub considered: usize,
}

pub fn rank_actions(input: &SelectionInput<'_>) -> Ranked {
    let considered = osint::registry()
        .iter()
        .filter(|tool| input.enabled.contains(tool.id))
        .count();
    let mut proposed = Vec::new();
    for tool in osint::registry() {
        if !input.enabled.contains(tool.id) {
            continue;
        }
        if let Some(mut action) = propose(tool.id, input) {
            let signature = action.signature();
            if input.already.contains(&signature) {
                continue;
            }
            action.cache_available = input.cached.contains(&signature);
            if action.cache_available {
                action.credit_cost = 0;
                action.rank_reason = format!(
                    "{} A fresh cached result can answer this without spending credits.",
                    action.rank_reason
                );
            }
            if let Some(provider_credits) = input.credits_left.get(&action.provider) {
                if action.spends() && action.credit_cost > *provider_credits {
                    action.rank_reason =
                        format!("{} Deferred because the {} credit budget is too low.", action.rank_reason, action.provider);
                    action.id = format!("deferred-{}", proposed.len());
                    // collected below as deferred via a flag on purpose? keep in a side vec
                    proposed.push(action);
                    continue;
                }
            }
            proposed.push(action);
        }
    }
    proposed.sort_by_key(action_order);
    let mut seen_gap_category = HashSet::new();
    let mut hunter = 0usize;
    let mut social = 0usize;
    let mut actions = Vec::new();
    let mut deferred = Vec::new();
    for mut action in proposed {
        if action.rank_reason.contains("credit budget is too low") {
            action.id = format!("defer-{}", deferred.len());
            deferred.push(action);
            continue;
        }
        let category = osint::definition(&action.tool_id)
            .map(|tool| tool.category)
            .unwrap_or("");
        let sweep_key = format!("{}:{category}:{}", action.gap_id, action.alternative_id);
        if !seen_gap_category.insert(sweep_key) {
            action.rank_reason = format!(
                "{} Deferred to avoid a second {} lookup for the same gap.",
                action.rank_reason, category
            );
            deferred.push(action);
            continue;
        }
        if action.tool_id.starts_with("hunter_") {
            if hunter >= input.hunter_cap {
                action.rank_reason = format!(
                    "{} Deferred because the Hunter opening allowance is {hunter}.",
                    action.rank_reason
                );
                deferred.push(action);
                continue;
            }
            hunter += 1;
        }
        if action.tool_id == "sociavault_profile" {
            if social >= input.sociavault_cap {
                action.rank_reason = format!(
                    "{} Deferred because the SociaVault opening allowance is {social}.",
                    action.rank_reason
                );
                deferred.push(action);
                continue;
            }
            social += 1;
        }
        actions.push(action);
    }
    for (index, action) in actions.iter_mut().enumerate() {
        action.id = format!("act-{index}");
    }
    Ranked {
        actions,
        deferred,
        considered,
    }
}

fn action_order(action: &ProposedAction) -> (u8, u8, u32, String) {
    (
        u8::from(!action.cache_available),
        u8::from(action.spends()),
        action.credit_cost,
        action.tool_id.clone(),
    )
}

pub fn discovery_batch(actions: &[ProposedAction]) -> (Vec<ProposedAction>, Vec<ProposedAction>) {
    let mut execute = Vec::new();
    let mut deferred = Vec::new();
    let mut supplemental = 0;
    let mut hunter = 0;
    let mut social = 0;
    let mut scrape = 0;
    for action in actions {
        let admit = if action.tool_id == "firecrawl_search" {
            false
        } else if action.tool_id.starts_with("hunter_") {
            hunter += 1;
            hunter == 1
        } else if action.tool_id == "sociavault_profile" {
            social += 1;
            social == 1
        } else if action.tool_id == "firecrawl_scrape" {
            scrape += 1;
            scrape == 1
        } else {
            supplemental += 1;
            supplemental <= 2
        };
        if admit {
            execute.push(action.clone());
        } else {
            deferred.push(action.clone());
        }
    }
    (execute, deferred)
}

pub fn adaptive_step(actions: &[ProposedAction]) -> (Vec<ProposedAction>, Option<ProposedAction>) {
    let mut free = Vec::new();
    let mut scarce = None;
    for action in actions {
        if action.spends() {
            if scarce.is_none() {
                scarce = Some(action.clone());
            }
        } else if free.len() < 3 {
            free.push(action.clone());
        }
    }
    (free, scarce)
}

pub fn hypothesis_batch(actions: &[ProposedAction]) -> (Vec<ProposedAction>, Vec<ProposedAction>) {
    let mut execute = Vec::new();
    let mut deferred = Vec::new();
    let mut spending = 0;
    let mut spent_alternatives = HashSet::new();
    for action in actions {
        if execute.len() >= 4 {
            deferred.push(action.clone());
            continue;
        }
        if action.spends() {
            if !action.alternative_id.is_empty()
                && !spent_alternatives.insert(action.alternative_id.clone())
            {
                deferred.push(action.clone());
                continue;
            }
            if spending >= 2 {
                deferred.push(action.clone());
                continue;
            }
            spending += 1;
        }
        execute.push(action.clone());
    }
    (execute, deferred)
}

fn propose(tool_id: &str, input: &SelectionInput<'_>) -> Option<ProposedAction> {
    let kind = gap_kind(input.question);
    let primary = input
        .gaps
        .iter()
        .find(|gap| gap.kind == kind)
        .or_else(|| input.gaps.first())?;
    let usable = |entity: &&SelectedEntity| entity.selected && !entity.ambiguous;
    let entity = input.entities.iter().find(usable);
    let org = input.entities.iter().find(|entity| {
        usable(entity) && identifier(entity, "domain").is_some()
    });
    let domain = org.and_then(|entity| identifier(entity, "domain"));
    let person = input.entities.iter().find(|entity| {
        usable(entity) && entity.entity_type == "person"
    });
    let social_entity = input.entities.iter().find(|entity| {
        usable(entity)
            && entity
                .identifiers
                .iter()
                .any(|identifier| platform_kind(&identifier.kind))
    });
    let social = social_entity.and_then(|entity| {
        entity
            .identifiers
            .iter()
            .find(|identifier| platform_kind(&identifier.kind))
    });
    let email = emails_in(input.question).into_iter().next();
    let ip = explicit_kind(input.question, "ip");
    let cve = explicit_kind(input.question, "cve");
    let page = entity_page(input, entity);
    let question_words = input.question.to_ascii_lowercase();
    let wants = |needles: &[&str]| needles.iter().any(|needle| question_words.contains(needle));
    let gap_for = |gap_kind_name: &str| {
        input
            .gaps
            .iter()
            .find(|gap| gap.kind == gap_kind_name)
            .unwrap_or(primary)
    };
    let action = match tool_id {
        "firecrawl_search" if !input.opening && matches!(input.strategy, ADAPTIVE | HYPOTHESIS) => {
            let query = clip_query(&format!(
                "{} {}",
                subject_of(input.question),
                primary.question
            ));
            grounded(
                tool_id,
                json!({"query": query, "limit": 5}),
                primary,
                "Search one new angle for the unresolved question.",
                vec!["question".into()],
                "A distinct follow-up search, not a repeat of the opening pair.",
                "The opening pair already ran, so another search has to cover a new gap.",
            )
        }
        "firecrawl_scrape"
            if matches!(kind, "overview" | "ownership" | "hypothesis" | "identity")
                && entity_page(input, org.or(entity)).is_some() =>
        {
            let (url, evidence) = entity_page(input, org.or(entity))?;
            grounded(
                tool_id,
                json!({"url": url}),
                primary,
                "Retrieve the page behind a consequential snippet.",
                vec![evidence],
                "Page content can support a claim that a snippet only mentions.",
                "One evidence URL, with no crawl or pagination.",
            )
        }
        "hunter_domain_search"
            if kind == "contacts" && !wants(&["email for", "email address of", "email of"]) =>
        {
            let entity = entity?;
            if entity.ambiguous {
                return None;
            }
            let domain = domain?;
            grounded(
                tool_id,
                json!({"domain": domain}),
                gap_for("contacts"),
                "Find the professional email pattern for this company domain.",
                entity.evidence_ids.clone(),
                "Company contacts or the email pattern, without a separate company-enrichment call.",
                "The question asks for professional contacts and the domain is already evidenced.",
            )
        }
        "hunter_email_finder"
            if kind == "contacts" && wants(&["email for", "email address of", "email of"]) =>
        {
            let entity = entity?;
            let person = person?;
            let domain = domain.or_else(|| identifier(entity, "domain"))?;
            grounded(
                tool_id,
                json!({"domain": domain, "full_name": person.canonical_name}),
                gap_for("contacts"),
                "Find the professional email for the named person at the evidenced domain.",
                person.evidence_ids.clone(),
                "A candidate professional address for that person.",
                "A person name and company domain are both supported, and the question asks for that email.",
            )
        }
        "hunter_email_verifier" if kind == "deliverability" => {
            let email = email?;
            grounded(
                tool_id,
                json!({"email": email}),
                gap_for("deliverability"),
                "Check deliverability because the question asks whether the address can receive mail.",
                vec!["question".into()],
                "A deliverability result. It does not establish who owns the address.",
                "Deliverability was requested for an address in the question.",
            )
        }
        "hunter_tech_lookup" if kind == "technology" => {
            let entity = entity?;
            let domain = domain?;
            grounded(
                tool_id,
                json!({"domain": domain}),
                gap_for("technology"),
                "Look up the company technology profile for this domain.",
                entity.evidence_ids.clone(),
                "Firmographics and technologies Hunter lists for the domain.",
                "The question asks about technology, so company enrichment is the Hunter endpoint.",
            )
        }
        "sociavault_profile" if social.is_some() && input.gaps.iter().any(|gap| gap.kind == "profile") => {
            let entity = social_entity?;
            if entity.ambiguous {
                return None;
            }
            let social = social?;
            grounded(
                tool_id,
                json!({"platform": social.kind, "handle": social.value}),
                gap_for("profile"),
                "Enrich the public profile for a handle that search evidence already supports.",
                entity.evidence_ids.clone(),
                "Public profile fields. A matching handle does not prove ownership.",
                "The platform and handle came from Firecrawl evidence, not from a guessed name.",
            )
        }
        "wikidata_entities"
            if matches!(kind, "overview" | "identity" | "ownership" | "hypothesis") =>
        {
            let name = entity
                .map(|entity| entity.canonical_name.clone())
                .unwrap_or_else(|| subject_of(input.question));
            if name.is_empty() {
                return None;
            }
            let evidence = entity
                .map(|entity| entity.evidence_ids.clone())
                .unwrap_or_else(|| vec!["question".into()]);
            grounded(
                tool_id,
                json!({"name": name}),
                primary,
                "Check the public entity record for the named subject.",
                evidence,
                "Candidate identifiers and claims. Search matches still need confirmation.",
                "Wikidata is the single organization record used for this identity gap.",
            )
        }
        "gleif_entities" if matches!(kind, "filing" | "ownership" | "identity") => {
            let entity = entity.filter(|entity| entity.entity_type == "organization")?;
            grounded(
                tool_id,
                json!({"company_name": entity.canonical_name}),
                primary,
                "Look up a legal-entity record for the named organization.",
                entity.evidence_ids.clone(),
                "LEI candidates that still require identity confirmation.",
                "The company name is grounded and the gap is legal identity or ownership.",
            )
        }
        "sec_submissions" if kind == "filing" => {
            let entity = entity.filter(|entity| entity.entity_type == "organization")?;
            grounded(
                tool_id,
                json!({"name": entity.canonical_name}),
                gap_for("filing"),
                "Look up US filing metadata for the named organization.",
                entity.evidence_ids.clone(),
                "Filing metadata if the name matches one reporting entity.",
                "The question asks about filings and the company name is evidenced.",
            )
        }
        "crtsh_certificates" if kind == "infrastructure" => {
            let domain = owned_domain(domain, input.question)?;
            let evidence = entity
                .map(|entity| entity.evidence_ids.clone())
                .filter(|ids| !ids.is_empty())
                .unwrap_or_else(|| vec!["question".into()]);
            grounded(
                tool_id,
                json!({"domain": domain}),
                gap_for("infrastructure"),
                "Find certificate hostnames for the evidenced domain.",
                evidence,
                "Hostnames observed on certificates. This is the one infrastructure lookup.",
                "A domain is grounded and the question is about infrastructure. Sibling DNS tools stay unused.",
            )
        }
        "mnemonic_passive_dns" if kind == "infrastructure" && wants(&["passive dns", "pdns"]) => {
            let domain = domain?;
            grounded(
                tool_id,
                json!({"domain_or_ip": domain}),
                gap_for("infrastructure"),
                "Check passive DNS because the question asks for it.",
                entity?.evidence_ids.clone(),
                "Historical hostname observations.",
                "Passive DNS was named, so the generic certificate lookup is not added as well.",
            )
        }
        "hackertarget_hostsearch" if kind == "infrastructure" && wants(&["subdomain"]) => {
            let domain = owned_domain(domain, input.question)?;
            grounded(
                tool_id,
                json!({"domain": domain}),
                gap_for("infrastructure"),
                "Look up indexed subdomains because the question asks for them.",
                vec!["question".into()],
                "Indexed subdomain hostnames.",
                "Subdomains were requested, so this replaces the generic certificate lookup.",
            )
        }
        "ripestat_network_info" if kind == "registration" && ip.is_some() && !wants(&["apnic", "arin"]) => {
            grounded(
                tool_id,
                json!({"ip": ip?}),
                gap_for("registration"),
                "Check the announced prefix for the evidenced IP.",
                vec!["question".into()],
                "Routing origin. It does not prove who operates a website.",
                "One routing lookup for the IP. Regional RDAP is not added beside it.",
            )
        }
        "arin_rdap" if kind == "registration" && ip.is_some() && wants(&["arin", "whois", "registr"]) => {
            grounded(
                tool_id,
                json!({"ip": ip?}),
                gap_for("registration"),
                "Read the ARIN registration for the evidenced IP.",
                vec!["question".into()],
                "Registration contacts for that address.",
                "The question asks for registration, so one RDAP source is enough.",
            )
        }
        "apnic_rdap" if kind == "registration" && ip.is_some() && wants(&["apnic", "asia-pacific", "asia pacific"]) => {
            grounded(
                tool_id,
                json!({"ip": ip?}),
                gap_for("registration"),
                "Read the APNIC registration because the question points at that registry.",
                vec!["question".into()],
                "Asia-Pacific registration contacts.",
                "APNIC was indicated, so ARIN is not also queried.",
            )
        }
        "wayback_availability" if kind == "archive" => {
            let url = explicit_kind(input.question, "url")
                .or_else(|| page.map(|(url, _)| url))?;
            grounded(
                tool_id,
                json!({"url": url}),
                gap_for("archive"),
                "Locate an archived snapshot of the evidenced URL.",
                vec!["question".into()],
                "Whether a snapshot exists. It does not return the page body.",
                "One archive lookup for the URL.",
            )
        }
        "commoncrawl_urls" if kind == "archive" && explicit_kind(input.question, "url").is_none() => {
            let domain = owned_domain(domain, input.question)?;
            grounded(
                tool_id,
                json!({"domain": domain}),
                gap_for("archive"),
                "Find crawled URLs for the evidenced domain.",
                vec!["question".into()],
                "Indexed crawl URLs.",
                "No specific page URL was named, so Common Crawl is the archive source.",
            )
        }
        "arquivo_history" if kind == "archive" && wants(&["arquivo"]) => {
            let domain = owned_domain(domain, input.question)?;
            grounded(
                tool_id,
                json!({"domain_or_url": domain}),
                gap_for("archive"),
                "Check Arquivo.pt because the question names it.",
                vec!["question".into()],
                "Archived versions from that collection.",
                "Arquivo was requested by name.",
            )
        }
        "nominatim_geocode" if kind == "place" => {
            let place = subject_of(input.question);
            grounded(
                tool_id,
                json!({"address_or_place": place}),
                gap_for("place"),
                "Geocode the place named in the question.",
                vec!["question".into()],
                "Coordinates for that place name.",
                "One geocode. Census and Overpass are not added without a US street or known coordinates.",
            )
        }
        "census_geocode" if kind == "place" && wants(&["street", "ave", "avenue", "blvd", "road"]) => {
            grounded(
                tool_id,
                json!({"us_address": input.question.trim()}),
                gap_for("place"),
                "Geocode the US street address in the question.",
                vec!["question".into()],
                "Coordinates for that address.",
                "A US street address is present.",
            )
        }
        "github_repositories" if kind == "code" && !wants(&["gitlab"]) => {
            grounded(
                tool_id,
                json!({"query": subject_of(input.question)}),
                gap_for("code"),
                "Search public repository names for the subject.",
                vec!["question".into()],
                "Repository metadata, not a code-host sweep.",
                "The question asks about code. GitLab and grep.app stay unused unless named.",
            )
        }
        "gitlab_projects" if kind == "code" && wants(&["gitlab"]) => {
            grounded(
                tool_id,
                json!({"query": subject_of(input.question)}),
                gap_for("code"),
                "Search public GitLab projects because the question names GitLab.",
                vec!["question".into()],
                "Project metadata.",
                "GitLab was named.",
            )
        }
        "grepapp_code_search" if kind == "code" && wants(&["grep.app", "code search", "source snippet"]) => {
            grounded(
                tool_id,
                json!({"query": subject_of(input.question)}),
                gap_for("code"),
                "Search public code snippets because the question asks for source text.",
                vec!["question".into()],
                "Untrusted snippets that mention the query.",
                "A code-search question was asked.",
            )
        }
        "nvd_cve" if kind == "vulnerability" && cve.is_some() && !wants(&["mitre"]) => {
            grounded(
                tool_id,
                json!({"cve_id": cve?}),
                gap_for("vulnerability"),
                "Read the NVD record for the CVE in the question.",
                vec!["question".into()],
                "Description and severity. It does not prove a live exposure.",
                "One vulnerability record for the named CVE.",
            )
        }
        "cve_record" if kind == "vulnerability" && cve.is_some() && wants(&["mitre", "cve record"]) => {
            grounded(
                tool_id,
                json!({"cve_id": cve?}),
                gap_for("vulnerability"),
                "Read the published CVE record because the question asks for it.",
                vec!["question".into()],
                "The published record and its references.",
                "The MITRE record was requested instead of a second vulnerability source.",
            )
        }
        "shodan_internetdb" if wants(&["shodan", "open port", "exposed port"]) && ip.is_some() => {
            grounded(
                tool_id,
                json!({"ip": ip?}),
                primary,
                "Check Shodan InternetDB because the question asks about exposed services.",
                vec!["question".into()],
                "Observed ports and hostnames. They may be stale.",
                "Exposure was requested for one IP.",
            )
        }
        "sans_ip_activity" if wants(&["sans", "isc", "attack traffic"]) && ip.is_some() => {
            grounded(
                tool_id,
                json!({"ip": ip?}),
                primary,
                "Check reported attack activity because the question asks about it.",
                vec!["question".into()],
                "Historical reports for that IP.",
                "SANS ISC was the matching exposure source.",
            )
        }
        "urlscan_search" if wants(&["urlscan", "website scan"]) => {
            let domain = owned_domain(domain, input.question)?;
            grounded(
                tool_id,
                json!({"domain": domain}),
                primary,
                "Search existing urlscan records because the question asks about scans.",
                vec!["question".into()],
                "Historical scan records. No new scan is submitted.",
                "urlscan was named.",
            )
        }
        "blockstream_address" if kind == "bitcoin" => {
            let address = bitcoin_in(input.question)?;
            grounded(
                tool_id,
                json!({"bitcoin_address": address}),
                gap_for("bitcoin"),
                "Read public activity for the Bitcoin address in the question.",
                vec!["question".into()],
                "Address activity. It does not identify the owner.",
                "One Bitcoin explorer. The other explorers are not queried.",
            )
        }
        "keybase_identity" if wants(&["keybase"]) => {
            let username = explicit_kind(input.question, "domain")
                .unwrap_or_else(|| subject_of(input.question));
            grounded(
                tool_id,
                json!({"username": username}),
                primary,
                "Look up the Keybase profile because the question names Keybase.",
                vec!["question".into()],
                "Public proofs. They do not establish physical identity.",
                "Keybase was named, so the other account directories are not queried.",
            )
        }
        "stackexchange_users" if wants(&["stack exchange", "stackoverflow", "stack overflow"]) => {
            grounded(
                tool_id,
                json!({"name": subject_of(input.question)}),
                primary,
                "Search Stack Exchange display names because the question asks about that site.",
                vec!["question".into()],
                "Public profiles. Display names can collide.",
                "Stack Exchange was named.",
            )
        }
        "wikipedia_users" if wants(&["wikipedia user", "wikipedia account"]) => {
            grounded(
                tool_id,
                json!({"username": subject_of(input.question)}),
                primary,
                "Look up the Wikipedia account because the question names one.",
                vec!["question".into()],
                "Registration metadata. It does not establish physical identity.",
                "A Wikipedia account was named.",
            )
        }
        _ => return None,
    }?;
    if action.purpose.is_empty() || action.evidence_ids.is_empty() || action.expected.is_empty() {
        return None;
    }
    if osint::validate(&action.tool_id, &action.arguments).is_err() {
        return None;
    }
    Some(price(action, input))
}

fn price(mut action: ProposedAction, input: &SelectionInput<'_>) -> ProposedAction {
    let default = osint::endpoint_cost(&action.tool_id);
    if let Some(cost) = default {
        action.provider = cost.provider.into();
        action.credit_cost = input
            .costs
            .get(&action.tool_id)
            .copied()
            .unwrap_or(cost.credits);
        action.scarce = action.credit_cost > 0;
    }
    if action.tool_id == "hunter_email_finder" || action.tool_id == "sociavault_profile" {
        if let Some(record_gap) = input.gaps.iter().find(|gap| gap.kind == "hypothesis") {
            action.alternative_id = record_gap
                .id
                .trim_start_matches("gap-")
                .to_string();
            action.gap_id = record_gap.id.clone();
        }
    }
    action
}

fn grounded(
    tool_id: &str,
    arguments: Value,
    gap: &Gap,
    purpose: &str,
    evidence_ids: Vec<String>,
    expected: &str,
    rank_reason: &str,
) -> Option<ProposedAction> {
    if evidence_ids.is_empty() {
        return None;
    }
    Some(ProposedAction {
        id: String::new(),
        tool_id: tool_id.into(),
        arguments,
        gap_id: gap.id.clone(),
        purpose: purpose.into(),
        evidence_ids,
        expected: expected.into(),
        credit_cost: 0,
        provider: String::new(),
        cache_available: false,
        scarce: false,
        rank_reason: rank_reason.into(),
        alternative_id: String::new(),
    })
}

fn owned_domain(domain: Option<&str>, question: &str) -> Option<String> {
    domain
        .map(str::to_string)
        .or_else(|| explicit_kind(question, "domain"))
}

fn identifier<'a>(entity: &'a SelectedEntity, kind: &str) -> Option<&'a str> {
    entity
        .identifiers
        .iter()
        .find(|identifier| identifier.kind == kind)
        .map(|identifier| identifier.value.as_str())
}

fn explicit_kind(question: &str, kind: &str) -> Option<String> {
    super::explicit_entities(question)
        .into_iter()
        .find(|(found, _)| found == kind)
        .map(|(_, value)| value)
}

fn entity_page(input: &SelectionInput<'_>, entity: Option<&SelectedEntity>) -> Option<(String, String)> {
    let entity = entity?;
    let domain = identifier(entity, "domain")?;
    input.hits.iter().find_map(|hit| {
        let host = url::Url::parse(&hit.url).ok()?.host_str()?.to_ascii_lowercase();
        let host = host.trim_start_matches("www.");
        if host == domain {
            Some((hit.url.clone(), hit.evidence_id.clone()))
        } else {
            None
        }
    })
}

fn emails_in(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for token in text.split_whitespace() {
        let token = token.trim_matches(|ch: char| {
            matches!(ch, ',' | ';' | '(' | ')' | '"' | '\'' | '<' | '>' | '.' | '?')
        });
        let Some((name, host)) = token.split_once('@') else {
            continue;
        };
        if name.is_empty() || !host.contains('.') || token.len() > 200 {
            continue;
        }
        let email = token.to_ascii_lowercase();
        if !found.contains(&email) {
            found.push(email);
        }
    }
    found
}

fn bitcoin_in(text: &str) -> Option<String> {
    text.split_whitespace().find_map(|token| {
        let token = token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric());
        if token.len() >= 26 && (token.starts_with('1') || token.starts_with('3') || token.starts_with("bc1"))
        {
            Some(token.to_string())
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hit(id: &str, title: &str, url: &str, snippet: &str, role: &str) -> SearchHit {
        SearchHit {
            evidence_id: id.into(),
            title: title.into(),
            url: url.into(),
            snippet: snippet.into(),
            retrieved_at: "2026-10-01T00:00:00Z".into(),
            query_role: role.into(),
        }
    }

    fn enabled_all() -> HashSet<String> {
        osint::registry()
            .iter()
            .map(|tool| tool.id.to_string())
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn input<'a>(
        question: &'a str,
        strategy: &'a str,
        opening: bool,
        entities: &'a [SelectedEntity],
        gaps: &'a [Gap],
        enabled: &'a HashSet<String>,
        already: &'a HashSet<String>,
        cached: &'a HashSet<String>,
        credits: &'a HashMap<String, u32>,
        costs: &'a HashMap<String, u32>,
        hits: &'a [SearchHit],
    ) -> SelectionInput<'a> {
        SelectionInput {
            question,
            strategy,
            opening,
            entities,
            gaps,
            enabled,
            already,
            cached,
            hunter_cap: if opening { 1 } else { 4 },
            sociavault_cap: if opening { 1 } else { 4 },
            credits_left: credits,
            costs,
            hits,
        }
    }

    #[test]
    fn unanswered_questions_name_three_more_tools_and_answered_ones_name_none() {
        let enabled = enabled_all();
        let mut used = HashSet::new();
        used.insert("firecrawl_search".into());
        let tools = additional_tools("certificates for example.org", &used, &enabled);
        assert!(tools.len() >= 3);
        assert!(tools.iter().all(|tool| tool.tool_id != "firecrawl_search"));
        assert!(tools.iter().any(|tool| tool.tool_id == "crtsh_certificates"));
        let fallback = tools.clone();
        let answered = assessment_from_model(&json!({"answered": true, "tools": [{"tool_id": "crtsh_certificates", "reason": "ignored"}]}), &fallback).unwrap();
        assert!(answered.answered);
        assert!(answered.tools.is_empty());
        let partial = assessment_from_model(
            &json!({"answered": false, "tools": [{"tool_id": "wikidata_entities", "reason": "Confirm the named organization."}]}),
            &fallback,
        )
        .unwrap();
        assert!(!partial.answered);
        assert!(partial.tools.len() >= 3);
        assert_eq!(partial.tools[0].tool_id, "wikidata_entities");
    }

    #[test]
    fn strategy_follows_the_question_and_can_change_without_erasing_work() {
        let discovery = select_strategy("who is jeff bezos?", true, false, true);
        assert_eq!(discovery.kind, DISCOVERY);
        assert!(!discovery.rationale.is_empty());
        let hypothesis = select_strategy(
            "Does Jeff Bezos or MacKenzie Scott own Blue Origin?",
            true,
            false,
            true,
        );
        assert_eq!(hypothesis.kind, HYPOTHESIS);
        let adaptive = select_strategy("certificates for example.org", false, true, false);
        assert_eq!(adaptive.kind, ADAPTIVE);
        let change = strategy_change_reason(DISCOVERY, &adaptive).unwrap();
        assert!(change.contains("Discovery"));
        assert!(change.contains("Adaptive"));
        assert!(strategy_change_reason(ADAPTIVE, &adaptive).is_none());
    }

    #[test]
    fn opening_queries_take_two_different_angles() {
        let queries = complementary_queries("who owns example.com?", HYPOTHESIS);
        assert_eq!(queries[0].role, "identity");
        assert_eq!(queries[1].role, "investigative");
        assert!(distinct_queries(&queries[0].query, &queries[1].query));
        assert!(queries[1].query.to_ascii_lowercase().contains("ownership"));
        assert!(!queries[0].query.to_ascii_lowercase().contains("ownership"));
        let history = complementary_queries("How did Amazon start?", DISCOVERY);
        assert!(history[1].query.to_ascii_lowercase().contains("found") || history[1].query.contains("history") || history[1].query.contains("timeline"));
    }

    #[test]
    fn entity_selection_keeps_related_identifiers_and_drops_incidental_pages() {
        let hits = vec![
            hit("e1", "Jeff Bezos - Wikipedia", "https://en.wikipedia.org/wiki/Jeff_Bezos", "American businessman", "identity"),
            hit("e2", "Amazon.com", "https://www.amazon.com/", "Company associated with Jeff Bezos", "identity"),
            hit("e3", "About Amazon", "https://www.aboutamazon.com/about", "Jeff Bezos ownership and leadership", "investigative"),
            hit("e4", "Jeff Bezos (@JeffBezos)", "https://twitter.com/JeffBezos", "profile", "investigative"),
            hit("e5", "Recipes", "https://www.nytimes.com/section/food", "no relation", "investigative"),
            hit("e6", "Someone else (@randomperson)", "https://twitter.com/randomperson", "unrelated profile", "identity"),
        ];
        let entities = select_entities("who is jeff bezos?", &dedupe_hits(hits));
        let selected: Vec<_> = entities.iter().filter(|entity| entity.selected).collect();
        assert!(selected.len() <= 3);
        assert!(selected.len() >= 2);
        assert!(selected.iter().any(|entity| {
            entity.canonical_name.to_ascii_lowercase().contains("bezos")
                && entity.identifiers.iter().any(|identifier| identifier.kind == "twitter")
        }));
        assert!(selected.iter().any(|entity| {
            entity.identifiers.iter().any(|identifier| identifier.value == "amazon.com" || identifier.value == "aboutamazon.com")
        }));
        assert!(entities.iter().all(|entity| {
            entity.identifiers.iter().all(|identifier| {
                identifier.value != "nytimes.com" && identifier.value != "wikipedia.org" && identifier.value != "randomperson"
            })
        }));
    }

    #[test]
    fn hypothesis_absence_stays_unresolved() {
        let mut record = draft_hypotheses("Does Jeff Bezos or MacKenzie Scott own Blue Origin?");
        assert_eq!(record.alternatives.len(), 2);
        classify_hypothesis(
            &mut record,
            &[(
                "e1".into(),
                "Jeff Bezos founded Blue Origin.".into(),
            )],
        );
        assert_eq!(record.status, "unresolved");
        assert!(record.alternatives[1].supporting.is_empty());
        assert!(record.alternatives[1].contradicting.is_empty());
        assert!(!record.alternatives[1].missing.is_empty());
        classify_hypothesis(
            &mut record,
            &[
                ("e1".into(), "Jeff Bezos founded Blue Origin.".into()),
                (
                    "e2".into(),
                    "MacKenzie Scott is not the owner of Blue Origin.".into(),
                ),
            ],
        );
        assert_eq!(record.status, "supported");
    }

    #[test]
    fn actions_are_grounded_capped_and_not_a_sweep() {
        let hits = vec![hit(
            "e1",
            "Example Org",
            "https://example.org/",
            "Example Org certificates",
            "identity",
        )];
        let entities = select_entities("certificates for example.org", &hits);
        let selected: Vec<_> = entities.into_iter().filter(|entity| entity.selected).collect();
        assert!(!selected.is_empty());
        let gaps = gaps_for("certificates for example.org", ADAPTIVE, &selected, None);
        let enabled = enabled_all();
        let credits = HashMap::from([
            ("firecrawl".into(), 20),
            ("hunter".into(), 5),
            ("sociavault".into(), 5),
        ]);
        let ranked = rank_actions(&input(
            "certificates for example.org",
            ADAPTIVE,
            true,
            &selected,
            &gaps,
            &enabled,
            &HashSet::new(),
            &HashSet::new(),
            &credits,
            &HashMap::new(),
            &[],
        ));
        assert_eq!(ranked.considered, osint::registry().len());
        assert!(ranked
            .actions
            .iter()
            .any(|action| action.tool_id == "crtsh_certificates"));
        assert!(ranked.actions.iter().all(|action| {
            action.tool_id != "hackertarget_hostsearch" && action.tool_id != "mnemonic_passive_dns"
        }));
        assert!(ranked.actions.iter().all(|action| {
            !action.purpose.is_empty()
                && !action.expected.is_empty()
                && !action.evidence_ids.is_empty()
                && !action.gap_id.is_empty()
                && action.arguments.get("offset").is_none()
                && action.arguments.get("page").is_none()
        }));
    }

    #[test]
    fn hunter_and_sociavault_follow_the_gap_and_the_opening_cap() {
        let hits = vec![
            hit("e1", "Amazon", "https://www.amazon.com/", "Amazon company", "identity"),
            hit("e2", "Amazon (@amazon)", "https://twitter.com/amazon", "profile", "investigative"),
            hit("e3", "Amazon photos", "https://www.instagram.com/amazon/", "profile", "investigative"),
        ];
        let entities = select_entities("who is Amazon?", &hits);
        let gaps = gaps_for("who is Amazon?", DISCOVERY, &entities, None);
        let enabled = enabled_all();
        let credits = HashMap::from([
            ("hunter".into(), 10),
            ("sociavault".into(), 10),
            ("firecrawl".into(), 10),
        ]);
        let contacts = gaps_for(
            "what email addresses does Amazon use?",
            DISCOVERY,
            &entities,
            None,
        );
        let contact_actions = rank_actions(&input(
            "what email addresses does Amazon use?",
            DISCOVERY,
            true,
            &entities,
            &contacts,
            &enabled,
            &HashSet::new(),
            &HashSet::new(),
            &credits,
            &HashMap::new(),
            &[],
        ));
        assert!(contact_actions
            .actions
            .iter()
            .any(|action| action.tool_id == "hunter_domain_search"));
        assert!(contact_actions
            .actions
            .iter()
            .all(|action| action.tool_id != "hunter_tech_lookup"));
        let tech = gaps_for(
            "what technology stack does Amazon use?",
            ADAPTIVE,
            &entities,
            None,
        );
        let tech_actions = rank_actions(&input(
            "what technology stack does Amazon use?",
            ADAPTIVE,
            true,
            &entities,
            &tech,
            &enabled,
            &HashSet::new(),
            &HashSet::new(),
            &credits,
            &HashMap::new(),
            &[],
        ));
        assert!(tech_actions
            .actions
            .iter()
            .any(|action| action.tool_id == "hunter_tech_lookup"));
        assert!(tech_actions
            .actions
            .iter()
            .all(|action| action.tool_id != "hunter_domain_search"));
        let verify = gaps_for(
            "is ada@example.org deliverable?",
            ADAPTIVE,
            &[],
            None,
        );
        let verify_actions = rank_actions(&input(
            "is ada@example.org deliverable?",
            ADAPTIVE,
            true,
            &[],
            &verify,
            &enabled,
            &HashSet::new(),
            &HashSet::new(),
            &credits,
            &HashMap::new(),
            &[],
        ));
        assert!(verify_actions
            .actions
            .iter()
            .any(|action| action.tool_id == "hunter_email_verifier"));
        let profiles = rank_actions(&input(
            "who is Amazon?",
            DISCOVERY,
            true,
            &entities,
            &gaps,
            &enabled,
            &HashSet::new(),
            &HashSet::new(),
            &credits,
            &HashMap::new(),
            &hits,
        ));
        assert_eq!(
            profiles
                .actions
                .iter()
                .filter(|action| action.tool_id == "sociavault_profile")
                .count(),
            1
        );
        let (now, later) = discovery_batch(&profiles.actions);
        assert!(later.len() + now.len() == profiles.actions.len());
        assert!(now.iter().all(|action| action.tool_id != "firecrawl_search"));
    }

    #[test]
    fn adaptive_expansion_spends_one_scarce_lookup_then_reranks() {
        let hits = vec![hit(
            "e1",
            "Example Org",
            "https://example.org/",
            "Example Org",
            "identity",
        )];
        let entities = select_entities("what email addresses does Example Org use at example.org?", &hits);
        let gaps = gaps_for(
            "what email addresses does Example Org use at example.org?",
            ADAPTIVE,
            &entities,
            None,
        );
        let enabled = enabled_all();
        let credits = HashMap::from([("hunter".into(), 5), ("firecrawl".into(), 5), ("sociavault".into(), 5)]);
        let ranked = rank_actions(&input(
            "what email addresses does Example Org use at example.org?",
            ADAPTIVE,
            false,
            &entities,
            &gaps,
            &enabled,
            &HashSet::new(),
            &HashSet::new(),
            &credits,
            &HashMap::new(),
            &hits,
        ));
        let (free, scarce) = adaptive_step(&ranked.actions);
        assert!(scarce.iter().filter(|action| action.spends()).count() <= 1);
        if let Some(spent) = &scarce {
            let mut already = HashSet::new();
            already.insert(spent.signature());
            let again = rank_actions(&input(
                "what email addresses does Example Org use at example.org?",
                ADAPTIVE,
                false,
                &entities,
                &gaps,
                &enabled,
                &already,
                &HashSet::new(),
                &credits,
                &HashMap::new(),
                &hits,
            ));
            assert!(again.actions.iter().all(|action| action.signature() != spent.signature()));
            let _ = free;
        }
    }
}
