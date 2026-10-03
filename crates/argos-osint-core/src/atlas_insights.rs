//! Final Atlas step: gate a cycle's fresh articles, extract a handful of claims,
//! and store them where Brain recall already looks.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::atlas::{category_tag, OriginStat};
use crate::provider::{self, ChatMessage};
use crate::secrets::ProviderSecret;
use crate::store::{
    insight_fingerprint, AtlasArticleRow, AtlasInsightClaim, AtlasStoredClaim, Store,
};

/// Articles sent in one extraction call.
pub const PACKET_LIMIT: usize = 12;
/// Lead claims kept from the classified packet.
pub const CLAIM_LIMIT: usize = 5;
/// Context claims kept from unk titles that name an extracted entity.
pub const CONTEXT_LIMIT: usize = 5;
const DESCRIPTION_CHARS: usize = 500;

const NAMESPACES: &[&str] = &["person", "org", "place", "agreement"];

const LEAD_PROMPT: &str = "\
Extract at most 5 concise atomic news claims. Return a JSON object {\"claims\":[{\"entity\":string,\"namespace\":string,\"predicate\":string,\"object\":string,\"topic\":string,\"claim\":string,\"classification\":\"fact\"|\"inference\",\"confidence\":number,\"evidence_ids\":[string]}]}. \
entity and object must be verbatim spans copied from the cited article title or description. Do not invent names. \
namespace is person, org, place, or agreement. \
predicate is the verb the article supports, such as sanctioned, deployed, or met. \
topic is the article category. \
evidence_ids must be ids from the packet. \
The claim is one sentence that contains the entity and the object. \
Set classification to fact only when both spans are in the title; otherwise inference. \
country is the publisher country, not the entity.";

const CONTEXT_PROMPT: &str = "\
Extract at most 5 context claims from unclassified articles. Return the same JSON object as a lead extraction. \
Use an entity from the Entities list, and only when that entity is a verbatim span of the article title. \
object must be a verbatim span of the same article. \
Set classification to inference. \
Do not extract an article whose title does not contain one of the entities. \
country is the publisher country, not the entity.";

/// Counts and claim rows shown under the country table.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct InsightStats {
    pub gated: u32,
    pub claims: u32,
    pub facts: u32,
    pub inferences: u32,
    pub context: u32,
    pub dropped: u32,
    pub conflict_or_revision: u32,
    pub cross_topic: u32,
    pub cross_country: u32,
    pub context_for: u32,
    pub co_mentioned: u32,
    pub rows: Vec<InsightRow>,
}

/// One kept claim, in the order the insights table renders it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InsightRow {
    pub entity: String,
    pub predicate: String,
    pub object: String,
    pub topic: String,
    pub classification: String,
}

/// Model output before the span gate.
#[derive(Clone, Debug, PartialEq)]
pub struct RawClaim {
    pub entity: String,
    pub namespace: String,
    pub predicate: String,
    pub object: String,
    pub topic: String,
    pub claim: String,
    pub classification: String,
    pub confidence: f64,
    pub evidence_ids: Vec<String>,
}

/// A claim that survived the span gate.
#[derive(Clone, Debug, PartialEq)]
pub struct KeptClaim {
    pub entity: String,
    pub namespace: String,
    pub predicate: String,
    pub object: String,
    pub topic: String,
    pub claim: String,
    pub classification: String,
    pub confidence: f64,
    pub article_id: String,
    pub source_url: String,
    /// Publisher country of the cited article. Never written onto the entity.
    pub country: String,
    pub context: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcceptMode {
    Lead,
    Context,
}

/// Lead extraction plus an optional context-pass failure that did not drop the lead claims.
pub struct Extraction {
    pub settled: Settled,
    pub context_error: Option<String>,
}

pub struct Settled {
    pub stats: InsightStats,
    pub claims: Vec<AtlasInsightClaim>,
    pub relations: Vec<(String, String, String)>,
    pub brief: String,
    pub entity_path: String,
}

/// Category is the only significance filter. Every non-unk article is kept, at every tier.
pub fn is_significant(category: &str) -> bool {
    category_tag(category) != "unk"
}

/// Significant articles ordered by tier, then temperature. Domain rank and provider count are ignored.
pub fn select_significant(
    articles: &[AtlasArticleRow],
    origins: &[OriginStat],
) -> Vec<AtlasArticleRow> {
    let mut kept: Vec<AtlasArticleRow> = articles
        .iter()
        .filter(|article| is_significant(&article.category))
        .cloned()
        .collect();
    kept.sort_by(|left, right| {
        tier_key(&left.country, origins)
            .cmp(&tier_key(&right.country, origins))
            .then_with(|| right.temperature.total_cmp(&left.temperature))
            .then_with(|| left.id.cmp(&right.id))
    });
    kept
}

/// The first [`PACKET_LIMIT`] significant articles. `gated` is the full significant count.
pub fn insight_packet(
    articles: &[AtlasArticleRow],
    origins: &[OriginStat],
) -> (Vec<AtlasArticleRow>, u32) {
    let significant = select_significant(articles, origins);
    let gated = significant.len() as u32;
    let packet = significant.into_iter().take(PACKET_LIMIT).collect();
    (packet, gated)
}

/// Unk titles in the same publisher country that already contain an extracted entity.
pub fn context_candidates(
    articles: &[AtlasArticleRow],
    claims: &[KeptClaim],
) -> Vec<AtlasArticleRow> {
    let mut matched = Vec::new();
    for article in articles {
        if is_significant(&article.category) {
            continue;
        }
        let hit = claims.iter().any(|claim| {
            !claim.context
                && countries_equal(&article.country, &claim.country)
                && contains_span(&article.title, &claim.entity)
        });
        if hit {
            matched.push(article.clone());
        }
    }
    matched
}

pub fn accept_claims(
    evidence: &[AtlasArticleRow],
    raw: &[RawClaim],
    mode: AcceptMode,
) -> (Vec<KeptClaim>, u32) {
    let mut kept = Vec::new();
    let mut dropped = 0u32;
    for claim in raw {
        match accept_one(evidence, claim, mode) {
            Some(claim) => kept.push(claim),
            None => dropped += 1,
        }
    }
    (kept, dropped)
}

/// Replace a shorter entity with a longer verbatim form when its tokens are a subset.
/// A country name or code is never the shorter or the longer form.
pub fn merge_aliases(claims: &mut [KeptClaim], articles: &[AtlasArticleRow]) {
    let entities: Vec<String> = claims.iter().map(|claim| claim.entity.clone()).collect();
    for claim in claims {
        claim.entity = longer_alias(&claim.entity, &entities, articles);
    }
}

pub fn dedupe_claims(claims: Vec<KeptClaim>) -> Vec<KeptClaim> {
    let mut kept: Vec<KeptClaim> = Vec::new();
    for claim in claims {
        if let Some(existing) = kept
            .iter_mut()
            .find(|item| fingerprint(item) == fingerprint(&claim))
        {
            if prefer_claim(&claim, existing) {
                let article_id = existing.article_id.clone();
                let source_url = existing.source_url.clone();
                let country = existing.country.clone();
                *existing = claim;
                if existing.article_id.is_empty() {
                    existing.article_id = article_id;
                    existing.source_url = source_url;
                    existing.country = country;
                }
            }
        } else {
            kept.push(claim);
        }
    }
    kept
}

pub fn cap_claims(mut claims: Vec<KeptClaim>, limit: usize) -> Vec<KeptClaim> {
    claims.sort_by(|left, right| right.confidence.total_cmp(&left.confidence));
    claims.truncate(limit);
    claims
}

pub fn link_claims(claims: &[KeptClaim]) -> Vec<(String, String, String)> {
    let mut relations = Vec::new();
    for left_index in 0..claims.len() {
        for right_index in (left_index + 1)..claims.len() {
            let left = &claims[left_index];
            let right = &claims[right_index];
            let left_key = fingerprint(left);
            let right_key = fingerprint(right);
            if left_key == right_key {
                continue;
            }
            if left.entity == right.entity
                && left.predicate == right.predicate
                && left.object != right.object
            {
                relations.push((
                    left_key.clone(),
                    right_key.clone(),
                    "conflict_or_revision".into(),
                ));
            }
            if left.entity == right.entity
                && !entity_is_country_alone(&left.entity)
                && topics_differ(&left.topic, &right.topic)
            {
                relations.push((left_key.clone(), right_key.clone(), "cross_topic".into()));
            }
            if left.entity == right.entity && countries_differ(&left.country, &right.country) {
                relations.push((left_key.clone(), right_key.clone(), "cross_country".into()));
            }
            if !left.article_id.is_empty()
                && left.article_id == right.article_id
                && left.entity != right.entity
            {
                relations.push((left_key.clone(), right_key.clone(), "co_mentioned".into()));
            }
            if left.context != right.context && left.entity == right.entity {
                let (context, lead) = if left.context {
                    (left, right)
                } else {
                    (right, left)
                };
                relations.push((
                    fingerprint(context),
                    fingerprint(lead),
                    "context_for".into(),
                ));
            }
        }
    }
    relations
}

/// Span-gate context claims, merge aliases against the lead set, cap both, and link.
pub fn finish_insights(
    gated: u32,
    lead_dropped: u32,
    packet: &[AtlasArticleRow],
    lead: Vec<KeptClaim>,
    context_articles: &[AtlasArticleRow],
    context_raw: &[RawClaim],
) -> Settled {
    let (context, context_dropped) =
        accept_claims(context_articles, context_raw, AcceptMode::Context);
    let mut articles = packet.to_vec();
    articles.extend(context_articles.iter().cloned());
    let mut all = lead;
    all.extend(context);
    merge_aliases(&mut all, &articles);
    let mut lead: Vec<KeptClaim> = all.iter().filter(|claim| !claim.context).cloned().collect();
    let mut context: Vec<KeptClaim> = all.into_iter().filter(|claim| claim.context).collect();
    lead = cap_claims(dedupe_claims(lead), CLAIM_LIMIT);
    let before_match = context.len();
    context.retain(|claim| lead.iter().any(|lead| lead.entity == claim.entity));
    let unmatched = before_match.saturating_sub(context.len()) as u32;
    context.retain(|claim| {
        lead.iter()
            .all(|lead| fingerprint(lead) != fingerprint(claim))
    });
    context = cap_claims(dedupe_claims(context), CONTEXT_LIMIT);

    let mut claims = lead;
    claims.extend(context);
    let relations = link_claims(&claims);
    let lead_claims: Vec<&KeptClaim> = claims.iter().filter(|claim| !claim.context).collect();
    let context_claims: Vec<&KeptClaim> = claims.iter().filter(|claim| claim.context).collect();
    let mut stats = InsightStats {
        gated,
        claims: lead_claims.len() as u32,
        facts: lead_claims
            .iter()
            .filter(|claim| claim.classification == "fact")
            .count() as u32,
        inferences: lead_claims
            .iter()
            .filter(|claim| claim.classification == "inference")
            .count() as u32,
        context: context_claims.len() as u32,
        dropped: lead_dropped + context_dropped + unmatched,
        rows: claims.iter().map(insight_row).collect(),
        ..InsightStats::default()
    };
    for (_, _, relation) in &relations {
        match relation.as_str() {
            "conflict_or_revision" => stats.conflict_or_revision += 1,
            "cross_topic" => stats.cross_topic += 1,
            "cross_country" => stats.cross_country += 1,
            "context_for" => stats.context_for += 1,
            "co_mentioned" => stats.co_mentioned += 1,
            _ => {}
        }
    }
    let brief = brief_text(&lead_claims);
    let entity_path = entity_path(&claims);
    let stored = claims.iter().map(stored_claim).collect();
    Settled {
        stats,
        claims: stored,
        relations,
        brief,
        entity_path,
    }
}

/// Rebuild the table from claims already stored for this run.
pub fn stats_from_stored(
    gated: u32,
    claims: &[AtlasStoredClaim],
    relations: &[(String, String, String)],
) -> InsightStats {
    let context_ids: Vec<&str> = relations
        .iter()
        .filter(|(_, _, relation)| relation == "context_for")
        .map(|(left, _, _)| left.as_str())
        .collect();
    let lead: Vec<&AtlasStoredClaim> = claims
        .iter()
        .filter(|claim| !context_ids.contains(&claim.fingerprint.as_str()))
        .collect();
    let context_count = claims
        .iter()
        .filter(|claim| context_ids.contains(&claim.fingerprint.as_str()))
        .count() as u32;
    let mut stats = InsightStats {
        gated,
        claims: lead.len() as u32,
        facts: lead
            .iter()
            .filter(|claim| claim.classification == "fact")
            .count() as u32,
        inferences: lead
            .iter()
            .filter(|claim| claim.classification == "inference")
            .count() as u32,
        context: context_count,
        rows: claims
            .iter()
            .map(|claim| InsightRow {
                entity: claim.entity.clone(),
                predicate: claim.predicate.clone(),
                object: claim.object.clone(),
                topic: claim.topic.clone(),
                classification: claim.classification.clone(),
            })
            .collect(),
        ..InsightStats::default()
    };
    for (_, _, relation) in relations {
        match relation.as_str() {
            "conflict_or_revision" => stats.conflict_or_revision += 1,
            "cross_topic" => stats.cross_topic += 1,
            "cross_country" => stats.cross_country += 1,
            "context_for" => stats.context_for += 1,
            "co_mentioned" => stats.co_mentioned += 1,
            _ => {}
        }
    }
    stats
}

pub fn insight_table_lines(stats: &InsightStats) -> Vec<String> {
    if stats.gated == 0
        && stats.claims == 0
        && stats.context == 0
        && stats.dropped == 0
        && stats.rows.is_empty()
    {
        return vec!["No insights extracted for this cycle.".into()];
    }
    let links = stats.conflict_or_revision
        + stats.cross_topic
        + stats.cross_country
        + stats.context_for
        + stats.co_mentioned;
    let mut lines = vec![
        format!(
            "Gated {}  Claims {}  Fact {}  Inference {}  Context {}  Dropped {}  Links {}",
            stats.gated,
            stats.claims,
            stats.facts,
            stats.inferences,
            stats.context,
            stats.dropped,
            links
        ),
        "Entity                    Predicate        Object                  Topic            Class"
            .into(),
    ];
    if stats.rows.is_empty() {
        lines.push("No insights extracted for this cycle.".into());
        return lines;
    }
    for row in &stats.rows {
        lines.push(format!(
            "{:<24} {:<16} {:<23} {:<16} {}",
            row.entity, row.predicate, row.object, row.topic, row.classification
        ));
    }
    lines
}

/// One lead call over the packet, then one context call over matching unk titles.
pub async fn extract(
    secret: &ProviderSecret,
    articles: &[AtlasArticleRow],
    origins: &[OriginStat],
) -> Result<Extraction> {
    let (packet, gated) = insight_packet(articles, origins);
    if packet.is_empty() {
        return Ok(Extraction {
            settled: finish_insights(gated, 0, &packet, Vec::new(), &[], &[]),
            context_error: None,
        });
    }
    let lead_raw = ask_claims(secret, LEAD_PROMPT, &packet_json(&packet)?).await?;
    let (mut lead, lead_dropped) = accept_claims(&packet, &lead_raw, AcceptMode::Lead);
    merge_aliases(&mut lead, &packet);
    let lead = cap_claims(dedupe_claims(lead), CLAIM_LIMIT);
    if lead.is_empty() {
        return Ok(Extraction {
            settled: finish_insights(gated, lead_dropped, &packet, lead, &[], &[]),
            context_error: None,
        });
    }
    let mut context_articles = context_candidates(articles, &lead);
    context_articles.truncate(PACKET_LIMIT);
    if context_articles.is_empty() {
        return Ok(Extraction {
            settled: finish_insights(gated, lead_dropped, &packet, lead, &[], &[]),
            context_error: None,
        });
    }
    let entities: Vec<String> = lead.iter().map(|claim| claim.entity.clone()).collect();
    let user = format!(
        "Entities:\n{}\nArticles:\n{}",
        entities.join("\n"),
        packet_json(&context_articles)?
    );
    match ask_claims(secret, CONTEXT_PROMPT, &user).await {
        Ok(context_raw) => Ok(Extraction {
            settled: finish_insights(
                gated,
                lead_dropped,
                &packet,
                lead,
                &context_articles,
                &context_raw,
            ),
            context_error: None,
        }),
        Err(err) => Ok(Extraction {
            settled: finish_insights(gated, lead_dropped, &packet, lead, &[], &[]),
            context_error: Some(err.to_string()),
        }),
    }
}

/// Skip the model when this run already has sources, and rebuild the table if the save was interrupted.
pub fn resume_stats(
    store: &Store,
    run_id: &str,
    articles: &[AtlasArticleRow],
    origins: &[OriginStat],
    current: &InsightStats,
) -> Result<Option<InsightStats>> {
    if !store.atlas_has_insights(run_id)? {
        return Ok(None);
    }
    if current.claims > 0 || !current.rows.is_empty() {
        return Ok(Some(current.clone()));
    }
    let stored = store.atlas_stored_claims(run_id)?;
    let fingerprints: Vec<String> = stored
        .iter()
        .map(|claim| claim.fingerprint.clone())
        .collect();
    let relations = store.insight_relations_among(&fingerprints)?;
    let (_, gated) = insight_packet(articles, origins);
    Ok(Some(stats_from_stored(gated, &stored, &relations)))
}

fn tier_key(country: &str, origins: &[OriginStat]) -> u8 {
    let tier = origins
        .iter()
        .find(|row| row.country.eq_ignore_ascii_case(country))
        .map(|row| row.tier)
        .unwrap_or(0);
    if tier == 0 {
        u8::MAX
    } else {
        tier
    }
}

fn accept_one(evidence: &[AtlasArticleRow], raw: &RawClaim, mode: AcceptMode) -> Option<KeptClaim> {
    let entity = raw.entity.trim();
    let object = raw.object.trim();
    let predicate = raw.predicate.trim().to_ascii_lowercase();
    let namespace = raw.namespace.trim().to_ascii_lowercase();
    let sentence = raw.claim.trim();
    if entity.is_empty()
        || object.is_empty()
        || predicate.is_empty()
        || sentence.is_empty()
        || !NAMESPACES.contains(&namespace.as_str())
        || !contains_span(sentence, entity)
        || !contains_span(sentence, object)
    {
        return None;
    }
    let cited: Vec<&AtlasArticleRow> = raw
        .evidence_ids
        .iter()
        .filter_map(|id| evidence.iter().find(|article| &article.id == id))
        .filter(|article| spans_article(article, entity, object))
        .collect();
    let cited: Vec<&AtlasArticleRow> = if mode == AcceptMode::Context {
        cited
            .into_iter()
            .filter(|article| contains_span(&article.title, entity))
            .collect()
    } else {
        cited
    };
    let article = cited.first().copied()?;
    let fact = mode == AcceptMode::Lead
        && cited.iter().any(|article| {
            contains_span(&article.title, entity) && contains_span(&article.title, object)
        });
    let topic = if raw.topic.trim().is_empty() {
        article.category.clone()
    } else {
        raw.topic.trim().to_string()
    };
    Some(KeptClaim {
        entity: entity.to_ascii_lowercase(),
        namespace,
        predicate,
        object: object.to_ascii_lowercase(),
        topic,
        claim: sentence.to_string(),
        classification: if fact { "fact" } else { "inference" }.into(),
        confidence: raw.confidence.clamp(0.0, 1.0),
        article_id: article.id.clone(),
        source_url: article.url.clone(),
        country: article.country.clone(),
        context: mode == AcceptMode::Context,
    })
}

fn spans_article(article: &AtlasArticleRow, entity: &str, object: &str) -> bool {
    let blob = format!("{} {}", article.title, article.description);
    contains_span(&blob, entity) && contains_span(&blob, object)
}

fn contains_span(haystack: &str, needle: &str) -> bool {
    let needle = needle.trim();
    if needle.chars().count() < 2 {
        return false;
    }
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

pub fn entity_is_country_alone(entity: &str) -> bool {
    let entity = entity.trim();
    if entity.is_empty() {
        return false;
    }
    if entity.len() == 2 && crate::iso3166::name(entity).is_some() {
        return true;
    }
    crate::iso3166::code_for_name(entity).is_some()
}

fn longer_alias(entity: &str, entities: &[String], articles: &[AtlasArticleRow]) -> String {
    if entity_is_country_alone(entity) {
        return entity.to_string();
    }
    let mine = tokens(entity);
    if mine.is_empty() {
        return entity.to_string();
    }
    let mut best = entity.to_string();
    let mut best_len = mine.len();
    for other in entities {
        if other == entity || entity_is_country_alone(other) {
            continue;
        }
        let theirs = tokens(other);
        if theirs.len() <= best_len {
            continue;
        }
        if !mine
            .iter()
            .all(|token| theirs.iter().any(|item| item == token))
        {
            continue;
        }
        if !articles_have_span(articles, other) {
            continue;
        }
        best = other.clone();
        best_len = theirs.len();
    }
    best
}

fn tokens(text: &str) -> Vec<String> {
    text.to_ascii_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect()
}

fn articles_have_span(articles: &[AtlasArticleRow], span: &str) -> bool {
    articles.iter().any(|article| {
        contains_span(&article.title, span) || contains_span(&article.description, span)
    })
}

fn prefer_claim(incoming: &KeptClaim, existing: &KeptClaim) -> bool {
    let incoming_fact = incoming.classification == "fact";
    let existing_fact = existing.classification == "fact";
    if incoming_fact != existing_fact {
        return incoming_fact;
    }
    incoming.confidence >= existing.confidence
}

fn topics_differ(left: &str, right: &str) -> bool {
    let left = left.trim();
    let right = right.trim();
    !left.is_empty() && !right.is_empty() && !left.eq_ignore_ascii_case(right)
}

fn countries_differ(left: &str, right: &str) -> bool {
    let left = left.trim();
    let right = right.trim();
    !left.is_empty() && !right.is_empty() && !left.eq_ignore_ascii_case(right)
}

fn countries_equal(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

fn fingerprint(claim: &KeptClaim) -> String {
    insight_fingerprint(
        &claim.namespace,
        &claim.entity,
        &claim.predicate,
        &claim.object,
    )
}

fn insight_row(claim: &KeptClaim) -> InsightRow {
    InsightRow {
        entity: claim.entity.clone(),
        predicate: claim.predicate.clone(),
        object: claim.object.clone(),
        topic: claim.topic.clone(),
        classification: claim.classification.clone(),
    }
}

fn stored_claim(claim: &KeptClaim) -> AtlasInsightClaim {
    AtlasInsightClaim {
        fingerprint: fingerprint(claim),
        entity: claim.entity.clone(),
        namespace: claim.namespace.clone(),
        predicate: claim.predicate.clone(),
        object: claim.object.clone(),
        topic: claim.topic.clone(),
        claim: claim.claim.clone(),
        classification: claim.classification.clone(),
        confidence: claim.confidence,
        article_id: claim.article_id.clone(),
        source_url: claim.source_url.clone(),
    }
}

fn brief_text(claims: &[&KeptClaim]) -> String {
    if claims.is_empty() {
        return String::new();
    }
    let lead = claims
        .iter()
        .take(2)
        .map(|claim| claim.claim.trim().trim_end_matches('.'))
        .collect::<Vec<_>>()
        .join(". ");
    let mut lines = vec![format!("{lead}.")];
    for claim in claims {
        lines.push(format!("- {} ({})", claim.claim.trim(), claim.article_id));
    }
    lines.join("\n")
}

fn entity_path(claims: &[KeptClaim]) -> String {
    claims
        .iter()
        .map(|claim| format!("{} → {} → {}", claim.entity, claim.predicate, claim.object))
        .collect::<Vec<_>>()
        .join("; ")
}

fn packet_json(articles: &[AtlasArticleRow]) -> Result<String> {
    let rows: Vec<Value> = articles
        .iter()
        .map(|article| {
            serde_json::json!({
                "id": article.id,
                "title": article.title,
                "description": clip_chars(&article.description, DESCRIPTION_CHARS),
                "category": article.category,
                "country": article.country,
            })
        })
        .collect();
    Ok(serde_json::to_string(&rows)?)
}

fn clip_chars(text: &str, limit: usize) -> String {
    text.trim().chars().take(limit).collect()
}

async fn ask_claims(secret: &ProviderSecret, system: &str, user: &str) -> Result<Vec<RawClaim>> {
    let messages = [
        ChatMessage {
            role: "system".into(),
            content: system.into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
        ChatMessage {
            role: "user".into(),
            content: user.into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
    ];
    let done = provider::complete(secret, &messages, &[], |_| {}).await?;
    parse_claims(&done.content)
}

fn parse_claims(text: &str) -> Result<Vec<RawClaim>> {
    let value = parse_json_value(text)?;
    let claims = value
        .get("claims")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("claims array missing"))?;
    Ok(claims.iter().map(raw_claim).collect())
}

fn parse_json_value(text: &str) -> Result<Value> {
    let trimmed = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Ok(value);
    }
    let start = trimmed
        .find('{')
        .ok_or_else(|| anyhow!("claims json missing"))?;
    let end = trimmed
        .rfind('}')
        .ok_or_else(|| anyhow!("claims json missing"))?;
    if end < start {
        return Err(anyhow!("claims json missing"));
    }
    Ok(serde_json::from_str(&trimmed[start..=end])?)
}

fn raw_claim(value: &Value) -> RawClaim {
    RawClaim {
        entity: value
            .get("entity")
            .and_then(Value::as_str)
            .unwrap_or("")
            .into(),
        namespace: value
            .get("namespace")
            .and_then(Value::as_str)
            .unwrap_or("")
            .into(),
        predicate: value
            .get("predicate")
            .and_then(Value::as_str)
            .unwrap_or("")
            .into(),
        object: value
            .get("object")
            .and_then(Value::as_str)
            .unwrap_or("")
            .into(),
        topic: value
            .get("topic")
            .and_then(Value::as_str)
            .unwrap_or("")
            .into(),
        claim: value
            .get("claim")
            .and_then(Value::as_str)
            .unwrap_or("")
            .into(),
        classification: value
            .get("classification")
            .and_then(Value::as_str)
            .unwrap_or("")
            .into(),
        confidence: value
            .get("confidence")
            .and_then(Value::as_f64)
            .unwrap_or(0.5),
        evidence_ids: value
            .get("evidence_ids")
            .and_then(Value::as_array)
            .map(|ids| {
                ids.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atlas::RunStats;
    use crate::brain::MemorySource;

    fn article(
        id: &str,
        category: &str,
        country: &str,
        title: &str,
        description: &str,
        temperature: f64,
    ) -> AtlasArticleRow {
        AtlasArticleRow {
            run_id: "run".into(),
            id: id.into(),
            title: title.into(),
            description: description.into(),
            url: format!("https://example.com/{id}"),
            country: country.into(),
            source_name: "Desk".into(),
            source_domain: "not-a-wire.example".into(),
            published_at: String::new(),
            provider: "currents".into(),
            temperature,
            category: category.into(),
            seen_at: String::new(),
            author: String::new(),
            image_url: String::new(),
        }
    }

    fn origin(country: &str, tier: u8, temperature: f64) -> OriginStat {
        OriginStat {
            country: country.into(),
            tier,
            temperature,
            volume: 1,
            articles: 1,
        }
    }

    fn raw(
        entity: &str,
        namespace: &str,
        predicate: &str,
        object: &str,
        topic: &str,
        claim: &str,
        classification: &str,
        confidence: f64,
        evidence: &str,
    ) -> RawClaim {
        RawClaim {
            entity: entity.into(),
            namespace: namespace.into(),
            predicate: predicate.into(),
            object: object.into(),
            topic: topic.into(),
            claim: claim.into(),
            classification: classification.into(),
            confidence,
            evidence_ids: vec![evidence.into()],
        }
    }

    fn kept(
        entity: &str,
        predicate: &str,
        object: &str,
        topic: &str,
        article_id: &str,
        country: &str,
        context: bool,
    ) -> KeptClaim {
        KeptClaim {
            entity: entity.into(),
            namespace: "person".into(),
            predicate: predicate.into(),
            object: object.into(),
            topic: topic.into(),
            claim: format!("{entity} {predicate} {object}"),
            classification: "inference".into(),
            confidence: 0.5,
            article_id: article_id.into(),
            source_url: format!("https://example.com/{article_id}"),
            country: country.into(),
            context,
        }
    }

    #[test]
    fn significance_keeps_every_tier_and_ignores_domain_and_provider() {
        let origins = vec![origin("us", 1, 1.0), origin("de", 3, 0.4)];
        let articles = vec![
            article("hot", "military", "us", "One", "", 1.0),
            article("cold", "economic", "de", "Two", "", 0.2),
            article("wire", "unk", "us", "Three", "", 1.0),
        ];
        let selected = select_significant(&articles, &origins);
        assert_eq!(
            selected
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            vec!["hot", "cold"]
        );
        assert!(selected
            .iter()
            .all(|row| row.source_domain == "not-a-wire.example"));
        assert!(selected.iter().all(|row| row.provider == "currents"));
        assert!(selected.iter().all(|row| row.category != "unk"));
    }

    #[test]
    fn the_packet_keeps_twelve_and_reports_the_full_gate() {
        let origins = vec![origin("us", 1, 1.0)];
        let articles: Vec<_> = (0..13)
            .map(|index| {
                article(
                    &format!("a{index:02}"),
                    "military",
                    "us",
                    "Title",
                    "",
                    f64::from(13 - index),
                )
            })
            .collect();
        let (packet, gated) = insight_packet(&articles, &origins);
        assert_eq!(gated, 13);
        assert_eq!(packet.len(), 12);
        assert_eq!(packet[0].id, "a00");
        assert!(packet.iter().all(|row| row.id != "a12"));
    }

    #[test]
    fn a_fact_requires_both_spans_in_the_title() {
        let rows = vec![article(
            "art-1",
            "stability",
            "fr",
            "Cabinet met the union",
            "The cabinet met the union in Paris",
            1.0,
        )];
        let (kept, dropped) = accept_claims(
            &rows,
            &[raw(
                "Cabinet",
                "org",
                "met",
                "union",
                "stability",
                "Cabinet met the union.",
                "inference",
                0.8,
                "art-1",
            )],
            AcceptMode::Lead,
        );
        assert_eq!(dropped, 0);
        assert_eq!(kept[0].classification, "fact");
        assert_eq!(kept[0].entity, "cabinet");
        assert_eq!(kept[0].country, "fr");
        assert_ne!(kept[0].entity, "fr");
        assert_ne!(kept[0].entity, "france");
    }

    #[test]
    fn a_description_only_span_is_inference_and_an_added_name_is_dropped() {
        let rows = vec![article(
            "art-1",
            "stability",
            "fr",
            "Cabinet meets",
            "The cabinet met the union in Paris",
            1.0,
        )];
        let (kept, dropped) = accept_claims(
            &rows,
            &[
                raw(
                    "Cabinet",
                    "org",
                    "met",
                    "union",
                    "stability",
                    "Cabinet met the union.",
                    "fact",
                    0.9,
                    "art-1",
                ),
                raw(
                    "NATO",
                    "org",
                    "met",
                    "union",
                    "military",
                    "NATO met the union.",
                    "fact",
                    0.9,
                    "art-1",
                ),
                raw(
                    "Cabinet",
                    "ip",
                    "met",
                    "union",
                    "stability",
                    "Cabinet met the union.",
                    "fact",
                    0.4,
                    "missing",
                ),
            ],
            AcceptMode::Lead,
        );
        assert_eq!(dropped, 2);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].classification, "inference");
        assert_eq!(kept[0].entity, "cabinet");
    }

    #[test]
    fn five_claims_is_the_cap_and_extra_spans_are_not_counted_as_dropped() {
        let claims: Vec<_> = (0..6)
            .map(|index| {
                let mut claim = kept(
                    &format!("person{index}"),
                    "met",
                    "union",
                    "stability",
                    "art",
                    "fr",
                    false,
                );
                claim.confidence = f64::from(index) / 10.0;
                claim
            })
            .collect();
        let capped = cap_claims(claims, CLAIM_LIMIT);
        assert_eq!(capped.len(), 5);
        assert!(capped.iter().all(|claim| claim.entity != "person0"));
    }

    #[test]
    fn aliases_merge_on_shared_tokens_when_the_longer_form_is_a_span() {
        let rows = vec![article(
            "art-1",
            "geopolitical",
            "ru",
            "Vladimir Putin met Macron",
            "",
            1.0,
        )];
        let mut claims = vec![
            kept(
                "putin",
                "met",
                "macron",
                "geopolitical",
                "art-1",
                "ru",
                false,
            ),
            kept(
                "vladimir putin",
                "met",
                "macron",
                "geopolitical",
                "art-1",
                "ru",
                false,
            ),
        ];
        merge_aliases(&mut claims, &rows);
        assert!(claims.iter().all(|claim| claim.entity == "vladimir putin"));
    }

    #[test]
    fn a_country_token_does_not_merge_into_a_longer_name() {
        let rows = vec![article(
            "art-1",
            "military",
            "us",
            "US Army deployed to Poland",
            "",
            1.0,
        )];
        let mut claims = vec![
            kept("us", "deployed", "poland", "military", "art-1", "us", false),
            kept(
                "us army", "deployed", "poland", "military", "art-1", "us", false,
            ),
        ];
        merge_aliases(&mut claims, &rows);
        assert_eq!(claims[0].entity, "us");
        assert_eq!(claims[1].entity, "us army");
    }

    #[test]
    fn links_follow_the_entity_and_a_country_alone_is_not_cross_topic() {
        let claims = vec![
            kept(
                "putin",
                "sanctioned",
                "acme",
                "military",
                "art-us",
                "us",
                false,
            ),
            kept(
                "putin",
                "sanctioned",
                "globex",
                "economic",
                "art-fr",
                "fr",
                false,
            ),
            kept(
                "macron",
                "met",
                "putin",
                "geopolitical",
                "art-us",
                "us",
                false,
            ),
            kept("us", "delayed", "vote", "stability", "art-us", "us", false),
            kept("us", "opened", "talks", "economic", "art-us", "us", false),
        ];
        let relations = link_claims(&claims);
        assert!(relations
            .iter()
            .any(|(_, _, kind)| kind == "conflict_or_revision"));
        assert!(relations.iter().any(|(_, _, kind)| kind == "cross_topic"));
        assert!(relations.iter().any(|(_, _, kind)| kind == "cross_country"));
        assert!(relations.iter().any(|(_, _, kind)| kind == "co_mentioned"));
        assert!(!relations.iter().any(|(left, _, kind)| {
            kind == "cross_topic"
                && (left.contains("\"us\"") || left.starts_with("[\"person\",\"us\""))
        }));
        let country_pairs = relations
            .iter()
            .filter(|(_, _, kind)| kind == "cross_topic");
        assert!(country_pairs
            .into_iter()
            .all(|(left, right, _)| { !left.contains(",\"us\",") && !right.contains(",\"us\",") }));
    }

    #[test]
    fn context_claims_require_the_entity_in_the_title_and_link_context_for() {
        let packet = vec![article(
            "art-1",
            "military",
            "us",
            "Putin sanctioned Acme",
            "",
            1.0,
        )];
        let unk = vec![article(
            "art-unk",
            "unk",
            "us",
            "Putin visits the port",
            "The port reopened",
            0.2,
        )];
        let (lead, dropped) = accept_claims(
            &packet,
            &[raw(
                "Putin",
                "person",
                "sanctioned",
                "Acme",
                "military",
                "Putin sanctioned Acme.",
                "fact",
                0.7,
                "art-1",
            )],
            AcceptMode::Lead,
        );
        assert_eq!(dropped, 0);
        let settled = finish_insights(
            1,
            dropped,
            &packet,
            lead,
            &unk,
            &[
                raw(
                    "Putin",
                    "person",
                    "visited",
                    "port",
                    "unk",
                    "Putin visited the port.",
                    "fact",
                    0.4,
                    "art-unk",
                ),
                raw(
                    "Macron",
                    "person",
                    "visited",
                    "port",
                    "unk",
                    "Macron visited the port.",
                    "inference",
                    0.4,
                    "art-unk",
                ),
            ],
        );
        assert_eq!(settled.stats.context, 1);
        assert_eq!(settled.stats.claims, 1);
        assert!(settled.stats.context_for >= 1);
        assert!(settled
            .claims
            .iter()
            .any(|claim| claim.classification == "inference" && claim.predicate == "visited"));
        assert!(!settled.claims.iter().any(|claim| claim.entity == "macron"));
    }

    #[test]
    fn older_run_stats_without_insights_still_parse() {
        let stats: RunStats =
            serde_json::from_str(r#"{"counts":{},"origins":[],"scored":false}"#).unwrap();
        assert_eq!(stats.insights.claims, 0);
        assert!(stats.insights.rows.is_empty());
        let lines = insight_table_lines(&stats.insights);
        assert_eq!(
            lines,
            vec!["No insights extracted for this cycle.".to_string()]
        );
    }

    #[test]
    fn persisted_claims_use_the_atlas_app_and_recall_hits_the_sentence() {
        let store = Store::memory().unwrap();
        let run_id = "atlas-cycle-1";
        let packet = vec![article(
            "art-9",
            "military",
            "us",
            "Putin sanctioned Acme",
            "over oil",
            1.0,
        )];
        let (lead, dropped) = accept_claims(
            &packet,
            &[raw(
                "Putin",
                "person",
                "sanctioned",
                "Acme",
                "military",
                "Putin sanctioned Acme over oil.",
                "fact",
                0.8,
                "art-9",
            )],
            AcceptMode::Lead,
        );
        let settled = finish_insights(1, dropped, &packet, lead, &[], &[]);
        store
            .persist_atlas_insights(
                run_id,
                &settled.claims,
                &settled.relations,
                &settled.brief,
                &settled.entity_path,
            )
            .unwrap();
        store
            .persist_atlas_insights(
                run_id,
                &settled.claims,
                &settled.relations,
                &settled.brief,
                &settled.entity_path,
            )
            .unwrap();
        let memories = store.list_memories().unwrap();
        let claim = memories
            .iter()
            .find(|memory| memory.text == "Putin sanctioned Acme over oil.")
            .unwrap();
        assert_eq!(
            claim.source,
            MemorySource {
                app: "atlas".into(),
                conversation_id: run_id.into(),
                message_id: None,
                reference: Some(run_id.into()),
            }
        );
        assert_eq!(memories.len(), 2);
        let summary = store
            .graph_summary(
                memories
                    .iter()
                    .find(|memory| memory.text.contains('\n'))
                    .unwrap()
                    .id
                    .as_str(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(summary.focus, run_id);
        assert!(summary.summary.contains("putin → sanctioned → acme"));
        let source: (String, String, String) = store
            .conn
            .query_row(
                "SELECT call_id, source_url, answer_id FROM insight_sources WHERE run_id=?1",
                [run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(source.0, "art-9");
        assert_eq!(source.1, "https://example.com/art-9");
        assert_eq!(source.2, "atlas-atlas-cycle-1");
        let hits = store.recall("Putin sanctioned Acme", 4).unwrap();
        assert!(hits
            .iter()
            .any(|hit| hit.memory.text.contains("Putin sanctioned Acme")));
        assert!(store.atlas_has_insights(run_id).unwrap());
    }
}
