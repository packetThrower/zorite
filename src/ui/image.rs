//! Rendering standalone markdown images. The `gpui-markdown` crate detects an
//! image block and hands it here (via [`gpui_markdown::ImageRenderer`]) so the
//! app — not the host-agnostic renderer — owns image loading and the resize
//! interaction (a draggable corner handle).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    AnyElement, Bounds, CursorStyle, ImageSource, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, Pixels, SharedString, SharedUri, StatefulInteractiveElement,
    Styled, WeakEntity, canvas, div, img, px, relative,
};
use gpui_markdown::{ImageInfo, ImageRenderer};

use crate::app::AppView;
use crate::slash::SlashTarget;
use crate::theme;

/// Build the image renderer handed to `MarkdownView::on_image` for `target`'s
/// editor. Captures the live drag (for width preview), the shared width map
/// (measure callbacks write into it), and a weak `AppView` (handle drag start).
pub fn renderer(
    app: &AppView,
    target: SlashTarget,
    cx: &mut gpui::Context<AppView>,
) -> ImageRenderer {
    let drag = app.image_drag_snapshot();
    let widths = app.image_widths();
    let weak = cx.entity().downgrade();
    Rc::new(move |info: ImageInfo| build(info, target.clone(), drag, widths.clone(), weak.clone()))
}

fn build(
    info: ImageInfo,
    target: SlashTarget,
    drag: Option<(usize, f32)>,
    widths: Rc<RefCell<HashMap<usize, f32>>>,
    weak: WeakEntity<AppView>,
) -> AnyElement {
    // A `![](file.pdf)` reference is a chip that opens the PDF viewer tab.
    if crate::pdf::is_pdf(&info.src) {
        return pdf_chip(&info, weak);
    }
    let Some(source) = image_source(&info.src) else {
        return fallback(&info);
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

    div()
        .py(px(4.0))
        .flex()
        .items_start()
        .child(div().relative().child(image).child(measure).child(handle))
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

/// Resolve a markdown image `src` to a gpui image source. http(s) URLs and
/// `file://` URLs load directly; absolute paths load as-is; relative paths
/// resolve against the data dir (where the managed `images/` folder lives).
fn image_source(src: &str) -> Option<ImageSource> {
    if src.starts_with("http://") || src.starts_with("https://") {
        Some(SharedUri::from(src.to_string()).into())
    } else if let Some(path) = src.strip_prefix("file://") {
        Some(PathBuf::from(path).into())
    } else if src.starts_with('/') {
        Some(PathBuf::from(src).into())
    } else {
        let path = crate::paths::data_dir().join(src);
        path.exists().then(|| path.into())
    }
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
