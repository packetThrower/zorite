//! Rendering a ` ```mermaid ` code block. `gpui-markdown` detects the fence and
//! hands the source here (via [`gpui_markdown::MermaidRenderer`]); the app owns
//! the render so the renderer stays host-agnostic. Shows the cached diagram, a
//! "rendering…" placeholder (which kicks off the off-thread render the first time
//! it paints), or the source text on failure.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AnyElement, Bounds, ImageSource, IntoElement, ParentElement, Pixels, SharedString, Styled,
    WeakEntity, canvas, div, img, px, relative,
};
use gpui_markdown::MermaidRenderer;

use crate::app::AppView;
use crate::mermaid::MermaidStore;
use crate::theme;

/// Build the renderer handed to `MarkdownView::on_mermaid`. Captures the diagram
/// cache and a weak `AppView` to drive the off-thread render.
pub fn renderer(app: &AppView, cx: &mut gpui::Context<AppView>) -> MermaidRenderer {
    let store = app.mermaid_store();
    let weak = cx.entity().downgrade();
    Rc::new(move |source: SharedString| build(source, store.clone(), weak.clone()))
}

fn build(
    source: SharedString,
    store: Rc<RefCell<MermaidStore>>,
    weak: WeakEntity<AppView>,
) -> AnyElement {
    {
        let store = store.borrow();
        if let Some(image) = store.get(&source) {
            return div()
                .py(px(4.0))
                .child(
                    img(ImageSource::from(image))
                        .max_w(relative(1.0))
                        .rounded(px(6.0)),
                )
                .into_any_element();
        }
        if store.failed(&source) {
            return code_fallback(&source);
        }
    }
    loading_placeholder(source, weak)
}

/// A sized box shown while a diagram renders. Its `canvas` kicks off the render
/// the first time it paints (the renderer closure has no `cx`); once the bitmap
/// lands, the store reports it ready and this is replaced by the diagram.
fn loading_placeholder(source: SharedString, weak: WeakEntity<AppView>) -> AnyElement {
    let trigger = canvas(
        move |_bounds: Bounds<Pixels>, _window, cx| {
            let src = source.clone();
            let _ = weak.update(cx, |this, cx| this.ensure_mermaid_loaded(src, cx));
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0();
    div()
        .py(px(4.0))
        .child(
            div()
                .relative()
                .child(
                    div()
                        .w_full()
                        .h(px(72.0))
                        .rounded(px(6.0))
                        .bg(theme::glass())
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(theme::text_tertiary())
                        .child("Rendering diagram…"),
                )
                .child(trigger),
        )
        .into_any_element()
}

/// On a render failure, show the source so the user still has the diagram code.
fn code_fallback(source: &SharedString) -> AnyElement {
    div()
        .w_full()
        .rounded(px(6.0))
        .bg(theme::glass())
        .px(px(12.0))
        .py(px(8.0))
        .text_color(theme::text_tertiary())
        .child(source.clone())
        .into_any_element()
}
