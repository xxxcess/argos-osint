//! The Glass Canvas material set.
//!
//! Smoked glass is painted behind opaque content. Committing actions carry the fractured-obsidian
//! texture. Decorative edges are independent of functional boundaries and keyboard focus.

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

/// Control finishes. Routine controls use smooth smoked glass. Fractured finishes add texture
/// for committing actions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MetalFinish {
    /// Primary utility action.
    GoldCta,
    Gunmetal,
    /// Destructive entry point, such as an action that opens a confirmation.
    CrimsonDanger,
    ObsidianPill,
    /// Submit, Create or Connect with a cyan tint.
    Fractured,
    /// Confirm with a mint tint.
    FracturedConfirm,
    /// The destructive confirmation itself, such as Delete, with a rose tint.
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

/// Shared tint, edge and same-window blur tiers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GlassSurface {
    Chrome,
    Panel,
    Control,
    Composer,
    Overlay,
}

/// Tint alpha and same-window blur for one glass surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GlassRecipe {
    pub alpha: f32,
    pub blur: f32,
    rim_alpha: f32,
}

impl GlassSurface {
    fn base(self) -> Color {
        theme::hex(match self {
            Self::Chrome | Self::Panel => theme::SURFACE,
            Self::Control | Self::Composer | Self::Overlay => theme::SURFACE_RAISED,
        })
    }

    fn tint(self, preferences: ViewPreferences) -> Color {
        glass_recipe(self, preferences)
            .map_or(self.base(), |recipe| self.base().alpha(recipe.alpha))
    }
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
        GlassSurface::Chrome => GlassRecipe {
            alpha: 0.82,
            blur: 10.0,
            rim_alpha: 0.12,
        },
        GlassSurface::Panel => GlassRecipe {
            alpha: 0.80,
            blur: 12.0,
            rim_alpha: 0.16,
        },
        GlassSurface::Control => GlassRecipe {
            alpha: 0.82,
            blur: 0.0,
            rim_alpha: 0.14,
        },
        GlassSurface::Composer => GlassRecipe {
            alpha: 0.84,
            blur: 10.0,
            rim_alpha: 0.16,
        },
        GlassSurface::Overlay => GlassRecipe {
            alpha: 0.88,
            blur: 16.0,
            rim_alpha: 0.20,
        },
    })
}

/// Paint one glass tier without affecting content opacity or bounds.
pub(crate) fn glass(el: El, surface: GlassSurface, preferences: ViewPreferences) -> El {
    let el = el.fill(surface.tint(preferences));
    match glass_recipe(surface, preferences) {
        Some(recipe) if surface == GlassSurface::Overlay => el
            .border_gradient(
                1.0,
                rim_color(recipe.rim_alpha),
                theme::hex(theme::BORDER_SUBTLE),
            )
            .backdrop_blur(recipe.blur),
        Some(recipe) => el
            .border(1.0, theme::hex(theme::BORDER_SUBTLE))
            .box_shadow(top_rim(recipe.rim_alpha))
            .backdrop_blur(recipe.blur),
        None => el.border(1.0, theme::hex(theme::BORDER_CONTROL)),
    }
}

/// Repeated and nested panels share the glass tint over a backdrop that is already flat or blurred.
pub(crate) fn glass_panel(el: El, preferences: ViewPreferences) -> El {
    glass(el, GlassSurface::Panel, preferences).backdrop_blur(0.0)
}

/// A routine control keeps a crisp functional outline in both appearances.
pub(crate) fn glass_control(el: El, preferences: ViewPreferences) -> El {
    glass(el, GlassSurface::Control, preferences).border(1.0, theme::hex(theme::BORDER_CONTROL))
}

fn rim_color(alpha: f32) -> Color {
    theme::hex(theme::LIGHT_SURFACE_GLOW).alpha(alpha)
}

fn top_rim(alpha: f32) -> Shadow {
    Shadow {
        y: 1.0,
        color: rim_color(alpha),
        inset: true,
        ..Shadow::default()
    }
}

fn control_tint(base: Color, preferences: ViewPreferences) -> Color {
    glass_recipe(GlassSurface::Control, preferences).map_or(base, |recipe| base.alpha(recipe.alpha))
}

/// Backend choice cards keep one functional outline and use a steady bevel for selection.
pub(crate) fn choice_tile(el: El, selected: bool, preferences: ViewPreferences) -> El {
    let tile = glass_control(el.radius(10.0), preferences);
    if selected {
        tile.fill(if preferences.matte() {
            theme::hex(theme::SURFACE_ELEVATED)
        } else {
            control_tint(theme::hex(theme::SELECTION_SURFACE), preferences)
        })
    } else {
        tile
    }
}

/// Open content tiles stay on the canvas at rest and use a slate wash on hover.
pub(crate) fn canvas_tile(el: El, selected: bool, preferences: ViewPreferences) -> El {
    let tile = el.radius(8.0).hover(control_tint(
        theme::hex(theme::SURFACE_ELEVATED),
        preferences,
    ));
    if selected {
        glass_control(tile, preferences)
    } else {
        let rim = rune::el()
            .key("tile-rim")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .radius(8.0)
            .group_opacity(0.0, 1.0)
            .a11y_hidden();
        let rim = if preferences.matte() {
            rim.border(1.0, theme::hex(theme::BORDER_CONTROL))
        } else {
            rim.border_gradient(1.0, rim_color(0.18), theme::hex(theme::BORDER_SUBTLE))
        };
        tile.hover_group_focus().child(rim)
    }
}

/// Map kit fields, selects, segments and short utility buttons onto the same material tiers.
pub(crate) fn configure_kit(kit: &mut Kit, preferences: ViewPreferences) {
    let functional_edge = Shadow {
        spread: 1.0,
        color: theme::hex(theme::BORDER_CONTROL),
        inset: true,
        ..Shadow::default()
    };
    let mut control_depth = vec![functional_edge];
    let mut floating_depth = vec![functional_edge];
    if let Some(recipe) = glass_recipe(GlassSurface::Control, preferences) {
        control_depth.push(top_rim(recipe.rim_alpha));
        floating_depth = vec![
            Shadow {
                color: theme::hex(theme::BORDER_SUBTLE),
                ..functional_edge
            },
            top_rim(0.20),
        ];
    }
    let mut elevation = Elevation::default().with_input(control_depth.clone());
    elevation.button_neutral = control_depth.clone();
    elevation.button_contrast = control_depth.clone();
    elevation.elements = control_depth;
    elevation.floating = floating_depth.clone();
    elevation.overlay = floating_depth;
    let control = GlassSurface::Control.tint(preferences);
    kit.c.bg_base = control;
    kit.c.bg_button_neutral = control;
    kit.c.layer_01 = GlassSurface::Overlay.tint(preferences);
    kit.c.layer_02 = control;
    kit.c.layer_03 = control_tint(theme::hex(theme::SURFACE_ELEVATED), preferences);
    kit.c.surface_base = GlassSurface::Panel.tint(preferences);
    kit.c.surface_weak = control;
    kit.c.surface_raised = control;
    kit.c.surface_raised_base = control;
    kit.c.surface_raised_hover = kit.c.layer_03;
    kit.c.submit_highlight = Color::TRANSPARENT;
    kit.c.depth_top = Color::TRANSPARENT;
    kit.c.depth_bottom = Color::TRANSPARENT;
    kit.elevation = elevation;
}

/// Glass for grouped content in scroll views. Repeated cards do not spend separate blur slots.
pub(crate) fn luminous_card(
    content: El,
    _tone: StatusTone,
    selected: bool,
    preferences: ViewPreferences,
    _pointer_offset: Option<&Live<[f32; 2]>>,
    _shadow_offset: Option<&Live<[f32; 2]>>,
) -> El {
    let card = glass_panel(group_shape(content, false), preferences);
    if selected {
        card.border(1.0, theme::hex(theme::BORDER_CONTROL))
    } else {
        card
    }
}

/// Adjust the setup card inset without changing the glass geometry in the opaque appearance.
pub(crate) fn responsive_card(content: El, compact: bool, preferences: ViewPreferences) -> El {
    glass_panel(group_shape(content, compact), preferences)
}

fn group_shape(content: El, compact: bool) -> El {
    if compact {
        content.padding(20.0).radius(16.0)
    } else {
        content.padding(24.0).radius(20.0)
    }
}

/// A clickable tile: a compact group with a one-step lift on hover.
pub(crate) fn interactive_tile(
    content: El,
    selected: bool,
    _tone: StatusTone,
    preferences: ViewPreferences,
    _pointer_offset: Option<&Live<[f32; 2]>>,
    _shadow_offset: Option<&Live<[f32; 2]>>,
) -> El {
    let tile = group_shape(content, true);
    if selected {
        canvas_tile(tile, true, preferences)
    } else {
        glass(tile, GlassSurface::Control, preferences).hover(control_tint(
            theme::hex(theme::SURFACE_ELEVATED),
            preferences,
        ))
    }
}

/// Texture opacity at rest, hover-free; disabled controls use [`TEXTURE_OPACITY_DISABLED`].
pub(crate) const TEXTURE_OPACITY: f32 = 0.12;
pub(crate) const TEXTURE_OPACITY_DISABLED: f32 = 0.04;
/// Texture opacity while hovered. shortcut: the pressed value 0.10 is not drawn, because Rune
/// has no pressed-state style for descendants; pressed darkens the fill by 2 levels instead.
pub(crate) const TEXTURE_OPACITY_HOVER: f32 = 0.14;

/// Base fill, functional border and hover fill. The textured finishes use semantic tints.
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
            control,
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

/// Apply a glass control and its finish. Pending controls keep the same material and geometry.
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
    let (mut fill, border, mut hover_fill) = finish_colors(finish);
    if preferences.matte() || (disabled && !shimmer && finish.fractured()) {
        fill = theme::hex(theme::SURFACE_RAISED);
    }
    if preferences.matte() {
        hover_fill = theme::hex(theme::SURFACE_ELEVATED);
    }
    let fill = control_tint(fill, preferences);
    let hover_fill = control_tint(hover_fill, preferences);
    let mut styled = button.fill(fill).border(1.0, border).radius(radius);
    if let Some(recipe) = glass_recipe(GlassSurface::Control, preferences) {
        let rim = if finish.fractured() {
            0.18
        } else {
            recipe.rim_alpha
        };
        styled = styled.box_shadow(top_rim(rim));
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
        return styled;
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
    // Tint by feature context. Reserve success, warning, and error colors for resolved states.
    let tint = match tone {
        StatusTone::Accent | StatusTone::Cyan | StatusTone::Info | StatusTone::Violet => {
            Some(tone.fg())
        }
        _ => None,
    };
    let base = theme::hex(SKELETON_BASE);
    let sheen = theme::hex(SHEEN);
    let mut block = el()
        .h(height)
        .radius(radius)
        .fill(tint.map_or(base, |tint| base.mix(tint, 0.08)))
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
    let sweep = el()
        .key("skeleton-shimmer-band")
        .absolute(0.0, 0.0)
        .w_frac(SHEEN_WIDTH)
        .h_full()
        .linear_gradient(
            PI * 0.5,
            [
                (0.0, Color::TRANSPARENT),
                (
                    0.5,
                    tint.map_or(sheen, |tint| sheen.mix(tint, 0.4))
                        .alpha(SHEEN_PEAK),
                ),
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
        for (surface, alpha, blur, rim) in [
            (GlassSurface::Chrome, 0.78..=0.86, 8.0..=12.0, 0.10..=0.18),
            (GlassSurface::Panel, 0.74..=0.84, 10.0..=14.0, 0.10..=0.18),
            (GlassSurface::Control, 0.76..=0.86, 0.0..=0.0, 0.10..=0.18),
            (GlassSurface::Composer, 0.80..=0.88, 8.0..=12.0, 0.10..=0.18),
            (GlassSurface::Overlay, 0.80..=0.88, 14.0..=18.0, 0.16..=0.24),
        ] {
            let recipe = glass_recipe(surface, prefs(false, false)).expect("glass");
            assert!(alpha.contains(&recipe.alpha), "{surface:?}");
            assert!(blur.contains(&recipe.blur), "{surface:?}");
            assert!(rim.contains(&recipe.rim_alpha), "{surface:?}");
        }
    }

    #[test]
    fn reduced_transparency_and_the_opaque_preference_never_blur() {
        for (reduced, matte) in [(true, false), (false, true), (true, true)] {
            let preferences = prefs(reduced, matte);
            for surface in [
                GlassSurface::Chrome,
                GlassSurface::Panel,
                GlassSurface::Control,
                GlassSurface::Composer,
                GlassSurface::Overlay,
            ] {
                assert!(glass_recipe(surface, preferences).is_none());
                assert_eq!(surface.tint(preferences), surface.base());
                assert_eq!(surface.tint(preferences).a, 1.0);
            }
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
