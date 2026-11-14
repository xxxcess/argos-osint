//! Logo-derived colors shared by the actual views and their fixtures.

use rune::color;
use rune::Color;
use rune_kit::Kit;
use rune_kit::PaletteFocus;
use rune_kit::PaletteSeed;
use rune_kit::StateColors;

pub const BACKGROUND: &str = "#060709";
pub const SURFACE: &str = "#0B0C0F";
pub const SURFACE_RAISED: &str = "#111317";
pub const SURFACE_ELEVATED: &str = "#181C26";
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
pub const ACCENT_HIGHLIGHT: &str = "#F2DFB6";
pub const ACCENT_DEEP: &str = "#8E6E3B";
pub const ON_ACCENT: &str = "#060709";
pub const SELECTION_SURFACE: &str = "#322C22";

/// Gunmetal metallic stops for neutral interactive controls.
pub const METAL_DARK_TOP: &str = "#222631";
pub const METAL_DARK_MID: &str = "#14171F";
pub const METAL_DARK_BOTTOM: &str = "#0D0F14";
pub const METAL_EDGE_TOP: &str = "#5A5448";
pub const METAL_EDGE_BOTTOM: &str = "#24211C";

/// Luminous ambient glow and deep cast shadow colors.
pub const LIGHT_SURFACE_GLOW: &str = "#F7E8C9";
pub const GOLD_GLOW: &str = "#D8BD8C";
pub const CAST_SHADOW: &str = "#020304";

/// Semantic status palette aligned with Fleetworks web tokens.
pub const STATUS_OK: &str = "#5FD39A";
pub const STATUS_OK_BG: &str = "#10241B";
pub const STATUS_OK_BORDER: &str = "#2B6E4E";
pub const STATUS_WARN: &str = "#E5B45B";
pub const STATUS_WARN_BG: &str = "#282010";
pub const STATUS_WARN_BORDER: &str = "#7A5D28";
pub const STATUS_ERR: &str = "#F06A8A";
pub const STATUS_ERR_BG: &str = "#291219";
pub const STATUS_ERR_BORDER: &str = "#7D2B3F";
pub const STATUS_INFO: &str = "#5AB8F0";
pub const STATUS_INFO_BG: &str = "#10202D";
pub const STATUS_INFO_BORDER: &str = "#2B5E80";

/// Entity & category accent hues derived from `intel-theme.ts`.
pub const ENTITY_PERSON: &str = "#C08CF5";
pub const ENTITY_ORG: &str = "#5AB8F0";
pub const ENTITY_TECH: &str = "#21D9EE";
pub const ENTITY_COMPUTE: &str = "#FFB347";
pub const ENTITY_CLUSTER: &str = "#CAD9F0";
pub const ENTITY_GRAPH: &str = "#B48EFF";

/// Static decorative edge opacity, independent of functional boundaries/focus.
pub const CARD_EDGE_ALPHA: f32 = 0.04;
pub const SATIN_CONTROL_EDGE_ALPHA: f32 = 0.07;

/// Parse a static theme hex color into a `rune::Color`.
pub(crate) fn hex(value: &str) -> Color {
    color::hex(value).expect("the static brand palette contains valid hexadecimal colors")
}

/// Visual tone for status badges, metrics and semantic highlights.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusTone {
    Neutral,
    Accent,
    Ok,
    Warn,
    Err,
    Info,
    Violet,
    Cyan,
}

impl StatusTone {
    /// Foreground ink color for text and icons.
    pub fn fg(self) -> Color {
        match self {
            Self::Neutral => hex(TEXT_SECONDARY),
            Self::Accent => hex(ACCENT_HOVER),
            Self::Ok => hex(STATUS_OK),
            Self::Warn => hex(STATUS_WARN),
            Self::Err => hex(STATUS_ERR),
            Self::Info => hex(STATUS_INFO),
            Self::Violet => hex(ENTITY_GRAPH),
            Self::Cyan => hex(ENTITY_TECH),
        }
    }

    /// Tinted dark fill backing a badge or indicator card.
    pub fn bg(self) -> Color {
        match self {
            Self::Neutral => hex(SURFACE_RAISED),
            Self::Accent => hex(SELECTION_SURFACE),
            Self::Ok => hex(STATUS_OK_BG),
            Self::Warn => hex(STATUS_WARN_BG),
            Self::Err => hex(STATUS_ERR_BG),
            Self::Info => hex(STATUS_INFO_BG),
            Self::Violet => hex(ENTITY_GRAPH).alpha(0.14),
            Self::Cyan => hex(ENTITY_TECH).alpha(0.14),
        }
    }

    /// Crisp 1px border color harmonizing with the dark surface.
    pub fn border(self) -> Color {
        match self {
            Self::Neutral => hex(BORDER_SUBTLE),
            Self::Accent => hex(ACCENT_DEEP),
            Self::Ok => hex(STATUS_OK_BORDER),
            Self::Warn => hex(STATUS_WARN_BORDER),
            Self::Err => hex(STATUS_ERR_BORDER),
            Self::Info => hex(STATUS_INFO_BORDER),
            Self::Violet => hex(ENTITY_GRAPH).alpha(0.45),
            Self::Cyan => hex(ENTITY_TECH).alpha(0.45),
        }
    }
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
    colors.border_focus = hex(ACCENT_HIGHLIGHT);
    colors.bg_deep = hex(SURFACE_INSET);
    colors.layer_01 = hex(SURFACE);
    colors.layer_02 = hex(SURFACE_RAISED);
    colors.layer_03 = hex(SURFACE_ELEVATED);
    colors.bg_button_neutral = hex(SURFACE_RAISED);
    colors.text_base = hex(TEXT_PRIMARY);
    colors.text_muted = hex(TEXT_SECONDARY);
    colors.text_faint = hex(TEXT_MUTED);
    colors.icon_base = hex(TEXT_PRIMARY);
    colors.icon_muted = hex(TEXT_SECONDARY);
    colors.icon_faint = hex(TEXT_MUTED);
    colors.border_muted = hex(BORDER_SUBTLE);
    colors.border_base = hex(BORDER_CONTROL);
    colors.border_strong = hex(ACCENT_DEEP);
    colors.border_weak = hex(BORDER_SUBTLE);
    colors.border_hover = hex(ACCENT_HOVER);
    colors.border_active = hex(ACCENT);
    colors.submit_highlight = Color::TRANSPARENT;
    colors.submit_hover = hex(ACCENT_HOVER);
    colors.contrast_pressed = hex(ACCENT_PRESSED);
    colors.submit_fg = hex(ON_ACCENT);
    colors.submit_icon = hex(ON_ACCENT);
    colors.text_weak = hex(TEXT_SECONDARY);
    colors.text_strong = hex(TEXT_PRIMARY);
    colors.surface_base = hex(SURFACE);
    colors.surface_weak = hex(SURFACE_RAISED);
    colors.surface_hover = hex(SURFACE_ELEVATED);
    colors.surface_raised = hex(SURFACE_RAISED);
    colors.surface_raised_base = hex(SURFACE_RAISED);
    colors.std_border = hex(BORDER_CONTROL);
    colors.success = StateColors {
        bg: hex(STATUS_OK_BG),
        fg: hex(STATUS_OK),
        border: hex(STATUS_OK_BORDER),
        icon: hex(STATUS_OK),
    };
    colors.warning = StateColors {
        bg: hex(STATUS_WARN_BG),
        fg: hex(STATUS_WARN),
        border: hex(STATUS_WARN_BORDER),
        icon: hex(STATUS_WARN),
    };
    colors.danger = StateColors {
        bg: hex(STATUS_ERR_BG),
        fg: hex(STATUS_ERR),
        border: hex(STATUS_ERR_BORDER),
        icon: hex(STATUS_ERR),
    };
    colors.info = StateColors {
        bg: hex(STATUS_INFO_BG),
        fg: hex(STATUS_INFO),
        border: hex(STATUS_INFO_BORDER),
        icon: hex(STATUS_INFO),
    };
    // All elevation/material layers are configured by material::configure_kit.
    kit.elevation.raised.clear();
    kit.elevation.md.clear();
    kit
}
