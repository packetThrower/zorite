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
- **Zoom & navigation, no flicker.** Built-in zoom (− / + / reset and ⌘=/⌘-/⌘0) and
  navigation (‹ / › with a click-to-edit page counter you can type a number into,
  plus PageUp / PageDown / Home / End). On a zoom or quality change the page never
  blanks — the current bitmap stays on screen (rescaled) until the crisp re-render
  lands. The nearest pages render first.
- **DPI-aware, host-settable quality.** Pages rasterize at the display's pixel ratio
  × zoom × a quality multiplier the host supplies (read reactively, like the theme),
  so a settings slider can trade sharpness for speed on slower machines — crisp on
  Retina by default.
- **Off-thread rendering.** The file is read, parsed *once*, and measured on a
  background thread; pages rasterize on the background executor and paint as they
  land. The UI never blocks.
- **Self-contained.** `PdfView` is a gpui entity that owns its document, scroll
  position, zoom, render/evict loop, and styling. Drop the `Entity<PdfView>` into
  your element tree — no per-frame plumbing from the host.
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
        Rc::new(|| 1.0),                        // render-quality multiplier (1.0 = native DPI)
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
| `new` | `fn new(path: PathBuf, style: PdfStyleFn, quality: PdfQualityFn, cx: &mut Context<Self>) -> Self` | Create a viewer and start the off-thread read + parse + measure. `style` and `quality` are read at paint time (see below). Call inside `cx.new(\|cx\| …)`. |
| `release` | `fn release(&mut self, window: &mut Window, cx: &mut Context<Self>)` | Free every rasterized page (CPU buffer + GPU atlas texture). Call before dropping the view — gpui only frees a `RenderImage`'s atlas texture via `drop_image`, never on plain drop. |
| `set_zoom` / `zoom_in` / `zoom_out` / `reset_zoom` | `fn …(&mut self, cx: &mut Context<Self>)` (`set_zoom` also takes `zoom: f32`) | Change zoom (clamped 0.5–3.0), keeping the current page in view; the visible pages re-rasterize crisp at the new scale, with no blank. |
| `go_to_page` / `next_page` / `prev_page` | `fn …(&mut self, cx: &mut Context<Self>)` (`go_to_page` also takes `index: usize`) | Scroll so the target page sits at the top of the viewport. |

The viewer renders a header with these controls — including a click-to-edit page
counter (type a number, Enter to jump) — and handles the keyboard shortcuts PageUp /
PageDown / Home / End and ⌘=/⌘-/⌘0 when focused (it focuses on click). Pages rasterize
at the display's pixel ratio × zoom × the quality multiplier.

Quality is host-set: there's no `set_quality` method because the viewer reads the
`PdfQualityFn` each paint, so changing the host's value re-renders every open viewer
(in every window) automatically.

Each `PdfView` owns its own scroll handle, so multiple open at once scroll
independently.

### `PdfStyle`, `PdfStyleFn`, and `PdfQualityFn`

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
pub type PdfQualityFn = Rc<dyn Fn() -> f32>;   // render-quality multiplier source
```

`PdfStyle::default()` is a neutral dark palette. The viewer reads its colors — and its
quality multiplier — through these closures at paint time (not stored), so returning
fresh values each call lets it follow live theme / settings changes (in every window)
with no push from the host. `quality` is `1.0` = native DPI; lower is faster and
softer, higher supersamples (clamped internally).

### Low-level primitives

```rust
pub type Document;                                      // a parsed PDF (hayro)

pub fn parse(bytes: Arc<Vec<u8>>) -> Result<Arc<Document>, String>;
pub fn page_dims(doc: &Document) -> Vec<(f32, f32)>;    // (w, h) points per page
pub fn render_page(doc: &Document, idx: usize, scale: f32)
    -> Result<Arc<gpui::RenderImage>, String>;          // BGRA over white
pub fn is_pdf(src: &str) -> bool;                       // extension check

pub const PAGE_WIDTH: f32;                             // base column width at zoom 1
pub fn keep_window(dims: &[(f32, f32)], page_width: f32, scroll_y: f32, viewport_h: f32)
    -> (usize, usize);                                  // inclusive visible range
```

Parse once, then rasterize pages on demand — `hayro::Pdf` is `Send + Sync` and caches
pages internally, so share it via `Arc` across background tasks.

## Markup (`markup` feature)

Opt-in text-anchored highlights, with **no heavyweight dependency** — a custom hayro
`Device` extracts the page's text + glyph rectangles (only `kurbo` geometry, already
in hayro's tree; no oxidize-pdf). Storage stays the host's: hand the viewer the
highlights to draw (e.g. derived from notes that quote the PDF) and it locates each
quote and boxes it.

```rust
// Text layer (also usable standalone, e.g. for search):
pub fn extract_page_text(doc: &Document, page: usize) -> Option<PageText>;
impl PageText {
    pub fn text(&self) -> String;                            // readable reconstruction
    pub fn locate(&self, needle: &str, occurrence: usize)
        -> Vec<NormRect>;                                    // one rect per line spanned
}
pub struct NormRect { pub x: f32, pub y: f32, pub w: f32, pub h: f32 } // 0..1 of the page

// Drawing highlights on the viewer:
pub struct Highlight { pub id: u64, pub page: usize, pub quote: String,
                       pub occurrence: usize, pub color: Hsla }
impl PdfView {
    pub fn set_highlights(&mut self, highlights: Vec<Highlight>, cx: &mut Context<Self>);
    pub fn set_on_highlight(&mut self, handler: HighlightClickFn);       // click → id
    // A color picker: the ✎ toggle pops down a palette of (label, fill) swatches; the
    // active color tints new highlights and its label is echoed back on create.
    pub fn set_highlight_palette(&mut self, palette: Vec<(SharedString, Hsla)>,
                                 cx: &mut Context<Self>);
    // Interactive creation: ✎ turns on "highlight mode", where dragging over text
    // resolves a selection and fires the create handler.
    pub fn set_on_create_highlight(&mut self, handler: CreateHighlightFn);
    // CreateHighlightFn = Fn(page, quote, occurrence, color_label, &mut Window, &mut App)
    pub fn toggle_select_mode(&mut self, cx: &mut Context<Self>);
    // Jump from a note: scroll a page in (to its first highlight) and flash them.
    pub fn reveal_highlight(&mut self, page: usize, cx: &mut Context<Self>);
}
impl PageText {              // drag → selection, also usable directly
    pub fn select(&self, from: NormPoint, to: NormPoint) -> Option<Selection>;
}
```

`locate` matches case- and whitespace-insensitively (so a quote survives PDF spacing
quirks) and returns one normalized rect per line it spans. The viewer extracts a
page's text lazily — off-thread, cached — when a highlighted page scrolls into view.
Because coordinates are normalized, highlights track zoom and DPI for free. The host
owns storage: on create it persists the quote and color label (however it likes) and
feeds the highlights back via `set_highlights`. For the reverse direction, a note can
link to `file.pdf#pN` and call [`reveal_highlight`] to scroll to and flash it.

## Status

Early, but solid for scroll-to-read viewing. Renders via the pure-Rust
[`hayro`](https://crates.io/crates/hayro) crate. Not yet published to crates.io.

Text extraction, highlight rendering, drag-to-select creation (with a color picker),
and note→PDF reverse links (scroll to + flash a highlight) are all available behind
`markup` (dep-free). A browser-style **find-in-PDF** bar (🔍 / ⌘F) sits on top of the
same text layer behind the `search` feature (`= ["markup"]`). Roadmap: area highlights
for pages with no text layer.

## License

GPL-3.0-or-later.
