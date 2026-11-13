//! Logo-derived colors shared by the actual views and their fixtures.

use rune::color;
use rune::Color;
use rune_kit::Kit;
use rune_kit::PaletteFocus;
use rune_kit::PaletteSeed;

pub const BACKGROUND: &str = "#060709";
pub const SURFACE: &str = "#0B0C0F";
pub const SURFACE_RAISED: &str = "#111317";
pub const SURFACE_INSET: &str = "#060709";
/// Decorative framing; essential controls use BORDER_CONTROL.
pub const BORDER_SUBTLE: &str = "#3A3833";
pub const BORDER_CONTROL: &str = "#898274";
pub const TEXT_PRIMARY: &str = "#F3EEE4";
pub const TEXT_SECONDARY: &str = "#D6D2C4";
pub const TEXT_MUTED: &str = "#B8B4A8";
pub const ACCENT: &str = "#C6A56E";
pub const ACCENT_PRESSED: &str = "#C4A265";
pub const ACCENT_HOVER: &str = "#D8BD8C";
pub const ON_ACCENT: &str = "#060709";
pub const SELECTION_SURFACE: &str = "#322C22";
/// Static decorative edge opacity, independent of functional boundaries/focus.
pub const CARD_EDGE_ALPHA: f32 = 0.02;
pub const SATIN_CONTROL_EDGE_ALPHA: f32 = 0.03;

fn hex(value: &str) -> Color {
    color::hex(value).expect("the static brand palette contains valid hexadecimal colors")
}

/// Construct the native palette without a theme download or web build.
pub fn kit() -> Kit {
    let mut kit = Kit::from_palette_focus(
        PaletteSeed {
            bg: hex(BACKGROUND),
            fg: hex(TEXT_PRIMARY),
            accent: hex(ACCENT),
        },
        PaletteFocus::Accent,
    );
    let colors = &mut kit.c;
    colors.page = hex(BACKGROUND);
    colors.bg_base = hex(BACKGROUND);
    colors.bg_accent = hex(ACCENT);
    colors.bg_icon_button_contrast = hex(ACCENT);
    colors.border_focus = hex(ACCENT_HOVER);
    colors.bg_deep = hex(SURFACE_INSET);
    colors.layer_01 = hex(SURFACE);
    colors.layer_02 = hex(SURFACE_RAISED);
    colors.bg_button_neutral = hex(SURFACE_RAISED);
    colors.text_muted = hex(TEXT_SECONDARY);
    colors.text_faint = hex(TEXT_MUTED);
    colors.icon_muted = hex(TEXT_SECONDARY);
    colors.icon_faint = hex(TEXT_MUTED);
    colors.border_muted = hex(BORDER_SUBTLE);
    colors.border_base = hex(BORDER_CONTROL);
    colors.border_strong = hex(BORDER_CONTROL);
    colors.border_weak = hex(BORDER_SUBTLE);
    colors.border_hover = hex(ACCENT_HOVER);
    colors.border_active = hex(ACCENT_PRESSED);
    colors.submit_highlight = Color::TRANSPARENT;
    colors.submit_hover = hex(ACCENT_HOVER);
    colors.contrast_pressed = hex(ACCENT_PRESSED);
    colors.submit_fg = hex(ON_ACCENT);
    colors.submit_icon = hex(ON_ACCENT);
    colors.text_weak = hex(TEXT_SECONDARY);
    colors.text_strong = hex(TEXT_PRIMARY);
    colors.surface_base = hex(SURFACE);
    colors.surface_weak = hex(SURFACE);
    colors.surface_raised = hex(SURFACE_RAISED);
    colors.surface_raised_base = hex(SURFACE_RAISED);
    colors.std_border = hex(BORDER_CONTROL);
    // Keep the kit's distinct success/warning/danger/info roles. Gold is branding.
    // All elevation/material layers are configured by material::configure_kit.
    kit.elevation.raised.clear();
    kit.elevation.md.clear();
    kit
}
