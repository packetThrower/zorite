//! Color tokens for rumin. Dark theme only, in the spirit of
//! `~/git/etch341`'s `theme.rs`: a small set of `from_rgb`-built
//! `Hsla` helpers read directly in the render tree. One accent, derived
//! hover/active/tint variants, and a layered set of near-neutral
//! backgrounds so panels read as gentle elevation rather than hard
//! boxes.

use gpui::{App, Hsla, Rgba};
use gpui_component::Theme;

/// Opaque-or-translucent color from a packed `0xRRGGBB` literal.
fn from_rgb(hex: u32, alpha: f32) -> Hsla {
    Rgba {
        r: ((hex >> 16) & 0xFF) as f32 / 255.0,
        g: ((hex >> 8) & 0xFF) as f32 / 255.0,
        b: (hex & 0xFF) as f32 / 255.0,
        a: alpha,
    }
    .into()
}

// --- Backgrounds (darkest first) ---

/// The app base — painted behind everything, including the title bar.
pub fn bg_window() -> Hsla {
    from_rgb(0x16171A, 1.0)
}

/// The sidebar panel: one step up from the base so it reads as a rail.
pub fn bg_sidebar() -> Hsla {
    from_rgb(0x1B1D21, 1.0)
}

/// The note-editing surface — same as the base so the writing area is
/// the calm center the eye lands on.
pub fn bg_content() -> Hsla {
    from_rgb(0x16171A, 1.0)
}

/// Subtle raised fill for chips, the "+ New page" row, etc.
pub fn glass() -> Hsla {
    from_rgb(0xFFFFFF, 0.05)
}
pub fn glass_strong() -> Hsla {
    from_rgb(0xFFFFFF, 0.09)
}

/// Hover wash for interactive rows.
pub fn hover() -> Hsla {
    from_rgb(0xFFFFFF, 0.06)
}

/// Hairline divider / border.
pub fn border_subtle() -> Hsla {
    from_rgb(0xFFFFFF, 0.08)
}

// --- Accent (a calm blue; change `ACCENT_HEX` to retheme) ---

pub const ACCENT_HEX: u32 = 0x0A84FF;

pub fn accent() -> Hsla {
    from_rgb(ACCENT_HEX, 1.0)
}
pub fn accent_hover() -> Hsla {
    let mut h = accent();
    h.l = (h.l + 0.12).min(1.0);
    h
}
pub fn accent_active() -> Hsla {
    let mut h = accent();
    h.l = (h.l - 0.08).max(0.0);
    h
}
/// Translucent accent for selected-row backgrounds.
pub fn accent_tint() -> Hsla {
    let mut h = accent();
    h.a = 0.16;
    h
}

// --- Text ---

pub fn text_primary() -> Hsla {
    from_rgb(0xFFFFFF, 0.92)
}
pub fn text_secondary() -> Hsla {
    from_rgb(0xFFFFFF, 0.60)
}
pub fn text_tertiary() -> Hsla {
    from_rgb(0xFFFFFF, 0.38)
}

// --- Outliner specifics ---

/// The bullet dot at the head of every block.
pub fn bullet() -> Hsla {
    from_rgb(0xFFFFFF, 0.30)
}
/// A `[[wiki-link]]` rendered in a non-focused block.
pub fn link() -> Hsla {
    accent()
}

/// Push our accent into gpui-component's embedded `Theme` so its
/// widgets (focus rings, etc.) track our accent instead of the
/// library's default blue. Call once at startup, after
/// `Theme::change(Dark)`. Mirrors etch341's `apply_accent_to_component_theme`.
pub fn apply_accent_to_component_theme(cx: &mut App) {
    let t = Theme::global_mut(cx);
    t.primary = accent();
    t.primary_hover = accent_hover();
    t.primary_active = accent_active();
    t.primary_foreground = from_rgb(0xFFFFFF, 0.95);
}
