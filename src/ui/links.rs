//! `[[wiki-link]]` parsing. Pure string work used to index a page's
//! outgoing links whenever its content is saved.

/// Distinct link target titles in a page's text, first-seen order,
/// case-insensitively de-duplicated. An unterminated or empty `[[ ]]`
/// is ignored.
pub fn parse_links(content: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = content;
    while let Some(open) = rest.find("[[") {
        let Some(close_rel) = rest[open + 2..].find("]]") else { break };
        let title = rest[open + 2..open + 2 + close_rel].trim();
        if !title.is_empty() && !out.iter().any(|t| t.eq_ignore_ascii_case(title)) {
            out.push(title.to_string());
        }
        rest = &rest[open + 2 + close_rel + 2..];
    }
    out
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
        assert_eq!(parse_links("[[  Spaced Name  ]]"), vec!["Spaced Name".to_string()]);
    }

    #[test]
    fn unterminated_yields_no_link() {
        assert!(parse_links("a [[ open only").is_empty());
    }

    #[test]
    fn empty_brackets_ignored() {
        assert!(parse_links("x [[]] y").is_empty());
    }
}
