//! Shared workstation UI components built atop Rune and `rune_kit`.

use std::rc::Rc;

use rune::col;
use rune::el;
use rune::icon;
use rune::row;
use rune::spacer;
use rune::text;
use rune::El;
use rune::Icon;
use rune::Live;
use rune::Shadow;
use rune::TextStyle;
use rune::Transition;
use rune::Tx;
use rune_kit::Dialog;
use rune_kit::IconButton;
use rune_kit::IconButtonVariant;
use rune_kit::Kit;
use rune_kit::Progress;
use rune_kit::Spinner;
use rune_kit::Switch;

use crate::accessibility::ViewPreferences;
use crate::material;
use crate::material::MetalFinish;
use crate::theme;
use crate::theme::StatusTone;

type ActionHandler = Box<dyn FnOnce(El) -> El>;

/// Liquid-metal workstation action button with inline preflight relay, shimmer trigger, and matte parity.
pub(crate) struct ActionButton {
    key: Option<String>,
    label: String,
    icon_glyph: Option<&'static Icon>,
    finish: MetalFinish,
    busy: bool,
    shimmer: bool,
    disabled: bool,
    status: Option<(String, StatusTone)>,
    handler: Option<ActionHandler>,
}

impl ActionButton {
    pub(crate) fn new(label: impl Into<String>) -> Self {
        Self {
            key: None,
            label: label.into(),
            icon_glyph: None,
            finish: MetalFinish::Gunmetal,
            busy: false,
            shimmer: false,
            disabled: false,
            status: None,
            handler: None,
        }
    }

    pub(crate) fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub(crate) fn icon(mut self, glyph: &'static Icon) -> Self {
        self.icon_glyph = Some(glyph);
        self
    }

    pub(crate) fn finish(mut self, finish: MetalFinish) -> Self {
        self.finish = finish;
        self
    }

    pub(crate) fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub(crate) fn status(mut self, label: impl Into<String>, tone: StatusTone) -> Self {
        self.status = Some((label.into(), tone));
        self
    }

    pub(crate) fn on_press<S: 'static>(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.handler = Some(Box::new(move |e: El| e.on_press(f)));
        self
    }

    pub(crate) fn on_press_tx<S: 'static>(mut self, f: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        self.handler = Some(Box::new(move |e: El| e.on_press_tx(f)));
        self
    }

    pub(crate) fn build(
        self,
        kit: &Kit,
        preferences: ViewPreferences,
        pointer_offset: Option<&Live<[f32; 2]>>,
    ) -> El {
        let ink = if self.disabled {
            theme::hex(theme::TEXT_DISABLED)
        } else {
            material::finish_ink(self.finish)
        };
        let active_shimmer = self.shimmer || self.busy;
        let disabled = self.disabled || self.busy;
        // Textured actions use the theme's 44 px control height and 16 px padding.
        let height = match self.finish {
            MetalFinish::ObsidianPill => 28.0,
            finish if finish.fractured() => 44.0,
            _ => 32.0,
        };
        let px = match self.finish {
            MetalFinish::ObsidianPill => 11.0,
            finish if finish.fractured() => 16.0,
            _ => 14.0,
        };
        let a11y_label = match &self.status {
            Some((status_label, _)) => format!("{} ({status_label})", self.label),
            None => self.label.clone(),
        };

        let mut content = row()
            .role("button")
            .label(a11y_label)
            .h(height)
            .px(px)
            .gap(7.0)
            .center()
            .focusable(!disabled)
            .disabled(disabled)
            .focus_ring(2.0, 2.0, theme::hex(theme::ACCENT_HIGHLIGHT));

        if self.busy {
            content = content.child(
                Spinner::new()
                    .size(14.0)
                    .color(ink)
                    .build(kit)
                    .a11y_hidden(),
            );
        } else if let Some(glyph) = self.icon_glyph {
            content = content.child(icon(glyph, 14.0).color(ink).a11y_hidden());
        }

        content = content.child(
            text(self.label)
                .font(kit.ty.ui_medium)
                .color(ink)
                .a11y_hidden(),
        );

        if let Some((status_label, tone)) = self.status {
            content = content.child(
                row()
                    .center_y()
                    .gap(5.0)
                    .h(20.0)
                    .px(7.0)
                    .radius(999.0)
                    .fill(tone.bg())
                    .border(1.0, tone.border())
                    .child(status_dot(tone, preferences))
                    .child(
                        text(status_label)
                            .font(TextStyle {
                                line_height: 12.0,
                                ..kit.ty.small_medium
                            })
                            .color(tone.fg())
                            .a11y_hidden(),
                    )
                    .a11y_hidden(),
            );
        }

        let tiles = if self.finish.fractured() {
            material::texture_tiles(self.key.as_deref())
        } else {
            0
        };
        let mut styled = material::apply_liquid_metal(
            content,
            self.finish,
            disabled,
            active_shimmer,
            preferences,
            pointer_offset,
            tiles,
        );
        if let Some(key) = self.key {
            styled = styled.key(key);
        }
        if !disabled {
            if let Some(handler) = self.handler {
                styled = handler(styled);
            }
        }
        styled
    }
}

/// High-contrast semantic pill badge with optional leading icon.
pub(crate) fn status_pill(
    label: impl Into<String>,
    glyph: Option<&'static Icon>,
    tone: StatusTone,
    kit: &Kit,
) -> El {
    let label = label.into();
    let fg = tone.fg();
    let mut badge = row()
        .role("status")
        .label(label.clone())
        .center_y()
        .gap(5.0)
        .h(22.0)
        .px(8.0)
        .radius(999.0)
        .fill(tone.bg().alpha(kit.c.bg_button_neutral.a))
        .border(1.0, tone.border());
    if let Some(glyph) = glyph {
        badge = badge.child(icon(glyph, 12.0).color(fg).a11y_hidden());
    }
    badge.child(
        text(label)
            .font(TextStyle {
                line_height: 13.0,
                ..kit.ty.small_medium
            })
            .color(fg)
            .a11y_hidden(),
    )
}

/// Visual status dot with subtle glow halo for telemetry & sidebar indicators.
pub(crate) fn status_dot(tone: StatusTone, preferences: ViewPreferences) -> El {
    let mut dot = el()
        .size(7.0, 7.0)
        .radius(999.0)
        .fill(tone.fg())
        .a11y_hidden();
    if !preferences.matte() {
        dot = dot.box_shadow(Shadow {
            blur: 8.0,
            spread: 1.0,
            color: tone.fg().alpha(0.55),
            ..Shadow::default()
        });
    }
    dot
}

/// Async loading indicator banner with radar spinner and progress shimmer track.
pub(crate) fn async_status_strip(
    key: impl Into<String>,
    title: impl Into<String>,
    detail: impl Into<String>,
    busy: bool,
    tone: StatusTone,
    preferences: ViewPreferences,
    kit: &Kit,
) -> El {
    let title = title.into();
    let detail = detail.into();
    let key = key.into();
    let mut header = row().w_full().min_w(0.0).gap(8.0).center_y();
    if busy {
        header = header.child(Spinner::new().size(15.0).color(tone.fg()).build(kit));
    } else {
        header = header.child(status_dot(tone, preferences));
    }
    header = header.child(
        text(title)
            .grow(1.0)
            .min_w(0.0)
            .font(kit.ty.ui_medium)
            .color(tone.fg())
            .wrap(),
    );

    let mut strip = col()
        .key(key)
        .w_full()
        .min_w(0.0)
        .gap(6.0)
        .padding(10.0)
        .radius(8.0)
        .fill(tone.bg())
        .border(1.0, tone.border())
        .child(header);
    if !detail.is_empty() {
        strip = strip.child(
            text(detail)
                .key("async-status-detail")
                .w_full()
                .min_w(0.0)
                .font(kit.ty.small)
                .color(kit.c.text_muted)
                .wrap_anywhere(),
        );
    }
    if busy {
        strip = strip.child(Progress::new(None).build(kit));
    }
    strip
}

/// Floating dialog with a dimmed backdrop and Rune-managed focus and dismissal.
#[allow(clippy::too_many_arguments)]
pub(crate) fn glass_dialog<S: 'static>(
    key: &str,
    title: &str,
    width: f32,
    body: El,
    dismiss_outside: bool,
    close: impl Fn(&mut S) + Clone + 'static,
    preferences: ViewPreferences,
    kit: &Kit,
) -> [El; 2] {
    let scrim = el()
        .key(format!("{key}-scrim"))
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .fill(theme::hex(theme::BACKGROUND).alpha(0.86))
        .a11y_hidden();
    let dismiss = IconButton::new(kit.glyph.close)
        .variant(IconButtonVariant::Ghost)
        .label("Close")
        .on_press(close.clone())
        .build(kit)
        .key("close");
    let panel = col()
        .key(format!("{key}-dialog"))
        .role("dialog")
        .label(title)
        .w_full()
        .max_w(width)
        .radius(10.0)
        .child(
            row()
                .key("header")
                .w_full()
                .px(16.0)
                .py(10.0)
                .gap(12.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], theme::hex(theme::BORDER_SUBTLE))
                .child(
                    text(title)
                        .grow(1.0)
                        .min_w(0.0)
                        .font(kit.ty.ui_medium)
                        .color(kit.c.text_base)
                        .wrap(),
                )
                .child(dismiss),
        )
        .child(col().key("body").w_full().child(body))
        .modal(true, close)
        .modal_click_outside(dismiss_outside);
    let panel = material::glass(panel, material::GlassSurface::Overlay, preferences);
    let (scrim, panel) = if preferences.reduced_motion {
        (scrim, panel)
    } else {
        (
            scrim.transition(Transition::opacity()),
            panel.transition(Transition::opacity()),
        )
    };
    [
        scrim,
        el().key(format!("{key}-layer"))
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .padding(16.0)
            .center()
            .child(panel),
    ]
}

/// Inline configuration toggle row backed by `rune_kit::Switch`.
pub(crate) fn inline_toggle_row<S: 'static>(
    key: impl Into<String>,
    checked: bool,
    label: impl Into<String>,
    description: Option<&str>,
    disabled: bool,
    kit: &Kit,
    on_toggle: impl Fn(&mut S) + 'static,
) -> El {
    let key = key.into();
    let label = label.into();
    let on_toggle = Rc::new(on_toggle);
    let toggle_switch = Rc::clone(&on_toggle);
    // The kit's inline label and description have intrinsic widths. Lay out the
    // copy separately so it can shrink and wrap while the switch keeps its hit area.
    let mut switch = Switch::new(checked)
        .label(label.clone())
        .hide_label()
        .standard()
        .disabled(disabled)
        .on_toggle(move |state| toggle_switch(state))
        .build(kit)
        .key("switch")
        .min_w(44.0)
        .min_h(32.0)
        .center();

    let mut copy = col().key("copy").grow(1.0).min_w(0.0).gap(2.0).child(
        text(label)
            .key("label")
            .font(kit.ty.text_small)
            .color(kit.c.std_text)
            .wrap()
            .a11y_hidden(),
    );
    if let Some(desc) = description {
        copy = copy.child(
            text(desc)
                .key("description")
                .font(kit.ty.text_xsmall)
                .color(kit.c.std_text)
                .wrap()
                .a11y_hidden(),
        );
        switch = switch.description(desc);
    }

    let focus_key = key.clone();
    let control = row()
        .key(key)
        .w_full()
        .min_w(0.0)
        .padding(10.0)
        .gap(8.0)
        .radius(8.0)
        .fill(kit.c.bg_button_neutral)
        .center_y()
        .child(copy)
        .child(switch);
    let control = kit
        .shadow_layers(&kit.elevation.button_neutral)
        .fold(control, El::box_shadow);
    if disabled {
        control
    } else {
        control
            .on_press_tx(move |state, tx| {
                on_toggle(state);
                tx.focus(&focus_key, false);
            })
            .role("group")
    }
}

/// Horizontal workflow stage stepper showing progress and current active step.
pub(crate) fn stage_stepper(
    key: impl Into<String>,
    steps: &[&str],
    active_index: usize,
    preferences: ViewPreferences,
    kit: &Kit,
) -> El {
    let mut track = row().key(key).w_full().gap(8.0).center_y();
    for (idx, step) in steps.iter().enumerate() {
        let current = idx == active_index;
        let completed = idx < active_index;
        let tone = if current {
            StatusTone::Accent
        } else if completed {
            StatusTone::Ok
        } else {
            StatusTone::Neutral
        };
        let pill = row()
            .grow(1.0)
            .h(28.0)
            .px(10.0)
            .gap(6.0)
            .center_y()
            .radius(7.0)
            .fill(if current {
                theme::hex(theme::SELECTION_SURFACE)
            } else {
                tone.bg()
            })
            .border(
                1.0,
                if current {
                    theme::hex(theme::ACCENT)
                } else {
                    tone.border()
                },
            )
            .child(status_dot(tone, preferences))
            .child(
                text(format!("{}. {step}", idx + 1))
                    .font(if current {
                        kit.ty.small_medium
                    } else {
                        kit.ty.small
                    })
                    .color(if current || completed {
                        tone.fg()
                    } else {
                        kit.c.text_faint
                    }),
            );
        track = track.child(pill);
    }
    track
}

/// Concise section header with icon, short title/subtitle and optional right accessory.
pub(crate) fn section_header(
    glyph: &'static Icon,
    title: impl Into<String>,
    subtitle: Option<&str>,
    badge: Option<El>,
    right: Option<El>,
    kit: &Kit,
) -> El {
    let mut left = col().min_w(0.0).grow(1.0).gap(2.0).child(
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(
                icon(glyph, 16.0)
                    .color(theme::hex(theme::ACCENT))
                    .a11y_hidden(),
            )
            .child(
                text(title.into())
                    .font(kit.ty.body_medium)
                    .color(kit.c.text_base),
            )
            .children(badge),
    );
    if let Some(sub) = subtitle {
        left = left.child(
            text(sub.to_owned())
                .font(kit.ty.small)
                .color(kit.c.text_muted)
                .wrap(),
        );
    }
    row()
        .w_full()
        .center_y()
        .gap(12.0)
        .child(left)
        .children(right)
}

/// Compact modal prompt helper using `rune_kit::Dialog`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn modal_prompt<S: 'static>(
    key: impl Into<String>,
    title: impl Into<String>,
    description: impl Into<String>,
    open: bool,
    width: f32,
    on_close: impl Fn(&mut S) + Clone + 'static,
    body: El,
    footer: Option<El>,
    kit: &Kit,
    layer: &mut Vec<El>,
) {
    let description = description.into();
    let mut content = col().w_full().px(16.0).pb(16.0).gap(12.0);
    if !description.is_empty() {
        content = content.child(
            text(description)
                .font(kit.ty.small)
                .color(kit.c.text_muted)
                .wrap(),
        );
    }
    content = content.child(body);
    if let Some(footer) = footer {
        content = content.child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(spacer())
                .child(footer),
        );
    }
    Dialog::new(key, title)
        .open(open)
        .width(width)
        .fit()
        .click_outside(true)
        .on_close(on_close)
        .child(content)
        .build(kit, layer);
}

fn skeleton_card_shell(body: El, tone: StatusTone, preferences: ViewPreferences, kit: &Kit) -> El {
    let fill = if preferences.matte() {
        kit.c.surface_raised
    } else {
        theme::hex(theme::SURFACE)
    };
    let border = if tone == StatusTone::Neutral {
        theme::hex(theme::BORDER_SUBTLE).alpha(0.65)
    } else {
        tone.border().alpha(0.50)
    };
    body.w_full().fill(fill).border(1.0, border).radius(10.0)
}

/// Readiness check list skeleton mirroring gateway readiness rows.
pub(crate) fn skeleton_readiness_list(preferences: ViewPreferences, kit: &Kit) -> El {
    let mut list = col()
        .key("readiness-loading")
        .role("status")
        .label("Refreshing backend checks")
        .w_full()
        .gap(8.0)
        .child(
            row()
                .w_full()
                .center_y()
                .gap(8.0)
                .child(material::skeleton_block(
                    None,
                    Some(82.0),
                    22.0,
                    999.0,
                    StatusTone::Info,
                    preferences,
                ))
                .child(material::skeleton_block(
                    None,
                    Some(132.0),
                    12.0,
                    4.0,
                    StatusTone::Neutral,
                    preferences,
                )),
        );
    for width_frac in [0.38, 0.46, 0.34, 0.42] {
        let card = col()
            .padding(12.0)
            .gap(6.0)
            .child(
                row()
                    .w_full()
                    .center_y()
                    .gap(8.0)
                    .child(material::skeleton_block(
                        None,
                        Some(7.0),
                        7.0,
                        999.0,
                        StatusTone::Accent,
                        preferences,
                    ))
                    .child(material::skeleton_block(
                        Some(width_frac),
                        None,
                        13.0,
                        4.0,
                        StatusTone::Neutral,
                        preferences,
                    ))
                    .child(spacer())
                    .child(material::skeleton_block(
                        None,
                        Some(64.0),
                        20.0,
                        999.0,
                        StatusTone::Neutral,
                        preferences,
                    )),
            )
            .child(material::skeleton_block(
                Some(0.68),
                None,
                10.0,
                4.0,
                StatusTone::Neutral,
                preferences,
            ));
        list = list.child(skeleton_card_shell(
            card,
            StatusTone::Neutral,
            preferences,
            kit,
        ));
    }
    list
}

/// Actor search result list skeleton mirroring Brain hit tiles.
pub(crate) fn skeleton_actor_results(preferences: ViewPreferences, kit: &Kit) -> El {
    let mut list = col()
        .key("search-loading")
        .role("status")
        .label("Searching actor catalog")
        .w_full()
        .gap(8.0);
    for (title_f, sub_f) in [
        (0.56, 0.82),
        (0.48, 0.74),
        (0.64, 0.68),
        (0.52, 0.86),
        (0.60, 0.72),
    ] {
        let tile = row()
            .w_full()
            .padding(10.0)
            .gap(10.0)
            .center_y()
            .child(material::skeleton_block(
                None,
                Some(7.0),
                7.0,
                999.0,
                StatusTone::Violet,
                preferences,
            ))
            .child(
                col()
                    .grow(1.0)
                    .gap(5.0)
                    .child(material::skeleton_block(
                        Some(title_f),
                        None,
                        13.0,
                        4.0,
                        StatusTone::Neutral,
                        preferences,
                    ))
                    .child(material::skeleton_block(
                        Some(sub_f),
                        None,
                        10.0,
                        4.0,
                        StatusTone::Neutral,
                        preferences,
                    )),
            )
            .child(material::skeleton_block(
                None,
                Some(14.0),
                14.0,
                4.0,
                StatusTone::Neutral,
                preferences,
            ));
        list = list.child(skeleton_card_shell(
            tile,
            StatusTone::Violet,
            preferences,
            kit,
        ));
    }
    list
}

/// Actor dossier right-pane skeleton mirroring dossier header, facet switcher, and sections.
pub(crate) fn skeleton_dossier(preferences: ViewPreferences, kit: &Kit) -> El {
    let card_body = col()
        .padding(16.0)
        .gap(12.0)
        .child(
            row()
                .w_full()
                .center_y()
                .gap(10.0)
                .child(
                    col()
                        .grow(1.0)
                        .gap(6.0)
                        .child(material::skeleton_block(
                            Some(0.48),
                            None,
                            18.0,
                            5.0,
                            StatusTone::Violet,
                            preferences,
                        ))
                        .child(
                            row()
                                .gap(6.0)
                                .child(material::skeleton_block(
                                    None,
                                    Some(68.0),
                                    22.0,
                                    999.0,
                                    StatusTone::Violet,
                                    preferences,
                                ))
                                .child(material::skeleton_block(
                                    None,
                                    Some(74.0),
                                    22.0,
                                    999.0,
                                    StatusTone::Cyan,
                                    preferences,
                                )),
                        ),
                )
                .child(material::skeleton_block(
                    None,
                    Some(156.0),
                    28.0,
                    8.0,
                    StatusTone::Neutral,
                    preferences,
                )),
        )
        .child(material::skeleton_block(
            None,
            Some(138.0),
            11.0,
            4.0,
            StatusTone::Neutral,
            preferences,
        ))
        .child(material::skeleton_block(
            Some(0.92),
            None,
            12.0,
            4.0,
            StatusTone::Neutral,
            preferences,
        ))
        .child(material::skeleton_block(
            Some(0.76),
            None,
            12.0,
            4.0,
            StatusTone::Neutral,
            preferences,
        ))
        .child(material::skeleton_block(
            Some(1.0),
            None,
            64.0,
            8.0,
            StatusTone::Neutral,
            preferences,
        ))
        .child(material::skeleton_block(
            Some(1.0),
            None,
            86.0,
            8.0,
            StatusTone::Neutral,
            preferences,
        ));
    skeleton_card_shell(card_body, StatusTone::Violet, preferences, kit)
        .key("dossier-loading")
        .role("status")
        .label("Loading actor dossier")
}

/// OSINT tool catalog skeleton mirroring tool cards.
pub(crate) fn skeleton_tool_catalog(preferences: ViewPreferences, kit: &Kit) -> El {
    let mut list = col()
        .key("tools-loading")
        .role("status")
        .label("Loading OSINT tools")
        .w_full()
        .gap(10.0);
    for (name_w, desc_f, rest_f) in [
        (136.0, 0.88, 0.62),
        (162.0, 0.80, 0.56),
        (124.0, 0.92, 0.66),
    ] {
        let card_body = col()
            .padding(14.0)
            .gap(8.0)
            .child(
                row()
                    .w_full()
                    .center_y()
                    .gap(8.0)
                    .child(material::skeleton_block(
                        None,
                        Some(15.0),
                        15.0,
                        4.0,
                        StatusTone::Cyan,
                        preferences,
                    ))
                    .child(material::skeleton_block(
                        None,
                        Some(name_w),
                        15.0,
                        5.0,
                        StatusTone::Neutral,
                        preferences,
                    ))
                    .child(spacer())
                    .child(material::skeleton_block(
                        None,
                        Some(72.0),
                        22.0,
                        999.0,
                        StatusTone::Cyan,
                        preferences,
                    )),
            )
            .child(material::skeleton_block(
                Some(desc_f),
                None,
                12.0,
                4.0,
                StatusTone::Neutral,
                preferences,
            ))
            .child(material::skeleton_block(
                Some(rest_f),
                None,
                10.0,
                4.0,
                StatusTone::Neutral,
                preferences,
            ));
        list = list.child(skeleton_card_shell(
            card_body,
            StatusTone::Cyan,
            preferences,
            kit,
        ));
    }
    list
}

/// Tool provider credential skeleton mirroring primary/fallback slot rows.
pub(crate) fn skeleton_tool_credentials(preferences: ViewPreferences, kit: &Kit) -> El {
    let mut list = col()
        .key("tool-credentials-loading")
        .role("status")
        .label("Loading provider credentials")
        .w_full()
        .gap(8.0);
    for name_w in [118.0, 140.0, 108.0] {
        let tile = row()
            .w_full()
            .padding(14.0)
            .gap(10.0)
            .center_y()
            .child(material::skeleton_block(
                None,
                Some(7.0),
                7.0,
                999.0,
                StatusTone::Info,
                preferences,
            ))
            .child(
                col()
                    .grow(1.0)
                    .gap(6.0)
                    .child(material::skeleton_block(
                        None,
                        Some(name_w),
                        15.0,
                        4.0,
                        StatusTone::Neutral,
                        preferences,
                    ))
                    .child(
                        row()
                            .gap(6.0)
                            .child(material::skeleton_block(
                                None,
                                Some(106.0),
                                22.0,
                                999.0,
                                StatusTone::Ok,
                                preferences,
                            ))
                            .child(material::skeleton_block(
                                None,
                                Some(102.0),
                                22.0,
                                999.0,
                                StatusTone::Neutral,
                                preferences,
                            )),
                    ),
            )
            .child(material::skeleton_block(
                None,
                Some(132.0),
                32.0,
                7.0,
                StatusTone::Neutral,
                preferences,
            ));
        list = list.child(skeleton_card_shell(
            tile,
            StatusTone::Neutral,
            preferences,
            kit,
        ));
    }
    list
}

/// Model role assignment list skeleton mirroring role cards.
pub(crate) fn skeleton_model_roles(preferences: ViewPreferences, kit: &Kit) -> El {
    let mut list = col()
        .key("models-loading")
        .role("status")
        .label("Loading role assignments")
        .w_full()
        .gap(8.0)
        .child(material::skeleton_block(
            None,
            Some(164.0),
            22.0,
            999.0,
            StatusTone::Info,
            preferences,
        ));
    for (role_w, route_w) in [
        (132.0, 190.0),
        (148.0, 174.0),
        (120.0, 206.0),
        (142.0, 168.0),
    ] {
        let card_body = col()
            .padding(14.0)
            .gap(8.0)
            .child(
                row()
                    .w_full()
                    .center_y()
                    .gap(8.0)
                    .child(material::skeleton_block(
                        None,
                        Some(7.0),
                        7.0,
                        999.0,
                        StatusTone::Info,
                        preferences,
                    ))
                    .child(material::skeleton_block(
                        None,
                        Some(role_w),
                        15.0,
                        5.0,
                        StatusTone::Neutral,
                        preferences,
                    ))
                    .child(spacer())
                    .child(material::skeleton_block(
                        None,
                        Some(64.0),
                        22.0,
                        999.0,
                        StatusTone::Accent,
                        preferences,
                    )),
            )
            .child(
                row()
                    .w_full()
                    .center_y()
                    .gap(8.0)
                    .child(material::skeleton_block(
                        None,
                        Some(route_w),
                        12.0,
                        4.0,
                        StatusTone::Neutral,
                        preferences,
                    ))
                    .child(spacer())
                    .child(material::skeleton_block(
                        None,
                        Some(88.0),
                        30.0,
                        7.0,
                        StatusTone::Neutral,
                        preferences,
                    )),
            );
        list = list.child(skeleton_card_shell(
            card_body,
            StatusTone::Info,
            preferences,
            kit,
        ));
    }
    list
}

/// Provider accounts list skeleton mirroring account credential cards.
pub(crate) fn skeleton_provider_accounts(preferences: ViewPreferences, kit: &Kit) -> El {
    let mut list = col()
        .key("providers-loading")
        .role("status")
        .label("Loading provider accounts")
        .w_full()
        .gap(8.0);
    for width_px in [156.0, 140.0, 172.0] {
        let card_body = col()
            .padding(14.0)
            .gap(10.0)
            .child(
                row()
                    .w_full()
                    .center_y()
                    .gap(8.0)
                    .child(material::skeleton_block(
                        None,
                        Some(7.0),
                        7.0,
                        999.0,
                        StatusTone::Ok,
                        preferences,
                    ))
                    .child(material::skeleton_block(
                        None,
                        Some(width_px),
                        15.0,
                        5.0,
                        StatusTone::Neutral,
                        preferences,
                    ))
                    .child(spacer())
                    .child(material::skeleton_block(
                        None,
                        Some(118.0),
                        22.0,
                        999.0,
                        StatusTone::Ok,
                        preferences,
                    )),
            )
            .child(material::skeleton_block(
                None,
                Some(116.0),
                32.0,
                7.0,
                StatusTone::Neutral,
                preferences,
            ));
        list = list.child(skeleton_card_shell(
            card_body,
            StatusTone::Ok,
            preferences,
            kit,
        ));
    }
    list
}

/// Workstation activity job list skeleton mirroring job tiles.
pub(crate) fn skeleton_job_list(preferences: ViewPreferences, kit: &Kit) -> El {
    let mut list = col()
        .key("jobs-loading")
        .role("status")
        .label("Loading workstation jobs")
        .w_full()
        .gap(8.0);
    for (title_f, id_f) in [(0.58, 0.42), (0.64, 0.48), (0.50, 0.38), (0.62, 0.44)] {
        let tile = row()
            .w_full()
            .padding(10.0)
            .gap(10.0)
            .center_y()
            .child(material::skeleton_block(
                None,
                Some(7.0),
                7.0,
                999.0,
                StatusTone::Info,
                preferences,
            ))
            .child(
                col()
                    .grow(1.0)
                    .gap(5.0)
                    .child(material::skeleton_block(
                        Some(title_f),
                        None,
                        13.0,
                        4.0,
                        StatusTone::Neutral,
                        preferences,
                    ))
                    .child(material::skeleton_block(
                        Some(id_f),
                        None,
                        10.0,
                        4.0,
                        StatusTone::Neutral,
                        preferences,
                    )),
            )
            .child(material::skeleton_block(
                None,
                Some(64.0),
                22.0,
                999.0,
                StatusTone::Info,
                preferences,
            ));
        list = list.child(skeleton_card_shell(
            tile,
            StatusTone::Info,
            preferences,
            kit,
        ));
    }
    list
}

/// Operational log table skeleton mirroring Timestamp | Severity | Safe summary columns.
pub(crate) fn skeleton_log_table(preferences: ViewPreferences, kit: &Kit) -> El {
    let mut table = col()
        .key("skeleton-logs-table")
        .role("status")
        .label("Loading operational logs")
        .w_full()
        .padding(10.0)
        .gap(8.0)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .pb(4.0)
                .child(material::skeleton_block(
                    None,
                    Some(88.0),
                    11.0,
                    4.0,
                    StatusTone::Neutral,
                    preferences,
                ))
                .child(material::skeleton_block(
                    None,
                    Some(54.0),
                    11.0,
                    4.0,
                    StatusTone::Neutral,
                    preferences,
                ))
                .child(material::skeleton_block(
                    None,
                    Some(96.0),
                    11.0,
                    4.0,
                    StatusTone::Neutral,
                    preferences,
                )),
        )
        .child(row().w_full().h(1.0).fill(theme::hex(theme::BORDER_SUBTLE)));
    for summary_f in [0.84, 0.72, 0.90, 0.64, 0.78] {
        table = table.child(
            row()
                .w_full()
                .gap(12.0)
                .center_y()
                .child(material::skeleton_block(
                    None,
                    Some(108.0),
                    12.0,
                    4.0,
                    StatusTone::Neutral,
                    preferences,
                ))
                .child(material::skeleton_block(
                    None,
                    Some(52.0),
                    18.0,
                    999.0,
                    StatusTone::Info,
                    preferences,
                ))
                .child(col().grow(1.0).child(material::skeleton_block(
                    Some(summary_f),
                    None,
                    12.0,
                    4.0,
                    StatusTone::Neutral,
                    preferences,
                ))),
        );
    }
    skeleton_card_shell(table, StatusTone::Neutral, preferences, kit)
}

/// Selected job detail skeleton mirroring detail header, progress row, and log slice.
pub(crate) fn skeleton_job_detail(preferences: ViewPreferences, kit: &Kit) -> El {
    let body = col()
        .key("job-detail-loading")
        .role("status")
        .label("Loading job details")
        .padding(16.0)
        .gap(12.0)
        .child(
            row()
                .w_full()
                .center_y()
                .gap(10.0)
                .child(
                    col()
                        .grow(1.0)
                        .gap(5.0)
                        .child(
                            row()
                                .gap(8.0)
                                .center_y()
                                .child(material::skeleton_block(
                                    None,
                                    Some(16.0),
                                    16.0,
                                    4.0,
                                    StatusTone::Accent,
                                    preferences,
                                ))
                                .child(material::skeleton_block(
                                    None,
                                    Some(146.0),
                                    16.0,
                                    5.0,
                                    StatusTone::Neutral,
                                    preferences,
                                ))
                                .child(material::skeleton_block(
                                    None,
                                    Some(62.0),
                                    22.0,
                                    999.0,
                                    StatusTone::Info,
                                    preferences,
                                )),
                        )
                        .child(material::skeleton_block(
                            None,
                            Some(230.0),
                            11.0,
                            4.0,
                            StatusTone::Neutral,
                            preferences,
                        )),
                )
                .child(material::skeleton_block(
                    None,
                    Some(122.0),
                    28.0,
                    14.0,
                    StatusTone::Neutral,
                    preferences,
                )),
        )
        .child(
            row()
                .w_full()
                .center_y()
                .gap(10.0)
                .child(material::skeleton_block(
                    None,
                    Some(24.0),
                    24.0,
                    999.0,
                    StatusTone::Accent,
                    preferences,
                ))
                .child(material::skeleton_block(
                    Some(1.0),
                    None,
                    10.0,
                    999.0,
                    StatusTone::Accent,
                    preferences,
                )),
        )
        .child(skeleton_log_table(preferences, kit));
    skeleton_card_shell(body, StatusTone::Accent, preferences, kit)
}
