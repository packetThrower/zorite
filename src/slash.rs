//! The `/` command palette: detecting a `/query` at the caret, and the
//! set of things it can insert — built-in markdown snippets (from
//! `gpui-markdown`) plus user **templates** parsed from a reserved
//! `Templates` page. `AppView` owns the open `Slash`, keyboard handling,
//! and insertion.

use gpui::{Bounds, Pixels};
use gpui_markdown::SNIPPETS;

use crate::models::Page;

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

/// Which completion is open, keyed by its trigger prefix.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trigger {
    /// `/` — markdown commands + templates (has submenu levels).
    Slash,
    /// `[[` — link to a page.
    Link,
    /// `#` — tag (also a page).
    Tag,
    /// `{{` — template placeholder.
    Placeholder,
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
    pub trigger: Trigger,
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

/// Detect a completion trigger ending at the caret: the trigger, the byte
/// offset of its first char (insertion replaces from there), and the query
/// typed after it. `[[` / `{{` (queries may contain spaces) take priority
/// over the single-char `#` / `/`.
pub fn detect(value: &str, cursor: usize) -> Option<(Trigger, usize, String)> {
    let cursor = cursor.min(value.len());
    if let Some((start, q)) = detect_bracket(value, cursor, "[[", "]]") {
        return Some((Trigger::Link, start, q));
    }
    if let Some((start, q)) = detect_bracket(value, cursor, "{{", "}}") {
        return Some((Trigger::Placeholder, start, q));
    }
    // Tag: `#` at a boundary with at least one tag char after it, so a lone
    // `#` and markdown headings (`# `) don't trigger.
    if let Some((start, q)) = detect_token(value, cursor, b'#', is_tag_char)
        && !q.is_empty()
    {
        return Some((Trigger::Tag, start, q));
    }
    if let Some((start, q)) = detect_token(value, cursor, b'/', is_token_char) {
        return Some((Trigger::Slash, start, q));
    }
    None
}

/// An open `open`..caret span with no `close`, newline, or nested `open`
/// between — i.e. an unclosed `[[` / `{{` on the current line.
fn detect_bracket(value: &str, cursor: usize, open: &str, close: &str) -> Option<(usize, String)> {
    let open_pos = value[..cursor].rfind(open)?;
    let query = &value[open_pos + open.len()..cursor];
    if query.contains(close) || query.contains('\n') || query.contains(open) {
        return None;
    }
    Some((open_pos, query.to_string()))
}

/// A `prefix` byte at a word boundary, followed by an `is_char` run up to
/// the caret. Returns the prefix offset and the run.
fn detect_token(
    value: &str,
    cursor: usize,
    prefix: u8,
    is_char: fn(u8) -> bool,
) -> Option<(usize, String)> {
    let bytes = value.as_bytes();
    let mut i = cursor;
    while i > 0 && is_char(bytes[i - 1]) {
        i -= 1;
    }
    if i == 0 || bytes[i - 1] != prefix {
        return None;
    }
    let start = i - 1;
    if start > 0 && !is_boundary(bytes[start - 1]) {
        return None;
    }
    Some((start, value[i..cursor].to_string()))
}

/// Build the palette for the current level + query:
/// - Root + empty query → the category rows (`Markdown ›`, `Templates ›`).
/// - Root + a query → a flattened search over everything (so `/table` works).
/// - a submenu → that category's items, filtered by the query.
pub fn build_slash_items(
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
            datetime_items(&q, &mut out);
        }
        SlashLevel::Root => {
            markdown_items(&q, &mut out);
            datetime_items(&q, &mut out);
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

/// `/date` and `/time`: insert the current local date/time directly. Distinct
/// from the `{{date}}` / `{{time}}` template placeholders, which only expand
/// inside a template body. The value to be inserted is shown in the label.
fn datetime_items(q: &str, out: &mut Vec<PaletteItem>) {
    for (label, value) in [("Date", current_date()), ("Time", current_time())] {
        if q.is_empty() || label.to_lowercase().contains(q) {
            out.push(insert_item(format!("{label} ({value})"), value));
        }
    }
}

/// Max page-sourced completion rows shown at once; type to narrow further.
const MAX_COMPLETION_ITEMS: usize = 8;

/// Page-link items for `[[query`: the best-matching page titles → `[[Title]]`,
/// plus a "Create" entry when the query names a page that doesn't exist yet.
pub fn build_link_items(query: &str, pages: &[Page]) -> Vec<PaletteItem> {
    let q = query.trim().to_lowercase();
    let (titles, exact) = ranked_titles(&q, pages, |_| true);
    let mut out: Vec<PaletteItem> = titles
        .into_iter()
        .map(|t| insert_item(t.clone(), format!("[[{t}]]")))
        .collect();
    let trimmed = query.trim();
    if !trimmed.is_empty() && !exact {
        out.push(insert_item(
            format!("Create \"{trimmed}\""),
            format!("[[{trimmed}]]"),
        ));
    }
    out
}

/// Tag items for `#query`: the best-matching tag-valid page titles → `#tag`,
/// plus a "Create" entry. (`#tag` links to a page named `tag`, so pages are
/// the source.)
pub fn build_tag_items(query: &str, pages: &[Page]) -> Vec<PaletteItem> {
    let q = query.trim().to_lowercase();
    let (titles, exact) = ranked_titles(&q, pages, is_valid_tag);
    let mut out: Vec<PaletteItem> = titles
        .into_iter()
        .map(|t| insert_item(format!("#{t}"), format!("#{t}")))
        .collect();
    let trimmed = query.trim();
    if !trimmed.is_empty() && is_valid_tag(trimmed) && !exact {
        out.push(insert_item(
            format!("Create #{trimmed}"),
            format!("#{trimmed}"),
        ));
    }
    out
}

/// Page titles matching `q` (already lowercased; empty = all), kept only when
/// `accept` holds, ranked prefix-matches-first then alphabetically, and capped
/// at `MAX_COMPLETION_ITEMS`. Returns the titles and whether one equals `q`.
fn ranked_titles(q: &str, pages: &[Page], accept: fn(&str) -> bool) -> (Vec<String>, bool) {
    // Empty query: `list_pages` is already alphabetical, so just take a few.
    if q.is_empty() {
        let titles = pages
            .iter()
            .filter(|p| accept(&p.title))
            .take(MAX_COMPLETION_ITEMS)
            .map(|p| p.title.clone())
            .collect();
        return (titles, false);
    }
    let mut matches: Vec<(u8, &str)> = Vec::new();
    let mut exact = false;
    for p in pages {
        if !accept(&p.title) {
            continue;
        }
        let lower = p.title.to_lowercase();
        if let Some(pos) = lower.find(q) {
            exact |= lower == q;
            // Rank 0 = prefix match, 1 = match elsewhere.
            matches.push((u8::from(pos != 0), p.title.as_str()));
        }
    }
    matches.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
    });
    let titles = matches
        .into_iter()
        .take(MAX_COMPLETION_ITEMS)
        .map(|(_, t)| t.to_string())
        .collect();
    (titles, exact)
}

/// Placeholder items for `{{query`: the template placeholders → `{{name}}`.
pub fn build_placeholder_items(query: &str) -> Vec<PaletteItem> {
    let q = query.trim().to_lowercase();
    let mut out = Vec::new();
    for name in ["date", "time", "title", "cursor"] {
        if q.is_empty() || name.contains(q.as_str()) {
            let ph = ["{{", name, "}}"].concat();
            out.push(insert_item(ph.clone(), ph));
        }
    }
    out
}

/// An `Insert` palette item that drops the caret at the end of `snippet`.
fn insert_item(label: String, snippet: String) -> PaletteItem {
    let caret = snippet.len();
    PaletteItem {
        label,
        kind: ItemKind::Insert { snippet, caret },
    }
}

fn is_valid_tag(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(is_tag_char)
}

fn is_tag_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
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
/// Local now, falling back to UTC when the offset can't be determined.
fn local_now() -> time::OffsetDateTime {
    time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc())
}

/// Current date as `YYYY-MM-DD`.
fn current_date() -> String {
    let now = local_now();
    format!(
        "{:04}-{:02}-{:02}",
        now.year(),
        u8::from(now.month()),
        now.day()
    )
}

/// Current time as `HH:MM` (24-hour).
fn current_time() -> String {
    let now = local_now();
    format!("{:02}:{:02}", now.hour(), now.minute())
}

fn expand_template(body: &str, title: &str) -> (String, usize) {
    let mut s = body
        .replace("{{date}}", &current_date())
        .replace("{{time}}", &current_time())
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

// --- Auto-pairing of brackets / quotes ---

/// What to do in reaction to a bracket/quote edit at the caret.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AutoPair {
    /// Insert this closing char after the caret (caret stays put).
    Close(char),
    /// The typed closer duplicates the one already at the caret — drop that
    /// existing char (this many bytes) so the caret just steps over it.
    TypeOver(usize),
    /// An opener was typed over a selection: re-insert the selected `inner`
    /// followed by `close` after the opener, wrapping it (`foo` → `(foo)`).
    Wrap { close: char, inner: String },
}

/// Decide the auto-pair reaction to an edit. `prev`/`new` are the editor text
/// before/after the change and `cursor` is the caret byte offset in `new`.
/// Recognizes a single bracket/quote typed at the caret (type-over or
/// auto-close) and an opener typed over a selection (wrap). Returns `None` for
/// anything else — deletes, pastes, ordinary typing, and no-op changes.
pub fn autopair_action(prev: &str, new: &str, cursor: usize) -> Option<AutoPair> {
    if cursor == 0 || cursor > new.len() || new == prev {
        return None;
    }
    let ch = new[..cursor].chars().next_back()?;
    let ch_len = ch.len_utf8();
    let prefix = &new[..cursor - ch_len];
    let suffix = &new[cursor..];
    // The change must be "replace prev's middle (the old selection, possibly
    // empty) with `ch`": prev == prefix + <middle> + suffix.
    if !prev.starts_with(prefix)
        || !prev.ends_with(suffix)
        || prev.len() < prefix.len() + suffix.len()
    {
        return None;
    }
    let inner = &prev[prefix.len()..prev.len() - suffix.len()];
    if !inner.is_empty() {
        // An opener typed over a selection wraps it; a non-opener just replaces.
        let close = open_to_close(ch)?;
        return Some(AutoPair::Wrap {
            close,
            inner: inner.to_string(),
        });
    }
    // Pure single-char insertion.
    let next = suffix.chars().next();
    // Type-over: a closer typed right in front of the same closer.
    if is_close_char(ch) && next == Some(ch) {
        return Some(AutoPair::TypeOver(ch_len));
    }
    // Auto-close an opener, subject to the prose-safe guards.
    if let Some(close) = open_to_close(ch) {
        let before = prefix.chars().next_back();
        if should_autoclose(ch, before, next) {
            return Some(AutoPair::Close(close));
        }
    }
    None
}

/// Backspacing an empty pair: if `new` is `prev` with a single opening bracket
/// deleted at the caret and its matching closer now sits right at the caret,
/// return that closer's byte length so the caller deletes it too (`(|)` → ``).
pub fn autopair_backspace(prev: &str, new: &str, cursor: usize) -> Option<usize> {
    if cursor > new.len() || prev.len() <= new.len() {
        return None;
    }
    let prefix = &new[..cursor];
    let suffix = &new[cursor..];
    if !prev.starts_with(prefix) {
        return None;
    }
    // Exactly one char (the deleted opener) sat between prefix and suffix.
    let deleted = prev[cursor..].chars().next()?;
    if &prev[cursor + deleted.len_utf8()..] != suffix {
        return None;
    }
    let close = open_to_close(deleted)?;
    if suffix.starts_with(close) {
        return Some(close.len_utf8());
    }
    None
}

/// The closing char for an opening bracket/quote (quotes pair with themselves).
fn open_to_close(c: char) -> Option<char> {
    Some(match c {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '<' => '>',
        '"' => '"',
        '\'' => '\'',
        _ => return None,
    })
}

fn is_close_char(c: char) -> bool {
    matches!(c, ')' | ']' | '}' | '>' | '"' | '\'')
}

/// A "word" char — auto-pairing avoids jamming pairs into identifiers/contractions.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether to auto-close `open`, given the chars surrounding the caret. The
/// shared rule is "don't insert a closer straight in front of a word". Quotes
/// also won't pair after a word (so `don't` survives); `<` only pairs after a
/// word (so prose `a < b` is left alone but `Vec<` becomes `Vec<>`).
fn should_autoclose(open: char, before: Option<char>, next: Option<char>) -> bool {
    let next_ok = next.is_none_or(|c| !is_word(c));
    match open {
        '"' | '\'' => next_ok && before.is_none_or(|c| !is_word(c)),
        '<' => next_ok && before.is_some_and(is_word),
        _ => next_ok,
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
        assert_eq!(detect("/", 1), Some((Trigger::Slash, 0, String::new())));
    }

    #[test]
    fn slash_query_at_start() {
        assert_eq!(
            detect("/todo", 5),
            Some((Trigger::Slash, 0, "todo".to_string()))
        );
    }

    #[test]
    fn midword_slash_is_ignored() {
        assert_eq!(detect("and/or", 6), None);
    }

    #[test]
    fn link_trigger_allows_spaces() {
        assert_eq!(
            detect("see [[Palo Al", 13),
            Some((Trigger::Link, 4, "Palo Al".to_string()))
        );
    }

    #[test]
    fn closed_link_does_not_trigger() {
        assert_eq!(detect("see [[Foo]] x", 13), None);
    }

    #[test]
    fn tag_needs_a_char_heading_does_not() {
        assert_eq!(
            detect("a #pro", 6),
            Some((Trigger::Tag, 2, "pro".to_string()))
        );
        assert_eq!(detect("# heading", 2), None);
    }

    #[test]
    fn placeholder_trigger() {
        assert_eq!(
            detect("x {{da", 6),
            Some((Trigger::Placeholder, 2, "da".to_string()))
        );
    }

    #[test]
    fn placeholder_items_insert_braces() {
        let items = build_placeholder_items("da");
        assert_eq!(items.len(), 1);
        let ItemKind::Insert { snippet, caret } = &items[0].kind else {
            panic!("expected insert");
        };
        assert_eq!(snippet, "{{date}}");
        assert_eq!(*caret, "{{date}}".len());
    }

    #[test]
    fn link_items_offer_create_for_new_title() {
        let items = build_link_items("New", &[]);
        assert_eq!(items.len(), 1);
        let ItemKind::Insert { snippet, .. } = &items[0].kind else {
            panic!("expected insert");
        };
        assert_eq!(snippet, "[[New]]");
    }

    #[test]
    fn link_items_are_capped() {
        let pages: Vec<Page> = (0..20)
            .map(|i| Page {
                id: i,
                title: format!("proj{i:02}"),
                is_journal: false,
                journal_date: None,
                content: String::new(),
            })
            .collect();
        let items = build_link_items("proj", &pages);
        // Capped matches + one "Create" entry (no exact "proj").
        assert_eq!(items.len(), MAX_COMPLETION_ITEMS + 1);
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

    #[test]
    fn date_time_commands_insert_current_values() {
        // Both appear at the root, and `/date` / `/time` narrow to each.
        let root = build_slash_items(SlashLevel::Root, "", &[], "");
        let labels: Vec<&str> = root.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.iter().any(|l| l.starts_with("Date (")));
        assert!(labels.iter().any(|l| l.starts_with("Time (")));

        let date = build_slash_items(SlashLevel::Root, "date", &[], "");
        let item = date
            .iter()
            .find(|i| i.label.starts_with("Date ("))
            .expect("date command");
        let ItemKind::Insert { snippet, caret } = &item.kind else {
            panic!("expected insert");
        };
        // YYYY-MM-DD, caret at the end.
        assert_eq!(snippet.len(), 10);
        assert_eq!(snippet.as_bytes()[4], b'-');
        assert_eq!(snippet.as_bytes()[7], b'-');
        assert_eq!(*caret, snippet.len());

        let time = build_slash_items(SlashLevel::Root, "time", &[], "");
        let item = time
            .iter()
            .find(|i| i.label.starts_with("Time ("))
            .expect("time command");
        let ItemKind::Insert { snippet, .. } = &item.kind else {
            panic!("expected insert");
        };
        // HH:MM
        assert_eq!(snippet.len(), 5);
        assert_eq!(snippet.as_bytes()[2], b':');
    }

    #[test]
    fn autopair_closes_brackets_at_end() {
        assert_eq!(autopair_action("", "(", 1), Some(AutoPair::Close(')')));
        assert_eq!(autopair_action("", "[", 1), Some(AutoPair::Close(']')));
        assert_eq!(autopair_action("", "{", 1), Some(AutoPair::Close('}')));
        assert_eq!(autopair_action("a ", "a (", 3), Some(AutoPair::Close(')')));
    }

    #[test]
    fn autopair_skips_bracket_in_front_of_word() {
        // `(` typed right before `word` shouldn't jam a `)` into it.
        assert_eq!(autopair_action("word", "(word", 1), None);
    }

    #[test]
    fn autopair_types_over_matching_closer() {
        // At `(|)` typing `)` steps over the existing one instead of adding.
        assert_eq!(autopair_action("()", "())", 2), Some(AutoPair::TypeOver(1)));
        // Walking out of `[[x|]]` by typing `]` (caret now sits after it, at 4).
        assert_eq!(
            autopair_action("[[x]]", "[[x]]]", 4),
            Some(AutoPair::TypeOver(1))
        );
    }

    #[test]
    fn autopair_quote_is_contraction_safe() {
        // `'` after a word char (don|t) is an apostrophe, not an open quote.
        assert_eq!(autopair_action("don", "don'", 4), None);
        // `'` after a space opens a quote pair.
        assert_eq!(
            autopair_action("say ", "say '", 5),
            Some(AutoPair::Close('\''))
        );
        assert_eq!(autopair_action("", "\"", 1), Some(AutoPair::Close('"')));
    }

    #[test]
    fn autopair_angle_only_after_word() {
        // `Vec<` is generic-like → pair; prose `a < b` is not.
        assert_eq!(
            autopair_action("Vec", "Vec<", 4),
            Some(AutoPair::Close('>'))
        );
        assert_eq!(autopair_action("a ", "a <", 3), None);
    }

    #[test]
    fn autopair_ignores_non_insertions() {
        // Deletion (text got shorter).
        assert_eq!(autopair_action("abc", "ab", 2), None);
        // Cursor moved with no edit.
        assert_eq!(autopair_action("[x]", "[x]", 1), None);
        // Caret at start.
        assert_eq!(autopair_action("x", "[x", 0), None);
        // A multi-char paste ending in a bracket isn't a single keystroke.
        assert_eq!(autopair_action("", "ab(", 3), None);
        // No-op change (caret-only) doesn't wrap the char before the caret.
        assert_eq!(autopair_action("()", "()", 1), None);
    }

    #[test]
    fn autopair_wraps_a_selection() {
        // Select "foo" (offsets 4..7) in "say foo" and type "(" -> "say (".
        assert_eq!(
            autopair_action("say foo", "say (", 5),
            Some(AutoPair::Wrap {
                close: ')',
                inner: "foo".to_string(),
            })
        );
        // Selecting everything and typing a quote wraps too.
        assert_eq!(
            autopair_action("foo", "\"", 1),
            Some(AutoPair::Wrap {
                close: '"',
                inner: "foo".to_string(),
            })
        );
    }

    #[test]
    fn autopair_non_bracket_over_selection_does_not_wrap() {
        assert_eq!(autopair_action("foo", "x", 1), None);
    }

    #[test]
    fn autopair_backspace_deletes_empty_pair() {
        // `(|)` backspace removes `(` -> `)` (caret 0); the `)` should go too.
        assert_eq!(autopair_backspace("()", ")", 0), Some(1));
        // `([|])` backspace removes `[` -> `(])` (caret 1); drop the orphan `]`.
        assert_eq!(autopair_backspace("([])", "(])", 1), Some(1));
    }

    #[test]
    fn autopair_backspace_ignores_non_empty_or_non_pairs() {
        // The pair isn't empty (an `x` sits inside) -> leave the closer.
        assert_eq!(autopair_backspace("(x)", "x)", 0), None);
        // Deleting a non-opener.
        assert_eq!(autopair_backspace("ab", "a", 1), None);
        // Deleting the closer itself, not the opener.
        assert_eq!(autopair_backspace("()", "(", 1), None);
    }
}
