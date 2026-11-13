//! Shared workstation UI components built atop Rune and `rune_kit`.

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
use rune::Tx;
use rune_kit::Checkbox;
use rune_kit::Dialog;
use rune_kit::Kit;
use rune_kit::Progress;
use rune_kit::ProgressCircle;
use rune_kit::RollingText;
use rune_kit::Spinner;
use rune_kit::Switch;

use crate::accessibility::ViewPreferences;
use crate::material;
use crate::material::MetalFinish;
use crate::theme;
use crate::theme::StatusTone;

type ActionHandler = Box<dyn FnOnce(El) -> El>;

/// Liquid-metal workstation action button with busy spinner, hover motion and matte parity.
pub(crate) struct ActionButton {
    key: Option<String>,
    label: String,
    icon_glyph: Option<&'static Icon>,
    finish: MetalFinish,
    busy: bool,
    disabled: bool,
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
            disabled: false,
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
        let ink = material::finish_ink(self.finish);
        let disabled = self.disabled || self.busy;
        let height = match self.finish {
            MetalFinish::ObsidianPill => 28.0,
            _ => 32.0,
        };
        let px = match self.finish {
            MetalFinish::ObsidianPill => 11.0,
            _ => 14.0,
        };

        let mut content = row()
            .role("button")
            .label(self.label.clone())
            .h(height)
            .px(px)
            .gap(7.0)
            .center()
            .focusable(!disabled)
            .disabled(disabled)
            .focus_ring(2.0, 1.0, theme::hex(theme::ACCENT_HIGHLIGHT));

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

        let mut styled = material::apply_liquid_metal(
            content,
            self.finish,
            disabled,
            preferences,
            pointer_offset,
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
        .fill(tone.bg())
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
    let mut header = row().gap(8.0).center_y();
    if busy {
        header = header.child(Spinner::new().size(15.0).color(tone.fg()).build(kit));
    } else {
        header = header.child(status_dot(tone, preferences));
    }
    header = header
        .child(text(title.clone()).font(kit.ty.ui_medium).color(tone.fg()))
        .child(text(detail).font(kit.ty.small).color(kit.c.text_muted));

    let mut strip = col()
        .key(key)
        .w_full()
        .gap(6.0)
        .padding(10.0)
        .radius(8.0)
        .fill(tone.bg())
        .border(1.0, tone.border())
        .child(header);
    if busy {
        strip = strip.child(Progress::new(None).build(kit));
    }
    strip
}

/// Interactive visual telemetry tile using `ProgressCircle`, `RollingText` and pointer-reactive glow.
#[allow(clippy::too_many_arguments)]
pub(crate) fn metric_ring_tile(
    key: impl Into<String>,
    label: impl Into<String>,
    value_text: impl Into<String>,
    numeric_value: Option<f64>,
    fraction: Option<f32>,
    subtitle: impl Into<String>,
    glyph: &'static Icon,
    tone: StatusTone,
    active: bool,
    preferences: ViewPreferences,
    kit: &Kit,
    pointer_offset: Option<&Live<[f32; 2]>>,
    shadow_offset: Option<&Live<[f32; 2]>>,
) -> El {
    let key = key.into();
    let label = label.into();
    let value_text = value_text.into();
    let subtitle = subtitle.into();
    let fg = tone.fg();

    let ring = match fraction {
        Some(value) => ProgressCircle::new(value.clamp(0.0, 1.0))
            .size(34.0)
            .label(label.clone())
            .build(kit),
        None => el()
            .size(34.0, 34.0)
            .radius(999.0)
            .fill(tone.bg())
            .border(1.0, tone.border())
            .center()
            .child(icon(glyph, 16.0).color(fg).a11y_hidden()),
    };

    let mut rolling = RollingText::new(format!("{key}-value"), value_text.clone())
        .font(TextStyle {
            size: 18.0,
            line_height: 22.0,
            ..kit.ty.ui_medium
        })
        .color(theme::hex(theme::TEXT_PRIMARY));
    if let Some(num) = numeric_value {
        rolling = rolling.value(num);
    }

    let body = col()
        .w_full()
        .padding(12.0)
        .gap(8.0)
        .child(
            row()
                .w_full()
                .center_y()
                .gap(8.0)
                .child(icon(glyph, 14.0).color(fg).a11y_hidden())
                .child(
                    text(label.clone())
                        .font(kit.ty.small_medium)
                        .color(kit.c.text_muted),
                )
                .child(spacer())
                .child(status_dot(tone, preferences)),
        )
        .child(
            row().w_full().center_y().gap(10.0).child(ring).child(
                col()
                    .gap(2.0)
                    .child(rolling.build(kit))
                    .child(text(subtitle).font(kit.ty.small).color(kit.c.text_faint)),
            ),
        );

    material::interactive_tile(
        body,
        active,
        tone,
        preferences,
        pointer_offset,
        shadow_offset,
    )
    .key(key)
    .role("button")
    .label(format!("{label}: {value_text}"))
    .focusable(true)
    .focus_ring(2.0, 0.0, theme::hex(theme::ACCENT_HIGHLIGHT))
}

/// Interactive checklist row backed by `rune_kit::Checkbox`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn checklist_row<S: 'static>(
    key: impl Into<String>,
    checked: bool,
    indeterminate: bool,
    label: impl Into<String>,
    description: impl Into<String>,
    disabled: bool,
    tone: StatusTone,
    kit: &Kit,
    on_toggle: impl Fn(&mut S) + 'static,
) -> El {
    let box_builder = Checkbox::new(checked)
        .label(label)
        .description(description)
        .indeterminate(indeterminate)
        .disabled(disabled)
        .on_toggle(on_toggle);
    row()
        .key(key)
        .w_full()
        .padding(10.0)
        .radius(8.0)
        .fill(theme::hex(theme::SURFACE_RAISED))
        .border(
            1.0,
            if checked {
                tone.border()
            } else {
                theme::hex(theme::BORDER_SUBTLE)
            },
        )
        .center_y()
        .child(box_builder.build(kit))
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
    let mut sw = Switch::new(checked)
        .label(label)
        .standard()
        .disabled(disabled)
        .on_toggle(on_toggle);
    if let Some(desc) = description {
        sw = sw.description(desc);
    }
    row()
        .key(key)
        .w_full()
        .padding(10.0)
        .radius(8.0)
        .fill(theme::hex(theme::SURFACE_RAISED))
        .border(
            1.0,
            if checked {
                theme::hex(theme::ACCENT_DEEP)
            } else {
                theme::hex(theme::BORDER_SUBTLE)
            },
        )
        .center_y()
        .child(sw.build(kit))
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
        let mut pill = row()
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
        if current && !preferences.matte() {
            pill = pill.box_shadow(Shadow {
                blur: 10.0,
                color: theme::hex(theme::GOLD_GLOW).alpha(0.24),
                ..Shadow::default()
            });
        }
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
