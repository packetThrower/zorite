//! The bundled cursor theme — Bibata-Catppuccin (Mocha), installed over the
//! system cursors via the `os-cursors` crate (NSCursor swizzle on macOS,
//! `WM_SETCURSOR` hook on Windows). The assets are the theme's own XCursor
//! files stripped to their 64px frames; hotspots ride along in the format.
//!
//! Linux is not wired here: `os_cursors::use_xcursor_theme` needs the pack
//! on disk before the display connection — that lands with the cursor
//! settings UI (packs in the data dir, like fonts).

use os_cursors::{Cursor, xcursor};

/// On-screen size in points (the native macOS arrow is ≈17pt; 64px frames
/// at 20pt are Retina-crisp). Ignored on Windows (system pixel size rules).
const SIZE_PT: f32 = 20.0;

macro_rules! pack {
    ($($cursor:ident => $file:literal),* $(,)?) => {
        &[$((
            Cursor::$cursor,
            include_bytes!(concat!("../assets/cursors/Bibata-Catppuccin-Mocha/cursors/", $file)).as_slice(),
        )),*]
    };
}

const PACK: &[(Cursor, &[u8])] = pack![
    Arrow => "default",
    IBeam => "text",
    Crosshair => "crosshair",
    ClosedHand => "grabbing",
    OpenHand => "grab",
    PointingHand => "pointer",
    ResizeLeft => "w-resize",
    ResizeRight => "e-resize",
    ResizeLeftRight => "ew-resize",
    ResizeUp => "n-resize",
    ResizeDown => "s-resize",
    ResizeUpDown => "ns-resize",
    ResizeUpLeftDownRight => "nwse-resize",
    ResizeUpRightDownLeft => "nesw-resize",
    IBeamVertical => "vertical-text",
    OperationNotAllowed => "not-allowed",
    DragLink => "alias",
    DragCopy => "copy",
    ContextualMenu => "context-menu",
];

/// Install the pack. Main thread, after AppKit is up (any time before the
/// first window works). Cursors the platform can't replace (Windows aliases
/// several onto one handle) are skipped by `os_cursors::install` itself.
pub fn install() {
    for (cursor, bytes) in PACK {
        if let Some(images) = xcursor::parse(bytes) {
            os_cursors::install(*cursor, &images, SIZE_PT);
        }
    }
}
