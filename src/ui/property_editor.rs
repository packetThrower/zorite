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

use std::{cell::RefCell, collections::HashMap, ops::Range, rc::Rc};

use gpui::{
    App, Bounds, ClipboardItem, Context, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels,
    Point, Render, SharedString, StatefulInteractiveElement, Styled, Window, anchored, canvas,
    deferred, div, prelude::FluentBuilder, px, svg,
};

use crate::theme;
use rust_i18n::t;

/// Line height of the "Add property" button (see `PropertyEditor::height`).
const ADD_LINE_H: f32 = 18.0;

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
    /// The field a left-button drag started in: moves extend its selection.
    drag: Option<(usize, bool)>,
    /// The right-click menu's window-space position, while it's open. Drawn by
    /// the form itself: a separate popup would take focus, and losing focus
    /// commits the form.
    menu: Option<Point<Pixels>>,
    focus: FocusHandle,
}

impl gpui::EventEmitter<PropExit> for PropertyEditor {}

struct Row {
    /// Everything before the key on the source line (indent + an optional
    /// list marker, e.g. `  ` or `- `) — written back verbatim so an
    /// indented / bulleted property keeps its place in the outline.
    prefix: String,
    key: Field,
    value: Field,
}

/// A single editable text field: its content, the caret's byte offset, and
/// the selection's other end (`anchor`) while text is selected.
#[derive(Default)]
struct Field {
    text: String,
    caret: usize,
    anchor: Option<usize>,
}

impl Field {
    fn new(s: &str) -> Self {
        Self {
            text: s.to_string(),
            caret: s.len(),
            anchor: None,
        }
    }

    /// The selected byte range, if any text is selected.
    fn selection(&self) -> Option<Range<usize>> {
        self.anchor
            .filter(|a| *a != self.caret)
            .map(|a| a.min(self.caret)..a.max(self.caret))
    }

    fn selected_text(&self) -> Option<&str> {
        self.selection().map(|r| &self.text[r])
    }

    /// Remove the selected text; `false` when nothing was selected.
    fn delete_selection(&mut self) -> bool {
        let Some(r) = self.selection() else {
            self.anchor = None;
            return false;
        };
        self.text.replace_range(r.clone(), "");
        self.caret = r.start;
        self.anchor = None;
        true
    }

    /// Type or paste `s`, replacing the selection.
    fn insert(&mut self, s: &str) {
        self.delete_selection();
        self.text.insert_str(self.caret, s);
        self.caret += s.len();
    }

    fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.caret > 0 {
            let prev = prev_boundary(&self.text, self.caret);
            self.text.replace_range(prev..self.caret, "");
            self.caret = prev;
        }
    }

    fn delete(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.caret < self.text.len() {
            let next = next_boundary(&self.text, self.caret);
            self.text.replace_range(self.caret..next, "");
        }
    }

    /// Put the caret at `i`; `extend` (Shift) grows the selection instead of
    /// dropping it.
    fn move_to(&mut self, i: usize, extend: bool) {
        if extend {
            self.anchor.get_or_insert(self.caret);
        } else {
            self.anchor = None;
        }
        self.caret = i;
    }

    /// Move the caret left; returns `false` when already at the start (so the
    /// caller can hop to the previous field). Without `extend`, a selection
    /// collapses to its start first.
    fn left(&mut self, extend: bool) -> bool {
        if !extend && let Some(r) = self.selection() {
            self.move_to(r.start, false);
            return true;
        }
        if self.caret == 0 {
            return false;
        }
        self.move_to(prev_boundary(&self.text, self.caret), extend);
        true
    }

    /// Move the caret right; returns `false` when already at the end.
    fn right(&mut self, extend: bool) -> bool {
        if !extend && let Some(r) = self.selection() {
            self.move_to(r.end, false);
            return true;
        }
        if self.caret >= self.text.len() {
            return false;
        }
        self.move_to(next_boundary(&self.text, self.caret), extend);
        true
    }

    fn select_all(&mut self) {
        self.anchor = Some(0);
        self.caret = self.text.len();
    }

    /// Select the word around the caret (a double-click).
    fn select_word(&mut self) {
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let start = self.text[..self.caret]
            .char_indices()
            .rev()
            .take_while(|(_, c)| is_word(*c))
            .last()
            .map_or(self.caret, |(i, _)| i);
        let end = self.text[self.caret..]
            .char_indices()
            .find(|(_, c)| !is_word(*c))
            .map_or(self.text.len(), |(i, _)| self.caret + i);
        self.anchor = Some(start);
        self.caret = end;
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
            .map(|(p, k, v)| Row {
                prefix: p,
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
            drag: None,
            menu: None,
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
        self.rows[row].value.move_to(caret, false);
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
                r.key.move_to(r.key.text.len(), false);
            }
            self.active = Some((i, true));
            self.dropdown_suppressed = false;
        }
        self.focus.focus(window, cx);
        cx.notify();
    }

    /// The current fields serialized back to `key:: value` lines (empty-key rows
    /// dropped), each behind its original prefix (indent / list marker). This is
    /// what the host writes over the source block on commit.
    pub fn to_source(&self, _cx: &App) -> String {
        self.rows
            .iter()
            .filter_map(|r| {
                let k = r.key.text.trim();
                if k.is_empty() {
                    return None;
                }
                Some(format!("{}{k}:: {}", r.prefix, r.value.text.trim()))
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
            let at = if caret_end { f.text.len() } else { 0 };
            f.move_to(at, false);
            self.active = Some((row, is_key));
            self.dropdown_suppressed = false;
        }
    }

    /// A click on a field: focus it with the caret at the character nearest the
    /// click's x. The field is shaped as its raw text — for pill-y values that's
    /// an approximation (pills render compressed), but the field reflows to
    /// near-raw on activation anyway and arrows refine.
    fn click_field(
        &mut self,
        row: usize,
        is_key: bool,
        x: Pixels,
        extend: bool,
        clicks: usize,
        window: &mut Window,
    ) {
        let caret = self.index_at(row, is_key, x, window);
        // Shift-click extends only within the field already being edited.
        let extend = extend && self.active == Some((row, is_key));
        if let Some(f) = self.field_mut(row, is_key) {
            match clicks {
                2 => {
                    f.move_to(caret, false);
                    f.select_word();
                }
                n if n >= 3 => f.select_all(),
                _ => f.move_to(caret, extend),
            }
            self.active = Some((row, is_key));
            self.dropdown_suppressed = false;
        }
    }

    /// The byte offset in field `(row, is_key)` nearest window-space `x`.
    fn index_at(&self, row: usize, is_key: bool, x: Pixels, window: &mut Window) -> usize {
        let origin = self.field_origins.borrow().get(&(row, is_key)).copied();
        match (origin, self.field(row, is_key)) {
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
        }
    }

    /// Copy the active field's selection to the clipboard.
    fn copy(&self, cx: &mut Context<Self>) {
        if let Some((row, is_key)) = self.active
            && let Some(t) = self.field(row, is_key).and_then(Field::selected_text)
        {
            cx.write_to_clipboard(ClipboardItem::new_string(t.to_string()));
        }
    }

    fn cut(&mut self, cx: &mut Context<Self>) {
        self.copy(cx);
        if let Some((row, is_key)) = self.active
            && let Some(f) = self.field_mut(row, is_key)
            && f.delete_selection()
        {
            self.dropdown_suppressed &= !is_key;
            cx.notify();
        }
    }

    /// Paste clipboard text at the caret. A property is one line, so line
    /// breaks become spaces.
    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else {
            return;
        };
        let text = text.replace("\r\n", " ").replace(['\n', '\r'], " ");
        if let Some((row, is_key)) = self.active
            && let Some(f) = self.field_mut(row, is_key)
        {
            f.insert(&text);
            self.dropdown_suppressed &= !is_key;
            cx.notify();
        }
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        if let Some((row, is_key)) = self.active
            && let Some(f) = self.field_mut(row, is_key)
        {
            f.select_all();
            cx.notify();
        }
    }

    /// The form's own right-click menu: Cut / Copy / Paste / Select all on the
    /// active field, at `pos` (window space).
    fn context_menu(&self, pos: Point<Pixels>, cx: &mut Context<Self>) -> gpui::AnyElement {
        let has_sel = self
            .active
            .and_then(|(r, k)| self.field(r, k))
            .is_some_and(|f| f.selection().is_some());
        let item = |id: &'static str, label: SharedString, enabled: bool| {
            div()
                .id(id)
                .px(px(10.0))
                .py(px(3.0))
                .rounded(px(4.0))
                .when(enabled, |d| {
                    d.cursor_pointer().hover(|s| s.bg(theme::hover()))
                })
                .when(!enabled, |d| d.text_color(theme::text_tertiary()))
                .child(label)
        };
        let act = |f: fn(&mut PropertyEditor, &mut Context<PropertyEditor>)| {
            cx.listener(
                move |this: &mut PropertyEditor, _: &MouseDownEvent, _w, cx| {
                    cx.stop_propagation();
                    this.menu = None;
                    f(this, cx);
                    cx.notify();
                },
            )
        };
        let menu = div()
            .occlude()
            .min_w(px(160.0))
            .p(px(4.0))
            .flex()
            .flex_col()
            .bg(theme::elevated())
            .border_1()
            .border_color(theme::border_subtle())
            .rounded(px(6.0))
            .shadow_md()
            .text_size(px(13.0))
            .text_color(theme::text_primary())
            .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _w, cx| {
                this.menu = None;
                cx.notify();
            }))
            .child(
                item("prop-menu-cut", t!("menu.cut").into(), has_sel).when(has_sel, |d| {
                    d.on_mouse_down(MouseButton::Left, act(Self::cut))
                }),
            )
            .child(
                item("prop-menu-copy", t!("menu.copy").into(), has_sel).when(has_sel, |d| {
                    d.on_mouse_down(MouseButton::Left, act(|this, cx| this.copy(cx)))
                }),
            )
            .child(
                item("prop-menu-paste", t!("menu.paste").into(), true)
                    .on_mouse_down(MouseButton::Left, act(Self::paste)),
            )
            .child(
                item("prop-menu-select-all", t!("menu.select_all").into(), true)
                    .on_mouse_down(MouseButton::Left, act(Self::select_all)),
            );
        deferred(anchored().position(pos).child(menu))
            .with_priority(1)
            .into_any_element()
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

    /// One property row's height (see `render_row`).
    fn row_height(&self) -> Pixels {
        px(self.text_size * 1.45 + 8.0)
    }

    /// The form's laid-out height: the rows plus the "Add property" button.
    /// The host reserves exactly this in the note (and re-reserves when a row
    /// is added or removed), so the text below sits right under the form.
    pub fn height(&self) -> Pixels {
        self.row_height() * self.rows.len().max(1) as f32 + px(2.0 + 3.0 * 2.0 + ADD_LINE_H)
    }

    fn add_row(&mut self, cx: &mut Context<Self>) {
        // A new row sits at the same outline position as the one above it.
        let prefix = self.rows.last().map_or(String::new(), |r| {
            // Under a `- key:: value` first row, siblings indent under the
            // marker rather than repeating the bullet.
            let ws = r.prefix.len() - r.prefix.trim_start().len();
            if r.prefix[ws..].is_empty() {
                r.prefix.clone()
            } else {
                " ".repeat(r.prefix.len())
            }
        });
        self.rows.push(Row {
            prefix,
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
        // Any key closes the right-click menu; Escape does only that.
        if self.menu.take().is_some() && ev.keystroke.key == "escape" {
            cx.notify();
            cx.stop_propagation();
            return;
        }
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
            // Arrows move VISUALLY, as everywhere else: on an RTL field the
            // character to the right is the logically previous one, and the
            // field to the visual left is the one that comes NEXT (the key sits
            // on the right there). Direction is decided once, from the field's
            // own text, so a Latin key beside a Persian value keeps its own.
            key @ ("left" | "right") => {
                let rtl = self
                    .field(row, is_key)
                    .is_some_and(|f| zorite_markdown::syntax::content_direction(&f.text).is_rtl());
                let forward = (key == "right") != rtl;
                let moved = self.field_mut(row, is_key).is_some_and(|f| {
                    if forward {
                        f.right(m.shift)
                    } else {
                        f.left(m.shift)
                    }
                });
                // Shift at a field's edge just stops: a selection never spans fields.
                if !moved && !m.shift {
                    if forward {
                        if is_key {
                            self.go(row, false, false);
                        } else if row + 1 < n {
                            self.go(row + 1, true, false);
                        }
                    } else if !is_key {
                        // Hop to the preceding field, caret at its end.
                        self.go(row, true, true);
                    } else if row > 0 {
                        self.go(row - 1, false, true);
                    }
                }
            }
            "up" if row > 0 => self.go(row - 1, is_key, true),
            "down" if row + 1 < n => self.go(row + 1, is_key, true),
            // Tab / Shift+Tab arrive as the PropNextField / PropPrevField actions
            // (see `crate::actions`) so the default focus traversal can't grab them.
            "home" => {
                if let Some(f) = self.field_mut(row, is_key) {
                    f.move_to(0, m.shift);
                }
            }
            "end" => {
                if let Some(f) = self.field_mut(row, is_key) {
                    let end = f.text.len();
                    f.move_to(end, m.shift);
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
        // Follow the note, like the rendered panel does: keyed off the VALUES,
        // since a Persian note's property keys are usually Latin (`tags`). The
        // panel is what the rendered one turns into on click, so it has to sit
        // on the same side — otherwise it jumps across the note as you edit it.
        let rtl = self
            .rows
            .iter()
            .any(|r| zorite_markdown::syntax::content_direction(&r.value.text).is_rtl());
        let rows: Vec<_> = (0..self.rows.len())
            .map(|i| self.render_row(i, key_col, rtl, cx))
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
            .on_action(cx.listener(|this, _: &crate::actions::PropCopy, _w, cx| {
                this.copy(cx);
            }))
            .on_action(cx.listener(|this, _: &crate::actions::PropCut, _w, cx| {
                this.cut(cx);
            }))
            .on_action(cx.listener(|this, _: &crate::actions::PropPaste, _w, cx| {
                this.paste(cx);
            }))
            .on_action(
                cx.listener(|this, _: &crate::actions::PropSelectAll, _w, cx| {
                    this.select_all(cx);
                }),
            )
            // A drag that began in a field extends that field's selection.
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, window, cx| {
                let Some((row, is_key)) = this.drag else {
                    return;
                };
                if ev.pressed_button != Some(MouseButton::Left) {
                    this.drag = None;
                    return;
                }
                let i = this.index_at(row, is_key, ev.position.x, window);
                if let Some(f) = this.field_mut(row, is_key)
                    && f.caret != i
                {
                    f.move_to(i, true);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _w, _cx| this.drag = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _w, _cx| this.drag = None),
            )
            .flex()
            .flex_col()
            // Match the rendered panel: the note's text size, rows stacked with
            // no gap (the row height carries the spacing).
            .text_size(px(self.text_size))
            // The 480px cap keeps a left-to-right panel from stretching across
            // the note. An RTL panel has to span so its rows can sit against
            // the right edge, where the rendered panel puts them — capped, it
            // right-aligns inside its own 480px and lands mid-note.
            .when(!rtl, |d| d.max_w(px(480.0)))
            .when(rtl, |d| d.w_full().items_end())
            .children(rows)
            .child(
                div()
                    .id("prop-add")
                    .mt(px(2.0))
                    .px(px(6.0))
                    .py(px(3.0))
                    .text_size(px(12.0))
                    // Explicit, so `height()` knows this row's size exactly.
                    .line_height(px(ADD_LINE_H))
                    .text_color(theme::accent())
                    .cursor_pointer()
                    .hover(|s| s.text_color(theme::text_primary()))
                    .child(t!("property_editor.add_property"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this: &mut PropertyEditor, _: &MouseDownEvent, _w, cx| {
                            this.add_row(cx);
                        }),
                    ),
            )
            .children(self.menu.map(|pos| self.context_menu(pos, cx)))
    }
}

impl PropertyEditor {
    fn render_row(
        &self,
        i: usize,
        key_col: Pixels,
        rtl: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let r = &self.rows[i];
        let key_active = self.active == Some((i, true));
        let value_active = self.active == Some((i, false));
        let icon = theme::property_icon(&r.key.text);
        let dropdown = (key_active && !self.dropdown_suppressed)
            .then(|| self.key_autocomplete(i, cx))
            .flatten();
        let icon_sz = px(self.text_size * 0.95);
        let row_h = self.row_height();

        div()
            .flex()
            // Key (and its icon) lead from the right, value beside it, the
            // remove affordance at the row's trailing end either way.
            .when(rtl, |d| d.flex_row_reverse())
            .items_center()
            .h(row_h)
            .gap(px(6.0))
            .child(
                div()
                    .relative()
                    .w(key_col)
                    .flex_shrink_0()
                    .flex()
                    .when(rtl, |d| d.flex_row_reverse())
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
                    this.menu = None;
                    this.click_field(
                        i,
                        is_key,
                        ev.position.x,
                        ev.modifiers.shift,
                        ev.click_count,
                        window,
                    );
                    this.drag = Some((i, is_key));
                    this.focus.focus(window, cx);
                    cx.notify();
                }),
            )
            // Right-click keeps a selection it lands in (so Copy acts on it);
            // anywhere else it moves the caret first, like a text editor.
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    let at = this.index_at(i, is_key, ev.position.x, window);
                    let inside = this.active == Some((i, is_key))
                        && this
                            .field(i, is_key)
                            .and_then(Field::selection)
                            .is_some_and(|r| r.contains(&at));
                    if !inside {
                        this.click_field(i, is_key, ev.position.x, false, 1, window);
                    }
                    this.menu = Some(ev.position);
                    this.focus.focus(window, cx);
                    cx.stop_propagation();
                    cx.notify();
                }),
            );
        if is_key {
            cell = cell.w_full().text_color(theme::text_tertiary());
        } else {
            cell = cell.flex_1();
        }
        // The field's OWN text decides here, not the panel: a Latin key next
        // to a Persian value must not be reversed along with it.
        let f_rtl = zorite_markdown::syntax::content_direction(&f.text).is_rtl();
        if active && (is_key || f.selection().is_some()) {
            // A key (keys aren't pills), or a value with a selection: plain
            // text around the caret, the selection tinted.
            cell.child(raw_field(f, sz, f_rtl)).into_any_element()
        } else if active {
            // Value: pills, revealing the segment under the caret as raw text.
            cell.child(active_value(f, sz, f_rtl)).into_any_element()
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

/// An active field as plain text: the part before the selection, the
/// selection tinted, the part after, and the caret at whichever end of the
/// selection it sits (with no selection, simply at the caret).
fn raw_field(f: &Field, text_size: f32, rtl: bool) -> impl IntoElement {
    let sel = f.selection().unwrap_or(f.caret..f.caret);
    let caret = || caret_bar(text_size).into_any_element();
    let mut kids: Vec<gpui::AnyElement> = Vec::new();
    if sel.start > 0 {
        kids.push(
            div()
                .child(f.text[..sel.start].to_string())
                .into_any_element(),
        );
    }
    if f.caret == sel.start {
        kids.push(caret());
    }
    if !sel.is_empty() {
        kids.push(
            div()
                .bg(theme::accent_tint())
                .child(f.text[sel.clone()].to_string())
                .into_any_element(),
        );
        if f.caret == sel.end {
            kids.push(caret());
        }
    }
    if sel.end < f.text.len() {
        kids.push(
            div()
                .child(f.text[sel.end..].to_string())
                .into_any_element(),
        );
    }
    div()
        .flex()
        .when(rtl, |d| d.flex_row_reverse())
        .items_center()
        .children(kids)
}

/// The focused value, rendered like the panel (tags/wiki-links as pills) except
/// the segment the caret sits in, which shows raw text + the caret so it can be
/// edited — reveal-on-caret, within the field.
fn active_value(f: &Field, text_size: f32, rtl: bool) -> impl IntoElement {
    let value = f.text.as_str();
    let caret = f.caret;
    let mut kids: Vec<gpui::AnyElement> = Vec::new();
    let mut placed = false;
    let mut pos = 0;
    for (range, _hit) in zorite_markdown::syntax::links(value) {
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
    // An active field is a SEQUENCE of children — text before the caret, the
    // caret bar, text after it, pills — so the row's direction is what puts
    // them in reading order. Laid out left-to-right, an RTL value comes apart:
    // `#فارسی #آزمایش` renders as `سی #آزمایش#فار`.
    div()
        .flex()
        .when(rtl, |d| d.flex_row_reverse())
        .items_center()
        .children(kids)
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
        zorite_markdown::syntax::wiki_target_display(inner)
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
    for seg in zorite_markdown::syntax::property_value_segments(value) {
        match seg {
            zorite_markdown::syntax::PropSeg::Text(t) => {
                let t = t.trim();
                if !t.is_empty() {
                    row = row.child(div().text_color(theme::text_primary()).child(t.to_string()));
                }
            }
            zorite_markdown::syntax::PropSeg::Pill { label, is_tag, .. } => {
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

/// Split a property block into `(prefix, key, value)` triples — prefix is the
/// line's indent + optional list marker, kept for writeback — ignoring lines
/// that aren't properties (via the shared grammar).
fn parse(source: &str) -> Vec<(String, String, String)> {
    source
        .lines()
        .filter_map(|l| {
            zorite_markdown::syntax::prefixed_property(l)
                .map(|(p, k, v)| (p.to_string(), k.to_string(), v.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Field, parse};

    #[test]
    fn parse_reads_property_lines_only() {
        let rows = parse("attendees:: Bob, Sue\n  time:: 3:00pm\n- status:: open\njust prose");
        assert_eq!(
            rows,
            vec![
                (String::new(), "attendees".into(), "Bob, Sue".into()),
                ("  ".into(), "time".into(), "3:00pm".into()),
                ("- ".into(), "status".into(), "open".into()),
            ]
        );
    }

    #[test]
    fn field_edits_at_the_caret() {
        let mut f = Field::new("ab");
        assert_eq!(f.caret, 2);
        assert!(f.left(false)); // between a|b
        f.insert("X");
        assert_eq!(f.text, "aXb");
        assert_eq!(f.caret, 2);
        f.backspace(); // a|b (caret at 1)
        assert_eq!(f.text, "ab");
        assert_eq!(f.caret, 1);
        assert!(f.left(false)); // |ab
        assert!(!f.left(false)); // at start → caller hops fields
    }

    #[test]
    fn field_selects_and_replaces() {
        let mut f = Field::new("hello world");
        // Shift+Left ×5 selects "world"; typing replaces it.
        for _ in 0..5 {
            assert!(f.left(true));
        }
        assert_eq!(f.selected_text(), Some("world"));
        f.insert("there");
        assert_eq!(f.text, "hello there");
        assert_eq!(f.selection(), None);
        // Left without Shift collapses a selection to its start first.
        f.select_all();
        assert!(f.left(false));
        assert_eq!((f.caret, f.selection()), (0, None));
        // Double-click selects the word; Backspace deletes just the selection.
        f.move_to(2, false);
        f.select_word();
        assert_eq!(f.selected_text(), Some("hello"));
        f.backspace();
        assert_eq!(f.text, " there");
        // Shift at the edge doesn't report a move (the caller won't hop fields).
        f.move_to(0, false);
        assert!(!f.left(true));
    }
}
