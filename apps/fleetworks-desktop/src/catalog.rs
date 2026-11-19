//! Fixture-only component catalog.
//!
//! It draws the production components in every state they can show without input, so one capture
//! can be reviewed per display preference. It has no route, no menu entry and no way to open it
//! from the running app; only a fixture scene turns it on. Hover, focus and pressed states need
//! pointer or key input and are covered by the scripted scenes instead.

use rune::col;
use rune::row;
use rune::text;
use rune::El;
use rune_kit::Kit;

use crate::accessibility::ViewPreferences;
use crate::icons;
use crate::material;
use crate::material::MetalFinish;
use crate::theme;
use crate::theme::StatusTone;
use crate::ui_components::section_header;
use crate::ui_components::status_pill;
use crate::ui_components::ActionButton;

const FINISHES: [(&str, MetalFinish); 7] = [
    ("gold", MetalFinish::GoldCta),
    ("gunmetal", MetalFinish::Gunmetal),
    ("danger", MetalFinish::CrimsonDanger),
    ("pill", MetalFinish::ObsidianPill),
    ("fractured", MetalFinish::Fractured),
    ("fractured-confirm", MetalFinish::FracturedConfirm),
    ("fractured-danger", MetalFinish::FracturedDanger),
];

const TONES: [(&str, StatusTone); 8] = [
    ("Neutral", StatusTone::Neutral),
    ("Accent", StatusTone::Accent),
    ("Ok", StatusTone::Ok),
    ("Warn", StatusTone::Warn),
    ("Error", StatusTone::Err),
    ("Info", StatusTone::Info),
    ("Violet", StatusTone::Violet),
    ("Cyan", StatusTone::Cyan),
];

fn group(title: &str, kit: &Kit, body: El) -> El {
    col()
        .key(format!(
            "catalog-{}",
            title.to_lowercase().replace(' ', "-")
        ))
        .w_full()
        .gap(10.0)
        .child(
            text(title)
                .font(kit.ty.body_medium)
                .color(theme::hex(theme::TEXT_PRIMARY)),
        )
        .child(body)
}

fn buttons(kit: &Kit, preferences: ViewPreferences) -> El {
    let mut rows = col().w_full().gap(8.0);
    for (name, finish) in FINISHES {
        let one = |state: &str, button: ActionButton| {
            button
                .key(format!("catalog-button-{name}-{state}"))
                .finish(finish)
                .build(kit, preferences, None)
        };
        rows = rows.child(
            row()
                .w_full()
                .gap(10.0)
                .center_y()
                .child(one("default", ActionButton::new("Default")))
                .child(one("busy", ActionButton::new("Busy").busy(true)))
                .child(one(
                    "disabled",
                    ActionButton::new("Disabled").disabled(true),
                ))
                .child(one(
                    "status",
                    ActionButton::new("With status").status("Ready", StatusTone::Ok),
                )),
        );
    }
    rows
}

fn pills(kit: &Kit) -> El {
    let mut line = row().w_full().gap(8.0).flex_wrap();
    for (label, tone) in TONES {
        line = line.child(status_pill(label, None, tone, kit));
    }
    line
}

fn tiles(kit: &Kit, preferences: ViewPreferences) -> El {
    let tile = |key: &str, label: &str, selected: bool| {
        material::interactive_tile(
            col()
                .key(key.to_owned())
                .w(220.0)
                .h(72.0)
                .padding(12.0)
                .gap(4.0)
                .child(
                    text(label.to_owned())
                        .font(kit.ty.small_medium)
                        .color(theme::hex(theme::TEXT_PRIMARY)),
                )
                .child(
                    text("Clickable tile on dim glass")
                        .font(kit.ty.small)
                        .color(theme::hex(theme::TEXT_MUTED)),
                ),
            selected,
            StatusTone::Accent,
            preferences,
            None,
            None,
        )
    };
    row()
        .w_full()
        .gap(12.0)
        .child(tile("catalog-tile-default", "Default", false))
        .child(tile("catalog-tile-selected", "Selected", true))
}

fn panels(kit: &Kit, preferences: ViewPreferences) -> El {
    // A reading panel stays opaque in every preference.
    let panel = material::luminous_card(
        col()
            .key("catalog-reading-panel".to_owned())
            .w(460.0)
            .padding(14.0)
            .gap(6.0)
            .child(
                text("Reading panel")
                    .font(kit.ty.small_medium)
                    .color(theme::hex(theme::TEXT_PRIMARY)),
            )
            .child(
                text("Evidence and long text sit on an opaque surface.")
                    .font(kit.ty.small)
                    .color(theme::hex(theme::TEXT_SECONDARY))
                    .wrap(),
            ),
        StatusTone::Neutral,
        false,
        preferences,
        None,
        None,
    );
    col().w_full().gap(10.0).child(panel).child(section_header(
        &icons::ASSISTANT,
        "Section header",
        Some("With a subtitle"),
        None,
        None,
        kit,
    ))
}

/// The whole catalog, scrolling like any other page body.
pub(crate) fn view(kit: &Kit, preferences: ViewPreferences) -> El {
    let body = col()
        .key("catalog-root")
        .w_full()
        .gap(20.0)
        .child(group("Buttons", kit, buttons(kit, preferences)))
        .child(group("Status pills", kit, pills(kit)))
        .child(group("Tiles", kit, tiles(kit, preferences)))
        .child(group("Panels and headers", kit, panels(kit, preferences)));
    crate::views::scroll(body, "catalog-body")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::State;
    use rune::Env;
    use rune::Runtime;

    fn runtime(opaque: bool) -> Runtime<State> {
        let mut state = crate::tests::registered_fixture();
        state.fixture_show_catalog();
        if opaque {
            state.accessibility.fixture_override =
                Some(crate::accessibility::FixtureAccessibility {
                    reduced_transparency: true,
                    ..crate::accessibility::FixtureAccessibility::default()
                });
        }
        let mut runtime = Runtime::new(
            state,
            crate::app::view,
            (1280.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        runtime.frame(0.0);
        runtime.frame(1.0 / 60.0);
        runtime
    }

    #[test]
    fn every_finish_shows_its_states_in_both_preferences() {
        for opaque in [false, true] {
            let rt = runtime(opaque);
            for (name, _) in FINISHES {
                for state in ["default", "busy", "disabled", "status"] {
                    assert!(
                        rt.find(&format!("catalog-button-{name}-{state}")).is_some(),
                        "{name} {state} (opaque={opaque})"
                    );
                }
            }
            for key in [
                "catalog-tile-default",
                "catalog-tile-selected",
                "catalog-reading-panel",
                "catalog-status-pills",
            ] {
                assert!(rt.find(key).is_some(), "{key} (opaque={opaque})");
            }
        }
    }
}
