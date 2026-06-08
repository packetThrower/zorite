//! PDF support. Rasterization and the page-virtualized viewer live in the
//! [`gpui_pdf`] crate; this module re-exports what the app uses and adds local
//! path resolution (which depends on the app's data-dir layout).
//!
//! A note links a PDF with `[[file.pdf]]` or `![](file.pdf)`, or a PDF dropped onto
//! a note is imported; either opens it in a dedicated [`PdfView`] tab.

use std::cell::Cell;
use std::path::{Path, PathBuf};

pub use gpui_pdf::{Highlight, PdfStyle, PdfView, is_pdf};

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

// --- Markup: per-PDF highlights page (Logseq-style) ---

/// Title of the per-PDF "highlights" page that collects a PDF's markup as blocks.
/// Not `.pdf`-suffixed, so it opens as a normal page (not the viewer).
pub fn highlights_title(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!("{name} (highlights)")
}

/// The highlight colors the picker offers, as `(name, hue, saturation, lightness)`.
/// The name is what's stored in the markdown (a trailing `{name}`), so it must
/// round-trip with [`known_color`]. Yellow is first (the default).
const PALETTE: &[(&str, f32, f32, f32)] = &[
    ("yellow", 0.14, 0.95, 0.55),
    ("green", 0.33, 0.60, 0.50),
    ("blue", 0.58, 0.85, 0.60),
    ("pink", 0.92, 0.80, 0.66),
    ("orange", 0.07, 0.90, 0.55),
];

/// The default highlight fill (yellow) — used for entries with no `{color}` tag.
fn default_highlight_color() -> gpui::Hsla {
    gpui::hsla(0.14, 0.95, 0.55, 1.0)
}

/// Resolve a stored color name to its fill, or `None` if it isn't a known palette
/// name (so a quote that merely ends in `{…}` isn't mistaken for a color tag).
fn known_color(name: &str) -> Option<gpui::Hsla> {
    PALETTE
        .iter()
        .find(|(n, ..)| n.eq_ignore_ascii_case(name))
        .map(|(_, h, s, l)| gpui::hsla(*h, *s, *l, 1.0))
}

/// The palette to hand the viewer's color picker, as `(name, fill)` pairs.
pub fn highlight_palette() -> Vec<(gpui::SharedString, gpui::Hsla)> {
    PALETTE
        .iter()
        .map(|(n, h, s, l)| ((*n).into(), gpui::hsla(*h, *s, *l, 1.0)))
        .collect()
}

/// Parse a highlights page's markdown into viewer highlights. Each line of the form
/// `- p{N}: {quote}` (1-based page; an optional trailing `[[link]]` is ignored)
/// becomes one highlight; repeated quote+page pairs get successive occurrence
/// indices so duplicates on a page each land on their own match.
pub fn parse_highlights(content: &str) -> Vec<Highlight> {
    use std::collections::HashMap;
    let mut out = Vec::new();
    let mut seen: HashMap<(usize, String), usize> = HashMap::new();
    for (i, line) in content.lines().enumerate() {
        let line = line.trim_start();
        let rest = line
            .strip_prefix("- ")
            .or_else(|| line.strip_prefix("* "))
            .unwrap_or(line);
        let Some(rest) = rest.strip_prefix('p').or_else(|| rest.strip_prefix('P')) else {
            continue;
        };
        let Some(colon) = rest.find(':') else {
            continue;
        };
        let Ok(n) = rest[..colon].trim().parse::<usize>() else {
            continue;
        };
        if n == 0 {
            continue;
        }
        let mut quote = rest[colon + 1..].trim().to_string();
        // Strip a trailing reverse-link `[[…]]` (note→PDF navigation).
        if let Some(b) = quote.find("[[") {
            quote = quote[..b].trim().to_string();
        }
        // Strip a trailing `{color}` tag — but only if it names a known color, so a
        // quote that merely happens to end in braces is left untouched.
        let mut color = default_highlight_color();
        if quote.ends_with('}')
            && let Some(b) = quote.rfind('{')
            && let Some(c) = known_color(quote[b + 1..quote.len() - 1].trim())
        {
            color = c;
            quote = quote[..b].trim().to_string();
        }
        if quote.is_empty() {
            continue;
        }
        let page = n - 1;
        let occurrence = {
            let c = seen.entry((page, quote.clone())).or_insert(0);
            let v = *c;
            *c += 1;
            v
        };
        out.push(Highlight {
            id: i as u64,
            page,
            quote,
            occurrence,
            color,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_title_strips_path() {
        assert_eq!(
            highlights_title(Path::new("images/Manual.pdf")),
            "Manual.pdf (highlights)"
        );
    }

    #[test]
    fn parses_page_and_quote() {
        let hs = parse_highlights("- p100: Smart Grid Monitors [[Manual.pdf]]\n- p1: Overview");
        assert_eq!(hs.len(), 2);
        assert_eq!(hs[0].page, 99);
        assert_eq!(hs[0].quote, "Smart Grid Monitors");
        assert_eq!(hs[0].occurrence, 0);
        assert_eq!(hs[1].page, 0);
        assert_eq!(hs[1].quote, "Overview");
    }

    #[test]
    fn duplicate_quotes_get_occurrence_indices() {
        let hs = parse_highlights("- p2: cat\n- p2: cat\n- p3: cat");
        assert_eq!(hs[0].occurrence, 0);
        assert_eq!(hs[1].occurrence, 1);
        assert_eq!(hs[2].occurrence, 0); // different page resets
    }

    #[test]
    fn ignores_non_highlight_lines() {
        let hs = parse_highlights("# Heading\n- a normal bullet\n- p0: bad\n- p5:   ");
        assert!(hs.is_empty());
    }

    #[test]
    fn parses_color_tag_and_reverse_link() {
        let hs = parse_highlights(
            "- p1: Counters for seconds {green} [[pdf/m41t81s-1.pdf#p1|↗]]\n\
             - p2: plain quote\n\
             - p3: math set {3}",
        );
        assert_eq!(hs.len(), 3);
        // Both the color tag and the reverse link are stripped from the stored quote.
        assert_eq!(hs[0].quote, "Counters for seconds");
        assert_eq!(hs[0].color, known_color("green").unwrap());
        // No tag → default yellow.
        assert_eq!(hs[1].quote, "plain quote");
        assert_eq!(hs[1].color, default_highlight_color());
        // A trailing `{…}` that isn't a known color stays part of the quote.
        assert_eq!(hs[2].quote, "math set {3}");
    }
}
