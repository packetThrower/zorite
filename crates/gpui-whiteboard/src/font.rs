//! On-canvas text rendering for the whiteboard.
//!
//! gpui paints text as glyph sprites whose transform is fixed to the identity,
//! so native text can't rotate or follow the camera the way the board needs.
//! Instead we render text as **vector outlines**: `ttf-parser` gives glyph
//! contours, which we lay out into board-local space and (in `lib.rs`) feed to a
//! `PathBuilder` fill — so text rotates, scales, and z-orders exactly like the
//! shapes. A face is just bytes, so a host can swap in a user-uploaded font; the
//! default (JetBrains Mono, OFL — see `assets/JetBrainsMono-OFL.txt`) is bundled
//! so the crate works standalone.

use std::sync::Arc;

/// The bundled default face.
const DEFAULT_FONT: &[u8] = include_bytes!("../assets/JetBrainsMono-Regular.ttf");

/// One glyph-outline command in text-local space: origin at the block's
/// top-left, x to the right, y *down* (screen-like), in the same world units as
/// `font_size`. Curves keep their control points so they transform under
/// rotation / the camera before being flattened by the path builder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Move([f32; 2]),
    Line([f32; 2]),
    /// Quadratic Bézier: control, end.
    Quad([f32; 2], [f32; 2]),
    /// Cubic Bézier: control1, control2, end.
    Cubic([f32; 2], [f32; 2], [f32; 2]),
    Close,
}

/// A laid-out block of text: glyph outline segments plus metrics, all in
/// text-local space (origin = the block's top-left corner).
#[derive(Clone, Debug)]
pub struct TextLayout {
    pub segs: Vec<Seg>,
    /// Width of the widest line.
    pub width: f32,
    /// Total height (line count × line height).
    pub height: f32,
    /// Distance between successive line tops.
    pub line_height: f32,
    /// Top-left of the caret (just past the content), for the editing cursor.
    pub caret: [f32; 2],
}

/// A font backing whiteboard text. Holds raw TTF/OTF bytes (parsed on demand) so
/// it's cheap to clone and a host can supply its own face.
#[derive(Clone)]
pub struct Font {
    bytes: Arc<Vec<u8>>,
    index: u32,
}

impl Default for Font {
    fn default() -> Self {
        Self {
            bytes: Arc::new(DEFAULT_FONT.to_vec()),
            index: 0,
        }
    }
}

impl Font {
    /// Build a font from raw face bytes (e.g. a user-uploaded `.ttf`/`.otf`),
    /// returning `None` if they don't parse. `index` selects a face within a
    /// collection (`.ttc`); pass 0 otherwise.
    pub fn from_bytes(bytes: Vec<u8>, index: u32) -> Option<Self> {
        ttf_parser::Face::parse(&bytes, index).ok()?;
        Some(Self {
            bytes: Arc::new(bytes),
            index,
        })
    }

    fn face(&self) -> Option<ttf_parser::Face<'_>> {
        ttf_parser::Face::parse(&self.bytes, self.index).ok()
    }

    /// The line height (top-to-top spacing) at `font_size`, world units.
    fn line_height_of(face: &ttf_parser::Face, scale: f32) -> f32 {
        (face.ascender() as f32 - face.descender() as f32 + face.line_gap() as f32) * scale
    }

    /// Lay out `content` at `font_size` (world units) into local-space outlines.
    /// Newlines break lines; the block's origin is its top-left corner.
    pub fn layout(&self, content: &str, font_size: f32) -> TextLayout {
        let Some(face) = self.face() else {
            return TextLayout {
                segs: Vec::new(),
                width: 0.0,
                height: font_size,
                line_height: font_size,
                caret: [0.0, 0.0],
            };
        };
        let upem = face.units_per_em() as f32;
        let scale = if upem > 0.0 { font_size / upem } else { 0.0 };
        let ascent = face.ascender() as f32 * scale;
        let line_height = Self::line_height_of(&face, scale);

        let mut segs = Vec::new();
        let mut width = 0.0_f32;
        let (mut last_pen, mut last_top, mut lines) = (0.0_f32, 0.0_f32, 0usize);
        // `split('\n')` always yields ≥1 item, so "" is one empty line and a
        // trailing newline yields a final empty line holding the caret.
        for (li, line) in content.split('\n').enumerate() {
            lines = li + 1;
            let top = li as f32 * line_height;
            let baseline = top + ascent;
            let mut pen = 0.0_f32;
            for ch in line.chars() {
                let gid = face.glyph_index(ch).unwrap_or(ttf_parser::GlyphId(0));
                let mut b = Outliner {
                    segs: &mut segs,
                    pen,
                    baseline,
                    scale,
                };
                face.outline_glyph(gid, &mut b);
                pen += face.glyph_hor_advance(gid).unwrap_or(0) as f32 * scale;
            }
            width = width.max(pen);
            last_pen = pen;
            last_top = top;
        }
        TextLayout {
            segs,
            width,
            height: lines.max(1) as f32 * line_height,
            line_height,
            caret: [last_pen, last_top],
        }
    }

    /// The block's `(width, height)` at `font_size` without building outlines —
    /// for selection bounds and hit-testing (no `Window` needed).
    pub fn measure(&self, content: &str, font_size: f32) -> (f32, f32) {
        let Some(face) = self.face() else {
            return (0.0, font_size);
        };
        let upem = face.units_per_em() as f32;
        let scale = if upem > 0.0 { font_size / upem } else { 0.0 };
        let line_height = Self::line_height_of(&face, scale);
        let mut width = 0.0_f32;
        let mut lines = 0usize;
        for line in content.split('\n') {
            lines += 1;
            let mut pen = 0.0_f32;
            for ch in line.chars() {
                let gid = face.glyph_index(ch).unwrap_or(ttf_parser::GlyphId(0));
                pen += face.glyph_hor_advance(gid).unwrap_or(0) as f32 * scale;
            }
            width = width.max(pen);
        }
        (width, lines.max(1) as f32 * line_height)
    }

    /// Per-line caret stops for `content`: for every line, the content **byte
    /// offset** and local x of each caret position (before each char and after the
    /// last), plus the line's top y. The backbone of caret placement, click
    /// hit-testing, and selection rects. Byte offsets index into `content`.
    fn line_stops(&self, content: &str, font_size: f32) -> (Vec<LineStops>, f32) {
        let Some(face) = self.face() else {
            return (
                vec![LineStops {
                    top: 0.0,
                    stops: vec![(0, 0.0)],
                }],
                font_size,
            );
        };
        let upem = face.units_per_em() as f32;
        let scale = if upem > 0.0 { font_size / upem } else { 0.0 };
        let line_height = Self::line_height_of(&face, scale);
        let mut lines = Vec::new();
        let mut byte = 0usize;
        for (li, line) in content.split('\n').enumerate() {
            let mut stops = Vec::with_capacity(line.chars().count() + 1);
            let mut pen = 0.0_f32;
            let mut b = byte;
            stops.push((b, 0.0));
            for ch in line.chars() {
                let gid = face.glyph_index(ch).unwrap_or(ttf_parser::GlyphId(0));
                pen += face.glyph_hor_advance(gid).unwrap_or(0) as f32 * scale;
                b += ch.len_utf8();
                stops.push((b, pen));
            }
            lines.push(LineStops {
                top: li as f32 * line_height,
                stops,
            });
            byte += line.len() + 1; // +1 for the '\n' (harmless past the last line)
        }
        (lines, line_height)
    }

    /// Local-space top-left of the caret at content byte offset `at`. Out-of-range
    /// offsets clamp to the end of the text.
    pub fn caret_pos(&self, content: &str, font_size: f32, at: usize) -> [f32; 2] {
        let (lines, _) = self.line_stops(content, font_size);
        for line in &lines {
            // A boundary byte belongs to exactly one line (the '\n' separates
            // line i's end from line i+1's start), so the first match is correct.
            for &(b, x) in &line.stops {
                if b == at {
                    return [x, line.top];
                }
            }
        }
        lines
            .last()
            .map(|l| {
                let &(_, x) = l.stops.last().unwrap();
                [x, l.top]
            })
            .unwrap_or([0.0, 0.0])
    }

    /// The content byte offset whose caret sits nearest the local point `p`
    /// (text-local space). Picks the line by y, then the closest caret stop by x —
    /// so a click lands the caret between letters like a real text field.
    pub fn index_at(&self, content: &str, font_size: f32, p: [f32; 2]) -> usize {
        let (lines, line_height) = self.line_stops(content, font_size);
        if lines.is_empty() {
            return 0;
        }
        let li = if line_height > 0.0 {
            (p[1] / line_height)
                .floor()
                .clamp(0.0, (lines.len() - 1) as f32) as usize
        } else {
            0
        };
        let line = &lines[li];
        let mut best = line.stops[0];
        for &(b, x) in &line.stops {
            if (x - p[0]).abs() < (best.1 - p[0]).abs() {
                best = (b, x);
            }
        }
        best.0
    }

    /// Local-space highlight rectangles `[x, y, w, h]` for the selection
    /// `[start, end)` (byte offsets), one per line it covers. Lines fully inside a
    /// multi-line selection get a small trailing stub so the selected newline reads.
    pub fn selection_rects(
        &self,
        content: &str,
        font_size: f32,
        start: usize,
        end: usize,
    ) -> Vec<[f32; 4]> {
        if start >= end {
            return Vec::new();
        }
        let (lines, line_height) = self.line_stops(content, font_size);
        let stub = font_size * 0.3; // trailing width shown for a selected newline
        let mut rects = Vec::new();
        for line in &lines {
            let lstart = line.stops[0].0;
            let lend = line.stops[line.stops.len() - 1].0;
            if end <= lstart || start > lend {
                continue; // selection doesn't touch this line
            }
            let x_at = |byte: usize| {
                line.stops
                    .iter()
                    .find(|(b, _)| *b == byte)
                    .map_or(0.0, |(_, x)| *x)
            };
            let x0 = x_at(start.max(lstart));
            let x1 = x_at(end.min(lend));
            // The newline (and anything below) is selected when `end` runs past
            // this line — show a stub so an end-of-line/empty-line selection reads.
            let w = (x1 - x0).max(if end > lend { stub } else { 0.0 });
            if w > 0.0 {
                rects.push([x0, line.top, w, line_height]);
            }
        }
        rects
    }
}

/// One line's caret stops: its top y and `(content byte offset, local x)` at each
/// caret position (before each char and after the last). See [`Font::line_stops`].
struct LineStops {
    top: f32,
    stops: Vec<(usize, f32)>,
}

/// Accumulates a glyph's outline into local space as `ttf-parser` walks it.
struct Outliner<'a> {
    segs: &'a mut Vec<Seg>,
    pen: f32,
    baseline: f32,
    scale: f32,
}

impl Outliner<'_> {
    /// Font units (y-up, baseline origin) → text-local (y-down, top-left origin).
    fn pt(&self, x: f32, y: f32) -> [f32; 2] {
        [self.pen + x * self.scale, self.baseline - y * self.scale]
    }
}

impl ttf_parser::OutlineBuilder for Outliner<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.pt(x, y);
        self.segs.push(Seg::Move(p));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.pt(x, y);
        self.segs.push(Seg::Line(p));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let c = self.pt(x1, y1);
        let e = self.pt(x, y);
        self.segs.push(Seg::Quad(c, e));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let c1 = self.pt(x1, y1);
        let c2 = self.pt(x2, y2);
        let e = self.pt(x, y);
        self.segs.push(Seg::Cubic(c1, c2, e));
    }
    fn close(&mut self) {
        self.segs.push(Seg::Close);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_font_parses_and_lays_out() {
        let f = Font::default();
        let l = f.layout("A", 20.0);
        assert!(l.width > 0.0, "width {}", l.width);
        assert!(l.line_height > 0.0);
        assert!(
            l.segs.iter().any(|s| matches!(s, Seg::Move(_))),
            "expected contours"
        );
        // JetBrains Mono is monospace: "MM" is twice the advance of "M".
        let one = f.measure("M", 20.0).0;
        let two = f.measure("MM", 20.0).0;
        assert!(one > 0.0 && (two - 2.0 * one).abs() < 0.5, "{one} {two}");
    }

    #[test]
    fn newlines_stack_lines_and_place_the_caret() {
        let f = Font::default();
        let one = f.measure("x", 20.0).1;
        let two = f.measure("x\ny", 20.0).1;
        assert!((two - 2.0 * one).abs() < 0.5, "{one} {two}");
        // The caret of a two-line block sits on the lower line.
        let l = f.layout("x\ny", 20.0);
        assert!(l.caret[1] > 0.0, "caret {:?}", l.caret);
    }

    #[test]
    fn empty_content_has_no_segments_but_a_caret() {
        let f = Font::default();
        let l = f.layout("", 20.0);
        assert!(l.segs.is_empty());
        assert_eq!(l.caret, [0.0, 0.0]);
        assert!(l.height > 0.0);
    }

    #[test]
    fn bad_bytes_are_rejected() {
        assert!(Font::from_bytes(vec![0, 1, 2, 3], 0).is_none());
    }

    #[test]
    fn caret_pos_walks_per_char_and_across_lines() {
        let f = Font::default();
        let a = f.measure("M", 20.0).0; // monospace advance
        let lh = f.layout("M", 20.0).line_height;
        // One stop per boundary on a line.
        assert_eq!(f.caret_pos("MMM", 20.0, 0), [0.0, 0.0]);
        assert!((f.caret_pos("MMM", 20.0, 1)[0] - a).abs() < 0.5);
        assert!((f.caret_pos("MMM", 20.0, 3)[0] - 3.0 * a).abs() < 0.5);
        // Byte 2 of "M\nM" is the start of the second line (col 0, lower row).
        let c = f.caret_pos("M\nM", 20.0, 2);
        assert!(c[0].abs() < 0.5 && (c[1] - lh).abs() < 0.5, "{c:?}");
        // Byte 3 is one advance into that second line.
        let c = f.caret_pos("M\nM", 20.0, 3);
        assert!((c[0] - a).abs() < 0.5 && (c[1] - lh).abs() < 0.5, "{c:?}");
    }

    #[test]
    fn index_at_picks_the_nearest_boundary() {
        let f = Font::default();
        let a = f.measure("M", 20.0).0;
        let lh = f.layout("M", 20.0).line_height;
        // Just past the first glyph's midpoint rounds to the next boundary.
        assert_eq!(f.index_at("MMM", 20.0, [0.4 * a, 0.0]), 0);
        assert_eq!(f.index_at("MMM", 20.0, [0.6 * a, 0.0]), 1);
        // Way off to the right clamps to the line end; clicking the lower line
        // (by y) lands on it.
        assert_eq!(f.index_at("MMM", 20.0, [99.0 * a, 0.0]), 3);
        assert_eq!(f.index_at("M\nMM", 20.0, [0.0, lh]), 2); // start of line 2
    }

    #[test]
    fn selection_rects_span_lines() {
        let f = Font::default();
        let a = f.measure("M", 20.0).0;
        // Single line, two chars selected → one rect ~2 advances wide.
        let r = f.selection_rects("MMMM", 20.0, 1, 3);
        assert_eq!(r.len(), 1);
        assert!(
            (r[0][0] - a).abs() < 0.5 && (r[0][2] - 2.0 * a).abs() < 0.5,
            "{r:?}"
        );
        // Across a newline → one rect per line.
        let r = f.selection_rects("M\nMM", 20.0, 0, 4);
        assert_eq!(r.len(), 2, "{r:?}");
        // Empty selection → nothing.
        assert!(f.selection_rects("MM", 20.0, 2, 2).is_empty());
    }
}
