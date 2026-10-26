//! Immutable database projections for background TUI reads.
use std::collections::HashMap;

use anyhow::Result;
use argos_osint_core::{
    atlas_metadata, intel_recon, investigation,
    related_global::RelatedScope,
    store::{AtlasArticleClaim, AtlasArticleRow},
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

/// Add the active output's entities not already listed as metadata actors.
fn merge_output_actors(
    actors: &mut Vec<atlas_metadata::ArticleActor>,
    output: investigation::insights::PipelineOutput,
) {
    atlas_metadata::merge_output_actors(actors, &output.entities);
}

#[cfg(feature = "surreal")]
impl IntelBriefSnapshot {
    /// The brief snapshot under a storage config: every part read from the
    /// repository. Brief-chain stages, outputs and the publication footer
    /// wait for the SurrealDB brief chain, so a job shows as unavailable.
    pub(crate) fn read_repository(
        article: &AtlasArticleRow,
        scope: RelatedScope,
        job: Option<&str>,
        related_limit: usize,
    ) -> Result<Self> {
        let parts = super::surreal_brain::brief_parts(article)?;
        let mut out = Self {
            actors: parts
                .metadata
                .as_ref()
                .map(|m| atlas_metadata::article_actors(article, m))
                .unwrap_or_default(),
            actor_state: parts
                .metadata
                .as_ref()
                .map(|m| m.entity_state.as_str())
                .unwrap_or("pending")
                .into(),
            metadata_present: parts.metadata.is_some(),
            publisher_prior: parts.metadata.and_then(|m| m.publisher_prior),
            lifecycle: parts.lifecycle,
            status: parts.status,
            claims: parts.claims,
            relations: parts.relations,
            explanations: parts.explanations,
            body: parts.body,
            ..Default::default()
        };
        let mut offset = 0;
        loop {
            let page = super::surreal_brain::related_global(
                &article.run_id,
                &article.id,
                scope,
                (offset > 0).then(|| offset.to_string()).as_deref(),
                (related_limit.max(20) - offset).clamp(1, 100),
            )?
            .into_page(offset);
            let more = page.has_more && !page.rows.is_empty();
            out.related.page.total = page.total;
            out.related.page.has_more = page.has_more;
            out.related.page.coverage = page.coverage;
            out.related.page.rows.extend(page.rows);
            offset = out.related.page.rows.len();
            if !more || offset >= related_limit {
                break;
            }
        }
        if let Some(output) = parts.output {
            merge_output_actors(&mut out.actors, output);
        }
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
            let (stages, outputs, titles) = super::surreal_intel::brief_chain(job)?;
            out.stages = stages;
            out.outputs = outputs;
            out.lead_titles = titles;
            // Canonical Brain publication is not on SurrealDB yet.
            out.publication = if out.stages.iter().any(|s| s.state == "failed") {
                "Brief stage failed · Retry".into()
            } else {
                "Brief kept in SurrealDB · Brain publication pending".into()
            };
        }
        Ok(out)
    }
}
