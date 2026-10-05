//! Final Atlas step: gate every non-unk article, extract lean claim elements with
//! synthesis, ask the classifier which cycle articles are most relevant to those
//! elements, then use that peer set for fact, inference, and link support before
//! storing them where Brain recall already looks.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::atlas::{category_tag, OriginStat};
use crate::osint::{
    best_credibility, information_credibility, scale_confidence, wikipedia_rsp, AdmiraltyCode,
    CredibilityInputs, InformationCredibility, SourceReliability,
};
use crate::provider::{self, ChatMessage};
use crate::secrets::ProviderSecret;
use crate::store::{
    insight_fingerprint, AtlasArticleRow, AtlasInsightClaim, AtlasStoredClaim, Store,
};

/// Articles sent in one extract call. Kept small so synthesis streams stay short.
pub const PACKET_LIMIT: usize = 4;
/// Related peers shown when judging support for one claim.
pub const RELATED_LIMIT: usize = 5;
/// Max atomic elements extracted from one full article body.
pub const BODY_CLAIM_LIMIT: usize = 8;
const DESCRIPTION_CHARS: usize = 160;
/// Body text appended into the span-gate blob / model packet for body re-extract.
const BODY_SPAN_CHARS: usize = 12_000;
const COL_ENTITY: usize = 24;
const COL_PREDICATE: usize = 16;
const COL_OBJECT: usize = 23;
const COL_TOPIC: usize = 16;
const COL_CLASS: usize = 10;

const NAMESPACES: &[&str] = &["person", "org", "place", "agreement"];

fn lead_prompt(limit: usize) -> String {
    format!(
        "Extract at most {limit} concise atomic news elements, one per article. Return a JSON object {{\"claims\":[{{\"entity\":string,\"namespace\":string,\"predicate\":string,\"object\":string,\"topic\":string,\"claim\":string,\"classification\":\"fact\"|\"inference\",\"confidence\":number,\"evidence_ids\":[string]}}]}}. \
entity and object must be verbatim spans copied from the cited article title or description. Do not invent names. \
namespace is person, org, place, or agreement. \
predicate is the verb the article supports, such as sanctioned, deployed, or met. \
topic is the article category. \
evidence_ids must be ids from the packet. \
The claim is one sentence that contains the entity and the object. \
Set classification to fact only when both spans are in the title; otherwise inference. \
Do not compare other articles here; peer support is judged later. \
published_at is the source time. \
country is the publisher country, not the entity."
    )
}

fn context_prompt(limit: usize) -> String {
    format!(
        "Extract at most {limit} context claims from unclassified articles, one per article. Return a JSON object {{\"claims\":[...]}} with the same claim fields as a lead extraction. \
Use an entity from the Entities list, and only when that entity is a verbatim span of the article title. \
object must be a verbatim span of the same article. \
Set classification to inference. \
Do not extract an article whose title does not contain one of the entities. \
If no article qualifies, return {{\"claims\":[]}}. \
published_at is the source time. \
country is the publisher country, not the entity."
    )
}

fn body_lead_prompt(limit: usize) -> String {
    format!(
        "Extract at most {limit} concise atomic news elements from the full article. Return a JSON object {{\"claims\":[{{\"entity\":string,\"namespace\":string,\"predicate\":string,\"object\":string,\"topic\":string,\"claim\":string,\"classification\":\"fact\"|\"inference\",\"confidence\":number,\"evidence_ids\":[string]}}]}}. \
entity and object must be verbatim spans copied from the cited article title, description, or body. Do not invent names. \
namespace is person, org, place, or agreement. \
predicate is the verb the article supports, such as sanctioned, deployed, or met. \
topic is the article category. \
evidence_ids must contain the article id. \
The claim is one sentence that contains the entity and the object. \
Set classification to fact only when both spans are in the title; otherwise inference. \
Include attribution, negation, event dates, and qualifications present in the body but missing from the headline. \
published_at is the source time. \
country is the publisher country, not the entity."
    )
}

/// Clone an article with body text folded into `description` so the span gate can accept body spans.
pub fn article_with_body_spans(article: &AtlasArticleRow, body_markdown: &str) -> AtlasArticleRow {
    let mut clone = article.clone();
    let body = clip_chars(body_markdown, BODY_SPAN_CHARS);
    if body.is_empty() {
        return clone;
    }
    if clone.description.trim().is_empty() {
        clone.description = body;
    } else {
        clone.description = format!("{}\n{body}", clone.description.trim());
    }
    clone
}

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
    #[serde(default)]
    pub admiralty: String,
    #[serde(default)]
    pub reliability: String,
    #[serde(default)]
    pub info_credibility: u8,
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
    /// Publisher time of the cited article.
    pub published_at: String,
    /// Publisher country of the cited article. Never written onto the entity.
    pub country: String,
    pub context: bool,
    /// Independent peers with title-level support (set by [`apply_peer_support`]).
    pub title_peers: u32,
    /// Independent peers with body-level support only.
    pub body_peers: u32,
    /// Admiralty Source Reliability letter (A–F).
    pub reliability: String,
    /// Information Credibility digit (1–6).
    pub info_credibility: u8,
    /// Combined code such as `B2`.
    pub admiralty: String,
    /// WP:RSP status code when listed (`gr`, `gu`, …).
    pub rsp_status: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcceptMode {
    Lead,
    Context,
}

/// Lead extraction plus optional packet failures that did not drop kept claims.
pub struct Extraction {
    pub settled: Settled,
    /// Lead packet failures that still left some claims.
    pub extract_error: Option<String>,
    /// Classifier peer-matching failure; deterministic overlap was used instead.
    pub peer_error: Option<String>,
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
    lead = dedupe_claims(lead);
    if gated > 0 {
        lead = cap_claims(lead, gated as usize);
    }
    let before_match = context.len();
    context.retain(|claim| lead.iter().any(|lead| lead.entity == claim.entity));
    let unmatched = before_match.saturating_sub(context.len()) as u32;
    context.retain(|claim| {
        lead.iter()
            .all(|lead| fingerprint(lead) != fingerprint(claim))
    });
    context = dedupe_claims(context);

    let mut claims = lead;
    claims.extend(context);
    let relations = link_claims(&claims);
    apply_admiralty_evaluation(&mut claims, &articles, &relations);
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
                admiralty: claim.admiralty.clone(),
                reliability: claim.reliability.clone(),
                info_credibility: claim.info_credibility,
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

/// The one stats line on a pipeline-run card: gated, claims, fact, inference, context, dropped, links.
pub fn insight_stats_line(stats: &InsightStats) -> String {
    if stats.gated == 0
        && stats.claims == 0
        && stats.context == 0
        && stats.dropped == 0
        && stats.rows.is_empty()
    {
        return "No insights extracted for this cycle.".into();
    }
    let links = stats.conflict_or_revision
        + stats.cross_topic
        + stats.cross_country
        + stats.context_for
        + stats.co_mentioned;
    format!(
        "Gated {}  Claims {}  Fact {}  Inference {}  Context {}  Dropped {}  Links {}",
        stats.gated,
        stats.claims,
        stats.facts,
        stats.inferences,
        stats.context,
        stats.dropped,
        links
    )
}

pub fn insight_table_lines(stats: &InsightStats) -> Vec<String> {
    let stats_line = insight_stats_line(stats);
    if stats.rows.is_empty() && !stats_line.starts_with("Gated ") {
        return vec![stats_line];
    }
    let header = insight_header();
    let mut lines = vec![stats_line, header];
    if stats.rows.is_empty() {
        lines.push("No insights extracted for this cycle.".into());
        return lines;
    }
    for row in &stats.rows {
        lines.push(insight_row_line(row));
    }
    lines
}

fn insight_header() -> String {
    format!(
        "{} {} {} {} {}",
        fit_cell("Entity", COL_ENTITY),
        fit_cell("Predicate", COL_PREDICATE),
        fit_cell("Object", COL_OBJECT),
        fit_cell("Topic", COL_TOPIC),
        fit_cell("Class", COL_CLASS),
    )
}

fn insight_row_line(row: &InsightRow) -> String {
    format!(
        "{} {} {} {} {}",
        fit_cell(&row.entity, COL_ENTITY),
        fit_cell(&row.predicate, COL_PREDICATE),
        fit_cell(&row.object, COL_OBJECT),
        fit_cell(&row.topic, COL_TOPIC),
        fit_cell(&row.classification, COL_CLASS),
    )
}

fn fit_cell(value: &str, width: usize) -> String {
    let text: String = value.chars().take(width).collect();
    format!("{text:<width$}")
}

/// Extract lean elements from every non-unk article with synthesis, ask the
/// classifier which cycle articles are most relevant to those elements, then
/// use that peer set for fact, inference, and link support.
/// `progress` receives `(done, total)` work units as each extract step finishes.
pub async fn extract(
    synthesis: &ProviderSecret,
    classifier: Option<&ProviderSecret>,
    articles: &[AtlasArticleRow],
    origins: &[OriginStat],
    mut progress: impl FnMut(u32, u32),
) -> Result<Extraction> {
    if provider::is_decisions_model(&synthesis.model) {
        return Err(anyhow!(
            "{} is a decisions model and cannot extract claims. Use the synthesis chat model.",
            synthesis.model
        ));
    }
    let significant = select_significant(articles, origins);
    let gated = significant.len() as u32;
    if significant.is_empty() {
        progress(0, 0);
        return Ok(Extraction {
            settled: finish_insights(gated, 0, &significant, Vec::new(), &[], &[]),
            extract_error: None,
            peer_error: None,
            context_error: None,
        });
    }
    let lead_packets = significant.chunks(PACKET_LIMIT).len() as u32;
    let mut total = lead_packets + u32::from(classifier.is_some());
    let mut done = 0u32;
    progress(done, total);
    let mut lead = Vec::new();
    let mut lead_dropped = 0u32;
    let mut extract_error = None;
    for packet in significant.chunks(PACKET_LIMIT) {
        match ask_claims(synthesis, &lead_prompt(packet.len()), &packet_json(packet)?).await {
            Ok(raw) => {
                let (kept, dropped) = accept_claims(packet, &raw, AcceptMode::Lead);
                lead_dropped += dropped;
                lead.extend(kept);
            }
            Err(err) => {
                extract_error = Some(err.to_string());
            }
        }
        done += 1;
        progress(done, total);
    }
    if lead.is_empty() {
        if let Some(err) = extract_error {
            return Err(anyhow!(err));
        }
        return Ok(Extraction {
            settled: finish_insights(gated, lead_dropped, &significant, lead, &[], &[]),
            extract_error: None,
            peer_error: None,
            context_error: None,
        });
    }
    merge_aliases(&mut lead, &significant);
    let (peers, peer_error) = match classifier {
        Some(classifier) => {
            let result = classify_relevant_articles(classifier, &lead, articles).await;
            done += 1;
            progress(done, total);
            match result {
                Ok(peers) => (peers, None),
                Err(err) => (Vec::new(), Some(err.to_string())),
            }
        }
        None => (Vec::new(), None),
    };
    apply_peer_support(&mut lead, articles, &peers);
    let lead = cap_claims(dedupe_claims(lead), significant.len().max(1));
    if lead.is_empty() {
        return Ok(Extraction {
            settled: finish_insights(gated, lead_dropped, &significant, lead, &[], &[]),
            extract_error,
            peer_error,
            context_error: None,
        });
    }
    let context_articles = context_candidates(articles, &lead);
    if context_articles.is_empty() {
        return Ok(Extraction {
            settled: finish_insights(gated, lead_dropped, &significant, lead, &[], &[]),
            extract_error,
            peer_error,
            context_error: None,
        });
    }
    let context_packets = context_articles.chunks(PACKET_LIMIT).len() as u32;
    total += context_packets;
    progress(done, total);
    let entities: Vec<String> = lead.iter().map(|claim| claim.entity.clone()).collect();
    let mut context_raw = Vec::new();
    let mut context_error = None;
    for packet in context_articles.chunks(PACKET_LIMIT) {
        let user = format!(
            "Entities:\n{}\nArticles:\n{}",
            entities.join("\n"),
            packet_json(packet)?
        );
        match ask_claims(synthesis, &context_prompt(packet.len()), &user).await {
            Ok(raw) => context_raw.extend(raw),
            Err(err) => {
                context_error = Some(err.to_string());
            }
        }
        done += 1;
        progress(done, total);
    }
    Ok(Extraction {
        settled: finish_insights(
            gated,
            lead_dropped,
            &significant,
            lead,
            &context_articles,
            &context_raw,
        ),
        extract_error,
        peer_error,
        context_error,
    })
}

/// Re-extract lean elements from one article using title, description, and cleaned body.
/// `peer_articles` are other same-cycle articles used for peer support (may be empty).
pub async fn extract_for_article_body(
    synthesis: &ProviderSecret,
    classifier: Option<&ProviderSecret>,
    article: &AtlasArticleRow,
    body_markdown: &str,
    peer_articles: &[AtlasArticleRow],
) -> Result<Settled> {
    if provider::is_decisions_model(&synthesis.model) {
        return Err(anyhow!(
            "{} is a decisions model and cannot extract claims. Use the synthesis chat model.",
            synthesis.model
        ));
    }
    if body_markdown.trim().is_empty() {
        return Ok(finish_insights(
            1,
            0,
            &[article.clone()],
            Vec::new(),
            &[],
            &[],
        ));
    }
    let span_article = article_with_body_spans(article, body_markdown);
    let packet = [span_article];
    let user = packet_json_with_body(article, body_markdown)?;
    let raw = ask_claims(synthesis, &body_lead_prompt(BODY_CLAIM_LIMIT), &user).await?;
    let (mut lead, lead_dropped) = accept_claims(&packet, &raw, AcceptMode::Lead);
    if lead.is_empty() {
        return Ok(finish_insights(1, lead_dropped, &packet, lead, &[], &[]));
    }
    merge_aliases(&mut lead, &packet);
    let mut catalog = peer_articles.to_vec();
    if !catalog.iter().any(|row| row.id == article.id) {
        catalog.push(article.clone());
    }
    let peers = match classifier {
        Some(classifier) => classify_relevant_articles(classifier, &lead, &catalog)
            .await
            .unwrap_or_default(),
        None => Vec::new(),
    };
    apply_peer_support(&mut lead, &catalog, &peers);
    let lead = cap_claims(dedupe_claims(lead), BODY_CLAIM_LIMIT);
    Ok(finish_insights(1, lead_dropped, &packet, lead, &[], &[]))
}

fn packet_json_with_body(article: &AtlasArticleRow, body_markdown: &str) -> Result<String> {
    let rows = [serde_json::json!({
        "id": article.id,
        "title": article.title,
        "description": clip_chars(&article.description, DESCRIPTION_CHARS),
        "body": clip_chars(body_markdown, BODY_SPAN_CHARS),
        "category": article.category,
        "country": article.country,
        "published_at": article.published_at,
    })];
    Ok(serde_json::to_string(&rows)?)
}

/// Classifier output: for each lead claim index, the most relevant peer article ids.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PeerMatch {
    pub claim_index: usize,
    pub article_ids: Vec<String>,
}

/// Other articles that share the claim's entity or enough subject tokens to judge support.
pub fn related_articles<'a>(
    claim: &KeptClaim,
    articles: &'a [AtlasArticleRow],
) -> Vec<&'a AtlasArticleRow> {
    let claim_tokens = tokens(&format!(
        "{} {} {}",
        claim.entity, claim.predicate, claim.object
    ));
    let mut scored: Vec<(usize, &AtlasArticleRow)> = articles
        .iter()
        .filter(|article| article.id != claim.article_id)
        .filter_map(|article| {
            let blob = format!("{} {}", article.title, article.description);
            let entity_hit = contains_span(&blob, &claim.entity);
            let object_hit = contains_span(&blob, &claim.object);
            let topic_hit = !claim.topic.is_empty()
                && category_tag(&article.category) == category_tag(&claim.topic);
            let overlap = tokens(&article.title)
                .into_iter()
                .filter(|token| claim_tokens.iter().any(|item| item == token))
                .count();
            if !entity_hit && !object_hit && !(topic_hit && overlap >= 2) {
                return None;
            }
            let score = usize::from(entity_hit) * 4
                + usize::from(object_hit) * 3
                + usize::from(topic_hit)
                + overlap;
            Some((score, article))
        })
        .collect();
    scored.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.id.cmp(&right.1.id))
    });
    scored
        .into_iter()
        .take(RELATED_LIMIT)
        .map(|(_, article)| article)
        .collect()
}

/// Raise fact/inference and confidence when related articles carry the same elements.
/// `classifier_peers` comes from the classifier; when empty, token overlap is used.
pub fn apply_peer_support(
    claims: &mut [KeptClaim],
    articles: &[AtlasArticleRow],
    classifier_peers: &[PeerMatch],
) {
    for (index, claim) in claims.iter_mut().enumerate() {
        if claim.context {
            continue;
        }
        let peers: Vec<&AtlasArticleRow> = classifier_peers
            .iter()
            .find(|item| item.claim_index == index)
            .map(|item| {
                item.article_ids
                    .iter()
                    .filter_map(|id| {
                        articles
                            .iter()
                            .find(|article| &article.id == id && article.id != claim.article_id)
                    })
                    .take(RELATED_LIMIT)
                    .collect()
            })
            .filter(|peers: &Vec<&AtlasArticleRow>| !peers.is_empty())
            .unwrap_or_else(|| related_articles(claim, articles));
        let mut title_support = 0u32;
        let mut body_support = 0u32;
        for peer in peers {
            let title_both = contains_span(&peer.title, &claim.entity)
                && contains_span(&peer.title, &claim.object);
            let body = format!("{} {}", peer.title, peer.description);
            let body_both =
                contains_span(&body, &claim.entity) && contains_span(&body, &claim.object);
            if title_both {
                title_support += 1;
            } else if body_both {
                body_support += 1;
            }
        }
        claim.title_peers = title_support;
        claim.body_peers = body_support;
        if title_support > 0 {
            claim.classification = "fact".into();
            claim.confidence = (claim.confidence + 0.15 * f64::from(title_support)).min(1.0);
        } else if body_support > 0 {
            if claim.classification != "fact" {
                claim.classification = "inference".into();
            }
            claim.confidence = (claim.confidence + 0.08 * f64::from(body_support)).min(1.0);
        }
    }
}

/// Assign Admiralty A–F / 1–6 from WP:RSP + peer support, then scale confidence.
pub fn apply_admiralty_evaluation(
    claims: &mut [KeptClaim],
    articles: &[AtlasArticleRow],
    relations: &[(String, String, String)],
) {
    let index = wikipedia_rsp::cached_index();
    let conflicted: std::collections::HashSet<String> = relations
        .iter()
        .filter(|(_, _, relation)| relation == "conflict_or_revision")
        .flat_map(|(left, right, _)| [left.clone(), right.clone()])
        .collect();
    for claim in claims.iter_mut() {
        let article = articles.iter().find(|row| row.id == claim.article_id);
        let domain = article.map(|row| row.source_domain.as_str()).unwrap_or("");
        let (reliability, entry) = match index.as_ref() {
            Some(index) => index.reliability_for_domain(domain),
            None => (SourceReliability::F, None),
        };
        let description_empty = article
            .map(|row| row.description.trim().is_empty())
            .unwrap_or(true);
        let fp = fingerprint(claim);
        let has_conflict = conflicted.contains(&fp);
        let credibility = information_credibility(CredibilityInputs {
            classification: &claim.classification,
            confidence: claim.confidence,
            title_peers: claim.title_peers,
            body_peers: claim.body_peers,
            description_empty,
            reliability,
            has_conflict,
        });
        let code = AdmiraltyCode::new(reliability, credibility);
        claim.confidence = scale_confidence(claim.confidence, code);
        claim.reliability = reliability.as_str().into();
        claim.info_credibility = credibility.as_u8();
        claim.admiralty = code.display();
        claim.rsp_status = entry
            .map(|item| item.status.as_str().to_string())
            .unwrap_or_default();
    }
}

/// Article-level Information Credibility: best (lowest digit) among its claims.
pub fn article_information_credibility(claims: &[KeptClaim]) -> InformationCredibility {
    let values: Vec<InformationCredibility> = claims
        .iter()
        .filter_map(|claim| InformationCredibility::from_u8(claim.info_credibility))
        .collect();
    best_credibility(&values)
}

/// Ask the classifier which cycle articles best support each extracted element.
pub async fn classify_relevant_articles(
    classifier: &ProviderSecret,
    claims: &[KeptClaim],
    articles: &[AtlasArticleRow],
) -> Result<Vec<PeerMatch>> {
    if claims.is_empty() || articles.is_empty() {
        return Ok(Vec::new());
    }
    if provider::is_decisions_model(&classifier.model) {
        return classify_peers_decisions(classifier, claims, articles).await;
    }
    classify_peers_chat(classifier, claims, articles).await
}

async fn classify_peers_decisions(
    classifier: &ProviderSecret,
    claims: &[KeptClaim],
    articles: &[AtlasArticleRow],
) -> Result<Vec<PeerMatch>> {
    let mut matches = Vec::new();
    for (index, claim) in claims.iter().enumerate() {
        if claim.context {
            continue;
        }
        let candidates = peer_candidates(claim, articles);
        if candidates.is_empty() {
            continue;
        }
        let (state, questions) = peer_decisions_request(claim, &candidates);
        let response = provider::decide(classifier, &state, &questions).await?;
        let choice = response
            .answers
            .get("peer")
            .and_then(|answer| answer.choice.as_deref())
            .unwrap_or("none");
        if choice == "none" || choice.is_empty() {
            continue;
        }
        if candidates.iter().any(|article| article.id == choice) {
            matches.push(PeerMatch {
                claim_index: index,
                article_ids: vec![choice.to_string()],
            });
        }
    }
    Ok(matches)
}

async fn classify_peers_chat(
    classifier: &ProviderSecret,
    claims: &[KeptClaim],
    articles: &[AtlasArticleRow],
) -> Result<Vec<PeerMatch>> {
    let elements: Vec<Value> = claims
        .iter()
        .enumerate()
        .filter(|(_, claim)| !claim.context)
        .map(|(index, claim)| {
            serde_json::json!({
                "index": index,
                "entity": claim.entity,
                "predicate": claim.predicate,
                "object": claim.object,
                "topic": claim.topic,
                "claim": claim.claim,
                "source_article_id": claim.article_id,
                "published_at": claim.published_at,
            })
        })
        .collect();
    if elements.is_empty() {
        return Ok(Vec::new());
    }
    let catalog = catalog_json(articles)?;
    let system = format!(
        "You match extracted news elements to the most relevant supporting articles in the same news cycle. \
Return a JSON object {{\"matches\":[{{\"index\":number,\"article_ids\":[string]}}]}}. \
Each index is an element index. article_ids are ids from the Articles list, never the element's source_article_id. \
Pick at most {RELATED_LIMIT} ids per element, ordered by relevance. \
Choose articles that share the same subject matter or elements and could support a fact, inference, or link. \
If none apply, return an empty article_ids list for that element."
    );
    let user = format!(
        "Elements:\n{}\nArticles:\n{}",
        serde_json::to_string(&elements)?,
        catalog
    );
    let messages = [
        ChatMessage {
            role: "system".into(),
            content: system,
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
        ChatMessage {
            role: "user".into(),
            content: user,
            tool_call_id: None,
            tool_calls: Vec::new(),
        },
    ];
    let done = provider::complete(classifier, &messages, &[], |_| {}).await?;
    Ok(parse_peer_matches(&done.content, claims, articles))
}

/// Lean Decisions question: one peer article id, or none.
pub fn peer_decisions_request(
    claim: &KeptClaim,
    candidates: &[&AtlasArticleRow],
) -> (Value, Value) {
    let mut criteria = serde_json::Map::new();
    for article in candidates {
        criteria.insert(
            article.id.clone(),
            serde_json::json!(format!(
                "{} · {} · {} · {}",
                article.title,
                category_tag(&article.category),
                article.country,
                article.published_at
            )),
        );
    }
    criteria.insert(
        "none".into(),
        serde_json::json!("No other article in this cycle supports this element."),
    );
    let questions = serde_json::json!({
        "peer": {
            "type": "choice",
            "instructions": "Choose the single most relevant supporting article for this extracted element. Choose none when no candidate shares the subject matter.",
            "criteria": criteria
        }
    });
    let state = serde_json::json!({
        "entity": claim.entity,
        "predicate": claim.predicate,
        "object": claim.object,
        "topic": claim.topic,
        "claim": claim.claim,
        "source_article_id": claim.article_id,
        "published_at": claim.published_at,
    });
    (state, questions)
}

/// Prefilter peers before the classifier ranks them.
pub fn peer_candidates<'a>(
    claim: &KeptClaim,
    articles: &'a [AtlasArticleRow],
) -> Vec<&'a AtlasArticleRow> {
    let mut related = related_articles(claim, articles);
    if related.len() >= RELATED_LIMIT {
        return related;
    }
    let known: std::collections::HashSet<&str> =
        related.iter().map(|article| article.id.as_str()).collect();
    let mut extras: Vec<&AtlasArticleRow> = articles
        .iter()
        .filter(|article| article.id != claim.article_id && !known.contains(article.id.as_str()))
        .filter(|article| {
            !claim.topic.is_empty()
                && category_tag(&article.category) == category_tag(&claim.topic)
                && category_tag(&article.category) != "unk"
        })
        .collect();
    extras.sort_by(|left, right| {
        right
            .temperature
            .total_cmp(&left.temperature)
            .then_with(|| left.id.cmp(&right.id))
    });
    for article in extras {
        if related.len() >= RELATED_LIMIT.max(8) {
            break;
        }
        related.push(article);
    }
    if related.is_empty() {
        articles
            .iter()
            .filter(|article| {
                article.id != claim.article_id && category_tag(&article.category) != "unk"
            })
            .take(RELATED_LIMIT.max(8))
            .collect()
    } else {
        related
    }
}

pub fn parse_peer_matches(
    text: &str,
    claims: &[KeptClaim],
    articles: &[AtlasArticleRow],
) -> Vec<PeerMatch> {
    let Ok(value) = parse_json_value(text) else {
        return Vec::new();
    };
    let Some(rows) = value.get("matches").and_then(Value::as_array) else {
        return Vec::new();
    };
    let known: std::collections::HashSet<&str> =
        articles.iter().map(|article| article.id.as_str()).collect();
    let mut matches = Vec::new();
    for row in rows {
        let Some(index) = row.get("index").and_then(Value::as_u64).map(|n| n as usize) else {
            continue;
        };
        let Some(claim) = claims.get(index) else {
            continue;
        };
        if claim.context {
            continue;
        }
        let ids = row
            .get("article_ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|id| known.contains(id) && *id != claim.article_id)
            .take(RELATED_LIMIT)
            .map(str::to_string)
            .collect::<Vec<_>>();
        if !ids.is_empty() {
            matches.push(PeerMatch {
                claim_index: index,
                article_ids: ids,
            });
        }
    }
    matches
}

fn catalog_json(articles: &[AtlasArticleRow]) -> Result<String> {
    let rows: Vec<Value> = articles
        .iter()
        .map(|article| {
            serde_json::json!({
                "id": article.id,
                "title": article.title,
                "category": article.category,
                "country": article.country,
                "published_at": article.published_at,
            })
        })
        .collect();
    Ok(serde_json::to_string(&rows)?)
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
        published_at: article.published_at.clone(),
        country: article.country.clone(),
        context: mode == AcceptMode::Context,
        title_peers: 0,
        body_peers: 0,
        reliability: String::new(),
        info_credibility: 0,
        admiralty: String::new(),
        rsp_status: String::new(),
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
        admiralty: claim.admiralty.clone(),
        reliability: claim.reliability.clone(),
        info_credibility: claim.info_credibility,
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
        published_at: claim.published_at.clone(),
        reliability: claim.reliability.clone(),
        info_credibility: claim.info_credibility,
        admiralty: claim.admiralty.clone(),
        rsp_status: claim.rsp_status.clone(),
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
        let code = if claim.admiralty.is_empty() {
            String::new()
        } else {
            format!(" [{}]", claim.admiralty)
        };
        lines.push(format!(
            "- {}{} ({})",
            claim.claim.trim(),
            code,
            claim.article_id
        ));
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
                "published_at": article.published_at,
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
    if let Some(claims) = value.get("claims").and_then(Value::as_array) {
        return Ok(claims.iter().map(raw_claim).collect());
    }
    if value.get("claims").is_some_and(Value::is_null) {
        return Ok(Vec::new());
    }
    if let Some(claims) = value.as_array() {
        return Ok(claims.iter().map(raw_claim).collect());
    }
    if value
        .get("entity")
        .and_then(Value::as_str)
        .is_some_and(|text| !text.is_empty())
        || value
            .get("claim")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty())
    {
        return Ok(vec![raw_claim(&value)]);
    }
    // Empty object, missing claims, or a soft "nothing found" reply.
    if value.get("claims").is_none() {
        return Ok(Vec::new());
    }
    Err(anyhow!("claims array missing"))
}

fn parse_json_value(text: &str) -> Result<Value> {
    let trimmed = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    if trimmed.is_empty() {
        return Ok(serde_json::json!({"claims": []}));
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Ok(value);
    }
    let Some(start) = trimmed.find(['{', '[']) else {
        // Prose with no JSON object — treat as no claims for this packet.
        return Ok(serde_json::json!({"claims": []}));
    };
    let open = trimmed.as_bytes()[start];
    let close = if open == b'[' { ']' } else { '}' };
    let Some(end) = trimmed.rfind(close) else {
        return Ok(serde_json::json!({"claims": []}));
    };
    if end < start {
        return Ok(serde_json::json!({"claims": []}));
    }
    match serde_json::from_str(&trimmed[start..=end]) {
        Ok(value) => Ok(value),
        Err(_) => Ok(serde_json::json!({"claims": []})),
    }
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
            published_at: String::new(),
            country: country.into(),
            context,
            title_peers: 0,
            body_peers: 0,
            reliability: String::new(),
            info_credibility: 0,
            admiralty: String::new(),
            rsp_status: String::new(),
        }
    }

    #[test]
    fn admiralty_scales_claim_confidence_from_rsp_and_peers() {
        use crate::osint::wikipedia_rsp::{install_index, parse_rsp_wikitext, RspIndex, RspStatus};
        let sample = r#"
|- class="s-gr" id="Wire"
| [[Wire]]
| {{WP:RSPSTATUS|gr}}
| [[WP:x|1]]
| {{WP:RSPLAST|2022}}
| Wire is generally reliable for news reporting according to community consensus discussions.
| {{WP:RSPUSES|wire.example}}
"#;
        install_index(RspIndex::build(parse_rsp_wikitext(sample), "t".into()));
        let mut art = article(
            "a1",
            "military",
            "us",
            "Alpha met Beta today",
            "Alpha met Beta in Geneva",
            1.0,
        );
        art.source_domain = "wire.example".into();
        let peer = article(
            "a2",
            "military",
            "us",
            "Alpha met Beta today",
            "More on Alpha and Beta",
            0.9,
        );
        let mut claims = vec![kept("alpha", "met", "beta", "military", "a1", "us", false)];
        claims[0].classification = "fact".into();
        claims[0].confidence = 0.70;
        apply_peer_support(&mut claims, &[art.clone(), peer], &[]);
        assert!(claims[0].title_peers >= 1);
        let before = claims[0].confidence;
        apply_admiralty_evaluation(&mut claims, &[art], &[]);
        assert_eq!(claims[0].reliability, "B");
        assert_eq!(claims[0].info_credibility, 1);
        assert_eq!(claims[0].admiralty, "B1");
        assert_eq!(claims[0].rsp_status, RspStatus::GenerallyReliable.as_str());
        assert!(claims[0].confidence > before);
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
    fn the_packet_keeps_four_and_reports_the_full_gate() {
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
        assert_eq!(packet.len(), PACKET_LIMIT);
        assert_eq!(packet[0].id, "a00");
        assert!(packet.iter().all(|row| row.id != "a04"));
    }

    #[test]
    fn peer_articles_raise_a_claim_to_fact_and_boost_confidence() {
        let articles = vec![
            article(
                "a1",
                "military",
                "us",
                "Yemen forces strike houthi targets",
                "Overnight raids continued",
                1.0,
            ),
            article(
                "a2",
                "military",
                "gb",
                "Yemen forces strike houthi targets again",
                "A second outlet confirms the raids",
                0.8,
            ),
            article(
                "a3",
                "economic",
                "de",
                "Oil prices rise on supply fears",
                "Markets reacted overnight",
                0.5,
            ),
        ];
        let mut claims = vec![kept(
            "yemen forces",
            "strike",
            "houthi targets",
            "military",
            "a1",
            "us",
            false,
        )];
        claims[0].classification = "inference".into();
        claims[0].confidence = 0.5;
        let related = related_articles(&claims[0], &articles);
        assert!(related.iter().any(|article| article.id == "a2"));
        assert!(!related.iter().any(|article| article.id == "a3"));
        apply_peer_support(&mut claims, &articles, &[]);
        assert_eq!(claims[0].classification, "fact");
        assert!(claims[0].confidence > 0.5);
    }

    #[test]
    fn classifier_peers_are_preferred_over_token_overlap() {
        let articles = vec![
            article(
                "a1",
                "military",
                "us",
                "Yemen forces strike houthi targets",
                "Overnight raids continued",
                1.0,
            ),
            article(
                "a2",
                "military",
                "gb",
                "Yemen forces strike houthi targets again",
                "A second outlet confirms the raids",
                0.8,
            ),
            article(
                "a3",
                "military",
                "de",
                "Yemen forces strike houthi targets in Red Sea",
                "A third desk carries the same elements",
                0.7,
            ),
        ];
        let mut claims = vec![kept(
            "yemen forces",
            "strike",
            "houthi targets",
            "military",
            "a1",
            "us",
            false,
        )];
        claims[0].classification = "inference".into();
        claims[0].confidence = 0.5;
        let peers = vec![PeerMatch {
            claim_index: 0,
            article_ids: vec!["a3".into()],
        }];
        apply_peer_support(&mut claims, &articles, &peers);
        assert_eq!(claims[0].classification, "fact");
        assert!(claims[0].confidence > 0.5);
    }

    #[test]
    fn peer_match_json_keeps_valid_cycle_ids() {
        let articles = vec![
            article("a1", "military", "us", "One", "", 1.0),
            article("a2", "military", "gb", "Two", "", 0.8),
            article("a3", "economic", "de", "Three", "", 0.5),
        ];
        let claims = vec![kept(
            "yemen forces",
            "strike",
            "houthi targets",
            "military",
            "a1",
            "us",
            false,
        )];
        let matches = parse_peer_matches(
            r#"{"matches":[{"index":0,"article_ids":["a1","a2","missing","a3"]}]}"#,
            &claims,
            &articles,
        );
        assert_eq!(
            matches,
            vec![PeerMatch {
                claim_index: 0,
                article_ids: vec!["a2".into(), "a3".into()],
            }]
        );
    }

    #[test]
    fn peer_decisions_offer_candidate_ids_or_none() {
        let claim = kept(
            "yemen forces",
            "strike",
            "houthi targets",
            "military",
            "a1",
            "us",
            false,
        );
        let articles = [
            article("a2", "military", "gb", "Peer title", "body", 0.8),
            article("a3", "military", "de", "Other peer", "body", 0.7),
        ];
        let candidates: Vec<&AtlasArticleRow> = articles.iter().collect();
        let (state, questions) = peer_decisions_request(&claim, &candidates);
        assert_eq!(state["entity"], "yemen forces");
        assert_eq!(state["source_article_id"], "a1");
        let criteria = &questions["peer"]["criteria"];
        assert!(criteria.get("a2").is_some());
        assert!(criteria.get("a3").is_some());
        assert!(criteria.get("none").is_some());
        assert!(criteria.get("a1").is_none());
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
    fn the_highest_confidence_claims_survive_a_budget() {
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
        let capped = cap_claims(claims, 5);
        assert_eq!(capped.len(), 5);
        assert!(capped.iter().all(|claim| claim.entity != "person0"));
    }

    #[test]
    fn lead_claims_scale_with_the_gate_and_context_is_not_capped_at_five() {
        let lead: Vec<_> = (0..6)
            .map(|index| {
                kept(
                    &format!("person{index}"),
                    "met",
                    "union",
                    "stability",
                    "art",
                    "fr",
                    false,
                )
            })
            .collect();
        let settled = finish_insights(6, 0, &[], lead, &[], &[]);
        assert_eq!(settled.stats.claims, 6);
        let lead = vec![kept(
            "putin",
            "visited",
            "paris",
            "geopolitical",
            "art-lead",
            "fr",
            false,
        )];
        let unk: Vec<_> = (0..6)
            .map(|index| {
                article(
                    &format!("unk{index}"),
                    "unk",
                    "fr",
                    &format!("Putin visited port{index}"),
                    "",
                    0.2,
                )
            })
            .collect();
        let raw: Vec<_> = (0..6)
            .map(|index| {
                raw(
                    "Putin",
                    "person",
                    "visited",
                    &format!("port{index}"),
                    "unk",
                    &format!("Putin visited port{index}."),
                    "inference",
                    0.4,
                    &format!("unk{index}"),
                )
            })
            .collect();
        let settled = finish_insights(1, 0, &[], lead, &unk, &raw);
        assert!(settled.stats.context > 5, "{}", settled.stats.context);
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
    fn body_spans_accept_entity_and_object_from_full_article() {
        let headline = article(
            "art-body",
            "geopolitical",
            "us",
            "Sanctions widen",
            "New measures announced today.",
            1.0,
        );
        let body = "The Treasury Department sanctioned Acme Shipping over shadow fleet activity.";
        let enriched = article_with_body_spans(&headline, body);
        let (kept, dropped) = accept_claims(
            &[enriched],
            &[raw(
                "Acme Shipping",
                "org",
                "sanctioned",
                "shadow fleet",
                "geopolitical",
                "Acme Shipping sanctioned shadow fleet activity.",
                "inference",
                0.75,
                "art-body",
            )],
            AcceptMode::Lead,
        );
        assert_eq!(dropped, 0);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].entity, "acme shipping");
        assert_eq!(kept[0].object, "shadow fleet");
        assert_eq!(kept[0].classification, "inference");
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
        let mut packet = vec![article(
            "art-9",
            "military",
            "us",
            "Putin sanctioned Acme",
            "over oil",
            1.0,
        )];
        packet[0].published_at = "2026-10-01T12:00:00Z".into();
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
        let brief = memories
            .iter()
            .find(|memory| memory.text.contains('\n'))
            .unwrap();
        assert!(store.graph_summary(&brief.id).unwrap().is_none());
        let source: (String, String, String, String) = store
            .conn
            .query_row(
                "SELECT call_id, source_url, answer_id, published_at FROM insight_sources WHERE run_id=?1",
                [run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(source.0, "art-9");
        assert_eq!(source.1, "https://example.com/art-9");
        assert_eq!(source.2, "atlas-atlas-cycle-1");
        assert_eq!(source.3, "2026-10-01T12:00:00Z");
        let hits = store.recall("Putin sanctioned Acme", 4).unwrap();
        assert!(hits
            .iter()
            .any(|hit| hit.memory.text.contains("Putin sanctioned Acme")));
        assert!(store.atlas_has_insights(run_id).unwrap());
    }

    #[test]
    fn claim_json_tolerates_empty_and_alternate_shapes() {
        assert!(parse_claims("").unwrap().is_empty());
        assert!(parse_claims("{}").unwrap().is_empty());
        assert!(parse_claims(r#"{"claims":null}"#).unwrap().is_empty());
        assert!(parse_claims("no claims this time").unwrap().is_empty());
        let bare = parse_claims(
            r#"[{"entity":"Cabinet","namespace":"org","predicate":"met","object":"union","topic":"stability","claim":"Cabinet met the union.","classification":"inference","confidence":0.7,"evidence_ids":["a1"]}]"#,
        )
        .unwrap();
        assert_eq!(bare.len(), 1);
        assert_eq!(bare[0].entity, "Cabinet");
        let single = parse_claims(
            r#"{"entity":"Cabinet","namespace":"org","predicate":"met","object":"union","topic":"stability","claim":"Cabinet met the union.","classification":"inference","confidence":0.7,"evidence_ids":["a1"]}"#,
        )
        .unwrap();
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].object, "union");
        let err = parse_claims(r#"{"claims":"oops"}"#).unwrap_err();
        assert!(err.to_string().contains("claims array missing"));
    }

    #[tokio::test]
    async fn a_decisions_model_does_not_extract_claims() {
        let secret = crate::secrets::ProviderSecret {
            kind: "openrouter".into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            model: "typesafe/jev-1.3".into(),
            api_key: Some("test".into()),
            stt_model: None,
            device: None,
        };
        let err = match extract(&secret, None, &[], &[], |_, _| {}).await {
            Ok(_) => panic!("a decisions model extracted claims"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("decisions model"));
        assert!(err.to_string().contains("synthesis"));
    }
}
