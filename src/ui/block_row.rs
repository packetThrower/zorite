//! One outliner row: indent, collapse caret, bullet, the editable body
//! (a chrome-less `Input` when focused, rendered text with clickable
//! `[[links]]` when not), and a delete affordance.
//!
//! Alignment: the caret, bullet, and delete glyph each live in a box of
//! the shared line height with their contents vertically centered, and
//! the body is centered within that same line height. So everything
//! lines up by construction — no per-element top-padding fudging — in
//! both the rendered and editing states, and the bullet stays on the
//! first line when a block wraps.

use gpui::{
    ClickEvent, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, px, prelude::FluentBuilder as _,
};
use gpui_component::input::{Input, InputState};

use crate::app::AppView;
use crate::models::BlockNode;
use crate::theme;
use crate::ui::links::{self, Segment};

/// Horizontal indent per outline level.
const INDENT_PX: f32 = 22.0;
/// Line box height every part of a row aligns to.
const LINE_H: f32 = 24.0;
/// Body text size (shared by the rendered view and the editor).
const TEXT_PX: f32 = 15.0;

pub fn render(
    node: &BlockNode,
    state: &Entity<InputState>,
    focused: bool,
    cx: &mut Context<AppView>,
) -> impl IntoElement {
    let id = node.block.id;

    div()
        .flex()
        .flex_row()
        .items_start()
        .pl(px(node.depth as f32 * INDENT_PX))
        .child(caret(node, cx))
        .child(bullet())
        .child(body(id, state, focused, cx))
        .child(delete_button(id, cx))
}

fn caret(node: &BlockNode, cx: &mut Context<AppView>) -> impl IntoElement {
    let id = node.block.id;
    // Always an id'd (stateful) box so the type is the same whether or
    // not the block has children; childless blocks are a bare spacer.
    div()
        .id(("caret", id as usize))
        .flex_shrink_0()
        .w(px(16.0))
        .h(px(LINE_H))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(10.0))
        .when(node.has_children, |d| {
            d.cursor_pointer()
                .text_color(theme::text_tertiary())
                .hover(|h| h.text_color(theme::text_secondary()))
                .child(if node.block.collapsed { "▸" } else { "▾" })
                .on_click(cx.listener(move |this: &mut AppView, _: &ClickEvent, window, cx| {
                    this.toggle_collapsed(id, window, cx);
                }))
        })
}

fn bullet() -> impl IntoElement {
    div()
        .flex_shrink_0()
        .w(px(18.0))
        .h(px(LINE_H))
        .flex()
        .items_center()
        .justify_center()
        .child(div().w(px(6.0)).h(px(6.0)).rounded_full().bg(theme::bullet()))
}

fn body(
    id: i64,
    state: &Entity<InputState>,
    focused: bool,
    cx: &mut Context<AppView>,
) -> impl IntoElement {
    if focused {
        // Raw editing: chrome-less single-line input, centered in the line.
        return div()
            .flex_1()
            .min_w_0()
            .min_h(px(LINE_H))
            .flex()
            .items_center()
            .child(
                Input::new(state)
                    .appearance(false)
                    .text_size(px(TEXT_PX))
                    .text_color(theme::text_primary()),
            )
            .into_any_element();
    }

    // Rendered: plain runs + clickable [[links]]. The full-width, full
    // line-height row makes even an empty block easy to click to edit.
    let value = state.read(cx).value();
    let mut row = div()
        .id(("body", id as usize))
        .flex_1()
        .min_w_0()
        .min_h(px(LINE_H))
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .text_size(px(TEXT_PX))
        .text_color(theme::text_primary())
        .cursor_pointer()
        .on_click(cx.listener(move |this: &mut AppView, _: &ClickEvent, window, cx| {
            this.focus_block(id, window, cx);
        }));

    for (i, seg) in links::segments(value.as_ref()).into_iter().enumerate() {
        match seg {
            Segment::Text(t) => row = row.child(div().child(t)),
            Segment::Link(title) => {
                let target = title.clone();
                row = row.child(
                    div()
                        .id(("lnk", id as usize * 64 + i))
                        .text_color(theme::link())
                        .cursor_pointer()
                        .hover(|h| h.text_color(theme::accent_hover()))
                        .child(title)
                        .on_click(cx.listener(
                            move |this: &mut AppView, _: &ClickEvent, window, cx| {
                                this.open_page_title(&target, window, cx);
                            },
                        )),
                );
            }
        }
    }
    row.into_any_element()
}

fn delete_button(id: i64, cx: &mut Context<AppView>) -> impl IntoElement {
    div()
        .id(("del", id as usize))
        .flex_shrink_0()
        .w(px(18.0))
        .h(px(LINE_H))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(13.0))
        .text_color(theme::text_tertiary())
        .cursor_pointer()
        .hover(|h| h.text_color(theme::accent()))
        .child("×")
        .on_click(cx.listener(move |this: &mut AppView, _: &ClickEvent, window, cx| {
            this.delete_block(id, window, cx);
        }))
}
