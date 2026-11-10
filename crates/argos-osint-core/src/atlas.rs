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
use crate::classifier::readiness::{
    self, ClassifierHome, CyclePermit, CyclePreflightError, CycleRequirement,
};
use crate::osint::atlas_news::{self, Hit};
use crate::osint::ProviderKeys;
use crate::secrets::ProviderSecret;
use crate::store::{AtlasArticleRow, AtlasRunRow};
use crate::telemetry::{self, EventKind, TelemetryEvent, Trigger};

/// Country-coded headlines collected from each phase-1 source.
const PHASE1_TARGET: u32 = 50;
/// Pages per source. Ten articles a page, so five pages cover the target.
const PHASE1_MAX_PAGES: usize = 5;

/// Current classifier facets. Historical labels never establish membership.
pub const CATEGORY_IDS: &[&str] = &crate::atlas_taxonomy::DOMAIN_IDS;
pub use crate::atlas_taxonomy::TAXONOMY_VERSION;
pub const MIN_ADMISSION_TAGS: usize = 1;
pub const MAX_ADMISSION_TAGS: usize = 2;
pub fn category_tag(id: &str) -> &'static str {
    crate::atlas_taxonomy::DomainId::parse(id)
        .map(crate::atlas_taxonomy::DomainId::as_str)
        .unwrap_or("unk")
}
pub fn category_name(id: &str) -> &'static str {
    crate::atlas_taxonomy::DomainId::parse(id)
        .map(crate::atlas_taxonomy::DomainId::display_name)
        .unwrap_or("unk")
}
pub fn normalize_category(raw: &str) -> String {
    let value = raw.trim().to_ascii_lowercase();
    category_tag(&value).into()
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
    provider_counts: ProviderCallCounts,
    classifier_metrics: serde_json::Value,
    classifier_queue_micros: Option<u64>,
    repo: Option<crate::storage::repo::Repository>,
    classifier_db_start: Option<ClassifierDbSample>,
    classifier_read: u64,
    classifier_claimed: u64,
    classifier_dispositions: std::collections::BTreeMap<String, u64>,
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
            provider_counts: ProviderCallCounts::default(),
            classifier_metrics: serde_json::Value::Null,
            classifier_queue_micros: Some(0),
            repo: None,
            classifier_db_start: None,
            classifier_read: 0,
            classifier_claimed: 0,
            classifier_dispositions: std::collections::BTreeMap::new(),
        }
    }

    /// [`CycleTelemetry::new`] on the selected store: under SurrealDB the
    /// candidate and stage numbering continue from the repository, so a
    /// resumed cycle appends instead of overwriting earlier ids.
    async fn open(
        store: &AtlasStore,
        db: &Path,
        run_id: &str,
        mode: &str,
        started_at: String,
    ) -> Self {
        let mut tel = Self::new(db, run_id, mode, started_at);
        tel.repo = store.repository().cloned();
        #[cfg(feature = "surreal")]
        if crate::storage::runtime::legacy_side_store_path(db).is_none() && !run_id.is_empty() {
            let mut seqs = std::collections::HashMap::new();
            if let Some(repo) = tel.repo.as_ref() {
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

    fn record_event(&self, event: &TelemetryEvent) {
        if let Some(repo) = &self.repo {
            let repo = repo.clone();
            let event = event.clone();
            crate::storage::runtime::spawn(async move {
                let _ = crate::storage::telemetry::record(repo.db(), &event).await;
            });
        } else {
            let _ = telemetry::record(&self.db, event);
        }
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
                "metrics":self.classifier_metrics,
                "provider_counts":self.provider_counts,
            }));
        let event = event.with_id(format!(
            "atlas-stage-{}-{}-{}",
            self.run_id, open.name, open.seq
        ));
        self.record_event(&event);
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
        self.record_event(&event);
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
            self.record_event(&event);
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
                "classifier_snapshot":stats.classifier_snapshot,
                "metrics":self.classifier_metrics,
                "provider_counts":self.provider_counts,
            }));
        let event = event.with_id(format!("atlas-cycle-{}", self.run_id));
        self.record_event(&event);
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
        "oversized_inputs": stats.coverage.oversized_inputs,
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
    #[serde(default)]
    pub actor_readiness: Option<crate::storage::atlas_derivative_status::StageReadiness>,
    #[serde(default)]
    pub reconciliation_readiness: Option<crate::storage::atlas_derivative_status::StageReadiness>,
    #[serde(default)]
    pub derivative_progress: Option<crate::storage::atlas_derivative_status::Summary>,

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
    /// Provider items excluded before staging because metadata exceeds bounds.
    #[serde(default)]
    pub oversized_inputs: u32,
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

impl CoverageStats {
    // Reconcile exact run-local counts rather than accumulating attempts:
    // retries/replaced receipts and resume must not inflate live coverage.
    fn apply_admission_summary(
        &mut self,
        summary: &crate::storage::classifier_admission::RunAdmissionSummary,
    ) {
        let count = |value: u64| u32::try_from(value).unwrap_or(u32::MAX);
        self.articles = count(summary.retained);
        self.classified = count(summary.admitted);
        self.admitted = count(summary.admitted);
        self.discarded = count(summary.rejected);
        self.admission_pending = count(summary.staged.saturating_sub(summary.failed));
        self.admission_failed = count(summary.failed);
        self.legacy_unknown = count(summary.legacy);
    }

    fn apply_native_summary(
        &mut self,
        summary: &crate::storage::classifier_admission::NativeRunSummary,
    ) {
        let count = |value: u64| u32::try_from(value).unwrap_or(u32::MAX);
        self.entities_available = count(summary.entities);
        self.publishers_available = count(summary.publishers);
        // Pending metadata has no unavailable verdict yet. Once it settles,
        // use the same retained cohort as terminal publisher coverage.
        self.publishers_unavailable = if summary.earliest_phase.is_none_or(|phase| phase > 4) {
            self.articles.saturating_sub(self.publishers_available)
        } else {
            0
        };
        self.actor_indexed = count(summary.actors);
        self.related_pairs = count(summary.related);
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RunStats {
    #[serde(default)]
    pub contract_version: String,
    #[serde(default)]
    pub coverage: CoverageStats,
    #[serde(default)]
    pub acquisition_retry: Option<Cursor>,
    /// Immutable local model selected before this run may acquire or admit articles.
    #[serde(default)]
    pub classifier_snapshot: Option<crate::classifier::ModelSnapshot>,
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

/// A provider rejection. Both fields contain bounded code/status diagnostics;
/// untrusted response text and credential-bearing URLs are never reflected.
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
    Stats(Box<RunStats>),
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

struct ReservedProvider<'a> {
    store: &'a AtlasStore,
    provider: &'a str,
    key: &'a str,
    day: String,
    spare: Option<&'a str>,
}

/// Reserve the primary account or, if it cannot send, the distinct fallback.
/// One reservation covers transport retries and same-host redirects.
async fn reserve_provider<'a>(
    store: &'a AtlasStore,
    provider: &'a str,
    primary: &'a str,
    fallback: &'a str,
) -> Result<Option<ReservedProvider<'a>>> {
    let primary = primary.trim();
    let fallback = fallback.trim();
    let spare =
        (!fallback.is_empty() && fallback != primary && !crate::osint::key_exhausted(fallback))
            .then_some(fallback);
    let day = utc_day();
    let cap = daily_cap(provider);
    if !primary.is_empty()
        && !crate::osint::key_exhausted(primary)
        && store.reserve_quota(provider, &day, cap, None).await?
    {
        return Ok(Some(ReservedProvider {
            store,
            provider,
            key: primary,
            day,
            spare,
        }));
    }
    if let Some(key) = spare {
        let bucket = format!("{provider}:fallback");
        if store.reserve_quota(&bucket, &day, cap, None).await? {
            return Ok(Some(ReservedProvider {
                store,
                provider,
                key,
                day,
                spare: None,
            }));
        }
    }
    Ok(None)
}

fn quota_fault(fault: &ProviderFault) -> bool {
    crate::osint::quota_limited("failed", Some(&fault.summary))
}

struct SpareFetch {
    body: Option<serde_json::Value>,
    fault: Option<ProviderFault>,
    metrics: ProviderCallCounts,
}

/// One logical provider call. A rate or quota fault can move the reservation
/// to the fallback before the fallback's first send.
async fn fetch_with_spare_key<F>(
    fetch: &mut F,
    user_agent: &str,
    reservation: ReservedProvider<'_>,
    mut spec_for: impl FnMut(&str) -> Result<CallSpec>,
) -> Result<SpareFetch>
where
    F: FnMut(HttpCall) -> FetchFut,
{
    let ReservedProvider {
        store,
        provider,
        key: mut active,
        day,
        mut spare,
    } = reservation;
    let mut metrics = ProviderCallCounts::default();
    loop {
        let spec = match spec_for(active) {
            Ok(spec) => spec,
            Err(_) => {
                return Ok(SpareFetch {
                    body: None,
                    fault: Some(ProviderFault {
                        summary: format!("{provider} request configuration invalid"),
                        body: String::new(),
                    }),
                    metrics,
                });
            }
        };
        match dispatch_counted(fetch, spec, user_agent, &mut metrics).await {
            Ok(body) => {
                return Ok(SpareFetch {
                    body: Some(body),
                    fault: None,
                    metrics,
                });
            }
            Err(fault) => {
                if let Some(next) = spare.take().filter(|_| quota_fault(&fault)) {
                    crate::osint::note_key_exhausted(active);
                    let bucket = format!("{provider}:fallback");
                    if !crate::osint::key_exhausted(next)
                        && store
                            .reserve_quota(
                                &bucket,
                                &utc_day(),
                                daily_cap(provider),
                                Some((provider, &day)),
                            )
                            .await?
                    {
                        active = next;
                        metrics.fallbacks += 1;
                        continue;
                    }
                }
                return Ok(SpareFetch {
                    body: None,
                    fault: Some(fault),
                    metrics,
                });
            }
        }
    }
}

/// Capture provider labels before country/window/admission filtering. Storage
/// failures are diagnostic: calibration cannot change the collection outcome.
async fn capture_provider_observations(
    store: &AtlasStore,
    run_id: &str,
    provider: &str,
    fetch_key: &str,
    body: &serde_json::Value,
    emit: &mut impl FnMut(AtlasEvent),
) -> HashMap<String, Vec<String>> {
    use crate::classifier::provider_labels::ProviderFormat;
    let (format, endpoint, array) = match provider {
        "newsdata" => (
            ProviderFormat::NewsDataV1,
            "https://newsdata.io/api/1/latest",
            "results",
        ),
        "currents" => (
            ProviderFormat::CurrentsV1,
            "https://api.currentsapi.services/v1/latest-news",
            "news",
        ),
        _ => return HashMap::new(),
    };
    let Some(owner) = store.admission_owner() else {
        return HashMap::new();
    };
    let context = crate::classifier::provider_capture::CaptureContext {
        cycle_id: run_id.into(),
        fetched_at: chrono::Utc::now().to_rfc3339(),
        fetch_receipt_key: fetch_key.into(),
        request_input_hash: crate::classifier::artifact::sha256_hex(fetch_key.as_bytes()),
    };
    let count = body
        .get(array)
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    let mut references: HashMap<String, Vec<String>> = HashMap::new();
    for offset in (0..count.max(1)).step_by(crate::classifier::INFERENCE_BATCH_SIZE) {
        let page = crate::classifier::provider_capture::capture_categories_page(
            format.clone(),
            endpoint,
            body,
            &context,
            offset,
        );
        let rejected = page.rejections.len();
        if crate::storage::classifier_calibration::capture_owned(
            owner,
            &context,
            &page.observations,
        )
        .await
        .is_err()
        {
            emit(AtlasEvent::Note(
                "Provider calibration capture unavailable; collection continues".into(),
            ));
            continue;
        }
        for observation in page.observations {
            references
                .entry(observation.url)
                .or_default()
                .push(observation.observation_id);
        }
        if rejected > 0 {
            emit(AtlasEvent::Note(format!(
                "Provider calibration capture: {rejected} metadata exclusions"
            )));
        }
    }
    for ids in references.values_mut() {
        ids.sort();
        ids.dedup();
    }
    references
}

fn oversized_hit(hit: &Hit) -> bool {
    hit.title
        .len()
        .checked_add(hit.description.len())
        .is_none_or(|bytes| bytes > crate::classifier::MAX_INPUT_TEXT_BYTES)
        || hit.url.len() > 4096
        || hit.image_url.len() > 4096
        || hit.published_at.len() > 256
        || hit.language.as_ref().is_some_and(|language| {
            language.is_empty() || language.len() > 128 || language.chars().any(char::is_control)
        })
}

async fn stage_hit(
    store: &AtlasStore,
    row: &AtlasArticleRow,
    revision: &str,
    hit: &Hit,
    observations: &HashMap<String, Vec<String>>,
) -> Result<bool> {
    use crate::storage::classifier_admission;
    let provenance = classifier_admission::StageProvenance::new(
        hit.language.clone(),
        observations.get(&hit.url).cloned().unwrap_or_default(),
    )?;
    Ok(classifier_admission::stage_article_with_provenance(
        store
            .admission_owner()
            .ok_or_else(|| anyhow!("Atlas run owner is missing"))?,
        store
            .run_claim()
            .ok_or_else(|| anyhow!("Atlas run claim is missing"))?,
        row,
        revision,
        &provenance,
        chrono::Utc::now(),
    )
    .await?)
}

async fn apply_hits(
    hits: &[Hit],
    observations: &HashMap<String, Vec<String>>,
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
    let repo = store
        .repository()
        .ok_or_else(|| anyhow!("SurrealDB storage is not configured"))?;
    let mut retained = HashMap::new();
    let ids: Vec<_> = hits
        .iter()
        .map(|hit| format!("art-{}", url_hash(&hit.url)))
        .collect();
    for ids in ids.chunks(64) {
        retained.extend(crate::storage::atlas::retained_identities(repo.db(), run_id, ids).await?);
    }
    for hit in hits {
        let temperature = stats
            .origins
            .iter()
            .find(|row| row.country == hit.country)
            .map(|row| row.temperature)
            .unwrap_or(0.0);
        let article = article_from(hit, provider, temperature);
        if oversized_hit(hit) {
            stats.coverage.oversized_inputs = stats.coverage.oversized_inputs.saturating_add(1);
            tel.candidate(&article, provider, "metadata_too_large", None, None);
            emit(AtlasEvent::Note(
                "Provider item excluded: metadata exceeds the local input bound".into(),
            ));
            continue;
        }

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
                if stage_hit(store, &row, &revision, hit, observations).await? {
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
                if stage_hit(store, &row, &revision, hit, observations).await? {
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
    /// Legacy host configuration field. Local admission uses the pinned classifier.
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
    let result = run_atlas_inner(input, tracked_emit, fetch, &tracker, None).await;
    // Read the run id first: the tracker guard is never held across an await.
    let tracked_run = tracker
        .lock()
        .ok()
        .map(|job| job.run_id.clone())
        .filter(|id| !id.is_empty());
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
            if err.downcast_ref::<CyclePreflightError>().is_some()
                || run_state().is_some_and(|(state, _)| state == "blocked")
            {
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

pub(crate) fn validate_resume_state(
    cursor: &Cursor,
    stats: &RunStats,
) -> std::result::Result<(), CyclePreflightError> {
    let invalid = |code: &str| CyclePreflightError::InvalidRunState { code: code.into() };
    if !(1..=6).contains(&cursor.phase)
        || cursor.leg.len() > 128
        || cursor.leg.chars().any(char::is_control)
        || (!stats.contract_version.is_empty()
            && !crate::atlas_work::is_collection_contract(&stats.contract_version))
    {
        return Err(invalid("unsupported_cursor_or_collection_contract"));
    }
    for position in std::iter::once(cursor).chain(stats.acquisition_retry.iter()) {
        if position.phase <= 2 {
            let valid = match position.phase {
                1 => {
                    matches!(position.leg.as_str(), "gnews" | "newsdata")
                        && position.chunk <= PHASE1_MAX_PAGES
                        && position.kept <= PHASE1_TARGET
                        && position.country == 0
                        && position.page_token.len() <= 64 * 1024
                }
                2 => {
                    matches!(position.leg.as_str(), "newsapi" | "currents")
                        && position.country <= stats.origins.len()
                        && position.chunk == 0
                        && position.page_token.is_empty()
                }
                _ => false,
            };
            if !valid {
                return Err(invalid("invalid_acquisition_cursor"));
            }
        }
    }
    if stats
        .acquisition_retry
        .as_ref()
        .is_some_and(|retry| !(1..=2).contains(&retry.phase))
    {
        return Err(invalid("invalid_acquisition_retry_phase"));
    }
    Ok(())
}

fn decode_resume_state(
    run: &AtlasRunRow,
) -> std::result::Result<(Cursor, RunStats), CyclePreflightError> {
    let cursor = serde_json::from_str(&run.cursor_json).map_err(|_| {
        CyclePreflightError::InvalidRunState {
            code: "invalid_cursor_json".into(),
        }
    })?;
    let stats = serde_json::from_str(&run.stats_json).map_err(|_| {
        CyclePreflightError::InvalidRunState {
            code: "invalid_stats_json".into(),
        }
    })?;
    validate_resume_state(&cursor, &stats)?;
    Ok((cursor, stats))
}

fn cycle_requirement(
    run_id: Option<&str>,
    cursor: &Cursor,
    stats: &RunStats,
    live_staging: bool,
) -> CycleRequirement {
    if run_id.is_none() {
        CycleRequirement::FreshAcquisition
    } else if cursor.phase <= 2 || stats.acquisition_retry.is_some() {
        CycleRequirement::AcquisitionAndAdmission
    } else if cursor.phase == 3 || live_staging {
        CycleRequirement::AdmissionOnly
    } else {
        CycleRequirement::PostAdmissionOnly
    }
}

/// @tag:classifier @tag:atlas-collection
/// Before-transition preflight: U6 must preserve this boundary and consume its
/// returned pin rather than resolving active models independently per batch.
async fn classifier_cycle_preflight(
    store: &AtlasStore,
    home: &ClassifierHome,
    run_id: Option<&str>,
    cursor: &Cursor,
    stats: &RunStats,
) -> std::result::Result<CyclePermit, CyclePreflightError> {
    let live_staging = if let Some(run_id) = run_id {
        let repo = store
            .repository()
            .ok_or_else(|| CyclePreflightError::Classifier {
                status: crate::classifier::ClassifierStatus::Unavailable,
                code: "storage_unavailable".into(),
                source: crate::classifier::ClassifierError::Storage(
                    "Atlas repository unavailable".into(),
                ),
            })?;
        crate::storage::atlas::admission_pending_exists(repo.db(), run_id)
            .await
            .map_err(|error| CyclePreflightError::Classifier {
                status: crate::classifier::ClassifierStatus::Unavailable,
                code: "admission_probe_failed".into(),
                source: crate::classifier::ClassifierError::Storage(error.to_string()),
            })?
    } else {
        false
    };
    readiness::preflight_cycle(
        home,
        cycle_requirement(run_id, cursor, stats, live_staging),
        stats.classifier_snapshot.as_ref(),
    )
    .await
}

async fn run_atlas_inner<F>(
    input: RunInput<'_>,
    emit: impl FnMut(AtlasEvent) + Send,
    fetch: F,
    tracker: &std::sync::Mutex<AtlasJob>,
    dispatch_id: Option<&str>,
) -> Result<Stop>
where
    F: FnMut(HttpCall) -> FetchFut + Send,
{
    let store = AtlasStore::open(input.db_path).await?;
    run_atlas_on_store(input, emit, fetch, tracker, store, dispatch_id).await
}

async fn run_atlas_on_store<F>(
    input: RunInput<'_>,
    mut emit: impl FnMut(AtlasEvent) + Send,
    mut fetch: F,
    tracker: &std::sync::Mutex<AtlasJob>,
    store: AtlasStore,
    dispatch_id: Option<&str>,
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
        classifier: _,
        synthesizer: _,
        synthesizer_fallbacks: _,
        run_id: resume_id,
    } = input;
    use crate::storage::classifier_admission as admission;
    let repo = store
        .repository()
        .ok_or_else(|| anyhow!("SurrealDB storage is not configured"))?;
    let mut dispatch_revision = None;
    let (run_id, mut cursor, mut stats, resume) = if let Some(id) = dispatch_id {
        // Read the revision before its checkpoint. A concurrent write after
        // this read invalidates the guarded startup instead of rolling back
        // the cursor or reopening a completed run.
        let revision = admission::run_revision(repo, id).await?;
        let saved = crate::storage::atlas::run_by_id(repo.db(), id).await?;
        if saved.as_ref().is_some_and(|run| run.state == "completed") {
            return Ok(Stop::Finished);
        }
        if revision.lease_active {
            return Err(admission::AdmissionError::LeaseLost.into());
        }
        dispatch_revision = Some(revision);
        match saved {
            Some(run) if run.state == "running" || resumable(&run) => {
                let (cursor, stats) = decode_resume_state(&run)?;
                (run.id, cursor, stats, true)
            }
            Some(_) => {
                return Ok(Stop::Failed(
                    "The dispatched Atlas run is not resumable".into(),
                ))
            }
            None => (
                id.to_owned(),
                Cursor {
                    from: lookback_from(),
                    ..Cursor::default()
                },
                RunStats::default(),
                false,
            ),
        }
    } else if resume {
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
        let (cursor, stats) = decode_resume_state(&run)?;
        if !resumable(&run) {
            return Ok(Stop::Failed("The latest Atlas run is not paused".into()));
        }
        (run.id, cursor, stats, true)
    } else {
        let cursor = Cursor {
            from: lookback_from(),
            ..Cursor::default()
        };
        (new_run_id(), cursor, RunStats::default(), false)
    };
    let home_path = std::path::absolute(db_path)?
        .parent()
        .ok_or_else(|| CyclePreflightError::InvalidRunState {
            code: "database_home_missing".into(),
        })?
        .to_path_buf();
    let classifier_home = ClassifierHome::new(home_path);
    let owner_id = format!("atlas-owner-{}", crate::store::new_id());
    let db_owner = crate::storage::classifier_admission::AdmissionOwner::new(repo.clone());
    let intent = crate::storage::classifier_admission::announce_foreground_intent(
        &db_owner,
        &owner_id,
        chrono::Utc::now(),
    )
    .await?;
    let handoff = std::time::Duration::from_secs(
        crate::storage::classifier_admission::FOREGROUND_HANDOFF_SECS,
    );
    // A fresh CPU worker can spend longer loading the checkpoint than the
    // legacy QuickJS handoff. The foreground intent still renews every second.
    #[cfg(feature = "classifier")]
    let handoff = if !resume
        || stats.classifier_snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.algorithm_version == crate::classifier::ZERO_SHOT_ALGORITHM_VERSION
        }) {
        handoff + crate::classifier::bert::STARTUP_TIMEOUT
    } else {
        handoff
    };
    let deadline = tokio::time::Instant::now() + handoff;
    let startup: Result<(CyclePermit, crate::storage::classifier_admission::RunClaim)> = async {
        for attempt in 0..3 {
            let revision = match dispatch_revision.take() {
                Some(revision) => revision,
                None => admission::run_revision(repo, &run_id).await?,
            };
            let permit = foreground_classifier_preflight(
                &store,
                &classifier_home,
                resume.then_some(run_id.as_str()),
                &cursor,
                &stats,
                &db_owner,
                &intent,
                deadline,
            )
            .await?;
            if let Some(pinned) = permit.pinned() {
                stats.classifier_snapshot = Some(pinned.snapshot.clone());
            }
            let cursor_json = serde_json::to_string(&cursor)?;
            let stats_json = serde_json::to_string(&stats)?;
            let request = crate::storage::classifier_admission::StartRunRequest::fresh(
                &run_id,
                &owner_id,
                &cursor_json,
                &stats_json,
                chrono::Utc::now(),
            );
            let request = if resume {
                request.resume(revision.revision, revision.work_generation)
            } else {
                request
            };
            match crate::storage::classifier_admission::start_run(&db_owner, &permit, &request)
                .await
            {
                Ok(claim) => return Ok((permit, claim)),
                Err(crate::storage::classifier_admission::AdmissionError::InvalidRunState {
                    code,
                }) if code == "preflight_stale"
                    && resume
                    && dispatch_id.is_none()
                    && attempt < 2 =>
                {
                    // Re-read and re-preflight the selected run. A stale permit
                    // is never reused against a changed checkpoint/cohort.
                    let selected = crate::storage::atlas::run_by_id(repo.db(), &run_id)
                        .await?
                        .ok_or_else(|| {
                            anyhow!("selected Atlas run disappeared during preflight")
                        })?;
                    if !resumable(&selected) {
                        return Err(CyclePreflightError::InvalidRunState {
                            code: "selected_run_no_longer_resumable".into(),
                        }
                        .into());
                    }
                    (cursor, stats) = decode_resume_state(&selected)?;
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(
            crate::storage::classifier_admission::AdmissionError::InvalidRunState {
                code: "preflight_stale".into(),
            }
            .into(),
        )
    }
    .await;
    let clear =
        crate::storage::classifier_admission::clear_foreground_intent(&db_owner, &intent).await;
    let (permit, claim) = match (startup, clear) {
        (Ok(pair), Ok(())) => pair,
        (result, cleared) => {
            if let Ok((_, claim)) = &result {
                let failed = store.clone().with_claim(db_owner.clone(), claim.clone());
                let _ = failed
                    .set_state(&run_id, "failed", "Foreground handoff cleanup failed", true)
                    .await;
            }
            let error = match cleared {
                Err(error) => anyhow!(error),
                Ok(()) => result
                    .err()
                    .ok_or_else(|| anyhow!("missing startup error"))?,
            };
            emit(AtlasEvent::Note(error.to_string()));
            let owner_cleanup = db_owner.shutdown().await;
            let home_cleanup = classifier_home.shutdown().await;
            if let Err(cleanup) = owner_cleanup {
                return Err(cleanup.into());
            }
            if let Err(cleanup) = home_cleanup {
                return Err(cleanup.into());
            }
            return Err(error);
        }
    };
    let store = store.with_claim(db_owner.clone(), claim.clone());
    let mut work=Box::pin(db_owner.scope(async {
    let repo=store.repository().ok_or_else(||anyhow!("SurrealDB storage is not configured"))?;
    if resume && cursor.phase <= 2 {
        stats.coverage.acquisition_failures = 0;
        stats.coverage.quota_skipped = 0;
        stats.acquisition_faults.clear();
        stats.acquisition_retry = None;
    }
    stats.contract_version = crate::atlas_work::COLLECTION_CONTRACT.into();
    save(&store, &run_id, &cursor, &stats).await?;
    if dispatch_id.is_none() {
        if let Ok(mut job) = tracker.lock() {
            job.start(&run_id);
        }
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
    let mut tel = CycleTelemetry::open(&store,db_path, &run_id, cycle_mode, cycle_started_at).await;
    if cursor.phase <= 1 {
        require_either(&keys.gnews, &keys.gnews_fallback, "GNews API key")?;
        require_either(&keys.newsdata, &keys.newsdata_fallback, "NewsData API key")?;
    }
    let mut seen: Vec<Seen> = feed.iter().map(seen_of).collect();
    // Seed thin history keys newest-first. Full descriptions, metadata and
    // staged payloads never enter the historical dedup read.
    let mut after:Option<(String,String,String)>=None;
    loop {
        let page=crate::storage::atlas::dedup_keys_page(repo.db(),after.as_ref().map(|(seen,id,run)|(seen.as_str(),id.as_str(),run.as_str()))).await?;
        if page.is_empty() {break;}
        for key in &page {
            if seen.iter().all(|item|item.id!=key.article_id) {
                seen.push(Seen {url_hash:url_hash(&key.url),title_norm:normalize_title(&key.title),domain:key.source_domain.clone(),rank:domain_rank(&key.source_domain),published_at:key.published_at.clone(),id:key.article_id.clone()});
            }
        }
        let last=page.last().ok_or_else(||anyhow!("dedup page missing final key"))?;
        after=Some((last.seen_at.clone(),last.article_id.clone(),last.run_id.clone()));
        if page.len()<64 {break;}
    }
    let mut last_gnews: Option<std::time::Instant> = None;
    let user_agent = if user_agent.trim().is_empty() {
        crate::osint::DEFAULT_USER_AGENT.to_string()
    } else {
        user_agent.to_string()
    };


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
                if pace && provider == "gnews" {
                    if let Some(last) = last_gnews {
                        let wait = std::time::Duration::from_secs(1).saturating_sub(last.elapsed());
                        if !wait.is_zero() {
                            tokio::time::sleep(wait).await;
                        }
                    }
                }
                let Some(reservation) = reserve_provider(&store, provider, primary, fallback).await? else {
                    emit(AtlasEvent::Note(format!("{provider} daily quota is spent")));
                    break;
                };
                emit(AtlasEvent::Status(format!(
                    "Latest {provider} page {} · {kept}/{PHASE1_TARGET}",
                    page + 1
                )));
                let mut stop_provider = false;
                let mut acquisition_ok = false;
                let fetched = fetch_with_spare_key(
                    &mut fetch,
                    &user_agent,
                    reservation,
                    |active| phase1_call(provider, page, &token, active),
                )
                .await?;
                tel.provider_calls(&fetched.metrics);
                match fetched.body {
                    Some(body) => {
                        if provider == "gnews" {
                            last_gnews = Some(std::time::Instant::now());
                        }
                        let observations = capture_provider_observations(&store, &run_id, provider, &receipt_key, &body, &mut emit).await;
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
                                    &hits, &observations, provider, &mut stats, &mut seen, &mut emit, &store,
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
                            Err(_) => {
                                let fault = ProviderFault {
                                    summary: format!("{provider} response schema invalid"),
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
                page += 1;
                emit(AtlasEvent::Stats(Box::new(stats.clone())));
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
        emit(AtlasEvent::Stats(Box::new(stats.clone())));
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
            let Some(reservation) = reserve_provider(&store, job.provider, primary, fallback).await? else {
                emit(AtlasEvent::Note(format!(
                    "{} daily quota is spent",
                    job.provider
                )));
                continue;
            };
            emit(AtlasEvent::Status(format!(
                "Headlines {} · {}",
                country.to_ascii_uppercase(),
                job.provider
            )));
            let fetched = fetch_with_spare_key(
                &mut fetch,
                &user_agent,
                reservation,
                |active| phase2_call(job.provider, country, active),
            )
            .await?;
            tel.provider_calls(&fetched.metrics);
            match fetched.body {
                Some(body) => {
                    let observations = capture_provider_observations(&store, &run_id, job.provider, &receipt_key, &body, &mut emit).await;
                    match atlas_news::headline_hits(job.provider, country, &body) {
                    Ok(hits) => {
                        apply_hits(
                            &hits,
                            &observations,
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
                    Err(_) => {
                        let fault = ProviderFault {
                            summary: format!("{} response schema invalid", job.provider),
                            body: String::new(),
                        };
                        if record_fault(&mut stats, job.provider, &fault)
                            && stats.acquisition_retry.is_none()
                        {
                            stats.acquisition_retry = Some(cursor_at(job, &cursor.from));
                        }
                        emit(parse_fault(job.provider, fault.summary, &body));
                    }
                }},
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
            let next = next_after_phase2(job, countries.len());
            save(&store, &run_id, &cursor_at(&next, &cursor.from), &stats).await?;
            emit(AtlasEvent::Stats(Box::new(stats.clone())));
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
        emit(AtlasEvent::Stats(Box::new(stats.clone())));
    }

    let admission_paused = if cursor.phase == 3 {
        track(3);
        tel.enter_stage(crate::atlas_work::stage_label(crate::atlas_work::STAGE_CLASSIFICATION),"article",&stats);
        let pinned=permit.pinned().cloned().ok_or_else(||anyhow!("admission requires its preflighted classifier"))?;
        let path=classifier_home.home().to_path_buf();
        let context=tokio::task::spawn_blocking(move ||crate::atlas_classify::AdmissionContext::load(&path,pinned)).await??;
        run_local_admission(LocalAdmission {store:&store,context:&context,resumed:resume,tracker,pause},&mut stats,&mut emit,&mut tel).await?
    } else { false };
    // Admission is finished (or paused after a committed chunk). Publish its
    // authoritative denominator before the next native-stage status. This is
    // the same run-local aggregate formerly deferred until cycle completion.
    let summary=crate::storage::classifier_admission::run_summary(repo,&run_id).await?;
    stats.coverage.apply_admission_summary(&summary);
    emit(AtlasEvent::Stats(Box::new(stats.clone())));
    if admission_paused {
        return park(&store,&run_id,&cursor,&stats,&mut emit,&mut tel).await;
    }
    if cursor.phase == 3 {
        cursor.phase=4;cursor.leg="metadata".into();
        save(&store,&run_id,&cursor,&stats).await?;
    }
    // Admission commits each independent derivative job atomically. Collection
    // completion never waits for metadata, related leads, actors or graph pages.
    // A resumed legacy phase also ends here; its durable receipts are reconciled
    // by the independent scheduler.
    let native_summary=crate::storage::classifier_admission::native_run_summary(repo,&run_id).await?;
    stats.coverage.apply_native_summary(&native_summary);
    let actor_index_incomplete=false;
    if let Some(baseline)=tel.classifier_db_start {tel.classifier_metrics["atlas.db"]=ClassifierDbSample::read(&db_owner.metrics()).delta(baseline);}
    // Completion depends on admission, never on Brain indexing. Staged work
    // counts as the fetched set; canonical rows are the admitted set.
    let summary=crate::storage::classifier_admission::run_summary(repo,&run_id).await?;
    stats.coverage.apply_admission_summary(&summary);
    stats.coverage.publishers_unavailable = stats
        .coverage
        .articles
        .saturating_sub(stats.coverage.publishers_available);
    let state = if summary.staged>0 {"waiting"} else {cycle_state(&stats.coverage, actor_index_incomplete)};
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
        stats.coverage.admitted,
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
            cursor.phase = native_summary.earliest_phase.unwrap_or(6);
            cursor.leg = match cursor.phase {4=>"metadata",5=>"related",_=>"actor_articles"}.into();
        } else {
            cursor.phase = 3;
            cursor.leg = "classify".into();
        }
    }
    save(&store, &run_id, &cursor, &stats).await?;
    store.set_state(&run_id, state, &note, true).await?;
    emit(AtlasEvent::Stats(Box::new(stats.clone())));
    emit(AtlasEvent::Status(note.clone()));
    tel.close_stage(state, &stats);
    tel.cycle(state, &stats);
    Ok(if state == "failed" {
        Stop::Failed(note)
    } else {
        Stop::Finished
    })
    }));
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(
        crate::storage::classifier_admission::HEARTBEAT_SECS,
    ));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut outcome = loop {
        tokio::select! {
            result=&mut work=>break result,
            _=heartbeat.tick(),if !store.stopping()=>{
                match crate::storage::classifier_admission::heartbeat_run(&db_owner,&claim,chrono::Utc::now()).await {
                    Ok(true)=>{},
                    Ok(false)=>break Err(crate::storage::classifier_admission::AdmissionError::LeaseLost.into()),
                    Err(error)=>break Err(error.into()),
                }
            }
        }
    };
    // Drop the awaiting work before joining its owned transaction workers.
    // This requests cancellation of in-flight JS and closes DB receivers; an
    // owned BEGIN/body then rolls back, while issued COMMIT resolves safely.
    drop(work);
    if let Err(error) = db_owner.join().await {
        outcome = Err(error.into());
    }
    if let Err(error) = &outcome {
        if !matches!(
            error.downcast_ref::<crate::storage::classifier_admission::AdmissionError>(),
            Some(
                crate::storage::classifier_admission::AdmissionError::LeaseLost
                    | crate::storage::classifier_admission::AdmissionError::CleanupUncertain { .. }
            )
        ) {
            let message = error.to_string();
            let state = if message.starts_with("Enter the ") {
                "blocked"
            } else {
                "failed"
            };
            if let Err(failure) = store.set_state(&run_id, state, &message, true).await {
                outcome = Err(failure);
            }
        }
    }
    if let Err(error) = db_owner.shutdown().await {
        outcome = Err(error.into());
    }
    if let Err(error) = classifier_home.shutdown().await {
        outcome = Err(error.into());
    }
    outcome
}

/// Performs one activation over bounded run-local cohorts. Durable failed work
/// is retried by a later explicit resume, never by an endless in-cycle loop.
struct LocalAdmission<'a> {
    store: &'a AtlasStore,
    context: &'a crate::atlas_classify::AdmissionContext,
    resumed: bool,
    tracker: &'a std::sync::Mutex<AtlasJob>,
    pause: &'a AtomicBool,
}
async fn run_local_admission(
    input: LocalAdmission<'_>,
    stats: &mut RunStats,
    emit: &mut impl FnMut(AtlasEvent),
    telemetry: &mut CycleTelemetry,
) -> Result<bool> {
    let LocalAdmission {
        store,
        context,
        resumed,
        tracker,
        pause,
    } = input;
    use crate::storage::classifier_admission as admission;
    let repo = store
        .repository()
        .ok_or_else(|| anyhow!("SurrealDB storage is not configured"))?;
    let owner = store
        .admission_owner()
        .ok_or_else(|| anyhow!("Atlas admission database owner is missing"))?;
    let run = store
        .run_claim()
        .ok_or_else(|| anyhow!("Atlas admission owner is missing"))?;
    loop {
        if cycle_paused(tracker, pause) {
            return Ok(true);
        }
        if admission::upgrade_legacy_page(owner, run, chrono::Utc::now()).await? == 0 {
            break;
        }
    }
    let mut token =
        admission::begin_admission(owner, run, context, resumed, chrono::Utc::now()).await?;
    let mut cache = crate::atlas_classify::ComputationCache::default();
    let started = std::time::Instant::now();
    loop {
        if cycle_paused(tracker, pause) {
            return Ok(true);
        }
        let before_page = ClassifierDbSample::read(&owner.metrics());
        let page = admission::claim_header_page(owner, run, &token, chrono::Utc::now()).await?;
        let groups = admission::hydration_groups(owner, &page.claims).await?;
        let mut first = true;
        for group in groups {
            if cycle_paused(tracker, pause) {
                return Ok(true);
            }
            let (claims, unavailable) = match group {
                admission::HydrationGroup::Ready(claims) => (claims, false),
                admission::HydrationGroup::Unavailable(claims) => (claims, true),
            };
            let before = if first {
                before_page
            } else {
                ClassifierDbSample::read(&owner.metrics())
            };
            first = false;
            if process_admission_chunk(
                LocalAdmissionChunk {
                    store,
                    context,
                    tracker,
                    pause,
                    claims,
                    unavailable,
                    db_before: before,
                },
                &mut cache,
                stats,
                emit,
                telemetry,
            )
            .await?
            {
                return Ok(true);
            }
        }
        if let Some(next) = page.next {
            token = next;
            continue;
        }
        let revision = admission::run_revision(repo, &run.run_id).await?;
        if revision.work_generation > token.through_seq {
            if started.elapsed() >= std::time::Duration::from_secs(30) {
                break;
            }
            token =
                admission::begin_admission(owner, run, context, false, chrono::Utc::now()).await?;
            continue;
        }
        break;
    }
    Ok(false)
}
struct LocalAdmissionChunk<'a> {
    store: &'a AtlasStore,
    context: &'a crate::atlas_classify::AdmissionContext,
    tracker: &'a std::sync::Mutex<AtlasJob>,
    pause: &'a AtomicBool,
    claims: Vec<crate::storage::classifier_admission::AdmissionClaim>,
    unavailable: bool,
    db_before: ClassifierDbSample,
}
async fn process_admission_chunk(
    input: LocalAdmissionChunk<'_>,
    cache: &mut crate::atlas_classify::ComputationCache,
    stats: &mut RunStats,
    emit: &mut impl FnMut(AtlasEvent),
    telemetry: &mut CycleTelemetry,
) -> Result<bool> {
    use crate::atlas_classify::{self, ItemClassification};
    use crate::storage::classifier_admission::{self as admission, ItemCommit};
    let LocalAdmissionChunk {
        store,
        context,
        tracker,
        pause,
        claims,
        unavailable,
        db_before,
    } = input;
    let repo = store
        .repository()
        .ok_or_else(|| anyhow!("SurrealDB storage is not configured"))?;
    let owner = store
        .admission_owner()
        .ok_or_else(|| anyhow!("Atlas admission database owner is missing"))?;
    let run = store
        .run_claim()
        .ok_or_else(|| anyhow!("Atlas admission owner is missing"))?;
    let started = std::time::Instant::now();
    let worker_before = classifier_worker_observation(context).await.ok();
    let cache_before = cache.metrics();
    emit(AtlasEvent::Status(format!(
        "Local admission · {} articles",
        claims.len()
    )));
    let mut rows_read = 0usize;
    let mut page_metrics = None;
    let inputs = if unavailable {
        Err(admission::AdmissionError::PayloadTooLarge {
            kind: "native_metadata".into(),
            limit: admission::MAX_RESPONSE_BYTES,
        })
    } else {
        admission::bulk_inputs_current(owner, &claims).await
    };
    let mut outcomes = match inputs {
        Ok(inputs) => {
            rows_read = inputs.len();
            let present: std::collections::BTreeSet<_> = inputs
                .iter()
                .map(|input| input.claim.occurrence.clone())
                .collect();
            let control = crate::classifier::WorkControl::new();
            let work = atlas_classify::classify_page(context, inputs, cache, &control);
            tokio::pin!(work);
            let result = loop {
                tokio::select! {
                    result=&mut work=>break result,
                    _=tokio::time::sleep(std::time::Duration::from_millis(25))=>{if cycle_paused(tracker,pause) {control.cancel.store(true,Ordering::Release);}}
                }
            };
            let mut values = match result {
                Ok(scored) => {
                    page_metrics = Some(scored.metrics);
                    scored.outcomes
                }
                Err(error) => {
                    if cycle_paused(tracker, pause) {
                        admission::release_owned(owner, run, &claims, chrono::Utc::now()).await?;
                        return Ok(true);
                    }
                    let error = match error {
                        admission::AdmissionError::Classifier(error) => error,
                        _ => crate::classifier::ClassifierError::InvalidArtifact(
                            "invalid_local_admission_response".into(),
                        ),
                    };
                    claims
                        .iter()
                        .filter(|claim| present.contains(&claim.occurrence))
                        .map(|claim| ItemClassification::Retryable {
                            claim: claim.clone(),
                            error: error.clone(),
                        })
                        .collect()
                }
            };
            for claim in claims
                .iter()
                .filter(|claim| !present.contains(&claim.occurrence))
            {
                values.push(ItemClassification::Retryable {
                    claim: claim.clone(),
                    error: crate::classifier::ClassifierError::Storage(
                        "claimed_source_changed".into(),
                    ),
                });
            }
            values
        }
        Err(_) => claims
            .iter()
            .map(|claim| ItemClassification::Retryable {
                claim: claim.clone(),
                error: if unavailable {
                    crate::classifier::ClassifierError::CorpusTooLarge
                } else {
                    crate::classifier::ClassifierError::Storage("claimed_input_unavailable".into())
                },
            })
            .collect(),
    };
    outcomes.sort_by_key(|outcome| match outcome {
        ItemClassification::Evaluated { claim, .. }
        | ItemClassification::Retryable { claim, .. } => claim.work_seq,
    });
    let committed = admission::commit_outcomes(owner, run, &outcomes, chrono::Utc::now()).await?;
    for (outcome, result) in outcomes.iter().zip(&committed.items) {
        match result {
            ItemCommit::Committed { .. } => {
                if let ItemClassification::Evaluated { claim, receipt } = outcome {
                    if let Some(primary) = receipt.receipt.selected_domains.first() {
                        stats.coverage.admitted = stats.coverage.admitted.saturating_add(1);
                        emit(AtlasEvent::Classified {
                            id: claim.occurrence.article_id.clone(),
                            category: primary.as_str().into(),
                        });
                    } else {
                        stats.coverage.discarded = stats.coverage.discarded.saturating_add(1);
                    }
                }
            }
            ItemCommit::AlreadyCommitted { .. } => {}
            ItemCommit::StaleInput { .. } => emit(AtlasEvent::Note(
                "A changed headline remains queued under its new source revision".into(),
            )),
            ItemCommit::LeaseLost { .. }
            | ItemCommit::Retryable { .. }
            | ItemCommit::InvalidReceipt { .. } => {
                stats.coverage.admission_failed = stats.coverage.admission_failed.saturating_add(1)
            }
            ItemCommit::Cancelled { .. } => {}
        }
    }
    let observation = ClassifierChunkObservation {
        db_before,
        db_after: ClassifierDbSample::read(&owner.metrics()),
        cache_before,
        cache: cache.metrics(),
        worker_before: worker_before.as_ref(),
        page: page_metrics.as_ref(),
        results: &committed.items,
        rows_read,
        wall_micros: u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
    };
    // Metrics are diagnostic: a storage failure here does not change the
    // durable classifier outcomes or spend another inference attempt.
    if telemetry
        .classifier_chunk(repo, context, &claims, observation)
        .await
        .is_err()
    {
        emit(AtlasEvent::Note(
            "Classifier telemetry write unavailable".into(),
        ));
    }
    Ok(cycle_paused(tracker, pause))
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
    emit(AtlasEvent::Stats(Box::new(stats.clone())));
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

fn parse_fault(provider: &str, _summary: String, body: &serde_json::Value) -> AtlasEvent {
    AtlasEvent::Fault(ProviderFault {summary:format!("{provider} response schema invalid"),body:serde_json::json!({"provider":provider,"code":"invalid_response_schema","top_level_fields":body.as_object().map(|object|object.len())}).to_string()})
}
fn one_line(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(180)
        .collect()
}
fn provider_fault(provider: &str, status: u16, raw: &str) -> ProviderFault {
    // Provider messages are untrusted. Inspect their class for retry policy,
    // while reporting only bounded status/code diagnostics.
    let quota = status == 429
        || atlas_news::http_error(provider, status, raw)
            .as_deref()
            .is_some_and(|summary| crate::osint::quota_limited("failed", Some(summary)));
    let code = if quota {
        "quota limit exceeded"
    } else if matches!(status, 401 | 403) {
        "authentication or access denied"
    } else {
        "request failed"
    };
    ProviderFault {summary:format!("{provider} HTTP {status}: {code}"),body:serde_json::json!({"provider":provider,"status":status,"code":if quota{"quota_limited"}else{"http_error"}}).to_string()}
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

#[cfg(test)]
async fn dispatch<F>(
    fetch: &mut F,
    spec: CallSpec,
    user_agent: &str,
) -> Result<serde_json::Value, ProviderFault>
where
    F: FnMut(HttpCall) -> FetchFut,
{
    dispatch_counted(fetch, spec, user_agent, &mut ProviderCallCounts::default()).await
}

async fn dispatch_counted<F>(
    fetch: &mut F,
    spec: CallSpec,
    user_agent: &str,
    metrics: &mut ProviderCallCounts,
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
        metrics.calls += 1;
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
                        metrics.redirects += 1;
                        url = next;
                        continue;
                    }
                    None => break reply,
                }
            }
            Ok(reply) if transient_status(reply.status) && attempt < DISPATCH_ATTEMPTS => {
                metrics.transport_retries += 1;
            }
            Ok(reply) => break reply,
            Err(err) if attempt < DISPATCH_ATTEMPTS => {
                let _ = err;
                metrics.transport_retries += 1;
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
                    summary: one_line(&format!("{provider} {kind} after {attempt} attempts")),
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
    serde_json::from_str(&reply.body).map_err(|_|ProviderFault {summary:format!("{provider} malformed JSON"),body:serde_json::json!({"provider":provider,"code":"malformed_json","bytes":reply.body.len()}).to_string()})
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
    /// Start or resume the exact run assigned by durable Atlas dispatch.
    /// The dispatcher owns its registry job and delivery cancellation.
    Dispatch(String),
}

fn validate_dispatch_id(run_id: &str) -> Result<()> {
    let valid = run_id
        .strip_prefix("fleet-atlas-run-")
        .is_some_and(|request| {
            (1..=64).contains(&request.len())
                && request
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        });
    anyhow::ensure!(valid, "invalid dispatched Atlas run ID");
    Ok(())
}

/// Read the exact durable run state assigned to a dispatch.
pub async fn dispatch_state(
    repo: &crate::storage::repo::Repository,
    run_id: &str,
) -> Result<Option<String>> {
    validate_dispatch_id(run_id)?;
    let mut response = repo
        .db()
        .query("SELECT VALUE state FROM ONLY type::record('atlas_run', $run)")
        .bind(("run", run_id.to_owned()))
        .await?
        .check()?;
    Ok(response.take(0)?)
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
    let dispatch_id = match &live {
        LiveRun::Dispatch(id) => {
            validate_dispatch_id(id)?;
            Some(id.as_str())
        }
        _ => None,
    };
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(25))
        .build()?;
    let input = RunInput {
        db_path,
        pause,
        keys,
        user_agent,
        pace: true,
        resume: matches!(&live, LiveRun::Latest | LiveRun::Run(_)),
        feed,
        classifier,
        synthesizer,
        synthesizer_fallbacks,
        run_id: match &live {
            LiveRun::Run(id) => Some(id.as_str()),
            _ => None,
        },
    };
    let fetch = move |call: HttpCall| -> FetchFut {
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
    };
    if let Some(id) = dispatch_id {
        let tracker = std::sync::Mutex::new(AtlasJob::new(db_path));
        run_atlas_inner(input, emit, fetch, &tracker, Some(id)).await
    } else {
        run_atlas(input, emit, fetch).await
    }
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
    fn live_coverage_reconciles_committed_cohorts_without_counting_retries() {
        use crate::storage::classifier_admission::{NativeRunSummary, RunAdmissionSummary};

        let mut coverage = CoverageStats {
            acquisition_failures: 2,
            classification_blocked: 1,
            oversized_inputs: 3,
            quota_skipped: 4,
            staged: 12,
            ..CoverageStats::default()
        };
        let admitted = RunAdmissionSummary {
            retained: 12,
            admitted: 6,
            rejected: 2,
            staged: 3,
            failed: 1,
            legacy: 1,
        };
        coverage.apply_admission_summary(&admitted);
        assert_eq!(
            (coverage.classified, coverage.articles),
            (6, 12),
            "the retained denominator includes unresolved/rejected/legacy work"
        );
        assert_eq!(coverage.admission_pending, 2);
        assert_eq!(coverage.admission_failed, 1);
        assert_eq!(coverage.discarded, 2);
        assert_eq!(coverage.legacy_unknown, 1);
        assert_eq!(coverage.entities_available, 0);
        assert_eq!(
            coverage.entities_unavailable, 0,
            "pending is not unavailable"
        );

        let metadata = NativeRunSummary {
            entities: 5,
            publishers: 4,
            ..NativeRunSummary::default()
        };
        coverage.apply_native_summary(&NativeRunSummary {
            earliest_phase: Some(4),
            entities: 5,
            publishers: 4,
            ..NativeRunSummary::default()
        });
        assert_eq!(
            coverage.publishers_unavailable, 0,
            "metadata is still pending"
        );
        coverage.apply_native_summary(&metadata);
        assert_eq!(
            coverage.publishers_unavailable, 8,
            "settled missing publisher coverage is visible before actors"
        );
        let checkpoint = coverage.clone();
        // Restored older checkpoints can lack every native counter even
        // though the current committed metadata is already available.
        coverage.entities_available = 0;
        coverage.publishers_available = 0;
        coverage.apply_admission_summary(&admitted);
        coverage.apply_native_summary(&metadata);
        assert_eq!(
            coverage, checkpoint,
            "restored coverage uses committed receipts"
        );
        coverage.apply_admission_summary(&admitted);
        coverage.apply_native_summary(&metadata);
        assert_eq!(coverage, checkpoint, "replaying a checkpoint is idempotent");
        assert_eq!(coverage.acquisition_failures, 2);
        assert_eq!(coverage.classification_blocked, 1);
        assert_eq!(coverage.oversized_inputs, 3);
        assert_eq!(coverage.quota_skipped, 4);
        assert_eq!(coverage.staged, 12);

        coverage.apply_native_summary(&NativeRunSummary {
            related: 7,
            actors: 3,
            ..metadata
        });
        assert_eq!(
            (coverage.entities_available, coverage.publishers_available),
            (5, 4)
        );
        assert_eq!((coverage.related_pairs, coverage.actor_indexed), (7, 3));
        assert_eq!((coverage.classified, coverage.articles), (6, 12));
    }

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
    fn current_domains_are_exactly_six_and_historical_labels_do_not_alias() {
        assert_eq!(
            CATEGORY_IDS,
            crate::atlas_taxonomy::DomainId::ALL.map(crate::atlas_taxonomy::DomainId::as_str)
        );
        assert_eq!((MIN_ADMISSION_TAGS, MAX_ADMISSION_TAGS), (1, 2));
        assert_eq!(normalize_category(" Governance "), "governance");
        assert_eq!(normalize_category("science"), "unk");
        assert_eq!(normalize_category("military"), "unk");
        assert_eq!(category_name("scientific"), "Science & Technology");
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
        assert!(card
            .lines()
            .any(|line| line == "Category: Economy & Supply Chains"));
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

#[cfg(test)]
mod classifier_preflight_tests {
    use super::*;

    #[test]
    fn phase_alone_does_not_authorize_post_admission_resume() {
        let mut cursor = Cursor {
            phase: 4,
            ..Cursor::default()
        };
        let mut stats = RunStats::default();
        assert_eq!(
            cycle_requirement(None, &cursor, &stats, false),
            CycleRequirement::FreshAcquisition
        );
        assert_eq!(
            cycle_requirement(Some("r"), &cursor, &stats, false),
            CycleRequirement::PostAdmissionOnly
        );
        assert_eq!(
            cycle_requirement(Some("r"), &cursor, &stats, true),
            CycleRequirement::AdmissionOnly
        );
        stats.acquisition_retry = Some(Cursor::default());
        assert_eq!(
            cycle_requirement(Some("r"), &cursor, &stats, false),
            CycleRequirement::AcquisitionAndAdmission
        );
        stats.acquisition_retry = None;
        cursor.phase = 6;
        assert_eq!(
            cycle_requirement(Some("r"), &cursor, &stats, false),
            CycleRequirement::PostAdmissionOnly
        );
        cursor.phase = 3;
        assert_eq!(
            cycle_requirement(Some("r"), &cursor, &stats, false),
            CycleRequirement::AdmissionOnly
        );
    }

    #[test]
    fn malformed_or_unsupported_saved_state_cannot_default_to_an_exemption() {
        let mut cursor = Cursor::default();
        let mut stats = RunStats::default();
        for phase in [0, 7, 255] {
            cursor.phase = phase;
            assert!(validate_resume_state(&cursor, &stats).is_err());
        }
        cursor = Cursor::default();
        cursor.chunk = PHASE1_MAX_PAGES + 1;
        assert!(validate_resume_state(&cursor, &stats).is_err());
        cursor = Cursor::default();
        stats.acquisition_retry = Some(Cursor {
            phase: 4,
            ..Cursor::default()
        });
        assert!(validate_resume_state(&cursor, &stats).is_err());
        stats.acquisition_retry = None;
        stats.contract_version = "unknown-future".into();
        assert!(validate_resume_state(&cursor, &stats).is_err());
    }

    #[cfg(feature = "embedded")]
    #[tokio::test]
    async fn dispatch_replay_respects_completion_and_the_exact_run_lease() {
        use crate::storage::classifier_admission::AdmissionError;
        use crate::storage::open::Target;
        use crate::storage::repo::Repository;

        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("argos.db");
        let repo = Repository::connect(
            &Target::Embedded {
                path: temp.path().join("storage"),
            },
            "argos",
            "dispatch_replay",
        )
        .await
        .unwrap();
        crate::storage::atlas::apply_schema(repo.db())
            .await
            .unwrap();
        let completed = "fleet-atlas-run-completed";
        let leased = "fleet-atlas-run-leased";
        let cursor = serde_json::to_string(&Cursor::default()).unwrap();
        let stats = serde_json::to_string(&RunStats::default()).unwrap();
        for run in [completed, leased] {
            crate::storage::atlas::insert_run(repo.db(), run, &cursor, &stats)
                .await
                .unwrap();
        }
        crate::storage::atlas::set_state(repo.db(), completed, "completed", "", true)
            .await
            .unwrap();
        repo.db()
            .query("CREATE type::record('atlas_classifier_run', $run) SET owner='fixture', revision=1, lease_until=time::now()+1h, updated_at=time::now() RETURN NONE")
            .bind(("run", leased))
            .await
            .unwrap()
            .check()
            .unwrap();
        let pause = AtomicBool::new(false);
        let keys = ProviderKeys::default();
        let input = || RunInput {
            db_path: &db_path,
            pause: &pause,
            keys: &keys,
            user_agent: "offline-fixture",
            pace: false,
            resume: false,
            feed: &[],
            classifier: None,
            synthesizer: None,
            synthesizer_fallbacks: &[],
            run_id: None,
        };
        let tracker = std::sync::Mutex::new(AtlasJob::new(&db_path));
        let fetch = |_: HttpCall| -> FetchFut { panic!("replayed delivery must not fetch") };
        let replay = run_atlas_on_store(
            input(),
            |_| {},
            fetch,
            &tracker,
            AtlasStore::from_repository(repo.clone()),
            Some(completed),
        )
        .await
        .unwrap();
        assert!(matches!(replay, Stop::Finished));
        let held = run_atlas_on_store(
            input(),
            |_| {},
            fetch,
            &tracker,
            AtlasStore::from_repository(repo.clone()),
            Some(leased),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            held.downcast_ref::<AdmissionError>(),
            Some(AdmissionError::LeaseLost)
        ));
        assert_eq!(
            dispatch_state(&repo, completed).await.unwrap().as_deref(),
            Some("completed")
        );
        assert_eq!(
            dispatch_state(&repo, leased).await.unwrap().as_deref(),
            Some("running")
        );
        assert!(tracker.lock().unwrap().run_id.is_empty());
    }

    #[cfg(feature = "embedded")]
    #[tokio::test]
    async fn denied_preflight_spends_no_quota_or_fetches_and_preserves_resume_state() {
        use crate::storage::open::Target;
        use crate::storage::repo::Repository;
        use surrealdb::types::SurrealValue;
        #[derive(Debug, PartialEq, SurrealValue)]
        struct QuotaState {
            key: String,
            used: i64,
        }
        async fn quota_snapshot(repo: &Repository) -> Vec<QuotaState> {
            let mut response = repo
                .db()
                .query("SELECT record::id(id) AS key,used FROM atlas_quota")
                .await
                .unwrap()
                .check()
                .unwrap();
            let mut rows: Vec<QuotaState> = response.take(0).unwrap();
            rows.sort_by(|left, right| left.key.cmp(&right.key));
            rows
        }
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("argos.db");
        let repo = Repository::connect(
            &Target::Embedded {
                path: temp.path().join("storage"),
            },
            "argos",
            "preflight_denied",
        )
        .await
        .unwrap();
        crate::storage::atlas::apply_schema(repo.db())
            .await
            .unwrap();
        // Existing quota state must also survive a denied invocation.
        crate::storage::atlas::quota_bump(repo.db(), "fixture-provider", "2026-01-01")
            .await
            .unwrap();
        assert_eq!(quota_snapshot(&repo).await[0].used, 1);
        let pause = AtomicBool::new(false);
        let keys = ProviderKeys::default();
        for resume in [false, true] {
            if resume {
                crate::storage::atlas::insert_run(
                    repo.db(),
                    "paused",
                    &serde_json::to_string(&Cursor::default()).unwrap(),
                    &serde_json::to_string(&RunStats::default()).unwrap(),
                )
                .await
                .unwrap();
                crate::storage::atlas::set_state(
                    repo.db(),
                    "paused",
                    "paused",
                    "saved unchanged",
                    false,
                )
                .await
                .unwrap();
            }
            let before = crate::storage::atlas::list_runs(repo.db()).await.unwrap();
            let before_quotas = quota_snapshot(&repo).await;
            let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let counted = calls.clone();
            let tracker = std::sync::Mutex::new(AtlasJob::new(&db_path));
            let result = run_atlas_on_store(
                RunInput {
                    db_path: &db_path,
                    pause: &pause,
                    keys: &keys,
                    user_agent: "offline-fixture",
                    pace: false,
                    resume,
                    feed: &[],
                    classifier: None,
                    synthesizer: None,
                    synthesizer_fallbacks: &[],
                    run_id: resume.then_some("paused"),
                },
                |_| {},
                move |_| {
                    counted.fetch_add(1, Ordering::SeqCst);
                    Box::pin(async {
                        anyhow::bail!("fetch must not run before denied classifier preflight")
                    })
                },
                &tracker,
                AtlasStore::from_repository(repo.clone()),
                None,
            )
            .await;
            let error = result.unwrap_err();
            let preflight = error.downcast_ref::<CyclePreflightError>().unwrap();
            if !resume && cfg!(feature = "classifier") {
                use crate::classifier::{ClassifierError, ClassifierStatus};
                let expected = temp.path().join("classifier/zero-shot.json");
                assert!(matches!(
                    preflight,
                    CyclePreflightError::Classifier {
                        status: ClassifierStatus::Missing,
                        code,
                        source: ClassifierError::MissingZeroShotConfig(path),
                    } if code == "classifier_unavailable" && path == &expected
                ));
                assert!(preflight.to_string().contains("worker_path and model_dir"));
            }
            assert_eq!(calls.load(Ordering::SeqCst), 0);
            assert!(tracker.lock().unwrap().run_id.is_empty());
            let after = crate::storage::atlas::list_runs(repo.db()).await.unwrap();
            assert_eq!(
                before
                    .iter()
                    .map(|run| (
                        &run.id,
                        &run.state,
                        run.phase,
                        &run.cursor_json,
                        &run.stats_json,
                        &run.note,
                        &run.started_at,
                        &run.finished_at
                    ))
                    .collect::<Vec<_>>(),
                after
                    .iter()
                    .map(|run| (
                        &run.id,
                        &run.state,
                        run.phase,
                        &run.cursor_json,
                        &run.stats_json,
                        &run.note,
                        &run.started_at,
                        &run.finished_at
                    ))
                    .collect::<Vec<_>>()
            );
            assert_eq!(quota_snapshot(&repo).await, before_quotas);
            assert!(!temp.path().join("classifier").exists());
        }
        let store = AtlasStore::from_repository(repo.clone());
        let home = ClassifierHome::new(temp.path().to_path_buf());
        let cursor = Cursor {
            phase: 6,
            ..Cursor::default()
        };
        let stats = RunStats::default();
        let other = AtlasArticleRow {
            run_id: "other-run".into(),
            id: "a".into(),
            ..AtlasArticleRow::default()
        };
        crate::storage::atlas::stage_article(repo.db(), &other, "revision", "fixture")
            .await
            .unwrap();
        let permit = classifier_cycle_preflight(&store, &home, Some("paused"), &cursor, &stats)
            .await
            .unwrap();
        assert_eq!(permit.requirement(), CycleRequirement::PostAdmissionOnly);
        assert!(permit.pinned().is_none());
        let pending = AtlasArticleRow {
            run_id: "paused".into(),
            id: "b".into(),
            ..AtlasArticleRow::default()
        };
        crate::storage::atlas::stage_article(repo.db(), &pending, "revision", "fixture")
            .await
            .unwrap();
        assert!(matches!(
            classifier_cycle_preflight(&store, &home, Some("paused"), &cursor, &stats).await,
            Err(CyclePreflightError::MissingPinnedRevision)
        ));
        home.shutdown().await.unwrap();
    }
}

/// Runtime handoff is signalled before pin allocation. Only transient admission
/// errors retry, under one absolute deadline; existing model pins are never
/// evicted to make a fourth runtime fit.
async fn foreground_classifier_preflight(
    store: &AtlasStore,
    home: &ClassifierHome,
    run_id: Option<&str>,
    cursor: &Cursor,
    stats: &RunStats,
    owner: &crate::storage::classifier_admission::AdmissionOwner,
    intent: &crate::storage::classifier_admission::ForegroundIntent,
    deadline: tokio::time::Instant,
) -> Result<CyclePermit> {
    use crate::classifier::{ClassifierError, ClassifierStatus};
    let pending = || CyclePreflightError::Classifier {
        status: ClassifierStatus::Unavailable,
        code: "foreground_handoff_pending".into(),
        source: ClassifierError::Busy,
    };
    let mut renewal = tokio::time::interval(std::time::Duration::from_secs(1));
    renewal.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    renewal.tick().await;
    loop {
        let attempt = classifier_cycle_preflight(store, home, run_id, cursor, stats);
        tokio::pin!(attempt);
        let result = loop {
            tokio::select! {
                result=&mut attempt=>break result,
                _=tokio::time::sleep_until(deadline)=>return Err(pending().into()),
                _=renewal.tick()=> {
                    if !crate::storage::classifier_admission::renew_foreground_intent(owner,intent,chrono::Utc::now()).await? {return Err(pending().into());}
                }
            }
        };
        match result {
            Ok(permit) => return Ok(permit),
            Err(error) if matches!(&error,CyclePreflightError::Classifier {source,..} if matches!(source,ClassifierError::Busy | ClassifierError::RuntimeBudgetExceeded {..})) =>
                {}
            Err(error) => return Err(error.into()),
        }
        tokio::select! {
            _=tokio::time::sleep_until(deadline)=>return Err(pending().into()),
            _=tokio::time::sleep(std::time::Duration::from_millis(100))=>{},
            _=renewal.tick()=> {
                if !crate::storage::classifier_admission::renew_foreground_intent(owner,intent,chrono::Utc::now()).await? {return Err(pending().into());}
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
struct ClassifierDbSample {
    round_trips: u64,
    conflicts: u64,
    begins: u64,
    commits: u64,
    rollbacks: u64,
}
impl ClassifierDbSample {
    fn read(metrics: &crate::storage::classifier_admission::DbMetrics) -> Self {
        Self {
            round_trips: metrics.round_trips(),
            conflicts: metrics.conflict_retries(),
            begins: metrics.begins(),
            commits: metrics.commits(),
            rollbacks: metrics.rollbacks(),
        }
    }
    fn delta(self, before: Self) -> serde_json::Value {
        serde_json::json!({"round_trips":self.round_trips.checked_sub(before.round_trips),"conflict_retries":self.conflicts.checked_sub(before.conflicts),"begins":self.begins.checked_sub(before.begins),"commits":self.commits.checked_sub(before.commits),"rollbacks":self.rollbacks.checked_sub(before.rollbacks)})
    }
}
#[derive(Clone, Debug, Default, serde::Serialize)]
struct ProviderCallCounts {
    calls: u64,
    transport_retries: u64,
    redirects: u64,
    fallbacks: u64,
}
pub(crate) async fn classifier_worker_observation(
    context: &crate::atlas_classify::AdmissionContext,
) -> Result<crate::classifier::ClassifierPerformance, &'static str> {
    let service = context.pin.service.clone();
    match tokio::task::spawn_blocking(move || service.performance()).await {
        Ok(Ok(performance))
            if performance.batch_execution_micros.len() == performance.batch_sizes.len()
                && performance.batch_sizes.len() <= 4096
                && performance
                    .batch_sizes
                    .iter()
                    .all(|size| (1..=64).contains(size)) =>
        {
            Ok(performance)
        }
        Ok(Ok(_)) => Err("invalid_worker_snapshot"),
        _ => Err("worker_performance_unavailable"),
    }
}
fn nearest_rank(samples: &[u64], percentile: usize) -> Option<u64> {
    if samples.is_empty() {
        return None;
    }
    let mut values = samples.to_vec();
    values.sort_unstable();
    let rank = (values.len() * percentile).div_ceil(100).max(1);
    values.get(rank - 1).copied()
}
pub(crate) fn classifier_worker_metrics(
    before: Option<&crate::classifier::ClassifierPerformance>,
    after: Result<&crate::classifier::ClassifierPerformance, &str>,
) -> serde_json::Value {
    match after {
        Ok(after) => {
            let queue = before
                .and_then(|before| after.queue_micros.checked_sub(before.queue_micros))
                .map(|micros| micros as f64 / 1000.0);
            let mut sizes = [0u64; 64];
            for size in &after.batch_sizes {
                sizes[*size - 1] += 1;
            }
            serde_json::json!({
                "atlas.classifier.queue_ms":queue,"atlas.classifier.queue_reason":if queue.is_some(){None}else{Some("baseline_unavailable_or_counter_discontinuity")},
                "atlas.classifier.p50_ms":nearest_rank(&after.batch_execution_micros,50).map(|micros|micros as f64/1000.0),
                "atlas.classifier.p95_ms":nearest_rank(&after.batch_execution_micros,95).map(|micros|micros as f64/1000.0),
                "atlas.classifier.samples":after.batch_execution_micros.len(),"atlas.classifier.latency_unit":"batch",
                "atlas.classifier.latency_reason":if after.batch_execution_micros.is_empty(){Some("no_execution_samples")}else{None},
                "atlas.classifier.batch_sizes":sizes.as_slice(),"atlas.classifier.batch_sizes_encoding":"histogram_index_plus_one",
                "atlas.classifier.js_heap_current_bytes":after.js_heap_current_bytes,"atlas.classifier.js_heap_peak_bytes":after.js_heap_peak_bytes,
                "atlas.classifier.runtime_reserved_bytes":after.runtime_reserved_bytes,"atlas.classifier.stack_limit_bytes":after.stack_limit_bytes,
            })
        }
        Err(reason) => {
            serde_json::json!({"atlas.classifier.queue_ms":null,"atlas.classifier.p50_ms":null,"atlas.classifier.p95_ms":null,"atlas.classifier.samples":null,"atlas.classifier.batch_sizes":null,"atlas.classifier.js_heap_current_bytes":null,"atlas.classifier.js_heap_peak_bytes":null,"atlas.classifier.runtime_reserved_bytes":null,"atlas.classifier.stack_limit_bytes":null,"atlas.classifier.unavailable_reason":reason})
        }
    }
}
impl CycleTelemetry {
    fn provider_calls(&mut self, counts: &ProviderCallCounts) {
        self.provider_counts.calls += counts.calls;
        self.provider_counts.transport_retries += counts.transport_retries;
        self.provider_counts.redirects += counts.redirects;
        self.provider_counts.fallbacks += counts.fallbacks;
    }
    /// One bounded aggregate per examined subgroup. No text, URLs, row labels or
    /// provenance enter this payload. Source/worker timings remain distinct.
    async fn classifier_chunk(
        &mut self,
        repo: &crate::storage::repo::Repository,
        context: &crate::atlas_classify::AdmissionContext,
        claims: &[crate::storage::classifier_admission::AdmissionClaim],
        observed: ClassifierChunkObservation<'_>,
    ) -> Result<()> {
        use crate::storage::classifier_admission::ItemCommit;
        let mut dispositions = std::collections::BTreeMap::<&str, u64>::new();
        for outcome in observed.results {
            let kind = match outcome {
                ItemCommit::Committed { .. } => "committed",
                ItemCommit::AlreadyCommitted { .. } => "already_committed",
                ItemCommit::StaleInput { .. } => "stale",
                ItemCommit::LeaseLost { .. } => "lease_lost",
                ItemCommit::Retryable { .. } => "retryable",
                ItemCommit::Cancelled { .. } => "cancelled",
                ItemCommit::InvalidReceipt { .. } => "invalid_receipt",
            };
            *dispositions.entry(kind).or_default() += 1;
        }
        let after = classifier_worker_observation(context).await;
        let mut metrics = classifier_worker_metrics(
            observed.worker_before,
            after.as_ref().map_err(|reason| *reason),
        );
        let object = metrics
            .as_object_mut()
            .ok_or_else(|| anyhow!("classifier metric shape"))?;
        let current = observed.cache;
        for (name, value) in [
            (
                "atlas.stage.wall_ms",
                serde_json::json!(observed.wall_micros as f64 / 1000.0),
            ),
            ("atlas.stage.queue_ms", serde_json::Value::Null),
            (
                "atlas.stage.queue_reason",
                serde_json::json!("registry_admission_timestamp_unavailable"),
            ),
            ("atlas.db", observed.db_after.delta(observed.db_before)),
            (
                "atlas.db.scope",
                serde_json::json!("classifier_owned_queries_and_transaction_boundaries"),
            ),
            ("atlas.rows.read", serde_json::json!(observed.rows_read)),
            ("atlas.rows.claimed", serde_json::json!(claims.len())),
            ("atlas.rows.dispositions", serde_json::json!(dispositions)),
            (
                "atlas.classifier.cache_hits",
                serde_json::json!(current.hits.checked_sub(observed.cache_before.hits)),
            ),
            (
                "atlas.classifier.cache_misses",
                serde_json::json!(current.misses.checked_sub(observed.cache_before.misses)),
            ),
            (
                "atlas.classifier.cache_evictions",
                serde_json::json!(current
                    .evictions
                    .checked_sub(observed.cache_before.evictions)),
            ),
            (
                "atlas.classifier.cache_bytes",
                serde_json::json!(current.bytes),
            ),
            (
                "atlas.classifier.model_id",
                serde_json::json!(context.pin.snapshot.model_id),
            ),
            (
                "atlas.classifier.artifact_sha256",
                serde_json::json!(context.pin.snapshot.artifact_sha256),
            ),
            (
                "atlas.classifier.manifest_sha256",
                serde_json::json!(context.manifest_sha256),
            ),
            (
                "atlas.classifier.taxonomy_version",
                serde_json::json!(context.pin.snapshot.taxonomy_version),
            ),
            (
                "atlas.classifier.policy_version",
                serde_json::json!(context.pin.snapshot.policy_version),
            ),
            (
                "atlas.classifier.computations",
                serde_json::json!(observed.page.map(|page| page.computations)),
            ),
            (
                "atlas.classifier.rejections",
                serde_json::json!(observed.page.map(|page| &page.rejections)),
            ),
            (
                "atlas.classifier.errors",
                serde_json::json!(observed.page.map(|page| &page.operational_errors)),
            ),
            (
                "atlas.classifier.outcome_metrics_reason",
                serde_json::json!(if observed.page.is_some() {
                    None
                } else {
                    Some("operation_did_not_return_page_metrics")
                }),
            ),
            (
                "atlas.host.buffer_peak_bytes",
                serde_json::json!(observed.page.map(|page| page.owned_text_buffer_peak_bytes)),
            ),
            (
                "atlas.host.buffer_reason",
                serde_json::json!(if observed.page.is_some() {
                    None
                } else {
                    Some("classification_not_completed")
                }),
            ),
            (
                "atlas.host.buffer_scope",
                serde_json::json!("adapter_title_description_string_capacities_only"),
            ),
            ("atlas.host.total_buffer_bytes", serde_json::Value::Null),
            (
                "atlas.host.total_buffer_reason",
                serde_json::json!("allocator_not_sampled"),
            ),
            ("atlas.process.rss_bytes", serde_json::Value::Null),
            (
                "atlas.process.rss_reason",
                serde_json::json!("process_rss_not_sampled"),
            ),
            (
                "atlas.provider.calls",
                serde_json::json!(self.provider_counts.calls),
            ),
            (
                "atlas.provider.transport_retries",
                serde_json::json!(self.provider_counts.transport_retries),
            ),
            (
                "atlas.provider.fallbacks",
                serde_json::json!(self.provider_counts.fallbacks),
            ),
        ] {
            object.insert(name.into(), value);
        }
        self.classifier_read += u64::try_from(observed.rows_read).unwrap_or(u64::MAX);
        self.classifier_claimed += u64::try_from(claims.len()).unwrap_or(u64::MAX);
        for (kind, count) in &dispositions {
            *self
                .classifier_dispositions
                .entry((*kind).into())
                .or_default() += count;
        }
        let baseline = *self.classifier_db_start.get_or_insert(observed.db_before);
        let queue_delta = observed
            .worker_before
            .zip(after.as_ref().ok())
            .and_then(|(before, after)| after.queue_micros.checked_sub(before.queue_micros));
        self.classifier_queue_micros = self
            .classifier_queue_micros
            .zip(queue_delta)
            .and_then(|(total, delta)| total.checked_add(delta));
        self.classifier_metrics = metrics.clone();
        self.classifier_metrics["atlas.classifier.queue_ms"] = serde_json::json!(self
            .classifier_queue_micros
            .map(|micros| micros as f64 / 1000.0));
        self.classifier_metrics["atlas.classifier.queue_reason"] =
            serde_json::json!(if self.classifier_queue_micros.is_some() {
                None
            } else {
                Some("one_or_more_queue_intervals_unavailable")
            });
        self.classifier_metrics["atlas.db"] = observed.db_after.delta(baseline);
        self.classifier_metrics["atlas.rows.read"] = serde_json::json!(self.classifier_read);
        self.classifier_metrics["atlas.rows.claimed"] = serde_json::json!(self.classifier_claimed);
        self.classifier_metrics["atlas.rows.dispositions"] =
            serde_json::json!(self.classifier_dispositions);
        self.classifier_metrics["atlas.classifier.cache_hits"] = serde_json::json!(current.hits);
        self.classifier_metrics["atlas.classifier.cache_misses"] =
            serde_json::json!(current.misses);
        self.classifier_metrics["atlas.classifier.cache_evictions"] =
            serde_json::json!(current.evictions);
        let first = claims.first().map(|claim| claim.work_seq).unwrap_or(0);
        let last = claims.last().map(|claim| claim.work_seq).unwrap_or(0);
        let payload = serde_json::json!({"stage":"classification","unit":"chunk","first_work_seq":first,"last_work_seq":last,"metrics":metrics});
        let payload = if serde_json::to_string(&payload)?.chars().count()
            <= telemetry::MAX_PAYLOAD_CHARS
        {
            payload
        } else {
            serde_json::json!({"stage":"classification","metrics":null,"unavailable_reason":"telemetry_payload_bound","rows":claims.len()})
        };
        let event = TelemetryEvent::new(EventKind::AtlasStage)
            .trigger(Trigger::AtlasCycle)
            .app("atlas")
            .run(&self.run_id)
            .tool("local_classifier")
            .category("batch")
            .model(&context.pin.snapshot.model_id)
            .outcome("examined")
            .duration_ms(i64::try_from(observed.wall_micros / 1000).ok())
            .count(1)
            .with_id(format!(
                "atlas-classifier-{}-{}-{first}-{last}",
                self.run_id, context.pin.snapshot.model_id
            ))
            .payload(payload);
        crate::storage::telemetry::record(repo.db(), &event).await?;
        Ok(())
    }
}
struct ClassifierChunkObservation<'a> {
    db_before: ClassifierDbSample,
    db_after: ClassifierDbSample,
    cache_before: crate::atlas_classify::CacheMetrics,
    cache: crate::atlas_classify::CacheMetrics,
    worker_before: Option<&'a crate::classifier::ClassifierPerformance>,
    page: Option<&'a crate::atlas_classify::PageMetrics>,
    results: &'a [crate::storage::classifier_admission::ItemCommit],
    rows_read: usize,
    wall_micros: u64,
}

#[cfg(test)]
#[path = "atlas_performance_tests.rs"]
mod classifier_performance_tests;
