//! The slash-command popup list, rendered as an anchored overlay by `AppView`.
//! Keyboard-driven (arrows + Enter) and mouse-driven (hover highlights a row,
//! click accepts it).

use gpui::{
    Context, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder as _, px,
};

use crate::app::AppView;
use crate::slash::{ItemKind, Slash};
use crate::theme;

pub fn render(slash: &Slash, cx: &mut Context<AppView>) -> impl IntoElement {
    let items = &slash.items;

    let mut col = div()
        .id("completion-menu")
        .min_w(px(220.0))
        // Bound the height (page lists are capped, but templates etc. can be
        // long) and scroll the overflow rather than spilling off-window.
        .max_h(px(280.0))
        .overflow_y_scroll()
        // Occlude so the wheel scrolls the menu, not the page beneath it: without
        // this the scroll bled through to the page on Windows, and on Linux the
        // menu never became the scroll target so it didn't scroll at all.
        .occlude()
        .bg(theme::bg_sidebar())
        .border_1()
        .border_color(theme::border_subtle())
        .rounded(px(8.0))
        .py(px(4.0))
        .flex()
        .flex_col();

    if items.is_empty() {
        return col
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_size(px(13.0))
                    .text_color(theme::text_tertiary())
                    .child("No commands"),
            )
            .into_any_element();
    }

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
    col.into_any_element()
}
