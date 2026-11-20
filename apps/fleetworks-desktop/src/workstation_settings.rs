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

impl State {
    /// Open Settings in the synthetic review session.
    pub fn fixture_show_workstation_settings(&mut self) {
        if self.fixture {
            self.settings_section = SettingsSection::General;
            self.settings_open = true;
        }
    }
}

fn setting_row(label: &str, detail: impl Into<String>, trailing: Option<El>, kit: &Kit) -> El {
    rune::row()
        .w_full()
        .min_w(0.0)
        .min_h(54.0)
        .px(12.0)
        .py(8.0)
        .gap(12.0)
        .center_y()
        .child(
            rune::col()
                .grow(1.0)
                .min_w(0.0)
                .gap(3.0)
                .child(
                    rune::text(label)
                        .font(kit.ty.ui_medium)
                        .color(theme::hex(theme::TEXT_PRIMARY))
                        .wrap(),
                )
                .child(
                    rune::text(detail.into())
                        .font(kit.ty.small)
                        .color(theme::hex(theme::TEXT_MUTED))
                        .wrap_anywhere(),
                ),
        )
        .children(trailing)
}

fn setting_divider() -> El {
    rune::el()
        .w_full()
        .h(1.0)
        .fill(theme::hex(theme::BORDER_SUBTLE))
        .a11y_hidden()
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
                .fill(theme::hex(theme::SURFACE))
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
                    .child(setting_divider())
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
                    "Surfaces",
                    surface_preference_description(cfg!(target_os = "macos")),
                    Some(ui_components::status_pill(
                        if preferences.matte() {
                            "Opaque"
                        } else {
                            "Transparent"
                        },
                        None,
                        StatusTone::Accent,
                        kit,
                    )),
                    kit,
                ))
                .child(setting_divider())
                .child(setting_row(
                    "Reduced motion",
                    motion_preference_description(cfg!(target_os = "macos")),
                    Some(ui_components::status_pill(
                        if preferences.reduced_motion {
                            "Reduced"
                        } else {
                            "Standard"
                        },
                        None,
                        StatusTone::Neutral,
                        kit,
                    )),
                    kit,
                )),
            kit,
        )),
        SettingsSection::Gateway => {
            let mut rows = rune::col().child(setting_row(
                if state.is_local_backend() {
                    "Embedded backend"
                } else {
                    "Distributed gateway URL"
                },
                if state.is_local_backend() {
                    format!(
                        "{} ({})",
                        state.connection.origin.text,
                        state.local_state_dir_label()
                    )
                } else {
                    state.connection.origin.text.clone()
                },
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
            ));
            if state.local_backend.active() {
                rows = rows.child(setting_divider()).child(setting_row(
                    "Embedded companion process",
                    "Gracefully shut down the embedded companion backend.",
                    Some(
                        ActionButton::new("Stop Embedded Backend")
                            .key("settings-stop-local-backend")
                            .finish(MetalFinish::Gunmetal)
                            .disabled(state.local_backend.stopping)
                            .on_press(|state: &mut State| state.stop_local_backend())
                            .build(kit, preferences, pointer),
                    ),
                    kit,
                ));
            }
            rows = rows.child(setting_divider()).child(setting_row(
                "Backend and access",
                "Select Embedded or Distributed, or inspect connection status.",
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
            ));
            rune::col()
                .w_full()
                .gap(22.0)
                .child(setting_group("Connection", rows, kit))
        }
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
    let page_size = cx.layout_size("page-content");
    let compact = page_size.is_none_or(|size| size[0] < 580.0);
    let height = page_size
        .map(|size| (size[1] - 62.0).clamp(290.0, 580.0))
        .unwrap_or(370.0);
    let content_padding = if compact { 12.0 } else { 20.0 };

    let mut rail = rune::col()
        .key("settings-category-rail")
        .w(if compact { 124.0 } else { 168.0 })
        .h_full()
        .padding(10.0)
        .gap(4.0)
        .fill(theme::hex(theme::SURFACE))
        .border_sides([0.0, 1.0, 0.0, 0.0], theme::hex(theme::BORDER_SUBTLE));
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
                .description(if selected { "Current section" } else { "" })
                .focusable(true)
                .focus_ring(2.0, 2.0, theme::hex(theme::ACCENT_HIGHLIGHT))
                .w_full()
                .h(34.0)
                .px(12.0)
                .center_y()
                .radius(17.0)
                .fill(theme::hex(if selected {
                    theme::MOLTEN_SELECTION
                } else {
                    theme::SURFACE
                }))
                .hover(theme::hex(if selected {
                    theme::MOLTEN_SELECTION
                } else {
                    theme::SURFACE_ELEVATED
                }))
                .child(
                    rune::text(section.title())
                        .font(if selected {
                            kit.ty.ui_medium
                        } else {
                            kit.ty.small_medium
                        })
                        .color(theme::hex(if selected {
                            theme::MOLTEN_ACCENT
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
        .child(
            rune::col()
                .w_full()
                .px(content_padding)
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
                        .color(theme::hex(theme::TEXT_MUTED))
                        .wrap(),
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
                .child(section_content(state, preferences, kit).padding(content_padding)),
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

fn surface_preference_description(is_macos: bool) -> &'static str {
    if is_macos {
        "Follows macOS Reduce Transparency and FLEETWORKS_OPAQUE_MATERIALS."
    } else {
        "Uses transparent surfaces by default, or opaque surfaces when FLEETWORKS_OPAQUE_MATERIALS is set."
    }
}

fn motion_preference_description(is_macos: bool) -> &'static str {
    if is_macos {
        "Follows macOS Reduce Motion."
    } else {
        "Follows your desktop reduced-motion accessibility preference."
    }
}

#[cfg(test)]
mod tests {
    use rune::Env;
    use rune::Runtime;

    use crate::app;
    use crate::app::Page;

    use super::motion_preference_description;
    use super::surface_preference_description;
    use super::SettingsSection;

    #[test]
    fn appearance_copy_omits_macos_wording_on_linux() {
        let linux_surfaces = surface_preference_description(false);
        let linux_motion = motion_preference_description(false);
        assert!(!linux_surfaces.contains("macOS"));
        assert!(!linux_motion.contains("macOS"));
        assert!(linux_surfaces.contains("FLEETWORKS_OPAQUE_MATERIALS"));
        assert!(linux_motion.to_ascii_lowercase().contains("reduced-motion"));

        let mac_surfaces = surface_preference_description(true);
        let mac_motion = motion_preference_description(true);
        assert!(mac_surfaces.contains("macOS Reduce Transparency"));
        assert!(mac_motion.contains("macOS Reduce Motion"));
    }

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
