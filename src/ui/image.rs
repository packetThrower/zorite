//! Rendering standalone markdown images. The `gpui-markdown` crate detects an
//! image block and hands it here (via [`gpui_markdown::ImageRenderer`]) so the
//! app — not the host-agnostic renderer — owns image loading and the resize
//! interaction (a draggable corner handle).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyElement, Bounds, CursorStyle, ImageSource, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, Pixels, SharedString, SharedUri, StatefulInteractiveElement,
    Styled, WeakEntity, canvas, div, img, px, relative,
};
use gpui_markdown::{ImageInfo, ImageRenderer};

use crate::app::AppView;
use crate::images::ImageStore;
use crate::slash::SlashTarget;
use crate::theme;

/// Build the image renderer handed to `MarkdownView::on_image` for `target`'s
/// editor. Captures the live drag (for width preview), the shared width map
/// (measure callbacks write into it), the downscaling image cache, and a weak
/// `AppView` (handle drag start / drive image loads).
pub fn renderer(
    app: &AppView,
    target: SlashTarget,
    cx: &mut gpui::Context<AppView>,
) -> ImageRenderer {
    let drag = app.image_drag_snapshot();
    let widths = app.image_widths();
    let store = app.image_store();
    let weak = cx.entity().downgrade();
    Rc::new(move |info: ImageInfo| {
        build(
            info,
            target.clone(),
            drag,
            widths.clone(),
            store.clone(),
            weak.clone(),
        )
    })
}

fn build(
    info: ImageInfo,
    target: SlashTarget,
    drag: Option<(usize, f32)>,
    widths: Rc<RefCell<HashMap<usize, f32>>>,
    store: Rc<RefCell<ImageStore>>,
    weak: WeakEntity<AppView>,
) -> AnyElement {
    // A `![](file.pdf)` reference is a chip that opens the PDF viewer tab.
    if crate::pdf::is_pdf(&info.src) {
        return pdf_chip(&info, weak);
    }
    // Local files render through the downscaling store (decoded at display size,
    // freed on view change); remote URLs use gpui's loader as-is.
    let source = if info.src.starts_with("http://") || info.src.starts_with("https://") {
        ImageSource::from(SharedUri::from(info.src.to_string()))
    } else {
        match local_source(&info, &store, &weak) {
            LocalImage::Ready(source) => source,
            LocalImage::Placeholder(el) => return el,
            LocalImage::Missing => return fallback(&info),
        }
    };
    let attr_start = info.attr_target.start;
    // Live drag width for this image wins; otherwise the saved `{width=N}`.
    let width = match drag {
        Some((k, w)) if k == attr_start => Some(w),
        _ => info.width,
    };
    let mut image = img(source).rounded(px(4.0));
    match width {
        Some(w) => image = image.w(px(w)),
        None => image = image.max_w(relative(1.0)),
    }

    // Measure the rendered image so a drag knows its starting width.
    let measure_widths = widths.clone();
    let measure = canvas(
        move |bounds: Bounds<Pixels>, _, _| {
            measure_widths
                .borrow_mut()
                .insert(attr_start, f32::from(bounds.size.width));
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0();

    // The bottom-right resize grip.
    let attr_target = info.attr_target.clone();
    let handle = div()
        .absolute()
        .bottom(px(-2.0))
        .right(px(-2.0))
        .w(px(14.0))
        .h(px(14.0))
        .rounded(px(3.0))
        .bg(theme::accent())
        .border_2()
        .border_color(theme::bg_content())
        .cursor(CursorStyle::ResizeLeftRight)
        .on_mouse_down(
            MouseButton::Left,
            move |ev: &MouseDownEvent, _window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.start_image_drag(target.clone(), attr_target.clone(), ev.position.x, cx);
                });
            },
        );

    // Wrap the image in a viewport-width horizontal scroll: if it's been resized
    // wider than the content area, it scrolls within its own row (and its resize
    // grip stays reachable) instead of running off the page — while sibling text
    // keeps wrapping at the normal width. `Cmd+Shift+I` fits oversized images back.
    // The inner `flex`/`items_start` sizes the relative holder to the image, so
    // the resize grip pins to the image's corner (not the viewport edge).
    div()
        .py(px(4.0))
        .child(
            div()
                .id(("img-scroll", attr_start))
                .w_full()
                .overflow_x_scroll()
                .flex()
                .items_start()
                .child(
                    // `flex_shrink_0` keeps the holder at the image's width so an
                    // over-wide image overflows (and scrolls) instead of being
                    // squeezed back to fit the rail.
                    div()
                        .relative()
                        .flex_shrink_0()
                        .child(image)
                        .child(measure)
                        .child(handle),
                ),
        )
        .into_any_element()
}

/// A `![](file.pdf)` reference renders as a clickable chip that opens the PDF in
/// its own viewer tab (keeping the note light — the pages live in the viewer).
fn pdf_chip(info: &ImageInfo, weak: WeakEntity<AppView>) -> AnyElement {
    let src = info.src.clone();
    let label = crate::pdf::resolve_path(&src)
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| src.to_string());
    div()
        .id(SharedString::from(format!("pdf-chip:{src}")))
        .my(px(4.0))
        .px_3()
        .py_2()
        .rounded(px(6.0))
        .border_1()
        .border_color(theme::border_subtle())
        .bg(theme::glass())
        .cursor_pointer()
        .hover(|h| h.bg(theme::glass_strong()))
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .child("📄")
        .child(
            div()
                .text_color(theme::accent())
                .child(format!("{label} — open")),
        )
        .on_click(move |_ev, window, cx| {
            if let Some(path) = crate::pdf::resolve_path(&src) {
                let _ = weak.update(cx, |this, cx| this.open_pdf(path, window, cx));
            }
        })
        .into_any_element()
}

/// Outcome of resolving a local image through the downscaling store.
enum LocalImage {
    /// Decoded and ready — render this source.
    Ready(ImageSource),
    /// Still decoding — render this placeholder (which triggers the decode).
    Placeholder(AnyElement),
    /// The file is missing or failed to decode.
    Missing,
}

/// Resolve a local image `src` (a `file://`, absolute, or data-dir-relative
/// ref) against the [`ImageStore`]: a decoded bitmap renders directly; an
/// unknown one becomes a placeholder that kicks off a downscaled decode the
/// first time it paints; a failed/missing one falls back.
fn local_source(
    info: &ImageInfo,
    store: &Rc<RefCell<ImageStore>>,
    weak: &WeakEntity<AppView>,
) -> LocalImage {
    let resolved = crate::paths::resolve_local(&info.src);
    match &resolved {
        Some(p) if p.exists() => {}
        _ => {
            log::warn!("image not found: src={:?} resolved={resolved:?}", info.src);
            return LocalImage::Missing;
        }
    }
    let store_ref = store.borrow();
    if let Some(arc) = store_ref.get(&info.src) {
        LocalImage::Ready(ImageSource::from(arc))
    } else if store_ref.failed(&info.src) {
        LocalImage::Missing
    } else {
        LocalImage::Placeholder(loading_placeholder(info, weak))
    }
}

/// A sized box shown while an image decodes. Its `canvas` triggers the decode
/// the first time it paints (the renderer closure has no `cx`); once the bitmap
/// lands, the store reports it ready and this is replaced by the real image.
fn loading_placeholder(info: &ImageInfo, weak: &WeakEntity<AppView>) -> AnyElement {
    let src = info.src.clone();
    let weak = weak.clone();
    let trigger = canvas(
        move |_bounds: Bounds<Pixels>, _window, cx| {
            let src = src.clone();
            let _ = weak.update(cx, |this, cx| this.ensure_image_loaded(src, cx));
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0();
    let mut box_ = div().rounded(px(4.0)).bg(theme::glass()).h(px(160.0));
    box_ = match info.width {
        Some(w) => box_.w(px(w)),
        None => box_.w(px(260.0)),
    };
    div()
        .py(px(4.0))
        .child(div().relative().child(box_).child(trigger))
        .into_any_element()
}

fn fallback(info: &ImageInfo) -> AnyElement {
    let label = if info.alt.is_empty() {
        "🖼 image (unresolved path)".to_string()
    } else {
        format!("🖼 {}", info.alt)
    };
    div()
        .text_color(theme::text_tertiary())
        .child(label)
        .into_any_element()
}
