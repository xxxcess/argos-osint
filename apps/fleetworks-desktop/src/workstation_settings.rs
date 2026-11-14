//! Workstation settings presented over the current native destination.

use rune::Cx;
use rune::El;
use rune::ScrollSpec;
use rune_kit::Kit;

use crate::accessibility::ViewPreferences;
use crate::app::Page;
use crate::app::State;
use crate::material::MetalFinish;
use crate::theme;
use crate::theme::StatusTone;
use crate::ui_components;
use crate::ui_components::ActionButton;

/// The selected section in the workstation settings dialog.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum SettingsSection {
    #[default]
    General,
    Appearance,
    Gateway,
    Models,
    Tools,
}

impl SettingsSection {
    const ALL: [Self; 5] = [
        Self::General,
        Self::Appearance,
        Self::Gateway,
        Self::Models,
        Self::Tools,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Gateway => "Gateway",
            Self::Models => "Models",
            Self::Tools => "Tools",
        }
    }

    fn summary(self) -> &'static str {
        match self {
            Self::General => "Workstation identity and readiness",
            Self::Appearance => "Display and motion preferences",
            Self::Gateway => "Connection and pairing",
            Self::Models => "Model routes and provider accounts",
            Self::Tools => "Tool provider credentials",
        }
    }
}

fn setting_row(label: &str, detail: impl Into<String>, trailing: Option<El>, kit: &Kit) -> El {
    rune::row()
        .w_full()
        .min_h(54.0)
        .px(12.0)
        .py(8.0)
        .gap(12.0)
        .center_y()
        .fill(theme::hex(theme::SURFACE_RAISED))
        .border_sides([0.0, 0.0, 1.0, 0.0], theme::hex(theme::BORDER_SUBTLE))
        .child(
            rune::col()
                .grow(1.0)
                .min_w(0.0)
                .gap(3.0)
                .child(
                    rune::text(label)
                        .font(kit.ty.ui_medium)
                        .color(theme::hex(theme::TEXT_PRIMARY)),
                )
                .child(
                    rune::text(detail.into())
                        .font(kit.ty.small)
                        .color(theme::hex(theme::TEXT_MUTED))
                        .wrap(),
                ),
        )
        .children(trailing)
}

fn setting_group(title: &str, body: El, kit: &Kit) -> El {
    rune::col()
        .w_full()
        .gap(8.0)
        .child(
            rune::text(title)
                .font(kit.ty.small_medium)
                .color(theme::hex(theme::TEXT_SECONDARY)),
        )
        .child(
            body.w_full()
                .radius(8.0)
                .clip_rounded(8.0)
                .border(1.0, theme::hex(theme::BORDER_SUBTLE)),
        )
}

fn destination_action(
    label: &'static str,
    key: &'static str,
    destination: Page,
    disabled: bool,
    state: &State,
    preferences: ViewPreferences,
    kit: &Kit,
) -> El {
    let pointer = (!preferences.matte()).then_some(&state.pointer_offset);
    ActionButton::new(label)
        .key(key)
        .finish(MetalFinish::ObsidianPill)
        .disabled(disabled)
        .on_press(move |state: &mut State| {
            state.settings_open = false;
            state.navigate(destination);
        })
        .build(kit, preferences, pointer)
}

fn section_content(state: &State, preferences: ViewPreferences, kit: &Kit) -> El {
    let pointer = (!preferences.matte()).then_some(&state.pointer_offset);
    match state.settings_section {
        SettingsSection::General => rune::col()
            .w_full()
            .gap(22.0)
            .child(setting_group(
                "Workstation",
                rune::col()
                    .child(setting_row(
                        "Registration",
                        if state.registration.context.is_some() {
                            "This installation is registered with the current gateway."
                        } else {
                            "Connect and register this installation to see private activity."
                        },
                        Some(ui_components::status_pill(
                            if state.registration.context.is_some() {
                                "Registered"
                            } else {
                                "Not registered"
                            },
                            None,
                            if state.registration.context.is_some() {
                                StatusTone::Ok
                            } else {
                                StatusTone::Warn
                            },
                            kit,
                        )),
                        kit,
                    ))
                    .child(setting_row(
                        "Readiness checks",
                        "Inspect data, index, classifier, and backend readiness.",
                        Some(destination_action(
                            "Open",
                            "settings-readiness-open",
                            Page::Workstation,
                            false,
                            state,
                            preferences,
                            kit,
                        )),
                        kit,
                    )),
                kit,
            ))
            .child(setting_group(
                "Backend",
                rune::col().child(setting_row(
                    "Deployment",
                    state.connection.deployment_label(),
                    Some(ui_components::status_pill(
                        if state.connected() {
                            "Online"
                        } else {
                            "Offline"
                        },
                        None,
                        if state.connected() {
                            StatusTone::Ok
                        } else {
                            StatusTone::Neutral
                        },
                        kit,
                    )),
                    kit,
                )),
                kit,
            )),
        SettingsSection::Appearance => rune::col().w_full().gap(22.0).child(setting_group(
            "Display",
            rune::col()
                .child(setting_row(
                    "Liquid metal",
                    "The app follows macOS Reduce Transparency.",
                    Some(ui_components::status_pill(
                        if preferences.matte() { "Matte" } else { "On" },
                        None,
                        StatusTone::Accent,
                        kit,
                    )),
                    kit,
                ))
                .child(setting_row(
                    "Motion",
                    "The app follows macOS Reduce Motion.",
                    Some(ui_components::status_pill(
                        if preferences.reduced_motion {
                            "Reduced"
                        } else {
                            "On"
                        },
                        None,
                        StatusTone::Neutral,
                        kit,
                    )),
                    kit,
                )),
            kit,
        )),
        SettingsSection::Gateway => rune::col().w_full().gap(22.0).child(setting_group(
            "Connection",
            rune::col()
                .child(setting_row(
                    "Gateway URL",
                    state.connection.origin.text.clone(),
                    Some(ui_components::status_pill(
                        if state.connected() {
                            "Connected"
                        } else {
                            "Disconnected"
                        },
                        None,
                        if state.connected() {
                            StatusTone::Ok
                        } else {
                            StatusTone::Warn
                        },
                        kit,
                    )),
                    kit,
                ))
                .child(setting_row(
                    "Pairing and access",
                    "Review the current origin or pair with a gateway.",
                    Some(
                        ActionButton::new("Setup Gateway")
                            .key("settings-gateway-open")
                            .finish(MetalFinish::Gunmetal)
                            .on_press(|state: &mut State| {
                                state.settings_open = false;
                                state.setup_gateway = true;
                            })
                            .build(kit, preferences, pointer),
                    ),
                    kit,
                )),
            kit,
        )),
        SettingsSection::Models => rune::col().w_full().gap(22.0).child(setting_group(
            "Routing",
            rune::col().child(setting_row(
                "Model assignments and accounts",
                "Inspect role routes, ordered fallbacks, and saved provider keys.",
                Some(destination_action(
                    "Open",
                    "settings-models-open",
                    Page::Models,
                    !state.connected(),
                    state,
                    preferences,
                    kit,
                )),
                kit,
            )),
            kit,
        )),
        SettingsSection::Tools => rune::col().w_full().gap(22.0).child(setting_group(
            "Integrations",
            rune::col().child(setting_row(
                "Tool credentials",
                "Inspect provider services and configure their API keys.",
                Some(destination_action(
                    "Open",
                    "settings-tools-open",
                    Page::Tools,
                    !state.connected(),
                    state,
                    preferences,
                    kit,
                )),
                kit,
            )),
            kit,
        )),
    }
}

/// Body for the Settings dialog, retaining the selected operational page behind it.
pub(crate) fn page(state: &State, cx: &Cx<State>, kit: &Kit) -> El {
    let preferences = state
        .accessibility
        .for_view(state.fixture, cx.is_reduced_motion());
    let pointer = (!preferences.matte()).then_some(&state.pointer_offset);
    let height = cx
        .layout_size("page-content")
        .map(|size| (size[1] - 24.0).clamp(300.0, 580.0))
        .unwrap_or(540.0);

    let mut rail = rune::col()
        .key("settings-category-rail")
        .w(176.0)
        .h_full()
        .padding(10.0)
        .gap(4.0)
        .fill(theme::hex(theme::SURFACE_RAISED))
        .border_sides([0.0, 1.0, 0.0, 0.0], theme::hex(theme::BORDER_SUBTLE))
        .child(
            rune::text("Settings")
                .font(kit.ty.small_medium)
                .color(theme::hex(theme::TEXT_MUTED))
                .px(8.0)
                .py(7.0),
        );
    for section in SettingsSection::ALL {
        let selected = state.settings_section == section;
        rail = rail.child(
            rune::row()
                .key(format!(
                    "settings-category-{}",
                    section.title().to_lowercase()
                ))
                .role("button")
                .label(section.title())
                .focusable(true)
                .focus_ring(2.0, 1.0, theme::hex(theme::ACCENT_HIGHLIGHT))
                .w_full()
                .h(31.0)
                .px(9.0)
                .center_y()
                .radius(6.0)
                .fill(theme::hex(if selected {
                    theme::SELECTION_SURFACE
                } else {
                    theme::SURFACE_RAISED
                }))
                .child(
                    rune::text(section.title())
                        .font(if selected {
                            kit.ty.ui_medium
                        } else {
                            kit.ty.small_medium
                        })
                        .color(theme::hex(if selected {
                            theme::TEXT_PRIMARY
                        } else {
                            theme::TEXT_SECONDARY
                        }))
                        .a11y_hidden(),
                )
                .on_press(move |state: &mut State| state.settings_section = section),
        );
    }
    rail = rail.child(rune::spacer()).child(
        ActionButton::new("Close Settings")
            .key("settings-close")
            .finish(MetalFinish::ObsidianPill)
            .on_press(|state: &mut State| state.settings_open = false)
            .build(kit, preferences, pointer),
    );

    let content = rune::col()
        .key("settings-content")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(theme::hex(theme::SURFACE))
        .child(
            rune::col()
                .w_full()
                .px(20.0)
                .pt(18.0)
                .pb(14.0)
                .gap(4.0)
                .border_sides([0.0, 0.0, 1.0, 0.0], theme::hex(theme::BORDER_SUBTLE))
                .child(
                    rune::text(state.settings_section.title())
                        .font(kit.ty.body_medium)
                        .color(theme::hex(theme::TEXT_PRIMARY)),
                )
                .child(
                    rune::text(state.settings_section.summary())
                        .font(kit.ty.small)
                        .color(theme::hex(theme::TEXT_MUTED)),
                ),
        )
        .child(
            rune::col()
                .key("settings-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(section_content(state, preferences, kit).padding(20.0)),
        );

    rune::row()
        .key("workstation-settings-body")
        .w_full()
        .h(height)
        .min_h(0.0)
        .clip_rounded(10.0)
        .child(rail)
        .child(content)
}

#[cfg(test)]
mod tests {
    use rune::Env;
    use rune::Runtime;

    use crate::app;
    use crate::app::Page;

    use super::SettingsSection;

    #[test]
    fn settings_categories_open_the_existing_configuration_destination() {
        let mut state = crate::tests::registered_fixture();
        state.settings_open = true;
        let mut runtime = Runtime::new(
            state,
            app::view,
            (1180.0, 780.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        runtime.frame(0.0);
        assert!(runtime.find("settings-category-rail").is_some());
        assert!(runtime.find("settings-readiness-open").is_some());

        let (x, y) = runtime
            .find("settings-category-models")
            .expect("Models category in actual Settings dialog");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(1.0 / 60.0);
        assert_eq!(runtime.state.settings_section, SettingsSection::Models);
        assert!(runtime.find("settings-models-open").is_some());

        let (x, y) = runtime
            .find("settings-models-open")
            .expect("Models destination in actual Settings dialog");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(2.0 / 60.0);
        assert_eq!(runtime.state.page, Page::Models);
        assert!(!runtime.state.settings_open);
    }
}
