//! PDF support. Rasterization and the page-virtualized viewer live in the
//! [`gpui_pdf`] crate; this module re-exports what the app uses and adds local
//! path resolution (which depends on the app's data-dir layout).
//!
//! A note links a PDF with `[[file.pdf]]` or `![](file.pdf)`, or a PDF dropped onto
//! a note is imported; either opens it in a dedicated [`PdfView`] tab.

use std::cell::Cell;
use std::path::PathBuf;

pub use gpui_pdf::{PdfStyle, PdfView, is_pdf};

use crate::paths;

/// Resolve a PDF reference to an existing local file. Cross-platform path resolution
/// (handling `file://`, absolute, and data-dir-relative refs) lives in
/// [`crate::paths::resolve_local`]; this just rejects remote (http) PDFs and requires
/// the file to exist.
pub fn resolve_path(src: &str) -> Option<PathBuf> {
    paths::resolve_local(src).filter(|p| p.exists())
}

thread_local! {
    /// Current PDF render-quality multiplier (1.0 = native DPI). Set from Settings and
    /// read — without a `cx` — by the closure each `PdfView` uses for its render scale.
    /// Lives on the (single) UI thread, so every window's viewers see one value.
    static QUALITY: Cell<f32> = const { Cell::new(1.0) };
}

/// The current PDF render-quality multiplier.
pub fn quality() -> f32 {
    QUALITY.with(Cell::get)
}

/// Set the PDF render-quality multiplier (clamped to a sane range). Open viewers pick
/// it up on their next paint.
pub fn set_quality(q: f32) {
    QUALITY.with(|c| c.set(q.clamp(0.25, 3.0)));
}
