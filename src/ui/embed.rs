//! The WYSIWYG transclusion view: one entity per `![[target]]`, overlaid by
//! gpui-editor in the gap its line reserves (see `EditorState::set_embed_provider`).
//! Mirrors the reader's embed box — a quoted border, a small clickable source
//! label, and the target content rendered as markdown — with the content
//! scrolling inside when it outgrows the reserved height. The host resolves
//! targets and keeps these fresh in [`crate::app::AppView::ensure_content_embeds`].

use std::collections::HashMap;

use gpui::{
    Context, Entity, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement,
    Pixels, Render, SharedString, StatefulInteractiveElement, Styled, WeakEntity, Window, div, px,
};

use crate::app::AppView;
use crate::theme;

/// `raw inner target` → (view, reserved height) — shared between the app (which
/// fills it) and the editors' embed providers (which read it).
pub type EmbedStore = HashMap<String, (Entity<EmbedView>, f32)>;

pub struct EmbedView {
    /// The navigation target (alias stripped) the label click opens.
    pub nav_target: SharedString,
    pub label: SharedString,
    pub content: SharedString,
    pub text_size: Pixels,
    pub list_indent: usize,
    pub app: WeakEntity<AppView>,
    /// Body scroll state — drives the hover-revealed scrollbar thumb when the
    /// content outgrows the reserved height.
    pub scroll: gpui::ScrollHandle,
    pub hovered: bool,
    /// The full renderer set, so embedded content shows images (read-only —
    /// resizing would rewrite the wrong page), mermaid, math, and highlighted
    /// code just like the note it came from. Built by `upsert_embed`.
    pub image: gpui_markdown::ImageRenderer,
    pub mermaid: gpui_markdown::MermaidRenderer,
    pub math: gpui_markdown::MathRenderer,
    pub inline_math: gpui_markdown::InlineMathRenderer,
    pub highlight: gpui_markdown::CodeHighlighter,
    /// Pre-resolved nested embeds (`![[…]]` inside this embed's content).
    pub nested: std::rc::Rc<HashMap<String, (SharedString, SharedString)>>,
}

impl Render for EmbedView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = self.app.clone();
        let nav = self.nav_target.clone();
        let wiki_app = self.app.clone();
        // Hover-revealed scrollbar thumb, exact from the handle's last-painted
        // geometry (content height = viewport + max scroll).
        let thumb = {
            let viewport = f32::from(self.scroll.bounds().size.height);
            let max = f32::from(self.scroll.max_offset().y);
            (self.hovered && max > 1.0 && viewport > 0.0).then(|| {
                let content = viewport + max;
                let scrolled = (-f32::from(self.scroll.offset().y)).clamp(0.0, max);
                let thumb_h = (viewport * viewport / content).max(24.0);
                let thumb_top = scrolled / max * (viewport - thumb_h);
                let mut c = theme::text_tertiary();
                c.a = 0.5;
                div()
                    .absolute()
                    .top(px(thumb_top))
                    .right(px(1.0))
                    .w(px(6.0))
                    .h(px(thumb_h))
                    .rounded(px(3.0))
                    .bg(c)
            })
        };
        let nested = self.nested.clone();
        let md = gpui_markdown::MarkdownView::new(
            format!("embed-{}", self.nav_target),
            self.content.clone(),
        )
        .style(theme::markdown_style(self.list_indent, self.text_size))
        .on_wiki_link(std::rc::Rc::new(move |title, window, cx| {
            let _ = wiki_app.update(cx, |this, cx| this.open_page_title(&title, window, cx));
        }))
        // The full renderer set, like the note this content came from — images
        // arrive through the read-only path (see `EmbedView::image`).
        .on_image(self.image.clone())
        .on_embed_image(self.image.clone())
        .on_mermaid(self.mermaid.clone())
        .on_math(self.math.clone())
        .on_inline_math(self.inline_math.clone())
        .on_highlight(self.highlight.clone())
        // Embeds nested inside this one, pre-resolved by the host (the
        // MarkdownView's own depth cap keeps cycles finite).
        .on_embed(std::rc::Rc::new(move |target| nested.get(target).cloned()));
        div()
            .id("embed-root")
            .size_full()
            .border_l_2()
            .border_color(theme::text_tertiary())
            .rounded(px(4.0))
            .pl(px(12.0))
            .py(px(4.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .on_hover(cx.listener(|this, hovered: &bool, _w, cx| {
                this.hovered = *hovered;
                cx.notify();
            }))
            // The overlay occludes the page's own wheel handling, so once the
            // embed can't consume the wheel itself (at its top/bottom, or no
            // overflow at all) hand the delta to the page — nested scrolling
            // that doesn't trap the pointer.
            .on_scroll_wheel(
                cx.listener(|this, ev: &gpui::ScrollWheelEvent, window, cx| {
                    let delta = ev.delta.pixel_delta(window.line_height()).y;
                    let off = this.scroll.offset().y;
                    let max = this.scroll.max_offset().y;
                    let at_top = off >= px(-1.0);
                    let at_bottom = off <= -max + px(1.0);
                    if (delta < px(0.0) && at_bottom) || (delta > px(0.0) && at_top) {
                        let _ = this
                            .app
                            .update(cx, |app, cx| app.scroll_active_surface(delta, cx));
                    }
                }),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(self.text_size * 0.8)
                    .text_color(theme::accent())
                    .cursor_pointer()
                    .child(self.label.clone())
                    .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        let _ = app.update(cx, |this, cx| this.open_page_title(&nav, window, cx));
                    }),
            )
            .child(
                // Long content scrolls inside the reserved gap; a thumb shows
                // its position while the pointer is over the embed.
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("embed-body")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .text_size(self.text_size)
                            .text_color(theme::text_primary())
                            .child(md),
                    )
                    .children(thumb),
            )
    }
}
