//! The infinite journal feed: today on top, older days below. The day
//! you're editing shows a raw markdown editor; every other day renders
//! as formatted markdown — click a day to edit it.

use gpui::{
    ClickEvent, Context, Entity, ExternalPaths, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Pixels, SharedString, StatefulInteractiveElement, Styled, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::input::{Input, InputState};

use crate::app::{self, AppView};
use crate::slash::SlashTarget;
use crate::theme;

pub fn render(app: &AppView, day_min: Pixels, cx: &mut Context<AppView>) -> impl IntoElement {
    let mut sections = Vec::new();
    for i in 0..app.loaded_days {
        let date = app::date_for_offset(i);
        if let Some(day) = app.day_editors.get(&date) {
            sections.push(day_section(app, i, &date, &day.state, day_min, cx).into_any_element());
        }
    }

    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(theme::bg_content())
        .child(
            div()
                .id("feed")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&app.feed_scroll)
                .on_scroll_wheel(cx.listener(|this: &mut AppView, _ev, window, cx| {
                    this.maybe_extend_feed(window, cx);
                }))
                .child(
                    div()
                        // Uniform padding on all sides; left-aligned (no
                        // centering) so content isn't pushed into the middle.
                        .p(px(28.0))
                        .flex()
                        .flex_col()
                        .gap(px(40.0))
                        .children(sections)
                        .child(load_older(cx)),
                ),
        )
}

fn day_section(
    app: &AppView,
    i: usize,
    date: &str,
    state: &Entity<InputState>,
    day_min: Pixels,
    cx: &mut Context<AppView>,
) -> impl IntoElement {
    // The date in the accent color (every day, not just today) so each day's
    // start clearly stands apart from the dark body text and headings.
    let header = div()
        .text_size(px(22.0))
        .font_weight(FontWeight::BOLD)
        .text_color(theme::accent())
        .child(app::date_label(i));

    let body = if app.is_editing_day(date) {
        Input::new(state)
            .appearance(false)
            .text_size(px(16.0))
            .text_color(theme::text_primary())
            .into_any_element()
    } else {
        rendered_day(app, i, date, state.read(cx).value(), cx).into_any_element()
    };

    let drop_date = date.to_string();
    div()
        .flex()
        .flex_col()
        // Each day fills most of the window so days read as distinct pages.
        .min_h(day_min)
        .gap(px(8.0))
        // A hairline above each day (except today), centered in the gap, to
        // clearly break the feed into separate days.
        .when(i > 0, |d| {
            d.pt(px(40.0)).border_t_1().border_color(theme::divider())
        })
        // Drop image files onto a day to add them to it.
        .on_drop(cx.listener(
            move |this: &mut AppView, paths: &ExternalPaths, window, cx| {
                this.insert_dropped_files(
                    SlashTarget::Day(drop_date.clone()),
                    paths.paths(),
                    window,
                    cx,
                );
            },
        ))
        .child(header)
        .child(body)
        .child(day_open_area(i, date, cx))
}

/// The empty space filling the rest of a day below its content. Clicking it
/// enters edit mode with the caret on a trailing blank line, so the whole day
/// reads as one writable surface — not just the lines that already have text.
fn day_open_area(i: usize, date: &str, cx: &mut Context<AppView>) -> impl IntoElement {
    let d = date.to_string();
    div()
        .id(("day-open", i))
        .flex_1()
        .min_h(px(60.0))
        .w_full()
        .cursor_text()
        .on_click(
            cx.listener(move |this: &mut AppView, _: &ClickEvent, window, cx| {
                this.edit_day_at_end(&d, window, cx);
            }),
        )
}

/// A non-editing day: rendered markdown (or a placeholder when empty),
/// clickable to enter edit mode.
fn rendered_day(
    app: &AppView,
    i: usize,
    date: &str,
    content: SharedString,
    cx: &mut Context<AppView>,
) -> impl IntoElement {
    let d = date.to_string();
    let inner = if content.trim().is_empty() {
        div()
            .text_size(px(16.0))
            .text_color(theme::text_tertiary())
            .child("Empty — click to write")
            .into_any_element()
    } else {
        let weak = cx.entity().downgrade();
        let click_weak = cx.entity().downgrade();
        let click_date = d.clone();
        let mut md = gpui_markdown::MarkdownView::new(format!("day-md-{i}"), content)
            .style(theme::markdown_style(app.list_indent()))
            .on_image(crate::ui::image::renderer(
                app,
                SlashTarget::Day(d.clone()),
                cx,
            ))
            .on_wiki_link(std::rc::Rc::new(move |title, window, cx| {
                let _ = weak.update(cx, |this, cx| this.open_page_title(&title, window, cx));
            }))
            // Click the rendered text → enter edit mode with the caret at the click.
            // Deferred so we don't swap to the editor mid-click.
            .on_click_source(std::rc::Rc::new(move |offset, click_y, window, cx| {
                let click_weak = click_weak.clone();
                let date = click_date.clone();
                window.defer(cx, move |window, cx| {
                    let _ = click_weak.update(cx, |this, cx| {
                        this.edit_day_at_offset(&date, offset, click_y, window, cx)
                    });
                });
            }));
        // Track the markdown root's bounds — click-to-caret's scroll anchor.
        if let Some(de) = app.day_editors.get(date) {
            md = md.track_blocks(de.md_scroll.clone());
        }
        md.into_any_element()
    };
    div()
        .id(("day-body", i))
        .w_full()
        .min_h(px(24.0))
        .cursor_text()
        .child(inner)
        .on_click(
            cx.listener(move |this: &mut AppView, _: &ClickEvent, window, cx| {
                this.edit_day(&d, window, cx);
            }),
        )
}

fn load_older(cx: &mut Context<AppView>) -> impl IntoElement {
    div()
        .id("load-older")
        .w_full()
        .py(px(8.0))
        .flex()
        .justify_center()
        .text_size(px(12.0))
        .text_color(theme::text_tertiary())
        .cursor_pointer()
        .hover(|h| h.text_color(theme::text_secondary()))
        .child("Load older days")
        .on_click(
            cx.listener(|this: &mut AppView, _: &ClickEvent, window, cx| {
                this.extend_feed(window, cx);
            }),
        )
}
