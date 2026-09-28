# Vendored: gpui-pre-wgpu 0.3.2

This is the crates.io `gpui-pre-wgpu` 0.3.2 source (Zed's `gpui_wgpu`,
zed@801c087; Apache-2.0, see `LICENSE-APACHE`), wired in through
`[patch.crates-io]` in the workspace `Cargo.toml`. It is gpui's renderer and
text system for Linux (and web). One change, marked `ZORITE PATCH` in
`src/cosmic_text_system.rs`:

- **Glyph offsets** (issue #108): `layout_line` copied each glyph's pen
  position from cosmic-text and dropped its `x_offset` / `y_offset`, so every
  mark the shaper positions relative to another glyph was painted at the pen:
  Arabic and Persian dots and maddas slid left of their letters, and a
  combining macron sat beside a `v`. Upstream: Zed's `crates/gpui_wgpu` has the
  same line (still unfixed as of 2026-09-28); zed-industries/zed#60387 is the
  same bug seen from Latin text.

The crate's own tests don't build outside Zed's repo (they need Zed's `naga`
dev-dependency and asset paths), so they're left untouched and unrun here.

**Drop this directory** (and the `[patch.crates-io]` entry) when moving to a
`gpui-pre` release that includes the fix. Until then, a gpui-pre bump means
re-vendoring the new version and re-applying the patch.
