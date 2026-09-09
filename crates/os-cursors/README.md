# os-cursors

[![crates.io](https://img.shields.io/crates/v/os-cursors.svg)](https://crates.io/crates/os-cursors) [![docs.rs](https://docs.rs/os-cursors/badge.svg)](https://docs.rs/os-cursors) [![license: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Custom mouse cursors for your app on macOS, Windows, and Linux, without
changing the UI toolkit.

Cursor packs are standard **XCursor theme directories**, the Linux cursor-theme
format, so any existing theme works as it is. The
[`xcursor`](API.md#module-xcursor) module reads and writes those files in pure
Rust, hotspots included. Apart from the per-platform system bindings there are
no dependencies, and no `gpui` dependency.

The complete API reference is in [API.md](API.md).

## How it works

UI toolkits don't draw cursors — they ask the OS for *named* ones, every
time the pointer moves over a region. This crate changes what those answers
return, for the current process only. Nothing outside your windows is
affected; the system cursor is untouched.

Because interception happens below the toolkit, it works with any AppKit-
or Win32-backed UI (gpui, winit, …) with zero toolkit patches — and keeps
working across toolkit upgrades.

## Adding the dependency

Published on [crates.io](https://crates.io/crates/os-cursors):

```toml
[dependencies]
os-cursors = "0.1"
```

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

MIT. (The Zorite app itself is GPL-3.0-or-later.)
