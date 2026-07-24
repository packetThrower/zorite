//! Async render-asset stores — image decoding, mermaid rendering, and
//! `![[…]]` embed resolution — split from `app.rs`.

use super::*;

/// How many image decodes may run concurrently. JPEGs now decode at a reduced
/// size (DCT scaling — see `images::decode_jpeg_reduced`), so their transient
/// buffer is small; only a non-JPEG fallback holds a full-resolution buffer
/// (~35 MB for a 12 MP photo). With that, a typical photo page decodes in a
/// single wave on any multi-core machine.
const MAX_IMAGE_DECODES: usize = 6;

impl AppView {
    /// Resolve every `![[target]]` embed in `content` — and, recursively, in
    /// the embedded content itself (depth-capped) — to `(label, content)`, for
    /// the reader's embed provider. Providers can't query mid-render, so hosts
    /// build this map up front.
    pub(crate) fn build_embed_map(
        &self,
        content: &str,
    ) -> std::rc::Rc<HashMap<String, (SharedString, SharedString)>> {
        let mut map = HashMap::new();
        let mut queue: Vec<(String, usize)> = gpui_markdown::syntax::embed_targets(content)
            .into_iter()
            .map(|t| (t, 0usize))
            .collect();
        while let Some((target, depth)) = queue.pop() {
            if depth >= 3 || map.contains_key(&target) {
                continue;
            }
            if let Some((label, body)) = self.resolve_embed(&target) {
                for t in gpui_markdown::syntax::embed_targets(&body) {
                    queue.push((t, depth + 1));
                }
                map.insert(target, (label, body));
            }
        }
        std::rc::Rc::new(map)
    }

    /// Resolve one embed target to `(source label, content)`: a whole page, a
    /// `#^id` block's line, or a `#Heading` section — with the same rules as
    /// navigation (a literal `#`-titled page wins; PDFs and whiteboards don't
    /// embed). `None` leaves the `![[…]]` line rendering as plain text.
    fn resolve_embed(&self, inner: &str) -> Option<(SharedString, SharedString)> {
        use gpui_markdown::syntax::{
            extract_block, extract_section, split_block_anchor, split_heading_anchor,
            wiki_target_display,
        };
        let (target, display) = wiki_target_display(inner);
        let (page_t, block) = split_block_anchor(target);
        let (page_t, heading) = if block.is_none() {
            if matches!(self.db.get_page_by_title(target), Ok(Some(_))) {
                (target, None)
            } else {
                split_heading_anchor(target)
            }
        } else {
            (page_t, None)
        };
        if crate::pdf::is_pdf(page_t)
            || matches!(self.db.get_whiteboard_by_title(page_t), Ok(Some(_)))
        {
            return None;
        }
        let page = self
            .db
            .get_page_by_title(page_t)
            .ok()
            .flatten()
            .or_else(|| self.db.get_page_by_alias(page_t).ok().flatten())?;
        let range = if let Some(id) = block {
            extract_block(&page.content, id)?
        } else if let Some(h) = heading {
            extract_section(&page.content, h)?
        } else {
            0..page.content.len()
        };
        let label = if display != target {
            display.to_string()
        } else if let Some(id) = block {
            format!("{page_t} → {id}")
        } else if let Some(h) = heading {
            format!("{page_t} → {}", h.trim())
        } else {
            page.title.clone()
        };
        Some((label.into(), page.content[range].to_string().into()))
    }

    /// Ensure the ```mermaid block `source` is rendering/rendered (idempotent).
    /// Called from a not-yet-rendered diagram's placeholder the first time it
    /// paints: claims the slot, then renders mermaid → SVG → bitmap off-thread
    /// (it's a layout-heavy parse) and repaints when it lands.
    pub fn ensure_mermaid_loaded(&mut self, source: SharedString, cx: &mut Context<Self>) {
        if !self.mermaid_store.borrow_mut().begin(source.clone()) {
            return; // already rendering / ready / failed
        }
        // Build the diagram theme from Zorite's current palette now (it's a
        // thread-local read on this main thread); the result is `Send`.
        let theme = crate::mermaid::current_theme();
        let svg = cx.svg_renderer();
        let store = self.mermaid_store.clone();
        cx.spawn(async move |this, cx| {
            let src = source.to_string();
            let result = cx
                .background_executor()
                .spawn(async move {
                    crate::mermaid::render_to_image(&src, theme, &svg, crate::mermaid::RASTER_SCALE)
                })
                .await;
            store.borrow_mut().finish(source, result);
            // `cx.notify()` alone can leave an editor's cached row layout stale
            // (it was built before the bitmap existed) — force a full repaint so
            // the diagram replaces the raw-source placeholder immediately.
            let _ = this.update(cx, |_, cx| {
                cx.notify();
                cx.refresh_windows();
            });
        })
        .detach();
    }

    /// Kick off decoding for every standalone image in `content`, so an editor in
    /// WYSIWYG mode can render them inline (W4) rather than as raw `![](src)`.
    /// `ensure_image_loaded` dedupes, so re-scanning is cheap; a finished decode
    /// notifies → repaint → the editor's block-image provider finds the bitmap.
    pub(super) fn ensure_content_images(&mut self, content: &str, cx: &mut Context<Self>) {
        // Every image, block AND inline — inline images render as rasters too.
        for src in gpui_markdown::all_image_srcs(content) {
            self.ensure_image_loaded(src, cx);
        }
    }

    /// Kick off the off-thread render of every ```mermaid block in `content`, so
    /// an editor in WYSIWYG mode can render them as diagrams. Idempotent (the
    /// store dedupes); a finished render notifies → repaint → the editor's mermaid
    /// provider finds the bitmap. Uses the editor's extraction so the cache key
    /// matches what the editor looks up.
    pub(super) fn ensure_content_mermaid(&mut self, content: &str, cx: &mut Context<Self>) {
        for source in gpui_editor::mermaid_sources(content) {
            self.ensure_mermaid_loaded(source, cx);
        }
    }

    /// Resolve every `![[target]]` embed in `content` into the shared store the
    /// editors' overlay provider reads: one `EmbedView` per target plus the row
    /// height to reserve — estimated from the embedded content's line count and
    /// capped (long content scrolls inside the view). A target that no longer
    /// resolves drops out, falling back to the chip.
    pub(super) fn ensure_content_embeds(&mut self, content: &str, cx: &mut Context<Self>) {
        for inner in gpui_markdown::syntax::embed_targets(content) {
            self.upsert_embed(inner, cx);
        }
    }

    /// Re-resolve every target already in the embed store against the database.
    /// Runs on each (debounced) doc change, so an embed live-updates when its
    /// SOURCE page is edited — the embedding page's own ensure-pass only runs
    /// when that page reloads.
    pub(crate) fn refresh_embed_store(&mut self, cx: &mut Context<Self>) {
        let targets: Vec<String> = self.embed_store.borrow().keys().cloned().collect();
        for inner in targets {
            self.upsert_embed(inner, cx);
        }
    }

    /// Resolve one embed target into the store: create or update its view and
    /// recompute the reserved height. A target that no longer resolves drops
    /// out (the editor falls back to the chip).
    fn upsert_embed(&mut self, inner: String, cx: &mut Context<Self>) {
        let Some((label, body)) = self.resolve_embed(&inner) else {
            self.embed_store.borrow_mut().remove(&inner);
            return;
        };
        // Rasterize/decode the embedded content's constructs into the shared
        // stores (images, mermaid, math), so the box renders them like the
        // note they came from — both here and in the reader's embeds.
        self.ensure_content_images(&body, cx);
        self.ensure_content_mermaid(&body, cx);
        self.ensure_content_math(&body, cx);
        let lh = f32::from(self.text_size()) * 1.45;
        let lines = body.lines().count().max(1) as f32;
        let height = (40.0 + lines * (lh + 6.0)).clamp(64.0, 340.0);
        let nav_target: SharedString = gpui_markdown::syntax::wiki_target_display(&inner)
            .0
            .to_string()
            .into();
        let text_size = self.text_size();
        let list_indent = self.list_indent();
        // Fresh renderers + nested-embed map each upsert: they're cheap Rc
        // closures over the shared stores, and the nested map tracks the
        // (possibly changed) body.
        let image = crate::ui::image::embed_renderer(self, cx);
        let mermaid = crate::ui::mermaid::renderer(self, cx);
        let math = crate::ui::math::renderer(self, cx);
        let inline_math = crate::ui::math::inline_renderer(self);
        let highlight = self.highlighter_fn();
        let nested = self.build_embed_map(&body);
        let existing = self.embed_store.borrow().get(&inner).cloned();
        match existing {
            Some((view, _)) => {
                view.update(cx, |v, cx| {
                    if v.content != body || v.label != label || v.text_size != text_size {
                        v.content = body;
                        v.label = label;
                        v.nav_target = nav_target;
                        v.text_size = text_size;
                        v.list_indent = list_indent;
                        cx.notify();
                    }
                    v.image = image;
                    v.mermaid = mermaid;
                    v.math = math;
                    v.inline_math = inline_math;
                    v.highlight = highlight;
                    v.nested = nested;
                });
                self.embed_store.borrow_mut().insert(inner, (view, height));
            }
            None => {
                let app = cx.entity().downgrade();
                let view = cx.new(|_| crate::ui::embed::EmbedView {
                    nav_target,
                    label,
                    content: body,
                    text_size,
                    list_indent,
                    app,
                    scroll: gpui::ScrollHandle::new(),
                    hovered: false,
                    image,
                    mermaid,
                    math,
                    inline_math,
                    highlight,
                    nested,
                });
                self.embed_store.borrow_mut().insert(inner, (view, height));
            }
        }
    }

    /// Ensure the image at `src` is decoding/decoded (idempotent). Called from a
    /// not-yet-loaded image's placeholder the first time it paints: claims the
    /// slot and queues a downscaled decode (run a bounded few at a time by
    /// [`Self::pump_image_decodes`]).
    pub fn ensure_image_loaded(&mut self, src: SharedString, cx: &mut Context<Self>) {
        if !self.image_store.borrow_mut().begin(src.clone()) {
            return; // already loading / ready / failed
        }
        match crate::paths::resolve_local(&src).filter(|p| p.exists()) {
            Some(path) => self.image_queue.push_back((src, path)),
            None => {
                self.image_store.borrow_mut().finish(src, None);
                cx.notify();
                return;
            }
        }
        self.pump_image_decodes(cx);
    }

    /// Decode queued images off-thread, up to [`MAX_IMAGE_DECODES`] at a time,
    /// each storing its bitmap, repainting, and pumping the next on completion.
    /// The cap bounds the transient full-resolution decode buffers (a page of
    /// 12 MP photos would otherwise hold one ~35 MB buffer per image at once).
    fn pump_image_decodes(&mut self, cx: &mut Context<Self>) {
        while self.image_decodes < MAX_IMAGE_DECODES {
            let Some((src, path)) = self.image_queue.pop_front() else {
                return;
            };
            self.image_decodes += 1;
            let store = self.image_store.clone();
            cx.spawn(async move |this, cx| {
                let decoded = cx
                    .background_executor()
                    .spawn(async move { crate::images::decode_scaled(&path) })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    store.borrow_mut().finish(src, decoded);
                    this.image_decodes -= 1;
                    this.pump_image_decodes(cx);
                    // See the analogous comment in `ensure_mermaid_loaded`:
                    // `cx.notify()` alone can leave a stale cached row layout,
                    // painted before the bitmap existed.
                    cx.notify();
                    cx.refresh_windows();
                });
            })
            .detach();
        }
    }
}
