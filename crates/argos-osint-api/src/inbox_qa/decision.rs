//! Decision scoring boundary for authorized, bounded activity candidates.

use std::future::Future;
use std::pin::Pin;

use fleetworks_contracts::chat::QaDecisionAvailabilityReason;
use fleetworks_contracts::chat::QaDecisionPreference;
use fleetworks_contracts::chat::QaDecisionSettings;
use fleetworks_contracts::chat::QaPhase;

/// Only fixed domain descriptions cross the decision worker boundary. The
/// caller keeps the mapping from menu positions to authorized job references.
pub struct DecisionMenu {
    pub state: String,
    pub candidates: Vec<String>,
}

/// A verified selection or the scorer's explicit abstention.
pub enum DecisionChoice {
    Candidate(usize),
    Abstain,
    Unavailable,
}

pub type DecisionFuture<'a> = Pin<Box<dyn Future<Output = DecisionChoice> + Send + 'a>>;

/// The Embedded host controls qualification, worker lifetime and resource admission.
pub trait DecisionRuntime: Send + Sync {
    fn status(&self, preference: QaDecisionPreference, generation: i64) -> QaDecisionSettings;

    fn score(
        &self,
        preference: QaDecisionPreference,
        generation: i64,
        menu: DecisionMenu,
    ) -> DecisionFuture<'_>;
}

/// Rules stay in effect until an eligible worker reports readiness.
#[must_use]
pub fn rules_status(
    preference: QaDecisionPreference,
    reason: Option<QaDecisionAvailabilityReason>,
) -> QaDecisionSettings {
    let phase = if preference == QaDecisionPreference::RulesOnly {
        QaPhase::Ready
    } else {
        QaPhase::Unavailable
    };
    QaDecisionSettings::new(preference, QaDecisionPreference::RulesOnly, phase, reason)
}
