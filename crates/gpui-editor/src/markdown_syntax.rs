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
            ..Default::default()
        },
    });
}

/// Scan one line `text[start..end]`. Markers are ASCII, so byte scanning is
/// UTF-8-safe (an ASCII byte never appears inside a multi-byte char).
fn scan_line(text: &str, start: usize, end: usize, st: &SyntaxStyle, out: &mut Vec<Span>) {
    let b = text.as_bytes();
    let mut i = start;
    while i < end {
        let c = b[i];
        // Inline code: `code` (whole span styled as a mono chip).
        if c == b'`'
            && let Some(close) = find1(b, i + 1, end, b'`')
        {
            out.push(Span {
                range: i..close + 1,
                style: Style {
                    mono: true,
                    color: Some(st.code),
                    bg: Some(st.code_bg),
                    ..Default::default()
                },
            });
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

fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn is_tag(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'/')
}
