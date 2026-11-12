//! Restrained static satin on compact toolbar controls only. No content panels,
//! text, input field or dense row receives a decorative material layer.

use rune::color;
use rune::Color;
use rune::El;
use rune::Shadow;
use rune_kit::Elevation;
use rune_kit::Kit;

use crate::accessibility::ViewPreferences;
use crate::theme;

/// Keep the kit's component behavior and metrics. Opaque base fills remain;
/// reduced-transparency replaces sheen/elevation with a fine visible matte edge.
pub(crate) fn configure_kit(kit: &mut Kit, preferences: ViewPreferences) {
    if !preferences.matte() {
        return;
    }
    let edge = Shadow {
        spread: 1.0,
        color: hex(theme::BORDER_CONTROL),
        inset: true,
        ..Shadow::default()
    };
    let mut elevation = Elevation::default();
    elevation.button_neutral = vec![edge];
    elevation.button_contrast = vec![edge];
    elevation.elements = vec![edge];
    kit.elevation = elevation;
    kit.c.submit_highlight = Color::TRANSPARENT;
    kit.c.depth_top = Color::TRANSPARENT;
    kit.c.depth_bottom = Color::TRANSPARENT;
}

/// Modify the existing control's paint without wrapping it or changing its
/// accessibility role, hit bounds, child keys, focus ring or layout.
pub(crate) fn compact_toolbar(control: El, preferences: ViewPreferences) -> El {
    if preferences.matte() {
        return control;
    }
    // A translucent static reflection leaves the kit's actual hover/pressed
    // fill visible. The base stays opaque graphite; these never animate or sample
    // time. The matte path simply omits this additional reflection.
    let stops = [
        (0.0, hex("#171716").alpha(0.36)),
        (0.24, hex("#252522").alpha(0.36)),
        (0.48, hex("#30302C").alpha(0.36)),
        (0.73, hex("#20201E").alpha(0.36)),
        (1.0, hex("#151514").alpha(0.36)),
    ];
    control.linear_gradient(std::f32::consts::PI, stops)
}

fn hex(value: &str) -> Color {
    color::hex(value).expect("static native material color")
}
