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
use gpui_component::{
    Icon, IconName, Sizable, input::Input, menu::ContextMenuExt, tooltip::Tooltip,
};

use crate::actions::{
    DeletePage, ExportPdf, NewPage, OpenInNewTab, OpenInNewWindow, RenamePage, ToggleFavorite,
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
        .filter_map(|id| {
            // A favorite can be a page or a whiteboard, so search both lists.
            app.pages
                .iter()
                .chain(app.whiteboards.iter())
                .find(|p| p.id == *id)
        })
        .collect();
    let mut fav_rows: Vec<AnyElement> = Vec::with_capacity(fav_pages.len());
    for page in fav_pages {
        fav_rows.push(favorite_row(page, app, window, cx));
    }

    // Whiteboards: every board, shown flat (a distinct surface from the notes
    // tree). New boards are created from the `+`-style button in the top toolbar.
    let mut wb_rows: Vec<AnyElement> = Vec::with_capacity(app.whiteboards.len());
    for page in &app.whiteboards {
        wb_rows.push(whiteboard_row(page, app, window, cx));
    }

    // Collapsible section headers (a click toggles; the rows are hidden when
    // collapsed).
    let fav_collapsed = app.is_section_collapsed("favorites");
    let wb_collapsed = app.is_section_collapsed("whiteboards");
    let recent_collapsed = app.is_section_collapsed("recent");
    let fav_header = section_header("Favorites", "favorites", fav_collapsed, cx);
    let wb_header = section_header("Whiteboards", "whiteboards", wb_collapsed, cx);
    let recent_header = section_header("Recent", "recent", recent_collapsed, cx);

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
                                .child(new_whiteboard_icon(cx))
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
                            let this = this.child(fav_header);
                            if fav_collapsed {
                                this
                            } else {
                                this.children(fav_rows)
                            }
                        })
                        // The Whiteboards section stays hidden until at least one
                        // board exists (same as Favorites) — no empty header.
                        .when(!wb_rows.is_empty(), |this| {
                            let this = this.child(wb_header);
                            if wb_collapsed {
                                this
                            } else {
                                this.children(wb_rows)
                            }
                        })
                        .child(recent_header)
                        .when(!recent_collapsed, |this| {
                            this.when(no_pages, |t| {
                                t.child(empty_hint(if app.pages.is_empty() {
                                    "No pages yet — right-click below to add one"
                                } else {
                                    "No recent pages"
                                }))
                            })
                            .children(page_rows)
                        }),
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
fn icon_btn(id: &'static str, icon: impl Into<Icon>) -> Stateful<Div> {
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
    icon_btn("new-page", Icon::empty().path("icons/sticky-note-plus.svg"))
        .on_click(
            cx.listener(|_this: &mut AppView, _: &ClickEvent, window, cx| {
                window.dispatch_action(Box::new(NewPage), cx);
            }),
        )
        .tooltip(|window, cx| Tooltip::new("New page").build(window, cx))
}

/// The settings gear. Opens the Settings window (deferred — opening a window
/// from inside a mouse callback aborts). Wears an amber dot when the boot-time
/// update check found a newer release (see `crate::updater`).
fn settings_gear(cx: &mut Context<AppView>) -> impl IntoElement {
    let update_available = cx
        .try_global::<crate::updater::UpdateState>()
        .is_some_and(|u| u.available.is_some());
    icon_btn("settings-gear", IconName::Settings)
        .relative()
        .when(update_available, |gear| {
            gear.child(
                div()
                    .absolute()
                    .top(px(2.0))
                    .right(px(2.0))
                    .size(px(7.0))
                    .rounded_full()
                    .bg(gpui::rgb(0xF59E0B)),
            )
        })
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
        let has_children = !node.children.is_empty();
        let collapsed = has_children && app.is_collapsed(&node.path);
        out.push(tree_row(
            node,
            depth,
            active,
            is_fav,
            has_children,
            collapsed,
            window,
            cx,
        ));
        // A collapsed node hides its subtree.
        if !collapsed {
            push_tree_rows(&node.children, depth + 1, app, window, cx, out);
        }
    }
}

/// Width of the disclosure-chevron gutter at the start of every tree row (the
/// chevron sits here for nodes with children; leaves leave it blank, so all
/// rows at a level align). Recent and favorite rows reserve it; the pinned
/// Journal link doesn't.
const CHEVRON_W: f32 = 16.0;

/// A tree row's left padding (px) at `depth`; the base matches the other rows'
/// `px_2`. Used both to indent the row and to work out how much room the title
/// has before it's clipped.
fn row_indent(depth: usize) -> f32 {
    8.0 + depth as f32 * 14.0
}

/// Whether `label`, rendered at the row's 13px, is wider than the room left in a
/// row with `pad_left` left padding — i.e. it will be ellipsized, so it wants a
/// tooltip. The sidebar is a fixed 240px; the scroll area pads 8px per side and a
/// row pads `pad_left` on the left and 8px (`pr_2`) on the right; the rest is the
/// text box.
fn label_overflows(label: &SharedString, pad_left: f32, window: &Window) -> bool {
    let avail = 240.0 - 16.0 - pad_left - 8.0;
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

/// One row in the recent page tree, indented by `depth`. Shows the leaf segment,
/// opens the page by its full path on click, draws the ancestor indent guides,
/// and — for a node with children — a disclosure chevron that collapses the
/// subtree. Real pages also get the right-click menu; virtual namespace nodes
/// (no page of their own yet) don't.
#[allow(clippy::too_many_arguments)]
fn tree_row(
    node: &PageNode,
    depth: usize,
    active: bool,
    is_fav: bool,
    has_children: bool,
    collapsed: bool,
    window: &mut Window,
    cx: &mut Context<AppView>,
) -> AnyElement {
    let label: SharedString = node.segment.clone().into();
    let full_path: SharedString = node.path.clone().into();
    // Text sits past the chevron gutter; the chevron (if any) and ancestor
    // guides live in the indent to its left.
    let pad_left = row_indent(depth) + CHEVRON_W;
    let truncated = label_overflows(&label, pad_left, window);
    let click_path = node.path.clone();
    let mut row = base_row(
        SharedString::from(format!("pn:{full_path}")),
        &label,
        active,
        pad_left,
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
    for level in 1..=depth {
        row = row.child(guide_line(level));
    }
    if has_children {
        row = row.child(chevron(full_path.clone(), collapsed, row_indent(depth), cx));
    }
    match node.id {
        Some(id) => with_page_menu(row, id, full_path, is_fav, cx),
        None => row.into_any_element(),
    }
}

/// A favorites-group row: the pinned page shown by its **full** title (flat, no
/// children), aligned with the recent rows' text and carrying the same click +
/// right-click menu.
fn favorite_row(
    page: &Page,
    app: &AppView,
    window: &mut Window,
    cx: &mut Context<AppView>,
) -> AnyElement {
    let title: SharedString = page.title.clone().into();
    let pad_left = row_indent(0) + CHEVRON_W;
    let truncated = label_overflows(&title, pad_left, window);
    // Open by id so a favorited whiteboard routes to the canvas (opening by
    // title would hit `get_or_create_page` and mis-open / duplicate it).
    let id = page.id;
    let mut row = base_row(
        SharedString::from(format!("fav:{}", page.id)),
        &title,
        app.is_page_active(page.id),
        pad_left,
    )
    .on_click(
        cx.listener(move |this: &mut AppView, _: &ClickEvent, window, cx| {
            this.open_page_id(id, window, cx);
        }),
    );
    if truncated {
        let full = title.clone();
        row = row.tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx));
    }
    with_page_menu(row, page.id, title, true, cx)
}

/// A "Whiteboards" section row: a board shown by its full title, opened by id
/// (so it routes to the canvas viewer, not the markdown editor), with the same
/// right-click menu (rename / delete / favorite) as page rows.
fn whiteboard_row(
    page: &Page,
    app: &AppView,
    window: &mut Window,
    cx: &mut Context<AppView>,
) -> AnyElement {
    let title: SharedString = page.title.clone().into();
    let pad_left = row_indent(0) + CHEVRON_W;
    let truncated = label_overflows(&title, pad_left, window);
    let id = page.id;
    let mut row = base_row(
        SharedString::from(format!("wb:{}", page.id)),
        &title,
        app.is_page_active(page.id),
        pad_left,
    )
    .on_click(
        cx.listener(move |this: &mut AppView, _: &ClickEvent, window, cx| {
            this.open_page_id(id, window, cx);
        }),
    );
    if truncated {
        let full = title.clone();
        row = row.tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx));
    }
    with_page_menu(row, page.id, title, app.is_favorite(page.id), cx)
}

/// The "new whiteboard" button in the sidebar's top toolbar — the Lucide
/// `clipboard-plus` icon (bundled in `assets/icons`, served by the app's asset
/// source), labelled by a tooltip.
fn new_whiteboard_icon(cx: &mut Context<AppView>) -> impl IntoElement {
    icon_btn(
        "new-whiteboard",
        Icon::empty().path("icons/clipboard-plus.svg"),
    )
    .on_click(
        cx.listener(|this: &mut AppView, _: &ClickEvent, window, cx| {
            this.new_whiteboard(window, cx);
        }),
    )
    .tooltip(|window, cx| Tooltip::new("New whiteboard").build(window, cx))
}

/// Shared styling for a clickable sidebar page row (the caller chains `on_click`,
/// guides, chevron, menu). `label` is what's shown; `pad_left` is its left
/// padding (indent + chevron gutter).
fn base_row(id: SharedString, label: &SharedString, active: bool, pad_left: f32) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .pl(px(pad_left))
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
        .child(label.clone())
}

/// A faint vertical guide for ancestor `level` — like the nested markdown list,
/// so the subpage hierarchy reads at a glance. An overlay in the left padding
/// (left of the text), aligned under the parent's chevron; flush rows join the
/// segments into a continuous line, and the full-width highlight is untouched.
fn guide_line(level: usize) -> impl IntoElement {
    let x = row_indent(level - 1) + CHEVRON_W / 2.0;
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .left(px(x))
        .w(px(1.0))
        .bg(theme::border_subtle())
}

/// The disclosure chevron for a node with children, in its gutter at `x`. Toggles
/// the node's collapsed state on press, stopping propagation so the row's
/// click-to-open doesn't also fire.
fn chevron(
    path: SharedString,
    collapsed: bool,
    x: f32,
    cx: &mut Context<AppView>,
) -> impl IntoElement {
    let icon = if collapsed {
        IconName::ChevronRight
    } else {
        IconName::ChevronDown
    };
    let toggle_path = path.to_string();
    div()
        .id(SharedString::from(format!("chev:{path}")))
        .absolute()
        .top_0()
        .bottom_0()
        .left(px(x))
        .w(px(CHEVRON_W))
        .flex()
        .items_center()
        .justify_center()
        .text_color(theme::text_tertiary())
        .cursor_pointer()
        .hover(|h| h.text_color(theme::text_primary()))
        .child(Icon::new(icon).with_size(px(12.0)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this: &mut AppView, _, _window, cx| {
                cx.stop_propagation();
                this.toggle_collapsed(&toggle_path, cx);
            }),
        )
}

/// Attach the page right-click menu (favorite toggle / open elsewhere / rename /
/// delete) to a built row. Shared by recent and favorite rows.
fn with_page_menu(
    row: Stateful<Div>,
    id: i64,
    full_path: SharedString,
    is_fav: bool,
    cx: &mut Context<AppView>,
) -> AnyElement {
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
            .menu("Export as PDF…", Box::new(ExportPdf))
            .separator()
            .menu("Rename page", Box::new(RenamePage))
            .menu("Delete page", Box::new(DeletePage))
    })
    .into_any_element()
}

/// A collapsible section header: the uppercase title, a hairline rule, and a
/// disclosure chevron at the right end of the rule. Clicking the header toggles
/// section `key`.
fn section_header(
    title: &str,
    key: &'static str,
    collapsed: bool,
    cx: &mut Context<AppView>,
) -> AnyElement {
    let chev = if collapsed {
        IconName::ChevronRight
    } else {
        IconName::ChevronDown
    };
    div()
        .id(SharedString::from(format!("sec:{key}")))
        .px_2()
        .pt_4()
        .pb_1()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::accent())
                .child(title.to_uppercase()),
        )
        // A hairline rule fills the rest of the row, separating the groups.
        .child(div().flex_1().h(px(1.0)).bg(theme::divider()))
        // The disclosure chevron, at the right end of the rule.
        .child(
            div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .text_color(theme::text_tertiary())
                .child(Icon::new(chev).with_size(px(12.0))),
        )
        .on_click(
            cx.listener(move |this: &mut AppView, _, _window, cx| this.toggle_section(key, cx)),
        )
        .into_any_element()
}

fn empty_hint(text: &str) -> impl IntoElement {
    div()
        .px_2()
        .py_1()
        .text_size(px(12.0))
        .text_color(theme::text_tertiary())
        .child(text.to_string())
}
