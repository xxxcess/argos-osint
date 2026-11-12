//! Bounded actor discovery and authoritative selected-dossier state.

use fleetworks_contracts::search::ActorSearchResponse;
use rune::TextEdit;
use serde::Deserialize;

use crate::app::Load;

/// An ordinary dossier record from the gateway, never an index document or graph view.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ActorDossier {
    pub id: String,
    pub display_name: String,
    pub subtype: String,
    pub identity_state: String,
    pub description: Option<String>,
    pub aliases: Vec<String>,
    pub metadata: Vec<ProfileField>,
    pub claims: Vec<Association>,
    pub briefs: Vec<Association>,
    pub inferences: Vec<Association>,
    pub gaps: Vec<String>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ProfileField {
    pub key: String,
    pub value: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Association {
    pub node: AssociationLabel,
    pub relationship: AssociationLabel,
    pub recorded_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AssociationLabel {
    pub id: String,
    pub label: String,
}

/// One outstanding search plus the latest pending query, fenced by query revision.
pub struct Brain {
    pub query: TextEdit,
    pub results: Load<ActorSearchResponse>,
    pub highlighted: Option<usize>,
    pub selected_actor: Option<String>,
    pub dossier: Load<ActorDossier>,
    pub showing_detail: bool,
    pub(crate) query_revision: u64,
    pub(crate) detail_revision: u64,
    pub(crate) pending_at: Option<f64>,
    pub(crate) in_flight: bool,
}

impl Default for Brain {
    fn default() -> Self {
        Self {
            query: TextEdit::new(""),
            results: Load::Idle,
            highlighted: None,
            selected_actor: None,
            dossier: Load::Idle,
            showing_detail: false,
            query_revision: 0,
            detail_revision: 0,
            pending_at: None,
            in_flight: false,
        }
    }
}

impl Brain {
    pub fn query_revision(&self) -> u64 {
        self.query_revision
    }

    /// Coalesce edits without a network request for empty input.
    pub fn edited(&mut self, now: f64) {
        self.query_revision = self.query_revision.wrapping_add(1);
        self.highlighted = None;
        self.showing_detail = false;
        self.detail_revision = self.detail_revision.wrapping_add(1);
        self.selected_actor = None;
        self.dossier = Load::Idle;
        self.pending_at = (!self.query.text.trim().is_empty()).then_some(now + 0.2);
        self.results = if self.pending_at.is_some() {
            Load::Pending
        } else {
            Load::Idle
        };
    }

    pub fn move_highlight(&mut self, forward: bool) {
        let Load::Ready(response) = &self.results else {
            return;
        };
        if response.rows.is_empty() {
            self.highlighted = None;
            return;
        }
        let last = response.rows.len() - 1;
        self.highlighted = Some(match (self.highlighted, forward) {
            (None, true) => 0,
            (None, false) => last,
            (Some(index), true) => (index + 1).min(last),
            (Some(index), false) => index.saturating_sub(1),
        });
    }

    /// Reject a reply superseded by another edit in this same deployment context.
    pub fn complete_search(&mut self, revision: u64, result: Result<ActorSearchResponse, String>) {
        self.in_flight = false;
        if revision != self.query_revision {
            return;
        }
        self.results = match result {
            Ok(response) => Load::Ready(response),
            Err(error) => Load::Failed(error),
        };
        self.highlighted = None;
    }

    pub fn complete_dossier(&mut self, revision: u64, result: Result<ActorDossier, String>) {
        if revision != self.detail_revision {
            return;
        }
        self.dossier = match result {
            Ok(dossier) => Load::Ready(dossier),
            Err(error) => Load::Failed(error),
        };
    }
}
