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

/// A structural math editor view: the model, the caret, the cached raster, and an
/// in-progress `\command` buffer.
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
                Some('\\') => self.pending = Some(String::new()),
                Some(c) => input::type_char(&mut self.root, &mut self.cursor, c),
                None => return false,
            },
        }
        true
    }

    /// `\command`-mode keys: build the buffer, commit, or cancel.
    fn handle_pending(&mut self, ks: &Keystroke) -> bool {
        match ks.key.as_str() {
            "escape" => self.pending = None,
            "enter" | "tab" | "space" => self.commit_pending(),
            "backspace" => {
                if self.pending.as_deref().is_some_and(|b| !b.is_empty()) {
                    self.pending.as_mut().unwrap().pop();
                } else {
                    self.pending = None; // backspaced past the '\'
                }
            }
            _ => match ks.key_char.as_ref().and_then(|s| s.chars().next()) {
                Some(c) if c.is_ascii_alphabetic() => self.pending.as_mut().unwrap().push(c),
                _ => return false,
            },
        }
        true
    }

    /// Resolve the pending `\name`: exact command, else the top autocomplete match, else
    /// fall back to inserting the literal letters as symbols.
    fn commit_pending(&mut self) {
        let name = self.pending.take().unwrap_or_default();
        if input::commit_command(&mut self.root, &mut self.cursor, &name) {
            return;
        }
        if let Some(best) = input::command_matches(&name).first() {
            input::commit_command(&mut self.root, &mut self.cursor, best);
        } else {
            for c in name.chars() {
                input::type_char(&mut self.root, &mut self.cursor, c);
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
        // at the caret instead.
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

        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(0xffffff))
            .child(
                div()
                    .relative()
                    .w(px(w))
                    .h(px(h))
                    .children(image)
                    .children(caret)
                    .children(pending),
            )
    }
}
