//! In-place editor for a run of `key:: value` properties. The host seats it in
//! the WYSIWYG editor's reserved gap (the same mechanism the math structural
//! editor uses) when the caret enters a property panel; on blur the host reads
//! [`PropertyEditor::to_source`] and writes the `key:: value` lines back.
//!
//! v1: a plain text field per key and value, with add/remove rows. A key
//! dropdown (fed by `Db::property_index`) and value pill-chips come next.

use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::input::{Input, InputState};

use crate::theme;

pub struct PropertyEditor {
    rows: Vec<Row>,
    focus: FocusHandle,
}

struct Row {
    key: Entity<InputState>,
    value: Entity<InputState>,
}

impl PropertyEditor {
    /// Build fields from the raw property block (`key:: value` lines).
    pub fn new(source: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let rows = parse(source)
            .into_iter()
            .map(|(k, v)| Row::new(&k, &v, window, cx))
            .collect();
        let focus = cx.focus_handle();
        Self { rows, focus }
    }

    /// Focus the first value field (or the container when empty) so a
    /// subsequent click-away blurs the whole editor and commits.
    pub fn focus_first(&self, window: &mut Window, cx: &mut Context<Self>) {
        match self.rows.first() {
            Some(row) => row.value.update(cx, |s, cx| s.focus(window, cx)),
            None => self.focus.focus(window, cx),
        }
    }

    /// The current fields serialized back to `key:: value` lines (empty-key rows
    /// dropped). This is what the host writes over the source block on commit.
    pub fn to_source(&self, cx: &App) -> String {
        self.rows
            .iter()
            .filter_map(|r| {
                let k = r.key.read(cx).value().trim().to_string();
                if k.is_empty() {
                    return None;
                }
                let v = r.value.read(cx).value().trim().to_string();
                Some(format!("{k}:: {v}"))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let row = Row::new("", "", window, cx);
        row.key.update(cx, |s, cx| s.focus(window, cx));
        self.rows.push(row);
        cx.notify();
    }

    fn remove_row(&mut self, i: usize, cx: &mut Context<Self>) {
        if i < self.rows.len() {
            self.rows.remove(i);
            cx.notify();
        }
    }
}

impl Row {
    fn new(key: &str, value: &str, window: &mut Window, cx: &mut Context<PropertyEditor>) -> Self {
        let key = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("key")
                .default_value(key.to_string())
        });
        let value = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("value")
                .default_value(value.to_string())
        });
        Self { key, value }
    }
}

impl Focusable for PropertyEditor {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for PropertyEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<_> = self
            .rows
            .iter()
            .enumerate()
            .map(|(i, r)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(div().w(px(140.0)).flex_shrink_0().child(Input::new(&r.key)))
                    .child(div().flex_1().child(Input::new(&r.value)))
                    .child(
                        div()
                            .id(("prop-remove", i))
                            .flex_shrink_0()
                            .px(px(6.0))
                            .text_color(theme::text_tertiary())
                            .cursor_pointer()
                            .hover(|s| s.text_color(theme::text_primary()))
                            .child("✕")
                            .on_click(cx.listener(move |this: &mut PropertyEditor, _, _w, cx| {
                                this.remove_row(i, cx);
                            })),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .gap(px(4.0))
            .p(px(6.0))
            .rounded(px(8.0))
            .bg(theme::elevated())
            .border_1()
            .border_color(theme::divider())
            .children(rows)
            .child(
                div()
                    .id("prop-add")
                    .mt(px(2.0))
                    .px(px(6.0))
                    .py(px(3.0))
                    .text_size(px(12.0))
                    .text_color(theme::accent())
                    .cursor_pointer()
                    .hover(|s| s.text_color(theme::text_primary()))
                    .child("+ Add property")
                    .on_click(cx.listener(|this: &mut PropertyEditor, _, w, cx| {
                        this.add_row(w, cx);
                    })),
            )
    }
}

/// Split a property block into `(key, value)` pairs, ignoring lines that aren't
/// properties (via the shared grammar).
fn parse(source: &str) -> Vec<(String, String)> {
    source
        .lines()
        .filter_map(|l| {
            gpui_markdown::syntax::property(l).map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parse_reads_property_lines_only() {
        let rows = parse("attendees:: Bob, Sue\ntime:: 3:00pm\njust prose");
        assert_eq!(
            rows,
            vec![
                ("attendees".to_string(), "Bob, Sue".to_string()),
                ("time".to_string(), "3:00pm".to_string()),
            ]
        );
    }
}
