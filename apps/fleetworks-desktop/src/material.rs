//! Opaque matte surfaces and static satin edges, shared by production views.
//!
//! Decorative edges never stand in for functional boundaries or keyboard focus.

use rune::color;
use rune::Color;
use rune::El;
use rune::Shadow;
use rune_kit::Elevation;
use rune_kit::Kit;

use crate::accessibility::ViewPreferences;
use crate::theme;

/// Keep component behavior/metrics and functional boundaries, removing broad
/// reflections and elevation in every appearance preference. Reduced motion and
/// reduced transparency therefore use the same static opaque surface recipes.
pub(crate) fn configure_kit(kit: &mut Kit, preferences: ViewPreferences) {
    let functional_edge = Shadow {
        spread: 1.0,
        color: hex(theme::BORDER_CONTROL),
        inset: true,
        ..Shadow::default()
    };
    let satin_edge = Shadow {
        spread: 1.0,
        color: hex(theme::TEXT_PRIMARY).alpha(if preferences.matte() {
            theme::CARD_EDGE_ALPHA
        } else {
            theme::SATIN_CONTROL_EDGE_ALPHA
        }),
        inset: true,
        ..Shadow::default()
    };
    let mut elevation = Elevation::default();
    elevation.button_neutral = vec![functional_edge];
    elevation.button_contrast = vec![satin_edge];
    elevation.elements = vec![functional_edge];
    kit.elevation = elevation;
    kit.c.submit_highlight = Color::TRANSPARENT;
    kit.c.depth_top = Color::TRANSPARENT;
    kit.c.depth_bottom = Color::TRANSPARENT;
}

/// Paint a read-only card without making it focusable or interactive.
pub(crate) fn matte_card(content: El) -> El {
    content
        .fill(hex(theme::SURFACE))
        .border(1.0, hex(theme::TEXT_PRIMARY).alpha(theme::CARD_EDGE_ALPHA))
        .radius(8.0)
}

/// Toolbar controls share the Kit's matte recipe without a reflection overlay.
/// Do not wrap the control or change accessibility keys, layout or focus rings.
pub(crate) fn compact_toolbar(control: El, _preferences: ViewPreferences) -> El {
    control
}

fn hex(value: &str) -> Color {
    color::hex(value).expect("static native material color")
}
