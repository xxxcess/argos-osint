//! The matte deep-space material set.
//!
//! Panels, fields, navigation and utility controls are flat tonal steps over one canvas. Only a
//! real Submit, Confirm, Create or Delete action carries the fractured-obsidian texture, and a
//! busy control may show a cool shimmer while it loads. Decorative edges never stand in for
//! functional boundaries or keyboard focus.

use std::f32::consts::PI;

use rune::el;
use rune::Color;
use rune::Easing;
use rune::El;
use rune::Live;
use rune::LoopSpec;
use rune::Shadow;
use rune::Shape;
use rune_kit::Elevation;
use rune_kit::Kit;

use crate::accessibility::ViewPreferences;
use crate::theme;
use crate::theme::StatusTone;

/// Control finishes. `GoldCta`, `Gunmetal`, `CrimsonDanger` and `ObsidianPill` are smooth matte
/// fills (the first keeps its name from the old theme and is now a cool primary). Only the two
/// fractured variants carry texture, and only real submit actions use them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MetalFinish {
    /// Primary matte action that is not a submission.
    GoldCta,
    Gunmetal,
    /// Matte destructive entry point, such as an action that opens a confirmation.
    CrimsonDanger,
    ObsidianPill,
    /// Submit, Confirm, Create or Connect: fractured obsidian, cool tint.
    Fractured,
    /// The destructive confirmation itself, such as Delete: fractured obsidian, rose tint.
    FracturedDanger,
}

impl MetalFinish {
    /// Whether this finish carries the fractured-obsidian texture.
    pub(crate) fn fractured(self) -> bool {
        matches!(self, Self::Fractured | Self::FracturedDanger)
    }
}

/// Surfaces that may blur the window's own content. Buttons and tiles never do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GlassSurface {
    Composer,
    Overlay,
}

/// Tint alpha and same-window blur for one glass surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GlassRecipe {
    pub alpha: f32,
    pub blur: f32,
}

/// `None` means opaque: reduced transparency, or the opaque preference, never blurs.
pub(crate) fn glass_recipe(
    surface: GlassSurface,
    preferences: ViewPreferences,
) -> Option<GlassRecipe> {
    if preferences.matte() {
        return None;
    }
    Some(match surface {
        GlassSurface::Composer => GlassRecipe {
            alpha: 0.84,
            blur: 10.0,
        },
        GlassSurface::Overlay => GlassRecipe {
            alpha: 0.91,
            blur: 14.0,
        },
    })
}

/// Dim obsidian glass: tint on the paint rather than subtree opacity, a narrow upper
/// reflection, and the window itself stays opaque. The opaque form keeps the same bounds.
pub(crate) fn glass(el: El, surface: GlassSurface, preferences: ViewPreferences) -> El {
    let raised = theme::hex(theme::SURFACE_RAISED);
    match glass_recipe(surface, preferences) {
        Some(recipe) => el
            .backdrop_blur(recipe.blur)
            .fill(raised.alpha(recipe.alpha))
            .border_sides([1.0, 0.0, 0.0, 0.0], theme::hex(theme::BORDER_SUBTLE)),
        None => el
            .fill(raised)
            .border_sides([1.0, 0.0, 0.0, 0.0], theme::hex(theme::BORDER_SUBTLE)),
    }
}

/// Configure `rune_kit` semantic depth tokens. Every control and card is matte: a thin
/// functional edge and no drop shadow, gloss or glow.
pub(crate) fn configure_kit(kit: &mut Kit, _preferences: ViewPreferences) {
    let functional_edge = Shadow {
        spread: 1.0,
        color: theme::hex(theme::BORDER_CONTROL),
        inset: true,
        ..Shadow::default()
    };
    let satin_edge = Shadow {
        spread: 1.0,
        color: theme::hex(theme::TEXT_PRIMARY).alpha(theme::CARD_EDGE_ALPHA),
        inset: true,
        ..Shadow::default()
    };
    let mut elevation = Elevation::default();
    elevation.button_neutral = vec![functional_edge];
    elevation.button_contrast = vec![satin_edge];
    elevation.elements = vec![functional_edge];
    kit.c.submit_highlight = Color::TRANSPARENT;
    kit.c.depth_top = Color::TRANSPARENT;
    kit.c.depth_bottom = Color::TRANSPARENT;
    kit.elevation = elevation;
}

/// A flat card: one tonal step over the canvas, a quiet edge that blends in, and a tone-tinted
/// edge only when selected.
pub(crate) fn luminous_card(
    content: El,
    tone: StatusTone,
    selected: bool,
    _preferences: ViewPreferences,
    _pointer_offset: Option<&Live<[f32; 2]>>,
    _shadow_offset: Option<&Live<[f32; 2]>>,
) -> El {
    let (fill, edge) = if selected {
        (theme::hex(theme::SURFACE_RAISED), tone.border())
    } else {
        (theme::hex(theme::SURFACE), theme::hex(theme::BORDER_SUBTLE))
    };
    content.fill(fill).border(1.0, edge).radius(10.0)
}

/// A clickable tile: the same flat card with a one-step lift on hover.
pub(crate) fn interactive_tile(
    content: El,
    selected: bool,
    tone: StatusTone,
    preferences: ViewPreferences,
    pointer_offset: Option<&Live<[f32; 2]>>,
    shadow_offset: Option<&Live<[f32; 2]>>,
) -> El {
    luminous_card(
        content,
        tone,
        selected,
        preferences,
        pointer_offset,
        shadow_offset,
    )
    .hover(theme::hex(theme::SURFACE_ELEVATED))
}

/// Fill, border and hover fill for one finish. Every finish is a flat fill.
fn finish_colors(finish: MetalFinish) -> (Color, Color, Color) {
    match finish {
        MetalFinish::GoldCta => (
            theme::hex("#172233"),
            theme::hex(theme::STATUS_INFO_BORDER),
            theme::hex("#1E2D44"),
        ),
        MetalFinish::Gunmetal | MetalFinish::ObsidianPill => (
            theme::hex(theme::SURFACE_RAISED),
            theme::hex(theme::BORDER_CONTROL),
            theme::hex(theme::SURFACE_ELEVATED),
        ),
        MetalFinish::CrimsonDanger => (
            theme::hex(theme::STATUS_ERR_BG),
            theme::hex(theme::STATUS_ERR_BORDER),
            theme::hex("#34161F"),
        ),
        MetalFinish::Fractured => (
            theme::hex("#0A0D12"),
            theme::hex(theme::STATUS_INFO_BORDER),
            theme::hex("#0F141C"),
        ),
        MetalFinish::FracturedDanger => (
            theme::hex("#150A0E"),
            theme::hex(theme::STATUS_ERR_BORDER),
            theme::hex("#1D0E14"),
        ),
    }
}

/// The fractured-obsidian texture: faceted shards alternating faint light and dark, with a few
/// cool cracks in the tint. Coordinates span a 100 x 32 box that stretches to the control, inset
/// so no facet pokes outside the rounded corners. It paints over the fill and under the label.
pub(crate) fn fracture_shape(tint: Color) -> Shape {
    let a = [3.0, 2.0];
    let b = [40.0, 2.0];
    let c = [97.0, 2.0];
    let d = [97.0, 30.0];
    let e = [60.0, 30.0];
    let f = [3.0, 30.0];
    let p = [34.0, 14.0];
    let q = [66.0, 9.0];
    let r = [80.0, 20.0];
    let s = [22.0, 22.0];
    let facets: [[[f32; 2]; 3]; 12] = [
        [a, b, p],
        [b, q, p],
        [b, c, q],
        [c, r, q],
        [c, d, r],
        [r, d, e],
        [q, r, e],
        [p, q, e],
        [p, e, f],
        [s, p, f],
        [a, p, s],
        [a, s, f],
    ];
    let mut shape = Shape::new();
    for (index, facet) in facets.iter().enumerate() {
        let color = if index % 2 == 0 {
            theme::hex(theme::TEXT_PRIMARY).alpha(0.045)
        } else {
            theme::hex(theme::CAST_SHADOW).alpha(0.24)
        };
        shape = shape.fill(facet.to_vec(), color);
    }
    for (from, to) in [(p, q), (q, r), (p, e), (q, e), (b, p), (b, q), (r, e)] {
        shape = shape.line(from, to, 0.6, rune::LineCap::Butt, tint.alpha(0.22));
    }
    shape.view_box(100.0, 32.0)
}

/// Apply a finish to a button. Everything is a flat matte fill; the fractured finishes add the
/// obsidian texture; a busy control gets a cool shimmer band unless motion is reduced.
pub(crate) fn apply_liquid_metal(
    button: El,
    finish: MetalFinish,
    disabled: bool,
    shimmer: bool,
    preferences: ViewPreferences,
    _pointer_offset: Option<&Live<[f32; 2]>>,
) -> El {
    let radius = match finish {
        MetalFinish::ObsidianPill => 14.0,
        _ => 7.0,
    };
    let (fill, border, hover_fill) = finish_colors(finish);
    let mut styled = button.fill(fill).border(1.0, border).radius(radius);
    if finish.fractured() {
        styled = styled.shape(fracture_shape(border));
    }
    if shimmer {
        if preferences.reduced_motion {
            return styled;
        }
        let sweep = el()
            .key("loading-shimmer-band")
            .absolute(0.0, 0.0)
            .w_frac(0.42)
            .h_full()
            .linear_gradient(
                PI * 0.42,
                [
                    (0.0, Color::TRANSPARENT),
                    (0.5, theme::hex(theme::TEXT_MUTED).alpha(0.16)),
                    (1.0, Color::TRANSPARENT),
                ],
            )
            .looping(LoopSpec::slide(1.5, [-1.2, 0.0], [3.2, 0.0]).easing(Easing::LINEAR));
        return styled.child(
            el().key("loading-shimmer")
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .a11y_hidden()
                .clip_rounded(radius)
                .child(sweep),
        );
    }
    if disabled {
        return styled.dim(0.55);
    }
    styled.hover(hover_fill)
}

/// Foreground ink color matching each `MetalFinish`.
pub(crate) fn finish_ink(finish: MetalFinish) -> Color {
    match finish {
        MetalFinish::FracturedDanger | MetalFinish::CrimsonDanger => theme::hex(theme::STATUS_ERR),
        _ => theme::hex(theme::TEXT_PRIMARY),
    }
}

/// Skeleton block: a steady base with one broad dim cool sheen while motion is allowed. The
/// shimmer belongs to unresolved loading only; reduced motion shows the static block.
pub(crate) fn skeleton_block(
    width_frac: Option<f32>,
    width_px: Option<f32>,
    height: f32,
    radius: f32,
    tone: StatusTone,
    preferences: ViewPreferences,
) -> El {
    let edge = if tone == StatusTone::Neutral {
        theme::hex(theme::BORDER_SUBTLE).alpha(0.55)
    } else {
        tone.border().alpha(0.45)
    };
    let mut block = el()
        .h(height)
        .radius(radius)
        .fill(theme::hex(theme::SURFACE_RAISED))
        .border(1.0, edge)
        .a11y_hidden();
    if let Some(frac) = width_frac {
        block = block.w_frac(frac);
    } else if let Some(px) = width_px {
        block = block.w(px);
    } else {
        block = block.w_full();
    }
    if preferences.reduced_motion {
        return block;
    }
    let shimmer_tint = if tone == StatusTone::Neutral {
        theme::hex(theme::TEXT_MUTED).alpha(0.10)
    } else {
        tone.fg().alpha(0.14)
    };
    let sweep = el()
        .key("skeleton-shimmer-band")
        .absolute(0.0, 0.0)
        .w_frac(0.45)
        .h_full()
        .linear_gradient(
            PI * 0.42,
            [
                (0.0, Color::TRANSPARENT),
                (0.5, shimmer_tint),
                (1.0, Color::TRANSPARENT),
            ],
        )
        .looping(LoopSpec::slide(1.5, [-1.2, 0.0], [3.2, 0.0]).easing(Easing::LINEAR));
    block.child(
        el().key("skeleton-shimmer")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .a11y_hidden()
            .clip_rounded(radius)
            .child(sweep),
    )
}

/// Toolbar controls share the Kit's matte recipe without a reflection overlay.
/// Do not wrap the control or change accessibility keys, layout or focus rings.
pub(crate) fn compact_toolbar(control: El, _preferences: ViewPreferences) -> El {
    control
}

#[cfg(test)]
mod glass_tests {
    use super::*;

    fn prefs(reduced_transparency: bool, matte_fallback: bool) -> ViewPreferences {
        ViewPreferences {
            reduced_motion: false,
            reduced_transparency,
            matte_fallback,
        }
    }

    #[test]
    fn recipes_stay_inside_the_spec_ranges() {
        let composer = glass_recipe(GlassSurface::Composer, prefs(false, false)).expect("glass");
        assert!((0.80..=0.88).contains(&composer.alpha));
        assert!((8.0..=12.0).contains(&composer.blur));
        let overlay = glass_recipe(GlassSurface::Overlay, prefs(false, false)).expect("glass");
        assert!((0.88..=0.94).contains(&overlay.alpha));
        assert!((12.0..=16.0).contains(&overlay.blur));
    }

    #[test]
    fn reduced_transparency_and_the_opaque_preference_never_blur() {
        for (reduced, matte) in [(true, false), (false, true), (true, true)] {
            assert!(glass_recipe(GlassSurface::Composer, prefs(reduced, matte)).is_none());
            assert!(glass_recipe(GlassSurface::Overlay, prefs(reduced, matte)).is_none());
        }
    }
}

#[cfg(test)]
mod finish_tests {
    use super::*;

    #[test]
    fn only_the_two_submit_finishes_carry_texture() {
        for finish in [
            MetalFinish::GoldCta,
            MetalFinish::Gunmetal,
            MetalFinish::CrimsonDanger,
            MetalFinish::ObsidianPill,
        ] {
            assert!(!finish.fractured(), "{finish:?}");
        }
        assert!(MetalFinish::Fractured.fractured());
        assert!(MetalFinish::FracturedDanger.fractured());
    }

    #[test]
    fn the_texture_has_facets_and_cracks() {
        let shape = fracture_shape(theme::hex(theme::STATUS_INFO_BORDER));
        assert!(!shape.is_empty());
    }

    #[test]
    fn no_finish_paints_an_ivory_or_gold_fill() {
        for finish in [
            MetalFinish::GoldCta,
            MetalFinish::Gunmetal,
            MetalFinish::CrimsonDanger,
            MetalFinish::ObsidianPill,
            MetalFinish::Fractured,
            MetalFinish::FracturedDanger,
        ] {
            let (fill, _, hover) = finish_colors(finish);
            for color in [fill, hover] {
                // Cool or neutral: blue is never the weakest channel.
                assert!(color.b + 0.02 >= color.r.min(color.g), "{finish:?}");
            }
        }
    }
}
