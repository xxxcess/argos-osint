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
use rune_kit::Elevation;
use rune_kit::Kit;

use crate::accessibility::ViewPreferences;
use crate::rock;
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
    /// Submit, Create or Connect: matte rock texture, cyan tint.
    Fractured,
    /// Confirm: matte rock texture, mint tint.
    FracturedConfirm,
    /// The destructive confirmation itself, such as Delete: matte rock texture, rose tint.
    FracturedDanger,
}

impl MetalFinish {
    /// Whether this finish carries the fractured-obsidian texture.
    pub(crate) fn fractured(self) -> bool {
        matches!(
            self,
            Self::Fractured | Self::FracturedConfirm | Self::FracturedDanger
        )
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
            alpha: 0.88,
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
            .border_gradient(1.0, rim_color(), theme::hex(theme::BORDER_SUBTLE)),
        None => el
            .fill(raised)
            .border(1.0, theme::hex(theme::BORDER_CONTROL)),
    }
}

fn rim_color() -> Color {
    theme::hex(theme::LIGHT_SURFACE_GLOW).alpha(0.18)
}

fn top_rim(el: El) -> El {
    el.box_shadow(Shadow {
        y: 1.0,
        color: rim_color(),
        inset: true,
        ..Shadow::default()
    })
}

/// Backend choice cards keep one functional outline and use a steady bevel for selection.
pub(crate) fn choice_tile(el: El, selected: bool, preferences: ViewPreferences) -> El {
    let tile = el
        .radius(10.0)
        .fill(theme::hex(if selected {
            theme::SELECTION_SURFACE
        } else {
            theme::SURFACE
        }))
        .border(
            1.0,
            theme::hex(if selected || preferences.matte() {
                theme::BORDER_CONTROL
            } else {
                theme::BORDER_SUBTLE
            }),
        );
    if selected && !preferences.matte() {
        top_rim(tile)
    } else {
        tile
    }
}

/// Open content tiles stay on the canvas at rest and use a slate wash on hover.
pub(crate) fn canvas_tile(el: El, selected: bool, preferences: ViewPreferences) -> El {
    let tile = el
        .radius(8.0)
        .fill(theme::hex(if selected {
            theme::SURFACE_RAISED
        } else {
            theme::BACKGROUND
        }))
        .hover(theme::hex(theme::SURFACE_ELEVATED));
    if selected {
        let tile = tile.border(1.0, theme::hex(theme::BORDER_CONTROL));
        if preferences.matte() {
            tile
        } else {
            top_rim(tile)
        }
    } else {
        tile
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

/// The diffuse perimeter of one group, in logical pixels (theme.md section 2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GroupEdge {
    /// Corner radius of the group boundary.
    pub(crate) radius: f32,
    /// Width of the alpha ramp inward from the boundary.
    pub(crate) ramp: f32,
    /// Content inset from the boundary, past the ramp.
    pub(crate) inset: f32,
    /// Spread and blur of the canvas-colored inset shadow that draws the ramp.
    spread: f32,
    blur: f32,
}

impl GroupEdge {
    /// Standard groups: 20 px radius, 16 px ramp, 24 px inset.
    pub(crate) const STANDARD: GroupEdge = GroupEdge {
        radius: 20.0,
        ramp: 16.0,
        inset: 24.0,
        spread: 8.0,
        blur: 7.5,
    };
    /// Compact groups under 160 px high: 16 px radius, 12 px ramp, 20 px inset.
    pub(crate) const COMPACT: GroupEdge = GroupEdge {
        radius: 16.0,
        ramp: 12.0,
        inset: 20.0,
        spread: 6.0,
        blur: 5.6,
    };

    /// The group's coverage `depth` px inside its boundary: `t*t*(3-2t)` for `t = depth / ramp`.
    #[cfg(test)]
    fn alpha(self, depth: f32) -> f32 {
        let t = (depth / self.ramp).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}

/// A flat group whose perimeter dissolves into the canvas, with no stroke. The ramp belongs to
/// the group's own background, so children are never attenuated. A selected item keeps a crisp
/// functional boundary instead.
/// shortcut: the renderer has no alpha-mask primitive, so a Gaussian inset shadow fitted to the
/// smoothstep ramp draws it (within 0.03 alpha everywhere, under one 8-bit level on these
/// colors). Replace it with a nine-slice alpha asset if a mask primitive lands.
pub(crate) fn luminous_card(
    content: El,
    tone: StatusTone,
    selected: bool,
    _preferences: ViewPreferences,
    _pointer_offset: Option<&Live<[f32; 2]>>,
    _shadow_offset: Option<&Live<[f32; 2]>>,
) -> El {
    group_surface(content, tone, selected, GroupEdge::STANDARD)
}

/// A group that takes the compact perimeter in a compact window, so every control of a setup
/// card stays reachable at the 640 x 480 floor.
pub(crate) fn responsive_card(content: El, tone: StatusTone, selected: bool, compact: bool) -> El {
    let edge = if compact {
        GroupEdge::COMPACT
    } else {
        GroupEdge::STANDARD
    };
    group_surface(content, tone, selected, edge)
}

/// Draw `content` as a group with the perimeter `edge`, inset to clear the ramp.
fn group_surface(content: El, tone: StatusTone, selected: bool, edge: GroupEdge) -> El {
    let content = content.padding(edge.inset).radius(edge.radius);
    if selected {
        content
            .fill(theme::hex(theme::SURFACE_RAISED))
            .border(1.0, tone.border())
    } else {
        content.fill(theme::hex(theme::SURFACE)).box_shadow(Shadow {
            blur: edge.blur,
            spread: edge.spread,
            color: theme::hex(theme::BACKGROUND),
            inset: true,
            ..Shadow::default()
        })
    }
}

/// A clickable tile: a compact group with a one-step lift on hover.
pub(crate) fn interactive_tile(
    content: El,
    selected: bool,
    tone: StatusTone,
    _preferences: ViewPreferences,
    _pointer_offset: Option<&Live<[f32; 2]>>,
    _shadow_offset: Option<&Live<[f32; 2]>>,
) -> El {
    group_surface(content, tone, selected, GroupEdge::COMPACT)
        .hover(theme::hex(theme::SURFACE_ELEVATED))
}

/// Texture opacity at rest, hover-free; disabled controls use [`TEXTURE_OPACITY_DISABLED`].
pub(crate) const TEXTURE_OPACITY: f32 = 0.12;
pub(crate) const TEXTURE_OPACITY_DISABLED: f32 = 0.04;
/// Texture opacity while hovered. shortcut: the pressed value 0.10 is not drawn, because Rune
/// has no pressed-state style for descendants; pressed darkens the fill by 2 levels instead.
pub(crate) const TEXTURE_OPACITY_HOVER: f32 = 0.14;

/// Fill, border and hover fill for one finish. Hover adds 2 to each base channel. Every finish is
/// a flat fill; the textured ones sit on the theme's action bases.
fn finish_colors(finish: MetalFinish) -> (Color, Color, Color) {
    let control = theme::hex(theme::BORDER_CONTROL);
    match finish {
        MetalFinish::GoldCta => (
            theme::hex(theme::SELECTION_SURFACE),
            control,
            theme::hex("#12292F"),
        ),
        MetalFinish::Gunmetal | MetalFinish::ObsidianPill => (
            theme::hex(theme::SURFACE_RAISED),
            control,
            theme::hex("#141A22"),
        ),
        MetalFinish::CrimsonDanger => (
            theme::hex(theme::STATUS_ERR_BG),
            theme::hex(theme::STATUS_ERR_BORDER),
            theme::hex("#241C24"),
        ),
        MetalFinish::Fractured => (theme::hex("#10262C"), control, theme::hex("#12282E")),
        MetalFinish::FracturedConfirm => (theme::hex("#10281F"), control, theme::hex("#122A21")),
        MetalFinish::FracturedDanger => (theme::hex("#2B1823"), control, theme::hex("#2D1A25")),
    }
}

/// Tiles laid side by side for an action not yet measured (its first frame, or no key).
const DEFAULT_TILES: usize = 8;
/// The most tiles under one action (16 384 px). shortcut: a wider action is flat past that, and
/// one taller than a tile (256 px) is flat below it.
const MAX_TILES: usize = 64;

/// Tiles needed to cover an action `width` px wide: the 256 px tile at fixed scale, repeated
/// from the left edge until the whole control is covered, never fewer than one.
pub(crate) fn tiles_for_width(width: f32) -> usize {
    if !width.is_finite() || width <= 0.0 {
        return 1;
    }
    ((width / rock::TILE).ceil() as usize).clamp(1, MAX_TILES)
}

/// The tile count for the action keyed `key`, from its laid-out width in the last frame. The read
/// is projected to the count, so the view reruns only when the count changes. A control that is
/// unkeyed or not measured yet gets [`DEFAULT_TILES`].
pub(crate) fn texture_tiles(key: Option<&str>) -> usize {
    key.and_then(|key| rune::memo::layout_with(key, |rect| tiles_for_width(rect[2])))
        .unwrap_or(DEFAULT_TILES)
}

/// The approved crescent rock, one 256 px logical tile from [`crate::rock`] repeated along x. It
/// is anchored to the control's top-left corner at a fixed scale, so a label, busy state or
/// width change never moves it, and it is clipped to the control's rounded corners. Each tile
/// is an image whose opacity follows the control's hover group: `rest` normally, `hover` while
/// hovered. It paints over the fill and under the label.
fn rock_texture(radius: f32, rest: f32, hover: f32, tiles: usize) -> El {
    let tile = rock::pixels();
    let images = (0..tiles).map(|n| {
        el().absolute(n as f32 * rock::TILE, 0.0)
            .size(rock::TILE, rock::TILE)
            .pixels(&tile)
            .group_opacity(rest, hover)
            .a11y_hidden()
    });
    el().key("rock-texture")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .a11y_hidden()
        .clip_rounded(radius)
        .children(images)
}

/// A color shifted by `levels` 8-bit steps in encoded sRGB, the space the theme's hex values and
/// "add or subtract 2 levels" are written in. Rune keeps channels linear, where two 8-bit steps
/// are far larger at the dark end and would clip red, so the shift is made on the encoded bytes
/// and converted back. Alpha is kept exactly.
fn shade(color: Color, levels: i16) -> Color {
    let encoded = color.hex();
    let shifted = |at: usize| {
        let byte = i16::from(u8::from_str_radix(&encoded[at..at + 2], 16).unwrap_or(0));
        (byte + levels).clamp(0, 255)
    };
    theme::hex(&format!(
        "#{:02x}{:02x}{:02x}",
        shifted(1),
        shifted(3),
        shifted(5)
    ))
    .alpha(color.a)
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
    tiles: usize,
) -> El {
    let radius = match finish {
        MetalFinish::ObsidianPill => 14.0,
        finish if finish.fractured() => 10.0,
        _ => 7.0,
    };
    let (mut fill, border, hover_fill) = finish_colors(finish);
    if disabled && !shimmer && finish.fractured() {
        // A disabled textured action returns to the raised-control base with a faint texture.
        fill = theme::hex(theme::SURFACE_RAISED);
    }
    let mut styled = button.fill(fill).border(1.0, border).radius(radius);
    if finish.fractured() && !disabled && !preferences.matte() {
        styled = top_rim(styled);
    }
    if finish.fractured() {
        let (rest, hover) = if disabled && !shimmer {
            (TEXTURE_OPACITY_DISABLED, TEXTURE_OPACITY_DISABLED)
        } else {
            (TEXTURE_OPACITY, TEXTURE_OPACITY_HOVER)
        };
        styled = styled
            .hover_group()
            .child(rock_texture(radius, rest, hover, tiles));
    }
    if shimmer {
        if preferences.reduced_motion {
            return styled;
        }
        let sweep = el()
            .key("loading-shimmer-band")
            .absolute(0.0, 0.0)
            .w_frac(SHEEN_WIDTH)
            .h_full()
            .linear_gradient(
                PI * 0.5,
                [
                    (0.0, Color::TRANSPARENT),
                    (0.5, theme::hex(SHEEN).alpha(SHEEN_PEAK)),
                    (1.0, Color::TRANSPARENT),
                ],
            )
            .looping(sheen_loop());
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
    let styled = styled.hover(hover_fill);
    if finish.fractured() {
        // Pressed subtracts 2 levels from the base, as hover adds 2.
        styled.press(shade(fill, -2))
    } else {
        styled
    }
}

/// Foreground ink color matching each `MetalFinish`.
pub(crate) fn finish_ink(finish: MetalFinish) -> Color {
    match finish {
        MetalFinish::FracturedDanger | MetalFinish::CrimsonDanger => theme::hex(theme::STATUS_ERR),
        MetalFinish::FracturedConfirm => theme::hex(theme::STATUS_OK),
        MetalFinish::Fractured | MetalFinish::GoldCta => theme::hex(theme::ACCENT),
        MetalFinish::Gunmetal | MetalFinish::ObsidianPill => theme::hex(theme::TEXT_PRIMARY),
    }
}

/// Steady skeleton base, constant opacity.
pub(crate) const SKELETON_BASE: &str = "#27313B";
/// The one cool loading sheen: color, peak alpha and width as a fraction of its region.
const SHEEN: &str = "#A9C8D9";
const SHEEN_PEAK: f32 = 0.06;
const SHEEN_WIDTH: f32 = 0.20;
/// One 1.6 s cycle: 1.2 s sweep across the region, then 0.4 s outside it. Offsets are in band
/// widths: the region is five bands wide, so the sweep runs from -1 to 5 and rests until 7.
pub(crate) const SHEEN_CYCLE_SECONDS: f64 = 1.6;
fn sheen_loop() -> LoopSpec {
    LoopSpec::slide(SHEEN_CYCLE_SECONDS, [-1.0, 0.0], [7.0, 0.0]).easing(Easing::LINEAR)
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
    let mut block = el()
        .h(height)
        .radius(radius)
        .fill(theme::hex(SKELETON_BASE))
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
    // One sheen color for every tone: loading is not a status.
    let _ = tone;
    let sweep = el()
        .key("skeleton-shimmer-band")
        .absolute(0.0, 0.0)
        .w_frac(SHEEN_WIDTH)
        .h_full()
        .linear_gradient(
            PI * 0.5,
            [
                (0.0, Color::TRANSPARENT),
                (0.5, theme::hex(SHEEN).alpha(SHEEN_PEAK)),
                (1.0, Color::TRANSPARENT),
            ],
        )
        .looping(sheen_loop());
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
        assert!((0.82..=0.90).contains(&overlay.alpha));
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

    const ALL: [MetalFinish; 7] = [
        MetalFinish::GoldCta,
        MetalFinish::Gunmetal,
        MetalFinish::CrimsonDanger,
        MetalFinish::ObsidianPill,
        MetalFinish::Fractured,
        MetalFinish::FracturedConfirm,
        MetalFinish::FracturedDanger,
    ];

    /// Abramowitz and Stegun 7.1.26, absolute error under 1.5e-7.
    fn erf(x: f32) -> f32 {
        let t = 1.0 / (1.0 + 0.327_591_1 * x.abs());
        let poly = t
            * (0.254_829_6
                + t * (-0.284_496_74 + t * (1.421_413_7 + t * (-1.453_152_1 + t * 1.061_405_4))));
        (1.0 - poly * (-x * x).exp()).copysign(x)
    }

    /// The coverage the renderer draws: an inset shadow of the canvas color with the edge's
    /// spread and `blur = 2 sigma` leaves the group `Phi((depth - spread) / sigma)`.
    fn drawn_alpha(edge: GroupEdge, depth: f32) -> f32 {
        let sigma = edge.blur / 2.0;
        0.5 * (1.0 + erf((depth - edge.spread) / sigma / std::f32::consts::SQRT_2))
    }

    #[test]
    fn the_group_ramp_is_smoothstep_over_the_theme_widths() {
        for (edge, ramp, radius, inset) in [
            (GroupEdge::STANDARD, 16.0, 20.0, 24.0),
            (GroupEdge::COMPACT, 12.0, 16.0, 20.0),
        ] {
            assert_eq!((edge.ramp, edge.radius, edge.inset), (ramp, radius, inset));
            assert_eq!(edge.alpha(0.0), 0.0);
            assert_eq!(edge.alpha(ramp / 2.0), 0.5);
            assert_eq!(edge.alpha(ramp), 1.0);
            assert_eq!(edge.alpha(ramp * 3.0), 1.0);
            assert_eq!(edge.alpha(-4.0), 0.0, "outside the boundary");
            // Content never sits in the fading perimeter.
            assert!(edge.inset >= edge.ramp);
        }
    }

    #[test]
    fn the_drawn_shadow_follows_the_smoothstep_ramp_closely() {
        for edge in [GroupEdge::STANDARD, GroupEdge::COMPACT] {
            let mut worst = 0.0f32;
            let mut depth = 0.0;
            while depth <= edge.ramp * 2.0 {
                worst = worst.max((drawn_alpha(edge, depth) - edge.alpha(depth)).abs());
                depth += 0.25;
            }
            assert!(worst < 0.03, "{edge:?} deviates by {worst}");
        }
    }

    #[test]
    fn only_submit_confirm_and_delete_finishes_carry_texture() {
        for finish in ALL {
            let textured = matches!(
                finish,
                MetalFinish::Fractured
                    | MetalFinish::FracturedConfirm
                    | MetalFinish::FracturedDanger
            );
            assert_eq!(finish.fractured(), textured, "{finish:?}");
        }
    }

    #[test]
    fn pressed_subtracts_two_encoded_levels_and_keeps_alpha() {
        // The submit base #10262C becomes #0E242A, the encoded two-step change, not a two-step
        // change of Rune's linear channels (which would clip the red channel).
        let pressed = shade(theme::hex("#10262C"), -2);
        assert_eq!(pressed.hex(), "#0e242aff");
        assert_eq!(pressed, theme::hex("#0E242A"));
        // Confirm and Delete bases follow the same rule.
        assert_eq!(shade(theme::hex("#10281F"), -2).hex(), "#0e261dff");
        assert_eq!(shade(theme::hex("#2B1823"), -2).hex(), "#291621ff");
        // Alpha is preserved exactly, including a non-opaque fill.
        let faint = theme::hex("#10262C").alpha(0.4);
        let shaded = shade(faint, -2);
        assert_eq!(shaded.a, 0.4);
        assert_eq!(shaded.hex(), "#0e242a66");
        // Darkening or lightening never leaves the 8-bit range.
        assert_eq!(shade(theme::hex("#000000"), -2).hex(), "#000000ff");
        assert_eq!(shade(theme::hex("#010101"), -2).hex(), "#000000ff");
        assert_eq!(shade(theme::hex("#FFFFFF"), 2).hex(), "#ffffffff");
        assert_eq!(shade(theme::hex("#10262C"), 2).hex(), "#12282eff");
    }

    #[test]
    fn textured_hover_fills_are_the_encoded_two_step_lift_of_their_bases() {
        for finish in [
            MetalFinish::Fractured,
            MetalFinish::FracturedConfirm,
            MetalFinish::FracturedDanger,
        ] {
            let (fill, _, hover) = finish_colors(finish);
            assert_eq!(hover.hex(), shade(fill, 2).hex(), "{finish:?}");
        }
    }

    #[test]
    fn the_texture_is_faint_at_rest_and_fainter_when_disabled() {
        // The overlay's strongest light pixel is alpha 0.22; at each state opacity it adds
        // at most this many channel levels (about 50 * opacity): matte relief, not gloss.
        let levels = |opacity: f32| 0.22 * opacity * 255.0;
        assert!(
            levels(TEXTURE_OPACITY) <= 7.0,
            "{}",
            levels(TEXTURE_OPACITY)
        );
        assert!(levels(TEXTURE_OPACITY_HOVER) <= 8.0);
        assert!(levels(TEXTURE_OPACITY_DISABLED) < levels(TEXTURE_OPACITY));
    }

    #[test]
    fn tiles_cover_the_full_control_width_at_any_width() {
        for width in [
            1.0, 120.0, 256.0, 257.0, 768.0, 769.0, 1090.0, 1440.0, 3840.0,
        ] {
            let covered = tiles_for_width(width) as f32 * rock::TILE;
            assert!(covered >= width, "{width} px is covered to {covered} px");
            assert!(
                covered - width < rock::TILE,
                "{width} px gets no spare whole tile"
            );
        }
        // The old three-tile limit left the right end of a wide Connect control flat.
        assert!(tiles_for_width(1090.0) * 256 >= 1090 && tiles_for_width(1090.0) > 3);
        assert_eq!(tiles_for_width(0.0), 1);
        assert_eq!(tiles_for_width(f32::NAN), 1);
        assert_eq!(tiles_for_width(1.0e9), MAX_TILES);
        // An unmeasured control still covers a wide window.
        assert!(DEFAULT_TILES as f32 * rock::TILE >= 1440.0);
        assert_eq!(texture_tiles(None), DEFAULT_TILES);
        assert_eq!(texture_tiles(Some("never-laid-out")), DEFAULT_TILES);
    }

    #[test]
    fn textured_actions_use_the_theme_bases_and_hover_adds_two_levels() {
        let bases = [
            (MetalFinish::Fractured, "#10262C", "#12282E"),
            (MetalFinish::FracturedConfirm, "#10281F", "#122A21"),
            (MetalFinish::FracturedDanger, "#2B1823", "#2D1A25"),
        ];
        for (finish, base, hover) in bases {
            let (fill, border, hovered) = finish_colors(finish);
            assert_eq!(fill, theme::hex(base), "{finish:?}");
            assert_eq!(hovered, theme::hex(hover), "{finish:?}");
            assert_eq!(border, theme::hex("#647587"), "functional boundary");
        }
        assert_eq!(finish_ink(MetalFinish::Fractured), theme::hex("#24DCE3"));
        assert_eq!(
            finish_ink(MetalFinish::FracturedConfirm),
            theme::hex("#23DFA0")
        );
        assert_eq!(
            finish_ink(MetalFinish::FracturedDanger),
            theme::hex("#F280A0")
        );
    }

    #[test]
    fn no_finish_paints_an_ivory_or_gold_fill() {
        for finish in ALL {
            let (fill, _, hover) = finish_colors(finish);
            for color in [fill, hover] {
                // Cool or neutral: blue is never the weakest channel.
                assert!(color.b + 0.02 >= color.r.min(color.g), "{finish:?}");
            }
        }
    }

    #[test]
    fn the_loading_sheen_is_one_steady_base_and_a_1_6_second_cycle() {
        assert_eq!(SKELETON_BASE, "#27313B");
        assert_eq!(SHEEN_CYCLE_SECONDS, 1.6);
        assert_eq!(SHEEN_WIDTH, 0.20);
        assert_eq!(SHEEN_PEAK, 0.06);
        // Band widths: it enters at -1 and clears the five-band region at 5, which is 1.2 s of
        // a 1.6 s cycle when the path runs to 7.
        let sweep = (5.0 - -1.0) / (7.0 - -1.0) * SHEEN_CYCLE_SECONDS;
        assert!((sweep - 1.2).abs() < 1e-9);
    }
}
