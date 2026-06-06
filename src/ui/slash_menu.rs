//! The slash-command popup list (keyboard-driven; rendered as an anchored
//! overlay by `AppView`).

use gpui::{IntoElement, ParentElement, Styled, div, prelude::FluentBuilder as _, px};

use crate::slash::{ItemKind, Slash};
use crate::theme;

pub fn render(slash: &Slash) -> impl IntoElement {
    let items = &slash.items;

    let mut col = div()
        .min_w(px(220.0))
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
                .when(selected, |d| {
                    d.bg(theme::accent_tint()).text_color(theme::text_primary())
                })
                .when(!selected, |d| d.text_color(theme::text_secondary()))
                .child(item.label.clone())
                .when(is_category, |d| {
                    d.child(div().text_color(theme::text_tertiary()).child("›"))
                }),
        );
    }
    col.into_any_element()
}
