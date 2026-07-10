//! Cursor themes — the bundled Bibata-Catppuccin (Mocha) pack plus any
//! XCursor theme directories the user drops in the notebook's `cursors/`
//! folder (like fonts), applied via the `os-cursors` crate (NSCursor swizzle
//! on macOS, `WM_SETCURSOR` hook on Windows, `XCURSOR_*` env on Linux).
//!
//! The selection lives in a `cursor-theme` sidecar file next to the database
//! (the window-bounds pattern), NOT the settings table: cursors apply at
//! launch, before an encrypted database unlocks. No sidecar = native cursors.
//!
//! Platform timing: [`apply`] is called from `main` before the gpui
//! application is built — on Linux the `XCURSOR_*` environment must be set
//! before the display connection (and env mutation needs the process still
//! single-threaded). On macOS/Windows [`apply`] also re-applies live when the
//! Settings picker changes the selection; on Linux a change takes effect on
//! the next launch.

use std::path::{Path, PathBuf};

use os_cursors::{Cursor, xcursor};

/// The pack compiled into the binary (assets/cursors/), always offered.
pub const BUNDLED: &str = "Bibata-Catppuccin-Mocha";

/// On-screen size in points (the native macOS arrow is ≈17pt; 64px frames
/// at 20pt are Retina-crisp). Ignored on Windows (system pixel size rules).
const SIZE_PT: f32 = 20.0;

/// `XCURSOR_SIZE` on Linux — the freedesktop default cursor size.
#[cfg(target_os = "linux")]
const SIZE_PX: u32 = 24;

macro_rules! pack {
    ($($cursor:ident => $file:literal),* $(,)?) => {
        &[$((
            Cursor::$cursor,
            include_bytes!(concat!("../assets/cursors/Bibata-Catppuccin-Mocha/cursors/", $file)).as_slice(),
        )),*]
    };
}

/// The bundled pack's files — names match `Cursor::freedesktop_name`.
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

fn sidecar() -> PathBuf {
    crate::paths::data_dir().join("cursor-theme")
}

/// Directory for user-added cursor themes (XCursor theme layout:
/// `cursors/<name>/cursors/*`), a managed sibling of `fonts/`.
pub fn cursors_dir() -> PathBuf {
    crate::paths::data_dir().join("cursors")
}

/// The selected theme name, `None` for native cursors.
pub fn selected() -> Option<String> {
    let name = std::fs::read_to_string(sidecar()).ok()?;
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Persist the selection and apply it live (macOS/Windows; on Linux the
/// change takes effect on the next launch — see the module doc).
pub fn set_selected(name: Option<&str>) {
    match name {
        Some(name) => {
            let _ = std::fs::write(sidecar(), name);
        }
        None => {
            let _ = std::fs::remove_file(sidecar());
        }
    }
    #[cfg(not(target_os = "linux"))]
    apply();
}

/// Theme choices: the bundled pack plus every theme directory on disk.
pub fn available() -> Vec<String> {
    let mut names = vec![BUNDLED.to_string()];
    if let Ok(entries) = std::fs::read_dir(cursors_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.join("cursors").is_dir()
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

/// Apply the sidecar's selection. Called from `main` before the gpui app is
/// built (a hard requirement on Linux), and again on live selection changes
/// (macOS/Windows).
pub fn apply() {
    let Some(name) = selected() else {
        // No selection: make sure nothing stays installed (live switch back
        // to "System"). At launch this is a no-op.
        os_cursors::reset();
        return;
    };
    #[cfg(target_os = "linux")]
    {
        if name == BUNDLED {
            materialize_bundled();
        }
        os_cursors::use_xcursor_theme(&cursors_dir(), &name, SIZE_PX);
    }
    #[cfg(not(target_os = "linux"))]
    {
        // Clear first so a pack that lacks some cursor doesn't keep the
        // previous pack's art for it.
        os_cursors::reset();
        if name == BUNDLED {
            for (cursor, bytes) in PACK {
                if let Some(images) = xcursor::parse(bytes) {
                    os_cursors::install(*cursor, &images, SIZE_PT);
                }
            }
        } else {
            let dir = cursors_dir().join(&name).join("cursors");
            for cursor in Cursor::all() {
                if let Ok(bytes) = std::fs::read(dir.join(cursor.freedesktop_name()))
                    && let Some(images) = xcursor::parse(&bytes)
                {
                    os_cursors::install(*cursor, &images, SIZE_PT);
                }
            }
        }
    }
}

/// Linux consumes themes from disk (the `XCURSOR_PATH` mechanism), so the
/// embedded pack is written out once when first selected.
#[cfg(target_os = "linux")]
fn materialize_bundled() {
    let root = cursors_dir().join(BUNDLED);
    let dir = root.join("cursors");
    if dir.is_dir() {
        return;
    }
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    for (cursor, bytes) in PACK {
        let _ = std::fs::write(dir.join(cursor.freedesktop_name()), bytes);
    }
    let _ = std::fs::write(
        root.join("index.theme"),
        format!("[Icon Theme]\nName={BUNDLED}\n"),
    );
}

/// Import an XCursor theme directory into `cursors/`: validate it, copy its
/// files (resolving symlinks — themes are full of them), and return the
/// theme's name. The error string is user-facing.
pub fn import(path: &Path) -> Result<String, String> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Pick the theme's folder.")?
        .to_string();
    let src = path.join("cursors");
    if !src.is_dir() {
        return Err("Not a cursor theme — no cursors/ folder inside.".into());
    }
    let usable = Cursor::all()
        .iter()
        .filter(|c| {
            std::fs::read(src.join(c.freedesktop_name()))
                .ok()
                .and_then(|bytes| xcursor::parse(&bytes))
                .is_some()
        })
        .count();
    if usable == 0 {
        return Err("No usable cursors found (expected XCursor files like default, text).".into());
    }
    let dst = cursors_dir().join(&name);
    if dst.exists() {
        return Err(format!("“{name}” is already installed."));
    }
    let copy = |from: &Path, to: &Path| -> std::io::Result<()> {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            // metadata() follows symlinks; copy regular files only (a theme
            // dir holds no meaningful subdirectories).
            if entry.path().metadata()?.is_file() {
                std::fs::copy(entry.path(), to.join(entry.file_name()))?;
            }
        }
        Ok(())
    };
    if let Err(e) = copy(&src, &dst.join("cursors")) {
        let _ = std::fs::remove_dir_all(&dst);
        return Err(format!("Copy failed: {e}"));
    }
    if path.join("index.theme").is_file() {
        let _ = std::fs::copy(path.join("index.theme"), dst.join("index.theme"));
    }
    Ok(name)
}
