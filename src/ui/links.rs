//! `[[wiki-link]]` parsing. Pure string work used to index a page's
//! outgoing links whenever its content is saved.

/// Distinct link target titles in a page's text, first-seen order,
/// case-insensitively de-duplicated. An unterminated or empty `[[ ]]`
/// is ignored.
pub fn parse_links(content: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();

    // [[wiki-links]]
    let mut rest = content;
    while let Some(open) = rest.find("[[") {
        let Some(close_rel) = rest[open + 2..].find("]]") else {
            break;
        };
        push_unique(&mut out, &rest[open + 2..open + 2 + close_rel]);
        rest = &rest[open + 2 + close_rel + 2..];
    }

    // #tags (a `#tag` links to a page named `tag`)
    let bytes = content.as_bytes();
    let mut i = 0;
    while i < content.len() {
        if bytes[i] == b'#' && (i == 0 || is_boundary(bytes[i - 1])) {
            let mut j = i + 1;
            while j < content.len() && is_tag_char(bytes[j]) {
                j += 1;
            }
            if j > i + 1 {
                push_unique(&mut out, &content[i + 1..j]);
                i = j;
                continue;
            }
        }
        i += content[i..].chars().next().map_or(1, |c| c.len_utf8());
    }
    out
}

/// Split a page's alias field — a comma-separated list like `hen, rooster,
/// chick` — into trimmed, case-insensitively de-duplicated names.
pub fn parse_alias_list(input: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in input.split(',') {
        push_unique(&mut out, name);
    }
    out
}

fn push_unique(out: &mut Vec<String>, title: &str) {
    let title = title.trim();
    if !title.is_empty() && !out.iter().any(|t| t.eq_ignore_ascii_case(title)) {
        out.push(title.to_string());
    }
}

fn is_boundary(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'(' | b'[')
}

fn is_tag_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_links() {
        assert!(parse_links("just plain text").is_empty());
    }

    #[test]
    fn single_link() {
        assert_eq!(parse_links("see [[Foo]] now"), vec!["Foo".to_string()]);
    }

    #[test]
    fn multiple_and_dedup_case_insensitive() {
        assert_eq!(
            parse_links("[[A]] and [[b]] and [[a]]\nmore [[B]]"),
            vec!["A".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn inner_is_trimmed() {
        assert_eq!(
            parse_links("[[  Spaced Name  ]]"),
            vec!["Spaced Name".to_string()]
        );
    }

    #[test]
    fn unterminated_yields_no_link() {
        assert!(parse_links("a [[ open only").is_empty());
    }

    #[test]
    fn empty_brackets_ignored() {
        assert!(parse_links("x [[]] y").is_empty());
    }

    #[test]
    fn extracts_tags_and_dedups_with_wikilinks() {
        assert_eq!(
            parse_links("see [[Foo]] and #bar then #foo"),
            vec!["Foo".to_string(), "bar".to_string()]
        );
    }

    #[test]
    fn tag_needs_boundary_and_chars() {
        assert!(parse_links("a#b is not a tag").is_empty());
        assert!(parse_links("# heading not a tag").is_empty());
    }

    #[test]
    fn alias_list_splits_trims_and_dedups() {
        assert_eq!(
            parse_alias_list("hen, rooster ,Hen,"),
            vec!["hen", "rooster"]
        );
    }

    #[test]
    fn alias_list_empty() {
        assert!(parse_alias_list("").is_empty());
        assert!(parse_alias_list("  ,  , ").is_empty());
    }
}
