//! Mermaid diagram rendering: parse + lay out + emit SVG with the pure-Rust
//! `mermaid-rs-renderer` (the same crate Zed uses — no JS), then rasterize that
//! SVG to a `gpui::RenderImage` with gpui's built-in SVG renderer. Renders are
//! expensive, so they run off-thread and are cached here, keyed by the block's
//! source text — mirroring [`crate::images::ImageStore`].

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{Hsla, RenderImage, SharedString, SvgRenderer};

use crate::theme;

/// Cache of rendered diagrams, keyed by a ```mermaid block's source text.
#[derive(Default)]
pub struct MermaidStore {
    slots: HashMap<SharedString, Slot>,
}

enum Slot {
    Loading,
    Ready(Arc<RenderImage>),
    Failed,
}

impl MermaidStore {
    /// The rendered diagram for `source`, if it's ready.
    pub fn get(&self, source: &SharedString) -> Option<Arc<RenderImage>> {
        match self.slots.get(source) {
            Some(Slot::Ready(image)) => Some(image.clone()),
            _ => None,
        }
    }

    /// Whether `source` failed to render — so the host can fall back to the code.
    pub fn failed(&self, source: &SharedString) -> bool {
        matches!(self.slots.get(source), Some(Slot::Failed))
    }

    /// Claim `source` for rendering. Returns `false` if it already has a slot
    /// (loading / ready / failed), so the render is kicked off at most once.
    pub fn begin(&mut self, source: SharedString) -> bool {
        if self.slots.contains_key(&source) {
            return false;
        }
        self.slots.insert(source, Slot::Loading);
        true
    }

    /// Record a finished render (ready or failed).
    pub fn finish(&mut self, source: SharedString, result: Result<Arc<RenderImage>, String>) {
        let slot = match result {
            Ok(image) => Slot::Ready(image),
            Err(e) => {
                log::warn!("mermaid render failed: {e}");
                Slot::Failed
            }
        };
        self.slots.insert(source, slot);
    }

    /// Drop every cached diagram — diagrams are themed at render time, so a theme
    /// change invalidates them all (they re-render on the next paint).
    pub fn clear(&mut self) {
        self.slots.clear();
    }
}

/// A `#rrggbb` hex string for a gpui color. The diagram theme's fields are opaque
/// hex, so a translucent token (glass / hover / accent-tint) is composited over the
/// page background first — yielding the opaque color it actually appears as, not a
/// near-white from a dropped alpha.
fn hex(color: Hsla) -> String {
    let bg = theme::bg_content().to_rgb();
    let c = color.to_rgb();
    let blend = |fg: f32, bg: f32| fg * c.a + bg * (1.0 - c.a);
    let to = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        to(blend(c.r, bg.r)),
        to(blend(c.g, bg.g)),
        to(blend(c.b, bg.b))
    )
}

/// Build a `mermaid_rs_renderer::Theme` from Zorite's current (thread-local) theme
/// palette, so a diagram matches whatever skin / light-dark mode is active. Call on
/// the **main thread** (the palette is thread-local); the result is all `String`, so
/// it's `Send` and crosses into the off-thread render.
pub fn current_theme() -> mermaid_rs_renderer::Theme {
    let bg = hex(theme::bg_content());
    let text = hex(theme::text_primary());
    let border = hex(theme::border_subtle());
    let line = hex(theme::text_tertiary());
    // A solid elevated surface for node fills (not the translucent `glass`), so node
    // text stays readable.
    let fill = hex(theme::elevated());

    let mut t = mermaid_rs_renderer::Theme::modern();
    // Nodes + background + text.
    t.background = bg.clone();
    t.text_color = text.clone();
    t.primary_color = fill.clone();
    t.primary_text_color = text.clone();
    t.primary_border_color = border.clone();
    t.secondary_color = hex(theme::elevated());
    t.tertiary_color = hex(theme::hover());
    // Edges + edge labels + subgraph clusters.
    t.line_color = line.clone();
    t.edge_label_background = bg;
    t.cluster_background = hex(theme::bg_sidebar());
    t.cluster_border = hex(theme::divider());
    // Sequence diagrams.
    t.sequence_actor_fill = fill.clone();
    t.sequence_actor_border = border.clone();
    t.sequence_actor_line = line;
    t.sequence_note_fill = hex(theme::elevated());
    t.sequence_note_border = border.clone();
    t.sequence_activation_fill = hex(theme::hover());
    t.sequence_activation_border = border.clone();
    // Categorical palettes (pie slices, git branches): the accent hue rotated.
    let accent = theme::accent();
    let rotated = |i: usize, n: usize| {
        let mut c = accent;
        c.h = (accent.h + i as f32 / n as f32).fract();
        hex(c)
    };
    t.pie_colors = std::array::from_fn(|i| rotated(i, 12));
    t.pie_title_text_color = text.clone();
    t.pie_legend_text_color = text.clone();
    t.pie_stroke_color = border.clone();
    t.pie_outer_stroke_color = border.clone();
    t.git_colors = std::array::from_fn(|i| rotated(i, 8));
    t.git_inv_colors = std::array::from_fn(|i| rotated(i, 8));
    t.git_commit_label_color = text.clone();
    t.git_commit_label_background = fill.clone();
    t.git_tag_label_color = text;
    t.git_tag_label_background = fill;
    t.git_tag_label_border = border;
    t
}

/// Render `source` to a diagram bitmap with `theme`: mermaid → SVG
/// (`mermaid-rs-renderer`) → `RenderImage` (gpui's SVG rasterizer at `scale`).
/// Pure CPU — safe off-thread.
pub fn render_to_image(
    source: &str,
    theme: mermaid_rs_renderer::Theme,
    svg: &SvgRenderer,
    scale: f32,
) -> Result<Arc<RenderImage>, String> {
    let options = mermaid_rs_renderer::RenderOptions {
        theme,
        layout: mermaid_rs_renderer::LayoutConfig::default(),
    };
    let svg_string =
        mermaid_rs_renderer::render_with_options(source, options).map_err(|e| e.to_string())?;
    svg.render_single_frame(svg_string.as_bytes(), scale)
        .map_err(|e| e.to_string())
}
