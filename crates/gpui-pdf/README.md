# gpui-pdf

**Page-virtualized PDF viewing for [GPUI](https://www.gpui.rs/)**, built on the
pure-Rust [`hayro`](https://crates.io/crates/hayro) rasterizer — no native libraries,
no system-font dependency, so it builds and runs the same on macOS, Linux, and
Windows.

It comes in two layers: low-level primitives (`parse`, `page_dims`, `render_page`,
`keep_window`) you can build your own viewer on, and a ready-made `PdfView`
component that handles loading, scrolling, rendering, and memory on its own.

**📖 Full reference:** every public item, with signatures, parameter tables,
return contracts, edge cases, and cost notes, lives in [API.md](API.md).

## Overview

- **Bounded memory.** `PdfView` is page-virtualized: every page gets a correctly
  sized slot up front (so the scrollbar reflects the whole document), but only the
  pages near the viewport are rasterized. Pages scrolled away are freed — CPU pixel
  buffer *and* GPU atlas texture — so an 800-page document stays as light as a
  one-pager.
- **Zoom & navigation, no flicker.** Built-in zoom and page navigation (header
  controls, a click-to-edit page counter, keyboard shortcuts, a scroll-to-top
  button). On a zoom or quality change the page never blanks — the current bitmap
  stays on screen, rescaled, until the crisp re-render lands.
- **Off-thread everything.** The file is read, parsed *once*, and measured on a
  background thread; pages rasterize on the background executor and paint as they
  land. The UI never blocks.
- **DPI-aware, host-settable quality.** Pages rasterize at the display's pixel
  ratio × zoom × a quality multiplier the host supplies — read reactively, like the
  theme, so a settings slider re-renders every open viewer automatically.
- **Password-protected PDFs.** An encrypted file doesn't fail to load — `PdfView`
  enters a *locked* state and emits an event so the host can render its own
  password prompt; `unlock(password)` retries (RC4 / AES-128 / AES-256 via hayro's
  standard security handler — the exact table is in [API.md](API.md)).
- **Outline & links.** A table-of-contents side panel from the document's outline,
  and clickable link annotations (internal → jump to page, external → open URL),
  both also exposed as plain functions (`outline`, `page_links`) for custom UIs.
- **Theme-reactive.** Colors come from a closure read at paint time, so the viewer
  follows live theme changes (and can differ per window) with no push from the host.

## Adding the dependency

Not yet published to crates.io — use a path (or git) dependency:

```toml
[dependencies]
gpui-pdf = { path = "../gpui-pdf" }

# Optional features:
#   markup — text layer + quote-anchored highlights (adds only `kurbo`)
#   search — find-in-PDF bar; implies markup
gpui-pdf = { path = "../gpui-pdf", features = ["search"] }
```

## Quick start

```rust
use std::rc::Rc;
use std::path::PathBuf;
use gpui_pdf::{PdfView, PdfStyle};

// Create the viewer (kicks off the off-thread load):
let view = cx.new(|cx| {
    PdfView::new(
        path,                                  // PathBuf to a local .pdf
        Rc::new(|| PdfStyle {                  // map your theme onto the chrome
            bg: my_theme::bg(),
            border: my_theme::border(),
            placeholder_bg: my_theme::muted_bg(),
            placeholder_fg: my_theme::muted_fg(),
            header_fg: my_theme::text(),
            header_muted: my_theme::muted_fg(),
        }),
        Rc::new(|| 1.0),                        // render-quality multiplier (1.0 = native DPI)
        cx,
    )
});

// Render it like any child view:
div().child(view.clone())

// Free its GPU textures before dropping it (e.g. when its tab closes):
view.update(cx, |v, cx| v.release(window, cx));
```

The exact contracts (locked-state flow, texture lifetime, zoom clamps, the
low-level primitives) are in [API.md](API.md).

## Markup & search (optional features)

`markup` adds a text layer — extracted by a custom hayro `Device`, no heavyweight
PDF dependency — and quote-anchored highlights on top of it: the host stores quotes
however it likes, hands them to the viewer to locate and draw, and gets callbacks
for clicks and drag-to-create (with a color picker). Coordinates are normalized, so
highlights track zoom and DPI for free. `search` builds a browser-style find-in-PDF
bar (⌘F, match highlighting, next/prev) on the same text layer.

## Status

Early, but solid for scroll-to-read viewing. Password-protected PDFs open behind a
host-rendered prompt. Markup and find-in-PDF are available behind their features.
Roadmap: area highlights for pages with no text layer. Not yet published to
crates.io.

## License

GPL-3.0-or-later.
