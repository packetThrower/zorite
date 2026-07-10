//! Per-app custom mouse cursors, no toolkit fork required.
//!
//! UI toolkits (gpui included) ask the OS for *named* cursors — `+[NSCursor
//! arrowCursor]` on macOS, `LoadCursorW(IDC_ARROW)` on Windows, a freedesktop
//! name like `"default"` on Linux. This crate changes what those answers look
//! like for the current process only, from below the toolkit:
//!
//! - **macOS** — the `NSCursor` class methods are swizzled (supported objc
//!   runtime API) to return cursors built from your images. Call [`install`]
//!   on the main thread.
//! - **Windows** — a thread-scoped `WH_CALLWNDPROCRET` hook watches
//!   `WM_SETCURSOR`; when the toolkit sets a standard `IDC_*` cursor, it is
//!   swapped for yours. Windows collapses several logical cursors onto one
//!   handle (all horizontal resizes are `IDC_SIZEWE`, hands are `IDC_HAND`),
//!   so only the cursors in that granularity install — the rest return
//!   `false`. Call [`install`] on the UI thread.
//! - **Linux** — the platform already does per-process theming:
//!   libXcursor/libwayland-cursor read `XCURSOR_THEME` / `XCURSOR_PATH` /
//!   `XCURSOR_SIZE`. [`use_xcursor_theme`] sets them; per-image [`install`]
//!   is a no-op. Must run before the display connection (i.e. first thing in
//!   `main`). Caveat: a Wayland client using the `cursor-shape-v1` protocol
//!   delegates drawing to the compositor and cannot be themed — gpui's
//!   current backend does not, but check yours.
//!
//! The pack currency is the **XCursor theme directory** — the standard Linux
//! cursor-theme format, so every existing theme is drop-in content.
//! [`xcursor`] parses and writes the binary files, pure Rust, no deps.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;
pub mod xcursor;

use std::path::Path;

/// The named cursors an app can replace — the intersection AppKit vends and
/// gpui requests. `ResizeColumn`/`ResizeRow` don't appear: every platform
/// aliases them onto [`ResizeLeftRight`]/[`ResizeUpDown`].
///
/// [`ResizeLeftRight`]: Cursor::ResizeLeftRight
/// [`ResizeUpDown`]: Cursor::ResizeUpDown
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cursor {
    Arrow,
    IBeam,
    Crosshair,
    ClosedHand,
    OpenHand,
    PointingHand,
    ResizeLeft,
    ResizeRight,
    ResizeLeftRight,
    ResizeUp,
    ResizeDown,
    ResizeUpDown,
    ResizeUpLeftDownRight,
    ResizeUpRightDownLeft,
    IBeamVertical,
    OperationNotAllowed,
    DragLink,
    DragCopy,
    ContextualMenu,
}

impl Cursor {
    /// The freedesktop cursor-file name (the file inside a theme's `cursors/`
    /// directory) for this cursor — `"default"`, `"text"`, `"ew-resize"`, …
    pub fn freedesktop_name(self) -> &'static str {
        match self {
            Cursor::Arrow => "default",
            Cursor::IBeam => "text",
            Cursor::Crosshair => "crosshair",
            Cursor::ClosedHand => "grabbing",
            Cursor::OpenHand => "grab",
            Cursor::PointingHand => "pointer",
            Cursor::ResizeLeft => "w-resize",
            Cursor::ResizeRight => "e-resize",
            Cursor::ResizeLeftRight => "ew-resize",
            Cursor::ResizeUp => "n-resize",
            Cursor::ResizeDown => "s-resize",
            Cursor::ResizeUpDown => "ns-resize",
            Cursor::ResizeUpLeftDownRight => "nwse-resize",
            Cursor::ResizeUpRightDownLeft => "nesw-resize",
            Cursor::IBeamVertical => "vertical-text",
            Cursor::OperationNotAllowed => "not-allowed",
            Cursor::DragLink => "alias",
            Cursor::DragCopy => "copy",
            Cursor::ContextualMenu => "context-menu",
        }
    }

    /// All replaceable cursors.
    pub fn all() -> &'static [Cursor] {
        &[
            Cursor::Arrow,
            Cursor::IBeam,
            Cursor::Crosshair,
            Cursor::ClosedHand,
            Cursor::OpenHand,
            Cursor::PointingHand,
            Cursor::ResizeLeft,
            Cursor::ResizeRight,
            Cursor::ResizeLeftRight,
            Cursor::ResizeUp,
            Cursor::ResizeDown,
            Cursor::ResizeUpDown,
            Cursor::ResizeUpLeftDownRight,
            Cursor::ResizeUpRightDownLeft,
            Cursor::IBeamVertical,
            Cursor::OperationNotAllowed,
            Cursor::DragLink,
            Cursor::DragCopy,
            Cursor::ContextualMenu,
        ]
    }
}

/// One cursor image (a single size — one XCursor frame).
///
/// Pixels are premultiplied BGRA, 4 bytes per pixel, row-major — exactly the
/// XCursor on-disk pixel format (little-endian ARGB), which is also what
/// Windows alpha cursors want; macOS swaps to RGBA on install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// The nominal size this frame serves (the XCursor TOC subtype). Usually
    /// equals `width`, but themes may pad.
    pub size: u32,
    pub width: u32,
    pub height: u32,
    /// Hotspot in pixels, from the top-left.
    pub hotspot: (u32, u32),
    /// Frame delay in ms — 0 for static cursors. Preserved so animated
    /// cursors round-trip through [`xcursor::write`]; [`install`] shows the
    /// first frame only.
    pub delay: u32,
    /// Premultiplied BGRA pixels, `width * height * 4` bytes.
    pub bgra: Vec<u8>,
}

/// From a parsed set of frames, the best single frame for a target pixel
/// size: the smallest nominal size ≥ `target_px`, else the largest available.
/// (First frame wins among animation siblings of one size.)
pub fn best_image(images: &[Image], target_px: u32) -> Option<&Image> {
    images
        .iter()
        .filter(|i| i.size >= target_px)
        .min_by_key(|i| i.size)
        .or_else(|| images.iter().max_by_key(|i| i.size))
}

/// Replace `cursor` with the best-fitting frame from `images`.
///
/// `points` is the on-screen size in typographic points (macOS honors it —
/// 20pt ≈ the native arrow; a 64px frame at 20pt is Retina-crisp). Windows
/// shows cursors at the system pixel size, so the frame nearest
/// `GetSystemMetrics(SM_CXCURSOR)` is used and `points` is ignored. Linux
/// returns `false` — use [`use_xcursor_theme`].
///
/// Call on the UI thread. Returns `false` when this cursor can't be replaced
/// on this platform or the images are unusable.
pub fn install(cursor: Cursor, images: &[Image], points: f32) -> bool {
    #[cfg(target_os = "macos")]
    return macos::install(cursor, images, points);
    #[cfg(windows)]
    return windows::install(cursor, images, points);
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (cursor, images, points);
        false
    }
}

/// Remove every installed cursor; the native ones return.
pub fn reset() {
    #[cfg(target_os = "macos")]
    macos::reset();
    #[cfg(windows)]
    windows::reset();
}

/// Linux: point this process at an XCursor theme — `themes_dir/name/cursors/`
/// must exist. Sets `XCURSOR_PATH` (prepended), `XCURSOR_THEME`, and
/// `XCURSOR_SIZE`, which libXcursor and libwayland-cursor read when the UI
/// first connects — so this must run **before any windowing/display setup and
/// before other threads exist** (environment mutation is process-global).
/// No-op `false` on macOS/Windows — use [`install`].
pub fn use_xcursor_theme(themes_dir: &Path, name: &str, size_px: u32) -> bool {
    #[cfg(target_os = "linux")]
    return linux::use_xcursor_theme(themes_dir, name, size_px);
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (themes_dir, name, size_px);
        false
    }
}
