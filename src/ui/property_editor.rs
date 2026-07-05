//! In-place editor for a run of `key:: value` properties. The host seats it in
//! the WYSIWYG editor's reserved gap (the same mechanism the math structural
//! editor uses) when the caret enters a property panel; on blur the host reads
//! [`PropertyEditor::to_source`] and writes the `key:: value` lines back.
//!
//! Custom fields: like the math editor, this owns every keystroke itself (one
//! focus handle for the whole form, per-field text+caret state) rather than
//! delegating to a component that claims the arrow keys. So the caret walks the
//! whole form like a table — Left/Right hop fields at the text edges, Up/Down
//! move rows, Tab steps field-to-field — and an idle field renders like the
//! panel (icon + muted key, value pills). Stage 1 shows plain text while a field
//! is focused; pill-while-editing is Stage 2.

use gpui::{
    App, Context, FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, ParentElement, Render, SharedString, Styled, Window, deferred,
    div, px, svg,
};

use crate::theme;

pub struct PropertyEditor {
    rows: Vec<Row>,
    /// Property keys already used across the vault — the autocomplete source.
    keys: Vec<SharedString>,
    /// The field being edited: `(row, is_key)`. `None` = nothing focused yet.
    active: Option<(usize, bool)>,
    focus: FocusHandle,
}

struct Row {
    key: Field,
    value: Field,
}

/// A single editable text field: its content and the caret's byte offset.
#[derive(Default)]
struct Field {
    text: String,
    caret: usize,
}

impl Field {
    fn new(s: &str) -> Self {
        Self {
            text: s.to_string(),
            caret: s.len(),
        }
    }

    fn insert(&mut self, s: &str) {
        self.text.insert_str(self.caret, s);
        self.caret += s.len();
    }

    fn backspace(&mut self) {
        if self.caret > 0 {
            let prev = prev_boundary(&self.text, self.caret);
            self.text.replace_range(prev..self.caret, "");
            self.caret = prev;
        }
    }

    fn delete(&mut self) {
        if self.caret < self.text.len() {
            let next = next_boundary(&self.text, self.caret);
            self.text.replace_range(self.caret..next, "");
        }
    }

    /// Move the caret left; returns `false` when already at the start (so the
    /// caller can hop to the previous field).
    fn left(&mut self) -> bool {
        if self.caret == 0 {
            return false;
        }
        self.caret = prev_boundary(&self.text, self.caret);
        true
    }

    /// Move the caret right; returns `false` when already at the end.
    fn right(&mut self) -> bool {
        if self.caret >= self.text.len() {
            return false;
        }
        self.caret = next_boundary(&self.text, self.caret);
        true
    }
}

impl PropertyEditor {
    /// Build fields from the raw property block (`key:: value` lines); `keys`
    /// seeds the key autocomplete.
    pub fn new(
        source: &str,
        keys: Vec<SharedString>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let rows = parse(source)
            .into_iter()
            .map(|(k, v)| Row {
                key: Field::new(&k),
                value: Field::new(&v),
            })
            .collect();
        Self {
            rows,
            keys,
            active: None,
            focus: cx.focus_handle(),
        }
    }

    /// Focus a value field on open: the last row's when entered by arrowing up
    /// from below (`at_end`), else the first — so the caret lands where it came
    /// from.
    pub fn focus_end(&mut self, at_end: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            self.focus.focus(window, cx);
            return;
        }
        let row = if at_end { self.rows.len() - 1 } else { 0 };
        // Caret at the value's end when entering from below/right, else start.
        let caret = if at_end {
            self.rows[row].value.text.len()
        } else {
            0
        };
        self.rows[row].value.caret = caret;
        self.active = Some((row, false));
        self.focus.focus(window, cx);
        cx.notify();
    }

    /// The current fields serialized back to `key:: value` lines (empty-key rows
    /// dropped). This is what the host writes over the source block on commit.
    pub fn to_source(&self, _cx: &App) -> String {
        self.rows
            .iter()
            .filter_map(|r| {
                let k = r.key.text.trim();
                if k.is_empty() {
                    return None;
                }
                Some(format!("{k}:: {}", r.value.text.trim()))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn field(&self, row: usize, is_key: bool) -> Option<&Field> {
        self.rows
            .get(row)
            .map(|r| if is_key { &r.key } else { &r.value })
    }

    fn field_mut(&mut self, row: usize, is_key: bool) -> Option<&mut Field> {
        self.rows
            .get_mut(row)
            .map(|r| if is_key { &mut r.key } else { &mut r.value })
    }

    /// Move focus to `(row, is_key)`, seating the caret at `caret_end` (true =
    /// end of the target field, for leftward moves; false = start).
    fn go(&mut self, row: usize, is_key: bool, caret_end: bool) {
        if let Some(f) = self.field_mut(row, is_key) {
            f.caret = if caret_end { f.text.len() } else { 0 };
            self.active = Some((row, is_key));
        }
    }

    /// Step to the next (`forward`) or previous field: key → value → next row's
    /// key, and back. Bound to Tab / Shift+Tab via actions.
    fn tab(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some((row, is_key)) = self.active else {
            return;
        };
        let n = self.rows.len();
        if forward {
            if is_key {
                self.go(row, false, false);
            } else if row + 1 < n {
                self.go(row + 1, true, false);
            }
        } else if !is_key {
            self.go(row, true, true);
        } else if row > 0 {
            self.go(row - 1, false, true);
        }
        cx.notify();
    }

    fn add_row(&mut self, cx: &mut Context<Self>) {
        self.rows.push(Row {
            key: Field::default(),
            value: Field::default(),
        });
        self.active = Some((self.rows.len() - 1, true));
        cx.notify();
    }

    fn remove_row(&mut self, i: usize, cx: &mut Context<Self>) {
        if i < self.rows.len() {
            self.rows.remove(i);
            if self.active.map(|(r, _)| r) == Some(i) {
                self.active = None;
            }
            cx.notify();
        }
    }

    /// All key handling — the form owns every keystroke, so the caret navigates
    /// the whole table.
    fn key_down(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some((row, is_key)) = self.active else {
            return;
        };
        let n = self.rows.len();
        let m = &ev.keystroke.modifiers;
        match ev.keystroke.key.as_str() {
            "left" => {
                let moved = self.field_mut(row, is_key).is_some_and(Field::left);
                if !moved {
                    // Hop to the field on the left, caret at its end.
                    if !is_key {
                        self.go(row, true, true);
                    } else if row > 0 {
                        self.go(row - 1, false, true);
                    }
                }
            }
            "right" => {
                let moved = self.field_mut(row, is_key).is_some_and(Field::right);
                if !moved {
                    if is_key {
                        self.go(row, false, false);
                    } else if row + 1 < n {
                        self.go(row + 1, true, false);
                    }
                }
            }
            "up" if row > 0 => self.go(row - 1, is_key, true),
            "down" if row + 1 < n => self.go(row + 1, is_key, true),
            // Tab / Shift+Tab arrive as the PropNextField / PropPrevField actions
            // (see `crate::actions`) so the default focus traversal can't grab them.
            "home" => {
                if let Some(f) = self.field_mut(row, is_key) {
                    f.caret = 0;
                }
            }
            "end" => {
                if let Some(f) = self.field_mut(row, is_key) {
                    f.caret = f.text.len();
                }
            }
            "backspace" => {
                if let Some(f) = self.field_mut(row, is_key) {
                    f.backspace();
                }
            }
            "delete" => {
                if let Some(f) = self.field_mut(row, is_key) {
                    f.delete();
                }
            }
            "enter" | "escape" => {
                // Commit: blur the form so the host's focus-out fires.
                self.active = None;
                cx.notify();
                return;
            }
            _ => {
                // Printable input: insert the produced character(s). Skip when a
                // command modifier is held (shortcuts aren't text).
                match &ev.keystroke.key_char {
                    Some(ch) if !m.control && !m.platform && !m.function && !ch.is_empty() => {
                        if let Some(f) = self.field_mut(row, is_key) {
                            f.insert(ch);
                        }
                    }
                    _ => return,
                }
            }
        }
        cx.notify();
        cx.stop_propagation();
    }
}

impl Focusable for PropertyEditor {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for PropertyEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<_> = (0..self.rows.len())
            .map(|i| self.render_row(i, cx))
            .collect();
        div()
            .track_focus(&self.focus)
            .key_context("PropertyEditor")
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                this.key_down(ev, window, cx);
            }))
            .on_action(
                cx.listener(|this, _: &crate::actions::PropNextField, _w, cx| {
                    this.tab(true, cx);
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::actions::PropPrevField, _w, cx| {
                    this.tab(false, cx);
                }),
            )
            .flex()
            .flex_col()
            .gap(px(2.0))
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
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this: &mut PropertyEditor, _: &MouseDownEvent, _w, cx| {
                            this.add_row(cx);
                        }),
                    ),
            )
    }
}

impl PropertyEditor {
    fn render_row(&self, i: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let r = &self.rows[i];
        let key_active = self.active == Some((i, true));
        let value_active = self.active == Some((i, false));
        let icon = theme::property_icon(&r.key.text);
        let dropdown = key_active.then(|| self.key_autocomplete(i, cx)).flatten();

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
                    .child(self.render_field(i, true, key_active, cx))
                    .children(dropdown),
            )
            .child(self.render_field(i, false, value_active, cx))
            .child(
                div()
                    .id(("prop-remove", i))
                    .flex_shrink_0()
                    .px(px(6.0))
                    .text_color(theme::text_tertiary())
                    .cursor_pointer()
                    .hover(|s| s.text_color(theme::text_primary()))
                    .child("✕")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(
                            move |this: &mut PropertyEditor, _: &MouseDownEvent, _w, cx| {
                                this.remove_row(i, cx);
                            },
                        ),
                    ),
            )
            .into_any_element()
    }

    /// A field: the editable text with a caret when active; the panel look
    /// (muted key / value pills) otherwise. Clicking focuses it.
    fn render_field(
        &self,
        i: usize,
        is_key: bool,
        active: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(f) = self.field(i, is_key) else {
            return div().into_any_element();
        };
        let mut cell = div()
            .id(("prop-field", i * 2 + usize::from(is_key)))
            .py(px(4.0))
            .cursor_text()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    this.go(i, is_key, true);
                    this.focus.focus(window, cx);
                    cx.notify();
                }),
            );
        if is_key {
            cell = cell.w_full().text_color(theme::text_tertiary());
        } else {
            cell = cell.flex_1();
        }
        if active {
            // Editable: text split at the caret with a bar between.
            let (before, after) = f.text.split_at(f.caret);
            cell.flex()
                .items_center()
                .child(before.to_string())
                .child(div().w(px(1.5)).h(px(16.0)).bg(theme::accent()))
                .child(after.to_string())
                .into_any_element()
        } else if is_key {
            let label = if f.text.is_empty() {
                "key".to_string()
            } else {
                f.text.clone()
            };
            cell.child(label).into_any_element()
        } else {
            cell.child(value_display(&f.text)).into_any_element()
        }
    }

    /// The autocomplete panel for row `i`'s key field (vault keys, filtered by
    /// what's typed), dropped directly below the field.
    fn key_autocomplete(&self, i: usize, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let typed = self.rows.get(i)?.key.text.to_lowercase();
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
                            if let Some(r) = this.rows.get_mut(i) {
                                r.key.text = key.to_string();
                                r.key.caret = r.key.text.len();
                            }
                            this.go(i, false, false);
                            this.focus.focus(window, cx);
                            cx.notify();
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

fn prev_boundary(s: &str, i: usize) -> usize {
    s[..i].char_indices().next_back().map_or(0, |(idx, _)| idx)
}

fn next_boundary(s: &str, i: usize) -> usize {
    s[i..].chars().next().map_or(i, |c| i + c.len_utf8())
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
    use super::{Field, parse};

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

    #[test]
    fn field_edits_at_the_caret() {
        let mut f = Field::new("ab");
        assert_eq!(f.caret, 2);
        assert!(f.left()); // between a|b
        f.insert("X");
        assert_eq!(f.text, "aXb");
        assert_eq!(f.caret, 2);
        f.backspace(); // a|b (caret at 1)
        assert_eq!(f.text, "ab");
        assert_eq!(f.caret, 1);
        assert!(f.left()); // |ab
        assert!(!f.left()); // at start → caller hops fields
    }
}
