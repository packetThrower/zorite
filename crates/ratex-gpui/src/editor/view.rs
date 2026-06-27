//! The interactive gpui view — renders the formula + caret and turns keystrokes into
//! structural edits. This is the visual seam: it owns the `render` raster and the gpui
//! input, while all editing logic stays in the gpui-free `editor::{model, cursor, input,
//! geometry}`.

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
}

/// A navigation signal the host listens for: the caret tried to move past a boundary of the
/// formula (`after` = past the end → seat the text caret after the block; else before it), so
/// focus should flow back out to the surrounding text editor — the way arrowing past a table
/// cell's edge exits the table.
pub enum MathNav {
    Exit { after: bool },
}

impl EventEmitter<MathNav> for MathEditor {}

impl MathEditor {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self::with_root(Row::new(), 48.0, false, MathTheme::default(), cx)
    }

    /// Build an editor seeded with the formula parsed from `latex`, rendered at `font_size`
    /// px/em — for editing an existing `$$…$$` block in-line at its displayed size. The caret
    /// lands at the end of the top row when `at_end`, else at the start — so arrowing *into*
    /// the block from below/right enters at the end, and from above/left enters at the start.
    pub fn from_latex(
        latex: &str,
        font_size: f32,
        at_end: bool,
        theme: MathTheme,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::with_root(
            crate::editor::latex::parse_latex(latex),
            font_size,
            true,
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

    fn with_root(
        root: Row,
        font_size: f32,
        inline: bool,
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
            focus: cx.focus_handle(),
            font_size,
            dpr: 2.0,
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
        };
        this.rendered = render::render_row(&this.root, this.font_size, this.dpr, this.theme.fg);
        this
    }

    /// The focus handle, so the host can focus the editor on open.
    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        if ks.modifiers.platform || ks.modifiers.control {
            return;
        }
        let was_normal = self.pending.is_none();
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
        // Re-rasterize, freeing the previous image's GPU texture.
        let old = self.rendered.take();
        self.rendered = render::render_row(&self.root, self.font_size, self.dpr, self.theme.fg);
        if let Some(old) = old {
            cx.drop_image(old.image, Some(window));
        }
        cx.notify();
    }

    /// Normal-mode keys: navigation, editing, typing, and `\` to start a command.
    fn handle_normal(&mut self, ks: &Keystroke) -> bool {
        match ks.key.as_str() {
            "left" => self.cursor.move_left(&self.root),
            "right" => self.cursor.move_right(&self.root),
            "up" => self.cursor.move_up(&self.root),
            "down" => self.cursor.move_down(&self.root),
            "backspace" => self.cursor.backspace(&mut self.root),
            _ => match ks.key_char.as_ref().and_then(|s| s.chars().next()) {
                Some('\\') => {
                    self.pending = Some(String::new());
                    self.selected = 0;
                    self.scroll_match_into_view();
                }
                Some(c) => input::type_char(&mut self.root, &mut self.cursor, c),
                None => return false,
            },
        }
        true
    }

    /// `\command`-mode keys: build the buffer, move the highlight, commit, or cancel.
    fn handle_pending(&mut self, ks: &Keystroke) -> bool {
        match ks.key.as_str() {
            "escape" => self.pending = None,
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

    /// Caret rect in logical px from the geometry walk (top row + fraction/script slots).
    /// `None` — hidden — for slots the walk doesn't handle yet (roots, delimiters, limits).
    fn caret_px(&self) -> Option<(f32, f32, f32)> {
        let r = geometry::caret_rect(&self.root, &self.cursor)?;
        let fs = self.font_size;
        let h = (r.h as f32 * fs).max(fs * 0.3);
        Some((PAD + r.x as f32 * fs, PAD + r.y as f32 * fs, h))
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

    /// The in-line palette's left, in image-container px: aligned with the matrix toolbar
    /// (which docks at the matrix's left) when the caret is in a matrix, else flush left.
    fn inline_palette_left(&self) -> f32 {
        if let Some(m) = geometry::matrix_rect(&self.root, &self.cursor) {
            PAD + m.x as f32 * self.font_size + self.toolbar_off.0
        } else {
            0.0
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
                        input::commit_command(&mut this.root, &mut this.cursor, cmd);
                        let old = this.rendered.take();
                        this.rendered =
                            render::render_row(&this.root, this.font_size, this.dpr, this.theme.fg);
                        if let Some(old) = old {
                            cx.drop_image(old.image, Some(window));
                        }
                        this.focus.focus(window, cx);
                        cx.notify();
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
            .w(px(200.0))
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
        // Dock at the matrix's bottom-left (image-container px) plus the draggable offset.
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
                    op(&mut this.cursor, &mut this.root);
                    let old = this.rendered.take();
                    this.rendered =
                        render::render_row(&this.root, this.font_size, this.dpr, this.theme.fg);
                    if let Some(old) = old {
                        cx.drop_image(old.image, Some(window));
                    }
                    this.focus.focus(window, cx);
                    cx.notify();
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .when(self.inline, |el| el.items_start().justify_start())
            .when(!self.inline, |el| {
                el.items_center().justify_center().bg(theme.panel)
            })
            .child(self.palette(cx))
            .child(
                div()
                    .relative()
                    .w(px(w))
                    .h(px(h))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, window, cx| {
                            // Capture clicks on the formula so they don't fall through to the
                            // host text editor — which would blur + close this in-line editor
                            // and drop the caret to the next line. Keep focus on the formula.
                            this.focus.focus(window, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .children(image)
                    .children(caret)
                    .children(pending)
                    .children(dropdown)
                    .children(self.matrix_toolbar(cx)),
            )
    }
}
