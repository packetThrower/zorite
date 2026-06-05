//! `[[wiki-link]]` parsing. Pure string work, shared by the save path
//! (which titles get linked) and the render path (how a block's text is
//! split into plain runs and clickable links).

/// A piece of a block's rendered text.
pub enum Segment {
    /// Plain text run.
    Text(String),
    /// A `[[Page Name]]` reference — the inner title, trimmed.
    Link(String),
}

/// Distinct link target titles in a block's text, first-seen order,
/// case-insensitively de-duplicated. Used to rebuild a block's outgoing
/// links on save.
pub fn parse_links(content: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for seg in segments(content) {
        if let Segment::Link(title) = seg {
            if !out.iter().any(|t| t.eq_ignore_ascii_case(&title)) {
                out.push(title);
            }
        }
    }
    out
}

/// Split a block's text into plain runs and `[[links]]`. An unterminated
/// or empty `[[ ]]` is left as literal text.
pub fn segments(content: &str) -> Vec<Segment> {
    let mut segs = Vec::new();
    let mut rest = content;
    loop {
        let Some(open) = rest.find("[[") else {
            if !rest.is_empty() {
                segs.push(Segment::Text(rest.to_string()));
            }
            break;
        };
        let Some(close_rel) = rest[open + 2..].find("]]") else {
            // No closing — the rest is plain text.
            segs.push(Segment::Text(rest.to_string()));
            break;
        };
        let title = rest[open + 2..open + 2 + close_rel].trim();
        let after = open + 2 + close_rel + 2;
        if open > 0 {
            segs.push(Segment::Text(rest[..open].to_string()));
        }
        if title.is_empty() {
            // "[[]]" or "[[   ]]" — keep it literal.
            segs.push(Segment::Text(rest[open..after].to_string()));
        } else {
            segs.push(Segment::Link(title.to_string()));
        }
        rest = &rest[after..];
    }
    segs
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
            parse_links("[[A]] and [[b]] and [[a]]"),
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

    #[test]
    fn segments_split_text_and_link() {
        let segs = segments("a [[L]] b");
        assert_eq!(segs.len(), 3);
        assert!(matches!(&segs[0], Segment::Text(t) if t == "a "));
        assert!(matches!(&segs[1], Segment::Link(t) if t == "L"));
        assert!(matches!(&segs[2], Segment::Text(t) if t == " b"));
    }
}
