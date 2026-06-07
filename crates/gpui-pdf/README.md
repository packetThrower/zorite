# gpui-pdf

**Page-virtualized PDF viewing for [GPUI](https://www.gpui.rs/)**, built on the
pure-Rust [`hayro`](https://crates.io/crates/hayro) rasterizer — no native libraries,
no system-font dependency, so it builds and runs the same on macOS, Linux, and
Windows.

It comes in two layers: low-level rasterization primitives you can build your own
viewer on, and a ready-made [`PdfView`](#pdfview) component that handles loading,
scrolling, rendering, and memory on its own.

## Features

- **Bounded memory.** `PdfView` is page-virtualized: every page gets a correctly
  sized slot up front (so the scrollbar reflects the whole document), but only the
  pages near the viewport are rasterized. Pages scrolled away are freed — CPU pixel
  buffer *and* GPU atlas texture — so an 800-page document stays as light as a
  one-pager.
- **Off-thread rendering.** The file is read, parsed *once*, and measured on a
  background thread; pages rasterize on the background executor and paint as they
  land. The UI never blocks.
- **Self-contained.** `PdfView` is a gpui entity that owns its document, scroll
  position, render/evict loop, and styling. Drop the `Entity<PdfView>` into your
  element tree — no per-frame plumbing from the host.
- **Theme-reactive.** Colors come from a closure read at paint time, so the viewer
  follows live theme changes (and can differ per window) with no push from the host.
- **Pure primitives.** [`parse`], [`page_dims`], [`render_page`], and the
  [`keep_window`] virtualization math are plain functions (no entity required) for
  custom viewers.

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
        cx,
    )
});

// Render it like any child view:
div().child(view.clone())

// Free its GPU textures before dropping it (e.g. when its tab closes):
view.update(cx, |v, cx| v.release(window, cx));
```

## API

### `PdfView`

A self-contained, page-virtualized viewer entity (`impl Render`).

| Method | Signature | Purpose |
| --- | --- | --- |
| `new` | `fn new(path: PathBuf, style: PdfStyleFn, cx: &mut Context<Self>) -> Self` | Create a viewer and start the off-thread read + parse + measure. Call inside `cx.new(\|cx\| …)`. |
| `release` | `fn release(&mut self, window: &mut Window, cx: &mut Context<Self>)` | Free every rasterized page (CPU buffer + GPU atlas texture). Call before dropping the view — gpui only frees a `RenderImage`'s atlas texture via `drop_image`, never on plain drop. |

Each `PdfView` owns its own scroll handle, so multiple open at once scroll
independently.

### `PdfStyle` and `PdfStyleFn`

```rust
pub struct PdfStyle {
    pub bg: Hsla,             // viewer background
    pub border: Hsla,         // page-slot border + header divider
    pub placeholder_bg: Hsla, // unrendered page slot
    pub placeholder_fg: Hsla, // "Page N" / "Loading…" text
    pub header_fg: Hsla,      // header filename
    pub header_muted: Hsla,   // header "· N pages"
}

pub type PdfStyleFn = Rc<dyn Fn() -> PdfStyle>;
```

`PdfStyle::default()` is a neutral dark palette. The viewer reads its colors through
the `PdfStyleFn` at paint time (not stored), so returning fresh colors each call lets
it track live theme changes.

### Low-level primitives

```rust
pub const SCALE: f32;                                   // default page render scale
pub type Document;                                      // a parsed PDF (hayro)

pub fn parse(bytes: Arc<Vec<u8>>) -> Result<Arc<Document>, String>;
pub fn page_dims(doc: &Document) -> Vec<(f32, f32)>;    // (w, h) points per page
pub fn render_page(doc: &Document, idx: usize, scale: f32)
    -> Result<Arc<gpui::RenderImage>, String>;          // BGRA over white
pub fn is_pdf(src: &str) -> bool;                       // extension check

pub const PAGE_WIDTH: f32;
pub fn keep_window(dims: &[(f32, f32)], scroll_y: f32, viewport_h: f32)
    -> (usize, usize);                                  // inclusive visible range
```

Parse once, then rasterize pages on demand — `hayro::Pdf` is `Send + Sync` and caches
pages internally, so share it via `Arc` across background tasks.

## Status

Early, but solid for scroll-to-read viewing. Renders via the pure-Rust
[`hayro`](https://crates.io/crates/hayro) crate. Not yet published to crates.io.

Roadmap: zoom + page navigation, DPI-aware scale, and text/annotation layers.

## License

GPL-3.0-or-later.
