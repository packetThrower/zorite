//! The `/` command palette: detecting a `/query` at the caret, and the
//! set of things it can insert — built-in markdown snippets (from
//! `gpui-markdown`) plus user **templates** parsed from a reserved
//! `Templates` page. `AppView` owns the open `Slash`, keyboard handling,
//! and insertion.

use gpui::{Bounds, Pixels};
use gpui_markdown::SNIPPETS;

/// The reserved page whose content defines templates. Each template is a
/// line `!name` followed by its body (until the next `!name` or EOF).
pub const TEMPLATES_PAGE: &str = "Templates";

/// Menu level: the root (two categories) or a submenu.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SlashLevel {
    Root,
    Markdown,
    Templates,
}

/// What a palette entry does when chosen.
pub enum ItemKind {
    /// Open a submenu (rendered with a `›`).
    Category(SlashLevel),
    /// Insert `snippet`, caret at byte offset `caret` within it.
    Insert { snippet: String, caret: usize },
}

/// One entry in the open palette.
pub struct PaletteItem {
    pub label: String,
    pub kind: ItemKind,
}

/// A user template parsed from the `Templates` page.
pub struct Template {
    pub name: String,
    pub body: String,
}

/// Which editor the open menu targets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlashTarget {
    Day(String),
    Page(i64),
}

/// Open palette state.
pub struct Slash {
    pub target: SlashTarget,
    pub query: String,
    /// Byte offset of the `/` in the editor text.
    pub start: usize,
    /// Caret bounds (window space) used to anchor the popup.
    pub caret: Bounds<Pixels>,
    pub selected: usize,
    /// Current level (root categories vs a submenu).
    pub level: SlashLevel,
    /// Filtered entries for the current level + query.
    pub items: Vec<PaletteItem>,
}

/// If the caret sits just after a `/token` (`token` = `[A-Za-z0-9-]*`, the
/// `/` at start-of-text or after whitespace), return the `/` byte offset
/// and the token. `None` otherwise (e.g. `and/or`).
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

/// Build the palette for the current level + query:
/// - Root + empty query → the category rows (`Markdown ›`, `Templates ›`).
/// - Root + a query → a flattened search over everything (so `/table` works).
/// - a submenu → that category's items, filtered by the query.
pub fn build_items(
    level: SlashLevel,
    query: &str,
    templates: &[Template],
    title: &str,
) -> Vec<PaletteItem> {
    let q = query.to_lowercase();
    let mut out = Vec::new();
    match level {
        SlashLevel::Root if q.is_empty() => {
            out.push(PaletteItem {
                label: "Markdown".to_string(),
                kind: ItemKind::Category(SlashLevel::Markdown),
            });
            if !templates.is_empty() {
                out.push(PaletteItem {
                    label: "Templates".to_string(),
                    kind: ItemKind::Category(SlashLevel::Templates),
                });
            }
        }
        SlashLevel::Root => {
            markdown_items(&q, &mut out);
            template_items(&q, templates, title, &mut out);
        }
        SlashLevel::Markdown => markdown_items(&q, &mut out),
        SlashLevel::Templates => template_items(&q, templates, title, &mut out),
    }
    out
}

fn markdown_items(q: &str, out: &mut Vec<PaletteItem>) {
    for s in SNIPPETS {
        if q.is_empty() || s.label.to_lowercase().contains(q) {
            out.push(PaletteItem {
                label: s.label.to_string(),
                kind: ItemKind::Insert {
                    snippet: s.snippet.to_string(),
                    caret: s.caret,
                },
            });
        }
    }
}

fn template_items(q: &str, templates: &[Template], title: &str, out: &mut Vec<PaletteItem>) {
    for t in templates {
        if q.is_empty() || t.name.to_lowercase().contains(q) {
            let (snippet, caret) = expand_template(&t.body, title);
            out.push(PaletteItem {
                label: format!("!{}", t.name),
                kind: ItemKind::Insert { snippet, caret },
            });
        }
    }
}

/// Parse the `Templates` page into named templates. A `!name` at the start
/// of a line begins a template; following lines (until the next `!name`)
/// are its body. `![image]()` lines are not headers (the char after `!`
/// must be alphanumeric).
pub fn parse_templates(content: &str) -> Vec<Template> {
    let mut out = Vec::new();
    let mut current: Option<(String, Vec<&str>)> = None;
    for line in content.lines() {
        if let Some(name) = template_header(line) {
            if let Some((n, body)) = current.take() {
                out.push(Template {
                    name: n,
                    body: body.join("\n").trim().to_string(),
                });
            }
            current = Some((name.to_string(), Vec::new()));
        } else if let Some((_, body)) = current.as_mut() {
            body.push(line);
        }
    }
    if let Some((n, body)) = current {
        out.push(Template {
            name: n,
            body: body.join("\n").trim().to_string(),
        });
    }
    out.retain(|t| !t.body.is_empty());
    out
}

fn template_header(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('!')?;
    if rest.chars().next()?.is_ascii_alphanumeric() {
        Some(rest.trim())
    } else {
        None
    }
}

/// Expand a template body: substitute `{{date}}`/`{{time}}`/`{{title}}`,
/// and use `{{cursor}}` (removed) for the caret — else caret at the end.
fn expand_template(body: &str, title: &str) -> (String, usize) {
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    let date = format!(
        "{:04}-{:02}-{:02}",
        now.year(),
        u8::from(now.month()),
        now.day()
    );
    let time = format!("{:02}:{:02}", now.hour(), now.minute());
    let mut s = body
        .replace("{{date}}", &date)
        .replace("{{time}}", &time)
        .replace("{{title}}", title);
    match s.find("{{cursor}}") {
        Some(pos) => {
            s.replace_range(pos..pos + "{{cursor}}".len(), "");
            (s, pos)
        }
        None => {
            let end = s.len();
            (s, end)
        }
    }
}

fn is_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-'
}

fn is_boundary(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_alone_triggers_empty_query() {
        assert_eq!(detect("/", 1), Some((0, String::new())));
    }

    #[test]
    fn slash_query_at_start() {
        assert_eq!(detect("/todo", 5), Some((0, "todo".to_string())));
    }

    #[test]
    fn midword_slash_is_ignored() {
        assert_eq!(detect("and/or", 6), None);
    }

    #[test]
    fn parse_templates_sections() {
        let content = "!meeting\n## Notes\n- a\n\n!standup\n- yesterday\n- today";
        let t = parse_templates(content);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].name, "meeting");
        assert_eq!(t[0].body, "## Notes\n- a");
        assert_eq!(t[1].name, "standup");
        assert_eq!(t[1].body, "- yesterday\n- today");
    }

    #[test]
    fn image_line_is_not_a_template_header() {
        let t = parse_templates("![alt](url)\nplain");
        assert!(t.is_empty());
    }

    #[test]
    fn expand_substitutes_title_and_cursor() {
        let (s, caret) = expand_template("# {{title}}\n{{cursor}}done", "Hi");
        assert_eq!(s, "# Hi\ndone");
        assert_eq!(caret, "# Hi\n".len());
    }
}
