//! Startup checklists using the latest observations for the selected backend.

use fleetworks_contracts::readiness::ReadinessCheck;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use rune::El;
use rune_kit::glyphs;
use rune_kit::Kit;

use crate::accessibility::ViewPreferences;
use crate::app::State;
use crate::material;
use crate::startup_data;
use crate::theme::StatusTone;
use crate::ui_components;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckStatus {
    Pending,
    Checking,
    Verifying,
    Passed,
    Failed,
    NeedsAttention,
    Unavailable,
}

pub(crate) fn list(key: &str, label: &str) -> El {
    rune::col()
        .key(key)
        .role("list")
        .label(label)
        .w_full()
        .min_w(0.0)
        .gap(3.0)
}

pub(crate) fn row(
    key: &str,
    title: &str,
    status: CheckStatus,
    preferences: ViewPreferences,
    kit: &Kit,
) -> El {
    let (caption, glyph, tone) = match status {
        CheckStatus::Pending => ("Pending", &glyphs::DASH, StatusTone::Neutral),
        CheckStatus::Checking => ("Checking", &glyphs::DASH, StatusTone::Cyan),
        CheckStatus::Verifying => ("Verifying", &glyphs::DASH, StatusTone::Cyan),
        CheckStatus::Passed => ("Passed", &glyphs::CHECK, StatusTone::Ok),
        CheckStatus::Failed => ("Failed", &glyphs::CLOSE, StatusTone::Err),
        CheckStatus::NeedsAttention => ("Needs attention", &glyphs::DASH, StatusTone::Warn),
        CheckStatus::Unavailable => ("Unavailable", &glyphs::DASH, StatusTone::Neutral),
    };
    let value = if matches!(status, CheckStatus::Checking | CheckStatus::Verifying) {
        rune::row()
            .h(22.0)
            .center_y()
            .child(material::skeleton_block(
                None,
                Some(68.0),
                11.0,
                4.0,
                tone,
                preferences,
            ))
    } else {
        ui_components::status_pill(caption, Some(glyph), tone, kit)
    };
    rune::row()
        .key(key)
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
        .child(value.a11y_hidden())
}

pub(crate) fn content(state: &State, preferences: ViewPreferences, kit: &Kit) -> El {
    let checks = startup_data::scoped_readiness(state).map(|readiness| &readiness.checks);
    let mut list = list("startup-data-checklist", "Workstation health checks");
    for (id, title) in [
        (ReadinessCheckId::Gateway, "Gateway"),
        (ReadinessCheckId::Datastore, "Datastore"),
        (ReadinessCheckId::Nlp, "Language service"),
        (ReadinessCheckId::Registration, "Registration"),
        (ReadinessCheckId::Jobs, "Jobs"),
        (ReadinessCheckId::ActorCatalog, "Actor catalog"),
        (ReadinessCheckId::ActorSearch, "Actor search"),
        (
            ReadinessCheckId::ClassifierInference,
            "Atlas BERT inference",
        ),
    ] {
        let check = checks.and_then(|checks| checks.iter().find(|check| check.id == id));
        match check {
            Some(check) if !state.readiness_check_applies(check) => continue,
            None if state.is_local_backend() && id.requires_distributed_backend() => continue,
            _ => {}
        }
        let current = check.filter(|check| state.readiness_check_fresh(check));
        list = list.child(row(
            &format!("startup-data-check-{}", id.as_str()),
            title,
            status(current),
            preferences,
            kit,
        ));
    }
    list
}

fn status(check: Option<&ReadinessCheck>) -> CheckStatus {
    let Some(check) = check else {
        return CheckStatus::Pending;
    };
    match check.state {
        ReadinessState::Ready => CheckStatus::Passed,
        ReadinessState::NotReady if check.reason_code == "process_not_ready" => CheckStatus::Failed,
        ReadinessState::NotReady | ReadinessState::Degraded => CheckStatus::NeedsAttention,
        ReadinessState::Running => CheckStatus::Checking,
        ReadinessState::Unavailable => CheckStatus::Unavailable,
        _ => CheckStatus::Pending,
    }
}
