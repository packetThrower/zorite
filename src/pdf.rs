//! PDF support. Rasterization and the page-virtualized viewer live in the
//! [`gpui_pdf`] crate; this module re-exports what the app uses and adds local
//! path resolution (which depends on the app's data-dir layout).
//!
//! A note links a PDF with `[[file.pdf]]` or `![](file.pdf)`, or a PDF dropped onto
//! a note is imported; either opens it in a dedicated [`PdfView`] tab.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub use gpui_pdf::{Highlight, PdfEvent, PdfStyle, PdfView, is_pdf};

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

/// Split a highlighted quote on PDF bullet glyphs (●, •, ▪, …) into one item per
/// bullet, so a multi-bullet selection becomes a markdown list (one `- pN:` line
/// each) rather than a run-on with literal bullet characters. The glyphs are
/// dropped; each item's text stays a substring of the page text, so it still
/// re-locates. Returns the whole quote as a single item when there are no bullets.
pub fn split_bullets(quote: &str) -> Vec<String> {
    const BULLETS: &[char] = &['●', '•', '▪', '◦', '‣', '○', '◆', '■', '∙', '·'];
    if !quote.contains(|c| BULLETS.contains(&c)) {
        return vec![quote.to_string()];
    }
    let items: Vec<String> = quote
        .split(|c| BULLETS.contains(&c))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if items.is_empty() {
        vec![quote.to_string()]
    } else {
        items
    }
}

/// Strip a trailing reverse-link `[[…]]` and a trailing `{color}` tag from a quote,
/// returning the cleaned text and its color (the default fill when there's no known
/// `{color}` tag — so a quote that merely ends in `{…}` is left untouched).
fn strip_quote_meta(s: &str) -> (String, gpui::Hsla) {
    let mut quote = s.to_string();
    if let Some(b) = quote.find("[[") {
        quote = quote[..b].trim().to_string();
    }
    let mut color = default_highlight_color();
    if quote.ends_with('}')
        && let Some(b) = quote.rfind('{')
        && let Some(c) = known_color(quote[b + 1..quote.len() - 1].trim())
    {
        color = c;
        quote = quote[..b].trim().to_string();
    }
    (quote, color)
}

/// Record a highlight, assigning the next occurrence index for its `(page, quote)`.
fn push_highlight(
    out: &mut Vec<Highlight>,
    seen: &mut HashMap<(usize, String), usize>,
    id: usize,
    page: usize,
    quote: String,
    color: gpui::Hsla,
) {
    let occurrence = {
        let c = seen.entry((page, quote.clone())).or_insert(0);
        let v = *c;
        *c += 1;
        v
    };
    out.push(Highlight {
        id: id as u64,
        page,
        quote,
        occurrence,
        color,
    });
}

/// Parse a highlights page's markdown into viewer highlights.
///
/// A `- p{N}: {quote}` line (1-based page; trailing `{color}` + `[[link]]` stripped)
/// is one highlight. A `- p{N}:` line with **no** quote (just `{color}`/link) is a
/// *group header*: the indented `- quote` items beneath it are highlights on that
/// page + color — so a bulleted PDF selection reads as a markdown list. Repeated
/// `(page, quote)` pairs get successive occurrence indices.
pub fn parse_highlights(content: &str) -> Vec<Highlight> {
    let mut out = Vec::new();
    let mut seen: HashMap<(usize, String), usize> = HashMap::new();
    // The current group header's (page, color), set by a quote-less `- pN:` line and
    // applied to the indented items that follow.
    let mut group: Option<(usize, gpui::Hsla)> = None;
    for (i, raw) in content.lines().enumerate() {
        let trimmed = raw.trim_start();
        let indented = raw.len() != trimmed.len();
        let Some(body) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        else {
            group = None;
            continue;
        };
        // A `p{N}: …` line: a group header (empty quote) or a standalone highlight.
        let pn = body
            .strip_prefix(['p', 'P'])
            .and_then(|r| r.find(':').map(|c| (r, c)))
            .and_then(|(r, c)| {
                r[..c]
                    .trim()
                    .parse::<usize>()
                    .ok()
                    .filter(|n| *n >= 1)
                    .map(|n| (n - 1, r[c + 1..].trim()))
            });
        if let Some((page, rest)) = pn {
            let (quote, color) = strip_quote_meta(rest);
            if quote.is_empty() {
                group = Some((page, color));
            } else {
                group = None;
                push_highlight(&mut out, &mut seen, i, page, quote, color);
            }
            continue;
        }
        // An indented item beneath a header is one of its quotes.
        if indented && let Some((page, color)) = group {
            let (quote, _) = strip_quote_meta(body.trim());
            if !quote.is_empty() {
                push_highlight(&mut out, &mut seen, i, page, quote, color);
            }
            continue;
        }
        group = None;
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
    fn split_bullets_makes_one_item_per_bullet() {
        // A multi-bullet selection → one item each, glyphs + surrounding space stripped.
        assert_eq!(
            split_bullets("● First item. ● Second item."),
            vec!["First item.", "Second item."]
        );
        // Leading non-bullet text before the first bullet is kept as its own item.
        assert_eq!(split_bullets("Causes: • a • b"), vec!["Causes:", "a", "b"]);
        // No bullets → the whole quote, unchanged, as a single item.
        assert_eq!(split_bullets("just a sentence"), vec!["just a sentence"]);
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
    fn grouped_header_with_indented_items() {
        // A quote-less `- pN:` header + indented items → one highlight per item, all on
        // the header's page, all inheriting its color.
        let hs = parse_highlights(
            "- p21: {pink} [[m41t81s.pdf#p21|↗]]\n    - First item.\n    - Second item.",
        );
        assert_eq!(hs.len(), 2);
        assert_eq!(hs[0].page, 20);
        assert_eq!(hs[0].quote, "First item.");
        assert_eq!(hs[1].quote, "Second item.");
        assert_eq!(hs[0].color, known_color("pink").unwrap());
        assert_eq!(hs[0].color, hs[1].color);
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
