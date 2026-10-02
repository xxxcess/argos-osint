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
#[allow(dead_code)]
pub struct ToolSuggestion {
    pub tool_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub struct AnswerAssessment {
    pub answered: bool,
    pub tools: Vec<ToolSuggestion>,
}

/// Tools that could supply context the current results do not. At least three when the registry allows it.
#[allow(dead_code)]
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

/// The tool-isolation step: catalog tools picked for the question, turned into
/// runnable lookups, plus the ones that cannot run and why. It never calls Firecrawl.
#[derive(Clone, Debug, Default, PartialEq)]
#[allow(dead_code)]
pub struct Isolation {
    pub actions: Vec<ProposedAction>,
    pub skipped: Vec<String>,
}

/// Turns tool suggestions into executable lookups whose inputs come from the focus
/// subject (its name, evidenced handle, or own domain), never from a source domain.
/// `missing_keys` names providers without a configured API key.
#[allow(dead_code)]
pub fn isolate_tools(
    suggestions: &[ToolSuggestion],
    input: &SelectionInput<'_>,
    missing_keys: &HashSet<String>,
    limit: usize,
) -> Isolation {
    let mut isolation = Isolation::default();
    let mut hunter = 0usize;
    let mut social = 0usize;
    let mut seen = HashSet::new();
    for suggestion in suggestions {
        if seen.len() >= limit && !seen.contains(suggestion.tool_id.as_str()) {
            break;
        }
        let Some(tool) = osint::definition(&suggestion.tool_id) else {
            continue;
        };
        if seen.contains(tool.id) {
            continue;
        }
        let skip = |reason: String| format!("{} — skipped: {reason}", tool.id);
        if tool.id.starts_with("firecrawl_") {
            isolation.skipped.push(skip(
                "Firecrawl is reserved for the two opening searches and grounded follow-ups.".into(),
            ));
            continue;
        }
        if !input.enabled.contains(tool.id) {
            isolation
                .skipped
                .push(skip(format!("{} is disabled in OSINT.", tool.name)));
            continue;
        }
        let cost = osint::endpoint_cost(tool.id);
        if let Some(cost) = cost {
            if missing_keys.contains(cost.provider) {
                isolation.skipped.push(skip(format!(
                    "no {} API key is configured. Enter it on the {} tool in OSINT or set {}_API_KEY.",
                    provider_label(cost.provider),
                    tool.name,
                    cost.provider.to_ascii_uppercase()
                )));
                continue;
            }
        }
        if tool.id.starts_with("hunter_") && hunter >= input.hunter_cap {
            isolation
                .skipped
                .push(skip(format!("the Hunter allowance for this turn is {}.", input.hunter_cap)));
            continue;
        }
        if tool.id == "sociavault_profile" && social >= input.sociavault_cap {
            isolation.skipped.push(skip(format!(
                "the SociaVault allowance for this turn is {}.",
                input.sociavault_cap
            )));
            continue;
        }
        // SociaVault runs once per evidenced account of the subject, up to the allowance.
        let candidates: Vec<ProposedAction> = if tool.id == "sociavault_profile" {
            sociavault_actions(input, &suggestion.reason)
        } else {
            propose(tool.id, input)
                .or_else(|| subject_action(tool, input, &suggestion.reason))
                .into_iter()
                .collect()
        };
        if candidates.is_empty() {
            isolation.skipped.push(skip(format!(
                "no input for {} can be derived from the subject ({}).",
                tool.name,
                tool.inputs.join(", ")
            )));
            continue;
        }
        seen.insert(tool.id);
        let mut spent = 0u32;
        for mut action in candidates {
            if input.already.contains(&action.signature()) {
                continue;
            }
            let label = format!("{} {}", tool.id, action.arguments);
            if tool.id == "sociavault_profile" && social >= input.sociavault_cap {
                isolation.skipped.push(format!(
                    "{label} — skipped: the SociaVault allowance for this turn is {}.",
                    input.sociavault_cap
                ));
                continue;
            }
            action.cache_available = input.cached.contains(&action.signature());
            if action.cache_available {
                action.credit_cost = 0;
            }
            if let Some(left) = input.credits_left.get(&action.provider) {
                if action.spends() && spent + action.credit_cost > *left {
                    isolation.skipped.push(format!(
                        "{label} — skipped: the {} credit budget is too low.",
                        provider_label(&action.provider)
                    ));
                    continue;
                }
            }
            if action.spends() {
                spent += action.credit_cost;
            }
            if tool.id.starts_with("hunter_") {
                hunter += 1;
            }
            if tool.id == "sociavault_profile" {
                social += 1;
            }
            action.rank_reason = format!(
                "Chosen by tool isolation. {} {}",
                suggestion.reason, action.rank_reason
            )
            .trim()
            .to_string();
            action.id = format!("isolate-{}", isolation.actions.len());
            isolation.actions.push(action);
        }
    }
    isolation
}

/// The focus subject entity: the selected, unambiguous entity named like the subject.
#[allow(dead_code)]
fn subject_entity<'a>(input: &SelectionInput<'a>) -> Option<&'a SelectedEntity> {
    let subject = subject_of(input.question);
    input.entities.iter().find(|entity| {
        entity.selected && !entity.ambiguous && matches_subject(&subject, &entity.canonical_name)
    })
}

/// One SociaVault profile lookup per SociaVault-supported account on the subject.
#[allow(dead_code)]
fn sociavault_actions(input: &SelectionInput<'_>, reason: &str) -> Vec<ProposedAction> {
    let Some(entity) = subject_entity(input) else {
        return Vec::new();
    };
    let Some(primary) = input
        .gaps
        .iter()
        .find(|gap| gap.kind == "profile")
        .or_else(|| input.gaps.first())
    else {
        return Vec::new();
    };
    let mut actions = Vec::new();
    for platform in SOCIAVAULT_ORDER {
        for account in entity.identifiers.iter().filter(|identifier| identifier.kind == *platform) {
            let Some(action) = grounded(
                "sociavault_profile",
                json!({"platform": account.kind, "handle": account.value}),
                primary,
                &format!(
                    "Tool isolation: SociaVault profile for the {} account {} of {}. {reason}",
                    account.kind, account.value, entity.canonical_name
                ),
                entity.evidence_ids.clone(),
                "Public profile fields. A matching handle does not prove ownership.",
                "The platform and handle were extracted from discovery results about the subject.",
            ) else {
                continue;
            };
            if osint::validate(&action.tool_id, &action.arguments).is_ok() {
                actions.push(price(action, input));
            }
        }
    }
    actions
}

/// SociaVault platforms in the order their profiles are looked up.
#[allow(dead_code)]
const SOCIAVAULT_ORDER: &[&str] = &[
    "twitter", "instagram", "facebook", "youtube", "tiktok", "threads", "linkedin", "twitch",
];

/// Platforms whose handle works as a plain username on another service.
#[allow(dead_code)]
const USERNAME_ORDER: &[&str] = &[
    "twitter", "truthsocial", "instagram", "github", "keybase", "tiktok", "threads", "youtube",
    "twitch",
];

#[allow(dead_code)]
fn provider_label(provider: &str) -> &str {
    match provider {
        "firecrawl" => "Firecrawl",
        "hunter" => "Hunter",
        "sociavault" => "SociaVault",
        other => other,
    }
}

/// Builds tool arguments from the focus subject. Returns none when a required input
/// would have to be guessed (for example a social handle no evidence supports).
#[allow(dead_code)]
fn subject_action(
    tool: &osint::ToolDefinition,
    input: &SelectionInput<'_>,
    reason: &str,
) -> Option<ProposedAction> {
    let subject = subject_of(input.question);
    let focus = subject_entity(input);
    let name = focus
        .map(|entity| entity.canonical_name.clone())
        .unwrap_or_else(|| display_name(&subject));
    if name.trim().is_empty() {
        return None;
    }
    let person = focus
        .map(|entity| entity.entity_type == "person")
        .unwrap_or_else(|| subject_type(input.question, &subject, input.hits) == "person");
    let handle = focus.and_then(|entity| {
        entity
            .identifiers
            .iter()
            .find(|identifier| platform_kind(&identifier.kind))
    });
    // A username for account directories: that service's own handle first, then the
    // subject's best-evidenced handle elsewhere.
    let own_platform = match tool.id {
        "keybase_identity" => "keybase",
        "wikipedia_users" => "wikipedia",
        "github_repositories" => "github",
        _ => "",
    };
    let username = focus.and_then(|entity| {
        std::iter::once(own_platform)
            .chain(USERNAME_ORDER.iter().copied())
            .filter(|platform| !platform.is_empty())
            .find_map(|platform| {
                entity
                    .identifiers
                    .iter()
                    .find(|identifier| identifier.kind == platform)
            })
    });
    let domain = focus
        .and_then(|entity| identifier(entity, "domain").map(str::to_string))
        .or_else(|| explicit_kind(input.question, "domain"));
    let mut arguments = serde_json::Map::new();
    for group in tool.inputs {
        let value = group.split('|').find_map(|key| {
            let value = match key {
                "name" | "query" => Some(name.clone()),
                "full_name" if person => Some(name.clone()),
                "company" | "company_name" if !person => Some(name.clone()),
                "username" => username.map(|identifier| identifier.value.clone()),
                "handle" => handle.map(|identifier| identifier.value.clone()),
                "platform" => handle.map(|identifier| identifier.kind.clone()),
                "linkedin_handle" => handle
                    .filter(|identifier| identifier.kind == "linkedin")
                    .map(|identifier| identifier.value.clone()),
                "domain" | "domain_or_url" | "domain_or_ip" => domain.clone(),
                "ip" | "url" | "cve" => explicit_kind(input.question, key),
                "cve_id" => explicit_kind(input.question, "cve"),
                "email" => emails_in(input.question).into_iter().next(),
                "bitcoin_address" => bitcoin_in(input.question),
                _ => None,
            }?;
            Some((key, value))
        });
        let (key, value) = value?;
        arguments.insert(key.into(), Value::String(value));
    }
    let primary = input
        .gaps
        .iter()
        .find(|gap| gap.kind == gap_kind(input.question))
        .or_else(|| input.gaps.first())?;
    let evidence = focus
        .map(|entity| entity.evidence_ids.clone())
        .filter(|ids| !ids.is_empty())
        .unwrap_or_else(|| vec!["question".into()]);
    let action = grounded(
        tool.id,
        Value::Object(arguments),
        primary,
        &format!("Tool isolation: {} for {name}. {reason}", tool.name),
        evidence,
        &format!(
            "{} for {name}. Matches are candidates until another source corroborates them.",
            tool.description.trim_end_matches('.')
        ),
        "Inputs come from the focus subject, not from search-result publishers.",
    )?;
    osint::validate(&action.tool_id, &action.arguments).ok()?;
    Some(price(action, input))
}

/// One online account of the subject, with where it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub platform: String,
    pub handle: String,
    pub evidence_id: String,
    /// `model`, `pattern`, or both.
    pub sources: Vec<String>,
}

/// Platforms whose accounts Recon extracts and attaches to the subject.
pub const ACCOUNT_PLATFORMS: &[&str] = &[
    "twitter", "truthsocial", "instagram", "facebook", "youtube", "tiktok", "threads", "linkedin",
    "twitch", "github", "keybase", "wikipedia", "pinterest",
];

/// Sites that host accounts. Their domains are never organizations in a result set.
fn account_platform_host(host: &str) -> bool {
    const HOSTS: &[&str] = &[
        "x.com", "twitter.com", "truthsocial.com", "instagram.com", "facebook.com", "fb.com",
        "youtube.com", "youtu.be", "tiktok.com", "threads.net", "linkedin.com", "twitch.tv",
        "github.com", "gitlab.com", "keybase.io", "reddit.com", "pinterest.com", "medium.com", "substack.com",
        "rumble.com", "gettr.com", "parler.com", "bsky.app", "mastodon.social", "linktr.ee",
    ];
    HOSTS
        .iter()
        .any(|known| host == *known || host.ends_with(&format!(".{known}")))
}

/// Deterministic extraction: profile URLs (x.com/<h>, twitter.com/<h>, truthsocial.com/@<h>,
/// instagram.com/<h>, facebook.com/<h>, github.com/<h>, keybase.io/<h>, youtube.com/@<h>, …)
/// in result links and text. Keeps only accounts that belong to the subject.
pub fn fallback_accounts(question: &str, hits: &[SearchHit]) -> Vec<Account> {
    let subject = subject_of(question);
    let mut accounts = Vec::new();
    for hit in hits {
        let titled = names_subject(&subject, &hit.title);
        for handle in super::extract_social_handles(std::slice::from_ref(&hit.url)) {
            if titled || names_subject(&subject, &handle.handle) {
                push_account(&mut accounts, &handle.platform, &handle.handle, &hit.evidence_id, "pattern");
            }
        }
        for handle in super::extract_social_handles(&[hit.title.clone(), hit.snippet.clone()]) {
            if names_subject(&subject, &handle.handle) {
                push_account(&mut accounts, &handle.platform, &handle.handle, &hit.evidence_id, "pattern");
            }
        }
    }
    accounts
}

/// Accounts the Recon model extracted. Each must use a known platform, be a valid handle,
/// appear in the results, and belong to the subject (its handle or its result names it).
#[allow(dead_code)]
pub fn accounts_from_model(value: &Value, question: &str, hits: &[SearchHit]) -> Vec<Account> {
    let subject = subject_of(question);
    let mut accounts = Vec::new();
    let Some(rows) = value.get("accounts").and_then(Value::as_array) else {
        return accounts;
    };
    for row in rows.iter().take(24) {
        let platform = row
            .get("platform")
            .and_then(Value::as_str)
            .map(normalize_platform)
            .unwrap_or_default();
        let raw = row.get("handle").and_then(Value::as_str).unwrap_or("").trim();
        if raw.is_empty() {
            continue;
        }
        let (platform, handle) = if raw.contains('/') {
            match super::extract_social_handles(&[raw.to_string()]).into_iter().next() {
                Some(found) => (found.platform, found.handle),
                None => continue,
            }
        } else {
            let Ok(token) = osint::social_token(raw) else {
                continue;
            };
            let handle = match platform.as_str() {
                "facebook" => format!("https://www.facebook.com/{token}"),
                "linkedin" => format!("https://www.linkedin.com/in/{token}"),
                _ => token,
            };
            (platform, handle)
        };
        if !ACCOUNT_PLATFORMS.contains(&platform.as_str()) {
            continue;
        }
        let needle = handle
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(&handle)
            .to_ascii_lowercase();
        let Some(source) = hits.iter().find(|hit| {
            appears_as_token(&format!("{} {} {}", hit.url, hit.title, hit.snippet), &needle)
        }) else {
            continue;
        };
        // A handle that does not carry the subject's name needs its own profile page
        // titled with the subject; a mention inside an article about the subject is not enough.
        let profile = hits.iter().any(|hit| {
            appears_as_token(&hit.url, &needle) && names_subject(&subject, &hit.title)
        });
        if names_subject(&subject, &handle) || profile {
            push_account(&mut accounts, &platform, &handle, &source.evidence_id, "model");
        }
    }
    accounts
}

/// The handle appears whole in the text, not inside a longer handle or word.
#[allow(dead_code)]
fn appears_as_token(text: &str, needle: &str) -> bool {
    let text = text.to_ascii_lowercase();
    let part = |ch: Option<char>| ch.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_');
    !needle.is_empty()
        && text.match_indices(needle).any(|(index, _)| {
            !part(text[..index].chars().next_back()) && !part(text[index + needle.len()..].chars().next())
        })
}

/// Tools the Recon model picked to answer the question, limited to enabled, non-Firecrawl tools.
#[allow(dead_code)]
pub fn model_tool_picks(value: &Value, enabled: &HashSet<String>) -> Vec<ToolSuggestion> {
    let mut tools: Vec<ToolSuggestion> = Vec::new();
    for row in value.get("tools").and_then(Value::as_array).into_iter().flatten().take(12) {
        let Some(id) = row.get("tool_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(tool) = osint::definition(id) else {
            continue;
        };
        if tool.id.starts_with("firecrawl_")
            || !enabled.contains(tool.id)
            || tools.iter().any(|item| item.tool_id == tool.id)
        {
            continue;
        }
        let reason = row
            .get("reason")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|reason| !reason.is_empty())
            .map(clip_query)
            .unwrap_or_else(|| format!("The Recon model picked {} for this question.", tool.name));
        tools.push(ToolSuggestion {
            tool_id: tool.id.into(),
            reason,
        });
    }
    tools
}

#[allow(dead_code)]
fn normalize_platform(value: &str) -> String {
    let compact: String = value
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect();
    match compact.as_str() {
        "x" | "xcom" | "twitter" | "xtwitter" | "twitterx" => "twitter".into(),
        "truth" | "truthsocial" => "truthsocial".into(),
        "ig" | "insta" | "instagram" => "instagram".into(),
        "fb" | "facebook" => "facebook".into(),
        "yt" | "youtube" => "youtube".into(),
        other => other.into(),
    }
}

fn push_account(accounts: &mut Vec<Account>, platform: &str, handle: &str, evidence: &str, source: &str) {
    if let Some(existing) = accounts.iter_mut().find(|account| {
        account.platform == platform && account.handle.eq_ignore_ascii_case(handle)
    }) {
        if !existing.sources.iter().any(|item| item == source) {
            existing.sources.push(source.into());
        }
        return;
    }
    accounts.push(Account {
        platform: platform.into(),
        handle: handle.into(),
        evidence_id: evidence.into(),
        sources: vec![source.into()],
    });
}

/// Model accounts first, then pattern accounts, deduplicated by platform and handle.
#[allow(dead_code)]
pub fn merge_accounts(model: &[Account], pattern: &[Account]) -> Vec<Account> {
    let mut merged = Vec::new();
    for account in model.iter().chain(pattern) {
        for source in &account.sources {
            push_account(&mut merged, &account.platform, &account.handle, &account.evidence_id, source);
        }
    }
    merged.truncate(16);
    merged
}

/// Attaches the accounts to the subject entity as handles, creating the subject entity
/// when the results named only its accounts. Account platforms never become entities.
#[allow(dead_code)]
pub fn attach_accounts(
    question: &str,
    hits: &[SearchHit],
    entities: &mut Vec<SelectedEntity>,
    accounts: &[Account],
) {
    if accounts.is_empty() {
        return;
    }
    let focus = Focus::new(question, hits);
    let index = match entities.iter().position(|entity| {
        entity.canonical_name.eq_ignore_ascii_case(&focus.name)
            || content_tokens(&entity.canonical_name) == content_tokens(&focus.subject)
    }) {
        Some(index) => index,
        None => {
            entities.insert(
                0,
                SelectedEntity {
                    canonical_name: focus.name.clone(),
                    entity_type: focus.entity_type.into(),
                    identifiers: Vec::new(),
                    evidence_ids: Vec::new(),
                    relationships: Vec::new(),
                    unresolved: vec!["needs a second independent source".into()],
                    certainty: "medium".into(),
                    why: "Online accounts in the discovery results belong to the subject.".into(),
                    ambiguous: false,
                    selected: true,
                },
            );
            let mut chosen = 0;
            for entity in entities.iter_mut() {
                if entity.selected {
                    chosen += 1;
                    entity.selected = chosen <= 3;
                }
            }
            0
        }
    };
    let entity = &mut entities[index];
    if entity.ambiguous {
        return;
    }
    for account in accounts {
        if !entity.identifiers.iter().any(|identifier| {
            identifier.kind == account.platform && identifier.value.eq_ignore_ascii_case(&account.handle)
        }) {
            entity.identifiers.push(EntityIdentifier {
                kind: account.platform.clone(),
                value: account.handle.clone(),
            });
        }
        if !entity.evidence_ids.contains(&account.evidence_id) {
            entity.evidence_ids.push(account.evidence_id.clone());
        }
    }
}

/// `twitter @realDonaldTrump (model, pattern)`, for the decision block.
#[allow(dead_code)]
pub fn account_line(account: &Account) -> String {
    let handle = if account.handle.contains('/') {
        account.handle.clone()
    } else {
        format!("@{}", account.handle)
    };
    format!("{} {handle} ({})", account.platform, account.sources.join(", "))
}

#[allow(dead_code)]
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

#[allow(dead_code)]
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
#[allow(dead_code)]
pub struct DiscoveryQuery {
    pub role: String,
    pub query: String,
    pub angle: String,
}

#[allow(dead_code)]
pub fn complementary_queries(question: &str, strategy: &str) -> [DiscoveryQuery; 2] {
    let subject = subject_of(question);
    let identity = clip_query(&format!("{subject} official name identifiers"));
    if accounts_flow(question, strategy) {
        return [
            DiscoveryQuery {
                role: "identity".into(),
                query: identity,
                angle: "Establish the subject and its authoritative identifiers.".into(),
            },
            DiscoveryQuery {
                role: ACCOUNTS.into(),
                query: clip_query(&format!(
                    "{subject} official social media accounts profiles handles"
                )),
                angle: "Find the subject's associated online accounts: X/Twitter, Truth Social, Instagram, Facebook, YouTube, GitHub, Keybase.".into(),
            },
        ];
    }
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

/// Query role for the associated-accounts search.
pub const ACCOUNTS: &str = "accounts";

/// Discovery for a person or organization searches for associated online accounts
/// second and extracts their handles. Topics, events, places, products, and technical
/// identifiers keep the investigative-question search.
#[allow(dead_code)]
pub fn accounts_flow(question: &str, strategy: &str) -> bool {
    strategy == DISCOVERY && matches!(target_kind(question), "person" | "organization")
}

/// Whether a query looks for online accounts rather than restating the question.
#[allow(dead_code)]
pub fn accounts_query(query: &str) -> bool {
    let query = query.to_ascii_lowercase();
    ["account", "profile", "handle", "social", "twitter", "instagram", "official site"]
        .iter()
        .any(|word| query.contains(word))
}

/// What the question targets, from the question alone: `person`, `organization`, or
/// `other` (topic, event, place, product, or a technical identifier).
pub fn target_kind(question: &str) -> &'static str {
    if super::explicit_entities(question)
        .iter()
        .any(|(kind, _)| matches!(kind.as_str(), "domain" | "url" | "ip" | "cve"))
        || !emails_in(question).is_empty()
        || bitcoin_in(question).is_some()
    {
        return "other";
    }
    let subject = subject_of(question);
    let words: Vec<String> = subject
        .split_whitespace()
        .map(|word| word.trim_matches(|ch: char| !ch.is_alphanumeric()).to_ascii_lowercase())
        .filter(|word| !word.is_empty())
        .collect();
    if words.is_empty() {
        return "other";
    }
    const OTHER_WORDS: &[&str] = &[
        "war", "election", "elections", "crisis", "attack", "shooting", "hurricane",
        "earthquake", "storm", "wildfire", "protest", "protests", "pandemic", "outbreak",
        "scandal", "trial", "summit", "olympics", "conference", "act", "bill", "policy", "law",
        "regulation", "price", "prices", "market", "inflation", "economy", "climate", "history",
        "vulnerability", "exploit", "malware", "ransomware", "breach", "leak", "incident",
        "release", "version", "update", "game", "movie", "film", "album", "book", "song",
        "product", "phone", "city", "country", "river", "mountain", "island", "street",
        "county", "province", "region", "tariff", "tariffs", "start", "started", "begin",
    ];
    if words.iter().any(|word| OTHER_WORDS.contains(&word.as_str())) {
        return "other";
    }
    if words.iter().any(|word| ORG_WORDS.contains(&word.as_str())) {
        return "organization";
    }
    let asked = format!(
        " {} ",
        question
            .to_ascii_lowercase()
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
            .collect::<String>()
    );
    let cue = |cues: &[&str]| cues.iter().any(|cue| asked.contains(cue));
    if cue(&[" company ", " organization ", " organisation ", " firm ", " brand ", " its "]) {
        return "organization";
    }
    if cue(&[" his ", " her ", " him ", " he ", " she ", " himself ", " herself "]) {
        return "person";
    }
    let who = asked.trim_start().starts_with("who ");
    let capitalized = subject
        .split_whitespace()
        .all(|word| word.chars().next().is_some_and(char::is_uppercase));
    let alphabetic = subject
        .split_whitespace()
        .all(|word| word.chars().all(|ch| ch.is_alphabetic() || matches!(ch, '.' | '-' | '\'')));
    if alphabetic && (2..=4).contains(&words.len()) && (who || capitalized) {
        return "person";
    }
    if alphabetic && words.len() == 1 && (who || capitalized) {
        return "organization";
    }
    "other"
}

#[allow(dead_code)]
pub fn distinct_queries(left: &str, right: &str) -> bool {
    let left_tokens = content_tokens(left);
    let right_tokens = content_tokens(right);
    !left.trim().eq_ignore_ascii_case(right.trim())
        && !left_tokens.is_empty()
        && left_tokens != right_tokens
}

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

pub fn select_entities(question: &str, hits: &[SearchHit]) -> Vec<SelectedEntity> {
    let focus = Focus::new(question, hits);
    let subject = focus.subject.clone();
    let mut groups: Vec<Candidate> = Vec::new();
    for hit in hits {
        absorb_hit(&mut groups, &focus, hit);
    }
    let mut entities: Vec<SelectedEntity> = groups
        .into_iter()
        .filter_map(|candidate| candidate.finish(&subject))
        .collect();
    let person_names: Vec<String> = entities
        .iter()
        .filter(|entity| entity.entity_type == "person" && entity.selected)
        .filter(|entity| names_subject(&subject, &entity.canonical_name))
        .map(|entity| entity.canonical_name.to_ascii_lowercase())
        .collect();
    if person_names.len() > 1 {
        for entity in &mut entities {
            if entity.entity_type == "person" && names_subject(&subject, &entity.canonical_name) {
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
    entities.sort_by_key(|entity| {
        let (certainty, sources, name) = rank_key(entity);
        (u8::from(!entity.canonical_name.eq_ignore_ascii_case(&focus.name)), certainty, sources, name)
    });
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

/// The subject the user asked about: how it is matched, displayed, and typed.
/// Search-result publishers never become this subject or its identifiers.
struct Focus {
    subject: String,
    name: String,
    entity_type: &'static str,
    explicit_domain: Option<String>,
}

impl Focus {
    fn new(question: &str, hits: &[SearchHit]) -> Self {
        let subject = subject_of(question);
        Self {
            name: display_name(&subject),
            entity_type: subject_type(question, &subject, hits),
            explicit_domain: explicit_kind(question, "domain"),
            subject,
        }
    }

    /// Whether this host is the subject's own site rather than a page about it.
    fn owns(&self, host: &str) -> bool {
        if let Some(domain) = &self.explicit_domain {
            return host == domain || host.ends_with(&format!(".{domain}"));
        }
        // A subject-named page on a wiki, Q&A, news, or account host (elonmusk.fandom.com)
        // is a citation, not the subject's own site.
        if publisher_host(host) || account_platform_host(host) {
            return false;
        }
        let tokens = content_tokens(&self.subject);
        if tokens.is_empty() {
            return false;
        }
        let labels: Vec<&str> = host.split('.').collect();
        let site: String = labels[..labels.len().saturating_sub(1)]
            .concat()
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .collect();
        tokens.iter().all(|token| site.contains(token.as_str()))
    }

    /// The canonical subject name and type when `name` refers to the subject itself.
    fn canonical(&self, name: &str, fallback_type: &'static str) -> (String, &'static str) {
        let tokens = content_tokens(name);
        if !tokens.is_empty() && tokens == content_tokens(&self.subject) {
            (self.name.clone(), self.entity_type)
        } else {
            (name.trim().to_string(), fallback_type)
        }
    }
}

/// Title-cases an all-lowercase subject such as `donald trump`; domains and typed casing stay.
fn display_name(subject: &str) -> String {
    if subject.chars().any(|ch| ch.is_uppercase())
        || subject.split_whitespace().any(|word| word.contains('.') && !word.ends_with('.'))
    {
        return subject.to_string();
    }
    subject
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Words that make a subject an organization.
const ORG_WORDS: &[&str] = &[
    "inc", "corp", "corporation", "llc", "ltd", "limited", "company", "group", "holdings",
    "university", "college", "school", "foundation", "institute", "association", "agency",
    "bank", "party", "department", "ministry", "council", "committee", "technologies", "labs",
    "media", "gmbh", "plc", "trust", "fund", "partners", "club", "network", "news", "times",
    "systems", "solutions", "studios", "records", "airlines", "motors",
];

/// Person or organization, from the question, the subject's shape, and how sources describe it.
fn subject_type(question: &str, subject: &str, hits: &[SearchHit]) -> &'static str {
    if super::explicit_entities(subject)
        .iter()
        .any(|(kind, _)| matches!(kind.as_str(), "domain" | "url" | "ip"))
    {
        return "organization";
    }
    let words: Vec<String> = subject
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
                .to_ascii_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect();
    if words.iter().any(|word| ORG_WORDS.contains(&word.as_str())) {
        return "organization";
    }
    let spaced = |text: &str| {
        format!(
            " {} ",
            text.to_ascii_lowercase()
                .chars()
                .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
                .collect::<String>()
        )
    };
    let asked = spaced(question);
    let has = |text: &str, cues: &[&str]| cues.iter().filter(|cue| text.contains(*cue)).count();
    if has(&asked, &[" his ", " her ", " him ", " he ", " she ", " himself ", " herself "]) > 0 {
        return "person";
    }
    if has(&asked, &[" its ", " company ", " organization ", " firm ", " brand "]) > 0 {
        return "organization";
    }
    let mut person = 0;
    let mut organization = 0;
    for hit in hits {
        if !names_subject(subject, &hit.title) && !names_subject(subject, &hit.snippet) {
            continue;
        }
        let text = spaced(&hit.snippet);
        person += has(
            &text,
            &[
                " born ", " politician ", " businessman ", " businesswoman ", " actor ",
                " actress ", " singer ", " musician ", " author ", " journalist ", " he ",
                " she ", " his ", " her ",
            ],
        );
        organization += has(
            &text,
            &[
                " company ", " corporation ", " headquartered ", " subsidiary ", " nonprofit ",
                " organization ", " its ",
            ],
        );
    }
    if person != organization {
        return if person > organization { "person" } else { "organization" };
    }
    let name_shaped = (2..=4).contains(&words.len())
        && subject
            .split_whitespace()
            .all(|word| word.chars().all(|ch| ch.is_alphabetic() || matches!(ch, '.' | '-' | '\'')));
    if name_shaped {
        "person"
    } else {
        "organization"
    }
}

/// News and reference publishers whose pages are citations, never entities.
fn publisher_host(host: &str) -> bool {
    const OUTLETS: &[&str] = &[
        "apnews", "reuters", "cnn", "foxnews", "nbcnews", "cbsnews", "abcnews", "npr", "pbs",
        "politico", "axios", "thehill", "rollcall", "usatoday", "wsj", "washingtonpost",
        "nytimes", "bbc", "theguardian", "bloomberg", "forbes", "newsweek", "time", "latimes",
        "cnbc", "msnbc", "aljazeera", "theatlantic", "vox", "huffpost", "businessinsider",
        "yahoo", "msn", "britannica", "factcheck", "snopes", "politifact", "ballotpedia",
        "c-span", "cspan", "vanityfair", "newyorker", "slate", "salon", "independent",
        "telegraph", "economist", "ft", "nypost", "dailymail", "mediaite", "semafor",
    ];
    const NEWS_WORDS: &[&str] = &[
        "times", "post", "tribune", "herald", "gazette", "journal", "daily", "press",
        "chronicle", "reporter", "magazine", "radio", "broadcast", "courier", "observer",
    ];
    let labels: Vec<&str> = host.split('.').collect();
    let mut index = labels.len().saturating_sub(2);
    if labels.len() >= 3 && matches!(labels[index], "co" | "com" | "org" | "net" | "ac" | "gov") {
        index -= 1;
    }
    let site = labels.get(index).copied().unwrap_or(host);
    // US broadcast call signs such as kark.com or wfaa.com are local news stations.
    let call_sign = site.starts_with(['k', 'w'])
        && (site.len() == 4 || (site.len() == 6 && site.ends_with("tv")))
        && site.chars().all(|ch| ch.is_ascii_lowercase());
    // Q&A, wiki, aggregator, scraper-marketplace, and other user-generated sites: a page
    // there is a citation about the subject, never the subject's own site or name.
    const CITATION_SITES: &[&str] = &[
        "quora", "reddit", "medium", "fandom", "wikia", "answers", "ask", "wikipedia", "wikiwand",
        "wikimili", "wikimedia", "wikidata", "everybodywiki", "dbpedia", "famousbirthdays",
        "celebritynetworth", "imdb", "pinterest", "tumblr", "scribd", "slideshare", "socialblade",
        "stackexchange", "stackoverflow", "apify", "rapidapi", "brainly", "chegg", "crunchbase",
        "ranker", "statista", "similarweb", "trustpilot", "glassdoor", "zoominfo", "rocketreach",
        "linktree", "substack", "blogspot", "wordpress", "weebly", "wix", "tiktokcounter",
        "livecounts", "socialcounts", "hypeauditor", "noxinfluencer", "influencermarketinghub",
    ];
    OUTLETS.contains(&site)
        || CITATION_SITES.contains(&site)
        || labels.iter().any(|label| matches!(*label, "quora" | "fandom" | "wikia" | "wikipedia" | "answers"))
        || call_sign
        || site.contains("news")
        || NEWS_WORDS
            .iter()
            .any(|word| site.ends_with(word) || (site.starts_with(word) && *word != "post"))
}

/// A shallow page whose title names the site itself, such as a homepage or about page.
/// Deeper article pages describe a subject; their site is the publisher.
fn site_page(host: &str, hit: &SearchHit) -> bool {
    let Ok(url) = url::Url::parse(&hit.url) else {
        return false;
    };
    let depth = url
        .path_segments()
        .map(|segments| segments.filter(|segment| !segment.is_empty()).count())
        .unwrap_or(0);
    if depth > 1 {
        return false;
    }
    let label = domain_label(host);
    let title: String = hit
        .title
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    !label.is_empty()
        && (title.contains(&label)
            || content_tokens(&hit.title)
                .iter()
                .any(|token| token.len() >= 4 && label.contains(token.as_str())))
}

/// Every content token of the subject appears in the value.
fn names_subject(subject: &str, value: &str) -> bool {
    let subject_tokens = content_tokens(subject);
    if subject_tokens.is_empty() {
        return false;
    }
    let value_tokens = content_tokens(value);
    if subject_tokens.is_subset(&value_tokens) {
        return true;
    }
    let compact: String = value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    // A possessive with the apostrophe dropped ("musks") still names "musk".
    subject_tokens.iter().all(|token| {
        compact.contains(token.as_str())
            || token.len() > 4 && token.ends_with('s') && !token.ends_with("ss") && compact.contains(&token[..token.len() - 1])
    })
}

/// Capitalized two-to-four word names such as `Eric Trump`, not headlines.
fn looks_like_name(value: &str) -> bool {
    let words: Vec<&str> = value.split_whitespace().collect();
    (2..=4).contains(&words.len())
        && words.iter().all(|word| {
            word.chars().next().is_some_and(char::is_uppercase)
                && word.chars().all(|ch| ch.is_alphabetic() || matches!(ch, '.' | '-'))
        })
}

fn absorb_hit(groups: &mut Vec<Candidate>, focus: &Focus, hit: &SearchHit) {
    let Ok(url) = url::Url::parse(&hit.url) else {
        return;
    };
    let host = url
        .host_str()
        .unwrap_or("")
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    let subject = focus.subject.as_str();
    if let Some(handle) = super::extract_social_handles(std::slice::from_ref(&hit.url))
        .into_iter()
        .next()
    {
        let name = title_name(&hit.title).unwrap_or_else(|| handle.handle.clone());
        if !matches_subject(subject, &name) && !matches_subject(subject, &handle.handle) {
            return;
        }
        let (name, entity_type) = if names_subject(subject, &handle.handle) {
            (focus.name.clone(), focus.entity_type)
        } else {
            focus.canonical(&name, "person")
        };
        let candidate = upsert(groups, &name, entity_type);
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
    if !host.is_empty() && focus.owns(&host) {
        let name = if focus.explicit_domain.is_some() {
            title_name(&hit.title)
                .filter(|title| matches_subject(subject, title))
                .unwrap_or_else(|| domain_label(&host))
        } else {
            focus.name.clone()
        };
        let candidate = upsert(groups, &name, focus.entity_type);
        push_id(
            candidate,
            EntityIdentifier {
                kind: "domain".into(),
                value: host.clone(),
            },
        );
        note_hit(candidate, hit, true, true);
        candidate.why.push(format!(
            "The domain {host} is the subject's own site, so it is an identifier of this entity."
        ));
        candidate
            .relationships
            .push(format!("{host} is a site for {}", candidate.name));
        return;
    }
    let mentioned = matches_subject(subject, &hit.title) || matches_subject(subject, &hit.snippet);
    let own_page = super::enrichable_domain(&host)
        && !publisher_host(&host)
        && !account_platform_host(&host)
        && site_page(&host, hit);
    if own_page && mentioned {
        // A related organization's homepage or about page that mentions the subject.
        let name = domain_label(&host);
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
            "{host} describes its own organization and mentions the subject."
        ));
        candidate
            .relationships
            .push(format!("{host} is a site for {}", candidate.name));
    }
    if names_subject(subject, &hit.title) || names_subject(subject, &hit.snippet) {
        let candidate = upsert(groups, &focus.name, focus.entity_type);
        note_hit(candidate, hit, true, false);
        let why = if host.is_empty() {
            "The subject is named by a source page.".to_string()
        } else {
            format!("The subject is named by a source page. The publisher {host} is a citation, not an entity or identifier.")
        };
        if !candidate.why.contains(&why) {
            candidate.why.push(why);
        }
        return;
    }
    if own_page {
        return;
    }
    if let Some(name) = title_name(&hit.title) {
        let publisher = domain_label(&host);
        let compact: String = name
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        if matches_subject(subject, &name) && looks_like_name(&name) && compact != publisher {
            let candidate = upsert(groups, &name, "person");
            note_hit(candidate, hit, true, false);
            candidate.why.push(
                "A related person is named by a source page. The publisher domain is not an identifier."
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

#[allow(dead_code)]
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

#[allow(dead_code)]
struct NamedAlternative {
    statement: String,
    distinctive: String,
}

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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
    #[allow(dead_code)]
    pub deferred: Vec<ProposedAction>,
    #[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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
        "hunter_company_enrichment" if kind == "technology" => {
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

// ---------------------------------------------------------------------------
// Tool picker contracts: binding vocabulary, declared dependencies, argument
// binding, fallback questions, rule binders, and the deterministic picker.
// ---------------------------------------------------------------------------

use super::{Binding, Directive};

pub(crate) mod directives;
pub use directives::{
    directive_entities, directive_for_target, directive_query, fallback_directives,
    grounded_query, parse_directives, refers_back, relevance_gate, GroundedQuery, QUALIFIERS,
};
mod tool_io;
pub use tool_io::{
    accept_bindings, bind_arguments, catalog_inputs, consumers_of, dependencies, dependency,
    domains_in, input_kinds, known_kind, output_kinds, packages_in, per_platform_targets,
    pickable, plausible_person_name, question_platforms, rule_bindings, tool_row, unmet_kinds,
    unmet_needs, vet_model_bindings, BINDING_KINDS, COORDINATES_KIND, URL_KIND,
};
pub use tool_io::{allowed_producer, binding_allowed, restricted_sources, GATES};
pub use tool_io::{bind_step, evidence_kinds};
use tool_io::coordinates_in_text;

pub(crate) fn social_or_publisher(domain: &str) -> bool {
    let host = domain.trim_start_matches("www.").to_ascii_lowercase();
    account_platform_host(&host) || publisher_host(&host)
}

fn first<'a>(bindings: &'a [Binding], kind: &str) -> Option<&'a Binding> {
    bindings.iter().find(|binding| binding.kind == kind)
}

fn first_domain(bindings: &[Binding]) -> Option<&Binding> {
    bindings
        .iter()
        .find(|binding| binding.kind == "domain" && !social_or_publisher(&binding.value))
}

/// Bindings already known from the user's question: every kind in `PROMPT_KINDS`.
pub fn question_bindings(question: &str) -> Vec<Binding> {
    fn add(found: &mut Vec<Binding>, kind: &str, value: String, qualifier: &str) {
        let value = value.trim().to_string();
        if value.is_empty()
            || found.iter().any(|binding| binding.kind == kind && binding.value.eq_ignore_ascii_case(&value) && binding.qualifier == qualifier)
        {
            return;
        }
        found.push(Binding {
            kind: kind.into(),
            value,
            evidence_id: "question".into(),
            qualifier: qualifier.into(),
            ..Binding::default()
        });
    }
    let mut found: Vec<Binding> = Vec::new();
    let mut explicit = super::explicit_entities(question);
    explicit.sort();
    let emails = emails_in(question);
    for (kind, value) in explicit {
        match kind.as_str() {
            "domain" => {
                let is_email_host = emails.iter().any(|email| email.ends_with(&format!("@{value}")));
                if domains_in(&value).contains(&value) && !account_platform_host(&value) && !is_email_host {
                    add(&mut found, "domain", value, "");
                }
            }
            "ip" | "cve" => add(&mut found, &kind, value, ""),
            "url" => {
                if let Some(host) = url::Url::parse(&value).ok().and_then(|url| url.host_str().map(|host| host.trim_start_matches("www.").to_string())) {
                    if !social_or_publisher(&host) && domains_in(&host).contains(&host) {
                        add(&mut found, "domain", host, "");
                    }
                }
                add(&mut found, URL_KIND, value, "");
            }
            _ => {}
        }
    }
    for email in &emails {
        add(&mut found, "email", email.clone(), "");
        if let Some((_, host)) = email.rsplit_once('@') {
            if !crate::osint::webmail_host(host) {
                add(&mut found, "domain", host.to_string(), "");
            }
        }
    }
    for wallet in tool_io::bitcoins_in(question) {
        add(&mut found, "wallet", wallet, "");
    }
    for package in packages_in(question) {
        add(&mut found, "package", package, "");
    }
    for value in coordinates_in_text(question) {
        add(&mut found, COORDINATES_KIND, value, "");
    }
    for handle in super::extract_social_handles(&[question.to_string()]) {
        add(&mut found, "handle", handle.handle, &handle.platform);
    }
    // A bare `@handle`, or "username X" / "handle X", with no platform named.
    let words: Vec<&str> = question.split_whitespace().collect();
    for (index, word) in words.iter().enumerate() {
        let token = word.trim_matches(|ch: char| matches!(ch, '?' | '!' | ',' | '.' | '"' | '\'' | '(' | ')'));
        let candidate = if let Some(rest) = token.strip_prefix('@') {
            Some(rest)
        } else if index > 0
            && matches!(words[index - 1].to_ascii_lowercase().trim_matches(|ch: char| !ch.is_alphanumeric()), "username" | "handle" | "user" | "alias" | "screenname")
        {
            Some(token)
        } else {
            None
        };
        if let Some(handle) = candidate.and_then(|value| osint::social_token(value).ok()) {
            if handle.len() >= 2 && !found.iter().any(|binding| binding.kind == "handle" && binding.value.eq_ignore_ascii_case(&handle)) {
                add(&mut found, "handle", handle, "");
            }
        }
    }
    let subject = super::question_subject(question);
    let name = display_name(&subject);
    match target_kind(question) {
        "person" if plausible_person_name(&name) => add(&mut found, "person_name", name, ""),
        "organization" if !subject.is_empty() && name.split_whitespace().count() <= 8 => add(&mut found, "org_name", name, ""),
        _ => {}
    }
    if gap_kind(question) == "place" && !subject.is_empty() {
        add(&mut found, "address", subject, "");
    }
    found
}

/// Handles the derived questions name ("the follower count of Twitter handle @elonmusk"):
/// handle and platform bindings with the question id as evidence, marked `unverified`.
/// The handle must occur verbatim in that question, must name the subject (a derived
/// question is model text), and is skipped when the user's own question already gave it.
pub fn derived_question_handles(question: &str, questions: &[Directive], known: &[Binding]) -> Vec<Binding> {
    let subject = subject_of(question);
    let mut found: Vec<Binding> = Vec::new();
    for item in questions {
        let text = item.goal.as_str();
        let mut pairs: Vec<(String, String)> = super::extract_social_handles(&[text.to_string()])
            .into_iter()
            .map(|handle| (handle.handle, handle.platform))
            .collect();
        for word in text.split_whitespace() {
            let token = word.trim_matches(|ch: char| matches!(ch, '?' | '!' | ',' | '.' | '"' | '\'' | '(' | ')' | '\u{201c}' | '\u{201d}'));
            if let Some(handle) = token.strip_prefix('@').and_then(|value| osint::social_token(value).ok()) {
                if handle.len() >= 2 && !pairs.iter().any(|(known, _)| known.eq_ignore_ascii_case(&handle)) {
                    pairs.push((handle, String::new()));
                }
            }
        }
        for (handle, platform) in pairs {
            let verbatim = text.to_ascii_lowercase().contains(&handle.to_ascii_lowercase());
            let duplicate = known.iter().chain(found.iter()).any(|binding| {
                binding.kind == "handle" && binding.value.eq_ignore_ascii_case(&handle) && (binding.qualifier == platform || platform.is_empty())
            });
            if verbatim && !duplicate && names_subject(&subject, &handle) {
                found.push(Binding {
                    kind: "handle".into(),
                    value: handle,
                    evidence_id: item.id.clone(),
                    qualifier: platform,
                    unverified: true,
                    ..Binding::default()
                });
            }
        }
    }
    found
}

/// The one accounts search a starved handle step may add: the entity of the directive
/// that targets handles plus the fixed `official account` qualifier (`Elon Musk official
/// account`), with its grounding label. `None` without a directive entity.
pub fn accounts_search_query(question: &str, directives: &[Directive]) -> Option<(String, GroundedQuery)> {
    let fallback;
    let directives = if directives.is_empty() {
        fallback = fallback_directives(question, &[]);
        fallback.as_slice()
    } else {
        directives
    };
    let directive = directive_for_target(directives, "handle").or_else(|| directives.first())?;
    let entity = directive.entities.first()?;
    let qualifier = QUALIFIERS.iter().find(|(kind, _)| *kind == "handle").map(|(_, value)| *value).unwrap_or("");
    let query = format!("{entity} {qualifier}");
    grounded_query(&query, &directive.entities, &[]).then(|| {
        (directive.id.clone(), GroundedQuery { query, source: format!("{} entity + qualifier", directive.id) })
    })
}

/// Default picking ladders when the picker model is unavailable, by what the question
/// already provides.
fn ladder(question: &str, bindings: &[Binding]) -> Vec<&'static str> {
    let mut order: Vec<&'static str> = Vec::new();
    // Identifier kinds first, each with the tools the table says consume it.
    for kind in ["cve", "wallet", "email", "ip", "domain", "address", COORDINATES_KIND, "package", URL_KIND, "handle"] {
        if bindings.iter().any(|binding| binding.kind == kind) {
            for tool in consumers_of(kind) {
                if !order.contains(&tool) {
                    order.push(tool);
                }
            }
        }
    }
    match target_kind(question) {
        // Primary providers first (Firecrawl, SociaVault, Hunter), then gap-fillers.
        "person" => order.extend([
            "firecrawl_search", "sociavault_profile", "sociavault_search_users", "sociavault_user_content", "firecrawl_scrape",
            "hunter_person_enrichment", "wikidata_entities", "keybase_identity", "stackexchange_users", "github_repositories",
        ]),
        "organization" => order.extend([
            "firecrawl_search", "hunter_domain_finder", "hunter_company_enrichment", "firecrawl_map", "firecrawl_batch_scrape",
            "hunter_email_count", "hunter_domain_search", "sociavault_profile", "wikidata_entities", "gleif_entities",
            "sec_submissions", "crtsh_certificates", "firecrawl_scrape",
        ]),
        _ => order.extend(["firecrawl_search", "sociavault_search", "firecrawl_scrape", "wikidata_entities", "github_repositories"]),
    }
    order
}

/// Deterministic picker: `rank_actions` on the questions and known bindings first, then
/// the default ladder, keeping only candidates not already picked. Unkeyed tools go last.
pub fn fallback_order(
    question: &str,
    questions: &[Directive],
    bindings: &[Binding],
    candidates: &[String],
    unkeyed: &HashSet<String>,
) -> Vec<String> {
    let named = first(bindings, "org_name")
        .map(|binding| (binding.value.clone(), "organization"))
        .or_else(|| first(bindings, "person_name").map(|binding| (binding.value.clone(), "person")));
    let mut entities = Vec::new();
    if let Some((name, entity_type)) = named {
        let mut identifiers = Vec::new();
        if let Some(domain) = first_domain(bindings) {
            identifiers.push(EntityIdentifier { kind: "domain".into(), value: domain.value.clone() });
        }
        for handle in bindings.iter().filter(|binding| binding.kind == "handle") {
            identifiers.push(EntityIdentifier { kind: handle.qualifier.clone(), value: handle.value.clone() });
        }
        entities.push(SelectedEntity {
            canonical_name: name,
            entity_type: entity_type.into(),
            identifiers,
            evidence_ids: vec!["question".into()],
            relationships: Vec::new(),
            unresolved: Vec::new(),
            certainty: "probable".into(),
            why: "Known binding".into(),
            ambiguous: false,
            selected: true,
        });
    }
    let mut gaps = gaps_for(question, DISCOVERY, &entities, None);
    for item in questions {
        gaps.push(Gap {
            id: item.id.clone(),
            question: item.goal.clone(),
            kind: gap_kind(&item.goal).into(),
        });
    }
    let enabled: HashSet<String> = candidates.iter().cloned().collect();
    let empty = HashSet::new();
    let credits = HashMap::new();
    let costs = HashMap::new();
    let ranked = rank_actions(&SelectionInput {
        question,
        strategy: DISCOVERY,
        opening: false,
        entities: &entities,
        gaps: &gaps,
        enabled: &enabled,
        already: &empty,
        cached: &empty,
        hunter_cap: 2,
        sociavault_cap: 2,
        credits_left: &credits,
        costs: &costs,
        hits: &[],
    });
    let mut order: Vec<String> = Vec::new();
    let ranked_ids = ranked.actions.iter().map(|action| action.tool_id.as_str());
    for id in ranked_ids.chain(ladder(question, bindings)) {
        if enabled.contains(id) && !order.iter().any(|known| known == id) {
            order.push(id.to_string());
        }
    }
    order.sort_by_key(|id| u8::from(unkeyed.contains(id)));
    order
}

/// Stable dependency fix on the pick order: when a tool's declared need is not known
/// yet and a declared producer (or a producer named by the chat reply) was picked later,
/// that producer moves to just before it. No model calls.
pub fn dependency_order(
    picked: &[String],
    known: &[Binding],
    chat_produces: &HashMap<String, Vec<String>>,
) -> Vec<String> {
    let mut order: Vec<String> = picked.to_vec();
    let produces = |tool: &str, kind: &str| {
        output_kinds(tool).contains(&kind)
            || chat_produces.get(tool).is_some_and(|kinds| kinds.iter().any(|item| item == kind))
    };
    for _ in 0..(order.len() * order.len() + 1) {
        let mut moved = false;
        'scan: for index in 0..order.len() {
            let tool = order[index].clone();
            let Some(row) = dependency(&tool) else { continue };
            let unmet = unmet_kinds(&tool, known);
            for group in &row.needs {
                if !unmet.contains(group) {
                    continue;
                }
                let earlier = order[..index].iter().any(|other| {
                    allowed_producer(&tool, other) && (row.producers.contains(&other.as_str()) || group.iter().any(|kind| produces(other, kind)))
                });
                if earlier {
                    continue;
                }
                let later = order[index + 1..]
                    .iter()
                    .position(|other| row.producers.contains(&other.as_str()))
                    .map(|offset| index + 1 + offset);
                if let Some(at) = later {
                    let producer = order.remove(at);
                    order.insert(index, producer);
                    moved = true;
                    break 'scan;
                }
            }
        }
        // Gates: email count before domain search, email insight before enrichment.
        if !moved {
            for (first, second) in GATES {
                let at_first = order.iter().position(|id| canonical(id) == *first);
                let at_second = order.iter().position(|id| canonical(id) == *second);
                if let (Some(at_first), Some(at_second)) = (at_first, at_second) {
                    if at_first > at_second {
                        let gate = order.remove(at_first);
                        order.insert(at_second, gate);
                        moved = true;
                        break;
                    }
                }
            }
        }
        if !moved {
            break;
        }
    }
    order
}

fn canonical(id: &str) -> &str {
    crate::osint::canonical_tool_id(id)
}

/// Earlier steps whose output a step needs: declared producers and output kinds for
/// unmet declared needs, plus the chat reply's `needs` against earlier `produces`.
pub fn depends_on(
    order: &[String],
    index: usize,
    known: &[Binding],
    chat_needs: &HashMap<String, Vec<String>>,
    chat_produces: &HashMap<String, Vec<String>>,
) -> Vec<usize> {
    let tool = &order[index];
    let mut wanted: Vec<String> = unmet_needs(tool, known)
        .iter()
        .flat_map(|group| group.split(" or ").map(String::from).collect::<Vec<_>>())
        .collect();
    for kind in chat_needs.get(tool).into_iter().flatten() {
        if !known.iter().any(|binding| &binding.kind == kind) && !wanted.contains(kind) {
            wanted.push(kind.clone());
        }
    }
    let producers = dependency(tool).map(|row| row.producers).unwrap_or(&[]);
    let mut deps = Vec::new();
    for (earlier, other) in order[..index].iter().enumerate() {
        let declared = !wanted.is_empty() && producers.contains(&other.as_str());
        let yields = allowed_producer(tool, other)
            && wanted.iter().any(|kind| {
                output_kinds(other).contains(&kind.as_str())
                    || chat_produces.get(other).is_some_and(|kinds| kinds.contains(kind))
            });
        let gated = GATES.iter().any(|(first, second)| *first == canonical(other) && *second == canonical(tool));
        if declared || yields || gated {
            deps.push(earlier);
        }
    }
    deps
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
            .all(|action| action.tool_id != "hunter_company_enrichment"));
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
            .any(|action| action.tool_id == "hunter_company_enrichment"));
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

    const TRUMP: &str = "what can you tell me about donald trump and his social media activity?";

    fn trump_hits() -> Vec<SearchHit> {
        dedupe_hits(vec![
            hit("e1", "Trump posts dozens of times overnight on Truth Social | AP News", "https://apnews.com/article/trump-truth-social-posts-overnight-0a1b2c", "President Donald Trump posted dozens of times on his social media platform overnight.", "investigative"),
            hit("e1", "Trump's social media activity draws scrutiny", "https://www.kark.com/news/politics/trump-social-media-activity/", "Donald Trump's posts on Truth Social and X drew new scrutiny this week.", "investigative"),
            hit("e2", "Donald Trump - Roll Call Factba.se", "https://rollcall.com/factbase/trump/topic/social-media/", "Every Donald Trump social media post, speech, and interview.", "identity"),
            hit("e2", "Donald J. Trump (@realDonaldTrump) / X", "https://x.com/realDonaldTrump", "45th and 47th President of the United States.", "identity"),
            hit("e2", "Donald Trump - Wikipedia", "https://en.wikipedia.org/wiki/Donald_Trump", "Donald John Trump is an American politician and businessman.", "identity"),
            hit("e1", "AP News: Breaking News, Latest News and Videos", "https://apnews.com/", "Latest on Donald Trump and the White House.", "investigative"),
        ])
    }

    #[test]
    fn trump_question_extracts_the_subject_not_the_source_publishers() {
        let entities = select_entities(TRUMP, &trump_hits());
        for publisher in ["apnews", "kark", "rollcall", "ap news", "roll call"] {
            assert!(
                entities.iter().all(|entity| !entity.canonical_name.eq_ignore_ascii_case(publisher)),
                "{publisher} became an entity: {entities:?}"
            );
        }
        assert!(entities.iter().all(|entity| entity.identifiers.iter().all(|identifier| {
            !matches!(identifier.value.as_str(), "apnews.com" | "kark.com" | "rollcall.com" | "wikipedia.org" | "en.wikipedia.org")
        })));
        let selected: Vec<_> = entities.iter().filter(|entity| entity.selected).collect();
        let trump = selected.first().expect("the subject is selected");
        assert_eq!(trump.canonical_name, "Donald Trump");
        assert_eq!(trump.entity_type, "person");
        assert_eq!(trump.certainty, "high");
        assert!(trump.identifiers.iter().all(|identifier| identifier.kind != "domain"));
        assert!(trump.identifiers.iter().any(|identifier| {
            identifier.kind == "twitter" && identifier.value.eq_ignore_ascii_case("realDonaldTrump")
        }));
        assert!(!trump.unresolved.iter().any(|item| item.contains("company domain")));
        assert_eq!(
            entities
                .iter()
                .filter(|entity| entity.canonical_name.to_ascii_lowercase().contains("trump"))
                .count(),
            1,
            "subject variants merge into one entity: {entities:?}"
        );
    }

    #[test]
    fn subject_type_follows_the_question_and_the_name() {
        assert_eq!(subject_type(TRUMP, "donald trump", &[]), "person");
        assert_eq!(subject_type("who is jeff bezos?", "jeff bezos", &[]), "person");
        assert_eq!(subject_type("who is Amazon?", "Amazon", &[]), "organization");
        assert_eq!(subject_type("what is Acme Holdings Inc?", "Acme Holdings Inc", &[]), "organization");
        assert_eq!(subject_type("certificates for example.org", "certificates for example.org", &[]), "organization");
        assert!(publisher_host("apnews.com"));
        assert!(publisher_host("kark.com"));
        assert!(publisher_host("rollcall.com"));
        assert!(publisher_host("bbc.co.uk"));
        assert!(!publisher_host("amazon.com"));
    }

    fn trump_input<'a>(
        entities: &'a [SelectedEntity],
        gaps: &'a [Gap],
        enabled: &'a HashSet<String>,
        empty: &'a HashSet<String>,
        credits: &'a HashMap<String, u32>,
        costs: &'a HashMap<String, u32>,
        hits: &'a [SearchHit],
    ) -> SelectionInput<'a> {
        input(TRUMP, DISCOVERY, true, entities, gaps, enabled, empty, empty, credits, costs, hits)
    }

    /// The legacy isolation path ranks by description; the SociaVault search, content,
    /// and Google tools added in #27 are picker-driven, so these tests leave them out.
    fn legacy_enabled() -> HashSet<String> {
        let mut enabled = enabled_all();
        for id in ["sociavault_search", "sociavault_search_users", "sociavault_user_content", "sociavault_google_search"] {
            enabled.remove(id);
        }
        enabled
    }

    #[test]
    fn tool_isolation_runs_the_suggested_tools_with_subject_inputs_and_no_firecrawl() {
        let hits = trump_hits();
        let entities = select_entities(TRUMP, &hits);
        let gaps = gaps_for(TRUMP, DISCOVERY, &entities, None);
        let enabled = legacy_enabled();
        let empty = HashSet::new();
        let credits = HashMap::from([
            ("firecrawl".into(), 20),
            ("hunter".into(), 5),
            ("sociavault".into(), 5),
        ]);
        let costs = HashMap::new();
        let used: HashSet<String> = ["firecrawl_search".to_string(), "firecrawl_scrape".to_string()].into();
        let suggestions = additional_tools(TRUMP, &used, &enabled);
        let ids: Vec<_> = suggestions.iter().map(|tool| tool.tool_id.as_str()).collect();
        assert_eq!(ids, ["sociavault_profile", "keybase_identity", "stackexchange_users"]);
        let selection = trump_input(&entities, &gaps, &enabled, &empty, &credits, &costs, &hits);
        let isolation = isolate_tools(&suggestions, &selection, &HashSet::new(), 3);
        let ran: Vec<_> = isolation.actions.iter().map(|action| action.tool_id.as_str()).collect();
        assert_eq!(ran, ids, "skipped: {:?}", isolation.skipped);
        let social = &isolation.actions[0];
        assert_eq!(social.arguments["platform"], "twitter");
        assert!(social.arguments["handle"].as_str().unwrap().eq_ignore_ascii_case("realDonaldTrump"));
        assert_eq!(isolation.actions[2].arguments, json!({"name": "Donald Trump"}));
        for action in &isolation.actions {
            assert!(!action.tool_id.starts_with("firecrawl_"));
            let text = action.arguments.to_string();
            assert!(!text.contains("apnews") && !text.contains("kark") && !text.contains("rollcall"));
            assert!(osint::validate(&action.tool_id, &action.arguments).is_ok());
            assert!(!action.purpose.is_empty() && !action.evidence_ids.is_empty());
        }
        assert_eq!(isolation.actions[0].credit_cost, 1);
        assert!(isolation.actions[0].spends());
    }

    #[test]
    fn tool_isolation_skips_with_a_reason_when_a_tool_cannot_run() {
        let hits = trump_hits();
        let entities = select_entities(TRUMP, &hits);
        let gaps = gaps_for(TRUMP, DISCOVERY, &entities, None);
        let enabled = legacy_enabled();
        let empty = HashSet::new();
        let credits = HashMap::from([("sociavault".into(), 5), ("firecrawl".into(), 20)]);
        let costs = HashMap::new();
        let mut suggestions = vec![ToolSuggestion {
            tool_id: "firecrawl_search".into(),
            reason: "Search again.".into(),
        }];
        suggestions.extend(additional_tools(TRUMP, &HashSet::new(), &enabled).into_iter().filter(|tool| tool.tool_id != "firecrawl_search"));
        suggestions.push(ToolSuggestion {
            tool_id: "overpass_places".into(),
            reason: "Places near the subject.".into(),
        });
        let selection = trump_input(&entities, &gaps, &enabled, &empty, &credits, &costs, &hits);
        let missing: HashSet<String> = ["sociavault".to_string()].into();
        let isolation = isolate_tools(&suggestions, &selection, &missing, 5);
        assert!(isolation.actions.iter().all(|action| {
            action.tool_id != "firecrawl_search" && action.tool_id != "sociavault_profile" && action.tool_id != "overpass_places"
        }));
        assert!(isolation.skipped.iter().any(|line| line.starts_with("firecrawl_search — skipped")));
        assert!(isolation.skipped.iter().any(|line| {
            line.starts_with("sociavault_profile — skipped") && line.contains("SOCIAVAULT_API_KEY")
        }));
        assert!(isolation.skipped.iter().any(|line| line.starts_with("overpass_places — skipped: no input")));
        let mut capped = trump_input(&entities, &gaps, &enabled, &empty, &credits, &costs, &hits);
        capped.sociavault_cap = 0;
        let isolation = isolate_tools(&suggestions, &capped, &HashSet::new(), 5);
        assert!(isolation.actions.iter().all(|action| action.tool_id != "sociavault_profile"));
        assert!(isolation.skipped.iter().any(|line| line.contains("SociaVault allowance")));
        let lonely: Vec<SelectedEntity> = Vec::new();
        let gaps = gaps_for(TRUMP, DISCOVERY, &lonely, None);
        let bare = trump_input(&lonely, &gaps, &enabled, &empty, &credits, &costs, &[]);
        let isolation = isolate_tools(&suggestions, &bare, &HashSet::new(), 5);
        assert!(isolation.actions.iter().any(|action| {
            action.tool_id == "stackexchange_users" && action.arguments == json!({"name": "Donald Trump"})
        }));
        assert!(isolation.skipped.iter().any(|line| line.starts_with("sociavault_profile — skipped: no input")));
    }


    fn account_hits() -> Vec<SearchHit> {
        let mut hits = trump_hits();
        hits.extend(dedupe_hits(vec![
            hit("e3", "Donald J. Trump (@realDonaldTrump) - Truth Social", "https://truthsocial.com/@realDonaldTrump", "Truth Social profile.", ACCOUNTS),
            hit("e3", "Donald J. Trump (@realdonaldtrump) • Instagram photos and videos", "https://www.instagram.com/realdonaldtrump/", "Instagram profile.", ACCOUNTS),
            hit("e3", "Truth Social", "https://truthsocial.com/", "Truth Social is the platform Donald Trump posts on.", ACCOUNTS),
            hit("e3", "AP reporter on X", "https://x.com/apreporter", "Covers Donald Trump at the White House.", ACCOUNTS),
            hit("e3", "Trump campaign ads on GitHub", "https://apnews.com/article/x", "Profiles: https://github.com/someoneelse and https://keybase.io/realdonaldtrump", ACCOUNTS),
        ]));
        hits
    }

    #[test]
    fn person_and_organization_discovery_searches_for_accounts_second() {
        for question in [TRUMP, "who is jeff bezos?", "who is Amazon?", "tell me about Acme Holdings Inc"] {
            assert!(accounts_flow(question, DISCOVERY), "{question}");
            let queries = complementary_queries(question, DISCOVERY);
            assert_eq!(queries[0].role, "identity");
            assert_eq!(queries[1].role, ACCOUNTS, "{question}");
            assert!(queries[1].query.contains("official social media accounts profiles handles"));
            assert!(accounts_query(&queries[1].query));
            assert!(distinct_queries(&queries[0].query, &queries[1].query));
        }
        assert_eq!(
            complementary_queries(TRUMP, DISCOVERY)[1].query,
            "donald trump official social media accounts profiles handles"
        );
        assert_eq!(target_kind(TRUMP), "person");
        assert_eq!(target_kind("who is Amazon?"), "organization");
    }

    #[test]
    fn other_subjects_keep_the_investigative_question_and_skip_accounts() {
        for question in [
            "what is the ukraine war?",
            "How did Amazon start?",
            "what is inflation?",
            "certificates for example.org",
            "what happened in the 2024 election?",
        ] {
            assert_eq!(target_kind(question), "other", "{question}");
            assert!(!accounts_flow(question, DISCOVERY), "{question}");
            let queries = complementary_queries(question, DISCOVERY);
            assert_eq!(queries[1].role, "investigative", "{question}");
            assert!(!queries[1].query.contains("official social media accounts"));
        }
        let history = complementary_queries("How did Amazon start?", DISCOVERY);
        assert!(history[1].query.contains("history") || history[1].query.contains("timeline"));
        assert!(!accounts_flow(TRUMP, HYPOTHESIS));
        assert!(!accounts_flow(TRUMP, ADAPTIVE));
        assert_eq!(complementary_queries(TRUMP, ADAPTIVE)[1].role, "investigative");
    }

    #[test]
    fn fallback_extractor_keeps_only_the_subject_accounts() {
        let accounts = fallback_accounts(TRUMP, &account_hits());
        let has = |platform: &str, handle: &str| {
            accounts.iter().any(|account| account.platform == platform && account.handle.eq_ignore_ascii_case(handle))
        };
        assert!(has("twitter", "realDonaldTrump"), "{accounts:?}");
        assert!(has("truthsocial", "realDonaldTrump"), "{accounts:?}");
        assert!(has("instagram", "realdonaldtrump"));
        assert!(has("keybase", "realdonaldtrump"));
        assert!(!has("twitter", "apreporter"));
        assert!(!has("github", "someoneelse"));
        assert!(accounts.iter().all(|account| account.sources == ["pattern"]));
    }

    #[test]
    fn model_accounts_are_grounded_owned_and_merged_with_the_fallback() {
        let hits = account_hits();
        let value = json!({
            "accounts": [
                {"platform": "X", "handle": "@realDonaldTrump", "evidence_id": "e2"},
                {"platform": "Truth Social", "handle": "https://truthsocial.com/@realDonaldTrump"},
                {"platform": "twitter", "handle": "trumpfakeaccount"},
                {"platform": "twitter", "handle": "apreporter"},
                {"platform": "myspace", "handle": "realDonaldTrump"},
                {"platform": "facebook", "handle": "DonaldTrump"}
            ],
            "tools": [
                {"tool_id": "firecrawl_search", "reason": "search again"},
                {"tool_id": "keybase_identity", "reason": "Check proofs for the handle."},
                {"tool_id": "not_a_tool", "reason": "x"}
            ]
        });
        let model = accounts_from_model(&value, TRUMP, &hits);
        assert_eq!(model.len(), 2, "{model:?}");
        assert!(model.iter().all(|account| account.handle.eq_ignore_ascii_case("realDonaldTrump")));
        let tools = model_tool_picks(&value, &enabled_all());
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_id, "keybase_identity");
        let merged = merge_accounts(&model, &fallback_accounts(TRUMP, &hits));
        let twitter = merged
            .iter()
            .find(|account| account.platform == "twitter")
            .unwrap();
        assert_eq!(twitter.sources, ["model", "pattern"]);
        assert_eq!(
            merged.iter().filter(|account| account.platform == "twitter").count(),
            1
        );
        assert_eq!(account_line(twitter), "twitter @realDonaldTrump (model, pattern)");
    }

    #[test]
    fn account_platforms_attach_to_the_subject_and_are_never_entities() {
        let hits = account_hits();
        let mut entities = select_entities(TRUMP, &hits);
        let accounts = merge_accounts(&[], &fallback_accounts(TRUMP, &hits));
        attach_accounts(TRUMP, &hits, &mut entities, &accounts);
        for name in ["truthsocial", "truth social", "instagram", "x", "github", "keybase", "apnews"] {
            assert!(entities.iter().all(|entity| !entity.canonical_name.eq_ignore_ascii_case(name)), "{name}: {entities:?}");
        }
        assert!(entities.iter().all(|entity| entity.identifiers.iter().all(|identifier| {
            !matches!(identifier.value.as_str(), "truthsocial.com" | "x.com" | "instagram.com" | "github.com")
        })));
        let trump = entities.iter().find(|entity| entity.canonical_name == "Donald Trump").unwrap();
        assert_eq!(trump.entity_type, "person");
        for platform in ["twitter", "truthsocial", "instagram", "keybase"] {
            assert!(trump.identifiers.iter().any(|identifier| identifier.kind == platform), "{platform}");
        }
        assert!(trump.identifiers.iter().all(|identifier| identifier.kind != "domain"));
    }

    #[test]
    fn extracted_handles_feed_sociavault_keybase_and_wikipedia() {
        let hits = account_hits();
        let mut entities = select_entities(TRUMP, &hits);
        attach_accounts(TRUMP, &hits, &mut entities, &fallback_accounts(TRUMP, &hits));
        let gaps = gaps_for(TRUMP, DISCOVERY, &entities, None);
        let enabled = enabled_all();
        let empty = HashSet::new();
        let credits = HashMap::from([("sociavault".into(), 10), ("firecrawl".into(), 20)]);
        let costs = HashMap::new();
        let suggestions: Vec<ToolSuggestion> = ["sociavault_profile", "keybase_identity", "wikipedia_users", "stackexchange_users"]
            .iter()
            .map(|id| ToolSuggestion { tool_id: (*id).into(), reason: "Suggested.".into() })
            .collect();
        let mut selection = trump_input(&entities, &gaps, &enabled, &empty, &credits, &costs, &hits);
        selection.sociavault_cap = 2;
        let isolation = isolate_tools(&suggestions, &selection, &HashSet::new(), 4);
        let social: Vec<_> = isolation
            .actions
            .iter()
            .filter(|action| action.tool_id == "sociavault_profile")
            .map(|action| action.arguments.clone())
            .collect();
        assert_eq!(social.len(), 2, "{:?}", isolation);
        assert_eq!(social[0]["platform"], "twitter");
        assert!(social[0]["handle"].as_str().unwrap().eq_ignore_ascii_case("realDonaldTrump"));
        assert_eq!(social[1]["platform"], "instagram");
        let args = |id: &str| isolation.actions.iter().find(|action| action.tool_id == id).map(|action| action.arguments.clone());
        assert_eq!(args("keybase_identity"), Some(json!({"username": "realdonaldtrump"})));
        assert_eq!(args("wikipedia_users").unwrap()["username"].as_str().unwrap().to_ascii_lowercase(), "realdonaldtrump");
        assert_eq!(args("stackexchange_users"), Some(json!({"name": "Donald Trump"})));
        selection.sociavault_cap = 1;
        let capped = isolate_tools(&suggestions, &selection, &HashSet::new(), 4);
        assert_eq!(capped.actions.iter().filter(|action| action.tool_id == "sociavault_profile").count(), 1);
        assert!(capped.skipped.iter().any(|line| {
            line.starts_with("sociavault_profile") && line.contains("instagram") && line.contains("allowance")
        }), "{:?}", capped.skipped);
    }


    fn binding(kind: &str, value: &str, evidence: &str) -> Binding {
        Binding { kind: kind.into(), value: value.into(), evidence_id: evidence.into(), ..Default::default() }
    }

    #[test]
    fn binder_maps_kinds_to_inputs_and_never_hands_hunter_a_social_host() {
        let social = vec![binding("domain", "x.com", "call-1")];
        let (_, _, missing) = bind_arguments("hunter_domain_search", &social, "who is jane example?", None);
        assert_eq!(missing, vec!["domain or company".to_string()]);
        // Hunter takes only prompt or primary-provider bindings (D1).
        let primary = |kind: &str, value: &str, evidence: &str| Binding { source_tool: "firecrawl_search".into(), ..binding(kind, value, evidence) };
        let mixed = vec![primary("domain", "nytimes.com", "call-1"), primary("domain", "example.org", "call-2")];
        let (args, filled, missing) = bind_arguments("hunter_domain_search", &mixed, "who is jane?", None);
        assert!(missing.is_empty());
        assert_eq!(args, json!({"domain": "example.org"}));
        assert_eq!(filled, vec!["domain=example.org (domain from call-2 via firecrawl_search)".to_string()]);
        let handle = Binding { qualifier: "twitter".into(), ..binding("handle", "janeexample", "call-3") };
        let (args, _, missing) = bind_arguments("sociavault_profile", std::slice::from_ref(&handle), "who is jane?", None);
        assert!(missing.is_empty());
        assert_eq!(args, json!({"platform": "twitter", "handle": "janeexample"}));
        assert!(osint::validate("sociavault_profile", &args).is_ok());
        let (args, _, _) = bind_arguments("keybase_identity", &[handle], "who is jane?", None);
        assert_eq!(args, json!({"username": "janeexample"}));
        // A search query is the directive entity plus its fixed qualifier, never question text.
        let directives = fallback_directives("who is jane example?", &[]);
        let (args, filled, _) = bind_arguments("firecrawl_search", &[], "who is jane example?", Some(&directives[1]));
        assert_eq!(args["query"], json!("Jane Example official account"));
        assert_eq!(filled, vec!["query=Jane Example official account (d2 entity + qualifier)".to_string()]);
        assert_eq!(unmet_needs("sociavault_profile", &[]), vec!["handle or platform_id".to_string()]);
        assert!(unmet_needs("crtsh_certificates", &mixed).is_empty());
    }

    #[test]
    fn question_bindings_and_dependency_fix() {
        let found = question_bindings("check ada@example.org and 8.8.8.8 for CVE-2021-44228 on example.org");
        let kinds: Vec<&str> = found.iter().map(|binding| binding.kind.as_str()).collect();
        for kind in ["email", "ip", "cve", "domain"] {
            assert!(kinds.contains(&kind), "{kind} missing from {kinds:?}");
        }
        assert!(found.iter().all(|binding| binding.evidence_id == "question"));
        let picked: Vec<String> = ["hunter_email_verifier", "keybase_identity", "hunter_email_finder", "sociavault_profile", "firecrawl_search"]
            .iter()
            .map(|id| id.to_string())
            .collect();
        let order = dependency_order(&picked, &[], &HashMap::new());
        let at = |id: &str| order.iter().position(|tool| tool == id).unwrap();
        assert!(at("hunter_email_finder") < at("hunter_email_verifier"));
        assert!(at("firecrawl_search") < at("sociavault_profile"));
        assert!(at("firecrawl_search") < at("keybase_identity"));
        assert!(at("firecrawl_search") < at("hunter_email_finder"));
        assert_eq!(order.len(), picked.len());
        // Known inputs need no producer, so the pick order stays.
        let known = vec![binding("email", "ada@example.org", "question")];
        let kept = dependency_order(&["hunter_email_verifier".to_string(), "hunter_email_finder".to_string()], &known, &HashMap::new());
        assert_eq!(kept[0], "hunter_email_verifier");
        let deps = depends_on(&order, at("sociavault_profile"), &[], &HashMap::new(), &HashMap::new());
        assert!(deps.contains(&at("firecrawl_search")));
        assert_eq!(fallback_directives("who is jane example?", &[]).len(), 3);
        // Geocoder coordinates make Overpass reachable.
        assert!(pickable("overpass_places"));
        assert!(dependencies().iter().all(|row| osint::definition(row.tool).is_some()));
    }
}
