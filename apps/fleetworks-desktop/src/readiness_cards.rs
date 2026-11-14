//! Read-only check cards distinguish an observed state from its freshness.

use fleetworks_contracts::readiness::ReadinessCheck;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use rune::El;
use rune_kit::Kit;

use crate::accessibility::ViewPreferences;
use crate::app::Load;
use crate::app::State;
use crate::icons;
use crate::material;
use crate::theme::StatusTone;
use crate::ui_components;
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

pub(crate) fn checks(state: &State, preferences: ViewPreferences, kit: &Kit) -> El {
    let mut body = rune::col().key("backend-readiness-checks").gap(8.0);
    match &state.readiness {
        Load::Idle => {
            return body.child(ui_components::async_status_strip(
                "readiness-idle",
                "Readiness not observed",
                "Refresh checks to inspect current backend health.",
                false,
                StatusTone::Neutral,
                preferences,
                kit,
            ));
        }
        Load::Pending | Load::Loading => {
            return body.child(ui_components::skeleton_readiness_list(preferences, kit));
        }
        Load::Failed(error) => {
            return body.child(
                ui_components::async_status_strip(
                    "readiness-failed",
                    "Readiness check failed",
                    error.clone(),
                    false,
                    StatusTone::Err,
                    preferences,
                    kit,
                )
                .role("alert"),
            );
        }
        Load::Ready(readiness) => {
            body = body.child(
                rune::row()
                    .w_full()
                    .center_y()
                    .gap(8.0)
                    .child(ui_components::status_pill(
                        format!("{} checks", readiness.checks.len()),
                        Some(&icons::SHIELD_CHECK),
                        StatusTone::Info,
                        kit,
                    ))
                    .child(quiet(format!("Observed {}", readiness.observed_at), kit)),
            );
            if readiness.checks.is_empty() {
                body = body.child(quiet(
                    "The backend returned no check observations. Readiness is unknown.",
                    kit,
                ));
            }
            for check in &readiness.checks {
                let fresh = state.readiness_check_fresh(check);
                let tone = tone_for_check(check.state, fresh);
                let status = label(check.state);
                let status = if fresh {
                    status.to_owned()
                } else {
                    format!("{status} · stale observation")
                };
                let check_id = check.id.as_str().to_owned();
                let expanded = state.selected_readiness.as_deref() == Some(check_id.as_str());
                let summary_title = format!("{}: {status}", title(check.id));
                let mut card_content = rune::col().w_full().padding(10.0).gap(6.0).child(
                    rune::row()
                        .w_full()
                        .center_y()
                        .gap(8.0)
                        .child(ui_components::status_dot(tone, preferences))
                        .child(heading(summary_title, kit))
                        .child(rune::spacer())
                        .child(ui_components::status_pill(
                            check.reason_code.clone(),
                            None,
                            tone,
                            kit,
                        )),
                );
                if expanded || !fresh {
                    card_content = card_content
                        .child(line(check.explanation.clone(), kit))
                        .child(quiet(
                            format!("Observed {} · {}", check.observed_at, check.reason_code),
                            kit,
                        ));
                } else {
                    card_content = card_content.child(line(check.explanation.clone(), kit));
                }
                if !fresh {
                    card_content = card_content.child(quiet(
                        "Refresh checks before relying on this status or starting a maintenance operation.",
                        kit,
                    ));
                }
                let card = material::interactive_tile(
                    card_content,
                    expanded,
                    tone,
                    preferences,
                    (!preferences.matte()).then_some(&state.pointer_offset),
                    (!preferences.matte()).then_some(&state.shadow_offset),
                )
                .key(format!("readiness-{}", check.id.as_str()))
                .on_press(move |state: &mut State| {
                    if state.selected_readiness.as_deref() == Some(check_id.as_str()) {
                        state.selected_readiness = None;
                    } else {
                        state.selected_readiness = Some(check_id.clone());
                    }
                });
                body = body.child(card);
            }
        }
    }
    body
}

fn tone_for_check(state: ReadinessState, fresh: bool) -> StatusTone {
    if !fresh {
        return StatusTone::Warn;
    }
    match state {
        ReadinessState::Ready => StatusTone::Ok,
        ReadinessState::Running => StatusTone::Info,
        ReadinessState::Degraded | ReadinessState::NotReady => StatusTone::Warn,
        ReadinessState::Unavailable => StatusTone::Err,
        _ => StatusTone::Neutral,
    }
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
