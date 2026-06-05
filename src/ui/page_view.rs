//! A single named/journal page: title, its markdown editor, and a
//! "Linked References" panel.

use gpui::{
    ClickEvent, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, px, prelude::FluentBuilder as _,
};
use gpui_component::input::Input;

use crate::app::AppView;
use crate::models::Backlink;
use crate::theme;

pub fn render(app: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let Some(pe) = app.page_editor.as_ref() else {
        return div().flex_1().min_w_0().h_full().bg(theme::bg_content()).into_any_element();
    };

    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(theme::bg_content())
        .child(
            div()
                .id("page-scroll")
                .size_full()
                .overflow_y_scroll()
                .child(
                    div()
                        .max_w(px(760.0))
                        .mx_auto()
                        .px(px(48.0))
                        .py(px(28.0))
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .mb_4()
                                .text_size(px(24.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::text_primary())
                                .child(pe.title.clone()),
                        )
                        .child(
                            Input::new(&pe.state)
                                .appearance(false)
                                .text_size(px(16.0))
                                .text_color(theme::text_primary()),
                        )
                        .when(!pe.backlinks.is_empty(), |this| {
                            this.child(backlinks_section(&pe.backlinks, cx))
                        }),
                ),
        )
        .into_any_element()
}

fn backlinks_section(backlinks: &[Backlink], cx: &mut Context<AppView>) -> impl IntoElement {
    div()
        .mt(px(28.0))
        .pt_4()
        .border_t_1()
        .border_color(theme::border_subtle())
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .pb_1()
                .text_size(px(11.0))
                .text_color(theme::text_tertiary())
                .child(format!("LINKED REFERENCES ({})", backlinks.len())),
        )
        .children(
            backlinks
                .iter()
                .enumerate()
                .map(|(i, bl)| backlink_row(i, bl, cx).into_any_element())
                .collect::<Vec<_>>(),
        )
}

fn backlink_row(i: usize, bl: &Backlink, cx: &mut Context<AppView>) -> impl IntoElement {
    let page_id = bl.source_page_id;
    div()
        .id(("bl", i))
        .px_3()
        .py_2()
        .rounded(px(6.0))
        .bg(theme::glass())
        .cursor_pointer()
        .hover(|h| h.bg(theme::glass_strong()))
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(px(11.0))
                .text_color(theme::accent())
                .child(bl.source_page_title.clone()),
        )
        .child(
            div()
                .text_size(px(13.0))
                .text_color(theme::text_secondary())
                .child(bl.snippet.clone()),
        )
        .on_click(cx.listener(move |this: &mut AppView, _: &ClickEvent, window, cx| {
            this.open_page_id(page_id, window, cx);
        }))
}
