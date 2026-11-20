//! Final startup health review using the latest observations for this registered backend.

use fleetworks_contracts::readiness::ReadinessCheck;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use rune::El;
use rune_kit::glyphs;
use rune_kit::Kit;

use crate::app::State;
use crate::startup_data;
use crate::theme::StatusTone;
use crate::ui_components;

pub(crate) fn content(state: &State, kit: &Kit) -> El {
    let checks = startup_data::scoped_readiness(state).map(|readiness| &readiness.checks);
    let mut list = rune::col()
        .key("startup-data-checklist")
        .role("list")
        .label("Workstation health checks")
        .w_full()
        .min_w(0.0)
        .gap(3.0);
    for (id, title) in [
        (ReadinessCheckId::Gateway, "Gateway"),
        (ReadinessCheckId::Datastore, "Datastore"),
        (ReadinessCheckId::Jobs, "Jobs"),
        (ReadinessCheckId::ActorCatalog, "Actor catalog"),
        (ReadinessCheckId::ActorSearch, "Actor search"),
        (ReadinessCheckId::ClassifierTraining, "Classifier training"),
    ] {
        let current = checks
            .and_then(|checks| checks.iter().find(|check| check.id == id))
            .filter(|check| state.readiness_check_fresh(check));
        let (caption, glyph, tone) = status(current);
        list = list.child(
            rune::row()
                .key(format!("startup-data-check-{}", id.as_str()))
                .role("listitem")
                .label(format!("{title}: {caption}"))
                .w_full()
                .min_w(0.0)
                .gap(8.0)
                .center_y()
                .child(
                    rune::text(title)
                        .font(kit.ty.text_small)
                        .color(kit.c.std_text)
                        .grow(1.0)
                        .min_w(0.0)
                        .wrap()
                        .a11y_hidden(),
                )
                .child(ui_components::status_pill(caption, Some(glyph), tone, kit).a11y_hidden()),
        );
    }
    list
}

fn status(check: Option<&ReadinessCheck>) -> (&'static str, &'static rune::Icon, StatusTone) {
    let Some(check) = check else {
        return ("Pending", &glyphs::DASH, StatusTone::Neutral);
    };
    match check.state {
        ReadinessState::Ready
            if check.id != ReadinessCheckId::ClassifierTraining
                || check.reason_code == "embedded_training_completed"
                || check.artifact_ids.iter().any(|id| !id.trim().is_empty()) =>
        {
            ("Passed", &glyphs::CHECK, StatusTone::Ok)
        }
        ReadinessState::NotReady if check.reason_code == "process_not_ready" => {
            ("Failed", &glyphs::CLOSE, StatusTone::Err)
        }
        ReadinessState::NotReady | ReadinessState::Degraded => {
            ("Needs attention", &glyphs::DASH, StatusTone::Neutral)
        }
        ReadinessState::Running => ("Checking", &glyphs::DASH, StatusTone::Neutral),
        ReadinessState::Unavailable => ("Unavailable", &glyphs::DASH, StatusTone::Neutral),
        ReadinessState::NotApplicable => ("N/A", &glyphs::DASH, StatusTone::Neutral),
        _ => ("Pending", &glyphs::DASH, StatusTone::Neutral),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmed_embedded_training_passes_without_a_candidate_id() {
        let mut check = ReadinessCheck::new(
            ReadinessCheckId::ClassifierTraining,
            ReadinessState::Ready,
            "embedded_training_completed".into(),
            "Classifier training completed on this Embedded backend.".into(),
            "2026-01-01T00:00:00Z".into(),
        );
        let (caption, glyph, tone) = status(Some(&check));
        assert_eq!(caption, "Passed");
        assert_eq!(glyph.name, glyphs::CHECK.name);
        assert_eq!(tone, StatusTone::Ok);

        check.reason_code = "training_not_verified".into();
        let (caption, glyph, tone) = status(Some(&check));
        assert_eq!(caption, "Pending");
        assert_eq!(glyph.name, glyphs::DASH.name);
        assert_eq!(tone, StatusTone::Neutral);
    }
}
