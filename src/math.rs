//! Math block rendering: a `$$…$$` block's LaTeX, typeset by `ratex-gpui` (a RaTeX port) to
//! a `gpui::RenderImage`. Renders are expensive, so they run off-thread and are cached here,
//! keyed by the block's LaTeX — mirroring [`crate::mermaid::MermaidStore`]. Unlike Mermaid's
//! vector SVG, RaTeX rasters are pixel-sized, so each slot also carries the image's logical
//! (pre-DPR) px size, for display at the right scale.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{Hsla, RenderImage, SharedString};

/// Font size (px/em) math is typeset at — a touch larger than body text, for display math.
pub const FONT_SIZE: f32 = 22.0;
/// Device-pixel ratio math is rasterized at (displayed at logical size, so 2× = crisp).
pub const DPR: f32 = 2.0;

/// A ready formula: the bitmap plus its logical (pre-DPR) px size.
type Image = (Arc<RenderImage>, f32, f32);

/// Cache of typeset formulas, keyed by a `$$…$$` block's LaTeX source.
#[derive(Default)]
pub struct MathStore {
    slots: HashMap<SharedString, Slot>,
    /// The text color the cached rasters were tinted for; a theme change clears them.
    color: Option<Hsla>,
}

enum Slot {
    Loading,
    Ready(Image),
    Failed,
}

impl MathStore {
    /// The typeset formula for `source` (image + logical size), if it's ready.
    pub fn get(&self, source: &SharedString) -> Option<Image> {
        match self.slots.get(source) {
            Some(Slot::Ready((image, w, h))) => Some((image.clone(), *w, *h)),
            _ => None,
        }
    }

    /// Whether `source` failed to typeset — so the host can fall back to the raw LaTeX.
    pub fn failed(&self, source: &SharedString) -> bool {
        matches!(self.slots.get(source), Some(Slot::Failed))
    }

    /// Whether `source` already has a slot, so the render is kicked off at most once.
    pub fn started(&self, source: &SharedString) -> bool {
        self.slots.contains_key(source)
    }

    /// Set the text color formulas are tinted for. If it changed (a theme switch), drop the
    /// cached rasters so they re-render in the new color. Call before kicking off renders.
    pub fn set_color(&mut self, color: Hsla) {
        if self.color != Some(color) {
            self.color = Some(color);
            self.slots.clear();
        }
    }

    /// Mark `source` as rendering.
    pub fn begin(&mut self, source: SharedString) {
        self.slots.insert(source, Slot::Loading);
    }

    /// Record a finished render (ready or failed).
    pub fn finish(&mut self, source: SharedString, result: Option<Image>) {
        let slot = match result {
            Some(img) => Slot::Ready(img),
            None => {
                log::warn!("math render failed for: {source}");
                Slot::Failed
            }
        };
        self.slots.insert(source, slot);
    }
}

/// Typeset `latex` to a bitmap + logical size via `ratex-gpui` (RaTeX). Pure CPU — safe
/// off-thread. `None` if the LaTeX fails to parse / lay out / rasterize.
pub fn render_to_image(latex: &str, font_size: f32, dpr: f32, color: Hsla) -> Option<Image> {
    let r = ratex_gpui::render::render_latex(latex, font_size, dpr, color)?;
    Some((r.image, r.width, r.height))
}
