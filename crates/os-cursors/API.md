# os-cursors API

The complete public API of [`os-cursors`](README.md) — every exported item,
with its signature, contract, and platform behavior. For the what-and-why
(interception techniques, quick start), see the [README](README.md).

## Public API at a glance

| Item | Kind | Signature | Purpose |
| --- | --- | --- | --- |
| [`Cursor`](#enum-cursor) | enum | 19 unit variants | The named cursors an app can replace |
| [`Cursor::freedesktop_name`](#cursorfreedesktop_name) | method | `fn freedesktop_name(self) -> &'static str` | The theme-file name for this cursor |
| [`Cursor::all`](#cursorall) | method | `fn all() -> &'static [Cursor]` | Every replaceable cursor |
| [`Image`](#struct-image) | struct | public fields | One cursor frame: pixels + hotspot + nominal size |
| [`best_image`](#fn-best_image) | fn | `fn best_image(&[Image], target_px: u32) -> Option<&Image>` | Pick the frame for a target pixel size |
| [`install`](#fn-install) | fn | `fn install(Cursor, &[Image], points: f32) -> bool` | Replace one cursor (macOS / Windows) |
| [`reset`](#fn-reset) | fn | `fn reset()` | Remove every installed cursor |
| [`use_xcursor_theme`](#fn-use_xcursor_theme) | fn | `fn use_xcursor_theme(&Path, name: &str, size_px: u32) -> bool` | Point the process at a theme (Linux) |
| [`xcursor::parse`](#xcursorparse) | fn | `fn parse(&[u8]) -> Option<Vec<Image>>` | Decode an XCursor file |
| [`xcursor::write`](#xcursorwrite) | fn | `fn write(&[Image]) -> Vec<u8>` | Encode an XCursor file |

---

## `enum Cursor`

```rust
pub enum Cursor {
    Arrow, IBeam, Crosshair, ClosedHand, OpenHand, PointingHand,
    ResizeLeft, ResizeRight, ResizeLeftRight, ResizeUp, ResizeDown,
    ResizeUpDown, ResizeUpLeftDownRight, ResizeUpRightDownLeft,
    IBeamVertical, OperationNotAllowed, DragLink, DragCopy, ContextualMenu,
}
```

`Copy`, `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`.

The named cursors an app can replace — the intersection of what AppKit
vends and what toolkits request. There is no `ResizeColumn`/`ResizeRow`:
every platform aliases them onto `ResizeLeftRight`/`ResizeUpDown`.

### `Cursor::freedesktop_name`

```rust
pub fn freedesktop_name(self) -> &'static str
```

The freedesktop cursor-file name — the file inside an XCursor theme's
`cursors/` directory — for this cursor: `Arrow` → `"default"`, `IBeam` →
`"text"`, `ResizeLeftRight` → `"ew-resize"`, `DragLink` → `"alias"`, etc.
Use it to look up pack files by `Cursor`.

### `Cursor::all`

```rust
pub fn all() -> &'static [Cursor]
```

Every replaceable cursor, for iterating a pack install.

---

## `struct Image`

```rust
pub struct Image {
    pub size: u32,            // nominal size (XCursor TOC subtype)
    pub width: u32,
    pub height: u32,
    pub hotspot: (u32, u32),  // pixels, from the top-left
    pub delay: u32,           // animation frame delay in ms; 0 = static
    pub bgra: Vec<u8>,        // premultiplied BGRA, width * height * 4
}
```

`Clone`, `Debug`, `PartialEq`, `Eq`.

One cursor frame. Pixels are **premultiplied BGRA** — exactly the XCursor
on-disk pixel format (little-endian ARGB), which is also what Windows alpha
cursors take; macOS swaps to RGBA on install. `delay` is preserved so
animated cursors round-trip through [`xcursor::write`](#xcursorwrite);
[`install`](#fn-install) shows the first frame only.

---

## `fn best_image`

```rust
pub fn best_image(images: &[Image], target_px: u32) -> Option<&Image>
```

The best single frame for a target pixel size: the smallest nominal size
`>= target_px`, else the largest available. Among animation siblings of one
size, the first frame wins. `None` only for an empty slice.

---

## `fn install`

```rust
pub fn install(cursor: Cursor, images: &[Image], points: f32) -> bool
```

Replace `cursor` with the best-fitting frame from `images`. Call on the
**UI thread**. Installing again replaces the previous image; mixing
`install` calls builds up a pack one cursor at a time.

- **macOS** — honors `points` (the native arrow is ≈17pt; 20pt with 64px
  frames renders Retina-crisp). Prefers a frame ≥3× `points` for headroom.
  All 19 cursors work; the themed cursor applies process-wide, and the
  system cursor returns outside your windows.
- **Windows** — `points` is ignored; cursors show at pixel size, so the
  frame nearest `GetSystemMetrics(SM_CXCURSOR)` is used. Only the nine
  cursors with their own standard handle install (see the README table);
  the aliased rest return `false`.
- **Linux** — always `false`; use [`use_xcursor_theme`](#fn-use_xcursor_theme).

Returns `false` when the cursor can't be replaced on this platform, the
slice is empty, or an image is malformed (undersized pixel buffer).

---

## `fn reset`

```rust
pub fn reset()
```

Remove every installed cursor — the native ones return. The interception
hooks stay armed (harmless), so a later `install` is cheap. Does not undo
[`use_xcursor_theme`](#fn-use_xcursor_theme)'s environment on Linux.

---

## `fn use_xcursor_theme`

```rust
pub fn use_xcursor_theme(themes_dir: &Path, name: &str, size_px: u32) -> bool
```

**Linux:** point this process at an XCursor theme —
`themes_dir/name/cursors/` must exist, or `false`. Sets `XCURSOR_PATH`
(prepending `themes_dir`, keeping the inherited search path for
`inherits=` fallbacks), `XCURSOR_THEME`, and `XCURSOR_SIZE`.

libXcursor and libwayland-cursor read these when the display connection
loads cursors, so this must run **before any windowing setup and before
other threads exist** (environment mutation is process-global) — first
thing in `main`. Works on X11 and Wayland alike; see the README for the
`cursor-shape-v1` caveat.

**macOS / Windows:** no-op, returns `false` — use [`install`](#fn-install).

---

## Module `xcursor`

The XCursor binary format, pure Rust. An XCursor file holds one logical
cursor at several nominal sizes (plus animation frames); a theme is a
directory of them under `cursors/`, named by
[`Cursor::freedesktop_name`](#cursorfreedesktop_name).

### `xcursor::parse`

```rust
pub fn parse(data: &[u8]) -> Option<Vec<Image>>
```

Every image frame in an XCursor file, in TOC order (sizes ascending,
animation frames consecutive). `None` on anything malformed — bad magic,
truncated chunks, absurd dimensions (> 1024px). Non-image chunks
(comments) are skipped.

### `xcursor::write`

```rust
pub fn write(images: &[Image]) -> Vec<u8>
```

Encode frames as an XCursor file. Give them in TOC order — sizes
ascending; [`parse`](#xcursorparse)'s output order round-trips exactly.
Useful for stripping themes to one size or generating packs (e.g. from
rasterized SVGs).
