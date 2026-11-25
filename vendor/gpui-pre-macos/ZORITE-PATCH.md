# Vendored: gpui-pre-macos 0.3.12

This is the crates.io `gpui-pre-macos` 0.3.12 source (Zed's `gpui_macos`,
zed@d2a4fb6; Apache-2.0, see `LICENSE-APACHE`), wired in through
`[patch.crates-io]` in the workspace `Cargo.toml`. It is gpui's macOS platform
layer. One change, marked `ZORITE PATCH` in `src/window.rs`:

- **Frame source**: on macOS 14+, gpui drives each window's frames from a
  `CAMetalDisplayLink`. After the window moves to another screen (docking,
  undocking, a lid-close sleep), that link reports itself running but never
  delivers another update, so the window stops repainting. Input still works
  and is saved, but the window looks frozen. Re-creating the link, or even the
  `CAMetalLayer`, doesn't bring it back. The patch sets `metal_frames` to
  `None`, so every window uses the per-display `CVDisplayLink` path that gpui
  still ships for older macOS. Upstream: zed-industries/zed#67295 (the same
  link stalling, from a different trigger).

**Drop this directory** (and its `[patch.crates-io]` entry) when a gpui-pre
release fixes the Metal frame source. Until then, a gpui-pre bump means
re-vendoring the new version and re-applying the one line.
