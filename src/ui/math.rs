//! Rendering a `$$…$$` math block. `gpui-markdown` detects the block and hands the LaTeX
//! here (via [`gpui_markdown::MathRenderer`]); the app owns the render so the renderer stays
//! host-agnostic. Shows the cached formula, a "typesetting…" placeholder (which kicks off the
//! off-thread render the first time it paints), or the raw LaTeX on failure.

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use gpui::{
    AnyElement, Bounds, ImageSource, InteractiveElement, IntoElement, MouseButton, ParentElement,
    Pixels, SharedString, Styled, WeakEntity, canvas, div, img, px,
};
use gpui_markdown::MathRenderer;

use crate::app::AppView;
use crate::math::MathStore;
use crate::theme;

/// Build the renderer handed to `MarkdownView::on_math`. Captures the formula cache and a
/// weak `AppView` to drive the off-thread render.
pub fn renderer(app: &AppView, cx: &mut gpui::Context<AppView>) -> MathRenderer {
    let store = app.math_store();
    let weak = cx.entity().downgrade();
    Rc::new(move |source: SharedString| build(source, store.clone(), weak.clone()))
}

fn build(
    source: SharedString,
    store: Rc<RefCell<MathStore>>,
    weak: WeakEntity<AppView>,
) -> AnyElement {
    {
        let store = store.borrow();
        if let Some((image, width, height)) = store.get(&source) {
            let mut hasher = DefaultHasher::new();
            source.hash(&mut hasher);
            let id = hasher.finish() as usize;
            let menu_weak = weak.clone();
            let menu_src = source.clone();
            // Right-click → the formula context menu. `stop_propagation` suppresses the
            // reader view's own (element-level) day/page "Edit" menu over the formula. A
            // left-click isn't handled here, so it bubbles to the markdown's click-to-edit.
            return div()
                .py(px(6.0))
                .child(
                    img(ImageSource::from(image))
                        .id(("math-formula", id))
                        .w(px(width))
                        .h(px(height))
                        .on_mouse_down(MouseButton::Right, move |ev, _window, cx| {
                            cx.stop_propagation();
                            let src = menu_src.clone();
                            let pos = ev.position;
                            let _ = menu_weak.update(cx, |this, cx| {
                                this.open_math_menu(src, pos, cx);
                            });
                        }),
                )
                .into_any_element();
        }
        if store.failed(&source) {
            return code_fallback(&source);
        }
    }
    loading_placeholder(source, weak)
}

/// A sized box shown while a formula typesets. Its `canvas` kicks off the render the first
/// time it paints (the renderer closure has no `cx`); once the bitmap lands, the store
/// reports it ready and this is replaced by the formula.
fn loading_placeholder(source: SharedString, weak: WeakEntity<AppView>) -> AnyElement {
    let trigger = canvas(
        move |_bounds: Bounds<Pixels>, _window, cx| {
            let src = source.clone();
            let _ = weak.update(cx, |this, cx| this.ensure_math_loaded(src, cx));
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0();
    div()
        .py(px(6.0))
        .child(
            div()
                .relative()
                .child(
                    div()
                        .w_full()
                        .h(px(44.0))
                        .rounded(px(6.0))
                        .bg(theme::glass())
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(theme::text_tertiary())
                        .child("Typesetting…"),
                )
                .child(trigger),
        )
        .into_any_element()
}

/// On a render failure, show the raw LaTeX so the user still has the source.
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
