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
use std::sync::Arc;

use gpui::{
    App, AvailableSpace, BorderStyle, Bounds, ClipboardItem, Context, Corners, CursorStyle, Edges,
    Element, ElementId, ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle,
    Focusable, Font, GlobalElementId, Hsla, InspectorElementId, InteractiveElement, IntoElement,
    KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad,
    ParentElement, PathBuilder, Pixels, Point, Render, RenderImage, ScrollHandle, SharedString,
    StatefulInteractiveElement, Style, Styled, TextRun, UTF16Selection, Window, WrappedLine,
    actions, div, fill, hsla, point, px, relative, rgb, rgba, size,
};
use unicode_segmentation::UnicodeSegmentation;

mod markdown_syntax;
pub use markdown_syntax::SyntaxStyle;

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
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        Dismiss,
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
        KeyBinding::new("alt-left", WordLeft, ctx),
        KeyBinding::new("alt-right", WordRight, ctx),
        KeyBinding::new("alt-shift-left", SelectWordLeft, ctx),
        KeyBinding::new("alt-shift-right", SelectWordRight, ctx),
        KeyBinding::new("escape", Dismiss, ctx),
    ]);
}

/// Cap on undo history (full snapshots) to bound memory.
const UNDO_LIMIT: usize = 256;

/// Line height as a multiple of the font size. Derived from the editor's own
/// font (not the ambient `window.line_height()`, which tracks the host's UI text
/// style and would leave the caret/rows mismatched against differently-sized
/// editor text). 1.25 matches the spacing the editor replaced.
const LINE_HEIGHT_RATIO: f32 = 1.25;

/// Horizontal inset (px) of fenced-code-block text from the box's left edge, so
/// code sits inside the padded box rather than flush against it. Mirrors the old
/// renderer's `px(12)` left padding.
const CODE_INSET: f32 = 12.;

/// Vertical padding (px) above the first / below the last line of a fenced code
/// block. Reserved as layout space (a gap in the line tops + total height) so the
/// box doesn't overlap adjacent lines, with no blank line required.
const CODE_PAD: f32 = 8.;

/// Horizontal inset (px) of blockquote text from the editor's left edge, leaving
/// room for the left border (2px) + a gap, matching the reading view's `pl(12)`.
const QUOTE_INSET: f32 = 14.;

/// Width (px) of a list item's bullet/number column — the body text is inset this
/// far past the item's indent, leaving room for the painted `•` / `N.` + a gap.
const BULLET_COL: f32 = 22.;

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

/// A flagged span (e.g. a misspelling) to underline. The host (e.g. a spell
/// checker) computes these and feeds them in via [`EditorState::set_diagnostics`].
/// Replacement suggestions are fetched lazily when the user right-clicks the
/// span, via the provider set with [`EditorState::on_suggest`] — so detection
/// can stay cheap and run on every edit.
#[derive(Clone)]
pub struct Diagnostic {
    /// Byte range in the document.
    pub range: Range<usize>,
}

/// An open right-click suggestions menu for a diagnostic.
#[derive(Clone)]
struct DiagMenu {
    /// Popup top-left, relative to the editor's top-left.
    anchor: Point<Pixels>,
    /// The diagnostic's byte range, replaced when a suggestion is chosen.
    range: Range<usize>,
    suggestions: Vec<SharedString>,
    /// Scroll state of the (capped-height) list, so a thumb can track it.
    scroll: ScrollHandle,
}

/// Events the editor emits so a host can react. Subscribe with
/// `cx.subscribe(&editor, …)` — e.g. to re-run spell-check after an edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorEvent {
    /// The document text changed via a user edit (typing, delete, paste, IME,
    /// applying a suggestion). Not emitted for programmatic `set_text`.
    Changed,
}

/// Provides replacement suggestions for a flagged word (best first); set by the
/// host via [`EditorState::on_suggest`] and consulted on right-click.
type SuggestFn = Box<dyn Fn(&str) -> Vec<String>>;

/// Resolves a standalone image line's `src` to a decoded image so the editor can
/// render it inline (W4). Set by the host via
/// [`EditorState::set_block_image_provider`]; the host owns loading + caching and
/// returns `None` while still decoding / on failure (the line shows raw source).
type BlockImageFn = Box<dyn Fn(&str) -> Option<Arc<RenderImage>>>;

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
    /// Per-logical-line wrap-row height. Variable so a heading (bigger font) gets
    /// a taller row (W2); `line_height` is the base/fallback for the empty doc
    /// and any row without a recorded height.
    line_heights: Vec<Pixels>,
    /// Per-logical-line flag: this row is painted as an inline image (W4), so a
    /// click on it places the caret at the line start instead of hit-testing
    /// source text. From the last paint.
    widget_rows: Vec<bool>,
    /// Per-logical-line display→source byte map for rows with hidden markers
    /// (W6); `None` when the painted text equals the source. From the last paint.
    offset_maps: Vec<Option<Vec<usize>>>,
    /// Per-logical-line horizontal text inset (and so the caret/selection/hit-test
    /// inset): non-zero for fenced code blocks and gutter marks (blockquotes,
    /// lists). From the last paint.
    line_insets: Vec<Pixels>,
    last_bounds: Option<Bounds<Pixels>>,
    line_height: Pixels,
    is_selecting: bool,
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,
    last_edit: EditKind,
    /// The target x for vertical (Up/Down) movement, so the caret keeps its
    /// column across short lines. `Some` only during a run of Up/Down.
    goal_x: Option<Pixels>,
    /// Spans to underline (misspellings, etc.), set by the host via
    /// [`Self::set_diagnostics`].
    diagnostics: Vec<Diagnostic>,
    /// Inline-markdown styling palette; `Some` turns on live-preview rendering
    /// (W1), `None` is plain text. Set by the host via [`Self::set_markdown_style`].
    markdown_style: Option<SyntaxStyle>,
    /// The open right-click suggestions menu, if any.
    menu: Option<DiagMenu>,
    /// Supplies replacement suggestions for a flagged word, fetched lazily when
    /// the user right-clicks it. Set by the host via [`Self::on_suggest`];
    /// without it, the right-click menu has nothing to offer.
    suggest: Option<SuggestFn>,
    /// Resolves a standalone image line's `src` to a decoded image for inline
    /// rendering (W4); set by the host via [`Self::set_block_image_provider`].
    block_image: Option<BlockImageFn>,
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
            line_heights: Vec::new(),
            widget_rows: Vec::new(),
            offset_maps: Vec::new(),
            line_insets: Vec::new(),
            last_bounds: None,
            line_height: px(20.),
            is_selecting: false,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            last_edit: EditKind::Other,
            goal_x: None,
            diagnostics: Vec::new(),
            markdown_style: None,
            menu: None,
            suggest: None,
            block_image: None,
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

    /// Replace the set of diagnostics (underlined spans). The host computes these
    /// (e.g. spell-check) and refreshes them as the text changes.
    pub fn set_diagnostics(&mut self, diagnostics: Vec<Diagnostic>, cx: &mut Context<Self>) {
        self.diagnostics = diagnostics;
        cx.notify();
    }

    /// Turn on live-preview markdown styling with the given color/font palette
    /// (call once at setup). Inline bold/italic/code/link/tag formatting then
    /// renders as you type — markers stay in the text, dimmed. Without it the
    /// editor renders plain text (spell-check underlines only).
    pub fn set_markdown_style(&mut self, style: SyntaxStyle, cx: &mut Context<Self>) {
        self.markdown_style = Some(style);
        cx.notify();
    }

    /// Turn off live-preview styling — the editor falls back to plain text
    /// (spell-check underlines only). Used when the host's WYSIWYG setting is
    /// switched off; a no-op if styling was already off.
    pub fn clear_markdown_style(&mut self, cx: &mut Context<Self>) {
        if self.markdown_style.take().is_some() {
            cx.notify();
        }
    }

    /// Install the provider consulted when the user right-clicks a flagged word.
    /// It's handed the offending word and returns replacements (best first).
    /// Kept lazy by design — the OS suggestion call can be slow, so it runs only
    /// on right-click, never in the per-edit detection pass.
    pub fn on_suggest(&mut self, provider: impl Fn(&str) -> Vec<String> + 'static) {
        self.suggest = Some(Box::new(provider));
    }

    /// Install the provider that resolves a standalone image line's `src` to a
    /// decoded image; with it, such lines render inline (W4) when the caret is
    /// elsewhere. Without it (or while an image is still loading), the line shows
    /// its raw `![](src)` source.
    pub fn set_block_image_provider(
        &mut self,
        provider: impl Fn(&str) -> Option<Arc<RenderImage>> + 'static,
    ) {
        self.block_image = Some(Box::new(provider));
    }

    /// The caret's byte offset into [`Self::text`] (the moving end of any
    /// selection). For hosts that drive a menu/completion off the caret position.
    pub fn cursor(&self) -> usize {
        self.cursor_offset()
    }

    /// Place the caret at `offset` (a byte offset into the document), collapsing
    /// any selection. Clamped to the document and snapped down to a char
    /// boundary, so a host can pass a raw click offset safely — e.g. to enter
    /// edit mode where rendered text was clicked.
    pub fn set_cursor(&mut self, offset: usize, cx: &mut Context<Self>) {
        let mut offset = offset.min(self.content.len());
        while !self.content.is_char_boundary(offset) {
            offset -= 1;
        }
        self.move_to(offset, cx);
    }

    /// The wrap-row height of logical line `row` (a heading is taller). Falls
    /// back to the base `line_height` for unrecorded rows / the empty document.
    fn line_h(&self, row: usize) -> Pixels {
        self.line_heights
            .get(row)
            .copied()
            .unwrap_or(self.line_height)
    }

    /// Horizontal text inset for logical line `row` (from the last paint): non-zero
    /// for fenced code blocks + gutter marks. Applied to the caret, selection,
    /// hit-test, and text paint so they all stay aligned.
    fn line_inset(&self, row: usize) -> Pixels {
        self.line_insets.get(row).copied().unwrap_or(px(0.))
    }

    /// Window-space bounds of the caret at `offset`, from the last paint's
    /// layout — for anchoring a popup (e.g. a slash menu) at a document offset.
    /// `None` before the first paint or if `offset`'s row isn't laid out.
    pub fn bounds_for_offset(&self, offset: usize) -> Option<Bounds<Pixels>> {
        let bounds = self.last_bounds?;
        let (row, col) = self.row_col(offset);
        let lh = self.line_h(row);
        let line = self.wrapped.get(row)?;
        let p = line.position_for_index(self.display_col(row, col), lh)?;
        let top = bounds.top() + self.line_tops.get(row).copied().unwrap_or(px(0.)) + p.y;
        let x = bounds.left() + p.x + self.line_inset(row);
        Some(Bounds::from_corners(point(x, top), point(x, top + lh)))
    }

    /// The document text as an owned [`SharedString`]; use [`Self::text`] for a
    /// borrowed `&str`.
    pub fn value(&self) -> SharedString {
        self.content.clone().into()
    }

    /// Focus the editor so it receives keyboard input. (`set_cursor` only moves
    /// the caret; call this to enter edit mode, e.g. on a click into rendered text.)
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
    }

    /// Keep diagnostics valid across an edit at `edited` (the replaced byte
    /// range) that inserted `new_len` bytes: spans before the edit are left
    /// alone, spans after it are shifted by the size delta, and spans that
    /// overlap the edited text are dropped (that text changed, so they're
    /// stale). The host still recomputes the edited region on its own schedule —
    /// this just keeps the *other* spans correct so they don't all flicker off
    /// on every keystroke.
    fn remap_diagnostics(&mut self, edited: &Range<usize>, new_len: usize) {
        let delta = new_len as isize - (edited.end - edited.start) as isize;
        self.diagnostics.retain_mut(|d| {
            if d.range.end <= edited.start {
                true
            } else if d.range.start >= edited.end {
                d.range.start = (d.range.start as isize + delta) as usize;
                d.range.end = (d.range.end as isize + delta) as usize;
                true
            } else {
                false
            }
        });
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
        let off = self.move_vertical(-1);
        // Set the caret directly (not via `move_to`) to keep the goal column.
        self.selected_range = off..off;
        self.last_edit = EditKind::Other;
        cx.notify();
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        let off = self.move_vertical(1);
        self.selected_range = off..off;
        self.last_edit = EditKind::Other;
        cx.notify();
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let off = self.move_vertical(-1);
        self.select_to(off, cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let off = self.move_vertical(1);
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
        let offset = self.index_for_mouse_position(event.position);
        self.menu = None;
        self.goal_x = None;
        self.last_edit = EditKind::Other;
        match event.click_count {
            // Double-click: select the word under the cursor.
            2 => {
                self.is_selecting = false;
                self.selected_range = self.word_range_at(offset).unwrap_or(offset..offset);
                self.selection_reversed = false;
                cx.notify();
            }
            // Triple-click (or more): select the whole logical line.
            n if n >= 3 => {
                self.is_selecting = false;
                let (row, _) = self.row_col(offset);
                let start = self.line_starts()[row];
                self.selected_range = start..self.line_end(row);
                self.selection_reversed = false;
                cx.notify();
            }
            // Single click: place the caret, or extend the selection with Shift.
            _ => {
                self.is_selecting = true;
                if event.modifiers.shift {
                    self.select_to(offset, cx);
                } else {
                    self.move_to(offset, cx);
                }
            }
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

    /// Right-click: if the click lands on a flagged word, fetch its suggestions
    /// (lazily, via the provider) and open a menu anchored there; otherwise close
    /// any open menu.
    fn on_right_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let offset = self.index_for_mouse_position(event.position);
        let anchor = self
            .last_bounds
            .as_ref()
            .map(|b| point(event.position.x - b.left(), event.position.y - b.top()))
            .unwrap_or_default();
        let hit = self.diagnostic_at(offset).map(|d| d.range.clone());
        self.menu = hit.and_then(|range| {
            let word = self.content[range.clone()].to_string();
            let suggestions = self.suggest.as_ref().map(|f| f(&word)).unwrap_or_default();
            (!suggestions.is_empty()).then(|| DiagMenu {
                anchor,
                range,
                suggestions: suggestions.into_iter().map(SharedString::from).collect(),
                scroll: ScrollHandle::new(),
            })
        });
        cx.notify();
    }

    /// The diagnostic whose range contains `offset`, if any.
    fn diagnostic_at(&self, offset: usize) -> Option<&Diagnostic> {
        self.diagnostics
            .iter()
            .find(|d| d.range.start <= offset && offset < d.range.end)
    }

    /// Close the suggestions menu (Escape, or a click elsewhere).
    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            cx.notify();
        }
    }

    /// Replace `range` with a chosen suggestion and close the menu.
    fn apply_suggestion(
        &mut self,
        range: Range<usize>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        self.selected_range = range;
        self.selection_reversed = false;
        self.replace_text_in_range(None, text, window, cx);
    }

    // --- Selection helpers ---------------------------------------------------

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        // A deliberate caret move ends the current typing/deleting run and the
        // vertical-movement goal column.
        self.last_edit = EditKind::Other;
        self.goal_x = None;
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

    /// Offset one *visual* row up/down from the caret, preserving the goal column
    /// (x) across the run. Falls back to logical-line movement before the first
    /// paint (when no wrapped layout is cached yet).
    fn move_vertical(&mut self, dir: i32) -> usize {
        if self.wrapped.is_empty() {
            return self.vertical_offset(dir);
        }
        let (row, col) = self.row_col(self.cursor_offset());
        let cur_lh = self.line_h(row);
        if cur_lh <= px(0.) {
            return self.vertical_offset(dir);
        }
        let Some(cur) = self
            .wrapped
            .get(row)
            .and_then(|l| l.position_for_index(self.display_col(row, col), cur_lh))
        else {
            return self.vertical_offset(dir);
        };
        let global_y = self.line_tops[row] + cur.y;
        let goal = self.goal_x.unwrap_or(cur.x);
        self.goal_x = Some(goal);
        // Step to the adjacent visual row. Down: to the bottom of the current
        // row (= the top of the next one). Up: just above the current row's top
        // — robust to the row above having a different height (e.g. a heading),
        // since it doesn't depend on the current row's height.
        let target_y = if dir >= 0 {
            global_y + cur_lh
        } else {
            global_y - px(1.)
        };
        if target_y < px(0.) {
            return 0;
        }
        let last = self.wrapped.len() - 1;
        let total = self.line_tops[last]
            + self.line_h(last) * (self.wrapped[last].wrap_boundaries().len() + 1) as f32;
        if target_y >= total {
            return self.content.len();
        }
        let mut trow = last;
        for i in 0..self.wrapped.len() {
            let h = self.line_h(i) * (self.wrapped[i].wrap_boundaries().len() + 1) as f32;
            if target_y < self.line_tops[i] + h {
                trow = i;
                break;
            }
        }
        let rel = point(goal, target_y - self.line_tops[trow]);
        let col = match self.wrapped[trow].closest_index_for_position(rel, self.line_h(trow)) {
            Ok(i) | Err(i) => i,
        };
        self.line_starts()[trow] + self.source_col(trow, col)
    }

    /// The end of the next word at/after `offset` (⌥→ on macOS).
    fn next_word(&self, offset: usize) -> usize {
        self.content
            .unicode_word_indices()
            .map(|(i, w)| i + w.len())
            .find(|&end| end > offset)
            .unwrap_or(self.content.len())
    }

    /// The start of the previous word before `offset` (⌥← on macOS).
    fn prev_word(&self, offset: usize) -> usize {
        self.content
            .unicode_word_indices()
            .map(|(i, _)| i)
            .rfind(|&start| start < offset)
            .unwrap_or(0)
    }

    /// The byte range of the word at `offset` (double-click); `None` in whitespace.
    fn word_range_at(&self, offset: usize) -> Option<Range<usize>> {
        let mut ends_at = None;
        for (i, w) in self.content.unicode_word_indices() {
            let range = i..i + w.len();
            if range.start <= offset && offset < range.end {
                return Some(range);
            }
            if range.end == offset {
                ends_at = Some(range);
            }
        }
        ends_at
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.prev_word(self.cursor_offset()), cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.next_word(self.cursor_offset()), cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.select_to(self.prev_word(self.cursor_offset()), cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.goal_x = None;
        self.select_to(self.next_word(self.cursor_offset()), cx);
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() || self.wrapped.is_empty() {
            return 0;
        }
        let Some(bounds) = self.last_bounds.as_ref() else {
            return 0;
        };
        let rel = point(position.x - bounds.left(), position.y - bounds.top());
        // Which logical line, by the vertical band each occupies (variable height).
        let mut row = self.wrapped.len() - 1;
        for i in 0..self.wrapped.len() {
            let height = self.line_h(i) * (self.wrapped[i].wrap_boundaries().len() + 1) as f32;
            if rel.y < self.line_tops[i] + height {
                row = i;
                break;
            }
        }
        // An inline-image row: clicking it puts the caret at the line start (the
        // line then shows its source — "raw on caret"), not a text column.
        if self.widget_rows.get(row).copied().unwrap_or(false) {
            return self.line_starts()[row];
        }
        let x = (rel.x - self.line_inset(row)).max(px(0.));
        let line_rel = point(x, rel.y - self.line_tops[row]);
        let col = match self.wrapped[row].closest_index_for_position(line_rel, self.line_h(row)) {
            Ok(i) | Err(i) => i,
        };
        self.line_starts()[row] + self.source_col(row, col)
    }

    /// Map a display byte column on `row` back to its source column. Identity
    /// unless the row's markers are hidden (W6), where the painted text is
    /// shorter than the source.
    fn source_col(&self, row: usize, display_col: usize) -> usize {
        match self.offset_maps.get(row).and_then(Option::as_ref) {
            Some(map) => map.get(display_col).copied().unwrap_or(display_col),
            None => display_col,
        }
    }

    /// Map a source byte column on `row` to its display column — the inverse of
    /// [`Self::source_col`], for positioning the caret/selection on a row whose
    /// markers are hidden (W6/#5). Uses the last painted map; in-paint code that
    /// has this frame's fresh map should call [`display_col_in`] directly.
    fn display_col(&self, row: usize, source_col: usize) -> usize {
        display_col_in(
            self.offset_maps.get(row).and_then(Option::as_ref),
            source_col,
        )
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
        self.goal_x = None;
        // Keep unaffected diagnostics valid across the edit (shift those after
        // it, drop those it overlapped); the host recomputes the edited region.
        self.remap_diagnostics(&range, new_text.len());
        cx.emit(EditorEvent::Changed);
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
        self.remap_diagnostics(&range, new_text.len());
        cx.emit(EditorEvent::Changed);
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
        let lh = self.line_h(row);
        let line = self.wrapped.get(row)?;
        let p = line.position_for_index(self.display_col(row, col), lh)?;
        let top = bounds.top() + self.line_tops.get(row).copied().unwrap_or(px(0.)) + p.y;
        let x = bounds.left() + p.x + self.line_inset(row);
        Some(Bounds::from_corners(point(x, top), point(x, top + lh)))
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

impl EventEmitter<EditorEvent> for EditorState {}

impl Render for EditorState {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
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
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::dismiss))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_right_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .child(EditorElement {
                editor: cx.entity(),
            })
            // Right-click suggestions menu, absolutely positioned over the
            // editor (anchored at the click). `Option`'s `IntoIterator` renders
            // zero or one popup; clicking a row replaces the misspelled span.
            .children(self.menu.clone().map(|menu| {
                let DiagMenu {
                    anchor,
                    range,
                    suggestions,
                    scroll,
                } = menu;
                let count = suggestions.len();
                // Collected eagerly (not a lazy iterator) so `cx` is only
                // borrowed here and stays free for the menu's own listeners below.
                let rows: Vec<_> = suggestions
                    .into_iter()
                    .enumerate()
                    .map(|(i, sugg)| {
                        let range = range.clone();
                        let replacement = sugg.to_string();
                        div()
                            // A stable per-row id so gpui tracks hover state and
                            // repaints as the pointer moves between rows. Without
                            // an id, the hover style only shows on a forced
                            // repaint (e.g. while scrolling).
                            .id(("suggestion-row", i))
                            // Don't let the scroll container's max-height squeeze
                            // the rows; they keep their height and overflow.
                            .flex_shrink_0()
                            .px(px(12.))
                            .py(px(5.))
                            // Highlight the row under the pointer.
                            .hover(|s| s.bg(rgb(0x2f6fd6)))
                            .child(sugg)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |editor, _: &MouseDownEvent, window, cx| {
                                    // Keep the editor's own mouse-down from clearing
                                    // the menu / moving the caret out from under us.
                                    cx.stop_propagation();
                                    editor.apply_suggestion(
                                        range.clone(),
                                        &replacement,
                                        window,
                                        cx,
                                    );
                                }),
                            )
                    })
                    .collect();
                // A thin scrollbar thumb, shown when the list overflows ~6 rows
                // so the scroll affordance is visible. Sized from the row count
                // (known now) and positioned from the live scroll offset — a
                // wheel scroll calls window.refresh(), which re-renders this.
                const ROW_H: f32 = 28.0;
                const PAD: f32 = 4.0;
                const MAX_H: f32 = 180.0;
                let rows_h = count as f32 * ROW_H;
                let view_h = MAX_H - 2.0 * PAD;
                let thumb = (rows_h > view_h).then(|| {
                    let scrolled = (-f32::from(scroll.offset().y)).clamp(0.0, rows_h - view_h);
                    let thumb_h = (view_h * view_h / rows_h).max(24.0);
                    let thumb_top = PAD + scrolled / (rows_h - view_h) * (view_h - thumb_h);
                    div()
                        .absolute()
                        .top(px(thumb_top))
                        .right(px(2.))
                        .w(px(6.))
                        .h(px(thumb_h))
                        .rounded(px(3.))
                        .bg(rgba(0xffffff66))
                });

                div()
                    .absolute()
                    .left(anchor.x)
                    .top(anchor.y)
                    .min_w(px(150.))
                    // Override the editor's I-beam — the menu is a normal pointer
                    // surface, not text (children inherit this hitbox's cursor).
                    .cursor(CursorStyle::Arrow)
                    .bg(rgb(0x26262b))
                    .border_1()
                    .border_color(rgb(0x45454c))
                    .rounded(px(6.))
                    // Clip rows + thumb to the rounded box.
                    .overflow_hidden()
                    .text_color(rgb(0xe6e6e6))
                    .text_size(px(14.))
                    // A click anywhere outside the menu (elsewhere in the
                    // window) dismisses it.
                    .on_mouse_down_out(cx.listener(|editor, _: &MouseDownEvent, _, cx| {
                        editor.menu = None;
                        cx.notify();
                    }))
                    .child(
                        // The scroll viewport: shows ~6 rows, the rest scroll.
                        div()
                            // Stable id so the scroll offset persists across frames.
                            .id("suggestion-menu")
                            .max_h(px(MAX_H))
                            .overflow_y_scroll()
                            .track_scroll(&scroll)
                            .flex()
                            .flex_col()
                            .py(px(PAD))
                            .children(rows),
                    )
                    .children(thumb)
            }))
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

/// Shape `text` with pre-built `runs`, so diagnostics can underline specific
/// spans. The plain-run [`shape_all`] is used for the placeholder + measurement.
fn shape_runs(
    window: &mut Window,
    text: &SharedString,
    font_size: Pixels,
    runs: &[TextRun],
    wrap_width: Option<Pixels>,
) -> Vec<WrappedLine> {
    window
        .text_system()
        .shape_text(text.clone(), font_size, runs, wrap_width, None)
        .map(|lines| lines.into_vec())
        .unwrap_or_default()
}

/// A line currently rendered as an inline image (W4) instead of its source text:
/// the decoded image plus its fit-to-width display size (logical px).
#[derive(Clone)]
struct BlockImg {
    img: Arc<RenderImage>,
    width: Pixels,
    height: Pixels,
}

/// A fenced-code-block line's background (W4b/refinement): the block reads as one
/// rounded, content-fit box (sized to its widest line, like a table — not the
/// full editor width). Each line carries the block color, the shared box width
/// (back-patched once the block's extent is known), and whether it's the
/// first/last visible line (to round the box's top/bottom corners).
#[derive(Clone, Copy)]
struct CodeBg {
    color: Hsla,
    width: Pixels,
    top: bool,
    bottom: bool,
}

/// A table row rendered as a grid (W4c): its cells, per-column alignment, the
/// content-fit per-column widths (shared across the table), header/separator/
/// last-row flags, and the border color. Built only when the caret is outside
/// the table — the caret's table shows source instead ("raw on caret").
#[derive(Clone)]
struct TableRow {
    cells: Vec<SharedString>,
    aligns: Vec<markdown_syntax::Align>,
    col_widths: Vec<Pixels>,
    is_header: bool,
    is_separator: bool,
    is_last: bool,
    border: Hsla,
}

/// A per-line "gutter" decoration: a left-margin treatment that hides its source
/// marker and renders something in its place, with the body text inset to make
/// room. Covers blockquotes now; list bullets + task checkboxes reuse it.
#[derive(Clone, Copy)]
enum LineMark {
    /// Blockquote: a muted left border; the `>` markers are hidden and the body
    /// text is muted (`SyntaxStyle::quote`).
    Quote(Hsla),
    /// List item: a painted bullet (`•`) or number (`N.`) at `indent`, muted; the
    /// source marker is hidden and the body inset past the bullet column.
    List {
        indent: Pixels,
        ordered: bool,
        num: u32,
        color: Hsla,
    },
    /// GFM task item: a painted ☐/☑ box at `indent`, muted; the source marker +
    /// checkbox are hidden and the body inset past the bullet column.
    Check {
        indent: Pixels,
        checked: bool,
        color: Hsla,
    },
}

impl LineMark {
    /// Horizontal inset (px) applied to the body text + caret for this mark.
    fn inset(self) -> Pixels {
        match self {
            LineMark::Quote(_) => px(QUOTE_INSET),
            LineMark::List { indent, .. } | LineMark::Check { indent, .. } => {
                indent + px(BULLET_COL)
            }
        }
    }
}

/// Per-logical-line shaping output — parallel vecs of equal length: the shaped
/// source line, its row height, an optional inline-image widget, an optional
/// fenced-code-block background, an optional table-row grid, the display→source
/// map, and an optional gutter decoration (blockquote / list / checkbox).
type ShapedLines = (
    Vec<WrappedLine>,
    Vec<Pixels>,
    Vec<Option<BlockImg>>,
    Vec<Option<CodeBg>>,
    Vec<Option<TableRow>>,
    // Per-line display→source byte map for lines with markers hidden (W6); `None`
    // when the displayed text equals the source (revealed / code / widget lines).
    Vec<Option<Vec<usize>>>,
    Vec<Option<LineMark>>,
);

/// Fit-to-width display size for an inline image from its natural (device) size:
/// cap to the content width (or an explicit `{width=N}`), preserving aspect.
fn block_img(
    img: Arc<RenderImage>,
    width_attr: Option<f32>,
    wrap_width: Option<Pixels>,
    scale_factor: f32,
) -> Option<BlockImg> {
    let dev = img.size(0);
    let (dw, dh) = (dev.width.0 as f32, dev.height.0 as f32);
    if dw <= 0. || dh <= 0. || scale_factor <= 0. {
        return None;
    }
    let natural_w = dw / scale_factor;
    let avail = wrap_width.map_or(natural_w, f32::from);
    let target_w = width_attr.unwrap_or(natural_w).min(avail).max(1.);
    Some(BlockImg {
        img,
        width: px(target_w),
        height: px(target_w * dh / dw),
    })
}

/// Invert a display→source offset map: the display column for `source_col`. The
/// map is ascending, so a source column that is hidden (a collapsed marker)
/// snaps to the next visible display column. `None` map → identity (a row shown
/// as full source). The prepaint cursor/selection pass this frame's fresh map
/// (the committed `EditorState::offset_maps` lags a frame); event handlers go
/// through [`EditorState::display_col`], which uses the committed map.
fn display_col_in(map: Option<&Vec<usize>>, source_col: usize) -> usize {
    match map {
        Some(m) => match m.binary_search(&source_col) {
            Ok(d) | Err(d) => d,
        },
        None => source_col,
    }
}

/// Horizontal text inset for a row from its decorations: [`CODE_INSET`] inside a
/// fenced code block, else the gutter mark's inset (blockquote/list), else zero.
/// At most one applies per line.
fn row_inset(bg: Option<CodeBg>, mark: Option<LineMark>) -> Pixels {
    if bg.is_some() {
        px(CODE_INSET)
    } else {
        mark.map_or(px(0.), LineMark::inset)
    }
}

/// The reserved vertical gap above (`.0`) and below (`.1`) a row, from its
/// code-block background: [`CODE_PAD`] above the block's first line and below its
/// last. Added to the line tops + total height so the padded box has real layout
/// space and never overlaps adjacent lines.
fn code_pads(bg: Option<CodeBg>) -> (Pixels, Pixels) {
    match bg {
        Some(cb) => (
            if cb.top { px(CODE_PAD) } else { px(0.) },
            if cb.bottom { px(CODE_PAD) } else { px(0.) },
        ),
        None => (px(0.), px(0.)),
    }
}

/// Shape `content` line-by-line so each logical line can use its own font size
/// (headings are larger — W2) and a standalone image line can render as the image
/// (W4). Returns, per logical line: the shaped source [`WrappedLine`], its row
/// height, and `Some(BlockImg)` when it paints as an image. `md` drives the
/// per-line size + inline styling (`None` keeps the base size, no images);
/// `diagnostics` are clipped + shifted to each line. The caret's line
/// (`caret_row`) always shows source, so an image stays editable ("raw on
/// caret"). A single line always shapes to one wrapped line (incl. empty), so the
/// counts match the logical lines and blank rows stay positionable.
#[allow(clippy::too_many_arguments)]
fn shape_document(
    window: &mut Window,
    content: &str,
    base_font: &Font,
    base_color: Hsla,
    base_font_size: Pixels,
    diagnostics: &[Diagnostic],
    md: Option<&SyntaxStyle>,
    wrap_width: Option<Pixels>,
    caret_row: Option<usize>,
    block_image: Option<&BlockImageFn>,
    scale_factor: f32,
    // The selected byte range; a line it touches keeps full source (markers
    // shown), the rest hide their markers (W6, reveal-on-caret).
    selection: (usize, usize),
) -> ShapedLines {
    let mut wrapped = Vec::new();
    let mut heights = Vec::new();
    let mut widgets = Vec::new();
    let mut backgrounds: Vec<Option<CodeBg>> = Vec::new();
    let mut tables = Vec::new();
    let mut maps = Vec::new();
    let mut marks: Vec<Option<LineMark>> = Vec::new();
    let lines: Vec<&str> = content.split('\n').collect();
    // Fenced-code-block regions; a block's ``` fence lines collapse (W6) unless
    // the caret is inside that block (then they show, so they stay editable).
    let code_regions = md
        .map(|_| markdown_syntax::code_regions(content))
        .unwrap_or_default();
    // Table regions (W4c); content-fit column widths shared by each region's rows.
    let regions = md
        .map(|_| markdown_syntax::table_regions(content))
        .unwrap_or_default();
    let mut region_cols: Vec<Vec<Pixels>> = Vec::with_capacity(regions.len());
    for r in &regions {
        region_cols.push(table_column_widths(
            &lines,
            r,
            window,
            base_font,
            base_font_size,
            base_color,
            wrap_width,
        ));
    }
    let table_row_h = base_font_size * LINE_HEIGHT_RATIO + px(12.);
    let table_sep_h = px(8.);
    // Fenced-code-block tracking: collect a block's line indices (so its box can
    // be sized to its widest line + the first/last line marked for rounding) and
    // the running max line width.
    let mut code_block: Vec<usize> = Vec::new();
    let mut code_w = px(0.);
    let mut line_start = 0;
    let mut in_fence = false;
    for (idx, &line) in lines.iter().enumerate() {
        let line_end = line_start + line.len();

        // Fenced code block (W4b): a ``` line toggles the fence; the delimiter
        // lines + the lines between render as monospace code over a content-fit
        // background (delimiters dimmed). Code is literal — no inline scanning,
        // no heading size, no squiggles. Styling-mode only.
        let is_fence = md.is_some() && line.trim_start().starts_with("```");
        let is_code = md.is_some() && (in_fence || is_fence);
        if is_fence {
            in_fence = !in_fence;
        }
        // A ``` fence line collapses (height 0, no text) unless the caret is in
        // its block — so a code block reads as just its boxed body (W6), with the
        // fences re-appearing while you edit inside it.
        let collapse_fence = is_fence
            && !code_regions
                .iter()
                .any(|r| r.contains(&idx) && caret_row.is_some_and(|cr| r.contains(&cr)));

        // Leaving a code block: size the box to its widest line (+ the inset on
        // each side, like a table) and mark its last line so the box rounds + pads
        // its bottom edge. The vertical padding is grown into the painted quad, so
        // line geometry — and the caret — stay untouched.
        if !is_code && !code_block.is_empty() {
            let bw = code_w + px(2. * CODE_INSET);
            let last = *code_block.last().unwrap();
            for &bi in &code_block {
                if let Some(cb) = &mut backgrounds[bi] {
                    cb.width = bw;
                    cb.bottom = bi == last;
                }
            }
            code_block.clear();
            code_w = px(0.);
        }

        let fs = if is_code {
            base_font_size
        } else {
            base_font_size * md.map_or(1.0, |_| markdown_syntax::line_scale(line))
        };

        // Inline image (non-code): a standalone image line that isn't the caret's
        // line and has a decoded image renders as that image, fit to width.
        let widget = if !is_code
            && md.is_some()
            && Some(idx) != caret_row
            && let Some((src, w_attr)) = markdown_syntax::image_line(line)
            && let Some(img) = block_image.and_then(|f| f(src))
        {
            block_img(img, w_attr, wrap_width, scale_factor)
        } else {
            None
        };

        // Table row (W4c): a line inside a detected table region renders as a
        // grid row — unless the caret is in that table (then it shows source).
        let table = regions
            .iter()
            .position(|r| r.lines.contains(&idx))
            .filter(|&ri| !is_code && !caret_row.is_some_and(|cr| regions[ri].lines.contains(&cr)))
            .map(|ri| {
                let r = &regions[ri];
                TableRow {
                    cells: markdown_syntax::table_cells(line)
                        .into_iter()
                        .map(|c| SharedString::from(c.to_string()))
                        .collect(),
                    aligns: r.aligns.clone(),
                    col_widths: region_cols[ri].clone(),
                    is_header: idx == r.lines.start,
                    is_separator: idx == r.lines.start + 1,
                    is_last: idx + 1 == r.lines.end,
                    border: md.map_or(base_color, |m| m.marker),
                }
            });

        // A line shows full source while a non-empty selection touches it (so the
        // markers are visible to select) or styling is off. Otherwise its markers
        // are hidden — except, on the caret's own line, the single construct the
        // caret sits in is revealed (per-construct reveal, #5: finer than the old
        // whole-line reveal, so the rest of the line stays rendered).
        let sel_empty = selection.0 == selection.1;
        let full_source = !sel_empty && selection.0 <= line_end && selection.1 >= line_start;
        let caret_col = (sel_empty && selection.0 >= line_start && selection.0 <= line_end)
            .then(|| selection.0 - line_start);
        // This line's diagnostics, clipped + shifted to line-local byte offsets —
        // used as spell-check squiggles whether the line shows source or hides its
        // markers.
        let line_diags: Vec<Diagnostic> = diagnostics
            .iter()
            .filter_map(|d| {
                let s = d.range.start.max(line_start);
                let e = d.range.end.min(line_end);
                (s < e).then(|| Diagnostic {
                    range: (s - line_start)..(e - line_start),
                })
            })
            .collect();
        // Gutter decoration (blockquote / list): a non-code/widget/table line with
        // a `>` or list marker. The decoration (border / bullet, marker hidden,
        // body inset) shows only while the caret is OFF the line; on the line it
        // reads as plain source with the prefix revealed (a line-level reveal — the
        // whole prefix shows wherever the caret sits, unlike inline #5).
        let gutter: Option<(usize, LineMark)> = md
            .filter(|_| !is_code && widget.is_none() && table.is_none())
            .and_then(|st| {
                let indent_px = |indent: usize| px(indent as f32 * f32::from(fs) * 0.5);
                if let Some(plen) = markdown_syntax::blockquote_prefix(line) {
                    Some((plen, LineMark::Quote(st.quote)))
                } else if let Some((plen, indent, checked)) = markdown_syntax::task_prefix(line) {
                    Some((
                        plen,
                        LineMark::Check {
                            indent: indent_px(indent),
                            checked,
                            color: st.quote,
                        },
                    ))
                } else {
                    markdown_syntax::list_prefix(line).map(|(plen, indent, ordered, num)| {
                        (
                            plen,
                            LineMark::List {
                                indent: indent_px(indent),
                                ordered,
                                num,
                                color: st.quote,
                            },
                        )
                    })
                }
            });
        let caret_here = caret_col.is_some();
        let mark = gutter
            .filter(|_| !caret_here && !full_source)
            .map(|(_, m)| m);
        let reveal_prefix = gutter.filter(|_| caret_here).map_or(0, |(plen, _)| plen);
        // A blockquote's body is muted; a list keeps the normal body color (only
        // its bullet is muted).
        let line_base = match mark {
            Some(LineMark::Quote(c)) => c,
            _ => base_color,
        };
        let (shaped_text, runs, bg, map) = if collapse_fence {
            // Hidden ``` fence line: nothing painted, zero height.
            (String::new(), Vec::new(), None, None)
        } else if let Some(st) = md.filter(|_| is_code) {
            // One monospace run for the whole line; ``` delimiters dimmed.
            let run = TextRun {
                len: line.len(),
                font: st.mono.clone(),
                color: if is_fence { st.marker } else { st.code },
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let runs = if line.is_empty() {
                Vec::new()
            } else {
                vec![run]
            };
            // First visible code line of the block rounds the box's top corners.
            let top = code_block.is_empty();
            (
                line.to_string(),
                runs,
                Some(CodeBg {
                    color: st.code_bg,
                    width: px(0.), // back-patched to the block's widest line
                    top,
                    bottom: false,
                }),
                None,
            )
        } else if let Some(st) = md.filter(|_| widget.is_none() && table.is_none() && !full_source)
        {
            // Markers hidden (except the caret's construct): shape the display
            // string + keep a map back to source.
            let (disp, runs, m) = markdown_syntax::hidden_runs(
                line,
                base_font,
                line_base,
                &line_diags,
                caret_col,
                reveal_prefix,
                st,
            );
            (disp, runs, None, Some(m))
        } else {
            // Full source with diagnostics (the caret/selected line, or md off).
            (
                line.to_string(),
                markdown_syntax::styled_runs(line, base_font, line_base, &line_diags, md),
                None,
                None,
            )
        };

        // Code lines are inset by CODE_INSET on each side; a gutter mark insets the
        // left only. Either wraps at a correspondingly narrower width.
        let line_wrap = if is_code {
            wrap_width.map(|w| (w - px(2. * CODE_INSET)).max(px(0.)))
        } else if let Some(m) = mark {
            wrap_width.map(|w| (w - m.inset()).max(px(0.)))
        } else {
            wrap_width
        };
        let shaped = shape_runs(
            window,
            &SharedString::from(shaped_text),
            fs,
            &runs,
            line_wrap,
        );
        if let Some(wl) = shaped.into_iter().next() {
            let h = if collapse_fence {
                px(0.)
            } else {
                match &table {
                    Some(t) if t.is_separator => table_sep_h,
                    Some(_) => table_row_h,
                    None => widget.as_ref().map_or(fs * LINE_HEIGHT_RATIO, |w| w.height),
                }
            };
            let line_w = wl.width();
            wrapped.push(wl);
            heights.push(h);
            widgets.push(widget);
            backgrounds.push(bg);
            tables.push(table);
            maps.push(map);
            marks.push(mark);
            // Track a (visible) code line + its width so the block's box can be
            // sized to its widest line and its last line marked.
            if is_code && !collapse_fence {
                code_block.push(backgrounds.len() - 1);
                code_w = code_w.max(line_w);
            }
        }
        line_start = line_end + 1; // skip the '\n'
    }
    // A code block running to the end of the document: size its box + mark its
    // last line (round the box bottom + pad).
    if !code_block.is_empty() {
        let bw = code_w + px(2. * CODE_INSET);
        let last = *code_block.last().unwrap();
        for &bi in &code_block {
            if let Some(cb) = &mut backgrounds[bi] {
                cb.width = bw;
                cb.bottom = bi == last;
            }
        }
    }
    (wrapped, heights, widgets, backgrounds, tables, maps, marks)
}

/// Content-fit column widths for a table region (W4c): each column sized to its
/// widest cell (header measured bold) + padding, with a minimum, and the whole
/// table scaled down proportionally to fit `wrap_width` if it would overflow.
#[allow(clippy::too_many_arguments)]
fn table_column_widths(
    lines: &[&str],
    region: &markdown_syntax::TableRegion,
    window: &mut Window,
    base_font: &Font,
    font_size: Pixels,
    color: Hsla,
    wrap_width: Option<Pixels>,
) -> Vec<Pixels> {
    let cols = region.aligns.len().max(1);
    let pad = px(8.);
    let mut widths = vec![px(0.); cols];
    for li in region.lines.clone() {
        if li == region.lines.start + 1 {
            continue; // skip the |---| separator
        }
        let header = li == region.lines.start;
        for (c, cell) in markdown_syntax::table_cells(lines[li])
            .iter()
            .enumerate()
            .take(cols)
        {
            if cell.is_empty() {
                continue;
            }
            let mut font = base_font.clone();
            if header {
                font.weight = gpui::FontWeight::BOLD;
            }
            let run = TextRun {
                len: cell.len(),
                font,
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let w = window
                .text_system()
                .shape_line(
                    SharedString::from(cell.to_string()),
                    font_size,
                    &[run],
                    None,
                )
                .width();
            widths[c] = widths[c].max(w + pad * 2.);
        }
    }
    for w in &mut widths {
        *w = (*w).max(px(48.));
    }
    if let Some(avail) = wrap_width {
        let total = widths.iter().fold(px(0.), |a, &w| a + w);
        if total > avail && total > px(0.) {
            let scale = f32::from(avail) / f32::from(total);
            for w in &mut widths {
                *w *= scale;
            }
        }
    }
    widths
}

/// Paint a table row as a grid (W4c): a top border (+ bottom on the last row),
/// a left border per column + a right outer border, and each cell's text aligned
/// within its (content-fit) column. A separator row is a single horizontal rule.
#[allow(clippy::too_many_arguments)]
fn paint_table_row(
    t: &TableRow,
    origin: Point<Pixels>,
    row_h: Pixels,
    font: &Font,
    font_size: Pixels,
    line_h: Pixels,
    color: Hsla,
    window: &mut Window,
    cx: &mut App,
) {
    let thick = px(1.);
    let table_w = t.col_widths.iter().fold(px(0.), |a, &w| a + w);
    if t.is_separator {
        let y = origin.y + (row_h - thick) / 2.;
        window.paint_quad(fill(
            Bounds::new(point(origin.x, y), size(table_w, thick)),
            t.border,
        ));
        return;
    }
    // Horizontal borders: top on every row; bottom only on the last.
    window.paint_quad(fill(Bounds::new(origin, size(table_w, thick)), t.border));
    if t.is_last {
        window.paint_quad(fill(
            Bounds::new(
                point(origin.x, origin.y + row_h - thick),
                size(table_w, thick),
            ),
            t.border,
        ));
    }
    let pad = px(8.);
    let mut cell_font = font.clone();
    if t.is_header {
        cell_font.weight = gpui::FontWeight::BOLD;
    }
    let mut x = origin.x;
    for (c, &cw) in t.col_widths.iter().enumerate() {
        window.paint_quad(fill(
            Bounds::new(point(x, origin.y), size(thick, row_h)),
            t.border,
        ));
        if let Some(cell) = t.cells.get(c).filter(|s| !s.is_empty()) {
            let run = TextRun {
                len: cell.len(),
                font: cell_font.clone(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let shaped = window
                .text_system()
                .shape_line(cell.clone(), font_size, &[run], None);
            let align = match t.aligns.get(c) {
                Some(markdown_syntax::Align::Center) => gpui::TextAlign::Center,
                Some(markdown_syntax::Align::Right) => gpui::TextAlign::Right,
                _ => gpui::TextAlign::Left,
            };
            let _ = shaped.paint(
                point(x + pad, origin.y + (row_h - line_h) / 2.),
                line_h,
                align,
                Some(cw - pad * 2.),
                window,
                cx,
            );
        }
        x += cw;
    }
    window.paint_quad(fill(
        Bounds::new(point(x, origin.y), size(thick, row_h)),
        t.border,
    ));
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
    /// Per-logical-line wrap-row height (variable for headings + images).
    line_heights: Vec<Pixels>,
    /// `Some` for a line painted as an inline image instead of its source text.
    widgets: Vec<Option<BlockImg>>,
    /// Per-line fenced-code-block background (rounded full-width box).
    backgrounds: Vec<Option<CodeBg>>,
    /// `Some` for a line painted as a table-grid row instead of source.
    tables: Vec<Option<TableRow>>,
    /// Per-line display→source byte map for marker-hidden rows (W6).
    maps: Vec<Option<Vec<usize>>>,
    /// Per-line gutter decoration (blockquote / list / checkbox).
    marks: Vec<Option<LineMark>>,
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
            let editor = editor.read(cx);
            let text_style = window.text_style();
            let font_size = text_style.font_size.to_pixels(window.rem_size());
            let base_lh = font_size * LINE_HEIGHT_RATIO;
            let wrap_width = match available.width {
                AvailableSpace::Definite(w) => Some(w),
                _ => known.width,
            };
            let height = if editor.content.is_empty() {
                // Placeholder rows at the base size.
                let rows = shape_all(
                    window,
                    &editor.placeholder,
                    font_size,
                    text_style.font(),
                    text_style.color,
                    wrap_width,
                )
                .iter()
                .map(|line| line.wrap_boundaries().len() + 1)
                .sum::<usize>()
                .max(1);
                base_lh * rows as f32
            } else {
                // Sum of per-line (variable) heights × each line's wrap rows.
                let caret_row = editor.row_col(editor.cursor_offset()).0;
                let sf = window.scale_factor();
                let (wrapped, heights, _, backgrounds, _, _, _) = shape_document(
                    window,
                    &editor.content,
                    &text_style.font(),
                    text_style.color,
                    font_size,
                    &editor.diagnostics,
                    editor.markdown_style.as_ref(),
                    wrap_width,
                    Some(caret_row),
                    editor.block_image.as_ref(),
                    sf,
                    (editor.selected_range.start, editor.selected_range.end),
                );
                wrapped
                    .iter()
                    .zip(&heights)
                    .zip(&backgrounds)
                    .map(|((line, h), bg)| {
                        let (top, bot) = code_pads(*bg);
                        *h * (line.wrap_boundaries().len() + 1) as f32 + top + bot
                    })
                    .fold(px(0.), |a, b| a + b)
                    .max(base_lh)
            };
            let width = wrap_width.or(known.width).unwrap_or(px(0.));
            size(width, height)
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
        let font = style.font();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let base_lh = font_size * LINE_HEIGHT_RATIO;
        let wrap_width = Some(bounds.size.width);
        let text_color = style.color;

        // Placeholder (uniform) when empty; else shape per line so headings get
        // their own taller rows (W2) and image lines render inline (W4).
        let caret_row = editor.row_col(editor.cursor_offset()).0;
        let sf = window.scale_factor();
        let (wrapped, line_heights, widgets, backgrounds, tables, maps, marks) =
            if editor.content.is_empty() {
                let w = shape_all(
                    window,
                    &editor.placeholder,
                    font_size,
                    font.clone(),
                    hsla(0., 0., 0.5, 0.5),
                    wrap_width,
                );
                let n = w.len();
                (
                    w,
                    vec![base_lh; n],
                    vec![None; n],
                    vec![None; n],
                    vec![None; n],
                    vec![None; n],
                    vec![None; n],
                )
            } else {
                shape_document(
                    window,
                    &editor.content,
                    &font,
                    text_color,
                    font_size,
                    &editor.diagnostics,
                    editor.markdown_style.as_ref(),
                    wrap_width,
                    Some(caret_row),
                    editor.block_image.as_ref(),
                    sf,
                    (editor.selected_range.start, editor.selected_range.end),
                )
            };

        // Top offset of each logical line (running sum of variable wrap heights),
        // reserving a gap above/below each code block so its padded box has its
        // own space (no overlap with the adjacent line, no blank line required).
        let mut line_tops = Vec::with_capacity(wrapped.len());
        let mut y = px(0.);
        for ((line, lh), bg) in wrapped
            .iter()
            .zip(line_heights.iter())
            .zip(backgrounds.iter())
        {
            let (top_pad, bot_pad) = code_pads(*bg);
            y += top_pad;
            line_tops.push(y);
            y += *lh * (line.wrap_boundaries().len() + 1) as f32 + bot_pad;
        }

        // Map a (line-relative) point to a screen point. Captures `bounds` (Copy)
        // only, so `line_tops` stays free to move into the prepaint state.
        let to_screen =
            |top: Pixels, p: Point<Pixels>| point(bounds.left() + p.x, bounds.top() + top + p.y);

        // Caret/selection positioning must use THIS frame's fresh per-row data —
        // `editor.offset_maps`/`line_insets` aren't committed until paint, so the
        // method forms would lag a frame (a one-frame caret jump after an edit
        // that hides/reveals markers).
        let disp_col =
            |row: usize, sc: usize| display_col_in(maps.get(row).and_then(Option::as_ref), sc);
        let code_inset = |row: usize| {
            row_inset(
                backgrounds.get(row).copied().flatten(),
                marks.get(row).copied().flatten(),
            )
        };

        let (cursor, selections) = if editor.content.is_empty() {
            let c = fill(
                Bounds::new(point(bounds.left(), bounds.top()), size(px(2.), base_lh)),
                text_color,
            );
            (Some(c), Vec::new())
        } else if editor.selected_range.is_empty() {
            let (row, col) = editor.row_col(editor.cursor_offset());
            let lh = line_heights.get(row).copied().unwrap_or(base_lh);
            let p = wrapped
                .get(row)
                .and_then(|l| l.position_for_index(disp_col(row, col), lh))
                .unwrap_or_default();
            let top = line_tops.get(row).copied().unwrap_or(px(0.));
            let inset = code_inset(row);
            let c = fill(
                Bounds::new(to_screen(top, point(p.x + inset, p.y)), size(px(2.), lh)),
                text_color,
            );
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
                let lh = line_heights.get(row).copied().unwrap_or(base_lh);
                let top = line_tops[row];
                let inset = code_inset(row);
                let line_start = starts[row];
                let a = s.max(line_start) - line_start;
                let b = e.min(editor.line_end(row)) - line_start;
                let pa = line
                    .position_for_index(disp_col(row, a), lh)
                    .unwrap_or_default();
                let pb = line
                    .position_for_index(disp_col(row, b), lh)
                    .unwrap_or_default();
                let pa = point(pa.x + inset, pa.y);
                let pb = point(pb.x + inset, pb.y);
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
            line_heights,
            widgets,
            backgrounds,
            tables,
            maps,
            marks,
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

        let style = window.text_style();
        let font = style.font();
        let text_color = style.color;
        let font_size = style.font_size.to_pixels(window.rem_size());
        let base_lh = font_size * LINE_HEIGHT_RATIO;
        for (i, ((line, top), lh)) in prepaint
            .wrapped
            .iter()
            .zip(prepaint.line_tops.iter())
            .zip(prepaint.line_heights.iter())
            .enumerate()
        {
            let origin = point(bounds.origin.x, bounds.origin.y + *top);
            // Fenced code block: one rounded, content-fit box (sized to the
            // widest line, like a table). The first line rounds + pads the top, the
            // last rounds + pads the bottom; the pad fills the layout gap reserved
            // for it (see `code_pads`), so the caret geometry stays text-height and
            // the box never overlaps an adjacent line.
            if let Some(cb) = prepaint.backgrounds.get(i).copied().flatten() {
                let r = px(6.);
                let z = px(0.);
                let (top_pad, bot_pad) = code_pads(Some(cb));
                let corners = Corners {
                    top_left: if cb.top { r } else { z },
                    top_right: if cb.top { r } else { z },
                    bottom_left: if cb.bottom { r } else { z },
                    bottom_right: if cb.bottom { r } else { z },
                };
                let box_origin = point(origin.x, origin.y - top_pad);
                let box_size = size(cb.width, *lh + top_pad + bot_pad);
                window.paint_quad(
                    fill(Bounds::new(box_origin, box_size), cb.color).corner_radii(corners),
                );
            }
            // Blockquote: a muted 2px left border down the line (the body is inset
            // past it by QUOTE_INSET).
            if let Some(LineMark::Quote(c)) = prepaint.marks.get(i).copied().flatten() {
                window.paint_quad(fill(Bounds::new(origin, size(px(2.), *lh)), c));
            }
            // List item: a muted bullet (`•`) or number (`N.`) glyph at the item's
            // indent; the body is inset past it (the source marker is hidden).
            if let Some(LineMark::List {
                indent,
                ordered,
                num,
                color,
            }) = prepaint.marks.get(i).copied().flatten()
            {
                let glyph: SharedString = if ordered {
                    format!("{num}.").into()
                } else {
                    "•".into()
                };
                let run = TextRun {
                    len: glyph.len(),
                    font: font.clone(),
                    color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let shaped = window
                    .text_system()
                    .shape_line(glyph, font_size, &[run], None);
                let _ = shaped.paint(
                    point(origin.x + indent, origin.y),
                    *lh,
                    gpui::TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
            // Task item: a crisp cap-height box (custom-drawn, not a font glyph so
            // it reads at the text's size) with a checkmark when done.
            if let Some(LineMark::Check {
                indent,
                checked,
                color,
            }) = prepaint.marks.get(i).copied().flatten()
            {
                let sz = font_size * 0.78; // ~cap height
                let bx = origin.x + indent;
                let by = origin.y + (*lh - sz) / 2.; // vertically centered on the line
                window.paint_quad(PaintQuad {
                    bounds: Bounds::new(point(bx, by), size(sz, sz)),
                    corner_radii: Corners::all(px(3.)),
                    background: hsla(0., 0., 0., 0.).into(),
                    border_widths: Edges::all(px(1.5)),
                    border_color: color,
                    border_style: BorderStyle::Solid,
                });
                if checked {
                    let s = f32::from(sz);
                    let mut pb = PathBuilder::stroke(px(1.6));
                    pb.move_to(point(bx + px(s * 0.24), by + px(s * 0.52)));
                    pb.line_to(point(bx + px(s * 0.42), by + px(s * 0.70)));
                    pb.line_to(point(bx + px(s * 0.76), by + px(s * 0.28)));
                    if let Ok(path) = pb.build() {
                        window.paint_path(path, color);
                    }
                }
            }
            if let Some(t) = prepaint.tables.get(i).and_then(Option::as_ref) {
                // Table grid row (W4c): cells + borders instead of source.
                paint_table_row(
                    t, origin, *lh, &font, font_size, base_lh, text_color, window, cx,
                );
            } else if let Some(w) = prepaint.widgets.get(i).and_then(Option::as_ref) {
                // Inline image (W4a): paint the decoded image instead of source.
                let img_bounds = Bounds::new(origin, size(w.width, w.height));
                let _ = window.paint_image(img_bounds, Corners::default(), w.img.clone(), 0, false);
            } else {
                // Code blocks + gutter marks inset their text (kept in sync with
                // `EditorState::line_inset` / the fresh prepaint inset).
                let inset = row_inset(
                    prepaint.backgrounds.get(i).copied().flatten(),
                    prepaint.marks.get(i).copied().flatten(),
                );
                let text_origin = point(origin.x + inset, origin.y);
                // Run backgrounds (the inline-code highlight) paint separately from
                // the glyphs — `paint` alone wouldn't show them.
                let _ = line.paint_background(
                    text_origin,
                    *lh,
                    gpui::TextAlign::Left,
                    None,
                    window,
                    cx,
                );
                let _ = line.paint(text_origin, *lh, gpui::TextAlign::Left, None, window, cx);
            }
        }

        if focus.is_focused(window)
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }

        let wrapped = std::mem::take(&mut prepaint.wrapped);
        let line_tops = std::mem::take(&mut prepaint.line_tops);
        let line_heights = std::mem::take(&mut prepaint.line_heights);
        let offset_maps = std::mem::take(&mut prepaint.maps);
        let widget_rows: Vec<bool> = prepaint
            .widgets
            .iter()
            .enumerate()
            .map(|(i, w)| w.is_some() || prepaint.tables.get(i).is_some_and(Option::is_some))
            .collect();
        let line_insets: Vec<Pixels> = prepaint
            .backgrounds
            .iter()
            .zip(prepaint.marks.iter())
            .map(|(bg, mark)| row_inset(*bg, *mark))
            .collect();
        self.editor.update(cx, |editor, _| {
            editor.wrapped = wrapped;
            editor.line_tops = line_tops;
            editor.line_heights = line_heights;
            editor.widget_rows = widget_rows;
            editor.offset_maps = offset_maps;
            editor.line_insets = line_insets;
            editor.last_bounds = Some(bounds);
            editor.line_height = base_lh;
        });
    }
}
