//! Native article annotations. Candidate mentions and publisher priors are not evidence.
use crate::{
    nlp,
    osint::wikipedia_rsp as rsp,
    store::{AtlasArticleRow, Store},
};
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
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
        let key = mention
            .original_text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
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
                display_name: mention.original_text.clone(),
                spellings: vec![mention.original_text.clone()],
                mentions: vec![mention.clone()],
            });
        }
    }
    actors
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
    out.entity_state = MetadataState::Complete;
    if mode == nlp::Mode::Off {
        out.entity_state = MetadataState::Skipped;
    } else if !cfg!(feature = "nlp") {
        out.entity_state = MetadataState::Unavailable;
        out.diagnostics
            .push("Local NLP is unavailable in this build".into());
    } else {
        for (field, text) in [
            ("title", article.title.as_str()),
            ("description", article.description.as_str()),
        ] {
            if text.is_empty() {
                continue;
            }
            let mut source = nlp::Source::derived(
                &format!("article:{}:{field}", article.id),
                &out.input_hash,
                text,
            );
            source.published_at = Some(article.published_at.clone());
            let policy = nlp::PreparePolicy {
                purpose: "atlas-entities".into(),
                coverage_requirement: nlp::CoverageRequirement::Complete,
                need_entities: true,
                byte_budget: usize::MAX,
            };
            let annotation =
                nlp::annotate_with_policy(&source, &policy, cancel.clone(), mode, db_path).await;
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
                    "{field} local NLP: {:?}{}",
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
        let json:Option<String>=self.conn.query_row("SELECT metadata_json FROM atlas_article_metadata WHERE run_id=?1 AND article_id=?2",params![run_id,article_id],|r|r.get(0)).optional()?;
        Ok(json.map(|j| serde_json::from_str(&j)).transpose()?)
    }
    /// Reject work captured before a source replacement. The savepoint atomically replaces mentions.
    pub fn atlas_save_metadata(&self, metadata: &ArticleMetadata) -> Result<bool> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let Some(article) = self.atlas_article(&metadata.run_id, &metadata.article_id)? else {
            return Ok(false);
        };
        if input_hash(&article) != metadata.input_hash {
            return Ok(false);
        }
        let mut metadata = metadata.clone();
        if let Some(latest) = self.atlas_metadata(&metadata.run_id, &metadata.article_id)? {
            if latest.input_hash == metadata.input_hash {
                metadata.classification_status = latest.classification_status;
                metadata.classification_receipt = latest.classification_receipt;
            }
        }
        let serialized = serde_json::to_string(&metadata)?;
        let existing:Option<String>=tx.query_row("SELECT metadata_json FROM atlas_article_metadata WHERE run_id=?1 AND article_id=?2",params![metadata.run_id,metadata.article_id],|r|r.get(0)).optional()?;
        if existing.as_deref() == Some(serialized.as_str()) {
            tx.commit()?;
            return Ok(true);
        }
        tx.execute("INSERT OR REPLACE INTO atlas_article_metadata(run_id,article_id,input_hash,entity_state,reliability_state,classification_status,metadata_json) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![metadata.run_id,metadata.article_id,metadata.input_hash,metadata.entity_state.as_str(),metadata.reliability_state.as_str(),metadata.classification_status,serde_json::to_string(&metadata)?])?;
        tx.execute(
            "DELETE FROM atlas_article_mentions WHERE run_id=?1 AND article_id=?2",
            params![metadata.run_id, metadata.article_id],
        )?;
        tx.execute(
            "DELETE FROM atlas_related_articles WHERE run_id=?1 AND (left_id=?2 OR right_id=?2)",
            params![metadata.run_id, metadata.article_id],
        )?;
        for m in &metadata.mentions {
            let text = match m.field.as_str() {
                "title" => &article.title,
                "description" => &article.description,
                _ => continue,
            };
            if text.get(m.start_byte..m.end_byte) != Some(m.original_text.as_str())
                || comparison_key(&m.original_text) != m.comparison_key
            {
                continue;
            }
            tx.execute(
                "INSERT OR IGNORE INTO atlas_article_mentions VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    metadata.run_id,
                    metadata.article_id,
                    m.type_hint,
                    m.comparison_key,
                    m.field,
                    m.start_byte as i64,
                    m.end_byte as i64,
                    m.original_text,
                    metadata.input_hash
                ],
            )?;
        }
        crate::intel_recon::persist_article_rating(&tx, &metadata.article_id)?;
        tx.commit()?;
        Ok(true)
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
    user_agent: &str,
) -> Result<Coverage> {
    let mut coverage = Coverage::default();
    if cancel.load(std::sync::atomic::Ordering::Acquire) {
        return Ok(coverage);
    }
    let publishers_enabled = crate::store::Store::open(db_path)?.tool_enabled(rsp::TOOL_ID)?;
    let articles = Store::open(db_path)?.atlas_list_articles(run_id)?;
    if articles.is_empty() {
        return Ok(coverage);
    }
    let entity_job = metadata_job(
        db_path,
        run_id,
        "atlas_entities",
        "Local entity inventory",
        cancel.clone(),
    );
    let publisher_job = metadata_job(
        db_path,
        run_id,
        "atlas_publishers",
        "Publisher initial confidence",
        cancel.clone(),
    );
    let total = articles.len() as i64;
    let stored = Store::open(db_path)?.app_state_get(rsp::APP_STATE_KEY)?;
    #[cfg(test)]
    let index = stored
        .as_deref()
        .and_then(|j| rsp::index_from_json(j).ok())
        .or_else(rsp::last_good_index);
    #[cfg(not(test))]
    let failed_in_inventory = {
        let store = Store::open(db_path)?;
        let key = format!("atlas-publisher-failure:{run_id}");
        let failed = store.app_state_get(&key)?.as_deref() == Some("1");
        store
            .conn
            .execute("DELETE FROM app_state WHERE key=?1", [key])?;
        failed
    };
    #[cfg(not(test))]
    let index = if !publishers_enabled || failed_in_inventory {
        None
    } else {
        let lookup = tokio::select! {
            result=rsp::warm_with_store_json_for_db(stored.as_deref(),user_agent,Some(db_path.to_path_buf()))=>Some(result),
            _=wait_cancelled(cancel.clone())=>None,
        };
        match lookup {
            None => None,
            Some(result) => match result {
                Ok((index, changed)) => {
                    if changed {
                        Store::open(db_path)?
                            .app_state_set(rsp::APP_STATE_KEY, &rsp::index_to_json(&index)?)?;
                    }
                    Some(index)
                }
                Err(e) => {
                    coverage
                        .warnings
                        .push(format!("Publisher lookup unavailable: {e}"));
                    None
                }
            },
        }
    };
    #[cfg(test)]
    let _ = user_agent;
    for (index_in_run, article) in articles.into_iter().enumerate() {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        let existing = Store::open(db_path)?.atlas_metadata(run_id, &article.id)?;
        let mut metadata = if let Some(existing) = existing.filter(|m| {
            let mode = nlp::configured_mode();
            let entity_reusable = m.entity_version == ENTITY_VERSION
                && match mode {
                    nlp::Mode::Off => m.entity_state == MetadataState::Skipped,
                    _ if !cfg!(feature = "nlp") => m.entity_state == MetadataState::Unavailable,
                    _ => m.entity_state == MetadataState::Complete,
                };
            let publisher_reusable = if !publishers_enabled {
                m.reliability_state == MetadataState::Skipped
            } else {
                m.prior_version == PRIOR_VERSION
                    && m.reliability_state == MetadataState::Complete
                    && index.as_ref().is_some_and(|i| {
                        m.publisher_prior.as_ref().is_some_and(|p| {
                            p.fetched_at == i.fetched_at
                                && p.version == PRIOR_VERSION
                                && p.stale != rsp::index_is_fresh(i)
                        })
                    })
            };
            m.input_hash == input_hash(&article) && entity_reusable && publisher_reusable
        }) {
            existing
        } else {
            collect_inner(
                &article,
                if publishers_enabled {
                    index.as_ref()
                } else {
                    None
                },
                cancel.clone(),
                Some(db_path),
            )
            .await
        };
        if !publishers_enabled {
            metadata.reliability_state = MetadataState::Skipped;
            metadata.publisher_prior = None;
            metadata
                .diagnostics
                .retain(|d| !d.starts_with("Publisher reliability"));
        }
        // Classification may finish while native work is in flight. Merge the durable receipt.
        let store = Store::open(db_path)?;
        if let Some(latest) = store.atlas_metadata(run_id, &article.id)? {
            if latest.input_hash == metadata.input_hash {
                metadata.classification_status = latest.classification_status;
                metadata.classification_receipt = latest.classification_receipt;
            }
        } else if article.category != "unk" {
            metadata.classification_status = "classified".into();
        }
        if metadata.entity_state == MetadataState::Complete {
            coverage.entities_complete += 1;
        } else if metadata.entity_state != MetadataState::Skipped {
            coverage.entities_unavailable += 1;
        }
        if metadata
            .publisher_prior
            .as_ref()
            .and_then(|p| p.score)
            .is_some()
        {
            coverage.publishers_available += 1;
        }
        coverage
            .warnings
            .extend(metadata.diagnostics.iter().cloned());
        store.atlas_save_metadata(&metadata)?;
        record_unit(
            db_path,
            &store,
            &metadata,
            crate::atlas_work::STAGE_ENTITIES,
            &metadata.entity_state,
            metadata.entity_ms,
        )?;
        record_unit(
            db_path,
            &store,
            &metadata,
            crate::atlas_work::STAGE_PUBLISHERS,
            &metadata.reliability_state,
            metadata.publisher_ms,
        )?;
        if let Some(job) = &entity_job {
            job.phase("Entities", Some(index_in_run as i64 + 1), Some(total));
        }
        if let Some(job) = &publisher_job {
            job.phase(
                "Publisher priors",
                Some(index_in_run as i64 + 1),
                Some(total),
            );
        }
    }

    let cancelled = cancel.load(std::sync::atomic::Ordering::Acquire);
    if let Some(job) = entity_job {
        job.finish(if cancelled {
            crate::job_registry::Finish::Paused {
                summary: "Native metadata paused".into(),
            }
        } else if coverage.entities_unavailable > 0 {
            crate::job_registry::Finish::Partial {
                summary: format!(
                    "{} article entity inventories unavailable",
                    coverage.entities_unavailable
                ),
            }
        } else {
            crate::job_registry::Finish::completed()
        });
    }
    if let Some(job) = publisher_job {
        job.finish(if cancelled {
            crate::job_registry::Finish::Paused {
                summary: "Native publisher lookups paused".into(),
            }
        } else {
            crate::job_registry::Finish::completed()
        });
    }
    Ok(coverage)
}
#[cfg(not(test))]
async fn wait_cancelled(cancel: Arc<AtomicBool>) {
    while !cancel.load(std::sync::atomic::Ordering::Acquire) {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

fn metadata_job(
    db: &std::path::Path,
    run: &str,
    operation: &str,
    title: &str,
    cancel: Arc<AtomicBool>,
) -> Option<crate::job_registry::JobHandle> {
    let mut spec = crate::job_registry::JobSpec::new("atlas", operation, title)
        .run(run)
        .resource(format!("atlas:{run}"))
        .cancellable();
    spec.parent_id = rusqlite::Connection::open(db)
        .ok()
        .and_then(|c| {
            crate::job_registry::job_for_run(&c, "atlas_cycle", run)
                .ok()
                .flatten()
        })
        .unwrap_or_default();
    crate::job_registry::begin_optional_with(db, spec, cancel)
}
fn record_unit(
    db: &std::path::Path,
    store: &Store,
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
    store.atlas_save_unit_manifest(&manifest)?;
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
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let Some(article) = self.atlas_article(run_id, article_id)? else {
            return Ok(false);
        };
        if input_hash(&article) != captured_hash {
            return Ok(false);
        }
        let mut metadata = self
            .atlas_metadata(run_id, article_id)?
            .unwrap_or_else(|| ArticleMetadata::pending(&article));
        metadata.classification_status = status.into();
        metadata.classification_receipt = if receipt_json.is_empty() {
            None
        } else {
            Some(serde_json::from_str(receipt_json)?)
        };
        if let Some(category) = category {
            tx.execute(
                "UPDATE atlas_articles SET category=?3 WHERE run_id=?1 AND article_id=?2",
                params![run_id, article_id, category],
            )?;
        }
        tx.execute("INSERT OR REPLACE INTO atlas_article_metadata(run_id,article_id,input_hash,entity_state,reliability_state,classification_status,metadata_json) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![run_id,article_id,captured_hash,metadata.entity_state.as_str(),metadata.reliability_state.as_str(),status,serde_json::to_string(&metadata)?])?;
        tx.commit()?;
        Ok(true)
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
    #[test]
    fn calendar_months_are_not_actors_or_related_entity_overlap() {
        let store = Store::memory().unwrap();
        store.atlas_insert_run("calendar", "{}", "{}").unwrap();
        for (id, title) in [
            ("a", "Jane Doe announced a project in August"),
            ("b", "Jane Doe reviewed a project in August"),
        ] {
            let article = AtlasArticleRow {
                run_id: "calendar".into(),
                id: id.into(),
                title: title.into(),
                url: format!("https://news.test/{id}"),
                category: "economic".into(),
                ..Default::default()
            };
            store.atlas_upsert_article(&article).unwrap();
            let mut metadata = ArticleMetadata::pending(&article);
            metadata.entity_state = MetadataState::Complete;
            for name in ["Jane Doe", "August"] {
                let start = title.find(name).unwrap();
                metadata.mentions.push(ArticleMention {
                    type_hint: "person".into(),
                    comparison_key: name.to_lowercase(),
                    field: "title".into(),
                    start_byte: start,
                    end_byte: start + name.len(),
                    original_text: name.into(),
                });
            }
            assert_eq!(
                article_actors(&article, &metadata)
                    .iter()
                    .map(|a| a.display_name.as_str())
                    .collect::<Vec<_>>(),
                vec!["Jane Doe"]
            );
            store.atlas_save_metadata(&metadata).unwrap();
        }
        assert_eq!(
            crate::related_articles::build(&store, "calendar").unwrap(),
            0,
            "a shared month cannot supply the second entity"
        );
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
    #[test]
    fn classification_receipts_preserve_native_metadata_and_reject_stale_revision() {
        let store = Store::memory().unwrap();
        store.atlas_insert_run("r", "{}", "{}").unwrap();
        let mut a = article();
        store.atlas_upsert_article(&a).unwrap();
        let mut m = ArticleMetadata::pending(&a);
        m.entity_state = MetadataState::Complete;
        m.mentions.push(ArticleMention {
            type_hint: "person".into(),
            comparison_key: "jane doe".into(),
            field: "title".into(),
            start_byte: 0,
            end_byte: 8,
            original_text: "Jane Doe".into(),
        });
        store.atlas_save_metadata(&m).unwrap();
        assert!(store
            .atlas_set_classification(
                "r",
                "a",
                &m.input_hash,
                "classified",
                r#"{"category":"unk"}"#,
                Some("unk")
            )
            .unwrap());
        let saved = store.atlas_metadata("r", "a").unwrap().unwrap();
        assert_eq!(saved.entity_state, MetadataState::Complete);
        assert_eq!(saved.mentions.len(), 1);
        assert_eq!(saved.classification_status, "classified");
        a.description = "Revised description".into();
        store.atlas_upsert_article(&a).unwrap();
        assert!(!store
            .atlas_set_classification(
                "r",
                "a",
                &m.input_hash,
                "classified",
                "{}",
                Some("economic")
            )
            .unwrap());
        assert!(store.atlas_metadata("r", "a").unwrap().is_none());
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

    #[test]
    fn concurrent_native_metadata_and_classifier_receipts_preserve_both_results() {
        let directory = tempfile::tempdir().unwrap();
        let db = directory.path().join("concurrent.db");
        let store = Store::open(&db).unwrap();
        store.atlas_insert_run("r", "{}", "{}").unwrap();
        let article = article();
        store.atlas_upsert_article(&article).unwrap();
        let mut metadata = ArticleMetadata::pending(&article);
        metadata.entity_state = MetadataState::Complete;
        let ready = Arc::new(std::sync::Barrier::new(3));
        let workers: Vec<_> = [false, true]
            .into_iter()
            .map(|classification| {
                let db = db.clone();
                let ready = ready.clone();
                let mut metadata = metadata.clone();
                std::thread::spawn(move || {
                    let store = Store::open(&db).unwrap();
                    ready.wait();
                    for iteration in 0..50 {
                        if classification {
                            assert!(store
                                .atlas_set_classification(
                                    "r",
                                    "a",
                                    &metadata.input_hash,
                                    "classified",
                                    &serde_json::json!({"iteration":iteration}).to_string(),
                                    Some("military")
                                )
                                .unwrap());
                        } else {
                            metadata.diagnostics = vec![format!("iteration {iteration}")];
                            assert!(store.atlas_save_metadata(&metadata).unwrap());
                        }
                    }
                })
            })
            .collect();
        ready.wait();
        for worker in workers {
            worker.join().unwrap();
        }
        let saved = store.atlas_metadata("r", "a").unwrap().unwrap();
        assert_eq!(saved.entity_state, MetadataState::Complete);
        assert_eq!(saved.classification_status, "classified");
        assert_eq!(saved.classification_receipt.unwrap()["iteration"], 49);
        assert_eq!(saved.diagnostics, vec!["iteration 49"]);
        assert_eq!(
            store.atlas_article("r", "a").unwrap().unwrap().category,
            "military"
        );
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
        if let Ok(store) = Store::open(db_path) {
            let _ = store.conn.execute(
                "DELETE FROM app_state WHERE key=?1",
                [format!("atlas-publisher-failure:{run_id}")],
            );
        }
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
    db: &std::path::Path,
    user_agent: &str,
    enabled: bool,
) -> Option<rsp::RspIndex> {
    if !enabled {
        return None;
    }
    let stored = Store::open(db)
        .ok()
        .and_then(|s| s.app_state_get(rsp::APP_STATE_KEY).ok().flatten());
    #[cfg(test)]
    {
        let _ = user_agent;
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
            Some(db.to_path_buf()),
        )
        .await
        {
            Ok((index, changed)) => {
                if changed {
                    if let Ok(store) = Store::open(db) {
                        if let Ok(json) = rsp::index_to_json(&index) {
                            let _ = store.app_state_set(rsp::APP_STATE_KEY, &json);
                        }
                    }
                }
                Some(index)
            }
            Err(_) => None,
        }
    }
}
fn flush_inventory_publishers(
    db: &std::path::Path,
    run: &str,
    pending: &mut Vec<(String, String)>,
    index: Option<&rsp::RspIndex>,
    enabled: bool,
) -> Result<()> {
    let store = Store::open(db)?;
    for (id, hash) in pending.drain(..) {
        let Some(article) = store.atlas_article(run, &id)? else {
            continue;
        };
        if input_hash(&article) != hash {
            continue;
        }
        let Some(mut metadata) = store.atlas_metadata(run, &id)? else {
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
        if store.atlas_save_metadata(&metadata)? {
            record_unit(
                db,
                &store,
                &metadata,
                crate::atlas_work::STAGE_PUBLISHERS,
                &metadata.reliability_state,
                metadata.publisher_ms,
            )?;
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
    let Some(first) = receiver.recv().await else {
        return Ok(());
    };
    let store = Store::open(&db)?;
    let enabled = store.tool_enabled(rsp::TOOL_ID)?;
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
    let publisher = inventory_index(&db, &user_agent, enabled);
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
            let native = collect_inner(&article, None, cancel.clone(), Some(&db));
            tokio::pin!(native);
            let mut metadata = loop {
                tokio::select! {
                 metadata=&mut native=>break metadata,
                 result=&mut publisher,if index.is_none()=>{
                  index=Some(result);flush_inventory_publishers(&db,&run,&mut pending,index.as_ref().and_then(|i|i.as_ref()),enabled)?;
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
            if store.atlas_save_metadata(&metadata)? {
                record_unit(
                    &db,
                    &store,
                    &metadata,
                    crate::atlas_work::STAGE_ENTITIES,
                    &metadata.entity_state,
                    metadata.entity_ms,
                )?;
                if index.is_some() {
                    record_unit(
                        &db,
                        &store,
                        &metadata,
                        crate::atlas_work::STAGE_PUBLISHERS,
                        &metadata.reliability_state,
                        metadata.publisher_ms,
                    )?;
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
         result=&mut publisher,if index.is_none()=>{index=Some(result);flush_inventory_publishers(&db,&run,&mut pending,index.as_ref().and_then(|i|i.as_ref()),enabled)?;}
        }
    }
    if !cancel.load(std::sync::atomic::Ordering::Acquire) {
        if index.is_none() {
            index = Some(publisher.await);
        }
        flush_inventory_publishers(
            &db,
            &run,
            &mut pending,
            index.as_ref().and_then(|i| i.as_ref()),
            enabled,
        )?;
    }
    if enabled && !cancel.load(std::sync::atomic::Ordering::Acquire) {
        let store = Store::open(&db)?;
        let key = format!("atlas-publisher-failure:{run}");
        if index.as_ref().is_none_or(|i| i.is_none()) {
            store.app_state_set(&key, "1")?;
        } else {
            store
                .conn
                .execute("DELETE FROM app_state WHERE key=?1", [key])?;
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
    use super::*;
    #[tokio::test]
    async fn collection_inventory_drains_every_admitted_revision_without_brain_writes() {
        let directory = tempfile::tempdir().unwrap();
        let db = directory.path().join("inventory.db");
        let store = Store::open(&db).unwrap();
        store.atlas_insert_run("r", "{}", "{}").unwrap();
        let index = rsp::RspIndex::build(vec![], chrono::Utc::now().to_rfc3339());
        store
            .app_state_set(rsp::APP_STATE_KEY, &rsp::index_to_json(&index).unwrap())
            .unwrap();
        let inventory = NativeInventory::start(&db, "r", "Argos fixture");
        for i in 0..40 {
            let article = AtlasArticleRow {
                run_id: "r".into(),
                id: format!("a{i}"),
                title: format!("Jane Doe reports story {i}"),
                description: String::new(),
                url: format!("https://unlisted.example/{i}"),
                country: String::new(),
                source_name: "Unlisted".into(),
                source_domain: "unlisted.example".into(),
                published_at: "2026-10-12".into(),
                provider: "fixture".into(),
                temperature: 0.0,
                category: "unk".into(),
                seen_at: String::new(),
                author: String::new(),
                image_url: String::new(),
            };
            store.atlas_upsert_article(&article).unwrap();
            if let Err(error) = inventory.queue(&article).await {
                let worker = inventory.finish().await;
                panic!("Inventory admission: {error}; worker: {worker:?}");
            }
        }
        inventory.finish().await.unwrap();
        let count:i64=store.conn.query_row("SELECT COUNT(*) FROM atlas_article_metadata WHERE run_id='r' AND reliability_state='complete'",[],|r|r.get(0)).unwrap();
        assert_eq!(count, 40);
        let memories: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
            .unwrap();
        assert_eq!(memories, 0);
    }
    #[tokio::test]
    async fn empty_inventory_starts_no_native_or_model_work() {
        let directory = tempfile::tempdir().unwrap();
        let db = directory.path().join("empty.db");
        let store = Store::open(&db).unwrap();
        store.atlas_insert_run("r", "{}", "{}").unwrap();
        NativeInventory::start(&db, "r", "Argos fixture")
            .finish()
            .await
            .unwrap();
        let jobs: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM argos_jobs WHERE run_ref='r'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(jobs, 0);
    }
}
