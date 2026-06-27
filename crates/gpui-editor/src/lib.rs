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
    Focusable, Font, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId,
    InteractiveElement, IntoElement, KeyBinding, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PaintQuad, ParentElement, PathBuilder, Pixels, Point, Render,
    RenderImage, ScrollHandle, SharedString, StatefulInteractiveElement, Style, Styled, TextRun,
    UTF16Selection, Window, WrappedLine, actions, div, fill, hsla, point, px, relative, rgb, rgba,
    size,
};
use unicode_segmentation::UnicodeSegmentation;

mod markdown_syntax;
pub use markdown_syntax::{MathAlign, SyntaxStyle};

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
        Indent,
        Outdent,
        Bold,
        Italic,
        Code,
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
        KeyBinding::new("tab", Indent, ctx),
        KeyBinding::new("shift-tab", Outdent, ctx),
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
        KeyBinding::new("cmd-b", Bold, ctx),
        KeyBinding::new("ctrl-b", Bold, ctx),
        KeyBinding::new("cmd-i", Italic, ctx),
        KeyBinding::new("ctrl-i", Italic, ctx),
        KeyBinding::new("cmd-e", Code, ctx),
        KeyBinding::new("ctrl-e", Code, ctx),
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

/// Caret thickness (px) — thin like a native text caret, so it doesn't blend into
/// the first glyph at the start of a line/cell.
const CARET_WIDTH: f32 = 1.0;

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

/// Vertical padding (px) inside a file chip (e.g. a PDF embed), above + below its
/// label, so the chip box reads as a button rather than a bare line of text.
const CHIP_PAD: f32 = 5.;

/// Total vertical breathing room (px) reserved around an inline image — split
/// above + below — so consecutive images (a bulleted photo list) don't touch.
const IMG_ROW_PAD: f32 = 12.;

/// Extra height (px) a text row gets beyond its tallest inline `$…$` formula, so a fraction
/// has a little breathing room above + below instead of touching the neighbouring rows.
const INLINE_MATH_ROW_PAD: f32 = 6.;

/// Side length (px) of the square drag-to-resize grip painted at an inline
/// image's bottom-right corner (matching the reading view's 14px handle).
const IMG_GRIP: f32 = 14.;

/// Smallest width (px) a drag may shrink an inline image to, so it can't vanish.
const IMG_MIN_W: f32 = 40.;

/// An in-progress drag of an inline image's corner grip: which logical line's
/// `![](src)` is being resized, its display width when the drag began, the
/// pointer x at grab, and the live (preview) width the drag has reached. The
/// image paints at `width` (aspect-preserved) until release writes `{width=N}`.
#[derive(Clone, Copy)]
struct ImageResize {
    line: usize,
    start_width: f32,
    start_x: Pixels,
    width: f32,
}

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
    /// Popup top-left, in window space (rendered on a deferred/anchored layer).
    anchor: Point<Pixels>,
    /// The diagnostic's byte range, replaced when a suggestion is chosen.
    range: Range<usize>,
    suggestions: Vec<SharedString>,
    /// Scroll state of the (capped-height) list, so a thumb can track it.
    scroll: ScrollHandle,
}

/// A column edit applied to every row of a table (insert/delete a cell at index).
#[derive(Clone, Copy)]
enum ColEdit {
    Insert(usize),
    Delete(usize),
}

/// An item in the table right-click menu (Word-style table editing).
#[derive(Clone, Copy)]
enum TableMenuAction {
    InsertRowAbove,
    InsertRowBelow,
    InsertColLeft,
    InsertColRight,
    DeleteRow,
    DeleteColumn,
    AlignLeft,
    AlignCenter,
    AlignRight,
    DeleteTable,
}

impl TableMenuAction {
    const ITEMS: &'static [(&'static str, TableMenuAction)] = &[
        ("Insert row above", TableMenuAction::InsertRowAbove),
        ("Insert row below", TableMenuAction::InsertRowBelow),
        ("Insert column left", TableMenuAction::InsertColLeft),
        ("Insert column right", TableMenuAction::InsertColRight),
        ("Delete row", TableMenuAction::DeleteRow),
        ("Delete column", TableMenuAction::DeleteColumn),
        ("Align left", TableMenuAction::AlignLeft),
        ("Align center", TableMenuAction::AlignCenter),
        ("Align right", TableMenuAction::AlignRight),
        ("Delete table", TableMenuAction::DeleteTable),
    ];

    fn apply(self, editor: &mut EditorState, cx: &mut Context<EditorState>) {
        match self {
            TableMenuAction::InsertRowAbove => editor.insert_table_row(false, cx),
            TableMenuAction::InsertRowBelow => editor.insert_table_row(true, cx),
            TableMenuAction::InsertColLeft => editor.insert_table_column(false, cx),
            TableMenuAction::InsertColRight => editor.insert_table_column(true, cx),
            TableMenuAction::DeleteRow => editor.delete_table_row(cx),
            TableMenuAction::DeleteColumn => editor.delete_table_column(cx),
            TableMenuAction::AlignLeft => editor.set_caret_table_align(CellAlign::Left, cx),
            TableMenuAction::AlignCenter => editor.set_caret_table_align(CellAlign::Center, cx),
            TableMenuAction::AlignRight => editor.set_caret_table_align(CellAlign::Right, cx),
            TableMenuAction::DeleteTable => editor.delete_table(cx),
        }
    }
}

/// Events the editor emits so a host can react. Subscribe with
/// `cx.subscribe(&editor, …)` — e.g. to re-run spell-check after an edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorEvent {
    /// The document text changed via a user edit (typing, delete, paste, IME,
    /// applying a suggestion). Not emitted for programmatic `set_text`.
    Changed,
    /// A file chip (e.g. a PDF embed) was left-clicked — the host opens its
    /// `src`. The chip itself stays in the document; this is a navigation hint.
    OpenLink(SharedString),
    /// The caret / selection moved without a text change — so a host can update a
    /// caret-anchored affordance (e.g. the table-alignment toolbar).
    SelectionChanged,
    /// The caret entered a `$$…$$` math block (by click, or by arrowing into it): its byte
    /// `range` in the document (covering both fences) and the LaTeX `source` between them, so
    /// the host can open a structural editor and replace the block's text on commit. `at_end`
    /// seats that editor's caret at the formula's end (entered from below/right or by click)
    /// vs its start (from above/left).
    EditMath {
        range: Range<usize>,
        source: SharedString,
        at_end: bool,
        /// `true` for an inline `$…$` span (host splices `$…$` back, seats the editor at the
        /// formula's spot); `false` for a `$$…$$` block (full-width gap).
        inline: bool,
    },
    /// A `$$…$$` math block was right-clicked: the LaTeX source and the window-space click
    /// position, so the host can show a context menu (Copy LaTeX / Export).
    MathMenu {
        source: SharedString,
        position: Point<Pixels>,
    },
}

/// A table column's text alignment, for the host-driven alignment toolbar
/// ([`EditorState::caret_table_align`] / [`EditorState::set_caret_table_align`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CellAlign {
    Left,
    Center,
    Right,
}

/// Provides replacement suggestions for a flagged word (best first); set by the
/// host via [`EditorState::on_suggest`] and consulted on right-click.
type SuggestFn = Box<dyn Fn(&str) -> Vec<String>>;

/// Resolves a standalone image line's `src` to a decoded image so the editor can
/// render it inline (W4). Set by the host via
/// [`EditorState::set_block_image_provider`]; the host owns loading + caching and
/// returns `None` while still decoding / on failure (the line shows raw source).
type BlockImageFn = Box<dyn Fn(&str) -> Option<Arc<RenderImage>>>;

/// Classifies an `![](src)` reference as a file chip (e.g. a PDF) rather than an
/// image, returning its display label. Set via
/// [`EditorState::set_block_chip_provider`]; the editor renders such a line as a
/// clickable chip (left-click emits [`EditorEvent::OpenLink`]).
type BlockChipFn = Box<dyn Fn(&str) -> Option<SharedString>>;

/// Resolves a ` ```mermaid ` block's source to a rendered diagram bitmap, so the
/// editor can render the block as the diagram (caret outside) instead of code.
/// Set via [`EditorState::set_block_mermaid_provider`]; the host renders +
/// caches off-thread (see [`EditorState::mermaid_sources`] to pre-render).
type BlockMermaidFn = Box<dyn Fn(&str) -> Option<Arc<RenderImage>>>;

/// Resolves a `$$…$$` math block's LaTeX to a typeset bitmap, so the editor can
/// render the block as the equation (caret outside) instead of raw source. Set via
/// [`EditorState::set_block_math_provider`]; pre-render with [`math_sources`].
type BlockMathFn = Box<dyn Fn(&str) -> Option<Arc<RenderImage>>>;

/// The diagram sources of every ` ```mermaid ` block in `content`, so a host can
/// pre-render them (the editor's mermaid provider then finds the ready bitmap).
pub fn mermaid_sources(content: &str) -> Vec<SharedString> {
    markdown_syntax::mermaid_blocks(content)
        .into_iter()
        .map(|(_, source)| source.into())
        .collect()
}

/// The LaTeX sources of every `$$…$$` math block in `content`, so a host can
/// pre-render them (the editor's math provider then finds the ready bitmap).
pub fn math_sources(content: &str) -> Vec<SharedString> {
    markdown_syntax::math_blocks(content)
        .into_iter()
        .map(|(_, source)| source.into())
        .collect()
}

/// The LaTeX sources of every inline `$…$` formula in `content` (the inner LaTeX, no `$`
/// delimiters), so a host can pre-render them into the same math store the block provider
/// reads. Skips lines inside fenced code blocks, where `$…$` is literal.
pub fn inline_math_sources(content: &str) -> Vec<SharedString> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for line in content.split('\n') {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        for span in markdown_syntax::inline_math_spans(line) {
            out.push(line[span.start + 1..span.end - 1].into());
        }
    }
    out
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
    /// Per-logical-line wrap-row height. Variable so a heading (bigger font) gets
    /// a taller row (W2); `line_height` is the base/fallback for the empty doc
    /// and any row without a recorded height.
    line_heights: Vec<Pixels>,
    /// Per-logical-line table-grid row (from the last paint), so a click /
    /// Tab / caret hit-tests against cells instead of the raw source line.
    table_rows: Vec<Option<TableRow>>,
    /// Hover-revealed "+" add-row / add-column strips for each table (issue #16),
    /// each paired with the table row to seat the caret in before inserting. From
    /// the last paint, committed only while the table is hovered; hit-tested on
    /// mouse-down.
    table_row_add_rects: Vec<(Bounds<Pixels>, usize)>,
    table_col_add_rects: Vec<(Bounds<Pixels>, usize)>,
    /// Each table's hover zone (grid + a thin margin), committed every paint so
    /// `on_mouse_move` can repaint when the pointer's table-affordance region
    /// changes (the editor otherwise only repaints on the caret blink).
    table_hover_zones: Vec<Bounds<Pixels>>,
    /// The affordance region the pointer was last in — `(table index, 0 = zone /
    /// 1 = below strip / 2 = right strip)` — so the repaint fires only on change.
    table_hover_region: Option<(usize, u8)>,
    /// Committed delete-handle rects (issue #16): the hovered row's "−" `(bounds,
    /// row)` and the hovered column's "−" `(bounds, row, col)`, hit-tested on click.
    table_row_del: Option<(Bounds<Pixels>, usize)>,
    table_col_del: Option<(Bounds<Pixels>, usize, usize)>,
    /// The table cell `(row, col)` the pointer was last over, so `on_mouse_move`
    /// repaints the delete handles when it changes.
    table_hover_cell: Option<(usize, usize)>,
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
    /// Whether the last content edit was a single typed grapheme or a single-char
    /// backspace — the only edits auto-pairing should react to, so programmatic /
    /// structural edits (table ops, etc.) don't trip it.
    last_edit_keystroke: bool,
    /// Spaces inserted per Tab / one list-nesting level (`Indent`/`Outdent`); set
    /// by the host via [`Self::set_tab_indent`] to match its list-indent setting.
    tab_indent: usize,
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
    /// The open table right-click menu's anchor (window space), if any. Its actions
    /// operate on the caret's table cell.
    table_menu: Option<Point<Pixels>>,
    /// Scroll state for the table menu, so its overflow scrolls + shows a thumb.
    table_menu_scroll: ScrollHandle,
    /// Supplies replacement suggestions for a flagged word, fetched lazily when
    /// the user right-clicks it. Set by the host via [`Self::on_suggest`];
    /// without it, the right-click menu has nothing to offer.
    suggest: Option<SuggestFn>,
    /// Resolves a standalone image line's `src` to a decoded image for inline
    /// rendering (W4); set by the host via [`Self::set_block_image_provider`].
    block_image: Option<BlockImageFn>,
    /// Classifies an `![](src)` as a file chip (e.g. a PDF) + its label; set by
    /// the host via [`Self::set_block_chip_provider`].
    block_chip: Option<BlockChipFn>,
    /// Resolves a ` ```mermaid ` block's source to a rendered diagram; set by the
    /// host via [`Self::set_block_mermaid_provider`].
    block_mermaid: Option<BlockMermaidFn>,
    /// Resolves a `$$…$$` block's LaTeX to a typeset equation; set by the host via
    /// [`Self::set_block_math_provider`].
    block_math: Option<BlockMathFn>,
    /// The em (px/font-size) the `block_math` provider rasterizes at — set via
    /// [`Self::set_block_math_em`]. Inline `$…$` formulas reuse those rasters scaled by
    /// `text_em / this`, so they sit at text size. `None` disables inline math rendering.
    block_math_em: Option<f32>,
    /// Per-logical-line `src` for rows painted as a file chip (from the last
    /// paint), so a left-click can open it and a right-click can edit it.
    chip_rows: Vec<Option<SharedString>>,
    /// Window-space painted bounds of each inline image, with its logical line
    /// index (from the last paint), so a press near a corner can start a resize
    /// and know which `![](src)` line to rewrite. One entry per rendered image.
    image_rects: Vec<(usize, Bounds<Pixels>)>,
    /// Window-space bounds of each painted task checkbox, with its logical line —
    /// so a click on the box toggles `[ ]`↔`[x]` instead of placing the caret.
    checkbox_rects: Vec<(usize, Bounds<Pixels>)>,
    /// The in-progress corner-grip drag, if any (see [`ImageResize`]). While set,
    /// that image paints at the live width and other mouse handling is suppressed.
    image_resize: Option<ImageResize>,
    /// A `$$…$$` block being edited in-line: its byte range + the host-supplied view (the
    /// structural editor) painted in a reserved gap at the block's spot. `None` = none.
    editing_block: Option<EditingBlock>,
    /// Window-space painted bounds of each inline `$…$` formula + its absolute byte range and
    /// inner LaTeX (from the last paint), so a click can open its structural editor and the
    /// seated editor can be positioned at the formula's spot.
    inline_math_rects: Vec<(Range<usize>, SharedString, Bounds<Pixels>)>,
    /// An inline `$…$` formula under structural edit: its byte range + the host's editor view,
    /// overlaid at the formula's spot. `None` = none.
    editing_inline: Option<EditingInline>,
}

/// A math block under in-line structural edit: the byte range to overwrite on commit, and
/// the host's editor view to render in the reserved gap.
struct EditingBlock {
    range: Range<usize>,
    view: gpui::AnyView,
    /// The block's displayed height — the gap reserved while editing, so the formula stays
    /// put instead of jumping to a fixed size.
    height: Pixels,
}

/// An inline `$…$` formula under structural edit: the byte range to overwrite on commit, and
/// the host's editor view, overlaid at the formula's painted spot.
struct EditingInline {
    range: Range<usize>,
    view: gpui::AnyView,
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
            table_rows: Vec::new(),
            table_row_add_rects: Vec::new(),
            table_col_add_rects: Vec::new(),
            table_hover_zones: Vec::new(),
            table_hover_region: None,
            table_row_del: None,
            table_col_del: None,
            table_hover_cell: None,
            last_bounds: None,
            line_height: px(20.),
            is_selecting: false,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            last_edit: EditKind::Other,
            last_edit_keystroke: false,
            tab_indent: 4,
            goal_x: None,
            diagnostics: Vec::new(),
            markdown_style: None,
            menu: None,
            table_menu: None,
            table_menu_scroll: ScrollHandle::new(),
            suggest: None,
            block_image: None,
            block_chip: None,
            block_mermaid: None,
            block_math: None,
            block_math_em: None,
            chip_rows: Vec::new(),
            image_rects: Vec::new(),
            checkbox_rects: Vec::new(),
            image_resize: None,
            editing_block: None,
            inline_math_rects: Vec::new(),
            editing_inline: None,
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

    /// Replace byte `range` with `text` as ONE recorded (undoable) edit, leaving the caret
    /// after the inserted text. Unlike [`Self::set_text`] this preserves — and extends — the
    /// undo history, so a host writing back a structural edit (e.g. a committed `$$…$$`
    /// formula) lands as a normal undo step rather than clobbering the history.
    pub fn replace_range(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        // Snap to char boundaries (start down, end up) so a stale/shifted range — e.g. one
        // captured before a prior formula commit moved the bytes — can't panic mid-UTF-8.
        let len = self.content.len();
        let mut start = range.start.min(len);
        while start > 0 && !self.content.is_char_boundary(start) {
            start -= 1;
        }
        let mut end = range.end.clamp(start, len);
        while end < len && !self.content.is_char_boundary(end) {
            end += 1;
        }
        let range = start..end;
        self.record_edit(&range, text);
        self.content.replace_range(range.clone(), text);
        self.remap_diagnostics(&range, text.len());
        let caret = range.start + text.len();
        self.selected_range = caret..caret;
        self.selection_reversed = false;
        self.marked_range = None;
        // Don't coalesce a following keystroke into this structural replacement.
        self.last_edit = EditKind::Other;
        cx.notify();
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

    /// Install the provider that classifies an `![](src)` reference as a file chip
    /// (e.g. a PDF) and supplies its label. With it, such lines render as a
    /// clickable chip when the caret is elsewhere; a left-click emits
    /// [`EditorEvent::OpenLink`] and a right-click places the caret to edit.
    pub fn set_block_chip_provider(
        &mut self,
        provider: impl Fn(&str) -> Option<SharedString> + 'static,
    ) {
        self.block_chip = Some(Box::new(provider));
    }

    /// Install the provider that resolves a ` ```mermaid ` block's source to a
    /// rendered diagram. With it, such a block renders as the diagram when the
    /// caret is elsewhere; with the caret inside (or while it renders) it shows
    /// the raw fenced source. Pre-render with [`mermaid_sources`].
    pub fn set_block_mermaid_provider(
        &mut self,
        provider: impl Fn(&str) -> Option<Arc<RenderImage>> + 'static,
    ) {
        self.block_mermaid = Some(Box::new(provider));
    }

    /// Install the provider that resolves a `$$…$$` block's LaTeX to a typeset
    /// equation. With it, such a block renders as the equation when the caret is
    /// elsewhere; with the caret inside (or while it renders) it shows the raw
    /// `$$…$$` source. Pre-render with [`math_sources`].
    pub fn set_block_math_provider(
        &mut self,
        provider: impl Fn(&str) -> Option<Arc<RenderImage>> + 'static,
    ) {
        self.block_math = Some(Box::new(provider));
    }

    /// Declare the em the `block_math` provider rasterizes at (e.g. the host's display-math
    /// font size). Turns on inline `$…$` rendering: each inline formula reuses the block
    /// raster for the same LaTeX, scaled by `text_em / em` so it sits at text size. Pre-render
    /// inline sources too (see [`inline_math_sources`]).
    pub fn set_block_math_em(&mut self, em: f32) {
        self.block_math_em = (em > 0.).then_some(em);
    }

    /// Begin an in-line structural edit of the `$$…$$` block at `range`: reserve a gap at
    /// its spot and paint `view` (the host's editor) there. The host focuses `view`.
    pub fn set_editing_block(
        &mut self,
        range: Range<usize>,
        view: gpui::AnyView,
        height: Pixels,
        cx: &mut Context<Self>,
    ) {
        self.editing_block = Some(EditingBlock {
            range,
            view,
            height,
        });
        cx.notify();
    }

    /// End an in-line math edit (the host has committed / cancelled). Returns the block's
    /// byte range, so the host can overwrite it.
    pub fn end_editing_block(&mut self, cx: &mut Context<Self>) -> Option<Range<usize>> {
        let range = self.editing_block.take().map(|eb| eb.range);
        cx.notify();
        range
    }

    /// Begin a structural edit of the inline `$…$` span at `range` (absolute bytes): overlay
    /// `view` (the host's editor) at the formula's painted spot. The host focuses `view`.
    pub fn set_editing_inline(
        &mut self,
        range: Range<usize>,
        view: gpui::AnyView,
        cx: &mut Context<Self>,
    ) {
        self.editing_inline = Some(EditingInline { range, view });
        cx.notify();
    }

    /// End an inline math edit. Returns the span's byte range, so the host can overwrite it.
    pub fn end_editing_inline(&mut self, cx: &mut Context<Self>) -> Option<Range<usize>> {
        let range = self.editing_inline.take().map(|e| e.range);
        cx.notify();
        range
    }

    /// Whether `range` still bounds an inline `$…$` span (a `$` at each end, content between, no
    /// newline, not a `$$` fence) — guards the inline commit against a stale/shifted range that
    /// would otherwise splice text at the wrong spot.
    pub fn is_inline_math_range(&self, range: &Range<usize>) -> bool {
        range.start < range.end
            && range.end <= self.content.len()
            && self.content.is_char_boundary(range.start)
            && self.content.is_char_boundary(range.end)
            && {
                let s = &self.content[range.clone()];
                s.len() >= 3
                    && s.starts_with('$')
                    && s.ends_with('$')
                    && !s.starts_with("$$")
                    && !s.contains('\n')
            }
    }

    /// The horizontal alignment of the `$$…$$` block whose byte range starts at `block_start`
    /// (its `<!-- math:ALIGN -->` marker, or `Center` by default) — so the host can seed the
    /// in-line editor at the right justification when opening it.
    pub fn math_align(&self, block_start: usize) -> MathAlign {
        let row = self.row_col(block_start).0;
        markdown_syntax::math_regions(&self.content)
            .into_iter()
            .find(|r| r.range.start == row)
            .map_or(MathAlign::default(), |r| r.align)
    }

    /// Compute the recorded edit that writes `align`'s marker for the `$$` block at byte
    /// `block`: the (possibly marker-extended) range to replace, and the marker prefix to
    /// prepend to the rewritten block. Center (default) → no marker (drops any existing one);
    /// left/right → add or replace it. The host appends the block text to the prefix. Folding
    /// the marker into the block's commit edit avoids a separate, range-shifting edit.
    pub fn math_marker_edit(
        &self,
        block: Range<usize>,
        align: MathAlign,
    ) -> (Range<usize>, String) {
        let row = self.row_col(block.start).0;
        let prefix = align.marker().map_or(String::new(), |m| format!("{m}\n"));
        let has_marker =
            row > 0 && markdown_syntax::math_align_marker(self.line_str(row - 1)).is_some();
        let start = if has_marker {
            self.line_starts()[row - 1]
        } else {
            block.start
        };
        (start..block.end, prefix)
    }

    /// Re-find a `$$…$$` block by its exact LaTeX `source`, returned as a BYTE range (nearest
    /// to the now-stale byte `approx` if several match) — so opening/committing one after a
    /// prior formula's commit shifted offsets targets the right block. `math_blocks` yields
    /// LINE ranges, so convert like `math_block_at` does (else the caret jumps to the top).
    pub fn find_math_block(&self, source: &str, approx: usize) -> Option<Range<usize>> {
        let starts = self.line_starts();
        markdown_syntax::math_blocks(&self.content)
            .into_iter()
            .filter(|(_, s)| s == source)
            .map(|(r, _)| starts[r.start]..self.line_end(r.end - 1))
            .min_by_key(|r| r.start.abs_diff(approx))
    }

    /// Re-find an inline `$…$` span by its exact inner LaTeX, as an absolute byte range (nearest
    /// to the now-stale byte `approx` if several match) — the inline counterpart of
    /// [`Self::find_math_block`], so opening/committing after a prior edit shifted offsets
    /// targets the right span.
    pub fn find_inline_math(&self, latex: &str, approx: usize) -> Option<Range<usize>> {
        let mut line_start = 0;
        let mut best: Option<Range<usize>> = None;
        for line in self.content.split('\n') {
            for span in markdown_syntax::inline_math_spans(line) {
                if &line[span.start + 1..span.end - 1] == latex {
                    let abs = line_start + span.start..line_start + span.end;
                    if best
                        .as_ref()
                        .is_none_or(|b| abs.start.abs_diff(approx) < b.start.abs_diff(approx))
                    {
                        best = Some(abs);
                    }
                }
            }
            line_start += line.len() + 1;
        }
        best
    }

    /// Whether byte `range` (half-open) still starts a `$$…$$` block — a commit guard so a
    /// stale/shifted range can't splice the block into the wrong place and corrupt the doc.
    pub fn is_math_block_range(&self, range: &Range<usize>) -> bool {
        range.end <= self.content.len()
            && range.start <= range.end
            && self.content.is_char_boundary(range.start)
            && self.content[range.start..range.end]
                .trim_start()
                .starts_with("$$")
    }

    /// The text of logical line `row` (without its trailing newline).
    fn line_str(&self, row: usize) -> &str {
        let starts = self.line_starts();
        match starts.get(row) {
            Some(&s) => &self.content[s..self.line_end(row)],
            None => "",
        }
    }

    /// The host-supplied editor view for an in-line math edit, positioned in the gap its
    /// block reserves (from the last paint's line tops/heights). An absolute child of the
    /// editor's `relative` root, so it scrolls with the content.
    fn editing_block_overlay(&self) -> Option<gpui::Div> {
        let eb = self.editing_block.as_ref()?;
        let row = self.row_col(eb.range.start).0;
        let top = *self.line_tops.get(row)?;
        let height = *self.line_heights.get(row)?;
        Some(
            div()
                .absolute()
                .top(top)
                .left(px(0.))
                .w_full()
                .h(height)
                // Occlude so clicks inside the hosted math editor don't fall through to the
                // text layer below — which would seat the caret on the next line and steal
                // focus, blurring (committing + closing) the structural editor.
                .occlude()
                .child(eb.view.clone()),
        )
    }

    /// The host-supplied editor view for an inline `$…$` edit, overlaid at the formula's last-
    /// painted spot (its window rect, made editor-relative via `content_origin`). Unlike a
    /// `$$` block it doesn't reserve a full-width gap — it floats over the formula, leaving the
    /// surrounding text in place.
    fn editing_inline_overlay(&self) -> Option<gpui::Div> {
        let ei = self.editing_inline.as_ref()?;
        let (_, _, rect) = self
            .inline_math_rects
            .iter()
            .find(|(r, _, _)| *r == ei.range)?;
        let origin = self.last_bounds.map_or(Point::default(), |b| b.origin);
        Some(
            div()
                .absolute()
                .top(rect.origin.y - origin.y)
                .left(rect.origin.x - origin.x)
                .occlude()
                .child(ei.view.clone()),
        )
    }

    /// Spaces inserted per Tab / list-nesting level (`Indent`/`Outdent`). The host
    /// keeps this in sync with its list-indent setting so nesting is configurable.
    pub fn set_tab_indent(&mut self, spaces: usize) {
        self.tab_indent = spaces.max(1);
    }

    /// The caret's byte offset into [`Self::text`] (the moving end of any
    /// selection). For hosts that drive a menu/completion off the caret position.
    pub fn cursor(&self) -> usize {
        self.cursor_offset()
    }

    /// Whether the last content change was a single typed character or single-char
    /// backspace (vs a programmatic / multi-char edit). Hosts gate auto-pairing on
    /// this so structural edits (table row/column ops, paste, …) don't trip it.
    pub fn last_edit_was_keystroke(&self) -> bool {
        self.last_edit_keystroke
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
        if !self.selected_range.is_empty() {
            self.move_to(self.selected_range.start, cx);
            return;
        }
        if self.caret_in_table()
            && let Some(off) = self.table_move_horizontal(-1)
        {
            self.move_to(off, cx);
            return;
        }
        let off = self.previous_boundary(self.cursor_offset());
        if let Some((range, source)) = self.inline_math_span_at(off) {
            cx.emit(EditorEvent::EditMath {
                range,
                source,
                at_end: true,
                inline: true,
            });
            return;
        }
        if let Some((range, source)) = self.math_block_at(self.row_col(off).0) {
            cx.emit(EditorEvent::EditMath {
                range,
                source,
                at_end: true,
                inline: false,
            });
            return;
        }
        self.move_to(off, cx);
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            self.move_to(self.selected_range.end, cx);
            return;
        }
        if self.caret_in_table()
            && let Some(off) = self.table_move_horizontal(1)
        {
            self.move_to(off, cx);
            return;
        }
        let off = self.next_boundary(self.cursor_offset());
        if let Some((range, source)) = self.inline_math_span_at(off) {
            cx.emit(EditorEvent::EditMath {
                range,
                source,
                at_end: false,
                inline: true,
            });
            return;
        }
        if let Some((range, source)) = self.math_block_at(self.row_col(off).0) {
            cx.emit(EditorEvent::EditMath {
                range,
                source,
                at_end: false,
                inline: false,
            });
            return;
        }
        self.move_to(off, cx);
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        // In a table, step cell-to-cell keeping the column; at the table's edge
        // `table_move_vertical` returns `None` and a normal move exits the table.
        if self.caret_in_table()
            && let Some(off) = self.table_move_vertical(-1)
        {
            self.move_to(off, cx);
            return;
        }
        let off = self.move_vertical(-1);
        if let Some((range, source)) = self.inline_math_span_at(off) {
            cx.emit(EditorEvent::EditMath {
                range,
                source,
                at_end: true,
                inline: true,
            });
            return;
        }
        if let Some((range, source)) = self.math_block_at(self.row_col(off).0) {
            cx.emit(EditorEvent::EditMath {
                range,
                source,
                at_end: true,
                inline: false,
            });
            return;
        }
        // Set the caret directly (not via `move_to`) to keep the goal column.
        self.selected_range = off..off;
        self.last_edit = EditKind::Other;
        cx.emit(EditorEvent::SelectionChanged);
        cx.notify();
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        if self.caret_in_table()
            && let Some(off) = self.table_move_vertical(1)
        {
            self.move_to(off, cx);
            return;
        }
        let off = self.move_vertical(1);
        if let Some((range, source)) = self.inline_math_span_at(off) {
            cx.emit(EditorEvent::EditMath {
                range,
                source,
                at_end: false,
                inline: true,
            });
            return;
        }
        if let Some((range, source)) = self.math_block_at(self.row_col(off).0) {
            cx.emit(EditorEvent::EditMath {
                range,
                source,
                at_end: false,
                inline: false,
            });
            return;
        }
        self.selected_range = off..off;
        self.last_edit = EditKind::Other;
        cx.emit(EditorEvent::SelectionChanged);
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
        // Inside a table, a raw newline would split the row's `| … |` markup.
        // Enter instead moves to the cell directly below (next row, same column,
        // spreadsheet-style); from the last row it exits onto a fresh line below
        // the table.
        if self.caret_in_table() {
            if let Some(off) = self.table_move_vertical(1)
                && self
                    .table_rows
                    .get(self.row_col(off).0)
                    .and_then(Option::as_ref)
                    .is_some_and(|t| !t.is_separator)
            {
                self.move_to(off, cx);
                return;
            }
            let (row, _) = self.row_col(self.cursor_offset());
            let mut last = row;
            while self
                .table_rows
                .get(last + 1)
                .and_then(Option::as_ref)
                .is_some()
            {
                last += 1;
            }
            let starts = self.line_starts();
            let end = starts.get(last + 1).map_or(self.content.len(), |&s| s - 1);
            self.selected_range = end..end;
            self.replace_text_in_range(None, "\n", window, cx);
            return;
        }
        // List auto-continuation: Enter on a list/task item opens the next item
        // (same marker + indent; ordered numbers increment); Enter on an *empty*
        // item removes the marker, exiting the list. Only with a collapsed
        // selection — a selection is just replaced by the newline.
        if self.selected_range.is_empty() {
            let cursor = self.cursor_offset();
            let line_start = self.content[..cursor].rfind('\n').map_or(0, |i| i + 1);
            let line_end = self.content[line_start..]
                .find('\n')
                .map_or(self.content.len(), |i| line_start + i);
            let line = &self.content[line_start..line_end];
            if let Some((prefix_len, indent, ordered, num)) = markdown_syntax::list_prefix(line) {
                let task = markdown_syntax::task_prefix(line);
                let content_start = task.map_or(prefix_len, |(l, ..)| l);
                let empty = line.get(content_start..).unwrap_or("").trim().is_empty();
                let cont = if empty {
                    None
                } else {
                    let ws = &line[..indent];
                    let bullet = line.as_bytes()[indent] as char;
                    Some(if task.is_some() {
                        format!("\n{ws}{bullet} [ ] ")
                    } else if ordered {
                        format!("\n{ws}{}. ", num + 1)
                    } else {
                        format!("\n{ws}{bullet} ")
                    })
                };
                match cont {
                    // Empty item: clear the marker, leaving an empty line.
                    None => {
                        self.selected_range = line_start..line_end;
                        self.replace_text_in_range(None, "", window, cx);
                    }
                    Some(text) => self.replace_text_in_range(None, &text, window, cx),
                }
                return;
            }
        }
        self.replace_text_in_range(None, "\n", window, cx);
    }

    /// Toggle an inline wrapping marker (`**` bold, `*` italic, `` ` `` code)
    /// around the selection. No-op on an empty selection. Unwraps when the
    /// selection is already wrapped (markers just inside or just outside it),
    /// otherwise wraps — keeping the same text selected so presses toggle.
    fn toggle_wrap(&mut self, marker: &str, cx: &mut Context<Self>) {
        let sel = self.selected_range.clone();
        if sel.start >= sel.end {
            return;
        }
        let ml = marker.len();
        let sel_text = &self.content[sel.clone()];
        let (range, new, new_sel) = if sel_text.len() >= 2 * ml
            && sel_text.starts_with(marker)
            && sel_text.ends_with(marker)
        {
            // `**foo**` selected → strip the markers inside the selection.
            let inner = self.content[sel.start + ml..sel.end - ml].to_string();
            (sel.clone(), inner, sel.start..sel.end - 2 * ml)
        } else if self.content[..sel.start].ends_with(marker)
            && self.content[sel.end..].starts_with(marker)
        {
            // `foo` selected with the markers just outside → strip them.
            (
                sel.start - ml..sel.end + ml,
                sel_text.to_string(),
                sel.start - ml..sel.end - ml,
            )
        } else {
            // Plain → wrap.
            (
                sel.clone(),
                format!("{marker}{sel_text}{marker}"),
                sel.start + ml..sel.end + ml,
            )
        };
        self.record_edit(&range, &new);
        self.content.replace_range(range.clone(), &new);
        self.selected_range = new_sel;
        self.selection_reversed = false;
        self.goal_x = None;
        self.remap_diagnostics(&range, new.len());
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    fn bold(&mut self, _: &Bold, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_wrap("**", cx);
    }

    fn italic(&mut self, _: &Italic, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_wrap("*", cx);
    }

    fn code(&mut self, _: &Code, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_wrap("`", cx);
    }

    /// Tab: on a list/quote item, indent the whole item one level (`tab_indent`
    /// spaces at the line start, caret shifts with it); elsewhere insert that many
    /// spaces at the caret (replacing any selection).
    fn indent(&mut self, _: &Indent, window: &mut Window, cx: &mut Context<Self>) {
        // In a table, Tab moves to the next cell rather than indenting.
        if self.caret_in_table() {
            if let Some(offset) = self.table_cell_nav(true) {
                self.move_to(offset, cx);
            }
            return;
        }
        let cursor = self.cursor_offset();
        let line_start = self.content[..cursor].rfind('\n').map_or(0, |i| i + 1);
        let line_end = self.content[line_start..]
            .find('\n')
            .map_or(self.content.len(), |i| line_start + i);
        let line = &self.content[line_start..line_end];
        let is_item = markdown_syntax::list_prefix(line).is_some()
            || markdown_syntax::blockquote_prefix(line).is_some();
        let indent = " ".repeat(self.tab_indent);
        if !is_item {
            self.replace_text_in_range(None, &indent, window, cx);
            return;
        }
        let range = line_start..line_start;
        self.record_edit(&range, &indent);
        self.content.insert_str(line_start, &indent);
        let caret = cursor + indent.len();
        self.selected_range = caret..caret;
        self.selection_reversed = false;
        self.goal_x = None;
        self.remap_diagnostics(&range, indent.len());
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    /// Shift+Tab: outdent the caret's line — remove up to `tab_indent` leading
    /// spaces (or one leading tab) from the line start. No-op if there's none.
    fn outdent(&mut self, _: &Outdent, _: &mut Window, cx: &mut Context<Self>) {
        // In a table, Shift+Tab moves to the previous cell rather than outdenting.
        if self.caret_in_table() {
            if let Some(offset) = self.table_cell_nav(false) {
                self.move_to(offset, cx);
            }
            return;
        }
        let cursor = self.cursor_offset();
        let line_start = self.content[..cursor].rfind('\n').map_or(0, |i| i + 1);
        let line = &self.content[line_start..];
        let removed = if line.starts_with('\t') {
            1
        } else {
            line.bytes()
                .take(self.tab_indent)
                .take_while(|b| *b == b' ')
                .count()
        };
        if removed == 0 {
            return;
        }
        let range = line_start..line_start + removed;
        self.record_edit(&range, "");
        self.content.replace_range(range.clone(), "");
        let caret = cursor.saturating_sub(removed).max(line_start);
        self.selected_range = caret..caret;
        self.selection_reversed = false;
        self.goal_x = None;
        self.remap_diagnostics(&range, 0);
        cx.emit(EditorEvent::Changed);
        cx.notify();
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
        // A keystroke is one typed grapheme (incl. typed over a selection — that's
        // an auto-pair "wrap") or a single-char backspace. Multi-char edits (paste,
        // table ops, …) are not, so auto-pairing skips them.
        self.last_edit_keystroke = (new_text != "\n" && new_text.graphemes(true).count() == 1)
            || (new_text.is_empty() && self.content[range.clone()].graphemes(true).count() == 1);
    }

    // --- Mouse ---------------------------------------------------------------

    /// If logical `row` is inside a `$$…$$` block, the block's byte range in the document
    /// (both fences) and the LaTeX between them — so a double-click can hand it to the host's
    /// structural editor.
    fn math_block_at(&self, row: usize) -> Option<(Range<usize>, SharedString)> {
        // The structural LaTeX editor is a WYSIWYG affordance (markdown_style is set only in
        // live-preview mode). In raw-markdown mode the user edits `$$…$$` as plain text, so
        // report no math block here — clicks / arrows / `/math` stay in the text editor.
        self.markdown_style.as_ref()?;
        let starts = self.line_starts();
        markdown_syntax::math_blocks(&self.content)
            .into_iter()
            .find(|(r, _)| r.contains(&row))
            .map(|(r, source)| (starts[r.start]..self.line_end(r.end - 1), source.into()))
    }

    /// The inline `$…$` span strictly containing source byte `off` (between the `$` delimiters),
    /// as an absolute byte range + inner LaTeX — so arrowing the caret into a formula opens its
    /// structural editor instead of landing in (and revealing) the raw source. WYSIWYG-only.
    fn inline_math_span_at(&self, off: usize) -> Option<(Range<usize>, SharedString)> {
        self.markdown_style.as_ref()?;
        let (row, _) = self.row_col(off);
        let line_start = *self.line_starts().get(row)?;
        let line = self.line_str(row);
        let col = off.saturating_sub(line_start);
        markdown_syntax::inline_math_spans(line)
            .into_iter()
            .find(|s| s.start < col && col < s.end)
            .map(|s| {
                (
                    line_start + s.start..line_start + s.end,
                    SharedString::from(line[s.start + 1..s.end - 1].to_string()),
                )
            })
    }

    /// If the caret sits inside a `$$…$$` block, ask the host to open the structural editor
    /// for it (caret at the formula's start). Lets the host turn a freshly-inserted, empty
    /// math block (the `/math` snippet) straight into a live editor instead of raw source.
    pub fn edit_math_at_caret(&mut self, cx: &mut Context<Self>) {
        let (row, _) = self.row_col(self.cursor_offset());
        if let Some((range, source)) = self.math_block_at(row) {
            cx.emit(EditorEvent::EditMath {
                range,
                source,
                at_end: false,
                inline: false,
            });
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A press on an image's corner grip starts a resize drag — this takes
        // precedence over placing the caret on the image row (which the press
        // would otherwise do). The image keeps its bounds; the drag previews a new
        // width and release writes `{width=N}` (see on_mouse_move / on_mouse_up).
        if let Some((line, width)) = self.grip_at(event.position) {
            self.image_resize = Some(ImageResize {
                line,
                start_width: width,
                start_x: event.position.x,
                width,
            });
            self.is_selecting = false;
            self.menu = None;
            self.table_menu = None;
            cx.notify();
            return;
        }
        // A press on a task checkbox toggles it (☐↔☑) instead of placing the
        // caret — the box sits in the gutter, so this never competes with editing
        // the body text. Same length swap, so the caret/selection stay valid.
        if let Some(row) = self.checkbox_at(event.position) {
            let range = self.line_starts()[row]..self.line_end(row);
            if let Some(new_line) =
                markdown_syntax::toggle_task_checkbox(&self.content[range.clone()])
            {
                self.record_edit(&range, &new_line);
                self.content =
                    self.content[..range.start].to_owned() + &new_line + &self.content[range.end..];
                self.remap_diagnostics(&range, new_line.len());
                cx.emit(EditorEvent::Changed);
                cx.notify();
            }
            return;
        }
        // Left-click a file chip (e.g. a PDF embed) opens it rather than editing —
        // the host handles the link. Right-click edits (see on_right_mouse_down).
        if let Some(src) = self.chip_at(event.position) {
            cx.emit(EditorEvent::OpenLink(src));
            return;
        }
        // Left-click an inline `$…$` formula opens its structural editor at the formula's spot
        // (the host seats it). Shift extends a selection; Control-click is the secondary button.
        if !event.modifiers.shift
            && !event.modifiers.control
            && let Some((range, source)) = self.inline_math_at(event.position)
        {
            cx.emit(EditorEvent::EditMath {
                range,
                source,
                at_end: true,
                inline: true,
            });
            return;
        }
        // A press on a table's hover "+" strip adds a row (below) or column (right).
        // The insert APIs are caret-driven, so seat the caret in the table to target
        // them — but capture the user's cell first and restore it after, so the
        // caret stays put instead of following the new row/column.
        if let Some(row) = self.table_add_row_at(event.position) {
            let keep = self.caret_table_cell_pos();
            if let Some(off) = self.cell_start_offset(row, 0) {
                self.selected_range = off..off;
                self.insert_table_row(true, cx);
            }
            if let Some((r, c, ic)) = keep {
                let caret = self.caret_pos_for_cell(r, c, ic);
                self.selected_range = caret..caret;
                cx.notify();
            }
            return;
        }
        if let Some(row) = self.table_add_col_at(event.position) {
            let keep = self.caret_table_cell_pos();
            if let Some(off) = self.last_cell_start_offset(row) {
                self.selected_range = off..off;
                self.insert_table_column(true, cx);
            }
            if let Some((r, c, ic)) = keep {
                let caret = self.caret_pos_for_cell(r, c, ic);
                self.selected_range = caret..caret;
                cx.notify();
            }
            return;
        }
        // A press on a row/column delete "−" handle removes that row/column (seat
        // the caret in it, then reuse the caret-driven delete APIs).
        if let Some((rect, row)) = self.table_row_del
            && rect.contains(&event.position)
        {
            if let Some(off) = self.cell_start_offset(row, 0) {
                self.selected_range = off..off;
                self.delete_table_row(cx);
            }
            return;
        }
        if let Some((rect, row, col)) = self.table_col_del
            && rect.contains(&event.position)
        {
            if let Some(off) = self.cell_start_offset(row, col) {
                self.selected_range = off..off;
                self.delete_table_column(cx);
            }
            return;
        }
        // A click on a table cell drops the caret inside the cell, not in the raw
        // `| … |` source.
        let offset = self
            .table_offset_at(event.position, window)
            .unwrap_or_else(|| self.index_for_mouse_position(event.position));
        self.menu = None;
        self.table_menu = None;
        self.goal_x = None;
        self.last_edit = EditKind::Other;
        match event.click_count {
            // Double-click selects the word under the cursor; a $$…$$ block already
            // opened the structural editor on the first (single) click.
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
                // A single left-click on a $$…$$ block opens the structural editor in
                // place; a Control-click (macOS secondary click, which AppKit delivers as
                // a left button + control modifier, NOT a right button) shows the formula
                // context menu instead. Shift-click still extends the selection.
                if !event.modifiers.shift {
                    let (row, _) = self.row_col(offset);
                    if let Some((range, source)) = self.math_block_at(row) {
                        if event.modifiers.control {
                            self.focus(window, cx);
                            cx.emit(EditorEvent::MathMenu {
                                source,
                                position: event.position,
                            });
                        } else {
                            cx.emit(EditorEvent::EditMath {
                                range,
                                source,
                                at_end: true,
                                inline: false,
                            });
                        }
                        return;
                    }
                }
                self.is_selecting = true;
                if event.modifiers.shift {
                    self.select_to(offset, cx);
                } else {
                    self.move_to(offset, cx);
                }
            }
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        // End an image-resize drag by persisting the rounded width as `{width=N}`
        // in that image's source line (through the normal mutation path, so it
        // joins the undo history + emits Changed); the next paint shows the saved
        // size and the live override clears.
        if let Some(resize) = self.image_resize.take() {
            self.commit_image_resize(resize, cx);
            cx.notify();
            return;
        }
        self.is_selecting = false;
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // While dragging an image's grip, track the pointer: the new width is the
        // grab width plus the horizontal travel, floored at `IMG_MIN_W` and capped
        // to the content width left of the image's inset (so a bulleted image's cap
        // matches `block_img`, no snap-back on release, and it can't run off the
        // page). The paint reads this live width for the dragged image (aspect
        // preserved).
        if let Some(resize) = self.image_resize {
            let avail = self
                .last_bounds
                .map_or(f32::MAX, |b| f32::from(b.size.width))
                - f32::from(self.line_inset(resize.line));
            let max_w = avail.max(IMG_MIN_W);
            let dx = f32::from(event.position.x - resize.start_x);
            let width = (resize.start_width + dx).clamp(IMG_MIN_W, max_w);
            if let Some(r) = self.image_resize.as_mut() {
                r.width = width;
            }
            cx.notify();
            return;
        }
        if self.is_selecting {
            let offset = self
                .table_offset_at(event.position, window)
                .unwrap_or_else(|| self.index_for_mouse_position(event.position));
            self.select_to(offset, cx);
            return;
        }
        // While the right-click menu is open it owns the pointer — don't let the
        // table hover (highlight/handles) track the mouse behind it.
        if self.table_menu.is_some() {
            return;
        }
        // Repaint table "+" affordances when the pointer's region changes, so the
        // hover fill + cursor track the mouse live (the editor otherwise only
        // repaints on the caret blink).
        let region = self.table_hover_region_at(event.position);
        let cell = self.hovered_table_cell(event.position);
        if region != self.table_hover_region || cell != self.table_hover_cell {
            self.table_hover_region = region;
            self.table_hover_cell = cell;
            cx.notify();
        }
    }

    /// Persist a finished grip drag: replace the resized image's source line with
    /// one carrying the rounded `{width=N}`, going through `record_edit` so it's
    /// one undoable edit and emits `Changed`. A no-op if the line vanished or
    /// isn't an image any more (it shaped to an image last paint, but guard
    /// anyway), or if the width didn't actually change.
    fn commit_image_resize(&mut self, resize: ImageResize, cx: &mut Context<Self>) {
        let starts = self.line_starts();
        let Some(&start) = starts.get(resize.line) else {
            return;
        };
        let end = self.line_end(resize.line);
        let line = &self.content[start..end];
        let new_line = set_image_width(line, resize.width.round().max(IMG_MIN_W) as u32);
        if new_line == line {
            return;
        }
        let range = start..end;
        self.record_edit(&range, &new_line);
        self.content = self.content[..start].to_owned() + &new_line + &self.content[end..];
        self.remap_diagnostics(&range, new_line.len());
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    /// Right-click: if the click lands on a flagged word, fetch its suggestions
    /// (lazily, via the provider) and open a menu anchored there; otherwise close
    /// any open menu.
    fn on_right_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Right-click a file chip places the caret to edit its source (the line
        // then reveals raw `![](src)`), instead of opening the spell menu.
        if self.chip_at(event.position).is_some() {
            self.menu = None;
            self.focus(window, cx);
            let offset = self.index_for_mouse_position(event.position);
            self.move_to(offset, cx);
            return;
        }
        // Right-click a $$…$$ block: emit a MathMenu event so the host can show a
        // context menu (Copy LaTeX / Export SVG / PNG). Focus the editor (not the caret
        // move of old) so it stays live after the menu closes.
        {
            let offset = self.index_for_mouse_position(event.position);
            let (row, _) = self.row_col(offset);
            if let Some((_range, source)) = self.math_block_at(row) {
                self.focus(window, cx);
                cx.emit(EditorEvent::MathMenu {
                    source,
                    position: event.position,
                });
                return;
            }
        }
        // Right-click in a table cell: place the caret there + open the table menu
        // (insert/delete rows + columns), instead of the spell menu.
        if let Some(offset) = self.table_offset_at(event.position, window) {
            self.menu = None;
            self.focus(window, cx);
            self.move_to(offset, cx);
            self.table_menu = Some(event.position);
            cx.notify();
            return;
        }
        let offset = self.index_for_mouse_position(event.position);
        // Window-space — the popup renders on a `deferred`/`anchored` layer.
        let anchor = event.position;
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
        if self.menu.take().is_some() || self.table_menu.take().is_some() {
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
        cx.emit(EditorEvent::SelectionChanged);
        cx.notify();
    }

    /// Seat the caret on the plain-text line just before (`after = false`) or after
    /// (`after = true`) the math `block`, and focus the editor — the keyboard counterpart to
    /// clicking away, for when the caret flows out of a `$$…$$` formula's structural editor
    /// (so it never lands on the hidden `$$` fence lines, which would reveal raw source).
    pub fn exit_math(
        &mut self,
        block: Range<usize>,
        after: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus(window, cx);
        let target = if after {
            let (end_row, _) = self.row_col(block.end.saturating_sub(1));
            self.line_starts()
                .get(end_row + 1)
                .copied()
                .unwrap_or(self.content.len())
        } else {
            let (start_row, _) = self.row_col(block.start);
            if start_row > 0 {
                self.line_end(start_row - 1)
            } else {
                0
            }
        };
        self.move_to(target, cx);
    }

    /// The caret's bounds in window space (its painted Y range), or `None` before
    /// the first paint. Lets a host scroll the caret into view; computed from the
    /// layout stored at the last paint, so it's valid for caret moves that don't
    /// change the text (arrow keys, click).
    pub fn caret_screen_bounds(&self) -> Option<Bounds<Pixels>> {
        let bounds = self.last_bounds?;
        let (row, col) = self.row_col(self.cursor_offset());
        let lh = self.line_h(row);
        let p = self
            .wrapped
            .get(row)?
            .position_for_index(self.display_col(row, col), lh)?;
        let top = bounds.top() + self.line_tops.get(row).copied().unwrap_or(px(0.)) + p.y;
        Some(Bounds::from_corners(
            point(bounds.left(), top),
            point(bounds.left(), top + lh),
        ))
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
        cx.emit(EditorEvent::SelectionChanged);
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
        // A reserved gutter gap (a table's top/bottom, a code block's pads) belongs
        // to no row, and the loop assigns it to the row *after* it — right going
        // down, but going up that strands the caret on the far side of the gap (e.g.
        // just below a table). Going up, target the row before the gap instead.
        if dir < 0 && trow > 0 && target_y < self.line_tops[trow] {
            trow -= 1;
        }
        // A table separator (`|---|`) row isn't editable — skip past it (in the
        // direction of travel) so the caret lands on the header/body row rather
        // than dropping the whole table to raw source.
        if self
            .table_rows
            .get(trow)
            .and_then(Option::as_ref)
            .is_some_and(|t| t.is_separator)
        {
            let skip = if dir >= 0 {
                trow + 1
            } else {
                trow.wrapping_sub(1)
            };
            if skip < self.wrapped.len() {
                trow = skip;
            }
        }
        let rel = point(goal, (target_y - self.line_tops[trow]).max(px(0.)));
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

    /// The `src` of a file chip on the row at window `position`, if that row is a
    /// chip (from the last paint) — left-click opens it, right-click edits.
    fn chip_at(&self, position: Point<Pixels>) -> Option<SharedString> {
        if self.wrapped.is_empty() || self.chip_rows.iter().all(Option::is_none) {
            return None;
        }
        let bounds = self.last_bounds.as_ref()?;
        let rel_y = position.y - bounds.top();
        let mut row = self.wrapped.len() - 1;
        for i in 0..self.wrapped.len() {
            let h = self.line_h(i) * (self.wrapped[i].wrap_boundaries().len() + 1) as f32;
            if rel_y < self.line_tops[i] + h {
                row = i;
                break;
            }
        }
        self.chip_rows.get(row).and_then(Option::clone)
    }

    /// The inline `$…$` formula under `position` (its absolute byte range + inner LaTeX), from
    /// the last paint's window-space `inline_math_rects` — so a click opens its editor.
    fn inline_math_at(&self, position: Point<Pixels>) -> Option<(Range<usize>, SharedString)> {
        self.inline_math_rects
            .iter()
            .find(|(_, _, rect)| rect.contains(&position))
            .map(|(range, latex, _)| (range.clone(), latex.clone()))
    }

    /// If `position` lands on an inline image's bottom-right resize grip, the
    /// `(logical line, current display width)` of that image — so a press can
    /// start a corner-grip drag. The grip is the `IMG_GRIP`-side square pinned to
    /// each image's painted corner (see [`Self::image_grip`]); checked against the
    /// last paint's window-space `image_rects`.
    fn grip_at(&self, position: Point<Pixels>) -> Option<(usize, f32)> {
        self.image_rects.iter().find_map(|&(line, rect)| {
            Self::image_grip(rect)
                .contains(&position)
                .then_some((line, f32::from(rect.size.width)))
        })
    }

    /// The window-space bounds of an image's corner grip, given the image's
    /// painted `rect`. A small square overhanging the bottom-right corner (its
    /// center on the corner, like the reading view's), so it's easy to grab
    /// without covering much of the image.
    fn image_grip(rect: Bounds<Pixels>) -> Bounds<Pixels> {
        let s = px(IMG_GRIP);
        Bounds::new(
            point(rect.right() - s / 2., rect.bottom() - s / 2.),
            size(s, s),
        )
    }

    /// If `position` lands on a task checkbox painted last frame, the logical line
    /// of that task — so a click can toggle it. The hit area is the box padded a
    /// little, to stay easy to tap without swallowing the body text beside it.
    fn checkbox_at(&self, position: Point<Pixels>) -> Option<usize> {
        let pad = px(4.);
        self.checkbox_rects.iter().find_map(|&(line, rect)| {
            Bounds::new(
                point(rect.origin.x - pad, rect.origin.y - pad),
                size(rect.size.width + pad * 2., rect.size.height + pad * 2.),
            )
            .contains(&position)
            .then_some(line)
        })
    }

    /// The table-affordance region the pointer is in — `(table index, 0 = in the
    /// hover zone / 1 = on the below "+" strip / 2 = on the right "+" strip)`, or
    /// `None` off every table. Drives `on_mouse_move`'s repaint-on-change.
    fn table_hover_region_at(&self, pos: Point<Pixels>) -> Option<(usize, u8)> {
        let i = self
            .table_hover_zones
            .iter()
            .position(|z| z.contains(&pos))?;
        let strip = if self
            .table_row_add_rects
            .iter()
            .any(|(b, _)| b.contains(&pos))
        {
            1
        } else if self
            .table_col_add_rects
            .iter()
            .any(|(b, _)| b.contains(&pos))
        {
            2
        } else {
            0
        };
        Some((i, strip))
    }

    /// The table cell `(row, col)` the pointer is over, or `None` off any table —
    /// drives the delete-handle repaint + reveal.
    fn hovered_table_cell(&self, pos: Point<Pixels>) -> Option<(usize, usize)> {
        let bounds = self.last_bounds.as_ref()?;
        // Hover bands start at the left gutter and extend a header's band up into the
        // top gutter, so moving onto a delete handle keeps its cell "hovered".
        let gutter_left = bounds.left();
        if pos.x < gutter_left {
            return None;
        }
        let rel_y = pos.y - bounds.top();
        let g = px(TABLE_GUTTER);
        let row = (0..self.wrapped.len()).find(|&i| {
            let Some(t) = self.table_rows.get(i).and_then(Option::as_ref) else {
                return false;
            };
            if t.is_separator {
                return false;
            }
            let h = self.line_h(i) * (self.wrapped[i].wrap_boundaries().len() + 1) as f32;
            let lo = if t.is_header {
                self.line_tops[i] - g
            } else {
                self.line_tops[i]
            };
            rel_y >= lo && rel_y < self.line_tops[i] + h
        })?;
        let t = self.table_rows.get(row).and_then(Option::as_ref)?;
        if t.col_widths.is_empty() {
            return None;
        }
        let table_left = gutter_left + g;
        let table_w: Pixels = t.col_widths.iter().copied().sum();
        if pos.x >= table_left + table_w {
            return None;
        }
        let rel_x = (pos.x - table_left).max(px(0.));
        let mut colx = px(0.);
        for (col, &cw) in t.col_widths.iter().enumerate() {
            if rel_x < colx + cw {
                return Some((row, col));
            }
            colx += cw;
        }
        Some((row, t.col_widths.len() - 1))
    }

    /// Hit-test the table add-row "+" strips → the row a new row lands after.
    fn table_add_row_at(&self, position: Point<Pixels>) -> Option<usize> {
        self.table_row_add_rects
            .iter()
            .find_map(|&(rect, row)| rect.contains(&position).then_some(row))
    }

    /// Hit-test the table add-column "+" strips → a row of that table (to seat the
    /// caret in its last cell).
    fn table_add_col_at(&self, position: Point<Pixels>) -> Option<usize> {
        self.table_col_add_rects
            .iter()
            .find_map(|&(rect, row)| rect.contains(&position).then_some(row))
    }

    /// Source offset at the start of `cell`'s content in table `row` (last paint).
    fn cell_start_offset(&self, row: usize, cell: usize) -> Option<usize> {
        let t = self.table_rows.get(row).and_then(Option::as_ref)?;
        Some(self.line_starts()[row] + t.cell_ranges.get(cell)?.start)
    }

    /// Source offset at the start of the last cell's content in table `row`.
    fn last_cell_start_offset(&self, row: usize) -> Option<usize> {
        let t = self.table_rows.get(row).and_then(Option::as_ref)?;
        let last = t.cell_ranges.len().checked_sub(1)?;
        Some(self.line_starts()[row] + t.cell_ranges.get(last)?.start)
    }

    /// If `position` lands on a table grid row (not the separator), the source
    /// byte offset of the closest cell-content position — so a click puts the
    /// caret inside the cell rather than in the raw `| … |` source. `None`
    /// otherwise (the caller falls back to [`Self::index_for_mouse_position`]).
    fn table_offset_at(&self, position: Point<Pixels>, window: &mut Window) -> Option<usize> {
        if self.wrapped.is_empty() || self.table_rows.iter().all(Option::is_none) {
            return None;
        }
        let bounds = self.last_bounds.as_ref()?;
        let rel = point(
            position.x - bounds.left() - px(TABLE_GUTTER),
            position.y - bounds.top(),
        );
        let mut row = self.wrapped.len() - 1;
        for i in 0..self.wrapped.len() {
            let h = self.line_h(i) * (self.wrapped[i].wrap_boundaries().len() + 1) as f32;
            if rel.y < self.line_tops[i] + h {
                row = i;
                break;
            }
        }
        let t = self.table_rows.get(row).and_then(Option::as_ref)?;
        if t.is_separator || t.col_widths.is_empty() {
            return None;
        }
        let style = window.text_style();
        let font = style.font();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let pad = px(TABLE_CELL_PAD);
        // Column the click is in, and its left x.
        let last = t.col_widths.len() - 1;
        let mut cx = px(0.);
        let mut cell = 0;
        for (c, &cw) in t.col_widths.iter().enumerate() {
            if rel.x < cx + cw || c == last {
                cell = c;
                break;
            }
            cx += cw;
        }
        let content = t.cells.get(cell)?;
        let cw = t.col_widths[cell];
        let cf = cell_font(&font, t.is_header);
        let full_w = measure_width(window, content, &cf, font_size);
        let avail = (cw - pad * 2.).max(px(0.));
        let align_off = match t.aligns.get(cell) {
            Some(markdown_syntax::Align::Center) => (avail - full_w).max(px(0.)) / 2.,
            Some(markdown_syntax::Align::Right) => (avail - full_w).max(px(0.)),
            _ => px(0.),
        };
        let target = (rel.x - cx - pad - align_off).max(px(0.));
        let in_cell = cell_offset_for_x(content, target, &cf, font_size, window);
        Some(self.line_starts()[row] + t.cell_ranges.get(cell)?.start + in_cell)
    }

    /// Whether the caret is currently inside an editable table cell (not the
    /// separator) — so Tab navigates cells instead of indenting.
    fn caret_in_table(&self) -> bool {
        let (row, _) = self.row_col(self.cursor_offset());
        self.table_rows
            .get(row)
            .and_then(Option::as_ref)
            .is_some_and(|t| !t.is_separator)
    }

    /// Cell-aware vertical caret move inside a table: keep the same column (cell +
    /// the offset within that cell) on the adjacent row, skipping the `|---|`
    /// separator. `None` when the caret isn't in a table cell, or the move would
    /// leave the table — the caller then does a normal vertical move (exiting it).
    fn table_move_vertical(&self, dir: i32) -> Option<usize> {
        let (row, col) = self.row_col(self.cursor_offset());
        let t = self.table_rows.get(row).and_then(Option::as_ref)?;
        if t.is_separator || t.cell_ranges.is_empty() {
            return None;
        }
        let cell = table_cell_at(t, col);
        let intra = col.saturating_sub(t.cell_ranges[cell].start);
        let starts = self.line_starts();
        let mut r = row as isize + dir as isize;
        loop {
            if r < 0 {
                return None;
            }
            let ru = r as usize;
            match self.table_rows.get(ru) {
                Some(Some(nt)) if !nt.is_separator && !nt.cell_ranges.is_empty() => {
                    let tc = cell.min(nt.cell_ranges.len() - 1);
                    let cr = &nt.cell_ranges[tc];
                    return Some(starts[ru] + cr.start + intra.min(cr.end - cr.start));
                }
                Some(Some(_)) => r += dir as isize, // separator — skip past it
                // A non-table row next to the table: exit onto it at the same byte
                // column (clamped to a char boundary). Done here rather than via
                // `move_vertical`, whose handling of the table's top gutter would
                // otherwise trap an upward exit back onto the header row.
                Some(None) => {
                    let end = self.line_end(ru);
                    // Skip the table's own `<!-- table:STYLE -->` style-marker line
                    // (a hidden directive) so an upward exit lands on real content,
                    // the way a downward move already skips its zero-height row.
                    if markdown_syntax::table_style_marker(&self.content[starts[ru]..end]).is_some()
                    {
                        r += dir as isize;
                        continue;
                    }
                    let mut target = starts[ru] + col.min(end - starts[ru]);
                    while !self.content.is_char_boundary(target) {
                        target -= 1;
                    }
                    return Some(target);
                }
                None => return None, // past the document edge — let move_vertical exit
            }
        }
    }

    /// Cell-aware horizontal caret move inside a table: step a character within the
    /// cell, hopping to the adjacent cell (the next/previous row's edge cell at a
    /// row boundary) so the caret never has to cross the hidden `|`/padding.
    /// `None` when the caret isn't in a table cell or the move would leave it.
    fn table_move_horizontal(&self, dir: i32) -> Option<usize> {
        let (row, col) = self.row_col(self.cursor_offset());
        let t = self.table_rows.get(row).and_then(Option::as_ref)?;
        if t.is_separator || t.cell_ranges.is_empty() {
            return None;
        }
        let cell = table_cell_at(t, col);
        let starts = self.line_starts();
        let cur = self.cursor_offset();
        let cell_start = starts[row] + t.cell_ranges[cell].start;
        let cell_end = starts[row] + t.cell_ranges[cell].end;
        if dir > 0 {
            if cur < cell_end {
                return Some(self.next_boundary(cur).min(cell_end));
            }
            if cell + 1 < t.cell_ranges.len() {
                return Some(starts[row] + t.cell_ranges[cell + 1].start);
            }
            // Last cell of the row → first cell of the next table row, else exit.
            for (r, slot) in self.table_rows.iter().enumerate().skip(row + 1) {
                match slot.as_ref() {
                    Some(nt) if !nt.is_separator && !nt.cell_ranges.is_empty() => {
                        return Some(starts[r] + nt.cell_ranges[0].start);
                    }
                    Some(_) => continue,
                    None => break,
                }
            }
            None
        } else {
            if cur > cell_start {
                return Some(self.previous_boundary(cur).max(cell_start));
            }
            if cell > 0 {
                return Some(starts[row] + t.cell_ranges[cell - 1].end);
            }
            // First cell of the row → last cell of the previous table row, else exit.
            for (r, slot) in self.table_rows.iter().enumerate().take(row).rev() {
                match slot.as_ref() {
                    Some(pt) if !pt.is_separator && !pt.cell_ranges.is_empty() => {
                        return Some(starts[r] + pt.cell_ranges[pt.cell_ranges.len() - 1].end);
                    }
                    Some(_) => continue,
                    None => break,
                }
            }
            None
        }
    }

    /// Target source offset to move the caret to the next (`forward`) / previous
    /// table cell, crossing rows (skipping the separator). Stays put at the table's
    /// final/first cell. `None` when the caret isn't in a table cell.
    fn table_cell_nav(&self, forward: bool) -> Option<usize> {
        let (row, col) = self.row_col(self.cursor_offset());
        let t = self.table_rows.get(row).and_then(Option::as_ref)?;
        if t.is_separator {
            return None;
        }
        let cell = table_cell_at(t, col);
        let starts = self.line_starts();
        let usable = |tr: &TableRow| !tr.is_separator && !tr.cell_ranges.is_empty();
        if forward {
            if cell + 1 < t.cell_ranges.len() {
                return Some(starts[row] + t.cell_ranges[cell + 1].start);
            }
            for (r, slot) in self.table_rows.iter().enumerate().skip(row + 1) {
                match slot.as_ref() {
                    Some(nt) if usable(nt) => return Some(starts[r] + nt.cell_ranges[0].start),
                    Some(_) => continue,
                    None => break,
                }
            }
        } else {
            if cell > 0 {
                return Some(starts[row] + t.cell_ranges[cell - 1].start);
            }
            for (r, slot) in self.table_rows.iter().enumerate().take(row).rev() {
                match slot.as_ref() {
                    Some(pt) if usable(pt) => {
                        return Some(starts[r] + pt.cell_ranges[pt.cell_ranges.len() - 1].start);
                    }
                    Some(_) => continue,
                    None => break,
                }
            }
        }
        Some(self.cursor_offset()) // at the boundary — no-op move (don't indent)
    }

    /// The alignment of the table column the caret sits in — but only while the
    /// caret is in the table's HEADER row (the toolbar lives there; alignment is a
    /// per-column property, set once from the header). `None` otherwise. Read from
    /// the current content, since the painted `table_rows` lag a frame right after
    /// a separator rewrite (which would highlight the just-changed-from button).
    pub fn caret_table_align(&self) -> Option<CellAlign> {
        let (row, col) = self.row_col(self.cursor_offset());
        // Fast-reject via the paint: only a header row gets the toolbar.
        let t = self.table_rows.get(row).and_then(Option::as_ref)?;
        if !t.is_header {
            return None;
        }
        let cell = table_cell_at(t, col);
        let regions = markdown_syntax::table_regions(&self.content);
        let region = regions.iter().find(|r| r.lines.contains(&row))?;
        Some(match region.aligns.get(cell) {
            Some(markdown_syntax::Align::Center) => CellAlign::Center,
            Some(markdown_syntax::Align::Right) => CellAlign::Right,
            _ => CellAlign::Left,
        })
    }

    /// Set the alignment of the caret's table column by rewriting that table's
    /// `|---|` separator row; the caret stays put. No-op outside a table cell.
    pub fn set_caret_table_align(&mut self, align: CellAlign, cx: &mut Context<Self>) {
        let (row, col) = self.row_col(self.cursor_offset());
        let Some(t) = self.table_rows.get(row).and_then(Option::as_ref) else {
            return;
        };
        if t.is_separator {
            return;
        }
        let cell = table_cell_at(t, col);
        // Read the table's columns from the current content (fresh), so repeated
        // clicks build on the latest alignment, not a stale painted snapshot.
        let regions = markdown_syntax::table_regions(&self.content);
        let Some(region) = regions.iter().find(|r| r.lines.contains(&row)) else {
            return;
        };
        let mut aligns = region.aligns.clone();
        if cell >= aligns.len() {
            return;
        }
        aligns[cell] = match align {
            CellAlign::Left => markdown_syntax::Align::Left,
            CellAlign::Center => markdown_syntax::Align::Center,
            CellAlign::Right => markdown_syntax::Align::Right,
        };
        let sep_row = region.lines.start + 1;
        let mut new_sep = String::from("|");
        for a in &aligns {
            new_sep.push_str(match a {
                markdown_syntax::Align::Left => " :-- |",
                markdown_syntax::Align::Center => " :-: |",
                markdown_syntax::Align::Right => " --: |",
            });
        }
        let starts = self.line_starts();
        let sep_start = starts[sep_row];
        let sep_end = starts
            .get(sep_row + 1)
            .map_or(self.content.len(), |&s| s - 1);
        let old_caret = self.cursor_offset();
        let range = sep_start..sep_end;
        self.record_edit(&range, &new_sep);
        self.content = self.content[..sep_start].to_owned() + &new_sep + &self.content[sep_end..];
        let delta = new_sep.len() as isize - (sep_end - sep_start) as isize;
        let caret = if old_caret >= sep_end {
            (old_caret as isize + delta).max(0) as usize
        } else {
            old_caret
        };
        self.selected_range = caret..caret;
        self.remap_diagnostics(&range, new_sep.len());
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    /// The caret's table block as `(header_line, separator_line, end_exclusive,
    /// columns)`, or `None` outside a table.
    fn caret_table_block(&self) -> Option<(usize, usize, usize, usize)> {
        let (row, _) = self.row_col(self.cursor_offset());
        let region = markdown_syntax::table_regions(&self.content)
            .into_iter()
            .find(|r| r.lines.contains(&row))?;
        Some((
            region.lines.start,
            region.lines.start + 1,
            region.lines.end,
            region.aligns.len().max(1),
        ))
    }

    /// Insert an empty row above/below the caret's row (Word-style); the caret
    /// moves into the new row's first cell. No-op outside a table.
    pub fn insert_table_row(&mut self, below: bool, cx: &mut Context<Self>) {
        let (row, _) = self.row_col(self.cursor_offset());
        let Some((header, sep, _end, cols)) = self.caret_table_block() else {
            return;
        };
        // From the header a new row always lands below the separator (the first
        // body row); above/below a body row is literal.
        let after = if row == header {
            sep
        } else if below {
            row
        } else {
            (row - 1).max(sep)
        };
        let new_row = format!("\n|{}", "  |".repeat(cols));
        let pos = self.line_end(after);
        let range = pos..pos;
        self.record_edit(&range, &new_row);
        self.content = self.content[..pos].to_owned() + &new_row + &self.content[pos..];
        self.remap_diagnostics(&range, new_row.len());
        self.selected_range = (pos + 3)..(pos + 3); // first cell, after "\n| "
        self.table_menu = None;
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    /// Delete the caret's table row (body rows only — the header + separator stay).
    /// The caret keeps its cell + in-cell offset, landing on the row that takes the
    /// deleted row's place. No-op outside a table.
    pub fn delete_table_row(&mut self, cx: &mut Context<Self>) {
        let Some((row, cell, in_cell)) = self.caret_table_cell_pos() else {
            return;
        };
        let Some((header, sep, end, _cols)) = self.caret_table_block() else {
            return;
        };
        if row == header || row == sep {
            return;
        }
        let start = self.line_starts()[row];
        let line_end = self.line_end(row);
        // Remove the line + its trailing newline; for the last line, eat the
        // preceding newline instead so no blank line is left behind.
        let (del_start, del_end) = if line_end < self.content.len() {
            (start, line_end + 1)
        } else {
            (start.saturating_sub(1), line_end)
        };
        let range = del_start..del_end;
        self.record_edit(&range, "");
        self.content = self.content[..del_start].to_owned() + &self.content[del_end..];
        self.remap_diagnostics(&range, 0);
        // Stay at the same cell/offset, on the row now at this position (shifted
        // up), or the header if no body rows remain.
        let target = if end <= sep + 2 {
            header
        } else {
            row.min(end - 2)
        };
        let caret = self.caret_pos_for_cell(target, cell, in_cell);
        self.selected_range = caret..caret;
        self.table_menu = None;
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    /// Delete the whole table the caret is in — its grid lines plus an optional
    /// `<!-- table:STYLE -->` marker line directly above — joining the surrounding
    /// text. The caret lands where the table was.
    pub fn delete_table(&mut self, cx: &mut Context<Self>) {
        let Some((header, _sep, end, _cols)) = self.caret_table_block() else {
            return;
        };
        let starts = self.line_starts();
        let mut first = header;
        if first > 0
            && markdown_syntax::table_style_marker(
                &self.content[starts[first - 1]..starts[first] - 1],
            )
            .is_some()
        {
            first -= 1;
        }
        let line_end_last = self.line_end(end - 1);
        // Remove the table's lines + the trailing newline; at the document end, eat
        // the preceding newline instead so no blank line is left behind.
        let (del_start, del_end) = if line_end_last < self.content.len() {
            (starts[first], line_end_last + 1)
        } else {
            (starts[first].saturating_sub(1), line_end_last)
        };
        let range = del_start..del_end;
        self.record_edit(&range, "");
        self.content = self.content[..del_start].to_owned() + &self.content[del_end..];
        self.remap_diagnostics(&range, 0);
        let caret = del_start.min(self.content.len());
        self.selected_range = caret..caret;
        self.selection_reversed = false;
        self.goal_x = None;
        self.table_menu = None;
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    /// The caret's table position as `(row, cell_index, offset_within_cell)`, or
    /// `None` outside a table. Lets structural edits keep the caret put.
    fn caret_table_cell_pos(&self) -> Option<(usize, usize, usize)> {
        let (row, _) = self.row_col(self.cursor_offset());
        self.caret_table_block()?;
        let starts = self.line_starts();
        let row_start = starts[row];
        let line = &self.content[row_start..self.line_end(row)];
        let line_col = self.cursor_offset() - row_start;
        let ranges = markdown_syntax::table_cell_ranges(line);
        let cell = ranges
            .iter()
            .position(|r| line_col <= r.end)
            .unwrap_or(ranges.len().saturating_sub(1));
        let in_cell = ranges
            .get(cell)
            .map_or(0, |r| line_col.saturating_sub(r.start).min(r.len()));
        Some((row, cell, in_cell))
    }

    /// Byte offset of `(row, cell, offset_within_cell)` in the current content,
    /// clamping the cell + offset to what that row actually has.
    fn caret_pos_for_cell(&self, row: usize, cell: usize, in_cell: usize) -> usize {
        let starts = self.line_starts();
        let Some(&row_start) = starts.get(row) else {
            return self.content.len();
        };
        let line = &self.content[row_start..self.line_end(row)];
        let ranges = markdown_syntax::table_cell_ranges(line);
        if ranges.is_empty() {
            return row_start;
        }
        let r = &ranges[cell.min(ranges.len() - 1)];
        // Keep the caret strictly inside the cell, before its closing pipe — an
        // empty cell's trimmed range collapses onto that pipe (the line end for the
        // last cell), which would drop the caret out of the rendered table.
        let bytes = line.as_bytes();
        let close = (r.end..bytes.len())
            .find(|&i| bytes[i] == b'|')
            .unwrap_or(bytes.len());
        row_start + (r.start + in_cell).min(close.saturating_sub(1))
    }

    /// Insert an empty column left/right of the caret's column (a cell added to
    /// every row; the separator gets a default-left marker). The caret stays in its
    /// cell. No-op outside a table.
    pub fn insert_table_column(&mut self, right: bool, cx: &mut Context<Self>) {
        let Some((row, cell, in_cell)) = self.caret_table_cell_pos() else {
            return;
        };
        let at = if right { cell + 1 } else { cell };
        if self.rewrite_table_columns(ColEdit::Insert(at)) {
            // Inserting to the left shifts the caret's cell one column right.
            let new_cell = if right { cell } else { cell + 1 };
            let caret = self.caret_pos_for_cell(row, new_cell, in_cell);
            self.selected_range = caret..caret;
            self.table_menu = None;
            cx.emit(EditorEvent::Changed);
            cx.notify();
        }
    }

    /// Delete the caret's column from every row; the caret stays near where the
    /// column was. No-op outside a table, or on the last remaining column.
    pub fn delete_table_column(&mut self, cx: &mut Context<Self>) {
        let Some((row, cell, in_cell)) = self.caret_table_cell_pos() else {
            return;
        };
        if self.rewrite_table_columns(ColEdit::Delete(cell)) {
            let caret = self.caret_pos_for_cell(row, cell, in_cell);
            self.selected_range = caret..caret;
            self.table_menu = None;
            cx.emit(EditorEvent::Changed);
            cx.notify();
        }
    }

    /// Rewrite every row of the caret's table to insert/delete a cell, normalizing
    /// cell spacing. Returns `false` (no edit) outside a table or when a delete
    /// would remove the last column; the caller restores the caret.
    fn rewrite_table_columns(&mut self, edit: ColEdit) -> bool {
        let Some((header, sep, end, _cols)) = self.caret_table_block() else {
            return false;
        };
        let lines: Vec<&str> = self.content.split('\n').collect();
        let mut new_rows: Vec<String> = Vec::with_capacity(end - header);
        for (i, &line) in lines[header..end].iter().enumerate() {
            let is_sep = header + i == sep;
            let mut cells: Vec<String> = markdown_syntax::table_cells(line)
                .iter()
                .map(|s| (*s).to_string())
                .collect();
            match edit {
                ColEdit::Insert(at) => cells.insert(
                    at.min(cells.len()),
                    if is_sep { "---".into() } else { String::new() },
                ),
                ColEdit::Delete(c) => {
                    if cells.len() <= 1 || c >= cells.len() {
                        return false; // never delete the last column
                    }
                    cells.remove(c);
                }
            }
            new_rows.push(format!("| {} |", cells.join(" | ")));
        }
        let starts = self.line_starts();
        let block_start = starts[header];
        let block_end = self.line_end(end - 1);
        let new_block = new_rows.join("\n");
        let range = block_start..block_end;
        self.record_edit(&range, &new_block);
        self.content =
            self.content[..block_start].to_owned() + &new_block + &self.content[block_end..];
        self.remap_diagnostics(&range, new_block.len());
        true
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
            // While a `$$` block OR an inline `$…$` formula is being edited, the hosted math
            // editor is focused but lives *inside* this element — so the editor's own
            // keybindings (arrows, typing, …) would capture keys before they reach it. Drop the
            // key context for the duration so raw keys flow to the math editor's on_key_down.
            .key_context(
                if self.editing_block.is_some() || self.editing_inline.is_some() {
                    ""
                } else {
                    CONTEXT
                },
            )
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
            .on_action(cx.listener(Self::bold))
            .on_action(cx.listener(Self::italic))
            .on_action(cx.listener(Self::code))
            .on_action(cx.listener(Self::indent))
            .on_action(cx.listener(Self::outdent))
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
            .children(self.editing_block_overlay())
            .children(self.editing_inline_overlay())
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
                // Menu chrome from the host's theme (fallbacks match the former
                // hardcoded dark menu when no markdown style is set).
                let st = self.markdown_style.as_ref();
                let menu_bg = st.map_or(rgb(0x26262b).into(), |s| s.popover_bg);
                let menu_border = st.map_or(rgb(0x45454c).into(), |s| s.popover_border);
                let menu_fg = st.map_or(rgb(0xe6e6e6).into(), |s| s.popover_fg);
                let hover = st.map_or(rgba(0x2f6fd628).into(), |s| s.popover_hover);
                let mut thumb_c = st.map_or(rgba(0xffffff66).into(), |s| s.marker);
                thumb_c.a = 0.5;
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
                            .hover(move |s| s.bg(hover))
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
                        .bg(thumb_c)
                });

                // Deferred + anchored to a window-space top layer with `.occlude()`,
                // so it renders above the page chrome and captures the wheel — else a
                // scroll over the popup scrolls the page behind it.
                gpui::deferred(
                    gpui::anchored().position(anchor).snap_to_window().child(
                        div()
                            .relative()
                            .occlude()
                            .min_w(px(150.))
                            // Override the editor's I-beam — the menu is a normal
                            // pointer surface (children inherit this hitbox's cursor).
                            .cursor(CursorStyle::Arrow)
                            .bg(menu_bg)
                            .border_1()
                            .border_color(menu_border)
                            .rounded(px(6.))
                            // Clip rows + thumb to the rounded box.
                            .overflow_hidden()
                            .text_color(menu_fg)
                            .text_size(px(14.))
                            // A click anywhere outside the menu dismisses it.
                            .on_mouse_down_out(cx.listener(|editor, _: &MouseDownEvent, _, cx| {
                                editor.menu = None;
                                cx.notify();
                            }))
                            .child(
                                // The scroll viewport: shows ~6 rows, the rest scroll.
                                div()
                                    .id("suggestion-menu")
                                    .max_h(px(MAX_H))
                                    .overflow_y_scroll()
                                    .track_scroll(&scroll)
                                    .flex()
                                    .flex_col()
                                    .py(px(PAD))
                                    .children(rows),
                            )
                            .children(thumb),
                    ),
                )
            }))
            // The table right-click menu (Word-style row/column editing), anchored
            // at the click; each row runs its action on the caret's table cell.
            .children(self.table_menu.map(|anchor| {
                // Menu chrome from the host's theme (fallbacks match the former
                // hardcoded dark menu when no markdown style is set).
                let st = self.markdown_style.as_ref();
                let menu_bg = st.map_or(rgb(0x26262b).into(), |s| s.popover_bg);
                let menu_border = st.map_or(rgb(0x45454c).into(), |s| s.popover_border);
                let menu_fg = st.map_or(rgb(0xe6e6e6).into(), |s| s.popover_fg);
                let hover = st.map_or(rgba(0x2f6fd628).into(), |s| s.popover_hover);
                let divider = st.map_or(rgba(0xffffff2e).into(), |s| s.popover_divider);
                let mut thumb_c = st.map_or(rgba(0xffffff66).into(), |s| s.marker);
                thumb_c.a = 0.5;
                const ROW_H: f32 = 28.0;
                const DIV_H: f32 = 9.0;
                const PAD: f32 = 4.0;
                const MAX_H: f32 = 240.0;
                // Rows in three groups (insert / delete / align) with a divider before
                // the delete group (index 4) and the align group (index 6).
                let mut rows: Vec<gpui::AnyElement> = Vec::new();
                for (i, &(label, action)) in TableMenuAction::ITEMS.iter().enumerate() {
                    if i == 4 || i == 6 || i == 9 {
                        rows.push(
                            div()
                                .flex_shrink_0()
                                .h(px(1.))
                                .my(px(4.))
                                .mx(px(8.))
                                .bg(divider)
                                .into_any_element(),
                        );
                    }
                    rows.push(
                        div()
                            .id(("table-menu-row", i))
                            .flex_shrink_0()
                            .px(px(12.))
                            .py(px(5.))
                            .hover(move |s| s.bg(hover))
                            .child(SharedString::from(label))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |editor, _: &MouseDownEvent, _, cx| {
                                    cx.stop_propagation();
                                    action.apply(editor, cx);
                                }),
                            )
                            .into_any_element(),
                    );
                }
                // Scrollbar thumb, shown when the items overflow the cap — sized from
                // the content height + positioned from the live scroll offset.
                let rows_h = TableMenuAction::ITEMS.len() as f32 * ROW_H + 3.0 * DIV_H;
                let view_h = MAX_H - 2.0 * PAD;
                let thumb = (rows_h > view_h).then(|| {
                    let scrolled =
                        (-f32::from(self.table_menu_scroll.offset().y)).clamp(0.0, rows_h - view_h);
                    let thumb_h = (view_h * view_h / rows_h).max(24.0);
                    let thumb_top = PAD + scrolled / (rows_h - view_h) * (view_h - thumb_h);
                    div()
                        .absolute()
                        .top(px(thumb_top))
                        .right(px(2.))
                        .w(px(6.))
                        .h(px(thumb_h))
                        .rounded(px(3.))
                        .bg(thumb_c)
                });
                gpui::deferred(
                    gpui::anchored().position(anchor).snap_to_window().child(
                        div()
                            .relative()
                            .occlude()
                            .min_w(px(170.))
                            .cursor(CursorStyle::Arrow)
                            .bg(menu_bg)
                            .border_1()
                            .border_color(menu_border)
                            .rounded(px(6.))
                            .overflow_hidden()
                            .text_color(menu_fg)
                            .text_size(px(14.))
                            .on_mouse_down_out(cx.listener(|editor, _: &MouseDownEvent, _, cx| {
                                editor.table_menu = None;
                                cx.notify();
                            }))
                            .child(
                                // Inner scroll viewport: caps the height + scrolls the
                                // overflow (max_h on a separate flex-col div, like the
                                // suggestion menu — combining it with the styled box
                                // above doesn't cap).
                                div()
                                    .id("table-menu")
                                    .max_h(px(MAX_H))
                                    .overflow_y_scroll()
                                    .track_scroll(&self.table_menu_scroll)
                                    .flex()
                                    .flex_col()
                                    .py(px(PAD))
                                    .children(rows),
                            )
                            .children(thumb),
                    ),
                )
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

/// The shaped width of `text` at `font_size` — used to inset a gutter line's body
/// to exactly where its (hidden) source prefix ends, so the rendered + raw views
/// line up (and tab/space nesting matches the actual whitespace width).
fn measure_width(window: &mut Window, text: &str, font: &Font, font_size: Pixels) -> Pixels {
    if text.is_empty() {
        return px(0.);
    }
    let run = TextRun {
        len: text.len(),
        font: font.clone(),
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(
            SharedString::from(text.to_string()),
            font_size,
            &[run],
            None,
        )
        .width()
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
    /// Whether to show a corner resize grip. `false` for math (nothing to persist a
    /// `{width=N}` to, and it renders at its natural typeset size); `true` for images.
    resizable: bool,
    /// Horizontal alignment in the content width. `Left` for images; display math sets its
    /// own (centered by default).
    align: MathAlign,
}

/// One inline `$…$` formula painted within a text line. `display_off` is the byte offset of its
/// invisible spacer in the shaped DISPLAY string (resolved to an x via the wrapped line at
/// paint); `source` is the formula's byte range within the *source line* (to hit-test a click
/// back to its edit range); `img`/`width`/`height` are the typeset raster scaled to text size.
#[derive(Clone)]
struct InlineMath {
    display_off: usize,
    /// ABSOLUTE byte range of the `$…$` span in the document — to hit-test a click on the
    /// formula back to its edit range and to position the seated editor.
    source: Range<usize>,
    /// The inner LaTeX (no `$` delimiters), to seed the structural editor on click.
    latex: SharedString,
    img: Arc<RenderImage>,
    width: Pixels,
    height: Pixels,
}

/// A line rendered as a block widget instead of its source text: a standalone
/// image, or a clickable file chip (e.g. a PDF — left-click opens it, right-click
/// edits). Shown only while the caret is off the line ("raw on caret").
#[derive(Clone)]
enum Block {
    Image(BlockImg),
    Chip {
        src: SharedString,
        label: SharedString,
        /// Label color (accent, signalling clickable), box fill, box border.
        link: Hsla,
        bg: Hsla,
        border: Hsla,
        height: Pixels,
    },
}

impl Block {
    fn height(&self) -> Pixels {
        match self {
            Block::Image(i) => i.height,
            Block::Chip { height, .. } => *height,
        }
    }
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
    /// Byte range of each cell's trimmed content within its source line — for
    /// placing the caret inside a cell + hit-testing a click back to a source
    /// offset (in-cell editing).
    cell_ranges: Vec<Range<usize>>,
    aligns: Vec<markdown_syntax::Align>,
    col_widths: Vec<Pixels>,
    is_header: bool,
    is_separator: bool,
    is_last: bool,
    /// 0-based position among the body rows (`None` for header/separator) — drives
    /// striping (shade odd indices) + the rule-under-header (index 0).
    body_index: Option<usize>,
    /// The table's visual style (from its `<!-- table:STYLE -->` marker).
    style: markdown_syntax::TableStyle,
    border: Hsla,
    /// Row-shade color for striped / header-shaded styles (a faint tint).
    shade: Hsla,
}

/// A per-line "gutter" decoration: a left-margin treatment that hides its source
/// marker and renders something in its place, with the body text inset to make
/// room. Covers blockquotes now; list bullets + task checkboxes reuse it.
#[derive(Clone, Copy)]
enum LineMark {
    /// Blockquote: a muted left border; the `>` markers are hidden and the body
    /// text is muted (`SyntaxStyle::quote`).
    Quote(Hsla),
    /// List item: a painted bullet (`•`) or number (`N.`) at `bullet_x` (where the
    /// hidden source marker began), muted; the body sits at `text_inset` — the
    /// measured width of the whole source prefix, so the rendered + raw views
    /// line up exactly and tab/space nesting stays in sync.
    List {
        bullet_x: Pixels,
        text_inset: Pixels,
        ordered: bool,
        num: u32,
        color: Hsla,
    },
    /// GFM task item: a painted ☐/☑ box at `bullet_x`, muted; the body sits at
    /// `text_inset` (measured prefix width) like a list item.
    Check {
        bullet_x: Pixels,
        text_inset: Pixels,
        checked: bool,
        color: Hsla,
    },
    /// Thematic break (`---`): a full-width muted divider painted in place of the
    /// source; the line has no body text (reveal-on-caret shows the raw `---`).
    Rule(Hsla),
}

impl LineMark {
    /// Horizontal inset (px) applied to the body text + caret for this mark.
    fn inset(self) -> Pixels {
        match self {
            LineMark::Quote(_) => px(QUOTE_INSET),
            LineMark::List { text_inset, .. } | LineMark::Check { text_inset, .. } => text_inset,
            LineMark::Rule(_) => px(0.),
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
    Vec<Option<Block>>,
    Vec<Option<CodeBg>>,
    Vec<Option<TableRow>>,
    // Per-line display→source byte map for lines with markers hidden (W6); `None`
    // when the displayed text equals the source (revealed / code / widget lines).
    Vec<Option<Vec<usize>>>,
    Vec<Option<LineMark>>,
    // Per-line inline `$…$` formulas painted over spacers in the shaped text (empty for
    // lines without any).
    Vec<Vec<InlineMath>>,
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
        resizable: true,
        align: MathAlign::Left, // images stay left; math overrides with its marker
    })
}

/// An inline image's painted size: the saved `BlockImg` size, unless its grip is
/// being dragged (`resize.line == line`), in which case the live drag width wins
/// and the height scales with it (aspect preserved). Used by both the prepaint
/// (grip hitbox) and the paint (image + grip), so the preview stays consistent.
fn image_display_size(w: &BlockImg, resize: Option<ImageResize>, line: usize) -> (Pixels, Pixels) {
    match resize {
        Some(r) if r.line == line => (px(r.width), w.height * (r.width / f32::from(w.width))),
        _ => (w.width, w.height),
    }
}

/// Rewrite an image source `line` to carry an explicit `{width=N}` after the
/// `![alt](src)` (replacing any existing `{width=...}`), preserving a leading
/// list marker and any trailing whitespace. Used to persist a corner-grip resize
/// back into the document. Returns `line` unchanged if it isn't an image row.
fn set_image_width(line: &str, width: u32) -> String {
    let Some((_, _, marker_len)) = markdown_syntax::image_row(line) else {
        return line.to_string();
    };
    // Split off any trailing whitespace so the attr lands right after `)` (or the
    // existing `{width=…}`), with the original trailing run re-appended.
    let trimmed_end = line.trim_end_matches([' ', '\t']);
    let trailing_ws = &line[trimmed_end.len()..];
    // The image body always ends at the first `)` after the list marker; an
    // existing `{width=…}` (only valid right after it) is dropped.
    let close = marker_len + line[marker_len..].find(')').map_or(0, |i| i + 1);
    let body = trimmed_end[..close.min(trimmed_end.len())].trim_end();
    format!("{body}{{width={width}}}{trailing_ws}")
}

/// Invert a display→source offset map: the display column for `source_col`. The
/// map is ascending, so a source column that is hidden (a collapsed marker)
/// snaps to the next visible display column. `None` map → identity (a row shown
/// as full source). The prepaint cursor/selection pass this frame's fresh map
/// (the committed `EditorState::offset_maps` lags a frame); event handlers go
/// through [`EditorState::display_col`], which uses the committed map.
fn display_col_in(map: Option<&Vec<usize>>, source_col: usize) -> usize {
    match map {
        // The first display byte whose source ≥ `source_col` (a leftmost lower-bound). Unlike
        // `binary_search`, this is deterministic when several display bytes share one source
        // offset — an inline `$…$` spacer maps its whole width to the span start, so the caret
        // just before the formula must land at the spacer's LEFT edge, not somewhere inside it.
        Some(m) => m.partition_point(|&s| s < source_col),
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

/// Splice inline `$…$` spacers into one line's shaped output. For each formula whose raster is
/// ready (`block_math`) and that the caret isn't inside (left raw for editing), reserve a spacer
/// of whole spaces ≥ the raster's text-em width and record where to paint it. The raster is
/// rasterized at `em`; scaling by `fs/em` puts it at this line's text size. Returns the
/// (possibly unchanged) display/runs/map plus the line's formula placements.
#[allow(clippy::too_many_arguments)]
fn shape_inline_math(
    window: &mut Window,
    line: &str,
    line_start: usize,
    disp: String,
    runs: Vec<TextRun>,
    map: Vec<usize>,
    caret_col: Option<usize>,
    base_font: &Font,
    fs: Pixels,
    sf: f32,
    block_math: &BlockMathFn,
    em: f32,
) -> (String, Vec<TextRun>, Vec<usize>, Vec<InlineMath>) {
    let spans = markdown_syntax::inline_math_spans(line);
    if spans.is_empty() || sf <= 0. || em <= 0. {
        return (disp, runs, map, Vec::new());
    }
    let space_w = f32::from(measure_width(window, " ", base_font, fs)).max(1.);
    let scale = f32::from(fs) / em;
    let mut formulas: Vec<(Range<usize>, usize)> = Vec::new();
    let mut imgs: Vec<(Arc<RenderImage>, Pixels, Pixels, SharedString)> = Vec::new();
    for span in spans {
        // A caret STRICTLY inside the span keeps it raw (a fallback — normally arrowing/clicking
        // into a formula opens its structural editor before the caret lands here). A caret AT a
        // boundary (just before/after the `$…$`, e.g. after exiting the editor) leaves it
        // rendered, so sitting beside a formula doesn't flip it to raw.
        if caret_col.is_some_and(|c| span.start < c && c < span.end) {
            continue;
        }
        let latex = &line[span.start + 1..span.end - 1];
        let Some(img) = block_math(latex) else {
            continue; // not yet rasterized — leave the raw source until it lands
        };
        let dev = img.size(0);
        let (dw, dh) = (dev.width.0 as f32, dev.height.0 as f32);
        if dw <= 0. || dh <= 0. {
            continue;
        }
        let (w, h) = (dw / sf * scale, dh / sf * scale);
        let n = ((w / space_w).ceil() as usize).max(1);
        let latex: SharedString = latex.to_string().into();
        formulas.push((span, n));
        imgs.push((img, px(w), px(h), latex));
    }
    if formulas.is_empty() {
        return (disp, runs, map, Vec::new());
    }
    let gap = TextRun {
        len: 0,
        font: base_font.clone(),
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let (nd, nr, nm, places) =
        markdown_syntax::splice_inline_math(&disp, &runs, &map, &formulas, &gap);
    debug_assert_eq!(places.len(), imgs.len());
    let inline = places
        .into_iter()
        .zip(imgs)
        .map(|(p, (img, width, height, latex))| InlineMath {
            display_off: p.display_off,
            // Absolute byte range in the document, for hit-test / seating / commit.
            source: line_start + p.source.start..line_start + p.source.end,
            latex,
            img,
            width,
            height,
        })
        .collect();
    (nd, nr, nm, inline)
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
    block_chip: Option<&BlockChipFn>,
    block_mermaid: Option<&BlockMermaidFn>,
    block_math: Option<&BlockMathFn>,
    // The em the `block_math` provider rasterizes at, so inline `$…$` formulas can reuse those
    // rasters scaled to text size. `None` disables inline math.
    block_math_em: Option<f32>,
    editing_math: Option<(usize, usize, Pixels)>,
    scale_factor: f32,
    // The selected byte range; a line it touches keeps full source (markers
    // shown), the rest hide their markers (W6, reveal-on-caret).
    selection: (usize, usize),
    // An in-progress grip resize: the dragged image is *sized* to its live width
    // here (driving its row height) so the layout reflows live, rather than the
    // image painting over a stale, saved-size row.
    resize: Option<ImageResize>,
) -> ShapedLines {
    let mut wrapped = Vec::new();
    let mut heights = Vec::new();
    let mut widgets = Vec::new();
    let mut backgrounds: Vec<Option<CodeBg>> = Vec::new();
    let mut tables = Vec::new();
    let mut maps = Vec::new();
    let mut marks: Vec<Option<LineMark>> = Vec::new();
    let mut inline_maths: Vec<Vec<InlineMath>> = Vec::new();
    let lines: Vec<&str> = content.split('\n').collect();
    // Fenced-code-block regions; a block's ``` fence lines collapse (W6) unless
    // the caret is inside that block (then they show, so they stay editable).
    let code_regions = md
        .map(|_| markdown_syntax::code_regions(content))
        .unwrap_or_default();
    // ```mermaid blocks ready to render as a diagram: the caret is outside the
    // block and the host has a rendered bitmap. The diagram paints on the block's
    // first line; the rest collapse. Caret inside / still rendering → raw code.
    let mermaid: Vec<(Range<usize>, BlockImg)> = match block_mermaid.filter(|_| md.is_some()) {
        Some(f) => markdown_syntax::mermaid_blocks(content)
            .into_iter()
            .filter(|(range, _)| caret_row.is_none_or(|cr| !range.contains(&cr)))
            .filter_map(|(range, source)| {
                let img = f(&source)?;
                let bi = block_img(img, None, wrap_width, scale_factor)?;
                Some((range, bi))
            })
            .collect(),
        None => Vec::new(),
    };
    // $$…$$ math blocks ready to render: caret outside + a typeset bitmap ready. Like
    // mermaid, the equation paints on the block's first line, the rest collapse.
    let math: Vec<(Range<usize>, BlockImg)> = match block_math.filter(|_| md.is_some()) {
        Some(f) => markdown_syntax::math_regions(content)
            .into_iter()
            .filter(|r| caret_row.is_none_or(|cr| !r.range.contains(&cr)))
            .filter_map(|r| {
                let img = f(&r.source)?;
                let bi = block_img(img, None, wrap_width, scale_factor)?;
                // Math renders at its natural typeset size — no resize grip (nothing to
                // persist a width to, and it goes inline eventually). It carries its
                // horizontal alignment (centered by default) for the paint to honor.
                Some((
                    r.range,
                    BlockImg {
                        resizable: false,
                        align: r.align,
                        ..bi
                    },
                ))
            })
            .collect(),
        None => Vec::new(),
    };
    // `<!-- math:ALIGN -->` marker lines to hide (revealed only when the caret lands on them),
    // like table style markers.
    let math_marker_lines: Vec<usize> = if md.is_some() {
        markdown_syntax::math_regions(content)
            .iter()
            .filter_map(|r| r.marker_line)
            .collect()
    } else {
        Vec::new()
    };
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
    // Fenced-code-block tracking: collect a block's line indices (so its box can
    // be sized to its widest line + the first/last line marked for rounding) and
    // the running max line width.
    let mut code_block: Vec<usize> = Vec::new();
    let mut code_w = px(0.);
    let mut line_start = 0;
    let mut in_fence = false;
    for (idx, &line) in lines.iter().enumerate() {
        let line_end = line_start + line.len();

        // A ready mermaid block renders as its diagram (on the first line) with the
        // rest of the block collapsed — bypassing the normal per-line handling. Its
        // ``` fences still toggle `in_fence` so later code blocks track correctly.
        if let Some((range, bi)) = mermaid.iter().find(|(r, _)| r.contains(&idx)) {
            if line.trim_start().starts_with("```") {
                in_fence = !in_fence;
            }
            let (h, widget) = if idx == range.start {
                (bi.height, Some(Block::Image(bi.clone())))
            } else {
                (px(0.), None)
            };
            let wl = shape_runs(
                window,
                &SharedString::default(),
                base_font_size,
                &[],
                wrap_width,
            )
            .into_iter()
            .next()
            .expect("a line always shapes to one wrapped line");
            wrapped.push(wl);
            heights.push(h);
            widgets.push(widget);
            backgrounds.push(None);
            tables.push(None);
            maps.push(None);
            marks.push(None);
            inline_maths.push(Vec::new());
            line_start = line_end + 1;
            continue;
        }

        // An in-line-edited $$ block reserves a fixed gap; the host paints the live editor
        // there (positioned from this line's top/height). Takes precedence over the image.
        if let Some((start_row, end_row, gap_h)) = editing_math
            && (start_row..=end_row).contains(&idx)
        {
            let h = if idx == start_row { gap_h } else { px(0.) };
            let wl = shape_runs(
                window,
                &SharedString::default(),
                base_font_size,
                &[],
                wrap_width,
            )
            .into_iter()
            .next()
            .expect("a line always shapes to one wrapped line");
            wrapped.push(wl);
            heights.push(h);
            widgets.push(None);
            backgrounds.push(None);
            tables.push(None);
            maps.push(None);
            marks.push(None);
            inline_maths.push(Vec::new());
            line_start = line_end + 1;
            continue;
        }

        // A ready $$…$$ math block renders as its equation on the first line, the rest
        // collapsed. Unlike mermaid it's not a ``` fence, so it never toggles `in_fence`.
        if let Some((range, bi)) = math.iter().find(|(r, _)| r.contains(&idx)) {
            let (h, widget) = if idx == range.start {
                (bi.height, Some(Block::Image(bi.clone())))
            } else {
                (px(0.), None)
            };
            let wl = shape_runs(
                window,
                &SharedString::default(),
                base_font_size,
                &[],
                wrap_width,
            )
            .into_iter()
            .next()
            .expect("a line always shapes to one wrapped line");
            wrapped.push(wl);
            heights.push(h);
            widgets.push(widget);
            backgrounds.push(None);
            tables.push(None);
            maps.push(None);
            marks.push(None);
            inline_maths.push(Vec::new());
            line_start = line_end + 1;
            continue;
        }

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
        // A `<!-- table:STYLE -->` or `<!-- math:ALIGN -->` marker line collapses (hidden)
        // too, unless the caret lands on it — so the marker stays out of the way but editable.
        let collapse_marker = caret_row != Some(idx)
            && (regions.iter().any(|r| r.marker_line == Some(idx))
                || math_marker_lines.contains(&idx));

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

        // Block widget (non-code): a standalone `![](src)` line that isn't the
        // caret's renders as a file chip (if the host classifies `src` as one,
        // e.g. a PDF) or else its decoded image, fit to width.
        // A renderable image: a standalone `![](src)` line, or the sole body of a
        // list item (`- ![](src)`). For the list case `marker_len` > 0, so the
        // image renders inset past the bullet (still painted by the gutter) and
        // sized to the remaining width — instead of the row collapsing to the
        // image (losing its bullet) or falling back to raw source.
        let img_row = (!is_code)
            .then(|| markdown_syntax::image_row(line))
            .flatten();
        let img_inset = match img_row {
            Some((_, _, marker_len)) if marker_len > 0 => {
                measure_width(window, &line[..marker_len], base_font, base_font_size)
            }
            _ => px(0.),
        };
        let widget: Option<Block> = if Some(idx) != caret_row
            && let Some(st) = md
            && let Some((src, w_attr, _)) = img_row
        {
            if let Some(label) = block_chip.and_then(|f| f(src)) {
                Some(Block::Chip {
                    src: src.into(),
                    label,
                    link: st.link,
                    bg: st.code_bg,
                    border: st.marker,
                    height: fs * LINE_HEIGHT_RATIO + px(CHIP_PAD * 2.),
                })
            } else {
                block_image
                    .and_then(|f| f(src))
                    .and_then(|img| {
                        // A live grip resize sizes the image to the drag width, so
                        // its row height tracks the drag and the layout reflows.
                        let width_attr = match resize {
                            Some(r) if r.line == idx => Some(r.width),
                            _ => w_attr,
                        };
                        block_img(
                            img,
                            width_attr,
                            wrap_width.map(|w| (w - img_inset).max(px(1.))),
                            scale_factor,
                        )
                    })
                    .map(Block::Image)
            }
        } else {
            None
        };

        // Table row (W4c + cell editing): renders as a grid row; the caret on a
        // header/body row edits in place (caret rendered inside the cell). Only the
        // caret on the `|---|` separator drops the whole table to raw source (to
        // edit alignment), avoiding a broken outer box around a half-raw table.
        let table = regions
            .iter()
            .position(|r| r.lines.contains(&idx))
            .filter(|&ri| !is_code && caret_row != Some(regions[ri].lines.start + 1))
            .map(|ri| {
                let r = &regions[ri];
                TableRow {
                    cells: markdown_syntax::table_cells(line)
                        .into_iter()
                        .map(|c| SharedString::from(c.to_string()))
                        .collect(),
                    cell_ranges: markdown_syntax::table_cell_ranges(line),
                    aligns: r.aligns.clone(),
                    col_widths: region_cols[ri].clone(),
                    is_header: idx == r.lines.start,
                    is_separator: idx == r.lines.start + 1,
                    is_last: idx + 1 == r.lines.end,
                    // 0 for the first body row; None for the header/separator.
                    body_index: idx.checked_sub(r.lines.start + 2),
                    style: r.style,
                    border: md.map_or(base_color, |m| m.marker),
                    shade: md.map_or(hsla(0., 0., 0., 0.), |m| m.code_bg),
                }
            });

        // A line shows full source while a non-empty selection touches it (so the
        // markers are visible to select) or styling is off. Otherwise its markers
        // are hidden — except, on the caret's own line, the single construct the
        // caret sits in is revealed (per-construct reveal, #5: finer than the old
        // whole-line reveal, so the rest of the line stays rendered).
        let sel_empty = selection.0 == selection.1;
        let caret_col = (sel_empty && selection.0 >= line_start && selection.0 <= line_end)
            .then(|| selection.0 - line_start);
        // An `![](src)` line (image/chip candidate) shows full raw source while the
        // caret is on it, so editing reveals the whole `![](src)` rather than the
        // per-construct view (where the caret at the `!` would hide the link).
        let widget_line = md.is_some() && img_row.is_some();
        let full_source = (!sel_empty && selection.0 <= line_end && selection.1 >= line_start)
            || (caret_col.is_some() && widget_line);
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
        // `bullet_x` = measured width of the leading whitespace (where the bullet
        // paints); `text_inset` = measured width of the whole source prefix (where
        // the body sits, matching the raw line so render + edit stay in sync).
        let gutter: Option<(usize, LineMark)> = if let Some(st) = md.filter(|_| {
            // A list-item image keeps its bullet: allow the List gutter even
            // though the row also carries an (inset) image widget.
            !is_code && table.is_none() && (widget.is_none() || img_inset > px(0.))
        }) {
            if let Some(plen) = markdown_syntax::blockquote_prefix(line) {
                Some((plen, LineMark::Quote(st.quote)))
            } else if let Some((plen, indent, checked)) = markdown_syntax::task_prefix(line) {
                let bullet_x = measure_width(window, &line[..indent], base_font, base_font_size);
                let text_inset = measure_width(window, &line[..plen], base_font, base_font_size);
                Some((
                    plen,
                    LineMark::Check {
                        bullet_x,
                        text_inset,
                        checked,
                        color: st.quote,
                    },
                ))
            } else if let Some((plen, indent, ordered, num)) = markdown_syntax::list_prefix(line) {
                let bullet_x = measure_width(window, &line[..indent], base_font, base_font_size);
                let text_inset = measure_width(window, &line[..plen], base_font, base_font_size);
                Some((
                    plen,
                    LineMark::List {
                        bullet_x,
                        text_inset,
                        ordered,
                        num,
                        color: st.quote,
                    },
                ))
            } else {
                None
            }
        } else {
            None
        };
        let caret_here = caret_col.is_some();
        // A thematic break (`---`) renders as a full-width divider, but only while
        // the caret is off it; on the line it reads as the raw `---` (editable).
        let is_rule = !is_code
            && widget.is_none()
            && table.is_none()
            && !caret_here
            && !full_source
            && md.is_some()
            && markdown_syntax::thematic_break(line);
        let mark = if is_rule {
            md.map(|st| LineMark::Rule(st.quote))
        } else {
            gutter
                .filter(|_| !caret_here && !full_source)
                .map(|(_, m)| m)
        };
        let reveal_prefix = gutter.filter(|_| caret_here).map_or(0, |(plen, _)| plen);
        // Footnote definitions (`[^1]: …`) and raw-HTML lines render muted, the way
        // the reading view shows them — a whole-line color, no hidden markers.
        let muted_line = md
            .filter(|_| !is_code && widget.is_none() && table.is_none())
            .filter(|_| {
                markdown_syntax::footnote_def(line).is_some() || markdown_syntax::html_block(line)
            })
            .map(|st| st.quote);
        // A blockquote's body is muted; a list keeps the normal body color (only
        // its bullet is muted).
        let line_base = match mark {
            Some(LineMark::Quote(c)) => c,
            _ => muted_line.unwrap_or(base_color),
        };
        // Inline `$…$` formulas spliced into this line (populated by the hidden-markers branch).
        let mut line_inline_math: Vec<InlineMath> = Vec::new();
        let (shaped_text, runs, bg, map) = if collapse_fence || collapse_marker {
            // Hidden ``` fence line or table-style marker: nothing, zero height.
            (String::new(), Vec::new(), None, None)
        } else if is_rule {
            // Thematic break: the divider is painted from the mark; no body text.
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
            // Inline `$…$` math: swap each ready formula's glyphs for a spacer to paint over.
            match (block_math, block_math_em) {
                (Some(mathf), Some(em)) => {
                    let (disp, runs, m, im) = shape_inline_math(
                        window,
                        line,
                        line_start,
                        disp,
                        runs,
                        m,
                        caret_col,
                        base_font,
                        fs,
                        scale_factor,
                        mathf,
                        em,
                    );
                    line_inline_math = im;
                    (disp, runs, None, Some(m))
                }
                _ => (disp, runs, None, Some(m)),
            }
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
            let h = if collapse_fence || collapse_marker {
                px(0.)
            } else {
                match &table {
                    // The `|---|` separator collapses in grid mode — the old
                    // renderer doesn't show it; the first body row's top divider
                    // becomes the header/body border.
                    Some(t) if t.is_separator => px(0.),
                    Some(_) => table_row_h,
                    None => match widget.as_ref() {
                        // Reserve a little space around an inline image so a list
                        // of photos doesn't stack edge-to-edge.
                        Some(Block::Image(i)) => i.height + px(IMG_ROW_PAD),
                        Some(b) => b.height(),
                        // A text row grows to fit its tallest inline `$…$` formula (a fraction
                        // is taller than the text), so the formula doesn't overlap neighbours.
                        None => {
                            let math_h = line_inline_math
                                .iter()
                                .map(|im| im.height)
                                .fold(px(0.), |a, b| if b > a { b } else { a });
                            (fs * LINE_HEIGHT_RATIO).max(math_h + px(INLINE_MATH_ROW_PAD))
                        }
                    },
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
            inline_maths.push(line_inline_math);
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
    (
        wrapped,
        heights,
        widgets,
        backgrounds,
        tables,
        maps,
        marks,
        inline_maths,
    )
}

/// Paint a file chip — a rounded, bordered button with a flat document icon +
/// `label` — filling the row (sized in `shape_document` to include vertical
/// padding), its width fit to the label. Left-click opens it, right-click edits
/// (handled by the mouse handlers via `chip_rows`).
#[allow(clippy::too_many_arguments)]
fn paint_chip(
    label: &str,
    link: Hsla,
    bg: Hsla,
    border: Hsla,
    origin: Point<Pixels>,
    row_h: Pixels,
    font: &Font,
    font_size: Pixels,
    window: &mut Window,
    cx: &mut App,
) {
    let text = SharedString::from(label.to_string());
    let run = TextRun {
        len: text.len(),
        font: font.clone(),
        color: link,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window
        .text_system()
        .shape_line(text, font_size, &[run], None);
    let pad_x = px(10.);
    let icon_h = font_size * 0.92;
    let icon_w = icon_h * 0.74;
    let gap = px(7.); // between the icon and the label
    let box_w = pad_x * 2. + icon_w + gap + shaped.width();
    window.paint_quad(PaintQuad {
        bounds: Bounds::new(origin, size(box_w, row_h)),
        corner_radii: Corners::all(px(6.)),
        background: bg.into(),
        border_widths: Edges::all(px(1.)),
        border_color: border,
        border_style: BorderStyle::Solid,
    });
    let ix = origin.x + pad_x;
    paint_doc_icon(
        ix,
        origin.y + (row_h - icon_h) / 2.,
        icon_w,
        icon_h,
        link,
        window,
    );
    let line_h = font_size * LINE_HEIGHT_RATIO;
    let _ = shaped.paint(
        point(ix + icon_w + gap, origin.y + (row_h - line_h) / 2.),
        line_h,
        gpui::TextAlign::Left,
        None,
        window,
        cx,
    );
}

/// Paint a flat, line-art document glyph (a page with a folded top-right corner +
/// two text lines) in `color`, the chip's file icon. Drawn with strokes — not a
/// font emoji — so it reads flat and on-theme at the text's size.
fn paint_doc_icon(x: Pixels, y: Pixels, w: Pixels, h: Pixels, color: Hsla, window: &mut Window) {
    let f = w * 0.33; // folded-corner size
    // Page silhouette, with the top-right corner cut away for the fold.
    let mut outline = PathBuilder::stroke(px(1.3));
    outline.move_to(point(x, y));
    outline.line_to(point(x + w - f, y));
    outline.line_to(point(x + w, y + f));
    outline.line_to(point(x + w, y + h));
    outline.line_to(point(x, y + h));
    outline.line_to(point(x, y));
    if let Ok(p) = outline.build() {
        window.paint_path(p, color);
    }
    // The folded corner (dog-ear).
    let mut fold = PathBuilder::stroke(px(1.3));
    fold.move_to(point(x + w - f, y));
    fold.line_to(point(x + w - f, y + f));
    fold.line_to(point(x + w, y + f));
    if let Ok(p) = fold.build() {
        window.paint_path(p, color);
    }
    // Two short text lines below the fold.
    for fy in [0.6_f32, 0.78] {
        let mut ln = PathBuilder::stroke(px(1.));
        ln.move_to(point(x + w * 0.26, y + h * fy));
        ln.line_to(point(x + w * 0.74, y + h * fy));
        if let Ok(p) = ln.build() {
            window.paint_path(p, color);
        }
    }
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
    let pad = px(TABLE_CELL_PAD);
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

/// Horizontal inset (px) of a table cell's text from its column's left edge.
const TABLE_CELL_PAD: f32 = 10.;
/// Left indent for tables, so per-row delete "−" handles sit in a gutter beside the
/// grid instead of over the first cell (issue #16).
const TABLE_GUTTER: f32 = 22.;

/// The font a table cell is rendered with — bold in the header row.
fn cell_font(font: &Font, is_header: bool) -> Font {
    let mut f = font.clone();
    if is_header {
        f.weight = gpui::FontWeight::BOLD;
    }
    f
}

/// Shape a table cell's `content` into a single (unwrapped) line, for exact
/// caret / hit-test geometry that matches the kerned glyphs `paint_table_row`
/// renders (measuring prefixes in isolation drifts by their kerning).
fn shape_cell(
    window: &mut Window,
    content: &str,
    font: &Font,
    font_size: Pixels,
) -> Option<WrappedLine> {
    let run = TextRun {
        len: content.len(),
        font: font.clone(),
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let runs: &[TextRun] = if content.is_empty() {
        &[]
    } else {
        std::slice::from_ref(&run)
    };
    window
        .text_system()
        .shape_text(
            SharedString::from(content.to_string()),
            font_size,
            runs,
            None,
            None,
        )
        .ok()?
        .into_iter()
        .next()
}

/// The cell a source column `col` (line-local) falls in for a table row — clamped
/// to the nearest cell when `col` is in a pipe/space between cells.
fn table_cell_at(t: &TableRow, col: usize) -> usize {
    t.cell_ranges
        .iter()
        .position(|r| col <= r.end)
        .unwrap_or(t.cell_ranges.len().saturating_sub(1))
}

/// Screen position of the caret at source column `col` (line-local) inside a
/// table row's rendered cells: `(x, cell_index, in_cell_offset)`. Mirrors
/// `paint_table_row`'s layout (cumulative column widths + pad + alignment).
fn table_caret_pos(
    t: &TableRow,
    col: usize,
    left: Pixels,
    font: &Font,
    font_size: Pixels,
    window: &mut Window,
) -> Option<(Pixels, usize, usize)> {
    if t.cell_ranges.is_empty() {
        return None;
    }
    let pad = px(TABLE_CELL_PAD);
    let cell = table_cell_at(t, col);
    let range = t.cell_ranges.get(cell)?;
    let content = t.cells.get(cell)?;
    let in_cell = col.saturating_sub(range.start).min(content.len());
    let cell_x = left + t.col_widths[..cell].iter().fold(px(0.), |a, &w| a + w);
    let cw = t.col_widths.get(cell).copied().unwrap_or(px(0.));
    // The header is bold, so shape with the bold font or the caret lands left of
    // the (wider) bold glyphs; position_for_index gives the exact kerned x.
    let cf = cell_font(font, t.is_header);
    let line_h = font_size * LINE_HEIGHT_RATIO;
    let wl = shape_cell(window, content, &cf, font_size)?;
    let prefix_w = wl
        .position_for_index(in_cell, line_h)
        .map_or(px(0.), |p| p.x);
    let full_w = wl.width();
    let avail = (cw - pad * 2.).max(px(0.));
    let align_off = match t.aligns.get(cell) {
        Some(markdown_syntax::Align::Center) => (avail - full_w).max(px(0.)) / 2.,
        Some(markdown_syntax::Align::Right) => (avail - full_w).max(px(0.)),
        _ => px(0.),
    };
    Some((cell_x + pad + align_off + prefix_w, cell, in_cell))
}

/// The byte offset within `content` whose rendered x (from the text's left edge)
/// is closest to `target` — hit-tests a click inside a table cell, using the
/// shaped line so it matches the kerned glyphs.
fn cell_offset_for_x(
    content: &str,
    target: Pixels,
    font: &Font,
    font_size: Pixels,
    window: &mut Window,
) -> usize {
    let line_h = font_size * LINE_HEIGHT_RATIO;
    let Some(wl) = shape_cell(window, content, font, font_size) else {
        return 0;
    };
    match wl.closest_index_for_position(point(target, line_h / 2.), line_h) {
        Ok(i) | Err(i) => i,
    }
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
    use markdown_syntax::TableStyle;
    let thick = px(1.);
    // The collapsed `|---|` separator draws nothing — the outer box + the next
    // row's top divider form the header/body border.
    if t.is_separator {
        return;
    }
    let style = t.style;
    let vlines = matches!(style, TableStyle::Grid);
    // A single rule under the header (Striped/Minimal) vs a divider under every
    // row (Grid) vs none (Header).
    let header_rule = matches!(style, TableStyle::Striped | TableStyle::Minimal);
    let table_w = t.col_widths.iter().fold(px(0.), |a, &w| a + w);
    // Row shading (painted first, behind everything): the header for the Header
    // style; alternate body rows for Striped.
    let shaded = match style {
        TableStyle::Header => t.is_header,
        TableStyle::Striped => t.body_index.is_some_and(|b| b % 2 == 1),
        _ => false,
    };
    if shaded {
        window.paint_quad(fill(Bounds::new(origin, size(table_w, row_h)), t.shade));
    }
    // Horizontal divider at the row's top: under every row (Grid, header excepted —
    // the box covers it), or just under the header (Striped/Minimal: the first body
    // row's top), or never (Header).
    let top_divider = if matches!(style, TableStyle::Grid) {
        !t.is_header
    } else {
        header_rule && t.body_index == Some(0)
    };
    if top_divider {
        window.paint_quad(fill(Bounds::new(origin, size(table_w, thick)), t.border));
    }
    let pad = px(TABLE_CELL_PAD);
    let mut cell_font = font.clone();
    if t.is_header {
        cell_font.weight = gpui::FontWeight::BOLD;
    }
    let mut x = origin.x;
    for (c, &cw) in t.col_widths.iter().enumerate() {
        // Inner cell separator at the left of every cell except the first (Grid
        // only; the other styles drop vertical lines).
        if vlines && c > 0 {
            window.paint_quad(fill(
                Bounds::new(point(x, origin.y), size(thick, row_h)),
                t.border,
            ));
        }
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
}

/// Paint a table add-row / add-column affordance: a thin strip with a centered
/// "+". Subtle by default; on hover a faint fill + a brighter glyph.
/// Paint a row/column delete handle: a small rounded button with a "−". Filled on
/// hover, a muted glyph otherwise.
fn paint_del_handle(bounds: Bounds<Pixels>, border: Hsla, hot: bool, window: &mut Window) {
    let mut bg = border;
    bg.a = if hot { 0.22 } else { 0.10 };
    window.paint_quad(fill(bounds, bg).corner_radii(Corners::all(px(4.))));
    let mut glyph = border;
    glyph.a = if hot { 0.95 } else { 0.6 };
    let cx = bounds.origin.x + bounds.size.width / 2.;
    let cy = bounds.origin.y + bounds.size.height / 2.;
    let arm = px(5.);
    let th = px(1.5);
    window.paint_quad(fill(
        Bounds::new(point(cx - arm, cy - th / 2.), size(arm * 2., th)),
        glyph,
    ));
}

fn paint_add_strip(bounds: Bounds<Pixels>, border: Hsla, hot: bool, window: &mut Window) {
    // A rounded box matching the delete handles — filled faintly, brighter on hover
    // — with a centered "+".
    let mut bg = border;
    bg.a = if hot { 0.22 } else { 0.10 };
    window.paint_quad(fill(bounds, bg).corner_radii(Corners::all(px(4.))));
    let mut glyph = border;
    glyph.a = if hot { 0.95 } else { 0.6 };
    let cx = bounds.origin.x + bounds.size.width / 2.;
    let cy = bounds.origin.y + bounds.size.height / 2.;
    let arm = px(5.);
    let th = px(1.5);
    window.paint_quad(fill(
        Bounds::new(point(cx - arm, cy - th / 2.), size(arm * 2., th)),
        glyph,
    ));
    window.paint_quad(fill(
        Bounds::new(point(cx - th / 2., cy - arm), size(th, arm * 2.)),
        glyph,
    ));
}

/// The custom element that lays out + paints the editor's wrapped lines, cursor,
/// and selection, and wires the input handler. Height is content-driven via a
/// measured layout (it depends on the resolved width once soft-wrap is applied).
struct EditorElement {
    editor: Entity<EditorState>,
}

/// Per-table hover-revealed "+" affordances (issue #16): a hover zone (the table
/// plus a thin margin) that gates visibility, plus the below/right "+" strips with
/// their own hitboxes (hover cursor) and the table row to seat the caret in.
struct TableAdds {
    zone: Bounds<Pixels>,
    below: Bounds<Pixels>,
    below_hit: Hitbox,
    below_row: usize,
    right: Bounds<Pixels>,
    right_hit: Hitbox,
    right_row: usize,
    border: Hsla,
}

/// A per-row or per-column delete handle for the hovered table cell (issue #16):
/// where to paint the "−", its hover-cursor hitbox, and the row/column to remove.
struct DelHandle {
    bounds: Bounds<Pixels>,
    /// The whole row's / column's cell rect, tinted on hover so the delete target
    /// is obvious.
    highlight: Bounds<Pixels>,
    hit: Hitbox,
    row: usize,
    col: usize,
    border: Hsla,
    /// Paint the row/column tint (kept for borderless tables to show the grid).
    show_highlight: bool,
}

struct PrepaintState {
    wrapped: Vec<WrappedLine>,
    /// Top offset of each logical line relative to the editor's top.
    line_tops: Vec<Pixels>,
    /// Per-logical-line wrap-row height (variable for headings + images).
    line_heights: Vec<Pixels>,
    /// `Some` for a line painted as an inline image instead of its source text.
    widgets: Vec<Option<Block>>,
    /// Per-line fenced-code-block background (rounded full-width box).
    backgrounds: Vec<Option<CodeBg>>,
    /// `Some` for a line painted as a table-grid row instead of source.
    tables: Vec<Option<TableRow>>,
    /// Per-line display→source byte map for marker-hidden rows (W6).
    maps: Vec<Option<Vec<usize>>>,
    /// Per-line gutter decoration (blockquote / list / checkbox).
    marks: Vec<Option<LineMark>>,
    /// Per-line inline `$…$` formulas (image + display offset + source range), painted over
    /// their spacers in the shaped text.
    inline_maths: Vec<Vec<InlineMath>>,
    /// Corner-grip hitbox for each painted inline image, in `widgets` order — so
    /// paint can set the resize cursor over each (hitboxes must be inserted in
    /// prepaint). Parallels the images paint walks, indexed by image count.
    image_grips: Vec<Hitbox>,
    /// Pointer-cursor hitboxes (`(line, hitbox)`) for clickable gutter checkboxes
    /// and file chips, so the cursor flips to a hand over them (like the image
    /// grips' resize cursor). Set in paint via `set_cursor_style`.
    checkbox_grips: Vec<(usize, Hitbox)>,
    chip_grips: Vec<(usize, Hitbox)>,
    /// Per-table hover-revealed add-row/add-column "+" affordances (issue #16).
    table_adds: Vec<TableAdds>,
    /// Hovered-cell delete handles (issue #16): the "−" for the hovered row + column.
    row_del: Option<DelHandle>,
    col_del: Option<DelHandle>,
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
                // Reveal-on-caret applies only while focused (matches prepaint).
                let focused = editor.focus_handle.is_focused(window);
                let caret_row = focused.then(|| editor.row_col(editor.cursor_offset()).0);
                let selection = if focused {
                    (editor.selected_range.start, editor.selected_range.end)
                } else {
                    (usize::MAX, usize::MAX)
                };
                let sf = window.scale_factor();
                let (wrapped, heights, _, backgrounds, _, _, _, _) = shape_document(
                    window,
                    &editor.content,
                    &text_style.font(),
                    text_style.color,
                    font_size,
                    &editor.diagnostics,
                    editor.markdown_style.as_ref(),
                    wrap_width,
                    caret_row,
                    editor.block_image.as_ref(),
                    editor.block_chip.as_ref(),
                    editor.block_mermaid.as_ref(),
                    editor.block_math.as_ref(),
                    editor.block_math_em,
                    editor.editing_block.as_ref().map(|eb| {
                        let sr = editor.row_col(eb.range.start).0;
                        let er = editor
                            .row_col(eb.range.end.saturating_sub(1).max(eb.range.start))
                            .0;
                        (sr, er, eb.height)
                    }),
                    sf,
                    selection,
                    editor.image_resize,
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
        // Reveal-on-caret (markers, raw-on-caret widgets, per-construct reveal)
        // applies only while the editor is focused. An unfocused editor — always
        // shown in WYSIWYG mode but not being edited — renders fully, like a
        // reading view. `caret_row = None` + a no-match selection do that.
        let focused = editor.focus_handle.is_focused(window);
        // The active image-resize drag (if any), so a dragged image's grip hitbox
        // tracks its live preview size (copied out — `editor` stays borrowed below).
        let image_resize = editor.image_resize;
        let style = window.text_style();
        let font = style.font();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let base_lh = font_size * LINE_HEIGHT_RATIO;
        let wrap_width = Some(bounds.size.width);
        let text_color = style.color;

        // Placeholder (uniform) when empty; else shape per line so headings get
        // their own taller rows (W2) and image lines render inline (W4).
        let caret_row = focused.then(|| editor.row_col(editor.cursor_offset()).0);
        let selection = if focused {
            (editor.selected_range.start, editor.selected_range.end)
        } else {
            (usize::MAX, usize::MAX)
        };
        let sf = window.scale_factor();
        let (wrapped, line_heights, widgets, backgrounds, tables, maps, marks, inline_maths) =
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
                    vec![Vec::new(); n],
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
                    caret_row,
                    editor.block_image.as_ref(),
                    editor.block_chip.as_ref(),
                    editor.block_mermaid.as_ref(),
                    editor.block_math.as_ref(),
                    editor.block_math_em,
                    editor.editing_block.as_ref().map(|eb| {
                        let sr = editor.row_col(eb.range.start).0;
                        let er = editor
                            .row_col(eb.range.end.saturating_sub(1).max(eb.range.start))
                            .0;
                        (sr, er, eb.height)
                    }),
                    sf,
                    selection,
                    editor.image_resize,
                )
            };

        // Top offset of each logical line (running sum of variable wrap heights),
        // reserving a gap above/below each code block so its padded box has its
        // own space (no overlap with the adjacent line, no blank line required).
        let mut line_tops = Vec::with_capacity(wrapped.len());
        let mut y = px(0.);
        for (idx, ((line, lh), bg)) in wrapped
            .iter()
            .zip(line_heights.iter())
            .zip(backgrounds.iter())
            .enumerate()
        {
            let (mut top_pad, mut bot_pad) = code_pads(*bg);
            // Reserve a gutter above the header (column-delete "−" handles) and below
            // the last row (the add-row "+" strip), mirroring the left gutter from
            // TABLE_GUTTER — baked into line_tops so the caret / click / paint all
            // shift with it, and neither affordance overlaps the adjacent line.
            if let Some(t) = tables.get(idx).and_then(Option::as_ref) {
                if t.is_header {
                    top_pad += px(TABLE_GUTTER);
                }
                if t.is_last {
                    bot_pad += px(TABLE_GUTTER);
                }
            }
            y += top_pad;
            line_tops.push(y);
            y += *lh * (line.wrap_boundaries().len() + 1) as f32 + bot_pad;
        }

        // Corner-grip hitboxes for each inline image, in `widgets` order (matching
        // the order paint walks them) — hitboxes must be inserted during prepaint,
        // but the resize cursor is set during paint via these. Mirrors the paint's
        // image-bounds math (row inset + IMG_ROW_PAD, live drag size) exactly so
        // the grip pins to the painted corner (incl. list-item images, which inset
        // past their bullet).
        let mut image_grips = Vec::new();
        for (i, w) in widgets.iter().enumerate() {
            if let Some(Block::Image(img)) = w
                && img.resizable
            {
                let inset = row_inset(
                    backgrounds.get(i).copied().flatten(),
                    marks.get(i).copied().flatten(),
                );
                let (img_w, img_h) = image_display_size(img, image_resize, i);
                let img_bounds = Bounds::new(
                    point(
                        bounds.origin.x + inset,
                        bounds.origin.y + line_tops[i] + px(IMG_ROW_PAD / 2.),
                    ),
                    size(img_w, img_h),
                );
                let grip = EditorState::image_grip(img_bounds);
                image_grips.push(window.insert_hitbox(grip, HitboxBehavior::Normal));
            }
        }

        // Pointer-cursor hitboxes for clickable gutter checkboxes + file chips, so
        // the cursor flips to a hand over them. Bounds mirror the paint math; keyed
        // by line so paint sets the cursor on each (see `set_cursor_style`).
        let mut checkbox_grips = Vec::new();
        let mut chip_grips = Vec::new();
        for (i, lh) in line_heights.iter().enumerate() {
            if let Some(LineMark::Check { bullet_x, .. }) = marks.get(i).copied().flatten() {
                let sz = font_size * 0.78;
                let pad = px(4.);
                let bx = bounds.origin.x + bullet_x;
                let by = bounds.origin.y + line_tops[i] + (*lh - sz) / 2.;
                let hit = Bounds::new(
                    point(bx - pad, by - pad),
                    size(sz + pad * 2., sz + pad * 2.),
                );
                checkbox_grips.push((i, window.insert_hitbox(hit, HitboxBehavior::Normal)));
            }
            if matches!(
                widgets.get(i).and_then(Option::as_ref),
                Some(Block::Chip { .. })
            ) {
                let hit = Bounds::new(
                    point(bounds.origin.x, bounds.origin.y + line_tops[i]),
                    size(bounds.size.width, *lh),
                );
                chip_grips.push((i, window.insert_hitbox(hit, HitboxBehavior::Normal)));
            }
        }

        // Per-table add-row / add-column "+" affordances (issue #16), revealed on
        // hover. Each table contributes a hover zone (the grid + a thin margin) plus
        // a "+" strip below (adds a row) and to the right (adds a column); bounds
        // follow the painted rows. Paint shows/cursors them only while the zone is
        // hovered; on_mouse_down hit-tests the committed strip rects.
        let mouse = window.mouse_position();
        let mut table_adds: Vec<TableAdds> = Vec::new();
        let mut row_del: Option<DelHandle> = None;
        let mut col_del: Option<DelHandle> = None;
        let mut tbl_top: Option<Pixels> = None;
        let mut tbl_header = 0usize;
        for (i, slot) in tables.iter().enumerate() {
            let Some(t) = slot else { continue };
            if t.is_header {
                tbl_top = Some(bounds.origin.y + line_tops[i]);
                tbl_header = i;
            }
            if t.is_last && !t.col_widths.is_empty() {
                let top = tbl_top.unwrap_or(bounds.origin.y + line_tops[i]);
                let bottom = bounds.origin.y + line_tops[i] + line_heights[i];
                let left = bounds.origin.x + px(TABLE_GUTTER);
                let width: Pixels = t.col_widths.iter().copied().sum();
                // Full-edge "+" tabs: a strip along the bottom (adds a row) and the
                // right (adds a column), each the table's full extent like the box.
                // paint rounds the two outer corners so the edge bulging away from
                // the table reads as a half-moon.
                let r = (line_heights[i] * 0.75).max(px(12.));
                let below = Bounds::new(point(left, bottom), size(width, r));
                let right = Bounds::new(point(left + width, top), size(r, bottom - top));
                let zone = Bounds::new(point(left, top), size(width + r, (bottom - top) + r));
                table_adds.push(TableAdds {
                    zone,
                    below,
                    below_hit: window.insert_hitbox(below, HitboxBehavior::Normal),
                    below_row: i,
                    right,
                    right_hit: window.insert_hitbox(right, HitboxBehavior::Normal),
                    right_row: tbl_header,
                    border: t.border,
                });

                // Per-row + per-column delete "−" handles (issue #16): full-height in
                // the left gutter, full-width in the top gutter. Hover bands reach
                // into the gutters so moving onto a handle keeps it shown.
                let g = px(TABLE_GUTTER);
                // Always available on hover (people delete rows/columns while editing,
                // too). The highlight stays for borderless (lineless) tables so the
                // otherwise-invisible grid still shows.
                let has_lines = matches!(t.style, markdown_syntax::TableStyle::Grid);
                let show_highlight = !has_lines;
                if mouse.x >= bounds.origin.x && mouse.x < left + width {
                    for line in tbl_header..=i {
                        let Some(rt) = tables.get(line).and_then(Option::as_ref) else {
                            continue;
                        };
                        if rt.is_separator || rt.is_header {
                            continue;
                        }
                        let rtop = bounds.origin.y + line_tops[line];
                        let rh = line_heights[line];
                        if mouse.y >= rtop && mouse.y < rtop + rh {
                            let rb = Bounds::new(
                                point(bounds.origin.x + px(2.), rtop + px(1.)),
                                size((g - px(5.)).max(px(12.)), (rh - px(2.)).max(px(8.))),
                            );
                            row_del = Some(DelHandle {
                                bounds: rb,
                                highlight: Bounds::new(point(left, rtop), size(width, rh)),
                                hit: window.insert_hitbox(rb, HitboxBehavior::Normal),
                                row: line,
                                col: 0,
                                border: rt.border,
                                show_highlight,
                            });
                            break;
                        }
                    }
                }
                if mouse.y >= top - g
                    && mouse.y < bottom
                    && mouse.x >= left
                    && mouse.x < left + width
                {
                    let mut colx = left;
                    for (col, &cw) in t.col_widths.iter().enumerate() {
                        if mouse.x < colx + cw || col + 1 == t.col_widths.len() {
                            let cb = Bounds::new(
                                point(colx + px(2.), top - g + px(2.)),
                                size((cw - px(4.)).max(px(12.)), (g - px(4.)).max(px(8.))),
                            );
                            col_del = Some(DelHandle {
                                bounds: cb,
                                highlight: Bounds::new(point(colx, top), size(cw, bottom - top)),
                                hit: window.insert_hitbox(cb, HitboxBehavior::Normal),
                                row: tbl_header,
                                col,
                                border: t.border,
                                show_highlight,
                            });
                            break;
                        }
                        colx += cw;
                    }
                }
                tbl_top = None;
            }
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
                Bounds::new(
                    point(bounds.left(), bounds.top()),
                    size(px(CARET_WIDTH), base_lh),
                ),
                text_color,
            );
            (Some(c), Vec::new())
        } else if editor.selected_range.is_empty() {
            let (row, col) = editor.row_col(editor.cursor_offset());
            let lh = line_heights.get(row).copied().unwrap_or(base_lh);
            let top = line_tops.get(row).copied().unwrap_or(px(0.));
            // Caret inside a table cell: position it within the rendered cell
            // (centered vertically like the cell text), not by the raw source line.
            if let Some(t) = tables.get(row).and_then(Option::as_ref)
                && let Some((x, _, _)) = table_caret_pos(
                    t,
                    col,
                    bounds.left() + px(TABLE_GUTTER),
                    &font,
                    font_size,
                    window,
                )
            {
                let y = bounds.top() + top + (lh - base_lh) / 2.;
                let c = fill(
                    Bounds::new(point(x, y), size(px(CARET_WIDTH), base_lh)),
                    text_color,
                );
                (Some(c), Vec::new())
            } else {
                let p = wrapped
                    .get(row)
                    .and_then(|l| l.position_for_index(disp_col(row, col), lh))
                    .unwrap_or_default();
                let inset = code_inset(row);
                let c = fill(
                    Bounds::new(
                        to_screen(top, point(p.x + inset, p.y)),
                        size(px(CARET_WIDTH), lh),
                    ),
                    text_color,
                );
                (Some(c), Vec::new())
            }
        } else {
            let (s, e) = (editor.selected_range.start, editor.selected_range.end);
            let starts = editor.line_starts();
            let (s_row, _) = editor.row_col(s);
            let (e_row, _) = editor.row_col(e);
            let right = bounds.size.width;
            // Selection tint = the theme accent at low alpha (fallback: a fixed blue).
            let color = editor
                .markdown_style
                .as_ref()
                .map_or(rgba(0x3b82f640).into(), |s| {
                    let mut c = s.link;
                    c.a = 0.25;
                    c
                });
            let mut sels = Vec::new();
            for row in s_row..=e_row {
                let Some(line) = wrapped.get(row) else {
                    continue;
                };
                let lh = line_heights.get(row).copied().unwrap_or(base_lh);
                let top = line_tops[row];
                let line_start = starts[row];
                let a = s.max(line_start) - line_start;
                let b = e.min(editor.line_end(row)) - line_start;
                // Table row: highlight between the cell positions of the selection
                // ends (not raw-source geometry).
                if let Some(t) = tables.get(row).and_then(Option::as_ref) {
                    if let (Some((xa, ..)), Some((xb, ..))) = (
                        table_caret_pos(
                            t,
                            a,
                            bounds.left() + px(TABLE_GUTTER),
                            &font,
                            font_size,
                            window,
                        ),
                        table_caret_pos(
                            t,
                            b,
                            bounds.left() + px(TABLE_GUTTER),
                            &font,
                            font_size,
                            window,
                        ),
                    ) {
                        let (lo, hi) = (xa.min(xb), xa.max(xb));
                        let cy = bounds.top() + top + (lh - base_lh) / 2.;
                        sels.push(fill(
                            Bounds::from_corners(
                                point(lo, cy),
                                point(hi.max(lo + px(2.)), cy + base_lh),
                            ),
                            color,
                        ));
                    }
                    continue;
                }
                let inset = code_inset(row);
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
            inline_maths,
            image_grips,
            checkbox_grips,
            chip_grips,
            table_adds,
            row_del,
            col_del,
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
        // Inline-image resize: the accent color for the corner grips, and the
        // active drag (if any) so the dragged image paints at its live width.
        let grip_color = self
            .editor
            .read(cx)
            .markdown_style
            .as_ref()
            .map_or(text_color, |s| s.link);
        let image_resize = self.editor.read(cx).image_resize;
        // Window-space bounds of each painted image + its logical line, collected
        // for the next frame's grip hit-testing (committed below).
        let mut image_rects: Vec<(usize, Bounds<Pixels>)> = Vec::new();
        // Window-space bounds of each inline `$…$` formula + its absolute range and LaTeX, for
        // the next frame's click-to-edit hit-testing + seating the structural editor.
        let mut inline_math_rects: Vec<(Range<usize>, SharedString, Bounds<Pixels>)> = Vec::new();
        // The span being structurally edited (if any): skip painting its raster — the seated
        // editor overlays its spot.
        let editing_inline = self
            .editor
            .read(cx)
            .editing_inline
            .as_ref()
            .map(|e| e.range.clone());
        // Window-space box bounds of each painted task checkbox + its line, for the
        // next frame's click-to-toggle hit-testing (committed below).
        let mut checkbox_rects: Vec<(usize, Bounds<Pixels>)> = Vec::new();
        // Logseq-style list nesting guides: `outline` holds the bullet x of each
        // active ancestor level, so a faint vertical line can drop from each down
        // through its descendants. Popped on dedent, reset off the list.
        let mut outline: Vec<Pixels> = Vec::new();
        for (i, ((line, top), lh)) in prepaint
            .wrapped
            .iter()
            .zip(prepaint.line_tops.iter())
            .zip(prepaint.line_heights.iter())
            .enumerate()
        {
            let origin = point(bounds.origin.x, bounds.origin.y + *top);
            // Nesting guides for a list/task row: a thin vertical line at each
            // ancestor bullet's x, spanning this row (contiguous rows stack into a
            // continuous guide).
            match prepaint.marks.get(i).copied().flatten() {
                Some(LineMark::List {
                    bullet_x, color, ..
                })
                | Some(LineMark::Check {
                    bullet_x, color, ..
                }) => {
                    while outline.last().is_some_and(|&x| x >= bullet_x) {
                        outline.pop();
                    }
                    let guide = Hsla {
                        a: color.a * 0.5,
                        ..color
                    };
                    for &gx in &outline {
                        window.paint_quad(fill(
                            Bounds::new(point(origin.x + gx + px(3.), origin.y), size(px(1.), *lh)),
                            guide,
                        ));
                    }
                    outline.push(bullet_x);
                }
                _ => outline.clear(),
            }
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
            // Thematic break: a 1px full-width divider centered in the row.
            if let Some(LineMark::Rule(c)) = prepaint.marks.get(i).copied().flatten() {
                let y = origin.y + (*lh - px(1.)) / 2.;
                let w = bounds.size.width;
                window.paint_quad(fill(Bounds::new(point(origin.x, y), size(w, px(1.))), c));
            }
            // List item: a muted bullet (`•`) or number (`N.`) glyph where the
            // hidden source marker began (`bullet_x`); the body is inset to the
            // measured prefix width so it lines up with the raw line.
            if let Some(LineMark::List {
                bullet_x,
                ordered,
                num,
                color,
                ..
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
                    point(origin.x + bullet_x, origin.y),
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
                bullet_x,
                checked,
                color,
                ..
            }) = prepaint.marks.get(i).copied().flatten()
            {
                let sz = font_size * 0.78; // ~cap height
                let bx = origin.x + bullet_x;
                let by = origin.y + (*lh - sz) / 2.; // vertically centered on the line
                let box_bounds = Bounds::new(point(bx, by), size(sz, sz));
                checkbox_rects.push((i, box_bounds));
                if let Some(hb) = prepaint
                    .checkbox_grips
                    .iter()
                    .find_map(|(l, hb)| (*l == i).then_some(hb))
                {
                    window.set_cursor_style(CursorStyle::PointingHand, hb);
                }
                window.paint_quad(PaintQuad {
                    bounds: box_bounds,
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
                // The header row paints the whole table's rounded outer border
                // (one box around all its rows, matching the reading view) — for the
                // Grid style only; the others are box-less. Each row then paints its
                // shading, dividers, + cell text.
                if t.is_header && matches!(t.style, markdown_syntax::TableStyle::Grid) {
                    let mut total_h = px(0.);
                    for j in i..prepaint.tables.len() {
                        match prepaint.tables[j].as_ref() {
                            Some(tr) => {
                                total_h += prepaint.line_heights[j];
                                if tr.is_last {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                    let table_w = t.col_widths.iter().fold(px(0.), |a, &w| a + w);
                    window.paint_quad(PaintQuad {
                        bounds: Bounds::new(
                            point(origin.x + px(TABLE_GUTTER), origin.y),
                            size(table_w, total_h),
                        ),
                        corner_radii: Corners::all(px(6.)),
                        background: hsla(0., 0., 0., 0.).into(),
                        border_widths: Edges::all(px(1.)),
                        border_color: t.border,
                        border_style: BorderStyle::Solid,
                    });
                }
                paint_table_row(
                    t,
                    point(origin.x + px(TABLE_GUTTER), origin.y),
                    *lh,
                    &font,
                    font_size,
                    base_lh,
                    text_color,
                    window,
                    cx,
                );
            } else if let Some(Block::Image(w)) = prepaint.widgets.get(i).and_then(Option::as_ref) {
                // Inline image (W4a): paint the decoded image instead of source,
                // inset to the row's gutter so a list-item image sits past its
                // bullet (painted above, like any list row).
                let inset = row_inset(
                    prepaint.backgrounds.get(i).copied().flatten(),
                    prepaint.marks.get(i).copied().flatten(),
                );
                // While this image's grip is being dragged, preview the live width
                // (aspect-preserved from the saved size) instead of the saved
                // `{width=N}` — the source isn't rewritten until release.
                let (img_w, img_h) = image_display_size(w, image_resize, i);
                // Honor the block's horizontal alignment within the content width. Display math
                // centers by default; left/right come from its `<!-- math:ALIGN -->` marker. A
                // real image is always `Left` (it sits at the row's inset).
                let slack = bounds.size.width - img_w;
                let img_x = match w.align {
                    _ if slack <= px(0.) => origin.x + inset,
                    MathAlign::Left => origin.x + inset,
                    MathAlign::Center => origin.x + px(f32::from(slack) / 2.0),
                    MathAlign::Right => origin.x + slack,
                };
                let img_bounds = Bounds::new(
                    point(img_x, origin.y + px(IMG_ROW_PAD / 2.)),
                    size(img_w, img_h),
                );
                let _ = window.paint_image(img_bounds, Corners::default(), w.img.clone(), 0, false);
                // A draggable corner grip (accent square) + the resize cursor over it,
                // via the hitbox inserted in prepaint. Recorded in `image_rects` for the
                // next frame's grip hit-testing. Skipped for non-resizable blocks (math),
                // keeping `image_grips` parallel to `image_rects`.
                if w.resizable {
                    let grip = EditorState::image_grip(img_bounds);
                    window.paint_quad(fill(grip, grip_color).corner_radii(Corners::all(px(3.))));
                    if let Some(hitbox) = prepaint.image_grips.get(image_rects.len()) {
                        window.set_cursor_style(CursorStyle::ResizeLeftRight, hitbox);
                    }
                    image_rects.push((i, img_bounds));
                }
            } else if let Some(Block::Chip {
                label,
                link,
                bg,
                border,
                ..
            }) = prepaint.widgets.get(i).and_then(Option::as_ref)
            {
                // File chip (e.g. a PDF embed): a rounded button with the label.
                paint_chip(
                    label, *link, *bg, *border, origin, *lh, &font, font_size, window, cx,
                );
                if let Some(hb) = prepaint
                    .chip_grips
                    .iter()
                    .find_map(|(l, hb)| (*l == i).then_some(hb))
                {
                    window.set_cursor_style(CursorStyle::PointingHand, hb);
                }
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
                // Inline `$…$` formulas: paint each typeset raster over its spacer, centered on
                // the text row. `position_for_index` gives the spacer's x + wrap-row offset.
                // Record each formula's window bounds for click-to-edit; the one being edited
                // shows the seated editor instead of its raster.
                for im in prepaint.inline_maths.get(i).into_iter().flatten() {
                    if let Some(p) = line.position_for_index(im.display_off, *lh) {
                        let x = text_origin.x + p.x;
                        // Center the formula in the (grown-to-fit) wrap row at p.y.
                        let y = origin.y + p.y + (*lh - im.height) / 2.0;
                        let b = Bounds::new(point(x, y), size(im.width, im.height));
                        inline_math_rects.push((im.source.clone(), im.latex.clone(), b));
                        if editing_inline.as_ref() != Some(&im.source) {
                            let _ =
                                window.paint_image(b, Corners::default(), im.img.clone(), 0, false);
                        }
                    }
                }
            }
        }

        if focus.is_focused(window)
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }

        // Table "+" affordances (issue #16): while the pointer is over a table's
        // hover zone, paint its add-row (below) + add-column (right) strips, cursor
        // them (unconditionally, so gpui applies the hand from its cached map as the
        // pointer moves onto a strip), and commit their rects for on_mouse_down. The
        // hovered strip fills; on_mouse_move drives the repaints (the editor
        // otherwise only repaints on the caret blink). Zones are committed every
        // frame so on_mouse_move knows where the tables are.
        let mouse = window.mouse_position();
        let mut table_hover_zones: Vec<Bounds<Pixels>> = Vec::new();
        let mut table_row_add_rects: Vec<(Bounds<Pixels>, usize)> = Vec::new();
        let mut table_col_add_rects: Vec<(Bounds<Pixels>, usize)> = Vec::new();
        for ta in &prepaint.table_adds {
            table_hover_zones.push(ta.zone);
            if !ta.zone.contains(&mouse) {
                continue;
            }
            paint_add_strip(ta.below, ta.border, ta.below.contains(&mouse), window);
            window.set_cursor_style(CursorStyle::PointingHand, &ta.below_hit);
            table_row_add_rects.push((ta.below, ta.below_row));
            paint_add_strip(ta.right, ta.border, ta.right.contains(&mouse), window);
            window.set_cursor_style(CursorStyle::PointingHand, &ta.right_hit);
            table_col_add_rects.push((ta.right, ta.right_row));
        }

        // Per-row / per-column delete "−" handles for the hovered cell (issue #16).
        let mut table_row_del = None;
        if let Some(d) = &prepaint.row_del {
            if d.show_highlight {
                let mut hi = d.border;
                hi.a = 0.10;
                window.paint_quad(fill(d.highlight, hi));
            }
            paint_del_handle(d.bounds, d.border, d.bounds.contains(&mouse), window);
            window.set_cursor_style(CursorStyle::PointingHand, &d.hit);
            table_row_del = Some((d.bounds, d.row));
        }
        let mut table_col_del = None;
        if let Some(d) = &prepaint.col_del {
            if d.show_highlight {
                let mut hi = d.border;
                hi.a = 0.10;
                window.paint_quad(fill(d.highlight, hi));
            }
            paint_del_handle(d.bounds, d.border, d.bounds.contains(&mouse), window);
            window.set_cursor_style(CursorStyle::PointingHand, &d.hit);
            table_col_del = Some((d.bounds, d.row, d.col));
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
        let table_rows = std::mem::take(&mut prepaint.tables);
        let line_insets: Vec<Pixels> = prepaint
            .backgrounds
            .iter()
            .zip(prepaint.marks.iter())
            .map(|(bg, mark)| row_inset(*bg, *mark))
            .collect();
        let chip_rows: Vec<Option<SharedString>> = prepaint
            .widgets
            .iter()
            .map(|w| match w {
                Some(Block::Chip { src, .. }) => Some(src.clone()),
                _ => None,
            })
            .collect();
        self.editor.update(cx, |editor, _| {
            editor.wrapped = wrapped;
            editor.line_tops = line_tops;
            editor.line_heights = line_heights;
            editor.widget_rows = widget_rows;
            editor.offset_maps = offset_maps;
            editor.chip_rows = chip_rows;
            editor.line_insets = line_insets;
            editor.table_rows = table_rows;
            editor.image_rects = image_rects;
            editor.inline_math_rects = inline_math_rects;
            editor.checkbox_rects = checkbox_rects;
            editor.table_row_add_rects = table_row_add_rects;
            editor.table_col_add_rects = table_col_add_rects;
            editor.table_hover_zones = table_hover_zones;
            editor.table_row_del = table_row_del;
            editor.table_col_del = table_col_del;
            editor.last_bounds = Some(bounds);
            editor.line_height = base_lh;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{display_col_in, set_image_width};

    #[test]
    fn display_col_leftmost_for_inline_math_spacer() {
        // An inline `$…$` spacer maps its whole width to the span's start offset (here source 2,
        // repeated across display 2..5). The caret at source 2 must land at the spacer's LEFT
        // edge (display 2), not an arbitrary spot inside it; source 5 (just past the formula)
        // lands at display 5.
        let map = vec![0, 1, 2, 2, 2, 5, 6, 7];
        assert_eq!(display_col_in(Some(&map), 2), 2);
        assert_eq!(display_col_in(Some(&map), 5), 5);
        // A strictly-increasing map (hidden markers) is unaffected.
        let plain = vec![0, 1, 2, 3];
        assert_eq!(display_col_in(Some(&plain), 2), 2);
        assert_eq!(display_col_in(None, 4), 4);
    }

    #[test]
    fn image_width_splice() {
        // No existing attr: append `{width=N}` right after `)`.
        assert_eq!(
            set_image_width("![a](b.png)", 200),
            "![a](b.png){width=200}"
        );
        // Existing `{width=N}` is replaced (not duplicated).
        assert_eq!(
            set_image_width("![a](b.png){width=320}", 200),
            "![a](b.png){width=200}"
        );
        // The `px` unit form is replaced too.
        assert_eq!(
            set_image_width("![a](b.png){width=320px}", 200),
            "![a](b.png){width=200}"
        );
        // List-item image: the leading marker is preserved, attr lands after `)`.
        assert_eq!(
            set_image_width("- ![](x){width=10}", 50),
            "- ![](x){width=50}"
        );
        // Trailing whitespace is preserved (attr lands before it).
        assert_eq!(
            set_image_width("![a](b.png)  ", 80),
            "![a](b.png){width=80}  "
        );
        // Not an image row: returned unchanged.
        assert_eq!(set_image_width("just text", 100), "just text");
    }
}
