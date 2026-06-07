//! PDF rasterization via [`hayro`] (pure Rust — no native library). PDFs open in
//! a dedicated viewer tab (`ui::pdf_view`); a note links to one with `[[file.pdf]]`
//! or `![](file.pdf)`. The file is parsed once into a [`Document`] (kept in the
//! viewer's `PdfDoc`); page sizes are read cheaply for instant layout, then pages
//! near the viewport are rasterized to `gpui::RenderImage`s on a background thread.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::RenderImage;
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use image::{Frame, RgbaImage};

/// Page render scale (PDF point-size × this). Higher = sharper but more memory.
pub const SCALE: f32 = 1.5;

/// A parsed PDF. Parsed once (not per page) — re-parsing a large file for every
/// page is slow and churns the allocator. `hayro::Pdf` is `Send + Sync` (std
/// feature) and caches pages internally, so it's shared via `Arc` across the
/// background render tasks.
pub type Document = Pdf;

/// Parse a PDF's bytes into a reusable [`Document`]. The `Document` owns the
/// bytes, so the caller can drop its own copy.
pub fn parse(bytes: Arc<Vec<u8>>) -> Result<Arc<Document>, String> {
    let pdf = Pdf::new(bytes).map_err(|e| format!("parse PDF: {e:?}"))?;
    Ok(Arc::new(pdf))
}

/// Each page's `(width, height)` in points — cheap to read (no rasterization), so
/// the viewer can lay out correctly-sized page slots before any page renders.
pub fn page_dims(doc: &Document) -> Vec<(f32, f32)> {
    doc.pages().iter().map(|p| p.render_dimensions()).collect()
}

/// Rasterize a single page (0-based) of an already-parsed [`Document`] to a BGRA
/// `RenderImage` composited onto white.
pub fn render_page(doc: &Document, idx: usize, scale: f32) -> Result<Arc<RenderImage>, String> {
    let pixmaps = hayro::render_pdf(doc, scale, InterpreterSettings::default(), Some(idx..=idx))
        .ok_or_else(|| format!("render page {idx}"))?;
    let pixmap = pixmaps
        .into_iter()
        .next()
        .ok_or_else(|| format!("no page {idx}"))?;

    let (w, h) = (u32::from(pixmap.width()), u32::from(pixmap.height()));
    let src = pixmap.data_as_u8_slice(); // premultiplied RGBA8, row-major
    let mut bgra = vec![0u8; src.len()];
    for (out, p) in bgra.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        // Composite premultiplied src over white (out = src + 255-a; src ≤ a so
        // no overflow), then RGBA→BGRA (gpui's RenderImage is BGRA).
        let add = 255 - p[3];
        out[0] = p[2].saturating_add(add); // B
        out[1] = p[1].saturating_add(add); // G
        out[2] = p[0].saturating_add(add); // R
        out[3] = 255;
    }
    let buf = RgbaImage::from_raw(w, h, bgra).ok_or_else(|| "bad pixel buffer".to_string())?;
    Ok(Arc::new(RenderImage::new(vec![Frame::new(buf)])))
}

/// True if a link/image `src` points at a PDF.
pub fn is_pdf(src: &str) -> bool {
    src.to_lowercase().trim_end().ends_with(".pdf")
}

/// Resolve a PDF reference to an existing local file. Path resolution (handling
/// `file://`, absolute, and data-dir-relative refs across platforms) lives in
/// [`crate::paths::resolve_local`]; this just rejects remote (http) PDFs and
/// requires the file to exist.
pub fn resolve_path(src: &str) -> Option<PathBuf> {
    crate::paths::resolve_local(src).filter(|p| p.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_pdf_extension() {
        assert!(is_pdf("a.pdf"));
        assert!(is_pdf("images/B.PDF"));
        assert!(!is_pdf("a.png"));
        assert!(!is_pdf("notapdf"));
    }
}
