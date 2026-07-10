//! Linux backend — per-process theming through the platform's own mechanism.
//!
//! libXcursor and libwayland-cursor both resolve the theme from
//! `XCURSOR_THEME` / `XCURSOR_PATH` / `XCURSOR_SIZE` when the display
//! connection loads cursors, so setting them early themes exactly this
//! process, X11 and Wayland alike — no interception needed.

use std::path::Path;

pub(crate) fn use_xcursor_theme(themes_dir: &Path, name: &str, size_px: u32) -> bool {
    if !themes_dir.join(name).join("cursors").is_dir() {
        return false;
    }
    let mut path = themes_dir.display().to_string();
    match std::env::var("XCURSOR_PATH") {
        // Keep the inherited search path for inherits= fallbacks.
        Ok(existing) if !existing.is_empty() => {
            path.push(':');
            path.push_str(&existing);
        }
        // libXcursor's compiled-in default list, since setting XCURSOR_PATH
        // replaces it entirely.
        _ => path.push_str(":~/.local/share/icons:~/.icons:/usr/share/icons:/usr/share/pixmaps"),
    }
    // SAFETY: the documented contract of `crate::use_xcursor_theme` — called
    // before the display connection and before other threads exist.
    unsafe {
        std::env::set_var("XCURSOR_PATH", path);
        std::env::set_var("XCURSOR_THEME", name);
        std::env::set_var("XCURSOR_SIZE", size_px.to_string());
    }
    true
}
