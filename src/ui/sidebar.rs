//! The left rail: jump to today's journal, browse recent journals and
//! named pages, and a "find or create page" box. Layout idioms follow
//! `etch341/src/gui/sidebar.rs`.

use gpui::{
    ClickEvent, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px, prelude::FluentBuilder as _,
};
use gpui_component::input::Input;

use crate::app::AppView;
use crate::models::Page;
use crate::theme;

pub fn render(app: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let current = app.current_page_id();

    div()
        .w(px(240.0))
        .h_full()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .bg(theme::bg_sidebar())
        .border_r_1()
        .border_color(theme::border_subtle())
        .child(
            // Scrollable nav area.
            div()
                .id("sidebar-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px_2()
                .pt_3()
                .child(today_row(app.is_viewing_today(), cx))
                .child(section_label("Journals"))
                .children(
                    app.journals
                        .iter()
                        .map(|p| nav_row(p, current, cx).into_any_element())
                        .collect::<Vec<_>>(),
                )
                .child(section_label("Pages"))
                .when(app.pages.is_empty(), |this| {
                    this.child(empty_hint("No pages yet — link one with [[ ]]"))
                })
                .children(
                    app.pages
                        .iter()
                        .map(|p| nav_row(p, current, cx).into_any_element())
                        .collect::<Vec<_>>(),
                ),
        )
        .child(
            // Pinned "find or create page" box at the bottom.
            div()
                .flex_shrink_0()
                .p_2()
                .border_t_1()
                .border_color(theme::border_subtle())
                .child(Input::new(&app.new_page_input)),
        )
}

fn today_row(active: bool, cx: &mut Context<AppView>) -> impl IntoElement {
    div()
        .id("today")
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1p5()
        .rounded(px(6.0))
        .text_size(px(13.0))
        .cursor_pointer()
        .when(active, |d| d.bg(theme::accent_tint()).text_color(theme::text_primary()))
        .when(!active, |d| {
            d.text_color(theme::text_secondary())
                .hover(|h| h.bg(theme::hover()).text_color(theme::text_primary()))
        })
        .child("Today")
        .on_click(cx.listener(|this: &mut AppView, _: &ClickEvent, window, cx| {
            this.open_today(window, cx);
        }))
}

fn nav_row(page: &Page, current: i64, cx: &mut Context<AppView>) -> impl IntoElement {
    let id = page.id;
    let active = id == current;
    let label: SharedString = page.title.clone().into();

    div()
        .id(("nav", id as usize))
        .px_2()
        .py_1()
        .rounded(px(6.0))
        .text_size(px(13.0))
        .cursor_pointer()
        .truncate()
        .when(active, |d| d.bg(theme::accent_tint()).text_color(theme::text_primary()))
        .when(!active, |d| {
            d.text_color(theme::text_secondary())
                .hover(|h| h.bg(theme::hover()).text_color(theme::text_primary()))
        })
        .child(label)
        .on_click(cx.listener(move |this: &mut AppView, _: &ClickEvent, window, cx| {
            this.open_page_id(id, window, cx);
        }))
}

fn section_label(text: &str) -> impl IntoElement {
    div()
        .px_2()
        .pt_4()
        .pb_1()
        .text_size(px(11.0))
        .text_color(theme::text_tertiary())
        .child(text.to_uppercase())
}

fn empty_hint(text: &str) -> impl IntoElement {
    div()
        .px_2()
        .py_1()
        .text_size(px(12.0))
        .text_color(theme::text_tertiary())
        .child(text.to_string())
}
