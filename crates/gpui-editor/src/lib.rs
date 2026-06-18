//! A from-scratch multi-line text editor for GPUI.
//!
//! Host-agnostic — depends only on `gpui` (+ `unicode-segmentation`); no
//! `gpui-component`. Built directly on gpui's text primitives: an
//! [`EntityInputHandler`] for keyboard + IME input, `shape_line` for per-line
//! text shaping, and a custom [`Element`] that lays out + paints the lines,
//! cursor, and selection. The editor **auto-grows** to its content height (no
//! inner scrollbar), so a host can stack many editors in one scroll view.
//!
//! This is the basis for Zorite's note editor, built so we own the editor and
//! can add things gpui-component gates behind its code-editor mode — first up,
//! spell-check squiggles. **Done:** multi-line text, cursor + selection,
//! type/backspace/delete/enter, arrow + Home/End nav, copy/cut/paste, click +
//! drag selection, IME, and **soft-wrap** with content-driven height. Undo/redo,
//! visual-row up/down, diagnostics/squiggles, and richer styling come next.
//!
//! Usage: create an [`EditorState`] entity and render it; call [`bind_keys`]
//! once at startup so the editing actions resolve while it's focused.

use std::ops::Range;

use gpui::{
    App, AvailableSpace, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, FocusHandle, Focusable, Font, GlobalElementId,
    Hsla, InspectorElementId, InteractiveElement, IntoElement, KeyBinding, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, ParentElement, Pixels, Point, Render,
    SharedString, Style, Styled, TextRun, UTF16Selection, Window, WrappedLine, actions, div, fill,
    hsla, point, px, relative, rgba, size,
};
use unicode_segmentation::UnicodeSegmentation;

/// Key context the editing actions are scoped to (so they only fire while an
/// editor is focused).
const CONTEXT: &str = "Editor";

actions!(
    gpui_editor,
    [
        Backspace,
        Delete,
        Left,
        Right,
        Up,
        Down,
        Home,
        End,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectAll,
        Newline,
        Paste,
        Copy,
        Cut,
        ShowCharacterPalette,
        Undo,
        Redo,
    ]
);

/// Bind the editor's editing keys. Call once at startup. Bindings are scoped to
/// the editor's key context, so they don't shadow the host's shortcuts.
pub fn bind_keys(cx: &mut App) {
    let ctx = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, ctx),
        KeyBinding::new("delete", Delete, ctx),
        KeyBinding::new("left", Left, ctx),
        KeyBinding::new("right", Right, ctx),
        KeyBinding::new("up", Up, ctx),
        KeyBinding::new("down", Down, ctx),
        KeyBinding::new("home", Home, ctx),
        KeyBinding::new("end", End, ctx),
        KeyBinding::new("shift-left", SelectLeft, ctx),
        KeyBinding::new("shift-right", SelectRight, ctx),
        KeyBinding::new("shift-up", SelectUp, ctx),
        KeyBinding::new("shift-down", SelectDown, ctx),
        KeyBinding::new("enter", Newline, ctx),
        KeyBinding::new("cmd-a", SelectAll, ctx),
        KeyBinding::new("ctrl-a", SelectAll, ctx),
        KeyBinding::new("cmd-c", Copy, ctx),
        KeyBinding::new("ctrl-c", Copy, ctx),
        KeyBinding::new("cmd-v", Paste, ctx),
        KeyBinding::new("ctrl-v", Paste, ctx),
        KeyBinding::new("cmd-x", Cut, ctx),
        KeyBinding::new("ctrl-x", Cut, ctx),
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, ctx),
        KeyBinding::new("cmd-z", Undo, ctx),
        KeyBinding::new("ctrl-z", Undo, ctx),
        KeyBinding::new("cmd-shift-z", Redo, ctx),
        KeyBinding::new("ctrl-shift-z", Redo, ctx),
        KeyBinding::new("ctrl-y", Redo, ctx),
    ]);
}

/// Cap on undo history (full snapshots) to bound memory.
const UNDO_LIMIT: usize = 256;

/// A restorable editor state, for undo/redo. Stores the caret offset (not a
/// selection), so undo/redo place the caret rather than re-selecting text.
#[derive(Clone)]
struct Snapshot {
    content: String,
    caret: usize,
}

/// The last edit's kind, for coalescing a run of edits into one undo step.
/// `Insert(end)` is a single-grapheme insert whose caret ends at `end`.
#[derive(Clone, Copy, PartialEq)]
enum EditKind {
    Insert(usize),
    Delete,
    Other,
}

/// The editor: text + cursor/selection state, an undo/redo history, plus a
/// cached layout (the wrapped lines from the last paint) for hit-testing + IME.
pub struct EditorState {
    focus_handle: FocusHandle,
    /// The whole document, newline-separated. Byte offsets index into this.
    content: String,
    placeholder: SharedString,
    /// Selection as a byte range; the caret is one end (see [`Self::cursor_offset`]).
    selected_range: Range<usize>,
    selection_reversed: bool,
    /// IME composition range, if any.
    marked_range: Option<Range<usize>>,
    /// Last paint's wrapped lines (one per logical line) and each line's top
    /// offset relative to the editor's top — both used for hit-testing and
    /// cursor/IME positioning.
    wrapped: Vec<WrappedLine>,
    line_tops: Vec<Pixels>,
    last_bounds: Option<Bounds<Pixels>>,
    line_height: Pixels,
    is_selecting: bool,
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,
    last_edit: EditKind,
}

impl EditorState {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: String::new(),
            placeholder: SharedString::default(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            wrapped: Vec::new(),
            line_tops: Vec::new(),
            last_bounds: None,
            line_height: px(20.),
            is_selecting: false,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            last_edit: EditKind::Other,
        }
    }

    /// Builder: start with the given text (caret at the start).
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.content = text.into();
        self.selected_range = 0..0;
        self
    }

    /// Builder: placeholder shown when empty.
    pub fn with_placeholder(mut self, text: impl Into<SharedString>) -> Self {
        self.placeholder = text.into();
        self
    }

    /// The current document text.
    pub fn text(&self) -> &str {
        &self.content
    }

    /// Replace the whole document; resets the caret to the start.
    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.content = text.into();
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.marked_range = None;
        // A programmatic load isn't undoable to the prior document.
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.last_edit = EditKind::Other;
        cx.notify();
    }

    // --- Cursor movement -----------------------------------------------------

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        let off = self.vertical_offset(-1);
        self.move_to(off, cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        let off = self.vertical_offset(1);
        self.move_to(off, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let off = self.vertical_offset(-1);
        self.select_to(off, cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let off = self.vertical_offset(1);
        self.select_to(off, cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        let (row, _) = self.row_col(self.cursor_offset());
        let starts = self.line_starts();
        self.move_to(starts[row], cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let (row, _) = self.row_col(self.cursor_offset());
        self.move_to(self.line_end(row), cx);
    }

    // --- Editing -------------------------------------------------------------

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let prev = self.previous_boundary(self.cursor_offset());
            if self.cursor_offset() == prev {
                return;
            }
            self.select_to(prev, cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let next = self.next_boundary(self.cursor_offset());
            if self.cursor_offset() == next {
                return;
            }
            self.select_to(next, cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_text_in_range(None, "\n", window, cx);
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    // --- Undo / redo ---------------------------------------------------------

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(prev) = self.undo_stack.pop() {
            self.redo_stack.push(self.snapshot());
            self.restore(prev);
            self.last_edit = EditKind::Other;
            cx.notify();
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(next) = self.redo_stack.pop() {
            self.undo_stack.push(self.snapshot());
            self.restore(next);
            self.last_edit = EditKind::Other;
            cx.notify();
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            content: self.content.clone(),
            // The forward caret (selection end), so undoing a backspace lands the
            // caret after the restored text rather than inside it.
            caret: self.selected_range.end,
        }
    }

    fn restore(&mut self, s: Snapshot) {
        self.content = s.content;
        let caret = s.caret.min(self.content.len());
        self.selected_range = caret..caret;
        self.selection_reversed = false;
        self.marked_range = None;
    }

    /// Snapshot the pre-edit state for undo, coalescing a run of single-grapheme
    /// inserts (or a run of deletes) into one undo step so typing isn't undone
    /// one character at a time.
    fn record_edit(&mut self, range: &Range<usize>, new_text: &str) {
        let kind = if new_text.is_empty() {
            EditKind::Delete
        } else if range.start == range.end
            && new_text != "\n"
            && new_text.graphemes(true).count() == 1
        {
            EditKind::Insert(range.start + new_text.len())
        } else {
            EditKind::Other
        };
        let coalesce = match (self.last_edit, kind) {
            (EditKind::Insert(end), EditKind::Insert(_)) => end == range.start,
            (EditKind::Delete, EditKind::Delete) => true,
            _ => false,
        };
        if !coalesce {
            self.undo_stack.push(self.snapshot());
            if self.undo_stack.len() > UNDO_LIMIT {
                self.undo_stack.remove(0);
            }
            self.redo_stack.clear();
        }
        self.last_edit = kind;
    }

    // --- Mouse ---------------------------------------------------------------

    fn on_mouse_down(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.is_selecting = true;
        let offset = self.index_for_mouse_position(event.position);
        if event.modifiers.shift {
            self.select_to(offset, cx);
        } else {
            self.move_to(offset, cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    // --- Selection helpers ---------------------------------------------------

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        // A deliberate caret move ends the current typing/deleting run.
        self.last_edit = EditKind::Other;
        cx.notify();
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify();
    }

    // --- Line / row-col mapping ---------------------------------------------

    /// Byte offset where each visual line starts (line 0 starts at 0; each line
    /// after a `\n`). Always has at least one entry.
    fn line_starts(&self) -> Vec<usize> {
        let mut starts = vec![0];
        for (i, b) in self.content.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        starts
    }

    /// The `(row, byte-column)` of a byte offset.
    fn row_col(&self, offset: usize) -> (usize, usize) {
        let starts = self.line_starts();
        let row = starts.partition_point(|&s| s <= offset).saturating_sub(1);
        (row, offset - starts[row])
    }

    /// Byte offset of the end of a row's text (before its `\n`, or the document
    /// end for the last row).
    fn line_end(&self, row: usize) -> usize {
        let starts = self.line_starts();
        starts
            .get(row + 1)
            .map(|&s| s - 1)
            .unwrap_or(self.content.len())
    }

    /// Offset one row up/down from the caret, preserving the byte column where
    /// possible. At the top/bottom edge, jumps to the document start/end.
    fn vertical_offset(&self, dir: i32) -> usize {
        let cursor = self.cursor_offset();
        let starts = self.line_starts();
        let (row, col) = self.row_col(cursor);
        let target = row as i32 + dir;
        if target < 0 {
            return 0;
        }
        if target as usize >= starts.len() {
            return self.content.len();
        }
        let target = target as usize;
        let target_start = starts[target];
        let target_len = self.line_end(target) - target_start;
        let mut new_col = col.min(target_len);
        while new_col > 0 && !self.content.is_char_boundary(target_start + new_col) {
            new_col -= 1;
        }
        target_start + new_col
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() || self.wrapped.is_empty() {
            return 0;
        }
        let Some(bounds) = self.last_bounds.as_ref() else {
            return 0;
        };
        let lh = self.line_height;
        let rel = point(position.x - bounds.left(), position.y - bounds.top());
        // Which logical line, by the vertical band each occupies.
        let mut row = self.wrapped.len() - 1;
        for i in 0..self.wrapped.len() {
            let height = lh * (self.wrapped[i].wrap_boundaries().len() + 1) as f32;
            if rel.y < self.line_tops[i] + height {
                row = i;
                break;
            }
        }
        let line_rel = point(rel.x, rel.y - self.line_tops[row]);
        let col = match self.wrapped[row].closest_index_for_position(line_rel, lh) {
            Ok(i) | Err(i) => i,
        };
        self.line_starts()[row] + col
    }

    // --- UTF-16 + grapheme boundaries (IME / cursor movement) ----------------

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8 = 0;
        let mut utf16 = 0;
        for ch in self.content.chars() {
            if utf16 >= offset {
                break;
            }
            utf16 += ch.len_utf16();
            utf8 += ch.len_utf8();
        }
        utf8
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16 = 0;
        let mut utf8 = 0;
        for ch in self.content.chars() {
            if utf8 >= offset {
                break;
            }
            utf8 += ch.len_utf8();
            utf16 += ch.len_utf16();
        }
        utf16
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }
}

impl EntityInputHandler for EditorState {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range.as_ref().map(|r| self.range_to_utf16(r))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        self.record_edit(&range, new_text);
        self.content =
            self.content[0..range.start].to_owned() + new_text + &self.content[range.end..];
        let caret = range.start + new_text.len();
        self.selected_range = caret..caret;
        self.selection_reversed = false;
        self.marked_range = None;
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        self.content =
            self.content[0..range.start].to_owned() + new_text + &self.content[range.end..];
        self.marked_range =
            (!new_text.is_empty()).then(|| range.start..range.start + new_text.len());
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .map(|r| r.start + range.start..r.end + range.start)
            .unwrap_or_else(|| {
                let caret = range.start + new_text.len();
                caret..caret
            });
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);
        let (row, col) = self.row_col(range.start);
        let line = self.wrapped.get(row)?;
        let p = line.position_for_index(col, self.line_height)?;
        let top = bounds.top() + self.line_tops.get(row).copied().unwrap_or(px(0.)) + p.y;
        Some(Bounds::from_corners(
            point(bounds.left() + p.x, top),
            point(bounds.left() + p.x, top + self.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.offset_to_utf16(self.index_for_mouse_position(point)))
    }
}

impl Focusable for EditorState {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for EditorState {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .child(EditorElement {
                editor: cx.entity(),
            })
    }
}

/// Shape `text` into wrapped lines at `wrap_width` (one [`WrappedLine`] per
/// logical line, each carrying its own wrap boundaries). Empty on a shaping
/// error, so the editor degrades to blank rather than panicking.
fn shape_all(
    window: &mut Window,
    text: &SharedString,
    font_size: Pixels,
    font: Font,
    color: Hsla,
    wrap_width: Option<Pixels>,
) -> Vec<WrappedLine> {
    let run = TextRun {
        len: text.len(),
        font,
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let runs: &[TextRun] = if text.is_empty() {
        &[]
    } else {
        std::slice::from_ref(&run)
    };
    window
        .text_system()
        .shape_text(text.clone(), font_size, runs, wrap_width, None)
        .map(|lines| lines.into_vec())
        .unwrap_or_default()
}

/// The custom element that lays out + paints the editor's wrapped lines, cursor,
/// and selection, and wires the input handler. Height is content-driven via a
/// measured layout (it depends on the resolved width once soft-wrap is applied).
struct EditorElement {
    editor: Entity<EditorState>,
}

struct PrepaintState {
    wrapped: Vec<WrappedLine>,
    /// Top offset of each logical line relative to the editor's top.
    line_tops: Vec<Pixels>,
    cursor: Option<PaintQuad>,
    selections: Vec<PaintQuad>,
}

impl IntoElement for EditorElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, ()) {
        // Height depends on the resolved width (soft-wrap), so measure it: shape
        // the content at the available width and count wrapped rows.
        let editor = self.editor.clone();
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        let id = window.request_measured_layout(style, move |known, available, window, cx| {
            let content: SharedString = editor.read(cx).content.clone().into();
            let text_style = window.text_style();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            let lh = window.line_height();
            let wrap_width = match available.width {
                AvailableSpace::Definite(w) => Some(w),
                _ => known.width,
            };
            let rows = shape_all(
                window,
                &content,
                font_size,
                text_style.font(),
                text_style.color,
                wrap_width,
            )
            .iter()
            .map(|line| line.wrap_boundaries().len() + 1)
            .sum::<usize>()
            .max(1);
            let width = wrap_width.or(known.width).unwrap_or(px(0.));
            size(width, lh * rows as f32)
        });
        (id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> PrepaintState {
        let editor = self.editor.read(cx);
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let lh = window.line_height();
        let wrap_width = Some(bounds.size.width);
        let text_color = style.color;

        // Shape the content (or placeholder, when empty) at the resolved width.
        let wrapped = if editor.content.is_empty() {
            shape_all(
                window,
                &editor.placeholder,
                font_size,
                style.font(),
                hsla(0., 0., 0.5, 0.5),
                wrap_width,
            )
        } else {
            let content: SharedString = editor.content.clone().into();
            shape_all(
                window,
                &content,
                font_size,
                style.font(),
                text_color,
                wrap_width,
            )
        };

        // Top offset of each logical line (running sum of wrapped heights).
        let mut line_tops = Vec::with_capacity(wrapped.len());
        let mut y = px(0.);
        for line in &wrapped {
            line_tops.push(y);
            y += lh * (line.wrap_boundaries().len() + 1) as f32;
        }

        // Map a (line-relative) point to a screen point. Captures `bounds` (Copy)
        // only, so `line_tops` stays free to move into the prepaint state.
        let to_screen =
            |top: Pixels, p: Point<Pixels>| point(bounds.left() + p.x, bounds.top() + top + p.y);

        let (cursor, selections) = if editor.content.is_empty() {
            let c = fill(
                Bounds::new(point(bounds.left(), bounds.top()), size(px(2.), lh)),
                text_color,
            );
            (Some(c), Vec::new())
        } else if editor.selected_range.is_empty() {
            let (row, col) = editor.row_col(editor.cursor_offset());
            let p = wrapped
                .get(row)
                .and_then(|l| l.position_for_index(col, lh))
                .unwrap_or_default();
            let top = line_tops.get(row).copied().unwrap_or(px(0.));
            let c = fill(Bounds::new(to_screen(top, p), size(px(2.), lh)), text_color);
            (Some(c), Vec::new())
        } else {
            let (s, e) = (editor.selected_range.start, editor.selected_range.end);
            let starts = editor.line_starts();
            let (s_row, _) = editor.row_col(s);
            let (e_row, _) = editor.row_col(e);
            let right = bounds.size.width;
            let color = rgba(0x3b82f640);
            let mut sels = Vec::new();
            for row in s_row..=e_row {
                let Some(line) = wrapped.get(row) else {
                    continue;
                };
                let top = line_tops[row];
                let line_start = starts[row];
                let a = s.max(line_start) - line_start;
                let b = e.min(editor.line_end(row)) - line_start;
                let pa = line.position_for_index(a, lh).unwrap_or_default();
                let pb = line.position_for_index(b, lh).unwrap_or_default();
                if pa.y == pb.y {
                    sels.push(fill(
                        Bounds::from_corners(
                            to_screen(top, pa),
                            to_screen(top, point(pb.x.max(pa.x + px(2.)), pb.y + lh)),
                        ),
                        color,
                    ));
                } else {
                    // First wrap row: start x → right edge.
                    sels.push(fill(
                        Bounds::from_corners(
                            to_screen(top, pa),
                            to_screen(top, point(right, pa.y + lh)),
                        ),
                        color,
                    ));
                    // Full middle wrap rows.
                    let mut yy = pa.y + lh;
                    while yy < pb.y {
                        sels.push(fill(
                            Bounds::from_corners(
                                to_screen(top, point(px(0.), yy)),
                                to_screen(top, point(right, yy + lh)),
                            ),
                            color,
                        ));
                        yy += lh;
                    }
                    // Last wrap row: left edge → end x.
                    sels.push(fill(
                        Bounds::from_corners(
                            to_screen(top, point(px(0.), pb.y)),
                            to_screen(top, point(pb.x, pb.y + lh)),
                        ),
                        color,
                    ));
                }
            }
            (None, sels)
        };

        PrepaintState {
            wrapped,
            line_tops,
            cursor,
            selections,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.editor.read(cx).focus_handle.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );

        for sel in prepaint.selections.drain(..) {
            window.paint_quad(sel);
        }

        let lh = window.line_height();
        for (line, top) in prepaint.wrapped.iter().zip(prepaint.line_tops.iter()) {
            let origin = point(bounds.origin.x, bounds.origin.y + *top);
            let _ = line.paint(origin, lh, gpui::TextAlign::Left, None, window, cx);
        }

        if focus.is_focused(window)
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }

        let wrapped = std::mem::take(&mut prepaint.wrapped);
        let line_tops = std::mem::take(&mut prepaint.line_tops);
        self.editor.update(cx, |editor, _| {
            editor.wrapped = wrapped;
            editor.line_tops = line_tops;
            editor.last_bounds = Some(bounds);
            editor.line_height = lh;
        });
    }
}
