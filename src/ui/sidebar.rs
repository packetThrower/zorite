//! The left rail: a row of icon buttons (jump-to-date and settings) above a
//! search box, then a link to the journal feed and the named pages. Right-click
//! the pages area to create a new page; older days are found via search or the
//! date picker.

use gpui::{
    ClickEvent, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{Icon, IconName, input::Input, menu::ContextMenuExt};

use crate::actions::{DeletePage, NewPage, OpenInNewTab, RenamePage};
use crate::app::AppView;
use crate::models::Page;
use crate::theme;

pub fn render(app: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
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
            // A row of icon buttons (a sidebar-collapse caret will join it
            // later), with the search box on its own row below.
            div()
                .flex_shrink_0()
                .p_2()
                .border_b_1()
                .border_color(theme::border_subtle())
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_end()
                        .gap_1()
                        .child(date_icon(cx))
                        .child(settings_gear(cx)),
                )
                .child(
                    Input::new(&app.search_input).prefix(
                        Icon::new(IconName::Search)
                            .size_4()
                            .text_color(theme::text_tertiary()),
                    ),
                ),
        )
        .child(
            div()
                .id("sidebar-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .px_2()
                .pt_3()
                .child(
                    // The list itself; never shrinks, so it scrolls instead of
                    // squishing when there are many pages.
                    div()
                        .flex_shrink_0()
                        .flex()
                        .flex_col()
                        .child(journal_row(app.is_journal_view(), cx))
                        .child(section_label("Pages"))
                        .when(app.pages.is_empty(), |this| {
                            this.child(empty_hint("No pages yet — right-click below to add one"))
                        })
                        .children(page_rows),
                )
                .child(
                    // The empty area below the list is right-clickable for
                    // "New page", extending the menu past the last row.
                    div()
                        .id("sidebar-empty")
                        .flex_1()
                        .min_h(px(48.0))
                        .context_menu(|menu, _window, _cx| {
                            menu.menu("New page", Box::new(NewPage))
                        }),
                ),
        )
}

/// The jump-to-date calendar icon, beside the search box. Toggles the calendar
/// overlay; picking a date opens that journal day.
fn date_icon(cx: &mut Context<AppView>) -> impl IntoElement {
    div()
        .id("date-icon")
        .flex_shrink_0()
        .p_1p5()
        .rounded(px(6.0))
        .text_color(theme::text_secondary())
        .cursor_pointer()
        .hover(|h| h.bg(theme::hover()).text_color(theme::text_primary()))
        .child(Icon::new(IconName::Calendar).size_4())
        .on_click(
            cx.listener(|this: &mut AppView, _: &ClickEvent, _window, cx| {
                this.toggle_calendar(cx);
            }),
        )
}

/// The settings gear, sitting beside the search box. Opens the Settings window
/// (deferred — opening a window from inside a mouse callback aborts).
fn settings_gear(cx: &mut Context<AppView>) -> impl IntoElement {
    div()
        .id("settings-gear")
        .flex_shrink_0()
        .p_1p5()
        .rounded(px(6.0))
        .text_color(theme::text_secondary())
        .cursor_pointer()
        .hover(|h| h.bg(theme::hover()).text_color(theme::text_primary()))
        .child(Icon::new(IconName::Settings).size_4())
        .on_click(
            cx.listener(|_this: &mut AppView, _: &ClickEvent, window, cx| {
                let view = cx.entity();
                window.defer(cx, move |_, cx| {
                    AppView::open_settings(view, cx);
                });
            }),
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
        .when(active, |d| {
            d.bg(theme::accent_tint()).text_color(theme::text_primary())
        })
        .when(!active, |d| {
            d.text_color(theme::text_secondary())
                .hover(|h| h.bg(theme::hover()).text_color(theme::text_primary()))
        })
        .child("Journal")
        .on_click(
            cx.listener(|this: &mut AppView, _: &ClickEvent, window, cx| {
                this.show_journal(window, cx);
            }),
        )
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
        .when(active, |d| {
            d.bg(theme::accent_tint()).text_color(theme::text_primary())
        })
        .when(!active, |d| {
            d.text_color(theme::text_secondary())
                .hover(|h| h.bg(theme::hover()).text_color(theme::text_primary()))
        })
        .child(label.clone())
        .on_click(
            cx.listener(move |this: &mut AppView, _: &ClickEvent, window, cx| {
                this.open_page_id(id, window, cx);
            }),
        );

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
                .menu("Rename page", Box::new(RenamePage))
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
