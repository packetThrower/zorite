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

/// A structural math editor view: the model, the caret, and the cached raster.
pub struct MathEditor {
    root: Row,
    cursor: Cursor,
    focus: FocusHandle,
    font_size: f32,
    dpr: f32,
    rendered: Option<Rendered>,
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
        match ks.key.as_str() {
            "left" => self.cursor.move_left(&self.root),
            "right" => self.cursor.move_right(&self.root),
            "up" => self.cursor.move_up(&self.root),
            "down" => self.cursor.move_down(&self.root),
            "backspace" => self.cursor.backspace(&mut self.root),
            _ => match ks.key_char.as_ref().and_then(|s| s.chars().next()) {
                Some(c) => input::type_char(&mut self.root, &mut self.cursor, c),
                None => return,
            },
        }
        // Re-rasterize, freeing the previous image's GPU texture.
        let old = self.rendered.take();
        self.rendered = render::render_row(&self.root, self.font_size, self.dpr);
        if let Some(old) = old {
            cx.drop_image(old.image, Some(window));
        }
        cx.notify();
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
        let caret = self.caret_px().map(|(x, top, ch)| {
            div()
                .absolute()
                .left(px(x))
                .top(px(top))
                .w(px(2.0))
                .h(px(ch))
                .bg(rgb(0x2563eb))
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
                    .children(caret),
            )
    }
}
