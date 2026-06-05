//! The infinite journal feed: today on top, older days below, each a
//! single multi-line markdown editor. Scrolling near the bottom grows
//! the feed (also via the "Load older days" affordance).

use gpui::{
    ClickEvent, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::input::{Input, InputState};

use crate::app::{self, AppView};
use crate::theme;

pub fn render(app: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let mut sections = Vec::new();
    for i in 0..app.loaded_days {
        let date = app::date_for_offset(i);
        if let Some(day) = app.day_editors.get(&date) {
            sections.push(day_section(i, &day.state).into_any_element());
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

fn day_section(i: usize, state: &Entity<InputState>) -> impl IntoElement {
    let is_today = i == 0;
    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if is_today { theme::accent() } else { theme::text_secondary() })
                .child(app::date_label(i)),
        )
        .child(
            Input::new(state)
                .appearance(false)
                .text_size(px(16.0))
                .text_color(theme::text_primary()),
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
        .on_click(cx.listener(|this: &mut AppView, _: &ClickEvent, window, cx| {
            this.extend_feed(window, cx);
        }))
}
