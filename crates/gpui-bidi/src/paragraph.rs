//! Logical-order line breaking for right-to-left paragraphs.
//!
//! # Why this exists
//!
//! gpui shapes a paragraph as ONE long line and only then slices it into wrap
//! rows, walking the glyph table by ascending x
//! (`LineLayout::compute_wrap_boundaries`). Glyph order is *visual*, so in an
//! RTL paragraph the leftmost glyphs — the ones that end up in the first row —
//! are the paragraph's LAST words. A wrapped Persian note therefore reads
//! bottom-to-top, and no amount of alignment fixes it: the row contents
//! themselves are wrong.
//!
//! UAX #9 says to do it the other way round: break into lines in *logical*
//! order first, then reorder each line independently. That is what this module
//! does, and it needs nothing from gpui that isn't already public:
//!
//! - `TextSystem::shape_text` splits its input on `\n` and shapes each line
//!   separately, in order. So injecting a `\n` at each logical break makes the
//!   shaper do the per-line reordering for us, in the right sequence.
//! - Where the breaks GO is ours. gpui's `LineWrapper` walks the text (its
//!   offsets are logical, so it looked like the tool for this) but its
//!   `is_word_char` knows Latin, Cyrillic, Vietnamese and Bengali and not
//!   Arabic, so Persian takes its "CJK may not be space separated" path and
//!   every character becomes a break candidate — words split down the middle.
//!   Arabic script is space-separated, so [`wrap_at_words`] measures words.
//!
//! The whole trick is [`insert_breaks`]: turn one paragraph into a `\n`-joined
//! sequence of lines that already fit. Everything after that is ordinary
//! painting.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use crate::VisualMap;

use gpui::{
    App, AvailableSpace, Bounds, Element, ElementId, GlobalElementId, HighlightStyle, Hitbox,
    HitboxBehavior, InspectorElementId, IntoElement, LayoutId, Pixels, Point, SharedString, Size,
    TextAlign, TextRun, TextStyle, Window, WrappedLine, px,
};

/// Split `text` into lines at `breaks` (logical byte offsets, ascending) by
/// injecting `\n`, and grow `runs` to cover the injected bytes.
///
/// `shape_text` requires the runs to span every byte of the text it is handed,
/// including the `\n`s (it consumes one byte of run per line break), so a
/// break inside a run lengthens that run by one rather than splitting it — the
/// newline inherits the style of the text it interrupts, which is invisible
/// either way since a line break paints nothing.
///
/// Offsets at 0, at `text.len()`, or repeated are ignored: they would produce
/// an empty line, which would paint as a blank row the reader never asked for.
pub fn insert_breaks(
    text: &str,
    runs: &[TextRun],
    breaks: &[usize],
) -> (SharedString, Vec<TextRun>) {
    let mut wanted: Vec<usize> = breaks
        .iter()
        .copied()
        .filter(|ix| *ix > 0 && *ix < text.len() && text.is_char_boundary(*ix))
        .collect();
    wanted.sort_unstable();
    wanted.dedup();
    if wanted.is_empty() {
        return (SharedString::from(text.to_string()), runs.to_vec());
    }

    let mut out = String::with_capacity(text.len() + wanted.len());
    let mut last = 0usize;
    for ix in &wanted {
        out.push_str(&text[last..*ix]);
        out.push('\n');
        last = *ix;
    }
    out.push_str(&text[last..]);

    // Walk the runs alongside the break list, widening whichever run contains
    // each break. A break exactly on a run boundary belongs to the run that
    // ENDS there, matching how the text was split above.
    let mut new_runs = Vec::with_capacity(runs.len());
    let mut run_start = 0usize;
    let mut next = 0usize;
    for run in runs {
        let run_end = run_start + run.len;
        let mut extra = 0usize;
        while next < wanted.len() && wanted[next] <= run_end {
            if wanted[next] > run_start {
                extra += 1;
            }
            next += 1;
        }
        let mut run = run.clone();
        run.len += extra;
        new_runs.push(run);
        run_start = run_end;
    }
    // Any break past the last run (runs that don't cover the text — gpui logs
    // and truncates in that case) has nowhere to go; the text still carries it.
    (SharedString::from(out), new_runs)
}

/// The byte range of each word in `text`, where a word runs up to and including
/// the spaces that follow it — trailing spaces belong to the line they end, the
/// same convention gpui's wrapper uses.
///
/// Only ASCII space and tab separate words. Notably NOT the zero-width
/// non-joiner (U+200C), which sits *inside* Persian words (می‌گیرد) and would
/// split them if treated as a break.
fn words(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        while i < bytes.len() && !matches!(bytes[i], b' ' | b'\t') {
            i += 1;
        }
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
            i += 1;
        }
        out.push(start..i);
    }
    out
}

/// The sub-runs covering `range`, so a slice of text can be measured with the
/// styles it actually has (a bold word is wider than the same word in regular).
fn slice_runs(runs: &[TextRun], range: Range<usize>) -> Vec<TextRun> {
    let mut out = Vec::new();
    let mut at = 0usize;
    for run in runs {
        let end = at + run.len;
        let lo = at.max(range.start);
        let hi = end.min(range.end);
        if lo < hi {
            let mut r = run.clone();
            r.len = hi - lo;
            out.push(r);
        }
        at = end;
        if at >= range.end {
            break;
        }
    }
    out
}

/// Greedy word wrap: the byte offsets where a new line should start.
///
/// Words are measured once each and summed, rather than re-measuring the whole
/// line as it grows — shaping is per-word for Arabic script (contextual forms
/// join within a word, never across a space), so the sum is exact and the pass
/// stays linear. A single word wider than `wrap_width` overflows rather than
/// being chopped mid-word: chopping is what the reader complained about.
fn wrap_at_words(
    text: &str,
    runs: &[TextRun],
    wrap_width: Pixels,
    font_size: Pixels,
    window: &Window,
) -> Vec<usize> {
    let mut breaks = Vec::new();
    let mut line_width = px(0.);
    let mut line_has_word = false;
    for word in words(text) {
        let measured = window.text_system().shape_line(
            SharedString::from(text[word.clone()].to_string()),
            font_size,
            &slice_runs(runs, word.clone()),
            None,
        );
        // Trailing spaces don't push a line over: they hang past the edge, as
        // they do in every text engine.
        let trimmed = text[word.clone()].trim_end();
        let ink = if trimmed.len() == word.len() {
            measured.width
        } else {
            window
                .text_system()
                .shape_line(
                    SharedString::from(trimmed.to_string()),
                    font_size,
                    &slice_runs(runs, word.start..word.start + trimmed.len()),
                    None,
                )
                .width
        };

        if line_has_word && line_width + ink > wrap_width {
            breaks.push(word.start);
            line_width = measured.width;
        } else {
            line_width += measured.width;
        }
        line_has_word = true;
    }
    breaks
}

/// A paragraph laid out in logical order, then painted right-aligned.
///
/// Build it for blocks whose base direction is RTL; LTR text needs none of
/// this and should keep using `StyledText`, which is cheaper and carries the
/// interactive-text machinery.
pub struct RtlText {
    text: SharedString,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    pointer_ranges: Vec<Range<usize>>,
    layout: RtlLayout,
}

/// Expand `highlights` into runs covering all of `text`, the way `StyledText`
/// does — its own `compute_runs` is private, but it is only `to_run` and
/// `highlight`, both public. Ranges must be sorted and non-overlapping.
fn runs_from_highlights(
    text: &str,
    default_style: &TextStyle,
    highlights: &[(Range<usize>, HighlightStyle)],
) -> Vec<TextRun> {
    let mut runs = Vec::new();
    let mut ix = 0;
    for (range, highlight) in highlights {
        if range.start > text.len() || range.end > text.len() || range.start < ix {
            continue;
        }
        if ix < range.start {
            runs.push(default_style.clone().to_run(range.start - ix));
        }
        runs.push(
            default_style
                .clone()
                .highlight(*highlight)
                .to_run(range.len()),
        );
        ix = range.end;
    }
    if ix < text.len() {
        runs.push(default_style.to_run(text.len() - ix));
    }
    runs
}

/// What the measure pass worked out, reused by paint.
struct Laid {
    lines: Vec<WrappedLine>,
    /// Per line, in the same order: where it starts in the ORIGINAL text (the
    /// injected `\n`s don't exist there) and how to read its glyphs.
    rows: Vec<Row>,
    line_height: Pixels,
    wrap_width: Option<Pixels>,
    size: Size<Pixels>,
    /// Where the element was last painted — hit-testing is in window space, so
    /// there is nothing to map against until it has been on screen once.
    bounds: Option<Bounds<Pixels>>,
}

/// One painted row: its span in the original text, plus the visual map that
/// turns a logical offset inside it into an x and back.
struct Row {
    /// Byte offset in the ORIGINAL text where this row starts.
    start: usize,
    /// Byte length of the row's own text.
    len: usize,
    width: Pixels,
    map: VisualMap,
}

/// A handle onto the last layout, for hosts that need to map between a point on
/// screen and an offset in the text — link hit-testing, click-to-caret, and
/// positioning an inline formula over its spacer.
///
/// Mirrors `StyledText::layout()`: cheap to clone, empty until first paint.
#[derive(Clone, Default)]
pub struct RtlLayout(Rc<RefCell<Option<Laid>>>);

impl RtlLayout {
    /// Height of one row, or zero before the first layout.
    pub fn line_height(&self) -> Pixels {
        self.0.borrow().as_ref().map_or(px(0.), |l| l.line_height)
    }

    /// The logical byte offset under `position`.
    ///
    /// `Ok` when the point is inside the painted text, `Err` with the nearest
    /// offset when it is outside — the same contract as gpui's
    /// `TextLayout::index_for_position`, so callers can treat them alike.
    pub fn index_for_position(&self, position: Point<Pixels>) -> Result<usize, usize> {
        let state = self.0.borrow();
        let Some(laid) = state.as_ref() else {
            return Err(0);
        };
        let Some(bounds) = laid.bounds else {
            return Err(0);
        };
        if laid.rows.is_empty() {
            return Err(0);
        }

        let rel_y = position.y - bounds.origin.y;
        let row_f = f32::from(rel_y) / f32::from(laid.line_height).max(1.0);
        let outside = row_f < 0.0 || row_f >= laid.rows.len() as f32;
        let row_ix = (row_f.floor().max(0.0) as usize).min(laid.rows.len() - 1);
        let row = &laid.rows[row_ix];

        // Rows are right-aligned across the full width (see `paint`), so the
        // row's own coordinate space starts at its left edge, not the element's.
        let row_left = bounds.origin.x + bounds.size.width - row.width;
        let local = f32::from(position.x - row_left);
        let offset = row.start + row.map.index_for_x(local);
        if outside || position.x < row_left || position.x > bounds.origin.x + bounds.size.width {
            Err(offset)
        } else {
            Ok(offset)
        }
    }

    /// Top-left corner of the visual box that logical `range` occupies.
    ///
    /// A logical range is one contiguous run of text but not necessarily one
    /// contiguous box — in mixed text it can be split across the line — so the
    /// leftmost edge of all its pieces is what's returned. That is where an
    /// inline raster (a formula, an image) sits over the spacer it reserved:
    /// anchoring to `range.start` would be the RIGHT edge in an RTL line and
    /// would paint the raster over the neighbouring words.
    pub fn left_edge_of(&self, range: Range<usize>) -> Option<Point<Pixels>> {
        let state = self.0.borrow();
        let laid = state.as_ref()?;
        let bounds = laid.bounds?;
        let (row_ix, row) = laid
            .rows
            .iter()
            .enumerate()
            .find(|(_, r)| range.start >= r.start && range.start <= r.start + r.len)?;
        let local = range.start.saturating_sub(row.start)..range.end.saturating_sub(row.start);
        let left = row
            .map
            .rects_for_range(local)
            .into_iter()
            .map(|(x0, _)| x0)
            .fold(f32::INFINITY, f32::min);
        if !left.is_finite() {
            return None;
        }
        let row_left = bounds.origin.x + bounds.size.width - row.width;
        Some(Point {
            x: row_left + px(left),
            y: bounds.origin.y + laid.line_height * row_ix,
        })
    }

    /// Where the glyph at logical `offset` starts, in window space. `None`
    /// before the first paint, or if the offset is past the end of the text.
    pub fn position_for_index(&self, offset: usize) -> Option<Point<Pixels>> {
        let state = self.0.borrow();
        let laid = state.as_ref()?;
        let bounds = laid.bounds?;
        let (row_ix, row) = laid
            .rows
            .iter()
            .enumerate()
            .find(|(_, r)| offset >= r.start && offset <= r.start + r.len)?;
        let row_left = bounds.origin.x + bounds.size.width - row.width;
        Some(Point {
            x: row_left + px(row.map.x_for_index(offset - row.start)),
            y: bounds.origin.y + laid.line_height * row_ix,
        })
    }
}

impl RtlText {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            highlights: Vec::new(),
            pointer_ranges: Vec::new(),
            layout: RtlLayout::default(),
        }
    }

    /// Ranges that should show the pointing-hand cursor on hover — links.
    ///
    /// `InteractiveText` does this for LTR text, but it hit-tests through
    /// gpui's layout, which is the thing that's wrong for an RTL line. Same
    /// approach as gpui's, though: one hitbox for the element, and the hover
    /// test runs at paint time against the mouse's current position.
    pub fn with_pointer_ranges(mut self, ranges: Vec<Range<usize>>) -> Self {
        self.pointer_ranges = ranges;
        self
    }

    /// The handle hosts hit-test through — clone it before the element is
    /// consumed by the tree, exactly as `StyledText::layout()` is used.
    pub fn layout(&self) -> &RtlLayout {
        &self.layout
    }

    /// Styled ranges, exactly as `StyledText::with_highlights` takes them:
    /// sorted, non-overlapping, on char boundaries.
    pub fn with_highlights(
        mut self,
        highlights: impl IntoIterator<Item = (Range<usize>, HighlightStyle)>,
    ) -> Self {
        self.highlights = highlights.into_iter().collect();
        self
    }
}

impl IntoElement for RtlText {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for RtlText {
    type RequestLayoutState = ();
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        _cx: &mut App,
    ) -> (LayoutId, ()) {
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let line_height = window.pixel_snap(
            text_style
                .line_height
                .to_pixels(font_size.into(), window.rem_size()),
        );
        let text = self.text.clone();
        let runs = runs_from_highlights(&text, &text_style, &self.highlights);
        let state = self.layout.0.clone();

        let id = window.request_measured_layout(
            Default::default(),
            move |known, available, window, _cx| {
                let wrap_width = known.width.or(match available.width {
                    AvailableSpace::Definite(w) => Some(w),
                    _ => None,
                });

                if let Some(laid) = state.borrow().as_ref()
                    && laid.wrap_width == wrap_width
                {
                    return laid.size;
                }

                // Break in LOGICAL order, at word boundaries we pick ourselves.
                //
                // gpui's `LineWrapper` walks the text (so its offsets ARE
                // logical, which is why this looked like the obvious tool), but
                // its `is_word_char` covers Latin, Cyrillic, Vietnamese and
                // Bengali — not Arabic. Persian therefore takes its "CJK may
                // not be space separated" branch, which makes every character a
                // break candidate and splits words down the middle. Arabic
                // script IS space-separated, so we measure words instead.
                let breaks: Vec<usize> = match wrap_width {
                    Some(w) => wrap_at_words(&text, &runs, w, font_size, window),
                    None => Vec::new(),
                };
                let (broken, broken_runs) = insert_breaks(&text, &runs, &breaks);

                // Each line now fits, so shaping with no wrap width leaves the
                // rows exactly where we put them — and each is reordered on
                // its own, which is the whole point.
                let lines = window
                    .text_system()
                    .shape_text(broken, font_size, &broken_runs, None, None)
                    .map(|l| l.into_iter().collect::<Vec<_>>())
                    .unwrap_or_default();

                // Walk the rows back onto the ORIGINAL text. `shape_text` hands
                // each line its own text and glyph indices relative to it, and
                // the injected `\n`s exist only in the string we built — so a
                // row starting at `orig` in the source is `orig + rel` here.
                let mut rows = Vec::with_capacity(lines.len());
                let mut orig = 0usize;
                for line in &lines {
                    let len = line.text.len();
                    rows.push(Row {
                        start: orig,
                        len,
                        width: line.width(),
                        map: crate::shaped::map_of_wrapped(line, len),
                    });
                    // +1 for the break we injected, which the source lacks.
                    orig += len + 1;
                }

                let height = line_height * lines.len().max(1);
                let widest = lines.iter().map(|l| l.width()).fold(px(0.), Pixels::max);
                let size = Size {
                    width: wrap_width.unwrap_or(widest),
                    height,
                };
                let bounds = state.borrow().as_ref().and_then(|l| l.bounds);
                *state.borrow_mut() = Some(Laid {
                    lines,
                    rows,
                    line_height,
                    wrap_width,
                    size,
                    bounds,
                });
                size
            },
        );
        (id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        _cx: &mut App,
    ) -> Option<Hitbox> {
        // Only paragraphs with links need one; plain prose shouldn't pay for a
        // hitbox it will never consult.
        (!self.pointer_ranges.is_empty())
            .then(|| window.insert_hitbox(bounds, HitboxBehavior::Normal))
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        hitbox: &mut Option<Hitbox>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut state = self.layout.0.borrow_mut();
        let Some(laid) = state.as_mut() else {
            return;
        };
        // Hit-testing happens in window space, so the mapping is only valid
        // once we know where the element actually landed.
        laid.bounds = Some(bounds);
        // Top-to-bottom in the order we broke them: logical order. Each row is
        // right-aligned across the full width, which is where an RTL reader
        // starts — gpui's own `TextAlign` handles the within-row placement.
        for (i, line) in laid.lines.iter().enumerate() {
            let origin = Point {
                x: bounds.origin.x,
                y: bounds.origin.y + laid.line_height * i,
            };
            let _ = line.paint(
                origin,
                laid.line_height,
                TextAlign::Right,
                Some(bounds),
                window,
                cx,
            );
        }
        drop(state);

        // Hovering a link shows the hand, same as `InteractiveText` gives LTR
        // text — resolved through OUR mapping, so it lands on the right words
        // in a reordered line.
        if let Some(hitbox) = hitbox
            && let Ok(ix) = self.layout.index_for_position(window.mouse_position())
            && self.pointer_ranges.iter().any(|r| r.contains(&ix))
        {
            window.set_cursor_style(gpui::CursorStyle::PointingHand, hitbox);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Hsla, font};

    fn run(len: usize) -> TextRun {
        TextRun {
            len,
            font: font("Helvetica"),
            color: Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }
    }

    #[test]
    fn breaks_split_text_and_widen_the_run_that_holds_them() {
        let text = "one two three";
        let (out, runs) = insert_breaks(text, &[run(text.len())], &[4, 8]);
        assert_eq!(out.as_ref(), "one \ntwo \nthree");
        // Runs must still cover every byte, newlines included, or `shape_text`
        // warns and drops the tail.
        assert_eq!(runs.iter().map(|r| r.len).sum::<usize>(), out.len());
    }

    #[test]
    fn a_break_falls_in_the_run_that_ends_at_it() {
        let text = "boldplain";
        // Two runs: "bold" then "plain". A break exactly at 4 belongs to the
        // first, so the newline inherits the style it interrupts.
        let (out, runs) = insert_breaks(text, &[run(4), run(5)], &[4]);
        assert_eq!(out.as_ref(), "bold\nplain");
        assert_eq!(runs[0].len, 5);
        assert_eq!(runs[1].len, 5);
        assert_eq!(runs.iter().map(|r| r.len).sum::<usize>(), out.len());
    }

    #[test]
    fn degenerate_offsets_never_make_an_empty_row() {
        let text = "hello";
        // 0 and len would each produce a blank line; duplicates would produce
        // one per repeat.
        let (out, runs) = insert_breaks(text, &[run(5)], &[0, 5, 2, 2]);
        assert_eq!(out.as_ref(), "he\nllo");
        assert_eq!(runs.iter().map(|r| r.len).sum::<usize>(), out.len());
    }

    /// Walk `runs` over `broken` exactly as `TextSystem::shape_text` does —
    /// per line, then consuming ONE byte of the run at the front for the `\n`
    /// it skips — and report the colour landing on each byte. This is the
    /// contract `insert_breaks` has to satisfy: get the newline's owner wrong
    /// and every colour after the first break slides.
    fn colours_per_byte(broken: &str, runs: &[TextRun]) -> Vec<Hsla> {
        let mut queue: Vec<TextRun> = runs.to_vec();
        let mut out = Vec::new();
        let mut q = 0usize;
        for (i, line) in broken.split('\n').enumerate() {
            if i > 0 {
                // The `\n` itself: shape_text charges it to the front run.
                if let Some(run) = queue.get_mut(q) {
                    run.len -= 1;
                    if run.len == 0 {
                        q += 1;
                    }
                }
                out.push(Hsla::default()); // placeholder for the newline byte
            }
            let mut taken = 0usize;
            while taken < line.len() {
                let Some(run) = queue.get_mut(q) else {
                    panic!(
                        "runs ran out with {} bytes of line left",
                        line.len() - taken
                    );
                };
                let take = (line.len() - taken).min(run.len);
                for _ in 0..take {
                    out.push(run.color);
                }
                run.len -= take;
                if run.len == 0 {
                    q += 1;
                }
                taken += take;
            }
        }
        out
    }

    #[test]
    fn a_break_does_not_slide_the_colours_after_it() {
        let text = "plain LINK plain";
        let blue = Hsla {
            h: 0.6,
            s: 1.0,
            l: 0.5,
            a: 1.0,
        };
        let style = TextStyle::default();
        let runs = runs_from_highlights(
            text,
            &style,
            &[(
                6..10,
                HighlightStyle {
                    color: Some(blue),
                    ..Default::default()
                },
            )],
        );
        // Break BEFORE the link and again after it, so the link sits on its own
        // row — the arrangement that shifts if the newline is charged wrong.
        let (broken, broken_runs) = insert_breaks(text, &runs, &[6, 11]);
        assert_eq!(broken.as_ref(), "plain \nLINK \nplain");
        let colours = colours_per_byte(&broken, &broken_runs);
        assert_eq!(colours.len(), broken.len());
        let link_at = broken.find("LINK").unwrap();
        for (i, c) in colours.iter().enumerate() {
            let in_link = (link_at..link_at + 4).contains(&i);
            assert_eq!(
                *c == blue,
                in_link,
                "byte {i} ({:?}) coloured wrong",
                &broken[i..(i + 1).min(broken.len())]
            );
        }
    }

    #[test]
    fn highlighted_ranges_keep_their_colour_and_still_cover_the_text() {
        // A link's colour reaches the shaper through these runs, so a gap or a
        // dropped range shows up as unstyled text on screen.
        let text = "before LINK after";
        let blue = Hsla {
            h: 0.6,
            s: 1.0,
            l: 0.5,
            a: 1.0,
        };
        let style = TextStyle::default();
        let runs = runs_from_highlights(
            text,
            &style,
            &[(
                7..11,
                HighlightStyle {
                    color: Some(blue),
                    ..Default::default()
                },
            )],
        );
        assert_eq!(runs.iter().map(|r| r.len).sum::<usize>(), text.len());
        let coloured: Vec<_> = runs.iter().filter(|r| r.color == blue).collect();
        assert_eq!(coloured.len(), 1, "exactly the highlighted range is blue");
        assert_eq!(coloured[0].len, 4, "and it covers LINK, nothing more");
    }

    #[test]
    fn words_keep_trailing_spaces_and_never_split_on_a_non_joiner() {
        assert_eq!(words("one two"), vec![0..4, 4..7]);
        assert_eq!(words("a  b"), vec![0..3, 3..4]);
        // U+200C (ZWNJ) is 3 bytes and lives INSIDE Persian words: می‌گیرد is
        // one word, and breaking at it would split it visually.
        let w = "می\u{200C}گیرد دنیا";
        assert_eq!(words(w).len(), 2);
        assert!(w[words(w)[0].clone()].contains('\u{200C}'));
    }

    #[test]
    fn sliced_runs_cover_exactly_the_requested_range() {
        let runs = [run(4), run(6)];
        let got = slice_runs(&runs, 2..7);
        assert_eq!(got.iter().map(|r| r.len).sum::<usize>(), 5);
        assert_eq!(got.len(), 2, "the range straddles both runs");
        assert_eq!(slice_runs(&runs, 0..0).len(), 0);
    }

    #[test]
    fn no_breaks_leaves_the_paragraph_untouched() {
        let text = "سلام دنیا";
        let (out, runs) = insert_breaks(text, &[run(text.len())], &[]);
        assert_eq!(out.as_ref(), text);
        assert_eq!(runs[0].len, text.len());
    }

    #[test]
    fn breaks_land_on_char_boundaries_of_multibyte_text() {
        // "سلام دنیا" — the space is at byte 8, inside multibyte content.
        let text = "سلام دنیا";
        let (out, _) = insert_breaks(text, &[run(text.len())], &[9]);
        assert_eq!(out.as_ref(), "سلام \nدنیا");
        // A mid-codepoint offset is refused rather than panicking the shaper.
        let (same, _) = insert_breaks(text, &[run(text.len())], &[1]);
        assert_eq!(same.as_ref(), text);
    }
}
