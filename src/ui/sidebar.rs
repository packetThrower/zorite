//! The left rail: the journal feed, recent journals, named pages, and a
//! "find or create page" box.

use gpui::{
    ClickEvent, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px, prelude::FluentBuilder as _,
};
use gpui_component::{input::Input, menu::ContextMenuExt};

use crate::actions::{DeletePage, OpenInNewTab};
use crate::app::AppView;
use crate::models::Page;
use crate::theme;

pub fn render(app: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let mut journal_rows = Vec::new();
    for p in &app.journals {
        journal_rows.push(nav_row(p, app.is_page_active(p.id), cx).into_any_element());
    }
    let mut page_rows = Vec::new();
    for p in &app.pages {
        page_rows.push(nav_row(p, app.is_page_active(p.id), cx).into_any_element());
    }

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
            div()
                .flex_shrink_0()
                .p_2()
                .border_b_1()
                .border_color(theme::border_subtle())
                .child(Input::new(&app.search_input)),
        )
        .child(
            div()
                .id("sidebar-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px_2()
                .pt_3()
                .child(journal_row(app.is_journal_view(), cx))
                .child(section_label("Journals"))
                .children(journal_rows)
                .child(section_label("Pages"))
                .when(app.pages.is_empty(), |this| {
                    this.child(empty_hint("No pages yet — link one with [[ ]]"))
                })
                .children(page_rows),
        )
        .child(
            div()
                .flex_shrink_0()
                .p_2()
                .border_t_1()
                .border_color(theme::border_subtle())
                .child(Input::new(&app.new_page_input)),
        )
}

fn journal_row(active: bool, cx: &mut Context<AppView>) -> impl IntoElement {
    div()
        .id("journal")
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
        .child("Journal")
        .on_click(cx.listener(|this: &mut AppView, _: &ClickEvent, window, cx| {
            this.show_journal(window, cx);
        }))
}

fn nav_row(page: &Page, active: bool, cx: &mut Context<AppView>) -> impl IntoElement {
    let id = page.id;
    let label: SharedString = page.title.clone().into();
    let deletable = !page.is_journal;

    let row = div()
        .id(("nav", id as usize))
        .px_2()
        .py_1p5()
        .rounded(px(6.0))
        .text_size(px(13.0))
        .cursor_pointer()
        .truncate()
        .when(active, |d| d.bg(theme::accent_tint()).text_color(theme::text_primary()))
        .when(!active, |d| {
            d.text_color(theme::text_secondary())
                .hover(|h| h.bg(theme::hover()).text_color(theme::text_primary()))
        })
        .child(label.clone())
        .on_click(cx.listener(move |this: &mut AppView, _: &ClickEvent, window, cx| {
            this.open_page_id(id, window, cx);
        }));

    // Named pages (never journals) get a right-click "Delete page" menu.
    // Right-click records the target; the menu item dispatches `DeletePage`,
    // handled on `AppView` (which confirms before deleting).
    if deletable {
        row.on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this: &mut AppView, _, _window, _cx| {
                this.set_context_page(id, label.clone());
            }),
        )
        .context_menu(|menu, _window, _cx| {
            menu.menu("Open in new tab", Box::new(OpenInNewTab))
                .separator()
                .menu("Delete page", Box::new(DeletePage))
        })
        .into_any_element()
    } else {
        row.into_any_element()
    }
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
