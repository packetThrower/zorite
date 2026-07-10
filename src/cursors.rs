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

use os_cursors::{Cursor, Image, xcursor};

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

/// The theme-reactive pack's sidecar value: Bibata recolored live from the
/// active palette (body ← accent, outline ← primary text). `@` so it can
/// never collide with a theme directory name.
pub const THEME_PACK: &str = "@theme";

/// Sidecar prefix for a *user* pack rendered theme-reactively:
/// `@theme:<pack name>` (the pack needs an `svg/` folder — see
/// [`reactive_available`]).
pub const THEME_PREFIX: &str = "@theme:";

/// Where the theme-reactive pack is written for Linux (the env mechanism
/// consumes themes from disk). Dot-prefixed: hidden from [`available`].
#[cfg(target_os = "linux")]
const THEME_DIR: &str = ".theme-cursors";

/// Sizes rasterized from the SVGs — covers Linux/Windows pixel-size picking
/// and macOS point scaling (which prefers the 64px frame).
const SVG_SIZES: &[u32] = &[24, 32, 48, 64];

macro_rules! svg_pack {
    ($($cursor:ident => $file:literal),* $(,)?) => {
        &[$((
            Cursor::$cursor,
            include_str!(concat!("../assets/cursors-svg/", $file, ".svg")),
        )),*]
    };
}

/// Bibata's SVG sources (256×256, from ful1e5/Bibata_Cursor `svg/modern`,
/// renamed to freedesktop names) with the upstream color slots intact:
/// `#00FF00` = body, `#0000FF` = outline, `#FF0000` = watch accent.
const SVGS: &[(Cursor, &str)] = svg_pack![
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
    // Linux applies on the next launch, but the theme-reactive pack's files
    // must exist on disk by then — generate them now.
    #[cfg(target_os = "linux")]
    if name.is_some_and(|n| n.starts_with(THEME_PACK)) {
        generate_theme_pack();
    }
}

/// Theme choices: the bundled pack plus every theme directory on disk.
pub fn available() -> Vec<String> {
    let mut names = vec![BUNDLED.to_string()];
    if let Ok(entries) = std::fs::read_dir(cursors_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.join("cursors").is_dir()
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
                && !name.starts_with('.')
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
    if name.starts_with(THEME_PACK) {
        // Rasterized from the palette active right now; theme::apply calls
        // theme_changed() on every skin/mode switch (including the one at
        // window open), which re-renders with the final colors.
        #[cfg(target_os = "linux")]
        os_cursors::use_xcursor_theme(&cursors_dir(), THEME_DIR, SIZE_PX);
        #[cfg(not(target_os = "linux"))]
        {
            os_cursors::reset();
            generate_theme_pack();
        }
        return;
    }
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

/// Re-render the theme-reactive pack from the active palette. A no-op unless
/// a theme-reactive selection (bundled or a user SVG pack) is current. Called
/// by `theme::apply` on every skin/mode change (macOS/Windows install live;
/// Linux rewrites the on-disk pack for the next launch).
pub fn theme_changed() {
    if selected().is_some_and(|s| s.starts_with(THEME_PACK)) {
        generate_theme_pack();
    }
}

/// User packs that can render theme-reactively: an `svg/` folder with at
/// least one recognized cursor SVG, plus a hotspot source (raster `cursors/`
/// twins or a `hotspots.json`).
pub fn reactive_available() -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(cursors_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            let svg = path.join("svg");
            if !svg.is_dir()
                || !(path.join("cursors").is_dir() || path.join("hotspots.json").is_file())
            {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str())
                && !name.starts_with('.')
                && Cursor::all()
                    .iter()
                    .any(|c| svg.join(format!("{}.svg", c.freedesktop_name())).is_file())
            {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    names
}

fn hex(color: gpui::Hsla) -> String {
    let rgba: gpui::Rgba = color.into();
    format!(
        "#{:02X}{:02X}{:02X}",
        (rgba.r * 255.0).round() as u8,
        (rgba.g * 255.0).round() as u8,
        (rgba.b * 255.0).round() as u8
    )
}

/// Hotspot normalized to 64px space from a raster XCursor file's frames —
/// the pack's own bitmaps are authoritative for where its SVGs point.
fn raster_hotspot(images: &[Image]) -> Option<(f32, f32)> {
    let img = os_cursors::best_image(images, 64)?;
    let side = img.size.max(1) as f32;
    Some((
        img.hotspot.0 as f32 * 64.0 / side,
        img.hotspot.1 as f32 * 64.0 / side,
    ))
}

/// Recolor one slot-colored SVG with the palette and rasterize it at
/// [`SVG_SIZES`]. `hotspot64` is in 64px space.
fn rasterize(svg: &str, hotspot64: (f32, f32)) -> Option<Vec<Image>> {
    let opts = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_str(svg, &opts).ok()?;
    let side = tree.size().width().max(1.0);
    let mut frames = Vec::new();
    for &size in SVG_SIZES {
        let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)?;
        let scale = size as f32 / side;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        // tiny-skia's premultiplied RGBA → the crate's premultiplied BGRA.
        let mut bgra = pixmap.take();
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        frames.push(Image {
            size,
            width: size,
            height: size,
            hotspot: (
                (hotspot64.0 * size as f32 / 64.0).round() as u32,
                (hotspot64.1 * size as f32 / 64.0).round() as u32,
            ),
            delay: 0,
            bgra,
        });
    }
    Some(frames)
}

/// The SVG source + hotspot (64px space) for each cursor of the current
/// theme-reactive selection: the bundled Bibata tables, or a user pack's
/// `svg/` files with hotspots from its raster twins / `hotspots.json`
/// (values in 64px space).
fn theme_sources(selection: &str) -> Vec<(Cursor, String, (f32, f32))> {
    if selection == THEME_PACK {
        return SVGS
            .iter()
            .filter_map(|(cursor, svg)| {
                let hotspot = PACK
                    .iter()
                    .find(|(c, _)| *c == *cursor)
                    .and_then(|(_, bytes)| raster_hotspot(&xcursor::parse(bytes)?))?;
                Some((*cursor, (*svg).to_string(), hotspot))
            })
            .collect();
    }
    let Some(name) = selection.strip_prefix(THEME_PREFIX) else {
        return Vec::new();
    };
    let dir = cursors_dir().join(name);
    let manifest: std::collections::HashMap<String, [f32; 2]> =
        std::fs::read_to_string(dir.join("hotspots.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
    Cursor::all()
        .iter()
        .filter_map(|cursor| {
            let fd = cursor.freedesktop_name();
            let svg = std::fs::read_to_string(dir.join("svg").join(format!("{fd}.svg"))).ok()?;
            let hotspot = manifest
                .get(fd)
                .map(|[x, y]| (*x, *y))
                .or_else(|| {
                    let bytes = std::fs::read(dir.join("cursors").join(fd)).ok()?;
                    raster_hotspot(&xcursor::parse(&bytes)?)
                })
                .unwrap_or((0.0, 0.0));
            Some((*cursor, svg, hotspot))
        })
        .collect()
}

/// Render the current theme-reactive pack: body ← accent, outline ← primary
/// text (light on dark themes, dark on light — always contrasts the body),
/// then install it (macOS/Windows) or write it to disk for the env mechanism
/// (Linux).
fn generate_theme_pack() {
    let Some(selection) = selected() else {
        return;
    };
    let body = hex(crate::theme::accent());
    let outline = hex(crate::theme::text_primary());
    #[cfg(target_os = "linux")]
    let root = cursors_dir().join(THEME_DIR);
    #[cfg(target_os = "linux")]
    if std::fs::create_dir_all(root.join("cursors")).is_err() {
        return;
    }
    for (cursor, svg, hotspot) in theme_sources(&selection) {
        let svg = svg
            .replace("#00FF00", &body)
            .replace("#0000FF", &outline)
            .replace("#FF0000", &body);
        let Some(frames) = rasterize(&svg, hotspot) else {
            continue;
        };
        #[cfg(target_os = "linux")]
        {
            let _ = std::fs::write(
                root.join("cursors").join(cursor.freedesktop_name()),
                xcursor::write(&frames),
            );
        }
        #[cfg(not(target_os = "linux"))]
        os_cursors::install(cursor, &frames, SIZE_PT);
    }
    #[cfg(target_os = "linux")]
    let _ = std::fs::write(
        root.join("index.theme"),
        "[Icon Theme]\nName=Zorite theme cursors\n",
    );
}

/// Import a cursor theme directory into `cursors/`: validate it, copy its
/// files (resolving symlinks — themes are full of them), and return the
/// theme's name. Accepts raster XCursor themes (`cursors/`), theme-reactive
/// SVG packs (`svg/` in the Bibata slot convention + a hotspot source), or
/// both. The error string is user-facing.
pub fn import(path: &Path) -> Result<String, String> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Pick the theme's folder.")?
        .to_string();
    let raster_src = path.join("cursors");
    let svg_src = path.join("svg");
    let raster = Cursor::all()
        .iter()
        .filter(|c| {
            std::fs::read(raster_src.join(c.freedesktop_name()))
                .ok()
                .and_then(|bytes| xcursor::parse(&bytes))
                .is_some()
        })
        .count();
    let svgs = Cursor::all()
        .iter()
        .filter(|c| {
            svg_src
                .join(format!("{}.svg", c.freedesktop_name()))
                .is_file()
        })
        .count();
    // SVGs need a hotspot source: raster twins or a hotspots.json.
    let svg_ok = svgs > 0 && (raster > 0 || path.join("hotspots.json").is_file());
    if raster == 0 && !svg_ok {
        return Err(if svgs > 0 {
            "SVG pack without hotspots — add raster cursors/ files or a hotspots.json.".into()
        } else {
            "No usable cursors found (expected XCursor files like default, text — or an svg/ \
             folder)."
                .into()
        });
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
    let mut result = Ok(());
    if raster > 0 {
        result = copy(&raster_src, &dst.join("cursors"));
    }
    if result.is_ok() && svg_ok {
        result = copy(&svg_src, &dst.join("svg"));
    }
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&dst);
        return Err(format!("Copy failed: {e}"));
    }
    for extra in ["index.theme", "hotspots.json"] {
        if path.join(extra).is_file() {
            let _ = std::fs::copy(path.join(extra), dst.join(extra));
        }
    }
    Ok(name)
}
