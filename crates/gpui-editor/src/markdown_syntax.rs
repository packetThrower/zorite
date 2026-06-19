//! Inline-markdown styling for live-preview ("WYSIWYG") editing — W1.
//!
//! A fast, line-scoped scanner that turns markdown source into styled text runs
//! **without changing the byte buffer**: syntax markers (`**`, `` ` ``, `[` …)
//! are kept in the text and merely dimmed, so the displayed characters still
//! match the buffer one-to-one and the caret/offset model is untouched.
//!
//! Scope (W1): inline constructs at the editor's normal text size — bold,
//! italic, strikethrough, inline code, links, wiki-links, and tags. Heading
//! sizes need per-line font sizes (variable line heights) and are deferred to
//! W2; block widgets (images, fenced code, tables, …) to W4.

use std::ops::Range;

use gpui::{
    Font, FontStyle, FontWeight, Hsla, StrikethroughStyle, TextRun, UnderlineStyle, hsla, px,
};

use crate::Diagnostic;

/// Colors + monospace font for inline markdown styling, supplied by the host so
/// the editor stays theme-agnostic. Install via
/// [`crate::EditorState::set_markdown_style`]; absent it, the editor renders
/// plain text (only spell-check underlines).
#[derive(Clone)]
pub struct SyntaxStyle {
    /// Dimmed color for the syntax markers themselves (`**`, `*`, `~~`, `[`,
    /// `]`, `[[`, `]]`, `]( … )`).
    pub marker: Hsla,
    /// Inline `code` text color.
    pub code: Hsla,
    /// Inline `code` background.
    pub code_bg: Hsla,
    /// `[text](url)` and `[[wiki-links]]`.
    pub link: Hsla,
    /// `#tags`.
    pub tag: Hsla,
    /// Monospace font for inline code.
    pub mono: Font,
}

/// Styling a scanned span adds on top of the editor's base run.
#[derive(Clone, Default)]
struct Style {
    bold: bool,
    italic: bool,
    strike: bool,
    mono: bool,
    color: Option<Hsla>,
    bg: Option<Hsla>,
    /// A syntax marker (`**`, `#`, `[`, …) — dimmed when shown, and removed
    /// entirely when the line's markers are hidden (W6, reveal-on-caret).
    hide: bool,
}

struct Span {
    range: Range<usize>,
    style: Style,
}

/// Build the editor's text runs: the base style, plus inline-markdown styling
/// when `md` is `Some`, plus a red wavy underline on each diagnostic span.
///
/// With `md = None` and no diagnostics this is a single plain run, so it
/// subsumes the former diagnostics-only builder.
pub(crate) fn styled_runs(
    text: &str,
    base_font: &Font,
    base_color: Hsla,
    diagnostics: &[Diagnostic],
    md: Option<&SyntaxStyle>,
) -> Vec<TextRun> {
    let spans = md.map(|s| scan(text, s)).unwrap_or_default();
    let squiggle = UnderlineStyle {
        color: Some(hsla(0., 0.8, 0.55, 1.)),
        thickness: px(1.5),
        wavy: true,
    };

    // Every point where styling can change: span + diagnostic edges (clamped).
    let mut bounds: Vec<usize> = vec![0, text.len()];
    for s in &spans {
        bounds.push(s.range.start);
        bounds.push(s.range.end);
    }
    for d in diagnostics {
        if d.range.start < d.range.end && d.range.end <= text.len() {
            bounds.push(d.range.start);
            bounds.push(d.range.end);
        }
    }
    bounds.retain(|&b| b <= text.len());
    bounds.sort_unstable();
    bounds.dedup();

    let mut runs = Vec::new();
    for win in bounds.windows(2) {
        let (a, b) = (win[0], win[1]);
        if a >= b {
            continue;
        }
        // Spans don't overlap, so the first covering one is THE one.
        let style = spans
            .iter()
            .find(|s| s.range.start <= a && a < s.range.end)
            .map(|s| &s.style);
        let underline = diagnostics
            .iter()
            .any(|d| d.range.start <= a && a < d.range.end && d.range.end <= text.len());

        let mut font = match style {
            Some(s) if s.mono => md.map_or_else(|| base_font.clone(), |m| m.mono.clone()),
            _ => base_font.clone(),
        };
        if let Some(s) = style {
            if s.bold {
                font.weight = FontWeight::BOLD;
            }
            if s.italic {
                font.style = FontStyle::Italic;
            }
        }
        runs.push(TextRun {
            len: b - a,
            font,
            color: style.and_then(|s| s.color).unwrap_or(base_color),
            background_color: style.and_then(|s| s.bg),
            underline: underline.then_some(squiggle),
            strikethrough: style.filter(|s| s.strike).map(|_| StrikethroughStyle {
                thickness: px(1.5),
                color: None,
            }),
        });
    }
    runs
}

/// Build a `TextRun` of byte length `len` from a scanned span's style (or the
/// base style when `None`). Shared by the hidden-line renderer.
fn run_for(
    len: usize,
    style: Option<&Style>,
    base_font: &Font,
    base_color: Hsla,
    md: Option<&SyntaxStyle>,
    underline: Option<UnderlineStyle>,
) -> TextRun {
    let mut font = match style {
        Some(s) if s.mono => md.map_or_else(|| base_font.clone(), |m| m.mono.clone()),
        _ => base_font.clone(),
    };
    if let Some(s) = style {
        if s.bold {
            font.weight = FontWeight::BOLD;
        }
        if s.italic {
            font.style = FontStyle::Italic;
        }
    }
    TextRun {
        len,
        font,
        color: style.and_then(|s| s.color).unwrap_or(base_color),
        background_color: style.and_then(|s| s.bg),
        underline,
        strikethrough: style.filter(|s| s.strike).map(|_| StrikethroughStyle {
            thickness: px(1.5),
            color: None,
        }),
    }
}

/// The maximal run of adjacent spans (a markdown construct: its markers + body)
/// containing source byte `c`, or an empty range when `c` is in plain text.
/// `spans` must be sorted by `range.start` and non-overlapping. Used to reveal
/// only the construct the caret sits in (#5).
fn construct_at(spans: &[Span], c: usize) -> Range<usize> {
    let mut i = 0;
    while i < spans.len() {
        let start = spans[i].range.start;
        let mut end = spans[i].range.end;
        let mut j = i + 1;
        while j < spans.len() && spans[j].range.start == end {
            end = spans[j].range.end;
            j += 1;
        }
        if start <= c && c <= end {
            return start..end;
        }
        i = j;
    }
    0..0
}

/// Render `line` with its syntax markers HIDDEN (W6): returns the display string
/// (source minus the marker chars), the styled runs over it, and a per-display-
/// byte map back to the source byte offset (length `display.len() + 1`, so the
/// end position maps too). Used for every styled line that isn't fully revealed;
/// the construct under `caret_col` (if any) keeps its markers (#5). The caller
/// maps caret/selection columns through the returned map (see `display_col`).
/// Spans don't overlap and cover the line in order, so each non-marker segment
/// contributes one run of its byte length.
pub(crate) fn hidden_runs(
    line: &str,
    base_font: &Font,
    base_color: Hsla,
    diagnostics: &[Diagnostic],
    caret_col: Option<usize>,
    md: &SyntaxStyle,
) -> (String, Vec<TextRun>, Vec<usize>) {
    let mut spans = scan(line, md);
    spans.sort_by_key(|s| s.range.start);

    // The construct (a maximal run of adjacent spans: its markers + body) the
    // caret sits in keeps its markers visible — everything else hides them (#5,
    // per-construct reveal). Empty range when the caret is in plain text / absent.
    let reveal = caret_col.map_or(0..0, |c| construct_at(&spans, c));

    // The visible segments (markers dropped), each a source byte range + style.
    // A visible segment is copied verbatim, so source↔display is 1:1 within it —
    // which lets a diagnostic (source coords) map straight onto the display.
    let mut segs: Vec<(Range<usize>, Option<&Style>)> = Vec::new();
    let mut pos = 0;
    for span in &spans {
        if span.range.start > pos {
            segs.push((pos..span.range.start, None));
        }
        let hidden =
            span.style.hide && !(reveal.start <= span.range.start && span.range.end <= reveal.end);
        if !hidden {
            segs.push((span.range.clone(), Some(&span.style)));
        }
        pos = span.range.end;
    }
    if pos < line.len() {
        segs.push((pos..line.len(), None));
    }

    let squiggle = UnderlineStyle {
        color: Some(hsla(0., 0.8, 0.55, 1.)),
        thickness: px(1.5),
        wavy: true,
    };
    let mut display = String::with_capacity(line.len());
    let mut runs: Vec<TextRun> = Vec::new();
    let mut map: Vec<usize> = Vec::with_capacity(line.len() + 1);
    for (src, style) in &segs {
        display.push_str(&line[src.clone()]);
        map.extend(src.clone());
        // Split the segment at any diagnostic edges falling inside it, so the
        // covered pieces get a spell-check squiggle (W6 lines kept their markers
        // hidden but were dropping these underlines).
        let mut edges = vec![src.start, src.end];
        for d in diagnostics.iter().filter(|d| d.range.start < d.range.end) {
            for e in [d.range.start, d.range.end] {
                if e > src.start && e < src.end {
                    edges.push(e);
                }
            }
        }
        edges.sort_unstable();
        edges.dedup();
        for w in edges.windows(2) {
            let (a, b) = (w[0], w[1]);
            let under = diagnostics
                .iter()
                .any(|d| d.range.start <= a && a < d.range.end);
            runs.push(run_for(
                b - a,
                *style,
                base_font,
                base_color,
                Some(md),
                under.then_some(squiggle),
            ));
        }
    }
    map.push(line.len());
    (display, runs, map)
}

/// Scan the whole document line by line for inline markdown constructs.
fn scan(text: &str, st: &SyntaxStyle) -> Vec<Span> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut line_start = 0;
    for i in 0..=bytes.len() {
        if i == bytes.len() || bytes[i] == b'\n' {
            scan_line(text, line_start, i, st, &mut out);
            line_start = i + 1;
        }
    }
    out
}

fn marker(out: &mut Vec<Span>, range: Range<usize>, color: Hsla) {
    out.push(Span {
        range,
        style: Style {
            color: Some(color),
            hide: true,
            ..Default::default()
        },
    });
}

/// Scan one line `text[start..end]`. Markers are ASCII, so byte scanning is
/// UTF-8-safe (an ASCII byte never appears inside a multi-byte char).
fn scan_line(text: &str, start: usize, end: usize, st: &SyntaxStyle, out: &mut Vec<Span>) {
    let b = text.as_bytes();
    // Heading: `#`..`######` + a space. Dim the marker, bold the rest. The
    // larger heading SIZE is applied per line at layout time (variable line
    // heights), not here — so the rest of the line isn't scanned for inline
    // constructs in W2. (W2)
    if let Some(level) = heading_level(&text[start..end]) {
        let mut marker_end = start + level as usize;
        if marker_end < end && b[marker_end] == b' ' {
            marker_end += 1;
        }
        marker(out, start..marker_end, st.marker);
        if marker_end < end {
            push(
                out,
                marker_end..end,
                Style {
                    bold: true,
                    ..Default::default()
                },
            );
        }
        return;
    }
    let mut i = start;
    while i < end {
        let c = b[i];
        // Inline code: `code` — backticks are hideable markers, the body a
        // highlight (code color on a tint, body font) matching the reading view.
        if c == b'`'
            && let Some(close) = find1(b, i + 1, end, b'`')
        {
            marker(out, i..i + 1, st.marker);
            push(
                out,
                i + 1..close,
                Style {
                    color: Some(st.code),
                    bg: Some(st.code_bg),
                    ..Default::default()
                },
            );
            marker(out, close..close + 1, st.marker);
            i = close + 1;
            continue;
        }
        // Bold: **text** (check before single-* italic).
        if c == b'*'
            && i + 1 < end
            && b[i + 1] == b'*'
            && let Some(close) = find2(b, i + 2, end, b'*', b'*')
        {
            marker(out, i..i + 2, st.marker);
            push(
                out,
                i + 2..close,
                Style {
                    bold: true,
                    ..Default::default()
                },
            );
            marker(out, close..close + 2, st.marker);
            i = close + 2;
            continue;
        }
        // Italic: *text* (asterisks only in W1 — `_` collides with snake_case).
        if c == b'*'
            && let Some(close) = find1(b, i + 1, end, b'*')
            && close > i + 1
        {
            marker(out, i..i + 1, st.marker);
            push(
                out,
                i + 1..close,
                Style {
                    italic: true,
                    ..Default::default()
                },
            );
            marker(out, close..close + 1, st.marker);
            i = close + 1;
            continue;
        }
        // Strikethrough: ~~text~~
        if c == b'~'
            && i + 1 < end
            && b[i + 1] == b'~'
            && let Some(close) = find2(b, i + 2, end, b'~', b'~')
        {
            marker(out, i..i + 2, st.marker);
            push(
                out,
                i + 2..close,
                Style {
                    strike: true,
                    ..Default::default()
                },
            );
            marker(out, close..close + 2, st.marker);
            i = close + 2;
            continue;
        }
        // Wiki-link: [[Page]] (check before single-[ link).
        if c == b'['
            && i + 1 < end
            && b[i + 1] == b'['
            && let Some(close) = find2(b, i + 2, end, b']', b']')
        {
            marker(out, i..i + 2, st.marker);
            push(
                out,
                i + 2..close,
                Style {
                    color: Some(st.link),
                    ..Default::default()
                },
            );
            marker(out, close..close + 2, st.marker);
            i = close + 2;
            continue;
        }
        // Link: [text](url) — `text` colored, brackets + target dimmed.
        if c == b'['
            && let Some(rb) = find1(b, i + 1, end, b']')
            && rb + 1 < end
            && b[rb + 1] == b'('
            && let Some(rp) = find1(b, rb + 2, end, b')')
        {
            marker(out, i..i + 1, st.marker);
            push(
                out,
                i + 1..rb,
                Style {
                    color: Some(st.link),
                    ..Default::default()
                },
            );
            marker(out, rb..rp + 1, st.marker);
            i = rp + 1;
            continue;
        }
        // Tag: #tag (at a non-word boundary; needs at least one tag char).
        if c == b'#' && (i == start || !is_word(b[i - 1])) {
            let mut j = i + 1;
            while j < end && is_tag(b[j]) {
                j += 1;
            }
            if j > i + 1 {
                push(
                    out,
                    i..j,
                    Style {
                        color: Some(st.tag),
                        ..Default::default()
                    },
                );
                i = j;
                continue;
            }
        }
        i += 1;
    }
}

fn push(out: &mut Vec<Span>, range: Range<usize>, style: Style) {
    out.push(Span { range, style });
}

/// First index of byte `c` in `b[from..end]`.
fn find1(b: &[u8], from: usize, end: usize, c: u8) -> Option<usize> {
    (from..end).find(|&k| b[k] == c)
}

/// First index of the pair `c1 c2` in `b[from..end]`.
fn find2(b: &[u8], from: usize, end: usize, c1: u8, c2: u8) -> Option<usize> {
    (from..end.saturating_sub(1)).find(|&k| b[k] == c1 && b[k + 1] == c2)
}

/// ATX heading depth (1–6) if `line` is a heading: 1–6 leading `#` followed by
/// a space or end-of-line. `None` otherwise.
pub(crate) fn heading_level(line: &str) -> Option<u8> {
    let b = line.as_bytes();
    let mut n = 0;
    while n < b.len() && b[n] == b'#' {
        n += 1;
    }
    ((1..=6).contains(&n) && (n == b.len() || b[n] == b' ')).then_some(n as u8)
}

/// If `line` is a standalone image — `![alt](src)`, optionally followed by a
/// `{width=N}` / `{width=Npx}` attribute, with only whitespace around it —
/// return `(src, explicit_width)`. The editor renders such a line as the image
/// (W4) when the caret is elsewhere; a line with any other trailing text stays
/// plain source.
pub(crate) fn image_line(line: &str) -> Option<(&str, Option<f32>)> {
    let rest = line.trim().strip_prefix("![")?;
    let close_alt = rest.find("](")?;
    let after_alt = &rest[close_alt + 2..];
    let close_src = after_alt.find(')')?;
    let src = after_alt[..close_src].trim();
    let tail = after_alt[close_src + 1..].trim();
    let width = if tail.is_empty() {
        None
    } else {
        let w = tail.strip_prefix("{width=")?.strip_suffix('}')?;
        Some(
            w.strip_suffix("px")
                .unwrap_or(w)
                .trim()
                .parse::<f32>()
                .ok()?,
        )
    };
    (!src.is_empty()).then_some((src, width))
}

/// Font-size multiplier for a line — larger for headings (matching the reading
/// view's scale), 1.0 for body text. Drives the editor's variable line heights.
pub(crate) fn line_scale(line: &str) -> f32 {
    match heading_level(line) {
        Some(1) => 1.8,
        Some(2) => 1.5,
        Some(3) => 1.3,
        Some(4) => 1.15,
        Some(5) => 1.05,
        _ => 1.0,
    }
}

/// Fenced code-block regions (W4b/W6): each ` ``` ` line toggles a block;
/// returns the line-index range `start..end` covering both fences (and the body
/// between). An unclosed fence runs to the last line.
pub(crate) fn code_regions(content: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut open: Option<usize> = None;
    let mut last = 0;
    for (i, line) in content.split('\n').enumerate() {
        last = i;
        if line.trim_start().starts_with("```") {
            match open {
                None => open = Some(i),
                Some(s) => {
                    out.push(s..i + 1);
                    open = None;
                }
            }
        }
    }
    if let Some(s) = open {
        out.push(s..last + 1);
    }
    out
}

/// Per-column text alignment of a GFM table.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Align {
    Left,
    Center,
    Right,
}

/// A detected GFM table region: the half-open range of logical line indices it
/// spans (header, separator, then body rows) and its per-column alignment.
pub(crate) struct TableRegion {
    pub lines: Range<usize>,
    pub aligns: Vec<Align>,
}

/// Detect GFM table regions in `content` (W4c). A region is a row line (trimmed
/// text starts with `|`) immediately followed by a separator row
/// (`| --- | :--: |`), then any further row lines. Returns regions in order.
pub(crate) fn table_regions(content: &str) -> Vec<TableRegion> {
    let lines: Vec<&str> = content.split('\n').collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < lines.len() {
        if is_table_row(lines[i])
            && let Some(aligns) = separator_aligns(lines[i + 1])
        {
            let start = i;
            let mut end = i + 2; // header + separator
            while end < lines.len() && is_table_row(lines[end]) {
                end += 1;
            }
            out.push(TableRegion {
                lines: start..end,
                aligns,
            });
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// A table row is a line whose trimmed text starts with `|`.
pub(crate) fn is_table_row(line: &str) -> bool {
    line.trim_start().starts_with('|')
}

/// Split a `| a | b |` row into trimmed cell strings (the bounding pipes drop the
/// empty leading/trailing cells they'd otherwise create).
pub(crate) fn table_cells(line: &str) -> Vec<&str> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').map(str::trim).collect()
}

/// If `line` is a table separator row (every cell is dashes with optional
/// alignment colons), return its per-column alignment, else `None`.
fn separator_aligns(line: &str) -> Option<Vec<Align>> {
    if !is_table_row(line) {
        return None;
    }
    let cells = table_cells(line);
    if cells.is_empty() {
        return None;
    }
    cells
        .iter()
        .map(|c| {
            let left = c.starts_with(':');
            let right = c.ends_with(':');
            let dashes = c.trim_matches(':');
            (!dashes.is_empty() && dashes.bytes().all(|b| b == b'-')).then_some(
                match (left, right) {
                    (true, true) => Align::Center,
                    (false, true) => Align::Right,
                    _ => Align::Left,
                },
            )
        })
        .collect()
}

fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn is_tag(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_a_gfm_table() {
        let md = "intro\n\n| A | B |\n| --- | :-: |\n| 1 | 2 |\n| 3 | 4 |\n\nafter";
        let regions = table_regions(md);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].lines, 2..6); // header, separator, 2 body rows
        assert_eq!(regions[0].aligns, vec![Align::Left, Align::Center]);
    }

    #[test]
    fn cells_split_and_trim() {
        assert_eq!(table_cells("| a | b |"), vec!["a", "b"]);
        assert_eq!(table_cells("|  |  |"), vec!["", ""]);
    }

    #[test]
    fn separator_required_and_alignment() {
        // Pipes in prose without a separator row are not a table.
        assert!(table_regions("a | b\nc | d").is_empty());
        assert!(table_regions("| not a table\njust text").is_empty());
        // Right alignment from `---:`.
        assert_eq!(
            table_regions("| h |\n| ---: |\n| x |")[0].aligns,
            vec![Align::Right]
        );
    }

    #[test]
    fn heading_and_image_line() {
        assert_eq!(heading_level("## Hi"), Some(2));
        assert_eq!(heading_level("#notaheading"), None);
        assert_eq!(image_line("![a](b.png)"), Some(("b.png", None)));
        assert_eq!(
            image_line("![a](b.png){width=320}"),
            Some(("b.png", Some(320.0)))
        );
        assert_eq!(image_line("text ![a](b.png)"), None);
    }

    fn test_style() -> SyntaxStyle {
        let c = hsla(0., 0., 0.5, 1.);
        SyntaxStyle {
            marker: c,
            code: c,
            code_bg: c,
            link: c,
            tag: c,
            mono: gpui::font("monospace"),
        }
    }

    #[test]
    fn hidden_runs_removes_markers_and_maps_back() {
        let font = gpui::font("Helvetica");
        let c = hsla(0., 0., 0., 1.);
        let st = test_style();

        // "**bold**" → "bold"; display 0 maps to source 2, end (4) to 8.
        let (disp, _, map) = hidden_runs("**bold**", &font, c, &[], None, &st);
        assert_eq!(disp, "bold");
        assert_eq!(map.len(), disp.len() + 1);
        assert_eq!(map[0], 2);
        assert_eq!(map[4], 8);

        // "## Hi" → "Hi"; the `## ` prefix is gone, display 0 maps to source 3.
        let (disp, _, map) = hidden_runs("## Hi", &font, c, &[], None, &st);
        assert_eq!(disp, "Hi");
        assert_eq!(map[0], 3);
        assert_eq!(map[2], 5);

        // No markers → unchanged, identity map.
        let (disp, _, map) = hidden_runs("plain text", &font, c, &[], None, &st);
        assert_eq!(disp, "plain text");
        assert_eq!(map, (0..=10).collect::<Vec<_>>());
    }

    #[test]
    fn hidden_runs_squiggle_diagnostics() {
        let font = gpui::font("Helvetica");
        let c = hsla(0., 0., 0., 1.);
        let st = test_style();

        // A diagnostic on the bold body ("bold" at source 2..6) underlines the
        // whole display string — squiggles survive marker hiding (W6).
        let (disp, runs, _) = hidden_runs(
            "**bold**",
            &font,
            c,
            &[Diagnostic { range: 2..6 }],
            None,
            &st,
        );
        assert_eq!(disp, "bold");
        assert_eq!(runs.iter().map(|r| r.len).sum::<usize>(), disp.len());
        assert!(runs.iter().all(|r| r.underline.is_some()));

        // A partial diagnostic splits the segment: only "text" (6..10) squiggles.
        let (disp, runs, _) = hidden_runs(
            "plain text",
            &font,
            c,
            &[Diagnostic { range: 6..10 }],
            None,
            &st,
        );
        assert_eq!(disp, "plain text");
        let underlined: usize = runs
            .iter()
            .filter(|r| r.underline.is_some())
            .map(|r| r.len)
            .sum();
        assert_eq!(underlined, 4); // "text"
    }

    #[test]
    fn hidden_runs_reveals_only_caret_construct() {
        let font = gpui::font("Helvetica");
        let c = hsla(0., 0., 0., 1.);
        let st = test_style();

        let line = "**bold** *it*";
        // Caret in "bold" reveals the bold markers; the italic stays hidden.
        let (disp, _, _) = hidden_runs(line, &font, c, &[], Some(4), &st);
        assert_eq!(disp, "**bold** it");
        // Caret in "it" reveals the italic; the bold hides.
        let (disp, _, _) = hidden_runs(line, &font, c, &[], Some(11), &st);
        assert_eq!(disp, "bold *it*");
        // Caret in plain text reveals nothing.
        let (disp, _, _) = hidden_runs("a **b**", &font, c, &[], Some(0), &st);
        assert_eq!(disp, "a b");
        // No caret on the line → fully hidden (W6).
        let (disp, _, _) = hidden_runs(line, &font, c, &[], None, &st);
        assert_eq!(disp, "bold it");
    }
}
