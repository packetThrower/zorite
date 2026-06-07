//! PDF support. Rasterization and the page-virtualized viewer live in the
//! [`gpui_pdf`] crate; this module re-exports what the app uses and adds local
//! path resolution (which depends on the app's data-dir layout).
//!
//! A note links a PDF with `[[file.pdf]]` or `![](file.pdf)`, or a PDF dropped onto
//! a note is imported; either opens it in a dedicated [`PdfView`] tab.

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
