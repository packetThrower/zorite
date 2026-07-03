//! The jump-to-date calendar overlay body: one month grid with a dot under
//! each day that has a journal entry (the reason this is hand-rolled —
//! gpui-component's Calendar has no per-day decoration hook). Clicking any
//! day jumps to it in the journal, entry or not; ‹ › steps months.

use gpui::{
    ClickEvent, Context, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, px,
};

use crate::app::AppView;
use crate::theme;

pub fn render(app: &AppView, cx: &mut Context<AppView>) -> impl IntoElement {
    let (year, month) = app.calendar_month();
    let month_e = time::Month::try_from(month).unwrap_or(time::Month::January);
    let days_in_month = month_e.length(year);
    // Sunday-first column of the month's day 1.
    let lead = time::Date::from_calendar_date(year, month_e, 1)
        .map(|d| d.weekday().number_days_from_sunday() as usize)
        .unwrap_or(0);
    let today = {
        let now = crate::dates::now_local();
        (now.year(), u8::from(now.month()), now.day())
    };

    let nav = |id: &'static str, glyph: &'static str, delta: i32| {
        div()
            .id(id)
            .w(px(24.0))
            .h(px(24.0))
            .rounded(px(6.0))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(13.0))
            .text_color(theme::text_secondary())
            .cursor_pointer()
            .hover(|s| s.bg(theme::hover()).text_color(theme::text_primary()))
            .on_click(
                cx.listener(move |this: &mut AppView, _: &ClickEvent, _w, cx| {
                    this.calendar_shift_month(delta, cx);
                }),
            )
            .child(glyph)
    };

    let mut grid = div().flex().flex_col().gap(px(2.0));
    // Weekday header row.
    grid = grid.child(div().flex().flex_row().children(
        ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"].map(|w| {
            div()
                .w(px(32.0))
                .h(px(20.0))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(10.0))
                .text_color(theme::text_tertiary())
                .child(w)
        }),
    ));
    let mut day: u8 = 1;
    while day <= days_in_month {
        let mut row = div().flex().flex_row();
        for col in 0..7 {
            // Leading blanks before day 1; trailing blanks after month end.
            if (day == 1 && col < lead) || day > days_in_month {
                row = row.child(div().w(px(32.0)).h(px(34.0)));
                continue;
            }
            let iso = format!("{year:04}-{month:02}-{day:02}");
            let has_entry = app.calendar_has_entry(&iso);
            let is_today = today == (year, month, day);
            let mut cell = div()
                .id(("cal-day", day as usize))
                .w(px(32.0))
                .h(px(34.0))
                .rounded(px(6.0))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(1.0))
                .cursor_pointer()
                .hover(|s| s.bg(theme::hover()));
            if is_today {
                cell = cell.border_1().border_color(theme::accent());
            }
            cell = cell
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(if has_entry {
                            theme::text_primary()
                        } else {
                            theme::text_tertiary()
                        })
                        .child(day.to_string()),
                )
                .child({
                    // The entry marker: a small dot (an empty spacer
                    // otherwise, so numbers align across rows).
                    let dot = div().w(px(4.0)).h(px(4.0)).rounded_full();
                    if has_entry {
                        dot.bg(theme::accent())
                    } else {
                        dot
                    }
                });
            row = row.child(cell.on_click(cx.listener(
                move |this: &mut AppView, _: &ClickEvent, window, cx| {
                    this.calendar_pick(&iso, window, cx);
                },
            )));
            day += 1;
        }
        grid = grid.child(row);
    }

    div()
        .p(px(10.0))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(nav("cal-prev", "‹", -1))
                .child(
                    div()
                        .text_size(px(13.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme::text_primary())
                        .child(format!("{} {year}", crate::dates::month_name(month_e))),
                )
                .child(nav("cal-next", "›", 1)),
        )
        .child(grid)
}
