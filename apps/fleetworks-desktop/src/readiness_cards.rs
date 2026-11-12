//! Read-only check cards distinguish an observed state from its freshness.

use fleetworks_contracts::readiness::ReadinessCheck;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use rune::El;
use rune_kit::Kit;

use crate::app::Load;
use crate::app::State;
use crate::views::heading;
use crate::views::line;
use crate::views::quiet;

impl State {
    pub(crate) fn readiness_check_fresh(&self, check: &ReadinessCheck) -> bool {
        check.fresh_for_ms > 0
            && self.readiness_received_at.is_some_and(|received| {
                self.clock >= received
                    && (self.clock - received) * 1000.0 <= check.fresh_for_ms as f64
            })
    }
}

pub(crate) fn checks(state: &State, kit: &Kit) -> El {
    let mut body = rune::col().key("backend-readiness-checks");
    match &state.readiness {
        Load::Idle => {
            return body.child(quiet(
                "Checks have not been observed. Refresh to see current backend readiness.",
                kit,
            ));
        }
        Load::Pending | Load::Loading => {
            return body.child(quiet("Refreshing backend checks…", kit).role("status"));
        }
        Load::Failed(error) => return body.child(line(error.clone(), kit).role("alert")),
        Load::Ready(readiness) => {
            body = body.child(quiet(format!("Observed {}", readiness.observed_at), kit));
            if readiness.checks.is_empty() {
                body = body.child(quiet(
                    "The backend returned no check observations. Readiness is unknown.",
                    kit,
                ));
            }
            for check in &readiness.checks {
                let fresh = state.readiness_check_fresh(check);
                let status = label(check.state);
                let status = if fresh {
                    status.to_owned()
                } else {
                    format!("{status} · stale observation")
                };
                let mut card = rune::col()
                    .gap(5.0)
                    .py(8.0)
                    .key(format!("readiness-{}", check.id.as_str()))
                    .child(heading(format!("{}: {status}", title(check.id)), kit))
                    .child(line(check.explanation.clone(), kit))
                    .child(quiet(
                        format!("Observed {} · {}", check.observed_at, check.reason_code),
                        kit,
                    ));
                if !fresh {
                    card = card.child(quiet("Refresh checks before relying on this status or starting a maintenance operation.", kit));
                }
                body = body.child(card);
            }
        }
    }
    body
}

fn label(state: ReadinessState) -> &'static str {
    match state {
        ReadinessState::Ready => "Ready",
        ReadinessState::NotReady => "Not ready",
        ReadinessState::Running => "Running",
        ReadinessState::Degraded => "Degraded",
        ReadinessState::Unknown => "Unknown",
        ReadinessState::Unavailable => "Unavailable",
        ReadinessState::NotApplicable => "Not applicable",
        _ => "Unknown",
    }
}

fn title(check: ReadinessCheckId) -> &'static str {
    match check {
        ReadinessCheckId::Gateway => "Gateway",
        ReadinessCheckId::Datastore => "Datastore",
        ReadinessCheckId::Jobs => "Jobs",
        ReadinessCheckId::Brain => "Brain",
        ReadinessCheckId::Control => "Backend control",
        ReadinessCheckId::Nlp => "Language processing",
        ReadinessCheckId::Classifier => "Classifier",
        ReadinessCheckId::Registration => "Registration",
        ReadinessCheckId::ActorCatalog => "Actor catalog",
        ReadinessCheckId::ActorSearch => "Actor search",
        ReadinessCheckId::ClassifierInference => "Classifier inference",
        ReadinessCheckId::ClassifierTraining => "Classifier training",
        _ => "Backend check",
    }
}
