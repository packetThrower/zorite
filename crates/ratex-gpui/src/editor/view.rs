//! The interactive gpui view — renders the formula + caret and turns keystrokes into
//! structural edits. This is the visual seam: it owns the `render` raster and the gpui
//! input, while all editing logic stays in the gpui-free `editor::{model, cursor, input,
//! geometry}`.

use crate::editor::cursor::Cursor;
use crate::editor::geometry;
use crate::editor::input;
use crate::editor::model::Row;
use crate::render::{self, PAD, Rendered};
use gpui::*;

/// How many autocomplete matches the dropdown shows / lets you select among.
const MAX_MATCHES: usize = 8;

/// A structural math editor view: the model, the caret, the cached raster, and an
/// in-progress `\command` buffer with autocomplete.
pub struct MathEditor {
    root: Row,
    cursor: Cursor,
    focus: FocusHandle,
    font_size: f32,
    dpr: f32,
    rendered: Option<Rendered>,
    /// The letters of a `\command` being typed (without the leading backslash), or `None`
    /// in normal mode.
    pending: Option<String>,
    /// Highlighted autocomplete match (index into the visible matches).
    selected: usize,
}

impl MathEditor {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            root: Row::new(),
            cursor: Cursor::start(),
            focus: cx.focus_handle(),
            font_size: 48.0,
            dpr: 2.0,
            rendered: None,
            pending: None,
            selected: 0,
        };
        this.rendered = render::render_row(&this.root, this.font_size, this.dpr);
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
        let consumed = if self.pending.is_some() {
            self.handle_pending(ks)
        } else {
            self.handle_normal(ks)
        };
        if !consumed {
            return;
        }
        // Re-rasterize, freeing the previous image's GPU texture.
        let old = self.rendered.take();
        self.rendered = render::render_row(&self.root, self.font_size, self.dpr);
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
                self.selected = (self.selected + 1)
                    .min(n.saturating_sub(1))
                    .min(MAX_MATCHES - 1);
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
        true
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

    /// The click-to-insert symbol palette (a floating panel). Shares the command table
    /// with `\command` typing, so a click is just a keyboard-free `commit_command`.
    fn palette(&self, cx: &mut Context<Self>) -> Div {
        div()
            .absolute()
            .top_4()
            .left_4()
            .flex()
            .flex_wrap()
            .w(px(200.0))
            .gap_1()
            .p_2()
            .bg(rgb(0xf8fafc))
            .border_1()
            .border_color(rgb(0xcbd5e1))
            .rounded_md()
            .children(input::PALETTE.iter().map(|(label, cmd)| {
                let cmd = *cmd;
                div()
                    .id(cmd)
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(32.0))
                    .bg(rgb(0xffffff))
                    .border_1()
                    .border_color(rgb(0xe2e8f0))
                    .rounded_md()
                    .text_size(px(17.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(0xeff6ff)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        input::commit_command(&mut this.root, &mut this.cursor, cmd);
                        let old = this.rendered.take();
                        this.rendered = render::render_row(&this.root, this.font_size, this.dpr);
                        if let Some(old) = old {
                            cx.drop_image(old.image, Some(window));
                        }
                        this.focus.focus(window, cx);
                        cx.notify();
                    }))
                    .child(*label)
            }))
    }
}

impl Render for MathEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .bg(rgb(0x2563eb))
            });
        let pending = self.pending.as_ref().map(|p| {
            let (x, top, _) = self.caret_px().unwrap_or((PAD, PAD, 0.0));
            div()
                .absolute()
                .left(px(x))
                .top(px(top))
                .px_1()
                .text_size(px(self.font_size * 0.42))
                .text_color(rgb(0x2563eb))
                .bg(rgb(0xeff6ff))
                .child(format!("\\{p}"))
        });
        let dropdown = self.pending.as_ref().and_then(|p| {
            let matches = input::command_matches(p);
            if matches.is_empty() {
                return None;
            }
            let (x, top, ch) = self.caret_px().unwrap_or((PAD, PAD, self.font_size));
            let selected = self.selected;
            Some(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(top + ch + 4.0))
                    .flex()
                    .flex_col()
                    .bg(rgb(0xffffff))
                    .border_1()
                    .border_color(rgb(0xcbd5e1))
                    .rounded_md()
                    .text_size(px(14.0))
                    .children(
                        matches
                            .iter()
                            .take(MAX_MATCHES)
                            .enumerate()
                            .map(|(i, name)| {
                                let row = div().px_2().py_1().child(format!("\\{name}"));
                                if i == selected {
                                    row.bg(rgb(0xdbeafe)).text_color(rgb(0x1d4ed8))
                                } else {
                                    row.text_color(rgb(0x334155))
                                }
                            }),
                    ),
            )
        });

        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .relative()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(0xffffff))
            .child(self.palette(cx))
            .child(
                div()
                    .relative()
                    .w(px(w))
                    .h(px(h))
                    .children(image)
                    .children(caret)
                    .children(pending)
                    .children(dropdown),
            )
    }
}
