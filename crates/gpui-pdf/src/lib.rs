//! **PDF viewing for [GPUI](https://www.gpui.rs/)** — a page-virtualized viewer
//! built on the pure-Rust [`hayro`](https://crates.io/crates/hayro) rasterizer (no
//! native libraries, no system-font dependency).
//!
//! Two layers, use whichever fits:
//!
//! - **Low-level primitives** (host-agnostic, pure): [`parse`] a PDF once, read page
//!   sizes with [`page_dims`], rasterize a page to a [`gpui::RenderImage`] with
//!   [`render_page`], and compute the on-screen page range with [`keep_window`].
//!   Build your own viewer on these.
//! - **A ready component**: [`PdfView`] — a self-contained gpui entity that owns its
//!   document, scroll position, off-thread rendering, and viewport eviction, so an
//!   800-page file stays as light as a one-pager. Construct it inside `cx.new` and
//!   render the `Entity<PdfView>` like any child view.
//!
//! ```no_run
//! # use std::rc::Rc;
//! # use std::path::PathBuf;
//! # use gpui::AppContext;
//! # use gpui_pdf::{PdfView, PdfStyle};
//! # fn demo(cx: &mut gpui::App, path: PathBuf) {
//! let view = cx.new(|cx| PdfView::new(path, Rc::new(PdfStyle::default), cx));
//! // then `view.clone()` into your element tree; call `view.update(cx, |v, cx|
//! // v.release(window, cx))` before dropping it (e.g. when its tab closes).
//! # let _ = view;
//! # }
//! ```

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    Context, Hsla, InteractiveElement, IntoElement, ParentElement, Render, RenderImage,
    ScrollHandle, StatefulInteractiveElement, Styled, Window, div, hsla, img, px,
};
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use image::{Frame, RgbaImage};

// ─────────────────────────────── Low-level primitives ───────────────────────────────

/// Page render scale (PDF point-size × this). Higher = sharper but more memory.
pub const SCALE: f32 = 1.5;

/// A parsed PDF. Parse once (not per page) — re-parsing a large file for every page
/// is slow and churns the allocator. [`hayro`]'s `Pdf` is `Send + Sync` (std
/// feature) and caches pages internally, so it's shared via `Arc` across the
/// background render tasks.
pub type Document = Pdf;

/// Parse a PDF's bytes into a reusable [`Document`]. The `Document` owns the bytes,
/// so the caller can drop its own copy.
pub fn parse(bytes: Arc<Vec<u8>>) -> Result<Arc<Document>, String> {
    let pdf = Pdf::new(bytes).map_err(|e| format!("parse PDF: {e:?}"))?;
    Ok(Arc::new(pdf))
}

/// Each page's `(width, height)` in points — cheap to read (no rasterization), so a
/// viewer can lay out correctly-sized page slots before any page renders.
pub fn page_dims(doc: &Document) -> Vec<(f32, f32)> {
    doc.pages().iter().map(|p| p.render_dimensions()).collect()
}

/// Rasterize a single page (0-based) of an already-parsed [`Document`] to a BGRA
/// `RenderImage` composited onto white.
pub fn render_page(doc: &Document, idx: usize, scale: f32) -> Result<Arc<RenderImage>, String> {
    let pixmaps = hayro::render_pdf(doc, scale, InterpreterSettings::default(), Some(idx..=idx))
        .ok_or_else(|| format!("render page {idx}"))?;
    let pixmap = pixmaps
        .into_iter()
        .next()
        .ok_or_else(|| format!("no page {idx}"))?;

    let (w, h) = (u32::from(pixmap.width()), u32::from(pixmap.height()));
    let src = pixmap.data_as_u8_slice(); // premultiplied RGBA8, row-major
    let mut bgra = vec![0u8; src.len()];
    for (out, p) in bgra.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        // Composite premultiplied src over white (out = src + 255-a; src ≤ a so no
        // overflow), then RGBA→BGRA (gpui's RenderImage is BGRA).
        let add = 255 - p[3];
        out[0] = p[2].saturating_add(add); // B
        out[1] = p[1].saturating_add(add); // G
        out[2] = p[0].saturating_add(add); // R
        out[3] = 255;
    }
    let buf = RgbaImage::from_raw(w, h, bgra).ok_or_else(|| "bad pixel buffer".to_string())?;
    Ok(Arc::new(RenderImage::new(vec![Frame::new(buf)])))
}

/// True if a link/image `src` points at a PDF (by extension, case-insensitive).
pub fn is_pdf(src: &str) -> bool {
    src.to_lowercase().trim_end().ends_with(".pdf")
}

// ─────────────────────────────── Viewport virtualization ───────────────────────────────

/// On-screen page width (points); pages keep their aspect ratio.
pub const PAGE_WIDTH: f32 = 820.0;
/// Vertical gap between page slots (matches the column `gap`).
const PAGE_GAP: f32 = 10.0;
/// Top/bottom padding of the page column (matches the column `py`).
const PAGE_PAD_Y: f32 = 16.0;
/// Extra pages to keep rasterized above and below the visible range, so scrolling
/// finds them already rendered (and small wiggles don't thrash render/evict).
const MARGIN: usize = 2;

/// The inclusive page-index range `(start, end)` to keep rasterized for the given
/// scroll position: the pages intersecting the viewport, padded by [`MARGIN`]. Pure
/// (mirrors [`PdfView`]'s slot layout) so it's unit-testable. `scroll_y` is how far
/// the content is scrolled down (px ≥ 0); `viewport_h` is the visible height (px).
pub fn keep_window(dims: &[(f32, f32)], scroll_y: f32, viewport_h: f32) -> (usize, usize) {
    if dims.is_empty() {
        return (0, 0);
    }
    // Before the first paint the viewport height is unknown (0); assume a page or so
    // high so the first pages still render.
    let vh = if viewport_h > 1.0 { viewport_h } else { 900.0 };
    let top = scroll_y.max(0.0);
    let bottom = top + vh;

    let mut y = PAGE_PAD_Y;
    let mut first: Option<usize> = None;
    let mut last = 0usize;
    for (i, (w, h)) in dims.iter().enumerate() {
        let disp_h = if *w > 0.0 {
            PAGE_WIDTH * (h / w)
        } else {
            PAGE_WIDTH
        };
        let page_top = y;
        let page_bottom = y + disp_h;
        if page_bottom > top && page_top < bottom {
            first.get_or_insert(i);
            last = i;
        }
        y = page_bottom + PAGE_GAP;
    }
    let first = first.unwrap_or(0);
    let start = first.saturating_sub(MARGIN);
    let end = (last + MARGIN).min(dims.len() - 1);
    (start, end)
}

// ─────────────────────────────── Component: PdfView ───────────────────────────────

/// A page's rasterization state. Off-screen pages are `Empty` (their image — and its
/// GPU atlas texture — freed) so an open PDF doesn't hold every page at once.
#[derive(Clone)]
enum PageSlot {
    /// Not rendered (never was, or evicted after scrolling away).
    Empty,
    /// A background rasterization is in flight.
    Loading,
    /// Rasterized and ready to paint.
    Ready(Arc<RenderImage>),
}

/// Colors for the [`PdfView`] chrome. Map your theme onto this; [`PdfStyle::default`]
/// is a neutral dark palette.
#[derive(Clone, Copy)]
pub struct PdfStyle {
    /// Viewer background.
    pub bg: Hsla,
    /// Page-slot border and the header divider.
    pub border: Hsla,
    /// Background of an unrendered page slot (placeholder).
    pub placeholder_bg: Hsla,
    /// "Page N" / "Loading…" placeholder text.
    pub placeholder_fg: Hsla,
    /// Header filename text.
    pub header_fg: Hsla,
    /// Header "· N pages" text.
    pub header_muted: Hsla,
}

impl Default for PdfStyle {
    fn default() -> Self {
        Self {
            bg: hsla(0.0, 0.0, 0.12, 1.0),
            border: hsla(0.0, 0.0, 1.0, 0.10),
            placeholder_bg: hsla(0.0, 0.0, 1.0, 0.04),
            placeholder_fg: hsla(0.0, 0.0, 1.0, 0.40),
            header_fg: hsla(0.0, 0.0, 1.0, 0.70),
            header_muted: hsla(0.0, 0.0, 1.0, 0.40),
        }
    }
}

/// Supplies the current [`PdfStyle`] at paint time. Because [`PdfView`] is a
/// persistent entity (not rebuilt by its parent each frame), it reads its colors
/// through this closure — returning fresh colors each call lets the viewer follow
/// live theme changes (and differ per window) without the host pushing updates.
pub type PdfStyleFn = Rc<dyn Fn() -> PdfStyle>;

/// A page-virtualized PDF viewer: a scrollable column of page slots, each sized from
/// the PDF's page dimensions up front (so the scrollbar is correct for the whole
/// document) but only rasterized while near the viewport. Pages scrolled away are
/// freed — CPU pixel buffer *and* GPU atlas texture — so memory is bounded by what's
/// on screen rather than the page count.
///
/// Construct with [`PdfView::new`] inside `cx.new`; it loads and measures the file
/// off-thread. Render the resulting `Entity<PdfView>` like any child view. Call
/// [`release`](PdfView::release) before dropping it (e.g. when its tab closes) to
/// free the atlas textures gpui won't free on plain drop.
pub struct PdfView {
    path: PathBuf,
    style: PdfStyleFn,
    /// The parsed PDF (shared with the background render tasks); `None` until the
    /// off-thread load finishes.
    pdf: Option<Arc<Document>>,
    /// `(width, height)` in points per page — drives page-slot sizing.
    dims: Vec<(f32, f32)>,
    /// Per-page render state; only pages near the viewport are `Ready`.
    pages: Vec<PageSlot>,
    scroll: ScrollHandle,
}

impl PdfView {
    /// Create a viewer for `path`, kicking off the off-thread read + parse + measure.
    /// `style` supplies chrome colors at paint time (see [`PdfStyleFn`]). Call inside
    /// `cx.new(|cx| PdfView::new(path, style, cx))`.
    pub fn new(path: PathBuf, style: PdfStyleFn, cx: &mut Context<Self>) -> Self {
        let load_path = path.clone();
        cx.spawn(async move |this, cx| {
            // Read + parse (once) + measure off-thread.
            let prepared = cx
                .background_executor()
                .spawn(async move {
                    let bytes = Arc::new(std::fs::read(&load_path).map_err(|e| e.to_string())?);
                    let doc = parse(bytes)?;
                    let dims = page_dims(&doc);
                    Ok::<_, String>((doc, dims))
                })
                .await;
            let (doc, dims) = match prepared {
                Ok(x) => x,
                Err(e) => {
                    log::error!("load pdf: {e}");
                    return;
                }
            };
            let n = dims.len();
            // Store the parsed doc + sizes and mark every page Empty; the next render
            // runs `ensure_window`, which rasterizes the visible window.
            let _ = this.update(cx, |this, cx| {
                this.pdf = Some(doc);
                this.dims = dims;
                this.pages = vec![PageSlot::Empty; n];
                cx.notify();
            });
        })
        .detach();

        Self {
            path,
            style,
            pdf: None,
            dims: Vec::new(),
            pages: Vec::new(),
            scroll: ScrollHandle::new(),
        }
    }

    /// Free every rasterized page — CPU pixel buffer (by dropping the `Arc`s) *and*
    /// the GPU atlas texture. gpui caches one atlas texture per `RenderImage` on
    /// paint and only frees it via `drop_image`; a raw `ImageSource::Render` is never
    /// auto-evicted, so call this before dropping the view or the textures leak.
    pub fn release(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for slot in std::mem::take(&mut self.pages) {
            if let PageSlot::Ready(arc) = slot {
                cx.drop_image(arc, Some(window));
            }
        }
    }

    /// Keep only the pages near the viewport rasterized: render missing visible pages
    /// and evict (drop image + GPU texture) the rest. Called every frame from
    /// `render` — cheap (a window calc + slot scan); it only spawns/evicts when the
    /// window actually changes. This is what bounds an open PDF's memory to the
    /// on-screen pages instead of the whole document.
    fn ensure_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dims.is_empty() || self.pdf.is_none() {
            return; // not loaded yet (still reading/parsing)
        }
        let scroll_y = f32::from(-self.scroll.offset().y);
        let viewport_h = f32::from(self.scroll.bounds().size.height);
        let (start, end) = keep_window(&self.dims, scroll_y, viewport_h);

        // Pass 1 (immutable): decide what to evict / render.
        let mut to_evict: Vec<(usize, Arc<RenderImage>)> = Vec::new();
        let mut to_render: Vec<usize> = Vec::new();
        for (i, slot) in self.pages.iter().enumerate() {
            let in_window = i >= start && i <= end;
            match slot {
                PageSlot::Ready(arc) if !in_window => to_evict.push((i, arc.clone())),
                PageSlot::Empty if in_window => to_render.push(i),
                _ => {}
            }
        }
        if to_evict.is_empty() && to_render.is_empty() {
            return;
        }

        // Pass 2 (mutable): apply the new slot states.
        for (i, _) in &to_evict {
            self.pages[*i] = PageSlot::Empty;
        }
        for i in &to_render {
            self.pages[*i] = PageSlot::Loading;
        }
        // Free the GPU atlas texture for each evicted page (see `release`).
        for (_, arc) in to_evict {
            cx.drop_image(arc, Some(window));
        }

        // Rasterize newly-visible pages off-thread (sharing the parsed doc); paint
        // each as it lands.
        let pdf = self.pdf.clone().unwrap();
        for i in to_render {
            let pdf = pdf.clone();
            cx.spawn(async move |this, cx| {
                let page = cx
                    .background_executor()
                    .spawn(async move { render_page(&pdf, i, SCALE).ok() })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    // Only store if still wanted (not evicted mid-flight or released).
                    if let Some(slot @ PageSlot::Loading) = this.pages.get_mut(i) {
                        *slot = match page {
                            Some(img) => PageSlot::Ready(img),
                            None => PageSlot::Empty, // failed; allow a later retry
                        };
                        cx.notify();
                    }
                });
            })
            .detach();
        }
    }
}

impl Render for PdfView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Keep only the on-screen pages rasterized for the current scroll position.
        self.ensure_window(window, cx);
        let style = (self.style)();

        if self.dims.is_empty() {
            return loading(style).into_any_element();
        }

        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let total = self.dims.len();

        let mut col = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(PAGE_GAP))
            .py(px(PAGE_PAD_Y));
        for (i, (w, h)) in self.dims.iter().enumerate() {
            let disp_h = if *w > 0.0 {
                PAGE_WIDTH * (h / w)
            } else {
                PAGE_WIDTH
            };
            let slot = div()
                .w(px(PAGE_WIDTH))
                .h(px(disp_h))
                .rounded(px(2.0))
                .border_1()
                .border_color(style.border)
                .overflow_hidden();
            let slot = match self.pages.get(i) {
                Some(PageSlot::Ready(page)) => {
                    slot.child(img(page.clone()).w(px(PAGE_WIDTH)).h(px(disp_h)))
                }
                // Empty (off-screen / not yet rendered) or Loading: a sized
                // placeholder so layout is stable; it fills in once rasterized.
                _ => slot
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(style.placeholder_bg)
                    .child(
                        div()
                            .text_color(style.placeholder_fg)
                            .child(format!("Page {}", i + 1)),
                    ),
            };
            col = col.child(slot);
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(style.bg)
            .child(
                div()
                    .flex_shrink_0()
                    .px(px(16.0))
                    .py(px(8.0))
                    .border_b_1()
                    .border_color(style.border)
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.0))
                    .text_color(style.header_fg)
                    .child(format!("📄 {name}"))
                    .child(
                        div()
                            .text_color(style.header_muted)
                            .child(format!("· {total} pages")),
                    ),
            )
            .child(
                div()
                    .id("pdf-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    // Scrolling doesn't re-run render on its own; notify so the next
                    // frame re-runs `ensure_window` for the new scroll position.
                    .on_scroll_wheel(cx.listener(|_this, _ev, _window, cx| {
                        cx.notify();
                    }))
                    .child(col),
            )
            .into_any_element()
    }
}

fn loading(style: PdfStyle) -> impl IntoElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(style.bg)
        .child(div().text_color(style.placeholder_fg).child("Loading PDF…"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_pdf_extension() {
        assert!(is_pdf("a.pdf"));
        assert!(is_pdf("images/B.PDF"));
        assert!(!is_pdf("a.png"));
        assert!(!is_pdf("notapdf"));
    }

    // 10 US-Letter-ish pages (portrait): disp_h = 820 * 11/8.5 ≈ 1061.2 px each.
    fn letter_pages(n: usize) -> Vec<(f32, f32)> {
        vec![(8.5, 11.0); n]
    }

    #[test]
    fn window_at_top_covers_first_pages_plus_margin() {
        let dims = letter_pages(10);
        // Top of the document, ~900px viewport → only page 0 is visible.
        assert_eq!(keep_window(&dims, 0.0, 900.0), (0, 2));
    }

    #[test]
    fn window_follows_scroll() {
        let dims = letter_pages(10);
        // Scrolled into page 2 (page tops ≈ 16, 1087, 2158, …).
        assert_eq!(keep_window(&dims, 2200.0, 900.0), (0, 4));
    }

    #[test]
    fn window_clamps_at_the_end() {
        let dims = letter_pages(10);
        // Scrolled near the bottom: last pages, end clamped to the final index.
        let (start, end) = keep_window(&dims, 9000.0, 900.0);
        assert_eq!(end, 9);
        assert!(start >= 6);
    }

    #[test]
    fn empty_doc_is_safe() {
        assert_eq!(keep_window(&[], 0.0, 900.0), (0, 0));
    }
}
