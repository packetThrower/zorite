//! In-place editor for a run of `key:: value` properties. The host seats it in
//! the WYSIWYG editor's reserved gap (the same mechanism the math structural
//! editor uses) when the caret enters a property panel; on blur the host reads
//! [`PropertyEditor::to_source`] and writes the `key:: value` lines back.
//!
//! Custom fields: like the math editor, this owns every keystroke itself (one
//! focus handle for the whole form, per-field text+caret state) rather than
//! delegating to a component that claims the arrow keys. So the caret walks the
//! whole form like a table — Left/Right hop fields at the text edges, Up/Down
//! move rows, Tab steps field-to-field. It's built to mirror the rendered panel:
//! icon + muted key + value pills (the focused value reveals only the caret's
//! segment as raw text), matched to the note's text size and the panel's
//! content-fit column widths, so opening the editor doesn't visibly jump.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use gpui::{
    App, Bounds, Context, FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, ParentElement, Pixels, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, canvas, deferred, div, px, svg,
};

use crate::theme;

/// Emitted when the user exits the form from the keyboard (Enter, or the last
/// Escape) — the host commits and seats the note caret after the block.
pub struct PropExit;

pub struct PropertyEditor {
    rows: Vec<Row>,
    /// Property keys already used across the vault — the autocomplete source.
    keys: Vec<SharedString>,
    /// The field being edited: `(row, is_key)`. `None` = nothing focused yet.
    active: Option<(usize, bool)>,
    /// Escape closed the key autocomplete without leaving the field; typing or
    /// refocusing re-shows it.
    dropdown_suppressed: bool,
    /// Scroll state of the key autocomplete (it caps at ~7 rows and scrolls).
    menu_scroll: gpui::ScrollHandle,
    /// The note's text size — the form matches the rendered panel's sizing.
    text_size: f32,
    /// Each field's painted x origin (captured at paint), keyed by
    /// `(row, is_key)` — lets a click map its x to a caret position.
    field_origins: Rc<RefCell<HashMap<(usize, bool), Pixels>>>,
    focus: FocusHandle,
}

impl gpui::EventEmitter<PropExit> for PropertyEditor {}

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
        text_size: f32,
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
            dropdown_suppressed: false,
            menu_scroll: gpui::ScrollHandle::new(),
            text_size,
            field_origins: Rc::new(RefCell::new(HashMap::new())),
            focus: cx.focus_handle(),
        }
    }

    /// Focus a value field on open. A click passes `row` (the clicked property
    /// line) and lands there, caret at the value's end; arrows pass `None` and
    /// land on the last row when entered by arrowing up from below (`at_end`),
    /// else the first — so the caret lands where it came from.
    pub fn focus_end(
        &mut self,
        at_end: bool,
        row: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.rows.is_empty() {
            self.focus.focus(window, cx);
            return;
        }
        let clicked = row.filter(|r| *r < self.rows.len());
        let row = clicked.unwrap_or(if at_end { self.rows.len() - 1 } else { 0 });
        // Caret at the value's end when clicked or entering from below/right,
        // else start.
        let caret = if at_end || clicked.is_some() {
            self.rows[row].value.text.len()
        } else {
            0
        };
        self.rows[row].value.caret = caret;
        self.active = Some((row, false));
        self.focus.focus(window, cx);
        cx.notify();
    }

    /// Focus row `i`'s key field as a fresh entry — the `/property` flow. The
    /// snippet's untouched `key` placeholder is cleared so the empty field shows
    /// its hint and the autocomplete offers every existing key; anything else
    /// (the user typed before the snippet, or an existing row) is kept.
    pub fn focus_new_key(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(r) = self.rows.get_mut(i) {
            if r.key.text == "key" {
                r.key = Field::default();
            } else {
                r.key.caret = r.key.text.len();
            }
            self.active = Some((i, true));
            self.dropdown_suppressed = false;
        }
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
            self.dropdown_suppressed = false;
        }
    }

    /// A click on a field: focus it with the caret at the character nearest the
    /// click's x. The field is shaped as its raw text — for pill-y values that's
    /// an approximation (pills render compressed), but the field reflows to
    /// near-raw on activation anyway and arrows refine.
    fn click_field(&mut self, row: usize, is_key: bool, x: Pixels, window: &mut Window) {
        let origin = self.field_origins.borrow().get(&(row, is_key)).copied();
        let caret = match (origin, self.field(row, is_key)) {
            (Some(ox), Some(f)) if !f.text.is_empty() => {
                let fs = px(self.text_size);
                let run = gpui::TextRun {
                    len: f.text.len(),
                    font: window.text_style().font(),
                    color: gpui::Hsla::default(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                window
                    .text_system()
                    .shape_line(SharedString::from(f.text.clone()), fs, &[run], None)
                    .closest_index_for_x(x - ox)
            }
            _ => self.field(row, is_key).map_or(0, |f| f.text.len()),
        };
        if let Some(f) = self.field_mut(row, is_key) {
            f.caret = caret;
            self.active = Some((row, is_key));
            self.dropdown_suppressed = false;
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
        // Escape backs out one layer at a time: close the key autocomplete,
        // then leave the field, then exit the form (the host commits + returns
        // the caret to the note). Handled before the active guard so the final
        // escape works with nothing focused.
        if ev.keystroke.key == "escape" {
            match self.active {
                Some((_, true)) if !self.dropdown_suppressed => {
                    self.dropdown_suppressed = true;
                }
                Some(_) => self.active = None,
                None => {
                    cx.emit(PropExit);
                    return;
                }
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
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
                // Editing the key re-shows a dismissed autocomplete.
                self.dropdown_suppressed &= !is_key;
            }
            "delete" => {
                if let Some(f) = self.field_mut(row, is_key) {
                    f.delete();
                }
                self.dropdown_suppressed &= !is_key;
            }
            "enter" => {
                // Done: the host commits and seats the note caret after the block.
                cx.emit(PropExit);
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
                        self.dropdown_suppressed &= !is_key;
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Content-fit the key column to the widest key (+ icon), like the panel,
        // so the value column starts right after the keys instead of at a fixed
        // width. Measured at the note's text size.
        let fs = px(self.text_size);
        let font = window.text_style().font();
        let mut max_key = 0.0f32;
        for r in &self.rows {
            let label = if r.key.text.is_empty() {
                "key"
            } else {
                r.key.text.as_str()
            };
            let run = gpui::TextRun {
                len: label.len(),
                font: font.clone(),
                color: gpui::Hsla::default(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let w = window
                .text_system()
                .shape_line(SharedString::from(label.to_string()), fs, &[run], None)
                .width();
            max_key = max_key.max(f32::from(w));
        }
        // icon + gap(6) + widest key + a small trailing gap before the value.
        let key_col = px(self.text_size * 0.95 + 6.0 + max_key + 14.0);
        let rows: Vec<_> = (0..self.rows.len())
            .map(|i| self.render_row(i, key_col, cx))
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
            // Match the rendered panel: the note's text size, rows stacked with
            // no gap (the row height carries the spacing).
            .text_size(px(self.text_size))
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
    fn render_row(&self, i: usize, key_col: Pixels, cx: &mut Context<Self>) -> gpui::AnyElement {
        let r = &self.rows[i];
        let key_active = self.active == Some((i, true));
        let value_active = self.active == Some((i, false));
        let icon = theme::property_icon(&r.key.text);
        let dropdown = (key_active && !self.dropdown_suppressed)
            .then(|| self.key_autocomplete(i, cx))
            .flatten();
        let icon_sz = px(self.text_size * 0.95);
        let row_h = px(self.text_size * 1.45 + 8.0);

        div()
            .flex()
            .items_center()
            .h(row_h)
            .gap(px(6.0))
            .child(
                div()
                    .relative()
                    .w(key_col)
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .children(icon.map(|p| {
                        svg()
                            .path(p)
                            .w(icon_sz)
                            .h(icon_sz)
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
        let sz = self.text_size;
        // Record where this field paints so a click can map its x to a caret
        // position (out of flow — doesn't affect the flex layout).
        let origins = self.field_origins.clone();
        let grip = canvas(
            move |bounds: Bounds<Pixels>, _window, _cx| {
                origins.borrow_mut().insert((i, is_key), bounds.origin.x);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0();
        // Full row height so an empty field (a template's blank value) is still
        // a click target — its content alone would be zero-height.
        let mut cell = div()
            .id(("prop-field", i * 2 + usize::from(is_key)))
            .relative()
            .h_full()
            .flex()
            .items_center()
            .child(grip)
            .cursor_text()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    this.click_field(i, is_key, ev.position.x, window);
                    this.focus.focus(window, cx);
                    cx.notify();
                }),
            );
        if is_key {
            cell = cell.w_full().text_color(theme::text_tertiary());
        } else {
            cell = cell.flex_1();
        }
        if active && is_key {
            // Key: plain text split at the caret (keys aren't pills).
            let (before, after) = f.text.split_at(f.caret);
            cell.child(before.to_string())
                .child(caret_bar(sz))
                .child(after.to_string())
                .into_any_element()
        } else if active {
            // Value: pills, revealing the segment under the caret as raw text.
            cell.child(active_value(f, sz)).into_any_element()
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
        // Fixed row height + a capped viewport: long key lists scroll (with a
        // thumb) instead of growing unbounded — same recipe as the editor's own
        // suggestion menu.
        const ROW_H: f32 = 26.0;
        const PAD: f32 = 4.0;
        const MAX_H: f32 = 186.0; // ~7 rows
        let count = matches.len();
        let items: Vec<_> = matches
            .into_iter()
            .enumerate()
            .map(|(n, k)| {
                let key = k.clone();
                div()
                    .id(("prop-key-opt", i * 1000 + n))
                    .flex_shrink_0()
                    .h(px(ROW_H))
                    .flex()
                    .items_center()
                    .px(px(10.0))
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
        // Scrollbar thumb, shown when the rows overflow the cap — sized from the
        // content height + positioned from the live scroll offset (a wheel scroll
        // re-renders, so this tracks).
        let rows_h = count as f32 * ROW_H;
        let view_h = MAX_H - 2.0 * PAD;
        let thumb = (rows_h > view_h).then(|| {
            let scrolled = (-f32::from(self.menu_scroll.offset().y)).clamp(0.0, rows_h - view_h);
            let thumb_h = (view_h * view_h / rows_h).max(24.0);
            let thumb_top = PAD + scrolled / (rows_h - view_h) * (view_h - thumb_h);
            let mut c = theme::text_tertiary();
            c.a = 0.5;
            div()
                .absolute()
                .top(px(thumb_top))
                .right(px(2.0))
                .w(px(6.0))
                .h(px(thumb_h))
                .rounded(px(3.0))
                .bg(c)
        });
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
                    .overflow_hidden()
                    .text_color(theme::text_primary())
                    .text_size(px(13.0))
                    .child(
                        div()
                            .id("prop-key-menu")
                            .max_h(px(MAX_H))
                            .overflow_y_scroll()
                            .track_scroll(&self.menu_scroll)
                            .flex()
                            .flex_col()
                            .py(px(PAD))
                            .children(items),
                    )
                    .children(thumb),
            )
            .into_any_element(),
        )
    }
}

/// A blinkless caret bar sized to the text.
fn caret_bar(text_size: f32) -> impl IntoElement {
    div().w(px(1.5)).h(px(text_size * 1.2)).bg(theme::accent())
}

/// The focused value, rendered like the panel (tags/wiki-links as pills) except
/// the segment the caret sits in, which shows raw text + the caret so it can be
/// edited — reveal-on-caret, within the field.
fn active_value(f: &Field, text_size: f32) -> impl IntoElement {
    let value = f.text.as_str();
    let caret = f.caret;
    let mut kids: Vec<gpui::AnyElement> = Vec::new();
    let mut placed = false;
    let mut pos = 0;
    for (range, _hit) in gpui_markdown::syntax::links(value) {
        if range.start > pos {
            push_editable(
                &mut kids,
                &value[pos..range.start],
                pos,
                caret,
                &mut placed,
                text_size,
            );
        }
        let raw = &value[range.clone()];
        // The link the caret touches reveals raw; the rest stay pills.
        if !placed && caret >= range.start && caret <= range.end {
            push_editable(&mut kids, raw, range.start, caret, &mut placed, text_size);
        } else {
            let is_tag = raw.starts_with('#');
            let color = if is_tag {
                theme::tag()
            } else {
                theme::accent()
            };
            let mut bg = color;
            bg.a = 0.16;
            kids.push(
                div()
                    // Margin (not a row gap) so pills stay separated but the
                    // caret sits tight against the text within a word.
                    .mx(px(2.0))
                    .px(px(7.0))
                    .py(px(1.0))
                    .rounded(px(6.0))
                    .bg(bg)
                    .text_color(color)
                    .child(pill_label(raw))
                    .into_any_element(),
            );
        }
        pos = range.end;
    }
    push_editable(&mut kids, &value[pos..], pos, caret, &mut placed, text_size);
    if !placed {
        kids.push(caret_bar(text_size).into_any_element());
    }
    div().flex().items_center().children(kids)
}

/// Push a plain-text run, splitting it at the caret (once) with a caret bar.
fn push_editable(
    kids: &mut Vec<gpui::AnyElement>,
    text: &str,
    base: usize,
    caret: usize,
    placed: &mut bool,
    text_size: f32,
) {
    if !*placed && caret >= base && caret <= base + text.len() {
        let split = caret - base;
        if split > 0 {
            kids.push(div().child(text[..split].to_string()).into_any_element());
        }
        kids.push(caret_bar(text_size).into_any_element());
        if split < text.len() {
            kids.push(div().child(text[split..].to_string()).into_any_element());
        }
        *placed = true;
    } else if !text.is_empty() {
        kids.push(div().child(text.to_string()).into_any_element());
    }
}

/// The display label of a link's raw span: a wiki-link's alias, a tag without
/// `#`, a `[text](url)`'s text, else the raw text.
fn pill_label(raw: &str) -> String {
    if let Some(inner) = raw.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")) {
        gpui_markdown::syntax::wiki_target_display(inner)
            .1
            .to_string()
    } else if let Some(tag) = raw.strip_prefix('#') {
        tag.to_string()
    } else if let Some(rest) = raw.strip_prefix('[') {
        rest.split_once(']').map_or(raw, |(t, _)| t).to_string()
    } else {
        raw.to_string()
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
