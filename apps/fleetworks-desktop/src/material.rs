//! Liquid-metal finishes, luminous brand surfaces and strict matte parity fallbacks.
//!
//! Decorative edges never stand in for functional boundaries or keyboard focus.

use std::f32::consts::PI;

use rune::el;
use rune::Color;
use rune::El;
use rune::Live;
use rune::Shadow;
use rune::Spring;
use rune_kit::Elevation;
use rune_kit::Kit;

use crate::accessibility::ViewPreferences;
use crate::theme;
use crate::theme::StatusTone;

/// Liquid-metal surface finish variants for interactive actions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MetalFinish {
    GoldCta,
    Gunmetal,
    CrimsonDanger,
    ObsidianPill,
}

/// Configure `rune_kit` semantic depth and sheen tokens while honoring `matte()` preferences.
pub(crate) fn configure_kit(kit: &mut Kit, preferences: ViewPreferences) {
    let functional_edge = Shadow {
        spread: 1.0,
        color: theme::hex(theme::BORDER_CONTROL),
        inset: true,
        ..Shadow::default()
    };
    let matte = preferences.matte();
    let satin_edge = Shadow {
        spread: 1.0,
        color: theme::hex(theme::TEXT_PRIMARY).alpha(if matte {
            theme::CARD_EDGE_ALPHA
        } else {
            theme::SATIN_CONTROL_EDGE_ALPHA
        }),
        inset: true,
        ..Shadow::default()
    };
    let mut elevation = Elevation::default();
    if matte {
        elevation.button_neutral = vec![functional_edge];
        elevation.button_contrast = vec![satin_edge];
        elevation.elements = vec![functional_edge];
        kit.c.submit_highlight = Color::TRANSPARENT;
        kit.c.depth_top = Color::TRANSPARENT;
        kit.c.depth_bottom = Color::TRANSPARENT;
    } else {
        let drop_shadow = Shadow {
            y: 4.0,
            blur: 14.0,
            color: theme::hex(theme::CAST_SHADOW).alpha(0.62),
            ..Shadow::default()
        };
        let gold_glow = Shadow {
            y: 2.0,
            blur: 16.0,
            spread: 1.0,
            color: theme::hex(theme::GOLD_GLOW).alpha(0.12),
            ..Shadow::default()
        };
        let card_rim = Shadow {
            spread: 1.0,
            color: theme::hex(theme::LIGHT_SURFACE_GLOW).alpha(0.05),
            inset: true,
            ..Shadow::default()
        };
        elevation.button_neutral = vec![functional_edge, drop_shadow];
        elevation.button_contrast = vec![satin_edge, gold_glow, drop_shadow];
        elevation.elements = vec![functional_edge];
        elevation.raised = vec![card_rim, drop_shadow];
        elevation.md = vec![card_rim, drop_shadow];
        elevation.overlay = vec![
            Shadow {
                spread: 1.0,
                color: theme::hex(theme::ACCENT).alpha(0.28),
                inset: true,
                ..Shadow::default()
            },
            Shadow {
                y: 12.0,
                blur: 32.0,
                color: theme::hex(theme::CAST_SHADOW).alpha(0.82),
                ..Shadow::default()
            },
        ];
        kit.c.submit_highlight = theme::hex(theme::ACCENT_HIGHLIGHT).alpha(0.18);
        kit.c.depth_top = theme::hex(theme::TEXT_PRIMARY).alpha(0.07);
        kit.c.depth_bottom = theme::hex(theme::CAST_SHADOW).alpha(0.28);
    }
    kit.elevation = elevation;
}

/// Decorate a card with subtle light-surface glow and pointer-reactive shadow/sheen motion.
/// Preserves exact outer bounds and accessibility structure in `matte()` mode.
pub(crate) fn luminous_card(
    content: El,
    tone: StatusTone,
    selected: bool,
    preferences: ViewPreferences,
    pointer_offset: Option<&Live<[f32; 2]>>,
    shadow_offset: Option<&Live<[f32; 2]>>,
) -> El {
    let base_fill = if selected {
        theme::hex(theme::SURFACE_RAISED)
    } else {
        theme::hex(theme::SURFACE)
    };
    if preferences.matte() {
        let border_color = if selected {
            tone.border()
        } else {
            theme::hex(theme::BORDER_SUBTLE)
        };
        return content
            .fill(base_fill)
            .border(1.0, border_color)
            .radius(10.0);
    }

    let top_rim = if selected {
        tone.fg().alpha(0.36)
    } else {
        theme::hex(theme::LIGHT_SURFACE_GLOW).alpha(0.07)
    };
    let bottom_rim = theme::hex(theme::BORDER_SUBTLE).alpha(0.48);
    let glow_color = if selected {
        tone.fg().alpha(0.11)
    } else {
        theme::hex(theme::LIGHT_SURFACE_GLOW).alpha(0.03)
    };

    let mut card = content
        .fill(base_fill)
        .border_gradient(1.0, top_rim, bottom_rim)
        .radius(10.0)
        .continuous_corners()
        .box_shadow(Shadow {
            y: 3.0,
            blur: 18.0,
            color: theme::hex(theme::CAST_SHADOW).alpha(0.58),
            ..Shadow::default()
        })
        .box_shadow(Shadow {
            blur: 22.0,
            spread: 0.5,
            color: glow_color,
            ..Shadow::default()
        });

    if let Some(pointer) = pointer_offset {
        let spot_color = if selected {
            tone.fg().alpha(0.06)
        } else {
            theme::hex(theme::LIGHT_SURFACE_GLOW).alpha(0.04)
        };
        let mut spot = el()
            .key("pointer-glow")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .a11y_hidden()
            .clip_rounded(10.0)
            .child(
                el().absolute(0.0, 0.0)
                    .w_full()
                    .h_full()
                    .radial_gradient(
                        [0.5, 0.42],
                        210.0,
                        [
                            (0.0, spot_color),
                            (0.55, spot_color.alpha(0.015)),
                            (1.0, Color::TRANSPARENT),
                        ],
                    )
                    .live_offset(pointer),
            );
        if let Some(shadow) = shadow_offset {
            spot = spot.child(
                el().key("cast-shadow")
                    .absolute(0.0, 0.0)
                    .w_full()
                    .h_full()
                    .radial_gradient(
                        [0.5, 0.66],
                        190.0,
                        [
                            (0.0, theme::hex(theme::CAST_SHADOW).alpha(0.22)),
                            (1.0, Color::TRANSPARENT),
                        ],
                    )
                    .live_offset(shadow),
            );
        }
        card = card.child(spot);
    }

    card
}

/// Apply hover elevation, liquid-metal glow and cast-shadow tracking to clickable tiles.
pub(crate) fn interactive_tile(
    content: El,
    selected: bool,
    tone: StatusTone,
    preferences: ViewPreferences,
    pointer_offset: Option<&Live<[f32; 2]>>,
    shadow_offset: Option<&Live<[f32; 2]>>,
) -> El {
    let mut tile = luminous_card(
        content,
        tone,
        selected,
        preferences,
        pointer_offset,
        shadow_offset,
    );
    if preferences.matte() {
        return tile.hover(theme::hex(theme::SURFACE_RAISED));
    }
    tile = tile
        .hover(theme::hex(theme::LIGHT_SURFACE_GLOW).alpha(0.03))
        .press(theme::hex(theme::ACCENT).alpha(0.08))
        .press_scale(0.985)
        .hover_group()
        .hover_motion(Spring::new(0.20, 0.14), Spring::new(0.28, 0.18));
    let hover_halo = el()
        .key("hover-metal-sheen")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .a11y_hidden()
        .clip_rounded(10.0)
        .group_opacity(0.0, 0.42)
        .linear_gradient(
            PI * 0.78,
            [
                (0.0, theme::hex(theme::LIGHT_SURFACE_GLOW).alpha(0.05)),
                (0.45, Color::TRANSPARENT),
                (1.0, theme::hex(theme::CAST_SHADOW).alpha(0.18)),
            ],
        );
    tile.child(hover_halo)
}

/// Apply brushed liquid-metal fill, bevel edge, hover specular sweep and tactile press physics.
pub(crate) fn apply_liquid_metal(
    mut button: El,
    finish: MetalFinish,
    disabled: bool,
    preferences: ViewPreferences,
    pointer_offset: Option<&Live<[f32; 2]>>,
) -> El {
    let matte = preferences.matte();
    let radius = match finish {
        MetalFinish::ObsidianPill => 14.0,
        _ => 7.0,
    };

    if matte {
        let (fill, border, hover_fill) = match finish {
            MetalFinish::GoldCta => (
                theme::hex(theme::ACCENT),
                theme::hex(theme::ACCENT_HOVER),
                theme::hex(theme::ACCENT_HOVER),
            ),
            MetalFinish::Gunmetal | MetalFinish::ObsidianPill => (
                theme::hex(theme::SURFACE_RAISED),
                theme::hex(theme::BORDER_CONTROL),
                theme::hex(theme::SURFACE_ELEVATED),
            ),
            MetalFinish::CrimsonDanger => (
                theme::hex(theme::STATUS_ERR_BG),
                theme::hex(theme::STATUS_ERR_BORDER),
                theme::hex(theme::STATUS_ERR_BG),
            ),
        };
        let mut styled = button.fill(fill).border(1.0, border).radius(radius);
        if !disabled {
            styled = styled.hover(hover_fill);
        }
        return if disabled { styled.dim(0.55) } else { styled };
    }

    let (stops, top_edge, bottom_edge, glow, sheen_color): (
        [(f32, Color); 4],
        Color,
        Color,
        Color,
        Color,
    ) = match finish {
        MetalFinish::GoldCta => (
            [
                (0.0, theme::hex(theme::ACCENT_HOVER)),
                (0.30, theme::hex(theme::ACCENT)),
                (0.72, theme::hex(theme::ACCENT_DEEP)),
                (1.0, theme::hex("#5E461F")),
            ],
            theme::hex("#FFF5DF").alpha(0.42),
            theme::hex("#5E451E").alpha(0.80),
            theme::hex(theme::GOLD_GLOW).alpha(0.16),
            theme::hex("#FFF9EB").alpha(0.10),
        ),
        MetalFinish::Gunmetal => (
            [
                (0.0, theme::hex(theme::METAL_DARK_TOP)),
                (0.38, theme::hex("#1B1F28")),
                (0.76, theme::hex(theme::METAL_DARK_MID)),
                (1.0, theme::hex(theme::METAL_DARK_BOTTOM)),
            ],
            theme::hex(theme::TEXT_PRIMARY).alpha(0.13),
            theme::hex(theme::METAL_EDGE_BOTTOM).alpha(0.80),
            theme::hex(theme::LIGHT_SURFACE_GLOW).alpha(0.05),
            theme::hex(theme::LIGHT_SURFACE_GLOW).alpha(0.08),
        ),
        MetalFinish::CrimsonDanger => (
            [
                (0.0, theme::hex("#3A1721")),
                (0.38, theme::hex("#2C1119")),
                (0.78, theme::hex(theme::STATUS_ERR_BG)),
                (1.0, theme::hex("#14070B")),
            ],
            theme::hex(theme::STATUS_ERR).alpha(0.28),
            theme::hex(theme::STATUS_ERR_BORDER).alpha(0.75),
            theme::hex(theme::STATUS_ERR).alpha(0.10),
            theme::hex(theme::STATUS_ERR).alpha(0.10),
        ),
        MetalFinish::ObsidianPill => (
            [
                (0.0, theme::hex("#1C2029")),
                (0.45, theme::hex(theme::SURFACE_RAISED)),
                (0.85, theme::hex(theme::METAL_DARK_BOTTOM)),
                (1.0, theme::hex(theme::BACKGROUND)),
            ],
            theme::hex(theme::BORDER_CONTROL).alpha(0.22),
            theme::hex(theme::BORDER_SUBTLE).alpha(0.75),
            theme::hex(theme::GOLD_GLOW).alpha(0.06),
            theme::hex(theme::LIGHT_SURFACE_GLOW).alpha(0.07),
        ),
    };

    button = button
        .fill(stops[2].1)
        .linear_gradient(PI * 0.92, stops)
        .border_gradient(1.0, top_edge, bottom_edge)
        .radius(radius)
        .continuous_corners()
        .box_shadow(Shadow {
            y: 4.0,
            blur: 14.0,
            color: theme::hex(theme::CAST_SHADOW).alpha(0.68),
            ..Shadow::default()
        })
        .box_shadow(Shadow {
            blur: 14.0,
            spread: 0.5,
            color: glow,
            ..Shadow::default()
        });

    if disabled {
        return button.dim(0.48);
    }

    let mut specular_gradient = el().absolute(0.0, 0.0).w_full().h_full().radial_gradient(
        [0.5, 0.2],
        140.0,
        [(0.0, sheen_color), (1.0, Color::TRANSPARENT)],
    );
    if let Some(pointer) = pointer_offset {
        specular_gradient = specular_gradient.live_offset(pointer);
    }

    let specular = el()
        .key("liquid-specular")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .a11y_hidden()
        .clip_rounded(radius)
        .group_opacity(0.02, 0.55)
        .child(specular_gradient);

    let mut styled = button
        .hover(sheen_color.alpha(0.06))
        .press(theme::hex(theme::CAST_SHADOW).alpha(0.24))
        .press_scale(0.965)
        .hover_group()
        .hover_motion(Spring::new(0.18, 0.12), Spring::new(0.28, 0.18));

    if let Some(pointer) = pointer_offset {
        // Treat the cursor as a very dim light source softly illuminating liquid-metal buttons.
        let pointer_light = el()
            .key("liquid-pointer-light")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .a11y_hidden()
            .clip_rounded(radius)
            .child(
                el().absolute(0.0, 0.0)
                    .w_full()
                    .h_full()
                    .radial_gradient(
                        [0.5, 0.35],
                        150.0,
                        [
                            (0.0, sheen_color),
                            (0.6, sheen_color.alpha(0.02)),
                            (1.0, Color::TRANSPARENT),
                        ],
                    )
                    .live_offset(pointer),
            );
        styled = styled.child(pointer_light);
    }

    styled.child(specular)
}

/// Foreground ink color matching each `MetalFinish`.
pub(crate) fn finish_ink(finish: MetalFinish) -> Color {
    match finish {
        MetalFinish::GoldCta => theme::hex(theme::ON_ACCENT),
        MetalFinish::Gunmetal | MetalFinish::ObsidianPill => theme::hex(theme::TEXT_PRIMARY),
        MetalFinish::CrimsonDanger => theme::hex(theme::STATUS_ERR),
    }
}

/// Toolbar controls share the Kit's matte recipe without a reflection overlay.
/// Do not wrap the control or change accessibility keys, layout or focus rings.
pub(crate) fn compact_toolbar(control: El, _preferences: ViewPreferences) -> El {
    control
}
