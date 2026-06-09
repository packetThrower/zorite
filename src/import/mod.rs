//! Importing notes from other software.
//!
//! Imports are two-phase: a **reader** for each source turns its files into a
//! source-agnostic [`ImportBundle`] (pure filesystem + string work, no DB),
//! and the **engine** here writes any bundle into zorite — one shared
//! implementation of the collision policy (existing content stays, imported
//! text appends below), `[[link]]`/`#tag` re-indexing, alias merging, and
//! asset copying into the managed `images/`/`pdf/` stores.
//!
//! # Adding an importer
//!
//! 1. Add a module (like [`logseq`]) exposing a `read_*(root, &Options) ->
//!    Result<ImportBundle, String>` for the source's layout, plus whatever
//!    source-specific `Options` it needs.
//! 2. Give it a `File → Import` menu entry and an options dialog
//!    (see `AppView::on_import_logseq` — the picker → options → background
//!    thread → summary flow is the same for every source).
//! 3. Run the bundle through [`write_bundle`]. Done — collisions, link
//!    indexing, aliases, assets, and the summary all come with it.

pub mod logseq;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::db::Db;

/// What a reader produces: everything destined for the database and the
/// managed asset stores, as plain data.
#[derive(Default)]
pub struct ImportBundle {
    pub pages: Vec<ImportPage>,
    pub days: Vec<ImportDay>,
    pub assets: Vec<AssetCopy>,
    /// Non-fatal problems found while reading (missing assets, …).
    pub warnings: Vec<String>,
}

/// A named page to import.
pub struct ImportPage {
    pub title: String,
    /// zorite-flavored markdown.
    pub content: String,
    /// Extra names for the page, merged into zorite's alias table.
    pub aliases: Vec<String>,
    /// A `<name>.pdf (highlights)` page (counted separately in the summary).
    pub is_highlights: bool,
}

/// A journal day to import.
pub struct ImportDay {
    /// ISO `YYYY-MM-DD`.
    pub date: String,
    pub content: String,
}

/// An asset file to copy into the managed stores.
pub struct AssetCopy {
    pub src: PathBuf,
    /// Data-dir-relative destination, e.g. `images/x.png` or `pdf/x.pdf` —
    /// the same string the imported markdown references.
    pub managed: String,
}

/// What an import did, for the summary dialog.
#[derive(Default)]
pub struct Summary {
    pub pages: usize,
    pub journals: usize,
    pub highlight_pages: usize,
    pub assets_copied: usize,
    /// Pages/days that already had content; the import appended below it.
    pub appended: Vec<String>,
    /// Non-fatal problems (missing assets, unparseable files, …).
    pub warnings: Vec<String>,
}

/// Write a bundle into the database and copy its assets under `data_dir`
/// (the app passes [`crate::paths::data_dir`]; tests a temp dir).
/// `progress(done, total)` is called per page/day.
pub fn write_bundle(
    db: &Db,
    data_dir: &Path,
    bundle: ImportBundle,
    mut progress: impl FnMut(usize, usize),
) -> Result<Summary, String> {
    let mut summary = Summary {
        warnings: bundle.warnings,
        ..Summary::default()
    };
    let total = bundle.pages.len() + bundle.days.len();
    let mut done = 0;
    for page in &bundle.pages {
        progress(done, total);
        done += 1;
        write_page(db, &page.title, &page.content, &page.aliases, &mut summary)?;
        if page.is_highlights {
            summary.highlight_pages += 1;
        } else {
            summary.pages += 1;
        }
    }
    for day in &bundle.days {
        progress(done, total);
        done += 1;
        write_journal(db, &day.date, &day.content, &mut summary)?;
        summary.journals += 1;
    }
    progress(total, total);

    // Copy assets (deduped by destination; an existing file is assumed to be
    // the same asset from an earlier run and left alone).
    let mut seen: HashSet<&str> = HashSet::new();
    for copy in &bundle.assets {
        if !seen.insert(&copy.managed) {
            continue;
        }
        let dest = data_dir.join(&copy.managed);
        if dest.exists() {
            continue;
        }
        if let Some(dir) = dest.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match std::fs::copy(&copy.src, &dest) {
            Ok(_) => summary.assets_copied += 1,
            Err(e) => summary
                .warnings
                .push(format!("copy {}: {e}", copy.src.display())),
        }
    }
    summary.warnings.dedup();
    Ok(summary)
}

/// Create-or-append a named page, then refresh its aliases and link index.
fn write_page(
    db: &Db,
    title: &str,
    content: &str,
    aliases: &[String],
    summary: &mut Summary,
) -> Result<(), String> {
    let page = db
        .get_or_create_page(title)
        .map_err(|e| format!("create page {title}: {e}"))?;
    let merged = append_below(&page.content, content, title, summary);
    db.set_page_content(page.id, &merged)
        .map_err(|e| format!("save page {title}: {e}"))?;
    db.rebuild_page_links(page.id, &link_targets(&merged))
        .map_err(|e| format!("index links for {title}: {e}"))?;
    if !aliases.is_empty() {
        let mut all = db.get_page_aliases(page.id).unwrap_or_default();
        for a in aliases {
            if !all.iter().any(|x| x.eq_ignore_ascii_case(a)) {
                all.push(a.clone());
            }
        }
        db.rebuild_page_aliases(page.id, &all)
            .map_err(|e| format!("save aliases for {title}: {e}"))?;
    }
    Ok(())
}

/// Create-or-append a journal day, then refresh its link index.
fn write_journal(db: &Db, date: &str, content: &str, summary: &mut Summary) -> Result<(), String> {
    let page = db
        .get_or_create_journal(date)
        .map_err(|e| format!("create journal {date}: {e}"))?;
    let merged = append_below(&page.content, content, date, summary);
    db.set_page_content(page.id, &merged)
        .map_err(|e| format!("save journal {date}: {e}"))?;
    db.rebuild_page_links(page.id, &link_targets(&merged))
        .map_err(|e| format!("index links for {date}: {e}"))?;
    Ok(())
}

/// Link targets to index, skipping managed-store refs — `[[pdf/x.pdf#p3|↗]]`
/// jump-links aren't pages and indexing them would create junk page rows.
fn link_targets(content: &str) -> Vec<String> {
    let mut titles = crate::ui::links::parse_links(content);
    titles.retain(|t| !t.starts_with("pdf/") && !t.starts_with("images/"));
    titles
}

/// Existing content stays; the imported content lands below it.
fn append_below(existing: &str, imported: &str, name: &str, summary: &mut Summary) -> String {
    if existing.trim().is_empty() {
        imported.to_string()
    } else {
        summary.appended.push(name.to_string());
        format!("{}\n\n{imported}", existing.trim_end())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_bundle_end_to_end() {
        let dir = std::env::temp_dir().join("zorite-test-import-engine");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pic.png"), b"png").unwrap();

        let bundle = ImportBundle {
            pages: vec![ImportPage {
                title: "Projects::Lab".into(),
                content: "see [[Other]] and ![x](images/pic.png) [[pdf/x.pdf#p1|↗]]".into(),
                aliases: vec!["lab".into()],
                is_highlights: false,
            }],
            days: vec![ImportDay {
                date: "2024-02-07".into(),
                content: "met [[Alan]]".into(),
            }],
            assets: vec![
                AssetCopy {
                    src: dir.join("pic.png"),
                    managed: "images/pic.png".into(),
                },
                // Duplicate destination — copied once.
                AssetCopy {
                    src: dir.join("pic.png"),
                    managed: "images/pic.png".into(),
                },
            ],
            warnings: vec!["reader warning".into()],
        };

        let db = Db::open_in_memory().unwrap();
        let summary = write_bundle(&db, &dir, bundle, |_, _| {}).unwrap();
        assert_eq!((summary.pages, summary.journals), (1, 1));
        assert_eq!(summary.assets_copied, 1);
        assert_eq!(summary.warnings, vec!["reader warning".to_string()]);
        assert!(dir.join("images/pic.png").is_file());

        let page = db.get_page_by_title("Projects::Lab").unwrap().unwrap();
        assert_eq!(db.get_page_aliases(page.id).unwrap(), vec!["lab"]);
        // Real links indexed; the pdf jump-link did NOT become a page.
        assert!(db.get_page_by_title("Other").unwrap().is_some());
        assert!(db.get_page_by_title("pdf/x.pdf#p1|↗").unwrap().is_none());
        assert!(db.get_journal_by_date("2024-02-07").unwrap().is_some());

        // A second write appends below instead of clobbering.
        let again = ImportBundle {
            pages: vec![ImportPage {
                title: "Projects::Lab".into(),
                content: "more".into(),
                aliases: vec![],
                is_highlights: false,
            }],
            ..ImportBundle::default()
        };
        let summary = write_bundle(&db, &dir, again, |_, _| {}).unwrap();
        assert_eq!(summary.appended, vec!["Projects::Lab".to_string()]);
        let page = db.get_page_by_title("Projects::Lab").unwrap().unwrap();
        assert!(page.content.ends_with("\n\nmore"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
