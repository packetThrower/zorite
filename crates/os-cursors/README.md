# os-cursors

Per-app **custom mouse cursors** without forking the UI toolkit:

- **macOS** — the `NSCursor` class factory methods (`+arrowCursor`, …) are
  swizzled via the objc runtime, so every AppKit caller in the process —
  your toolkit included — vends your cursors.
- **Windows** — a thread-scoped `WH_CALLWNDPROCRET` hook watches
  `WM_SETCURSOR` and swaps the standard `IDC_*` cursor the toolkit just set
  for yours.
- **Linux** — the platform already themes per process: `use_xcursor_theme`
  sets `XCURSOR_THEME` / `XCURSOR_PATH` / `XCURSOR_SIZE`, which libXcursor
  and libwayland-cursor honor. Per-image install is a no-op there.

The pack currency is the **XCursor theme directory** — the standard Linux
cursor-theme format — so every existing theme is drop-in content. The
[`xcursor`](API.md#module-xcursor) module parses and writes the binary
files in pure Rust, hotspots included; there are no dependencies beyond the
per-platform system bindings, and **no `gpui` dependency**.

**📖 Full reference:** every public item, with signatures, contracts, and
platform caveats, lives in [API.md](API.md).

## How it works

UI toolkits don't draw cursors — they ask the OS for *named* ones, every
time the pointer moves over a region. This crate changes what those answers
return, for the current process only. Nothing outside your windows is
affected; the system cursor is untouched.

Because interception happens below the toolkit, it works with any AppKit-
or Win32-backed UI (gpui, winit, …) with zero toolkit patches — and keeps
working across toolkit upgrades.

## Quick start

```rust
use os_cursors::{Cursor, xcursor};

// One cursor file from an XCursor theme's cursors/ directory:
let bytes = std::fs::read("Bibata-Modern-Ice/cursors/default")?;
let images = xcursor::parse(&bytes).expect("valid XCursor file");

// Show it at 20pt for the arrow (macOS honors points; Windows picks the
// frame nearest the system cursor pixel size).
os_cursors::install(Cursor::Arrow, &images, 20.0);

// Linux instead points the process at the theme, before any UI starts:
os_cursors::use_xcursor_theme("/path/to/themes".as_ref(), "Bibata-Modern-Ice", 24);

// Native cursors return any time:
os_cursors::reset();
```

Call `install` on the UI thread. On Linux, `use_xcursor_theme` must run
**before the display connection and before other threads exist** — first
thing in `main`.

## Platform granularity

| | Replaceable cursors | Notes |
| --- | --- | --- |
| macOS | all 19 `Cursor` variants | point-size honored; a 64px frame at 20pt is Retina-crisp |
| Windows | 9 (`Arrow`, `IBeam`, `Crosshair`, `PointingHand`, 4 resize axes, `OperationNotAllowed`) | Windows aliases the rest onto these standard handles |
| Linux | whole theme at once | the OS mechanism; X11 and Wayland alike |

One Wayland caveat: a client using the `cursor-shape-v1` protocol delegates
cursor drawing to the compositor and cannot be themed per-app. Toolkits on
the classic `wl_cursor` path (gpui today) are fine.

## License

GPL-3.0-or-later, like the rest of the Zorite workspace.
