//! Type-aware global search: pages, whiteboards (by title), plus the PDF and
//! image *files* referenced in pages. A `pdf:` / `img:` / `page:` / `wb:` prefix
//! (or the matching results-pane chip) filters by kind; a bare query returns
//! every kind that matches.
//!
//! Files aren't their own rows in the database — they live inside pages as
//! `[[pdf/x.pdf]]` / `![](images/x.png)` references. So a file search runs the
//! normal full-text query, then extracts the referenced files (whose name or alt
//! text matches the term) from the matched pages. A bare type filter with no term
//! (`pdf:`) browses every file of that kind from the managed `pdf/` / `images/`
//! store.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::db::Db;

/// How the results are narrowed by kind — set by a search-box prefix or a chip.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Filter {
    #[default]
    All,
    Page,
    Whiteboard,
    Pdf,
    Image,
}

impl Filter {
    /// The search-box prefix that selects this filter (empty for `All`).
    pub fn prefix(self) -> &'static str {
        match self {
            Filter::All => "",
            Filter::Page => "page:",
            Filter::Whiteboard => "wb:",
            Filter::Pdf => "pdf:",
            Filter::Image => "img:",
        }
    }
}

/// What a hit is — drives its icon and how a click opens it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Page,
    Whiteboard,
    Pdf,
    Image,
}

/// What clicking a hit opens.
#[derive(Clone)]
pub enum Target {
    /// Open the page.
    Page(i64),
    /// Open the PDF in the viewer.
    Pdf(PathBuf),
    /// Open the page that shows the image (resolved lazily by the host when the
    /// referencing page isn't known, e.g. when browsing all images).
    Image { src: String, in_page: Option<i64> },
}

/// One search result row.
#[derive(Clone)]
pub struct Hit {
    pub kind: Kind,
    pub title: String,
    pub subtitle: String,
    pub target: Target,
}

/// Per-kind totals over the unfiltered set — drives the chip counts.
#[derive(Clone, Copy, Default)]
pub struct Counts {
    pub page: usize,
    pub whiteboard: usize,
    pub pdf: usize,
    pub image: usize,
}

impl Counts {
    pub fn total(&self) -> usize {
        self.page + self.whiteboard + self.pdf + self.image
    }
}

/// A completed search: kind-filtered hits plus the counts and active filter.
#[derive(Clone, Default)]
pub struct Results {
    pub hits: Vec<Hit>,
    pub counts: Counts,
    pub filter: Filter,
    pub term: String,
}

const LIMIT: i64 = 50;

/// Parse a leading `pdf:` / `img:` / `page:` prefix, returning the filter and the
/// remaining (trimmed) search term.
pub fn parse_prefix(query: &str) -> (Filter, String) {
    let q = query.trim();
    for (p, f) in [
        ("pdf:", Filter::Pdf),
        ("img:", Filter::Image),
        ("page:", Filter::Page),
        ("wb:", Filter::Whiteboard),
    ] {
        if let Some(rest) = q.strip_prefix(p) {
            return (f, rest.trim().to_string());
        }
    }
    (Filter::All, q.to_string())
}

/// Run a search for `query` (prefix included). Returns the kind-filtered hits plus
/// per-kind counts for the results-pane chips.
pub fn run(db: &Db, query: &str) -> Results {
    let (filter, term) = parse_prefix(query);
    let mut hits = if term.is_empty() {
        // A bare empty query shows nothing; a type filter with no term browses
        // every item of that kind (files from the managed store; all boards).
        match filter {
            Filter::Pdf => all_files(Kind::Pdf),
            Filter::Image => all_files(Kind::Image),
            Filter::Whiteboard => whiteboard_hits(db, ""),
            _ => Vec::new(),
        }
    } else {
        collect(db, &term)
    };
    let counts = Counts {
        page: hits.iter().filter(|h| h.kind == Kind::Page).count(),
        whiteboard: hits.iter().filter(|h| h.kind == Kind::Whiteboard).count(),
        pdf: hits.iter().filter(|h| h.kind == Kind::Pdf).count(),
        image: hits.iter().filter(|h| h.kind == Kind::Image).count(),
    };
    if let Some(want) = filter_kind(filter) {
        hits.retain(|h| h.kind == want);
    }
    Results {
        hits,
        counts,
        filter,
        term,
    }
}

fn filter_kind(filter: Filter) -> Option<Kind> {
    match filter {
        Filter::All => None,
        Filter::Page => Some(Kind::Page),
        Filter::Whiteboard => Some(Kind::Whiteboard),
        Filter::Pdf => Some(Kind::Pdf),
        Filter::Image => Some(Kind::Image),
    }
}

/// Whiteboards whose title contains `needle` (empty → all). Title-only — a
/// board's content is canvas JSON, not searchable text — so this is a plain
/// filter over [`Db::list_whiteboards`], newest first, capped at `LIMIT`.
fn whiteboard_hits(db: &Db, needle: &str) -> Vec<Hit> {
    let needle = needle.to_lowercase();
    db.list_whiteboards()
        .unwrap_or_default()
        .into_iter()
        .filter(|w| needle.is_empty() || w.title.to_lowercase().contains(&needle))
        .take(LIMIT as usize)
        .map(|w| Hit {
            kind: Kind::Whiteboard,
            title: w.title,
            subtitle: "Whiteboard".into(),
            target: Target::Page(w.id),
        })
        .collect()
}

/// Search pages for `term`, then pull the PDF / image files referenced on each
/// matched page whose filename (or, for images, alt text) also contains the term.
/// Files are deduplicated across pages (first referencing page wins for context).
fn collect(db: &Db, term: &str) -> Vec<Hit> {
    let rows = db.search_rows(term, LIMIT).unwrap_or_default();
    let needle = term.to_lowercase();
    let mut hits = Vec::new();
    let mut seen_pdf = HashSet::new();
    let mut seen_img = HashSet::new();
    for (id, title, content) in &rows {
        // The page itself (it matched the term somewhere in its title/content).
        hits.push(Hit {
            kind: Kind::Page,
            title: title.clone(),
            subtitle: crate::db::snippet_for_query(content, term),
            target: Target::Page(*id),
        });
        // PDFs referenced here whose filename matches the term.
        for path in pdf_refs(content) {
            let name = file_name(&path);
            if name.to_lowercase().contains(&needle)
                && seen_pdf.insert(path.clone())
                && let Some(abs) = crate::pdf::resolve_path(&path)
            {
                hits.push(Hit {
                    kind: Kind::Pdf,
                    title: name,
                    subtitle: format!("PDF · in {title}"),
                    target: Target::Pdf(abs),
                });
            }
        }
        // Images referenced here whose filename or alt text matches the term.
        for img in gpui_markdown::images(content) {
            let src = img.src.to_string();
            if !src.starts_with("images/") {
                continue; // managed local images only (remote URLs aren't files)
            }
            let name = file_name(&src);
            let alt = img.alt.to_string();
            let hit = name.to_lowercase().contains(&needle) || alt.to_lowercase().contains(&needle);
            if hit && seen_img.insert(src.clone()) {
                hits.push(Hit {
                    kind: Kind::Image,
                    title: if alt.trim().is_empty() { name } else { alt },
                    subtitle: format!("Image · in {title}"),
                    target: Target::Image {
                        src,
                        in_page: Some(*id),
                    },
                });
            }
        }
    }
    // Whiteboards that match by title (their canvas JSON isn't full-text indexed).
    hits.extend(whiteboard_hits(db, term));
    hits
}

/// PDF references on a page: `[[pdf/x.pdf]]` wiki-links (any `#pN` jump fragment
/// stripped) and `![](pdf/x.pdf)` image-syntax chips.
fn pdf_refs(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    for t in crate::ui::links::parse_links(content) {
        if t.starts_with("pdf/") {
            out.push(strip_fragment(&t).to_string());
        }
    }
    for img in gpui_markdown::images(content) {
        if img.src.starts_with("pdf/") {
            out.push(img.src.to_string());
        }
    }
    out
}

/// Drop a trailing `#p6`-style page fragment from a ref.
fn strip_fragment(path: &str) -> &str {
    path.split('#').next().unwrap_or(path)
}

/// The filename part of a `dir/name.ext` ref (fragment stripped).
fn file_name(path: &str) -> String {
    let p = strip_fragment(path);
    p.rsplit('/').next().unwrap_or(p).to_string()
}

/// Every file of `kind` in the managed store — for browsing with an empty term.
fn all_files(kind: Kind) -> Vec<Hit> {
    let sub = match kind {
        Kind::Pdf => "pdf",
        _ => "images",
    };
    let dir = crate::paths::data_dir().join(sub);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<Hit> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                return None;
            }
            match kind {
                Kind::Pdf if name.to_lowercase().ends_with(".pdf") => Some(Hit {
                    kind,
                    title: name,
                    subtitle: "PDF".into(),
                    target: Target::Pdf(e.path()),
                }),
                Kind::Image => Some(Hit {
                    kind,
                    title: name.clone(),
                    subtitle: "Image".into(),
                    target: Target::Image {
                        src: format!("images/{name}"),
                        in_page: None,
                    },
                }),
                _ => None,
            }
        })
        .collect();
    out.sort_by_key(|h| h.title.to_lowercase());
    out.truncate(LIMIT as usize);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_prefix_splits_known_filters() {
        assert!(matches!(parse_prefix("pdf:estimate"), (Filter::Pdf, t) if t == "estimate"));
        assert!(matches!(parse_prefix("img:  cat "), (Filter::Image, t) if t == "cat"));
        assert!(matches!(parse_prefix("page:notes"), (Filter::Page, t) if t == "notes"));
        // No (or unknown) prefix → All, with the whole query as the term.
        assert!(matches!(parse_prefix("estimate"), (Filter::All, t) if t == "estimate"));
        assert!(matches!(parse_prefix("foo:bar"), (Filter::All, t) if t == "foo:bar"));
    }

    #[test]
    fn pdf_refs_finds_wikilinks_and_chips_without_fragments() {
        // PDFs appear as inline `[[pdf/…]]` wiki-links (found anywhere) and as
        // standalone `![](pdf/…)` chips (one per block, like real notes).
        let md = "- [[pdf/Estimate.pdf#p6|↗]]\n- ![](pdf/Plan.pdf)\n- ![](images/x.png)";
        let mut refs = pdf_refs(md);
        refs.sort();
        assert_eq!(refs, vec!["pdf/Estimate.pdf", "pdf/Plan.pdf"]);
    }

    #[test]
    fn file_name_strips_dir_and_fragment() {
        assert_eq!(file_name("pdf/Estimate.pdf#p6"), "Estimate.pdf");
        assert_eq!(file_name("images/2024-03-15.jpeg"), "2024-03-15.jpeg");
    }

    #[test]
    fn run_finds_pages_and_image_files_and_filters_by_kind() {
        let db = crate::db::Db::open_in_memory().unwrap();
        let p = db.get_or_create_page("Daily Notes").unwrap();
        db.set_page_content(p.id, "groceries\n- ![mango photo](images/mango_crate.jpg)")
            .unwrap();

        // Bare query: the page matches and so does its image (by alt + filename).
        let r = run(&db, "mango");
        assert_eq!(r.filter, Filter::All);
        assert!(r.counts.image >= 1, "image counted for the chip");
        assert!(
            r.hits.iter().any(|h| h.kind == Kind::Image),
            "image hit present"
        );

        // `img:` keeps only image hits; `page:` keeps only page hits.
        let imgs = run(&db, "img:mango");
        assert!(!imgs.hits.is_empty() && imgs.hits.iter().all(|h| h.kind == Kind::Image));
        let pages = run(&db, "page:daily");
        assert!(!pages.hits.is_empty() && pages.hits.iter().all(|h| h.kind == Kind::Page));
    }

    #[test]
    fn whiteboards_match_by_title_only() {
        let db = crate::db::Db::open_in_memory().unwrap();
        let board = db.create_whiteboard().unwrap(); // "Untitled Whiteboard"
        // Even with searchable-looking text in the canvas JSON, only the title
        // matches — content is never indexed for boards.
        db.set_page_content(board.id, r#"{"marker":"zphirium"}"#)
            .unwrap();

        // The auto title contains "whiteboard", so a title term finds it.
        let r = run(&db, "untitled");
        assert!(r.counts.whiteboard >= 1, "board counted for the chip");
        assert!(r.hits.iter().any(|h| h.kind == Kind::Whiteboard));
        // Content text does NOT surface the board.
        assert!(
            run(&db, "zphirium")
                .hits
                .iter()
                .all(|h| h.kind != Kind::Whiteboard)
        );
        // `wb:` keeps only boards; with no term it browses them all.
        let only = run(&db, "wb:untitled");
        assert!(!only.hits.is_empty() && only.hits.iter().all(|h| h.kind == Kind::Whiteboard));
        assert!(!run(&db, "wb:").hits.is_empty(), "wb: browses all boards");
    }
}
