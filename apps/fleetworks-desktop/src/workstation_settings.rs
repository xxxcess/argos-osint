//! Workstation settings presented over the current native destination.

use fleetworks_contracts::embedded_resources::ResourcePreset;
use fleetworks_workstation::native::BackendKind;
use rune::Cx;
use rune::El;
use rune::ScrollSpec;
use rune::Tx;
use rune_kit::Kit;
use rune_kit::SegmentedControl;

use crate::accessibility::ViewPreferences;
use crate::app::Load;
use crate::app::Page;
use crate::app::State;
use crate::material;
use crate::material::MetalFinish;
use crate::resource_settings::Resource;
use crate::theme;
use crate::theme::StatusTone;
use crate::ui_components;
use crate::ui_components::ActionButton;

pub(crate) const PRODUCT_NAME: &str = "Fleetworks";

/// The selected section in the workstation settings dialog.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum SettingsSection {
    #[default]
    General,
    Resources,
    Connection,
    Assistant,
    Models,
}

impl SettingsSection {
    const ALL: [Self; 5] = [
        Self::General,
        Self::Connection,
        Self::Assistant,
        Self::Resources,
        Self::Models,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Resources => "Resources",
            Self::Connection => "Connection",
            Self::Assistant => "Assistant",
            Self::Models => "Models",
        }
    }

    fn summary(self) -> &'static str {
        match self {
            Self::General => "About and workstation readiness",
            Self::Resources => "Backend resource limits",
            Self::Connection => "Backend setup and pairing",
            Self::Assistant => "Decision models",
            Self::Models => "Model routes and provider accounts",
        }
    }
}

impl State {
    pub(crate) fn open_workstation_settings(&mut self) {
        self.settings_open = true;
        self.select_settings_section(self.settings_section);
    }

    fn select_settings_section(&mut self, section: SettingsSection) {
        self.settings_section = section;
        if section == SettingsSection::Resources && !self.resource_settings.saving {
            self.read_resource_settings();
        }
    }

    pub(crate) fn open_connection_settings(&mut self) {
        self.setup_gateway = false;
        self.settings_section = SettingsSection::Connection;
        self.settings_open = true;
    }

    pub(crate) fn open_distributed_setup(&mut self, tx: &mut Tx) {
        self.set_backend_kind(BackendKind::Remote);
        if self.startup.active() {
            self.setup_gateway = true;
        } else {
            self.open_connection_settings();
        }
        self.prefill_startup_distributed_url();
        tx.scroll_into_view("gateway-origin");
        tx.focus("gateway-origin", tx.by_keys());
    }

    /// Open Settings in the synthetic review session.
    pub fn fixture_show_workstation_settings(&mut self) {
        if self.fixture {
            self.settings_section = SettingsSection::General;
            self.settings_open = true;
        }
    }

    /// Open the Assistant section in the synthetic review session.
    #[doc(hidden)]
    pub fn fixture_show_assistant_settings(&mut self) {
        if self.fixture {
            self.settings_section = SettingsSection::Assistant;
            self.settings_open = true;
        }
    }

    /// Open the Resources section in the synthetic review session.
    #[doc(hidden)]
    pub fn fixture_show_resource_settings(&mut self) {
        if self.fixture {
            self.backend_kind = BackendKind::Local;
            self.settings_section = SettingsSection::Resources;
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

fn setting_group(title: &str, body: El, preferences: ViewPreferences, kit: &Kit) -> El {
    rune::col()
        .w_full()
        .gap(8.0)
        .child(
            rune::text(title)
                .font(kit.ty.small_medium)
                .color(theme::hex(theme::TEXT_SECONDARY)),
        )
        .child(material::glass_panel(
            body.w_full().radius(8.0).clip_rounded(8.0),
            preferences,
        ))
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

fn resource_settings(state: &State, preferences: ViewPreferences, kit: &Kit) -> El {
    let text = |value: &str| {
        rune::text(value)
            .font(kit.ty.small)
            .color(theme::hex(theme::TEXT_MUTED))
            .wrap()
    };
    if !state.is_local_backend() {
        return text("Your operator manages resource limits for the Distributed backend.");
    }
    let settings = &state.resource_settings;
    let saved = settings.displayed(state.fixture);
    let mut controls = rune::col().w_full();
    let labels = ResourcePreset::ALL.map(|preset| format!("{}%", preset.percent()));
    for (index, resource) in Resource::ALL.into_iter().enumerate() {
        if index != 0 {
            controls = controls.child(setting_divider());
        }
        let selected = saved.and_then(|saved| {
            ResourcePreset::ALL
                .iter()
                .position(|&preset| preset == resource.selected(saved))
        });
        let mut options = SegmentedControl::new(&labels, selected)
            .label(resource.label())
            .fit()
            .on_pick(move |state: &mut State, index, _| {
                if let Some(&preset) = ResourcePreset::ALL.get(index) {
                    state.save_resource_setting(resource, preset);
                }
            });
        if saved.is_none() || settings.saving {
            for index in 0..ResourcePreset::ALL.len() {
                options = options.segment_state(index, rune_kit::State::Disabled);
            }
        }
        controls = controls.child(
            rune::row()
                .w_full()
                .min_h(54.0)
                .px(12.0)
                .py(8.0)
                .gap(8.0)
                .center_y()
                .child(
                    rune::text(resource.label())
                        .font(kit.ty.ui_medium)
                        .color(theme::hex(theme::TEXT_PRIMARY))
                        .grow(1.0)
                        .a11y_hidden(),
                )
                .child(options.build(kit).key(resource.key())),
        );
    }
    let mut body = rune::col()
        .w_full()
        .gap(12.0)
        .child(
            rune::col()
                .gap(4.0)
                .child(text("Saved limits take effect the next time Embedded starts."))
                .child(text(
                    "CPU sets how many available cores workers can use. Memory and disk apply to available memory and free storage at startup.",
                )),
        )
        .child(material::glass_panel(
            controls.radius(8.0).clip_rounded(8.0),
            preferences,
        ));
    let pending = if settings.saving {
        Some("Saving resource preferences…")
    } else if saved.is_none() && !matches!(settings.preferences, Load::Failed(_)) {
        Some("Loading saved resource preferences…")
    } else {
        None
    };
    if let Some(label) = pending {
        body = body.child(
            ui_components::skeleton_text_value(label, 180.0, StatusTone::Info, preferences)
                .key("settings-resources-progress"),
        );
    } else if settings.save_failed {
        body =
            body.child(text("Could not save the change. Select a limit to retry.").role("alert"));
    } else if let Load::Failed(error) = &settings.preferences {
        body = body.child(text(error).role("alert")).child(
            ActionButton::new("Retry")
                .key("settings-resources-retry")
                .finish(MetalFinish::Gunmetal)
                .on_press(State::read_resource_settings)
                .build(kit, preferences, None),
        );
    }
    body
}

fn section_content(
    state: &State,
    section: SettingsSection,
    preferences: ViewPreferences,
    kit: &Kit,
    connection_size: Option<[f32; 2]>,
) -> El {
    match section {
        SettingsSection::General => rune::col()
            .w_full()
            .gap(22.0)
            .child(setting_group(
                "About",
                setting_row(
                    PRODUCT_NAME,
                    concat!("Version ", env!("CARGO_PKG_VERSION")),
                    None,
                    kit,
                )
                .key("settings-about-app"),
                preferences,
                kit,
            ))
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
                preferences,
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
                preferences,
                kit,
            )),
        SettingsSection::Resources => resource_settings(state, preferences, kit),
        SettingsSection::Connection => rune::col()
            .w_full()
            .gap(22.0)
            .child(crate::views::setup_gateway_content(
                state,
                kit,
                preferences,
                connection_size,
            ))
            .children(state.is_local_backend().then(|| {
                setting_group(
                    "Backend inference",
                    crate::inbox_qa::settings_content(state, kit, preferences).padding(12.0),
                    preferences,
                    kit,
                )
            })),
        SettingsSection::Assistant => material::glass_panel(
            crate::inbox_qa::assistant_settings_content(state, kit)
                .radius(8.0)
                .clip_rounded(8.0),
            preferences,
        ),
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
            preferences,
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
    let assistant_visible = state.assistant_decision_settings().is_some();
    let current_section =
        if state.settings_section == SettingsSection::Assistant && !assistant_visible {
            SettingsSection::Connection
        } else {
            state.settings_section
        };
    let page_size = cx.layout_size("page-content");
    let compact = page_size.is_none_or(|size| size[0] < 580.0);
    let height = page_size
        .map(|size| (size[1] - 62.0).clamp(290.0, 580.0))
        .unwrap_or(370.0);
    let content_padding = if compact { 12.0 } else { 20.0 };
    let connection_size = Some(
        cx.layout_size("settings-scroll")
            .map(|size| [(size[0] - 2.0 * content_padding).max(0.0), size[1]])
            .unwrap_or([400.0, height]),
    );

    let mut rail = rune::col()
        .key("settings-category-rail")
        .w(if compact { 124.0 } else { 168.0 })
        .h_full()
        .padding(10.0)
        .gap(4.0)
        .fill(kit.c.surface_base)
        .border_sides([0.0, 1.0, 0.0, 0.0], theme::hex(theme::BORDER_SUBTLE));
    for section in SettingsSection::ALL {
        if section == SettingsSection::Assistant && !assistant_visible {
            continue;
        }
        let selected = current_section == section;
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
                .fill(if selected {
                    theme::hex(theme::MOLTEN_SELECTION)
                } else {
                    rune::Color::TRANSPARENT
                })
                .hover(if selected {
                    theme::hex(theme::MOLTEN_SELECTION)
                } else {
                    kit.c.layer_03
                })
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
                .on_press(move |state: &mut State| state.select_settings_section(section)),
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
                    rune::text(current_section.title())
                        .key("settings-section-title")
                        .role("heading")
                        .focusable(true)
                        .tab_index(-1)
                        .font(kit.ty.body_medium)
                        .color(theme::hex(theme::TEXT_PRIMARY)),
                )
                .child(
                    rune::text(current_section.summary())
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
                .child(
                    section_content(state, current_section, preferences, kit, connection_size)
                        .padding(content_padding),
                ),
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
    use fleetworks_contracts::embedded_resources::EmbeddedResourcePreferences;
    use fleetworks_contracts::embedded_resources::ResourcePreset;
    use fleetworks_workstation::native::BackendKind;
    use rune::Env;
    use rune::Runtime;

    use crate::app;
    use crate::app::Page;
    use crate::app::State;

    use super::SettingsSection;

    #[test]
    fn resources_are_independent_and_only_editable_for_embedded() {
        let mut state = State::fixture();
        state.fixture_show_resource_settings();
        let mut runtime = Runtime::new(
            state,
            app::view,
            (640.0, 480.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        runtime.frame(0.0);
        runtime.frame(1.0 / 60.0);
        assert!(runtime.find("settings-category-resources").is_some());
        assert!(runtime.find("settings-category-appearance").is_none());
        assert!(runtime
            .accessibility_json()
            .to_string()
            .contains("Saved limits take effect the next time Embedded starts."));
        let scroll = runtime
            .elements()
            .into_iter()
            .find(|element| element.key.as_deref() == Some("settings-scroll"))
            .expect("Settings scroll area")
            .rect;
        for (index, key) in [
            "settings-resource-cpu/segment1",
            "settings-resource-memory/segment2",
        ]
        .into_iter()
        .enumerate()
        {
            let (x, y) = runtime.find(key).expect("editable preset in Resources");
            assert!((scroll[0]..scroll[0] + scroll[2]).contains(&x));
            assert!((scroll[1]..scroll[1] + scroll[3]).contains(&y));
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame((2 + index) as f64 / 60.0);
        }
        assert_eq!(
            runtime.state.resource_settings.displayed(true),
            Some(EmbeddedResourcePreferences {
                cpu: ResourcePreset::Sixty,
                memory: ResourcePreset::Eighty,
                ..EmbeddedResourcePreferences::default()
            })
        );
        runtime.update(|state, _| state.backend_kind = BackendKind::Remote);
        runtime.frame(4.0 / 60.0);
        assert!(runtime.find("settings-resource-cpu").is_none());
        assert!(runtime
            .accessibility_json()
            .to_string()
            .contains("Your operator manages resource limits for the Distributed backend."));
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
        assert!(runtime.find("settings-about-app").is_some());
        let about = runtime.accessibility_json().to_string();
        assert!(about.contains(concat!("Version ", env!("CARGO_PKG_VERSION"))));

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

    #[test]
    fn connection_settings_exposes_the_existing_backend_stepper_on_a_compact_window() {
        let mut state = State::fixture();
        state.fixture_show_workstation_settings();
        let mut runtime = Runtime::new(
            state,
            app::view,
            (640.0, 480.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        runtime.frame(0.0);
        runtime.frame(1.0 / 60.0);
        let (x, y) = runtime
            .find("settings-category-connection")
            .expect("Connection in Settings");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(2.0 / 60.0);
        assert_eq!(runtime.state.settings_section, SettingsSection::Connection);
        assert!(runtime.find("gateway-setup-stepper").is_some());
        assert!(runtime.find("gateway-origin").is_some());
        assert!(runtime.find("connect").is_some());

        runtime.update(|_, tx| tx.scroll_into_view("backend-mode-local"));
        runtime.frame(3.0 / 60.0);
        let (x, y) = runtime
            .find("backend-mode-local")
            .expect("Embedded backend choice");
        let settings = runtime
            .elements()
            .into_iter()
            .find(|el| el.key.as_deref() == Some("settings-scroll"))
            .expect("Settings scroll area");
        assert!((settings.rect[0]..settings.rect[0] + settings.rect[2]).contains(&x));
        assert!((settings.rect[1]..settings.rect[1] + settings.rect[3]).contains(&y));
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(4.0 / 60.0);
        assert_eq!(runtime.state.backend_kind, BackendKind::Local);
        assert!(runtime.find("connect-local").is_some());
        assert!(runtime.find("gateway-origin").is_none());
        assert!(runtime.state.settings_open);
        assert!(!runtime.state.setup_gateway);
    }
}
