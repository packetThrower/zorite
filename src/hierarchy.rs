//! Page-namespace hierarchy. Pages nest via `::` in their titles, Logseq-style
//! (`Projects::Tasks` is "Tasks" under "Projects"). There's no parent column —
//! the title carries the path — so the tree and a page's direct children are
//! derived from the flat page list.

use crate::models::Page;

/// The namespace separator inside page titles.
pub const SEP: &str = "::";

/// A node in the derived page tree. `id` is `Some` for a real page and `None`
/// for a virtual namespace segment that only appears as a parent in some title
/// (e.g. `Projects` when only `Projects::Tasks` exists).
#[derive(Debug)]
pub struct PageNode {
    /// The leaf label at this level (e.g. `Tasks`).
    pub segment: String,
    /// The full page title this node opens (e.g. `Projects::Tasks`).
    pub path: String,
    /// The real page id, or `None` for a virtual namespace node.
    pub id: Option<i64>,
    pub children: Vec<PageNode>,
}

/// Build the page tree from a flat list of named pages, splitting each title on
/// `::`. A title with an empty segment (e.g. a leading `::`) is kept as a single
/// flat node. Each level is sorted case-insensitively by segment.
pub fn build_tree(pages: &[Page]) -> Vec<PageNode> {
    let mut roots: Vec<PageNode> = Vec::new();
    for page in pages {
        let split: Vec<&str> = page.title.split(SEP).map(str::trim).collect();
        // A malformed path (empty segment) is treated as one flat title.
        let segments: Vec<&str> = if split.iter().any(|s| s.is_empty()) {
            vec![page.title.as_str()]
        } else {
            split
        };

        let mut level = &mut roots;
        let mut prefix = String::new();
        let last = segments.len() - 1;
        for (depth, seg) in segments.iter().enumerate() {
            if depth > 0 {
                prefix.push_str(SEP);
            }
            prefix.push_str(seg);

            let idx = match level
                .iter()
                .position(|n| n.segment.eq_ignore_ascii_case(seg))
            {
                Some(i) => i,
                None => {
                    level.push(PageNode {
                        segment: (*seg).to_string(),
                        path: prefix.clone(),
                        id: None,
                        children: Vec::new(),
                    });
                    level.len() - 1
                }
            };
            if depth == last {
                level[idx].id = Some(page.id);
                level[idx].path = page.title.clone();
            }
            level = &mut level[idx].children;
        }
    }
    sort_level(&mut roots);
    roots
}

fn sort_level(nodes: &mut [PageNode]) {
    nodes.sort_by_key(|n| n.segment.to_lowercase());
    for n in nodes.iter_mut() {
        sort_level(&mut n.children);
    }
}

/// The direct children of `parent_title`: pages titled `parent_title::<leaf>`
/// with no further `::`. Case-insensitive on the prefix; input order preserved.
pub fn direct_children<'a>(pages: &'a [Page], parent_title: &str) -> Vec<&'a Page> {
    let pt = parent_title.as_bytes();
    let sep = SEP.as_bytes();
    pages
        .iter()
        .filter(|p| {
            let tb = p.title.as_bytes();
            tb.len() > pt.len() + sep.len()
                && tb[..pt.len()].eq_ignore_ascii_case(pt)
                && &tb[pt.len()..pt.len() + sep.len()] == sep
                && !p.title[pt.len() + sep.len()..].contains(SEP)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(id: i64, title: &str) -> Page {
        Page {
            id,
            title: title.to_string(),
            is_journal: false,
            journal_date: None,
            content: String::new(),
        }
    }

    #[test]
    fn flat_pages_are_roots() {
        let tree = build_tree(&[page(1, "Home"), page(2, "Work")]);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].segment, "Home");
        assert_eq!(tree[0].id, Some(1));
        assert!(tree[0].children.is_empty());
    }

    #[test]
    fn nests_on_separator_with_virtual_parent() {
        let tree = build_tree(&[page(1, "Projects::Tasks")]);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].segment, "Projects");
        assert_eq!(tree[0].id, None); // virtual — no real "Projects" page
        assert_eq!(tree[0].path, "Projects");
        assert_eq!(tree[0].children.len(), 1);
        let child = &tree[0].children[0];
        assert_eq!(child.segment, "Tasks");
        assert_eq!(child.id, Some(1));
        assert_eq!(child.path, "Projects::Tasks");
    }

    #[test]
    fn real_parent_merges_and_children_sort() {
        let tree = build_tree(&[
            page(1, "Projects"),
            page(2, "Projects::Tasks"),
            page(3, "Projects::Ideas"),
        ]);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].id, Some(1));
        let kids: Vec<&str> = tree[0]
            .children
            .iter()
            .map(|n| n.segment.as_str())
            .collect();
        assert_eq!(kids, vec!["Ideas", "Tasks"]); // alphabetical
    }

    #[test]
    fn deep_nesting() {
        let tree = build_tree(&[page(1, "A::B::C")]);
        let c = &tree[0].children[0].children[0];
        assert_eq!(c.segment, "C");
        assert_eq!(c.id, Some(1));
    }

    #[test]
    fn direct_children_by_prefix_and_depth() {
        let pages = vec![
            page(1, "Projects"),
            page(2, "Projects::Tasks"),
            page(3, "Projects::Tasks::Urgent"), // grandchild, excluded
            page(4, "Projects::Ideas"),
            page(5, "ProjectsX"), // not a child (no separator)
            page(6, "Other::Thing"),
        ];
        let mut got: Vec<&str> = direct_children(&pages, "Projects")
            .iter()
            .map(|p| p.title.as_str())
            .collect();
        got.sort();
        assert_eq!(got, vec!["Projects::Ideas", "Projects::Tasks"]);
    }

    #[test]
    fn empty_segment_title_stays_flat() {
        let tree = build_tree(&[page(1, "::alias")]);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].segment, "::alias");
        assert_eq!(tree[0].id, Some(1));
    }
}
