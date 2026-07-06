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
/// people naturally type it). An Obsidian-style fold char directly after the
/// `]` makes the callout foldable: `[!NOTE]-` = folded by default, `[!NOTE]+`
/// = open (`None` = not foldable). Returns the kind, how many bytes to strip
/// (marker, fold char, and the newline/space separator), and the fold state.
pub fn alert_marker(value: &str) -> Option<(AlertKind, usize, Option<bool>)> {
    for (kind, m) in ALERT_MARKERS {
        if let Some(rest) = value.strip_prefix(m) {
            let (fold, flen) = match rest.as_bytes().first() {
                Some(b'-') => (Some(true), 1),
                Some(b'+') => (Some(false), 1),
                _ => (None, 0),
            };
            let rest = &rest[flen..];
            if rest.is_empty() {
                return Some((kind, m.len() + flen, fold));
            }
            if rest.starts_with('\n') || rest.starts_with(' ') {
                return Some((kind, m.len() + flen + 1, fold));
            }
        }
    }
    None
}

/// [`alert_marker`] for a single line's body (text after a blockquote's `>`
/// prefix): tolerates leading spaces and returns the kind, the byte length
/// consumed within `body` (spaces, marker, fold char, one separator space) —
/// what a line-oriented editor hides before painting the label — and the fold
/// state (`Some(true)` = folded).
pub fn alert_prefix(body: &str) -> Option<(AlertKind, usize, Option<bool>)> {
    let trimmed = body.trim_start();
    let ws = body.len() - trimmed.len();
    for (kind, m) in ALERT_MARKERS {
        if let Some(rest) = trimmed.strip_prefix(m) {
            let (fold, flen) = match rest.as_bytes().first() {
                Some(b'-') => (Some(true), 1),
                Some(b'+') => (Some(false), 1),
                _ => (None, 0),
            };
            let rest = &rest[flen..];
            if rest.is_empty() {
                return Some((kind, ws + m.len() + flen, fold));
            }
            if rest.starts_with(' ') {
                return Some((kind, ws + m.len() + flen + 1, fold));
            }
        }
    }
    None
}

/// The fold char of the alert marker on `line` (a full source line, `>` prefix
/// included): its byte offset within the line and the current state
/// (`true` = `-`/folded). `None` when the line isn't a foldable alert marker.
pub fn alert_fold_char(line: &str) -> Option<(usize, bool)> {
    let b = line.as_bytes();
    let mut p = 0;
    while p < b.len() && (b[p] == b'>' || b[p] == b' ') {
        p += 1;
    }
    let (_, _, fold) = alert_prefix(&line[p..])?;
    let folded = fold?;
    // The fold char sits right after the marker's closing `]`.
    let close = line[p..].find(']')? + p;
    Some((close + 1, folded))
}

/// Flip the fold state (`-` ↔ `+`) of the foldable alert marker on the line
/// containing byte `offset`, returning the new content — what a click on a
/// callout's chevron persists (the checkbox-toggle pattern).
pub fn toggle_alert_fold_at(content: &str, offset: usize) -> Option<String> {
    if offset > content.len() {
        return None;
    }
    let line_start = content[..offset].rfind('\n').map_or(0, |p| p + 1);
    let line_end = content[offset..]
        .find('\n')
        .map_or(content.len(), |p| offset + p);
    let (at, folded) = alert_fold_char(&content[line_start..line_end])?;
    let mut out = content.to_string();
    out.replace_range(
        line_start + at..line_start + at + 1,
        if folded { "+" } else { "-" },
    );
    Some(out)
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

/// The marker for ordered item `n` (1-based) at nesting `depth`, Word-style:
/// `1.` -> `a.` -> `i.`, cycling for deeper levels. Both views paint ordered
/// lists with this scheme (a deliberate divergence from CommonMark's
/// digits-everywhere), so nesting is readable at a glance.
pub fn ordered_marker(depth: usize, n: u32) -> String {
    match depth % 3 {
        0 => format!("{n}."),
        1 => format!("{}.", letters(n)),
        _ => format!("{}.", roman(n)),
    }
}

/// 1 → `a`, 26 → `z`, 27 → `aa` (bijective base 26).
fn letters(mut n: u32) -> String {
    let mut s = String::new();
    while n > 0 {
        n -= 1;
        s.insert(0, (b'a' + (n % 26) as u8) as char);
        n /= 26;
    }
    s
}

/// Lowercase roman numerals (`0` has none; empty string).
fn roman(mut n: u32) -> String {
    let mut s = String::new();
    for (v, r) in [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ] {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

// --- Linkables ---

/// What a click on a link-like construct targets. `Page` opens a page by
/// title (a `[[wiki-link]]` or a `#tag` — Logseq semantics); `Url` is an
/// inline or bare URL (hosts open http(s) externally, resolve files
/// themselves).
#[derive(Debug, PartialEq, Clone)]
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

/// The Obsidian block-id anchor at the end of `line` (` ^some-id`): the byte
/// where its leading space starts (so renderers can hide the whole tail) and
/// the id itself. The id must be non-empty, made of word chars / `-`, and sit
/// at the line's end (trailing whitespace tolerated).
pub fn block_id(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_end();
    let (before, id) = trimmed.rsplit_once(" ^")?;
    if id.is_empty() || !id.bytes().all(|b| is_word_char(b) || b == b'-') {
        return None;
    }
    Some((before.len(), id))
}

/// Split a wiki-link target into `(page, block id)`: `Note#^id` links to the
/// block carrying `^id` on the page `Note`; anything else is a plain page
/// target. Only the `#^` form is an anchor — a bare `#` stays part of the
/// title (page names may contain it, and `file.pdf#p3` has its own meaning).
pub fn split_block_anchor(target: &str) -> (&str, Option<&str>) {
    match target.split_once("#^") {
        Some((page, id)) if !page.is_empty() && !id.is_empty() => (page, Some(id)),
        _ => (target, None),
    }
}

/// Split a wiki-link target into `(page, heading)`: `Note#My Heading` links to
/// the heading on the page `Note`. Splits at the first `#` when both sides are
/// non-empty and the page part isn't a PDF (`file.pdf#p3` keeps its page-jump
/// meaning). Block anchors (`#^`) are the caller's first check —
/// [`split_block_anchor`] — and a Zorite page title may itself contain `#`, so
/// navigation should prefer an existing literal-titled page before splitting.
pub fn split_heading_anchor(target: &str) -> (&str, Option<&str>) {
    match target.split_once('#') {
        Some((page, heading))
            if !page.is_empty()
                && !heading.trim().is_empty()
                && !heading.starts_with('^')
                && !page.to_ascii_lowercase().ends_with(".pdf") =>
        {
            (page, Some(heading))
        }
        _ => (target, None),
    }
}

/// The byte offset of the start of the line carrying the ATX heading whose
/// text matches `heading` (case-insensitive, trimmed; fenced code skipped),
/// searching top to bottom. Drives navigation for `[[Note#Heading]]` links.
pub fn find_heading_line(content: &str, heading: &str) -> Option<usize> {
    let want = heading.trim().to_lowercase();
    let mut start = 0;
    let mut in_fence = false;
    for line in content.split('\n') {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
        } else if !in_fence {
            let t = line.trim_start();
            let level = t.bytes().take_while(|&b| b == b'#').count();
            if (1..=6).contains(&level)
                && let Some(text) = t[level..].strip_prefix(' ')
                && text.trim().to_lowercase() == want
            {
                return Some(start);
            }
        }
        start += line.len() + 1;
    }
    None
}

/// The byte offset of the start of the line carrying the block anchor `^id`,
/// searching top to bottom. Drives navigation for `[[Note#^id]]` links.
pub fn find_block_line(content: &str, id: &str) -> Option<usize> {
    let mut start = 0;
    for line in content.split('\n') {
        if block_id(line).is_some_and(|(_, i)| i == id) {
            return Some(start);
        }
        start += line.len() + 1;
    }
    None
}

/// The embed target when `line` is a standalone transclusion — exactly
/// `![[target]]` (Obsidian's embed syntax) and nothing else on the line.
/// Mid-text embeds don't count; they render as plain links.
pub fn embed_line(line: &str) -> Option<&str> {
    let t = line.trim();
    let inner = t.strip_prefix("![[")?.strip_suffix("]]")?;
    (!inner.trim().is_empty() && !inner.contains("]]")).then(|| inner.trim())
}

/// Every standalone embed target in `content`, in order — what a host
/// pre-resolves before rendering (recursing into resolved content itself for
/// nested embeds).
pub fn embed_targets(content: &str) -> Vec<String> {
    content
        .split('\n')
        .filter_map(embed_line)
        .map(str::to_string)
        .collect()
}

/// The source range of the block carrying the anchor `^id` — its whole line —
/// for embedding (`![[Note#^id]]`).
pub fn extract_block(content: &str, id: &str) -> Option<std::ops::Range<usize>> {
    let start = find_block_line(content, id)?;
    let end = content[start..]
        .find('\n')
        .map_or(content.len(), |p| start + p);
    Some(start..end)
}

/// The source range of the section under `heading` — the heading line through
/// the line before the next heading of the same or higher level (fenced code
/// skipped) — for embedding (`![[Note#Heading]]`).
pub fn extract_section(content: &str, heading: &str) -> Option<std::ops::Range<usize>> {
    let start = find_heading_line(content, heading)?;
    let level = content[start..]
        .trim_start()
        .bytes()
        .take_while(|&b| b == b'#')
        .count();
    let mut pos = content[start..]
        .find('\n')
        .map_or(content.len(), |p| start + p + 1);
    let mut in_fence = false;
    while pos < content.len() {
        let line_end = content[pos..].find('\n').map_or(content.len(), |p| pos + p);
        let line = &content[pos..line_end];
        let t = line.trim_start();
        if t.starts_with("```") {
            in_fence = !in_fence;
        } else if !in_fence {
            let l = t.bytes().take_while(|&b| b == b'#').count();
            if (1..=level).contains(&l) && t[l..].starts_with(' ') {
                return Some(start..pos.saturating_sub(1).max(start));
            }
        }
        pos = line_end + 1;
    }
    Some(start..content.len())
}

/// Split a `key:: value` property line into `(key, value)`. The key must look
/// like an identifier (starts with a letter; letters/digits/`-_.` after) so
/// prose containing `::` — Zorite `[[wiki]]` links, `C++::method` — isn't
/// mistaken for a property. Leading indentation is ignored; the value is
/// trimmed. One grammar for the reader, the editor, and the importers.
pub fn property(line: &str) -> Option<(&str, &str)> {
    let rest = line.trim_start();
    let idx = rest.find("::")?;
    let key = &rest[..idx];
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        || !key.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
    {
        return None;
    }
    Some((key, rest[idx + 2..].trim()))
}

/// A rendered piece of a property value: literal text, or a link "pill" (a
/// wiki-link, `#tag`, or URL shown as a rounded chip). Both panels render values
/// through this so they pill-ify identically.
pub enum PropSeg {
    Text(String),
    Pill {
        /// The chip's display text: a wiki-link's label, a tag without its `#`,
        /// or a link's text.
        label: String,
        target: LinkHit,
        /// A `#tag` (vs a wiki-link / URL) — panels tint tags differently.
        is_tag: bool,
    },
}

/// Split a property value into display segments — plain runs and link pills
/// (wiki-links show their label, tags drop the `#`, `[text](url)` shows its
/// text, bare URLs show themselves). Built on [`links`], so the pill spans match
/// the reader's and editor's click hit-tests.
pub fn property_value_segments(value: &str) -> Vec<PropSeg> {
    let mut out = Vec::new();
    let mut pos = 0;
    for (range, hit) in links(value) {
        if range.start > pos {
            out.push(PropSeg::Text(value[pos..range.start].to_string()));
        }
        let raw = &value[range.clone()];
        let (label, is_tag) =
            if let Some(inner) = raw.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")) {
                (wiki_target_display(inner).1.to_string(), false)
            } else if let Some(tag) = raw.strip_prefix('#') {
                (tag.to_string(), true)
            } else if let Some(rest) = raw.strip_prefix('[') {
                // `[text](url)` — show the text.
                (
                    rest.split_once(']').map_or(raw, |(t, _)| t).to_string(),
                    false,
                )
            } else {
                (raw.to_string(), false) // bare URL
            };
        out.push(PropSeg::Pill {
            label,
            target: hit,
            is_tag,
        });
        pos = range.end;
    }
    if pos < value.len() {
        out.push(PropSeg::Text(value[pos..].to_string()));
    }
    out
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
            Some((AlertKind::Note, 8, None))
        ));
        assert!(matches!(
            alert_marker("[!NOTE] inline"),
            Some((AlertKind::Note, 8, None))
        ));
        assert!(alert_marker("[!note] no").is_none());
        assert!(alert_marker("[!NOTEXT]").is_none());

        assert!(matches!(
            alert_prefix("  [!TIP] x"),
            Some((AlertKind::Tip, 9, None))
        ));
        assert_eq!(AlertKind::Caution.label(), "Caution");
    }

    #[test]
    fn alert_fold_markers_and_toggle() {
        // `-` = folded, `+` = open; the strip consumes the fold char.
        assert!(matches!(
            alert_marker("[!NOTE]-\nbody"),
            Some((AlertKind::Note, 9, Some(true)))
        ));
        assert!(matches!(
            alert_marker("[!NOTE]+ inline"),
            Some((AlertKind::Note, 9, Some(false)))
        ));
        assert!(matches!(
            alert_prefix(" [!TIP]- x"),
            Some((AlertKind::Tip, 9, Some(true)))
        ));
        // A `-` not directly after `]` is body text, not a fold marker.
        assert!(matches!(
            alert_marker("[!NOTE] - item"),
            Some((AlertKind::Note, 8, None))
        ));

        // The fold char locates + flips within a full source line.
        assert_eq!(alert_fold_char("> [!NOTE]- body"), Some((9, true)));
        assert_eq!(alert_fold_char("> [!NOTE] body"), None);
        let src = "before\n> [!TIP]- hidden\n> more\nafter";
        let toggled = toggle_alert_fold_at(src, 10).unwrap();
        assert_eq!(toggled, "before\n> [!TIP]+ hidden\n> more\nafter");
        let back = toggle_alert_fold_at(&toggled, 10).unwrap();
        assert_eq!(back, src);
        assert!(toggle_alert_fold_at("plain text", 2).is_none());
    }

    #[test]
    fn block_ids_and_anchor_links() {
        assert_eq!(
            block_id("Decision made. ^decision1"),
            Some((14, "decision1"))
        );
        assert_eq!(block_id("trailing space ^id  "), Some((14, "id")));
        assert_eq!(block_id("no anchor"), None);
        assert_eq!(block_id("mid ^id not at end"), None);
        assert_eq!(block_id("bad chars ^a b"), None);

        assert_eq!(split_block_anchor("Note#^id"), ("Note", Some("id")));
        assert_eq!(split_block_anchor("Note"), ("Note", None));
        // A bare `#` is part of the title, not an anchor.
        assert_eq!(split_block_anchor("C# Notes"), ("C# Notes", None));
        assert_eq!(split_block_anchor("file.pdf#p3"), ("file.pdf#p3", None));

        let src = "intro\nthe fact ^fact-1\nmore";
        assert_eq!(find_block_line(src, "fact-1"), Some(6));
        assert_eq!(find_block_line(src, "nope"), None);
    }

    #[test]
    fn embeds_and_extraction() {
        assert_eq!(embed_line("![[Note]]"), Some("Note"));
        assert_eq!(embed_line("  ![[Note#^id]]  "), Some("Note#^id"));
        assert_eq!(embed_line("text ![[Note]]"), None); // not standalone
        assert_eq!(embed_line("![[]]"), None);
        assert_eq!(embed_line("[[Note]]"), None);

        let src = "pre\nthe block ^b1\n## Sec\nbody\nmore\n### Sub\ndeep\n## Next\nafter";
        assert_eq!(&src[extract_block(src, "b1").unwrap()], "the block ^b1");
        // A section runs through its subsections, stopping at the next
        // same-or-higher heading.
        assert_eq!(
            &src[extract_section(src, "Sec").unwrap()],
            "## Sec\nbody\nmore\n### Sub\ndeep"
        );
        assert_eq!(
            &src[extract_section(src, "Next").unwrap()],
            "## Next\nafter"
        );
        assert!(extract_section(src, "missing").is_none());
    }

    #[test]
    fn heading_anchors() {
        assert_eq!(
            split_heading_anchor("Note#My Heading"),
            ("Note", Some("My Heading"))
        );
        assert_eq!(split_heading_anchor("Note"), ("Note", None));
        // Block anchors, PDFs, and empty sides don't split as headings.
        assert_eq!(split_heading_anchor("Note#^id"), ("Note#^id", None));
        assert_eq!(split_heading_anchor("file.pdf#p3"), ("file.pdf#p3", None));
        assert_eq!(split_heading_anchor("#Heading"), ("#Heading", None));
        assert_eq!(split_heading_anchor("Note#"), ("Note#", None));

        let src = "intro\n## My Heading\nbody\n```\n# not a heading\n```\n### Deep One";
        // Case-insensitive, trimmed; fences skipped.
        assert_eq!(find_heading_line(src, "my heading"), Some(6));
        assert_eq!(find_heading_line(src, " Deep One "), Some(49));
        assert_eq!(find_heading_line(src, "not a heading"), None);
        assert_eq!(find_heading_line(src, "missing"), None);
    }

    #[test]
    fn property_recognition() {
        assert_eq!(
            property("attendees:: Bob, Sue"),
            Some(("attendees", "Bob, Sue"))
        );
        assert_eq!(property("  time::3:00pm"), Some(("time", "3:00pm")));
        assert_eq!(property("owner:: [[Sue]]"), Some(("owner", "[[Sue]]")));
        // Not properties: prose with `::`, wiki links, empty/bad keys.
        assert_eq!(property("See [[Page::sub]] here"), None);
        assert_eq!(property("just prose"), None);
        assert_eq!(property(":: value"), None);
        assert_eq!(property("1key:: v"), None);
    }

    #[test]
    fn property_value_segments_pill_and_plain() {
        let segs = property_value_segments("[[Bob]], [[Sue|Susan]] and #work done");
        // Bob pill, ", " text, Susan pill, " and " text, work tag, " done" text.
        assert!(matches!(&segs[0], PropSeg::Pill { label, is_tag: false, .. } if label == "Bob"));
        assert!(matches!(&segs[1], PropSeg::Text(t) if t == ", "));
        assert!(matches!(&segs[2], PropSeg::Pill { label, .. } if label == "Susan"));
        assert!(matches!(&segs[4], PropSeg::Pill { label, is_tag: true, .. } if label == "work"));
        // A plain value is a single text segment.
        assert!(
            matches!(property_value_segments("active").as_slice(), [PropSeg::Text(t)] if t == "active")
        );
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
    fn ordered_markers_cycle_word_style() {
        assert_eq!(ordered_marker(0, 2), "2.");
        assert_eq!(ordered_marker(1, 1), "a.");
        assert_eq!(ordered_marker(1, 27), "aa.");
        assert_eq!(ordered_marker(2, 4), "iv.");
        assert_eq!(ordered_marker(2, 9), "ix.");
        assert_eq!(ordered_marker(3, 2), "2."); // cycle restarts
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
