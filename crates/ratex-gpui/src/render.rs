//! Render a formula to a gpui image — the display primitive (the `ratex-gtk4` analog).
//!
//! `model → LaTeX → RaTeX layout → display list → ratex-render raster (PNG) → decode →
//! BGRA → gpui RenderImage`. RaTeX only exposes a PNG encoder (not the raw pixmap), so we
//! round-trip through PNG for now; a pixmap accessor (or a small fork) would skip the
//! re-decode. White opaque background, so RGBA→BGRA is a plain channel swap; theming /
//! transparency is a later polish.

use crate::editor::model::Row;
use gpui::RenderImage;
use image::{Frame, RgbaImage};
use ratex_layout::{LayoutOptions, layout, to_display_list};
use ratex_parser::parse;
use ratex_render::{RenderOptions, render_to_png};
use std::sync::Arc;

/// Logical padding (px) RaTeX leaves around the formula. The view offsets caret/slot
/// geometry by this much, since the same value feeds `RenderOptions::padding`.
pub const PAD: f32 = 8.0;

/// A rasterized formula plus its logical (pre-DPR) size in px.
pub struct Rendered {
    pub image: Arc<RenderImage>,
    pub width: f32,
    pub height: f32,
}

/// Render a row to a gpui image at `font_size` px/em and `dpr` device-pixel-ratio.
/// `None` if the row's LaTeX fails to parse / lay out / rasterize.
pub fn render_row(row: &Row, font_size: f32, dpr: f32) -> Option<Rendered> {
    let (bgra, w, h) = rasterize(row, font_size, dpr)?;
    let buf = RgbaImage::from_raw(w, h, bgra)?;
    Some(Rendered {
        image: Arc::new(RenderImage::new(vec![Frame::new(buf)])),
        width: w as f32 / dpr,
        height: h as f32 / dpr,
    })
}

/// The gpui-free half: model → BGRA pixels + pixel dimensions. Separated so it can be
/// unit-tested without a gpui context.
fn rasterize(row: &Row, font_size: f32, dpr: f32) -> Option<(Vec<u8>, u32, u32)> {
    let nodes = parse(&row.to_latex()).ok()?;
    let lbox = layout(&nodes, &LayoutOptions::default());
    let dl = to_display_list(&lbox);
    let opts = RenderOptions {
        font_size,
        padding: PAD,
        device_pixel_ratio: dpr,
        ..Default::default()
    };
    let png = render_to_png(&dl, &opts).ok()?;
    let rgba = image::load_from_memory(&png).ok()?.into_rgba8();
    let (w, h) = rgba.dimensions();
    let mut bytes = rgba.into_raw();
    // gpui's RenderImage is BGRA; RaTeX's PNG is RGBA on opaque white, so a channel swap
    // suffices (alpha is already 255 → premultiplied == itself).
    for px in bytes.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    Some((bytes, w, h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::model::Row;

    #[test]
    fn rasterizes_to_nonempty_bgra() {
        let (bytes, w, h) = rasterize(&Row::syms("abc"), 40.0, 2.0).expect("renders");
        assert!(w > 0 && h > 0, "non-empty dims");
        assert_eq!(
            bytes.len(),
            w as usize * h as usize * 4,
            "BGRA buffer matches dims"
        );
    }

    #[test]
    fn dpr_scales_pixels() {
        let (_, w1, _) = rasterize(&Row::syms("x"), 40.0, 1.0).unwrap();
        let (_, w2, _) = rasterize(&Row::syms("x"), 40.0, 2.0).unwrap();
        assert!(w2 > w1, "2x DPR yields more pixels ({w2} vs {w1})");
    }
}
