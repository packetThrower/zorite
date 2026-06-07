//! Themes ("skins"). A `Skin` pairs a light + dark `Palette`; `AppView`
//! picks one by id and resolves it against the current Light/Dark/Auto
//! mode. Built-ins live here; user-supplied JSON themes are loaded on top
//! (see `load_user_skins`).

use serde::Deserialize;

use crate::theme::{Palette, make_palette};

/// A named theme with a light and a dark variant.
pub struct Skin {
    pub id: String,
    pub name: String,
    pub light: Palette,
    pub dark: Palette,
    /// "Always dark": ignore the Light/Dark/Auto mode and render dark (including
    /// the window chrome / titlebar). For themes that only define a dark look.
    pub dark_only: bool,
}

/// Base colors for one mode: `(bg_window, bg_sidebar, bg_content, fg, accent, tag, code)`.
type Base = (u32, u32, u32, u32, u32, u32, u32);

/// The default ("Zorite") base colors — also the fallback for any token a
/// partial user theme leaves out.
const ZORITE_DARK: Base = (
    0x16171A, 0x1B1D21, 0x16171A, 0xFFFFFF, 0x0A84FF, 0x9D7CD8, 0xD7BA7D,
);
const ZORITE_LIGHT: Base = (
    0xF2F2F4, 0xEAEAEE, 0xFFFFFF, 0x1D1D1F, 0x0A84FF, 0x7A4FB5, 0xB0852A,
);

impl Skin {
    fn builtin(id: &str, name: &str, dark: Base, light: Base) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            dark: make_palette(dark.0, dark.1, dark.2, dark.3, dark.4, dark.5, dark.6, true),
            light: make_palette(
                light.0, light.1, light.2, light.3, light.4, light.5, light.6, false,
            ),
            dark_only: false,
        }
    }

    /// A dark-only theme: both Light and Dark modes use the dark palette (built
    /// as dark so the derived overlays/borders stay correct), and the window
    /// chrome / titlebar is forced dark via `dark_only`.
    fn builtin_dark(id: &str, name: &str, dark: Base) -> Self {
        let palette = || make_palette(dark.0, dark.1, dark.2, dark.3, dark.4, dark.5, dark.6, true);
        Self {
            id: id.to_string(),
            name: name.to_string(),
            dark: palette(),
            light: palette(),
            dark_only: true,
        }
    }
}

/// The bundled skins, in display order. "zorite" is the default.
pub fn builtin_skins() -> Vec<Skin> {
    vec![
        Skin::builtin("zorite", "Zorite", ZORITE_DARK, ZORITE_LIGHT),
        Skin::builtin(
            "nord",
            "Nord",
            (
                0x2E3440, 0x3B4252, 0x2E3440, 0xECEFF4, 0x88C0D0, 0xB48EAD, 0xEBCB8B,
            ),
            (
                0xECEFF4, 0xE5E9F0, 0xFFFFFF, 0x2E3440, 0x5E81AC, 0xB48EAD, 0xA3BE8C,
            ),
        ),
        Skin::builtin(
            "solarized",
            "Solarized",
            (
                0x002B36, 0x073642, 0x002B36, 0x93A1A1, 0x268BD2, 0x6C71C4, 0xB58900,
            ),
            (
                0xFDF6E3, 0xEEE8D5, 0xFDF6E3, 0x586E75, 0x268BD2, 0x6C71C4, 0xB58900,
            ),
        ),
        // --- Ported from Baudrun ---
        Skin::builtin(
            "tokyo-night",
            "Tokyo Night",
            (
                0x1A1B26, 0x16161E, 0x1A1B26, 0xC0CAF5, 0x7AA2F7, 0xBB9AF7, 0xE0AF68,
            ),
            (
                0xE1E2E7, 0xC4C8DA, 0xE1E2E7, 0x3760BF, 0x2E7DE9, 0x9854F1, 0x8C6C3E,
            ),
        ),
        Skin::builtin(
            "foundry",
            "Foundry",
            (
                0x1C1208, 0x160D04, 0x221709, 0xFFE9CF, 0xFF9D2E, 0xFFB863, 0xFFE066,
            ),
            (
                0xFAF4EA, 0xF1E8D8, 0xFFFDF8, 0x3D2410, 0xB3560A, 0xD4691A, 0xB88600,
            ),
        ),
        // Cyberpunk/Synthwave is dark-only in Baudrun — same palette in both modes.
        Skin::builtin_dark(
            "cyberpunk",
            "Cyberpunk",
            (
                0x120522, 0x0A0317, 0x1A0D2E, 0xF0E6FF, 0xFF006E, 0x00E5FF, 0xFFE600,
            ),
        ),
        // E-Ink (paper/ink) — monochrome; its native accent is near-invisible on
        // its dark ground, so we use the sepia tone so the active tab / headings
        // stay legible.
        Skin::builtin(
            "e-ink",
            "E-Ink",
            (
                0x1A1A1A, 0x1F1F1F, 0x1A1A1A, 0xD0C9BB, 0xC0A060, 0x6A6458, 0x7A9070,
            ),
            (
                0xF4ECE0, 0xEBE3D6, 0xF8F1E5, 0x1A1A1A, 0x5A4A1A, 0x3A3A3A, 0x2A4A2A,
            ),
        ),
        Skin::builtin(
            "dracula",
            "Dracula",
            (
                0x282A36, 0x343746, 0x282A36, 0xF8F8F2, 0xBD93F9, 0xFF79C6, 0xF1FA8C,
            ),
            (
                0xF5F5FA, 0xECECF5, 0xFFFFFF, 0x282A36, 0x7C3AED, 0xC026A3, 0x8A6D00,
            ),
        ),
    ]
}

// --- User themes (JSON) ---

/// Per-mode color overrides in a user theme. Any omitted token falls back
/// to the base ("Zorite") palette, so a theme can be just a few colors.
#[derive(Default, Deserialize)]
#[serde(default)]
struct ColorSet {
    bg_window: Option<String>,
    bg_sidebar: Option<String>,
    bg_content: Option<String>,
    fg: Option<String>,
    accent: Option<String>,
    tag: Option<String>,
    code: Option<String>,
}

/// A user theme file: `{ "id", "name", "dark": {…}, "light": {…} }`. Set
/// `"dark_only": true` for an always-dark theme — the light block is ignored and
/// the window chrome stays dark regardless of the Light/Dark/Auto setting.
#[derive(Deserialize)]
struct SkinFile {
    id: String,
    name: String,
    #[serde(default)]
    dark: ColorSet,
    #[serde(default)]
    light: ColorSet,
    #[serde(default)]
    dark_only: bool,
}

/// Parse `#RRGGBB` / `#RRGGBBAA` (alpha ignored) to packed RGB.
fn parse_hex(s: &str) -> Option<u32> {
    let s = s.trim().trim_start_matches('#');
    let s = if s.len() == 8 { &s[..6] } else { s };
    if s.len() != 6 {
        return None;
    }
    u32::from_str_radix(s, 16).ok()
}

fn pick(opt: &Option<String>, base: u32) -> u32 {
    opt.as_deref().and_then(parse_hex).unwrap_or(base)
}

fn build_palette(cs: &ColorSet, base: Base, is_dark: bool) -> Palette {
    make_palette(
        pick(&cs.bg_window, base.0),
        pick(&cs.bg_sidebar, base.1),
        pick(&cs.bg_content, base.2),
        pick(&cs.fg, base.3),
        pick(&cs.accent, base.4),
        pick(&cs.tag, base.5),
        pick(&cs.code, base.6),
        is_dark,
    )
}

/// Load user themes from the themes dir (created if missing). Invalid files
/// are skipped with a warning.
pub fn load_user_skins() -> Vec<Skin> {
    let dir = crate::paths::themes_dir();
    let _ = std::fs::create_dir_all(&dir);
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        match serde_json::from_str::<SkinFile>(&text) {
            Ok(f) => {
                let dark = build_palette(&f.dark, ZORITE_DARK, true);
                // A dark-only theme renders the dark palette in both modes.
                let light = if f.dark_only {
                    build_palette(&f.dark, ZORITE_DARK, true)
                } else {
                    build_palette(&f.light, ZORITE_LIGHT, false)
                };
                out.push(Skin {
                    id: f.id,
                    name: f.name,
                    light,
                    dark,
                    dark_only: f.dark_only,
                });
            }
            Err(e) => log::warn!("theme {}: {e}", path.display()),
        }
    }
    out
}
