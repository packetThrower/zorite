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

use std::sync::{Arc, OnceLock};

/// The bundled default face.
const DEFAULT_FONT: &[u8] = include_bytes!("../assets/JetBrainsMono-Regular.ttf");

/// Floor for shape-label auto-shrink (world units): a label never shrinks below
/// this, even if it then slightly overflows a very small box. See [`Font::fit_size`].
const MIN_LABEL_SIZE: f32 = 1.0;

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
        Self::system_cjk_fallback().unwrap_or_else(|| Self {
            bytes: Arc::new(DEFAULT_FONT.to_vec()),
            index: 0,
        })
    }
}

impl Font {
    fn system_cjk_fallback() -> Option<Self> {
        static SYSTEM_CJK: OnceLock<Option<(Arc<Vec<u8>>, u32)>> = OnceLock::new();

        SYSTEM_CJK
            .get_or_init(Self::load_system_cjk_fallback)
            .as_ref()
            .map(|(bytes, index)| Self {
                bytes: bytes.clone(),
                index: *index,
            })
    }

    fn load_system_cjk_fallback() -> Option<(Arc<Vec<u8>>, u32)> {
        // The editor uses GPUI's text stack, which gets system font fallback for
        // free. Whiteboard text is converted to vector outlines, so we need a
        // single face that actually contains CJK glyphs. Prefer broad Unicode / CJK
        // system fonts, then fall back to the bundled Latin-only JetBrains Mono.
        // This lookup is cached by `system_cjk_fallback`: CJK collections are often
        // tens of megabytes and must not be re-read for every whiteboard entity.
        let candidates: &[(&str, u32)] = &[
            // macOS
            ("/Library/Fonts/Arial Unicode.ttf", 0),
            ("/System/Library/Fonts/PingFang.ttc", 0),
            ("/System/Library/Fonts/Hiragino Sans GB.ttc", 0),
            ("/System/Library/Fonts/STHeiti Medium.ttc", 0),
            ("/System/Library/Fonts/STHeiti Light.ttc", 0),
            // Windows
            ("C:/Windows/Fonts/msyh.ttc", 0),
            ("C:/Windows/Fonts/simhei.ttf", 0),
            ("C:/Windows/Fonts/simsun.ttc", 0),
            // Linux common CJK packages
            ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0),
            ("/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc", 0),
            (
                "/usr/share/fonts/opentype/noto/NotoSansCJKsc-Regular.otf",
                0,
            ),
        ];

        candidates.iter().find_map(|(path, index)| {
            let bytes = std::fs::read(path).ok()?;
            ttf_parser::Face::parse(&bytes, *index).ok()?;
            Some((Arc::new(bytes), *index))
        })
    }

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
        self.layout_wrapped(content, font_size, None)
    }

    /// Like [`layout`](Self::layout) but word-wraps to `max_width` (world units)
    /// when `Some` — for fitting a label inside a shape. Glyph positions come
    /// from the same [`line_stops`](Self::line_stops) the caret uses, so the
    /// caret always lands exactly between the rendered glyphs.
    pub fn layout_wrapped(
        &self,
        content: &str,
        font_size: f32,
        max_width: Option<f32>,
    ) -> TextLayout {
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
        let (lines, line_height) = self.line_stops(content, font_size, max_width);

        let mut segs = Vec::new();
        let mut width = 0.0_f32;
        // `line_stops` always yields ≥1 visual line, so "" is one empty line and
        // a trailing newline yields a final empty line holding the caret.
        for line in &lines {
            let lstart = line.stops.first().map_or(0, |&(b, _)| b);
            let lend = line.stops.last().map_or(lstart, |&(b, _)| b);
            let baseline = line.top + ascent;
            for (k, ch) in content.get(lstart..lend).unwrap_or("").chars().enumerate() {
                let pen = line.stops.get(k).map_or(0.0, |&(_, x)| x);
                let gid = face.glyph_index(ch).unwrap_or(ttf_parser::GlyphId(0));
                let mut b = Outliner {
                    segs: &mut segs,
                    pen,
                    baseline,
                    scale,
                    shear: 0.0,
                };
                face.outline_glyph(gid, &mut b);
            }
            width = width.max(line.stops.last().map_or(0.0, |&(_, x)| x));
        }
        let caret = lines.last().map_or([0.0, 0.0], |l| {
            let &(_, x) = l.stops.last().unwrap();
            [x, l.top]
        });
        TextLayout {
            segs,
            width,
            height: lines.len().max(1) as f32 * line_height,
            line_height,
            caret,
        }
    }

    /// Like [`layout_wrapped`](Self::layout_wrapped) but applies a per-character
    /// [`GlyphStyle`] from `style_at` (keyed by byte offset): synthetic italic
    /// (shear) and bold (a doubled, offset pass) are baked into the glyph
    /// outlines, and underline / strikethrough / highlight runs become
    /// [`Decoration`]s. Wrapping — and therefore the caret geometry — is identical
    /// to the unstyled path (synthetic styling doesn't change advances).
    pub fn layout_styled(
        &self,
        content: &str,
        font_size: f32,
        max_width: Option<f32>,
        style_at: impl Fn(usize) -> GlyphStyle,
    ) -> StyledLayout {
        let Some(face) = self.face() else {
            return StyledLayout {
                segs: Vec::new(),
                bold_segs: Vec::new(),
                bold_width: 0.0,
                decorations: Vec::new(),
                width: 0.0,
                height: font_size,
                line_height: font_size,
                caret: [0.0, 0.0],
            };
        };
        let upem = face.units_per_em() as f32;
        let scale = if upem > 0.0 { font_size / upem } else { 0.0 };
        let ascent = face.ascender() as f32 * scale;
        let (lines, line_height) = self.line_stops(content, font_size, max_width);

        const SHEAR: f32 = 0.22; // ~12° oblique
        let bar = (font_size * 0.07).max(1.0); // underline / strike thickness

        let mut segs = Vec::new();
        // Bold glyphs' outlines, stroked over the solid fill (`bold_width`). A
        // doubled fill would cancel under even-odd winding → hollow glyphs.
        let mut bold_segs = Vec::new();
        let mut decorations = Vec::new();
        let mut width = 0.0_f32;
        for line in &lines {
            let lstart = line.stops.first().map_or(0, |&(b, _)| b);
            let lend = line.stops.last().map_or(lstart, |&(b, _)| b);
            let baseline = line.top + ascent;
            // The current decoration run: its style and starting local x. Only the
            // underline/strike/highlight bits matter (bold/italic are glyph-baked).
            let mut run: Option<(GlyphStyle, f32)> = None;
            let mut byte = lstart;
            for (k, ch) in content.get(lstart..lend).unwrap_or("").chars().enumerate() {
                let st = style_at(byte);
                let pen = line.stops.get(k).map_or(0.0, |&(_, x)| x);
                let gid = face.glyph_index(ch).unwrap_or(ttf_parser::GlyphId(0));
                let shear = if st.italic { SHEAR } else { 0.0 };
                let start = segs.len();
                {
                    let mut b = Outliner {
                        segs: &mut segs,
                        pen,
                        baseline,
                        scale,
                        shear,
                    };
                    face.outline_glyph(gid, &mut b);
                }
                if st.bold {
                    bold_segs.extend_from_slice(&segs[start..]);
                }
                // Track the decoration run; flush when underline/strike/highlight change.
                let deco = GlyphStyle {
                    bold: false,
                    italic: false,
                    ..st
                };
                let has_deco = deco.underline || deco.strike || deco.highlight.is_some();
                let same = matches!(&run, Some((rs, _)) if *rs == deco);
                if !same {
                    if let Some((rs, x0)) = run.take() {
                        flush_deco(
                            &mut decorations,
                            rs,
                            x0,
                            pen,
                            line.top,
                            line_height,
                            baseline,
                            bar,
                        );
                    }
                    run = has_deco.then_some((deco, pen));
                }
                byte += ch.len_utf8();
            }
            if let Some((rs, x0)) = run.take() {
                let x1 = line.stops.last().map_or(x0, |&(_, x)| x);
                flush_deco(
                    &mut decorations,
                    rs,
                    x0,
                    x1,
                    line.top,
                    line_height,
                    baseline,
                    bar,
                );
            }
            width = width.max(line.stops.last().map_or(0.0, |&(_, x)| x));
        }
        let caret = lines.last().map_or([0.0, 0.0], |l| {
            let &(_, x) = l.stops.last().unwrap();
            [x, l.top]
        });
        StyledLayout {
            segs,
            bold_segs,
            bold_width: font_size * 0.06,
            decorations,
            width,
            height: lines.len().max(1) as f32 * line_height,
            line_height,
            caret,
        }
    }

    /// The block's `(width, height)` at `font_size` without building outlines —
    /// for selection bounds and hit-testing (no `Window` needed).
    pub fn measure(&self, content: &str, font_size: f32) -> (f32, f32) {
        self.measure_wrapped(content, font_size, None)
    }

    /// Like [`measure`](Self::measure) but wraps to `max_width` (world units)
    /// when `Some` — the height then reflects the wrapped line count.
    pub fn measure_wrapped(
        &self,
        content: &str,
        font_size: f32,
        max_width: Option<f32>,
    ) -> (f32, f32) {
        let (lines, line_height) = self.line_stops(content, font_size, max_width);
        let width = lines.iter().fold(0.0_f32, |m, l| {
            m.max(l.stops.last().map_or(0.0, |&(_, x)| x))
        });
        (width, lines.len().max(1) as f32 * line_height)
    }

    /// The largest font size in `(0, max_size]` at which `content`, wrapped to
    /// `max_w`, fits within a `max_w × max_h` box (world units) — so a shape
    /// label can shrink until it no longer crosses the shape's border. Returns
    /// `max_size` for empty content or a non-positive box, and never goes below
    /// [`MIN_LABEL_SIZE`] (a tiny box may then still be overflowed).
    pub fn fit_size(&self, content: &str, max_w: f32, max_h: f32, max_size: f32) -> f32 {
        if content.is_empty() || max_w <= 0.0 || max_h <= 0.0 {
            return max_size;
        }
        let fits = |size: f32| {
            let (w, h) = self.measure_wrapped(content, size, Some(max_w));
            w <= max_w + 0.5 && h <= max_h + 0.5
        };
        if fits(max_size) {
            return max_size;
        }
        let (mut lo, mut hi) = (MIN_LABEL_SIZE, max_size.max(MIN_LABEL_SIZE));
        for _ in 0..20 {
            let mid = 0.5 * (lo + hi);
            if fits(mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    }

    /// Per-visual-line caret stops for `content`. Lines break on `'\n'` always
    /// and, when `max_width` is `Some(w)` (world units), also wrap greedily at
    /// word boundaries so each visual line stays within `w`; a single word wider
    /// than `w` is split between characters so it never overflows. For every
    /// visual line: the content **byte offset** and local x of each caret
    /// position (before each char and after the last), plus the line's top y.
    /// Soft wraps consume no byte, so offsets stay contiguous across visual
    /// lines. The backbone of caret placement, click hit-testing, and selection
    /// rects. Byte offsets index into `content`.
    fn line_stops(
        &self,
        content: &str,
        font_size: f32,
        max_width: Option<f32>,
    ) -> (Vec<LineStops>, f32) {
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
        let wrap = max_width.filter(|w| *w > 0.0);

        let mut lines = Vec::new();
        let mut byte = 0usize;
        let mut top = 0.0_f32;
        for line in content.split('\n') {
            // Flat caret stops for the whole hard line (x cumulative from its
            // start), plus whether each char is a space (a wrap opportunity).
            let mut flat: Vec<(usize, f32)> = Vec::with_capacity(line.len() + 1);
            let mut space_after: Vec<bool> = Vec::with_capacity(line.len());
            let mut pen = 0.0_f32;
            let mut b = byte;
            flat.push((b, 0.0));
            for ch in line.chars() {
                let gid = face.glyph_index(ch).unwrap_or(ttf_parser::GlyphId(0));
                pen += face.glyph_hor_advance(gid).unwrap_or(0) as f32 * scale;
                b += ch.len_utf8();
                flat.push((b, pen));
                space_after.push(ch == ' ');
            }

            match wrap {
                // No wrapping: the whole hard line is one visual line.
                None => {
                    lines.push(LineStops { top, stops: flat });
                    top += line_height;
                }
                // Greedy word-wrap of `flat` into visual segments ≤ `w`.
                Some(w) => {
                    let n = flat.len();
                    let mut s = 0usize; // segment start index into `flat`
                    let mut e = 1usize; // current end index
                    let mut last_break: Option<usize> = None; // index just past a space
                    while e < n {
                        if flat[e].1 - flat[s].1 > w && e - s > 1 {
                            // Break at the last space if any, else split the word.
                            let brk = match last_break {
                                Some(lb) if lb > s => lb,
                                _ => e - 1,
                            };
                            let base = flat[s].1;
                            lines.push(LineStops {
                                top,
                                stops: flat[s..=brk]
                                    .iter()
                                    .map(|&(by, x)| (by, x - base))
                                    .collect(),
                            });
                            top += line_height;
                            s = brk;
                            e = s + 1;
                            last_break = None;
                            continue;
                        }
                        if space_after.get(e - 1).copied().unwrap_or(false) {
                            last_break = Some(e);
                        }
                        e += 1;
                    }
                    let base = flat[s].1;
                    lines.push(LineStops {
                        top,
                        stops: flat[s..n].iter().map(|&(by, x)| (by, x - base)).collect(),
                    });
                    top += line_height;
                }
            }
            byte += line.len() + 1; // +1 for the '\n' (harmless past the last line)
        }
        (lines, line_height)
    }

    /// Local-space top-left of the caret at content byte offset `at`. Out-of-range
    /// offsets clamp to the end of the text.
    pub fn caret_pos(&self, content: &str, font_size: f32, at: usize) -> [f32; 2] {
        self.caret_pos_wrapped(content, font_size, None, at)
    }

    /// [`caret_pos`](Self::caret_pos) honoring a `max_width` wrap (label editing).
    pub fn caret_pos_wrapped(
        &self,
        content: &str,
        font_size: f32,
        max_width: Option<f32>,
        at: usize,
    ) -> [f32; 2] {
        let (lines, _) = self.line_stops(content, font_size, max_width);
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
        self.index_at_wrapped(content, font_size, None, p)
    }

    /// [`index_at`](Self::index_at) honoring a `max_width` wrap (label editing).
    pub fn index_at_wrapped(
        &self,
        content: &str,
        font_size: f32,
        max_width: Option<f32>,
        p: [f32; 2],
    ) -> usize {
        let (lines, line_height) = self.line_stops(content, font_size, max_width);
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
        self.selection_rects_wrapped(content, font_size, None, start, end)
    }

    /// [`selection_rects`](Self::selection_rects) honoring a `max_width` wrap.
    pub fn selection_rects_wrapped(
        &self,
        content: &str,
        font_size: f32,
        max_width: Option<f32>,
        start: usize,
        end: usize,
    ) -> Vec<[f32; 4]> {
        if start >= end {
            return Vec::new();
        }
        let (lines, line_height) = self.line_stops(content, font_size, max_width);
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

/// The renderable style of one character — what [`Font::layout_styled`] needs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GlyphStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// Highlight color behind the glyphs, packed `0xRRGGBBAA`.
    pub highlight: Option<u32>,
}

/// A non-glyph text decoration in text-local space — transformed like the
/// glyphs at paint time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decoration {
    /// `[x, y, w, h]`.
    pub rect: [f32; 4],
    pub kind: DecoKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DecoKind {
    /// Filled rect behind the glyphs, in this color (`0xRRGGBBAA`).
    Highlight(u32),
    /// A bar drawn in the text color.
    Underline,
    Strike,
}

/// A laid-out styled block: glyph outlines (italic / bold baked in) plus
/// decoration rects, all text-local. Like [`TextLayout`] otherwise.
#[derive(Clone, Debug)]
pub struct StyledLayout {
    pub segs: Vec<Seg>,
    /// The bold glyphs' outlines, stroked over the fill for synthetic bold.
    pub bold_segs: Vec<Seg>,
    /// Local stroke width for `bold_segs` (scale to screen px by the zoom).
    pub bold_width: f32,
    pub decorations: Vec<Decoration>,
    pub width: f32,
    pub height: f32,
    pub line_height: f32,
    pub caret: [f32; 2],
}

/// Emit the highlight / underline / strikethrough rects for a decoration run
/// spanning local x `[x0, x1)` on one line.
// Each argument is a distinct glyph-run geometry/style value; bundling them into
// a struct would obscure more than it clarifies.
#[allow(clippy::too_many_arguments)]
fn flush_deco(
    out: &mut Vec<Decoration>,
    st: GlyphStyle,
    x0: f32,
    x1: f32,
    top: f32,
    line_height: f32,
    baseline: f32,
    bar: f32,
) {
    if x1 <= x0 {
        return;
    }
    let w = x1 - x0;
    if let Some(c) = st.highlight {
        out.push(Decoration {
            rect: [x0, top, w, line_height],
            kind: DecoKind::Highlight(c),
        });
    }
    if st.underline {
        out.push(Decoration {
            rect: [x0, baseline + bar, w, bar],
            kind: DecoKind::Underline,
        });
    }
    if st.strike {
        // Through the x-height (about halfway from the line top to the baseline).
        out.push(Decoration {
            rect: [x0, (top + baseline) * 0.5, w, bar],
            kind: DecoKind::Strike,
        });
    }
}

/// Accumulates a glyph's outline into local space as `ttf-parser` walks it.
struct Outliner<'a> {
    segs: &'a mut Vec<Seg>,
    pen: f32,
    baseline: f32,
    scale: f32,
    /// Synthetic-italic slant: local x shifts right with height above the
    /// baseline (`0.0` = upright).
    shear: f32,
}

impl Outliner<'_> {
    /// Font units (y-up, baseline origin) → text-local (y-down, top-left origin),
    /// applying the italic shear.
    fn pt(&self, x: f32, y: f32) -> [f32; 2] {
        [
            self.pen + (x + self.shear * y) * self.scale,
            self.baseline - y * self.scale,
        ]
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

    #[test]
    fn wrap_breaks_long_lines_at_spaces() {
        let f = Font::default();
        let lh = f.measure("x", 20.0).1;
        let full = f.measure("hello world", 20.0).0;
        // Wrapping below the phrase width forces a second line.
        let (w, h) = f.measure_wrapped("hello world", 20.0, Some(full * 0.6));
        assert!(
            (h - 2.0 * lh).abs() < 0.5,
            "expected 2 lines: h={h} lh={lh}"
        );
        assert!(w <= full * 0.6 + 0.5, "lines stay within the width: {w}");
        // A width wider than the text wraps nothing — identical to no wrap.
        assert_eq!(
            f.measure_wrapped("hello world", 20.0, Some(10_000.0)),
            f.measure("hello world", 20.0),
        );
    }

    #[test]
    fn wrap_keeps_byte_offsets_contiguous() {
        let f = Font::default();
        let s = "alpha beta gamma";
        let full = f.measure(s, 20.0).0;
        // The end-of-content caret is still reachable (no byte dropped at a soft
        // break) and sits lower than the single-line end (more visual lines).
        let end = f.caret_pos_wrapped(s, 20.0, Some(full * 0.5), s.len());
        let plain = f.caret_pos(s, 20.0, s.len());
        assert!(end[1] > plain[1], "wrapped end lower: {end:?} vs {plain:?}");
    }

    #[test]
    fn fit_size_shrinks_for_a_small_box() {
        let f = Font::default();
        let big = f.fit_size("hello world", 1000.0, 1000.0, 40.0);
        let small = f.fit_size("hello world", 60.0, 60.0, 40.0);
        assert_eq!(big, 40.0, "fits at max in a big box");
        assert!(small < big, "shrinks in a small box: {small} vs {big}");
        // The shrunk label actually fits inside its box.
        let (w, h) = f.measure_wrapped("hello world", small, Some(60.0));
        assert!(w <= 60.5 && h <= 60.5, "fits: {w}x{h}");
    }

    #[test]
    fn styled_layout_bolds_and_decorates() {
        let f = Font::default();
        let plain = f.layout_styled("Ab", 20.0, None, |_| GlyphStyle::default());
        let bold = f.layout_styled("Ab", 20.0, None, |_| GlyphStyle {
            bold: true,
            ..Default::default()
        });
        assert!(
            !bold.bold_segs.is_empty() && plain.bold_segs.is_empty(),
            "bold collects outlines to stroke"
        );
        assert!(plain.decorations.is_empty());
        let und = f.layout_styled("Ab", 20.0, None, |_| GlyphStyle {
            underline: true,
            ..Default::default()
        });
        assert!(
            und.decorations
                .iter()
                .any(|d| d.kind == DecoKind::Underline)
        );
        let hi = f.layout_styled("Ab", 20.0, None, |_| GlyphStyle {
            highlight: Some(0xffff00ff),
            ..Default::default()
        });
        assert!(
            hi.decorations
                .iter()
                .any(|d| d.kind == DecoKind::Highlight(0xffff00ff))
        );
    }

    #[test]
    fn styled_layout_decoration_runs_split() {
        let f = Font::default();
        // Underline only the first of two chars → one underline run ~1 advance wide.
        let l = f.layout_styled("MM", 20.0, None, |b| GlyphStyle {
            underline: b == 0,
            ..Default::default()
        });
        let unders: Vec<_> = l
            .decorations
            .iter()
            .filter(|d| d.kind == DecoKind::Underline)
            .collect();
        assert_eq!(unders.len(), 1, "{:?}", l.decorations);
        let adv = f.measure("M", 20.0).0;
        assert!(
            (unders[0].rect[2] - adv).abs() < 0.5,
            "≈1 advance: {}",
            unders[0].rect[2]
        );
    }
}
