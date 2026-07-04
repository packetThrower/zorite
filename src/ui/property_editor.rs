//! In-place editor for a run of `key:: value` properties. The host seats it in
//! the WYSIWYG editor's reserved gap (the same mechanism the math structural
//! editor uses) when the caret enters a property panel; on blur the host reads
//! [`PropertyEditor::to_source`] and writes the `key:: value` lines back.
//!
//! v2: a text field per key and value (add/remove rows). Focusing a key field
//! drops an autocomplete of property keys already used across the vault (from
//! `Db::property_index`) below it — free text still allowed, so new keys can be
//! typed. Value pill-chips come next.

use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Window, deferred, div, px,
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

    /// The autocomplete panel for row `i`'s key field — the vault's keys filtered
    /// by what's typed, dropped directly below the field. Shows every key when
    /// the field already holds an exact key (so a pre-filled row can still switch)
    /// or is empty; filters as a partial is typed.
    fn key_autocomplete(&self, i: usize, cx: &mut Context<Self>) -> Option<impl IntoElement> {
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
        // Deferred so it paints above the rows below it; absolute + top_full drops
        // it directly under the (relative) key field.
        Some(deferred(
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
        ))
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
                // The key autocomplete shows while this key field holds focus.
                let key_focused = r.key.read(cx).focus_handle(cx).is_focused(window);
                let dropdown = key_focused.then(|| self.key_autocomplete(i, cx)).flatten();
                let r = &self.rows[i];
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .relative()
                            .w(px(150.0))
                            .flex_shrink_0()
                            .child(Input::new(&r.key))
                            .children(dropdown),
                    )
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
