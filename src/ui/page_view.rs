//! A single named/journal page: title, its markdown editor, and a
//! "Linked References" panel.

use gpui::{
    ClickEvent, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder as _, px, relative,
};
use gpui_component::input::{Input, InputState};

use crate::app::{AppView, PageEditor};
use crate::models::Backlink;
use crate::theme;

pub fn render(app: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let Some(pe) = app.page_editor.as_ref() else {
        return div()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(theme::bg_content())
            .into_any_element();
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
                        // Match the journal feed: uniform padding, left-aligned.
                        .p(px(28.0))
                        .flex()
                        .flex_col()
                        // Fill the viewport so the open area below the content
                        // is clickable all the way down.
                        .min_h(relative(1.0))
                        .child(page_title(pe))
                        .child(if app.is_page_editing() {
                            Input::new(&pe.state)
                                .appearance(false)
                                .text_size(px(16.0))
                                .text_color(theme::text_primary())
                                .into_any_element()
                        } else {
                            page_rendered(&pe.state, cx).into_any_element()
                        })
                        .when(!pe.backlinks.is_empty(), |this| {
                            this.child(backlinks_section(&pe.backlinks, cx))
                        })
                        .child(page_open_area(cx)),
                ),
        )
        .into_any_element()
}

/// The page heading. Journals keep their date as static text; named pages
/// get a borderless, heading-styled `Input` that renames the page when
/// edited (commit on Enter/blur is wired in `load_page_editor`).
fn page_title(pe: &PageEditor) -> impl IntoElement {
    if pe.is_journal {
        div()
            .mb_4()
            .text_size(px(24.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme::text_primary())
            .child(pe.title.clone())
            .into_any_element()
    } else {
        div()
            .mb_4()
            .child(
                // The input's default line-height/height are sized for body
                // text; at 24px they clip descenders, so override them.
                Input::new(&pe.title_state)
                    .appearance(false)
                    .text_size(px(24.0))
                    .line_height(px(30.0))
                    .py(px(0.0))
                    .h(px(36.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text_primary()),
            )
            .into_any_element()
    }
}

/// The page body in reading mode: rendered markdown (or a placeholder
/// when empty), clickable to enter edit mode.
fn page_rendered(state: &Entity<InputState>, cx: &mut Context<AppView>) -> impl IntoElement {
    let content = state.read(cx).value();
    let inner = if content.trim().is_empty() {
        div()
            .text_size(px(16.0))
            .text_color(theme::text_tertiary())
            .child("Empty — click to write")
            .into_any_element()
    } else {
        let weak = cx.entity().downgrade();
        gpui_markdown::MarkdownView::new("page-md", content)
            .style(theme::markdown_style())
            .on_wiki_link(std::rc::Rc::new(move |title, window, cx| {
                let _ = weak.update(cx, |this, cx| this.open_page_title(&title, window, cx));
            }))
            .into_any_element()
    };
    div()
        .id("page-body")
        .w_full()
        .min_h(px(24.0))
        .cursor_text()
        .child(inner)
        .on_click(
            cx.listener(|this: &mut AppView, _: &ClickEvent, window, cx| {
                this.edit_page(window, cx);
            }),
        )
}

/// The empty space below the page content (and any backlinks). Clicking it
/// enters edit mode with the caret on a trailing blank line — same affordance
/// as the journal feed's open day area.
fn page_open_area(cx: &mut Context<AppView>) -> impl IntoElement {
    div()
        .id("page-open")
        .flex_1()
        .min_h(px(60.0))
        .w_full()
        .cursor_text()
        .on_click(
            cx.listener(|this: &mut AppView, _: &ClickEvent, window, cx| {
                this.edit_page_at_end(window, cx);
            }),
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
                .child(bl.snippet.clone()),
        )
        .on_click(
            cx.listener(move |this: &mut AppView, _: &ClickEvent, window, cx| {
                this.open_page_id(page_id, window, cx);
            }),
        )
}
