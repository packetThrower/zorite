//! The interactive gpui view — renders the formula + caret and turns keystrokes into
//! structural edits. This is the visual seam: it owns the `render` raster and the gpui
//! input, while all editing logic stays in the gpui-free `editor::{model, cursor, input,
//! geometry}`.

use std::cell::Cell;
use std::rc::Rc;

use crate::editor::cursor::Cursor;
use crate::editor::geometry;
use crate::editor::input;
use crate::editor::model::Row;
use crate::render::{self, PAD, Rendered};
use gpui::prelude::FluentBuilder;
use gpui::*;

/// Autocomplete dropdown geometry (px): row height + the scrollable height cap. Shared by
/// the dropdown render and `scroll_match_into_view` so the thumb + scroll-into-view agree.
const DROP_ITEM_H: f32 = 26.0;
const DROP_MAX_H: f32 = 240.0;

/// The host's theme colors for the editor chrome (palette, toolbar, caret, autocomplete) and
/// the formula glyphs, so the editor matches the surrounding app. Filled by the host from its
/// palette; `Default` is a light scheme for the standalone example.
#[derive(Clone, Copy)]
pub struct MathTheme {
    /// Formula glyphs + primary text (button labels).
    pub fg: Hsla,
    /// Secondary text — grips, dropdown rows.
    pub muted: Hsla,
    /// Panel + button surfaces (palette, toolbar, dropdown).
    pub panel: Hsla,
    /// Panel + button borders.
    pub border: Hsla,
    /// The caret and active/selected highlights.
    pub accent: Hsla,
    /// A subtle accent fill — hover, selected row, command preview.
    pub accent_bg: Hsla,
}

impl Default for MathTheme {
    fn default() -> Self {
        Self {
            fg: rgb(0x334155).into(),
            muted: rgb(0x64748b).into(),
            panel: rgb(0xf8fafc).into(),
            border: rgb(0xcbd5e1).into(),
            accent: rgb(0x2563eb).into(),
            accent_bg: rgb(0xeff6ff).into(),
        }
    }
}

/// A structural math editor view: the model, the caret, the cached raster, an in-progress
/// `\command` buffer with autocomplete, and the draggable palette's position.
pub struct MathEditor {
    root: Row,
    cursor: Cursor,
    /// The fixed end of a selection (the moving end is `cursor`); `None` = no selection.
    /// Always shares `cursor`'s row — selection is single-row for now.
    anchor: Option<Cursor>,
    focus: FocusHandle,
    font_size: f32,
    dpr: f32,
    /// The host's theme colors for the chrome + formula glyphs.
    theme: MathTheme,
    rendered: Option<Rendered>,
    /// The letters of a `\command` being typed (without the leading backslash), or `None`
    /// in normal mode.
    pending: Option<String>,
    /// Highlighted autocomplete match (index into the visible matches).
    selected: usize,
    /// Scroll offset of the open autocomplete dropdown, so a long match list scrolls to keep
    /// the highlight in view (keyboard nav can run past the height cap).
    match_scroll: ScrollHandle,
    /// The palette panel's top-left, in window px (draggable by its grip).
    palette_pos: (f32, f32),
    /// While dragging the palette: the (cursor − panel-origin) offset, kept for 1:1
    /// tracking with no jump on grab.
    palette_drag: Option<(f32, f32)>,
    /// The matrix toolbar's offset from the matrix's bottom-left, so it tracks the grid as
    /// the formula reflows; adjustable by dragging the toolbar's grip.
    toolbar_off: (f32, f32),
    /// During a toolbar drag: the previous cursor position, for delta-based movement.
    toolbar_drag: Option<(f32, f32)>,
    /// In-line edit mode (hosted in a note's text flow): left-align the formula at its spot
    /// and hide the floating palette + white background, vs the centered standalone editor.
    inline: bool,
    /// Horizontal justification of the formula in in-line mode — matches the display block's
    /// alignment so entering edit doesn't shift it.
    align: MathAlign,
    /// Undo / redo history: `(model, caret)` snapshots taken before each edit. The host
    /// editor's Cmd+Z is inert while we're hosted (its key context is dropped), so the
    /// formula owns its own in-place undo. A committed formula is one step in the document's
    /// history (the host records it), so the two levels compose.
    undo_stack: Vec<(Row, Cursor)>,
    redo_stack: Vec<(Row, Cursor)>,
    /// The formula image's window-space bounds, captured each paint by a `canvas` overlay so a
    /// click can be mapped to a position in the formula. `Cell` since the capture closure has
    /// no `&mut self`.
    img_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

/// Palette panel width (px) — used to dock a right-aligned formula's palette to its right edge.
const PALETTE_W: f32 = 200.0;

/// Cap on the formula's in-place undo history, to bound memory.
const UNDO_CAP: usize = 200;

/// Horizontal alignment of the in-line formula, so it matches the centered (or left/right)
/// display block and doesn't jump when entered. The host maps its own marker to this.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum MathAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// A signal the host listens for from the hosted editor.
pub enum MathNav {
    /// The caret tried to move past a boundary of the formula (`after` = past the end → seat
    /// the text caret after the block; else before it), so focus should flow back out to the
    /// surrounding text editor — the way arrowing past a table cell's edge exits the table.
    Exit { after: bool },
    /// The formula was right-clicked while being edited — the host shows its formula context
    /// menu (copy LaTeX / export) at `position` (window-space). The hosted editor occludes the
    /// formula, so the host's own right-click handler can't fire; this routes it back out.
    ContextMenu { position: Point<Pixels> },
}

impl EventEmitter<MathNav> for MathEditor {}

impl MathEditor {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self::with_root(
            Row::new(),
            48.0,
            false,
            MathAlign::Center,
            MathTheme::default(),
            cx,
        )
    }

    /// Build an editor seeded with the formula parsed from `latex`, rendered at `font_size`
    /// px/em — for editing an existing `$$…$$` block in-line at its displayed size. The caret
    /// lands at the end of the top row when `at_end`, else at the start — so arrowing *into*
    /// the block from below/right enters at the end, and from above/left enters at the start.
    /// `align` matches the display block's justification so entering edit doesn't shift it.
    pub fn from_latex(
        latex: &str,
        font_size: f32,
        at_end: bool,
        align: MathAlign,
        theme: MathTheme,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::with_root(
            crate::editor::latex::parse_latex(latex),
            font_size,
            true,
            align,
            theme,
            cx,
        );
        if !at_end {
            this.cursor = Cursor::start();
        }
        this
    }

    /// The current formula as LaTeX, to write back into the `$$…$$` block.
    pub fn to_latex(&self) -> String {
        self.root.to_latex()
    }

    /// The current horizontal alignment, so the host writes the matching marker on commit.
    pub fn align(&self) -> MathAlign {
        self.align
    }

    /// Re-justify the in-line formula (from the right-click "Align" menu) — immediate visual
    /// feedback; the host persists the marker on commit.
    pub fn set_align(&mut self, align: MathAlign, cx: &mut Context<Self>) {
        self.align = align;
        cx.notify();
    }

    fn with_root(
        root: Row,
        font_size: f32,
        inline: bool,
        align: MathAlign,
        theme: MathTheme,
        cx: &mut Context<Self>,
    ) -> Self {
        let index = root.atoms.len();
        let mut this = Self {
            root,
            cursor: Cursor {
                path: vec![],
                index,
            },
            anchor: None,
            focus: cx.focus_handle(),
            font_size,
            // Corrected to the window's scale factor on first render.
            dpr: 2.0,
            align,
            theme,
            rendered: None,
            pending: None,
            selected: 0,
            match_scroll: ScrollHandle::new(),
            palette_pos: (16.0, 16.0),
            palette_drag: None,
            toolbar_off: (0.0, 8.0),
            toolbar_drag: None,
            inline,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            img_bounds: Rc::new(Cell::new(None)),
        };
        this.rendered = render::render_row(&this.root, this.font_size, this.dpr, this.theme.fg);
        this
    }

    /// The focus handle, so the host can focus the editor on open.
    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// Re-rasterize the formula, freeing the previous image's GPU texture, and notify.
    fn rerender(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let old = self.rendered.take();
        self.rendered = render::render_row(&self.root, self.font_size, self.dpr, self.theme.fg);
        if let Some(old) = old {
            cx.drop_image(old.image, Some(window));
        }
        cx.notify();
    }

    /// Finish a possibly-mutating action: if it changed the model, record `before` for undo
    /// (clearing the redo branch), then re-rasterize. Caret-only moves don't record a step.
    fn commit_edit(&mut self, before: (Row, Cursor), window: &mut Window, cx: &mut Context<Self>) {
        if self.root != before.0 {
            self.undo_stack.push(before);
            self.redo_stack.clear();
            if self.undo_stack.len() > UNDO_CAP {
                self.undo_stack.remove(0);
            }
        }
        self.rerender(window, cx);
    }

    /// Restore the previous snapshot (pushing the current one onto the redo branch). `false`
    /// if there's nothing to undo.
    fn undo(&mut self) -> bool {
        let Some((root, cursor)) = self.undo_stack.pop() else {
            return false;
        };
        let prev = (
            std::mem::replace(&mut self.root, root),
            std::mem::replace(&mut self.cursor, cursor),
        );
        self.redo_stack.push(prev);
        self.anchor = None;
        self.pending = None;
        true
    }

    /// Re-apply the last undone snapshot. `false` if there's nothing to redo.
    fn redo(&mut self) -> bool {
        let Some((root, cursor)) = self.redo_stack.pop() else {
            return false;
        };
        let prev = (
            std::mem::replace(&mut self.root, root),
            std::mem::replace(&mut self.cursor, cursor),
        );
        self.undo_stack.push(prev);
        self.anchor = None;
        self.pending = None;
        true
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        if ks.modifiers.platform || ks.modifiers.control {
            // Undo / redo within the formula (Cmd/Ctrl+Z, Cmd+Shift+Z, Cmd/Ctrl+Y). The host
            // editor's binding is inert while we're hosted (its key context is dropped), so
            // the formula handles it. Other modified keys are left for the host.
            if !ks.modifiers.alt && !ks.modifiers.function {
                let did = match ks.key.as_str() {
                    "z" if ks.modifiers.shift => self.redo(),
                    "z" => self.undo(),
                    "y" => self.redo(),
                    _ => false,
                };
                if did {
                    self.rerender(window, cx);
                }
            }
            return;
        }
        // Escape backs out one layer at a time: cancel a pending `\command`, else clear a
        // selection, else exit the formula entirely (commit + flow the caret out, like
        // arrowing past the end).
        if ks.key == "escape" {
            if self.pending.is_some() {
                self.pending = None;
                cx.notify();
            } else if self.anchor.is_some() {
                self.anchor = None;
                cx.notify();
            } else {
                cx.emit(MathNav::Exit { after: true });
            }
            return;
        }
        let was_normal = self.pending.is_none();
        let root_before = self.root.clone();
        let cursor_before = self.cursor.clone();
        let consumed = if self.pending.is_some() {
            self.handle_pending(ks)
        } else {
            self.handle_normal(ks)
        };
        // An arrow that left the caret unmoved in normal mode is a boundary: hand focus back
        // to the host so the text caret flows out of the formula (left/up → before the block,
        // right/down → after it), the way arrowing past a table cell's edge exits the table.
        if was_normal
            && !ks.modifiers.shift
            && self.cursor == cursor_before
            && let Some(after) = match ks.key.as_str() {
                "left" | "up" => Some(false),
                "right" | "down" => Some(true),
                _ => None,
            }
        {
            cx.emit(MathNav::Exit { after });
            return;
        }
        if !consumed {
            return;
        }
        // Record an undo step iff the model actually changed (so navigation / selection
        // don't), then re-rasterize.
        self.commit_edit((root_before, cursor_before), window, cx);
    }

    /// Normal-mode keys: navigation, selection (Shift+←/→), editing, typing, `\` to start a
    /// command, and wrapping a selection — `(` → parens, `/` → fraction.
    fn handle_normal(&mut self, ks: &Keystroke) -> bool {
        let shift = ks.modifiers.shift;
        let sel = self.selection_range();
        match ks.key.as_str() {
            "left" if shift => self.select_step(false),
            "right" if shift => self.select_step(true),
            "left" => {
                self.anchor = None;
                self.cursor.move_left(&self.root);
            }
            "right" => {
                self.anchor = None;
                self.cursor.move_right(&self.root);
            }
            "up" => {
                self.anchor = None;
                self.cursor.move_up(&self.root);
            }
            "down" => {
                self.anchor = None;
                self.cursor.move_down(&self.root);
            }
            "backspace" => match sel {
                Some((lo, hi)) => {
                    self.cursor.delete_range(&mut self.root, lo, hi);
                    self.anchor = None;
                }
                None => self.cursor.backspace(&mut self.root),
            },
            _ => match ks.key_char.as_ref().and_then(|s| s.chars().next()) {
                Some('\\') => {
                    self.anchor = None;
                    self.pending = Some(String::new());
                    self.selected = 0;
                    self.scroll_match_into_view();
                }
                Some('/') if sel.is_some() => {
                    let (lo, hi) = sel.unwrap();
                    self.cursor.wrap_fraction(&mut self.root, lo, hi);
                    self.anchor = None;
                }
                // A bracket / brace / bar typed over a selection wraps it in that delimiter.
                Some(c) if sel.is_some() && input::delim_pair(c).is_some() => {
                    let (open, close) = input::delim_pair(c).unwrap();
                    let (lo, hi) = sel.unwrap();
                    self.cursor.wrap_delim(&mut self.root, lo, hi, open, close);
                    self.anchor = None;
                }
                // Any other character collapses the selection (non-destructively — there's
                // no undo) and inserts at the caret.
                Some(c) => {
                    self.anchor = None;
                    input::type_char(&mut self.root, &mut self.cursor, c);
                }
                None => return false,
            },
        }
        true
    }

    /// Extend (or begin) the selection by one atom within the current row, in `right` /
    /// left direction. Single-row: the moving end stays in `cursor`'s row, never descending.
    fn select_step(&mut self, right: bool) {
        if self.anchor.is_none() {
            self.anchor = Some(self.cursor.clone());
        }
        let len = self.cursor.row(&self.root).atoms.len();
        if right {
            if self.cursor.index < len {
                self.cursor.index += 1;
            }
        } else if self.cursor.index > 0 {
            self.cursor.index -= 1;
        }
    }

    /// The selected atom range `lo..hi` in the current row, or `None` when there's no
    /// (non-empty) selection. Guards that the anchor shares the cursor's row.
    fn selection_range(&self) -> Option<(usize, usize)> {
        let a = self.anchor.as_ref()?;
        if a.path != self.cursor.path {
            return None;
        }
        let lo = a.index.min(self.cursor.index);
        let hi = a.index.max(self.cursor.index);
        (lo < hi).then_some((lo, hi))
    }

    /// `\command`-mode keys: build the buffer, move the highlight, commit, or cancel. (Escape
    /// cancels, but `on_key` intercepts it before this is reached.)
    fn handle_pending(&mut self, ks: &Keystroke) -> bool {
        match ks.key.as_str() {
            "enter" | "tab" | "space" => self.commit_pending(),
            "up" => self.selected = self.selected.saturating_sub(1),
            "down" => {
                let n = self
                    .pending
                    .as_deref()
                    .map_or(0, |p| input::command_matches(p).len());
                self.selected = (self.selected + 1).min(n.saturating_sub(1));
            }
            "backspace" => {
                if self.pending.as_deref().is_some_and(|b| !b.is_empty()) {
                    self.pending.as_mut().unwrap().pop();
                } else {
                    self.pending = None; // backspaced past the '\'
                }
                self.selected = 0;
            }
            _ => match ks.key_char.as_ref().and_then(|s| s.chars().next()) {
                Some(c) if c.is_ascii_alphabetic() => {
                    self.pending.as_mut().unwrap().push(c);
                    self.selected = 0;
                }
                _ => return false,
            },
        }
        self.scroll_match_into_view();
        true
    }

    /// Scroll the autocomplete dropdown so the highlighted match stays visible — the list can
    /// run past the height cap (the full `\` menu is ~75 entries). Mirrors the host slash menu.
    fn scroll_match_into_view(&self) {
        let top = self.selected as f32 * DROP_ITEM_H;
        let bot = top + DROP_ITEM_H;
        let cur = -f32::from(self.match_scroll.offset().y);
        let new = if top < cur {
            top
        } else if bot > cur + DROP_MAX_H {
            bot - DROP_MAX_H
        } else {
            return;
        };
        self.match_scroll.set_offset(point(px(0.0), px(-new)));
    }

    /// Resolve the pending `\name`: the highlighted match, else the literal letters.
    fn commit_pending(&mut self) {
        let name = self.pending.take().unwrap_or_default();
        let matches = input::command_matches(&name);
        match matches.get(self.selected).or_else(|| matches.first()) {
            Some(&chosen) => {
                input::commit_command(&mut self.root, &mut self.cursor, chosen);
            }
            None => {
                for c in name.chars() {
                    input::type_char(&mut self.root, &mut self.cursor, c);
                }
            }
        }
    }

    /// Caret rect (left x, top y, height) in logical px for `cursor`, from the geometry walk
    /// (top row + fraction/script slots). `None` for slots the walk doesn't handle yet.
    fn caret_px_of(&self, cursor: &Cursor) -> Option<(f32, f32, f32)> {
        let r = geometry::caret_rect(&self.root, cursor)?;
        let fs = self.font_size;
        let h = (r.h as f32 * fs).max(fs * 0.3);
        Some((PAD + r.x as f32 * fs, PAD + r.y as f32 * fs, h))
    }

    /// Caret rect for the live cursor.
    fn caret_px(&self) -> Option<(f32, f32, f32)> {
        self.caret_px_of(&self.cursor)
    }

    /// A window-space click in formula em-coords (the geometry walk's units): undo the render
    /// padding + layout font size, mirroring `caret_px_of`. `None` until the first paint has
    /// captured the formula's bounds.
    fn em_at(&self, pos: Point<Pixels>) -> Option<(f64, f64)> {
        let b = self.img_bounds.get()?;
        let ex = ((f32::from(pos.x - b.origin.x) - PAD) / self.font_size) as f64;
        let ey = ((f32::from(pos.y - b.origin.y) - PAD) / self.font_size) as f64;
        Some((ex, ey))
    }

    /// Single click: place the caret at the click, collapsing any selection / pending command.
    fn click_to_caret(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) {
        let Some((ex, ey)) = self.em_at(pos) else {
            return;
        };
        self.anchor = None;
        self.pending = None;
        self.cursor = geometry::cursor_at(&self.root, ex, ey);
        cx.notify();
    }

    /// Double click: select the atom (or structure) under the click.
    fn select_cell_at(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) {
        let Some((ex, ey)) = self.em_at(pos) else {
            return;
        };
        let (path, lo, hi) = geometry::span_at(&self.root, ex, ey);
        if lo == hi {
            self.click_to_caret(pos, cx); // empty row → just place the caret
            return;
        }
        self.pending = None;
        self.anchor = Some(Cursor {
            path: path.clone(),
            index: lo,
        });
        self.cursor = Cursor { path, index: hi };
        cx.notify();
    }

    /// Triple click: select the whole row / slot under the click.
    fn select_row_at(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) {
        let Some((ex, ey)) = self.em_at(pos) else {
            return;
        };
        let (path, len) = geometry::row_len_at(&self.root, ex, ey);
        if len == 0 {
            self.click_to_caret(pos, cx);
            return;
        }
        self.pending = None;
        self.anchor = Some(Cursor {
            path: path.clone(),
            index: 0,
        });
        self.cursor = Cursor { path, index: len };
        cx.notify();
    }

    /// The click-to-insert symbol palette (a floating, draggable panel). Shares the command
    /// table with `\command` typing, so a click is just a keyboard-free `commit_command`.
    /// The in-line palette's top, in image-container px: just below the formula normally, but
    /// below the matrix toolbar when the caret is in a matrix (else the two panels overlap).
    fn inline_palette_top(&self) -> f32 {
        if let Some(m) = geometry::matrix_rect(&self.root, &self.cursor) {
            // Clear the toolbar, which docks at the matrix's bottom (see `matrix_toolbar`):
            // its dock top + the toolbar's own height (24px row + padding) + a small gap.
            PAD + (m.y + m.h) as f32 * self.font_size + self.toolbar_off.1 + 40.0
        } else {
            self.rendered.as_ref().map_or(0.0, |r| r.height) + 4.0
        }
    }

    /// The in-line palette's left, in formula-container px. The palette is a child of the
    /// (flex-justified) formula container, so this is formula-relative and the panel tracks the
    /// formula automatically. For a RIGHT-aligned formula it docks at the formula's RIGHT edge
    /// (extends left), so its right edge sits at the formula's right (≈ the editor's right) and
    /// can't run off-screen — computed from the formula width, no painted-position measurement
    /// (which would lag a frame). Left/center dock at the formula's left; a matrix docks beside
    /// the grid.
    fn inline_palette_left(&self) -> f32 {
        if let Some(m) = geometry::matrix_rect(&self.root, &self.cursor) {
            return PAD + m.x as f32 * self.font_size + self.toolbar_off.0;
        }
        match self.align {
            MathAlign::Right => self.rendered.as_ref().map_or(0.0, |r| r.width) - PALETTE_W,
            _ => 0.0,
        }
    }

    fn palette(&self, cx: &mut Context<Self>) -> Div {
        let theme = self.theme;
        // The grip "ear": press and hold here to move the panel.
        let handle = div()
            .id("palette-handle")
            .flex()
            .items_center()
            .justify_center()
            .w_full()
            .h(px(16.0))
            .bg(theme.border)
            .cursor_pointer()
            .text_size(px(11.0))
            .text_color(theme.muted)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _window, cx| {
                    this.palette_drag = Some((
                        f32::from(ev.position.x) - this.palette_pos.0,
                        f32::from(ev.position.y) - this.palette_pos.1,
                    ));
                    cx.notify();
                }),
            )
            .child("⠿ ⠿ ⠿");

        let buttons = div()
            .flex()
            .flex_wrap()
            .gap_1()
            .p_2()
            .children(input::PALETTE.iter().map(|(label, cmd)| {
                let cmd = *cmd;
                div()
                    .id(cmd)
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(32.0))
                    .bg(theme.panel)
                    .border_1()
                    .border_color(theme.border)
                    .rounded_md()
                    .text_size(px(17.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.accent_bg))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        // With a selection active, the fraction / root buttons WRAP it (it
                        // becomes the numerator / radicand); other buttons just insert.
                        let before = (this.root.clone(), this.cursor.clone());
                        let sel = this.selection_range();
                        input::commit_command_selecting(&mut this.root, &mut this.cursor, cmd, sel);
                        this.anchor = None;
                        this.focus.focus(window, cx);
                        this.commit_edit(before, window, cx);
                    }))
                    .child(*label)
            }));

        div()
            .absolute()
            .left(px(if self.inline {
                self.inline_palette_left()
            } else {
                self.palette_pos.0
            }))
            .top(px(if self.inline {
                self.inline_palette_top()
            } else {
                self.palette_pos.1
            }))
            .flex()
            .flex_col()
            .w(px(PALETTE_W))
            .bg(theme.panel)
            .border_1()
            .border_color(theme.border)
            .rounded_md()
            // Occlude: in-line the palette floats below the formula, outside the host's
            // reserved gap — without this, glyph clicks fall through to the text editor,
            // which seats the caret on the next line and closes this editor (insert lost).
            .occlude()
            .child(handle)
            .child(buttons)
    }

    /// A small contextual toolbar — shown only when the caret is in a matrix — docked just
    /// below the grid so it stays near the matrix (vital once a formula is embedded in a
    /// doc). Draggable by its grip; columns have no natural keyboard gesture, so it's also
    /// the discoverable way to grow/shrink width, and it doubles as row/column removal.
    fn matrix_toolbar(&self, cx: &mut Context<Self>) -> Option<Div> {
        let theme = self.theme;
        let m = geometry::matrix_rect(&self.root, &self.cursor)?;
        let fs = self.font_size;
        // Dock at the matrix's bottom-left (formula-container px) plus the draggable offset.
        let left = PAD + m.x as f32 * fs + self.toolbar_off.0;
        let top = PAD + (m.y + m.h) as f32 * fs + self.toolbar_off.1;

        // The grip "ear": press and hold to move the toolbar.
        let grip = div()
            .id("matrix-toolbar-handle")
            .flex()
            .items_center()
            .justify_center()
            .px_1()
            .h(px(24.0))
            .bg(theme.border)
            .cursor_pointer()
            .text_size(px(11.0))
            .text_color(theme.muted)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _window, cx| {
                    this.toolbar_drag = Some((f32::from(ev.position.x), f32::from(ev.position.y)));
                    cx.notify();
                }),
            )
            .child("⠿");

        let btn = |label: &'static str, op: fn(&mut Cursor, &mut Row)| {
            div()
                .id(label)
                .flex()
                .items_center()
                .justify_center()
                .px_2()
                .h(px(24.0))
                .bg(theme.panel)
                .border_1()
                .border_color(theme.border)
                .rounded_md()
                .text_size(px(13.0))
                .text_color(theme.fg)
                .cursor_pointer()
                .hover(|s| s.bg(theme.accent_bg))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    let before = (this.root.clone(), this.cursor.clone());
                    op(&mut this.cursor, &mut this.root);
                    this.focus.focus(window, cx);
                    this.commit_edit(before, window, cx);
                }))
                .child(label)
        };
        Some(
            div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .flex()
                .gap_1()
                .p_1()
                .bg(theme.panel)
                .border_1()
                .border_color(theme.border)
                .rounded_md()
                // Occlude — same overflow as the palette: it floats below the matrix.
                .occlude()
                .child(grip)
                .child(btn("+ row", Cursor::matrix_add_row))
                .child(btn("− row", Cursor::matrix_remove_row))
                .child(btn("+ col", Cursor::matrix_add_col))
                .child(btn("− col", Cursor::matrix_remove_col)),
        )
    }
}

impl Render for MathEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Rasterize at the window's real pixel density — the construction
        // default (2.0) is only right on a 2× display; a 1× screen rendered
        // soft and a 3× wasted texture. Guarded, so this settles in one
        // extra frame after a display change.
        let dpr = window.scale_factor().max(1.0);
        if (dpr - self.dpr).abs() > f32::EPSILON {
            self.dpr = dpr;
            self.rerender(window, cx);
        }
        let theme = self.theme;
        let (w, h) = self
            .rendered
            .as_ref()
            .map_or((40.0, 40.0), |r| (r.width, r.height));
        let image = self
            .rendered
            .as_ref()
            .map(|r| img(r.image.clone()).w(px(w)).h(px(h)));

        // In normal mode show the caret bar; while typing a \command show the pending text
        // (and an autocomplete dropdown) at the caret instead.
        let caret = self
            .pending
            .is_none()
            .then(|| self.caret_px())
            .flatten()
            .map(|(x, top, ch)| {
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(top))
                    .w(px(2.0))
                    .h(px(ch))
                    .bg(theme.accent)
            });

        // Selection highlight: a translucent band from the anchor caret to the live caret
        // (same row), painted behind the formula glyphs.
        let selection = self.selection_range().is_some().then(|| {
            let a = self.anchor.as_ref()?;
            let (ax, _, _) = self.caret_px_of(a)?;
            let (fx, top, ch) = self.caret_px()?;
            let left = ax.min(fx);
            let w = (ax - fx).abs();
            let mut fill = theme.accent;
            fill.a = 0.22;
            (w > 0.5).then(|| {
                div()
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(w))
                    .h(px(ch))
                    .rounded(px(2.0))
                    .bg(fill)
            })
        });
        let selection = selection.flatten();

        let pending = self.pending.as_ref().map(|p| {
            let (x, top, _) = self.caret_px().unwrap_or((PAD, PAD, 0.0));
            div()
                .absolute()
                .left(px(x))
                .top(px(top))
                .px_1()
                .text_size(px(self.font_size * 0.42))
                .text_color(theme.accent)
                .bg(theme.accent_bg)
                .child(format!("\\{p}"))
        });
        let dropdown = self.pending.as_ref().and_then(|p| {
            let matches = input::command_matches(p);
            if matches.is_empty() {
                return None;
            }
            let (x, top, ch) = self.caret_px().unwrap_or((PAD, PAD, self.font_size));
            let selected = self.selected;

            // Inner scroll viewport: the full match list (no cap) scrolls within DROP_MAX_H.
            let viewport = div()
                .id("ratex-cmd-menu")
                .max_h(px(DROP_MAX_H))
                .overflow_y_scroll()
                .track_scroll(&self.match_scroll)
                .flex()
                .flex_col()
                .children(matches.iter().enumerate().map(|(i, name)| {
                    let row = div().px_2().py_1().child(format!("\\{name}"));
                    if i == selected {
                        row.bg(theme.accent_bg).text_color(theme.accent)
                    } else {
                        row.text_color(theme.fg)
                    }
                }));

            // Scrollbar thumb, shown only when the rows overflow the cap — sized from the
            // content height + positioned from the live offset (mirrors the host slash menu).
            let rows_h = matches.len() as f32 * DROP_ITEM_H;
            let thumb = (rows_h > DROP_MAX_H).then(|| {
                let scrolled =
                    (-f32::from(self.match_scroll.offset().y)).clamp(0.0, rows_h - DROP_MAX_H);
                let thumb_h = (DROP_MAX_H * DROP_MAX_H / rows_h).max(24.0);
                let thumb_top = scrolled / (rows_h - DROP_MAX_H) * (DROP_MAX_H - thumb_h);
                let mut thumb_c = theme.muted;
                thumb_c.a = 0.5;
                div()
                    .absolute()
                    .top(px(thumb_top))
                    .right(px(2.0))
                    .w(px(5.0))
                    .h(px(thumb_h))
                    .rounded(px(3.0))
                    .bg(thumb_c)
            });

            Some(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(top + ch + 4.0))
                    .occlude()
                    .bg(theme.panel)
                    .border_1()
                    .border_color(theme.border)
                    .rounded_md()
                    .overflow_hidden()
                    .text_size(px(14.0))
                    .child(viewport)
                    .children(thumb),
            )
        });

        // Records the formula's window-space bounds each paint, so a click can be mapped to a
        // caret position (the closure has no `&mut self`, hence the shared `Cell`).
        let bounds_cell = self.img_bounds.clone();
        let bounds_probe = canvas(
            move |bounds: Bounds<Pixels>, _window, _cx| bounds_cell.set(Some(bounds)),
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0();
        // In-line: the palette lives inside the (flex-justified) formula container so it tracks
        // a centered / right-aligned formula. Standalone: it's a draggable root child.
        let palette = self.palette(cx);
        let (root_palette, inner_palette) = if self.inline {
            (None, Some(palette))
        } else {
            (Some(palette), None)
        };

        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _window, cx| {
                let (mx, my) = (f32::from(ev.position.x), f32::from(ev.position.y));
                if let Some((ox, oy)) = this.palette_drag {
                    this.palette_pos = (mx - ox, my - oy);
                    cx.notify();
                } else if let Some((lx, ly)) = this.toolbar_drag {
                    this.toolbar_off.0 += mx - lx;
                    this.toolbar_off.1 += my - ly;
                    this.toolbar_drag = Some((mx, my));
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _window, cx| {
                    let dragged = this.palette_drag.take().is_some();
                    let dragged = this.toolbar_drag.take().is_some() || dragged;
                    if dragged {
                        cx.notify();
                    }
                }),
            )
            .relative()
            .size_full()
            .flex()
            // In-line: top-aligned, justified to match the display block so entering edit
            // doesn't shift the formula. Standalone: fully centered.
            .when(self.inline, |el| {
                let el = el.items_start();
                match self.align {
                    MathAlign::Left => el.justify_start(),
                    MathAlign::Center => el.justify_center(),
                    MathAlign::Right => el.justify_end(),
                }
            })
            .when(!self.inline, |el| {
                el.items_center().justify_center().bg(theme.panel)
            })
            .children(root_palette)
            .child(
                div()
                    .relative()
                    .w(px(w))
                    .h(px(h))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                            // Capture clicks on the formula so they don't fall through to the
                            // host text editor — which would blur + close this in-line editor
                            // and drop the caret to the next line. Keep focus on the formula.
                            this.focus.focus(window, cx);
                            cx.stop_propagation();
                            // macOS Control-click is a secondary click (delivered as left +
                            // control, not a right button) → the formula menu, like right-click.
                            if ev.modifiers.control {
                                cx.emit(MathNav::ContextMenu {
                                    position: ev.position,
                                });
                                return;
                            }
                            // 1 click → caret, 2 → select the atom, 3+ → select the row/slot.
                            match ev.click_count {
                                1 => this.click_to_caret(ev.position, cx),
                                2 => this.select_cell_at(ev.position, cx),
                                _ => this.select_row_at(ev.position, cx),
                            }
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                            // Right-click while editing → ask the host for the formula menu
                            // (copy LaTeX / export). Keep focus + swallow the press so it
                            // doesn't blur the editor.
                            this.focus.focus(window, cx);
                            cx.stop_propagation();
                            cx.emit(MathNav::ContextMenu {
                                position: ev.position,
                            });
                        }),
                    )
                    // Capture the formula's window-space bounds for click-to-caret mapping.
                    .child(bounds_probe)
                    // Selection band first, so it paints behind the formula glyphs.
                    .children(selection)
                    .children(image)
                    .children(caret)
                    .children(pending)
                    .children(dropdown)
                    // In-line palette + matrix toolbar live in the container, so they track the
                    // formula's centered / right-aligned position automatically.
                    .children(inner_palette)
                    .children(self.matrix_toolbar(cx)),
            )
    }
}
