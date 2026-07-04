//! In-place editor for a run of `key:: value` properties. The host seats it in
//! the WYSIWYG editor's reserved gap (the same mechanism the math structural
//! editor uses) when the caret enters a property panel; on blur the host reads
//! [`PropertyEditor::to_source`] and writes the `key:: value` lines back.
//!
//! The form mirrors the rendered panel: each row shows the key's icon + a muted
//! key and the value as pills, exactly like the read view — and a cell becomes
//! an editable field only while it holds focus (click a cell to edit it). A
//! focused key field also drops an autocomplete of keys already used across the
//! vault (from `Db::property_index`); free text is allowed, so new keys can be
//! typed.

use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Window, deferred, div, px, svg,
};
use gpui_component::input::{Input, InputState};

use crate::theme;

pub struct PropertyEditor {
    rows: Vec<Row>,
    /// Property keys already used across the vault — the autocomplete source.
    keys: Vec<SharedString>,
    focus: FocusHandle,
}

struct Row {
    key: Entity<InputState>,
    value: Entity<InputState>,
}

impl PropertyEditor {
    /// Build fields from the raw property block (`key:: value` lines); `keys`
    /// seeds the key autocomplete.
    pub fn new(
        source: &str,
        keys: Vec<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let rows = parse(source)
            .into_iter()
            .map(|(k, v)| Row::new(&k, &v, window, cx))
            .collect();
        let focus = cx.focus_handle();
        Self { rows, keys, focus }
    }

    /// Focus a value field on open: the last row's when entered by arrowing up
    /// from below (`at_end`), else the first — so the caret lands where it came
    /// from. Empty block focuses the container so a click-away still commits.
    pub fn focus_end(&self, at_end: bool, window: &mut Window, cx: &mut Context<Self>) {
        let row = if at_end {
            self.rows.last()
        } else {
            self.rows.first()
        };
        match row {
            Some(r) => r.value.update(cx, |s, cx| s.focus(window, cx)),
            None => self.focus.focus(window, cx),
        }
    }

    /// The `(row, is_key)` of the currently focused field, if any.
    fn focused_cell(&self, window: &Window, cx: &App) -> Option<(usize, bool)> {
        self.rows.iter().enumerate().find_map(|(i, r)| {
            if r.key.read(cx).focus_handle(cx).is_focused(window) {
                Some((i, true))
            } else if r.value.read(cx).focus_handle(cx).is_focused(window) {
                Some((i, false))
            } else {
                None
            }
        })
    }

    /// Focus row `row`'s key or value field (a no-op if out of range).
    fn focus_cell(&self, row: usize, is_key: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(r) = self.rows.get(row) {
            let input = if is_key { &r.key } else { &r.value };
            input.update(cx, |s, cx| s.focus(window, cx));
            cx.notify();
        }
    }

    /// Up/Down move between rows in the same column. Only these reach here: the
    /// text input claims Left/Right/Tab (and every other bound key) via its own
    /// key context, and gpui routes bound keys straight to the input — a capture
    /// handler never sees them — so horizontal movement stays inside the field.
    fn nav_key(&mut self, ev: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some((row, is_key)) = self.focused_cell(window, cx) else {
            return;
        };
        let n = self.rows.len();
        let handled = match ev.keystroke.key.as_str() {
            "up" if row > 0 => {
                self.focus_cell(row - 1, is_key, window, cx);
                true
            }
            "down" if row + 1 < n => {
                self.focus_cell(row + 1, is_key, window, cx);
                true
            }
            _ => false,
        };
        if handled {
            cx.stop_propagation();
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

    fn focus_key(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.get(i) {
            row.key.update(cx, |s, cx| s.focus(window, cx));
        }
        cx.notify();
    }

    fn focus_value(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.get(i) {
            row.value.update(cx, |s, cx| s.focus(window, cx));
        }
        cx.notify();
    }

    /// Pick a key from the autocomplete: fill the field, then move focus to the
    /// value (which closes the dropdown).
    fn pick_key(
        &mut self,
        i: usize,
        key: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(row) = self.rows.get(i) {
            row.key.update(cx, |s, cx| s.set_value(key, window, cx));
            row.value.update(cx, |s, cx| s.focus(window, cx));
        }
        cx.notify();
    }

    /// The autocomplete panel for row `i`'s key field (vault keys, filtered by
    /// what's typed), dropped directly below the field.
    fn key_autocomplete(&self, i: usize, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let typed = self.rows.get(i)?.key.read(cx).value().to_lowercase();
        let exact = self.keys.iter().any(|k| k.to_lowercase() == typed);
        let matches: Vec<SharedString> = if typed.is_empty() || exact {
            self.keys.clone()
        } else {
            self.keys
                .iter()
                .filter(|k| k.to_lowercase().contains(&typed))
                .cloned()
                .collect()
        };
        if matches.is_empty() {
            return None;
        }
        let items: Vec<_> = matches
            .into_iter()
            .enumerate()
            .map(|(n, k)| {
                let key = k.clone();
                div()
                    .id(("prop-key-opt", i * 1000 + n))
                    .px(px(10.0))
                    .py(px(4.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::accent_tint()))
                    .child(k)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                            this.pick_key(i, key.clone(), window, cx);
                        }),
                    )
                    .into_any_element()
            })
            .collect();
        Some(
            deferred(
                div()
                    .absolute()
                    .top_full()
                    .left_0()
                    .mt(px(2.0))
                    .w(px(220.0))
                    .occlude()
                    .bg(theme::elevated())
                    .border_1()
                    .border_color(theme::divider())
                    .rounded(px(6.0))
                    .text_color(theme::text_primary())
                    .text_size(px(13.0))
                    .children(items),
            )
            .into_any_element(),
        )
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<_> = (0..self.rows.len())
            .map(|i| {
                let r = &self.rows[i];
                let key_focused = r.key.read(cx).focus_handle(cx).is_focused(window);
                let value_focused = r.value.read(cx).focus_handle(cx).is_focused(window);
                let key_val = r.key.read(cx).value();
                let value_val = r.value.read(cx).value();
                let icon = theme::property_icon(&key_val);
                let dropdown = key_focused.then(|| self.key_autocomplete(i, cx)).flatten();

                // The inputs are ALWAYS rendered (so focusing them from a click
                // works); when a cell isn't focused, a panel-styled overlay (muted
                // key / value pills) covers its input and, on click, focuses it.
                let key_overlay = (!key_focused).then(|| {
                    let label = if key_val.is_empty() {
                        SharedString::from("key")
                    } else {
                        key_val
                    };
                    div()
                        .id(("prop-key-ov", i))
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .px(px(2.0))
                        .bg(theme::elevated())
                        .text_color(theme::text_tertiary())
                        .cursor_pointer()
                        .child(label)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                this.focus_key(i, window, cx);
                            }),
                        )
                });
                let value_overlay = (!value_focused).then(|| {
                    div()
                        .id(("prop-val-ov", i))
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .px(px(2.0))
                        .bg(theme::elevated())
                        .cursor_pointer()
                        .child(value_display(&value_val))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                this.focus_value(i, window, cx);
                            }),
                        )
                });

                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .relative()
                            .w(px(150.0))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .children(icon.map(|p| {
                                svg()
                                    .path(p)
                                    .w(px(16.0))
                                    .h(px(16.0))
                                    .text_color(theme::text_tertiary())
                                    .flex_shrink_0()
                            }))
                            .child(
                                div()
                                    .relative()
                                    .flex_1()
                                    .child(Input::new(&self.rows[i].key).appearance(false))
                                    .children(key_overlay),
                            )
                            .children(dropdown),
                    )
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .child(Input::new(&self.rows[i].value).appearance(false))
                            .children(value_overlay),
                    )
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
            .capture_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, window, cx| {
                this.nav_key(ev, window, cx);
            }))
            .flex()
            .flex_col()
            .gap(px(2.0))
            // Keep the block compact so the row-delete ✕ sits near the values,
            // not flung to the far edge of the note.
            .max_w(px(480.0))
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

/// The value rendered like the panel: plain runs, tags/wiki-links as pills.
fn value_display(value: &str) -> impl IntoElement {
    let mut row = div().flex().flex_wrap().items_center().gap(px(5.0));
    for seg in gpui_markdown::syntax::property_value_segments(value) {
        match seg {
            gpui_markdown::syntax::PropSeg::Text(t) => {
                let t = t.trim();
                if !t.is_empty() {
                    row = row.child(div().text_color(theme::text_primary()).child(t.to_string()));
                }
            }
            gpui_markdown::syntax::PropSeg::Pill { label, is_tag, .. } => {
                let color = if is_tag {
                    theme::tag()
                } else {
                    theme::accent()
                };
                let mut bg = color;
                bg.a = 0.16;
                row = row.child(
                    div()
                        .px(px(7.0))
                        .py(px(1.0))
                        .rounded(px(6.0))
                        .bg(bg)
                        .text_color(color)
                        .child(label),
                );
            }
        }
    }
    row
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
