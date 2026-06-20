//! The `/table` rows×cols size picker — a hover grid rendered as an anchored
//! overlay by `AppView`. Hovering a cell previews that many rows × columns;
//! clicking inserts a Markdown table of that size at the `/table` position.

use gpui::{
    Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    ParentElement, Styled, div, px,
};
use gpui_component::{Sizable, input::Input};

use crate::app::{AppView, TableDesign, TablePicker};
use crate::theme;

const MAX_COLS: usize = 8;
const MAX_ROWS: usize = 8;

pub fn render(picker: &TablePicker, cx: &mut Context<AppView>) -> impl IntoElement {
    let (hr, hc) = (picker.rows, picker.cols);
    let label = if hr > 0 && hc > 0 {
        format!("{hc} × {hr}")
    } else {
        "Insert table".to_string()
    };

    let mut grid = div().flex().flex_col().gap(px(3.0));
    for r in 0..MAX_ROWS {
        let mut row = div().flex().flex_row().gap(px(3.0));
        for c in 0..MAX_COLS {
            let selected = r < hr && c < hc;
            let (rr, cc) = (r + 1, c + 1);
            row = row.child(
                div()
                    .size(px(16.0))
                    .rounded(px(2.0))
                    .border_1()
                    .border_color(theme::border_subtle())
                    .bg(if selected {
                        theme::accent()
                    } else {
                        theme::glass()
                    })
                    .on_mouse_move(cx.listener(
                        move |this: &mut AppView, _: &MouseMoveEvent, _, cx| {
                            this.table_picker_hover(rr, cc, cx);
                        },
                    ))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this: &mut AppView, _: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.table_picker_pick(rr, cc, cx);
                        }),
                    ),
            );
        }
        grid = grid.child(row);
        // The first row is always the table's header — mark the boundary so the
        // picker shows what it inserts (header row + `|---|` separator).
        if r == 0 {
            grid = grid.child(
                div()
                    .w_full()
                    .h(px(2.0))
                    .rounded(px(1.0))
                    .bg(theme::divider()),
            );
        }
    }

    // Design buttons (pick a visual style, then a size). The selected one shows an
    // accent border; Grid is the plain default.
    let mut designs = div().flex().flex_row().gap(px(4.0));
    for d in TableDesign::ALL {
        let selected = picker.style == d;
        designs = designs.child(
            div()
                .px(px(7.0))
                .py(px(3.0))
                .rounded(px(5.0))
                .border_1()
                .border_color(if selected {
                    theme::accent()
                } else {
                    theme::border_subtle()
                })
                .bg(theme::glass())
                .text_size(px(11.0))
                .text_color(if selected {
                    theme::text_primary()
                } else {
                    theme::text_secondary()
                })
                .child(d.label())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this: &mut AppView, _: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.table_picker_set_style(d, cx);
                    }),
                ),
        );
    }

    div()
        .occlude()
        .bg(theme::elevated())
        .border_1()
        .border_color(theme::border_subtle())
        .rounded(px(8.0))
        .p(px(8.0))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(designs)
        .child(grid)
        .child(
            div()
                .text_size(px(12.0))
                .text_color(theme::text_secondary())
                .child(label),
        )
        // Custom dimensions, for tables larger than the hover grid.
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(5.0))
                .child(Input::new(&picker.rows_input).small().w(px(46.0)))
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(theme::text_tertiary())
                        .child("×"),
                )
                .child(Input::new(&picker.cols_input).small().w(px(46.0)))
                .child(
                    div()
                        .px(px(8.0))
                        .py(px(3.0))
                        .rounded(px(5.0))
                        .border_1()
                        .border_color(theme::border_subtle())
                        .bg(theme::glass())
                        .text_size(px(12.0))
                        .text_color(theme::text_secondary())
                        .child("Insert")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this: &mut AppView, _: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                this.table_picker_insert_custom(cx);
                            }),
                        ),
                ),
        )
}
