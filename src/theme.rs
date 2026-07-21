//! Color tokens for Zorite. A runtime-switchable **palette** (light /
//! dark) held in a thread-local, so the `theme::token()` accessors stay
//! `cx`-free and every call site is unchanged when the theme switches.
//! On a switch the active palette is also overlaid onto gpui-component's
//! `Theme` so its widgets match — mirroring Baudrun's "tokens as a global,
//! overlay the component Theme" approach.

use std::cell::RefCell;

use gpui::{App, Hsla, Pixels, Rgba, Window, px};
use gpui_component::{Theme, ThemeMode};

/// Opaque-or-translucent color from a packed `0xRRGGBB` literal.
pub(crate) fn from_rgb(hex: u32, alpha: f32) -> Hsla {
    Rgba {
        r: ((hex >> 16) & 0xFF) as f32 / 255.0,
        g: ((hex >> 8) & 0xFF) as f32 / 255.0,
        b: (hex & 0xFF) as f32 / 255.0,
        a: alpha,
    }
    .into()
}

/// All of Zorite's semantic color tokens for one appearance.
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
    /// GitHub-style markdown alerts (`> [!NOTE]` …) — border + title colors.
    /// Per-mode defaults from GitHub's palette (not derived from the base
    /// colors); each is overridable per theme like the other tokens.
    pub alert_note: Hsla,
    pub alert_tip: Hsla,
    pub alert_important: Hsla,
    pub alert_warning: Hsla,
    pub alert_caution: Hsla,
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
        alert_note: from_rgb(if is_dark { 0x4493F8 } else { 0x0969DA }, 1.0),
        alert_tip: from_rgb(if is_dark { 0x3FB950 } else { 0x1A7F37 }, 1.0),
        alert_important: from_rgb(if is_dark { 0xAB7DF8 } else { 0x8250DF }, 1.0),
        alert_warning: from_rgb(if is_dark { 0xD29922 } else { 0x9A6700 }, 1.0),
        alert_caution: from_rgb(if is_dark { 0xF85149 } else { 0xCF222E }, 1.0),
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
pub fn tag() -> Hsla {
    get().tag
}

/// Styling for the markdown reading view (the `gpui-markdown` crate),
/// mapped from the active palette.
/// SVG asset paths for the GitHub-alert title icons, served by the app's
/// `AssetSource` (bundled Lucide faces — see `main.rs`). One set of names for
/// both the reader and WYSIWYG, so the views can't drift.
const ALERT_ICON_NOTE: &str = "icons/info.svg";
const ALERT_ICON_TIP: &str = "icons/lightbulb.svg";
const ALERT_ICON_IMPORTANT: &str = "icons/message-square-warning.svg";
const ALERT_ICON_WARNING: &str = "icons/triangle-alert.svg";
const ALERT_ICON_CAUTION: &str = "icons/octagon-alert.svg";

thread_local! {
    /// User icon overrides for property keys (lowercased key → icon name),
    /// loaded from the `property_icons` setting at startup and edited from the
    /// Properties page. Checked before the built-in map.
    static PROPERTY_ICON_OVERRIDES: RefCell<std::collections::HashMap<String, String>> =
        RefCell::new(std::collections::HashMap::new());
}

/// Replace the property-icon overrides (parsed from the `property_icons`
/// setting's JSON map). Called at startup and after every edit.
pub(crate) fn set_property_icon_overrides(map: std::collections::HashMap<String, String>) {
    PROPERTY_ICON_OVERRIDES.with(|m| *m.borrow_mut() = map);
}

/// A copy of the current overrides (lowercased key → icon name), for the
/// Properties page to edit and persist.
pub(crate) fn property_icon_overrides() -> std::collections::HashMap<String, String> {
    PROPERTY_ICON_OVERRIDES.with(|m| m.borrow().clone())
}

/// The icon shown before a property key in the property panel: a user override
/// (set on the Properties page) when present, else a small built-in map of
/// well-known keys (case-insensitive), with a generic text-field icon as the
/// fallback so every property gets one (Obsidian-style). Paths are lucide
/// assets bundled under `assets/icons/lucide`.
pub(crate) fn property_icon(key: &str) -> Option<gpui::SharedString> {
    let lower = key.trim().to_ascii_lowercase();
    if let Some(name) = PROPERTY_ICON_OVERRIDES.with(|m| m.borrow().get(&lower).cloned()) {
        return Some(format!("icons/{name}.svg").into());
    }
    let name = builtin_property_icon(&lower);
    Some(format!("icons/{name}.svg").into())
}

/// The built-in icon name for a (lowercased) property key.
pub(crate) fn builtin_property_icon(lower: &str) -> &'static str {
    match lower {
        "alias" | "aliases" => "arrow-up-right",
        "tag" | "tags" => "tag",
        "date" | "due" | "created" | "updated" | "modified" => "calendar",
        "time" => "clock",
        "attendee" | "attendees" | "people" | "author" | "owner" | "assignee" => "user",
        "status" | "priority" => "list",
        "location" | "place" => "map-pin",
        "link" | "url" | "source" => "link",
        "project" | "projects" => "folder",
        "type" | "kind" | "category" => "shapes",
        "email" | "mail" => "mail",
        "phone" => "phone",
        "rating" | "score" | "stars" => "star",
        "price" | "cost" | "budget" => "dollar-sign",
        "deadline" => "alarm-clock",
        "done" | "complete" | "completed" => "circle-check",
        "goal" | "target" => "target",
        "topic" | "subject" => "bookmark",
        "company" | "org" | "organization" | "client" | "vendor" => "building",
        "id" | "number" | "version" => "hash",
        "description" | "summary" | "notes" => "text",
        "book" | "reading" => "book",
        "mood" => "smile",
        "weather" => "cloud",
        "repo" | "code" => "code",
        _ => "align-left", // generic (text-field look)
    }
}

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
/// that many spaces of the editor font, so reading and editing line up. `text_size`
/// is the user's note text size — the same value the editor wrappers set, so the
/// reader and the editor render at one size.
pub fn markdown_style(indent_spaces: usize, text_size: Pixels) -> gpui_markdown::MarkdownStyle {
    let p = get();
    gpui_markdown::MarkdownStyle {
        // The host injects the block-label and ref-count resolvers after
        // construction (they need AppView state); the theme stays data-only.
        block_label: None,
        block_ref_count: None,
        text_color: p.text_primary,
        text_size,
        line_height: gpui_editor::LINE_HEIGHT_RATIO,
        heading_color: p.text_primary,
        link_color: p.accent,
        tag_color: p.tag,
        code_color: p.code,
        code_bg: p.glass,
        muted_color: p.text_tertiary,
        // A note's `---` follows the divider token, like the journal-day rule;
        // the nested-list guides stay on the fainter hairline token.
        rule_color: p.divider,
        guide_color: p.border_subtle,
        // Translucent amber highlight for `<mark>`; blends over any theme's background.
        mark_bg: gpui::rgba(0xFFD60066).into(),
        // In-page find: a soft yellow on every match, a stronger orange on the
        // active one (browser-style, theme-independent so matches always pop).
        search_bg: gpui::rgba(0xFFD60055).into(),
        search_current_bg: gpui::rgba(0xFF9500DD).into(),
        list_indent: px(indent_spaces as f32 * 4.5),
        mono_font: mono_font().into(),
        alerts: gpui_markdown::AlertColors {
            note: p.alert_note,
            tip: p.alert_tip,
            important: p.alert_important,
            warning: p.alert_warning,
            caution: p.alert_caution,
        },
        alert_icons: Some(gpui_markdown::AlertIcons {
            note: ALERT_ICON_NOTE.into(),
            tip: ALERT_ICON_TIP.into(),
            important: ALERT_ICON_IMPORTANT.into(),
            warning: ALERT_ICON_WARNING.into(),
            caution: ALERT_ICON_CAUTION.into(),
        }),
        property_icon: Some(std::rc::Rc::new(property_icon)),
    }
}

/// Inline-markdown styling palette for the live-preview editor (gpui-editor).
/// Mirrors [`markdown_style`]'s colors so editing looks like the rendered view.
pub fn editor_syntax_style() -> gpui_editor::SyntaxStyle {
    let p = get();
    gpui_editor::SyntaxStyle {
        block_label: None,
        block_label_gen: 0,
        block_ref_count: None,
        marker: p.text_tertiary,
        code: p.code,
        code_bg: p.glass,
        link: p.accent,
        tag: p.tag,
        quote: p.text_tertiary,
        alert_note: p.alert_note,
        alert_tip: p.alert_tip,
        alert_important: p.alert_important,
        alert_warning: p.alert_warning,
        alert_caution: p.alert_caution,
        alert_icons: Some(gpui_editor::AlertIcons {
            note: ALERT_ICON_NOTE.into(),
            tip: ALERT_ICON_TIP.into(),
            important: ALERT_ICON_IMPORTANT.into(),
            warning: ALERT_ICON_WARNING.into(),
            caution: ALERT_ICON_CAUTION.into(),
        }),
        property_icon: Some(std::rc::Rc::new(property_icon)),
        rule: p.divider,
        mark_bg: gpui::rgba(0xFFD60066).into(),
        popover_bg: p.bg_sidebar,
        popover_border: p.border_subtle,
        popover_fg: p.text_primary,
        popover_hover: p.accent_tint,
        popover_divider: p.divider,
        // Destructive menu rows (Delete row/column/table). A Radix-red that
        // reads on both light and dark surfaces.
        popover_danger: gpui::rgb(0xE5484D).into(),
        mono: gpui::font(mono_font()),
    }
}

// --- Theme mode + application ---

/// Light / Dark / Auto (follow the OS appearance). Auto is the default —
/// a persisted "dark"/"light" choice always wins over it.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    Light,
    Dark,
    #[default]
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
            "dark" => Mode::Dark,
            _ => Mode::Auto,
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
    // The theme-reactive cursor pack re-renders from the new palette (a
    // no-op unless it's the selected pack).
    crate::cursors::theme_changed();
    cx.refresh_windows();
}

/// Override the app-wide font family on gpui-component's `Theme` (its `Root`
/// propagates it to every window, so the editors and the reader inherit it).
/// Empty = gpui-component's default. Run after [`apply`], which can reset it.
pub fn set_ui_font(family: &str, cx: &mut App) {
    Theme::global_mut(cx).font_family = if family.is_empty() {
        // gpui-component's own default (see its `Theme` impl).
        ".SystemUIFont".into()
    } else {
        family.to_string().into()
    };
}

/// Register every font file in the managed `fonts/` dir with gpui's text
/// system, so a user-added face is usable by family name like an installed
/// one. Run once at startup, before any window opens.
pub fn register_user_fonts(cx: &App) {
    let Ok(entries) = std::fs::read_dir(crate::paths::fonts_dir()) else {
        return;
    };
    let fonts: Vec<std::borrow::Cow<'static, [u8]>> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()),
                Some(e) if e.eq_ignore_ascii_case("ttf") || e.eq_ignore_ascii_case("otf")
            )
        })
        .filter_map(|p| std::fs::read(p).ok())
        .map(Into::into)
        .collect();
    if !fonts.is_empty()
        && let Err(e) = cx.text_system().add_fonts(fonts)
    {
        log::warn!("registering user fonts: {e}");
    }
}

/// Overlay the palette onto gpui-component's `Theme` so its widgets
/// (Select, inputs, tabs, dialogs) track Zorite's colors. Run after
/// `Theme::change`, which resets colors to the mode's defaults.
fn apply_to_component_theme(p: &Palette, cx: &mut App) {
    let t = Theme::global_mut(cx);
    t.background = p.bg_content;
    t.foreground = p.text_primary;
    t.primary = p.accent;
    t.primary_hover = p.accent_hover;
    t.primary_active = p.accent_active;
    // Readable label on the accent: dark text on a bright accent (e.g. CRT's
    // phosphor green), white on a dark/saturated one (the usual blue).
    t.primary_foreground = if p.accent.l > 0.6 {
        from_rgb(0x000000, 0.9)
    } else {
        from_rgb(0xFFFFFF, 0.95)
    };
    t.border = p.border_subtle;
    t.input = p.border_subtle;
    t.popover = p.bg_sidebar;
    t.popover_foreground = p.text_primary;
    t.accent = p.accent_tint;
    t.accent_foreground = p.text_primary;
    t.muted = p.glass;
    t.muted_foreground = p.text_tertiary;
    // Tabs have their own tokens (the strip renders white labels on a theme
    // whose foreground isn't near-white otherwise — e.g. green-on-black CRT).
    t.tab_foreground = p.text_secondary;
    t.tab_active_foreground = p.text_primary;
    t.secondary_foreground = p.text_secondary;
    // So do the per-widget families — without these, buttons and sliders keep
    // the stock white regardless of the `primary`/`foreground` overlay above.
    let on_accent = t.primary_foreground;
    t.button_primary = p.accent;
    t.button_primary_hover = p.accent_hover;
    t.button_primary_active = p.accent_active;
    t.button_primary_foreground = on_accent;
    t.button_foreground = p.text_primary;
    t.button_hover = p.hover;
    t.button_active = p.glass_strong;
    t.button_secondary = p.glass_strong;
    t.button_secondary_hover = p.hover;
    t.button_secondary_active = p.glass_strong;
    t.button_secondary_foreground = p.text_primary;
    t.slider_bar = p.accent;
    t.slider_thumb = p.accent;
    // Focus ring on inputs/selects — stock is a bright near-white in dark mode.
    t.ring = p.accent;
    // gpui-component is mid-migration to a parallel `tokens` color store;
    // newer widget paths (Button, Slider, some Tab styles) read
    // `theme.tokens.*` instead of the legacy fields above — without this,
    // e.g. the primary button keeps the stock white pill on every custom
    // theme. Regenerate the tokens from the overlaid colors so both stores
    // agree. Keep this LAST.
    t.tokens = (**t).into();
}
