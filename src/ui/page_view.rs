//! A single named/journal page: title, its markdown editor, and a
//! "Linked References" panel.

use gpui::{
    ClickEvent, Context, ExternalPaths, FontWeight, InteractiveElement, IntoElement, MouseButton,
    ParentElement, StatefulInteractiveElement, Styled, div, prelude::FluentBuilder as _, px,
    relative,
};
use gpui_component::input::Input;
use gpui_component::menu::ContextMenuExt;

use crate::actions::EditNote;
use crate::app::{AppView, PageEditor, PageFind};
use crate::hierarchy;
use crate::models::{Backlink, Page};
use crate::slash::SlashTarget;
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

    let page_id = pe.id;
    // Pages titled `<this>::<leaf>` are sub-pages; this page acts as their index.
    let children = hierarchy::direct_children(&app.pages, &pe.title);
    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .bg(theme::bg_content())
        // The find bar (⌘F) sits above the scrollable content so it stays put
        // while you step through matches.
        .children(app.page_find.as_ref().map(|pf| find_bar(pf, cx)))
        .child(
            div()
                .id("page-scroll")
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_y_scroll()
                .track_scroll(&app.page_scroll)
                // Drop image files onto the page to add them.
                .on_drop(cx.listener(
                    move |this: &mut AppView, paths: &ExternalPaths, window, cx| {
                        this.insert_dropped_files(
                            SlashTarget::Page(page_id),
                            paths.paths(),
                            window,
                            cx,
                        );
                    },
                ))
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
                            page_rendered(app, pe, cx).into_any_element()
                        })
                        // A large editable surface right under the content (like the
                        // journal's open day area), so the page stays easy to click
                        // into even when a PDF chip fills the body and sub-page /
                        // reference sections sit below. It grows to fill, pushing
                        // those sections to the bottom.
                        .child(page_open_area(page_id, cx))
                        .when(!children.is_empty(), |this| {
                            this.child(sub_pages_section(&pe.title, &children, cx))
                        })
                        .when(!pe.backlinks.is_empty(), |this| {
                            this.child(backlinks_section(&pe.backlinks, cx))
                        }),
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
            .flex()
            .flex_col()
            .gap_1()
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
            .child(alias_row(pe))
            .into_any_element()
    }
}

/// The subdued `alias::` field under a named page's title — edits the page's
/// aliases as a comma-separated list (committed on Enter/blur). Replaces typing
/// the property in the body.
fn alias_row(pe: &PageEditor) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .text_size(px(12.0))
        .text_color(theme::text_tertiary())
        .child("alias::")
        .child(
            div().flex_1().min_w_0().child(
                Input::new(&pe.alias_state)
                    .appearance(false)
                    .text_size(px(12.0))
                    .line_height(px(16.0))
                    .py(px(0.0))
                    .h(px(18.0))
                    .text_color(theme::text_secondary()),
            ),
        )
}

/// The page body in reading mode: rendered markdown (or a placeholder
/// when empty), clickable to enter edit mode.
fn page_rendered(app: &AppView, pe: &PageEditor, cx: &mut Context<AppView>) -> impl IntoElement {
    let content = pe.state.read(cx).value();
    let inner = if content.trim().is_empty() {
        div()
            .text_size(px(16.0))
            .text_color(theme::text_tertiary())
            .child("Empty — click to write")
            .into_any_element()
    } else {
        let weak = cx.entity().downgrade();
        let click_weak = cx.entity().downgrade();
        let mut md = gpui_markdown::MarkdownView::new("page-md", content)
            .style(theme::markdown_style(app.list_indent()))
            // Track block bounds so find can scroll the active match into view.
            .track_blocks(app.md_block_scroll.clone())
            .on_image(crate::ui::image::renderer(
                app,
                SlashTarget::Page(pe.id),
                cx,
            ))
            .on_mermaid(crate::ui::mermaid::renderer(app, cx))
            .on_wiki_link(std::rc::Rc::new(move |title, window, cx| {
                let _ = weak.update(cx, |this, cx| this.open_page_title(&title, window, cx));
            }))
            // Click the rendered text → enter edit mode with the caret at the click.
            // Deferred so we don't swap to the editor mid-click.
            .on_click_source(std::rc::Rc::new(move |offset, click_y, window, cx| {
                let click_weak = click_weak.clone();
                window.defer(cx, move |window, cx| {
                    let _ = click_weak.update(cx, |this, cx| {
                        this.edit_page_at_offset(offset, click_y, window, cx)
                    });
                });
            }));
        // Paint in-page find matches (⌘F) when the bar is open.
        if let Some(pf) = app.page_find.as_ref() {
            md = md.search(pf.query.clone(), pf.current);
        }
        md.into_any_element()
    };
    let page_id = pe.id;
    div()
        .id("page-body")
        .w_full()
        .min_h(px(24.0))
        .cursor_text()
        .child(inner)
        // Right-click → Edit: remember this page, then `EditNote` puts it in edit mode.
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this: &mut AppView, _, _window, _cx| {
                this.set_context_edit(SlashTarget::Page(page_id));
            }),
        )
        .on_click(
            cx.listener(|this: &mut AppView, _: &ClickEvent, window, cx| {
                this.edit_page(window, cx);
            }),
        )
        // `context_menu` returns a non-interactive wrapper, so it must come last.
        .context_menu(|menu, _window, _cx| menu.menu("Edit", Box::new(EditNote)))
}

/// The in-page find bar (⌘F), shown above a named page. Reads the `PageFind`
/// state; its query field recomputes the match count on change, and the buttons
/// step / close. Lives above the scroll area so it persists while stepping.
fn find_bar(pf: &PageFind, cx: &mut Context<AppView>) -> impl IntoElement {
    let status = if pf.query.is_empty() {
        String::new()
    } else if pf.count == 0 {
        "No matches".to_string()
    } else {
        format!("{} / {}", pf.current + 1, pf.count)
    };
    div()
        .flex_shrink_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(16.0))
        .py(px(8.0))
        .bg(theme::elevated())
        .border_b_1()
        .border_color(theme::border_subtle())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(Input::new(&pf.input).text_size(px(13.0))),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(12.0))
                .text_color(theme::text_secondary())
                .child(status),
        )
        .child(
            find_btn("find-prev", "↑")
                .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| this.page_find_step(-1, cx))),
        )
        .child(
            find_btn("find-next", "↓")
                .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| this.page_find_step(1, cx))),
        )
        .child(
            find_btn("find-close", "✕")
                .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| this.close_page_find(cx))),
        )
}

/// A small clickable glyph button for the find bar (caller attaches `on_click`).
fn find_btn(id: &'static str, glyph: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .w(px(22.0))
        .h(px(22.0))
        .rounded(px(4.0))
        .text_size(px(13.0))
        .text_color(theme::text_secondary())
        .cursor_pointer()
        .hover(|h| h.bg(theme::hover()))
        .child(glyph)
}

/// The large editable surface directly below the page content (and above the
/// sub-pages / references sections). Clicking it enters edit mode with the caret
/// on a trailing blank line — the same affordance as the journal feed's open day
/// area, so the page stays easy to click into even with a PDF chip in the body.
fn page_open_area(page_id: i64, cx: &mut Context<AppView>) -> impl IntoElement {
    div()
        .id("page-open")
        .flex_1()
        .min_h(px(60.0))
        .w_full()
        .cursor_text()
        // Right-click → Edit here too, matching the page body above.
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this: &mut AppView, _, _window, _cx| {
                this.set_context_edit(SlashTarget::Page(page_id));
            }),
        )
        .on_click(
            cx.listener(|this: &mut AppView, _: &ClickEvent, window, cx| {
                this.edit_page_at_end(window, cx);
            }),
        )
        // `context_menu` returns a non-interactive wrapper, so it must come last.
        .context_menu(|menu, _window, _cx| menu.menu("Edit", Box::new(EditNote)))
}

/// The "Sub-pages" index: pages nested directly under this one (`<title>::*`),
/// shown by their leaf segment as a clickable, comma-separated list.
fn sub_pages_section(
    parent_title: &str,
    children: &[&Page],
    cx: &mut Context<AppView>,
) -> impl IntoElement {
    let base = parent_title.len() + hierarchy::SEP.len();
    let last = children.len().saturating_sub(1);
    div()
        .mt(px(28.0))
        .pt_4()
        .border_t_1()
        .border_color(theme::border_subtle())
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .pb_1()
                .text_size(px(11.0))
                .text_color(theme::text_tertiary())
                .child(format!("SUB-PAGES ({})", children.len())),
        )
        .child(
            // One wrapping line of `Leaf, Leaf, Leaf`, each name clickable.
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_y(px(2.0))
                .text_size(px(14.0))
                .children(
                    children
                        .iter()
                        .enumerate()
                        .map(|(i, p)| {
                            let leaf = p.title.get(base..).unwrap_or(&p.title).to_string();
                            sub_page_item(i, p.id, leaf, i != last, cx).into_any_element()
                        })
                        .collect::<Vec<_>>(),
                ),
        )
}

/// One clickable sub-page name, with a trailing comma unless it's the last.
fn sub_page_item(
    i: usize,
    id: i64,
    leaf: String,
    comma: bool,
    cx: &mut Context<AppView>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .flex_shrink_0()
        .child(
            div()
                .id(("subpage", i))
                .py(px(1.0))
                .rounded(px(4.0))
                .text_color(theme::accent())
                .cursor_pointer()
                .hover(|h| h.bg(theme::glass()))
                .child(leaf)
                .on_click(
                    cx.listener(move |this: &mut AppView, _: &ClickEvent, window, cx| {
                        this.open_page_id(id, window, cx);
                    }),
                ),
        )
        .when(comma, |d| {
            d.child(
                div()
                    .pr(px(5.0))
                    .text_color(theme::text_tertiary())
                    .child(","),
            )
        })
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
