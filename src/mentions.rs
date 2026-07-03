//! Finding **unlinked mentions**: occurrences of a page title in note text
//! that aren't already a `[[link]]`, `#tag`, or part of some other link —
//! the "Unlinked References" panel and its one-click Link action. Linked
//! constructs are recognized by the shared `gpui_markdown::syntax::links`
//! grammar, so what counts as "already linked" can't drift from what the
//! views render as links.

use std::ops::Range;

use gpui_markdown::syntax::{LinkHit, is_word_char, links};

/// Byte ranges of every unlinked, word-bounded mention of `title` in
/// `content`, skipping fenced code blocks, inline code, and anything inside
/// an existing link construct. Matching is ASCII-case-insensitive (offsets
/// stay valid; non-ASCII titles match exactly).
pub fn unlinked_mention_ranges(content: &str, title: &str) -> Vec<Range<usize>> {
    let title = title.trim();
    let mut out = Vec::new();
    if title.len() < 2 {
        return out; // single characters are noise, not mentions
    }
    let needle = title.to_ascii_lowercase();
    let mut offset = 0;
    let mut in_fence = false;
    for line in content.split('\n') {
        let line_start = offset;
        offset += line.len() + 1;
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let linked = links(line);
        let code = inline_code_ranges(line);
        let hay = line.to_ascii_lowercase();
        let bytes = line.as_bytes();
        let mut from = 0;
        while let Some(rel) = hay[from..].find(&needle) {
            let start = from + rel;
            let end = start + needle.len();
            from = start + 1;
            // Word boundaries — but only where the title itself starts/ends
            // with a word character ("C++" needs no trailing boundary).
            let bounded_left = !needle.as_bytes()[0].is_ascii_alphanumeric()
                || start == 0
                || !is_word_char(bytes[start - 1]);
            let bounded_right = !needle.as_bytes()[needle.len() - 1].is_ascii_alphanumeric()
                || end >= line.len()
                || !is_word_char(bytes[end]);
            if !bounded_left || !bounded_right {
                continue;
            }
            // Already part of a link ([[..]], #tag, [t](url), bare URL) or
            // inside inline code: not an unlinked mention.
            let overlaps = |r: &Range<usize>| r.start < end && start < r.end;
            if linked.iter().any(|(r, _)| overlaps(r)) || code.iter().any(overlaps) {
                continue;
            }
            out.push(line_start + start..line_start + end);
        }
    }
    out
}

/// Rewrite `[[wiki-links]]` targeting `old_title` to `new_title`, tolerating
/// whitespace variants (`[[ Foo ]]`, `[[Foo |label]]`) and preserving alias
/// labels. Case-SENSITIVE by design: a differently-cased link reads as the
/// writer's choice, and links resolve case-insensitively anyway. Fenced code
/// is left alone. `None` when nothing matched.
pub fn rewrite_wiki_links(content: &str, old_title: &str, new_title: &str) -> Option<String> {
    let old = old_title.trim();
    let mut changed = false;
    let mut out = String::with_capacity(content.len());
    let mut in_fence = false;
    for (i, line) in content.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            out.push_str(line);
            continue;
        }
        if in_fence {
            out.push_str(line);
            continue;
        }
        let mut last = 0;
        for (range, hit) in links(line) {
            // `Page` hits are wiki-links AND #tags; only bracketed ones here.
            if !matches!(hit, LinkHit::Page(_)) || !line[range.clone()].starts_with("[[") {
                continue;
            }
            let inner = &line[range.start + 2..range.end - 2];
            let (target, label) = match inner.split_once('|') {
                Some((t, l)) => (t, Some(l)),
                None => (inner, None),
            };
            if target.trim() != old {
                continue;
            }
            out.push_str(&line[last..range.start]);
            out.push_str("[[");
            out.push_str(new_title);
            if let Some(label) = label {
                out.push('|');
                out.push_str(label);
            }
            out.push_str("]]");
            last = range.end;
            changed = true;
        }
        out.push_str(&line[last..]);
    }
    changed.then_some(out)
}

/// Inline `code` span ranges (the shared `links` grammar treats them as
/// opaque but doesn't report them, and a mention inside backticks isn't a
/// mention).
fn inline_code_ranges(line: &str) -> Vec<Range<usize>> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'`'
            && let Some(close) = (i + 1..b.len()).find(|&j| b[j] == b'`')
        {
            out.push(i..close + 1);
            i = close + 1;
            continue;
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_plain_mentions_only() {
        let content = "saw the Substation today\nand [[Substation]] linked\n#Substation too";
        let hits = unlinked_mention_ranges(content, "Substation");
        assert_eq!(hits.len(), 1);
        assert_eq!(&content[hits[0].clone()], "Substation");
    }

    #[test]
    fn case_insensitive_with_word_boundaries() {
        let content = "the substation hums; substations plural; SUBSTATION caps";
        let hits = unlinked_mention_ranges(content, "Substation");
        // "substations" fails the right boundary; the other two match.
        assert_eq!(hits.len(), 2);
        assert_eq!(&content[hits[0].clone()], "substation");
        assert_eq!(&content[hits[1].clone()], "SUBSTATION");
    }

    #[test]
    fn code_and_links_are_opaque() {
        let content = "```\nSubstation in a fence\n```\n`Substation` inline\n[Substation](https://x.io) linked\n[[Other|Substation]] alias";
        assert!(unlinked_mention_ranges(content, "Substation").is_empty());
    }

    #[test]
    fn rewrite_handles_whitespace_and_labels_not_case() {
        let content = "a [[ Foo ]] b [[Foo|nick]] c [[FOO]] d [[Foobar]] e `[[Foo]]`";
        let out = rewrite_wiki_links(content, "Foo", "Bar").unwrap();
        assert_eq!(
            out,
            "a [[Bar]] b [[Bar|nick]] c [[FOO]] d [[Foobar]] e `[[Foo]]`"
        );
        // Fenced code is untouched; no match → None.
        assert!(rewrite_wiki_links("```\n[[Foo]]\n```", "Foo", "Bar").is_none());
        assert!(rewrite_wiki_links("no links here", "Foo", "Bar").is_none());
    }

    #[test]
    fn multi_word_and_tiny_titles() {
        let content = "met Don Jensen at the plant";
        let hits = unlinked_mention_ranges(content, "Don Jensen");
        assert_eq!(hits.len(), 1);
        // 1-char titles are noise by policy.
        assert!(unlinked_mention_ranges("a b c", "a").is_empty());
    }
}
