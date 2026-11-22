//! Bounded safe Jobs result metadata, with no worker paths or raw reports.

use rune::El;
use rune_kit::Kit;
use serde::Deserialize;

use crate::views::heading;
use crate::views::quiet;

#[derive(Clone, Debug, Deserialize)]
pub struct MaintenanceResult {
    pub operation: String,
    pub engine: Option<String>,
    pub atlas_engine: Option<String>,
    pub candidate_model_id: Option<String>,
    pub active_model_id: Option<String>,
    pub candidate_sha256: Option<String>,
    pub report_sha256: Option<String>,
    pub dataset_sha256: Option<String>,
    pub state: Option<String>,
    pub actor_search_state: Option<String>,
    pub generation: Option<String>,
    pub index_generation: Option<String>,
    pub original_job_id: Option<String>,
    pub processed: Option<u64>,
    pub scanned: Option<u64>,
    pub total: Option<u64>,
    pub aliases: Option<u64>,
    pub visible_checkpoint: Option<u64>,
    pub activation_generation: Option<u64>,
    pub activation_requested: Option<bool>,
    pub database_seeded: Option<bool>,
    pub reader_visible: Option<bool>,
}

impl MaintenanceResult {
    /// Render only recognized safe metadata. Omitted values remain unknown;
    /// training completion does not imply model activation or search visibility.
    pub(crate) fn view(&self, kit: &Kit) -> El {
        let title = match self.operation.as_str() {
            "actor_seed" => "Actor seed result",
            "classifier_train" => "Historical Natural training result",
            "actor_reindex" => "Actor search index result",
            "classifier_activate" => "Historical Natural activation result",
            _ => return rune::col(),
        };
        let mut body = rune::col()
            .gap(5.0)
            .key("maintenance-job-result")
            .child(heading(title, kit));
        let generation = self.generation.as_ref().or(self.index_generation.as_ref());
        for (label, value) in [
            ("State", self.state.as_ref()),
            ("Original job", self.original_job_id.as_ref()),
            ("Training engine", self.engine.as_ref()),
            ("Atlas engine", self.atlas_engine.as_ref()),
            ("Candidate model", self.candidate_model_id.as_ref()),
            ("Active model", self.active_model_id.as_ref()),
            ("Dataset SHA-256", self.dataset_sha256.as_ref()),
            ("Candidate SHA-256", self.candidate_sha256.as_ref()),
            ("Report SHA-256", self.report_sha256.as_ref()),
            ("Actor search", self.actor_search_state.as_ref()),
            ("Index generation", generation),
        ] {
            if let Some(value) = value.map(String::as_str).filter(|value| {
                !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
            }) {
                body = body.child(quiet(format!("{label}: {value}"), kit));
            }
        }
        if let Some(processed) = self.processed {
            body = body.child(quiet(
                self.total.map_or_else(
                    || format!("Processed: {processed}"),
                    |total| format!("Processed: {processed} / {total}"),
                ),
                kit,
            ));
        }
        if let Some(scanned) = self.scanned {
            body = body.child(quiet(
                self.total.map_or_else(
                    || format!("Scanned actors: {scanned}"),
                    |total| format!("Scanned actors: {scanned} / {total}"),
                ),
                kit,
            ));
        }
        for (label, value) in [
            ("Aliases", self.aliases),
            ("Visible checkpoint", self.visible_checkpoint),
            ("Activation generation", self.activation_generation),
        ] {
            if let Some(value) = value {
                body = body.child(quiet(format!("{label}: {value}"), kit));
            }
        }
        if let Some(reader_visible) = self.reader_visible {
            body = body.child(quiet(
                format!(
                    "Reader visible: {}",
                    if reader_visible { "yes" } else { "no" }
                ),
                kit,
            ));
        }
        if self.operation == "classifier_train" && self.active_model_id.is_none() {
            body = body.child(quiet(
                "This historical result records Natural candidate creation.",
                kit,
            ));
        }
        body
    }
}
