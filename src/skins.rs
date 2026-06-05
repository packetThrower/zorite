//! Themes ("skins"). A `Skin` pairs a light + dark `Palette`; `AppView`
//! picks one by id and resolves it against the current Light/Dark/Auto
//! mode. Built-ins live here; user-supplied JSON themes are loaded on top
//! (see `load_user_skins`).

use crate::theme::{Palette, make_palette};

/// A named theme with a light and a dark variant.
pub struct Skin {
    pub id: String,
    pub name: String,
    pub light: Palette,
    pub dark: Palette,
}

/// Base colors for one mode: `(bg_window, bg_sidebar, bg_content, fg, accent, tag, code)`.
type Base = (u32, u32, u32, u32, u32, u32, u32);

impl Skin {
    fn builtin(id: &str, name: &str, dark: Base, light: Base) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            dark: make_palette(dark.0, dark.1, dark.2, dark.3, dark.4, dark.5, dark.6, true),
            light: make_palette(light.0, light.1, light.2, light.3, light.4, light.5, light.6, false),
        }
    }
}

/// The bundled skins, in display order. "zorite" is the default.
pub fn builtin_skins() -> Vec<Skin> {
    vec![
        Skin::builtin(
            "zorite",
            "Zorite",
            (0x16171A, 0x1B1D21, 0x16171A, 0xFFFFFF, 0x0A84FF, 0x9D7CD8, 0xD7BA7D),
            (0xF2F2F4, 0xEAEAEE, 0xFFFFFF, 0x1D1D1F, 0x0A84FF, 0x7A4FB5, 0xB0852A),
        ),
        Skin::builtin(
            "nord",
            "Nord",
            (0x2E3440, 0x3B4252, 0x2E3440, 0xECEFF4, 0x88C0D0, 0xB48EAD, 0xEBCB8B),
            (0xECEFF4, 0xE5E9F0, 0xFFFFFF, 0x2E3440, 0x5E81AC, 0xB48EAD, 0xA3BE8C),
        ),
        Skin::builtin(
            "solarized",
            "Solarized",
            (0x002B36, 0x073642, 0x002B36, 0x93A1A1, 0x268BD2, 0x6C71C4, 0xB58900),
            (0xFDF6E3, 0xEEE8D5, 0xFDF6E3, 0x586E75, 0x268BD2, 0x6C71C4, 0xB58900),
        ),
        Skin::builtin(
            "gruvbox",
            "Gruvbox",
            (0x282828, 0x3C3836, 0x282828, 0xEBDBB2, 0xFE8019, 0xD3869B, 0xFABD2F),
            (0xFBF1C7, 0xEBDBB2, 0xFBF1C7, 0x3C3836, 0xD65D0E, 0x8F3F71, 0xB57614),
        ),
        Skin::builtin(
            "dracula",
            "Dracula",
            (0x282A36, 0x343746, 0x282A36, 0xF8F8F2, 0xBD93F9, 0xFF79C6, 0xF1FA8C),
            (0xF5F5FA, 0xECECF5, 0xFFFFFF, 0x282A36, 0x7C3AED, 0xC026A3, 0x8A6D00),
        ),
    ]
}
