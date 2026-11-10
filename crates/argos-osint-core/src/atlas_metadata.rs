//! Native article annotations. Candidate mentions and publisher priors are not evidence.
use crate::atlas_store::AtlasStore;
use crate::{
    nlp,
    osint::wikipedia_rsp as rsp,
    store::{AtlasArticleRow, Store},
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::{atomic::AtomicBool, Arc};

pub const ENTITY_VERSION: &str = "atlas-entities-v2-temporal-mentions";
pub const PRIOR_VERSION: &str = "publisher-rsp-v1";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataState {
    Pending,
    Complete,
    Skipped,
    Unavailable,
    Unsupported,
    Failed,
    Cancelled,
}
impl MetadataState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Complete => "complete",
            Self::Skipped => "skipped",
            Self::Unavailable => "unavailable",
            Self::Unsupported => "unsupported",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArticleMention {
    pub type_hint: String,
    pub comparison_key: String,
    pub field: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub original_text: String,
}

/// An article-local named candidate, never a claim of involvement or identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArticleActor {
    pub id: String,
    pub type_hint: String,
    pub display_name: String,
    pub spellings: Vec<String>,
    pub mentions: Vec<ArticleMention>,
}

pub fn article_actors(article: &AtlasArticleRow, metadata: &ArticleMetadata) -> Vec<ArticleActor> {
    if metadata.input_hash != input_hash(article)
        || metadata.entity_state != MetadataState::Complete
    {
        return Vec::new();
    }
    let mut actors: Vec<ArticleActor> = Vec::new();
    for mention in &metadata.mentions {
        let kind = match mention.type_hint.as_str() {
            "person" => "person",
            "org" | "organization" => "organization",
            "place" => "place",
            "unknown" | "unresolved" => "unresolved",
            _ => continue,
        };
        let text = match mention.field.as_str() {
            "title" => &article.title,
            "description" => &article.description,
            _ => continue,
        };
        if mention.start_byte >= mention.end_byte
            || text.get(mention.start_byte..mention.end_byte) != Some(&mention.original_text)
            || mention.original_text.trim().is_empty()
        {
            continue;
        }
        if temporal_person_mention(text, mention, &metadata.mentions) {
            continue;
        }
        // Normalize spelling, not aliases. Typed names from this packet share
        // only article-local identity; cross-source identity requires assessment.
        let key = crate::brain_actors::normalize_alias(&mention.original_text);
        let id = format!(
            "actor:{}:{}",
            article.id,
            nlp::cache::hash(&format!("{kind}:{key}"))
        );
        if let Some(actor) = actors.iter_mut().find(|actor| actor.id == id) {
            if !actor.spellings.contains(&mention.original_text) {
                actor.spellings.push(mention.original_text.clone());
            }
            if !actor.mentions.contains(mention) {
                actor.mentions.push(mention.clone());
            }
        } else {
            actors.push(ArticleActor {
                id,
                type_hint: kind.into(),
                display_name: crate::brain_actors::normalize_name(&mention.original_text),
                spellings: vec![mention.original_text.clone()],
                mentions: vec![mention.clone()],
            });
        }
    }
    actors
}

/// Add extraction entities that are not already article-local actors.
/// Identity is the actor id. A normalized display name can differ from the
/// raw spelling stored on the extraction entity, and those two rows still
/// name the same actor.
pub fn merge_output_actors(actors: &mut Vec<ArticleActor>, entities: &[nlp::EntityCandidate]) {
    for entity in entities {
        let name = entity
            .original_forms
            .first()
            .map(String::as_str)
            .unwrap_or("")
            .trim();
        if name.is_empty()
            || actors.iter().any(|actor| {
                actor.id == entity.id
                    || (actor.type_hint == entity.type_hint
                        && (actor.display_name.eq_ignore_ascii_case(name)
                            || actor
                                .spellings
                                .iter()
                                .any(|spelling| spelling.eq_ignore_ascii_case(name))))
            })
        {
            continue;
        }
        actors.push(ArticleActor {
            id: entity.id.clone(),
            type_hint: entity.type_hint.clone(),
            display_name: name.to_string(),
            spellings: entity.original_forms.clone(),
            mentions: Vec::new(),
        });
    }
}

/// Calendar mentions cannot become people just because a tagger also labels
/// them Person. Full names such as August Wilson and May Chen remain eligible.
pub(crate) fn temporal_person_mention(
    text: &str,
    mention: &ArticleMention,
    mentions: &[ArticleMention],
) -> bool {
    if mention.type_hint != "person" {
        return false;
    }
    nlp::calendar_month_person(
        text,
        mention.start_byte,
        mention.end_byte,
        &mention.original_text,
        mentions
            .iter()
            .filter(|m| m.type_hint == "date" && m.field == mention.field)
            .map(|m| (m.start_byte, m.end_byte)),
    )
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublisherPrior {
    pub observation: rsp::SourceReliabilityObservation,
    pub tool_id: String,
    pub matched_domain: Option<String>,
    pub fetched_at: String,
    pub score: Option<f64>,
    pub version: String,
    pub stale: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArticleMetadata {
    pub run_id: String,
    pub article_id: String,
    pub input_hash: String,
    pub entity_state: MetadataState,
    pub reliability_state: MetadataState,
    pub entity_version: String,
    pub prior_version: String,
    pub classification_status: String,
    pub classification_receipt: Option<serde_json::Value>,
    pub publisher_prior: Option<PublisherPrior>,
    pub mentions: Vec<ArticleMention>,
    pub diagnostics: Vec<String>,
    pub entity_ms: u64,
    pub publisher_ms: u64,
}
pub fn input_hash(article: &AtlasArticleRow) -> String {
    nlp::cache::hash(
        &serde_json::to_string(&(
            &article.title,
            &article.description,
            &article.url,
            &article.source_domain,
            &article.source_name,
        ))
        .expect("strings serialize"),
    )
}
pub fn comparison_key(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
impl ArticleMetadata {
    pub fn pending(article: &AtlasArticleRow) -> Self {
        Self {
            run_id: article.run_id.clone(),
            article_id: article.id.clone(),
            input_hash: input_hash(article),
            entity_state: MetadataState::Pending,
            reliability_state: MetadataState::Pending,
            entity_version: ENTITY_VERSION.into(),
            prior_version: PRIOR_VERSION.into(),
            classification_status: "pending".into(),
            classification_receipt: None,
            publisher_prior: None,
            mentions: vec![],
            diagnostics: vec![],
            entity_ms: 0,
            publisher_ms: 0,
        }
    }
}
/// One immutable index is supplied by the cycle; no per-article fetch or model call.
pub async fn collect(
    article: &AtlasArticleRow,
    index: Option<&rsp::RspIndex>,
    cancel: Arc<AtomicBool>,
) -> ArticleMetadata {
    collect_inner(article, index, cancel, None).await
}
async fn collect_inner(
    article: &AtlasArticleRow,
    index: Option<&rsp::RspIndex>,
    cancel: Arc<AtomicBool>,
    db_path: Option<&std::path::Path>,
) -> ArticleMetadata {
    let mut out = ArticleMetadata::pending(article);
    let started = std::time::Instant::now();
    let mode = nlp::configured_mode();
    let use_remote = nlp::remote::is_configured();
    out.entity_state = MetadataState::Complete;
    if mode == nlp::Mode::Off {
        out.entity_state = MetadataState::Skipped;
    } else if !cfg!(feature = "nlp") && !use_remote {
        out.entity_state = MetadataState::Unavailable;
        out.diagnostics
            .push("Local NLP is unavailable in this build".into());
    } else {
        let fields: Vec<_> = [
            ("title", article.title.as_str()),
            ("description", article.description.as_str()),
        ]
        .into_iter()
        .filter(|(_, text)| !text.is_empty())
        .map(|(field, text)| {
            let mut source = nlp::Source::derived(
                &format!("article:{}:{field}", article.id),
                &out.input_hash,
                text,
            );
            source.published_at = Some(article.published_at.clone());
            (field, text, source)
        })
        .collect();
        let sources: Vec<_> = fields.iter().map(|(_, _, source)| source).collect();
        let annotations = if use_remote {
            nlp::remote::annotate_configured(&sources, &cancel).await
        } else {
            let policy = nlp::PreparePolicy {
                purpose: "atlas-entities".into(),
                coverage_requirement: nlp::CoverageRequirement::Complete,
                need_entities: true,
                byte_budget: usize::MAX,
            };
            let mut annotations = Vec::with_capacity(sources.len());
            for source in sources {
                annotations.push(
                    nlp::annotate_with_policy(source, &policy, cancel.clone(), mode, db_path).await,
                );
            }
            annotations
        };
        let parser = if use_remote {
            "NLP service"
        } else {
            "local NLP"
        };
        for ((field, text, source), annotation) in fields.into_iter().zip(annotations) {
            if annotation.status == nlp::Status::Complete && annotation.validate(&source) {
                for date in &annotation.date_candidates {
                    if let Some(original) = date.span.slice(text) {
                        out.mentions.push(ArticleMention {
                            type_hint: "date".into(),
                            comparison_key: comparison_key(original),
                            field: field.into(),
                            start_byte: date.span.start_byte,
                            end_byte: date.span.end_byte,
                            original_text: original.into(),
                        });
                    }
                }
                for mention in annotation.mentions {
                    if let Some(original) = mention.span.slice(text) {
                        out.mentions.push(ArticleMention {
                            type_hint: mention.kind,
                            comparison_key: comparison_key(original),
                            field: field.into(),
                            start_byte: mention.span.start_byte,
                            end_byte: mention.span.end_byte,
                            original_text: original.into(),
                        });
                    }
                }
            } else {
                out.entity_state = match annotation.status {
                    nlp::Status::Unsupported => MetadataState::Unsupported,
                    nlp::Status::Disabled => MetadataState::Unavailable,
                    nlp::Status::Cancelled => MetadataState::Cancelled,
                    _ => MetadataState::Failed,
                };
                let reasons = annotation
                    .flags
                    .iter()
                    .take(3)
                    .map(|s| {
                        s.chars()
                            .filter(|c| !c.is_control())
                            .take(160)
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                out.diagnostics.push(format!(
                    "{field} {parser}: {:?}{}",
                    annotation.status,
                    if reasons.is_empty() {
                        String::new()
                    } else {
                        format!(" ({reasons})")
                    }
                ));
            }
        }
    }
    out.entity_ms = started.elapsed().as_millis() as u64;
    set_publisher(article, index, &mut out);
    out
}
fn set_publisher(
    article: &AtlasArticleRow,
    index: Option<&rsp::RspIndex>,
    out: &mut ArticleMetadata,
) {
    out.diagnostics
        .retain(|d| !d.starts_with("Publisher reliability"));
    out.publisher_prior = None;
    let started = std::time::Instant::now();
    let host = rsp::normalize_host(&article.url);
    let host = if host.is_empty() {
        rsp::normalize_host(&article.source_domain)
    } else {
        host
    };
    if let Some(index) = index {
        let entry = index.lookup_domain(&host);
        let observation =
            rsp::SourceReliabilityObservation::from_lookup(&host, &article.source_name, entry);
        out.publisher_prior = Some(PublisherPrior {
            score: entry.map(|e| e.status.reliability().meter() * 100.0),
            observation,
            tool_id: rsp::TOOL_ID.into(),
            matched_domain: entry.and_then(|e| {
                e.domains
                    .iter()
                    .filter(|d| host == **d || host.ends_with(&format!(".{d}")))
                    .max_by_key(|d| d.len())
                    .cloned()
            }),
            fetched_at: index.fetched_at.clone(),
            version: PRIOR_VERSION.into(),
            stale: !rsp::index_is_fresh(index),
        });
        out.reliability_state = MetadataState::Complete;
    } else {
        out.reliability_state = MetadataState::Unavailable;
        out.diagnostics
            .push("Publisher reliability index unavailable; initial confidence unassessed".into());
    }
    out.publisher_ms = started.elapsed().as_millis() as u64;
}
impl Store {
    pub fn atlas_metadata(
        &self,
        run_id: &str,
        article_id: &str,
    ) -> Result<Option<ArticleMetadata>> {
        crate::storage::jobs_store::atlas_metadata(run_id, article_id)
    }
    /// Reject work captured before a source replacement. The savepoint atomically replaces mentions.
    pub fn atlas_save_metadata(&self, metadata: &ArticleMetadata) -> Result<bool> {
        crate::storage::jobs_store::atlas_save_metadata(metadata)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Coverage {
    pub entities_complete: u32,
    pub entities_unavailable: u32,
    pub publishers_available: u32,
    pub related_articles: u32,
    pub warnings: Vec<String>,
}
/// Backfill native metadata without reclassifying historical completed cycles.
pub async fn process_run(
    db_path: &std::path::Path,
    run_id: &str,
    cancel: Arc<AtomicBool>,
    _user_agent: &str,
) -> Result<Coverage> {
    use crate::storage::classifier_admission::{self, DerivativeStage};
    let store = AtlasStore::open(db_path).await?;
    let repo = store
        .repository()
        .ok_or_else(|| anyhow::anyhow!("native metadata repository missing"))?;
    let owner = classifier_admission::AdmissionOwner::new(repo.clone());
    let result = async {
        let mut after = None;
        let mut coverage = Coverage::default();
        loop {
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                break;
            }
            let keys = crate::storage::atlas::current_occurrence_keys_page(
                repo,
                Some(run_id),
                None,
                after.as_ref(),
            )
            .await?;
            if keys.is_empty() {
                break;
            }
            after = keys.last().cloned();
            let report = classifier_admission::process_native_refresh_stage(
                &owner,
                &keys,
                DerivativeStage::Metadata,
                &cancel,
            )
            .await?;
            for state in report.states {
                if !state
                    .stages
                    .iter()
                    .find(|stage| stage.stage == DerivativeStage::Metadata)
                    .and_then(|stage| stage.disposition.as_ref())
                    .is_some_and(classifier_admission::DerivativeDisposition::terminal)
                {
                    coverage
                        .warnings
                        .push("Native metadata remains unavailable or changed".into());
                }
            }
        }
        let summary = classifier_admission::native_run_summary(repo, run_id).await?;
        let retained = classifier_admission::run_summary(repo, run_id).await?;
        if retained.legacy > 0 || retained.staged > 0 {
            coverage.warnings.push(
                "Legacy or pending occurrences remain ineligible for new native metadata".into(),
            );
        }
        coverage.entities_complete = u32::try_from(summary.entities).unwrap_or(u32::MAX);
        coverage.publishers_available = u32::try_from(summary.publishers).unwrap_or(u32::MAX);
        coverage.entities_unavailable = coverage.warnings.len() as u32;
        Ok::<_, anyhow::Error>(coverage)
    }
    .await;
    let cleanup = owner.shutdown().await;
    let result = result?;
    cleanup?;
    Ok(result)
}

async fn record_unit(
    db: &std::path::Path,
    store: &AtlasStore,
    m: &ArticleMetadata,
    stage: i32,
    state: &MetadataState,
    elapsed: u64,
) -> Result<()> {
    let disposition = match state {
        MetadataState::Complete | MetadataState::Skipped => crate::atlas_work::DISPOSITION_SUCCESS,
        MetadataState::Pending | MetadataState::Cancelled => {
            crate::atlas_work::DISPOSITION_INCOMPLETE
        }
        _ => crate::atlas_work::DISPOSITION_FAILED,
    };
    let manifest = crate::atlas_work::UnitManifest {
        run_id: m.run_id.clone(),
        unit_id: format!(
            "{}:{}:{}",
            crate::atlas_work::stage_label(stage),
            m.article_id,
            m.input_hash
        ),
        stage,
        input_ids: vec![m.article_id.clone()],
        input_rev: m.input_hash.clone(),
        contract_version: crate::atlas_work::COLLECTION_CONTRACT.into(),
        dependency_ids: vec![],
        is_required: false,
        output_refs: vec![format!("metadata:{}:{}", m.run_id, m.article_id)],
        effective_model: String::new(),
        attempt_history: vec![],
        next_eligible_at: None,
        terminal_reason: if matches!(state, MetadataState::Complete | MetadataState::Skipped) {
            None
        } else {
            Some(state.as_str().into())
        },
        output_json: serde_json::to_string(
            &serde_json::json!({"state":state,"elapsed_ms":elapsed}),
        )?,
        disposition: disposition.into(),
        receipt_json: serde_json::to_string(m)?,
    };
    store.save_unit_manifest(&manifest).await?;
    // `telemetry::record` routes to the selected store.
    crate::atlas_work::record_unit_stage_telemetry(db, &manifest);
    Ok(())
}
impl Store {
    pub fn atlas_set_classification(
        &self,
        run_id: &str,
        article_id: &str,
        captured_hash: &str,
        status: &str,
        receipt_json: &str,
        category: Option<&str>,
    ) -> Result<bool> {
        crate::storage::jobs_store::atlas_set_classification(
            run_id,
            article_id,
            captured_hash,
            status,
            receipt_json,
            category,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actor_projection_validates_utf8_repeated_mentions_types_and_staleness() {
        let mut a = article();
        a.title = "🌎 Jane Doe meets Jane Doe".into();
        a.description = "Jane Doe met Acme.".into();
        let mut metadata = ArticleMetadata::pending(&a);
        metadata.entity_state = MetadataState::Complete;
        for (field, start, end, text, kind) in [
            ("title", 5, 13, "Jane Doe", "person"),
            ("title", 20, 28, "Jane Doe", "person"),
            ("description", 0, 8, "Jane Doe", "person"),
            ("description", 13, 17, "Acme", "org"),
            ("title", 1, 3, "bad", "person"),
            ("description", 0, 8, "Jane Doe", "date"),
        ] {
            metadata.mentions.push(ArticleMention {
                type_hint: kind.into(),
                comparison_key: text.to_lowercase(),
                field: field.into(),
                start_byte: start,
                end_byte: end,
                original_text: text.into(),
            });
        }
        let actors = article_actors(&a, &metadata);
        assert_eq!(actors.len(), 2);
        assert_eq!(actors[0].mentions.len(), 3);
        a.description.push('!');
        assert!(article_actors(&a, &metadata).is_empty());
        metadata.input_hash = input_hash(&a);
        metadata.entity_state = MetadataState::Unavailable;
        assert!(article_actors(&a, &metadata).is_empty());
    }

    fn article() -> AtlasArticleRow {
        AtlasArticleRow {
            run_id: "r".into(),
            id: "a".into(),
            title: "Jane Doe speaks".into(),
            description: "Original body 🌎".into(),
            url: "https://news.bbc.co.uk/story".into(),
            country: String::new(),
            source_name: "BBC".into(),
            source_domain: "bbc.co.uk".into(),
            published_at: "2026-10-12".into(),
            provider: "fixture".into(),
            temperature: 0.0,
            category: "unk".into(),
            seen_at: String::new(),
            author: String::new(),
            image_url: String::new(),
            categories: Vec::new(),
            admitted_at: String::new(),
        }
    }
    #[tokio::test]
    async fn native_metadata_preserves_f_null_and_typed_complete_input() {
        let a = article();
        let index = rsp::RspIndex::build(vec![], chrono::Utc::now().to_rfc3339());
        let m = collect(&a, Some(&index), Arc::new(AtomicBool::new(false))).await;
        assert_eq!(m.publisher_prior.unwrap().score, None);
        assert_eq!(m.reliability_state, MetadataState::Complete);
        #[cfg(not(feature = "nlp"))]
        assert_eq!(m.entity_state, MetadataState::Unavailable);
        assert_eq!(m.input_hash, input_hash(&a));
    }
    #[tokio::test]
    async fn publisher_prior_uses_real_url_host_not_spoofed_publisher() {
        let mut a = article();
        a.url = "https://bbc.co.uk@evil.example/story".into();
        let index = rsp::RspIndex::build(
            vec![rsp::RspEntry {
                name: "BBC".into(),
                domains: vec!["bbc.co.uk".into()],
                status: rsp::RspStatus::GenerallyReliable,
                blacklisted: false,
                last_year: String::new(),
                summary: String::new(),
            }],
            chrono::Utc::now().to_rfc3339(),
        );
        let m = collect(&a, Some(&index), Arc::new(AtomicBool::new(false))).await;
        let prior = m.publisher_prior.unwrap();
        assert_eq!(prior.observation.domain, "evil.example");
        assert_eq!(prior.score, None);
    }
}

/// Collection feeds this bounded worker. Admission waits rather than omitting
/// annotations under pressure, and no publisher request starts for an empty cycle.
pub struct NativeInventory {
    sender: Option<tokio::sync::mpsc::Sender<AtlasArticleRow>>,
    task: Option<tokio::task::JoinHandle<Result<()>>>,
    cancel: Arc<AtomicBool>,
}
impl NativeInventory {
    pub fn start(db_path: &std::path::Path, run_id: &str, user_agent: &str) -> Self {
        let (sender, receiver) = tokio::sync::mpsc::channel(32);
        let cancel = Arc::new(AtomicBool::new(false));
        let db = db_path.to_path_buf();
        let run = run_id.to_owned();
        let agent = user_agent.to_owned();
        let stop = cancel.clone();
        let task =
            tokio::spawn(async move { inventory_worker(db, run, agent, receiver, stop).await });
        Self {
            sender: Some(sender),
            task: Some(task),
            cancel,
        }
    }
    pub async fn queue(&self, article: &AtlasArticleRow) -> Result<()> {
        if self.cancel.load(std::sync::atomic::Ordering::Acquire) {
            anyhow::bail!("Native article inventory cancelled");
        }
        self.sender
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Native article inventory closed"))?
            .send(article.clone())
            .await
            .map_err(|_| anyhow::anyhow!("Native article inventory stopped"))
    }
    pub fn cancel(&self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Release);
        if let Some(task) = &self.task {
            task.abort();
        }
    }
    pub async fn finish(mut self) -> Result<()> {
        self.sender.take();
        if let Some(task) = self.task.as_mut() {
            task.await
                .map_err(|e| anyhow::anyhow!("Native inventory worker: {e}"))??;
        }
        self.task.take();
        Ok(())
    }
}
impl Drop for NativeInventory {
    fn drop(&mut self) {
        self.cancel();
    }
}

async fn inventory_index(
    store: &AtlasStore,
    db: &std::path::Path,
    user_agent: &str,
    enabled: bool,
) -> Option<rsp::RspIndex> {
    if !enabled {
        return None;
    }
    let stored = store.app_state_get(rsp::APP_STATE_KEY).await.ok().flatten();
    #[cfg(test)]
    {
        let _ = (user_agent, db);
        stored
            .as_deref()
            .and_then(|j| rsp::index_from_json(j).ok())
            .or_else(rsp::last_good_index)
    }
    #[cfg(not(test))]
    {
        match rsp::warm_with_store_json_for_db(
            stored.as_deref(),
            user_agent,
            crate::recon::backend::legacy_side_store_path(db).map(std::path::Path::to_path_buf),
        )
        .await
        {
            Ok((index, changed)) => {
                if changed {
                    if let Ok(json) = rsp::index_to_json(&index) {
                        let _ = store.app_state_set(rsp::APP_STATE_KEY, &json).await;
                    }
                }
                Some(index)
            }
            Err(_) => None,
        }
    }
}
async fn flush_inventory_publishers(
    store: &AtlasStore,
    db: &std::path::Path,
    run: &str,
    pending: &mut Vec<(String, String)>,
    index: Option<&rsp::RspIndex>,
    enabled: bool,
) -> Result<()> {
    for (id, hash) in pending.drain(..) {
        let Some(article) = store.article(run, &id).await? else {
            continue;
        };
        if input_hash(&article) != hash {
            continue;
        }
        let Some(mut metadata) = store.metadata(run, &id).await? else {
            continue;
        };
        if metadata.input_hash != hash {
            continue;
        }
        if enabled {
            set_publisher(&article, index, &mut metadata);
        } else {
            metadata.reliability_state = MetadataState::Skipped;
            metadata.publisher_prior = None;
        }
        if store.save_metadata(&metadata).await? {
            record_unit(
                db,
                store,
                &metadata,
                crate::atlas_work::STAGE_PUBLISHERS,
                &metadata.reliability_state,
                metadata.publisher_ms,
            )
            .await?;
        }
    }
    Ok(())
}
async fn inventory_worker(
    db: std::path::PathBuf,
    run: String,
    user_agent: String,
    mut receiver: tokio::sync::mpsc::Receiver<AtlasArticleRow>,
    cancel: Arc<AtomicBool>,
) -> Result<()> {
    let store = AtlasStore::open(&db).await?;
    store
        .app_state_delete(&format!("atlas-publisher-failure:{run}"))
        .await?;
    let Some(first) = receiver.recv().await else {
        return Ok(());
    };
    let legacy_db = crate::recon::backend::legacy_side_store_path(&db);
    let enabled = store.tool_enabled(rsp::TOOL_ID).await?;
    let entity_job = metadata_job(
        &db,
        &run,
        "atlas_entities",
        "Collection entity inventory",
        cancel.clone(),
    );
    let publisher_job = metadata_job(
        &db,
        &run,
        "atlas_publishers",
        "Collection publisher priors",
        cancel.clone(),
    );
    let publisher = inventory_index(&store, &db, &user_agent, enabled);
    tokio::pin!(publisher);
    let mut index: Option<Option<rsp::RspIndex>> = None;
    let mut pending = Vec::new();
    let mut next = Some(first);
    let mut done = 0;
    loop {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        if let Some(article) = next.take() {
            let native = collect_inner(&article, None, cancel.clone(), legacy_db.as_deref());
            tokio::pin!(native);
            let mut metadata = loop {
                tokio::select! {
                 metadata=&mut native=>break metadata,
                 result=&mut publisher,if index.is_none()=>{
                  index=Some(result);flush_inventory_publishers(&store,&db,&run,&mut pending,index.as_ref().and_then(|i|i.as_ref()),enabled).await?;
                 }
                }
            };
            if let Some(index) = &index {
                if enabled {
                    set_publisher(&article, index.as_ref(), &mut metadata);
                } else {
                    metadata.reliability_state = MetadataState::Skipped;
                    metadata.publisher_prior = None;
                    metadata
                        .diagnostics
                        .retain(|d| !d.starts_with("Publisher reliability"));
                }
            } else {
                metadata.reliability_state = MetadataState::Pending;
                metadata.publisher_prior = None;
                metadata
                    .diagnostics
                    .retain(|d| !d.starts_with("Publisher reliability"));
            }
            if store.save_metadata(&metadata).await? {
                record_unit(
                    &db,
                    &store,
                    &metadata,
                    crate::atlas_work::STAGE_ENTITIES,
                    &metadata.entity_state,
                    metadata.entity_ms,
                )
                .await?;
                if index.is_some() {
                    record_unit(
                        &db,
                        &store,
                        &metadata,
                        crate::atlas_work::STAGE_PUBLISHERS,
                        &metadata.reliability_state,
                        metadata.publisher_ms,
                    )
                    .await?;
                } else {
                    pending.push((article.id.clone(), metadata.input_hash.clone()));
                }
            }
            done += 1;
            if let Some(job) = &entity_job {
                job.phase("Collected article entities", Some(done), None);
            }
        }
        tokio::select! {
         article=receiver.recv()=>{match article {Some(article)=>next=Some(article),None=>{break;}}},
         result=&mut publisher,if index.is_none()=>{index=Some(result);flush_inventory_publishers(&store,&db,&run,&mut pending,index.as_ref().and_then(|i|i.as_ref()),enabled).await?;}
        }
    }
    if !cancel.load(std::sync::atomic::Ordering::Acquire) {
        if index.is_none() {
            index = Some(publisher.await);
        }
        flush_inventory_publishers(
            &store,
            &db,
            &run,
            &mut pending,
            index.as_ref().and_then(|i| i.as_ref()),
            enabled,
        )
        .await?;
    }
    if enabled && !cancel.load(std::sync::atomic::Ordering::Acquire) {
        let key = format!("atlas-publisher-failure:{run}");
        if index.as_ref().is_none_or(|i| i.is_none()) {
            store.app_state_set(&key, "1").await?;
        } else {
            store.app_state_delete(&key).await?;
        }
    }
    if let Some(job) = entity_job {
        job.finish(crate::job_registry::Finish::completed());
    }
    if let Some(job) = publisher_job {
        job.finish(crate::job_registry::Finish::completed());
    }
    Ok(())
}

#[cfg(test)]
mod inventory_tests {
    use super::{merge_output_actors, ArticleActor};
    use crate::nlp::EntityCandidate;

    #[test]
    fn output_entities_do_not_repeat_an_actor_id() {
        let mut actors = vec![ArticleActor {
            id: "actor:art-1:same".into(),
            type_hint: "organization".into(),
            display_name: "Port Authority".into(),
            spellings: vec!["Port Authority's".into()],
            mentions: Vec::new(),
        }];
        merge_output_actors(
            &mut actors,
            &[
                EntityCandidate {
                    id: "actor:art-1:same".into(),
                    type_hint: "organization".into(),
                    comparison_key: "port authority s".into(),
                    original_forms: vec!["Port Authority's".into()],
                    mention_refs: Vec::new(),
                },
                EntityCandidate {
                    id: "actor:art-1:other".into(),
                    type_hint: "person".into(),
                    comparison_key: "ada".into(),
                    original_forms: vec!["Ada".into()],
                    mention_refs: Vec::new(),
                },
            ],
        );
        assert_eq!(
            actors
                .iter()
                .map(|actor| actor.id.as_str())
                .collect::<Vec<_>>(),
            vec!["actor:art-1:same", "actor:art-1:other"]
        );
    }
}

fn metadata_job(
    _db: &std::path::Path,
    _run: &str,
    _operation: &str,
    _title: &str,
    _cancel: Arc<AtomicBool>,
) -> Option<crate::job_registry::JobHandle> {
    // Job tracking lived in the legacy store; under SurrealDB Atlas runs untracked.
    None
}

/// Enrichs one bounded captured occurrence using local NLP and the retained
/// publisher index. Maintenance never refreshes the index over the network.
pub(crate) async fn process_derivative(
    owner: &crate::storage::classifier_admission::AdmissionOwner,
    fence: &crate::storage::classifier_admission::DerivativeFence,
    claim: &crate::storage::classifier_admission::DerivativeClaim,
    cancel: &AtomicBool,
) -> Result<crate::storage::classifier_admission::DerivativeDisposition> {
    use crate::storage::classifier_admission::{self, DerivativeDisposition};
    use crate::storage::tx::{RetryPolicy, TxError};
    if cancel.load(std::sync::atomic::Ordering::Acquire) {
        return Ok(DerivativeDisposition::Retryable {
            code: "cancelled".into(),
        });
    }
    let repo = owner.repository();
    let enabled = crate::storage::recon_threads::tool_enabled(repo, rsp::TOOL_ID).await?;
    let raw = crate::storage::pipeline_inputs::app_state_get(repo, rsp::APP_STATE_KEY).await?;
    let index = if enabled {
        raw.as_deref()
            .and_then(|json| rsp::index_from_json(json).ok())
    } else {
        None
    };
    let signal = Arc::new(AtomicBool::new(false));
    struct CancelNative(Arc<AtomicBool>);
    impl Drop for CancelNative {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::Release);
        }
    }
    let _stop = CancelNative(signal.clone());
    let mut work = Box::pin(collect_inner(
        &claim.article,
        index.as_ref(),
        signal.clone(),
        None,
    ));
    let mut metadata = tokio::select! {
        metadata=&mut work=>metadata,
        _=async {while !cancel.load(std::sync::atomic::Ordering::Acquire){tokio::time::sleep(std::time::Duration::from_millis(50)).await;}}=> {signal.store(true,std::sync::atomic::Ordering::Release);work.await},
    };
    if !enabled {
        metadata.reliability_state = MetadataState::Skipped;
        metadata.publisher_prior = None;
        metadata
            .diagnostics
            .retain(|line| !line.starts_with("Publisher reliability"));
    }
    let entities_ready = matches!(
        metadata.entity_state,
        MetadataState::Complete | MetadataState::Skipped | MetadataState::Unsupported
    );
    let publisher_ready = matches!(
        metadata.reliability_state,
        MetadataState::Complete | MetadataState::Skipped
    );
    let disposition = if cancel.load(std::sync::atomic::Ordering::Acquire) {
        DerivativeDisposition::Retryable {
            code: "cancelled".into(),
        }
    } else if entities_ready && publisher_ready {
        if metadata.entity_state == MetadataState::Skipped && !enabled {
            DerivativeDisposition::Disabled {
                reason: "native_tools_disabled".into(),
            }
        } else {
            DerivativeDisposition::Current
        }
    } else {
        DerivativeDisposition::Retryable {
            code: "native_metadata_unavailable".into(),
        }
    };
    let fence = fence.clone();
    let claim = claim.clone();
    let result = disposition.clone();
    let committed = classifier_admission::owned_tx(owner, RetryPolicy::default(), move |tx| {
        let fence = fence.clone();
        let claim = claim.clone();
        let metadata = metadata.clone();
        let disposition = result.clone();
        async move {
            let now = chrono::Utc::now();
            if !classifier_admission::check_derivative_in(&tx, &fence, &claim, now).await? {
                return Ok(DerivativeDisposition::StaleInput);
            }
            if !crate::storage::atlas_metadata::save_metadata_in(&tx, &metadata).await? {
                return Err(TxError::Abort("native_metadata_source_changed".into()));
            }
            if !classifier_admission::complete_derivative_in(
                &tx,
                &fence,
                &claim,
                &disposition,
                "",
                now,
            )
            .await?
            {
                return Err(TxError::Abort("native_metadata_lease_lost".into()));
            }
            Ok(disposition)
        }
    })
    .await?;
    Ok(committed.value)
}
