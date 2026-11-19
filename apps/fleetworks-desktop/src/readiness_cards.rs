//! Check cards distinguish observed state from freshness and offer admitted repairs.

use fleetworks_contracts::readiness::DatasetKind;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::ReadinessActionId;
use fleetworks_contracts::readiness::ReadinessCheck;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use rune::El;
use rune_kit::Collapsible;
use rune_kit::Kit;

use crate::accessibility::ViewPreferences;
use crate::app::Load;
use crate::app::State;
use crate::material::MetalFinish;
use crate::theme::StatusTone;
use crate::ui_components;
use crate::ui_components::ActionButton;
use crate::views::line;
use crate::views::quiet;
use crate::views::unsynced_glyph;

impl State {
    pub(crate) fn readiness_check_fresh(&self, check: &ReadinessCheck) -> bool {
        self.readiness_check_fresh_at(check, self.clock)
    }

    pub(crate) fn readiness_check_fresh_at(&self, check: &ReadinessCheck, now: f64) -> bool {
        check.fresh_for_ms > 0
            && self.readiness_received_at.is_some_and(|received| {
                now >= received && (now - received) * 1000.0 <= check.fresh_for_ms as f64
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
            let applicable = readiness
                .checks
                .iter()
                .filter(|check| check.state != ReadinessState::NotApplicable)
                .count();
            body = body.child(quiet(
                if applicable == 1 {
                    "1 applicable check".to_owned()
                } else {
                    format!("{applicable} applicable checks")
                },
                kit,
            ));
            if let Some(error) = &state.readiness_error {
                body = body.child(
                    rune::row()
                        .key("readiness-update-failed")
                        .center_y()
                        .gap(6.0)
                        .child(unsynced_glyph(
                            "readiness-refresh-unsynced",
                            format!("Readiness refresh failed: {error}"),
                            kit,
                        ))
                        .child(quiet("Showing last observed checks", kit))
                        .child(
                            ActionButton::new("Retry")
                                .key("readiness-retry")
                                .finish(MetalFinish::Gunmetal)
                                .on_press(|state: &mut State| state.refresh_readiness())
                                .build(kit, preferences, None),
                        ),
                );
            }
            if readiness.checks.is_empty() {
                body = body.child(quiet("No check observations", kit));
            }
            for check in &readiness.checks {
                let fresh = state.readiness_check_fresh(check);
                let check_id = check.id.as_str();
                let expanded = state.selected_readiness.as_deref() == Some(check_id);
                let mut disclosure = Collapsible::new(
                    format!("{}: {}", title(check.id), label(check.state)),
                    expanded,
                )
                .on_toggle(move |state: &mut State| {
                    state.selected_readiness =
                        if state.selected_readiness.as_deref() == Some(check_id) {
                            None
                        } else {
                            Some(check_id.to_owned())
                        };
                });
                if expanded {
                    let mut detail = rune::col()
                        .w_full()
                        .gap(4.0)
                        .child(line(check.explanation.clone(), kit))
                        .child(quiet(
                            format!("Observed {} · {}", check.observed_at, check.reason_code),
                            kit,
                        ));
                    for action in &check.actions {
                        if let Some((button, reason)) =
                            maintenance_button(state, preferences, kit, check.id, action.id)
                        {
                            detail = detail
                                .child(button)
                                .children(reason.map(|reason| quiet(reason, kit)));
                        }
                    }
                    disclosure = disclosure.child(detail);
                }
                let indicator =
                    (!fresh && check.state != ReadinessState::NotApplicable).then(|| {
                        unsynced_glyph(
                            "unsynced",
                            format!(
                                "{} is unsynced. Last observed {}.",
                                title(check.id),
                                check.observed_at
                            ),
                            kit,
                        )
                    });
                body = body.child(
                    rune::row()
                        .w_full()
                        .gap(6.0)
                        .key(format!("readiness-{check_id}"))
                        .child(disclosure.build(kit).grow(1.0))
                        .children(indicator),
                );
            }
        }
    }
    body
}

/// Present the next non-actor issue when the actor summary has no work to advance.
pub(crate) fn primary_issue(state: &State, preferences: ViewPreferences, kit: &Kit) -> Option<El> {
    let Load::Ready(readiness) = &state.readiness else {
        return None;
    };
    let check = readiness.checks.iter().find(|check| {
        !matches!(
            check.id,
            ReadinessCheckId::ActorCatalog
                | ReadinessCheckId::ActorSearch
                | ReadinessCheckId::Registration
        ) && matches!(
            check.state,
            ReadinessState::Running
                | ReadinessState::NotReady
                | ReadinessState::Degraded
                | ReadinessState::Unavailable
        ) && state.readiness_check_fresh(check)
    })?;
    let check_id = check.id.as_str();
    let button = check
        .actions
        .iter()
        .find_map(|action| {
            let (button, reason) =
                maintenance_button(state, preferences, kit, check.id, action.id)?;
            reason
                .is_none()
                .then_some(button.key("workstation-primary-action"))
        })
        .unwrap_or_else(|| {
            ActionButton::new("Inspect check")
                .key("workstation-inspect-check")
                .finish(MetalFinish::Gunmetal)
                .on_press_tx(move |state: &mut State, tx| {
                    state.selected_readiness = Some(check_id.to_owned());
                    tx.scroll_into_view(format!("readiness-{check_id}"));
                    tx.focus(format!("readiness-{check_id}/trigger"), tx.focus_visible());
                })
                .build(kit, preferences, None)
        });
    Some(
        rune::row()
            .key("workstation-primary-check")
            .flex_wrap()
            .center_y()
            .gap(6.0)
            .child(line(title(check.id), kit))
            .child(ui_components::status_pill(
                label(check.state),
                None,
                tone_for_check(check.state),
                kit,
            ))
            .child(button),
    )
}

fn maintenance_button(
    state: &State,
    preferences: ViewPreferences,
    kit: &Kit,
    check: ReadinessCheckId,
    action: ReadinessActionId,
) -> Option<(El, Option<String>)> {
    let (operation, caption) = match action {
        ReadinessActionId::SeedActors => (MaintenanceOperation::SeedActors, "Seed actors…"),
        ReadinessActionId::TrainClassifier => {
            (MaintenanceOperation::TrainClassifier, "Train classifier…")
        }
        ReadinessActionId::RebuildActorIndex => {
            (MaintenanceOperation::ReindexActors, "Build actor index")
        }
        _ => return None,
    };
    let reason = state.maintenance_disabled_reason(operation);
    let button = ActionButton::new(caption)
        .key(format!("readiness-action-{}-{action:?}", check.as_str()))
        .finish(MetalFinish::Gunmetal)
        .disabled(reason.is_some())
        .on_press_tx(move |state: &mut State, tx| match operation {
            MaintenanceOperation::SeedActors => state.choose_dataset(DatasetKind::ActorCatalog, tx),
            MaintenanceOperation::TrainClassifier => {
                state.choose_dataset(DatasetKind::ClassifierTraining, tx)
            }
            MaintenanceOperation::ReindexActors => state.preview_maintenance(operation),
            _ => {}
        })
        .build(
            kit,
            preferences,
            (!preferences.matte()).then_some(&state.pointer_offset),
        );
    Some((button, reason))
}

fn tone_for_check(state: ReadinessState) -> StatusTone {
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
