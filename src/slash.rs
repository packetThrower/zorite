//! Slash-command menu: the command table and the logic for detecting a
//! `/query` at the caret. The popup state lives here; `AppView` owns the
//! open `Slash`, wires keyboard handling, and performs insertion.

use gpui::{Bounds, Pixels};

/// One slash command: a label and the markdown it inserts. `caret` is the
/// byte offset within `snippet` where the cursor lands after insertion.
pub struct Command {
    pub label: &'static str,
    pub snippet: &'static str,
    pub caret: usize,
}

/// The slash command table. `caret` is the byte offset within `snippet`
/// where the cursor lands (e.g. inside a wrap, or the first table cell).
pub const COMMANDS: &[Command] = &[
    // Blocks
    Command { label: "Heading 1", snippet: "# ", caret: 2 },
    Command { label: "Heading 2", snippet: "## ", caret: 3 },
    Command { label: "Heading 3", snippet: "### ", caret: 4 },
    Command { label: "Bullet list", snippet: "- ", caret: 2 },
    Command { label: "Numbered list", snippet: "1. ", caret: 3 },
    Command { label: "To-do", snippet: "- [ ] ", caret: 6 },
    Command { label: "Quote", snippet: "> ", caret: 2 },
    Command { label: "Code block", snippet: "```\n\n```", caret: 4 },
    Command { label: "Table", snippet: "|  |  |\n| --- | --- |\n|  |  |\n", caret: 2 },
    Command { label: "Divider", snippet: "---\n", caret: 4 },
    // Inline (caret lands between the markers)
    Command { label: "Bold", snippet: "****", caret: 2 },
    Command { label: "Italic", snippet: "**", caret: 1 },
    Command { label: "Strikethrough", snippet: "~~~~", caret: 2 },
    Command { label: "Inline code", snippet: "``", caret: 1 },
    Command { label: "Link", snippet: "[]()", caret: 1 },
    Command { label: "Image", snippet: "![]()", caret: 4 },
];

/// Which editor the open menu targets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlashTarget {
    Day(String),
    Page(i64),
}

/// Open slash-menu state.
pub struct Slash {
    pub target: SlashTarget,
    pub query: String,
    /// Byte offset of the `/` in the editor text.
    pub start: usize,
    /// Caret bounds (window space) used to anchor the popup.
    pub caret: Bounds<Pixels>,
    pub selected: usize,
}

impl Slash {
    /// Commands whose label matches the query (case-insensitive substring).
    pub fn matches(&self) -> Vec<&'static Command> {
        let q = self.query.to_lowercase();
        COMMANDS
            .iter()
            .filter(|c| q.is_empty() || c.label.to_lowercase().contains(&q))
            .collect()
    }
}

/// If the caret sits just after a `/token` — `token` is `[A-Za-z0-9-]*`
/// and the `/` is at start-of-text or right after whitespace — return the
/// byte offset of the `/` and the token text. Returns `None` otherwise
/// (e.g. `and/or`, or a space after the token).
pub fn detect(value: &str, cursor: usize) -> Option<(usize, String)> {
    let cursor = cursor.min(value.len());
    let bytes = value.as_bytes();
    let mut i = cursor;
    while i > 0 && is_token_char(bytes[i - 1]) {
        i -= 1;
    }
    if i == 0 || bytes[i - 1] != b'/' {
        return None;
    }
    let slash = i - 1;
    if slash > 0 && !is_boundary(bytes[slash - 1]) {
        return None;
    }
    Some((slash, value[i..cursor].to_string()))
}

fn is_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-'
}

fn is_boundary(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n')
}

#[cfg(test)]
mod tests {
    use super::detect;

    #[test]
    fn slash_alone_triggers_empty_query() {
        assert_eq!(detect("/", 1), Some((0, String::new())));
    }

    #[test]
    fn slash_query_at_start() {
        assert_eq!(detect("/todo", 5), Some((0, "todo".to_string())));
    }

    #[test]
    fn slash_after_whitespace() {
        assert_eq!(detect("hi /h", 5), Some((3, "h".to_string())));
    }

    #[test]
    fn midword_slash_is_ignored() {
        assert_eq!(detect("and/or", 6), None);
    }

    #[test]
    fn space_after_token_closes() {
        assert_eq!(detect("/h ", 3), None);
    }
}
