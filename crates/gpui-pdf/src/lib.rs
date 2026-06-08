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
    AnyView, App, AppContext, Context, FocusHandle, Hsla, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, ParentElement, Render, RenderImage, ScrollHandle,
    SharedString, StatefulInteractiveElement, Styled, Window, div, hsla, img, point, px,
};
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use image::{Frame, RgbaImage};

#[cfg(feature = "markup")]
use gpui::{MouseMoveEvent, deferred};

#[cfg(feature = "markup")]
mod text;
#[cfg(feature = "markup")]
pub use text::{NormPoint, NormRect, PageText, Selection, extract_page_text};

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

/// A highlight to draw on the PDF, located by its quote. The host derives these from
/// its own store (e.g. the markdown blocks that link this PDF) and hands them to the
/// viewer via [`PdfView::set_highlights`]; the viewer finds the quote with the text
/// layer and draws a translucent box over each line it spans. (`markup` feature.)
#[cfg(feature = "markup")]
#[derive(Clone)]
pub struct Highlight {
    /// Host identifier, echoed back on click (e.g. to jump to the source note).
    pub id: u64,
    /// 0-based page the quote is on.
    pub page: usize,
    /// The quoted text to locate (matched case- and whitespace-insensitively).
    pub quote: String,
    /// Which occurrence on the page (0-based), for a quote that repeats.
    pub occurrence: usize,
    /// Fill color; drawn translucent.
    pub color: Hsla,
}

/// Invoked with a [`Highlight`]'s `id` when the user clicks it. (`markup` feature.)
#[cfg(feature = "markup")]
pub type HighlightClickFn = Rc<dyn Fn(u64, &mut Window, &mut gpui::App)>;

/// Invoked when the user finishes a drag-selection in "highlight mode": the page
/// (0-based), the selected one-line quote, which occurrence of it on the page, and the
/// label of the picked color (the opaque tag from [`set_highlight_palette`], for the
/// host to store). The host turns this into a stored note. (`markup` feature.)
#[cfg(feature = "markup")]
pub type CreateHighlightFn =
    Rc<dyn Fn(usize, String, usize, SharedString, &mut Window, &mut gpui::App)>;

/// Cache state for a page's extracted text layer. (`markup` feature.)
#[cfg(feature = "markup")]
enum TextSlot {
    Loading,
    Ready(PageText),
    Failed,
}

/// One find-in-PDF match: the page it's on and one normalized rect per line it spans.
/// (`search` feature.)
#[cfg(feature = "search")]
struct SearchMatch {
    page: usize,
    rects: Vec<NormRect>,
}

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
    /// Highlights to draw (markup), provided by the host.
    #[cfg(feature = "markup")]
    highlights: Vec<Highlight>,
    /// Per-page extracted text layer, built lazily for pages with highlights.
    #[cfg(feature = "markup")]
    page_text: std::collections::HashMap<usize, TextSlot>,
    /// Click handler for a highlight (markup).
    #[cfg(feature = "markup")]
    on_highlight: Option<HighlightClickFn>,
    /// "Highlight mode": dragging over text selects + creates a highlight (markup).
    #[cfg(feature = "markup")]
    selecting: bool,
    /// In-progress drag selection: (page, start, current) in normalized coords.
    #[cfg(feature = "markup")]
    sel_drag: Option<(usize, NormPoint, NormPoint)>,
    /// Called when a drag-selection finishes, so the host stores the note (markup).
    #[cfg(feature = "markup")]
    on_create: Option<CreateHighlightFn>,
    /// Host-supplied highlight colors `(label, fill)`; the picker shows these and the
    /// label is echoed back on create. Empty → a single default yellow.
    #[cfg(feature = "markup")]
    palette: Vec<(SharedString, Hsla)>,
    /// Index into `palette` for new highlights.
    #[cfg(feature = "markup")]
    active_color: usize,
    /// Whether the color picker dropdown is showing.
    #[cfg(feature = "markup")]
    palette_open: bool,
    /// Page whose highlights are briefly flashing (after a jump from a note), if any.
    #[cfg(feature = "markup")]
    flash: Option<usize>,
    /// Bumped on each reveal; the deferred clear no-ops if a newer flash superseded it.
    #[cfg(feature = "markup")]
    flash_gen: u64,
    /// A reveal requested before the document finished loading; applied once it does.
    #[cfg(feature = "markup")]
    pending_reveal: Option<usize>,
    /// Whether the find-in-PDF bar is open. (`search` feature.)
    #[cfg(feature = "search")]
    search_open: bool,
    /// The current search query (edited in the find bar).
    #[cfg(feature = "search")]
    search_query: String,
    /// Matches across the document, in reading order (page, then top-to-bottom).
    #[cfg(feature = "search")]
    matches: Vec<SearchMatch>,
    /// Index into `matches` of the focused match (the one ↑/↓/Enter cycle through).
    #[cfg(feature = "search")]
    current_match: Option<usize>,
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
                // A note→PDF jump that arrived before the document loaded: apply it now.
                #[cfg(feature = "markup")]
                if let Some(p) = this.pending_reveal.take() {
                    this.reveal_highlight(p, cx);
                }
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
            #[cfg(feature = "markup")]
            highlights: Vec::new(),
            #[cfg(feature = "markup")]
            page_text: std::collections::HashMap::new(),
            #[cfg(feature = "markup")]
            on_highlight: None,
            #[cfg(feature = "markup")]
            selecting: false,
            #[cfg(feature = "markup")]
            sel_drag: None,
            #[cfg(feature = "markup")]
            on_create: None,
            #[cfg(feature = "markup")]
            palette: Vec::new(),
            #[cfg(feature = "markup")]
            active_color: 0,
            #[cfg(feature = "markup")]
            palette_open: false,
            #[cfg(feature = "markup")]
            flash: None,
            #[cfg(feature = "markup")]
            flash_gen: 0,
            #[cfg(feature = "markup")]
            pending_reveal: None,
            #[cfg(feature = "search")]
            search_open: false,
            #[cfg(feature = "search")]
            search_query: String::new(),
            #[cfg(feature = "search")]
            matches: Vec::new(),
            #[cfg(feature = "search")]
            current_match: None,
        }
    }

    /// Set the highlights to draw — the host derives these from its own store (e.g.
    /// the markdown blocks that link this PDF). Pages with highlights extract their
    /// text layer lazily as they scroll into view, then each quote is located and
    /// boxed. (`markup` feature.)
    #[cfg(feature = "markup")]
    pub fn set_highlights(&mut self, highlights: Vec<Highlight>, cx: &mut Context<Self>) {
        self.highlights = highlights;
        cx.notify();
    }

    /// Set the handler invoked with a highlight's `id` when it's clicked (e.g. to jump
    /// to the source note). (`markup` feature.)
    #[cfg(feature = "markup")]
    pub fn set_on_highlight(&mut self, handler: HighlightClickFn) {
        self.on_highlight = Some(handler);
    }

    /// Set the handler invoked when a drag-selection finishes. (`markup` feature.)
    #[cfg(feature = "markup")]
    pub fn set_on_create_highlight(&mut self, handler: CreateHighlightFn) {
        self.on_create = Some(handler);
    }

    /// Toggle "highlight mode": when on, dragging over text selects it and fires the
    /// create handler instead of doing nothing. (`markup` feature.)
    #[cfg(feature = "markup")]
    pub fn toggle_select_mode(&mut self, cx: &mut Context<Self>) {
        self.selecting = !self.selecting;
        self.sel_drag = None;
        // Turning highlight mode on pops the color picker down; off hides it.
        self.palette_open = self.selecting && !self.palette.is_empty();
        cx.notify();
    }

    /// Set the highlight colors the picker offers, as `(label, fill)` pairs. The label
    /// is opaque to the viewer — it's echoed back via [`CreateHighlightFn`] so the host
    /// can store it (and map it back to a fill for [`set_highlights`]). (`markup`.)
    #[cfg(feature = "markup")]
    pub fn set_highlight_palette(
        &mut self,
        palette: Vec<(SharedString, Hsla)>,
        cx: &mut Context<Self>,
    ) {
        self.palette = palette;
        if self.active_color >= self.palette.len() {
            self.active_color = 0;
        }
        cx.notify();
    }

    /// The fill of the currently-selected palette color (default yellow if unset).
    #[cfg(feature = "markup")]
    fn active_color_hsla(&self) -> Hsla {
        self.palette
            .get(self.active_color)
            .map(|(_, c)| *c)
            .unwrap_or_else(|| hsla(0.14, 0.95, 0.55, 1.0))
    }

    /// The label of the currently-selected palette color (empty if unset).
    #[cfg(feature = "markup")]
    fn active_color_name(&self) -> SharedString {
        self.palette
            .get(self.active_color)
            .map(|(n, _)| n.clone())
            .unwrap_or_default()
    }

    /// Jump to a highlight from its note: scroll `page` into view (bringing its first
    /// highlight near the top when that page's text is already extracted) and briefly
    /// flash the page's highlights so the eye finds them. (`markup` feature.)
    #[cfg(feature = "markup")]
    pub fn reveal_highlight(&mut self, page: usize, cx: &mut Context<Self>) {
        if self.dims.is_empty() {
            // The document is still loading; apply the jump once it's measured.
            self.pending_reveal = Some(page);
            return;
        }
        let page = page.min(self.dims.len() - 1);
        let pw = self.page_width();
        // Default to the page top (like `go_to_page`); if the text is ready, scroll so
        // the first highlight on the page sits just below the viewport top.
        let mut y = if page == 0 {
            0.0
        } else {
            page_top_y(&self.dims, pw, page)
        };
        if let Some(TextSlot::Ready(pt)) = self.page_text.get(&page)
            && let Some(h) = self.highlights.iter().find(|h| h.page == page)
            && let Some(r) = pt.locate(&h.quote, h.occurrence).first()
        {
            let disp_h = display_height(self.dims[page], pw);
            y = (page_top_y(&self.dims, pw, page) + r.y * disp_h - 48.0).max(0.0);
        }
        self.scroll.set_offset(point(px(0.0), px(-y)));
        // Flash, then clear after a beat (unless a newer reveal supersedes this one).
        self.flash = Some(page);
        self.flash_gen = self.flash_gen.wrapping_add(1);
        let token = self.flash_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(1200))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.flash_gen == token {
                    this.flash = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    // ───────────────────────────── Find-in-PDF (search) ─────────────────────────────

    /// Toggle the find bar. On open, extract every page's text (off-thread, cached)
    /// and compute matches; on close, drop them. (`search` feature.)
    #[cfg(feature = "search")]
    pub fn toggle_search(&mut self, cx: &mut Context<Self>) {
        self.search_open = !self.search_open;
        if self.search_open {
            self.ensure_all_text(cx);
            self.recompute_matches(true, cx);
            if let Some(i) = self.current_match {
                self.goto_match(i, cx);
            }
        } else {
            self.matches.clear();
            self.current_match = None;
        }
        cx.notify();
    }

    /// Close the find bar and clear matches. (`search` feature.)
    #[cfg(feature = "search")]
    pub fn close_search(&mut self, cx: &mut Context<Self>) {
        self.search_open = false;
        self.matches.clear();
        self.current_match = None;
        cx.notify();
    }

    /// Re-run the search after the query changed: recompute matches and jump to the
    /// first one. (`search` feature.)
    #[cfg(feature = "search")]
    fn on_search_query_changed(&mut self, cx: &mut Context<Self>) {
        self.ensure_all_text(cx);
        self.recompute_matches(true, cx);
        if let Some(i) = self.current_match {
            self.goto_match(i, cx);
        }
    }

    /// Kick off text extraction for every page (idempotent, cached), so a search sees
    /// pages that were never scrolled into view. (`search` feature.)
    #[cfg(feature = "search")]
    fn ensure_all_text(&mut self, cx: &mut Context<Self>) {
        for p in 0..self.dims.len() {
            self.ensure_page_text(p, cx);
        }
    }

    /// Rebuild the match list from every page whose text is ready. With
    /// `reset_current`, focus the first match; otherwise keep the focused match (by
    /// page + position) across the rebuild, so a mid-sweep refresh doesn't jump. (`search`.)
    #[cfg(feature = "search")]
    fn recompute_matches(&mut self, reset_current: bool, cx: &mut Context<Self>) {
        let prev = if reset_current {
            None
        } else {
            self.current_match
                .and_then(|i| self.matches.get(i))
                .map(|m| (m.page, m.rects.first().map(|r| r.y).unwrap_or(0.0)))
        };
        self.matches.clear();
        let q = self.search_query.trim().to_string();
        if !q.is_empty() {
            for page in 0..self.dims.len() {
                if let Some(TextSlot::Ready(pt)) = self.page_text.get(&page) {
                    for rects in pt.find_matches(&q) {
                        if !rects.is_empty() {
                            self.matches.push(SearchMatch { page, rects });
                        }
                    }
                }
            }
        }
        self.current_match = if self.matches.is_empty() {
            None
        } else if let Some((pg, y)) = prev {
            self.matches
                .iter()
                .enumerate()
                .filter(|(_, m)| m.page == pg)
                .min_by(|(_, a), (_, b)| {
                    let da = (a.rects.first().map(|r| r.y).unwrap_or(0.0) - y).abs();
                    let db = (b.rects.first().map(|r| r.y).unwrap_or(0.0) - y).abs();
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(i, _)| i)
                .or(Some(0))
        } else {
            // Fresh query: start from the page the user is on, not the document top.
            self.match_from_viewport()
        };
        cx.notify();
    }

    /// The index of the first match at or below the current viewport top, so opening
    /// the find bar (or editing the query) starts from the page being read rather than
    /// the start of the document. Wraps to the first match if none are below. Matches
    /// are in reading order, so the first one past the fold is just `position`. (`search`.)
    #[cfg(feature = "search")]
    fn match_from_viewport(&self) -> Option<usize> {
        if self.matches.is_empty() {
            return None;
        }
        let pw = self.page_width();
        let scroll_y = f32::from(-self.scroll.offset().y).max(0.0);
        let idx = self.matches.iter().position(|m| {
            let ry = m.rects.first().map(|r| r.y).unwrap_or(0.0);
            page_top_y(&self.dims, pw, m.page) + ry * display_height(self.dims[m.page], pw)
                >= scroll_y
        });
        Some(idx.unwrap_or(0))
    }

    /// Focus the next match (wrapping) and scroll to it. (`search` feature.)
    #[cfg(feature = "search")]
    pub fn next_match(&mut self, cx: &mut Context<Self>) {
        if self.matches.is_empty() {
            return;
        }
        let n = self.matches.len();
        let i = self.current_match.map_or(0, |c| (c + 1) % n);
        self.current_match = Some(i);
        self.goto_match(i, cx);
    }

    /// Focus the previous match (wrapping) and scroll to it. (`search` feature.)
    #[cfg(feature = "search")]
    pub fn prev_match(&mut self, cx: &mut Context<Self>) {
        if self.matches.is_empty() {
            return;
        }
        let n = self.matches.len();
        let i = self.current_match.map_or(0, |c| (c + n - 1) % n);
        self.current_match = Some(i);
        self.goto_match(i, cx);
    }

    /// Bring match `idx` into view — but only scroll if it isn't already comfortably
    /// visible, so starting a search on the page you're reading doesn't yank it around.
    /// When it does scroll, the match lands a little below the viewport top. (`search`.)
    #[cfg(feature = "search")]
    fn goto_match(&mut self, idx: usize, cx: &mut Context<Self>) {
        if self.dims.is_empty() {
            return;
        }
        let Some(m) = self.matches.get(idx) else {
            return;
        };
        let page = m.page.min(self.dims.len() - 1);
        let ry = m.rects.first().map(|r| r.y).unwrap_or(0.0);
        let rh = m.rects.first().map(|r| r.h).unwrap_or(0.0);
        let pw = self.page_width();
        let disp_h = display_height(self.dims[page], pw);
        let top = page_top_y(&self.dims, pw, page) + ry * disp_h;
        let bottom = top + rh * disp_h;
        let scroll_y = f32::from(-self.scroll.offset().y).max(0.0);
        let viewport_h = f32::from(self.scroll.bounds().size.height).max(1.0);
        if top < scroll_y + 8.0 || bottom > scroll_y + viewport_h - 8.0 {
            let y = (top - 80.0).max(0.0);
            self.scroll.set_offset(point(px(0.0), px(-y)));
        }
        cx.notify();
    }

    /// Map a window-space point to the page it's over and normalized page coords.
    /// (`markup` feature.)
    #[cfg(feature = "markup")]
    fn point_to_page(&self, pos: gpui::Point<gpui::Pixels>) -> Option<(usize, NormPoint)> {
        if self.dims.is_empty() {
            return None;
        }
        let page_width = self.page_width();
        let b = self.scroll.bounds();
        let scroll_y = f32::from(-self.scroll.offset().y);
        // The page column is centered horizontally in the viewport.
        let col_left =
            f32::from(b.origin.x) + (f32::from(b.size.width) - page_width).max(0.0) / 2.0;
        let content_y = scroll_y + (f32::from(pos.y) - f32::from(b.origin.y));
        let local_x = f32::from(pos.x) - col_left;
        let mut y = PAGE_PAD_Y;
        for (i, dim) in self.dims.iter().enumerate() {
            let h = display_height(*dim, page_width);
            if content_y >= y && content_y < y + h {
                return Some((
                    i,
                    NormPoint {
                        x: (local_x / page_width).clamp(0.0, 1.0),
                        y: ((content_y - y) / h).clamp(0.0, 1.0),
                    },
                ));
            }
            y += h + PAGE_GAP;
        }
        None
    }

    /// Extract `page`'s text layer off-thread (cached), so its highlights can be
    /// located on the next frame. (`markup` feature.)
    #[cfg(feature = "markup")]
    fn ensure_page_text(&mut self, page: usize, cx: &mut Context<Self>) {
        if self.page_text.contains_key(&page) {
            return;
        }
        let Some(pdf) = self.pdf.clone() else {
            return;
        };
        self.page_text.insert(page, TextSlot::Loading);
        cx.spawn(async move |this, cx| {
            let extracted = cx
                .background_executor()
                .spawn(async move { extract_page_text(&pdf, page) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.page_text.insert(
                    page,
                    match extracted {
                        Some(pt) => TextSlot::Ready(pt),
                        None => TextSlot::Failed,
                    },
                );
                cx.notify();
                // If a search is running and this was the last page to extract, fold in
                // its matches (keeping the focused one). Doing it once at the end keeps
                // the whole-document sweep from re-searching on every page.
                #[cfg(feature = "search")]
                if this.search_open
                    && !this.search_query.trim().is_empty()
                    && !this
                        .page_text
                        .values()
                        .any(|s| matches!(s, TextSlot::Loading))
                {
                    this.recompute_matches(false, cx);
                }
            });
        })
        .detach();
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

        // Extract the text layer (off-thread, cached) for visible pages that need it:
        // pages with highlights, so they can be located + drawn — and, while in
        // highlight mode, *every* visible page, so a drag can select text even on a
        // page that has no highlights yet. Before the early-out below, so an
        // already-rendered page still gets its text extracted.
        #[cfg(feature = "markup")]
        {
            let mut pages: Vec<usize> = self
                .highlights
                .iter()
                .map(|h| h.page)
                .filter(|p| (start..=end).contains(p))
                .collect();
            if self.selecting {
                pages.extend(start..=end);
            }
            pages.sort_unstable();
            pages.dedup();
            for p in pages {
                self.ensure_page_text(p, cx);
            }
        }

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

    /// Build a `.tooltip(..)` closure for a header control. gpui core has the tooltip
    /// *hook* but no tooltip *view* (those live in higher-level UI crates we don't
    /// depend on), so we render a small themed one ([`Tip`]), reading colors through
    /// the same style closure at show time.
    fn tip(
        &self,
        text: impl Into<SharedString>,
    ) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
        let style_fn = self.style.clone();
        let text = text.into();
        move |_window, cx| {
            let s = style_fn();
            let text = text.clone();
            cx.new(move |_| Tip {
                text,
                fg: s.header_fg,
                bg: s.placeholder_bg,
                border: s.border,
            })
            .into()
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
                .relative()
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
            // Markup: overlay a translucent, clickable box on each line of every
            // located quote on this page.
            #[cfg(feature = "markup")]
            let slot = {
                let mut slot = slot;
                if let Some(TextSlot::Ready(pt)) = self.page_text.get(&i) {
                    // Brighten + outline the page's highlights briefly after a jump from
                    // a note, so the clicked one is easy to spot.
                    let flashing = self.flash == Some(i);
                    for h in self.highlights.iter().filter(|h| h.page == i) {
                        let fill = Hsla {
                            a: if flashing { 0.6 } else { 0.35 },
                            ..h.color
                        };
                        for (ri, r) in pt.locate(&h.quote, h.occurrence).into_iter().enumerate() {
                            let id = h.id;
                            let mut hl = div()
                                .id(gpui::SharedString::from(format!(
                                    "pdf-hl-{i}-{}-{ri}",
                                    h.id
                                )))
                                .absolute()
                                .left(px(r.x * page_width))
                                .top(px(r.y * disp_h))
                                .w(px(r.w * page_width))
                                .h(px(r.h * disp_h))
                                .rounded(px(1.0))
                                .bg(fill)
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if let Some(cb) = this.on_highlight.clone() {
                                        cb(id, window, cx);
                                    }
                                }));
                            if flashing {
                                hl = hl.border_1().border_color(Hsla { a: 0.95, ..h.color });
                            }
                            slot = slot.child(hl);
                        }
                    }
                }
                // Live drag-selection feedback (highlight mode).
                if let Some((pg, a, b)) = self.sel_drag
                    && pg == i
                    && let Some(TextSlot::Ready(pt)) = self.page_text.get(&i)
                    && let Some(sel) = pt.select(a, b)
                {
                    for r in sel.rects {
                        slot = slot.child(
                            div()
                                .absolute()
                                .left(px(r.x * page_width))
                                .top(px(r.y * disp_h))
                                .w(px(r.w * page_width))
                                .h(px(r.h * disp_h))
                                .rounded(px(1.0))
                                .bg(hsla(0.58, 0.9, 0.55, 0.3)),
                        );
                    }
                }
                // Find-in-PDF: box every match on this page; emphasize the focused one.
                #[cfg(feature = "search")]
                for (mi, m) in self.matches.iter().enumerate() {
                    if m.page != i {
                        continue;
                    }
                    let current = self.current_match == Some(mi);
                    let fill = hsla(0.09, 0.95, 0.5, if current { 0.55 } else { 0.3 });
                    for r in &m.rects {
                        let mut b = div()
                            .absolute()
                            .left(px(r.x * page_width))
                            .top(px(r.y * disp_h))
                            .w(px(r.w * page_width))
                            .h(px(r.h * disp_h))
                            .rounded(px(1.0))
                            .bg(fill);
                        if current {
                            b = b.border_1().border_color(hsla(0.09, 0.95, 0.4, 0.95));
                        }
                        slot = slot.child(b);
                    }
                }
                slot
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
            }))
            .tooltip(self.tip("Go to page…"));

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
                    .on_click(cx.listener(|this, _, _window, cx| this.prev_page(cx)))
                    .tooltip(self.tip("Previous page")),
            )
            .child(counter)
            .child(
                self.control("pdf-next", "›")
                    .on_click(cx.listener(|this, _, _window, cx| this.next_page(cx)))
                    .tooltip(self.tip("Next page")),
            )
            .child(div().w(px(1.0)).h(px(14.0)).mx(px(4.0)).bg(style.border))
            .child(
                self.control("pdf-zoom-out", "−")
                    .on_click(cx.listener(|this, _, _window, cx| this.zoom_out(cx)))
                    .tooltip(self.tip("Zoom out")),
            )
            .child(
                self.control(
                    "pdf-zoom-reset",
                    format!("{}%", (self.zoom * 100.0).round() as i32),
                )
                .on_click(cx.listener(|this, _, _window, cx| this.reset_zoom(cx)))
                .tooltip(self.tip("Reset zoom")),
            )
            .child(
                self.control("pdf-zoom-in", "+")
                    .on_click(cx.listener(|this, _, _window, cx| this.zoom_in(cx)))
                    .tooltip(self.tip("Zoom in")),
            );

        // Highlight-mode toggle + color picker (markup): the pen turns drag-to-select
        // on and pops a palette down; the active color shows as a chip beneath it.
        #[cfg(feature = "markup")]
        let header = {
            let mark_bg = if self.selecting {
                style.placeholder_bg
            } else {
                Hsla { a: 0.0, ..style.bg }
            };
            let active = self.active_color_hsla();
            let pen = div()
                .id("pdf-mark")
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(1.0))
                .min_w(px(20.0))
                .px(px(6.0))
                .py(px(1.0))
                .rounded(px(4.0))
                .cursor_pointer()
                .text_color(style.header_fg)
                .bg(mark_bg)
                .hover(|s| s.bg(style.placeholder_bg))
                .child("✎")
                .child(div().w(px(12.0)).h(px(2.0)).rounded(px(1.0)).bg(active))
                .on_click(cx.listener(|this, _, _window, cx| this.toggle_select_mode(cx)))
                .tooltip(self.tip("Highlight — pick a color"));

            // Color-picker dropdown, deferred so it paints over the page area below.
            let dropdown = if self.palette_open && !self.palette.is_empty() {
                let mut row = div()
                    .absolute()
                    .top(px(30.0))
                    .right(px(0.0))
                    .flex()
                    .flex_row()
                    .gap_1()
                    .p(px(5.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(style.border)
                    .bg(style.bg);
                for (i, (name, color)) in self.palette.iter().enumerate() {
                    let selected = i == self.active_color;
                    let color = *color;
                    row = row.child(
                        div()
                            .id(SharedString::from(format!("pdf-swatch-{i}")))
                            .w(px(16.0))
                            .h(px(16.0))
                            .rounded(px(8.0))
                            .bg(color)
                            .border_2()
                            .border_color(if selected {
                                style.header_fg
                            } else {
                                Hsla { a: 0.0, ..style.bg }
                            })
                            .cursor_pointer()
                            .tooltip(self.tip(name.clone()))
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.active_color = i;
                                this.selecting = true;
                                this.palette_open = false;
                                cx.notify();
                            })),
                    );
                }
                Some(deferred(row))
            } else {
                None
            };

            header.child(div().relative().child(pen).children(dropdown))
        };

        // Find toggle (search): a magnifier that opens the find bar.
        #[cfg(feature = "search")]
        let header = {
            let bg = if self.search_open {
                style.placeholder_bg
            } else {
                Hsla { a: 0.0, ..style.bg }
            };
            header.child(
                self.control("pdf-find", "🔍")
                    .bg(bg)
                    .on_click(cx.listener(|this, _, _window, cx| this.toggle_search(cx)))
                    .tooltip(self.tip("Find (⌘F)")),
            )
        };

        let root = div()
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(style.bg)
            // Click the viewer to focus it (so keyboard shortcuts work); in highlight
            // mode a mouse-down also starts a drag selection.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _ev: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    #[cfg(feature = "markup")]
                    if this.selecting
                        && let Some((pg, n)) = this.point_to_page(_ev.position)
                    {
                        this.sel_drag = Some((pg, n, n));
                        cx.notify();
                    }
                }),
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
                #[cfg(feature = "search")]
                {
                    // ⌘F / Ctrl-F toggles the find bar.
                    if secondary && key == "f" {
                        this.toggle_search(cx);
                        return;
                    }
                    // While the bar is open, type to edit the query and Enter/⇧Enter to
                    // step matches. Keys we don't consume (arrows, PageUp/Down…) fall
                    // through, so the page still scrolls with the bar open.
                    if this.search_open {
                        match key {
                            "escape" => {
                                this.close_search(cx);
                                return;
                            }
                            "enter" => {
                                if ev.keystroke.modifiers.shift {
                                    this.prev_match(cx);
                                } else {
                                    this.next_match(cx);
                                }
                                return;
                            }
                            "backspace" => {
                                this.search_query.pop();
                                this.on_search_query_changed(cx);
                                return;
                            }
                            _ => {
                                if let Some(ch) =
                                    ev.keystroke.key_char.as_ref().filter(|s| {
                                        !s.is_empty() && !s.chars().any(char::is_control)
                                    })
                                {
                                    this.search_query.push_str(ch);
                                    this.on_search_query_changed(cx);
                                    return;
                                }
                            }
                        }
                    }
                }
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
            }));

        // Highlight-mode drag handlers (markup): update the selection on move, and on
        // release resolve it to a quote and hand it to the host to store.
        #[cfg(feature = "markup")]
        let root = root
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _window, cx| {
                let Some((pg, start, _)) = this.sel_drag else {
                    return;
                };
                if let Some((p, n)) = this.point_to_page(ev.position)
                    && p == pg
                {
                    this.sel_drag = Some((pg, start, n));
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _ev, window, cx| {
                    let Some((pg, a, b)) = this.sel_drag.take() else {
                        return;
                    };
                    let sel = match this.page_text.get(&pg) {
                        Some(TextSlot::Ready(pt)) => pt.select(a, b),
                        _ => None,
                    };
                    if let Some(sel) = sel
                        && let Some(cb) = this.on_create.clone()
                    {
                        let color = this.active_color_name();
                        cb(pg, sel.quote, sel.occurrence, color, window, cx);
                    }
                    cx.notify();
                }),
            );

        // Find bar overlay (search): a floating bar with the query, match count, and
        // prev/next/close. Deferred so it paints over the page area below the header.
        #[cfg(feature = "search")]
        let root = if self.search_open {
            let count = if self.search_query.trim().is_empty() {
                // Empty query: the field already shows the "Find…" placeholder, so the
                // count reads "0 / 0" rather than repeating it.
                "0 / 0".to_string()
            } else if self
                .page_text
                .values()
                .any(|s| matches!(s, TextSlot::Loading))
            {
                "searching…".to_string()
            } else if self.matches.is_empty() {
                "no results".to_string()
            } else {
                format!(
                    "{} / {}",
                    self.current_match.map_or(0, |i| i + 1),
                    self.matches.len()
                )
            };
            let has_query = !self.search_query.is_empty();
            // Query field with a caret. It's static: a blinking caret would need either a
            // focused text widget or a per-frame animation that re-renders the whole
            // viewer; the viewer captures keystrokes directly instead.
            let field = div()
                .min_w(px(120.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(1.0));
            let caret = || {
                div()
                    .w(px(1.5))
                    .h(px(13.0))
                    .rounded(px(1.0))
                    .bg(style.header_fg)
            };
            let field = if has_query {
                field
                    .text_color(style.header_fg)
                    .child(SharedString::from(self.search_query.clone()))
                    .child(caret())
            } else {
                field
                    .child(caret())
                    .child(div().text_color(style.header_muted).child("Find…"))
            };
            root.child(deferred(
                div()
                    .absolute()
                    .top(px(44.0))
                    .right(px(12.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(style.border)
                    .bg(style.bg)
                    .text_size(px(12.0))
                    .child(field)
                    .child(div().text_color(style.header_muted).child(count))
                    .child(
                        self.control("pdf-find-prev", "‹")
                            .on_click(cx.listener(|this, _, _w, cx| this.prev_match(cx)))
                            .tooltip(self.tip("Previous match (⇧⏎)")),
                    )
                    .child(
                        self.control("pdf-find-next", "›")
                            .on_click(cx.listener(|this, _, _w, cx| this.next_match(cx)))
                            .tooltip(self.tip("Next match (⏎)")),
                    )
                    .child(
                        self.control("pdf-find-close", "✕")
                            .on_click(cx.listener(|this, _, _w, cx| this.close_search(cx)))
                            .tooltip(self.tip("Close (Esc)")),
                    ),
            ))
        } else {
            root
        };

        root.child(header)
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

/// A minimal themed tooltip view — gpui's `.tooltip()` takes any view, and we don't
/// pull in a UI crate just for one label.
struct Tip {
    text: SharedString,
    fg: Hsla,
    bg: Hsla,
    border: Hsla,
}

impl Render for Tip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // gpui anchors the tooltip's top-left at the mouse position (+1px), i.e. *inside*
        // the hovered control. A transparent top padding on the root shifts the visible
        // box down to clear the control + its bar/header padding. (Padding is applied to
        // a `layout_as_root` element; a top *margin* on the root is ignored.)
        div().pt(px(22.0)).child(
            div()
                .px(px(6.0))
                .py(px(2.0))
                .rounded(px(4.0))
                .border_1()
                .border_color(self.border)
                .bg(self.bg)
                .text_color(self.fg)
                .text_size(px(11.0))
                .child(self.text.clone()),
        )
    }
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
