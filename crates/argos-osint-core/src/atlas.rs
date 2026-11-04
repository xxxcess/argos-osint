//! @tag:atlas-collection
//! Atlas collects regional headlines, admits them by taxonomy, and stores
//! the run's statistics.
//!
//! Phase 1 reads the latest headlines. Phase 2 headlines are staged into the
//! bounded private `atlas_admission` table and evaluated by the Atlas
//! evaluator in ranked multi-category batches. Admission writes the canonical
//! article with ordered categories; a confirmed unknown is discarded with a
//! minimal rejection receipt. Local entity and publisher metadata builds
//! tentative Related article leads; cycles never publish Brain memories. The
//! database keeps the admitted articles and the daily request counts.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::atlas_insights::{self, InsightStats};
use crate::atlas_memory;
use crate::atlas_store::AtlasStore;
use crate::osint::atlas_news::{self, Hit};
use crate::osint::ProviderKeys;
use crate::provider::DecisionsResponse;
use crate::secrets::ProviderSecret;
use crate::store::{AtlasArticleRow, AtlasRunRow};
use crate::telemetry::{self, EventKind, TelemetryEvent, Trigger};

/// Country-coded headlines collected from each phase-1 source.
const PHASE1_TARGET: u32 = 50;
/// Pages per source. Ten articles a page, so five pages cover the target.
const PHASE1_MAX_PAGES: usize = 5;

const CLUSTERS: &[(&str, &[&str])] = &[
    (
        "geopolitical",
        &[
            "joint statement",
            "strategic partnership",
            "memorandum of understanding",
            "bilateral agreement",
            "multilateral pact",
            "diplomatic alignment",
            "voting bloc",
            "treaty ratification",
            "security guarantee",
            "envoy recalled",
            "diplomat expelled",
            "consulate closed",
            "sanctions package",
            "diplomatic retaliation",
            "sovereignty dispute",
            "demarche",
            "official protest",
            "border dispute",
        ],
    ),
    (
        "economic",
        &[
            "critical minerals",
            "semiconductor supply",
            "rare earth elements",
            "lithium production",
            "cobalt sourcing",
            "export restriction",
            "export embargo",
            "export quota",
            "chokepoint vulnerability",
            "de-dollarization",
            "local currency settlement",
            "bilateral swap line",
            "currency decoupling",
            "financial sanctions",
            "SWIFT alternative",
            "asset freeze",
            "central bank reserves",
            "energy security",
            "LNG terminal",
            "pipeline disruption",
            "trade asymmetry",
            "strategic tariff",
            "protectionism",
            "maritime chokepoint",
            "freight route disruption",
        ],
    ),
    (
        "military",
        &[
            "joint military exercise",
            "live-fire drill",
            "troop deployment",
            "combat readiness",
            "troop mobilization",
            "border buildup",
            "naval transit",
            "airspace intrusion",
            "freedom of navigation",
            "defense budget",
            "arms sale",
            "military procurement",
            "hypersonic missile",
            "deep-water port",
            "naval base expansion",
            "air defense system",
            "dual-use infrastructure",
            "satellite reconnaissance",
        ],
    ),
    (
        "science",
        &[
            "scientific breakthrough",
            "research breakthrough",
            "space exploration",
            "space mission",
            "satellite launch",
            "academic research",
            "university study",
            "research paper",
            "climate change",
            "global warming",
            "extreme weather study",
            "conservation effort",
            "endangered species",
            "wildlife protection",
            "environmental policy",
            "emissions regulation",
            "pollution control",
        ],
    ),
    (
        "health",
        &[
            "public health",
            "disease outbreak",
            "epidemiological update",
            "medical discovery",
            "clinical trial",
            "drug approval",
            "pharmaceutical regulation",
            "drug pricing",
            "mental health research",
            "psychiatric care study",
            "wellness guide",
            "preventive care guidance",
        ],
    ),
    (
        "stability",
        &[
            "civil unrest",
            "mass protest",
            "general strike",
            "curfew declared",
            "anti-government demonstration",
            "food rationing",
            "hyperinflation",
            "energy blackouts",
            "labor shortage",
            "succession plan",
            "political factionalism",
            "cabinet reshuffle",
            "anti-corruption purge",
            "regime fragility",
            "martial law",
            "snap election",
            "constitutional amendment",
        ],
    ),
    (
        "technology",
        &[
            "6G standards",
            "AI governance",
            "biotech protocol",
            "quantum computing regulation",
            "technology decoupling",
            "intellectual property theft",
            "tech controls",
            "cyber reconnaissance",
            "industrial control system",
            "power grid vulnerability",
            "undersea cable",
            "ransomware attack",
            "critical infrastructure breach",
            "firmware exploit",
        ],
    ),
];

pub const CATEGORY_IDS: &[&str] = &[
    "geopolitical",
    "economic",
    "military",
    "science",
    "health",
    "stability",
    "technology",
    "unk",
];

/// Version of the admission taxonomy. A rejection receipt is reconsidered
/// when the input hash or this version changes.
pub const TAXONOMY_VERSION: &str = "atlas-taxonomy-v2";
/// Admitted articles carry at least this many distinct categories, ranked
/// most likely first.
pub const MIN_ADMISSION_TAGS: usize = 2;
/// Hard ceiling on stored categories; never padded with weak tags.
pub const MAX_ADMISSION_TAGS: usize = 7;
/// Ranked category picks requested per article — one question per rank, so
/// the evaluator can justify up to [`MAX_ADMISSION_TAGS`] tags. Ranks beyond
/// the first are answered `done` when no further category fits, so tags are
/// never padded.
pub const RANKED_PICKS: usize = MAX_ADMISSION_TAGS;

/// Short tag stored on the article and shown in the news list. Retired
/// labels (for example the former `information`) read as `unk` until the
/// resumable migration reclassifies the row by subject.
pub fn category_tag(id: &str) -> &'static str {
    match id {
        "geopolitical" => "geopolitical",
        "economic" => "economic",
        "military" => "military",
        "science" => "science",
        "health" => "health",
        "stability" => "stability",
        "technology" => "technology",
        "ambiguous" => "ambiguous",
        "out_of_scope" => "out_of_scope",
        "failed" => "failed",
        _ => "unk",
    }
}

/// Full OSINT category name for the article card.
pub fn category_name(id: &str) -> &'static str {
    match id {
        "geopolitical" => "Geopolitical & Diplomatic",
        "economic" => "Economic & Resource",
        "military" => "Military Posture",
        "science" => "Science & Environment",
        "health" => "Health & Medicine",
        "stability" => "Domestic Stability",
        "technology" => "Technological Vectors",
        _ => "unk",
    }
}

/// Compact per-category glosses for the second and later ranked picks; the
/// model has the article in state and the rank-1 criteria carry the full
/// signal lists.
fn additional_criteria() -> serde_json::Value {
    let mut criteria = serde_json::Map::new();
    for id in CATEGORY_IDS {
        if *id == "unk" {
            continue;
        }
        criteria.insert(
            (*id).to_string(),
            serde_json::json!(format!("{} also fits this article.", category_name(id))),
        );
    }
    criteria.insert(
        "done".into(),
        serde_json::json!("No further category fits this article."),
    );
    serde_json::Value::Object(criteria)
}

/// One ranked Decisions choice set. Rank 1 carries the category criteria and
/// `unk` as the miss; ranks 2..=[`RANKED_PICKS`] ask for the next fitting
/// category and stop with `done`, so tags are never padded.
pub fn classification_request(article: &FeedArticle) -> (serde_json::Value, serde_json::Value) {
    let mut primary = serde_json::Map::new();
    for (id, phrases) in CLUSTERS {
        primary.insert(
            (*id).to_string(),
            serde_json::json!(format!(
                "{} Signals include: {}.",
                category_name(id),
                phrases.join(", ")
            )),
        );
    }
    primary.insert(
        "unk".into(),
        serde_json::json!("The article does not fit any of the other categories."),
    );
    let rank1 = serde_json::json!({
        "type": "choice",
        "instructions": "Choose the single OSINT category this article fits best from its title and description. Choose unk when no category fits.",
        "criteria": primary,
    });
    let mut questions = serde_json::Map::new();
    questions.insert("category".into(), rank1);
    let additional = serde_json::json!({
        "type": "choice",
        "instructions": "Choose the category that fits this article next best, different from the categories already chosen. Choose done when no further category fits.",
        "criteria": additional_criteria(),
    });
    for rank in 2..=RANKED_PICKS {
        questions.insert(format!("category_{rank}"), additional.clone());
    }
    let questions = serde_json::Value::Object(questions);
    let state = serde_json::json!({
        "title": article.title,
        "description": article.description,
        "source": article.source_name,
        "country": article.country,
    });
    (state, questions)
}

/// Question id of one ranked pick for an article in a batch.
pub fn rank_question_id(article_id: &str, rank: usize) -> String {
    format!("{article_id}#category_{rank}")
}

/// The categories a ranked response admits, or `None` when the evaluator
/// reported fewer than [`MIN_ADMISSION_TAGS`] fitting categories.
pub fn categories_from_decisions(
    response: &DecisionsResponse,
    article_id: &str,
) -> Result<Option<Vec<String>>, String> {
    let rank = |n: usize| -> Result<String, String> {
        response
            .answers
            .get(&rank_question_id(article_id, n))
            .and_then(|answer| answer.choice.clone())
            .ok_or_else(|| format!("missing ranked choice for `{article_id}` rank {n}"))
    };
    let first = rank(1)?;
    if first == "unk" {
        // Mixed tags plus unk is a retryable validation error, not unknown.
        for n in 2..=RANKED_PICKS {
            let label = rank(n)?;
            if label != "done" {
                return Err(format!("`{article_id}` mixes unk with category `{label}`"));
            }
        }
        return Ok(None);
    }
    let mut tags = vec![normalize_category(&first)];
    if tags[0] == "unk" {
        return Err(format!("`{article_id}` picked invalid category `{first}`"));
    }
    let mut done = false;
    for n in 2..=RANKED_PICKS {
        let raw = rank(n)?;
        if raw == "done" {
            done = true;
            continue;
        }
        if done {
            return Err(format!(
                "`{article_id}` picked `{raw}` after done at rank {n}"
            ));
        }
        let category = normalize_category(&raw);
        if category == "unk" {
            return Err(format!("`{article_id}` picked invalid category `{raw}`"));
        }
        if tags.contains(&category) {
            return Err(format!("`{article_id}` picked duplicate category `{raw}`"));
        }
        tags.push(category);
    }
    if tags.len() > MAX_ADMISSION_TAGS {
        return Err(format!(
            "`{article_id}` picked {} categories, over the {MAX_ADMISSION_TAGS} cap",
            tags.len()
        ));
    }
    if tags.len() < MIN_ADMISSION_TAGS {
        return Ok(None);
    }
    Ok(Some(tags))
}

pub fn normalize_category(raw: &str) -> String {
    let id = raw.trim().to_ascii_lowercase();
    let id = id.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_');
    if CATEGORY_IDS.contains(&id) {
        id.to_string()
    } else {
        "unk".into()
    }
}

const DOMAIN_RANK: &[(&str, i32)] = &[
    ("reuters.com", 100),
    ("apnews.com", 96),
    ("afp.com", 94),
    ("bbc.co.uk", 90),
    ("bbc.com", 90),
    ("theguardian.com", 84),
    ("nytimes.com", 82),
    ("washingtonpost.com", 80),
    ("wsj.com", 80),
    ("ft.com", 78),
    ("bloomberg.com", 78),
    ("aljazeera.com", 74),
    ("cnn.com", 70),
    ("economist.com", 70),
    ("nikkei.com", 66),
    ("scmp.com", 64),
];

pub fn daily_cap(provider: &str) -> u32 {
    let provider = provider.split(':').next().unwrap_or(provider);
    match provider {
        "gnews" => 100,
        "newsdata" => 200,
        "newsapi" => 100,
        "currents" => 250,
        _ => 0,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Band {
    pub country: String,
    pub count: u32,
    pub tier: u8,
    pub temperature: f64,
}

/// 20% / 30% / 30% / 20%, rounded so the four sizes sum to `n`. One country is Group 1.
pub fn band_sizes(n: usize) -> (usize, usize, usize, usize) {
    if n == 0 {
        return (0, 0, 0, 0);
    }
    if n == 1 {
        return (1, 0, 0, 0);
    }
    let mut sizes = [
        (n as f64 * 0.20).round() as usize,
        (n as f64 * 0.30).round() as usize,
        (n as f64 * 0.30).round() as usize,
        (n as f64 * 0.20).round() as usize,
    ];
    let sum: usize = sizes.iter().sum();
    if sum > n {
        let mut extra = sum - n;
        for size in sizes.iter_mut().rev() {
            let take = extra.min(*size);
            *size -= take;
            extra -= take;
        }
    } else if sum < n {
        sizes[0] += n - sum;
    }
    if sizes[0] == 0 {
        for index in 1..4 {
            if sizes[index] > 0 {
                sizes[index] -= 1;
                sizes[0] = 1;
                break;
            }
        }
    }
    (sizes[0], sizes[1], sizes[2], sizes[3])
}

fn band_temperature(tier: u8, index: usize, len: usize) -> f64 {
    let (hot, cool) = match tier {
        1 => return 1.0,
        2 => (0.99, 0.70),
        _ => (0.69, 0.10),
    };
    if len <= 1 {
        return hot;
    }
    let step = index as f64 / (len - 1) as f64;
    hot + (cool - hot) * step
}

/// Countries sorted by volume, then code. Group 4 is omitted.
pub fn score_counts(counts: &BTreeMap<String, u32>) -> Vec<Band> {
    let mut ranked: Vec<(String, u32)> = counts
        .iter()
        .filter(|(_, count)| **count > 0)
        .map(|(country, count)| (country.clone(), *count))
        .collect();
    ranked.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let (g1, g2, g3, _) = band_sizes(ranked.len());
    let mut bands = Vec::new();
    for (offset, (country, count)) in ranked.into_iter().enumerate() {
        let (tier, index, len) = if offset < g1 {
            (1, offset, g1)
        } else if offset < g1 + g2 {
            (2, offset - g1, g2)
        } else if offset < g1 + g2 + g3 {
            (3, offset - g1 - g2, g3)
        } else {
            continue;
        };
        bands.push(Band {
            country,
            count,
            tier,
            temperature: band_temperature(tier, index, len),
        });
    }
    bands
}

pub fn normalize_title(title: &str) -> String {
    title
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

pub fn canonical_url(raw: &str) -> String {
    let Ok(mut url) = Url::parse(raw.trim()) else {
        return raw.trim().to_ascii_lowercase();
    };
    if let Some(host) = url.host_str() {
        let host = host.to_ascii_lowercase();
        let _ = url.set_host(Some(&host));
    }
    url.set_fragment(None);
    let mut text = url.to_string();
    if text.ends_with('/') && url.path() == "/" {
        text.pop();
    }
    text
}

pub fn url_hash(raw: &str) -> String {
    let digest = Sha256::digest(canonical_url(raw).as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn host_of(raw: &str) -> String {
    Url::parse(raw)
        .ok()
        .and_then(|url| {
            url.host_str()
                .map(|host| host.trim_start_matches("www.").to_ascii_lowercase())
        })
        .unwrap_or_default()
}

pub fn domain_rank(host: &str) -> i32 {
    let host = host.trim_start_matches("www.");
    DOMAIN_RANK
        .iter()
        .find(|(domain, _)| host == *domain || host.ends_with(&format!(".{domain}")))
        .map(|(_, rank)| *rank)
        .unwrap_or(0)
}

/// Version of the origin scoring method (bands, tiers, temperatures). Two
/// origin snapshots are only comparable when this value matches.
pub const ORIGIN_SCORING_VERSION: &str = "atlas-origin-v1";

/// Milliseconds between two RFC 3339 stamps. `None` when either is missing or
/// unparsable, so a missing timestamp is N/A rather than zero.
fn rfc_ms(from: &str, to: &str) -> Option<i64> {
    let parse = |raw: &str| {
        chrono::DateTime::parse_from_rfc3339(raw.trim())
            .ok()
            .map(|stamp| stamp.with_timezone(&chrono::Utc))
    };
    let start = parse(from)?;
    let end = parse(to)?;
    Some((end - start).num_milliseconds())
}

/// One open Atlas stage interval, closed on the next transition or at cycle end.
struct OpenStage {
    name: &'static str,
    unit: &'static str,
    seq: i64,
    started_at: String,
}

/// Telemetry for one Atlas cycle.
///
/// Every write is best-effort: a failure is dropped here so instrumentation can
/// never break a cycle. Event ids are the logical key of the fact they describe,
/// so a resumed or replayed cycle replaces a row instead of double counting it.
struct CycleTelemetry {
    db: std::path::PathBuf,
    run_id: String,
    mode: String,
    started_at: String,
    clock: telemetry::Measured,
    candidates: i64,
    stage: Option<OpenStage>,
    /// Under SurrealDB: rows already written per stage, loaded once by
    /// [`CycleTelemetry::open`] and advanced locally (records are written
    /// asynchronously, so re-counting per stage would race them).
    stage_seqs: Option<std::collections::HashMap<String, i64>>,
}

impl CycleTelemetry {
    fn new(db: &Path, run_id: &str, mode: &str, started_at: String) -> Self {
        // Continue the occurrence numbering so a resumed cycle appends.
        // Legacy store only without a storage config: under SurrealDB the count
        // starts at 0 rather than reopening the legacy store.
        // [`CycleTelemetry::open`] continues the numbering from the store.
        let candidates = 0;
        Self {
            db: db.to_path_buf(),
            run_id: run_id.to_string(),
            mode: mode.to_string(),
            started_at,
            clock: telemetry::Measured::start(),
            candidates,
            stage: None,
            stage_seqs: None,
        }
    }

    /// [`CycleTelemetry::new`] on the selected store: under SurrealDB the
    /// candidate and stage numbering continue from the repository, so a
    /// resumed cycle appends instead of overwriting earlier ids.
    async fn open(db: &Path, run_id: &str, mode: &str, started_at: String) -> Self {
        let mut tel = Self::new(db, run_id, mode, started_at);
        #[cfg(feature = "surreal")]
        if crate::recon::backend::legacy_side_store_path(db).is_none() && !run_id.is_empty() {
            let mut seqs = std::collections::HashMap::new();
            if let Ok(repo) = crate::storage::runtime::repository().await {
                tel.candidates = crate::storage::telemetry::count_ids_with_prefix(
                    repo.db(),
                    &format!("atlas-cand-{run_id}-"),
                )
                .await
                .unwrap_or(0);
                let prefix = format!("atlas-stage-{run_id}-");
                for id in crate::storage::telemetry::ids_with_prefix(repo.db(), &prefix)
                    .await
                    .unwrap_or_default()
                {
                    if let Some((stage, _)) = id
                        .strip_prefix(&prefix)
                        .and_then(|rest| rest.rsplit_once('-'))
                    {
                        *seqs.entry(stage.to_string()).or_insert(0) += 1;
                    }
                }
            }
            tel.stage_seqs = Some(seqs);
        }
        tel
    }

    /// Opens a stage, closing the previous interval with the counts known now.
    /// A stage a resumed cursor skips simply never opens, so it is never reported
    /// as zero work.
    fn enter_stage(&mut self, name: &'static str, unit: &'static str, stats: &RunStats) {
        if self.stage.as_ref().map(|open| open.name) == Some(name) {
            return;
        }
        self.close_stage("completed", stats);
        let seq = match self.stage_seqs.as_mut() {
            Some(seqs) => {
                let n = seqs.entry(name.to_string()).or_insert(0);
                let seq = *n;
                *n += 1;
                seq
            }
            None => 0,
        };
        self.stage = Some(OpenStage {
            name,
            unit,
            seq,
            started_at: chrono::Utc::now().to_rfc3339(),
        });
    }

    /// Closes the open interval. `state` is `completed` or `interrupted` so a
    /// stage a pause cut short is not reported as finished work.
    fn close_stage(&mut self, state: &str, stats: &RunStats) {
        let Some(open) = self.stage.take() else {
            return;
        };
        let ended_at = chrono::Utc::now().to_rfc3339();
        let duration = rfc_ms(&open.started_at, &ended_at);
        let event = TelemetryEvent::new(EventKind::AtlasStage)
            .at(ended_at.clone())
            .trigger(Trigger::AtlasCycle)
            .app("atlas")
            .run(&self.run_id)
            .tool(open.name)
            .call("cycle")
            .category(open.unit)
            .outcome(state)
            .duration_ms(duration)
            .count(1)
            .payload(serde_json::json!({
                "stage": open.name,
                "unit": open.unit,
                "state": state,
                "unit_id": "cycle",
                "started_at": open.started_at,
                "ended_at": ended_at,
                "error_category": if state == "interrupted" { "interrupted" } else { "none" },
                "counts": stage_counts(stats),
            }));
        let event = event.with_id(format!(
            "atlas-stage-{}-{}-{}",
            self.run_id, open.name, open.seq
        ));
        let _ = telemetry::record(&self.db, &event);
    }

    /// One candidate article occurrence, with exactly one mutually exclusive
    /// disposition. A repeated headline is never recounted as new volume.
    fn candidate(
        &mut self,
        article: &FeedArticle,
        provider: &str,
        disposition: &str,
        replaced: Option<&str>,
        first_seen: Option<&str>,
    ) {
        self.candidates += 1;
        let event = TelemetryEvent::new(EventKind::AtlasCandidate)
            .at(if article.seen_at.trim().is_empty() {
                chrono::Utc::now().to_rfc3339()
            } else {
                article.seen_at.trim().to_string()
            })
            .trigger(Trigger::AtlasCycle)
            .app("atlas")
            .run(&self.run_id)
            .article(&article.id)
            .origin(&article.country)
            .provider(provider)
            .category(category_tag(&article.category))
            .outcome(disposition)
            .count(1)
            .payload(serde_json::json!({
                "disposition": disposition,
                "source_domain": article.source_domain,
                "url_hash": crate::atlas::url_hash(&article.url),
                "replaced": replaced.unwrap_or_default(),
                "first_seen_at": first_seen.unwrap_or_default(),
                "tag": category_tag(&article.category),
                "temperature": article.temperature,
            }));
        let event = event.with_id(format!(
            "atlas-cand-{}-{}-{}",
            self.run_id, article.id, self.candidates
        ));
        let _ = telemetry::record(&self.db, &event);
    }

    /// One origin observation in the cycle snapshot, never one per repaint.
    /// `comparable` marks whether the snapshot may be diffed against others.
    fn origin_snapshot(&self, origins: &[OriginStat], snapshot_at: &str, comparable: bool) {
        for row in origins {
            let event = TelemetryEvent::new(EventKind::AtlasOriginSnapshot)
                .at(snapshot_at.to_string())
                .trigger(Trigger::AtlasCycle)
                .app("atlas")
                .run(&self.run_id)
                .origin(&row.country)
                .outcome(if comparable { "scored" } else { "incomparable" })
                .count(i64::from(row.articles))
                .payload(serde_json::json!({
                    "tier": row.tier,
                    "temperature": row.temperature,
                    "volume": row.volume,
                    "articles": row.articles,
                    "version": ORIGIN_SCORING_VERSION,
                    "scoring_version": ORIGIN_SCORING_VERSION,
                    "eligible": comparable,
                    "comparable": comparable,
                    "snapshot_at": snapshot_at,
                }));
            let event = event.with_id(format!("atlas-origin-{}-{}", self.run_id, row.country));
            let _ = telemetry::record(&self.db, &event);
        }
    }

    /// Mean queue residence for this cycle's registry job, when the job recorded
    /// both a queue and an admission timestamp. Never derived from retry waits.
    fn queue_wait_ms(&self) -> Option<i64> {
        // The registry is the repository `job` table; it records no
        // admission timestamp, so there is no queue wait to report.
        None
    }

    /// The terminal cycle row. The event id is the run key, so a cycle that is
    /// finished again (indexing caught up later) replaces its row exactly once.
    fn cycle(&self, outcome: &str, stats: &RunStats) {
        let finished_at = chrono::Utc::now().to_rfc3339();
        let active_ms = self.clock.elapsed_ms();
        // A healthy cycle carries no reason; anything else carries a coarse
        // class, never the stored lifecycle prose.
        let reason = match outcome {
            "completed" => "none",
            "waiting" => "incomplete_work",
            "blocked" => "configuration",
            "failed" => "classification_failed",
            _ => "unknown",
        };
        let event = TelemetryEvent::new(EventKind::AtlasCycle)
            .at(finished_at.clone())
            .trigger(Trigger::AtlasCycle)
            .app("atlas")
            .run(&self.run_id)
            .mode(&self.mode)
            .outcome(outcome)
            .reason(reason)
            .duration_ms(Some(active_ms))
            .count(1)
            .payload(serde_json::json!({
                "outcome": outcome,
                "mode": self.mode,
                "started_at": self.started_at,
                "finished_at": finished_at,
                "wall_ms": rfc_ms(&self.started_at, &finished_at),
                "active_ms": active_ms,
                "queue_ms": self.queue_wait_ms(),
                "discovered": stats.counts.values().sum::<u32>(),
                "retained": stats.article_total(),
                "candidates": self.candidates,
                "origins": stats.origins.len(),
                "scoring_version": ORIGIN_SCORING_VERSION,
                "contract_version": stats.contract_version,
                "coverage": stats.coverage,
            }));
        let event = event.with_id(format!("atlas-cycle-{}", self.run_id));
        let _ = telemetry::record(&self.db, &event);
    }
}

/// Counts reported on every stage row. Articles, packets and memories stay in
/// their own fields and are never summed into one total.
fn stage_counts(stats: &RunStats) -> serde_json::Value {
    serde_json::json!({
        "discovered": stats.counts.values().sum::<u32>(),
        "articles": stats.article_total(),
        "origins": stats.origins.len(),
        "classified": stats.coverage.classified,
        "entities_available": stats.coverage.entities_available,
        "publishers_available": stats.coverage.publishers_available,
        "related_pairs": stats.coverage.related_pairs,
    })
}

#[derive(Clone, Debug)]
struct Seen {
    url_hash: String,
    title_norm: String,
    domain: String,
    rank: i32,
    published_at: String,
    id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Verdict {
    Keep,
    Drop,
    Replace(String),
}

fn judge(article: &FeedArticle, seen: &[Seen]) -> Verdict {
    let hash = url_hash(&article.url);
    let title = normalize_title(&article.title);
    if title.is_empty() || article.url.is_empty() {
        return Verdict::Drop;
    }
    if seen.iter().any(|item| item.url_hash == hash) {
        return Verdict::Drop;
    }
    if seen.iter().any(|item| item.title_norm == title) {
        return Verdict::Drop;
    }
    let rank = domain_rank(&article.source_domain);
    let mut replace: Option<&Seen> = None;
    for item in seen {
        if item.domain == article.source_domain || item.title_norm.is_empty() {
            continue;
        }
        let score = strsim::normalized_levenshtein(&title, &item.title_norm);
        if score <= 0.85 {
            continue;
        }
        let incoming_wins = rank > item.rank
            || (rank == item.rank
                && article.published_at < item.published_at
                && !article.published_at.is_empty());
        if incoming_wins {
            replace = Some(item);
        } else {
            return Verdict::Drop;
        }
    }
    match replace {
        Some(item) => Verdict::Replace(item.id.clone()),
        None => Verdict::Keep,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OriginStat {
    pub country: String,
    pub tier: u8,
    pub temperature: f64,
    pub volume: u32,
    pub articles: u32,
}

/// Current collection stages count article coverage independently.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CoverageStats {
    pub articles: u32,
    #[serde(default)]
    pub acquisition_failures: u32,
    pub classified: u32,
    #[serde(default)]
    pub legacy_unknown: u32,
    pub classification_failed: u32,
    pub classification_blocked: u32,
    pub entities_available: u32,
    pub entities_unavailable: u32,
    pub publishers_available: u32,
    pub publishers_unavailable: u32,
    pub related_pairs: u32,
    pub warnings: u32,
    /// Phase 6 (v2): occurrences with a current `atlas_article_index` receipt.
    #[serde(default)]
    pub actor_indexed: u32,
    /// Phase 6 occurrences indexed with zero bindable actors.
    #[serde(default)]
    pub actor_zero: u32,
    /// Phase 6 occurrences whose index write failed (no receipt).
    #[serde(default)]
    pub actor_index_failed: u32,
    /// Provider calls skipped because the provider's quota or rate limit was
    /// exhausted. Expected and daily-resetting, so not an acquisition failure.
    #[serde(default)]
    pub quota_skipped: u32,
    /// v2 admission, counted separately per spec §3: articles staged for
    /// evaluation this run (the fetched-and-deduplicated set).
    #[serde(default)]
    pub staged: u32,
    /// Admitted articles (canonical, ordered categories, indexed denominator).
    #[serde(default)]
    pub admitted: u32,
    /// Confirmed-unknown payloads discarded with a rejection receipt.
    #[serde(default)]
    pub discarded: u32,
    /// Staged articles awaiting evaluation (retryable, outages included).
    #[serde(default)]
    pub admission_pending: u32,
    /// Staged articles that exhausted this pass with a validation/transport
    /// error; their inputs stay privately retained in staging.
    #[serde(default)]
    pub admission_failed: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RunStats {
    #[serde(default)]
    pub contract_version: String,
    #[serde(default)]
    pub coverage: CoverageStats,
    #[serde(default)]
    pub acquisition_retry: Option<Cursor>,
    /// One line per acquisition failure (provider and cause), capped, so the
    /// status text can name what failed.
    #[serde(default)]
    pub acquisition_faults: Vec<String>,
    pub counts: BTreeMap<String, u32>,
    pub origins: Vec<OriginStat>,
    pub scored: bool,
    /// Span-checked claims written for this cycle. Absent on runs saved before insights.
    #[serde(default)]
    pub insights: InsightStats,
    /// Phase 4/5 memory lifecycle: extraction, publication and indexing tracked
    /// independently. Absent on runs saved before phase 5.
    #[serde(default)]
    pub memories: atlas_memory::MemoryPhase,
}

impl RunStats {
    pub fn share(&self, articles: u32) -> f64 {
        let total: u32 = self.origins.iter().map(|row| row.articles).sum();
        if total == 0 {
            0.0
        } else {
            f64::from(articles) / f64::from(total)
        }
    }

    /// Articles kept for this news cycle across every origin.
    pub fn article_total(&self) -> u32 {
        if crate::atlas_work::is_collection_contract(&self.contract_version) {
            self.coverage.articles
        } else {
            self.origins.iter().map(|row| row.articles).sum()
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Cursor {
    pub phase: u8,
    pub chunk: usize,
    pub leg: String,
    pub country: usize,
    pub from: String,
    /// NewsData `nextPage` token for the request this cursor points at.
    #[serde(default)]
    pub page_token: String,
    /// Country-coded hits already kept for the current phase-1 source.
    #[serde(default)]
    pub kept: u32,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            phase: 1,
            chunk: 0,
            leg: "gnews".into(),
            country: 0,
            from: String::new(),
            page_token: String::new(),
            kept: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeedArticle {
    pub id: String,
    pub title: String,
    pub description: String,
    pub url: String,
    pub country: String,
    pub source_name: String,
    pub source_domain: String,
    pub author: String,
    pub image_url: String,
    pub published_at: String,
    pub provider: String,
    pub temperature: f64,
    pub seen_at: String,
    /// OSINT category id, or `unk` until the classifier tags it.
    pub category: String,
}

#[derive(Clone, Debug)]
pub struct HttpCall {
    pub provider: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
pub struct HttpReply {
    pub status: u16,
    pub body: String,
    /// `Location` of a 3xx reply, so the dispatcher can follow same-host redirects.
    pub location: Option<String>,
}

/// A provider rejection. `summary` is the log line. `body` is the full response.
#[derive(Clone, Debug)]
pub struct ProviderFault {
    pub summary: String,
    pub body: String,
}

#[derive(Clone, Debug)]
pub enum AtlasEvent {
    Status(String),
    /// Insight extraction step progress (`done` of `total` work units).
    InsightProgress {
        done: u32,
        total: u32,
    },
    Stats(RunStats),
    Article(FeedArticle),
    Replaced {
        id: String,
        article: FeedArticle,
    },
    Classified {
        id: String,
        category: String,
    },
    Note(String),
    Fault(ProviderFault),
    /// Memories were durably published or repaired; Brain must refresh.
    MemoriesChanged {
        run_id: String,
        memory_ids: Vec<String>,
    },
    /// Phase 5 progress: required memories verified in the active index.
    MemoryProgress {
        indexed: u32,
        required: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    Paused,
    Finished,
    Failed(String),
}

#[derive(Clone, Debug)]
struct Job {
    phase: u8,
    chunk: usize,
    leg: &'static str,
    country: usize,
    provider: &'static str,
}

fn leg_order(leg: &str) -> u8 {
    match leg {
        "gnews" | "newsapi" => 0,
        _ => 1,
    }
}

fn job_is_before(job: &Job, cursor: &Cursor) -> bool {
    if job.phase < cursor.phase {
        return true;
    }
    if job.phase > cursor.phase {
        return false;
    }
    if job.phase == 1 {
        (job.chunk, leg_order(job.leg)) < (cursor.chunk, leg_order(&cursor.leg))
    } else {
        (job.country, leg_order(job.leg)) < (cursor.country, leg_order(&cursor.leg))
    }
}

fn cursor_at(job: &Job, from: &str) -> Cursor {
    Cursor {
        phase: job.phase,
        chunk: job.chunk,
        leg: job.leg.into(),
        country: job.country,
        from: from.into(),
        page_token: String::new(),
        kept: 0,
    }
}

/// `United States (US)`. Unknown codes keep a readable fallback.
pub fn country_label(code: &str) -> String {
    let code = code.trim().to_ascii_lowercase();
    let name = country_name(&code).unwrap_or("Unknown");
    format!("{name} ({})", code.to_ascii_uppercase())
}

pub fn country_name(code: &str) -> Option<&'static str> {
    crate::iso3166::name(code)
}

/// Headline card. Category is the OSINT classification. Published is the article time.
pub fn format_article_card(
    title: &str,
    source_name: &str,
    source_domain: &str,
    author: &str,
    country: &str,
    category: &str,
    published_at: &str,
    temperature: f64,
    description: &str,
    image_url: &str,
    url: &str,
) -> String {
    let (publisher, author) = publisher_and_author(source_name, source_domain, author);
    let published = friendly_date(published_at);
    let image = if image_url.trim().is_empty() {
        "none"
    } else {
        image_url.trim()
    };
    let mut lines = vec![
        format!("Title: {title}"),
        format!("Publisher: {publisher}"),
        format!("Author: {author}"),
        format!("Country: {}", country_label(country)),
        format!("Category: {}", category_name(category)),
        format!("Published: {published}"),
        format!("Temperature: {temperature:.2}"),
    ];
    if !description.trim().is_empty() {
        lines.push(String::new());
        lines.push(description.trim().to_string());
    }
    lines.push(String::new());
    lines.push(format!("Image: {image}"));
    lines.push(format!("Article: {url}"));
    lines.join("\n")
}

/// Publisher is the outlet. A personal name, or a semicolon byline left in
/// `source_name` by older Currents rows, is the author. The article host fills
/// the publisher when the outlet name is missing. When only one of the two
/// values is present, both fields use it.
pub fn publisher_and_author(source_name: &str, domain: &str, author: &str) -> (String, String) {
    let source_name = source_name.trim();
    let domain = domain.trim();
    let mut author = author.trim().to_string();
    let mut publisher = source_name.to_string();
    if author.is_empty() && publisher.contains(';') {
        author = publisher.clone();
        publisher = domain.to_string();
    } else if personal_name(&publisher) && !domain.is_empty() {
        if author.is_empty() {
            author = publisher.clone();
        }
        publisher = domain.to_string();
    }
    if publisher.is_empty() {
        publisher = if !domain.is_empty() {
            domain.to_string()
        } else {
            author.clone()
        };
    }
    if author.is_empty() {
        author = publisher.clone();
    }
    if publisher.is_empty() {
        publisher = "unknown".into();
    }
    if author.is_empty() {
        author = publisher.clone();
    }
    (publisher, author)
}

/// Two or three capitalized words, with no outlet token, is a byline.
fn personal_name(value: &str) -> bool {
    let words: Vec<&str> = value.split_whitespace().collect();
    if !(2..=3).contains(&words.len()) {
        return false;
    }
    const STOP: &[&str] = &[
        "the",
        "of",
        "and",
        "for",
        "news",
        "times",
        "post",
        "press",
        "herald",
        "tribune",
        "gazette",
        "journal",
        "daily",
        "review",
        "wire",
        "media",
        "radio",
        "reuters",
        "agency",
        "associated",
        "guardian",
        "bloomberg",
    ];
    words.iter().all(|word| {
        let bare = word.trim_matches(|c: char| matches!(c, ',' | '.' | '\'' | '’'));
        if bare.is_empty() || STOP.contains(&bare.to_ascii_lowercase().as_str()) {
            return false;
        }
        let mut chars = bare.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        first.is_uppercase() && chars.all(|c| c.is_alphabetic() || c == '-' || c == '\'')
    })
}

/// Same clock as `friendly_date`, from a unix timestamp.
pub fn friendly_unix(secs: u64) -> String {
    match chrono::DateTime::from_timestamp(secs as i64, 0) {
        Some(stamp) => friendly_date(&stamp.to_rfc3339()),
        None => "unknown".into(),
    }
}

/// Compact relative age from a stored timestamp (`3h ago`, `45m ago`, `2d ago`).
pub fn relative_ago(raw: &str) -> String {
    let Some(stamp) = parse_timestamp(raw.trim()) else {
        return "—".into();
    };
    let secs = (chrono::Utc::now() - stamp).num_seconds().max(0);
    if secs < 60 {
        return "just now".into();
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m ago");
    }
    let hours = mins / 60;
    if hours < 48 {
        return format!("{hours}h ago");
    }
    let days = hours / 24;
    format!("{days}d ago")
}

/// Local calendar day, plus a 12-hour clock when the value has a time.
/// A bare date stays a date. Unparsed text is returned as stored.
pub fn friendly_date(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return "unknown".into();
    }
    if !raw.contains(':') {
        if let Ok(date) = chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
            return loosen_date(&date.format("%b %d, %Y").to_string());
        }
    }
    parse_timestamp(raw)
        .map(|stamp| {
            loosen_date(
                &stamp
                    .with_timezone(&chrono::Local)
                    .format("%b %d, %Y, %I:%M %p")
                    .to_string(),
            )
        })
        .unwrap_or_else(|| raw.to_string())
}

/// Drops the leading zero on the day and on a 12-hour clock.
fn loosen_date(value: &str) -> String {
    value.replace(" 0", " ")
}

fn parse_timestamp(raw: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Ok(stamp) = chrono::DateTime::parse_from_rfc3339(raw) {
        return Some(stamp.with_timezone(&chrono::Utc));
    }
    if let Ok(stamp) = chrono::DateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S %z") {
        return Some(stamp.with_timezone(&chrono::Utc));
    }
    for format in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(stamp) = chrono::NaiveDateTime::parse_from_str(raw, format) {
            return Some(stamp.and_utc());
        }
    }
    chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|stamp| stamp.and_utc())
}

pub fn format_run_card(run: &AtlasRunRow) -> String {
    let stats = parse_stats(&run.stats_json);
    let mut lines = vec![
        format!("State: {}", run.state),
        format!("Started: {}", friendly_date(&run.started_at)),
    ];
    if !run.finished_at.is_empty() {
        lines.push(format!("Finished: {}", friendly_date(&run.finished_at)));
    }
    if !run.note.is_empty() {
        lines.push(run.note.clone());
    }
    lines.push(String::new());
    lines.push(format!("Articles {}", stats.article_total()));
    if crate::atlas_work::is_collection_contract(&stats.contract_version) {
        lines.push(format!(
            "Coverage · classified {}/{} · entities {} · publisher priors {} · Related {}",
            stats.coverage.classified,
            stats.coverage.articles,
            stats.coverage.entities_available,
            stats.coverage.publishers_available,
            stats.coverage.related_pairs
        ));
    } else {
        lines.push(format!(
            "Legacy · {}",
            atlas_insights::insight_stats_line(&stats.insights)
        ));
        if stats.memories.started() {
            lines.push(stats.memories.line());
        }
    }
    lines.push(String::new());
    let origins: Vec<&OriginStat> = stats
        .origins
        .iter()
        .filter(|row| row.articles > 0)
        .collect();
    if origins.is_empty() {
        lines.push("No country statistics for this run.".into());
    } else {
        lines.push("Country                         Tier   Temp   Volume  Articles  Share".into());
        for row in origins {
            lines.push(format!(
                "{:<32} {:<6} {:<6.2} {:<7} {:<9} {:.0}%",
                country_label(&row.country),
                row.tier,
                row.temperature,
                row.volume,
                row.articles,
                stats.share(row.articles) * 100.0
            ));
        }
    }
    lines.join("\n")
}

/// Drop flashpoints that never yielded a kept article after country collection.
pub fn prune_empty_origins(stats: &mut RunStats) {
    stats.origins.retain(|row| row.articles > 0);
}

fn parse_stats(raw: &str) -> RunStats {
    serde_json::from_str(raw).unwrap_or_default()
}

fn parse_cursor(raw: &str) -> Cursor {
    serde_json::from_str(raw).unwrap_or_default()
}

fn lookback_from() -> String {
    (chrono::Utc::now() - chrono::Duration::hours(48))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub(crate) fn utc_day() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

fn new_run_id() -> String {
    format!(
        "atlas-{}-{}",
        chrono::Utc::now().timestamp_millis(),
        std::process::id()
    )
}

fn article_from(hit: &Hit, provider: &str, temperature: f64) -> FeedArticle {
    let domain = host_of(&hit.url);
    let (publisher, author) = publisher_and_author(&hit.source_name, &domain, &hit.author);
    FeedArticle {
        id: format!("art-{}", url_hash(&hit.url)),
        title: hit.title.clone(),
        description: hit.description.clone(),
        url: hit.url.clone(),
        country: hit.country.clone(),
        source_name: publisher,
        source_domain: domain,
        author,
        image_url: hit.image_url.clone(),
        published_at: hit.published_at.clone(),
        provider: provider.into(),
        temperature,
        seen_at: chrono::Utc::now().to_rfc3339(),
        category: "unk".into(),
    }
}

fn seen_of(article: &FeedArticle) -> Seen {
    Seen {
        url_hash: url_hash(&article.url),
        title_norm: normalize_title(&article.title),
        domain: article.source_domain.clone(),
        rank: domain_rank(&article.source_domain),
        published_at: article.published_at.clone(),
        id: article.id.clone(),
    }
}

fn remember(seen: &mut Vec<Seen>, article: &FeedArticle, replaced: Option<&str>) {
    if let Some(id) = replaced {
        seen.retain(|item| item.id != id);
    }
    seen.push(seen_of(article));
}

type FetchFut = Pin<Box<dyn Future<Output = Result<HttpReply>> + Send>>;

struct CallSpec {
    provider: &'static str,
    url: String,
    headers: Vec<(String, String)>,
}

fn phase1_call(provider: &str, page: usize, page_token: &str, key: &str) -> Result<CallSpec> {
    let prepared = atlas_news::prepare_latest(provider, page, page_token, key)?;
    Ok(CallSpec {
        provider: if provider == "gnews" {
            "gnews"
        } else {
            "newsdata"
        },
        url: prepared.url,
        headers: prepared.headers,
    })
}

fn phase2_call(provider: &str, country: &str, key: &str) -> Result<CallSpec> {
    let prepared = atlas_news::prepare_headlines(provider, country, key)?;
    Ok(CallSpec {
        provider: if provider == "newsapi" {
            "newsapi"
        } else {
            "currents"
        },
        url: prepared.url,
        headers: prepared.headers,
    })
}

/// Primary account while its local daily cap is open and the key is not exhausted.
/// Otherwise the fallback account, when that cap is still open.
async fn open_key<'a>(
    store: &AtlasStore,
    provider: &str,
    primary: &'a str,
    fallback: &'a str,
) -> Result<Option<&'a str>> {
    let primary = primary.trim();
    let fallback = fallback.trim();
    if !primary.is_empty()
        && !crate::osint::key_exhausted(primary)
        && quota_open(store, provider).await?
    {
        return Ok(Some(primary));
    }
    if !fallback.is_empty()
        && fallback != primary
        && !crate::osint::key_exhausted(fallback)
        && quota_open(store, &format!("{provider}:fallback")).await?
    {
        return Ok(Some(fallback));
    }
    Ok(None)
}

fn quota_bucket(provider: &str, active: &str, primary: &str) -> String {
    if active.trim() == primary.trim() {
        provider.to_string()
    } else {
        format!("{provider}:fallback")
    }
}

fn quota_fault(fault: &ProviderFault) -> bool {
    crate::osint::quota_limited("failed", Some(&fault.summary))
}

struct SpareFetch {
    body: Option<serde_json::Value>,
    fault: Option<ProviderFault>,
    bucket: String,
}

/// One provider call. A rate or quota fault retries once with `spare` when it differs.
async fn fetch_with_spare_key<F>(
    fetch: &mut F,
    user_agent: &str,
    provider: &str,
    primary: &str,
    key: &str,
    spare: &str,
    mut spec_for: impl FnMut(&str) -> Result<CallSpec>,
) -> SpareFetch
where
    F: FnMut(HttpCall) -> FetchFut,
{
    let mut active = key.to_string();
    let mut tried_spare = false;
    loop {
        let spec = match spec_for(&active) {
            Ok(spec) => spec,
            Err(err) => {
                return SpareFetch {
                    body: None,
                    fault: Some(ProviderFault {
                        summary: one_line(&err.to_string()),
                        body: String::new(),
                    }),
                    bucket: quota_bucket(provider, &active, primary),
                };
            }
        };
        match dispatch(fetch, spec, user_agent).await {
            Ok(body) => {
                return SpareFetch {
                    body: Some(body),
                    fault: None,
                    bucket: quota_bucket(provider, &active, primary),
                };
            }
            Err(fault)
                if !tried_spare && quota_fault(&fault) && !spare.is_empty() && spare != active =>
            {
                crate::osint::note_key_exhausted(&active);
                active = spare.to_string();
                tried_spare = true;
                continue;
            }
            Err(fault) => {
                return SpareFetch {
                    body: None,
                    fault: Some(fault),
                    bucket: quota_bucket(provider, &active, primary),
                };
            }
        }
    }
}

async fn apply_hits(
    hits: &[Hit],
    provider: &str,
    stats: &mut RunStats,
    seen: &mut Vec<Seen>,
    emit: &mut impl FnMut(AtlasEvent),
    store: &AtlasStore,
    run_id: &str,
    tel: &mut CycleTelemetry,
) -> Result<()> {
    if provider == "gnews" || provider == "newsdata" {
        for hit in hits {
            *stats.counts.entry(hit.country.clone()).or_default() += 1;
        }
        if !stats.scored {
            stats.origins = stats
                .counts
                .iter()
                .map(|(country, volume)| OriginStat {
                    country: country.clone(),
                    tier: 0,
                    temperature: 0.0,
                    volume: *volume,
                    articles: 0,
                })
                .collect();
        }
        return Ok(());
    }
    // Already-staged and already-admitted rows for this run, with the
    // ingestion time first seen, so a repeated headline is reported as
    // retained-existing instead of new volume.
    let mut retained: HashMap<String, String> = store
        .list_articles(run_id)
        .await?
        .into_iter()
        .map(|row| (row.id, row.seen_at))
        .collect();
    for staged in admission_rows_of(store, run_id).await? {
        let seen_at = staged.article().ok().map(|a| a.seen_at).unwrap_or_default();
        retained.entry(staged.article_id).or_insert(seen_at);
    }
    let repo = store
        .repository()
        .ok_or_else(|| anyhow!("SurrealDB storage is not configured"))?;
    for hit in hits {
        let temperature = stats
            .origins
            .iter()
            .find(|row| row.country == hit.country)
            .map(|row| row.temperature)
            .unwrap_or(0.0);
        let article = article_from(hit, provider, temperature);
        match judge(&article, seen) {
            Verdict::Drop => {
                // A headline with no usable title or URL is a rejected candidate;
                // everything else that judge drops is a repeat.
                let disposition =
                    if article.title.trim().is_empty() || article.url.trim().is_empty() {
                        "rejected"
                    } else {
                        "duplicate_in_cycle"
                    };
                let first_seen = retained.get(&article.id).cloned();
                tel.candidate(&article, provider, disposition, None, first_seen.as_deref());
            }
            Verdict::Keep => {
                if let Some(row) = stats
                    .origins
                    .iter_mut()
                    .find(|row| row.country == article.country)
                {
                    row.articles += 1;
                }
                let disposition = if retained.contains_key(&article.id) {
                    "retained_existing"
                } else {
                    "retained_new"
                };
                remember(seen, &article, None);
                let row = article_row(run_id, &article);
                // Fetch/dedup ends in bounded private staging; the canonical
                // row is written at admission with its evaluated categories.
                let revision = crate::atlas_metadata::input_hash(&row);
                if crate::storage::atlas::stage_article(
                    repo.db(),
                    &row,
                    &revision,
                    crate::atlas::TAXONOMY_VERSION,
                )
                .await?
                {
                    stats.coverage.staged += 1;
                }
                retained.insert(article.id.clone(), article.seen_at.clone());
                tel.candidate(&article, provider, disposition, None, None);
                emit(AtlasEvent::Article(article));
            }
            Verdict::Replace(id) => {
                // Only replace inside this run. A hit already stored in another
                // retained run is ignored so TTL history is not reprocessed.
                let replaced_admitted = store.article(run_id, &id).await?;
                let replaced_staged =
                    crate::storage::atlas::admission_row(repo.db(), run_id, &id).await?;
                if replaced_admitted.is_none() && replaced_staged.is_none() {
                    tel.candidate(&article, provider, "duplicate_in_cycle", None, None);
                    continue;
                }
                if let Some(replaced) = &replaced_admitted {
                    if replaced.country != article.country {
                        if let Some(origin) = stats
                            .origins
                            .iter_mut()
                            .find(|o| o.country == replaced.country)
                        {
                            origin.articles = origin.articles.saturating_sub(1);
                        }
                        if let Some(origin) = stats
                            .origins
                            .iter_mut()
                            .find(|o| o.country == article.country)
                        {
                            origin.articles += 1;
                        }
                    }
                    store.delete_article(run_id, &id).await?;
                }
                remember(seen, &article, Some(&id));
                let row = article_row(run_id, &article);
                let revision = crate::atlas_metadata::input_hash(&row);
                if crate::storage::atlas::stage_article(
                    repo.db(),
                    &row,
                    &revision,
                    crate::atlas::TAXONOMY_VERSION,
                )
                .await?
                {
                    stats.coverage.staged += 1;
                }
                retained.remove(&id);
                retained.insert(article.id.clone(), article.seen_at.clone());
                tel.candidate(&article, provider, "retained_new", Some(id.as_str()), None);
                emit(AtlasEvent::Replaced { id, article });
            }
        }
    }
    Ok(())
}

/// Staging rows of one run through the repository (`store.repository()`).
async fn admission_rows_of(
    store: &AtlasStore,
    run_id: &str,
) -> Result<Vec<crate::storage::atlas::AdmissionRow>> {
    let repo = store
        .repository()
        .ok_or_else(|| anyhow!("SurrealDB storage is not configured"))?;
    Ok(crate::storage::atlas::admission_rows(repo.db(), run_id).await?)
}

fn article_row(run_id: &str, article: &FeedArticle) -> AtlasArticleRow {
    AtlasArticleRow {
        run_id: run_id.into(),
        id: article.id.clone(),
        title: article.title.clone(),
        description: article.description.clone(),
        url: article.url.clone(),
        country: article.country.clone(),
        source_name: article.source_name.clone(),
        source_domain: article.source_domain.clone(),
        author: article.author.clone(),
        image_url: article.image_url.clone(),
        published_at: article.published_at.clone(),
        provider: article.provider.clone(),
        temperature: article.temperature,
        category: category_tag(&article.category).into(),
        seen_at: article.seen_at.clone(),
        categories: Vec::new(),
        admitted_at: String::new(),
    }
}

fn article_from_row(row: &AtlasArticleRow) -> FeedArticle {
    FeedArticle {
        id: row.id.clone(),
        title: row.title.clone(),
        description: row.description.clone(),
        url: row.url.clone(),
        country: row.country.clone(),
        source_name: row.source_name.clone(),
        source_domain: row.source_domain.clone(),
        author: row.author.clone(),
        image_url: row.image_url.clone(),
        published_at: row.published_at.clone(),
        provider: row.provider.clone(),
        temperature: row.temperature,
        seen_at: row.seen_at.clone(),
        category: row.category.clone(),
    }
}

fn require_key(value: &str, name: &str) -> Result<()> {
    if value.trim().is_empty() {
        Err(anyhow!(
            "Enter the {name} on its OSINT tool, or set its environment variable"
        ))
    } else {
        Ok(())
    }
}

fn require_either(primary: &str, fallback: &str, name: &str) -> Result<()> {
    if primary.trim().is_empty() && fallback.trim().is_empty() {
        require_key("", name)
    } else {
        Ok(())
    }
}

/// Run or resume Atlas. `resume` continues the newest paused run. Dedup seeds from
/// the in-memory `feed`, the current run's saved rows, and every article still inside
/// the retention window so a new cycle skips headlines already processed.
pub struct RunInput<'a> {
    pub db_path: &'a Path,
    pub pause: &'a AtomicBool,
    pub keys: &'a ProviderKeys,
    pub user_agent: &'a str,
    pub pace: bool,
    pub resume: bool,
    pub feed: &'a [FeedArticle],
    /// Classifier account. Empty means every saved article stays `unk`.
    /// A Jev decisions model tags categories through the decisions endpoint.
    pub classifier: Option<ProviderSecret>,
    /// Synthesis account. Extracts claims. A decisions model is not used here.
    pub synthesizer: Option<ProviderSecret>,
    /// Ordered synthesis fallbacks tried after primary retries.
    pub synthesizer_fallbacks: &'a [ProviderSecret],
    /// Resume this run instead of the latest one (targeted re-extraction or a
    /// phase-5 retry of an older cycle). Only used when `resume` is true.
    pub run_id: Option<&'a str>,
}

/// A finished run can be resumed when it stopped in phase 4/5 with retained
/// checkpoints (publication retry, indexing retry, configuration fixed).
pub fn resumable(run: &AtlasRunRow) -> bool {
    if run.state == "paused" {
        return true;
    }
    let cursor = parse_cursor(&run.cursor_json);
    matches!(
        run.state.as_str(),
        "failed" | "partial" | "blocked" | "waiting"
    ) && cursor.phase >= 1
}

/// Phase names shown as child jobs of an Atlas cycle.
pub fn phase_title(phase: u8) -> &'static str {
    match phase {
        1 => "Latest headlines",
        2 => "Local coverage",
        3 => "Admit articles",
        4 => "Local article metadata",
        5 => "Related article leads",
        _ => "Index actors and articles",
    }
}

/// Registry tracking for one Atlas cycle: one canonical parent job per run
/// (reused when the run resumes) and one child per phase. Completed phases
/// are skipped by the cursor on resume, so their children are never re-run.
#[derive(Debug)]
pub struct AtlasJob {
    db: std::path::PathBuf,
    run_id: String,
    job: Option<crate::job_registry::JobHandle>,
    phase: Option<(u8, crate::job_registry::JobHandle)>,
    /// The same registry rows under SurrealDB.
    surreal: Option<SurrealCycleJob>,
}

/// Registry rows of one Atlas cycle under SurrealDB. Writes go through the
/// ordered runtime queue so a phase never lands before its job's start; the
/// cancel flag is refreshed from each phase / progress write.
#[derive(Debug)]
#[cfg_attr(not(feature = "surreal"), allow(dead_code))]
struct SurrealCycleJob {
    id: String,
    started: std::time::Instant,
    cancel: Arc<AtomicBool>,
    phase: Option<(u8, String, std::time::Instant)>,
}

/// State, error category, summary and result ref of a [`Finish`].
#[cfg_attr(not(feature = "surreal"), allow(dead_code))]
fn finish_fields(outcome: &crate::job_registry::Finish) -> (&'static str, String, String, String) {
    use crate::job_registry::Finish;
    let state = outcome.state();
    match outcome {
        Finish::Completed { result_ref } => {
            (state, String::new(), String::new(), result_ref.clone())
        }
        Finish::Failed { category, summary } => {
            (state, category.clone(), summary.clone(), String::new())
        }
        Finish::Partial { summary }
        | Finish::Paused { summary }
        | Finish::Waiting { summary }
        | Finish::Blocked { summary }
        | Finish::Cancelled { summary } => (state, String::new(), summary.clone(), String::new()),
    }
}

#[cfg(feature = "surreal")]
impl SurrealCycleJob {
    fn begin(run_id: &str) -> Self {
        let id = format!("atlas_cycle-{run_id}");
        let (job, run) = (id.clone(), run_id.to_string());
        crate::storage::runtime::spawn_ordered(async move {
            let Ok(repo) = crate::storage::runtime::repository().await else {
                return;
            };
            let title = format!("Atlas news cycle · {run}");
            let resource = format!("atlas:{run}");
            let spec = crate::storage::recon_jobs::AppJob {
                id: &job,
                operation: "atlas_cycle",
                app: "atlas",
                title: &title,
                run_ref: &run,
                resource_ref: &resource,
                parent: None,
            };
            if let Err(e) = crate::storage::recon_jobs::begin_app_job(&repo, &spec).await {
                tracing::warn!("atlas job start not recorded: {e:#} (job {job})");
            }
        });
        Self {
            id,
            started: std::time::Instant::now(),
            cancel: Arc::new(AtomicBool::new(false)),
            phase: None,
        }
    }

    fn progress(&self, id: String, phase: &'static str, done: Option<i64>, total: Option<i64>) {
        let cancel = self.cancel.clone();
        let parent = self.id.clone();
        crate::storage::runtime::spawn_ordered(async move {
            let Ok(repo) = crate::storage::runtime::repository().await else {
                return;
            };
            let mut stop = false;
            for (job, done, total) in [(id, done, total), (parent, None, None)] {
                match crate::storage::recon_jobs::set_job_progress(&repo, &job, phase, done, total)
                    .await
                {
                    Ok(flag) => stop |= flag,
                    Err(e) => tracing::warn!("atlas job progress not recorded: {e:#} (job {job})"),
                }
            }
            if stop {
                crate::recon::cancel_with_notify(&cancel);
            }
        });
    }

    fn enter(&mut self, phase: u8) {
        self.close_phase(&crate::job_registry::Finish::completed());
        let child = format!("{}-p{phase}", self.id);
        let (job, parent) = (child.clone(), self.id.clone());
        crate::storage::runtime::spawn_ordered(async move {
            let Ok(repo) = crate::storage::runtime::repository().await else {
                return;
            };
            let spec = crate::storage::recon_jobs::AppJob {
                id: &job,
                operation: match phase {
                    3 => "atlas_classification",
                    4 => "atlas_metadata",
                    5 => "atlas_related",
                    _ => "atlas_collection",
                },
                app: "atlas",
                title: phase_title(phase),
                run_ref: "",
                resource_ref: "",
                parent: Some(&parent),
            };
            if let Err(e) = crate::storage::recon_jobs::begin_app_job(&repo, &spec).await {
                tracing::warn!("atlas phase job not recorded: {e:#} (job {job})");
            }
        });
        self.progress(child.clone(), phase_title(phase), None, None);
        self.phase = Some((phase, child, std::time::Instant::now()));
    }

    fn finish(
        id: String,
        outcome: &crate::job_registry::Finish,
        run: Option<String>,
        active_ms: i64,
    ) {
        let (state, category, summary, result_ref) = finish_fields(outcome);
        crate::storage::runtime::spawn_ordered(async move {
            let Ok(repo) = crate::storage::runtime::repository().await else {
                return;
            };
            if let Err(e) = crate::storage::recon_jobs::finish_recon_job(
                &repo,
                &id,
                state,
                run.as_deref(),
                &category,
                &summary,
                &result_ref,
                active_ms,
            )
            .await
            {
                tracing::warn!("atlas job finish not recorded: {e:#} (job {id})");
            }
        });
    }

    fn close_phase(&mut self, outcome: &crate::job_registry::Finish) {
        if let Some((_, child, started)) = self.phase.take() {
            Self::finish(child, outcome, None, started.elapsed().as_millis() as i64);
        }
    }

    fn close(mut self, outcome: crate::job_registry::Finish, run: &str) {
        use crate::job_registry::Finish;
        let phase_outcome = match &outcome {
            Finish::Completed { .. } => Finish::completed(),
            other => other.clone(),
        };
        self.close_phase(&phase_outcome);
        let active = self.started.elapsed().as_millis() as i64;
        Self::finish(self.id, &outcome, Some(run.to_string()), active);
    }
}

impl AtlasJob {
    fn new(db: &Path) -> Self {
        Self {
            db: db.to_path_buf(),
            run_id: String::new(),
            job: None,
            phase: None,
            surreal: None,
        }
    }

    fn start(&mut self, run_id: &str) {
        self.run_id = run_id.to_string();
        // Under SurrealDB the registry rows live in the repository; never
        // reopen the legacy store there.
        #[cfg(feature = "surreal")]
        {
            self.surreal = Some(SurrealCycleJob::begin(run_id));
        }
    }

    fn enter(&mut self, phase: u8) {
        if self.phase.as_ref().map(|(n, _)| *n) == Some(phase) {
            return;
        }
        #[cfg(feature = "surreal")]
        if let Some(job) = self.surreal.as_mut() {
            if job.phase.as_ref().map(|(n, _, _)| *n) != Some(phase) {
                job.enter(phase);
            }
            return;
        }
        if let Some((_, done)) = self.phase.take() {
            done.finish(crate::job_registry::Finish::completed());
        }
        let Some(parent) = &self.job else {
            return;
        };
        parent.phase(phase_title(phase), None, None);
        let spec = crate::job_registry::JobSpec::new(
            "atlas",
            match phase {
                3 => "atlas_classification",
                4 => "atlas_metadata",
                5 => "atlas_related",
                _ => "atlas_collection",
            },
            phase_title(phase),
        )
        .with_id(format!("{}-p{phase}", parent.id()));
        self.phase = parent.child(spec).ok().map(|child| (phase, child));
    }

    fn progress(&self, done: u32, total: u32) {
        #[cfg(feature = "surreal")]
        if let Some(job) = &self.surreal {
            if let Some((phase, child, _)) = &job.phase {
                job.progress(
                    child.clone(),
                    phase_title(*phase),
                    Some(done as i64),
                    Some(total as i64),
                );
            }
            return;
        }
        if let Some((phase, child)) = &self.phase {
            child.phase(phase_title(*phase), Some(done as i64), Some(total as i64));
        }
    }

    /// Canonical job id once the run is known.
    pub fn job_id(&self) -> Option<String> {
        self.job
            .as_ref()
            .map(|job| job.id().to_string())
            .or_else(|| self.surreal.as_ref().map(|job| job.id.clone()))
    }

    /// Jobs → Cancel was requested for this cycle (either registry).
    fn is_cancelled(&self) -> bool {
        self.job.as_ref().is_some_and(|job| job.is_cancelled())
            || self
                .surreal
                .as_ref()
                .is_some_and(|job| job.cancel.load(Ordering::Relaxed))
    }

    fn close(&mut self, outcome: crate::job_registry::Finish) {
        use crate::job_registry::Finish;
        #[cfg(feature = "surreal")]
        if let Some(job) = self.surreal.take() {
            job.close(outcome, &self.run_id);
            return;
        }
        if let Some((_, child)) = self.phase.take() {
            child.finish(match &outcome {
                Finish::Completed { .. } => Finish::completed(),
                other => other.clone(),
            });
        }
        if let Some(job) = self.job.take() {
            job.finish(outcome);
        }
    }
}

/// Run one Atlas cycle, tracked as a registry job (see [`AtlasJob`]).
pub async fn run_atlas<F>(
    input: RunInput<'_>,
    emit: impl FnMut(AtlasEvent) + Send,
    fetch: F,
) -> Result<Stop>
where
    F: FnMut(HttpCall) -> FetchFut + Send,
{
    use crate::job_registry::Finish;
    let db_path = input.db_path.to_path_buf();
    let tracker = Arc::new(std::sync::Mutex::new(AtlasJob::new(&db_path)));
    let progress = tracker.clone();
    let mut emit = emit;
    let tracked_emit = move |event: AtlasEvent| {
        match &event {
            AtlasEvent::InsightProgress { done, total } => {
                if let Ok(job) = progress.lock() {
                    job.progress(*done, *total);
                }
            }
            AtlasEvent::MemoryProgress { indexed, required } => {
                if let Ok(job) = progress.lock() {
                    job.progress(*indexed, *required);
                }
            }
            _ => {}
        }
        emit(event);
    };
    let result = run_atlas_inner(input, tracked_emit, fetch, &tracker).await;
    // Read the run id first: the tracker guard is never held across an await.
    let tracked_run = tracker
        .lock()
        .ok()
        .map(|job| job.run_id.clone())
        .filter(|id| !id.is_empty());
    if let (Err(error), Some(run_id)) = (&result, tracked_run.as_deref()) {
        if let Ok(store) = AtlasStore::open(&db_path).await {
            let message = error.to_string();
            let state = if message.starts_with("Enter the ") {
                "blocked"
            } else {
                "failed"
            };
            let _ = store.set_state(run_id, state, &message, true).await;
        }
    }
    let state_now = match tracked_run.as_deref() {
        Some(run_id) => match AtlasStore::open(&db_path).await {
            Ok(store) => store
                .list_runs()
                .await
                .ok()
                .and_then(|runs| runs.into_iter().find(|run| run.id == run_id))
                .map(|run| (run.state, run.note)),
            Err(_) => None,
        },
        None => None,
    };
    let run_state = || state_now.clone();
    let outcome = match &result {
        Ok(Stop::Paused) => Finish::Paused {
            summary: "paused; resume continues from the saved cursor".into(),
        },
        Ok(Stop::Failed(message)) => Finish::failed("atlas", message.clone()),
        Ok(Stop::Finished) => match run_state() {
            Some((state, _)) if state == "completed" => Finish::completed(),
            Some((state, note)) if state == "failed" => Finish::failed("atlas", note),
            Some((state, note)) if state == "waiting" => Finish::Waiting {
                summary: if note.is_empty() { state } else { note },
            },
            Some((state, note)) if state == "blocked" => Finish::Blocked {
                summary: if note.is_empty() { state } else { note },
            },
            Some((state, note)) => Finish::Partial {
                summary: if note.is_empty() {
                    state
                } else {
                    format!("{state}: {note}")
                },
            },
            None => Finish::completed(),
        },
        Err(err) => {
            if run_state().is_some_and(|(state, _)| state == "blocked") {
                Finish::Blocked {
                    summary: format!("{err:#}"),
                }
            } else {
                Finish::failed("error", format!("{err:#}"))
            }
        }
    };
    if let Ok(mut job) = tracker.lock() {
        job.close(outcome);
    }
    result
}

fn cycle_paused(job: &std::sync::Mutex<AtlasJob>, pause: &AtomicBool) -> bool {
    if job
        .lock()
        .ok()
        .map(|job| job.is_cancelled())
        .unwrap_or(false)
    {
        pause.store(true, Ordering::Relaxed);
    }
    pause.load(Ordering::Relaxed)
}

/// Run outcome from coverage alone. An all-unknown cycle (every staged
/// article discarded, nothing failed or blocked) completes empty — discards
/// are determined outcomes, not failures.
fn cycle_state(coverage: &CoverageStats, actor_index_incomplete: bool) -> &'static str {
    if coverage.classification_blocked > 0 {
        "blocked"
    } else if coverage.admission_failed > 0
        || coverage.classification_failed > 0
        || coverage.acquisition_failures > 0
        || actor_index_incomplete
    {
        if coverage.classified > 0 || coverage.legacy_unknown > 0 {
            "partial"
        } else {
            "failed"
        }
    } else {
        "completed"
    }
}

/// One migration outcome for a canonical row without v2 categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MigrationStep {
    /// The row already carries the ordered tags.
    SkipMigrated,
    /// A rejection receipt covers this input at the current taxonomy: the
    /// occurrence is confirmed-unknown and loses its canonical row.
    DiscardConfirmedUnknown,
    /// Published evidence keeps its historical scalar identity.
    SkipPublished,
    /// Stage the row for reclassification (resumable while unpublished).
    Reclassify,
}

/// The pure decision behind [`migrate_legacy_categories`].
fn migration_step(row: &AtlasArticleRow, rejected: bool, published: bool) -> MigrationStep {
    if !row.categories.is_empty() || !row.admitted_at.is_empty() {
        MigrationStep::SkipMigrated
    } else if rejected {
        MigrationStep::DiscardConfirmedUnknown
    } else if published {
        MigrationStep::SkipPublished
    } else {
        MigrationStep::Reclassify
    }
}

/// Resumable legacy migration, driven by admission state (spec §3): every
/// retained canonical row without v2 categories is reclassified by subject
/// while it is unpublished. Published rows keep their historical scalar
/// identity; a confirmed-unknown input (a rejection receipt at the current
/// hash and taxonomy) loses its occurrence through lifecycle cleanup, and a
/// reclassification failure stays privately staged for retry. Idempotent:
/// migrated rows carry categories and are skipped.
async fn migrate_legacy_categories(
    store: &AtlasStore,
    stats: &mut RunStats,
    emit: &mut impl FnMut(AtlasEvent),
) -> Result<()> {
    let Some(repo) = store.repository() else {
        anyhow::bail!("SurrealDB storage is not configured");
    };
    for row in store.recent_articles().await? {
        let hash = crate::atlas_metadata::input_hash(&row);
        let rejected = crate::storage::atlas::rejection_exists(
            repo.db(),
            &row.id,
            &hash,
            crate::atlas::TAXONOMY_VERSION,
        )
        .await?;
        let published = crate::storage::brief::published_brief(repo, &row.id)
            .await?
            .is_some();
        match migration_step(&row, rejected, published) {
            MigrationStep::SkipMigrated => continue,
            MigrationStep::SkipPublished => continue,
            MigrationStep::DiscardConfirmedUnknown => {
                store.delete_article(&row.run_id, &row.id).await?;
                stats.coverage.discarded += 1;
            }
            MigrationStep::Reclassify => {
                crate::storage::atlas::stage_article(
                    repo.db(),
                    &row,
                    &hash,
                    crate::atlas::TAXONOMY_VERSION,
                )
                .await?;
                emit(AtlasEvent::Note(format!(
                    "Reclassifying retained article {} under the current taxonomy",
                    row.id
                )));
            }
        }
    }
    Ok(())
}

async fn run_atlas_inner<F>(
    input: RunInput<'_>,
    mut emit: impl FnMut(AtlasEvent) + Send,
    mut fetch: F,
    tracker: &std::sync::Mutex<AtlasJob>,
) -> Result<Stop>
where
    F: FnMut(HttpCall) -> FetchFut + Send,
{
    let track = |phase: u8| {
        if let Ok(mut job) = tracker.lock() {
            job.enter(phase);
        }
    };
    let RunInput {
        db_path,
        pause,
        keys,
        user_agent,
        pace,
        resume,
        feed,
        classifier,
        synthesizer: _,
        synthesizer_fallbacks: _,
        run_id: resume_id,
    } = input;
    let store = AtlasStore::open(db_path).await?;
    let (run_id, mut cursor, mut stats) = if resume {
        let run = match resume_id {
            Some(id) => store
                .list_runs()
                .await?
                .into_iter()
                .find(|run| run.id == id),
            None => store.latest_run().await?,
        };
        let Some(run) = run else {
            return Ok(Stop::Failed("No Atlas run to resume".into()));
        };
        if !resumable(&run) {
            return Ok(Stop::Failed("The latest Atlas run is not paused".into()));
        }
        store
            .set_state(&run.id, "running", &run.note, false)
            .await?;
        (
            run.id,
            parse_cursor(&run.cursor_json),
            parse_stats(&run.stats_json),
        )
    } else {
        let id = new_run_id();
        let from = lookback_from();
        let cursor = Cursor {
            from: from.clone(),
            ..Cursor::default()
        };
        store
            .insert_run(
                &id,
                &serde_json::to_string(&cursor)?,
                &serde_json::to_string(&RunStats::default())?,
            )
            .await?;
        (id, cursor, RunStats::default())
    };
    if resume && cursor.phase <= 2 {
        stats.coverage.acquisition_failures = 0;
        stats.coverage.quota_skipped = 0;
        stats.acquisition_faults.clear();
        stats.acquisition_retry = None;
    }
    stats.contract_version = crate::atlas_work::COLLECTION_CONTRACT.into();
    save(&store, &run_id, &cursor, &stats).await?;
    if let Ok(mut job) = tracker.lock() {
        job.start(&run_id);
    }
    if cursor.from.is_empty() {
        cursor.from = lookback_from();
    }
    // Cycle telemetry. `started_at` is the durable run start, so wall duration is
    // completion minus start even when this invocation is a resume.
    let cycle_started_at = store
        .list_runs()
        .await?
        .into_iter()
        .find(|run| run.id == run_id)
        .map(|run| run.started_at)
        .unwrap_or_default();
    let cycle_mode = if resume { "resumed" } else { "fresh" };
    let mut tel = CycleTelemetry::open(db_path, &run_id, cycle_mode, cycle_started_at).await;
    if cursor.phase <= 1 {
        require_either(&keys.gnews, &keys.gnews_fallback, "GNews API key")?;
        require_either(&keys.newsdata, &keys.newsdata_fallback, "NewsData API key")?;
    }
    let mut seen: Vec<Seen> = feed.iter().map(seen_of).collect();
    // Seed from every retained article (36h prune window), then this run's rows.
    // URL/title matches against that history are dropped so cycles stay efficient.
    for row in store.recent_articles().await? {
        let article = article_from_row(&row);
        if seen.iter().all(|item| item.id != article.id) {
            seen.push(seen_of(&article));
        }
    }
    for row in store.list_articles(&run_id).await? {
        let article = article_from_row(&row);
        if seen.iter().all(|item| item.id != article.id) {
            seen.push(seen_of(&article));
        }
    }
    let mut last_gnews: Option<std::time::Instant> = None;
    let user_agent = if user_agent.trim().is_empty() {
        crate::osint::DEFAULT_USER_AGENT.to_string()
    } else {
        user_agent.to_string()
    };

    let inventory = crate::atlas_metadata::NativeInventory::start(db_path, &run_id, &user_agent);

    if cursor.phase <= 1 {
        track(1);
        tel.enter_stage("collection", "article", &stats);
        let providers = ["gnews", "newsdata"];
        let start = providers
            .iter()
            .position(|provider| *provider == cursor.leg)
            .unwrap_or(0);
        for provider in providers.into_iter().skip(start) {
            let continuing = provider == cursor.leg.as_str();
            let mut page = if continuing { cursor.chunk } else { 0 };
            let mut token = if continuing {
                cursor.page_token.clone()
            } else {
                String::new()
            };
            let mut kept = if continuing { cursor.kept } else { 0 };
            while kept < PHASE1_TARGET && page < PHASE1_MAX_PAGES {
                let here = Cursor {
                    phase: 1,
                    chunk: page,
                    leg: provider.into(),
                    country: 0,
                    from: cursor.from.clone(),
                    page_token: token.clone(),
                    kept,
                };
                if cycle_paused(tracker, pause) {
                    return park(&store, &run_id, &here, &stats, &mut emit, &mut tel).await;
                }
                let receipt_key = format!("atlas-native-v1:{run_id}:discovery:{provider}:{page}");
                if let Some(raw) = store.app_state_get(&receipt_key).await? {
                    let receipt: serde_json::Value = serde_json::from_str(&raw)?;
                    kept = receipt["kept"].as_u64().unwrap_or(kept as u64) as u32;
                    token = receipt["token"].as_str().unwrap_or("").into();
                    page += 1;
                    if receipt["stop"].as_bool().unwrap_or(false) {
                        break;
                    }
                    continue;
                }
                let (primary, fallback) = keys.pair(provider);
                let Some(key) = open_key(&store, provider, primary, fallback).await? else {
                    emit(AtlasEvent::Note(format!("{provider} daily quota is spent")));
                    break;
                };
                let spare = if key == primary.trim() {
                    fallback.trim()
                } else {
                    ""
                };
                if pace && provider == "gnews" {
                    if let Some(last) = last_gnews {
                        let wait = std::time::Duration::from_secs(1).saturating_sub(last.elapsed());
                        if !wait.is_zero() {
                            tokio::time::sleep(wait).await;
                        }
                    }
                }
                emit(AtlasEvent::Status(format!(
                    "Latest {provider} page {} · {kept}/{PHASE1_TARGET}",
                    page + 1
                )));
                let mut stop_provider = false;
                let mut acquisition_ok = false;
                let fetched = fetch_with_spare_key(
                    &mut fetch,
                    &user_agent,
                    provider,
                    primary,
                    key,
                    spare,
                    |active| phase1_call(provider, page, &token, active),
                )
                .await;
                let bucket = fetched.bucket;
                match fetched.body {
                    Some(body) => {
                        if provider == "gnews" {
                            last_gnews = Some(std::time::Instant::now());
                        }
                        let more = atlas_news::next_page(provider, &body);
                        match atlas_news::discovery_hits(provider, &body) {
                            Ok(mut hits) => {
                                acquisition_ok = true;
                                hits.retain(|hit| {
                                    published_in_window(&hit.published_at, &cursor.from)
                                });
                                let room = (PHASE1_TARGET - kept) as usize;
                                hits.truncate(room);
                                kept += hits.len() as u32;
                                apply_hits(
                                    &hits, provider, &mut stats, &mut seen, &mut emit, &store,
                                    &run_id, &mut tel,
                                )
                                .await?;
                                token = if provider == "newsdata" {
                                    more.clone().unwrap_or_default()
                                } else {
                                    String::new()
                                };
                                if more.is_none() {
                                    stop_provider = true;
                                }
                            }
                            Err(err) => {
                                let fault = ProviderFault {
                                    summary: format!("{provider} unreadable response: {err}"),
                                    body: String::new(),
                                };
                                if record_fault(&mut stats, provider, &fault)
                                    && stats.acquisition_retry.is_none()
                                {
                                    stats.acquisition_retry = Some(here.clone());
                                }
                                emit(parse_fault(provider, fault.summary, &body));
                                stop_provider = true;
                            }
                        }
                    }
                    None => {
                        if let Some(fault) = fetched.fault {
                            if record_fault(&mut stats, provider, &fault)
                                && stats.acquisition_retry.is_none()
                            {
                                stats.acquisition_retry = Some(here.clone());
                            }
                            emit(AtlasEvent::Fault(fault));
                        }
                        stop_provider = true;
                    }
                }
                if acquisition_ok {
                    save_native_receipt(
                        &store,
                        &run_id,
                        &receipt_key,
                        &here,
                        &stats,
                        &serde_json::json!({"kept":kept,"token":token,"stop":stop_provider || kept>=PHASE1_TARGET || page+1>=PHASE1_MAX_PAGES}),
                    ).await?;
                }
                let _ = store.quota_bump(&bucket, &utc_day()).await?;
                page += 1;
                emit(AtlasEvent::Stats(stats.clone()));
                if stop_provider || kept >= PHASE1_TARGET || page >= PHASE1_MAX_PAGES {
                    break;
                }
                let next = Cursor {
                    phase: 1,
                    chunk: page,
                    leg: provider.into(),
                    country: 0,
                    from: cursor.from.clone(),
                    page_token: token.clone(),
                    kept,
                };
                save(&store, &run_id, &next, &stats).await?;
                cursor = next;
            }
        }
        let bands = score_counts(&stats.counts);
        let previous: HashMap<String, u32> = stats
            .origins
            .iter()
            .map(|row| (row.country.clone(), row.articles))
            .collect();
        stats.origins = bands
            .iter()
            .map(|band| OriginStat {
                country: band.country.clone(),
                tier: band.tier,
                temperature: band.temperature,
                volume: band.count,
                articles: previous.get(&band.country).copied().unwrap_or(0),
            })
            .collect();
        stats.scored = true;
        cursor = Cursor {
            phase: 2,
            chunk: 0,
            leg: "newsapi".into(),
            country: 0,
            from: cursor.from.clone(),
            page_token: String::new(),
            kept: 0,
        };
        save(&store, &run_id, &cursor, &stats).await?;
        emit(AtlasEvent::Stats(stats.clone()));
        // One origin snapshot per cycle, taken here where the bands are computed,
        // never once per repaint.
        let snapshot_at = chrono::Utc::now().to_rfc3339();
        tel.origin_snapshot(&stats.origins, &snapshot_at, !stats.counts.is_empty());
    }

    if cursor.phase == 2 {
        track(2);
        tel.enter_stage("collection", "article", &stats);
        require_either(&keys.newsapi, &keys.newsapi_fallback, "NewsAPI key")?;
        require_either(&keys.currents, &keys.currents_fallback, "Currents API key")?;
        if stats.origins.is_empty() && stats.coverage.acquisition_failures == 0 {
            store
                .set_state(
                    &run_id,
                    "completed",
                    "No flashpoints in the 48-hour window",
                    true,
                )
                .await?;
            emit(AtlasEvent::Status(
                "No flashpoints in the 48-hour window".into(),
            ));
            tel.close_stage("completed", &stats);
            tel.cycle("completed", &stats);
            return Ok(Stop::Finished);
        }
        let countries: Vec<String> = stats
            .origins
            .iter()
            .map(|row| row.country.clone())
            .collect();
        let phase2: Vec<Job> = countries
            .iter()
            .enumerate()
            .flat_map(|(index, _)| {
                [
                    Job {
                        phase: 2,
                        chunk: 0,
                        leg: "newsapi",
                        country: index,
                        provider: "newsapi",
                    },
                    Job {
                        phase: 2,
                        chunk: 0,
                        leg: "currents",
                        country: index,
                        provider: "currents",
                    },
                ]
            })
            .collect();
        let mut newsapi_skipped: Vec<String> = Vec::new();
        let mut currents_skipped: Vec<String> = Vec::new();
        for job in &phase2 {
            if job_is_before(job, &cursor) {
                continue;
            }
            if cycle_paused(tracker, pause) {
                return park(
                    &store,
                    &run_id,
                    &cursor_at(job, &cursor.from),
                    &stats,
                    &mut emit,
                    &mut tel,
                )
                .await;
            }
            let receipt_key = format!(
                "atlas-native-v1:{run_id}:coverage:{}:{}",
                job.provider, countries[job.country]
            );
            if store.app_state_get(&receipt_key).await?.is_some() {
                continue;
            }
            let country = &countries[job.country];
            let supported = match job.provider {
                "newsapi" => atlas_news::newsapi_country_supported(country),
                "currents" => atlas_news::currents_country_supported(country),
                _ => true,
            };
            if !supported {
                match job.provider {
                    "newsapi" => newsapi_skipped.push(country.clone()),
                    "currents" => currents_skipped.push(country.clone()),
                    _ => {}
                }
                continue;
            }
            let (primary, fallback) = keys.pair(job.provider);
            let Some(key) = open_key(&store, job.provider, primary, fallback).await? else {
                emit(AtlasEvent::Note(format!(
                    "{} daily quota is spent",
                    job.provider
                )));
                continue;
            };
            let spare = if key == primary.trim() {
                fallback.trim()
            } else {
                ""
            };
            emit(AtlasEvent::Status(format!(
                "Headlines {} · {}",
                country.to_ascii_uppercase(),
                job.provider
            )));
            let fetched = fetch_with_spare_key(
                &mut fetch,
                &user_agent,
                job.provider,
                primary,
                key,
                spare,
                |active| phase2_call(job.provider, country, active),
            )
            .await;
            let bucket = fetched.bucket;
            match fetched.body {
                Some(body) => match atlas_news::headline_hits(job.provider, country, &body) {
                    Ok(hits) => {
                        apply_hits(
                            &hits,
                            job.provider,
                            &mut stats,
                            &mut seen,
                            &mut emit,
                            &store,
                            &run_id,
                            &mut tel,
                        )
                        .await?;
                        save_native_receipt(
                            &store,
                            &run_id,
                            &receipt_key,
                            &cursor_at(job, &cursor.from),
                            &stats,
                            &serde_json::json!({"completed":true}),
                        )
                        .await?;
                    }
                    Err(err) => {
                        let fault = ProviderFault {
                            summary: format!("{} unreadable response: {err}", job.provider),
                            body: String::new(),
                        };
                        if record_fault(&mut stats, job.provider, &fault)
                            && stats.acquisition_retry.is_none()
                        {
                            stats.acquisition_retry = Some(cursor_at(job, &cursor.from));
                        }
                        emit(parse_fault(job.provider, fault.summary, &body));
                    }
                },
                None => {
                    if let Some(fault) = fetched.fault {
                        if record_fault(&mut stats, job.provider, &fault)
                            && stats.acquisition_retry.is_none()
                        {
                            stats.acquisition_retry = Some(cursor_at(job, &cursor.from));
                        }
                        emit(AtlasEvent::Fault(fault));
                    }
                }
            }
            let _ = store.quota_bump(&bucket, &utc_day()).await?;
            let next = next_after_phase2(job, countries.len());
            save(&store, &run_id, &cursor_at(&next, &cursor.from), &stats).await?;
            emit(AtlasEvent::Stats(stats.clone()));
        }
        if !newsapi_skipped.is_empty() {
            let names = newsapi_skipped
                .iter()
                .map(|code| country_label(code))
                .collect::<Vec<_>>()
                .join(", ");
            emit(AtlasEvent::Note(format!(
                "NewsAPI was not queried for {names}. Top headlines has no feed for those countries."
            )));
        }
        if !currents_skipped.is_empty() {
            let names = currents_skipped
                .iter()
                .map(|code| country_label(code))
                .collect::<Vec<_>>()
                .join(", ");
            emit(AtlasEvent::Note(format!(
                "Currents was not queried for {names}. Latest news has no feed for those countries."
            )));
        }
        if stats.coverage.acquisition_failures == 0 {
            prune_empty_origins(&mut stats);
        }
        cursor = Cursor {
            phase: 3,
            chunk: 0,
            leg: "classify".into(),
            country: 0,
            from: cursor.from.clone(),
            page_token: String::new(),
            kept: 0,
        };
        save(&store, &run_id, &cursor, &stats).await?;
        emit(AtlasEvent::Stats(stats.clone()));
    }

    // Local actor/metadata processing runs after admission admits the
    // canonical rows. Dropping an invocation cancels any admitted NLP work;
    // no detached cycle worker survives Pause.
    struct NativeCancel(Arc<AtomicBool>);
    impl Drop for NativeCancel {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }
    let native_cancel = NativeCancel(Arc::new(AtomicBool::new(false)));
    if cursor.phase == 3 {
        track(3);
        tel.enter_stage(
            crate::atlas_work::stage_label(crate::atlas_work::STAGE_CLASSIFICATION),
            "article",
            &stats,
        );
        let repo = store
            .repository()
            .ok_or_else(|| anyhow!("SurrealDB storage is not configured"))?;
        // An interrupted cycle leaves `evaluating` rows behind; they retry.
        crate::storage::atlas::admission_reset_stale(repo.db()).await?;
        migrate_legacy_categories(&store, &mut stats, &mut emit).await?;
        let staged = crate::storage::atlas::admission_work(repo.db()).await?;
        let mut staging: Vec<(crate::storage::atlas::AdmissionRow, AtlasArticleRow)> =
            Vec::with_capacity(staged.len());
        for row in staged {
            match row.article() {
                Ok(article) => staging.push((row, article)),
                Err(error) => {
                    // An unparsable payload is a retryable failure; the input
                    // stays privately retained until it is re-staged.
                    crate::storage::atlas::admission_failed(
                        repo.db(),
                        &row.run_id,
                        &row.article_id,
                        &row.input_hash,
                        &format!("staged payload: {error}"),
                    )
                    .await?;
                    stats.coverage.admission_failed += 1;
                }
            }
        }
        stats.coverage.admission_pending = staging
            .iter()
            .filter(|(row, _)| row.state == "pending")
            .count() as u32;
        stats.coverage.admission_failed = staging
            .iter()
            .filter(|(row, _)| row.state == "failed")
            .count() as u32;
        if let Some(secret) = classifier.as_ref() {
            // Borrowed pause flags are mirrored for the shared cancellable executor.
            let cancel = Arc::new(AtomicBool::new(cycle_paused(tracker, pause)));
            let articles: Vec<_> = staging.iter().map(|(_, a)| a.clone()).collect();
            for (batch_index, batch) in
                crate::atlas_classify::batches_for_model(&articles, &secret.model)
                    .into_iter()
                    .enumerate()
            {
                if cycle_paused(tracker, pause) {
                    return park(&store, &run_id, &cursor, &stats, &mut emit, &mut tel).await;
                }
                emit(AtlasEvent::Status(format!(
                    "Admission batch {} · {} articles",
                    batch_index + 1,
                    batch.len()
                )));
                for article in &batch {
                    let Some((row, _)) = staging
                        .iter()
                        .find(|(_, a)| a.id == article.id && a.run_id == article.run_id)
                    else {
                        continue;
                    };
                    crate::storage::atlas::admission_begin(
                        repo.db(),
                        &row.run_id,
                        &row.article_id,
                        &row.input_hash,
                    )
                    .await?;
                }
                let work = crate::atlas_classify::execute(
                    db_path,
                    &run_id,
                    secret,
                    batch.clone(),
                    &cancel,
                );
                tokio::pin!(work);
                let (receipt, error) = loop {
                    tokio::select! {
                        result = &mut work => break result,
                        _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {
                            if cycle_paused(tracker, pause) {
                                crate::recon::cancel_with_notify(&cancel);
                            }
                        }
                    }
                };
                if let Some(error) = &error {
                    emit(AtlasEvent::Note(format!(
                        "Admission batch incomplete: {error}"
                    )));
                }
                for article in &batch {
                    let Some((staged, _)) = staging
                        .iter()
                        .find(|(_, a)| a.id == article.id && a.run_id == article.run_id)
                    else {
                        continue;
                    };
                    let outcome = receipt
                        .get(&article.id)
                        .and_then(|raw| crate::atlas_classify::Outcome::decode(raw));
                    let cancelled = cancel.load(Ordering::Relaxed);
                    match outcome {
                        Some(crate::atlas_classify::Outcome::Admitted(tags)) => {
                            let mut admitted = article.clone();
                            admitted.categories = tags;
                            crate::storage::atlas::admission_admit(
                                repo.db(),
                                &admitted,
                                &staged.input_hash,
                                &staged.taxonomy_version,
                            )
                            .await?;
                            stats.coverage.admitted += 1;
                            let first = admitted.categories.first().cloned().unwrap_or_default();
                            let receipt_json = serde_json::to_string(&serde_json::json!({
                                "categories": admitted.categories,
                                "prompt_version": crate::atlas_classify::PROMPT_VERSION,
                                "diagnostic": error,
                            }))?;
                            store
                                .set_classification(
                                    &admitted.run_id,
                                    &admitted.id,
                                    &staged.input_hash,
                                    "classified",
                                    &receipt_json,
                                    Some(&first),
                                )
                                .await?;
                            emit(AtlasEvent::Classified {
                                id: article.id.clone(),
                                category: first,
                            });
                            // Local entity/publisher metadata overlaps the
                            // remaining admission batches in the worker task.
                            inventory.queue(&admitted).await?;
                        }
                        Some(crate::atlas_classify::Outcome::Unknown(reason)) => {
                            let was_canonical = store
                                .article(&staged.run_id, &staged.article_id)
                                .await?
                                .is_some();
                            crate::storage::atlas::admission_discard(
                                repo.db(),
                                &staged.run_id,
                                &staged.article_id,
                                &staged.input_hash,
                                &staged.source_revision,
                                &staged.taxonomy_version,
                                &reason,
                            )
                            .await?;
                            if was_canonical {
                                // A legacy occurrence reclassified as
                                // confirmed-unknown loses its canonical row
                                // and article-owned derivatives.
                                store
                                    .delete_article(&staged.run_id, &staged.article_id)
                                    .await?;
                            }
                            stats.coverage.discarded += 1;
                        }
                        None if cancelled => {
                            // Pause: the row stays staged and retries on resume.
                        }
                        None => {
                            let detail = error
                                .clone()
                                .unwrap_or_else(|| "admission did not resolve this article".into());
                            let blocked = detail.starts_with("all routes blocked");
                            crate::storage::atlas::admission_failed(
                                repo.db(),
                                &staged.run_id,
                                &staged.article_id,
                                &staged.input_hash,
                                &detail,
                            )
                            .await?;
                            stats.coverage.admission_failed += 1;
                            if blocked {
                                stats.coverage.classification_blocked += 1;
                            }
                        }
                    }
                }
                save(&store, &run_id, &cursor, &stats).await?;
                if cycle_paused(tracker, pause) {
                    return park(&store, &run_id, &cursor, &stats, &mut emit, &mut tel).await;
                }
            }
        } else if !staging.is_empty() {
            emit(AtlasEvent::Note(
                "Classifier is not configured; required classification is blocked.".into(),
            ));
            stats.coverage.classification_blocked = staging.len() as u32;
        }
        cursor.phase = 4;
        cursor.leg = "metadata".into();
        save(&store, &run_id, &cursor, &stats).await?;
    }
    // Metadata collection joined the admitted canonical rows: the spawned
    // inventory worker processed them during the admission batches; this
    // joins it and sweeps anything its per-article saves missed.
    let native_work = async {
        inventory.finish().await?;
        crate::atlas_metadata::process_run(db_path, &run_id, native_cancel.0.clone(), &user_agent)
            .await
    };
    tokio::pin!(native_work);
    let mut native_result = None;
    // Stage-specific admission: a phase-6 resume never replays 4/5.
    if (4..6).contains(&cursor.phase) {
        track(4);
        tel.enter_stage("metadata", "article", &stats);
        if cycle_paused(tracker, pause) {
            return park(&store, &run_id, &cursor, &stats, &mut emit, &mut tel).await;
        }
        emit(AtlasEvent::Status(
            "Local entities and publisher priors".into(),
        ));
        let coverage = if let Some(coverage) = native_result.take() {
            coverage
        } else {
            loop {
                tokio::select! {
                    result = &mut native_work => break result?,
                    _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {
                        if cycle_paused(tracker,pause) {
                            native_cancel.0.store(true,Ordering::Relaxed);
                            return park(&store,&run_id,&cursor,&stats,&mut emit,&mut tel).await;
                        }
                    }
                }
            }
        };
        stats.coverage.entities_available = coverage.entities_complete;
        stats.coverage.entities_unavailable = coverage.entities_unavailable;
        stats.coverage.publishers_available = coverage.publishers_available;

        stats.coverage.warnings = coverage.warnings.len() as u32;
        for warning in coverage.warnings {
            emit(AtlasEvent::Note(warning));
        }
        if cycle_paused(tracker, pause) {
            return park(&store, &run_id, &cursor, &stats, &mut emit, &mut tel).await;
        }
        cursor.phase = 5;
        cursor.leg = "related".into();
        track(5);
        tel.enter_stage("related", "pair", &stats);
        emit(AtlasEvent::Status(
            "Building shared headline/description leads".into(),
        ));
        stats.coverage.related_pairs =
            crate::related_articles::build_on(&store, &run_id).await? as u32;
        cursor.phase = 6;
        cursor.leg = "actor_articles".into();
        save(&store, &run_id, &cursor, &stats).await?;
    }
    let mut actor_index_incomplete = false;
    if cursor.phase >= 6 {
        track(6);
        tel.enter_stage(
            crate::atlas_work::stage_label(crate::atlas_work::STAGE_ACTOR_ARTICLES),
            crate::atlas_work::stage_unit(crate::atlas_work::STAGE_ACTOR_ARTICLES),
            &stats,
        );
        emit(AtlasEvent::Status("Index actors and articles".into()));
        let report = crate::atlas_actor_index::index_run_store(&store, &run_id, &|| {
            cycle_paused(tracker, pause)
        })
        .await?;
        stats.coverage.actor_indexed = report.reused + report.indexed;
        stats.coverage.actor_zero = report.zero_actor;
        stats.coverage.actor_index_failed = report.failed;
        for error in report.errors.iter().take(5) {
            emit(AtlasEvent::Note(format!("Actor index failed: {error}")));
        }
        if report.stopped {
            return park(&store, &run_id, &cursor, &stats, &mut emit, &mut tel).await;
        }
        actor_index_incomplete = !report.complete();
        cursor.leg = "actor_articles_done".into();
        save(&store, &run_id, &cursor, &stats).await?;
    }
    // Completion depends on admission, never on Brain indexing. Staged work
    // counts as the fetched set; canonical rows are the admitted set.
    let articles = store.list_articles(&run_id).await?;
    let live_staging = admission_rows_of(&store, &run_id).await?.len() as u32;
    stats.coverage.articles = articles.len() as u32 + live_staging;
    stats.coverage.classified = articles.iter().filter(|a| !a.categories.is_empty()).count() as u32;
    stats.coverage.admitted = stats.coverage.classified;
    // Legacy unknowns no longer persist: the migration reclassifies or
    // discards them, so only unpublished legacy rows can remain here.
    stats.coverage.legacy_unknown = articles
        .iter()
        .filter(|a| a.categories.is_empty() && a.category == "unk")
        .count() as u32;
    stats.coverage.publishers_unavailable = stats
        .coverage
        .articles
        .saturating_sub(stats.coverage.publishers_available);
    let state = cycle_state(&stats.coverage, actor_index_incomplete);
    let note = format!(
        "Admitted {}/{} · discarded {} · pending {} · failed {} · legacy unk {} · entities {} · publisher priors {} · Related {} · actors indexed {}/{} (zero-actor {}, failed {}) · acquisition failures {}{}{}{}",
        stats.coverage.admitted,
        stats.coverage.articles,
        stats.coverage.discarded,
        stats.coverage.admission_pending,
        stats.coverage.admission_failed,
        stats.coverage.legacy_unknown,
        stats.coverage.entities_available,
        stats.coverage.publishers_available,
        stats.coverage.related_pairs,
        stats.coverage.actor_indexed,
        stats.coverage.articles,
        stats.coverage.actor_zero,
        stats.coverage.actor_index_failed,
        stats.coverage.acquisition_failures,
        if stats.acquisition_faults.is_empty() {
            String::new()
        } else {
            format!(" ({})", stats.acquisition_faults.join("; "))
        },
        if stats.coverage.quota_skipped > 0 {
            format!(" · quota-limited calls {}", stats.coverage.quota_skipped)
        } else {
            String::new()
        },
        if stats.coverage.warnings > 0 {
            " · metadata warnings"
        } else {
            ""
        }
    );
    // Resume retries unresolved admission work and reuses all receipts.
    if state != "completed" {
        if let Some(retry) = &stats.acquisition_retry {
            cursor = retry.clone();
        } else if stats.coverage.classification_failed == 0
            && stats.coverage.classification_blocked == 0
            && stats.coverage.admission_failed == 0
            && actor_index_incomplete
        {
            // Only phase 6 is unfinished: resume retries missing receipts only.
            cursor.phase = 6;
            cursor.leg = "actor_articles".into();
        } else {
            cursor.phase = 3;
            cursor.leg = "classify".into();
        }
    }
    save(&store, &run_id, &cursor, &stats).await?;
    store.set_state(&run_id, state, &note, true).await?;
    emit(AtlasEvent::Stats(stats.clone()));
    emit(AtlasEvent::Status(note.clone()));
    tel.close_stage(state, &stats);
    tel.cycle(state, &stats);
    Ok(if state == "failed" {
        Stop::Failed(note)
    } else {
        Stop::Finished
    })
}

/// The completion receipt and statistics commit together, preventing a crash
/// from skipping unapplied discovery counts or replaying an applied page.
async fn save_native_receipt(
    store: &AtlasStore,
    run_id: &str,
    key: &str,
    cursor: &Cursor,
    stats: &RunStats,
    receipt: &serde_json::Value,
) -> Result<()> {
    store
        .save_with_receipt(
            run_id,
            key,
            &serde_json::to_string(receipt)?,
            &serde_json::to_string(cursor)?,
            &serde_json::to_string(stats)?,
        )
        .await
}

fn next_after_phase2(job: &Job, countries: usize) -> Job {
    if job.leg == "newsapi" {
        Job {
            leg: "currents",
            provider: "currents",
            ..job.clone()
        }
    } else if job.country + 1 < countries {
        Job {
            country: job.country + 1,
            leg: "newsapi",
            provider: "newsapi",
            ..job.clone()
        }
    } else {
        job.clone()
    }
}

fn published_in_window(published: &str, from: &str) -> bool {
    let Ok(from) = chrono::DateTime::parse_from_rfc3339(from) else {
        return true;
    };
    let published = published.trim();
    if published.is_empty() {
        return true;
    }
    if let Ok(stamp) = chrono::DateTime::parse_from_rfc3339(published) {
        return stamp >= from;
    }
    chrono::NaiveDateTime::parse_from_str(published, "%Y-%m-%d %H:%M:%S")
        .map(|naive| naive.and_utc() >= from)
        .unwrap_or(true)
}

async fn quota_open(store: &AtlasStore, provider: &str) -> Result<bool> {
    let used = store.quota_used(provider, &utc_day()).await?;
    Ok(used < daily_cap(provider))
}

async fn save(store: &AtlasStore, id: &str, cursor: &Cursor, stats: &RunStats) -> Result<()> {
    store
        .save(
            id,
            &serde_json::to_string(cursor)?,
            &serde_json::to_string(stats)?,
        )
        .await
}

async fn park(
    store: &AtlasStore,
    id: &str,
    cursor: &Cursor,
    stats: &RunStats,
    emit: &mut impl FnMut(AtlasEvent),
    tel: &mut CycleTelemetry,
) -> Result<Stop> {
    save(store, id, cursor, stats).await?;
    store.set_state(id, "paused", "Paused", false).await?;
    // The open stage interval is closed as interrupted, so the age of work a
    // pause stopped mid-flight is derivable from its start timestamp.
    tel.close_stage("interrupted", stats);
    emit(AtlasEvent::Stats(stats.clone()));
    emit(AtlasEvent::Status("Paused".into()));
    Ok(Stop::Paused)
}

/// Records one provider fault. A quota or rate limit is an expected daily
/// condition and is counted separately; anything else is an acquisition
/// failure that the status text names. Returns true for a real failure.
fn record_fault(stats: &mut RunStats, provider: &str, fault: &ProviderFault) -> bool {
    if quota_fault(fault) {
        stats.coverage.quota_skipped += 1;
        return false;
    }
    stats.coverage.acquisition_failures += 1;
    if stats.acquisition_faults.len() < 5 {
        let line = one_line(&fault.summary);
        let line = if line.to_ascii_lowercase().starts_with(provider) {
            line
        } else {
            format!("{provider}: {line}")
        };
        stats
            .acquisition_faults
            .push(line.chars().take(110).collect());
    }
    true
}

fn parse_fault(_provider: &str, summary: String, body: &serde_json::Value) -> AtlasEvent {
    AtlasEvent::Fault(ProviderFault {
        summary: one_line(&summary),
        body: serde_json::to_string_pretty(body).unwrap_or_else(|_| body.to_string()),
    })
}

fn one_line(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(180).collect()
}

fn pretty_body(raw: &str) -> String {
    let raw = raw.trim();
    let text = serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .unwrap_or_else(|| raw.to_string());
    const MAX: usize = 16_000;
    if text.chars().count() <= MAX {
        text
    } else {
        let mut cut: String = text.chars().take(MAX).collect();
        cut.push_str("\n…");
        cut
    }
}

fn json_message(raw: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let message = value
        .pointer("/results/message")
        .or_else(|| value.get("message"))
        .or_else(|| value.get("errors"))?;
    let text = message
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| message.to_string());
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn provider_fault(provider: &str, status: u16, raw: &str) -> ProviderFault {
    let named = atlas_news::http_error(provider, status, raw);
    let summary = match named {
        Some(text) if !text.contains('{') => text,
        _ => match json_message(raw) {
            Some(message) => format!("{provider} HTTP {status}: {}", one_line(&message)),
            None => format!("{provider} HTTP {status}"),
        },
    };
    ProviderFault {
        summary: one_line(&summary),
        body: pretty_body(raw),
    }
}

/// Whether a failed call is worth one more try: transport errors, timeouts,
/// request timeouts and 5xx. A 429 is not retried here (quota is handled by the
/// spare-key path) and neither is any other 4xx.
fn transient_status(status: u16) -> bool {
    status == 408 || (500..600).contains(&status)
}

/// The redirect target when it keeps the same https host (so a key in the URL or
/// headers never leaves the provider), resolved against the request URL.
fn same_host_redirect(from: &str, location: &str) -> Option<String> {
    let base = url::Url::parse(from).ok()?;
    let next = base.join(location).ok()?;
    (next.scheme() == "https" && next.host_str() == base.host_str()).then(|| next.to_string())
}

const DISPATCH_ATTEMPTS: u32 = 2;
const DISPATCH_REDIRECTS: u32 = 3;

async fn dispatch<F>(
    fetch: &mut F,
    spec: CallSpec,
    user_agent: &str,
) -> Result<serde_json::Value, ProviderFault>
where
    F: FnMut(HttpCall) -> FetchFut,
{
    let provider = spec.provider;
    let mut headers = spec.headers;
    headers.push(("User-Agent".into(), user_agent.into()));
    headers.push(("Accept".into(), "application/json".into()));
    let mut url = spec.url;
    let mut attempt = 0;
    let mut redirects = 0;
    let reply = loop {
        attempt += 1;
        let sent = fetch(HttpCall {
            provider: provider.into(),
            url: url.clone(),
            headers: headers.clone(),
        })
        .await;
        match sent {
            Ok(reply) if (300..400).contains(&reply.status) && redirects < DISPATCH_REDIRECTS => {
                match reply
                    .location
                    .as_deref()
                    .and_then(|location| same_host_redirect(&url, location))
                {
                    Some(next) => {
                        redirects += 1;
                        url = next;
                        continue;
                    }
                    None => break reply,
                }
            }
            Ok(reply) if transient_status(reply.status) && attempt < DISPATCH_ATTEMPTS => {}
            Ok(reply) => break reply,
            Err(err) if attempt < DISPATCH_ATTEMPTS => {
                let _ = err;
            }
            Err(err) => {
                let kind = if err
                    .downcast_ref::<reqwest::Error>()
                    .is_some_and(reqwest::Error::is_timeout)
                {
                    "timed out"
                } else {
                    "request failed"
                };
                return Err(ProviderFault {
                    summary: one_line(&format!(
                        "{provider} {kind} after {attempt} attempts: {err}"
                    )),
                    body: String::new(),
                });
            }
        }
        #[cfg(not(test))]
        tokio::time::sleep(std::time::Duration::from_millis(750)).await;
    };
    if (300..400).contains(&reply.status) {
        return Err(ProviderFault {
            summary: format!(
                "{provider} HTTP {} redirect to another host was not followed",
                reply.status
            ),
            body: String::new(),
        });
    }
    if !(200..300).contains(&reply.status) {
        return Err(provider_fault(provider, reply.status, &reply.body));
    }
    serde_json::from_str(&reply.body).map_err(|err| ProviderFault {
        summary: format!("{provider} malformed JSON: {err}"),
        body: pretty_body(&reply.body),
    })
}

/// Which run [`run_live`] works on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LiveRun {
    /// Start a new cycle.
    Fresh,
    /// Resume the latest run (paused, or stopped in phase 4/5).
    Latest,
    /// Resume this run (an older cycle stopped in phase 4/5).
    Run(String),
}

/// Live pipeline. Uses the process HTTP client and paces GNews by one second.
pub async fn run_live(
    db_path: &Path,
    pause: &AtomicBool,
    keys: &ProviderKeys,
    user_agent: &str,
    live: LiveRun,
    feed: &[FeedArticle],
    classifier: Option<ProviderSecret>,
    synthesizer: Option<ProviderSecret>,
    synthesizer_fallbacks: &[ProviderSecret],
    emit: impl FnMut(AtlasEvent) + Send,
) -> Result<Stop> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(25))
        .build()?;
    run_atlas(
        RunInput {
            db_path,
            pause,
            keys,
            user_agent,
            pace: true,
            resume: live != LiveRun::Fresh,
            feed,
            classifier,
            synthesizer,
            synthesizer_fallbacks,
            run_id: match &live {
                LiveRun::Run(id) => Some(id.as_str()),
                _ => None,
            },
        },
        emit,
        move |call: HttpCall| {
            let client = client.clone();
            Box::pin(async move {
                let mut request = client.get(&call.url);
                for (name, value) in &call.headers {
                    request = request.header(name.as_str(), value.as_str());
                }
                let response = request.send().await?;
                let status = response.status().as_u16();
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string);
                let body = response.text().await?;
                Ok(HttpReply {
                    status,
                    body,
                    location,
                })
            })
        },
    )
    .await
}

pub async fn charge_newsapi(store: &AtlasStore) {
    charge_quota(store, "newsapi").await;
}

pub async fn charge_quota(store: &AtlasStore, bucket: &str) {
    let _ = store.quota_bump(bucket, &utc_day()).await;
}

#[cfg(test)]
mod telemetry_tests {
    use super::*;

    fn stats() -> RunStats {
        RunStats {
            scored: true,
            origins: vec![OriginStat {
                country: "us".into(),
                tier: 1,
                temperature: 1.0,
                volume: 12,
                articles: 3,
            }],
            ..RunStats::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_all_unknown_cycle_completes_empty_but_failures_do_not() {
        let mut coverage = CoverageStats {
            staged: 5,
            admitted: 0,
            discarded: 5,
            ..CoverageStats::default()
        };
        // Every staged article was a determined unknown: completed, empty.
        assert_eq!(cycle_state(&coverage, false), "completed");
        // A blocked classifier or a failed admission is never a completion.
        coverage.classification_blocked = 3;
        assert_eq!(cycle_state(&coverage, false), "blocked");
        coverage.classification_blocked = 0;
        coverage.admission_failed = 2;
        assert_eq!(cycle_state(&coverage, false), "failed");
        coverage.classified = 1;
        assert_eq!(cycle_state(&coverage, false), "partial");
        coverage.admission_failed = 0;
        assert_eq!(cycle_state(&coverage, false), "completed");
        assert_eq!(cycle_state(&coverage, true), "partial");
    }

    #[test]
    fn legacy_migration_steps_are_resume_safe_and_preserve_publication() {
        let migrated = AtlasArticleRow {
            categories: vec!["science".into(), "health".into()],
            admitted_at: "at".into(),
            ..Default::default()
        };
        assert_eq!(
            migration_step(&migrated, false, false),
            MigrationStep::SkipMigrated
        );
        let legacy = AtlasArticleRow {
            category: "information".into(),
            ..Default::default()
        };
        // A confirmed-unknown input loses its occurrence.
        assert_eq!(
            migration_step(&legacy, true, false),
            MigrationStep::DiscardConfirmedUnknown
        );
        // Published evidence keeps its identity across the migration.
        assert_eq!(
            migration_step(&legacy, false, true),
            MigrationStep::SkipPublished
        );
        // Unpublished legacy rows reclassify, and re-running is idempotent.
        assert_eq!(
            migration_step(&legacy, false, false),
            MigrationStep::Reclassify
        );
    }

    #[test]
    fn the_taxonomy_registers_the_new_scopes_and_ranked_picks() {
        // Science and Health replaced Information; retired labels miss.
        assert_eq!(normalize_category("  Science "), "science");
        assert_eq!(normalize_category("HEALTH"), "health");
        assert_eq!(normalize_category("information"), "unk");
        assert_eq!(normalize_category("sports"), "unk");
        assert_eq!(category_name("science"), "Science & Environment");
        assert_eq!(category_name("health"), "Health & Medicine");
        assert_eq!(category_tag("information"), "unk");
        let article = FeedArticle {
            id: "art".into(),
            title: "Joint military exercise begins".into(),
            description: "A live-fire drill followed the troop deployment.".into(),
            url: "https://example.com/drill".into(),
            country: "us".into(),
            source_name: "Wire".into(),
            source_domain: "example.com".into(),
            author: String::new(),
            image_url: String::new(),
            published_at: String::new(),
            provider: "newsapi".into(),
            temperature: 1.0,
            seen_at: String::new(),
            category: "unk".into(),
        };
        let (state, questions) = classification_request(&article);
        assert_eq!(state["title"], "Joint military exercise begins");
        // Rank 1 carries the full category criteria with unk as the miss;
        // later ranks offer `done` so tags are never padded.
        assert_eq!(questions["category"]["type"], "choice");
        assert!(questions["category"]["criteria"]["military"]
            .as_str()
            .unwrap()
            .contains("live-fire drill"));
        assert!(questions["category"]["criteria"]["science"]
            .as_str()
            .unwrap()
            .contains("space exploration"));
        assert!(questions["category"]["criteria"]["health"]
            .as_str()
            .unwrap()
            .contains("public health"));
        assert!(questions["category"]["criteria"]["unk"].is_string());
        assert!(questions["category"]["criteria"]
            .get("information")
            .is_none());
        assert!(questions["category_2"]["criteria"]["done"].is_string());
        assert!(questions["category_3"]["criteria"]["done"].is_string());
    }

    #[test]
    fn one_country_is_max_heat_and_the_bottom_band_is_dropped() {
        assert_eq!(band_sizes(1), (1, 0, 0, 0));
        assert_eq!(band_sizes(10), (2, 3, 3, 2));
        let mut counts = BTreeMap::new();
        for (index, code) in ["aa", "bb", "cc", "dd", "ee", "ff", "gg", "hh", "ii", "jj"]
            .into_iter()
            .enumerate()
        {
            counts.insert(code.into(), (10 - index) as u32);
        }
        let bands = score_counts(&counts);
        assert_eq!(bands.len(), 8);
        assert_eq!(bands[0].country, "aa");
        assert_eq!(bands[0].temperature, 1.0);
        assert_eq!(bands[1].temperature, 1.0);
        assert!((bands[2].temperature - 0.99).abs() < 0.001);
        assert!((bands[4].temperature - 0.70).abs() < 0.001);
        assert!((bands[5].temperature - 0.69).abs() < 0.001);
        assert!((bands[7].temperature - 0.10).abs() < 0.001);
        assert!(bands
            .iter()
            .all(|band| band.country != "ii" && band.country != "jj"));
    }

    #[test]
    fn empty_article_origins_are_pruned_after_collection() {
        let mut stats = RunStats {
            scored: true,
            origins: vec![
                OriginStat {
                    country: "us".into(),
                    tier: 1,
                    temperature: 1.0,
                    volume: 9,
                    articles: 4,
                },
                OriginStat {
                    country: "bd".into(),
                    tier: 3,
                    temperature: 0.2,
                    volume: 3,
                    articles: 0,
                },
            ],
            ..RunStats::default()
        };
        assert_eq!(stats.article_total(), 4);
        prune_empty_origins(&mut stats);
        assert_eq!(stats.origins.len(), 1);
        assert_eq!(stats.origins[0].country, "us");
    }

    #[test]
    fn dedup_ignores_a_better_source_when_the_prior_hit_is_outside_this_run() {
        // Simulate TTL history: a near-duplicate already stored under another run.
        let prior = FeedArticle {
            id: "prior".into(),
            title: "Export restriction widens now".into(),
            description: String::new(),
            url: "https://example.com/near".into(),
            country: "us".into(),
            source_name: "Example".into(),
            source_domain: "example.com".into(),
            author: String::new(),
            image_url: String::new(),
            published_at: "2026-10-02T00:00:00Z".into(),
            provider: "newsapi".into(),
            temperature: 0.5,
            seen_at: String::new(),
            category: "unk".into(),
        };
        let seen = vec![seen_of(&prior)];
        let better = FeedArticle {
            id: "d".into(),
            title: "Export restriction widens nows".into(),
            url: "https://www.reuters.com/world/other".into(),
            source_domain: "reuters.com".into(),
            source_name: "Reuters".into(),
            ..prior.clone()
        };
        // Judge still says Replace; ingest refuses when that id is not in this run.
        assert_eq!(judge(&better, &seen), Verdict::Replace("prior".into()));
    }

    #[test]
    fn dedup_drops_a_url_an_exact_title_and_keeps_the_higher_domain() {
        let reuters = FeedArticle {
            id: "a".into(),
            title: "Export restriction widens".into(),
            description: String::new(),
            url: "https://www.reuters.com/world/story".into(),
            country: "us".into(),
            source_name: "Reuters".into(),
            source_domain: "reuters.com".into(),
            author: String::new(),
            image_url: String::new(),
            published_at: "2026-10-01T00:00:00Z".into(),
            provider: "newsapi".into(),
            temperature: 1.0,
            seen_at: String::new(),
            category: "unk".into(),
        };
        let mut seen = vec![seen_of(&reuters)];
        let copy = FeedArticle {
            url: "https://www.reuters.com/world/story/".into(),
            ..reuters.clone()
        };
        assert_eq!(judge(&copy, &seen), Verdict::Drop);
        let syndicated = FeedArticle {
            id: "b".into(),
            title: "Export restriction widens".into(),
            url: "https://example.com/same".into(),
            source_domain: "example.com".into(),
            source_name: "Example".into(),
            ..reuters.clone()
        };
        assert_eq!(judge(&syndicated, &seen), Verdict::Drop);
        let near = FeedArticle {
            id: "c".into(),
            title: "Export restriction widens now".into(),
            url: "https://example.com/near".into(),
            source_domain: "example.com".into(),
            source_name: "Example".into(),
            published_at: "2026-10-02T00:00:00Z".into(),
            ..reuters.clone()
        };
        assert_eq!(judge(&near, &seen), Verdict::Drop);
        let better = FeedArticle {
            id: "d".into(),
            title: "Export restriction widens nows".into(),
            url: "https://www.reuters.com/world/other".into(),
            source_domain: "reuters.com".into(),
            source_name: "Reuters".into(),
            ..near.clone()
        };
        seen.push(seen_of(&near));
        assert_eq!(judge(&better, &seen), Verdict::Replace("c".into()));
    }

    #[test]
    fn gnews_and_newsdata_payloads_yield_country_codes() {
        let gnews = serde_json::json!({"articles":[
            {"title":"Drill","url":"https://example.com/a","source":{"name":"Times","country":"in"}},
            {"title":"No country","url":"https://example.com/b","source":{"name":"Times"}}
        ]});
        let hits = atlas_news::discovery_hits("gnews", &gnews).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].country, "in");
        let newsdata = serde_json::json!({"status":"success","results":[
            {"title":"Quota","link":"https://example.com/c","country":["us","United States"],"source_name":"Wire"}
        ]});
        let hits = atlas_news::discovery_hits("newsdata", &newsdata).unwrap();
        assert_eq!(hits[0].country, "us");
    }

    #[test]
    fn article_card_labels_title_publisher_author_and_classification() {
        let card = format_article_card(
            "Opinion: International flavour",
            "Joel Schlesinger; Laurie",
            "winnipegfreepress.com",
            "",
            "ca",
            "economic",
            "2026-10-03 07:01:24 +0000",
            0.69,
            "",
            "https://cdn.example/photo.jpg",
            "https://www.winnipegfreepress.com/story",
        );
        assert!(card.contains("Title: Opinion: International flavour"));
        assert!(card.contains("Publisher: winnipegfreepress.com"));
        assert!(card.contains("Author: Joel Schlesinger; Laurie"));
        assert!(card.contains("Category: Economic & Resource"));
        assert!(card.contains(&format!(
            "Published: {}",
            friendly_date("2026-10-03 07:01:24 +0000")
        )));
        assert!(card.contains("Temperature: 0.69"));
        assert!(card.contains("Image: https://cdn.example/photo.jpg"));
        assert!(card.contains("Article: https://www.winnipegfreepress.com/story"));
        assert!(!card.contains("Provider:"));
        let (publisher, author) =
            publisher_and_author("Jane Reporter", "reuters.com", "Jane Reporter");
        assert_eq!(publisher, "reuters.com");
        assert_eq!(author, "Jane Reporter");
        let (publisher, author) = publisher_and_author("Reuters", "reuters.com", "");
        assert_eq!(publisher, "Reuters");
        assert_eq!(author, "Reuters");
        let (publisher, author) = publisher_and_author("", "", "Jane Reporter");
        assert_eq!(publisher, "Jane Reporter");
        assert_eq!(author, "Jane Reporter");
        assert_eq!(friendly_date("2026-10-01"), "Oct 1, 2026");
        let stamp = chrono::DateTime::parse_from_rfc3339("2026-10-01T15:04:00Z").unwrap();
        let expected = loosen_date(
            &stamp
                .with_timezone(&chrono::Local)
                .format("%b %d, %Y, %I:%M %p")
                .to_string(),
        );
        assert_eq!(friendly_date("2026-10-01T15:04:00Z"), expected);
        assert!(expected.contains("AM") || expected.contains("PM"));
    }

    #[test]
    fn country_labels_show_the_name_and_the_code() {
        assert_eq!(country_label("us"), "United States (US)");
        assert_eq!(country_label("CN"), "China (CN)");
        assert_eq!(country_label("qa"), "Qatar (QA)");
        let card = format_run_card(&crate::store::AtlasRunRow {
            id: "atlas-1".into(),
            state: "completed".into(),
            phase: 2,
            cursor_json: String::new(),
            stats_json: serde_json::to_string(&RunStats {
                scored: true,
                origins: vec![OriginStat {
                    country: "de".into(),
                    tier: 1,
                    temperature: 1.0,
                    volume: 3,
                    articles: 2,
                }],
                ..RunStats::default()
            })
            .unwrap(),
            note: String::new(),
            started_at: "2026-10-01T00:00:00Z".into(),
            finished_at: String::new(),
        });
        let country = card.find("Germany (DE)").unwrap();
        let insights = card.find("No insights extracted for this cycle.").unwrap();
        assert!(card.contains("Articles 2"));
        assert!(insights < country);
    }

    fn keys() -> ProviderKeys {
        ProviderKeys {
            gnews: "gnews-key".into(),
            newsdata: "newsdata-key".into(),
            currents: "currents-key".into(),
            newsapi: "newsapi-key".into(),
            ..ProviderKeys::default()
        }
    }

    fn reply_for(call: &HttpCall) -> HttpReply {
        let body = if call.provider == "gnews" || call.provider == "newsdata" {
            let code = ["us", "cn", "de", "fr", "gb", "in", "jp"][call.url.len() % 7];
            if call.provider == "gnews" {
                serde_json::json!({"articles":[{"title":"Story","url":"https://example.com/a","source":{"name":"Wire","country": code}}]}).to_string()
            } else {
                serde_json::json!({"status":"success","results":[{"title":"Story","link":"https://example.com/b","country":[code],"source_name":"Wire"}]}).to_string()
            }
        } else if call.provider == "newsapi" {
            serde_json::json!({"status":"ok","articles":[{"title":"Local","url":"https://www.reuters.com/local","source":{"name":"Reuters"},"publishedAt":"2026-10-01T00:00:00Z"}]}).to_string()
        } else {
            serde_json::json!({"status":"ok","news":[{"title":"Local wire","url":"https://example.com/local","author":"Desk","published":"2026-10-01T01:00:00Z"}]}).to_string()
        };
        HttpReply {
            status: 200,
            body,
            location: None,
        }
    }

    /// Top headlines with no `source.country`, and a NewsData row named Germany.
    fn regional_reply(call: &HttpCall) -> HttpReply {
        let body = if call.provider == "gnews" {
            serde_json::json!({"articles":[{
                "title":"Cabinet meets",
                "url":"https://www.reuters.com/world/story",
                "source":{"name":"Reuters","url":"https://www.reuters.com"}
            }]})
            .to_string()
        } else if call.provider == "newsdata" {
            serde_json::json!({"status":"success","results":[{
                "title":"Berlin wire",
                "link":"https://example.com/berlin",
                "country":["Germany"],
                "source_name":"Wire"
            }]})
            .to_string()
        } else {
            let code = url::Url::parse(&call.url)
                .ok()
                .and_then(|url| {
                    url.query_pairs()
                        .find(|(key, _)| key == "country")
                        .map(|(_, value)| value.to_ascii_lowercase())
                })
                .unwrap_or_else(|| "xx".into());
            if call.provider == "newsapi" {
                serde_json::json!({"status":"ok","articles":[{
                    "title": format!("Local {code}"),
                    "url": format!("https://www.reuters.com/{code}"),
                    "source":{"name":"Reuters"},
                    "publishedAt":"2026-10-01T00:00:00Z"
                }]})
                .to_string()
            } else {
                serde_json::json!({"status":"ok","news":[{
                    "title": format!("Desk {code}"),
                    "url": format!("https://example.com/{code}"),
                    "author":"Desk",
                    "published":"2026-10-01T01:00:00Z"
                }]})
                .to_string()
            }
        };
        HttpReply {
            status: 200,
            body,
            location: None,
        }
    }
}

#[cfg(test)]
mod acquisition_tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn spec(url: &str) -> CallSpec {
        CallSpec {
            provider: "gnews",
            url: url.into(),
            headers: Vec::new(),
        }
    }

    fn scripted(
        replies: Vec<Result<HttpReply>>,
        seen: Arc<Mutex<Vec<String>>>,
    ) -> impl FnMut(HttpCall) -> FetchFut {
        let queue = Arc::new(Mutex::new(
            replies
                .into_iter()
                .collect::<std::collections::VecDeque<_>>(),
        ));
        move |call: HttpCall| {
            seen.lock().unwrap().push(call.url.clone());
            let next = queue.lock().unwrap().pop_front().expect("scripted reply");
            Box::pin(async move { next })
        }
    }

    fn reply(status: u16, body: &str, location: Option<&str>) -> Result<HttpReply> {
        Ok(HttpReply {
            status,
            body: body.into(),
            location: location.map(str::to_string),
        })
    }

    #[tokio::test]
    async fn one_server_error_is_retried_before_it_counts_as_a_failure() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut fetch = scripted(
            vec![
                reply(503, "unavailable", None),
                reply(200, "{\"articles\":[]}", None),
            ],
            seen.clone(),
        );
        let value = dispatch(&mut fetch, spec("https://gnews.io/api/v4/x"), "ua").await;
        assert!(value.is_ok(), "{value:?}");
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_transport_error_is_retried_once_and_then_named() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut fetch = scripted(
            vec![
                Err(anyhow::anyhow!("connection reset")),
                Err(anyhow::anyhow!("connection reset")),
            ],
            seen.clone(),
        );
        let fault = dispatch(&mut fetch, spec("https://gnews.io/x"), "ua")
            .await
            .unwrap_err();
        assert!(
            fault
                .summary
                .starts_with("gnews request failed after 2 attempts"),
            "{}",
            fault.summary
        );
    }

    #[tokio::test]
    async fn a_same_host_redirect_is_followed_and_a_foreign_one_is_named() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut fetch = scripted(
            vec![reply(301, "", Some("/api/v5/x")), reply(200, "{}", None)],
            seen.clone(),
        );
        assert!(
            dispatch(&mut fetch, spec("https://gnews.io/api/v4/x"), "ua")
                .await
                .is_ok()
        );
        assert_eq!(seen.lock().unwrap()[1], "https://gnews.io/api/v5/x");

        let mut fetch = scripted(
            vec![reply(302, "", Some("https://evil.example/x"))],
            Arc::new(Mutex::new(Vec::new())),
        );
        let fault = dispatch(&mut fetch, spec("https://gnews.io/api/v4/x"), "ua")
            .await
            .unwrap_err();
        assert!(
            fault
                .summary
                .contains("redirect to another host was not followed"),
            "{}",
            fault.summary
        );
    }

    #[test]
    fn a_quota_limit_is_not_an_acquisition_failure_but_a_real_fault_is_named() {
        let mut stats = RunStats::default();
        let quota = ProviderFault {
            summary: "gnews HTTP 429: daily quota exceeded".into(),
            body: String::new(),
        };
        assert!(!record_fault(&mut stats, "gnews", &quota));
        assert_eq!(stats.coverage.quota_skipped, 1);
        assert_eq!(stats.coverage.acquisition_failures, 0);
        let bad = ProviderFault {
            summary: "newsdata malformed JSON: eof".into(),
            body: String::new(),
        };
        assert!(record_fault(&mut stats, "newsdata", &bad));
        assert_eq!(stats.coverage.acquisition_failures, 1);
        assert_eq!(
            stats.acquisition_faults,
            vec!["newsdata malformed JSON: eof".to_string()]
        );
    }
}
