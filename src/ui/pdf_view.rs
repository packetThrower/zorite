//! The PDF viewer tab: a scrollable column of page slots. Every page gets a slot
//! sized from the PDF's page dimensions up front (so the scrollbar and layout are
//! correct for the whole document), but only the pages near the viewport are
//! rasterized — `AppView::ensure_pdf_window` renders the visible window and evicts
//! the rest, keeping memory bounded by what's on screen rather than the page count.

use std::path::Path;

use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled,
    div, img, px,
};

use crate::app::{AppView, PageSlot};
use crate::theme;

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
/// scroll position: the pages intersecting the viewport, padded by `MARGIN`. Pure
/// (mirrors the slot layout below) so it's unit-testable. `scroll_y` is how far the
/// content is scrolled down (px ≥ 0); `viewport_h` is the visible height (px).
pub fn keep_window(dims: &[(f32, f32)], scroll_y: f32, viewport_h: f32) -> (usize, usize) {
    if dims.is_empty() {
        return (0, 0);
    }
    // Before the first paint the viewport height is unknown (0); assume a page or
    // so high so the first pages still render.
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

pub fn render(app: &AppView, path: &Path, cx: &mut Context<AppView>) -> impl IntoElement {
    let Some(doc) = app.pdf_docs.get(path) else {
        return loading().into_any_element();
    };
    if doc.dims.is_empty() {
        return loading().into_any_element();
    }

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let total = doc.dims.len();

    let mut col = div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(PAGE_GAP))
        .py(px(PAGE_PAD_Y));
    for (i, (w, h)) in doc.dims.iter().enumerate() {
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
            .border_color(theme::border_subtle())
            .overflow_hidden();
        let slot = match doc.pages.get(i) {
            Some(PageSlot::Ready(page)) => {
                slot.child(img(page.clone()).w(px(PAGE_WIDTH)).h(px(disp_h)))
            }
            // Empty (off-screen / not yet rendered) or Loading: a sized placeholder
            // so layout is stable; it fills in once rasterized.
            _ => slot
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::glass())
                .child(
                    div()
                        .text_color(theme::text_tertiary())
                        .child(format!("Page {}", i + 1)),
                ),
        };
        col = col.child(slot);
    }

    div()
        .size_full()
        .flex()
        .flex_col()
        .bg(theme::bg_content())
        .child(
            div()
                .flex_shrink_0()
                .px(px(16.0))
                .py(px(8.0))
                .border_b_1()
                .border_color(theme::border_subtle())
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .text_size(px(12.0))
                .text_color(theme::text_secondary())
                .child(format!("📄 {name}"))
                .child(
                    div()
                        .text_color(theme::text_tertiary())
                        .child(format!("· {total} pages")),
                ),
        )
        .child(
            div()
                .id("pdf-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&app.pdf_scroll)
                // Scrolling doesn't re-run render on its own; notify so the next
                // frame re-runs `ensure_pdf_window` for the new scroll position.
                .on_scroll_wheel(cx.listener(|_this: &mut AppView, _ev, _window, cx| {
                    cx.notify();
                }))
                .child(col),
        )
        .into_any_element()
}

fn loading() -> impl IntoElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::bg_content())
        .child(
            div()
                .text_color(theme::text_tertiary())
                .child("Loading PDF…"),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

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
