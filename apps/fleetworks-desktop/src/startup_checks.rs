//! Startup checklists using the latest observations for the selected backend.

use fleetworks_contracts::readiness::ReadinessCheck;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use fleetworks_contracts::readiness::WorkstationReadiness;
use rune::El;
use rune_kit::glyphs;
use rune_kit::Kit;

use crate::accessibility::ViewPreferences;
use crate::app::Load;
use crate::app::State;
use crate::material;
use crate::theme::StatusTone;
use crate::ui_components;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckStatus {
    Pending,
    Checking,
    Passed,
    Failed,
    NeedsSetup,
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
        CheckStatus::Passed => ("Passed", &glyphs::CHECK, StatusTone::Ok),
        CheckStatus::Failed => ("Failed", &glyphs::CLOSE, StatusTone::Err),
        CheckStatus::NeedsSetup => ("Needs setup", &glyphs::DASH, StatusTone::Warn),
        CheckStatus::NeedsAttention => ("Needs attention", &glyphs::DASH, StatusTone::Warn),
        CheckStatus::Unavailable => ("Unavailable", &glyphs::DASH, StatusTone::Neutral),
    };
    let value = if status == CheckStatus::Checking {
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

pub(crate) fn scoped_readiness(state: &State) -> Option<&WorkstationReadiness> {
    let Load::Ready(readiness) = &state.readiness else {
        return None;
    };
    let context = state.registration.context.as_ref()?;
    let receipt = readiness.registration.as_ref()?;
    (state.connected()
        && readiness.deployment_id == context.deployment_id
        && receipt.registration_id == context.registration_id
        && receipt.revision == context.registration_revision)
        .then_some(readiness)
}

pub(crate) fn embedded_content(
    state: &State,
    pending: bool,
    preferences: ViewPreferences,
    kit: &Kit,
) -> El {
    let missing = if pending {
        CheckStatus::Checking
    } else {
        CheckStatus::Unavailable
    };
    let checks = scoped_readiness(state).map(|readiness| &readiness.checks);
    let mut list = list(
        "startup-wiring-health-checklist",
        "Workstation health checks",
    );
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
            &format!("startup-wiring-health-check-{}", id.as_str()),
            title,
            current.map_or(missing, status),
            preferences,
            kit,
        ));
    }
    list
}

fn status(check: &ReadinessCheck) -> CheckStatus {
    match check.state {
        ReadinessState::Ready => CheckStatus::Passed,
        ReadinessState::NotReady if check.reason_code == "process_not_ready" => CheckStatus::Failed,
        ReadinessState::NotReady
            if matches!(
                check.reason_code.as_str(),
                "database_catalog_empty"
                    | "database_seed_incomplete"
                    | "index_not_built"
                    | "index_source_not_started"
            ) =>
        {
            CheckStatus::NeedsSetup
        }
        ReadinessState::NotReady | ReadinessState::Degraded => CheckStatus::NeedsAttention,
        ReadinessState::Running => CheckStatus::Checking,
        ReadinessState::Unavailable => CheckStatus::Unavailable,
        _ => CheckStatus::Pending,
    }
}
