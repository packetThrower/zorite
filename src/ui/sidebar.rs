//! The left rail. Expanded, it's a row of icon buttons (collapse caret,
//! jump-to-date, settings) above a search box, then the journal feed link and a
//! "Recent" tree of the recently-viewed pages (`Foo::Bar` titles nest).
//! Collapsed, it shrinks to a thin icon rail with an expand caret (`>`) at the
//! top plus the calendar/settings icons. Right-click the pages area to create a
//! new page; non-recent pages and older days are found via search.

use gpui::{
    AnyElement, ClickEvent, Context, Div, InteractiveElement, IntoElement, MouseButton,
    ParentElement, SharedString, Stateful, StatefulInteractiveElement, Styled, div,
    prelude::FluentBuilder as _, px, relative,
};
use gpui_component::{Icon, IconName, input::Input, menu::ContextMenuExt};

use crate::actions::{DeletePage, NewPage, OpenInNewTab, OpenInNewWindow, RenamePage};
use crate::app::AppView;
use crate::hierarchy::{self, PageNode};
use crate::theme;

pub fn render(app: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    if app.sidebar_collapsed {
        collapsed_rail(cx).into_any_element()
    } else {
        expanded(app, cx).into_any_element()
    }
}

/// The full sidebar: a header (collapse caret + jump-to-date/settings icons,
/// then the search box) above the journal feed link and the page list.
fn expanded(app: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    // The tree is filtered to recently-viewed pages. `Foo::Bar` titles nest;
    // a namespace segment with no recent page of its own still shows as a
    // virtual (clickable) node so the path to a recent page is visible.
    let tree = hierarchy::build_tree(
        app.pages
            .iter()
            .filter(|p| app.recent_pages.contains(&p.id)),
    );
    let mut page_rows: Vec<AnyElement> = Vec::new();
    push_tree_rows(&tree, 0, app, cx, &mut page_rows);
    let no_pages = page_rows.is_empty();

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
            // Header: the collapse caret on the left, the jump-to-date and
            // settings icons on the right, with the search box on the row below.
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
                        .justify_between()
                        .child(collapse_caret(cx))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_1()
                                .child(new_page_icon(cx))
                                .child(date_icon(cx))
                                .child(settings_gear(cx)),
                        ),
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
                // Scroll both ways: down through many pages, and right when a page
                // title is wider than the rail. `items_start` lets the list size to
                // its widest (non-wrapping) row instead of being clamped to the
                // rail width — that overflow is what the horizontal scroll reveals.
                .overflow_scroll()
                .items_start()
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
                        .child(section_label("Recent"))
                        .when(no_pages, |this| {
                            this.child(empty_hint(if app.pages.is_empty() {
                                "No pages yet — right-click below to add one"
                            } else {
                                "No recent pages"
                            }))
                        })
                        .children(page_rows),
                )
                .child(
                    // The empty area below the list is right-clickable for
                    // "New page", extending the menu past the last row. Pinned to
                    // the rail width (the scroll container is `items_start`, so it
                    // wouldn't stretch on its own).
                    div()
                        .id("sidebar-empty")
                        .flex_1()
                        .w(relative(1.0))
                        .min_h(px(48.0))
                        .context_menu(|menu, _window, _cx| {
                            menu.menu("New page", Box::new(NewPage))
                        }),
                ),
        )
}

/// The collapsed sidebar: a thin icon rail with the expand caret (`>`) at the
/// top, then the jump-to-date and settings icons.
fn collapsed_rail(cx: &mut Context<AppView>) -> impl IntoElement {
    div()
        .w(px(48.0))
        .h_full()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .items_center()
        .gap_1()
        .pt_2()
        .bg(theme::bg_sidebar())
        .border_r_1()
        .border_color(theme::border_subtle())
        .child(expand_caret(cx))
        .child(new_page_icon(cx))
        .child(date_icon(cx))
        .child(settings_gear(cx))
}

/// Shared styling for the square sidebar icon buttons. The caller chains
/// `.on_click(...)`.
fn icon_btn(id: &'static str, icon: IconName) -> Stateful<Div> {
    div()
        .id(id)
        .flex_shrink_0()
        .p_1p5()
        .rounded(px(6.0))
        .text_color(theme::text_secondary())
        .cursor_pointer()
        .hover(|h| h.bg(theme::hover()).text_color(theme::text_primary()))
        .child(Icon::new(icon).size_4())
}

/// Collapse caret (`<`), in the expanded header — hides the sidebar to a rail.
fn collapse_caret(cx: &mut Context<AppView>) -> impl IntoElement {
    icon_btn("collapse-sidebar", IconName::ChevronLeft).on_click(cx.listener(
        |this: &mut AppView, _: &ClickEvent, _window, cx| {
            this.toggle_sidebar(cx);
        },
    ))
}

/// Expand caret (`>`), at the top of the collapsed rail — reopens the sidebar.
fn expand_caret(cx: &mut Context<AppView>) -> impl IntoElement {
    icon_btn("expand-sidebar", IconName::ChevronRight).on_click(cx.listener(
        |this: &mut AppView, _: &ClickEvent, _window, cx| {
            this.toggle_sidebar(cx);
        },
    ))
}

/// The jump-to-date calendar icon. Toggles the calendar overlay; picking a date
/// opens that journal day.
fn date_icon(cx: &mut Context<AppView>) -> impl IntoElement {
    icon_btn("date-icon", IconName::Calendar).on_click(cx.listener(
        |this: &mut AppView, _: &ClickEvent, _window, cx| {
            this.toggle_calendar(cx);
        },
    ))
}

/// The "new page" plus button, next to the calendar. Dispatches `NewPage`, which
/// prompts for a title (same path as the pages-area right-click "New page" menu).
fn new_page_icon(cx: &mut Context<AppView>) -> impl IntoElement {
    icon_btn("new-page", IconName::Plus).on_click(cx.listener(
        |_this: &mut AppView, _: &ClickEvent, window, cx| {
            window.dispatch_action(Box::new(NewPage), cx);
        },
    ))
}

/// The settings gear. Opens the Settings window (deferred — opening a window
/// from inside a mouse callback aborts).
fn settings_gear(cx: &mut Context<AppView>) -> impl IntoElement {
    icon_btn("settings-gear", IconName::Settings).on_click(cx.listener(
        |_this: &mut AppView, _: &ClickEvent, window, cx| {
            let view = cx.entity();
            window.defer(cx, move |_, cx| {
                AppView::open_settings(view, cx);
            });
        },
    ))
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

/// Flatten the page tree into indented rows in pre-order (parent, then its
/// children one level deeper).
fn push_tree_rows(
    nodes: &[PageNode],
    depth: usize,
    app: &AppView,
    cx: &mut Context<AppView>,
    out: &mut Vec<AnyElement>,
) {
    for node in nodes {
        let active = node.id.is_some_and(|id| app.is_page_active(id));
        out.push(tree_row(node, depth, active, cx));
        push_tree_rows(&node.children, depth + 1, app, cx, out);
    }
}

/// One row in the page tree, indented by `depth`. Real pages get a right-click
/// menu (open in new tab / rename / delete); virtual namespace nodes don't.
/// Clicking either opens the page by its full path, creating it if needed.
fn tree_row(node: &PageNode, depth: usize, active: bool, cx: &mut Context<AppView>) -> AnyElement {
    let label: SharedString = node.segment.clone().into();
    let click_path = node.path.clone();

    let row = div()
        .id(SharedString::from(format!("pn:{}", node.path)))
        // Indent each level; the base matches the other rows' `px_2`.
        .pl(px(8.0 + depth as f32 * 14.0))
        .pr_2()
        .py_1p5()
        .rounded(px(6.0))
        .text_size(px(13.0))
        .cursor_pointer()
        // Keep each title on one line; over-long ones overflow and the sidebar
        // scrolls horizontally to reveal them rather than clipping with an ellipsis.
        .whitespace_nowrap()
        .when(active, |d| {
            d.bg(theme::accent_tint()).text_color(theme::text_primary())
        })
        .when(!active, |d| {
            d.text_color(theme::text_secondary())
                .hover(|h| h.bg(theme::hover()).text_color(theme::text_primary()))
        })
        .child(label)
        .on_click(
            cx.listener(move |this: &mut AppView, _: &ClickEvent, window, cx| {
                this.open_page_title(&click_path, window, cx);
            }),
        );

    // Right-click records the target page; the menu item dispatches an action
    // handled on `AppView` (delete confirms first).
    if let Some(id) = node.id {
        let menu_label: SharedString = node.path.clone().into();
        row.on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this: &mut AppView, _, _window, _cx| {
                this.set_context_page(id, menu_label.clone());
            }),
        )
        .context_menu(|menu, _window, _cx| {
            menu.menu("Open in new tab", Box::new(OpenInNewTab))
                .menu("Open in new window", Box::new(OpenInNewWindow))
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
