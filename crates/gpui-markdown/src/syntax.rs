//! Shared markdown-construct **recognition** — the definitions both of
//! Zorite's engines consume so they can never drift apart (links navigated in
//! the reader for months while WYSIWYG ignored clicks; alerts were once
//! recognized in three separate places). The reader (this crate's view),
//! the WYSIWYG editor (`gpui-editor`), and any other consumer (PDF export)
//! share *what counts as a construct and what's its payload*; each keeps its
//! own rendering. Everything here is engine-neutral and gpui-free.

/// The five GitHub alert kinds (`> [!NOTE]` …).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AlertKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

/// `(kind, marker text)` for each alert, in matching order.
pub const ALERT_MARKERS: [(AlertKind, &str); 5] = [
    (AlertKind::Note, "[!NOTE]"),
    (AlertKind::Tip, "[!TIP]"),
    (AlertKind::Important, "[!IMPORTANT]"),
    (AlertKind::Warning, "[!WARNING]"),
    (AlertKind::Caution, "[!CAUTION]"),
];

impl AlertKind {
    /// The title rendered in place of the marker ("Note", "Tip", …).
    pub fn label(self) -> &'static str {
        match self {
            Self::Note => "Note",
            Self::Tip => "Tip",
            Self::Important => "Important",
            Self::Warning => "Warning",
            Self::Caution => "Caution",
        }
    }
}

/// Match an alert marker at the start of a blockquote's text content: the
/// marker must be uppercase and either alone on its first line (GitHub's
/// form) or followed by a space and the body (`[!NOTE] like so` — the way
/// people naturally type it). Returns the kind and how many bytes to strip
/// (the marker plus its newline/space separator).
pub fn alert_marker(value: &str) -> Option<(AlertKind, usize)> {
    for (kind, m) in ALERT_MARKERS {
        if let Some(rest) = value.strip_prefix(m) {
            if rest.is_empty() {
                return Some((kind, m.len()));
            }
            if rest.starts_with('\n') || rest.starts_with(' ') {
                return Some((kind, m.len() + 1));
            }
        }
    }
    None
}

/// [`alert_marker`] for a single line's body (text after a blockquote's `>`
/// prefix): tolerates leading spaces and returns the kind plus the byte
/// length consumed within `body` (spaces, marker, one separator space) — what
/// a line-oriented editor hides before painting the label.
pub fn alert_prefix(body: &str) -> Option<(AlertKind, usize)> {
    let trimmed = body.trim_start();
    let ws = body.len() - trimmed.len();
    for (kind, m) in ALERT_MARKERS {
        if let Some(rest) = trimmed.strip_prefix(m) {
            if rest.is_empty() {
                return Some((kind, ws + m.len()));
            }
            if rest.starts_with(' ') {
                return Some((kind, ws + m.len() + 1));
            }
        }
    }
    None
}

/// Visual style of a GFM table, chosen per-table via a `<!-- table:STYLE -->`
/// marker comment on the line directly above it. The renderers honor it;
/// standard Markdown viewers ignore the comment and show a plain table.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TableStyle {
    /// Full outer box + all row/column gridlines.
    #[default]
    Grid,
    /// Alternate body rows shaded; no gridlines; a rule under the header.
    Striped,
    /// Only the header row shaded; no gridlines.
    Header,
    /// No box or gridlines — just a rule under the header.
    Minimal,
}

impl TableStyle {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "grid" => Some(Self::Grid),
            "striped" => Some(Self::Striped),
            "header" => Some(Self::Header),
            "minimal" => Some(Self::Minimal),
            _ => None,
        }
    }
}

/// Parse a `<!-- table:STYLE -->` marker (a whole line or an HTML comment's
/// value) into its [`TableStyle`]. `None` for anything unrecognized, so an
/// unknown marker stays a plain HTML comment.
pub fn table_style_marker(text: &str) -> Option<TableStyle> {
    let inner = text
        .trim()
        .strip_prefix("<!--")?
        .strip_suffix("-->")?
        .trim();
    TableStyle::from_name(inner.strip_prefix("table:")?.trim())
}

/// Font-size multiplier for a heading of the given depth (h1 largest, h6 =
/// body) — one scale for reading, editing, and export.
pub fn heading_scale(depth: u8) -> f32 {
    match depth {
        1 => 1.8,
        2 => 1.5,
        3 => 1.3,
        4 => 1.15,
        5 => 1.05,
        _ => 1.0,
    }
}

// --- Linkables ---

/// What a click on a link-like construct targets. `Page` opens a page by
/// title (a `[[wiki-link]]` or a `#tag` — Logseq semantics); `Url` is an
/// inline or bare URL (hosts open http(s) externally, resolve files
/// themselves).
#[derive(Debug, PartialEq)]
pub enum LinkHit {
    Page(String),
    Url(String),
}

/// Split a wiki-link's inner text into `(target, display)`:
/// `target|label` shows `label` (falling back to the target when the label is
/// empty); `name` shows itself. Both sides trimmed.
pub fn wiki_target_display(inner: &str) -> (&str, &str) {
    match inner.split_once('|') {
        Some((t, l)) if !l.trim().is_empty() => (t.trim(), l.trim()),
        Some((t, _)) => (t.trim(), t.trim()),
        None => (inner.trim(), inner.trim()),
    }
}

/// Whether `c` can appear inside a `#tag` name (after the `#`). `/` is
/// included — Logseq-style namespaced tags (`#area/sub`) are one tag.
pub fn is_tag_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'/')
}

/// A word character for boundary checks (a `#` glued to a word isn't a tag;
/// a URL glued to a word isn't a link).
pub fn is_word_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Where a bare URL starting at `start` ends: consumes to whitespace or a
/// wrapping delimiter, then backs off trailing punctuation (GFM-ish).
pub fn url_end(line: &str, start: usize) -> usize {
    let b = line.as_bytes();
    let mut j = start;
    while j < line.len()
        && !b[j].is_ascii_whitespace()
        && !matches!(b[j], b'<' | b'>' | b'"' | b'`')
    {
        j += 1;
    }
    while j > start
        && matches!(
            b[j - 1],
            b'.' | b',' | b';' | b':' | b'!' | b'?' | b')' | b']'
        )
    {
        j -= 1;
    }
    j
}

/// Every clickable link in `line`, as `(source byte range, target)`.
/// Wiki-links (anywhere on one opens its target; the alias is display-only),
/// inline `[text](url)` links, `#tags`, and bare `http(s)://` URLs. Images
/// (`![](src)`), footnote refs, and anything inside inline code are opaque —
/// not links. One grammar for every renderer's click hit-tests, hover
/// cursors, and styling.
pub fn links(line: &str) -> Vec<(std::ops::Range<usize>, LinkHit)> {
    let b = line.as_bytes();
    let end = line.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < end {
        let c = b[i];
        // Inline code: the span is opaque (a URL inside backticks is verbatim).
        if c == b'`'
            && let Some(close) = find1(b, i + 1, end, b'`')
        {
            i = close + 1;
            continue;
        }
        // Wiki-link: [[target]] / [[target|alias]].
        if c == b'['
            && i + 1 < end
            && b[i + 1] == b'['
            && let Some(close) = find2(b, i + 2, end, b']', b']')
        {
            let (target, _) = wiki_target_display(&line[i + 2..close]);
            if !target.is_empty() {
                out.push((i..close + 2, LinkHit::Page(target.to_string())));
            }
            i = close + 2;
            continue;
        }
        // Footnote reference [^label]: styled like a link but not one.
        if c == b'['
            && i + 1 < end
            && b[i + 1] == b'^'
            && let Some(rb) = find1(b, i + 2, end, b']')
            && rb > i + 2
        {
            i = rb + 1;
            continue;
        }
        // Inline link [text](url) — or an image ![alt](src), which is NOT a
        // link click (images render as widgets / have their own machinery).
        if c == b'['
            && let Some(rb) = find1(b, i + 1, end, b']')
            && rb + 1 < end
            && b[rb + 1] == b'('
            && let Some(rp) = find1(b, rb + 2, end, b')')
        {
            let is_image = i > 0 && b[i - 1] == b'!';
            let url = line[rb + 2..rp].trim();
            if !is_image && !url.is_empty() {
                out.push((i..rp + 1, LinkHit::Url(url.to_string())));
            }
            i = rp + 1;
            continue;
        }
        // Tag: #tag → the page of that name (Logseq semantics).
        if c == b'#' && (i == 0 || !is_word_char(b[i - 1])) {
            let mut j = i + 1;
            while j < end && is_tag_char(b[j]) {
                j += 1;
            }
            if j > i + 1 {
                out.push((i..j, LinkHit::Page(line[i + 1..j].to_string())));
                i = j;
                continue;
            }
        }
        // Bare URL: http(s)://… at a word boundary (GFM autolink literal).
        // Compare BYTES: `i` walks bytes, so a str slice here would panic
        // mid-char on any non-ASCII text.
        if (b[i..].starts_with(b"http://") || b[i..].starts_with(b"https://"))
            && (i == 0 || !is_word_char(b[i - 1]))
        {
            let j = url_end(line, i);
            if j > i + 8 {
                out.push((i..j, LinkHit::Url(line[i..j].to_string())));
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// The link under byte `col` of `line`, if any (see [`links`]).
pub fn link_at(line: &str, col: usize) -> Option<LinkHit> {
    links(line)
        .into_iter()
        .find(|(r, _)| r.contains(&col))
        .map(|(_, hit)| hit)
}

fn find1(b: &[u8], from: usize, end: usize, c: u8) -> Option<usize> {
    (from..end).find(|&i| b[i] == c)
}

fn find2(b: &[u8], from: usize, end: usize, c1: u8, c2: u8) -> Option<usize> {
    (from..end.saturating_sub(1)).find(|&i| b[i] == c1 && b[i + 1] == c2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alert_recognition_both_forms() {
        assert!(matches!(
            alert_marker("[!NOTE]\nbody"),
            Some((AlertKind::Note, 8))
        ));
        assert!(matches!(
            alert_marker("[!NOTE] inline"),
            Some((AlertKind::Note, 8))
        ));
        assert!(alert_marker("[!note] no").is_none());
        assert!(alert_marker("[!NOTEXT]").is_none());

        assert!(matches!(
            alert_prefix("  [!TIP] x"),
            Some((AlertKind::Tip, 9))
        ));
        assert_eq!(AlertKind::Caution.label(), "Caution");
    }

    #[test]
    fn links_cover_every_kind() {
        let hits = links("see [[Page|alias]] and [x](https://a.io) #tag/sub https://b.io/p, done");
        assert_eq!(
            hits.iter().map(|(_, h)| h).collect::<Vec<_>>(),
            vec![
                &LinkHit::Page("Page".into()),
                &LinkHit::Url("https://a.io".into()),
                &LinkHit::Page("tag/sub".into()),
                &LinkHit::Url("https://b.io/p".into()), // trailing comma trimmed
            ]
        );
        // Opaque: code spans, images, footnotes, glued #.
        assert!(links("`https://x.io` ![a](i.png) [^1] word#no").is_empty());
        // Regression: multi-byte text before a URL must not panic the
        // byte-wise walk (it once str-sliced at a continuation byte).
        let hits = links("shrug ¯\\_(ツ)_/¯ then https://a.io done");
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn table_style_markers_parse() {
        assert_eq!(
            table_style_marker("<!-- table:striped -->"),
            Some(TableStyle::Striped)
        );
        assert_eq!(table_style_marker("<!-- math:left -->"), None);
        assert_eq!(table_style_marker("plain text"), None);
    }
}
