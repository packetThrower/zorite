//! Search results in the main pane, driven by the sidebar search box.

use gpui::{
    ClickEvent, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder as _, px,
};

use crate::app::AppView;
use crate::models::SearchHit;
use crate::theme;

pub fn render(app: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let hits = &app.search_results;
    let rows: Vec<_> = hits
        .iter()
        .enumerate()
        .map(|(i, hit)| hit_row(i, hit, cx).into_any_element())
        .collect();

    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(theme::bg_content())
        .child(
            div()
                .id("search-scroll")
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
                        .gap(px(8.0))
                        .child(
                            div()
                                .pb_2()
                                .text_size(px(13.0))
                                .text_color(theme::text_tertiary())
                                .child(format!(
                                    "{} result{}",
                                    hits.len(),
                                    if hits.len() == 1 { "" } else { "s" }
                                )),
                        )
                        .when(hits.is_empty(), |d| {
                            d.child(div().text_color(theme::text_tertiary()).child("No matches"))
                        })
                        .children(rows),
                ),
        )
}

fn hit_row(i: usize, hit: &SearchHit, cx: &mut Context<AppView>) -> impl IntoElement {
    let id = hit.page_id;
    div()
        .id(("hit", i))
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
                .text_size(px(14.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text_primary())
                .child(hit.title.clone()),
        )
        .when(!hit.snippet.trim().is_empty(), |d| {
            d.child(
                div()
                    .text_size(px(13.0))
                    .text_color(theme::text_secondary())
                    .child(hit.snippet.clone()),
            )
        })
        .on_click(
            cx.listener(move |this: &mut AppView, _: &ClickEvent, window, cx| {
                this.open_page_id(id, window, cx);
            }),
        )
}
