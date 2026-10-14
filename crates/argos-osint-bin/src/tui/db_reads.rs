//! Immutable database projections for background TUI reads.
use std::collections::{HashMap, HashSet};

use anyhow::Result;
use argos_osint_core::{
    atlas_metadata, intel_recon, investigation,
    related_global::RelatedScope,
    store::{AtlasArticleClaim, AtlasArticleRow, Store},
};

#[derive(Debug)]
pub(super) struct ReconSnapshot {
    pub messages: Vec<argos_osint_core::recon::Message>,
    pub events: Vec<investigation::InvestigationEvent>,
    pub calls: Vec<argos_osint_core::recon::Call>,
    pub runs: Vec<argos_osint_core::recon::Run>,
    pub memories: HashMap<String, Vec<argos_osint_core::brain::Memory>>,
    pub memory_summaries: HashMap<String, String>,
    pub logged_calls: Vec<String>,
    pub log_error: Option<String>,
}

impl ReconSnapshot {
    pub fn read(store: &Store, id: &str) -> Result<Self> {
        let memories = store.answer_memories(id)?;
        let ids: HashSet<_> = memories.values().flatten().map(|m| m.id.as_str()).collect();
        let mut memory_summaries = HashMap::new();
        for memory_id in ids {
            if let Some(insight) = store.insight_for_memory(memory_id).ok().flatten() {
                memory_summaries.insert(memory_id.to_string(), format_insight_summary(&insight));
            }
        }
        Ok(Self {
            messages: store.list_messages(id)?,
            events: store.list_investigation_events(id)?,
            calls: store.all_calls_for_thread(id)?,
            runs: store.runs_for_thread(id)?,
            memories,
            memory_summaries,
            logged_calls: Vec::new(),
            log_error: None,
        })
    }

    /// Format and record historical tool results on the reader worker, after
    /// its read transaction ends. Large results never clone/format on the UI.
    pub fn record_call_logs(&mut self, store: &Store, logged: &HashSet<String>) {
        for call in &self.calls {
            if call.result.is_none() || logged.contains(&call.id) {
                continue;
            }
            if let Some(entry) = super::ui::tool_result_log(call) {
                let event = super::logs::session_event(entry.level, &entry.summary, &entry.detail);
                if let Err(error) = store.record_event(&event) {
                    self.log_error = Some(error.to_string());
                    break;
                }
            }
            self.logged_calls.push(call.id.clone());
        }
    }
}

pub(super) fn format_insight_summary(insight: &argos_osint_core::recon::InsightView) -> String {
    let mut lines = vec![insight.presentation()];
    lines.push(format!(
        "{} evidence link{}",
        insight.sources.len(),
        if insight.sources.len() == 1 { "" } else { "s" }
    ));
    if !insight.related.is_empty() {
        lines.push(format!("Related: {}", insight.related.join(", ")));
    }
    lines.join("\n")
}

#[derive(Debug, Default)]
pub(super) struct IntelBriefSnapshot {
    pub actors: Vec<atlas_metadata::ArticleActor>,
    pub actor_state: String,
    pub metadata_present: bool,
    pub publisher_prior: Option<atlas_metadata::PublisherPrior>,
    pub progress: Option<investigation::insight_execution::Progress>,
    pub lifecycle: String,
    pub status: String,
    pub confidence: Option<intel_recon::EvidenceConfidence>,
    pub related: super::related_articles::ArticleRelatedView,
    pub claims: Vec<AtlasArticleClaim>,
    pub relations: Vec<(String, String, String)>,
    pub explanations: HashMap<(String, String), String>,
    pub body: Option<intel_recon::ArticleBodyRow>,
    /// A forced generation must not display the previous retrieval's legacy row.
    pub body_requires_commit: bool,
    pub stages: Vec<intel_recon::brief_pipeline::StageView>,
    pub outputs: HashMap<String, serde_json::Value>,
    pub lead_titles: HashMap<String, String>,
    pub publication: String,
}

impl IntelBriefSnapshot {
    pub fn read(
        store: &Store,
        article: &AtlasArticleRow,
        scope: RelatedScope,
        job: Option<&str>,
        related_limit: usize,
    ) -> Result<Self> {
        let conn = store.connection();
        let metadata = store.atlas_metadata(&article.run_id, &article.id)?;
        let mut out = Self {
            actors: metadata
                .as_ref()
                .map(|m| atlas_metadata::article_actors(article, m))
                .unwrap_or_default(),
            actor_state: metadata
                .as_ref()
                .map(|m| m.entity_state.as_str())
                .unwrap_or("pending")
                .into(),
            metadata_present: metadata.is_some(),
            publisher_prior: metadata.and_then(|m| m.publisher_prior),
            related: super::related_articles::ArticleRelatedView {
                page: store
                    .related_articles_global(&article.run_id, &article.id, scope, None, 20)?
                    .into_page(0),
                ..Default::default()
            },
            progress: investigation::insight_execution::owner_progress(
                conn,
                "article",
                &article.id,
            )?,
            status: investigation::insight_persist::article_refresh_status(conn, &article.id)?,
            confidence: intel_recon::article_confidence(conn, &article.id)?,
            claims: store.atlas_claims_for_article(&article.run_id, &article.id)?,
            explanations: store.get_intel_link_explanations(&article.id)?,
            body: store.article_body_for_article(&article.id)?,
            ..Default::default()
        };
        while out.related.page.rows.len() < related_limit && out.related.page.has_more {
            let offset = out.related.page.rows.len();
            let page = store
                .related_articles_global(
                    &article.run_id,
                    &article.id,
                    scope,
                    Some(&offset.to_string()),
                    (related_limit - offset).min(100),
                )?
                .into_page(offset);
            if page.rows.is_empty() {
                break;
            }
            out.related.page.rows.extend(page.rows);
            out.related.page.total = page.total;
            out.related.page.has_more = page.has_more;
        }
        out.lifecycle = conn.query_row("SELECT COALESCE((SELECT lifecycle FROM insight_owner_state WHERE owner_kind IN ('article','atlas') AND owner_id=?1 ORDER BY CASE owner_kind WHEN 'article' THEN 0 ELSE 1 END LIMIT 1),'legacy/unassessed')", [&article.id], |r| r.get(0))?;
        if let Some(output) =
            investigation::insight_persist::active_output(conn, "article", &article.id)?
        {
            for entity in output.entities {
                let name = entity.original_forms.first().cloned().unwrap_or_default();
                if name.trim().is_empty()
                    || out.actors.iter().any(|a| {
                        a.type_hint == entity.type_hint
                            && a.display_name.eq_ignore_ascii_case(&name)
                    })
                {
                    continue;
                }
                out.actors.push(atlas_metadata::ArticleActor {
                    id: entity.id,
                    type_hint: entity.type_hint,
                    display_name: name,
                    spellings: entity.original_forms,
                    mentions: Vec::new(),
                });
            }
        }
        let ids: Vec<_> = out.claims.iter().map(|c| c.fingerprint.clone()).collect();
        out.relations = store.insight_relations_among(&ids)?;
        for (left, right) in out.explanations.keys() {
            if !out
                .relations
                .iter()
                .any(|(l, r, _)| l == left && r == right)
            {
                out.relations
                    .push((left.clone(), right.clone(), "assessed".into()));
            }
        }
        if let Some(job) = job {
            let mut statement = conn.prepare("SELECT input_revision,created_at FROM intel_brief_runs WHERE job_id=?1 AND article_id=?2")?;
            let generation: Option<(String, String)> = statement
                .query_map([job, &article.id], |row| Ok((row.get(0)?, row.get(1)?)))?
                .next()
                .transpose()?;
            let Some((revision, created)) = generation else {
                // Navigation/retention can retire a job while its read is queued.
                // Never attach another article's stage artifacts to this snapshot.
                out.publication = "Brief job unavailable for this article · Reload".into();
                return Ok(out);
            };
            out.body_requires_commit = {
                revision.contains(":reload:")
                    && !out.body.as_ref().is_some_and(|body| {
                        body.state == "ready"
                            && chrono::DateTime::parse_from_rfc3339(&body.updated_at)
                                .ok()
                                .zip(chrono::DateTime::parse_from_rfc3339(&created).ok())
                                .is_some_and(|(updated, created)| updated >= created)
                    })
            };
            out.stages = intel_recon::brief_pipeline::stage_views(conn, job)?;
            out.outputs = intel_recon::brief_pipeline::stage_outputs(conn, job)?;
            if let Some(leads) = out
                .outputs
                .get("intel_brief_links")
                .and_then(|v| v["leads"].as_array())
            {
                for lead in leads {
                    if let (Some(id), Some(run)) =
                        (lead["article_id"].as_str(), lead["article_run_id"].as_str())
                    {
                        if let Some(a) = store.atlas_article(run, id)? {
                            out.lead_titles.insert(id.into(), a.title);
                        }
                    }
                }
            }
            out.publication = intel_recon::brief_canonical::footer(
                conn,
                &article.id,
                job,
                out.stages.iter().any(|s| s.state == "failed"),
            )?;
        }
        Ok(out)
    }
}
