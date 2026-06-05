//! The main pane: the current page's title, its outliner, and a
//! "Linked References" panel listing blocks elsewhere that link here.

use gpui::{
    ClickEvent, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, px, prelude::FluentBuilder as _,
};

use crate::app::AppView;
use crate::models::Backlink;
use crate::theme;
use crate::ui;

pub fn render(app: &AppView, _window: &mut Window, cx: &mut Context<AppView>) -> impl IntoElement {
    // One element per visible block, in outline order.
    let rows: Vec<_> = app
        .nodes
        .iter()
        .filter_map(|node| {
            let editor = app.editors.get(&node.block.id)?;
            let focused = app.focused_block == Some(node.block.id);
            Some(ui::block_row::render(node, &editor.state, focused, cx).into_any_element())
        })
        .collect();

    let backlinks = app.backlinks.clone();
    let title = app.page_title().to_string();

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
                        .max_w(px(820.0))
                        .mx_auto()
                        .px(px(40.0))
                        .py(px(32.0))
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .mb_4()
                                .text_size(px(24.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::text_primary())
                                .child(title),
                        )
                        .child(div().flex().flex_col().gap(px(1.0)).children(rows))
                        .when(!backlinks.is_empty(), |this| {
                            this.child(backlinks_section(&backlinks, cx))
                        }),
                ),
        )
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
                .child(bl.block_content.clone()),
        )
        .on_click(cx.listener(move |this: &mut AppView, _: &ClickEvent, window, cx| {
            this.open_page_id(page_id, window, cx);
        }))
}
