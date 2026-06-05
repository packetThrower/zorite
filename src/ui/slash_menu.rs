//! The slash-command popup list (keyboard-driven; rendered as an anchored
//! overlay by `AppView`).

use gpui::{IntoElement, ParentElement, Styled, div, px, prelude::FluentBuilder as _};

use crate::slash::Slash;
use crate::theme;

pub fn render(slash: &Slash) -> impl IntoElement {
    let matches = slash.matches();

    let mut col = div()
        .min_w(px(220.0))
        .bg(theme::bg_sidebar())
        .border_1()
        .border_color(theme::border_subtle())
        .rounded(px(8.0))
        .py(px(4.0))
        .flex()
        .flex_col();

    if matches.is_empty() {
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

    for (i, cmd) in matches.iter().enumerate() {
        let selected = i == slash.selected;
        col = col.child(
            div()
                .px_3()
                .py_1()
                .text_size(px(13.0))
                .when(selected, |d| d.bg(theme::accent_tint()).text_color(theme::text_primary()))
                .when(!selected, |d| d.text_color(theme::text_secondary()))
                .child(cmd.label),
        );
    }
    col.into_any_element()
}
