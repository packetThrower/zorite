//! The infinite journal feed: today on top, older days below. The day
//! you're editing shows a raw markdown editor; every other day renders
//! as formatted markdown — click a day to edit it.

use gpui::{
    ClickEvent, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::input::{Input, InputState};
use gpui_component::text::TextView;

use crate::app::{self, AppView};
use crate::theme;

pub fn render(app: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let mut sections = Vec::new();
    for i in 0..app.loaded_days {
        let date = app::date_for_offset(i);
        if let Some(day) = app.day_editors.get(&date) {
            sections.push(day_section(app, i, &date, &day.state, cx).into_any_element());
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
                        .max_w(px(760.0))
                        .mx_auto()
                        .px(px(48.0))
                        .py(px(28.0))
                        .flex()
                        .flex_col()
                        .gap(px(28.0))
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
    cx: &mut Context<AppView>,
) -> impl IntoElement {
    let is_today = i == 0;
    let header = div()
        .text_size(px(13.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(if is_today { theme::accent() } else { theme::text_secondary() })
        .child(app::date_label(i));

    let body = if app.is_editing_day(date) {
        Input::new(state)
            .appearance(false)
            .text_size(px(16.0))
            .text_color(theme::text_primary())
            .into_any_element()
    } else {
        rendered_day(i, date, state.read(cx).value(), cx).into_any_element()
    };

    div().flex().flex_col().gap(px(8.0)).child(header).child(body)
}

/// A non-editing day: rendered markdown (or a placeholder when empty),
/// clickable to enter edit mode.
fn rendered_day(
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
        TextView::markdown(("day-md", i), content)
            .text_size(px(16.0))
            .text_color(theme::text_primary())
            .into_any_element()
    };
    div()
        .id(("day-body", i))
        .w_full()
        .min_h(px(24.0))
        .cursor_text()
        .child(inner)
        .on_click(cx.listener(move |this: &mut AppView, _: &ClickEvent, window, cx| {
            this.edit_day(&d, window, cx);
        }))
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
        .on_click(cx.listener(|this: &mut AppView, _: &ClickEvent, window, cx| {
            this.extend_feed(window, cx);
        }))
}
