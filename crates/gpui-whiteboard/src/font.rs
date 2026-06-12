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
}
