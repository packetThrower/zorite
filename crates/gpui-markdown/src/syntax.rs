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
    fn table_style_markers_parse() {
        assert_eq!(
            table_style_marker("<!-- table:striped -->"),
            Some(TableStyle::Striped)
        );
        assert_eq!(table_style_marker("<!-- math:left -->"), None);
        assert_eq!(table_style_marker("plain text"), None);
    }
}
