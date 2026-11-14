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
    pub index_generation: Option<String>,
    pub processed: Option<u64>,
    pub total: Option<u64>,
    pub aliases: Option<u64>,
    pub visible_checkpoint: Option<u64>,
    pub activation_generation: Option<u64>,
    pub activation_requested: Option<bool>,
    pub database_seeded: Option<bool>,
}

impl MaintenanceResult {
    /// Render only recognized safe metadata. Omitted values remain unknown;
    /// training completion does not imply model activation or search visibility.
    pub(crate) fn view(&self, kit: &Kit) -> El {
        let title = match self.operation.as_str() {
            "actor_seed" => "Actor seed result",
            "classifier_train" => "Classifier training result",
            "actor_reindex" => "Actor search index result",
            "classifier_activate" => "Classifier activation result",
            _ => return rune::col(),
        };
        let mut body = rune::col()
            .gap(5.0)
            .key("maintenance-job-result")
            .child(heading(title, kit));
        for (label, value) in [
            ("State", &self.state),
            ("Training engine", &self.engine),
            ("Atlas engine", &self.atlas_engine),
            ("Candidate model", &self.candidate_model_id),
            ("Active model", &self.active_model_id),
            ("Dataset SHA-256", &self.dataset_sha256),
            ("Candidate SHA-256", &self.candidate_sha256),
            ("Report SHA-256", &self.report_sha256),
            ("Actor search", &self.actor_search_state),
            ("Index generation", &self.index_generation),
        ] {
            if let Some(value) = value.as_deref().filter(|value| {
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
        for (label, value) in [
            ("Aliases", self.aliases),
            ("Visible checkpoint", self.visible_checkpoint),
            ("Activation generation", self.activation_generation),
        ] {
            if let Some(value) = value {
                body = body.child(quiet(format!("{label}: {value}"), kit));
            }
        }
        if self.operation == "classifier_train" && self.active_model_id.is_none() {
            body = body.child(quiet(
                "Training produced a candidate. Model activation is a separate confirmed action.",
                kit,
            ));
        }
        body
    }
}
