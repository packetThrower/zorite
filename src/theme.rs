//! Color tokens for zorite. A runtime-switchable **palette** (light /
//! dark) held in a thread-local, so the `theme::token()` accessors stay
//! `cx`-free and every call site is unchanged when the theme switches.
//! On a switch the active palette is also overlaid onto gpui-component's
//! `Theme` so its widgets match — mirroring Baudrun's "tokens as a global,
//! overlay the component Theme" approach.

use std::cell::RefCell;

use gpui::{App, Hsla, Rgba, Window, px};
use gpui_component::{Theme, ThemeMode};

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

/// All of zorite's semantic color tokens for one appearance.
#[derive(Clone, Copy)]
pub struct Palette {
    pub bg_window: Hsla,
    pub bg_sidebar: Hsla,
    pub bg_content: Hsla,
    /// A raised card/panel surface (lighter than the window in dark, white
    /// in light) — for settings cards etc.
    pub elevated: Hsla,
    pub glass: Hsla,
    pub glass_strong: Hsla,
    pub hover: Hsla,
    pub border_subtle: Hsla,
    /// A clearly visible rule (stronger than `border_subtle`), e.g. between
    /// journal days.
    pub divider: Hsla,
    pub accent: Hsla,
    pub accent_hover: Hsla,
    pub accent_active: Hsla,
    pub accent_tint: Hsla,
    pub text_primary: Hsla,
    pub text_secondary: Hsla,
    pub text_tertiary: Hsla,
    pub tag: Hsla,
    pub code: Hsla,
}

/// Derive hover/active/tint variants from a base accent.
fn accent_variants(accent: Hsla) -> (Hsla, Hsla, Hsla) {
    let mut hover = accent;
    hover.l = (hover.l + 0.12).min(1.0);
    let mut active = accent;
    active.l = (active.l - 0.08).max(0.0);
    let mut tint = accent;
    tint.a = 0.16;
    (hover, active, tint)
}

/// Build a palette from a few base colors. Glass / hover / borders and the
/// secondary/tertiary text tints derive from the overlay color (white on
/// dark, black on light), so a skin is just ~7 colors per mode. Args:
/// `bg_window, bg_sidebar, bg_content, fg, accent, tag, code` (packed RGB).
// A flat list of base colors is clearer here than a wrapper struct.
#[allow(clippy::too_many_arguments)]
pub fn make_palette(
    bg_window: u32,
    bg_sidebar: u32,
    bg_content: u32,
    fg: u32,
    accent: u32,
    tag: u32,
    code: u32,
    is_dark: bool,
) -> Palette {
    let overlay = if is_dark { 0xFFFFFF } else { 0x000000 };
    // Raised surface: the rail color in dark (lighter than window), white
    // in light (brighter than the gray window).
    let elevated = if is_dark { bg_sidebar } else { bg_content };
    let accent = from_rgb(accent, 1.0);
    let (accent_hover, accent_active, accent_tint) = accent_variants(accent);
    Palette {
        bg_window: from_rgb(bg_window, 1.0),
        bg_sidebar: from_rgb(bg_sidebar, 1.0),
        bg_content: from_rgb(bg_content, 1.0),
        elevated: from_rgb(elevated, 1.0),
        glass: from_rgb(overlay, 0.05),
        glass_strong: from_rgb(overlay, 0.09),
        hover: from_rgb(overlay, 0.06),
        border_subtle: from_rgb(overlay, if is_dark { 0.08 } else { 0.10 }),
        divider: from_rgb(overlay, if is_dark { 0.18 } else { 0.22 }),
        accent,
        accent_hover,
        accent_active,
        accent_tint,
        text_primary: from_rgb(fg, 0.92),
        text_secondary: from_rgb(fg, 0.60),
        text_tertiary: from_rgb(fg, 0.40),
        tag: from_rgb(tag, 1.0),
        code: from_rgb(code, 1.0),
    }
}

/// The default dark palette (the "Zorite" skin) — also the thread-local seed.
pub fn dark_palette() -> Palette {
    make_palette(
        0x16171A, 0x1B1D21, 0x16171A, 0xFFFFFF, 0x0A84FF, 0x9D7CD8, 0xD7BA7D, true,
    )
}

thread_local! {
    /// The active palette. Dark until `apply` runs at startup.
    static CURRENT: RefCell<Palette> = RefCell::new(dark_palette());
}

fn get() -> Palette {
    CURRENT.with(|p| *p.borrow())
}

// --- Token accessors (read the active palette; cx-free) ---

pub fn bg_window() -> Hsla {
    get().bg_window
}
pub fn bg_sidebar() -> Hsla {
    get().bg_sidebar
}
pub fn bg_content() -> Hsla {
    get().bg_content
}
pub fn elevated() -> Hsla {
    get().elevated
}
pub fn glass() -> Hsla {
    get().glass
}
pub fn glass_strong() -> Hsla {
    get().glass_strong
}
pub fn hover() -> Hsla {
    get().hover
}
pub fn border_subtle() -> Hsla {
    get().border_subtle
}
pub fn divider() -> Hsla {
    get().divider
}
pub fn accent() -> Hsla {
    get().accent
}
pub fn accent_tint() -> Hsla {
    get().accent_tint
}
pub fn text_primary() -> Hsla {
    get().text_primary
}
pub fn text_secondary() -> Hsla {
    get().text_secondary
}
pub fn text_tertiary() -> Hsla {
    get().text_tertiary
}

/// Styling for the markdown reading view (the `gpui-markdown` crate),
/// mapped from the active palette.
/// Per-OS monospace family for code. An unknown name falls back to the default
/// font, so this is safe everywhere.
fn mono_font() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Menlo"
    }
    #[cfg(target_os = "windows")]
    {
        "Consolas"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        "DejaVu Sans Mono"
    }
}

/// Build the markdown render style from the active palette. `indent_spaces` is the
/// user's list-indent setting; the per-level pixel indent is sized to roughly match
/// that many spaces of the editor font, so reading and editing line up.
pub fn markdown_style(indent_spaces: usize) -> gpui_markdown::MarkdownStyle {
    let p = get();
    gpui_markdown::MarkdownStyle {
        text_color: p.text_primary,
        text_size: px(16.0),
        heading_color: p.text_primary,
        link_color: p.accent,
        tag_color: p.tag,
        code_color: p.code,
        code_bg: p.glass,
        muted_color: p.text_tertiary,
        rule_color: p.border_subtle,
        list_indent: px(indent_spaces as f32 * 4.5),
        mono_font: mono_font().into(),
    }
}

// --- Theme mode + application ---

/// Light / Dark / Auto (follow the OS appearance).
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    Light,
    #[default]
    Dark,
    Auto,
}

impl Mode {
    /// Stable string for persistence.
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Light => "light",
            Mode::Dark => "dark",
            Mode::Auto => "auto",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "light" => Mode::Light,
            "auto" => Mode::Auto,
            _ => Mode::Dark,
        }
    }

    /// A human label for the settings UI / quick toggle.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Light => "Light",
            Mode::Dark => "Dark",
            Mode::Auto => "Auto",
        }
    }
}

/// Resolve `mode` (+ the OS appearance, for `Auto`) to dark/light, swap
/// the active palette, push it onto gpui-component's `Theme`, and repaint.
pub fn apply(palette: Palette, is_dark: bool, window: &mut Window, cx: &mut App) {
    CURRENT.with(|c| *c.borrow_mut() = palette);
    Theme::change(
        if is_dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        Some(window),
        cx,
    );
    apply_to_component_theme(&palette, cx);
    cx.refresh_windows();
}

/// Overlay the palette onto gpui-component's `Theme` so its widgets
/// (Select, inputs, tabs, dialogs) track zorite's colors. Run after
/// `Theme::change`, which resets colors to the mode's defaults.
fn apply_to_component_theme(p: &Palette, cx: &mut App) {
    let t = Theme::global_mut(cx);
    t.background = p.bg_content;
    t.foreground = p.text_primary;
    t.primary = p.accent;
    t.primary_hover = p.accent_hover;
    t.primary_active = p.accent_active;
    t.primary_foreground = from_rgb(0xFFFFFF, 0.95);
    t.border = p.border_subtle;
    t.input = p.border_subtle;
    t.popover = p.bg_sidebar;
    t.popover_foreground = p.text_primary;
    t.accent = p.accent_tint;
    t.accent_foreground = p.text_primary;
    t.muted = p.glass;
    t.muted_foreground = p.text_tertiary;
}
