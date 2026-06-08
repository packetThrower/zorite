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
//!   800-page file stays as light as a one-pager. It also has built-in **zoom**,
//!   **page navigation** (including a jump-to-page input), and **DPI-aware**
//!   rendering with a host-settable **quality** multiplier. Construct it inside
//!   `cx.new` and render the `Entity<PdfView>` like any child view.
//!
//! ```no_run
//! # use std::rc::Rc;
//! # use std::path::PathBuf;
//! # use gpui::AppContext;
//! # use gpui_pdf::{PdfView, PdfStyle};
//! # fn demo(cx: &mut gpui::App, path: PathBuf) {
//! let view = cx.new(|cx| {
//!     PdfView::new(path, Rc::new(PdfStyle::default), Rc::new(|| 1.0), cx)
//! });
//! // then `view.clone()` into your element tree; call `view.update(cx, |v, cx|
//! // v.release(window, cx))` before dropping it (e.g. when its tab closes).
//! # let _ = view;
//! # }
//! ```

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    Context, FocusHandle, Hsla, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    ParentElement, Render, RenderImage, ScrollHandle, StatefulInteractiveElement, Styled, Window,
    div, hsla, img, point, px,
};
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use image::{Frame, RgbaImage};

#[cfg(feature = "markup")]
mod text;
#[cfg(feature = "markup")]
pub use text::{NormRect, PageText, extract_page_text};

// ─────────────────────────────── Low-level primitives ───────────────────────────────

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

/// Rasterize a single page (0-based) of an already-parsed [`Document`] at `scale`
/// (PDF point-size × this) to a BGRA `RenderImage` composited onto white. Higher
/// scale = sharper but more memory; [`PdfView`] picks `scale` from the display's
/// pixel ratio, zoom, and quality so pages are crisp without wasting memory.
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

/// Base on-screen page width (points) at zoom 1.0; pages keep their aspect ratio.
pub const PAGE_WIDTH: f32 = 820.0;
/// Vertical gap between page slots (matches the column `gap`).
const PAGE_GAP: f32 = 10.0;
/// Top/bottom padding of the page column (matches the column `py`).
const PAGE_PAD_Y: f32 = 16.0;
/// Extra pages to keep rasterized above and below the visible range, so scrolling
/// finds them already rendered (and small wiggles don't thrash render/evict).
const MARGIN: usize = 3;

/// A page's on-screen height for a given column width, preserving aspect ratio.
fn display_height((w, h): (f32, f32), page_width: f32) -> f32 {
    if w > 0.0 {
        page_width * (h / w)
    } else {
        page_width
    }
}

/// The inclusive page-index range `(start, end)` to keep rasterized for the given
/// scroll position: the pages intersecting the viewport, padded by [`MARGIN`]. Pure
/// (mirrors [`PdfView`]'s slot layout) so it's unit-testable. `page_width` is the
/// on-screen column width (base × zoom); `scroll_y` is how far the content is
/// scrolled down (px ≥ 0); `viewport_h` is the visible height (px).
pub fn keep_window(
    dims: &[(f32, f32)],
    page_width: f32,
    scroll_y: f32,
    viewport_h: f32,
) -> (usize, usize) {
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
    for (i, dim) in dims.iter().enumerate() {
        let page_top = y;
        let page_bottom = y + display_height(*dim, page_width);
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

/// The index of the topmost page intersecting the viewport top — the "current" page
/// for a page counter. Pure; mirrors the slot layout.
fn current_page(dims: &[(f32, f32)], page_width: f32, scroll_y: f32) -> usize {
    let top = scroll_y.max(0.0);
    let mut y = PAGE_PAD_Y;
    for (i, dim) in dims.iter().enumerate() {
        let page_bottom = y + display_height(*dim, page_width);
        if page_bottom > top {
            return i;
        }
        y = page_bottom + PAGE_GAP;
    }
    dims.len().saturating_sub(1)
}

/// The y offset (px) of page `index`'s top in the laid-out column. Pure.
fn page_top_y(dims: &[(f32, f32)], page_width: f32, index: usize) -> f32 {
    let mut y = PAGE_PAD_Y;
    for dim in dims.iter().take(index) {
        y += display_height(*dim, page_width) + PAGE_GAP;
    }
    y
}

/// The rasterization scale for one page: enough pixels to fill its on-screen size at
/// the display's pixel ratio × the host's quality multiplier, clamped so high
/// zoom/DPI can't mint runaway bitmaps. Pure. `page_width` is the on-screen column
/// width (base × zoom); `page_pt_width` is the page's width in PDF points.
fn render_scale(page_width: f32, scale_factor: f32, quality: f32, page_pt_width: f32) -> f32 {
    if page_pt_width > 0.0 {
        (page_width * scale_factor * quality / page_pt_width).clamp(0.5, MAX_RENDER_SCALE)
    } else {
        1.5
    }
}

// ─────────────────────────────── Component: PdfView ───────────────────────────────

/// Smallest / largest zoom the viewer allows.
const MIN_ZOOM: f32 = 0.5;
const MAX_ZOOM: f32 = 3.0;
/// Multiplicative step for one zoom-in / zoom-out.
const ZOOM_STEP: f32 = 1.25;
/// Smallest / largest render-quality multiplier the viewer honors.
const MIN_QUALITY: f32 = 0.25;
const MAX_QUALITY: f32 = 3.0;
/// Cap on the rasterization scale, so high zoom on a Retina display can't mint
/// enormous page bitmaps. Above this, pages soften slightly instead of ballooning.
const MAX_RENDER_SCALE: f32 = 4.0;

/// A page's render state. The bitmap is kept while a re-render (zoom / quality
/// change) is in flight, so the page never blanks — gpui rescales the old bitmap
/// until the crisp one lands.
#[derive(Default, Clone)]
struct Slot {
    /// Last rasterized bitmap (may be a stale scale during a re-render); shown scaled
    /// meanwhile. `None` when never rendered or evicted.
    image: Option<Arc<RenderImage>>,
    /// Generation `image` was rendered at; if it differs from the view's generation,
    /// the bitmap is stale and the page is re-rendered (while still shown).
    image_gen: u64,
    /// A background rasterization is in flight (don't spawn another).
    loading: bool,
}

/// Colors for the [`PdfView`] chrome. Map your theme onto this; [`PdfStyle::default`]
/// is a neutral dark palette.
#[derive(Clone, Copy)]
pub struct PdfStyle {
    /// Viewer background.
    pub bg: Hsla,
    /// Page-slot border and the header divider.
    pub border: Hsla,
    /// Background of an unrendered page slot (placeholder) and control hover.
    pub placeholder_bg: Hsla,
    /// "Page N" / "Loading…" placeholder text.
    pub placeholder_fg: Hsla,
    /// Header filename and control text.
    pub header_fg: Hsla,
    /// Header "· N pages" / page counter text.
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

/// Supplies the current render-quality multiplier at paint time (1.0 = native DPI;
/// < 1 is faster and softer, > 1 supersamples). Read like [`PdfStyleFn`], so a host
/// setting change (e.g. a Settings slider) re-renders all open viewers — in every
/// window — automatically. Clamped to a sane range internally.
pub type PdfQualityFn = Rc<dyn Fn() -> f32>;

/// A page-virtualized PDF viewer: a scrollable column of page slots, each sized from
/// the PDF's page dimensions up front (so the scrollbar is correct for the whole
/// document) but only rasterized while near the viewport. Pages scrolled away are
/// freed — CPU pixel buffer *and* GPU atlas texture — so memory is bounded by what's
/// on screen rather than the page count.
///
/// Built-in controls: a header with page navigation (‹ / ›, a click-to-edit page
/// counter you can type a number into) and zoom (−, a percentage that resets to
/// 100%, +); the keyboard shortcuts PageUp / PageDown / Home / End and ⌘=/⌘-/⌘0
/// (when the viewer is focused — click it first); DPI-aware rasterization scaled by
/// the host's [quality](PdfQualityFn) multiplier; and no blanking on zoom/quality
/// changes (the old bitmap is shown, rescaled, until the crisp one lands).
///
/// Construct with [`PdfView::new`] inside `cx.new`; it loads and measures the file
/// off-thread. Render the resulting `Entity<PdfView>` like any child view. Call
/// [`release`](PdfView::release) before dropping it (e.g. when its tab closes) to
/// free the atlas textures gpui won't free on plain drop.
pub struct PdfView {
    path: PathBuf,
    style: PdfStyleFn,
    quality: PdfQualityFn,
    /// The parsed PDF (shared with the background render tasks); `None` until the
    /// off-thread load finishes.
    pdf: Option<Arc<Document>>,
    /// `(width, height)` in points per page — drives page-slot sizing.
    dims: Vec<(f32, f32)>,
    /// Per-page render state; only pages near the viewport hold a bitmap.
    pages: Vec<Slot>,
    scroll: ScrollHandle,
    /// On-screen zoom factor (1.0 = base width). Affects layout and render scale.
    zoom: f32,
    /// The quality multiplier the pages were last rendered at; compared against the
    /// `quality` source each frame to detect a host setting change.
    last_quality: f32,
    /// Bumped whenever the render scale changes (zoom or quality). Visible pages with
    /// an older `image_gen` re-render; in-flight renders from an older generation are
    /// discarded so a stale-scale bitmap never lands.
    generation: u64,
    /// Painted bitmaps awaiting a `drop_image` (which needs a `Window`); drained at
    /// the top of `ensure_window`. Used when a fresh bitmap replaces an old one.
    pending_drops: Vec<Arc<RenderImage>>,
    /// `Some` while the page-number field is being edited (the typed digits).
    page_input: Option<String>,
    focus: FocusHandle,
}

impl PdfView {
    /// Create a viewer for `path`, kicking off the off-thread read + parse + measure.
    /// `style` supplies chrome colors and `quality` the DPI multiplier, both read at
    /// paint time (see [`PdfStyleFn`] / [`PdfQualityFn`]). Call inside
    /// `cx.new(|cx| PdfView::new(path, style, quality, cx))`.
    pub fn new(
        path: PathBuf,
        style: PdfStyleFn,
        quality: PdfQualityFn,
        cx: &mut Context<Self>,
    ) -> Self {
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
            // Store the parsed doc + sizes and give every page an empty slot; the next
            // render runs `ensure_window`, which rasterizes the visible window.
            let _ = this.update(cx, |this, cx| {
                this.pdf = Some(doc);
                this.dims = dims;
                this.pages = vec![Slot::default(); n];
                cx.notify();
            });
        })
        .detach();

        let last_quality = quality().clamp(MIN_QUALITY, MAX_QUALITY);
        Self {
            path,
            style,
            quality,
            pdf: None,
            dims: Vec::new(),
            pages: Vec::new(),
            scroll: ScrollHandle::new(),
            zoom: 1.0,
            last_quality,
            generation: 0,
            pending_drops: Vec::new(),
            page_input: None,
            focus: cx.focus_handle(),
        }
    }

    /// Free every rasterized page — CPU pixel buffer (by dropping the `Arc`s) *and*
    /// the GPU atlas texture. gpui caches one atlas texture per `RenderImage` on
    /// paint and only frees it via `drop_image`; a raw `ImageSource::Render` is never
    /// auto-evicted, so call this before dropping the view or the textures leak.
    pub fn release(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for arc in std::mem::take(&mut self.pending_drops) {
            cx.drop_image(arc, Some(window));
        }
        for slot in std::mem::take(&mut self.pages) {
            if let Some(arc) = slot.image {
                cx.drop_image(arc, Some(window));
            }
        }
    }

    /// Set the zoom factor (clamped), keeping the current page in view. Visible pages
    /// re-render crisp at the new scale; their current bitmaps stay on screen
    /// (rescaled) until the fresh ones land, so nothing blanks.
    pub fn set_zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        let z = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        if (z - self.zoom).abs() < 0.001 {
            return;
        }
        let anchor = self.current_page_index();
        self.zoom = z;
        self.generation = self.generation.wrapping_add(1);
        self.go_to_page(anchor, cx);
        cx.notify();
    }

    /// Zoom in one step.
    pub fn zoom_in(&mut self, cx: &mut Context<Self>) {
        self.set_zoom(self.zoom * ZOOM_STEP, cx);
    }

    /// Zoom out one step.
    pub fn zoom_out(&mut self, cx: &mut Context<Self>) {
        self.set_zoom(self.zoom / ZOOM_STEP, cx);
    }

    /// Reset zoom to 100%.
    pub fn reset_zoom(&mut self, cx: &mut Context<Self>) {
        self.set_zoom(1.0, cx);
    }

    /// Scroll so page `index` (0-based, clamped) is at the top of the viewport.
    pub fn go_to_page(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.dims.is_empty() {
            return;
        }
        let i = index.min(self.dims.len() - 1);
        // Align page `i`'s top with the viewport top so the page counter (which reads
        // the topmost visible page from the scroll offset) agrees with where we land.
        // Page 0 goes to the true document top (keeping its column padding).
        let y = if i == 0 {
            0.0
        } else {
            page_top_y(&self.dims, self.page_width(), i)
        };
        self.scroll.set_offset(point(px(0.0), px(-y)));
        cx.notify();
    }

    /// Go to the next page.
    pub fn next_page(&mut self, cx: &mut Context<Self>) {
        self.go_to_page(self.current_page_index() + 1, cx);
    }

    /// Go to the previous page.
    pub fn prev_page(&mut self, cx: &mut Context<Self>) {
        self.go_to_page(self.current_page_index().saturating_sub(1), cx);
    }

    /// On-screen column width at the current zoom.
    fn page_width(&self) -> f32 {
        PAGE_WIDTH * self.zoom
    }

    /// The topmost visible page index for the current scroll position.
    fn current_page_index(&self) -> usize {
        let scroll_y = f32::from(-self.scroll.offset().y);
        current_page(&self.dims, self.page_width(), scroll_y)
    }

    /// Keep only the pages near the viewport rasterized: render missing / stale
    /// visible pages (at a DPI-, zoom-, and quality-aware scale) and evict the rest.
    /// Called every frame from `render` — cheap (a window calc + slot scan); it only
    /// spawns/evicts when something actually changed. This is what bounds an open
    /// PDF's memory to the on-screen pages instead of the whole document.
    fn ensure_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Free atlas textures retired by an earlier re-render (we have a Window here).
        for arc in std::mem::take(&mut self.pending_drops) {
            cx.drop_image(arc, Some(window));
        }
        if self.dims.is_empty() || self.pdf.is_none() {
            return; // not loaded yet (still reading/parsing)
        }

        // A host quality change invalidates every bitmap (new scale), like a zoom.
        let quality = (self.quality)().clamp(MIN_QUALITY, MAX_QUALITY);
        if (quality - self.last_quality).abs() > 0.01 {
            self.last_quality = quality;
            self.generation = self.generation.wrapping_add(1);
        }

        let page_width = self.page_width();
        let scale_factor = window.scale_factor();
        let scroll_y = f32::from(-self.scroll.offset().y);
        let viewport_h = f32::from(self.scroll.bounds().size.height);
        let (start, end) = keep_window(&self.dims, page_width, scroll_y, viewport_h);
        let generation = self.generation;

        // Decide what to (re-)render and what to evict. A visible page renders if it
        // has no bitmap or a stale-generation one; it keeps showing the old bitmap
        // meanwhile. An off-window page drops its bitmap.
        let mut to_render: Vec<usize> = Vec::new();
        let mut to_evict: Vec<Arc<RenderImage>> = Vec::new();
        for (i, slot) in self.pages.iter_mut().enumerate() {
            let in_window = i >= start && i <= end;
            if in_window {
                if !slot.loading && (slot.image.is_none() || slot.image_gen != generation) {
                    slot.loading = true;
                    to_render.push(i);
                }
            } else if let Some(arc) = slot.image.take() {
                slot.image_gen = 0;
                to_evict.push(arc);
            }
        }
        for arc in to_evict {
            cx.drop_image(arc, Some(window));
        }
        if to_render.is_empty() {
            return;
        }

        // Render the pages closest to the middle of the window first, so the page
        // you're looking at fills in before its neighbors.
        let center = (start + end) / 2;
        to_render.sort_by_key(|&i| (i as i64 - center as i64).unsigned_abs());

        let pdf = self.pdf.clone().unwrap();
        for i in to_render {
            let pdf = pdf.clone();
            let scale = render_scale(page_width, scale_factor, quality, self.dims[i].0);
            cx.spawn(async move |this, cx| {
                let page = cx
                    .background_executor()
                    .spawn(async move { render_page(&pdf, i, scale).ok() })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    // Store only if still wanted: same generation (scale unchanged)
                    // and still inside the viewport window. Otherwise discard — the
                    // bitmap was never painted, so it holds no atlas texture.
                    let in_window = {
                        let pw = this.page_width();
                        let sy = f32::from(-this.scroll.offset().y);
                        let vh = f32::from(this.scroll.bounds().size.height);
                        let (s, e) = keep_window(&this.dims, pw, sy, vh);
                        i >= s && i <= e
                    };
                    let gen_now = this.generation;
                    let mut retired = None;
                    if let Some(slot) = this.pages.get_mut(i) {
                        slot.loading = false;
                        if gen_now == generation
                            && in_window
                            && let Some(img) = page
                        {
                            retired = slot.image.replace(img);
                            slot.image_gen = generation;
                        }
                    }
                    // The replaced bitmap was painted, so its atlas texture must be
                    // freed — defer to the next `ensure_window` (which has a Window).
                    if let Some(old) = retired {
                        this.pending_drops.push(old);
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// One header control button (clickable, hover-highlighted).
    fn control(
        &self,
        id: &'static str,
        label: impl Into<gpui::SharedString>,
    ) -> gpui::Stateful<gpui::Div> {
        let style = (self.style)();
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .min_w(px(20.0))
            .px(px(6.0))
            .py(px(1.0))
            .rounded(px(4.0))
            .cursor_pointer()
            .text_color(style.header_fg)
            .hover(|s| s.bg(style.placeholder_bg))
            .child(label.into())
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
        let page_width = self.page_width();
        let current = current_page(&self.dims, page_width, f32::from(-self.scroll.offset().y));

        let mut col = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(PAGE_GAP))
            .py(px(PAGE_PAD_Y));
        for (i, dim) in self.dims.iter().enumerate() {
            let disp_h = display_height(*dim, page_width);
            let slot = div()
                .w(px(page_width))
                .h(px(disp_h))
                .rounded(px(2.0))
                .border_1()
                .border_color(style.border)
                .overflow_hidden();
            let slot = match self.pages.get(i).and_then(|s| s.image.as_ref()) {
                Some(image) => slot.child(img(image.clone()).w(px(page_width)).h(px(disp_h))),
                // No bitmap yet: a sized placeholder so layout is stable; it fills in
                // once rasterized.
                None => slot
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

        // Click-to-edit page counter: shows "N / total", or the typed digits while
        // editing (Enter jumps, Esc cancels — handled in `on_key_down`).
        let counter_label = match &self.page_input {
            Some(buf) if !buf.is_empty() => format!("{buf} / {total}"),
            Some(_) => format!("⌷ / {total}"),
            None => format!("{} / {total}", current + 1),
        };
        let editing = self.page_input.is_some();
        let counter = div()
            .id("pdf-page-counter")
            .min_w(px(78.0))
            .flex()
            .items_center()
            .justify_center()
            .px(px(8.0))
            .py(px(1.0))
            .rounded(px(4.0))
            .border_1()
            .border_color(if editing {
                style.header_fg
            } else {
                style.border
            })
            .text_color(style.header_muted)
            .cursor_pointer()
            .hover(|s| s.bg(style.placeholder_bg))
            .child(counter_label)
            .on_click(cx.listener(|this, _, window, cx| {
                this.page_input.get_or_insert_with(String::new);
                window.focus(&this.focus, cx);
                cx.notify();
            }));

        // Header: filename · N pages … (spacer) … page nav · zoom.
        let header = div()
            .flex_shrink_0()
            .px(px(16.0))
            .py(px(6.0))
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
            )
            .child(div().flex_1())
            .child(
                self.control("pdf-prev", "‹")
                    .on_click(cx.listener(|this, _, _window, cx| this.prev_page(cx))),
            )
            .child(counter)
            .child(
                self.control("pdf-next", "›")
                    .on_click(cx.listener(|this, _, _window, cx| this.next_page(cx))),
            )
            .child(div().w(px(1.0)).h(px(14.0)).mx(px(4.0)).bg(style.border))
            .child(
                self.control("pdf-zoom-out", "−")
                    .on_click(cx.listener(|this, _, _window, cx| this.zoom_out(cx))),
            )
            .child(
                self.control(
                    "pdf-zoom-reset",
                    format!("{}%", (self.zoom * 100.0).round() as i32),
                )
                .on_click(cx.listener(|this, _, _window, cx| this.reset_zoom(cx))),
            )
            .child(
                self.control("pdf-zoom-in", "+")
                    .on_click(cx.listener(|this, _, _window, cx| this.zoom_in(cx))),
            );

        div()
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(style.bg)
            // Click the viewer to focus it, so the keyboard shortcuts below work.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _ev, window, cx| window.focus(&this.focus, cx)),
            )
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _window, cx| {
                let key = ev.keystroke.key.as_str();
                // Page-number entry mode swallows keys until Enter/Esc.
                if this.page_input.is_some() {
                    match key {
                        "escape" => {
                            this.page_input = None;
                            cx.notify();
                        }
                        "enter" => {
                            let n = this
                                .page_input
                                .as_deref()
                                .and_then(|s| s.trim().parse::<usize>().ok());
                            this.page_input = None;
                            if let Some(n) = n
                                && !this.dims.is_empty()
                            {
                                this.go_to_page(n.saturating_sub(1).min(this.dims.len() - 1), cx);
                            }
                            cx.notify();
                        }
                        "backspace" => {
                            if let Some(b) = this.page_input.as_mut() {
                                b.pop();
                            }
                            cx.notify();
                        }
                        k if k.len() == 1 && k.chars().all(|c| c.is_ascii_digit()) => {
                            if let Some(b) = this.page_input.as_mut()
                                && b.len() < 7
                            {
                                b.push_str(k);
                            }
                            cx.notify();
                        }
                        _ => {}
                    }
                    return;
                }
                let secondary = ev.keystroke.modifiers.secondary();
                match key {
                    "pagedown" => this.next_page(cx),
                    "pageup" => this.prev_page(cx),
                    "home" => this.go_to_page(0, cx),
                    "end" => {
                        let last = this.dims.len().saturating_sub(1);
                        this.go_to_page(last, cx);
                    }
                    "=" | "+" if secondary => this.zoom_in(cx),
                    "-" if secondary => this.zoom_out(cx),
                    "0" if secondary => this.reset_zoom(cx),
                    _ => {}
                }
            }))
            .child(header)
            .child(
                div()
                    .id("pdf-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    // Scrolling doesn't re-run render on its own; notify so the next
                    // frame re-runs `ensure_window` (and updates the page counter).
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
        // Top of the document, ~900px viewport → page 0 visible, ±MARGIN(3).
        assert_eq!(keep_window(&dims, PAGE_WIDTH, 0.0, 900.0), (0, 3));
    }

    #[test]
    fn window_follows_scroll() {
        let dims = letter_pages(10);
        // Scrolled into page 2 (page tops ≈ 16, 1087, 2158, …).
        assert_eq!(keep_window(&dims, PAGE_WIDTH, 2200.0, 900.0), (0, 5));
    }

    #[test]
    fn window_clamps_at_the_end() {
        let dims = letter_pages(10);
        // Scrolled near the bottom: last pages, end clamped to the final index.
        let (start, end) = keep_window(&dims, PAGE_WIDTH, 9000.0, 900.0);
        assert_eq!(end, 9);
        assert!(start >= 5);
    }

    #[test]
    fn empty_doc_is_safe() {
        assert_eq!(keep_window(&[], PAGE_WIDTH, 0.0, 900.0), (0, 0));
    }

    #[test]
    fn current_page_tracks_scroll() {
        let dims = letter_pages(10);
        assert_eq!(current_page(&dims, PAGE_WIDTH, 0.0), 0);
        // Page tops ≈ 16, 1087, 2158; scrolled to 2200 sits in page 2.
        assert_eq!(current_page(&dims, PAGE_WIDTH, 2200.0), 2);
    }

    #[test]
    fn page_top_y_accumulates() {
        let dims = letter_pages(10);
        assert_eq!(page_top_y(&dims, PAGE_WIDTH, 0), PAGE_PAD_Y);
        // page 1 top = pad + one page height + gap.
        let expected = PAGE_PAD_Y + display_height((8.5, 11.0), PAGE_WIDTH) + PAGE_GAP;
        assert!((page_top_y(&dims, PAGE_WIDTH, 1) - expected).abs() < 0.01);
    }

    #[test]
    fn zoom_widens_layout() {
        let dims = letter_pages(3);
        // A wider column pushes later pages further down.
        let one = page_top_y(&dims, PAGE_WIDTH, 2);
        let two = page_top_y(&dims, PAGE_WIDTH * 2.0, 2);
        assert!(two > one);
    }

    #[test]
    fn render_scale_scales_with_dpi_quality_and_clamps() {
        // US-Letter (612pt) at base width, native (1×): ~1.34.
        let s = render_scale(PAGE_WIDTH, 1.0, 1.0, 612.0);
        assert!((s - 1.339).abs() < 0.01);
        // 2× display ratio doubles it.
        assert!((render_scale(PAGE_WIDTH, 2.0, 1.0, 612.0) - 2.0 * s).abs() < 0.01);
        // Quality multiplies too.
        assert!((render_scale(PAGE_WIDTH, 1.0, 2.0, 612.0) - 2.0 * s).abs() < 0.01);
        // Clamped at the top end.
        assert_eq!(render_scale(PAGE_WIDTH, 4.0, 3.0, 100.0), MAX_RENDER_SCALE);
        // Zero-width page falls back, never divides by zero.
        assert_eq!(render_scale(PAGE_WIDTH, 2.0, 1.0, 0.0), 1.5);
    }
}
