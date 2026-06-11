//! The left rail. Expanded, it's a row of icon buttons (collapse caret,
//! jump-to-date, settings) above a search box, then the journal feed link and a
//! "Recent" tree of the recently-viewed pages (`Foo::Bar` titles nest).
//! Collapsed, it shrinks to a thin icon rail with an expand caret (`>`) at the
//! top plus the calendar/settings icons. Right-click the pages area to create a
//! new page; non-recent pages and older days are found via search.

use gpui::{
    AnyElement, ClickEvent, Context, Div, FontWeight, InteractiveElement, IntoElement, MouseButton,
    ParentElement, SharedString, Stateful, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder as _, px, relative,
};
use gpui_component::{Icon, IconName, input::Input, menu::ContextMenuExt, tooltip::Tooltip};

use crate::actions::{
    DeletePage, NewPage, OpenInNewTab, OpenInNewWindow, RenamePage, ToggleFavorite,
};
use crate::app::AppView;
use crate::hierarchy::{self, PageNode};
use crate::models::Page;
use crate::theme;

pub fn render(app: &AppView, window: &mut Window, cx: &mut Context<AppView>) -> impl IntoElement {
    if app.sidebar_collapsed {
        collapsed_rail(cx).into_any_element()
    } else {
        expanded(app, window, cx).into_any_element()
    }
}

/// The full sidebar: a header (collapse caret + jump-to-date/settings icons,
/// then the search box) above the journal feed link and the page list.
fn expanded(app: &AppView, window: &mut Window, cx: &mut Context<AppView>) -> impl IntoElement {
    // The tree is filtered to recently-viewed pages. `Foo::Bar` titles nest;
    // a namespace segment with no recent page of its own still shows as a
    // virtual (clickable) node so the path to a recent page is visible.
    let tree = hierarchy::build_tree(
        app.pages
            .iter()
            .filter(|p| app.recent_pages.contains(&p.id)),
    );
    let mut page_rows: Vec<AnyElement> = Vec::new();
    push_tree_rows(&tree, 0, app, window, cx, &mut page_rows);
    let no_pages = page_rows.is_empty();

    // Favorites: the pinned pages, shown by full title above the recent list, in
    // the order added. A favorited page that's since been deleted is skipped.
    let fav_pages: Vec<&Page> = app
        .favorites
        .iter()
        .filter_map(|id| app.pages.iter().find(|p| p.id == *id))
        .collect();
    let mut fav_rows: Vec<AnyElement> = Vec::with_capacity(fav_pages.len());
    for page in fav_pages {
        fav_rows.push(favorite_row(page, app, window, cx));
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
                // Scroll vertically through the list. Rows stretch to the rail width
                // (default cross-axis stretch) and a title wider than that is clipped
                // with an ellipsis — so a row and its selection highlight never run
                // past the sidebar edge. The full title is shown in a tooltip.
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
                        .when(!fav_rows.is_empty(), |this| {
                            this.child(section_label("Favorites")).children(fav_rows)
                        })
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
    window: &mut Window,
    cx: &mut Context<AppView>,
    out: &mut Vec<AnyElement>,
) {
    for node in nodes {
        let active = node.id.is_some_and(|id| app.is_page_active(id));
        let is_fav = node.id.is_some_and(|id| app.is_favorite(id));
        out.push(tree_row(node, depth, active, is_fav, window, cx));
        push_tree_rows(&node.children, depth + 1, app, window, cx, out);
    }
}

/// A tree row's left padding (px) at `depth`; the base matches the other rows'
/// `px_2`. Used both to indent the row and to work out how much room the title
/// has before it's clipped.
fn row_indent(depth: usize) -> f32 {
    8.0 + depth as f32 * 14.0
}

/// Whether `label`, rendered at the row's 13px, is wider than the room left in a
/// row indented to `depth` — i.e. it will be ellipsized, so it wants a tooltip.
/// The sidebar is a fixed 240px; the scroll area pads 8px per side and a row pads
/// its indent on the left and 8px (`pr_2`) on the right. The rest is the text box.
fn label_overflows(label: &SharedString, depth: usize, window: &Window) -> bool {
    let avail = 240.0 - 16.0 - row_indent(depth) - 8.0;
    if avail <= 0.0 {
        return true;
    }
    // Shape the title at the row's 13px in the window's default font and compare
    // its width to the room available. `to_run` carries the font from the current
    // text style, which the rows inherit (they only override the size).
    let run = window.text_style().to_run(label.len());
    let width = window
        .text_system()
        .layout_line(label.as_ref(), px(13.0), &[run], None)
        .width;
    width > px(avail)
}

/// One row in the recent page tree, indented by `depth`. A real page delegates
/// to [`page_row`] (click + full right-click menu); a virtual namespace node is a
/// bare clickable row. Clicking either opens the page by its full path.
fn tree_row(
    node: &PageNode,
    depth: usize,
    active: bool,
    is_fav: bool,
    window: &mut Window,
    cx: &mut Context<AppView>,
) -> AnyElement {
    let label: SharedString = node.segment.clone().into();
    let full_path: SharedString = node.path.clone().into();
    if let Some(id) = node.id {
        return page_row(
            id,
            label,
            full_path.clone(),
            SharedString::from(format!("pn:{full_path}")),
            active,
            is_fav,
            depth,
            window,
            cx,
        );
    }
    // Virtual namespace node (no page of its own yet) — clickable, no menu.
    let truncated = label_overflows(&label, depth, window);
    let click_path = node.path.clone();
    let mut row = base_row(
        SharedString::from(format!("pn:{full_path}")),
        &label,
        false,
        depth,
    )
    .on_click(
        cx.listener(move |this: &mut AppView, _: &ClickEvent, window, cx| {
            this.open_page_title(&click_path, window, cx);
        }),
    );
    if truncated {
        let full = label.clone();
        row = row.tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx));
    }
    row.into_any_element()
}

/// A favorites-group row: the pinned page shown by its **full** title, with the
/// same click + right-click menu as a recent row.
fn favorite_row(
    page: &Page,
    app: &AppView,
    window: &mut Window,
    cx: &mut Context<AppView>,
) -> AnyElement {
    let title: SharedString = page.title.clone().into();
    page_row(
        page.id,
        title.clone(),
        title,
        SharedString::from(format!("fav:{}", page.id)),
        app.is_page_active(page.id),
        true,
        0,
        window,
        cx,
    )
}

/// Shared styling for a clickable sidebar page row (the caller chains `on_click`
/// etc.). `label` is what's shown; `depth` sets the indent.
fn base_row(id: SharedString, label: &SharedString, active: bool, depth: usize) -> Stateful<Div> {
    let mut row = div()
        .id(id)
        .relative()
        // Indent each level; the base matches the other rows' `px_2`.
        .pl(px(row_indent(depth)))
        .pr_2()
        .py_1p5()
        .rounded(px(6.0))
        .text_size(px(13.0))
        .cursor_pointer()
        // Clip an over-long title to the rail width with an ellipsis so the row —
        // and its selection highlight — never runs past the sidebar edge.
        .truncate()
        .when(active, |d| {
            d.bg(theme::accent_tint()).text_color(theme::text_primary())
        })
        .when(!active, |d| {
            d.text_color(theme::text_secondary())
                .hover(|h| h.bg(theme::hover()).text_color(theme::text_primary()))
        })
        .child(label.clone());
    // A faint vertical guide per ancestor level — like the nested markdown list —
    // so the subpage hierarchy reads at a glance. Drawn as overlays in the left
    // padding (left of the text), so the full-width row highlight is untouched;
    // rows are flush, so each segment joins the next into a continuous line.
    for level in 1..=depth {
        row = row.child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(px(guide_x(level)))
                .w(px(1.0))
                .bg(theme::border_subtle()),
        );
    }
    row
}

/// X (px from a row's left edge) of the indent guide for ancestor `level` — the
/// gutter just left of that level's text.
fn guide_x(level: usize) -> f32 {
    row_indent(level - 1) + 6.0
}

/// A real-page sidebar row (recent tree or favorites): opens `full_path` on
/// click, with a right-click menu to (un)favorite, open elsewhere, rename, or
/// delete. `label` is the shown text (tree leaf or full title); `elem_id` keeps
/// it unique across the two lists.
#[allow(clippy::too_many_arguments)]
fn page_row(
    id: i64,
    label: SharedString,
    full_path: SharedString,
    elem_id: SharedString,
    active: bool,
    is_fav: bool,
    depth: usize,
    window: &mut Window,
    cx: &mut Context<AppView>,
) -> AnyElement {
    let truncated = label_overflows(&label, depth, window);
    let click_path = full_path.to_string();
    let mut row = base_row(elem_id, &label, active, depth).on_click(cx.listener(
        move |this: &mut AppView, _: &ClickEvent, window, cx| {
            this.open_page_title(&click_path, window, cx);
        },
    ));
    if truncated {
        let full = label.clone();
        row = row.tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx));
    }
    // Right-click records the target page; the menu item dispatches an action
    // handled on `AppView` (delete confirms first).
    let fav_label = if is_fav {
        "Remove from favorites"
    } else {
        "Add to favorites"
    };
    row.on_mouse_down(
        MouseButton::Right,
        cx.listener(move |this: &mut AppView, _, _window, _cx| {
            this.set_context_page(id, full_path.clone());
        }),
    )
    .context_menu(move |menu, _window, _cx| {
        menu.menu(fav_label, Box::new(ToggleFavorite))
            .separator()
            .menu("Open in new tab", Box::new(OpenInNewTab))
            .menu("Open in new window", Box::new(OpenInNewWindow))
            .separator()
            .menu("Rename page", Box::new(RenamePage))
            .menu("Delete page", Box::new(DeletePage))
    })
    .into_any_element()
}

fn section_label(text: &str) -> impl IntoElement {
    div()
        .px_2()
        .pt_4()
        .pb_1()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::accent())
                .child(text.to_uppercase()),
        )
        // A hairline rule fills the rest of the row, separating the groups.
        .child(div().flex_1().h(px(1.0)).bg(theme::divider()))
}

fn empty_hint(text: &str) -> impl IntoElement {
    div()
        .px_2()
        .py_1()
        .text_size(px(12.0))
        .text_color(theme::text_tertiary())
        .child(text.to_string())
}
