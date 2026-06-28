//! The slash-command popup list, rendered as an anchored overlay by `AppView`.
//! Keyboard-driven (arrows + Enter) and mouse-driven (hover highlights a row,
//! click accepts it).

use gpui::{
    Context, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder as _, px,
};

use crate::app::AppView;
use crate::slash::{ItemKind, Slash, Trigger};
use crate::theme;

/// Row height (px) + the height cap, shared with `AppView::scroll_slash_into_view` so the
/// keyboard scroll-into-view and the scrollbar thumb agree on the menu's geometry.
pub const ITEM_H: f32 = 25.0;
pub const MAX_H: f32 = 280.0;
const PAD: f32 = 4.0;
/// Scrollable viewport height (the cap minus top/bottom padding).
pub const VIEW_H: f32 = MAX_H - 2.0 * PAD;

pub fn render(
    slash: &Slash,
    scroll: &gpui::ScrollHandle,
    cx: &mut Context<AppView>,
) -> impl IntoElement {
    let items = &slash.items;

    // The `\` LaTeX menu has short entries — keep it narrow like the structural editor's
    // dropdown; other completions (pages, templates) want the wider column.
    let min_w = if slash.trigger == Trigger::Math {
        120.0
    } else {
        220.0
    };

    // Inner scroll viewport: caps the height + scrolls the overflow. The chrome (bg/border)
    // lives on the outer `relative` box below so the scrollbar thumb can position against it.
    let mut col = div()
        .id("completion-menu")
        .max_h(px(MAX_H))
        .overflow_y_scroll()
        .track_scroll(scroll)
        .flex()
        .flex_col()
        .py(px(PAD));

    if items.is_empty() {
        col = col.child(
            div()
                .px_3()
                .py_1()
                .text_size(px(13.0))
                .text_color(theme::text_tertiary())
                .child("No commands"),
        );
    } else {
        for (i, item) in items.iter().enumerate() {
            let selected = i == slash.selected;
            let is_category = matches!(item.kind, ItemKind::Category(_));
            col = col.child(
                div()
                    .px_3()
                    .py_1()
                    .text_size(px(13.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .cursor_pointer()
                    .when(selected, |d| {
                        d.bg(theme::accent_tint()).text_color(theme::text_primary())
                    })
                    .when(!selected, |d| d.text_color(theme::text_secondary()))
                    // Hover moves the keyboard selection to this row, so the one
                    // highlight is what both a click and Enter accept.
                    .on_mouse_move(cx.listener(move |this, _, _window, cx| {
                        this.slash_hover(i, cx);
                    }))
                    // Mouse-DOWN (not click) + stop_propagation: accept before the press
                    // can blur the editor, so the insertion lands and focus stays put.
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.click_slash(i, window, cx);
                        }),
                    )
                    .child(item.label.clone())
                    .when(is_category, |d| {
                        d.child(div().text_color(theme::text_tertiary()).child("›"))
                    }),
            );
        }
    }

    // Scrollbar thumb — only when the rows overflow the cap; sized from the content height
    // and positioned from the live scroll offset (mirrors the gpui-editor table/suggestion
    // menus). Wheel + keyboard scroll both re-render, so the offset read here stays fresh.
    let rows_h = items.len().max(1) as f32 * ITEM_H;
    let thumb = (rows_h > VIEW_H).then(|| {
        let scrolled = (-f32::from(scroll.offset().y)).clamp(0.0, rows_h - VIEW_H);
        let thumb_h = (VIEW_H * VIEW_H / rows_h).max(24.0);
        let thumb_top = PAD + scrolled / (rows_h - VIEW_H) * (VIEW_H - thumb_h);
        let mut thumb_c = theme::text_tertiary();
        thumb_c.a = 0.5;
        div()
            .absolute()
            .top(px(thumb_top))
            .right(px(2.0))
            .w(px(6.0))
            .h(px(thumb_h))
            .rounded(px(3.0))
            .bg(thumb_c)
    });

    // Outer chrome: `relative` so the absolute thumb anchors to it, `occlude` so the wheel
    // scrolls the menu rather than bleeding through to the page beneath.
    div()
        .relative()
        .occlude()
        .min_w(px(min_w))
        .bg(theme::bg_sidebar())
        .border_1()
        .border_color(theme::border_subtle())
        .rounded(px(8.0))
        .overflow_hidden()
        .child(col)
        .children(thumb)
        .into_any_element()
}
