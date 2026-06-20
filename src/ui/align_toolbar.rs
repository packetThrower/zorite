//! The table column-alignment toolbar (Left / Center / Right), shown while the
//! caret is in a table cell and anchored just above the caret. Clicking a button
//! rewrites that column's alignment in the table's `|---|` separator row.

use gpui::{
    Context, Entity, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement,
    Styled, div, px,
};
use gpui_editor::{CellAlign, EditorState};

use crate::app::AppView;
use crate::theme;

pub fn render(
    current: CellAlign,
    editor: Entity<EditorState>,
    cx: &mut Context<AppView>,
) -> impl IntoElement {
    let mut row = div()
        .occlude()
        .flex()
        .flex_row()
        .gap(px(2.0))
        .bg(theme::elevated())
        .border_1()
        .border_color(theme::border_subtle())
        .rounded(px(6.0))
        .p(px(3.0));
    for (align, label) in [
        (CellAlign::Left, "L"),
        (CellAlign::Center, "C"),
        (CellAlign::Right, "R"),
    ] {
        let selected = current == align;
        let ed = editor.clone();
        row = row.child(
            div()
                .px(px(8.0))
                .py(px(2.0))
                .rounded(px(4.0))
                .border_1()
                .border_color(if selected {
                    theme::accent()
                } else {
                    theme::border_subtle()
                })
                .bg(theme::glass())
                .text_size(px(12.0))
                .text_color(if selected {
                    theme::text_primary()
                } else {
                    theme::text_secondary()
                })
                .child(label)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |_this: &mut AppView, _: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        ed.update(cx, |e, cx| e.set_caret_table_align(align, cx));
                    }),
                ),
        );
    }
    row
}
