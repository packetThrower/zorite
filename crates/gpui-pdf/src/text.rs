//! **Text layer** (`markup` feature): pull a page's text and per-glyph rectangles
//! out of the PDF by running a custom [`hayro`] [`Device`] over it — no
//! rasterization, no heavyweight PDF library (only `kurbo` geometry, already in
//! hayro's tree).
//!
//! This is the substrate for text-anchored markup. A host can take a quote stored in
//! a note, [`PageText::locate`] it on the page, and draw a highlight over the
//! returned rectangles — all in **normalized** (0..1) page coordinates, so it's
//! independent of zoom and display DPI. Matching ignores whitespace, so a quote
//! survives the small spacing quirks of PDF text extraction.

use hayro::hayro_interpret::font::Glyph;
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache,
    InterpreterSettings, Paint, PathDrawMode, SoftMask, TransformExt, interpret_page,
};
use kurbo::{Affine, BezPath, Point, Rect, Shape};

use crate::Document;

/// A rectangle in normalized page coordinates: each component is a fraction (0..1) of
/// the page's width/height, origin at the top-left. Resolution- and zoom-independent,
/// so a host maps it to the on-screen page rect at paint time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NormRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl NormRect {
    fn right(&self) -> f32 {
        self.x + self.w
    }
    fn bottom(&self) -> f32 {
        self.y + self.h
    }
    fn center_y(&self) -> f32 {
        self.y + self.h / 2.0
    }
}

/// A point in normalized page coordinates (0..1 of width/height, top-left origin).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NormPoint {
    pub x: f32,
    pub y: f32,
}

/// The result of a drag selection: the selected text (as a single-line quote), which
/// occurrence of that quote on the page it is (so it re-locates unambiguously), and
/// the rects to draw while selecting.
#[derive(Clone, Debug)]
pub struct Selection {
    pub quote: String,
    pub occurrence: usize,
    pub rects: Vec<NormRect>,
}

/// Squared distance from a normalized point to a rect (0 if inside).
fn dist2(p: NormPoint, r: NormRect) -> f32 {
    let dx = (r.x - p.x).max(p.x - r.right()).max(0.0);
    let dy = (r.y - p.y).max(p.y - r.bottom()).max(0.0);
    dx * dx + dy * dy
}

/// One extracted glyph cluster: its text (usually one char; a ligature may be
/// several) and its bounding rect on the page.
struct Run {
    text: String,
    rect: NormRect,
}

/// A device that ignores everything except glyphs, recording each one's unicode text
/// and normalized bounding rect — the entire text layer in one interpret pass.
struct Collector {
    runs: Vec<Run>,
    page_w: f32,
    page_h: f32,
}

impl<'a> Device<'a> for Collector {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'a>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}

    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        _paint: &Paint<'a>,
        // Note: we record *all* glyphs, including `Invisible` ones — that's the
        // searchable text layer over scanned/OCR'd page images.
        _draw_mode: &GlyphDrawMode,
    ) {
        let text = match glyph.as_unicode() {
            Some(BfString::Char(c)) => c.to_string(),
            Some(BfString::String(s)) => s,
            None => return, // no unicode → can't match against a quote; skip
        };
        if text.trim().is_empty() {
            return;
        }
        // On-page bbox: the glyph outline transformed by `transform * glyph_transform`
        // (mirrors hayro's own renderer). Device space here is page points with a
        // top-left origin (we run at scale 1 with `initial_transform(true)`).
        let m = transform * glyph_transform;
        let bbox = match glyph {
            Glyph::Outline(o) => {
                // Transform the glyph-space bbox corners by `m`, take their extent.
                let gb = o.outline().bounding_box();
                let corners = [
                    m * Point::new(gb.x0, gb.y0),
                    m * Point::new(gb.x1, gb.y0),
                    m * Point::new(gb.x1, gb.y1),
                    m * Point::new(gb.x0, gb.y1),
                ];
                let mut min = corners[0];
                let mut max = corners[0];
                for c in &corners[1..] {
                    min.x = min.x.min(c.x);
                    min.y = min.y.min(c.y);
                    max.x = max.x.max(c.x);
                    max.y = max.y.max(c.y);
                }
                Rect::new(min.x, min.y, max.x, max.y)
            }
            // Type3 glyphs (rare: procedural/bitmap) — approximate with a point box
            // at the pen origin so the text is still searchable.
            Glyph::Type3(_) => {
                let p = m.translation();
                Rect::new(p.x, p.y - 1.0, p.x + 1.0, p.y)
            }
        };
        if self.page_w <= 0.0 || self.page_h <= 0.0 {
            return;
        }
        let rect = NormRect {
            x: (bbox.x0 as f32 / self.page_w).clamp(0.0, 1.0),
            y: (bbox.y0 as f32 / self.page_h).clamp(0.0, 1.0),
            w: ((bbox.x1 - bbox.x0) as f32 / self.page_w).clamp(0.0, 1.0),
            h: ((bbox.y1 - bbox.y0) as f32 / self.page_h).clamp(0.0, 1.0),
        };
        self.runs.push(Run { text, rect });
    }
}

/// Extract the text layer of page `index` (0-based). Runs a non-rasterizing
/// interpret pass — cheaper than rendering, but still parses the page, so a host
/// should cache the result. Returns `None` if the page doesn't exist.
pub fn extract_page_text(doc: &Document, index: usize) -> Option<PageText> {
    let page = doc.pages().get(index)?;
    let (page_w, page_h) = page.render_dimensions();
    // Scale 1 + `initial_transform(true)` → device space is page points, top-left
    // origin, which we normalize by the page size.
    let initial = page.initial_transform(true).to_kurbo();
    let cache = InterpreterCache::new();
    let mut ctx = Context::new(
        initial,
        Rect::new(0.0, 0.0, page_w as f64, page_h as f64),
        &cache,
        page.xref(),
        InterpreterSettings::default(),
    );
    let mut collector = Collector {
        runs: Vec::new(),
        page_w,
        page_h,
    };
    interpret_page(page, &mut ctx, &mut collector);
    Some(PageText::new(collector.runs))
}

/// A page's extracted text: the runs in draw order, plus a whitespace-stripped,
/// lowercased index for robust quote matching.
pub struct PageText {
    runs: Vec<Run>,
    /// Lowercased, whitespace-removed concatenation of every run's text — what
    /// [`locate`](PageText::locate) searches.
    letters: String,
    /// `owner[b]` is the run index that produced byte `b` of `letters` (byte-aligned,
    /// so byte offsets from `find` / slicing map straight to runs, even across
    /// multi-byte chars).
    owner: Vec<usize>,
}

impl PageText {
    fn new(runs: Vec<Run>) -> Self {
        let mut letters = String::new();
        let mut owner = Vec::new();
        for (ri, run) in runs.iter().enumerate() {
            for ch in run.text.chars() {
                if ch.is_whitespace() {
                    continue;
                }
                for lc in ch.to_lowercase() {
                    let before = letters.len();
                    letters.push(lc);
                    // One owner entry per *byte*, so byte indices from `find` /
                    // slicing line up with `owner` even for multi-byte (non-ASCII)
                    // chars — otherwise a byte index can land mid-char and panic.
                    for _ in before..letters.len() {
                        owner.push(ri);
                    }
                }
            }
        }
        Self {
            runs,
            letters,
            owner,
        }
    }

    /// Whether the page has any extractable text (false for pure scans with no OCR
    /// layer — the host should then fall back to area markup).
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// A readable reconstruction of the page text (spaces inserted on horizontal
    /// gaps, newlines on baseline changes). For search / display — `locate` uses the
    /// whitespace-insensitive index instead.
    pub fn text(&self) -> String {
        let mut out = String::new();
        let mut prev: Option<&NormRect> = None;
        for run in &self.runs {
            if let Some(p) = prev {
                let line_h = run.rect.h.max(p.h).max(0.001);
                if (run.rect.center_y() - p.center_y()).abs() > line_h * 0.6 {
                    out.push('\n');
                } else if run.rect.x - p.right() > line_h * 0.25 {
                    out.push(' ');
                }
            }
            out.push_str(&run.text);
            prev = Some(&run.rect);
        }
        out
    }

    /// Locate the `occurrence`-th (0-based) case- and whitespace-insensitive match of
    /// `needle` on the page and return one normalized rect per line it spans (so a
    /// wrapped quote highlights as multiple line boxes). Empty if not found.
    pub fn locate(&self, needle: &str, occurrence: usize) -> Vec<NormRect> {
        let key: String = needle
            .chars()
            .filter(|c| !c.is_whitespace())
            .flat_map(|c| c.to_lowercase())
            .collect();
        if key.is_empty() {
            return Vec::new();
        }
        // Find the nth occurrence's byte range in `letters`.
        let mut from = 0;
        let mut hit = None;
        for _ in 0..=occurrence {
            match self.letters[from..].find(&key) {
                Some(rel) => {
                    let start = from + rel;
                    hit = Some(start);
                    from = start + 1;
                }
                None => {
                    hit = None;
                    break;
                }
            }
        }
        let Some(start) = hit else {
            return Vec::new();
        };
        let end = start + key.len();
        // Map the matched byte range to the set of runs it covers (in order).
        let mut run_ids: Vec<usize> = self.owner[start..end].to_vec();
        run_ids.dedup();
        self.group_lines(&run_ids)
    }

    /// Merge a set of run indices into one rect per text line (runs sharing a
    /// baseline), so a multi-line match yields one box per line.
    fn group_lines(&self, run_ids: &[usize]) -> Vec<NormRect> {
        let mut rects: Vec<NormRect> = run_ids.iter().map(|&i| self.runs[i].rect).collect();
        // Reading order: top-to-bottom, then left-to-right.
        rects.sort_by(|a, b| {
            a.y.partial_cmp(&b.y)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
        });
        let mut lines: Vec<NormRect> = Vec::new();
        for r in rects {
            if let Some(last) = lines.last_mut() {
                let line_h = last.h.max(r.h).max(0.001);
                if (r.center_y() - last.center_y()).abs() <= line_h * 0.6 {
                    // Same line — extend the union.
                    let x = last.x.min(r.x);
                    let y = last.y.min(r.y);
                    let right = last.right().max(r.right());
                    let bottom = last.bottom().max(r.bottom());
                    *last = NormRect {
                        x,
                        y,
                        w: right - x,
                        h: bottom - y,
                    };
                    continue;
                }
            }
            lines.push(r);
        }
        lines
    }

    /// The index of the run nearest a normalized point (0 distance if inside it).
    fn nearest_run(&self, p: NormPoint) -> Option<usize> {
        self.runs
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                dist2(p, a.rect)
                    .partial_cmp(&dist2(p, b.rect))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i)
    }

    /// The text of runs `lo..=hi` joined into one line (gaps and line breaks become
    /// spaces), suitable for storing as a one-line quote.
    fn runs_text(&self, lo: usize, hi: usize) -> String {
        let mut out = String::new();
        let mut prev: Option<&NormRect> = None;
        for run in &self.runs[lo..=hi] {
            if let Some(p) = prev {
                let line_h = run.rect.h.max(p.h).max(0.001);
                let new_line = (run.rect.center_y() - p.center_y()).abs() > line_h * 0.6;
                if new_line || run.rect.x - p.right() > line_h * 0.25 {
                    out.push(' ');
                }
            }
            out.push_str(&run.text);
            prev = Some(&run.rect);
        }
        out
    }

    /// Which occurrence (0-based) of `quote` on the page the run at `lo` begins — so
    /// the stored highlight re-locates to the right match.
    fn occurrence_at(&self, lo: usize, quote: &str) -> usize {
        let key: String = quote
            .chars()
            .filter(|c| !c.is_whitespace())
            .flat_map(|c| c.to_lowercase())
            .collect();
        if key.is_empty() {
            return 0;
        }
        let start = self.owner.iter().position(|&r| r == lo).unwrap_or(0);
        self.letters
            .get(..start)
            .map_or(0, |s| s.matches(&key).count())
    }

    /// Resolve a drag from `from` to `to` into a [`Selection`]: the run range between
    /// the nearest glyphs (draw order ≈ reading order), its one-line quote, the
    /// occurrence index, and the rects to draw. `None` if there's no text or the
    /// selection is empty.
    pub fn select(&self, from: NormPoint, to: NormPoint) -> Option<Selection> {
        let i0 = self.nearest_run(from)?;
        let i1 = self.nearest_run(to)?;
        let (lo, hi) = (i0.min(i1), i0.max(i1));
        let quote = self.runs_text(lo, hi);
        if quote.trim().is_empty() {
            return None;
        }
        let occurrence = self.occurrence_at(lo, &quote);
        let ids: Vec<usize> = (lo..=hi).collect();
        let rects = self.group_lines(&ids);
        Some(Selection {
            quote,
            occurrence,
            rects,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, x: f32, y: f32, w: f32, h: f32) -> Run {
        Run {
            text: text.to_string(),
            rect: NormRect { x, y, w, h },
        }
    }

    // "Hello World" on one line, then "second" on the next.
    fn sample() -> PageText {
        PageText::new(vec![
            run("Hello", 0.10, 0.10, 0.10, 0.02),
            run("World", 0.22, 0.10, 0.10, 0.02),
            run("second", 0.10, 0.14, 0.12, 0.02),
        ])
    }

    #[test]
    fn locate_ignores_case_and_whitespace() {
        let pt = sample();
        // "hello world" with a space matches "Hello"+"World" (no space stored).
        let rects = pt.locate("hello world", 0);
        assert_eq!(rects.len(), 1); // one line
        let r = rects[0];
        // Spans from Hello's x to World's right edge.
        assert!((r.x - 0.10).abs() < 1e-4);
        assert!((r.right() - 0.32).abs() < 1e-4);
    }

    #[test]
    fn locate_missing_is_empty() {
        assert!(sample().locate("absent", 0).is_empty());
    }

    #[test]
    fn locate_occurrence_index() {
        let pt = PageText::new(vec![
            run("cat", 0.1, 0.1, 0.06, 0.02),
            run("cat", 0.1, 0.2, 0.06, 0.02),
        ]);
        assert_eq!(pt.locate("cat", 0)[0].y, 0.1);
        assert_eq!(pt.locate("cat", 1)[0].y, 0.2);
        assert!(pt.locate("cat", 2).is_empty());
    }

    #[test]
    fn multi_line_match_yields_a_rect_per_line() {
        // "Helloworldsecond" spans two lines (Hello+World on line 1, second on line 2).
        let rects = sample().locate("HelloWorldsecond", 0);
        assert_eq!(rects.len(), 2);
        assert!(rects[0].y < rects[1].y);
    }

    #[test]
    fn text_reconstruction_inserts_space_and_newline() {
        let t = sample().text();
        assert_eq!(t, "Hello World\nsecond");
    }

    #[test]
    fn select_spans_runs_into_a_quote() {
        let pt = sample();
        // Drag from inside "Hello" to inside "World".
        let sel = pt
            .select(
                NormPoint { x: 0.12, y: 0.11 },
                NormPoint { x: 0.27, y: 0.11 },
            )
            .unwrap();
        assert_eq!(sel.quote, "Hello World");
        assert_eq!(sel.occurrence, 0);
        assert_eq!(sel.rects.len(), 1);
    }

    #[test]
    fn select_occurrence_counts_earlier_matches() {
        let pt = PageText::new(vec![
            run("cat", 0.1, 0.1, 0.06, 0.02),
            run("cat", 0.1, 0.2, 0.06, 0.02),
        ]);
        // Selecting the second "cat" reports occurrence 1.
        let sel = pt
            .select(NormPoint { x: 0.12, y: 0.2 }, NormPoint { x: 0.14, y: 0.2 })
            .unwrap();
        assert_eq!(sel.quote, "cat");
        assert_eq!(sel.occurrence, 1);
    }
}
